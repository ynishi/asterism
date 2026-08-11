//! SQLite adapter for the `ThumbRepository` port.
//!
//! Backs onto the `thumb_cache` table declared in the initial schema:
//! `(asset_id, size_px)` primary key with the raw encoded image bytes
//! and a `created_at` timestamp. `INSERT OR REPLACE` on upsert so the
//! same importer run can safely re-emit a thumbnail if the source
//! changed.

use asterism_core::domain::repository::ThumbRepository;
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use std::collections::HashMap;

use crate::sqlite::map::infra_err;

/// Ids per `IN (...)` statement in [`ThumbRepository::get_many`].
///
/// Every id is a bound parameter and SQLite caps how many one
/// statement may carry — 32,766 on current builds, 999 on older ones.
/// This clears both, and a screenful is two orders of magnitude below
/// it anyway; the chunking exists for the backfill-shaped caller that
/// asks for thousands.
const SELECT_CHUNK: usize = 500;

/// SQLite adapter for the thumbnail cache (uses a writer isle).
#[derive(Clone)]
pub struct SqliteThumbRepository {
    isle: AsyncIsle,
}

impl SqliteThumbRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl ThumbRepository for SqliteThumbRepository {
    async fn upsert(
        &self,
        asset_id: &AssetId,
        size_px: u32,
        data: Vec<u8>,
    ) -> Result<(), DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        let now = Utc::now().timestamp_millis();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO thumb_cache \
                     (asset_id, size_px, data, created_at) VALUES (?, ?, ?, ?)",
                    params![bytes, size_px, data, now],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn get(&self, asset_id: &AssetId, size_px: u32) -> Result<Option<Vec<u8>>, DomainError> {
        let bytes = asset_id.as_uuid().as_bytes().to_vec();
        let blob = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT data FROM thumb_cache WHERE asset_id = ? AND size_px = ?",
                    params![bytes, size_px],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        Ok(blob)
    }

    async fn get_many(
        &self,
        asset_ids: &[AssetId],
        size_px: u32,
    ) -> Result<Vec<Option<Vec<u8>>>, DomainError> {
        if asset_ids.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<Vec<u8>> = asset_ids
            .iter()
            .map(|id| id.as_uuid().as_bytes().to_vec())
            .collect();
        // De-duplicated for the query, re-expanded for the answer: the
        // caller is allowed to ask for the same asset twice and the
        // contract is per slot, but the database has no reason to be
        // told twice.
        let mut wanted: Vec<Vec<u8>> = keys.clone();
        wanted.sort_unstable();
        wanted.dedup();

        let found = self
            .isle
            .call(move |conn| {
                let mut hits: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
                // Chunked because every id is a bound parameter and
                // SQLite caps how many a statement may carry. The cap
                // is 32,766 on current builds and 999 on older ones;
                // this stays under both without the caller having to
                // know which it is talking to.
                for chunk in wanted.chunks(SELECT_CHUNK) {
                    let placeholders = std::iter::repeat_n("?", chunk.len())
                        .collect::<Vec<_>>()
                        .join(",");
                    let sql = format!(
                        "SELECT asset_id, data FROM thumb_cache \
                         WHERE size_px = ? AND asset_id IN ({placeholders})"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
                    params.push(&size_px);
                    for key in chunk {
                        params.push(key);
                    }
                    let rows = stmt.query_map(params.as_slice(), |row| {
                        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                    })?;
                    for row in rows {
                        let (id, data) = row?;
                        hits.insert(id, data);
                    }
                }
                Ok::<HashMap<Vec<u8>, Vec<u8>>, rusqlite::Error>(hits)
            })
            .await
            .map_err(infra_err)?;

        Ok(keys
            .into_iter()
            .map(|key| found.get(&key).cloned())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;

    fn id_of(byte: u8) -> AssetId {
        AssetId::from_uuid(uuid::Uuid::from_bytes([byte; 16]))
    }

    /// `thumb_cache.asset_id` is a foreign key, so a fixture that only
    /// writes thumbnails writes nothing at all. Every id used below
    /// gets a row first.
    async fn seed_assets(isle: &AsyncIsle, ids: &[AssetId]) {
        let persona = uuid::Uuid::now_v7();
        let rows: Vec<uuid::Uuid> = ids.iter().map(|id| *id.as_uuid()).collect();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, 'p', 'P', 0, 0)",
                params![persona],
            )?;
            for (n, id) in rows.iter().enumerate() {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                        modality, labels, occurred_at, created_at, updated_at) \
                     VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                    params![id, persona, format!("/fixture/{n}.png")],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Teeth for the contract the caller depends on: slot `i` is
    /// `asset_ids[i]`.
    ///
    /// The fixture makes the request order disagree with both the
    /// insertion order and the sorted order the query runs on — `IN`
    /// gives no ordering guarantee, and the implementation sorts the
    /// ids before asking, so a version that returned rows in whatever
    /// order the database produced would hand the grid one asset's
    /// thumbnail under another's id. That is a wrong picture on screen,
    /// not a slow one, which is why this is asserted rather than
    /// assumed.
    #[tokio::test]
    async fn answers_in_the_order_asked_with_gaps_for_misses() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThumbRepository::new(isle);

        let (a, b, c) = (id_of(0xAA), id_of(0xBB), id_of(0xCC));
        seed_assets(&repo.isle, &[a, b, c]).await;
        repo.upsert(&a, 256, b"aaa".to_vec()).await.unwrap();
        repo.upsert(&c, 256, b"ccc".to_vec()).await.unwrap();
        // `b` is deliberately absent.

        // Asked newest-first, which is neither insertion nor sorted
        // order (0xCC > 0xBB > 0xAA).
        let out = repo.get_many(&[c, b, a], 256).await.unwrap();
        assert_eq!(
            out,
            vec![Some(b"ccc".to_vec()), None, Some(b"aaa".to_vec())],
            "each slot must carry the thumbnail of the id in that position"
        );
    }

    /// A screenful can name the same asset twice (two sizes of the same
    /// card, a re-render mid-scroll). The contract is per slot, so both
    /// slots answer rather than the second one silently going missing.
    #[tokio::test]
    async fn duplicate_ids_answer_in_every_slot() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThumbRepository::new(isle);

        let a = id_of(0x11);
        seed_assets(&repo.isle, &[a]).await;
        repo.upsert(&a, 256, b"dup".to_vec()).await.unwrap();

        let out = repo.get_many(&[a, a], 256).await.unwrap();
        assert_eq!(out, vec![Some(b"dup".to_vec()), Some(b"dup".to_vec())]);
    }

    /// The size is part of the key, and a batch asks for exactly one of
    /// them: a 256 px row must not answer a 512 px request.
    #[tokio::test]
    async fn size_is_part_of_the_match() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThumbRepository::new(isle);

        let a = id_of(0x22);
        seed_assets(&repo.isle, &[a]).await;
        repo.upsert(&a, 256, b"small".to_vec()).await.unwrap();

        assert_eq!(
            repo.get_many(&[a], 256).await.unwrap(),
            vec![Some(b"small".to_vec())]
        );
        assert_eq!(repo.get_many(&[a], 512).await.unwrap(), vec![None]);
    }

    /// An empty ask is an empty answer, not a query with `IN ()` in it
    /// (which SQLite rejects as a syntax error).
    #[tokio::test]
    async fn an_empty_batch_is_not_a_query() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThumbRepository::new(isle);
        assert!(repo.get_many(&[], 256).await.unwrap().is_empty());
    }

    /// More ids than fit in one statement: the chunking must not drop
    /// or reorder anything across the boundary.
    #[tokio::test]
    async fn chunking_preserves_every_slot() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteThumbRepository::new(isle);

        // Two chunks' worth, with only the even ones cached so the
        // gaps have to line up on the far side of the boundary too.
        let count = SELECT_CHUNK + 7;
        let ids: Vec<AssetId> = (0..count)
            .map(|i| AssetId::from_uuid(uuid::Uuid::from_u128(i as u128 + 1)))
            .collect();
        seed_assets(&repo.isle, &ids).await;
        for (i, id) in ids.iter().enumerate() {
            if i % 2 == 0 {
                repo.upsert(id, 128, vec![i as u8]).await.unwrap();
            }
        }

        let out = repo.get_many(&ids, 128).await.unwrap();
        assert_eq!(out.len(), count);
        for (i, slot) in out.iter().enumerate() {
            if i % 2 == 0 {
                assert_eq!(slot.as_deref(), Some(&[i as u8][..]), "slot {i}");
            } else {
                assert!(slot.is_none(), "slot {i} was never cached");
            }
        }
    }
}
