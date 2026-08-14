# asterism-core::domain::constellation

Constellation edge planning — pure domain logic that decides how a
`target` asset should connect to its `candidates`.

The `edge_rebuild` job (in `asterism-infra`) fetches candidates, then
calls [`plan_edges`] here to compute the edges. Keeping this step pure
(no I/O) lets us pin the weight rules with unit tests.

## Weight rules (v1)

| Condition                                | Kind             | Weight                | Label                    |
|------------------------------------------|------------------|-----------------------|--------------------------|
| Same `bundle_id`                         | `TimeProximity`  | 1.0                   | `same-bundle`            |
| `|Δt| < 24h`                             | `TimeProximity`  | 0.7                   | `same-day`               |
| `|Δt| < 48h`                             | `TimeProximity`  | 0.5                   | `near`                   |
| Shared keywords (n)                      | `KeywordOverlap` | `min(0.4 + 0.1n, 0.9)`| `shared-keyword: <top>`  |
| Different persona + `|Δt| < 24h`         | `CoPresence`     | 0.5                   | `co-presence`            |

`bundle_id` replaced the old `session_id` grouping key after the
Session-model refactor:
Session is now the Dialog-modality 1st-class entity and its id
is scoped to a single conversation, so grouping edges by it would
collapse the "tape bundle / journal kind bundle / PNG note pair"
constellations the edge fabric was designed for. `bundle_id` is
modality-agnostic and carries that role verbatim.

For each `(to, kind)` pair only the highest-weighted edge is kept. The
window restriction on candidates (bundle id / ±48h) is the fetcher's
responsibility — this function assumes they are already trimmed.

## Functions

- `plan_edges` — Plans constellation edges from `target` to each `candidate`. `target` is

