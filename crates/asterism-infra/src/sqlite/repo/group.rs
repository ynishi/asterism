//! SQLite adapter for `GroupRepository`.
//!
//! Storage note: the domain type is `Group`, but the SQL table is
//! named `bucket` because SQLite reserves `GROUP` for `GROUP BY`.
//! Every SQL string in this module hard-codes `bucket` /
//! `asset_bucket`; the wire, DTO, HTTP path and UI layers still say
//! "group" so users never see the rename.

use asterism_core::domain::group::{Group, GroupKind, GroupLink, GroupSummary};
use asterism_core::domain::repository::GroupRepository;
use asterism_core::domain::value::{AssetId, DirId, GroupId, PersonaId, SnapshotId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::fault::StoreFault;
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// Raw row shape used inside the isle closure.
struct GroupRow {
    id: Uuid,
    persona_id: Uuid,
    name: String,
    description: Option<String>,
    dir_id: Option<Uuid>,
    created_at_ms: i64,
    updated_at_ms: i64,
    kind: String,
    query_json: Option<String>,
    origin_snapshot_id: Option<Uuid>,
    last_refresh_at_ms: Option<i64>,
    last_refresh_status: Option<String>,
    last_refresh_error: Option<String>,
}

impl GroupRow {
    const COLUMNS: &'static str = "id, persona_id, name, description, dir_id, created_at, \
         updated_at, kind, query_json, origin_snapshot_id, last_refresh_at, \
         last_refresh_status, last_refresh_error";
    /// Index of the first extra column when a query appends one after
    /// [`COLUMNS`] (for example the aggregate count in `list`).
    const NEXT_INDEX: usize = 13;

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,
            dir_id: row.get(4)?,
            created_at_ms: row.get(5)?,
            updated_at_ms: row.get(6)?,
            kind: row.get(7)?,
            query_json: row.get(8)?,
            origin_snapshot_id: row.get(9)?,
            last_refresh_at_ms: row.get(10)?,
            last_refresh_status: row.get(11)?,
            last_refresh_error: row.get(12)?,
        })
    }

    fn into_domain(self) -> Result<Group, DomainError> {
        // The `kind` column is written by this crate against a CHECK
        // constraint, so a token the model does not have is a row that
        // could not have been written — not something the caller asked
        // for and not something a different request avoids.
        let kind = GroupKind::parse(&self.kind).map_err(|_| {
            StoreFault::CorruptRow(format!(
                "a stored group row names a kind this model does not have: {:?}",
                self.kind
            ))
        })?;
        Ok(Group {
            id: GroupId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            name: self.name,
            description: self.description,
            dir_id: self.dir_id.map(DirId::from_uuid),
            kind,
            query_json: self.query_json,
            origin_snapshot_id: self.origin_snapshot_id.map(SnapshotId::from_uuid),
            last_refresh_at: self.last_refresh_at_ms.map(ms_to_datetime).transpose()?,
            last_refresh_status: self.last_refresh_status,
            last_refresh_error: self.last_refresh_error,
            created_at: ms_to_datetime(self.created_at_ms)?,
            updated_at: ms_to_datetime(self.updated_at_ms)?,
        })
    }
}

/// SQLite adapter (writer isle).
#[derive(Clone)]
pub struct SqliteGroupRepository {
    isle: AsyncIsle,
}

impl SqliteGroupRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl GroupRepository for SqliteGroupRepository {
    async fn find(&self, id: &GroupId) -> Result<Option<Group>, DomainError> {
        let uuid = *id.as_uuid();
        let row: Option<GroupRow> = self
            .isle
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    &format!("SELECT {} FROM bucket WHERE id = ?1", GroupRow::COLUMNS),
                    params![uuid],
                    GroupRow::from_row,
                )
                .optional()
            })
            .await
            .map_err(infra_err)?;
        row.map(GroupRow::into_domain).transpose()
    }

    async fn create(
        &self,
        persona_id: PersonaId,
        name: String,
        description: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Group, DomainError> {
        // Domain constructor performs the name-non-empty check.
        let group = Group::new(persona_id, name, description, now)?;
        let uuid = *group.id.as_uuid();
        let persona_uuid = *group.persona_id.as_uuid();
        let name_owned = group.name.clone();
        let description_owned = group.description.clone();
        let now_ms = datetime_to_ms(&now);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO bucket \
                        (id, persona_id, name, description, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![uuid, persona_uuid, name_owned, description_owned, now_ms],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| {
                // Name what storage did with the unique index on
                // `(persona_id, name)`; the conversion decides that a
                // taken value is what the HTTP / Tauri layer returns
                // 409 for.
                let msg = err.to_string();
                if msg.contains("UNIQUE") || msg.contains("unique") {
                    StoreFault::taken("a group name for this persona", format!("{:?}", group.name))
                        .into()
                } else {
                    infra_err(err)
                }
            })?;
        Ok(group)
    }

    async fn trash(&self, id: &GroupId, at: DateTime<Utc>) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let stamp = datetime_to_ms(&at);
        // Two statements, one closure: the UPDATE is conditional on
        // `trashed_at IS NULL` so re-trashing keeps the original stamp
        // (the retention clock must not restart), which means a
        // zero-row result cannot distinguish "already trashed" from
        // "no such group" on its own.
        let existed: bool = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE bucket SET trashed_at = ?1 \
                     WHERE id = ?2 AND trashed_at IS NULL",
                    params![stamp, uuid],
                )?;
                let existed: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM bucket WHERE id = ?1",
                    params![uuid],
                    |r| r.get(0),
                )?;
                Ok(existed > 0)
            })
            .await
            .map_err(infra_err)?;
        if !existed {
            return Err(DomainError::not_found("group", id));
        }
        Ok(())
    }

    async fn restore(&self, id: &GroupId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let existed: bool = self
            .isle
            .call(move |conn| {
                let updated = conn.execute(
                    "UPDATE bucket SET trashed_at = NULL WHERE id = ?1",
                    params![uuid],
                )?;
                Ok(updated > 0)
            })
            .await
            .map_err(infra_err)?;
        if !existed {
            return Err(DomainError::not_found("group", id));
        }
        Ok(())
    }

    async fn purge(&self, id: &GroupId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        // Guard inlined into the DELETE predicate rather than a
        // preceding SELECT — same reasoning as
        // `AssetRepository::purge`: two processes share this database
        // file, so a check-then-delete pair could destroy a group that
        // a concurrent `restore` had just brought back. FK
        // `ON DELETE CASCADE` takes the `asset_bucket` rows.
        //
        // 0 = purged (or already absent), 1 = still live.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                let deleted = conn.execute(
                    "DELETE FROM bucket WHERE id = ?1 AND trashed_at IS NOT NULL",
                    params![uuid],
                )?;
                if deleted > 0 {
                    return Ok(0);
                }
                let live: Option<bool> = conn
                    .query_row(
                        "SELECT trashed_at IS NULL FROM bucket WHERE id = ?1",
                        params![uuid],
                        |r| r.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                match live {
                    None => Ok(0),
                    Some(true) => Ok(1),
                    // Trashed but nothing deleted: a concurrent purge
                    // won. Same end state.
                    Some(false) => Ok(0),
                }
            })
            .await
            .map_err(infra_err)?;
        if verdict == 1 {
            return Err(StoreFault::blocked_by(
                format!("group {id} is still live"),
                "trash it before purging",
            )
            .into());
        }
        Ok(())
    }

    async fn scan_purgeable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<GroupId>, DomainError> {
        let cutoff_ms = datetime_to_ms(&cutoff);
        let limit_i = limit.clamp(1, 5_000) as i64;
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // Served by the partial `idx_bucket_trashed`; oldest
                // first so a capped sweep drains the longest-held rows.
                let mut stmt = conn.prepare(
                    "SELECT id FROM bucket \
                     WHERE trashed_at IS NOT NULL AND trashed_at < ?1 \
                     ORDER BY trashed_at ASC LIMIT ?2",
                )?;
                stmt.query_map(params![cutoff_ms, limit_i], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(GroupId::from_uuid).collect())
    }

    async fn list(&self, persona_id: Option<&PersonaId>) -> Result<Vec<GroupSummary>, DomainError> {
        let persona_uuid = persona_id.map(|p| *p.as_uuid());
        // LEFT JOIN so groups with zero assets still surface — a
        // freshly created empty bucket must appear in the sidebar
        // right away.
        //
        // The second LEFT JOIN onto `asset` exists only to reach
        // `trashed_at` and `folded_into`: `asset_bucket` rows
        // deliberately survive trashing (V30 keeps the asset row so
        // restore needs no replay) and survive a fold as well, so
        // counting the join table alone would advertise members the
        // grid will not show. `COUNT(asset.id)` skips the NULLs the
        // outer join produces, which keeps the zero-asset case working.
        // The fold half matters most on exactly the Groups a person has
        // been de-duplicating: those filings are what the count would
        // otherwise keep double-counting.
        //
        // The `persona` subquery is the third exclusion, and the one a
        // Group cannot express itself: trashing a persona stamps its
        // assets, not its Groups, so without it a trashed persona's
        // Groups keep sitting in the sidebar showing `count = 0` — the
        // "looks empty, is actually in the trash" reading that the same
        // fix removed from the Sessions view.
        let rows: Vec<(GroupRow, i64)> = self
            .isle
            .call(move |conn| {
                let (sql, mut params_vec): (String, Vec<rusqlite::types::Value>) =
                    if persona_uuid.is_some() {
                        (
                            format!(
                                "SELECT {}, COUNT(asset.id) AS c \
                             FROM bucket \
                             LEFT JOIN asset_bucket \
                                 ON asset_bucket.bucket_id = bucket.id \
                             LEFT JOIN asset \
                                 ON asset.id = asset_bucket.asset_id \
                                AND asset.trashed_at IS NULL \
                                AND asset.folded_into IS NULL \
                             WHERE bucket.persona_id = ?1 \
                               AND bucket.trashed_at IS NULL \
                               AND bucket.persona_id IN \
                                   (SELECT id FROM persona WHERE trashed_at IS NULL) \
                             GROUP BY bucket.id \
                             ORDER BY c DESC, bucket.name ASC",
                                GroupRow::COLUMNS
                                    .split(", ")
                                    .map(|c| format!("bucket.{c}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            Vec::new(),
                        )
                    } else {
                        (
                            format!(
                                "SELECT {}, COUNT(asset.id) AS c \
                             FROM bucket \
                             LEFT JOIN asset_bucket \
                                 ON asset_bucket.bucket_id = bucket.id \
                             LEFT JOIN asset \
                                 ON asset.id = asset_bucket.asset_id \
                                AND asset.trashed_at IS NULL \
                                AND asset.folded_into IS NULL \
                             WHERE bucket.trashed_at IS NULL \
                               AND bucket.persona_id IN \
                                   (SELECT id FROM persona WHERE trashed_at IS NULL) \
                             GROUP BY bucket.id \
                             ORDER BY c DESC, bucket.name ASC",
                                GroupRow::COLUMNS
                                    .split(", ")
                                    .map(|c| format!("bucket.{c}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            Vec::new(),
                        )
                    };
                if let Some(uuid) = persona_uuid {
                    params_vec.push(rusqlite::types::Value::Blob(uuid.as_bytes().to_vec()));
                }
                let mut stmt = conn.prepare(&sql)?;
                let iter =
                    stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                        Ok((
                            GroupRow::from_row(row)?,
                            row.get::<_, i64>(GroupRow::NEXT_INDEX)?,
                        ))
                    })?;
                iter.collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(infra_err)?;

        rows.into_iter()
            .map(|(row, count)| {
                Ok(GroupSummary {
                    group: row.into_domain()?,
                    asset_count: count.max(0) as u64,
                })
            })
            .collect()
    }

    async fn add(
        &self,
        asset_id: &AssetId,
        group_id: &GroupId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let asset = *asset_id.as_uuid();
        let group = *group_id.as_uuid();
        let now_ms = datetime_to_ms(&now);
        self.isle
            .call(move |conn| {
                // `position` = current row-count in the target bucket,
                // so a newly-added asset lands at the tail. Using a
                // subselect (not the app-side count) keeps the assignment
                // race-free under the writer isle's serialized calls.
                conn.execute(
                    "INSERT OR IGNORE INTO asset_bucket \
                        (asset_id, bucket_id, added_at, position) \
                     VALUES (?1, ?2, ?3, \
                        (SELECT COUNT(*) FROM asset_bucket WHERE bucket_id = ?2))",
                    params![asset, group, now_ms],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn add_bulk(
        &self,
        group_id: &GroupId,
        ordered: &[AssetId],
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError> {
        if ordered.is_empty() {
            return Ok(0);
        }
        let bucket = *group_id.as_uuid();
        let ids: Vec<Uuid> = ordered.iter().map(|a| *a.as_uuid()).collect();
        let now_ms = datetime_to_ms(&now);
        let written = self
            .isle
            .call(move |conn| {
                // One transaction, one prepared statement reused across
                // the loop — the 100k-member promote path.
                // Positions continue after the current tail; OR IGNORE
                // keeps the per-pair idempotence of `add`.
                let tx = conn.transaction()?;
                let start: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM asset_bucket \
                     WHERE bucket_id = ?1",
                    params![bucket],
                    |r| r.get(0),
                )?;
                let mut written = 0u64;
                {
                    let mut stmt = tx.prepare(
                        "INSERT OR IGNORE INTO asset_bucket \
                            (asset_id, bucket_id, added_at, position) \
                         VALUES (?1, ?2, ?3, ?4)",
                    )?;
                    for (offset, asset) in ids.iter().enumerate() {
                        written +=
                            stmt.execute(params![asset, bucket, now_ms, start + offset as i64])?
                                as u64;
                    }
                }
                tx.commit()?;
                Ok(written)
            })
            .await
            .map_err(infra_err)?;
        Ok(written)
    }

    async fn set_origin_snapshot(
        &self,
        group_id: &GroupId,
        snapshot_id: &asterism_core::domain::value::SnapshotId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let bucket = *group_id.as_uuid();
        let snapshot = *snapshot_id.as_uuid();
        let now_ms = datetime_to_ms(&now);
        let updated = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE bucket SET origin_snapshot_id = ?1, updated_at = ?2 \
                     WHERE id = ?3",
                    params![snapshot, now_ms, bucket],
                )
            })
            .await
            .map_err(infra_err)?;
        if updated == 0 {
            return Err(DomainError::not_found("group", group_id));
        }
        Ok(())
    }

    async fn remove(&self, asset_id: &AssetId, group_id: &GroupId) -> Result<(), DomainError> {
        let asset = *asset_id.as_uuid();
        let group = *group_id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM asset_bucket \
                     WHERE asset_id = ?1 AND bucket_id = ?2",
                    params![asset, group],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn remove_bulk(
        &self,
        group_id: &GroupId,
        asset_ids: &[AssetId],
    ) -> Result<u64, DomainError> {
        if asset_ids.is_empty() {
            return Ok(0);
        }
        let bucket = *group_id.as_uuid();
        let ids: Vec<Uuid> = asset_ids.iter().map(|a| *a.as_uuid()).collect();
        self.isle
            .call(move |conn| {
                // One transaction, one prepared statement — mirrors
                // `add_bulk` so a large AI-driven cleanup batch is a
                // single writer-isle call.
                let tx = conn.transaction()?;
                let mut removed = 0u64;
                {
                    let mut stmt = tx.prepare(
                        "DELETE FROM asset_bucket \
                         WHERE asset_id = ?1 AND bucket_id = ?2",
                    )?;
                    for asset in &ids {
                        removed += stmt.execute(params![asset, bucket])? as u64;
                    }
                }
                tx.commit()?;
                Ok(removed)
            })
            .await
            .map_err(infra_err)
    }

    async fn merge(
        &self,
        from: &GroupId,
        into: &GroupId,
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError> {
        let from_id = *from.as_uuid();
        let into_id = *into.as_uuid();
        let now_ms = datetime_to_ms(&now);
        let from_disp = from.to_string();
        let into_disp = into.to_string();
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                // Existence gate inside the same transaction so the
                // whole merge is atomic against concurrent deletes.
                //
                // Both ends must be **live**. Merge dissolves the source
                // with a bare `DELETE FROM bucket`, which is the one
                // bucket delete that does not go through `purge`; letting
                // it accept a trashed end would either hard-delete a
                // group the user thought was recoverable, or quietly move
                // members into an invisible target.
                for (id, disp) in [(from_id, &from_disp), (into_id, &into_disp)] {
                    let present: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM bucket WHERE id = ?1 AND trashed_at IS NULL",
                        params![id],
                        |r| r.get(0),
                    )?;
                    if present == 0 {
                        // Only failure path in this closure — the outer
                        // `map_err` turns the id into a `NotFound`.
                        return Ok(Err(disp.clone()));
                    }
                }
                let start: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM asset_bucket \
                     WHERE bucket_id = ?1",
                    params![into_id],
                    |r| r.get(0),
                )?;
                // Move only the members the target lacks, appended
                // after its tail in the source's position order.
                let moved = tx.execute(
                    "INSERT INTO asset_bucket (asset_id, bucket_id, added_at, position) \
                     SELECT ab.asset_id, ?1, ?2, \
                            ?3 + (ROW_NUMBER() OVER (ORDER BY ab.position)) - 1 \
                     FROM asset_bucket ab \
                     WHERE ab.bucket_id = ?4 \
                       AND NOT EXISTS (SELECT 1 FROM asset_bucket t \
                                       WHERE t.bucket_id = ?1 \
                                         AND t.asset_id = ab.asset_id)",
                    params![into_id, now_ms, start, from_id],
                )? as u64;
                // Dissolve the source; FK cascade drops its links.
                tx.execute("DELETE FROM bucket WHERE id = ?1", params![from_id])?;
                tx.commit()?;
                Ok(Ok(moved))
            })
            .await
            .map_err(infra_err)?
            .map_err(|id| DomainError::not_found("group", id))
    }

    async fn reorder(&self, group_id: &GroupId, ordered: &[AssetId]) -> Result<(), DomainError> {
        let group = *group_id.as_uuid();
        let ids: Vec<Uuid> = ordered.iter().map(|a| *a.as_uuid()).collect();
        self.isle
            .call(move |conn| {
                // One transaction so a mid-way error rolls back cleanly;
                // partial reorders would leave the collection in a
                // nonsense state that the UI cannot recover from.
                let tx = conn.transaction()?;
                for (index, asset) in ids.iter().enumerate() {
                    tx.execute(
                        "UPDATE asset_bucket SET position = ?1 \
                         WHERE bucket_id = ?2 AND asset_id = ?3",
                        params![index as i64, group, asset],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn groups_of(&self, asset_id: &AssetId) -> Result<Vec<Group>, DomainError> {
        let uuid = *asset_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM bucket \
                     JOIN asset_bucket ON asset_bucket.bucket_id = bucket.id \
                     WHERE asset_bucket.asset_id = ?1 \
                       AND bucket.trashed_at IS NULL \
                     ORDER BY bucket.name",
                    GroupRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![uuid], GroupRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(GroupRow::into_domain).collect()
    }

    async fn member_asset_ids(&self, group_id: &GroupId) -> Result<Vec<AssetId>, DomainError> {
        let uuid = *group_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT asset_id FROM asset_bucket \
                     WHERE bucket_id = ?1 \
                     ORDER BY position, asset_id",
                )?;
                let rows = stmt
                    .query_map(params![uuid], |row| row.get::<_, Uuid>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn rename(
        &self,
        id: &GroupId,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Group, DomainError> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err(DomainError::Validation(
                "Group name must not be empty".into(),
            ));
        }
        let uuid = *id.as_uuid();
        let now_ms = datetime_to_ms(&now);
        let name_for_sql = trimmed.clone();
        let row = self
            .isle
            .call(move |conn| {
                let updated = conn.execute(
                    "UPDATE bucket SET name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![name_for_sql, now_ms, uuid],
                )?;
                if updated == 0 {
                    return Ok(None);
                }
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM bucket WHERE id = ?1",
                    GroupRow::COLUMNS
                ))?;
                let row = stmt.query_row(params![uuid], GroupRow::from_row)?;
                Ok(Some(row))
            })
            .await
            .map_err(|err| {
                let msg = err.to_string();
                if msg.contains("UNIQUE") || msg.contains("unique") {
                    StoreFault::taken("a group name for this persona", format!("{trimmed:?}"))
                        .into()
                } else {
                    infra_err(err)
                }
            })?;
        match row {
            Some(row) => row.into_domain(),
            None => Err(DomainError::not_found("group", id)),
        }
    }

    async fn set_dir(
        &self,
        id: &GroupId,
        dir_id: Option<&DirId>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let dir_uuid = dir_id.map(|d| *d.as_uuid());
        let now_ms = datetime_to_ms(&now);
        // 0 = ok, 1 = persona mismatch / missing dir, 2 = missing group.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                // Guard: the target dir must belong to the same
                // persona as the group. NULL (root) always passes.
                if let Some(dir) = dir_uuid {
                    let same_persona: bool = conn.query_row(
                        "SELECT EXISTS (
                             SELECT 1 FROM dir
                              JOIN bucket ON bucket.id = ?1
                             WHERE dir.id = ?2
                               AND dir.persona_id = bucket.persona_id)",
                        params![uuid, dir],
                        |row| row.get(0),
                    )?;
                    if !same_persona {
                        return Ok(1);
                    }
                }
                let updated = conn.execute(
                    "UPDATE bucket SET dir_id = ?1, updated_at = ?2 WHERE id = ?3",
                    params![dir_uuid, now_ms, uuid],
                )?;
                Ok(if updated == 0 { 2 } else { 0 })
            })
            .await
            .map_err(infra_err)?;
        match verdict {
            1 => Err(DomainError::Validation(
                "dir and group belong to different personas (or the dir does not exist)".into(),
            )),
            2 => Err(DomainError::not_found("group", id)),
            _ => Ok(()),
        }
    }

    async fn link(
        &self,
        parent: &GroupId,
        child: &GroupId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if parent == child {
            return Err(StoreFault::Impossible("a group cannot contain itself".into()).into());
        }
        let parent_uuid = *parent.as_uuid();
        let child_uuid = *child.as_uuid();
        let now_ms = datetime_to_ms(&now);
        // 0 = ok, 1 = persona mismatch / missing group, 2 = cycle.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                let same_persona: bool = conn.query_row(
                    "SELECT EXISTS (
                         SELECT 1 FROM bucket p, bucket c
                          WHERE p.id = ?1 AND c.id = ?2
                            AND p.persona_id = c.persona_id)",
                    params![parent_uuid, child_uuid],
                    |row| row.get(0),
                )?;
                if !same_persona {
                    return Ok(1);
                }
                // Cycle check: connecting parent → child closes a
                // cycle iff parent is already reachable *from* child
                // through existing links (the child's descendant set
                // contains the parent). Serialized writer calls make
                // check-then-insert race-free.
                let cycle: bool = conn.query_row(
                    "WITH RECURSIVE reach(id) AS (
                         SELECT ?2
                         UNION
                         SELECT bl.child_id FROM bucket_link bl
                           JOIN reach ON bl.parent_id = reach.id
                     )
                     SELECT EXISTS (SELECT 1 FROM reach WHERE id = ?1)",
                    params![parent_uuid, child_uuid],
                    |row| row.get(0),
                )?;
                if cycle {
                    return Ok(2);
                }
                // Composite guard (write-site (b)): the CTE above
                // only sees bucket_link edges; query-rule references
                // form dependency edges too, and the union must stay
                // acyclic (a cycle turns the refresh into a
                // mutual-trigger loop). Same isle call as the insert →
                // check-then-write stays atomic (review F1).
                let persona: Uuid = conn.query_row(
                    "SELECT persona_id FROM bucket WHERE id = ?1",
                    params![parent_uuid],
                    |row| row.get(0),
                )?;
                let graph = crate::sqlite::repo::query_group::load_dependency_graph(conn, persona)?;
                if asterism_core::domain::query_group_eval::reaches(
                    &graph,
                    &GroupId::from_uuid(child_uuid),
                    &GroupId::from_uuid(parent_uuid),
                ) {
                    return Ok(3);
                }
                // Tail placement, mirroring `asset_bucket` add.
                conn.execute(
                    "INSERT OR IGNORE INTO bucket_link \
                        (parent_id, child_id, added_at, position) \
                     VALUES (?1, ?2, ?3, \
                        (SELECT COUNT(*) FROM bucket_link WHERE parent_id = ?1))",
                    params![parent_uuid, child_uuid, now_ms],
                )?;
                Ok(0)
            })
            .await
            .map_err(infra_err)?;
        match verdict {
            1 => Err(DomainError::Validation(
                "groups belong to different personas (or one of them does not exist)".into(),
            )),
            2 => Err(StoreFault::blocked_by(
                "link rejected: it would make the groups contain each other (cycle)",
                "unlink the opposing edge first, and the same link then works",
            )
            .into()),
            3 => Err(StoreFault::blocked_by(
                "link rejected: it would close a dependency cycle through a \
                 query group's references",
                "break the cycle at one of its other edges first, and the same link then works",
            )
            .into()),
            _ => Ok(()),
        }
    }

    async fn unlink(&self, parent: &GroupId, child: &GroupId) -> Result<(), DomainError> {
        let parent_uuid = *parent.as_uuid();
        let child_uuid = *child.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM bucket_link WHERE parent_id = ?1 AND child_id = ?2",
                    params![parent_uuid, child_uuid],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn links(&self, persona_id: Option<&PersonaId>) -> Result<Vec<GroupLink>, DomainError> {
        let persona_uuid = persona_id.map(|p| *p.as_uuid());
        let rows: Vec<(Uuid, Uuid, i64)> = self
            .isle
            .call(move |conn| {
                // Both ends must be live: this feeds the sidebar tree,
                // and `list` no longer returns trashed groups, so a link
                // pointing at one would render as a child with no node.
                // The unscoped branch has to join `bucket` twice purely
                // to reach the two `trashed_at` columns.
                let (sql, params_vec): (&str, Vec<rusqlite::types::Value>) = match persona_uuid {
                    Some(uuid) => (
                        "SELECT bl.parent_id, bl.child_id, bl.position \
                             FROM bucket_link bl \
                             JOIN bucket p ON p.id = bl.parent_id \
                             JOIN bucket c ON c.id = bl.child_id \
                             WHERE p.persona_id = ?1 \
                               AND p.trashed_at IS NULL \
                               AND c.trashed_at IS NULL \
                             ORDER BY bl.parent_id, bl.position, bl.added_at",
                        vec![rusqlite::types::Value::Blob(uuid.as_bytes().to_vec())],
                    ),
                    None => (
                        "SELECT bl.parent_id, bl.child_id, bl.position \
                             FROM bucket_link bl \
                             JOIN bucket p ON p.id = bl.parent_id \
                             JOIN bucket c ON c.id = bl.child_id \
                             WHERE p.trashed_at IS NULL \
                               AND c.trashed_at IS NULL \
                             ORDER BY bl.parent_id, bl.position, bl.added_at",
                        Vec::new(),
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let iter = stmt
                    .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?;
                iter.collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(|(parent, child, position)| GroupLink {
                parent_id: GroupId::from_uuid(parent),
                child_id: GroupId::from_uuid(child),
                position,
            })
            .collect())
    }

    async fn reorder_children(
        &self,
        parent: &GroupId,
        ordered: &[GroupId],
    ) -> Result<(), DomainError> {
        let parent_uuid = *parent.as_uuid();
        let ids: Vec<Uuid> = ordered.iter().map(|g| *g.as_uuid()).collect();
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                for (index, child) in ids.iter().enumerate() {
                    tx.execute(
                        "UPDATE bucket_link SET position = ?1 \
                         WHERE parent_id = ?2 AND child_id = ?3",
                        params![index as i64, parent_uuid, child],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::error::ConflictKind;

    /// Persona row seeded directly — this repo does not own persona
    /// writes, but bucket / asset FKs need one.
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
        // One locator per asset, derived from the id. The uniqueness
        // that used to require it is gone (V61), but a distinct value
        // per row costs nothing and keeps these fixtures readable.
        // Written as the column holds it, since the rows are read back
        // through the repository.
        let locator = crate::sqlite::stored_locator(&asset.to_string());
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

    /// Members of a group as `(asset_id, position)` in position order.
    async fn members(isle: &AsyncIsle, group: Uuid) -> Vec<(Uuid, i64)> {
        isle.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT asset_id, position FROM asset_bucket \
                 WHERE bucket_id = ?1 ORDER BY position",
            )?;
            let iter = stmt.query_map(params![group], |r| Ok((r.get(0)?, r.get(1)?)))?;
            iter.collect::<Result<Vec<_>, _>>()
        })
        .await
        .unwrap()
    }

    /// `member_asset_ids` answers with the filing, not the live view:
    /// a trashed member keeps its `asset_bucket` row by design, and
    /// the caller (the #65 remark fan-out) wants the batch a sentence
    /// was said over — which includes the member that happens to be
    /// in the trash — in hand-arranged order.
    #[tokio::test]
    async fn member_asset_ids_returns_the_filing_in_order_trashed_included() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = Uuid::now_v7();
        seed_persona(&isle, persona).await;
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        seed_asset(&isle, persona, a).await;
        seed_asset(&isle, persona, b).await;

        let group = repo
            .create(PersonaId::from_uuid(persona), "g".into(), None, Utc::now())
            .await
            .unwrap();
        for id in [a, b] {
            repo.add(&AssetId::from_uuid(id), &group.id, Utc::now())
                .await
                .unwrap();
        }
        isle.call(move |conn| {
            conn.execute("UPDATE asset SET trashed_at = 1 WHERE id = ?1", params![a])
        })
        .await
        .unwrap();

        let listed = repo.member_asset_ids(&group.id).await.unwrap();
        assert_eq!(
            listed,
            vec![AssetId::from_uuid(a), AssetId::from_uuid(b)],
            "filing order, trashed member included"
        );
    }

    /// The sidebar count is what a Group *is* to the person reading it,
    /// so it counts what clicking the Group would show. A headstone
    /// (V49) keeps its `asset_bucket` filing exactly the way a trashed
    /// asset does, so the count is where a resolved duplicate would go
    /// on being counted twice — on precisely the Groups somebody has
    /// just been de-duplicating.
    #[tokio::test]
    async fn a_folded_member_stops_counting_towards_its_group() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let (keeper, headstone) = (AssetId::new(), AssetId::new());
        for id in [&keeper, &headstone] {
            seed_asset(&isle, *persona.as_uuid(), *id.as_uuid()).await;
        }
        let group = repo
            .create(persona, "Keepers".into(), None, Utc::now())
            .await
            .unwrap();
        repo.add_bulk(&group.id, &[keeper, headstone], Utc::now())
            .await
            .unwrap();

        let before = repo.list(Some(&persona)).await.unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].asset_count, 2, "two filings before the fold");

        let (h, k) = (*headstone.as_uuid(), *keeper.as_uuid());
        isle.call(move |conn| {
            let marked = conn.execute(
                "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
                params![h, k],
            )?;
            assert_eq!(marked, 1, "the fixture must actually stand a headstone");
            Ok(())
        })
        .await
        .unwrap();

        let after = repo.list(Some(&persona)).await.unwrap();
        assert_eq!(after.len(), 1, "the Group itself is untouched");
        assert_eq!(
            after[0].asset_count, 1,
            "the filing outlives the fold; the count must not"
        );
    }

    /// The Group trash has to be *worth* having: what it protects is the
    /// drag-arranged member order, which no user wants to rebuild by
    /// hand. This walks the full lifecycle and asserts the order survives
    /// trash → restore, that the sidebar hides the group while it is
    /// trashed, and that purge is unreachable without trashing first.
    #[tokio::test]
    async fn group_trash_preserves_member_order_and_purge_requires_trashing_first() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let (a, b) = (AssetId::new(), AssetId::new());
        for id in [&a, &b] {
            seed_asset(&isle, *persona.as_uuid(), *id.as_uuid()).await;
        }
        let group = repo
            .create(persona, "Keepers".into(), None, Utc::now())
            .await
            .unwrap();
        // Hand-arranged order: b before a.
        repo.add_bulk(&group.id, &[b, a], Utc::now()).await.unwrap();
        let seeded = members(&isle, *group.id.as_uuid()).await;
        assert_eq!(seeded.len(), 2);
        assert_eq!(seeded[0].0, *b.as_uuid(), "b was placed first");

        // Purge before trashing is refused — the destructive verb is
        // reachable only through the trash.
        let refused = repo.purge(&group.id).await;
        assert!(
            matches!(
                refused,
                Err(DomainError::Conflict {
                    kind: ConflictKind::Blocked,
                    ..
                })
            ),
            "purging a live group must conflict, got {refused:?}"
        );
        assert_eq!(
            members(&isle, *group.id.as_uuid()).await.len(),
            2,
            "the refused purge must not have touched the membership"
        );

        repo.trash(&group.id, Utc::now()).await.unwrap();
        assert!(
            repo.list(Some(&persona))
                .await
                .unwrap()
                .iter()
                .all(|s| s.group.id != group.id),
            "a trashed group leaves the sidebar listing"
        );
        assert!(
            repo.find(&group.id).await.unwrap().is_some(),
            "but stays reachable by id so restore has something to act on"
        );
        assert_eq!(
            members(&isle, *group.id.as_uuid()).await,
            seeded,
            "trashing preserves the membership and its order exactly"
        );
        assert!(
            repo.groups_of(&a).await.unwrap().is_empty(),
            "the detail overlay stops offering a trashed group as a toggle"
        );

        repo.restore(&group.id).await.unwrap();
        assert!(
            repo.list(Some(&persona))
                .await
                .unwrap()
                .iter()
                .any(|s| s.group.id == group.id),
            "restore brings the group back to the sidebar"
        );
        assert_eq!(
            members(&isle, *group.id.as_uuid()).await,
            seeded,
            "restore replays nothing — the order was never lost"
        );

        // Now purge is allowed, and it takes the filing with it.
        repo.trash(&group.id, Utc::now()).await.unwrap();
        repo.purge(&group.id).await.unwrap();
        assert!(repo.find(&group.id).await.unwrap().is_none());
        assert!(
            members(&isle, *group.id.as_uuid()).await.is_empty(),
            "purge cascades the asset_bucket rows"
        );
        // The member assets themselves outlive the filing.
        let live_assets: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(live_assets, 2, "purging a Group never deletes its members");
    }

    /// A Group has no trash stamp of its own when its *persona* is
    /// trashed — the persona trash stamps assets, not Groups — so the
    /// sidebar can only hide it by asking about the persona. Left in, it
    /// reads as an ordinary empty Group (`count = 0`, because the member
    /// count already excludes trashed assets) until the retention sweep
    /// destroys it.
    #[tokio::test]
    async fn group_list_hides_a_trashed_personas_groups() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let group = repo
            .create(persona, "Filed".into(), None, Utc::now())
            .await
            .unwrap();

        assert!(
            repo.list(None)
                .await
                .unwrap()
                .iter()
                .any(|s| s.group.id == group.id),
            "listed while the persona is live"
        );

        let pid = *persona.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "UPDATE persona SET trashed_at = 1000 WHERE id = ?1",
                params![pid],
            )
        })
        .await
        .unwrap();

        assert!(
            repo.list(None).await.unwrap().is_empty(),
            "a trashed persona's Groups leave the unscoped listing"
        );
        assert!(
            repo.list(Some(&persona)).await.unwrap().is_empty(),
            "…and the persona-scoped listing too"
        );
        assert!(
            repo.find(&group.id).await.unwrap().is_some(),
            "but the row survives, ready for the persona's restore"
        );
    }

    /// Browsing one Group must label and arrange the page by *that*
    /// Group. The primary group (`group_ids[0]`, and the owner of
    /// `primary_group_position`) used to be whichever id sorted lower, so
    /// a card filed in two Groups reported the other Group's name — the
    /// grid band read `nnn (query)` while `nnn` was the checked Group —
    /// and the client comparator then re-sorted the page by the other
    /// Group's slots, overriding the `asset_bucket.position` order this
    /// very query had selected.
    #[tokio::test]
    async fn primary_group_follows_the_filtered_group() {
        use crate::sqlite::repo::asset::SqliteAssetRepository;
        use asterism_core::domain::asset::AssetQuery;
        use asterism_core::domain::repository::AssetRepository;

        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let groups = SqliteGroupRepository::new(isle.clone());
        let assets = SqliteAssetRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let first = AssetId::new();
        let second = AssetId::new();
        for asset in [&first, &second] {
            seed_asset(&isle, *persona.as_uuid(), *asset.as_uuid()).await;
        }

        let a = groups
            .create(persona, "A".into(), None, Utc::now())
            .await
            .unwrap();
        let b = groups
            .create(persona, "B".into(), None, Utc::now())
            .await
            .unwrap();
        // Both Groups hold the same pair in opposite order, so a card's
        // slot says which Group answered: slot 0 in one is slot 1 in the
        // other.
        for (group, order) in [(&a, [&first, &second]), (&b, [&second, &first])] {
            for asset in order {
                groups.add(asset, &group.id, Utc::now()).await.unwrap();
            }
        }
        // Filter on the Group the old `bucket_id` rule would *not* have
        // picked, so the assertion fails on the bug instead of passing by
        // coincidence.
        let filtered = if a.id.as_uuid() < b.id.as_uuid() {
            &b
        } else {
            &a
        };
        let query = AssetQuery {
            persona_id: Some(persona),
            group_ids: vec![filtered.id],
            ..Default::default()
        };

        let page = assets.list(&query).await.unwrap();
        assert_eq!(page.items.len(), 2, "both members of the filtered Group");
        for (slot, card) in page.items.iter().enumerate() {
            assert_eq!(
                card.group_ids.first(),
                Some(&filtered.id),
                "the filtered Group is the primary one"
            );
            assert_eq!(
                card.primary_group_position,
                Some(slot as i64),
                "…so the card's slot is its slot *there*, which is the \
                 order the page already arrived in"
            );
        }

        // The grid reads the index projection, so it needs the same
        // answer — this is the path the band label and the `As arranged`
        // comparator actually consume.
        let index = assets.list_index(&query).await.unwrap();
        assert_eq!(index.items.len(), 2);
        for (slot, item) in index.items.iter().enumerate() {
            assert_eq!(item.group_ids.first(), Some(&filtered.id));
            assert_eq!(item.primary_group_position, Some(slot as i64));
        }
    }

    /// The grid card must not carry the id of a Group the sidebar has
    /// stopped listing. `asset_bucket` rows survive a trashed Group by
    /// design, so the bulk group-id join has to reach `bucket` — and the
    /// group-axis sort resolves those ids against `list`, so a stale id
    /// silently files the card under "unfiled".
    #[tokio::test]
    async fn card_group_ids_drop_trashed_groups() {
        use crate::sqlite::repo::asset::SqliteAssetRepository;
        use asterism_core::domain::asset::AssetQuery;
        use asterism_core::domain::repository::AssetRepository;

        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let groups = SqliteGroupRepository::new(isle.clone());
        let assets = SqliteAssetRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let asset = AssetId::new();
        seed_asset(&isle, *persona.as_uuid(), *asset.as_uuid()).await;

        let kept = groups
            .create(persona, "Kept".into(), None, Utc::now())
            .await
            .unwrap();
        let doomed = groups
            .create(persona, "Doomed".into(), None, Utc::now())
            .await
            .unwrap();
        for group in [&kept, &doomed] {
            groups.add(&asset, &group.id, Utc::now()).await.unwrap();
        }

        let ids_on_card = |assets: SqliteAssetRepository| async move {
            let page = assets
                .list(&AssetQuery {
                    persona_id: Some(persona),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(page.items.len(), 1);
            let mut ids: Vec<_> = page.items[0].group_ids.clone();
            ids.sort_by_key(|g| *g.as_uuid());
            ids
        };

        let mut both = vec![kept.id, doomed.id];
        both.sort_by_key(|g| *g.as_uuid());
        assert_eq!(ids_on_card(assets.clone()).await, both, "seed state");

        groups.trash(&doomed.id, Utc::now()).await.unwrap();
        assert_eq!(
            ids_on_card(assets.clone()).await,
            vec![kept.id],
            "a trashed group must not appear on the card"
        );
        assert_eq!(
            members(&isle, *doomed.id.as_uuid()).await.len(),
            1,
            "…while the filing itself is still on disk, ready for restore"
        );

        groups.restore(&doomed.id).await.unwrap();
        assert_eq!(
            ids_on_card(assets).await,
            both,
            "restore puts the id back on the card"
        );
    }

    /// Re-trashing must not restart the retention clock: a user who hits
    /// the button twice should not win their group extra days of life.
    #[tokio::test]
    async fn group_trash_is_idempotent_and_keeps_the_original_stamp() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let group = repo
            .create(persona, "G".into(), None, Utc::now())
            .await
            .unwrap();
        let uuid = *group.id.as_uuid();
        let stamp_of = |isle: AsyncIsle| async move {
            isle.call(move |conn| {
                conn.query_row(
                    "SELECT trashed_at FROM bucket WHERE id = ?1",
                    params![uuid],
                    |r| r.get::<_, Option<i64>>(0),
                )
            })
            .await
            .unwrap()
        };

        let first = DateTime::from_timestamp_millis(1_000).unwrap();
        let later = DateTime::from_timestamp_millis(9_000).unwrap();
        repo.trash(&group.id, first).await.unwrap();
        assert_eq!(stamp_of(isle.clone()).await, Some(1_000));
        repo.trash(&group.id, later).await.unwrap();
        assert_eq!(
            stamp_of(isle.clone()).await,
            Some(1_000),
            "the second trash keeps the original stamp"
        );

        repo.restore(&group.id).await.unwrap();
        assert_eq!(stamp_of(isle.clone()).await, None);
        // A missing id is an error on both reversible verbs — the caller
        // named something that does not exist.
        let ghost = GroupId::new();
        assert!(repo.trash(&ghost, first).await.is_err());
        assert!(repo.restore(&ghost).await.is_err());
        // Purge stays a no-op, because "make sure this is gone" already
        // holds for a row that is not there.
        assert!(repo.purge(&ghost).await.is_ok());
    }

    #[tokio::test]
    async fn remove_bulk_removes_only_listed_pairs_and_counts() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let (a, b, c) = (AssetId::new(), AssetId::new(), AssetId::new());
        for id in [&a, &b, &c] {
            seed_asset(&isle, *persona.as_uuid(), *id.as_uuid()).await;
        }
        let group = repo
            .create(persona, "g".into(), None, Utc::now())
            .await
            .unwrap();
        repo.add_bulk(&group.id, &[a, b, c], Utc::now())
            .await
            .unwrap();

        // Remove a + c plus one never-linked id: count reflects only
        // the rows that actually existed.
        let stranger = AssetId::new();
        let removed = repo
            .remove_bulk(&group.id, &[a, c, stranger])
            .await
            .unwrap();
        assert_eq!(removed, 2);
        let left = members(&isle, *group.id.as_uuid()).await;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, *b.as_uuid());

        // Second run is a no-op (idempotent per pair).
        let removed_again = repo.remove_bulk(&group.id, &[a, c]).await.unwrap();
        assert_eq!(removed_again, 0);
    }

    #[tokio::test]
    async fn merge_appends_missing_members_after_tail_and_deletes_source() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let (a, b, c, d) = (
            AssetId::new(),
            AssetId::new(),
            AssetId::new(),
            AssetId::new(),
        );
        for id in [&a, &b, &c, &d] {
            seed_asset(&isle, *persona.as_uuid(), *id.as_uuid()).await;
        }
        let into = repo
            .create(persona, "into".into(), None, Utc::now())
            .await
            .unwrap();
        let from = repo
            .create(persona, "from".into(), None, Utc::now())
            .await
            .unwrap();
        repo.add_bulk(&into.id, &[a, b], Utc::now()).await.unwrap();
        // `b` overlaps — only c, d move; their source order (c before
        // d) must survive the append.
        repo.add_bulk(&from.id, &[b, c, d], Utc::now())
            .await
            .unwrap();

        let moved = repo.merge(&from.id, &into.id, Utc::now()).await.unwrap();
        assert_eq!(moved, 2);

        let rows = members(&isle, *into.id.as_uuid()).await;
        let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids,
            vec![*a.as_uuid(), *b.as_uuid(), *c.as_uuid(), *d.as_uuid()]
        );
        // Appended after the target's tail with contiguous positions.
        assert_eq!(rows[2].1, 2);
        assert_eq!(rows[3].1, 3);

        // Source group is gone (row + links via cascade).
        assert!(repo.find(&from.id).await.unwrap().is_none());
        assert!(members(&isle, *from.id.as_uuid()).await.is_empty());
    }

    #[tokio::test]
    async fn merge_missing_group_is_not_found() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteGroupRepository::new(isle.clone());
        let persona = PersonaId::new();
        seed_persona(&isle, *persona.as_uuid()).await;
        let real = repo
            .create(persona, "real".into(), None, Utc::now())
            .await
            .unwrap();
        let ghost = GroupId::from_uuid(Uuid::new_v4());

        let err = repo.merge(&ghost, &real.id, Utc::now()).await.unwrap_err();
        assert!(matches!(err, DomainError::NotFound { .. }));
        // Target side missing fails the same way.
        let err2 = repo.merge(&real.id, &ghost, Utc::now()).await.unwrap_err();
        assert!(matches!(err2, DomainError::NotFound { .. }));
        // The existing group is untouched by the failed merges.
        assert!(repo.find(&real.id).await.unwrap().is_some());
    }
}
