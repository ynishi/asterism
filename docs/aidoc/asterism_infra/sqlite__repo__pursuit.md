# asterism-infra::sqlite::repo::pursuit

SQLite adapter for the `PursuitRepository` port (#29).

Three tables, one concern: `pursuit` (thin, immutable, insert-only),
`pursuit_event` (append-only lifecycle facts), and `pursuit_restamp`
(the recorded repair verb). The one multi-table write — restamp —
runs in a single transaction here, because "the move is recorded"
and "the stamp moved" must not be separable facts.

## Types

- `SqlitePursuitRepository` — SQLite adapter for `PursuitRepository`.

