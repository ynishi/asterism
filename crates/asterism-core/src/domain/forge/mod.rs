//! The forge layer — the intentional history over the catalogue: a line
//! of work, the rounds it sent out, and the conclusion it reached.
//!
//! Everything else in [`domain`](crate::domain) answers what is true of
//! the stored bytes. This module answers what somebody was *trying to
//! do*, and that is a different kind of statement: it has an actor, it
//! has a beginning and an end, and it is the operator's account rather
//! than the store's observation. Keeping the two apart is what stops
//! intent from being smuggled into content facts — a fold that quietly
//! means "this one was better", a rating pressed into service as a
//! verdict — which is the failure mode this split exists to prevent.
//!
//! # The loop
//!
//! ```text
//!   Fork  ──> OUT ──> [ the work happens elsewhere ] ──> IN
//!    │         │                                         │
//!  pursuit  dispatch                              returns resolve
//!  .parent_id  + frozen inputs + sidecar          through _trace
//!    │                                                   │
//!    └────────────> Culling ────> Merge ─────────────────┘
//!                                pursuit_event
//!                                .closed_satisfied + kept set
//! ```
//!
//! [`pursuit`] is the minted unit of work and its lifecycle facts;
//! [`dispatch`] is one round — an exporter invocation against a frozen
//! input set, stamped with the pursuit it files under. A round's outputs
//! and the artefacts that come back are ordinary catalogue rows: the
//! forge does not hold a working copy, and there is no state to
//! integrate at the end. What the close integrates is a *decision*.
//!
//! **Culling** — the narrowing between a return and a close — has no
//! record of its own in this layer. It moves through the catalogue's
//! working state (rating, labels, trash), and what survives it is the
//! kept set the close freezes. Whether the act itself should be
//! recorded, and out of which candidate set, is open (#22).
//!
//! # The boundary
//!
//! - **The forge names the core; what it writes there is correlation,
//!   never judgement.** A pursuit refers to content through frozen sets
//!   ([`Snapshot`]) and ids. The one thing this layer puts on a core row
//!   is the id that lets the two rejoin after a round trip — the
//!   `_dispatch` stamp on a reified output, the `_trace` claim a
//!   returning artefact carries. No table here holds a verdict row per
//!   asset: that would put the forge's vocabulary on the core's rows and
//!   hand every downstream reader (dedupe, lineage, restore) an
//!   ambiguity to inherit.
//! - **Intent lives only here.** `title`, `note`, and the actor triple
//!   are forge properties. A core row may record who wrote it — that is
//!   bookkeeping, and doctrine 2 already allows it — but never *why*.
//! - **The core does not need the forge.** Importing, deduplicating,
//!   rating and trashing all work with no pursuit in sight. The minting
//!   rule (doctrine 5) binds the forge's own events, not the catalogue.
//!
//! # What is deliberately not here
//!
//! [`snapshot`](crate::domain::snapshot) is the handle the forge holds
//! the core by, and belongs to the core: it is content-addressed,
//! deduplicated persona-wide, and carries no story about who froze it.
//! [`duplicate_conflict`](crate::domain::duplicate_conflict) and
//! [`merge_plan`](crate::domain::merge_plan) answer identity ("are these
//! the same thing"), which the store asks of itself without being told
//! to — the shape rhymes with a gate, and the resemblance has misled
//! before: worth is not identity, and a fold is not a selection.
//! [`provenance`](crate::domain::provenance) is how a returning artefact
//! reattaches to the round that produced it: what it declares about
//! where it came from, and whether that resolved. It is a claim the
//! artefact
//! carries rather than a statement the operator makes: the exporter
//! writes it beside the file, ingest resolves it on the way back in, and
//! nobody decides anything. So it stays low in the stack, running
//! whether or not anybody is pursuing anything.
//! [`thread`](crate::domain::thread) and
//! [`asset_comment`](crate::domain::asset_comment) are annotation
//! surfaces both layers write to.
//!
//! Background: the workflow design on #21, implemented by #29 and #34.
//!
//! [`Snapshot`]: crate::domain::snapshot::Snapshot

pub mod dispatch;
pub mod pursuit;
