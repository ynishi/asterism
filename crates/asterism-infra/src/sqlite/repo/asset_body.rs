//! SQLite adapter for the `AssetBodyRepository` port.
//!
//! Backs onto the `asset_body` side table (migration V11): 1:1 with
//! `asset` by `asset_id`, holding the resolved plain-text body used
//! by the Tantivy full-text index and the session Reader fallback.
//! `WITHOUT ROWID` because the natural key is the BLOB `asset_id`.

use asterism_core::domain::repository::AssetBodyRepository;
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// Hard cap on `scan_after` page size (guards the `IndexRebuild`
/// backfill worker from asking for tens of thousands at once).
const MAX_SCAN_LIMIT: u32 = 2_000;

/// SQLite adapter for the asset-body cache (uses a writer isle).
#[derive(Clone)]
pub struct SqliteAssetBodyRepository {
    isle: AsyncIsle,
}

impl SqliteAssetBodyRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl AssetBodyRepository for SqliteAssetBodyRepository {
    async fn upsert(&self, asset_id: &AssetId, body_text: &str) -> Result<(), DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        let text = body_text.to_string();
        let now = Utc::now().timestamp_millis();
        let byte_len = body_text.len() as i64;
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO asset_body \
                     (asset_id, body_text, body_bytes, indexed_at) VALUES (?, ?, ?, ?)",
                    params![bytes, text, byte_len, now],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn get(&self, asset_id: &AssetId) -> Result<Option<String>, DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        let body = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT body_text FROM asset_body WHERE asset_id = ?",
                    params![bytes],
                    |row| row.get::<_, String>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        Ok(body)
    }

    async fn scan_after(
        &self,
        cursor: Option<&AssetId>,
        limit: u32,
    ) -> Result<Vec<(AssetId, String)>, DomainError> {
        let limit = limit.clamp(1, MAX_SCAN_LIMIT) as i64;
        let cursor_bytes = cursor.map(|id| id.as_uuid().as_bytes().to_vec());
        let rows = self
            .isle
            .call(move |conn| {
                let (sql, params_vec): (&str, Vec<rusqlite::types::Value>) = match cursor_bytes {
                    Some(c) => (
                        "SELECT asset_id, body_text FROM asset_body \
                         WHERE asset_id > ? ORDER BY asset_id ASC LIMIT ?",
                        vec![c.into(), limit.into()],
                    ),
                    None => (
                        "SELECT asset_id, body_text FROM asset_body \
                         ORDER BY asset_id ASC LIMIT ?",
                        vec![limit.into()],
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec
                    .iter()
                    .map(|v| v as &dyn rusqlite::ToSql)
                    .collect();
                let rows = stmt.query_map(params_ref.as_slice(), |row| {
                    let id: Uuid = row.get(0)?;
                    let body: String = row.get(1)?;
                    Ok((id, body))
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok::<Vec<(Uuid, String)>, rusqlite::Error>(out)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, body)| (AssetId::from_uuid(id), body))
            .collect())
    }

    async fn count(&self) -> Result<u64, DomainError> {
        let n: i64 = self
            .isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM asset_body", [], |row| row.get(0)))
            .await
            .map_err(infra_err)?;
        Ok(n.max(0) as u64)
    }
}
