//! SQLite adapter for the `DispatchRepository` port.
//!
//! One row per dispatch job — the enum-carrying `DispatchState` is
//! stored as a slug column plus three payload columns
//! (`state_message`, `progress_current`, `progress_total`) that
//! together round-trip the variants without a JSON-tagged shape (the
//! wire DTO does the same split).

use asterism_core::domain::dispatch::{DispatchJob, DispatchState};
use asterism_core::domain::repository::DispatchRepository;
use asterism_core::domain::value::{AssetId, CorrelationId, DispatchId, PersonaId, SnapshotId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `DispatchRepository`.
#[derive(Clone)]
pub struct SqliteDispatchRepository {
    isle: AsyncIsle,
}

impl SqliteDispatchRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

struct DispatchRow {
    id: Uuid,
    snapshot_id: Uuid,
    persona_id: Uuid,
    exporter_slug: String,
    action: String,
    params_json: String,
    state_slug: String,
    state_message: Option<String>,
    progress_current: Option<i64>,
    progress_total: Option<i64>,
    handle_kind: Option<String>,
    handle_payload: Option<String>,
    // The exporter's record of its latest call (V83). NULL on a row
    // whose exporter recorded nothing, and on every row written before
    // the column — the calls those describe are over, so there is
    // nothing to backfill from.
    attempt_kind: Option<String>,
    attempt_payload: Option<String>,
    output_asset_ids_json: String,
    created_at: i64,
    updated_at: i64,
    completed_at: Option<i64>,
    source_group_id: Option<Uuid>,
    source_query_json: Option<String>,
    operator_ai: Option<String>,
    // Attribution the request carried (V50): the author pair alongside
    // the operator V48 already stored, plus the channel all three
    // arrived through. NULL on a V48-era row, which records an operator
    // and no channel.
    author_kind: Option<String>,
    author_subject: Option<String>,
    attributed_via: Option<String>,
    // The pursuit stamp (V79). NULL wherever the caller named no
    // pursuit, which is most rounds; the V79 backfill is why no
    // pre-V79 row reads that way by accident of age.
    pursuit_id: Option<Uuid>,
}

impl DispatchRow {
    const COLUMNS: &'static str = "id, snapshot_id, persona_id, exporter_slug, action,
                                   params_json, state_slug, state_message,
                                   progress_current, progress_total,
                                   handle_kind, handle_payload,
                                   output_asset_ids, created_at, updated_at, completed_at,
                                   source_group_id, source_query_json, operator_ai,
                                   author_kind, author_subject, attributed_via, pursuit_id,
                                   attempt_kind, attempt_payload";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            snapshot_id: row.get(1)?,
            persona_id: row.get(2)?,
            exporter_slug: row.get(3)?,
            action: row.get(4)?,
            params_json: row.get(5)?,
            state_slug: row.get(6)?,
            state_message: row.get(7)?,
            progress_current: row.get(8)?,
            progress_total: row.get(9)?,
            handle_kind: row.get(10)?,
            handle_payload: row.get(11)?,
            output_asset_ids_json: row.get(12)?,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
            completed_at: row.get(15)?,
            source_group_id: row.get(16)?,
            source_query_json: row.get(17)?,
            operator_ai: row.get(18)?,
            author_kind: row.get(19)?,
            author_subject: row.get(20)?,
            attributed_via: row.get(21)?,
            pursuit_id: row.get(22)?,
            // Read from the tail of the select rather than from beside
            // the handle: the column list is append-only for the same
            // reason the migrations are, and inserting in the middle
            // renumbers every index below it for no gain in what the
            // query returns.
            attempt_kind: row.get(23)?,
            attempt_payload: row.get(24)?,
        })
    }

    fn into_domain(self) -> Result<DispatchJob, DomainError> {
        let params: serde_json::Value = serde_json::from_str(&self.params_json)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt params_json: {e}")))?;
        let state = match self.state_slug.as_str() {
            "pending" => DispatchState::Pending,
            "running" => DispatchState::Running {
                current: self.progress_current.map(|v| v.max(0) as u64),
                total: self.progress_total.map(|v| v.max(0) as u64),
                message: self.state_message,
            },
            "done" => DispatchState::Done,
            "failed" => DispatchState::Failed {
                message: self.state_message.unwrap_or_default(),
            },
            "cancelled" => DispatchState::Cancelled {
                reason: self.state_message,
            },
            other => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "unknown state_slug: {other:?}"
                )));
            }
        };
        let handle =
            match self.handle_payload {
                None => None,
                Some(text) => Some(serde_json::from_str(&text).map_err(|e| {
                    DomainError::Infra(anyhow::anyhow!("corrupt handle_payload: {e}"))
                })?),
            };
        let attempt = match self.attempt_payload {
            None => None,
            Some(text) => Some(serde_json::from_str(&text).map_err(|e| {
                DomainError::Infra(anyhow::anyhow!("corrupt attempt_payload: {e}"))
            })?),
        };
        let output_ids: Vec<String> = serde_json::from_str(&self.output_asset_ids_json)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt output_asset_ids: {e}")))?;
        let output_asset_ids = output_ids
            .into_iter()
            .map(|s| {
                Uuid::parse_str(&s)
                    .map(AssetId::from_uuid)
                    .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt output uuid: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // The same reader the asset side uses, so a corrupt pair, a
        // blank operator (an assertion that says nothing, which must not
        // be storable as one that does) or an unknown channel fails
        // identically on both tables instead of degrading into a
        // plausible-looking attribution.
        let attribution = asterism_core::domain::attribution::PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        // Seeded through the hydration constructor because the
        // attribution fields are private; the lifecycle columns are
        // assigned below.
        let mut job = DispatchJob::from_persisted(
            DispatchId::from_uuid(self.id),
            SnapshotId::from_uuid(self.snapshot_id),
            PersonaId::from_uuid(self.persona_id),
            self.exporter_slug,
            self.action,
            params,
            ms_to_datetime(self.created_at)?,
            ms_to_datetime(self.updated_at)?,
            attribution,
        );
        job.state = state;
        job.handle = handle;
        job.handle_kind = self.handle_kind;
        job.attempt = attempt;
        job.attempt_kind = self.attempt_kind;
        job.output_asset_ids = output_asset_ids;
        job.source_group_id = self
            .source_group_id
            .map(asterism_core::domain::value::GroupId::from_uuid);
        job.source_query_json = self.source_query_json;
        job.pursuit_id = self.pursuit_id.map(CorrelationId::from_uuid);
        job.completed_at = self.completed_at.map(ms_to_datetime).transpose()?;
        Ok(job)
    }
}

#[async_trait]
impl DispatchRepository for SqliteDispatchRepository {
    async fn find(&self, id: &DispatchId) -> Result<Option<DispatchJob>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM dispatch_job WHERE id = ?1",
                        DispatchRow::COLUMNS
                    ),
                    params![uuid],
                    DispatchRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(DispatchRow::into_domain).transpose()
    }

    async fn save(&self, job: &DispatchJob) -> Result<(), DomainError> {
        let id = *job.id.as_uuid();
        let snapshot_id = *job.snapshot_id.as_uuid();
        let persona_id = *job.persona_id.as_uuid();
        let exporter_slug = job.exporter_slug.clone();
        let action = job.action.clone();
        let params_json = job.params.to_string();
        let state_slug = job.state.slug().to_string();
        let (state_message, progress_current, progress_total) = match &job.state {
            DispatchState::Pending | DispatchState::Done => (None, None, None),
            DispatchState::Running {
                current,
                total,
                message,
            } => (
                message.clone(),
                current.map(|v| v as i64),
                total.map(|v| v as i64),
            ),
            DispatchState::Failed { message } => (Some(message.clone()), None, None),
            DispatchState::Cancelled { reason } => (reason.clone(), None, None),
        };
        let handle_kind = job.handle_kind.clone();
        let handle_payload = job.handle.as_ref().map(|v| v.to_string());
        let attempt_kind = job.attempt_kind.clone();
        let attempt_payload = job.attempt.as_ref().map(|v| v.to_string());
        let output_ids: Vec<String> = job.output_asset_ids.iter().map(|a| a.to_string()).collect();
        let output_json = serde_json::Value::Array(
            output_ids
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        )
        .to_string();
        let created = datetime_to_ms(&job.created_at);
        let updated = datetime_to_ms(&job.updated_at);
        let completed = job.completed_at.as_ref().map(datetime_to_ms);
        let source_group = job.source_group_id.map(|g| *g.as_uuid());
        let source_query = job.source_query_json.clone();
        let operator_ai = job.operator_ai().map(|o| o.as_str().to_string());
        // The author pair is written from `Author::encode`, so the two
        // halves can never disagree; the channel travels with it.
        let (author_kind, author_subject) = match job.author() {
            Some(author) => {
                let (kind, subject) = author.encode();
                (Some(kind.to_string()), subject.map(str::to_string))
            }
            None => (None, None),
        };
        let attributed_via = job.attributed_via().map(|c| c.slug().to_string());
        let pursuit_id = job.pursuit_id.map(|p| *p.as_uuid());
        // Same write-side rule as the asset table: a row that records
        // somebody records the channel that answer arrived through.
        super::attribution_guard::assert_channel_recorded(
            "dispatch_job",
            author_kind.as_deref(),
            operator_ai.as_deref(),
            attributed_via.as_deref(),
        )?;
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO dispatch_job
                         (id, snapshot_id, persona_id, exporter_slug, action, params_json,
                          state_slug, state_message, progress_current, progress_total,
                          handle_kind, handle_payload, output_asset_ids,
                          created_at, updated_at, completed_at,
                          source_group_id, source_query_json, operator_ai,
                          author_kind, author_subject, attributed_via, pursuit_id,
                          attempt_kind, attempt_payload)
                     VALUES
                         (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                          ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
                     -- The attribution columns are absent from the
                     -- update set for the same reason `source_*` is: who
                     -- asked for the run, and how the request arrived,
                     -- are settled when the row is created, and the
                     -- runner's own saves (state, handle, outputs) have
                     -- nothing to say about them. `pursuit_id` is absent
                     -- for a stronger reason: the restamp verb is the
                     -- only sanctioned mover, and a runner save carrying
                     -- a stale in-memory stamp must not undo a recorded
                     -- restamp that landed in between.
                     ON CONFLICT(id) DO UPDATE SET
                         exporter_slug = excluded.exporter_slug,
                         action = excluded.action,
                         params_json = excluded.params_json,
                         state_slug = excluded.state_slug,
                         state_message = excluded.state_message,
                         progress_current = excluded.progress_current,
                         progress_total = excluded.progress_total,
                         handle_kind = excluded.handle_kind,
                         handle_payload = excluded.handle_payload,
                         attempt_kind = excluded.attempt_kind,
                         attempt_payload = excluded.attempt_payload,
                         output_asset_ids = excluded.output_asset_ids,
                         updated_at = excluded.updated_at,
                         completed_at = excluded.completed_at",
                    params![
                        id,
                        snapshot_id,
                        persona_id,
                        exporter_slug,
                        action,
                        params_json,
                        state_slug,
                        state_message,
                        progress_current,
                        progress_total,
                        handle_kind,
                        handle_payload,
                        output_json,
                        created,
                        updated,
                        completed,
                        source_group,
                        source_query,
                        operator_ai,
                        author_kind,
                        author_subject,
                        attributed_via,
                        pursuit_id,
                        attempt_kind,
                        attempt_payload,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn list_rounds(
        &self,
        pursuit_id: &CorrelationId,
    ) -> Result<Vec<DispatchJob>, DomainError> {
        let uuid = *pursuit_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM dispatch_job WHERE pursuit_id = ?1 \
                     ORDER BY created_at ASC, id ASC",
                    DispatchRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![uuid], DispatchRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(DispatchRow::into_domain).collect()
    }

    async fn list(
        &self,
        persona_id: Option<&PersonaId>,
        snapshot_id: Option<&SnapshotId>,
        state_slug: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DispatchJob>, DomainError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let persona_uuid = persona_id.map(|p| *p.as_uuid());
        let snapshot_uuid = snapshot_id.map(|s| *s.as_uuid());
        let state_owned = state_slug.map(|s| s.to_string());
        let cap = limit as i64;
        let rows = self
            .isle
            .call(move |conn| {
                let mut clauses: Vec<&str> = Vec::new();
                if persona_uuid.is_some() {
                    clauses.push("persona_id = :persona_id");
                }
                if snapshot_uuid.is_some() {
                    clauses.push("snapshot_id = :snapshot_id");
                }
                if state_owned.is_some() {
                    clauses.push("state_slug = :state_slug");
                }
                let where_clause = if clauses.is_empty() {
                    String::new()
                } else {
                    format!("WHERE {}", clauses.join(" AND "))
                };
                let sql = format!(
                    "SELECT {} FROM dispatch_job {}
                        ORDER BY created_at DESC, id
                        LIMIT :cap",
                    DispatchRow::COLUMNS,
                    where_clause
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut named: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
                if let Some(p) = &persona_uuid {
                    named.push((":persona_id", p));
                }
                if let Some(s) = &snapshot_uuid {
                    named.push((":snapshot_id", s));
                }
                if let Some(s) = &state_owned {
                    named.push((":state_slug", s));
                }
                named.push((":cap", &cap));
                let rows = stmt
                    .query_map(&named[..], DispatchRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(DispatchRow::into_domain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::attribution::{
        AttributionChannel, AttributionContext, Author, OperatorRef,
    };

    use crate::sqlite::open_and_migrate_in_memory;

    /// The unrecorded context, spelled the way a crate outside
    /// `asterism-core` has to spell it: `unrecorded()` is crate-private
    /// there on purpose, and an empty assertion is defined to be the
    /// same value (attribution rule 3).
    fn nobody() -> AttributionContext {
        AttributionContext::asserted(None, None).unwrap()
    }

    /// Persona + snapshot, the two foreign keys a dispatch row needs.
    async fn seed_snapshot(isle: &AsyncIsle) -> (Uuid, Uuid) {
        let persona = Uuid::now_v7();
        let snapshot = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, 'P', 0, 0)",
                params![persona, format!("p-{persona}")],
            )?;
            conn.execute(
                "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
                 VALUES (?1, ?2, ?3, 0)",
                params![snapshot, persona, format!("h-{snapshot}")],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        (persona, snapshot)
    }

    fn job(persona: Uuid, snapshot: Uuid, attribution: &AttributionContext) -> DispatchJob {
        DispatchJob::new(
            SnapshotId::from_uuid(snapshot),
            PersonaId::from_uuid(persona),
            "comfy",
            "run",
            serde_json::json!({}),
            chrono::Utc::now(),
            attribution,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_requested_attribution_round_trips_with_its_channel() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let (persona, snapshot) = seed_snapshot(&isle).await;
        let repo = SqliteDispatchRepository::new(isle.clone());

        // The triple has to survive the row, because the outputs are
        // stamped from it minutes or hours later — long after the
        // request that supplied it is gone.
        let requested = job(
            persona,
            snapshot,
            &AttributionContext::asserted(
                Some(Author::Subject("alice".into())),
                Some(OperatorRef::new("claude-code").unwrap()),
            )
            .unwrap(),
        );
        repo.save(&requested).await.unwrap();

        let loaded = repo.find(&requested.id).await.unwrap().unwrap();
        assert_eq!(loaded.author(), Some(&Author::Subject("alice".into())));
        assert_eq!(
            loaded.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(loaded.attributed_via(), Some(AttributionChannel::Asserted));

        // The owner half of the pair (kind without a subject) is the
        // shape the owner's own surface writes.
        let owned = job(persona, snapshot, &AttributionContext::owner_surface());
        repo.save(&owned).await.unwrap();
        let loaded_owner = repo.find(&owned.id).await.unwrap().unwrap();
        assert_eq!(loaded_owner.author(), Some(&Author::Owner));
        assert_eq!(
            loaded_owner.attributed_via(),
            Some(AttributionChannel::OwnerSurface)
        );

        // Recording nobody round-trips as nobody, never as the owner.
        let silent = job(persona, snapshot, &nobody());
        repo.save(&silent).await.unwrap();
        let loaded_silent = repo.find(&silent.id).await.unwrap().unwrap();
        assert_eq!(
            (
                loaded_silent.author().cloned(),
                loaded_silent.operator_ai().cloned(),
                loaded_silent.attributed_via()
            ),
            (None, None, None)
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_v48_era_row_reads_back_as_an_operator_without_a_channel() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let (persona, snapshot) = seed_snapshot(&isle).await;
        let repo = SqliteDispatchRepository::new(isle.clone());

        // Written straight into the table, because a row of this shape
        // is exactly what the entity path is meant to stop producing:
        // an operator recorded with no channel behind it.
        let legacy = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO dispatch_job (id, snapshot_id, persona_id, exporter_slug, action, \
                                           params_json, state_slug, output_asset_ids, \
                                           created_at, updated_at, operator_ai) \
                 VALUES (?1, ?2, ?3, 'comfy', 'run', '{}', 'pending', '[]', 0, 0, 'claude-code')",
                params![legacy, snapshot, persona],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let loaded = repo
            .find(&DispatchId::from_uuid(legacy))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(loaded.author(), None);
        assert_eq!(
            loaded.attributed_via(),
            None,
            "the read accepts the legacy shape instead of guessing the channel it never had"
        );

        // …and the write side refuses to mint another one. Reading a
        // legacy row back and saving it unchanged is the shortest path
        // to that shape, and the guard is what stops it: a row landing
        // in the legacy bucket today would be indistinguishable from one
        // that predates the column.
        let err = repo
            .save(&loaded)
            .await
            .expect_err("an operator with no channel cannot be written anew");
        assert!(
            err.to_string().contains("attribution without a channel"),
            "the refusal should name the rule it enforces: {err}"
        );

        driver.shutdown().await.unwrap();
    }
}
