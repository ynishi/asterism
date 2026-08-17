# asterism-infra::sqlite::repo::project

SQLite adapter for the `ProjectRepository` port (#63 decisions 1–2).

Two tables: `project` (thin, immutable, insert-only, like `pursuit`)
and `line`, which is the same but owned by a project rather than a
persona. The pair is written in one transaction — a project whose
line is missing has nothing a merge could land on, and no later
read would notice the difference between that and a project nobody
has merged into yet.

## Types

- `SqliteProjectRepository` — SQLite adapter for `ProjectRepository`.

