//! The forge's persistence ports.
//!
//! Split from [`domain::repository`](crate::domain::repository), which
//! held these two beside the raw layer's twenty-eight and so made the
//! forge's storage contract part of the file a new raw-layer port is
//! added to. Nothing about the traits changed in the move.
//!
//! The raw layer does not name these, and needs nothing of a pursuit.

use async_trait::async_trait;

use crate::domain::forge::line::Line;
use crate::domain::forge::project::Project;
use crate::domain::forge::pursuit::{Pursuit, PursuitEvent, PursuitEventKind};
use crate::domain::forge::tx::PursuitTx;
use crate::domain::forge::value::{ProjectId, PursuitId};
use crate::domain::value::PersonaId;
use crate::error::DomainError;

/// Persistence port for the pursuit family (#29): the minted unit of
/// work, its lifecycle facts, and its ledger.
///
/// One port for the family rather than one per table: they are one
/// cohesive concern (the correlation layer over the record) and share
/// every caller. The pursuit row itself has no update and no delete —
/// it is immutable, standing is derived from the events, and the only
/// deletion path is the persona purge, which is hand-rolled in the
/// adapter.
#[async_trait]
pub trait PursuitRepository: Send + Sync {
    /// Persists a fresh pursuit — opening one is the only way a row
    /// gets here, and it is always somebody's explicit act.
    /// Insert-only: a pursuit is never re-saved.
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

    /// Appends a close event. Separate from [`append_event`] only in
    /// name — a close is one row, and there is nothing it has to land
    /// beside.
    ///
    /// [`append_event`]: Self::append_event
    async fn append_close(&self, event: &PursuitEvent) -> Result<(), DomainError>;
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
