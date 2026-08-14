# asterism-infra::sqlite::repo::edge

SQLite adapter for the `EdgeRepository` port.

The port boundary is intentionally narrow so an alternative graph
backend could sit behind it later. v1 stores edges in the dedicated
`edge` table introduced in schema v1.

## Types

- `SqliteEdgeRepository` — SQLite adapter for `EdgeRepository` (uses a writer isle).

