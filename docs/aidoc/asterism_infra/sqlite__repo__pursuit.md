# asterism-infra::sqlite::repo::pursuit

SQLite adapter for the `PursuitRepository` port (#29, extended by
#22).

Six tables, one concern: `pursuit` (thin, immutable, insert-only),
`pursuit_event` (append-only lifecycle facts), `pursuit_restamp`
(the recorded repair verb), `pursuit_tx` (the append-only
membership ledger), and `cull` / `cull_member` (the record of a
close's narrowing). The multi-table writes — restamp, and the
close-with-cull — each run in a single transaction here, because
"the move is recorded" and "the stamp moved" (respectively "the
pursuit closed" and "this is what it decided") must not be
separable facts.

## Types

- `SqlitePursuitRepository` — SQLite adapter for `PursuitRepository`.

