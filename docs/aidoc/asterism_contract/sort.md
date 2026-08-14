# asterism-contract::sort

Sort specification DTO — the wire form of the grid's sort axis.

Historically `sort_json` was an opaque `String` blob owned entirely
by the UI (`SavedQuery.sort_json`, see
`asterism_core::domain::saved_query`). The backend had no type and no
evaluator: SQL order was hard-wired to `occurred_at DESC` (or a single
group's `position`). The Query Group model materialises a query's
members
into `asset_bucket` rows with a frozen `position`, which means the
backend must now *evaluate* the sort itself. This module gives that
sort a real type; the evaluator lives in
`asterism_core::domain::sort_eval` (behaviour stays in the core, shape
stays in the leaf contract crate).

# UI correspondence (drift watch)

These enums mirror the TypeScript unions `SortTarget` / `SortOrder`
in `crates/asterism-ui/src/lib/stores/filter.svelte.ts` and the
`{ target, order, reverse }` object `saveCurrentQuery` builds in
`crates/asterism-ui/src/App.svelte` (symbol references on purpose —
line numbers here drifted the moment either file moved). The serde
`rename_all = "snake_case"` reproduces the exact string tokens the UI
writes into `sort_json`, so a round-trip through this type is
byte-identical to what `saveCurrentQuery` persists. Keep the variant
set in lock-step with the UI union; a new axis on either side without
the other is the classic drift bug this module is documented against.

## Types

- `SortOrder` — Direction of ordering *inside* the chosen [`SortTarget`].
- `SortSpec` — Serialised sort axis (`sort_json` payload, and the `sort` field of
- `SortTarget` — Which asset dimension the grid sorts on.

