# asterism-core::domain::forge

The forge layer — the intentional history over the raw layer: a line
of work, what it took up, and the conclusion it reached.

Everything else in [`domain`](crate::domain) answers what is true of
the stored bytes. This module answers what somebody was *trying to
do*, and that is a different kind of statement: it has an actor, it
has a beginning and an end, and it is the operator's account rather
than the store's observation. Keeping the two apart is what stops
intent from being smuggled into content facts — a fold that quietly
means "this one was better", a rating pressed into service as a
verdict — which is the failure mode this split exists to prevent.

# The loop

```text
   Line = repository            Pursuit = the pursuit
    └ History                    └ Open → Round* → Close
       Genesis → ChangePoint*       Open → Round* → Close
              ▲ head                  └ base ── cut from a ChangePoint
              │                              │
              └──── close(Satisfied) ────────┘
```

[`model`] is where that is written down, and it is the whole of what
the forge holds: a line's history as a chain of change points, work
as a log of rounds, and the one act that moves both. What work takes
up is an asset the owner already manages — an ordinary raw-layer
row, named by an operation. The forge does not hold a working copy,
and there is no state to integrate at the end. What the close
integrates is a *decision*.

Nothing here stores anything. The ports are [`lines`], [`pursuits`],
[`closings`] and [`threads`]; what satisfies them lives outside this
crate, and reading one back goes through [`model::restore`], the one
door a stored value comes in by.

# The boundary

- **The forge names the core; what it writes there is correlation,
  never judgement.** A pursuit refers to content by id — one
  reference, [`Content`](model::value::Content), and no other — and
  writes nothing onto a core row at all.
  What the forge has to say about an asset — lifecycle events,
  ledger gestures — lives on forge rows that name core ids. An
  intent written onto a core row itself would put the forge's
  vocabulary on the core's rows and hand every downstream reader
  (dedupe, lineage, restore) an ambiguity to inherit.
- **Intent lives only here.** `title` and `note` are forge
  properties; a core row may record who wrote it but never *why*.
  Who wrote something is **not** a forge property either —
  [`Asset`](crate::domain::asset::Asset) records it too, so
  [`attribution`](crate::domain::attribution) is a core module the
  forge uses rather than one it owns. Moving it here would make
  `Asset::new` depend on the forge, which is the arrow above turned
  around.

  What the forge does own is its own word for it. A node records an
  [`Actor`](model::act::Actor) — a handle and a kind — and who a
  handle stands for is asked through [`boundary::Actors`] and
  answered outside. Why a handle rather than the triple is in
  [`boundary::actors`].
- **The core does not need the forge.** Importing, deduplicating,
  rating and trashing all work with no pursuit in sight. Sending
  anything out is the raw layer's own business, and the forge has
  no part in it.

# What the forge may depend on, and what enforces it

The rule is one sentence: **the forge may not name anything else in
`asterism-core` except the shared vocabulary**, which is
[`DomainError`](crate::error::DomainError),
[`AssetId`](crate::domain::value::AssetId),
`define_uuid_id` (the crate-private macro an id newtype is spelled
with) and
[`AttributionContext`](crate::domain::attribution::AttributionContext).
`tests/forge_boundary.rs` holds that list with a reason beside each
entry and fails on anything else.

`PersonaId` came off that list, and how it came off is the example
worth keeping: nothing removed it from the list on purpose.
[`boundary::Store`] stopped asking whose an asset is — the reason
is in [`boundary::store`] — and the word simply stopped appearing.
The list shrinks when the forge needs less, and never because
somebody tidied it.

**The constraint is mutual dependency with the core, and nothing
wider.** Two things follow that are easy to get backwards:

- **External crates are ordinary here.** The forge imports `chrono`,
  `uuid`, `async_trait`, `thiserror` and `std` directly, and the
  guard does not look at them on purpose — it reads `use crate::`
  lines, because what it answers for is what the forge names *in
  this crate*. A leaf crate is the same case: `asterism-contract`
  imports no Asterism crate at all, so naming it would create no
  cycle and cost the forge nothing it is protecting.
- **Where the DTO conversions live is a separate decision, and it
  is not this one.** They sit in
  [`application::mapping`](crate::application::mapping) because that
  module's own claim is that every conversion goes through it — not
  because putting them here would breach the boundary. It would
  not.

The direction is what matters: the outside may name the forge, and
the forge may not name the outside. #101 turns that into a crate
graph, where the compiler holds it instead of a test. Until then the
list above is the whole of the contract, and adding to it is a
decision about what the lifted crate would have to carry rather
than a note that something compiles.

# What is deliberately not here

**Sending work out.** The forge stages what the owner already holds
and records what became of it; it does not export, does not start a
dispatch, and does not wait for anything to come back. Export lives
in the raw layer, where what it records is a thing that happened to
the bytes.
[`snapshot`](crate::domain::snapshot) belongs to the core: it is
content-addressed, deduplicated persona-wide, and carries no story
about who froze it.
[`duplicate_conflict`](crate::domain::duplicate_conflict) and
[`merge_plan`](crate::domain::merge_plan) answer identity ("are these
the same thing"), which the store asks of itself without being told
to — the shape rhymes with a gate, and the resemblance has misled
before: worth is not identity, and a fold is not a selection.
[`provenance`](crate::domain::provenance) is how a returning artefact
reattaches to the dispatch that produced it: what it declares about
where it came from, and whether that resolved. It is a claim the
artefact
carries rather than a statement the operator makes: the exporter
writes it beside the file, ingest resolves it on the way back in, and
nobody decides anything. So it stays low in the stack, running
whether or not anybody is pursuing anything.
[`thread`](crate::domain::thread) and
[`asset_comment`](crate::domain::asset_comment) are annotation
surfaces both layers write to.

Background: the workflow design on #21. The model this layer holds
was settled on #63, and the first one — a pursuit whose standing
derived from lifecycle events, a ledger beside it, a line moved one
verb at a time — was removed whole on #102 rather than migrated.

