# asterism-infra::sqlite::migrations

SQLite schema migrations — `PRAGMA user_version` scheme.

## How it works

`MIGRATIONS[i]` is the DDL batch that upgrades from version `i` to
`i + 1`. [`migrate`] applies every pending batch inside its own
transaction and bumps `user_version` on success. **Never rewrite a
past batch** — schema changes go at the end (append-only, mirroring
the discipline used elsewhere in the workspace).

## Schema decisions

- **Ids are 16-byte BLOBs (UUID v7).** Smaller index footprint than
  36-byte TEXT ids at Asterism's 100k+ scale. The `uuid` feature of
  `rusqlite` provides the `ToSql` / `FromSql` bridge.
- **Timestamps are `INTEGER` (unix epoch ms).** Range filters and sorts
  use indexes directly; ISO 8601 TEXT would be readable but slower and
  larger.
- **`STRICT` tables** disallow implicit type conversion (SQLite 3.37+
  is bundled by `rusqlite`).
- **Visibility is split into two columns** (`vis_restricted` and
  `vis_sharing` as a JSON array). This lets the visibility filter be
  written directly with the JSON1 extension.
- **`labels` / `keywords` / `extra` are JSON TEXT.** They stay
  denormalised until a query needs to join on them.
- **No dedicated `job` table.** Job persistence is owned by
  `apalis-sql`, which creates its own table (`Jobs`, and so on) inside
  the same DB via `SqliteStorage::setup`. Duplicating that in a
  domain-side mirror would give us two sources of truth for the same
  data.

## Functions

- `migrate` — Applies every pending migration up to the latest version. Idempotent:

## Constants

- `LATEST_VERSION` — Latest schema version (`MIGRATIONS.len()`).

