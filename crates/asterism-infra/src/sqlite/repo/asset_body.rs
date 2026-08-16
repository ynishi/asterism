//! SQLite adapter for the `AssetBodyRepository` port.
//!
//! Backs onto the `asset_body` side table (migration V11): 1:1 with
//! `asset` by `asset_id`, holding the resolved plain-text body used
//! by the Tantivy full-text index and the session Reader fallback.
//! `WITHOUT ROWID` because the natural key is the BLOB `asset_id`.

use asterism_core::domain::derived_text::COMPOSITION_VERSION;
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
    /// Writes the body and, in the same row, the reading that composed
    /// it ([`COMPOSITION_VERSION`]).
    ///
    /// The stamp is taken from the domain rather than passed in by the
    /// caller: it is a fact about which version of `derive_text` ran,
    /// and every caller of this verb has just run the current one. A
    /// parameter would let a handler write a number that did not match
    /// the code that produced the text, which is the one thing the
    /// column exists to make impossible.
    async fn upsert(&self, asset_id: &AssetId, body_text: &str) -> Result<(), DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        let text = body_text.to_string();
        let now = Utc::now().timestamp_millis();
        let byte_len = body_text.len() as i64;
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO asset_body \
                     (asset_id, body_text, body_bytes, indexed_at, derived_version) \
                     VALUES (?, ?, ?, ?, ?)",
                    params![bytes, text, byte_len, now, COMPOSITION_VERSION],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn delete(&self, asset_id: &AssetId) -> Result<bool, DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        let removed = self
            .isle
            .call(move |conn| {
                conn.execute("DELETE FROM asset_body WHERE asset_id = ?", params![bytes])
            })
            .await
            .map_err(infra_err)?;
        Ok(removed > 0)
    }

    /// Takes the composition stamp off a body without touching the text.
    ///
    /// The recovery path for a re-index that could not be queued. The
    /// body on the row was composed by the current reading, so the
    /// backfill's predicate — `derived_version` below the current one —
    /// does not select it, and the row would keep a document made from
    /// text that has since changed until somebody edited the asset
    /// again. Clearing the stamp puts it back in front of the walk,
    /// which is the one mechanism that runs without anybody asking.
    ///
    /// The text is left alone deliberately: it is what search answers
    /// from until the walk arrives, and a body deleted here would take
    /// the asset out of search on the strength of a queue failure.
    async fn unstamp(&self, asset_id: &AssetId) -> Result<(), DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset_body SET derived_version = NULL WHERE asset_id = ?",
                    params![bytes],
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeds a persona and an asset for it, returning the asset's id.
    /// `asset_body.asset_id` is a foreign key, so a body cannot be
    /// written for an id nothing else knows about.
    async fn seed_asset(isle: &AsyncIsle) -> AssetId {
        let persona = Uuid::now_v7();
        let asset = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, 'P', 0, 0)",
                params![persona, format!("pack-{persona}")],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                params![asset, persona, format!("/notes/{asset}.md")],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        AssetId::from_uuid(asset)
    }

    /// Teeth for the verb this port was missing: a body has to be
    /// *removable*, because the words that produced it can be deleted.
    ///
    /// The cache is the durable side — Tantivy is rebuilt from it — so a
    /// row left behind here is not a stale nicety, it is the hit coming
    /// back at the next rebuild. Asserted through `get` rather than by
    /// counting rows, so the test says what a reader would see.
    #[tokio::test]
    async fn a_body_can_be_dropped_when_there_is_nothing_left_to_say() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetBodyRepository::new(isle.clone());
        let asset = seed_asset(&isle).await;

        repo.upsert(&asset, "the one we printed for the hallway")
            .await
            .unwrap();
        assert_eq!(
            repo.get(&asset).await.unwrap().as_deref(),
            Some("the one we printed for the hallway")
        );

        repo.delete(&asset).await.unwrap();
        assert_eq!(
            repo.get(&asset).await.unwrap(),
            None,
            "the body goes with the document it fed"
        );
        assert_eq!(repo.count().await.unwrap(), 0, "and the row is gone");
    }

    /// Idempotent, as the port says. The handler calls this on every
    /// row that derives to nothing, and most of those never had a body
    /// — a delete that failed there would fail the job for a row that
    /// was already in the state being asked for.
    #[tokio::test]
    async fn deleting_a_body_that_was_never_there_is_a_no_op() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetBodyRepository::new(isle.clone());
        let asset = seed_asset(&isle).await;

        repo.delete(&asset).await.unwrap();
        repo.delete(&asset).await.unwrap();
        assert_eq!(repo.get(&asset).await.unwrap(), None);
    }

    /// The stamp is the whole reason the backfill can tell a body
    /// composed from the asset apart from one composed from a file, so
    /// the write path has to leave it on the row — not merely accept a
    /// column that exists.
    #[tokio::test]
    async fn an_upsert_stamps_the_reading_that_composed_the_body() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetBodyRepository::new(isle.clone());
        let asset = seed_asset(&isle).await;
        repo.upsert(&asset, "composed from everything the row says")
            .await
            .unwrap();

        let bytes = asset.as_uuid().as_bytes().to_vec();
        let stamped: Option<i64> = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT derived_version FROM asset_body WHERE asset_id = ?",
                    params![bytes],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            stamped,
            Some(COMPOSITION_VERSION),
            "a body written by this build says so"
        );
    }
}
