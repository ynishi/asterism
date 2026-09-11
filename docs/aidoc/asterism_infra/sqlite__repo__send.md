# asterism-infra::sqlite::repo::send

SQLite adapter for the `SendRepository` port.

`forge_send` holds where a release went, when, and by whose hand. It
is written once and read; nothing here updates a row, for the reason
[`asterism_core::domain::send`] gives.

Why this is not on `SqliteForge` is the same answer the release
adapter beside it gives, and [`super::release`] is where it is
written. What it borrows is the same thing, and for the same reason:
[`ActRow`], because an act is three columns wherever it appears.

## Types

- `SqliteSendRepository` — SQLite adapter for `SendRepository`.

