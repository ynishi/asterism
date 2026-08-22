# teams-infra::sqlite

SQLite backend for the teams plane — connection lifecycle and schema
migration built on `rusqlite-isle`, mirroring `asterism-infra`'s
conventions over the teams-owned database.

## Layout

- [`migrations`] — `PRAGMA user_version`-based append-only
  migrations, a fresh series starting at V1 (this database shares
  nothing with the app database — #83 §4).
- [`repo`] — the repository over the state tables and the ledger.
- [`open_and_migrate`] / [`open_and_migrate_in_memory`] — entry
  points that return an `(AsyncIsle, AsyncIsleDriver)` pair with the
  pragmas set and the schema migrated.
- [`open_existing_at_latest`] — the maintenance verbs' entry point
  (#95): pragmas only, **no migration**, refused unless the schema
  is already current.

## Pragma choices

Same set as the app database, for the same reasons:

- `journal_mode = WAL` — the 1-writer / N-readers setup #83 §4
  names as the workload fit (append-only ledger, short tx).
- `synchronous = NORMAL` — the usual tradeoff paired with WAL.
- `foreign_keys = ON` — SQLite defaults to `OFF` per connection, so
  it is reissued inside every init closure.
- `busy_timeout = 5000` — one process by deployment shape, but a
  backup command will eventually share the file, and waiting beats
  an immediate `SQLITE_BUSY`.

One non-pragma setting rides along: transactions default to `BEGIN
IMMEDIATE`. Every repository write opens a transaction that will
write, and a DEFERRED transaction that upgrades from read to write
bypasses the busy handler on the upgrade (the same trap
`asterism-infra` documents from its 2026-07/08 flakes) — taking the
write lock at `BEGIN` keeps every wait on the busy-handler path.

## Functions

- `open_and_migrate` — Opens the teams database, applying any pending migrations before
- `open_and_migrate_in_memory` — Opens an in-memory teams database with migrations applied (used by
- `open_existing_at_latest` — Opens the teams database **without migrating**, refusing unless its
- `schema_version` — Reads the current schema version from an already-open isle.

