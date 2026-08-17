//! `Pursuit` — the minted unit of work: one line of generation and
//! curation toward an intent, identified by a minted id that its events
//! are stamped with (#29, design on #21).
//!
//! Content ancestry cannot express this unit: round N+1 built from the
//! same sources with a new prompt shares no derivation edge with round
//! N's outputs, rejected rounds have no descendants, and abandonment has
//! no ancestry expression at all. So the identity is minted up front —
//! the convergent form of every surviving forge (Gerrit's Change-Id,
//! Jujutsu's change id, Radicle's COB ids) — and everything else about
//! the pursuit is a projection over the stamped events.
//!
//! # Shape
//!
//! - [`Pursuit`] is a thin, immutable row: identity, persona, optional
//!   filing, optional parent, optional human label. No status column,
//!   no members.
//! - [`PursuitEvent`] is a one-way lifecycle fact (close / reopen);
//!   **standing is derived on read** by [`standing`] — latest event by
//!   `(created_at, id)` wins, no row means open. A repeat close is a new
//!   fact, not an error.
//! - [`PursuitRestamp`] records a move of a stamped event between
//!   pursuits — the repair verb for mis-filed correlation. The stamped
//!   column holds the current filing; the restamp row holds the move.
//!
//! # Invariants (service-enforced, entity-checked where local)
//!
//! - A stamped event's persona equals its pursuit's persona; `parent_id`
//!   never crosses personas; a parent exists before its child. These are
//!   cross-aggregate and live in the application service, like the
//!   persona cascade.
//! - `project_id` never crosses personas either, and the foreign key
//!   cannot say so: `project` carries its own `persona_id`, and
//!   `pursuit.project_id` references only `project(id)`. So filing
//!   under someone else's project is refused where both rows are
//!   visible — the same place, and for the same reason, as `parent_id`.
//! - `snapshot_id` may only accompany `closed_satisfied` (checked here):
//!   it is the kept set frozen at close. `None` on a `closed_satisfied`
//!   is a defined state — "concluded with nothing kept" — because an
//!   empty snapshot is domain-rejected.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::attribution::{AttributionContext, PersistedAttribution};
use crate::domain::value::{
    DispatchId, PersonaId, ProjectId, PursuitEventId, PursuitId, PursuitRestampId, SnapshotId,
};
use crate::error::DomainError;

/// Trims an optional human label; whitespace-only collapses to `None`
/// so "no title" has one representation in storage.
fn normalized(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The minted unit of work. Thin and immutable: identity plus intent
/// (`title` / `note`) plus lineage of work (`parent_id`) plus filing
/// (`project_id`) — never content, never status.
#[derive(Debug, Clone, PartialEq)]
pub struct Pursuit {
    /// Surrogate id (UUID v7) — minted, never derived from content.
    pub id: PursuitId,
    /// Persona bucket; every stamped event shares it (service-enforced,
    /// the same rule `dispatch_job` states for its snapshot).
    pub persona_id: PersonaId,
    /// Pursuit this one was spawned from, set at creation and never
    /// rewritten. A closed parent with open children is legal; rollups
    /// are projections.
    pub parent_id: Option<PursuitId>,
    /// The project this work files under — what makes it forge work,
    /// and what a merge derives its target line from.
    ///
    /// `None` on every row today: nothing sets this yet, and
    /// always-mint still opens a pursuit for each dispatch that
    /// arrives without one. The column is nullable for what comes
    /// after that rule is retired (#63) rather than for today — filing
    /// becomes what mints a pursuit, so an unfiled row is residue
    /// rather than a mode, and residue is left as it is instead of
    /// being given a project it never had.
    ///
    /// Set at creation and never rewritten, like `parent_id`. Restamp
    /// does not reach it — that verb moves a *dispatch* between
    /// pursuits — so work filed under the wrong project is re-opened
    /// under the right one with its rounds restamped across.
    pub project_id: Option<ProjectId>,
    /// Short human label — provenance of intent, not state. `None` for
    /// an anonymous (implicitly minted) pursuit; display names for
    /// those are synthesized by the read side, not stored.
    pub title: Option<String>,
    /// One short free-text slot.
    pub note: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Who opened the pursuit. Private as a triple, like
    /// [`DispatchJob`](crate::domain::forge::dispatch::DispatchJob)'s: set
    /// whole from the context at construction, restored whole by
    /// [`from_persisted`](Self::from_persisted). All `None` on
    /// migration-backfilled rows — nobody opened those, the migration
    /// did, and absent bookkeeping stays absent.
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    author: Option<crate::domain::attribution::Author>,
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl Pursuit {
    /// Builds a fresh pursuit. `parent_id` must name an existing
    /// pursuit of the same persona — checked by the application
    /// service, which is the only caller that can see both rows.
    pub fn new(
        persona_id: PersonaId,
        project_id: Option<ProjectId>,
        parent_id: Option<PursuitId>,
        title: Option<String>,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Self {
        Self::new_at(
            PursuitId::new(),
            persona_id,
            project_id,
            parent_id,
            title,
            note,
            now,
            attribution,
        )
    }

    /// Builds a pursuit at an id the caller names, rather than a
    /// minted one — [`new`](Self::new) with the mint taken out.
    ///
    /// This exists for one shape: an id that is already recorded
    /// somewhere else and has no row here. A returning artefact's
    /// sidecar claims a `pursuit_id` written on another machine, the
    /// ingest records the claim unresolved because nothing answers to
    /// that id, and creating the pursuit *under that id* is the only
    /// thing that lets the re-resolve sweep join them; minting a new
    /// one and telling the operator to re-file would leave the claim
    /// broken forever.
    ///
    /// It is not an upsert and it is not a merge: a chosen id already
    /// in use collides at the repository, which is the honest answer —
    /// the row that is there was written by something, and this call
    /// knows nothing about it. Nor is the id inspected for meaning;
    /// a pursuit id is a surrogate, so there is nothing in one to
    /// validate beyond its form.
    #[allow(clippy::too_many_arguments)]
    pub fn new_at(
        id: PursuitId,
        persona_id: PersonaId,
        project_id: Option<ProjectId>,
        parent_id: Option<PursuitId>,
        title: Option<String>,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Self {
        Self {
            id,
            persona_id,
            project_id,
            parent_id,
            title: normalized(title),
            note: normalized(note),
            created_at: now,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Read-path twin of [`new`](Self::new): restores a stored row as a
    /// fact rather than a request to accept.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: PursuitId,
        persona_id: PersonaId,
        project_id: Option<ProjectId>,
        parent_id: Option<PursuitId>,
        title: Option<String>,
        note: Option<String>,
        created_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            persona_id,
            project_id,
            parent_id,
            title,
            note,
            created_at,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Subject that opened this pursuit (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that opened this pursuit (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the triple back out whole, for the same reason
    /// `DispatchJob` does: the only assemblable form is a recorded
    /// fact, not a mintable one.
    pub fn persisted_attribution(&self) -> PersistedAttribution {
        PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }
}

/// The closed set of lifecycle facts. One-way: no event edits another,
/// standing re-derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursuitEventKind {
    /// The intent was met; the kept set at this moment is frozen into
    /// the event's snapshot (or nothing was kept — `snapshot_id` None).
    ClosedSatisfied,
    /// The line was tried and dropped — the state ancestry cannot
    /// express, recorded first-class.
    ClosedAbandoned,
    /// Work resumed. Legal on an already-open pursuit (recorded,
    /// changes nothing).
    Reopened,
}

impl PursuitEventKind {
    /// Storage slug.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::ClosedSatisfied => "closed_satisfied",
            Self::ClosedAbandoned => "closed_abandoned",
            Self::Reopened => "reopened",
        }
    }

    /// Parses a storage slug (closed set — an unknown value is a
    /// corrupt row, not a forward-compat case).
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "closed_satisfied" => Ok(Self::ClosedSatisfied),
            "closed_abandoned" => Ok(Self::ClosedAbandoned),
            "reopened" => Ok(Self::Reopened),
            other => Err(DomainError::Validation(format!(
                "unknown pursuit event kind: {other:?}"
            ))),
        }
    }
}

/// One lifecycle fact about a pursuit.
#[derive(Debug, Clone, PartialEq)]
pub struct PursuitEvent {
    /// Surrogate id (UUID v7) — the tie-break in [`standing`].
    pub id: PursuitEventId,
    /// Pursuit the fact is about.
    pub pursuit_id: PursuitId,
    /// Redundant persona copy for cheap persona-scoped queries and the
    /// purge path (the `dispatch_job.persona_id` precedent).
    pub persona_id: PersonaId,
    /// Which fact.
    pub kind: PursuitEventKind,
    /// `ClosedSatisfied` only: the kept set at close, frozen by the
    /// close path (ascending member order — a forge-side calling
    /// convention; the core keeps caller order as snapshot identity).
    pub snapshot_id: Option<SnapshotId>,
    /// One short free-text slot.
    pub note: Option<String>,
    /// Creation time — the primary standing sort key.
    pub created_at: DateTime<Utc>,
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    author: Option<crate::domain::attribution::Author>,
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl PursuitEvent {
    /// Builds a fresh event. Rejects a snapshot on anything but
    /// `ClosedSatisfied` — the frozen conclusion is what that kind
    /// *means*, and no other fact has one.
    pub fn new(
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        kind: PursuitEventKind,
        snapshot_id: Option<SnapshotId>,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Result<Self, DomainError> {
        if snapshot_id.is_some() && kind != PursuitEventKind::ClosedSatisfied {
            return Err(DomainError::Validation(
                "only closed_satisfied freezes a kept set".into(),
            ));
        }
        Ok(Self {
            id: PursuitEventId::new(),
            pursuit_id,
            persona_id,
            kind,
            snapshot_id,
            note: normalized(note),
            created_at: now,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        })
    }

    /// Read-path twin of [`new`](Self::new).
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: PursuitEventId,
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        kind: PursuitEventKind,
        snapshot_id: Option<SnapshotId>,
        note: Option<String>,
        created_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            pursuit_id,
            persona_id,
            kind,
            snapshot_id,
            note,
            created_at,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Subject that recorded this fact (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that recorded this fact (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the triple back out whole (see
    /// [`Pursuit::persisted_attribution`]).
    pub fn persisted_attribution(&self) -> PersistedAttribution {
        PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }
}

/// Live standing of a pursuit, derived on read — never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursuitStanding {
    /// No event yet, or the latest is `reopened`.
    Open,
    /// The latest event is `closed_satisfied`.
    ClosedSatisfied,
    /// The latest event is `closed_abandoned`.
    ClosedAbandoned,
}

impl PursuitStanding {
    /// Wire slug. No parse counterpart on purpose: standing is derived,
    /// never accepted as input.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ClosedSatisfied => "closed_satisfied",
            Self::ClosedAbandoned => "closed_abandoned",
        }
    }

    /// The one kind→standing mapping, shared by [`standing`] and every
    /// batched read that carries only the latest kind — so the two can
    /// never drift, and a new event kind fails to compile until both
    /// answer for it.
    pub fn from_latest(kind: Option<PursuitEventKind>) -> Self {
        match kind {
            None | Some(PursuitEventKind::Reopened) => Self::Open,
            Some(PursuitEventKind::ClosedSatisfied) => Self::ClosedSatisfied,
            Some(PursuitEventKind::ClosedAbandoned) => Self::ClosedAbandoned,
        }
    }
}

/// Derives standing from a pursuit's events: latest by
/// `(created_at, id)` wins, no row means open. The id tie-break is not
/// decoration — v7 ids make it agree with mint order when two events
/// share a `created_at`, so the answer is total instead of
/// scan-order-dependent.
pub fn standing<'a, I>(events: I) -> PursuitStanding
where
    I: IntoIterator<Item = &'a PursuitEvent>,
{
    PursuitStanding::from_latest(
        events
            .into_iter()
            .max_by_key(|event| (event.created_at, event.id))
            .map(|event| event.kind),
    )
}

/// What a restamp moved. An enum rather than a `(kind, uuid)` pair so a
/// caller cannot file a move under a kind the id does not have; the
/// `judgment` variant arrives with the worth gate and its table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestampSubject {
    /// A dispatch round — its returns follow it automatically, because
    /// return membership derives through the dispatch join.
    Dispatch(DispatchId),
}

impl RestampSubject {
    /// Storage slug for the subject kind.
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::Dispatch(_) => "dispatch",
        }
    }

    /// The subject's raw id, for the untyped `subject_id` column.
    pub fn subject_uuid(&self) -> Uuid {
        match self {
            Self::Dispatch(id) => *id.as_uuid(),
        }
    }

    /// Parses the stored `(kind, id)` pair (closed set).
    pub fn from_columns(kind: &str, id: Uuid) -> Result<Self, DomainError> {
        match kind {
            "dispatch" => Ok(Self::Dispatch(DispatchId::from_uuid(id))),
            other => Err(DomainError::Validation(format!(
                "unknown restamp subject kind: {other:?}"
            ))),
        }
    }
}

/// One recorded move of a stamped event between pursuits — the repair
/// verb every surveyed correlation system ships for when the carrying
/// fails. The stamp is correlation metadata, not event content:
/// restamping never touches what happened, only which pursuit it files
/// under.
#[derive(Debug, Clone, PartialEq)]
pub struct PursuitRestamp {
    /// Surrogate id (UUID v7).
    pub id: PursuitRestampId,
    /// What moved.
    pub subject: RestampSubject,
    /// Filing before the move — `None` repairs a legacy NULL stamp.
    pub from_pursuit_id: Option<PursuitId>,
    /// Filing after the move. Same persona as the subject and as
    /// `from` (service-enforced; a restamp never crosses personas).
    pub to_pursuit_id: PursuitId,
    /// When the move happened.
    pub created_at: DateTime<Utc>,
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    author: Option<crate::domain::attribution::Author>,
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl PursuitRestamp {
    /// Records a move. Rejects a no-op (`from == Some(to)`) — a move
    /// that moves nothing is a row that can only mislead.
    pub fn new(
        subject: RestampSubject,
        from_pursuit_id: Option<PursuitId>,
        to_pursuit_id: PursuitId,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Result<Self, DomainError> {
        if from_pursuit_id == Some(to_pursuit_id) {
            return Err(DomainError::Validation(
                "restamp target equals the current stamp".into(),
            ));
        }
        Ok(Self {
            id: PursuitRestampId::new(),
            subject,
            from_pursuit_id,
            to_pursuit_id,
            created_at: now,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        })
    }

    /// Read-path twin of [`new`](Self::new).
    pub fn from_persisted(
        id: PursuitRestampId,
        subject: RestampSubject,
        from_pursuit_id: Option<PursuitId>,
        to_pursuit_id: PursuitId,
        created_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            subject,
            from_pursuit_id,
            to_pursuit_id,
            created_at,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Subject that ordered this move (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that performed this move (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the triple back out whole (see
    /// [`Pursuit::persisted_attribution`]).
    pub fn persisted_attribution(&self) -> PersistedAttribution {
        PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::attribution::AttributionContext;
    use chrono::Duration;

    fn ctx() -> AttributionContext {
        AttributionContext::owner_surface()
    }

    fn event(
        pursuit: PursuitId,
        persona: PersonaId,
        kind: PursuitEventKind,
        at: DateTime<Utc>,
    ) -> PursuitEvent {
        PursuitEvent::new(pursuit, persona, kind, None, None, at, &ctx()).unwrap()
    }

    #[test]
    fn no_event_means_open() {
        assert_eq!(
            standing(std::iter::empty::<&PursuitEvent>()),
            PursuitStanding::Open
        );
    }

    #[test]
    fn latest_event_wins() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let t0 = Utc::now();
        let closed = event(pursuit, persona, PursuitEventKind::ClosedSatisfied, t0);
        let reopened = event(
            pursuit,
            persona,
            PursuitEventKind::Reopened,
            t0 + Duration::seconds(1),
        );
        assert_eq!(standing([&closed, &reopened]), PursuitStanding::Open);
        let abandoned = event(
            pursuit,
            persona,
            PursuitEventKind::ClosedAbandoned,
            t0 + Duration::seconds(2),
        );
        assert_eq!(
            standing([&closed, &reopened, &abandoned]),
            PursuitStanding::ClosedAbandoned
        );
    }

    #[test]
    fn equal_timestamps_break_ties_on_id_mint_order() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let t0 = Utc::now();
        // Same created_at; the later-minted (larger v7) id must win.
        let first = event(pursuit, persona, PursuitEventKind::ClosedSatisfied, t0);
        let second = event(pursuit, persona, PursuitEventKind::Reopened, t0);
        assert!(first.id < second.id, "v7 ids mint in order");
        assert_eq!(standing([&first, &second]), PursuitStanding::Open);
    }

    #[test]
    fn repeat_close_is_a_new_fact_not_an_error() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let t0 = Utc::now();
        let a = event(pursuit, persona, PursuitEventKind::ClosedAbandoned, t0);
        let b = event(
            pursuit,
            persona,
            PursuitEventKind::ClosedSatisfied,
            t0 + Duration::seconds(1),
        );
        assert_eq!(standing([&a, &b]), PursuitStanding::ClosedSatisfied);
    }

    #[test]
    fn snapshot_only_rides_closed_satisfied() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let snap = SnapshotId::new();
        let now = Utc::now();
        assert!(
            PursuitEvent::new(
                pursuit,
                persona,
                PursuitEventKind::Reopened,
                Some(snap),
                None,
                now,
                &ctx(),
            )
            .is_err()
        );
        let ok = PursuitEvent::new(
            pursuit,
            persona,
            PursuitEventKind::ClosedSatisfied,
            Some(snap),
            None,
            now,
            &ctx(),
        )
        .unwrap();
        assert_eq!(ok.snapshot_id, Some(snap));
    }

    #[test]
    fn blank_labels_collapse_to_none() {
        let p = Pursuit::new(
            PersonaId::new(),
            None,
            None,
            Some("  ".into()),
            Some(" keep ".into()),
            Utc::now(),
            &ctx(),
        );
        assert_eq!(p.title, None);
        assert_eq!(p.note, Some("keep".into()));
    }

    /// Filing rides with the pursuit rather than being derived from
    /// anything it holds: what is passed in is what comes back, and an
    /// unfiled pursuit says so rather than guessing a project.
    #[test]
    fn a_pursuit_carries_the_filing_it_was_opened_under() {
        let project = ProjectId::new();
        let filed = Pursuit::new(
            PersonaId::new(),
            Some(project),
            None,
            None,
            None,
            Utc::now(),
            &ctx(),
        );
        assert_eq!(filed.project_id, Some(project));

        let unfiled = Pursuit::new(PersonaId::new(), None, None, None, None, Utc::now(), &ctx());
        assert_eq!(unfiled.project_id, None);
    }

    #[test]
    fn restamp_rejects_a_no_op_move() {
        let target = PursuitId::new();
        let subject = RestampSubject::Dispatch(DispatchId::new());
        assert!(PursuitRestamp::new(subject, Some(target), target, Utc::now(), &ctx()).is_err());
        assert!(PursuitRestamp::new(subject, None, target, Utc::now(), &ctx()).is_ok());
    }

    #[test]
    fn restamp_subject_round_trips_through_columns() {
        let dispatch = DispatchId::new();
        let subject = RestampSubject::Dispatch(dispatch);
        let back =
            RestampSubject::from_columns(subject.kind_slug(), subject.subject_uuid()).unwrap();
        assert_eq!(back, subject);
        assert!(RestampSubject::from_columns("judgment", Uuid::now_v7()).is_err());
    }
}
