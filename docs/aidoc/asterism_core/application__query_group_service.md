# asterism-core::application::query_group_service

`QueryGroupService` — the Query Group evaluate-and-materialize
pipeline.

One entry point,
[`evaluate_and_materialize`](QueryGroupService::evaluate_and_materialize),
runs the whole pass for a single query group:

```text
query_json ──parse──▶ QueryGroupQuery
           │
           ├─ filter ──to_asset_query──▶ AssetQuery
           │                 │
           │   raw group_ids ─┴─ expand_group_closure (recursive CTE)
           │                                │
           ├─ search_text ──▶ AssetQuery::text_match (a SQL term)
           │                                │
           │            fetch_sortable_assets (SQL filter, no LIMIT)
           │                                │
           ├─ sort ─ SortContext (persona / modality / group lookups)
           │                                │
           │                        sort_asset_ids  → ordered ids
           │                                │
           └────────────── replace_membership (bulk DELETE + INSERT)
```

# Callers

Three of them sit on the transport side and each evaluates the one
group it just touched: [`create_query_group`](QueryGroupService::create_query_group)
and [`update_query`](QueryGroupService::update_query) (both a Tauri
command *and* an HTTP route), plus `DispatchService::run`'s
pre-freeze refresh. The fourth is
[`QueryGroupRefreshService`](crate::application_support::QueryGroupRefreshService),
which loops this over every group for the W4 refresh job and the
startup pass — that sweep has no wire surface, which is why it
lives in `application_support` while this evaluator stays here.
The service touches no jobs infrastructure either way: it is pure
orchestration over the repository ports, callable from any context.

The two wire verbs take an [`AttributionContext`] they do not
persist (no group column carries attribution); the evaluator itself
takes none at all — see its own doc comment for why that asymmetry
is the doctrine rather than an omission.

# A Query Group is defined by predicates only

`search_text` resolves to `AssetQuery::text_match`, a `WHERE` term
evaluated in SQL alongside the tag / modality / date terms. It is
**not** routed through the retrieval port, and this service holds no
handle on one.

It used to. The `search_text` branch asked for a ranked shortlist and
intersected it with the SQL result, which put two properties into a
stored set definition that a stored set definition cannot have:

- a shortlist is capped by construction, so a text matching more
  assets than the ceiling dropped the tail out of the membership with
  nothing on screen to say so, and
- retrieval promises no determinism, so two refreshes over unchanged
  data could name different members.

As a predicate both go away — membership is exact, countable, and the
same on every refresh.

## Types

- `QueryGroupService` — Application-layer surface for Query Group evaluation.

