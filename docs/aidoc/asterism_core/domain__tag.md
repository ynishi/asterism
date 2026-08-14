# asterism-core::domain::tag

`Tag` — the channel entity (a classification axis shared across
personas).

The pipeline is `Keyword` (raw auto-tag output on an asset) → `Tag`
(materialised channel). The `asset_tag` many-to-many table is the source
of truth for the relationship; a persona-to-tag join is derived through
it. A dedicated `persona_tag` table is deferred until sidebar pinning
demands it.

## Types

- `Tag` — A single channel, shared across personas.
- `TagCount` — A tag paired with the number of assets currently linked to it.
- `TagMergeOutcome` — Outcome of folding one tag into another

