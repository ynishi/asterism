//! SQLite adapter for the `PursuitRepository` port (#29, extended by
//! #22).
//!
//! Six tables, one concern: `pursuit` (thin, immutable, insert-only),
//! `pursuit_event` (append-only lifecycle facts), `pursuit_restamp`
//! (the recorded repair verb), `pursuit_tx` (the append-only
//! membership ledger), and `cull` / `cull_member` (the record of a
//! close's narrowing). The multi-table writes — restamp, and the
//! close-with-cull — each run in a single transaction here, because
//! "the move is recorded" and "the stamp moved" (respectively "the
//! pursuit closed" and "this is what it decided") must not be
//! separable facts.

use asterism_core::domain::attribution::PersistedAttribution;
use asterism_core::domain::forge::cull::{Cull, CullMember, CullVerdict};
use asterism_core::domain::forge::pursuit::{
    Pursuit, PursuitEvent, PursuitEventKind, PursuitRestamp, RestampSubject,
};
use asterism_core::domain::forge::repository::PursuitRepository;
use asterism_core::domain::forge::tx::{PursuitTx, PursuitTxKind};
use asterism_core::domain::forge::value::{
    CullId, LineEntryId, LineEventId, ProjectId, PursuitEventId, PursuitId, PursuitTxId,
};
use asterism_core::domain::repository::CorrelationResolver;
use asterism_core::domain::value::CorrelationId;
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

struct CullRow {
    id: Uuid,
    pursuit_id: Uuid,
    persona_id: Uuid,
    pursuit_event_id: Uuid,
    candidate_snapshot_id: Uuid,
    note: Option<String>,
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
    attributed_via: Option<String>,
    created_at: i64,
}

impl CullRow {
    const COLUMNS: &'static str =
        "id, pursuit_id, persona_id, pursuit_event_id, candidate_snapshot_id, note,
         author_kind, author_subject, operator_ai, attributed_via, created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            pursuit_id: row.get(1)?,
            persona_id: row.get(2)?,
            pursuit_event_id: row.get(3)?,
            candidate_snapshot_id: row.get(4)?,
            note: row.get(5)?,
            author_kind: row.get(6)?,
            author_subject: row.get(7)?,
            operator_ai: row.get(8)?,
            attributed_via: row.get(9)?,
            created_at: row.get(10)?,
        })
    }

    fn into_domain(self) -> Result<Cull, DomainError> {
        let attribution = PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        Ok(Cull::from_persisted(
            CullId::from_uuid(self.id),
            PursuitId::from_uuid(self.pursuit_id),
            PersonaId::from_uuid(self.persona_id),
            PursuitEventId::from_uuid(self.pursuit_event_id),
            SnapshotId::from_uuid(self.candidate_snapshot_id),
            self.note,
            ms_to_datetime(self.created_at)?,
            attribution,
        ))
    }
}

/// Maps a `cull_member` row into the domain shape. Standalone rather
/// than a `Row` struct: the member has no attribution and no clock of
/// its own — it is a line item of its cull.
fn member_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<(Uuid, Uuid, String, Option<String>), rusqlite::Error> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

/// Finishes the member mapping outside the closure, where the verdict
/// parse can be a domain error.
fn member_into_domain(
    (cull_id, asset_id, verdict, note): (Uuid, Uuid, String, Option<String>),
) -> Result<CullMember, DomainError> {
    Ok(CullMember {
        cull_id: CullId::from_uuid(cull_id),
        asset_id: AssetId::from_uuid(asset_id),
        verdict: CullVerdict::parse(&verdict)?,
        note,
    })
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
        // that is only `dispatch_job` (the CHECK's `cull` value is
        // pre-paid vocabulary, not yet a movable subject).
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

    async fn append_close(
        &self,
        event: &PursuitEvent,
        cull: Option<(&Cull, &[CullMember])>,
    ) -> Result<(), DomainError> {
        let event_id = *event.id.as_uuid();
        let event_pursuit = *event.pursuit_id.as_uuid();
        let event_persona = *event.persona_id.as_uuid();
        let event_kind = event.kind.slug().to_string();
        let event_snapshot = event.snapshot_id.map(|s| *s.as_uuid());
        let event_note = event.note.clone();
        let (e_author_kind, e_author_subject, e_operator_ai, e_attributed_via) =
            attribution_columns("pursuit_event", &event.persisted_attribution())?;
        let event_created = datetime_to_ms(&event.created_at);
        let cull_row = cull
            .map(|(cull, members)| -> Result<_, DomainError> {
                let (c_author_kind, c_author_subject, c_operator_ai, c_attributed_via) =
                    attribution_columns("cull", &cull.persisted_attribution())?;
                Ok((
                    *cull.id.as_uuid(),
                    *cull.pursuit_id.as_uuid(),
                    *cull.persona_id.as_uuid(),
                    *cull.pursuit_event_id.as_uuid(),
                    *cull.candidate_snapshot_id.as_uuid(),
                    cull.note.clone(),
                    c_author_kind,
                    c_author_subject,
                    c_operator_ai,
                    c_attributed_via,
                    datetime_to_ms(&cull.created_at),
                    members
                        .iter()
                        .map(|m| {
                            (
                                *m.asset_id.as_uuid(),
                                m.verdict.slug().to_string(),
                                m.note.clone(),
                            )
                        })
                        .collect::<Vec<_>>(),
                ))
            })
            .transpose()?;
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
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
                if let Some((
                    cull_id,
                    cull_pursuit,
                    cull_persona,
                    cull_event,
                    candidate_snapshot,
                    cull_note,
                    c_author_kind,
                    c_author_subject,
                    c_operator_ai,
                    c_attributed_via,
                    cull_created,
                    members,
                )) = cull_row
                {
                    tx.execute(
                        "INSERT INTO cull
                             (id, pursuit_id, persona_id, pursuit_event_id,
                              candidate_snapshot_id, note,
                              author_kind, author_subject, operator_ai, attributed_via,
                              created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            cull_id,
                            cull_pursuit,
                            cull_persona,
                            cull_event,
                            candidate_snapshot,
                            cull_note,
                            c_author_kind,
                            c_author_subject,
                            c_operator_ai,
                            c_attributed_via,
                            cull_created,
                        ],
                    )?;
                    let mut insert = tx.prepare(
                        "INSERT INTO cull_member (cull_id, asset_id, verdict, note)
                         VALUES (?1, ?2, ?3, ?4)",
                    )?;
                    for (asset_id, verdict, note) in members {
                        insert.execute(params![cull_id, asset_id, verdict, note])?;
                    }
                    drop(insert);
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn culls_of(
        &self,
        pursuit_id: &PursuitId,
    ) -> Result<Vec<(Cull, Vec<CullMember>)>, DomainError> {
        let uuid = *pursuit_id.as_uuid();
        let (cull_rows, member_rows) = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM cull WHERE pursuit_id = ?1 \
                     ORDER BY created_at ASC, id ASC",
                    CullRow::COLUMNS
                ))?;
                let culls = stmt
                    .query_map(params![uuid], CullRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut stmt = conn.prepare(
                    "SELECT m.cull_id, m.asset_id, m.verdict, m.note \
                       FROM cull_member m JOIN cull c ON c.id = m.cull_id \
                      WHERE c.pursuit_id = ?1 \
                      ORDER BY m.cull_id, m.asset_id",
                )?;
                let members = stmt
                    .query_map(params![uuid], member_from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((culls, members))
            })
            .await
            .map_err(infra_err)?;
        let members = member_rows
            .into_iter()
            .map(member_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        cull_rows
            .into_iter()
            .map(|row| {
                let cull = row.into_domain()?;
                let own = members
                    .iter()
                    .filter(|m| m.cull_id == cull.id)
                    .cloned()
                    .collect();
                Ok((cull, own))
            })
            .collect()
    }

    async fn culls_for_asset(
        &self,
        asset_id: &AssetId,
        limit: u32,
    ) -> Result<Vec<(Cull, CullMember)>, DomainError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let uuid = *asset_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {}, m.cull_id, m.asset_id, m.verdict, m.note \
                       FROM cull_member m JOIN cull c ON c.id = m.cull_id \
                      WHERE m.asset_id = ?1 \
                      ORDER BY c.created_at DESC, c.id DESC LIMIT ?2",
                    CullRow::COLUMNS
                        .split(',')
                        .map(|c| format!("c.{}", c.trim()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?;
                let rows = stmt
                    .query_map(params![uuid, limit], |row| {
                        let cull = CullRow::from_row(row)?;
                        let member = (
                            row.get::<_, Uuid>(11)?,
                            row.get::<_, Uuid>(12)?,
                            row.get::<_, String>(13)?,
                            row.get::<_, Option<String>>(14)?,
                        );
                        Ok((cull, member))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(cull, member)| Ok((cull.into_domain()?, member_into_domain(member)?)))
            .collect()
    }
}

/// The catalogue's narrow view of the same table.
///
/// Ingest asks whether a returning artefact's stamp names anything live
/// in its persona, and that is all it may ask — the port is a `bool`
/// and lives in `domain::repository` so the catalogue never holds
/// [`PursuitRepository`]. This adapter answers it because the answer is
/// a row lookup, and putting the implementation in the forge would hand
/// the composition root a forge type to wire into `AssetService` — the
/// same dependency, arriving by a longer route.
///
/// One `EXISTS` rather than `find` plus a persona comparison: the
/// caller discards the row either way, and the pair `(id, persona_id)`
/// is what the question is about.
///
/// That changes one answer, deliberately. A row that is there but
/// cannot hydrate — bad attribution columns, an out-of-range
/// `created_at` — used to reach the caller as an error and be recorded
/// unresolved. It now answers `true`, because the claim does name a
/// pursuit of this persona and that is the whole question; whether the
/// row reads back cleanly is a different one, and the reads that ask it
/// still fail as before.
#[async_trait]
impl CorrelationResolver for SqlitePursuitRepository {
    async fn resolves(
        &self,
        stamp: &CorrelationId,
        persona_id: &PersonaId,
    ) -> Result<bool, DomainError> {
        let uuid = *stamp.as_uuid();
        let persona = *persona_id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pursuit WHERE id = ?1 AND persona_id = ?2)",
                    params![uuid, persona],
                    |row| row.get::<_, i64>(0),
                )
                .map(|found| found != 0)
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
    async fn restamp_moves_the_stamp_and_refuses_stale_or_missing() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();

        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();
        let target = Pursuit::new(persona, None, None, None, None, t0, &ctx);
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
            Pursuit::new(persona, None, None, None, None, t0, &ctx).id,
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
            Pursuit::new(persona, None, None, None, None, t0, &ctx).id,
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
        let home = Pursuit::new(persona_a, None, None, None, None, t0, &ctx);
        let foreign = Pursuit::new(persona_b, None, None, None, None, t0, &ctx);
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

        let home = Pursuit::new(persona, None, None, None, None, t0, &ctx);
        let other = Pursuit::new(persona, None, None, None, None, t0, &ctx);
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

    /// What the port answers, for the four cases ingest turns into
    /// `pursuit_resolved` (#81). It exercises the port and stops there
    /// — nothing between here and ingest is covered, so this is the
    /// whole test of the swap that put a `bool` port under
    /// `AssetService::resolve_pursuit_claim`.
    ///
    /// The persona half is the one worth pinning. A claim naming
    /// another persona's pursuit is *unresolved*, not
    /// resolved-elsewhere: the stamp is a real pursuit id, so a query
    /// keyed on the id alone would answer `true` and file a return
    /// across a boundary nothing else in the tree crosses. The second
    /// assertion is what fails against such an implementation.
    #[tokio::test]
    async fn a_claim_resolves_only_against_its_own_persona() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona_a = seed_persona(&isle).await;
        let persona_b = seed_persona(&isle).await;
        let repo = SqlitePursuitRepository::new(isle.clone());
        let ctx = AttributionContext::owner_surface();
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let mine = Pursuit::new(persona_a, None, None, None, None, t0, &ctx);
        repo.create(&mine).await.unwrap();
        let stamp = mine.id.as_correlation();

        assert!(
            repo.resolves(&stamp, &persona_a).await.unwrap(),
            "a claim naming a pursuit of the ingesting persona resolves"
        );
        assert!(
            !repo.resolves(&stamp, &persona_b).await.unwrap(),
            "the same stamp read from another persona is unresolved, not resolved elsewhere"
        );
        assert!(
            !repo
                .resolves(&PursuitId::new().as_correlation(), &persona_a)
                .await
                .unwrap(),
            "a stamp naming nothing is false rather than an error — an artefact may \
             carry a claim to a purged pursuit, and that is a fact to record"
        );

        // Standing is deliberately not consulted: filing a return under
        // a closed pursuit is a legal act, so a close must not change
        // any of the answers above.
        let close = PursuitEvent::new(
            mine.id,
            persona_a,
            PursuitEventKind::ClosedSatisfied,
            None,
            None,
            t0,
            &ctx,
        )
        .unwrap();
        repo.append_event(&close).await.unwrap();
        assert!(
            repo.resolves(&stamp, &persona_a).await.unwrap(),
            "a closed pursuit still resolves — the claim lane asks existence, not standing"
        );

        driver.shutdown().await.unwrap();
    }
}
