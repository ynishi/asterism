# asterism-infra::sqlite

SQLite backend — connection lifecycle and schema migration built on
`rusqlite-isle`.

## Layout

- [`migrations`] — `PRAGMA user_version`-based append-only migrations.
- [`open`] / [`open_in_memory`] — entry points that return an
  `(AsyncIsle, AsyncIsleDriver)` pair with the pragmas set and the
  schema migrated.

## Responsibilities

Repository adapters (`SqlitePersonaRepository`, and so on) take an
`AsyncIsle` handle from this module and issue queries through
`isle.call(|conn| ...)`. Connection pragmas and schema initialisation
are the sole concern of this module — adapters never reissue them.

## Where migrations run

The authoritative entry point for schema migration is the
`asterism-server init` / `migrate` CLI subcommand. To keep the
first-run experience friendly, [`open_and_migrate`] also applies any
pending migrations before returning — the CLI is the source of
truth, the in-process call is a convenience for the app / server
processes so they never fail to start on a fresh install.

## Pragma choices

- `journal_mode = WAL` — the standard 1-writer / N-readers setup. A
  reader pool (`IslePool`) can be added later once contention is
  measurable.
- `synchronous = NORMAL` — the usual tradeoff paired with WAL.
- `foreign_keys = ON` — enables cascading deletes at the DB layer.
  SQLite defaults to `OFF` per connection, so this must be reissued
  inside every init closure.
- `busy_timeout = 5000` — Asterism ships two processes that share the
  same file (UI + server), so writer contention can happen; waiting for
  a few seconds is nicer than an immediate `SQLITE_BUSY`.

One non-pragma setting rides along: transactions default to `BEGIN
IMMEDIATE`. `busy_timeout` does not protect a DEFERRED transaction
that opens with a read and upgrades to a write — SQLite bypasses
the busy handler on the upgrade (waiting cannot help, the read
snapshot is already stale) and returns `SQLITE_BUSY` on the spot.
The apalis job pool writes to the same file every poll tick, so
that window is hit for real (snapshot-freeze e2e flake, 2026-07-28
/ 2026-08-01). Taking the write lock at `BEGIN` keeps every wait
on the busy-handler path.

## Functions

- `open_and_migrate` — Opens the Asterism database, applying any pending migrations before
- `open_and_migrate_in_memory` — Opens an in-memory Asterism database with migrations applied (used
- `open_expecting_latest` — Opens the database and asserts that it is already at
- `schema_version` — Reads the current schema version from an already-open isle.

