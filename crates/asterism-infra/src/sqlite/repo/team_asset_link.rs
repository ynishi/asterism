//! SQLite adapter for the `AssetLinkRepository` port — what a
//! promotion left at home (#148 decisions 8 and 9).
//!
//! The table carries no foreign key, so the check the schema declines
//! to enforce is written here instead: [`dangling_locally`] is an
//! anti-join against `asset` — a `NOT EXISTS` looking for the rows a
//! delete left behind — and nothing else in this file reaches outside
//! `team_asset_link`. The V104 migration argues why the key is absent;
//! this module is the other half of that argument, the part that goes
//! looking.
//!
//! [`dangling_locally`]: SqliteAssetLinkRepository::dangling_locally

use asterism_core::domain::repository::AssetLinkRepository;
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// SQLite adapter for `AssetLinkRepository`.
#[derive(Clone)]
pub struct SqliteAssetLinkRepository {
    isle: AsyncIsle,
}

impl SqliteAssetLinkRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// The row's own columns, in the order the statements below read them.
struct LinkRow {
    team_id: Uuid,
    line_id: Uuid,
    entry_id: Uuid,
    local_asset_id: Uuid,
    pushed_at: i64,
}

impl LinkRow {
    const COLUMNS: &'static str = "team_id, line_id, entry_id, local_asset_id, pushed_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            team_id: row.get(0)?,
            line_id: row.get(1)?,
            entry_id: row.get(2)?,
            local_asset_id: row.get(3)?,
            pushed_at: row.get(4)?,
        })
    }

    fn into_domain(self) -> AssetLink {
        AssetLink {
            key: AssetLinkKey {
                team_id: TeamScopedId::from_uuid(self.team_id),
                line_id: TeamScopedId::from_uuid(self.line_id),
                entry_id: TeamScopedId::from_uuid(self.entry_id),
            },
            local_asset_id: AssetId::from_uuid(self.local_asset_id),
            pushed_at_ms: self.pushed_at,
        }
    }
}

#[async_trait]
impl AssetLinkRepository for SqliteAssetLinkRepository {
    async fn record(&self, link: &AssetLink) -> Result<(), DomainError> {
        let team_id = *link.key.team_id.as_uuid();
        let line_id = *link.key.line_id.as_uuid();
        let entry_id = *link.key.entry_id.as_uuid();
        let asset_id = *link.local_asset_id.as_uuid();
        let pushed_at = link.pushed_at_ms;
        self.isle
            .call(move |conn| {
                // `DO NOTHING` rather than an upsert: the row records
                // that a promotion happened, and a retry of the same
                // promotion is the same fact. Overwriting would move
                // `pushed_at` onto the retry, which would make the
                // record say the promotion happened later than it did.
                conn.execute(
                    "INSERT INTO team_asset_link
                         (team_id, line_id, entry_id, local_asset_id, pushed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (team_id, line_id, entry_id) DO NOTHING",
                    params![team_id, line_id, entry_id, asset_id, pushed_at],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn list_for_team(&self, team_id: TeamScopedId) -> Result<Vec<AssetLink>, DomainError> {
        let team = *team_id.as_uuid();
        let rows: Vec<LinkRow> = self
            .isle
            .call(move |conn| {
                let sql = format!(
                    "SELECT {} FROM team_asset_link
                      WHERE team_id = ?1
                      ORDER BY pushed_at, line_id, entry_id",
                    LinkRow::COLUMNS
                );
                let mut stmt = conn.prepare(&sql)?;
                let found = stmt
                    .query_map(params![team], LinkRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(found)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(LinkRow::into_domain).collect())
    }

    async fn for_asset(
        &self,
        team_id: TeamScopedId,
        local_asset_id: &AssetId,
    ) -> Result<Vec<AssetLink>, DomainError> {
        let team = *team_id.as_uuid();
        let asset = *local_asset_id.as_uuid();
        let rows: Vec<LinkRow> = self
            .isle
            .call(move |conn| {
                // The read `idx_team_asset_link_on_asset` exists for.
                let sql = format!(
                    "SELECT {} FROM team_asset_link
                      WHERE team_id = ?1 AND local_asset_id = ?2
                      ORDER BY pushed_at, line_id, entry_id",
                    LinkRow::COLUMNS
                );
                let mut stmt = conn.prepare(&sql)?;
                let found = stmt
                    .query_map(params![team, asset], LinkRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(found)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(LinkRow::into_domain).collect())
    }

    async fn dangling_locally(&self, team_id: TeamScopedId) -> Result<Vec<AssetLink>, DomainError> {
        let team = *team_id.as_uuid();
        let rows: Vec<LinkRow> = self
            .isle
            .call(move |conn| {
                // The join the schema does not enforce. A trashed Asset
                // is deliberately *not* dangling: `asset.trashed_at`
                // marks something the local plane can still restore,
                // and a row pointing at it still corresponds to
                // something. Gone means gone from the table.
                let sql = format!(
                    "SELECT {} FROM team_asset_link AS link
                      WHERE link.team_id = ?1
                        AND NOT EXISTS (
                            SELECT 1 FROM asset WHERE asset.id = link.local_asset_id
                        )
                      ORDER BY link.pushed_at, link.line_id, link.entry_id",
                    LinkRow::COLUMNS
                        .split(", ")
                        .map(|column| format!("link.{column}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                let mut stmt = conn.prepare(&sql)?;
                let found = stmt
                    .query_map(params![team], LinkRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(found)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(LinkRow::into_domain).collect())
    }

    async fn reap(&self, keys: &[AssetLinkKey]) -> Result<u64, DomainError> {
        if keys.is_empty() {
            return Ok(0);
        }
        let keys: Vec<(Uuid, Uuid, Uuid)> = keys
            .iter()
            .map(|key| {
                (
                    *key.team_id.as_uuid(),
                    *key.line_id.as_uuid(),
                    *key.entry_id.as_uuid(),
                )
            })
            .collect();
        let removed = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut removed = 0u64;
                {
                    // One statement, one table, keyed exactly. Nothing
                    // in a reap may reach an Asset, a mark or anything
                    // a team holds — the relation tidying itself up
                    // must not be a path by which either end loses
                    // something (#148 decision 9).
                    let mut stmt = tx.prepare(
                        "DELETE FROM team_asset_link
                          WHERE team_id = ?1 AND line_id = ?2 AND entry_id = ?3",
                    )?;
                    for (team_id, line_id, entry_id) in keys {
                        removed += stmt.execute(params![team_id, line_id, entry_id])? as u64;
                    }
                }
                tx.commit()?;
                Ok(removed)
            })
            .await
            .map_err(infra_err)?;
        Ok(removed)
    }
}
