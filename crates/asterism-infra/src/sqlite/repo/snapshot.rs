//! SQLite adapter for the `SnapshotRepository` port.
//!
//! `snapshot` holds the row's identity (`id`, `persona_id`,
//! `content_hash`, `created_at`); `snapshot_asset` holds the ordered
//! members. Reads join the two so the domain `Snapshot::asset_ids` field
//! is populated in one round trip.
//!
//! Writes go through a single `create_or_reuse` path: the freeze
//! is deduped on `(persona_id, content_hash)` inside one transaction, so
//! two producers that froze the same ordered members collapse onto one
//! row instead of the membership table growing linearly with every repeat
//! dispatch.

use asterism_core::domain::repository::SnapshotRepository;
use asterism_core::domain::snapshot::Snapshot;
use asterism_core::domain::value::{AssetId, PersonaId, SnapshotId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::{OptionalExtension, params};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `SnapshotRepository`.
#[derive(Clone)]
pub struct SqliteSnapshotRepository {
    isle: AsyncIsle,
}

impl SqliteSnapshotRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

struct SnapshotRow {
    id: Uuid,
    persona_id: Uuid,
    content_hash: String,
    created_at: i64,
}

impl SnapshotRow {
    const COLUMNS: &'static str = "id, persona_id, content_hash, created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            content_hash: row.get(2)?,
            created_at: row.get(3)?,
        })
    }

    fn into_domain(self, asset_ids: Vec<AssetId>) -> Result<Snapshot, DomainError> {
        Ok(Snapshot {
            id: SnapshotId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            content_hash: self.content_hash,
            asset_ids,
            created_at: ms_to_datetime(self.created_at)?,
        })
    }
}

/// Loads the ordered members of one snapshot inside an open connection.
fn load_members(
    conn: &rusqlite::Connection,
    snapshot_id: Uuid,
) -> Result<Vec<Uuid>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT asset_id FROM snapshot_asset
            WHERE snapshot_id = ?1
            ORDER BY position, asset_id",
    )?;
    stmt.query_map(params![snapshot_id], |r| r.get::<_, Uuid>(0))?
        .collect::<Result<_, _>>()
}

#[async_trait]
impl SnapshotRepository for SqliteSnapshotRepository {
    async fn create_or_reuse(&self, snapshot: &Snapshot) -> Result<Snapshot, DomainError> {
        let id = *snapshot.id.as_uuid();
        let persona_id = *snapshot.persona_id.as_uuid();
        let content_hash = snapshot.content_hash.clone();
        let created = datetime_to_ms(&snapshot.created_at);
        let asset_ids: Vec<Uuid> = snapshot.asset_ids.iter().map(|a| *a.as_uuid()).collect();

        // Reuse-or-insert is one transaction so the dedupe lookup and the
        // insert are atomic under the serialized writer (no torn double
        // insert if two producers race the same content hash).
        let (row_id, row_created): (Uuid, i64) = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let existing: Option<(Uuid, i64)> = tx
                    .query_row(
                        "SELECT id, created_at FROM snapshot
                            WHERE persona_id = ?1 AND content_hash = ?2",
                        params![persona_id, content_hash],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some((existing_id, existing_created)) = existing {
                    // Nothing to write — the canonical row already exists.
                    return Ok((existing_id, existing_created));
                }
                tx.execute(
                    "INSERT INTO snapshot (id, persona_id, content_hash, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, persona_id, content_hash, created],
                )?;
                {
                    let mut stmt = tx.prepare(
                        "INSERT INTO snapshot_asset (snapshot_id, asset_id, position)
                         VALUES (?1, ?2, ?3)",
                    )?;
                    for (idx, asset_id) in asset_ids.into_iter().enumerate() {
                        stmt.execute(params![id, asset_id, idx as i64])?;
                    }
                }
                tx.commit()?;
                Ok((id, created))
            })
            .await
            .map_err(infra_err)?;

        // The content hash guarantees identical ordered membership, so the
        // input members carry over verbatim; only the canonical id +
        // created_at differ on reuse.
        Ok(Snapshot {
            id: SnapshotId::from_uuid(row_id),
            persona_id: snapshot.persona_id,
            content_hash: snapshot.content_hash.clone(),
            asset_ids: snapshot.asset_ids.clone(),
            created_at: ms_to_datetime(row_created)?,
        })
    }

    async fn find(&self, id: &SnapshotId) -> Result<Option<Snapshot>, DomainError> {
        let uuid = *id.as_uuid();
        let loaded: Option<(SnapshotRow, Vec<Uuid>)> = self
            .isle
            .call(move |conn| {
                let row = conn.query_row(
                    &format!(
                        "SELECT {} FROM snapshot WHERE id = ?1",
                        SnapshotRow::COLUMNS
                    ),
                    params![uuid],
                    SnapshotRow::from_row,
                );
                let row = match row {
                    Ok(r) => r,
                    Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                    Err(e) => return Err(e),
                };
                let assets = load_members(conn, uuid)?;
                Ok(Some((row, assets)))
            })
            .await
            .map_err(infra_err)?;
        loaded
            .map(|(row, assets)| {
                let asset_ids = assets.into_iter().map(AssetId::from_uuid).collect();
                row.into_domain(asset_ids)
            })
            .transpose()
    }

    async fn list_containing_asset(
        &self,
        asset_id: &AssetId,
        limit: u32,
    ) -> Result<Vec<Snapshot>, DomainError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let aid = *asset_id.as_uuid();
        let cap = limit as i64;
        let rows: Vec<(SnapshotRow, Vec<Uuid>)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM snapshot
                        WHERE id IN (
                            SELECT snapshot_id FROM snapshot_asset WHERE asset_id = ?1
                        )
                        ORDER BY created_at DESC, id
                        LIMIT ?2",
                    SnapshotRow::COLUMNS
                ))?;
                let snap_rows: Vec<SnapshotRow> = stmt
                    .query_map(params![aid, cap], SnapshotRow::from_row)?
                    .collect::<Result<_, _>>()?;
                let mut out: Vec<(SnapshotRow, Vec<Uuid>)> = Vec::with_capacity(snap_rows.len());
                for row in snap_rows {
                    let assets = load_members(conn, row.id)?;
                    out.push((row, assets));
                }
                Ok(out)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(row, assets)| {
                let asset_ids = assets.into_iter().map(AssetId::from_uuid).collect();
                row.into_domain(asset_ids)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use chrono::Utc;

    /// Inserts a persona row directly (the repo under test does not own
    /// persona writes) so snapshot FKs resolve.
    async fn seed_persona(isle: &AsyncIsle, persona: Uuid) {
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived,
                                      created_at, updated_at)
                 VALUES (?1, 'p', NULL, 0, 0, 0, 0)",
                params![persona],
            )
        })
        .await
        .unwrap();
    }

    async fn seed_asset(isle: &AsyncIsle, persona: Uuid, asset: Uuid) {
        // Each asset needs a unique `source_locator` — the schema carries a
        // UNIQUE(source_kind, source_locator) constraint (V2).
        let locator = asset.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'state', 0, 0, 0)",
                params![asset, persona, locator],
            )
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reuse_returns_same_row_for_same_ordered_members() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = PersonaId::new();
        let a = AssetId::new();
        let b = AssetId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        seed_asset(&isle, *persona.as_uuid(), *a.as_uuid()).await;
        seed_asset(&isle, *persona.as_uuid(), *b.as_uuid()).await;
        let repo = SqliteSnapshotRepository::new(isle.clone());

        let first = repo
            .create_or_reuse(&Snapshot::new(persona, vec![a, b], Utc::now()).unwrap())
            .await
            .unwrap();
        // A distinct fresh entity (new id) but identical content hash must
        // collapse onto the first row.
        let second = repo
            .create_or_reuse(&Snapshot::new(persona, vec![a, b], Utc::now()).unwrap())
            .await
            .unwrap();
        assert_eq!(first.id, second.id, "same members reuse the canonical row");
        assert_eq!(second.asset_ids, vec![a, b]);

        // Exactly one snapshot row + two member rows exist.
        let (snaps, members): (i64, i64) = isle
            .call(|conn| {
                let s: i64 = conn.query_row("SELECT COUNT(*) FROM snapshot", [], |r| r.get(0))?;
                let m: i64 =
                    conn.query_row("SELECT COUNT(*) FROM snapshot_asset", [], |r| r.get(0))?;
                Ok((s, m))
            })
            .await
            .unwrap();
        assert_eq!(snaps, 1, "dedupe kept a single snapshot row");
        assert_eq!(members, 2, "members were not duplicated on reuse");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn different_order_is_a_different_snapshot() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = PersonaId::new();
        let a = AssetId::new();
        let b = AssetId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        seed_asset(&isle, *persona.as_uuid(), *a.as_uuid()).await;
        seed_asset(&isle, *persona.as_uuid(), *b.as_uuid()).await;
        let repo = SqliteSnapshotRepository::new(isle.clone());

        let ab = repo
            .create_or_reuse(&Snapshot::new(persona, vec![a, b], Utc::now()).unwrap())
            .await
            .unwrap();
        let ba = repo
            .create_or_reuse(&Snapshot::new(persona, vec![b, a], Utc::now()).unwrap())
            .await
            .unwrap();
        assert_ne!(ab.id, ba.id, "order is part of the identity");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn find_hydrates_members_in_frozen_position_order() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = PersonaId::new();
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        for id in [a, b, c] {
            seed_asset(&isle, *persona.as_uuid(), *id.as_uuid()).await;
        }
        let repo = SqliteSnapshotRepository::new(isle.clone());
        let created = repo
            .create_or_reuse(&Snapshot::new(persona, vec![c, a, b], Utc::now()).unwrap())
            .await
            .unwrap();

        let loaded = repo.find(&created.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.asset_ids,
            vec![c, a, b],
            "position column preserves the frozen order"
        );
        assert_eq!(loaded.content_hash, created.content_hash);
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn list_containing_asset_reverse_lookup() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = PersonaId::new();
        let a = AssetId::new();
        let b = AssetId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        seed_asset(&isle, *persona.as_uuid(), *a.as_uuid()).await;
        seed_asset(&isle, *persona.as_uuid(), *b.as_uuid()).await;
        let repo = SqliteSnapshotRepository::new(isle.clone());
        let snap = repo
            .create_or_reuse(&Snapshot::new(persona, vec![a, b], Utc::now()).unwrap())
            .await
            .unwrap();

        let containing_a = repo.list_containing_asset(&a, 10).await.unwrap();
        assert_eq!(containing_a.len(), 1);
        assert_eq!(containing_a[0].id, snap.id);

        let unrelated = AssetId::new();
        let containing_unrelated = repo.list_containing_asset(&unrelated, 10).await.unwrap();
        assert!(containing_unrelated.is_empty());
        driver.shutdown().await.unwrap();
    }
}
