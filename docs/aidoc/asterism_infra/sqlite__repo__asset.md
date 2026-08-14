# asterism-infra::sqlite::repo::asset

SQLite adapter for the `AssetRepository` port.

Hot-path methods (`list` / `list_index`) build [`AssetCard`] /
[`AssetIndex`](asterism_core::domain::asset::AssetIndex) projections
directly from a row scan without materialising the full entity.
Visibility filtering is always applied inside SQL; to make it
impossible to forget, the `WHERE` clause is built by a single
[`QueryParts`] helper — `filter_ids` (the SQL half of the search
path) goes through the same builder.

## Types

- `BodyCandidate` — One asset the body-cache backfill is considering.
- `SqliteAssetRepository` — SQLite adapter for `AssetRepository` (uses a writer isle).

