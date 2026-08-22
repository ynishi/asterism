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

Nothing here is stored yet. The ports are [`lines`], [`pursuits`],
[`closings`] and [`threads`]; what satisfies them lives outside this
crate, and reading one back goes through
[`model::restore`] — the one door a stored value
comes in by.

# The boundary

- **The forge names the core; what it writes there is correlation,
  never judgement.** A pursuit refers to content through frozen sets
  ([`Snapshot`]) and ids, and writes nothing onto a core row at all.
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
  [`Actor`](model::act::Actor) — a handle and a kind — because the
  forge's actors include a line's own rule, which is not a person
  and cannot be written as one. Who a handle stands for is asked
  through [`boundary::Actors`], and answered outside.
- **The core does not need the forge.** Importing, deduplicating,
  rating and trashing all work with no pursuit in sight. Sending
  anything out is the raw layer's own business, and the forge has
  no part in it.

# What is deliberately not here

**Sending work out.** The forge stages what the owner already holds
and records what became of it; it does not export, does not start a
dispatch, and does not wait for anything to come back. Export lives
in the raw layer, where what it records is a thing that happened to
the bytes.
[`snapshot`](crate::domain::snapshot) is the handle the forge holds
the core by, and belongs to the core: it is content-addressed,
deduplicated persona-wide, and carries no story about who froze it.
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

[`Snapshot`]: crate::domain::snapshot::Snapshot

