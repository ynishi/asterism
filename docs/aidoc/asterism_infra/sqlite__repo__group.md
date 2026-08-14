# asterism-infra::sqlite::repo::group

SQLite adapter for `GroupRepository`.

Storage note: the domain type is `Group`, but the SQL table is
named `bucket` because SQLite reserves `GROUP` for `GROUP BY`.
Every SQL string in this module hard-codes `bucket` /
`asset_bucket`; the wire, DTO, HTTP path and UI layers still say
"group" so users never see the rename.

## Types

- `SqliteGroupRepository` — SQLite adapter (writer isle).

