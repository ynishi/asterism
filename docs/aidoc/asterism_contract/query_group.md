# asterism-contract::query_group

Query Group `query_json` — the persisted rule of a query-backed Group.

A Query Group stores its *rule* (not its members) as a versioned
JSON blob on
the `bucket.query_json` column. The members are materialised into
`asset_bucket` rows by the evaluation job. This module is the typed
form of that blob:

```json
{
  "v": 1,
  "filter": { ...ListAssetsQuery... },
  "sort":   { "target": "...", "order": "...", "reverse": false },
  "search_text": "optional free text or null"
}
```

# Why a version tag

The v1 shape replaces the legacy `saved_query.filter_json` hack — where
`search_text` piggybacked *inside* the filter object
(`App.svelte:2603-2607`) and `group_ids` were pre-expanded to a flat
list (`App.svelte:618/2379`). The v1 shape splits `search_text` out as
a first-class field and keeps `group_ids` **raw** (nesting expansion is
the evaluation job's re-entrant CTE, not something frozen here). The
explicit `v` lets the migration and every future read reject an
unknown shape loudly instead of silently mis-parsing.

# `filter` reuse

`filter` is the existing [`ListAssetsQuery`](crate::query::ListAssetsQuery)
verbatim — no re-definition. `group_ids` inside it are raw (un-expanded)
group ids; the evaluator walks the nesting graph itself.

## Types

- `QueryGroupQuery` — Typed `query_json` v1 payload.
- `QueryJsonError` — Failure modes of [`QueryGroupQuery::parse`].

## Constants

- `QUERY_JSON_VERSION` — The only `query_json` schema version this build understands.

