//! The forge's surrogate ids.
//!
//! Split from [`domain::value`](crate::domain::value) so that the raw
//! layer's id vocabulary contains no forge type. Every id declared here
//! is named nowhere outside `domain::forge` and `application::forge` —
//! nothing on the raw side holds one, because a raw export carries no
//! filing.

use uuid::Uuid;

use crate::domain::value::define_uuid_id;

define_uuid_id!(
    /// Surrogate id for a `Pursuit` — the minted unit of work that
    /// filed rows are stamped with (#29). Minted, never
    /// derived from content: content identity changes whenever work is
    /// redone, so succession, rejection, and abandonment need an id
    /// that survives rework. UUID v7 so `(created_at, id)` totally
    /// orders rows minted in the same instant.
    PursuitId
);

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
