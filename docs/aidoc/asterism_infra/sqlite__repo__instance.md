# asterism-infra::sqlite::repo::instance

SQLite adapter for the `InstanceRepository` port.

One row, minted by the V49 migration. The adapter deliberately does
**not** create it on read: a migrated database always has it, so an
empty table is an anomaly, and minting a replacement would hand out
an identity that disagrees with whatever the rows on disk were
attributed against.

## Types

- `SqliteInstanceRepository` — SQLite adapter for `InstanceRepository` (uses a writer isle).

