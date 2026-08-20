# asterism-infra::sqlite::repo::pursuit

SQLite adapter for the `PursuitRepository` port (#29, extended by
#22).

Three tables, one concern: `pursuit` (thin, immutable,
insert-only), `pursuit_event` (append-only lifecycle facts), and
`pursuit_tx` (the append-only membership ledger). Every write here
lands on one table; nothing in this adapter spans two.

## Types

- `SqlitePursuitRepository` — SQLite adapter for `PursuitRepository`.

