# asterism-core::application::snapshot_service

`SnapshotService` — application surface for the immutable `Snapshot`
entity, reborn from the old `SelectionService`.

- [`create`](SnapshotService::create) — *internal* materialise of a
  new Snapshot from a set of picked assets. No public command exposes
  it (the create surface was deliberately made internal); the
  dispatch / promote handlers are
  its only callers.
- [`get_snapshot`](SnapshotService::get_snapshot) — fetch one freeze
  by id (opening it from its referencing source).
- [`list_containing`](SnapshotService::list_containing) — the P5
  reverse lookup (`asset → freezes that include it`).
- [`promote_to_group`](SnapshotService::promote_to_group) —
  materialise a hand-owned Group from a freeze's members.
- [`promote_volatile_selection`](SnapshotService::promote_volatile_selection)
  — freeze the grid's volatile pick and promote it in one step
  (right-click "Group-ify selection", W5-d).

Snapshots have no list / rename / delete surface; deletion is
the later GC job's concern, so nothing here removes rows.

Every write here takes an [`AttributionContext`] it does not persist:
neither `snapshot` nor the Group a promote mints carries an
attribution column, and none is being added (see the
[`application`](crate::application) module doc for why the argument
is required anyway).

## Types

- `SnapshotService` — Application-layer surface for `Snapshot`.

