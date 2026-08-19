//! The forge layer — the intentional history over the catalogue: a line
//! of work, what it filed under itself, and the conclusion it reached.
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
//!   open
//!     │
//!     v
//!   round ──> OUT ──> [ the work happens elsewhere ] ──> IN
//!     ^        │                                          │
//!     │     dispatch + frozen inputs + sidecar     returns resolve
//!     │                                            through _trace
//!     │                                                   │
//!     └──── what survives feeds the next round <── culling
//!                                                        │
//!                                                        v
//!                                                      close
//!                          pursuit_event.closed_satisfied + kept set
//!                          (or .closed_abandoned — nothing lands)
//! ```
//!
//! [`pursuit`] is the minted unit of work and its lifecycle facts;
//! [`dispatch`](crate::domain::dispatch) is one round — an exporter
//! invocation against a frozen
//! input set, stamped with the pursuit it files under. A round's outputs
//! and the artefacts that come back are ordinary catalogue rows: the
//! forge does not hold a working copy, and there is no state to
//! integrate at the end. What the close integrates is a *decision*.
//!
//! **Culling** — the narrowing between a return and the next round or
//! the close — is recorded (#22, model on #63). Mid-work it moves
//! through the ledger ([`tx`]): every entry, removal and reversal is
//! an append-only gesture, and membership derives on read. At a
//! satisfied close the [`cull`] converts the final state into
//! verdicts — keep or reject, out of the candidate set the ledger
//! accumulated, frozen at that moment — and what survives is the next
//! round's input, or the kept set the close freezes.
//!
//! # The boundary
//!
//! - **The forge names the core; what it writes there is correlation,
//!   never judgement.** A pursuit refers to content through frozen sets
//!   ([`Snapshot`]) and ids. The one thing this layer puts on a core row
//!   is the id that lets the two rejoin after a round trip — the
//!   `_dispatch` stamp on a reified output, the `_trace` claim a
//!   returning artefact carries. What the forge has to say about an
//!   asset — lifecycle events, ledger gestures, cull verdicts — lives
//!   on forge rows that name core ids. A verdict written onto a core
//!   row itself would put the forge's vocabulary on the core's rows and
//!   hand every downstream reader (dedupe, lineage, restore) an
//!   ambiguity to inherit.
//! - **Intent lives only here.** `title` and `note` are forge
//!   properties; a core row may record who wrote it but never *why*.
//!   The actor triple is **not** a forge property —
//!   [`Asset`](crate::domain::asset::Asset) carries it too, so
//!   [`attribution`](crate::domain::attribution) is a core module the
//!   forge uses rather than one it owns. Moving it here would make
//!   `Asset::new` depend on the forge, which is the arrow above turned
//!   around.
//! - **The core does not need the forge.** Importing, deduplicating,
//!   rating and trashing all work with no pursuit in sight, and so
//!   does exporting: a round is filed under a pursuit where its caller
//!   named one, and under none where nobody did.
//!
//! # What is deliberately not here
//!
//! [`dispatch`](crate::domain::dispatch) is a catalogue module, though
//! it reads as the forge's own: it records that an exporter ran over a
//! frozen set, which is something that happened to the bytes. Delete
//! the forge and dispatch still works — `pursuit_id` is an `Option`
//! both constructors leave `None`. Delete dispatch and there is no way
//! to send anything out. It keeps the stamp because an id is the one
//! thing the catalogue may name.
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
//! The full domain model this layer is growing toward — mainline,
//! targeted IN, cull, merge-on-close — is drafted on #63.
//!
//! [`Snapshot`]: crate::domain::snapshot::Snapshot

pub mod cull;
pub mod line;
pub mod project;
pub mod pursuit;
pub mod repository;
pub mod tx;
pub mod value;
