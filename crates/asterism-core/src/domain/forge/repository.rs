//! The forge's persistence ports.
//!
//! Split from [`domain::repository`](crate::domain::repository), which
//! held these two beside the catalogue's twenty-eight and so made the
//! forge's storage contract part of the file a new catalogue port is
//! added to. Nothing about the traits changed in the move.
//!
//! The catalogue does not name these. What it needs of a pursuit — has
//! this stamp got something live behind it — is
//! [`CorrelationResolver`](crate::domain::repository::CorrelationResolver),
//! which the catalogue owns and answers with a `bool`.

use async_trait::async_trait;

use crate::domain::forge::cull::{Cull, CullMember};
use crate::domain::forge::line::Line;
use crate::domain::forge::project::Project;
use crate::domain::forge::pursuit::{Pursuit, PursuitEvent, PursuitEventKind, PursuitRestamp};
use crate::domain::forge::tx::PursuitTx;
use crate::domain::forge::value::{ProjectId, PursuitId};
use crate::domain::value::{AssetId, PersonaId};
use crate::error::DomainError;

/// Persistence port for the pursuit family (#29): the minted unit of
/// work, its lifecycle facts, and the restamp record.
///
/// One port for the three tables rather than three: they are one
/// cohesive concern (the correlation layer over the record), share
/// every caller, and the two write verbs that must be atomic across
/// tables (`restamp`) could not live on a single-table port. The
/// pursuit row itself has no update and no delete — it is immutable,
/// standing is derived from the events, and the only deletion path is
/// the persona purge, which is hand-rolled in the adapter.
#[async_trait]
pub trait PursuitRepository: Send + Sync {
    /// Persists a fresh pursuit — the explicit pre-create, and the
    /// mint half of always-mint. Insert-only: a pursuit is never
    /// re-saved.
    async fn create(&self, pursuit: &Pursuit) -> Result<(), DomainError>;

    /// Fetches one pursuit by id.
    async fn find(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError>;

    /// Lists a persona's pursuits, most-recent first, capped at
    /// `limit`. Standing is not part of this read — a caller that
    /// needs it derives it from [`events_of`](Self::events_of) (or a
    /// batched projection later; the row stores no status by design).
    async fn list(&self, persona_id: &PersonaId, limit: u32) -> Result<Vec<Pursuit>, DomainError>;

    /// Appends one lifecycle fact. Append-only: facts are never
    /// edited, a repeat close is a new fact, standing re-derives.
    async fn append_event(&self, event: &PursuitEvent) -> Result<(), DomainError>;

    /// A pursuit's lifecycle facts in standing order —
    /// `(created_at, id)` ascending, so the last element is the one
    /// [`standing`](crate::domain::forge::pursuit::standing) lets win.
    async fn events_of(&self, pursuit_id: &PursuitId) -> Result<Vec<PursuitEvent>, DomainError>;

    /// Records a restamp and moves the stamp, atomically: the
    /// `pursuit_restamp` row and the `UPDATE` of the subject's
    /// `pursuit_id` column land in one transaction, and the write is
    /// refused with a `Conflict` when the subject's current stamp does
    /// not equal the restamp's recorded `from` — a stale `from` means
    /// the caller is moving a filing it has not looked at.
    async fn restamp(&self, restamp: &PursuitRestamp) -> Result<(), DomainError>;

    /// A pursuit's **returns**: assets whose resolved `_trace` names
    /// one of its rounds (the dispatch join, which is why a restamped
    /// round's returns follow it automatically), plus assets whose
    /// resolved direct pursuit claim names it while no dispatch hop
    /// resolved — the claim-lane authority order, evaluated over the
    /// V80 lookup columns so each probe is an index seek, never a
    /// scan (the documented scale is 100k+ assets). Fold headstones
    /// are dropped (this is an enumeration path); trashed rows stay
    /// (a return in the trash is still a return, and restorable).
    /// Ordered by ingest time, then id.
    ///
    /// **A round's own outputs are not returns.** What `reify` mints
    /// in-library rides on the round itself
    /// (`DispatchJob::output_asset_ids`, stamped `_dispatch`, not
    /// `_trace`) and reaches a view through its rounds; *returns* are
    /// what came back from outside — files an external tool produced,
    /// re-ingested with a claim. The two populations answer different
    /// questions and deliberately do not mix here.
    async fn returns_of(&self, pursuit_id: &PursuitId) -> Result<Vec<AssetId>, DomainError>;

    /// The latest lifecycle event kind per pursuit of a persona — the
    /// standing read for listings, one window query instead of one
    /// `events_of` per row. A pursuit with no events is absent from
    /// the result (= open).
    async fn latest_event_kinds(
        &self,
        persona_id: &PersonaId,
    ) -> Result<Vec<(PursuitId, PursuitEventKind)>, DomainError>;

    /// Appends one membership gesture to the ledger (#22). Append-only:
    /// gestures are never edited, membership re-derives.
    async fn append_tx(&self, tx: &PursuitTx) -> Result<(), DomainError>;

    /// A pursuit's ledger, `(created_at, id)` ascending — the order
    /// [`ledger`](crate::domain::forge::tx::ledger) derives over.
    async fn txs_of(&self, pursuit_id: &PursuitId) -> Result<Vec<PursuitTx>, DomainError>;

    /// Appends a close event together with its cull, atomically: "the
    /// pursuit closed" and "this is what that close decided, out of
    /// what" must not be separable facts. `None` is a close with
    /// nothing to record — an abandoned close, or a satisfied close of
    /// a pursuit whose ledger is empty.
    async fn append_close(
        &self,
        event: &PursuitEvent,
        cull: Option<(&Cull, &[CullMember])>,
    ) -> Result<(), DomainError>;

    /// A pursuit's culls with their member verdicts, oldest first —
    /// one cull per close event, so a repeat close reads as a second
    /// record, not an overwrite.
    async fn culls_of(
        &self,
        pursuit_id: &PursuitId,
    ) -> Result<Vec<(Cull, Vec<CullMember>)>, DomainError>;

    /// Every verdict ever recorded about one asset, most-recent first,
    /// capped at `limit` — the acceptance read of #22: who decided to
    /// keep or drop this, out of which set, in which line of work.
    async fn culls_for_asset(
        &self,
        asset_id: &AssetId,
        limit: u32,
    ) -> Result<Vec<(Cull, CullMember)>, DomainError>;
}

/// Persistence port for the forge's project and its lines (#63
/// decisions 1–2).
///
/// Separate from [`PursuitRepository`] rather than folded into it,
/// because the two answer different questions: a pursuit is one
/// attempt and its record, a project is the shared context attempts
/// file under and the canonical set they land on. They meet at exactly
/// one column (`pursuit.project_id`), which is a filing rather than an
/// aggregate boundary being crossed.
#[async_trait]
pub trait ProjectRepository: Send + Sync {
    /// Persists a project together with the line it opens with, in one
    /// transaction. Two rows rather than one call each because "the
    /// project exists" and "it has a line to land on" must not be
    /// separable facts — a project whose line is missing has nothing a
    /// merge could target, and nothing would ever notice.
    ///
    /// v1 passes exactly one line, named
    /// [`MAIN`](crate::domain::forge::line::Line::MAIN); the signature
    /// takes a slice so a later multi-line model is a caller change
    /// rather than a port change. An empty slice is refused — it would
    /// commit the very state the transaction exists to prevent.
    async fn create(&self, project: &Project, lines: &[Line]) -> Result<(), DomainError>;

    /// Fetches one project by id.
    async fn find(&self, id: &ProjectId) -> Result<Option<Project>, DomainError>;

    /// Fetches a persona's project by the exact name it was stored
    /// under. Comparison is the column's, which is byte-exact: case,
    /// internal spacing and Unicode normal form all distinguish two
    /// names.
    ///
    /// This exists because project-name uniqueness is an application
    /// rule rather than a schema one, so nothing raises a constraint
    /// violation to catch — a caller enforcing it reads first. Two of
    /// those reads can both come back empty and both go on to write,
    /// which this port does not close: `create` is a separate call, so
    /// the rule is advisory under concurrency. Whether it stays that
    /// way is open, and the alternative is a partial UNIQUE (the
    /// `dir(persona_id, name)` precedent, which survives archival by
    /// excluding archived rows) rather than a check moved elsewhere.
    async fn find_named(
        &self,
        persona_id: &PersonaId,
        name: &str,
    ) -> Result<Option<Project>, DomainError>;

    /// Lists a persona's projects, most-recent first, capped at
    /// `limit`.
    async fn list(&self, persona_id: &PersonaId, limit: u32) -> Result<Vec<Project>, DomainError>;

    /// A project's lines, oldest first. v1 returns exactly one; the
    /// merge target derives pursuit → project → this.
    async fn lines_of(&self, project_id: &ProjectId) -> Result<Vec<Line>, DomainError>;
}
