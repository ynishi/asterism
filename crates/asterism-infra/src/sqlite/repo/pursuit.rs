//! SQLite adapter for the `PursuitRepository` port (#29, extended by
//! #22).
//!
//! Three tables, one concern: `pursuit` (thin, immutable,
//! insert-only), `pursuit_event` (append-only lifecycle facts), and
//! `pursuit_tx` (the append-only membership ledger). Every write here
//! lands on one table; nothing in this adapter spans two.

use asterism_core::domain::attribution::PersistedAttribution;
use asterism_core::domain::forge::pursuit::{Pursuit, PursuitEvent, PursuitEventKind};
use asterism_core::domain::forge::repository::PursuitRepository;
use asterism_core::domain::forge::tx::{PursuitTx, PursuitTxKind};
use asterism_core::domain::forge::value::{
    LineEntryId, LineEventId, ProjectId, PursuitEventId, PursuitId, PursuitTxId,
};
use asterism_core::domain::value::{AssetId, PersonaId, SnapshotId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};
use crate::sqlite::repo::attribution_guard::attribution_columns;

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

struct PursuitRow {
    id: Uuid,
    persona_id: Uuid,
    project_id: Option<Uuid>,
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
    const COLUMNS: &'static str = "id, persona_id, project_id, parent_id, title, note,
                                   author_kind, author_subject, operator_ai, attributed_via,
                                   created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            project_id: row.get(2)?,
            parent_id: row.get(3)?,
            title: row.get(4)?,
            note: row.get(5)?,
            author_kind: row.get(6)?,
            author_subject: row.get(7)?,
            operator_ai: row.get(8)?,
            attributed_via: row.get(9)?,
            created_at: row.get(10)?,
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
            self.project_id.map(ProjectId::from_uuid),
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

struct TxRow {
    id: Uuid,
    pursuit_id: Uuid,
    persona_id: Uuid,
    kind: String,
    asset_id: Uuid,
    origin: Option<String>,
    target_entry_id: Option<Uuid>,
    base_event_id: Option<Uuid>,
    out_of_scope: bool,
    supersedes_asset_id: Option<Uuid>,
    note: Option<String>,
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
    attributed_via: Option<String>,
    created_at: i64,
}

impl TxRow {
    const COLUMNS: &'static str = "id, pursuit_id, persona_id, kind, asset_id, origin,
                                   target_entry_id, base_event_id, out_of_scope,
                                   supersedes_asset_id, note,
                                   author_kind, author_subject, operator_ai, attributed_via,
                                   created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            pursuit_id: row.get(1)?,
            persona_id: row.get(2)?,
            kind: row.get(3)?,
            asset_id: row.get(4)?,
            origin: row.get(5)?,
            target_entry_id: row.get(6)?,
            base_event_id: row.get(7)?,
            out_of_scope: row.get(8)?,
            supersedes_asset_id: row.get(9)?,
            note: row.get(10)?,
            author_kind: row.get(11)?,
            author_subject: row.get(12)?,
            operator_ai: row.get(13)?,
            attributed_via: row.get(14)?,
            created_at: row.get(15)?,
        })
    }

    fn into_domain(self) -> Result<PursuitTx, DomainError> {
        let attribution = PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        Ok(PursuitTx::from_persisted(
            PursuitTxId::from_uuid(self.id),
            PursuitId::from_uuid(self.pursuit_id),
            PersonaId::from_uuid(self.persona_id),
            PursuitTxKind::from_columns(
                &self.kind,
                self.origin.as_deref(),
                self.target_entry_id.map(LineEntryId::from_uuid),
                self.base_event_id.map(LineEventId::from_uuid),
                self.out_of_scope,
                self.supersedes_asset_id.map(AssetId::from_uuid),
            )?,
            AssetId::from_uuid(self.asset_id),
            self.note,
            ms_to_datetime(self.created_at)?,
            attribution,
        ))
    }
}

#[async_trait]
impl PursuitRepository for SqlitePursuitRepository {
    async fn create(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
        let id = *pursuit.id.as_uuid();
        let persona_id = *pursuit.persona_id.as_uuid();
        let project_id = pursuit.project_id.map(|p| *p.as_uuid());
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
                         (id, persona_id, project_id, parent_id, title, note,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        id,
                        persona_id,
                        project_id,
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
            .map_err(|err| {
                // Primary-key violation → `Conflict` (409 at the HTTP
                // boundary), the modality adapter's mapping. Unreachable
                // while every id is minted; it stops being unreachable
                // with the caller-chosen id the repair path opens, and a
                // taken id is a real answer to give back rather than a
                // 500.
                let msg = err.to_string();
                if msg.contains("UNIQUE") || msg.contains("unique") {
                    DomainError::Conflict(format!("pursuit {id} already exists"))
                } else {
                    infra_err(err)
                }
            })
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

    async fn append_tx(&self, tx: &PursuitTx) -> Result<(), DomainError> {
        let id = *tx.id.as_uuid();
        let pursuit_id = *tx.pursuit_id.as_uuid();
        let persona_id = *tx.persona_id.as_uuid();
        let kind = tx.kind.kind_slug().to_string();
        let origin = tx.kind.origin_slug().map(str::to_string);
        let target = tx.kind.target();
        let target_entry_id = target.map(|t| *t.entry_id.as_uuid());
        let base_event_id = target.and_then(|t| t.base_event_id).map(|e| *e.as_uuid());
        let out_of_scope = tx.kind.out_of_scope();
        let supersedes_asset_id = tx.kind.supersedes_asset_id().map(|a| *a.as_uuid());
        let asset_id = *tx.asset_id.as_uuid();
        let note = tx.note.clone();
        let (author_kind, author_subject, operator_ai, attributed_via) =
            attribution_columns("pursuit_tx", &tx.persisted_attribution())?;
        let created = datetime_to_ms(&tx.created_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO pursuit_tx
                         (id, pursuit_id, persona_id, kind, asset_id, origin,
                          target_entry_id, base_event_id, out_of_scope,
                          supersedes_asset_id, note,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                             ?15, ?16)",
                    params![
                        id,
                        pursuit_id,
                        persona_id,
                        kind,
                        asset_id,
                        origin,
                        target_entry_id,
                        base_event_id,
                        out_of_scope,
                        supersedes_asset_id,
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

    async fn txs_of(&self, pursuit_id: &PursuitId) -> Result<Vec<PursuitTx>, DomainError> {
        let uuid = *pursuit_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM pursuit_tx WHERE pursuit_id = ?1 \
                     ORDER BY created_at ASC, id ASC",
                    TxRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![uuid], TxRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(TxRow::into_domain).collect()
    }

    async fn append_close(&self, event: &PursuitEvent) -> Result<(), DomainError> {
        let event_id = *event.id.as_uuid();
        let event_pursuit = *event.pursuit_id.as_uuid();
        let event_persona = *event.persona_id.as_uuid();
        let event_kind = event.kind.slug().to_string();
        let event_snapshot = event.snapshot_id.map(|s| *s.as_uuid());
        let event_note = event.note.clone();
        let (e_author_kind, e_author_subject, e_operator_ai, e_attributed_via) =
            attribution_columns("pursuit_event", &event.persisted_attribution())?;
        let event_created = datetime_to_ms(&event.created_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO pursuit_event
                         (id, pursuit_id, persona_id, kind, snapshot_id, note,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        event_id,
                        event_pursuit,
                        event_persona,
                        event_kind,
                        event_snapshot,
                        event_note,
                        e_author_kind,
                        e_author_subject,
                        e_operator_ai,
                        e_attributed_via,
                        event_created,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::attribution::AttributionContext;
    use asterism_core::domain::forge::pursuit::{PursuitStanding, standing};
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

    /// One project row for the persona, seeded directly. `project_id`
    /// carries a foreign key and the pool runs with `foreign_keys` on,
    /// so a filing test cannot invent an id — it needs a project that
    /// is really there.
    async fn seed_project(isle: &AsyncIsle, persona: PersonaId) -> ProjectId {
        let project = Uuid::now_v7();
        let persona = *persona.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO project (id, persona_id, name, created_at) \
                 VALUES (?1, ?2, 'album', 0)",
                params![project, persona],
            )
        })
        .await
        .unwrap();
        ProjectId::from_uuid(project)
    }

    /// One line entry with one `add` behind it, so a targeted IN has
    /// something real to aim at and to pin. Both columns carry foreign
    /// keys, so the whole chain has to exist: a line under the project,
    /// an entry on the line, and a merge for the event to land under.
    ///
    /// **Closes the pursuit as a side effect.** A merge hangs on a
    /// satisfied close, so one is written for `pursuit` — harmless for
    /// a caller reading the ledger, wrong for one reading standing.
    async fn seed_entry_and_event(
        isle: &AsyncIsle,
        persona: PersonaId,
        project: ProjectId,
        pursuit: PursuitId,
    ) -> (LineEntryId, LineEventId) {
        let line = Uuid::now_v7();
        let entry = Uuid::now_v7();
        let close = Uuid::now_v7();
        let merge = Uuid::now_v7();
        let event = Uuid::now_v7();
        let persona_uuid = *persona.as_uuid();
        let project_uuid = *project.as_uuid();
        let pursuit_uuid = *pursuit.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO line (id, project_id, name, created_at) VALUES (?1, ?2, 'main', 0)",
                params![line, project_uuid],
            )?;
            conn.execute(
                "INSERT INTO line_entry (id, line_id, persona_id, created_at) \
                 VALUES (?1, ?2, ?3, 0)",
                params![entry, line, persona_uuid],
            )?;
            conn.execute(
                "INSERT INTO pursuit_event (id, pursuit_id, persona_id, kind, created_at) \
                 VALUES (?1, ?2, ?3, 'closed_satisfied', 0)",
                params![close, pursuit_uuid, persona_uuid],
            )?;
            conn.execute(
                "INSERT INTO line_merge (id, pursuit_event_id, persona_id, created_at) \
                 VALUES (?1, ?2, ?3, 0)",
                params![merge, close, persona_uuid],
            )?;
            conn.execute(
                "INSERT INTO line_event \
                     (id, entry_id, persona_id, verb, asset_id, name, merge_id, created_at) \
                 VALUES (?1, ?2, ?3, 'add', ?4, 'key visual', ?5, 0)",
                params![event, entry, persona_uuid, Uuid::now_v7(), merge],
            )
        })
        .await
        .unwrap();
        (LineEntryId::from_uuid(entry), LineEventId::from_uuid(event))
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
            None,
            Some("line".into()),
            None,
            t0,
            &AttributionContext::owner_surface(),
        );
        repo.create(&parent).await.unwrap();
        let child = Pursuit::new(
            persona,
            None,
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

    /// A targeted IN survives the round trip with its aim intact.
    /// `target_entry_id`, `base_event_id` and `supersedes_asset_id` are
    /// three nullable uuid columns sitting together in the SELECT list,
    /// the positional reads and the INSERT, and every other ledger test
    /// leaves all three `None` — so a slip among them is invisible
    /// everywhere but here. Each is given a distinct value, and the
    /// unaimed gesture beside it is asserted to stay unaimed rather
    /// than picking anything up.
    #[tokio::test]
    async fn a_targeted_in_round_trips_its_aim_and_leaves_a_plain_one_unaimed() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let project = seed_project(&isle, persona).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let pursuit = Pursuit::new(persona, Some(project), None, None, None, t0, &ctx);
        repo.create(&pursuit).await.unwrap();
        let (entry, base_event) = seed_entry_and_event(&isle, persona, project, pursuit.id).await;

        let aimed = PursuitTx::new(
            pursuit.id,
            persona,
            PursuitTxKind::In {
                origin: asterism_core::domain::forge::tx::TxOrigin::Existing,
                target: Some(asterism_core::domain::forge::tx::TxTarget {
                    entry_id: entry,
                    base_event_id: Some(base_event),
                }),
                out_of_scope: true,
            },
            AssetId::new(),
            None,
            t0,
            &ctx,
        )
        .unwrap();
        repo.append_tx(&aimed).await.unwrap();

        let plain = PursuitTx::new(
            pursuit.id,
            persona,
            PursuitTxKind::In {
                origin: asterism_core::domain::forge::tx::TxOrigin::Generated,
                target: None,
                out_of_scope: false,
            },
            AssetId::new(),
            None,
            t0 + Duration::seconds(1),
            &ctx,
        )
        .unwrap();
        repo.append_tx(&plain).await.unwrap();

        let read = repo.txs_of(&pursuit.id).await.unwrap();
        assert_eq!(read, vec![aimed.clone(), plain.clone()]);

        let back = read[0].kind.target().expect("the aim came back");
        assert_eq!(back.entry_id, entry);
        assert_eq!(back.base_event_id, Some(base_event));
        assert!(read[0].kind.out_of_scope());
        assert_eq!(read[0].kind.supersedes_asset_id(), None);

        assert_eq!(read[1].kind.target(), None);
        assert!(!read[1].kind.out_of_scope());

        driver.shutdown().await.unwrap();
    }

    /// The filing survives the round trip, and it survives it *as the
    /// filing*. `project_id` and `parent_id` are the row's only two
    /// nullable uuid columns and they sit next to each other in the
    /// SELECT list, the positional reads, and the INSERT — so a
    /// one-place slip swaps them and every other test still passes,
    /// because every other test leaves both `None`. This is the one
    /// that sets them to different values and looks.
    #[tokio::test]
    async fn a_filed_pursuit_round_trips_its_project_without_taking_the_parent() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let project = seed_project(&isle, persona).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let parent = Pursuit::new(persona, Some(project), None, None, None, t0, &ctx);
        repo.create(&parent).await.unwrap();
        let child = Pursuit::new(
            persona,
            Some(project),
            Some(parent.id),
            None,
            None,
            t0 + Duration::seconds(1),
            &ctx,
        );
        repo.create(&child).await.unwrap();

        let found = repo.find(&child.id).await.unwrap().unwrap();
        assert_eq!(found, child);
        assert_eq!(found.project_id, Some(project));
        assert_eq!(found.parent_id, Some(parent.id));

        // The unfiled case reads back unfiled rather than inheriting
        // anything from the rows beside it.
        let loose = Pursuit::new(persona, None, None, None, None, t0, &ctx);
        repo.create(&loose).await.unwrap();
        assert_eq!(
            repo.find(&loose.id).await.unwrap().unwrap().project_id,
            None
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn events_append_in_standing_order_and_derive() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();

        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();
        let pursuit = Pursuit::new(persona, None, None, None, None, t0, &ctx);
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
    async fn latest_event_kinds_agree_with_per_pursuit_standing() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let closed = Pursuit::new(persona, None, None, None, None, t0, &ctx);
        let tied = Pursuit::new(persona, None, None, None, None, t0, &ctx);
        let untouched = Pursuit::new(persona, None, None, None, None, t0, &ctx);
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
