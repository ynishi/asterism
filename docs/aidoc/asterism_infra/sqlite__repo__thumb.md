# asterism-infra::sqlite::repo::thumb

SQLite adapter for the `ThumbRepository` port.

Backs onto the `thumb_cache` table declared in the initial schema:
`(asset_id, size_px)` primary key with the raw encoded image bytes
and a `created_at` timestamp. `INSERT OR REPLACE` on upsert so the
same importer run can safely re-emit a thumbnail if the source
changed.

## Types

- `SqliteThumbRepository` — SQLite adapter for the thumbnail cache (uses a writer isle).

