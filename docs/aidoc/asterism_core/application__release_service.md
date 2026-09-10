# asterism-core::application::release_service

`ReleaseService` — writing out what a change point carries, and
stamping what leaves.

One verb and one hook:

- [`release`](ReleaseService::release) — freeze the change point's
  folded state, start a `file` dispatch in `copy` mode over it, and
  record that it happened.
- [`OutboundStamping`] — what the run calls back when it has written
  the files, so that each copy carries the disclosure and the history
  that chose it before the dispatch reports done.

# The freeze is driven from here, not from inside the forge

[`boundary::Store`](crate::domain::forge::boundary::store::Store)
asks the layer below exactly one question, and its module doc says
the rest — freezing a set among them — waits for the work that needs
it. This is that work, and the answer is that the store does not grow
a `freeze` method.

Two reasons, and the second is the one that decides it.

**The forge may not name what a freeze produces.** A `SnapshotId` and
a `DispatchId` are core words, `tests/forge_boundary.rs` holds the
list of words a contract across that boundary may be written in, and
`freeze(change_point) -> SnapshotId` would put two more on it. That
list is a statement about what the forge would have to carry when it
is lifted into a crate of its own, and paying two entries of it for a
record that does not have to live inside the forge is the reversal
#254 asked to be argued before it was written.

**The same walk already happens outward, from outside.**
`asterism-teams-client::publish` hands a line's current state to a
receiver the forge does not control, and it does that by reading the
line and calling the far side — not by asking the forge to hand
anything down. A release to a filesystem is that walk with a
different transport, so it is driven the same way: read the line,
fold the chain, and call the services that freeze and dispatch.

What the forge keeps is what it kept before: it decides, and the raw
layer carries. Nothing here writes a forge word onto a core row, and
nothing writes a core id onto a forge one — the release is a row of
its own that names both ([`Release`]).

# A release with nothing live is refused

A change point whose folded state has no live entry has nothing to
write out. Recording one would produce a release naming a snapshot
that cannot exist — [`Snapshot::new`](crate::domain::snapshot::Snapshot::new)
rejects an empty membership — so the choice is between refusing and
inventing a record of an export that never happened. It is refused,
as [`Blocked`](crate::error::ConflictKind::Blocked): put something on
the line and the same request works.

# Stamping is the same writer, pointed at the copy

[`DisclosureService::apply_to`] is what the `disclosure_stamp` job
already calls on the library's own artefacts. A release calls it on
the *copies* the exporter wrote, and never on the library's own
file — the record is derived from stored rows either way, so what
differs is only which path is handed in.

The manifest half being [`Skipped`](crate::domain::disclosure::Skipped)
on a build with no certificate is a supported state and not a
failure, so it is recorded per file and reported rather than raised.

## Types

- `OutboundFile` — One file a run wrote, and the library row it is a copy of.
- `ReleaseService` — Releasing a change point, and stamping what leaves.

## Traits

- `OutboundStamping` — What a dispatch calls when it has written its files and has not yet

