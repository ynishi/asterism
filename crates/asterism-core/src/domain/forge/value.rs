//! The forge's surrogate ids.
//!
//! Split from [`domain::value`](crate::domain::value) so that the
//! catalogue's id vocabulary contains no forge type. All but one of
//! those declared here are named nowhere outside `domain::forge` and
//! `application::forge`; the exception, [`PursuitId`], is the one the
//! catalogue has a reason to hold — a row filed under a pursuit
//! carries which one.
//!
//! That one is handled by conversion rather than by sharing the type.
//! The catalogue stamps a
//! [`CorrelationId`], an opaque
//! UUID it can carry without knowing what a pursuit is, and the forge
//! converts at its own boundary. The field, the column, and the sidecar
//! key stay `pursuit_id` on both sides: renaming them would leave one
//! value with three names, and the name was never the coupling. The
//! type was.

use uuid::Uuid;

use crate::domain::value::{CorrelationId, define_uuid_id};

define_uuid_id!(
    /// Surrogate id for a `Pursuit` — the minted unit of work that
    /// filed rows are stamped with (#29). Minted, never
    /// derived from content: content identity changes whenever work is
    /// redone, so succession, rejection, and abandonment need an id
    /// that survives rework. UUID v7 so `(created_at, id)` totally
    /// orders rows minted in the same instant.
    PursuitId
);

impl PursuitId {
    /// The stamp form the catalogue carries.
    pub fn as_correlation(&self) -> CorrelationId {
        CorrelationId::from_uuid(*self.as_uuid())
    }

    /// Reads a stamp back as the pursuit it names.
    ///
    /// Total, and deliberately so: the catalogue cannot check that a
    /// stamp still names a live pursuit, and a fallible conversion here
    /// would only move that check somewhere it still could not be made.
    /// Resolution is the forge's own lookup.
    pub fn from_correlation(value: CorrelationId) -> Self {
        Self::from_uuid(*value.as_uuid())
    }
}

impl From<PursuitId> for CorrelationId {
    fn from(value: PursuitId) -> Self {
        value.as_correlation()
    }
}

impl From<CorrelationId> for PursuitId {
    fn from(value: CorrelationId) -> Self {
        Self::from_correlation(value)
    }
}

define_uuid_id!(
    /// Surrogate id for a `PursuitEvent` — one one-way lifecycle fact
    /// about a pursuit (close / reopen). The v7 timestamp is the
    /// tie-break that makes "latest event" total when two events share
    /// a `created_at`.
    PursuitEventId
);
define_uuid_id!(
    /// Surrogate id for a `PursuitTx` — one entry in a pursuit's
    /// append-only membership ledger (#22, model on #63): an asset
    /// entering, a mid-work removal, or its reversal. Membership is
    /// derived by "latest tx per asset wins" over `(created_at, id)`;
    /// the id tie-break makes that answer total and stable when two
    /// gestures share a millisecond, though within one it is an
    /// ordering, not a causal claim.
    PursuitTxId
);
define_uuid_id!(
    /// Surrogate id for a `Project` — the repo of the forge's git
    /// analogy (#63): the shared context pursuits file under and the
    /// owner of a mainline. Minted, like a pursuit's: a project is a
    /// deliberate act, not a derived fact.
    ProjectId
);
define_uuid_id!(
    /// Surrogate id for a `Line` — one named line of a project, the
    /// branch of the forge's git analogy. v1 restricts a project to
    /// exactly one, named `main`, so "the mainline" is a description
    /// rather than a type (the V82 admit-ahead stance: schema admits
    /// siblings, code restricts).
    LineId
);
define_uuid_id!(
    /// Surrogate id for a `LineEntry` — the name-like forge identity
    /// above raw asset ids (#63 decision 1): the thing that stays
    /// "the living one" while replacement and renaming move beneath
    /// it. Raw asset ids stay one-off; the entry is what a targeted
    /// IN declares.
    LineEntryId
);
define_uuid_id!(
    /// Surrogate id for a `LineEvent` — one merge verb applied to an
    /// entry (add / replace / delete / rename, #63 decision 2).
    /// Liveness and naming derive on read by "latest event per entry
    /// wins" over `(created_at, id)`; the v7 tie-break makes that
    /// answer total when two verbs share a millisecond.
    LineEventId
);
define_uuid_id!(
    /// Surrogate id for a `Merge` — the record that one satisfied
    /// close applied its verbs (#63 decision 3: approval *is* the
    /// merge event). One merge per close event; the verbs group under
    /// it on `line_event`.
    MergeId
);
