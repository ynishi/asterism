# asterism-infra::sqlite::repo::dir

SQLite adapter for `DirRepository`.

`dir` is the sidebar organisation tree (see
`asterism_core::domain::dir` for the axis rationale). All tree
semantics that need recursion — the move-cycle guard — run as a
recursive CTE inside the writer isle, so check-then-write stays
race-free under the isle's serialized calls.

## Types

- `SqliteDirRepository` — SQLite adapter (writer isle).

