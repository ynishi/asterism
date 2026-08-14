# asterism-core::domain::snapshot

`Snapshot` — an immutable, content-addressed freeze of an ordered
asset set (a git-tree analogue).

A Snapshot is a *pure content object*: it knows its ordered members
and the hash of that membership, and nothing else. Where it came from
(a dispatch, a promote) is recorded on the referencing event
(`dispatch_job` / `bucket.origin_snapshot_id`), never on the Snapshot
itself — that is what lets two producers that happen to freeze the
same members in the same order share one row without their provenance
bleeding together.

# Invariants

- `asset_ids` is non-empty (a freeze with no members is meaningless —
  [`Snapshot::new`] rejects it).
- `content_hash` is derived from `asset_ids` in frozen order via
  [`content_hash`](crate::domain::snapshot_hash::content_hash); order
  is part of the identity, so the same set in a different order is a
  different Snapshot.
- `asset_ids` is a *snapshot* — if the underlying assets are later
  deleted the row retains its ids; dispatch runs validate freshness
  when they materialise the input set.
- Every asset belongs to `persona_id` (enforced by the application
  service at construction; the entity only knows the ids).

## Types

- `Snapshot` — An immutable content-addressed freeze of an ordered asset set.

