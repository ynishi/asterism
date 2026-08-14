# asterism-infra::sqlite::repo::asset_body

SQLite adapter for the `AssetBodyRepository` port.

Backs onto the `asset_body` side table (migration V11): 1:1 with
`asset` by `asset_id`, holding the resolved plain-text body used
by the Tantivy full-text index and the session Reader fallback.
`WITHOUT ROWID` because the natural key is the BLOB `asset_id`.

## Types

- `SqliteAssetBodyRepository` — SQLite adapter for the asset-body cache (uses a writer isle).

