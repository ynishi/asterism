# asterism-infra::sqlite::repo::asset_text_index

SQLite adapter for the write side of the **Query-side** text index
(`asset_fts` + `asset_fts_key`, migration V58).

# Why an `AssetIndexer`, and not a trigger

The rows this keeps in step are written and deleted by paths that
already call [`AssetIndexer`] explicitly — index rebuild, trash,
fold, purge. Riding those calls means the SQL index and the Tantivy
index go stale or fresh together, and it needs no dependency on
whether a foreign-key cascade happens to fire a trigger (SQLite only
fires those under `PRAGMA recursive_triggers`, which is exactly the
kind of silent condition this wave is removing).

Compose it with the Tantivy adapter through
[`FanOutIndexer`](crate::search::FanOutIndexer): one `upsert` from a
caller lands in both indexes.

# No flush

`flush` is a no-op. FTS5 writes are ordinary SQL inside the isle's
transaction, so they are durable when the statement returns; there
is no pending-writer state to commit. The method stays on the port
because the Tantivy side genuinely has one.

## Types

- `SqliteAssetTextIndex` — SQLite adapter for the Query-side text index (uses a writer isle).

