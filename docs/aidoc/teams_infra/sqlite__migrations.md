# teams-infra::sqlite::migrations

Teams-database schema migrations — `PRAGMA user_version` scheme.

A fresh series starting at V1: this database shares nothing with the
app database, not even a version counter (#83 §4). The mechanism is
`asterism-infra`'s: `MIGRATIONS[i]` upgrades from version `i` to
`i + 1`, [`migrate`] applies every pending batch inside its own
transaction and bumps `user_version` on success. **Never rewrite a
past batch** — schema changes go at the end.

## Schema decisions

- **Ids are 16-byte BLOBs (UUID v7)**, timestamps are `INTEGER`
  epoch ms, tables are `STRICT` — the workspace conventions.
- **State tables are the SoT, the ledger is the record** (#83 §2
  audit-log pattern). `team` / `membership` / `team_blob_link` /
  `locator` are authoritative; `ledger_event` is what happened.
- **`ledger_event` is append-only in the schema, not only in the
  API**: no `updated_at`, no soft-delete column anywhere near it,
  and `BEFORE UPDATE` / `BEFORE DELETE` triggers that abort — the
  repository exposes no update/delete path, and the schema backs
  that up against raw SQL too.
- **`ledger_event` carries no foreign key to `team`.** The record
  outlives the state on purpose: deleting a team removes its rows
  (memberships and links cascade) while the same transaction
  appends `teams.team.deleted/1` — a stream that must survive the
  row it chronicles cannot reference it.
- **Subjects land in `ledger_subject`, keyed `(ref_type,
  ref_value)`** with an index in that order, so trace queries walk
  the index and never parse payload JSON (#83 §2). The table is the
  only store of an event's subjects — rebuilding the envelope joins
  it, so there is no second copy to drift.
- **`seq` is assigned by storage inside the write transaction**
  (`MAX(seq) + 1` per team): monotonic by the primary key, gapless
  under the single-writer deployment shape because no two
  transactions compute it concurrently.
- **`role` is TEXT with no CHECK constraint**: the word list is the
  domain's ([`teams_core::domain::identity::Role::parse`]), so a
  later tier is a new word and a new match arm, not a schema
  migration (#83 §1). The repository passes every stored value back
  through the domain parser on read.

## Functions

- `migrate` — Applies every pending migration up to the latest version.

## Constants

- `LATEST_VERSION` — Latest schema version (`MIGRATIONS.len()`).

