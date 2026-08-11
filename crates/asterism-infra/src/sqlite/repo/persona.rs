//! SQLite adapter for the `PersonaRepository` port (backed by rusqlite-isle).

use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::PersonaRepository;
use asterism_core::domain::value::{PackId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// Primitive row built inside the isle closure; promotion to the domain
/// type happens outside.
struct PersonaRow {
    id: Uuid,
    pack_id: Option<String>,
    name: String,
    accent_color: Option<String>,
    display_order: i64,
    archived: bool,
    created_at: i64,
    updated_at: i64,
    trashed_at: Option<i64>,
}

impl PersonaRow {
    const COLUMNS: &'static str = "id, pack_id, name, accent_color, display_order, archived, \
         created_at, updated_at, trashed_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            pack_id: row.get(1)?,
            name: row.get(2)?,
            accent_color: row.get(3)?,
            display_order: row.get(4)?,
            archived: row.get::<_, i64>(5)? != 0,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            trashed_at: row.get(8)?,
        })
    }

    fn into_domain(self) -> Result<Persona, DomainError> {
        Ok(Persona {
            id: PersonaId::from_uuid(self.id),
            pack_id: self.pack_id.map(PackId::new).transpose()?,
            name: self.name,
            accent_color: self.accent_color,
            display_order: self.display_order,
            archived: self.archived,
            created_at: ms_to_datetime(self.created_at)?,
            updated_at: ms_to_datetime(self.updated_at)?,
            trashed_at: self.trashed_at.map(ms_to_datetime).transpose()?,
        })
    }
}

/// SQLite adapter for `PersonaRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqlitePersonaRepository {
    isle: AsyncIsle,
}

impl SqlitePersonaRepository {
    /// Wraps a writer `AsyncIsle` (pragma / schema initialisation is done
    /// by `sqlite::open`).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl PersonaRepository for SqlitePersonaRepository {
    async fn find(&self, id: &PersonaId) -> Result<Option<Persona>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT {} FROM persona WHERE id = ?1", PersonaRow::COLUMNS),
                    params![uuid],
                    PersonaRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(PersonaRow::into_domain).transpose()
    }

    async fn find_by_pack_id(&self, pack_id: &PackId) -> Result<Option<Persona>, DomainError> {
        let pack = pack_id.as_str().to_string();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM persona WHERE pack_id = ?1",
                        PersonaRow::COLUMNS
                    ),
                    params![pack],
                    PersonaRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(PersonaRow::into_domain).transpose()
    }

    async fn list(&self) -> Result<Vec<Persona>, DomainError> {
        let rows = self
            .isle
            .call(move |conn| {
                // Trashed personas leave the sidebar. `find` still
                // returns them by id, which is what restore and the
                // purge guard read.
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM persona WHERE trashed_at IS NULL \
                     ORDER BY display_order, created_at",
                    PersonaRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map([], PersonaRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(PersonaRow::into_domain).collect()
    }

    /// Upserts identity + presentation columns only. `trashed_at` is
    /// deliberately **not** in the write list: it is owned by
    /// [`trash`](Self::trash) / [`restore`](Self::restore), and letting a
    /// plain save carry it would mean a stale entity (say, one read
    /// before a trash) could silently un-trash the persona while its
    /// assets stayed in the trash.
    async fn save(&self, persona: &Persona) -> Result<(), DomainError> {
        let id = *persona.id.as_uuid();
        let pack_id = persona.pack_id.as_ref().map(|p| p.as_str().to_string());
        let name = persona.name.clone();
        let accent = persona.accent_color.clone();
        let order = persona.display_order;
        let archived = persona.archived as i64;
        let created = datetime_to_ms(&persona.created_at);
        let updated = datetime_to_ms(&persona.updated_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO persona
                         (id, pack_id, name, accent_color, display_order, archived,
                          created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                         pack_id = excluded.pack_id,
                         name = excluded.name,
                         accent_color = excluded.accent_color,
                         display_order = excluded.display_order,
                         archived = excluded.archived,
                         updated_at = excluded.updated_at",
                    params![id, pack_id, name, accent, order, archived, created, updated],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn trash(&self, id: &PersonaId, at: DateTime<Utc>) -> Result<DateTime<Utc>, DomainError> {
        let uuid = *id.as_uuid();
        let stamp = datetime_to_ms(&at);
        // Write and read-back share one closure so the returned stamp is
        // the one actually on the row — the caller keys the asset side
        // on it, and a value re-derived afterwards could belong to a
        // concurrent writer.
        let effective: Option<Option<i64>> = self
            .isle
            .call(move |conn| {
                // Conditional on `trashed_at IS NULL` so a second trash
                // keeps the original stamp — both for the retention
                // clock and because the stamp is the key
                // `restore_by_persona` matches assets on.
                conn.execute(
                    "UPDATE persona SET trashed_at = ?1 \
                     WHERE id = ?2 AND trashed_at IS NULL",
                    params![stamp, uuid],
                )?;
                conn.query_row(
                    "SELECT trashed_at FROM persona WHERE id = ?1",
                    params![uuid],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        match effective.flatten() {
            Some(ms) => ms_to_datetime(ms),
            // No row at all → the id is unknown. (A present row always
            // has a stamp here: the UPDATE either set one or found one.)
            None => Err(DomainError::not_found("persona", id)),
        }
    }

    async fn restore(&self, id: &PersonaId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let updated = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE persona SET trashed_at = NULL WHERE id = ?1",
                    params![uuid],
                )
            })
            .await
            .map_err(infra_err)?;
        if updated == 0 {
            return Err(DomainError::not_found("persona", id));
        }
        Ok(())
    }

    async fn trashed_at(&self, id: &PersonaId) -> Result<Option<DateTime<Utc>>, DomainError> {
        let uuid = *id.as_uuid();
        let stamp: Option<Option<i64>> = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT trashed_at FROM persona WHERE id = ?1",
                    params![uuid],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        stamp.flatten().map(ms_to_datetime).transpose()
    }

    async fn scan_purgeable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<PersonaId>, DomainError> {
        let cutoff_ms = datetime_to_ms(&cutoff);
        let limit_i = limit.clamp(1, 5_000) as i64;
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM persona \
                     WHERE trashed_at IS NOT NULL AND trashed_at < ?1 \
                     ORDER BY trashed_at ASC LIMIT ?2",
                )?;
                stmt.query_map(params![cutoff_ms, limit_i], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(PersonaId::from_uuid).collect())
    }

    async fn purge(&self, id: &PersonaId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        // Guard first, inside the same transaction as the delete: a live
        // persona must not be purgeable, and the check cannot sit in a
        // separate statement pair for the reason documented on
        // `AssetRepository::purge` (two processes share this file).
        //
        // 0 = purged (or absent), 1 = still live.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                // Persona delete cascades to snapshot / dispatch_job /
                // bucket, but two edges point at `snapshot` with
                // `ON DELETE RESTRICT` — `dispatch_job.snapshot_id` and
                // `bucket.origin_snapshot_id`. SQLite fires RESTRICT even
                // mid-cascade and does not guarantee sibling-table
                // ordering, so the cascade can
                // hit `snapshot` while a restricting child still exists and
                // abort. Remove the restricting references explicitly, in
                // order, inside one transaction before the persona row (and
                // its remaining cascades) go:
                //   dispatch_job → bucket.origin clear → snapshot → persona.
                let tx = conn.transaction()?;
                let live: Option<bool> = tx
                    .query_row(
                        "SELECT trashed_at IS NULL FROM persona WHERE id = ?1",
                        params![uuid],
                        |r| r.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                match live {
                    // Absent: the caller's intent already holds.
                    None => return Ok(0),
                    Some(true) => return Ok(1),
                    Some(false) => {}
                }
                tx.execute(
                    "DELETE FROM dispatch_job WHERE persona_id = ?1",
                    params![uuid],
                )?;
                tx.execute(
                    "UPDATE bucket SET origin_snapshot_id = NULL WHERE persona_id = ?1",
                    params![uuid],
                )?;
                tx.execute("DELETE FROM snapshot WHERE persona_id = ?1", params![uuid])?;
                tx.execute("DELETE FROM persona WHERE id = ?1", params![uuid])?;
                tx.commit()?;
                Ok(0)
            })
            .await
            .map_err(infra_err)?;
        if verdict == 1 {
            return Err(DomainError::Conflict(format!(
                "persona {id} is still live; trash it before purging"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod delete_order_tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::repository::PersonaRepository;

    /// A persona carrying a snapshot that is referenced by both a
    /// `dispatch_job` (RESTRICT) and a `bucket.origin_snapshot_id`
    /// (RESTRICT) must delete without an FK error, sweeping every
    /// dependent row (the persona-delete FK hazard).
    /// `trash` returns the **effective** stamp, not the one it was
    /// handed. The persona owns that value, and the caller keys the
    /// asset side on it — if a re-trash minted a fresh stamp, the
    /// already-trashed assets would carry the old one and no restore
    /// could ever match them again.
    #[tokio::test]
    async fn persona_trash_returns_the_effective_stamp_on_every_call() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, 'p', 0, 0, 0, 0)",
                params![persona],
            )
        })
        .await
        .unwrap();

        let repo = SqlitePersonaRepository::new(isle.clone());
        let id = PersonaId::from_uuid(persona);
        let first = DateTime::from_timestamp_millis(1_000).unwrap();
        let later = DateTime::from_timestamp_millis(9_000).unwrap();

        assert_eq!(repo.trash(&id, first).await.unwrap(), first);
        assert_eq!(
            repo.trash(&id, later).await.unwrap(),
            first,
            "a repeat trash reports the original stamp, not the new clock read"
        );
        assert_eq!(repo.trashed_at(&id).await.unwrap(), Some(first));

        repo.restore(&id).await.unwrap();
        assert_eq!(repo.trashed_at(&id).await.unwrap(), None);
        // After a restore the persona is live again, so the next trash
        // starts a fresh retention clock.
        assert_eq!(repo.trash(&id, later).await.unwrap(), later);

        let ghost = PersonaId::new();
        assert!(repo.trash(&ghost, first).await.is_err());
        assert!(repo.restore(&ghost).await.is_err());
        assert!(repo.trashed_at(&ghost).await.unwrap().is_none());
        // Purge stays a no-op for an unknown id.
        assert!(repo.purge(&ghost).await.is_ok());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_persona_sweeps_restrict_referenced_snapshot() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = Uuid::now_v7();
        let asset = Uuid::now_v7();
        let snapshot = Uuid::now_v7();
        let dispatch = Uuid::now_v7();
        let bucket = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived,
                                      created_at, updated_at)
                 VALUES (?1, 'p', NULL, 0, 0, 0, 0)",
                params![persona],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', 'x.md', 'state', 0, 0, 0)",
                params![asset, persona],
            )?;
            conn.execute(
                "INSERT INTO snapshot (id, persona_id, content_hash, created_at)
                 VALUES (?1, ?2, 'deadbeef', 0)",
                params![snapshot, persona],
            )?;
            conn.execute(
                "INSERT INTO snapshot_asset (snapshot_id, asset_id, position)
                 VALUES (?1, ?2, 0)",
                params![snapshot, asset],
            )?;
            // dispatch_job.snapshot_id → snapshot (ON DELETE RESTRICT).
            conn.execute(
                "INSERT INTO dispatch_job
                     (id, snapshot_id, persona_id, exporter_slug, action, state_slug,
                      created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'file', 'copy', 'pending', 0, 0)",
                params![dispatch, snapshot, persona],
            )?;
            // bucket.origin_snapshot_id → snapshot (ON DELETE RESTRICT).
            conn.execute(
                "INSERT INTO bucket (id, persona_id, name, created_at, updated_at,
                                     origin_snapshot_id)
                 VALUES (?1, ?2, 'g', 0, 0, ?3)",
                params![bucket, persona, snapshot],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let repo = SqlitePersonaRepository::new(isle.clone());
        let id = PersonaId::from_uuid(persona);
        // Purge is trash-gated now, so the ordered-cleanup path is only
        // reachable through the trash.
        assert!(
            matches!(repo.purge(&id).await, Err(DomainError::Conflict(_))),
            "a live persona must not be purgeable"
        );
        repo.trash(&id, chrono::Utc::now()).await.unwrap();
        repo.purge(&id)
            .await
            .expect("ordered cleanup must delete the persona without an FK error");

        let counts: (i64, i64, i64, i64, i64) = isle
            .call(|conn| {
                let personas: i64 =
                    conn.query_row("SELECT COUNT(*) FROM persona", [], |r| r.get(0))?;
                let snapshots: i64 =
                    conn.query_row("SELECT COUNT(*) FROM snapshot", [], |r| r.get(0))?;
                let members: i64 =
                    conn.query_row("SELECT COUNT(*) FROM snapshot_asset", [], |r| r.get(0))?;
                let dispatches: i64 =
                    conn.query_row("SELECT COUNT(*) FROM dispatch_job", [], |r| r.get(0))?;
                let buckets: i64 =
                    conn.query_row("SELECT COUNT(*) FROM bucket", [], |r| r.get(0))?;
                Ok((personas, snapshots, members, dispatches, buckets))
            })
            .await
            .unwrap();
        assert_eq!(
            counts,
            (0, 0, 0, 0, 0),
            "persona + snapshot + members + dispatch + bucket all swept"
        );
        driver.shutdown().await.unwrap();
    }
}
