//! SQLite adapter for the write side of the **Query-side** text index
//! (`asset_fts` + `asset_fts_key`, migration V58).
//!
//! # Why an `AssetIndexer`, and not a trigger
//!
//! The rows this keeps in step are written and deleted by paths that
//! already call [`AssetIndexer`] explicitly — index rebuild, trash,
//! fold, purge. Riding those calls means the SQL index and the Tantivy
//! index go stale or fresh together, and it needs no dependency on
//! whether a foreign-key cascade happens to fire a trigger (SQLite only
//! fires those under `PRAGMA recursive_triggers`, which is exactly the
//! kind of silent condition this wave is removing).
//!
//! Compose it with the Tantivy adapter through
//! [`FanOutIndexer`](crate::search::FanOutIndexer): one `upsert` from a
//! caller lands in both indexes.
//!
//! # No flush
//!
//! `flush` is a no-op. FTS5 writes are ordinary SQL inside the isle's
//! transaction, so they are durable when the statement returns; there
//! is no pending-writer state to commit. The method stays on the port
//! because the Tantivy side genuinely has one.

use asterism_core::domain::repository::{AssetIndexer, IndexDoc};
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;

use crate::sqlite::map::infra_err;

/// SQLite adapter for the Query-side text index (uses a writer isle).
#[derive(Clone)]
pub struct SqliteAssetTextIndex {
    isle: AsyncIsle,
}

impl SqliteAssetTextIndex {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl AssetIndexer for SqliteAssetTextIndex {
    async fn upsert(&self, doc: &IndexDoc) -> Result<(), DomainError> {
        let bytes = doc.asset_id.as_uuid().as_bytes().to_vec();
        // An asset with no resolved body still gets a row: the empty
        // document matches nothing, which is the honest answer, and
        // keeping the key row means a later body upsert replaces rather
        // than duplicates.
        let body = doc.text.clone().unwrap_or_default();
        self.isle
            .call(move |conn| {
                // The key row is what makes the FTS rowid stable across
                // re-indexing: take it if it exists, mint it if not.
                conn.execute(
                    "INSERT INTO asset_fts_key (asset_id) VALUES (?)
                     ON CONFLICT(asset_id) DO NOTHING",
                    params![bytes],
                )?;
                let seq: i64 = conn.query_row(
                    "SELECT seq FROM asset_fts_key WHERE asset_id = ?",
                    params![bytes],
                    |row| row.get(0),
                )?;
                // FTS5 has no upsert; replacing means delete-then-insert
                // at the same rowid. Both statements run inside the
                // isle's transaction, so a reader never observes the
                // gap between them.
                conn.execute("DELETE FROM asset_fts WHERE rowid = ?", params![seq])?;
                conn.execute(
                    "INSERT INTO asset_fts (rowid, body) VALUES (?, ?)",
                    params![seq, body],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn remove(&self, asset_id: &AssetId) -> Result<(), DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        self.isle
            .call(move |conn| {
                let seq: Option<i64> = conn
                    .query_row(
                        "SELECT seq FROM asset_fts_key WHERE asset_id = ?",
                        params![bytes],
                        |row| row.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                // Absent is success: `remove` is called on paths that
                // do not know whether the asset ever had a body.
                if let Some(seq) = seq {
                    conn.execute("DELETE FROM asset_fts WHERE rowid = ?", params![seq])?;
                    conn.execute("DELETE FROM asset_fts_key WHERE seq = ?", params![seq])?;
                }
                Ok::<(), rusqlite::Error>(())
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn flush(&self) -> Result<(), DomainError> {
        Ok(())
    }
}
