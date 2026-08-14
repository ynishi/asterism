# asterism-infra::sqlite::repo::snapshot

SQLite adapter for the `SnapshotRepository` port.

`snapshot` holds the row's identity (`id`, `persona_id`,
`content_hash`, `created_at`); `snapshot_asset` holds the ordered
members. Reads join the two so the domain `Snapshot::asset_ids` field
is populated in one round trip.

Writes go through a single `create_or_reuse` path: the freeze
is deduped on `(persona_id, content_hash)` inside one transaction, so
two producers that froze the same ordered members collapse onto one
row instead of the membership table growing linearly with every repeat
dispatch.

## Types

- `SqliteSnapshotRepository` — SQLite adapter for `SnapshotRepository`.

