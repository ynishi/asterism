//! SQLite adapter for the `PursuitRepository` port (#29).
//!
//! Three tables, one concern: `pursuit` (thin, immutable, insert-only),
//! `pursuit_event` (append-only lifecycle facts), and `pursuit_restamp`
//! (the recorded repair verb). The one multi-table write — restamp —
//! runs in a single transaction here, because "the move is recorded"
//! and "the stamp moved" must not be separable facts.

use asterism_core::domain::attribution::PersistedAttribution;
use asterism_core::domain::pursuit::{
    Pursuit, PursuitEvent, PursuitEventKind, PursuitRestamp, RestampSubject,
};
use asterism_core::domain::repository::PursuitRepository;
use asterism_core::domain::value::{AssetId, PersonaId, PursuitEventId, PursuitId, SnapshotId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `PursuitRepository`.
#[derive(Clone)]
pub struct SqlitePursuitRepository {
    isle: AsyncIsle,
}

impl SqlitePursuitRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// The four attribution column values in write order:
/// `(author_kind, author_subject, operator_ai, attributed_via)`.
type AttributionColumns = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Encodes an entity's attribution triple into the three column values,
/// running the same write-side channel guard as `asset` /
/// `dispatch_job` (a row that records somebody records the channel the
/// answer arrived through).
fn attribution_columns(
    table: &'static str,
    attribution: &PersistedAttribution,
) -> Result<AttributionColumns, DomainError> {
    let (author_kind, author_subject) = match attribution.author() {
        Some(author) => {
            let (kind, subject) = author.encode();
            (Some(kind.to_string()), subject.map(str::to_string))
        }
        None => (None, None),
    };
    let operator_ai = attribution.operator_ai().map(|o| o.as_str().to_string());
    let attributed_via = attribution.attributed_via().map(|c| c.slug().to_string());
    super::attribution_guard::assert_channel_recorded(
        table,
        author_kind.as_deref(),
        operator_ai.as_deref(),
        attributed_via.as_deref(),
    )?;
    Ok((author_kind, author_subject, operator_ai, attributed_via))
}

struct PursuitRow {
    id: Uuid,
    persona_id: Uuid,
    parent_id: Option<Uuid>,
    title: Option<String>,
    note: Option<String>,
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
    attributed_via: Option<String>,
    created_at: i64,
}

impl PursuitRow {
    const COLUMNS: &'static str = "id, persona_id, parent_id, title, note,
                                   author_kind, author_subject, operator_ai, attributed_via,
                                   created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            parent_id: row.get(2)?,
            title: row.get(3)?,
            note: row.get(4)?,
            author_kind: row.get(5)?,
            author_subject: row.get(6)?,
            operator_ai: row.get(7)?,
            attributed_via: row.get(8)?,
            created_at: row.get(9)?,
        })
    }

    fn into_domain(self) -> Result<Pursuit, DomainError> {
        let attribution = PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        Ok(Pursuit::from_persisted(
            PursuitId::from_uuid(self.id),
            PersonaId::from_uuid(self.persona_id),
            self.parent_id.map(PursuitId::from_uuid),
            self.title,
            self.note,
            ms_to_datetime(self.created_at)?,
            attribution,
        ))
    }
}

struct EventRow {
    id: Uuid,
    pursuit_id: Uuid,
    persona_id: Uuid,
    kind: String,
    snapshot_id: Option<Uuid>,
    note: Option<String>,
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
    attributed_via: Option<String>,
    created_at: i64,
}

impl EventRow {
    const COLUMNS: &'static str = "id, pursuit_id, persona_id, kind, snapshot_id, note,
                                   author_kind, author_subject, operator_ai, attributed_via,
                                   created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            pursuit_id: row.get(1)?,
            persona_id: row.get(2)?,
            kind: row.get(3)?,
            snapshot_id: row.get(4)?,
            note: row.get(5)?,
            author_kind: row.get(6)?,
            author_subject: row.get(7)?,
            operator_ai: row.get(8)?,
            attributed_via: row.get(9)?,
            created_at: row.get(10)?,
        })
    }

    fn into_domain(self) -> Result<PursuitEvent, DomainError> {
        let attribution = PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        Ok(PursuitEvent::from_persisted(
            PursuitEventId::from_uuid(self.id),
            PursuitId::from_uuid(self.pursuit_id),
            PersonaId::from_uuid(self.persona_id),
            PursuitEventKind::parse(&self.kind)?,
            self.snapshot_id.map(SnapshotId::from_uuid),
            self.note,
            ms_to_datetime(self.created_at)?,
            attribution,
        ))
    }
}

/// What the restamp transaction saw, carried out of the `isle` closure
/// so the refusal can be typed as a domain error rather than smuggled
/// through `rusqlite::Error`.
enum RestampOutcome {
    Done,
    SubjectMissing,
    TargetMissing,
    PersonaMismatch,
    StaleFrom(Option<Uuid>),
}

#[async_trait]
impl PursuitRepository for SqlitePursuitRepository {
    async fn create(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
        let id = *pursuit.id.as_uuid();
        let persona_id = *pursuit.persona_id.as_uuid();
        let parent_id = pursuit.parent_id.map(|p| *p.as_uuid());
        let title = pursuit.title.clone();
        let note = pursuit.note.clone();
        let (author_kind, author_subject, operator_ai, attributed_via) =
            attribution_columns("pursuit", &pursuit.persisted_attribution())?;
        let created = datetime_to_ms(&pursuit.created_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO pursuit
                         (id, persona_id, parent_id, title, note,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id,
                        persona_id,
                        parent_id,
                        title,
                        note,
                        author_kind,
                        author_subject,
                        operator_ai,
                        attributed_via,
                        created,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn find(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT {} FROM pursuit WHERE id = ?1", PursuitRow::COLUMNS),
                    params![uuid],
                    PursuitRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(PursuitRow::into_domain).transpose()
    }

    async fn list(&self, persona_id: &PersonaId, limit: u32) -> Result<Vec<Pursuit>, DomainError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let persona = *persona_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM pursuit WHERE persona_id = ?1 \
                     ORDER BY created_at DESC, id DESC LIMIT ?2",
                    PursuitRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![persona, limit], PursuitRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(PursuitRow::into_domain).collect()
    }

    async fn append_event(&self, event: &PursuitEvent) -> Result<(), DomainError> {
        let id = *event.id.as_uuid();
        let pursuit_id = *event.pursuit_id.as_uuid();
        let persona_id = *event.persona_id.as_uuid();
        let kind = event.kind.slug().to_string();
        let snapshot_id = event.snapshot_id.map(|s| *s.as_uuid());
        let note = event.note.clone();
        let (author_kind, author_subject, operator_ai, attributed_via) =
            attribution_columns("pursuit_event", &event.persisted_attribution())?;
        let created = datetime_to_ms(&event.created_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO pursuit_event
                         (id, pursuit_id, persona_id, kind, snapshot_id, note,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        id,
                        pursuit_id,
                        persona_id,
                        kind,
                        snapshot_id,
                        note,
                        author_kind,
                        author_subject,
                        operator_ai,
                        attributed_via,
                        created,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn events_of(&self, pursuit_id: &PursuitId) -> Result<Vec<PursuitEvent>, DomainError> {
        let uuid = *pursuit_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM pursuit_event WHERE pursuit_id = ?1 \
                     ORDER BY created_at ASC, id ASC",
                    EventRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![uuid], EventRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(EventRow::into_domain).collect()
    }

    async fn restamp(&self, restamp: &PursuitRestamp) -> Result<(), DomainError> {
        let id = *restamp.id.as_uuid();
        let subject_kind = restamp.subject.kind_slug().to_string();
        let subject_id = restamp.subject.subject_uuid();
        let from = restamp.from_pursuit_id.map(|p| *p.as_uuid());
        let to = *restamp.to_pursuit_id.as_uuid();
        let (author_kind, author_subject, operator_ai, attributed_via) =
            attribution_columns("pursuit_restamp", &restamp.persisted_attribution())?;
        let created = datetime_to_ms(&restamp.created_at);
        // The stamped column lives on the subject's own table; today
        // that is only `dispatch_job` (the `judgment` variant arrives
        // with its table).
        let RestampSubject::Dispatch(_) = restamp.subject;
        let outcome = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let row: Option<(Option<Uuid>, Uuid)> = tx
                    .query_row(
                        "SELECT pursuit_id, persona_id FROM dispatch_job WHERE id = ?1",
                        params![subject_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                let (current, subject_persona) = match row {
                    None => return Ok(RestampOutcome::SubjectMissing),
                    Some(row) => row,
                };
                // The recorded `from` must be the filing the caller
                // looked at; a mismatch means the move was decided over
                // a stale read (or a restamp raced in between), and a
                // repair verb that guesses is worse than one that
                // refuses.
                if current != from {
                    return Ok(RestampOutcome::StaleFrom(current));
                }
                // A restamp never crosses personas, and the check lives
                // here rather than in a service: the persona purge
                // deletes this table through the pursuits it points at,
                // so one cross-persona row would leave a RESTRICT edge
                // no purge order can satisfy — the persona becomes
                // permanently unpurgeable. An invariant whose violation
                // is that expensive is enforced next to the write.
                let target_persona: Option<Uuid> = tx
                    .query_row(
                        "SELECT persona_id FROM pursuit WHERE id = ?1",
                        params![to],
                        |r| r.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                match target_persona {
                    None => return Ok(RestampOutcome::TargetMissing),
                    Some(persona) if persona != subject_persona => {
                        return Ok(RestampOutcome::PersonaMismatch);
                    }
                    Some(_) => {}
                }
                tx.execute(
                    "INSERT INTO pursuit_restamp
                         (id, subject_kind, subject_id, from_pursuit_id, to_pursuit_id,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id,
                        subject_kind,
                        subject_id,
                        from,
                        to,
                        author_kind,
                        author_subject,
                        operator_ai,
                        attributed_via,
                        created,
                    ],
                )?;
                tx.execute(
                    "UPDATE dispatch_job SET pursuit_id = ?1 WHERE id = ?2",
                    params![to, subject_id],
                )?;
                tx.commit()?;
                Ok(RestampOutcome::Done)
            })
            .await
            .map_err(infra_err)?;
        match outcome {
            RestampOutcome::Done => Ok(()),
            RestampOutcome::SubjectMissing => Err(DomainError::Conflict(format!(
                "restamp subject {subject_id} does not exist"
            ))),
            RestampOutcome::TargetMissing => Err(DomainError::Conflict(format!(
                "restamp target pursuit {} does not exist",
                restamp.to_pursuit_id
            ))),
            RestampOutcome::PersonaMismatch => Err(DomainError::Validation(format!(
                "restamp of {subject_id} to pursuit {} crosses personas; \
                 a filing never leaves its persona",
                restamp.to_pursuit_id
            ))),
            RestampOutcome::StaleFrom(current) => Err(DomainError::Conflict(format!(
                "restamp of {subject_id} recorded from={:?} but the current stamp is {:?}; \
                 re-read before moving the filing",
                restamp.from_pursuit_id.map(|p| p.to_string()),
                current.map(|c| c.to_string()),
            ))),
        }
    }

    async fn returns_of(&self, pursuit_id: &PursuitId) -> Result<Vec<AssetId>, DomainError> {
        let pursuit = *pursuit_id.as_uuid();
        let pursuit_str = pursuit_id.to_string();
        let rows: Vec<(Uuid, i64)> = self
            .isle
            .call(move |conn| {
                // The rounds first: their ids are what the dispatch-join
                // probe matches against. `_trace` stores them as
                // hyphenated strings, so the conversion happens here
                // rather than in SQL.
                let round_ids: Vec<String> = {
                    let mut stmt =
                        conn.prepare("SELECT id FROM dispatch_job WHERE pursuit_id = ?1")?;
                    stmt.query_map(params![pursuit], |r| r.get::<_, Uuid>(0))?
                        .map(|r| r.map(|u| u.to_string()))
                        .collect::<Result<_, _>>()?
                };
                let mut out: Vec<(Uuid, i64)> = Vec::new();
                // Probe 1, the dispatch join — chunked so a pursuit with
                // very many rounds never exceeds SQLite's bind limit.
                for chunk in round_ids.chunks(500) {
                    let placeholders = vec!["?"; chunk.len()].join(", ");
                    let sql = format!(
                        "SELECT id, created_at FROM asset \
                          WHERE trace_dispatch_id IN ({placeholders}) \
                            AND trace_dispatch_id IS NOT NULL \
                            AND folded_into IS NULL"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let hits = stmt
                        .query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                            Ok((r.get::<_, Uuid>(0)?, r.get::<_, i64>(1)?))
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    out.extend(hits);
                }
                // Probe 2, the direct claim — consumed only where no
                // dispatch hop resolved (`trace_dispatch_id IS NULL` is
                // that rule as a predicate), so a stale sidecar copy
                // loses to the join without adjudication.
                let mut stmt = conn.prepare(
                    "SELECT id, created_at FROM asset \
                      WHERE trace_pursuit_id = ?1 \
                        AND trace_pursuit_id IS NOT NULL \
                        AND trace_dispatch_id IS NULL \
                        AND folded_into IS NULL",
                )?;
                let hits = stmt
                    .query_map(params![pursuit_str], |r| {
                        Ok((r.get::<_, Uuid>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                out.extend(hits);
                Ok(out)
            })
            .await
            .map_err(infra_err)?;
        let mut rows = rows;
        rows.sort_by_key(|(id, created)| (*created, *id));
        rows.dedup_by_key(|(id, _)| *id);
        Ok(rows
            .into_iter()
            .map(|(id, _)| AssetId::from_uuid(id))
            .collect())
    }

    async fn latest_event_kinds(
        &self,
        persona_id: &PersonaId,
    ) -> Result<Vec<(PursuitId, PursuitEventKind)>, DomainError> {
        let persona = *persona_id.as_uuid();
        let rows: Vec<(Uuid, String)> = self
            .isle
            .call(move |conn| {
                // Greatest-per-group over the standing sort key
                // (created_at, id) — the same tie-break `standing`
                // derives with, so this read and the per-pursuit one
                // can never disagree.
                let mut stmt = conn.prepare(
                    "SELECT pursuit_id, kind FROM ( \
                        SELECT pursuit_id, kind, \
                               ROW_NUMBER() OVER ( \
                                   PARTITION BY pursuit_id \
                                   ORDER BY created_at DESC, id DESC \
                               ) AS rn \
                          FROM pursuit_event WHERE persona_id = ?1 \
                     ) WHERE rn = 1",
                )?;
                let rows = stmt
                    .query_map(params![persona], |r| {
                        Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(id, kind)| Ok((PursuitId::from_uuid(id), PursuitEventKind::parse(&kind)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::attribution::AttributionContext;
    use asterism_core::domain::pursuit::{PursuitStanding, standing};
    use asterism_core::domain::value::DispatchId;
    use chrono::{Duration, TimeZone, Utc};

    async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
        let persona = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, 'p', 0, 0, 0, 0)",
                params![persona],
            )
        })
        .await
        .unwrap();
        PersonaId::from_uuid(persona)
    }

    /// One asset row with a caller-supplied `_trace` note (raw JSON)
    /// and clock — the shape the ingest writes, seeded directly so the
    /// membership probes are tested against the storage contract.
    async fn seed_traced_asset(isle: &AsyncIsle, persona: PersonaId, trace: &str, at: i64) -> Uuid {
        let id = Uuid::now_v7();
        let persona = *persona.as_uuid();
        let extra = format!(r#"{{"_trace":{trace}}}"#);
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, occurred_at, created_at, updated_at, extra) \
                 VALUES (?1, ?2, 'fs', ?3, 'state', 0, ?4, ?4, ?5)",
                params![id, persona, format!("a-{id}.md"), at, extra],
            )
        })
        .await
        .unwrap();
        id
    }

    /// A dispatch row this test can restamp — snapshot included, since
    /// `dispatch_job.snapshot_id` is NOT NULL.
    async fn seed_dispatch(isle: &AsyncIsle, persona: PersonaId) -> DispatchId {
        let snapshot = Uuid::now_v7();
        let dispatch = Uuid::now_v7();
        let persona = *persona.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
                 VALUES (?1, ?2, ?3, 0)",
                params![snapshot, persona, format!("h-{snapshot}")],
            )?;
            conn.execute(
                "INSERT INTO dispatch_job \
                     (id, snapshot_id, persona_id, exporter_slug, action, state_slug, \
                      created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'file', 'copy', 'pending', 0, 0)",
                params![dispatch, snapshot, persona],
            )
        })
        .await
        .unwrap();
        DispatchId::from_uuid(dispatch)
    }

    #[tokio::test]
    async fn create_find_list_round_trips_the_row() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());

        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();
        let parent = Pursuit::new(
            persona,
            None,
            Some("line".into()),
            None,
            t0,
            &AttributionContext::owner_surface(),
        );
        repo.create(&parent).await.unwrap();
        let child = Pursuit::new(
            persona,
            Some(parent.id),
            None,
            Some("spawned".into()),
            t0 + Duration::seconds(1),
            &AttributionContext::owner_surface(),
        );
        repo.create(&child).await.unwrap();

        let found = repo.find(&parent.id).await.unwrap().unwrap();
        assert_eq!(found, parent);
        assert!(repo.find(&PursuitId::new()).await.unwrap().is_none());

        let listed = repo.list(&persona, 10).await.unwrap();
        assert_eq!(
            listed,
            vec![child.clone(), parent.clone()],
            "most-recent first"
        );
        assert_eq!(listed[0].parent_id, Some(parent.id));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn events_append_in_standing_order_and_derive() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();

        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();
        let pursuit = Pursuit::new(persona, None, None, None, t0, &ctx);
        repo.create(&pursuit).await.unwrap();
        assert_eq!(
            standing(&repo.events_of(&pursuit.id).await.unwrap()),
            PursuitStanding::Open,
            "no event means open"
        );

        let close = PursuitEvent::new(
            pursuit.id,
            persona,
            PursuitEventKind::ClosedSatisfied,
            None,
            Some("done, nothing kept".into()),
            t0 + Duration::seconds(1),
            &ctx,
        )
        .unwrap();
        let reopen = PursuitEvent::new(
            pursuit.id,
            persona,
            PursuitEventKind::Reopened,
            None,
            None,
            t0 + Duration::seconds(2),
            &ctx,
        )
        .unwrap();
        repo.append_event(&close).await.unwrap();
        repo.append_event(&reopen).await.unwrap();

        let events = repo.events_of(&pursuit.id).await.unwrap();
        assert_eq!(events, vec![close, reopen]);
        assert_eq!(standing(&events), PursuitStanding::Open);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn restamp_moves_the_stamp_and_refuses_stale_or_missing() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();

        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();
        let target = Pursuit::new(persona, None, None, None, t0, &ctx);
        repo.create(&target).await.unwrap();
        let dispatch = seed_dispatch(&isle, persona).await;

        // Legacy NULL → stamped is a legal repair (`from` = None).
        let repair = PursuitRestamp::new(
            RestampSubject::Dispatch(dispatch),
            None,
            target.id,
            t0 + Duration::seconds(1),
            &ctx,
        )
        .unwrap();
        repo.restamp(&repair).await.unwrap();
        let stamped: Option<Uuid> = {
            let dispatch = *dispatch.as_uuid();
            isle.call(move |conn| {
                conn.query_row(
                    "SELECT pursuit_id FROM dispatch_job WHERE id = ?1",
                    params![dispatch],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap()
        };
        assert_eq!(stamped, Some(*target.id.as_uuid()));

        // A stale `from` (the pre-repair NULL) is refused, and the
        // refusal writes nothing: no second restamp row, stamp intact.
        let stale = PursuitRestamp::new(
            RestampSubject::Dispatch(dispatch),
            None,
            Pursuit::new(persona, None, None, None, t0, &ctx).id,
            t0 + Duration::seconds(2),
            &ctx,
        )
        .unwrap();
        assert!(matches!(
            repo.restamp(&stale).await,
            Err(DomainError::Conflict(_))
        ));
        let rows: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM pursuit_restamp", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(rows, 1, "a refused restamp records nothing");

        let ghost = PursuitRestamp::new(
            RestampSubject::Dispatch(DispatchId::new()),
            Some(target.id),
            Pursuit::new(persona, None, None, None, t0, &ctx).id,
            t0 + Duration::seconds(3),
            &ctx,
        )
        .unwrap();
        assert!(matches!(
            repo.restamp(&ghost).await,
            Err(DomainError::Conflict(_))
        ));

        driver.shutdown().await.unwrap();
    }

    /// A filing never leaves its persona — and the refusal has to live
    /// in the adapter, because one cross-persona row makes the pointed
    /// persona permanently unpurgeable (the purge sweeps this table
    /// through the pursuits it references).
    #[tokio::test]
    async fn restamp_refuses_to_cross_personas() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona_a = seed_persona(&isle).await;
        let persona_b = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();

        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();
        let home = Pursuit::new(persona_a, None, None, None, t0, &ctx);
        let foreign = Pursuit::new(persona_b, None, None, None, t0, &ctx);
        repo.create(&home).await.unwrap();
        repo.create(&foreign).await.unwrap();
        let dispatch = seed_dispatch(&isle, persona_a).await;

        let crossing = PursuitRestamp::new(
            RestampSubject::Dispatch(dispatch),
            None,
            foreign.id,
            t0 + Duration::seconds(1),
            &ctx,
        )
        .unwrap();
        assert!(matches!(
            repo.restamp(&crossing).await,
            Err(DomainError::Validation(_))
        ));
        let (rows, stamp): (i64, Option<Uuid>) = {
            let dispatch = *dispatch.as_uuid();
            isle.call(move |conn| {
                let rows =
                    conn.query_row("SELECT COUNT(*) FROM pursuit_restamp", [], |r| r.get(0))?;
                let stamp = conn.query_row(
                    "SELECT pursuit_id FROM dispatch_job WHERE id = ?1",
                    params![dispatch],
                    |r| r.get(0),
                )?;
                Ok((rows, stamp))
            })
            .await
            .unwrap()
        };
        assert_eq!(
            (rows, stamp),
            (0, None),
            "a refused crossing writes nothing"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn returns_follow_the_dispatch_join_first_and_the_claim_second() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let home = Pursuit::new(persona, None, None, None, t0, &ctx);
        let other = Pursuit::new(persona, None, None, None, t0, &ctx);
        repo.create(&home).await.unwrap();
        repo.create(&other).await.unwrap();
        let dispatch = seed_dispatch(&isle, persona).await;
        {
            let dispatch = *dispatch.as_uuid();
            let home_id = *home.id.as_uuid();
            isle.call(move |conn| {
                conn.execute(
                    "UPDATE dispatch_job SET pursuit_id = ?1 WHERE id = ?2",
                    params![home_id, dispatch],
                )
            })
            .await
            .unwrap();
        }

        // Via the dispatch join.
        let joined = seed_traced_asset(
            &isle,
            persona,
            &format!(r#"{{"resolved":true,"dispatch_id":"{dispatch}"}}"#),
            10,
        )
        .await;
        // Via the direct claim (no dispatch hop resolved).
        let claimed = seed_traced_asset(
            &isle,
            persona,
            &format!(
                r#"{{"resolved":false,"pursuit_resolved":true,"pursuit_id":"{}"}}"#,
                home.id
            ),
            30,
        )
        .await;
        // Stale sidecar copy: the resolved dispatch hop names `home`,
        // the claim names `other` — the join wins without adjudication.
        let stale_copy = seed_traced_asset(
            &isle,
            persona,
            &format!(
                r#"{{"resolved":true,"dispatch_id":"{dispatch}","pursuit_resolved":true,"pursuit_id":"{}"}}"#,
                other.id
            ),
            20,
        )
        .await;
        // Unresolved claims never surface.
        seed_traced_asset(
            &isle,
            persona,
            &format!(
                r#"{{"resolved":false,"pursuit_resolved":false,"pursuit_id":"{}"}}"#,
                home.id
            ),
            40,
        )
        .await;
        // A fold headstone drops out of the enumeration.
        let folded = seed_traced_asset(
            &isle,
            persona,
            &format!(r#"{{"resolved":true,"dispatch_id":"{dispatch}"}}"#),
            50,
        )
        .await;
        {
            let keeper = joined;
            isle.call(move |conn| {
                conn.execute(
                    "UPDATE asset SET folded_into = ?1 WHERE id = ?2",
                    params![keeper, folded],
                )
            })
            .await
            .unwrap();
        }

        let returns = repo.returns_of(&home.id).await.unwrap();
        assert_eq!(
            returns,
            vec![
                AssetId::from_uuid(joined),
                AssetId::from_uuid(stale_copy),
                AssetId::from_uuid(claimed),
            ],
            "join first, stale copies resolved by the join, claims second, \
             ingest order — unresolved and folded rows absent"
        );
        assert!(
            repo.returns_of(&other.id).await.unwrap().is_empty(),
            "a stale sidecar copy never files a return under the pursuit it names"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn latest_event_kinds_agree_with_per_pursuit_standing() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let closed = Pursuit::new(persona, None, None, None, t0, &ctx);
        let tied = Pursuit::new(persona, None, None, None, t0, &ctx);
        let untouched = Pursuit::new(persona, None, None, None, t0, &ctx);
        for p in [&closed, &tied, &untouched] {
            repo.create(p).await.unwrap();
        }
        let event = |pursuit: &Pursuit, kind, at| {
            PursuitEvent::new(pursuit.id, persona, kind, None, None, at, &ctx).unwrap()
        };
        repo.append_event(&event(
            &closed,
            PursuitEventKind::ClosedSatisfied,
            t0 + Duration::seconds(1),
        ))
        .await
        .unwrap();
        // Two events sharing one clock reading: the v7 id tie-break
        // must pick the later mint, exactly as `standing` does.
        let first = event(&tied, PursuitEventKind::ClosedAbandoned, t0);
        let second = event(&tied, PursuitEventKind::Reopened, t0);
        repo.append_event(&first).await.unwrap();
        repo.append_event(&second).await.unwrap();

        let mut latest = repo.latest_event_kinds(&persona).await.unwrap();
        latest.sort_by_key(|(id, _)| *id.as_uuid());
        let mut expected = vec![
            (closed.id, PursuitEventKind::ClosedSatisfied),
            (tied.id, PursuitEventKind::Reopened),
        ];
        expected.sort_by_key(|(id, _)| *id.as_uuid());
        assert_eq!(
            latest, expected,
            "one row per evented pursuit, tie broken on id; no-event pursuits absent"
        );
        let _ = untouched;

        driver.shutdown().await.unwrap();
    }
}
