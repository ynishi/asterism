# asterism-infra::sqlite::repo::query_group

SQLite adapter for `QueryGroupRepository` — the persistence half of
the Query Group evaluation core.

Three primitives, all against the existing schema (no migration — the
`bucket.kind` / `query_json` columns land in W2):

- [`expand_group_closure`](SqliteQueryGroupRepository::expand_group_closure)
  — recursive-CTE nesting expansion over `bucket_link`, reusing the
  reachability shape the link cycle check uses (`group.rs`).
- [`fetch_sortable_assets`](SqliteQueryGroupRepository::fetch_sortable_assets)
  — the SQL filter evaluated with no `LIMIT`, projecting exactly the
  columns the sort evaluator reads. The `WHERE` clause is built by the
  shared [`QueryParts`] so the evaluate path and the read path can
  never drift.
- [`replace_membership`](SqliteQueryGroupRepository::replace_membership)
  — bulk `DELETE` + positioned bulk `INSERT` in one transaction.

## Types

- `SqliteQueryGroupRepository` — SQLite adapter (writer isle).

