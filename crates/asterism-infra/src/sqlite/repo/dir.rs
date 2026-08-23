//! SQLite adapter for `DirRepository`.
//!
//! `dir` is the sidebar organisation tree (see
//! `asterism_core::domain::dir` for the axis rationale). All tree
//! semantics that need recursion — the move-cycle guard — run as a
//! recursive CTE inside the writer isle, so check-then-write stays
//! race-free under the isle's serialized calls.

use asterism_core::domain::dir::Dir;
use asterism_core::domain::repository::DirRepository;
use asterism_core::domain::value::{DirId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::fault::StoreFault;
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// Raw row shape used inside the isle closure.
struct DirRow {
    id: Uuid,
    persona_id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    position: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl DirRow {
    const COLUMNS: &'static str =
        "id, persona_id, parent_id, name, position, created_at, updated_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            parent_id: row.get(2)?,
            name: row.get(3)?,
            position: row.get(4)?,
            created_at_ms: row.get(5)?,
            updated_at_ms: row.get(6)?,
        })
    }

    fn into_domain(self) -> Result<Dir, DomainError> {
        Ok(Dir {
            id: DirId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            parent_id: self.parent_id.map(DirId::from_uuid),
            name: self.name,
            position: self.position,
            created_at: ms_to_datetime(self.created_at_ms)?,
            updated_at: ms_to_datetime(self.updated_at_ms)?,
        })
    }
}

/// Maps a unique-index violation to a sibling-name `Conflict`,
/// passing every other failure through as `Infra`.
fn map_name_conflict(err: rusqlite_isle::IsleError, name: &str) -> DomainError {
    let msg = err.to_string();
    if msg.contains("UNIQUE") || msg.contains("unique") {
        StoreFault::taken("a dir name at this level", format!("{name:?}")).into()
    } else {
        infra_err(err)
    }
}

/// SQLite adapter (writer isle).
#[derive(Clone)]
pub struct SqliteDirRepository {
    isle: AsyncIsle,
}

impl SqliteDirRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl DirRepository for SqliteDirRepository {
    async fn create(
        &self,
        persona_id: PersonaId,
        parent_id: Option<DirId>,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Dir, DomainError> {
        // Domain constructor performs the name-non-empty check.
        let dir = Dir::new(persona_id, parent_id, name, now)?;
        let uuid = *dir.id.as_uuid();
        let persona_uuid = *dir.persona_id.as_uuid();
        let parent_uuid = dir.parent_id.map(|p| *p.as_uuid());
        let name_owned = dir.name.clone();
        let now_ms = datetime_to_ms(&now);
        // 0 = ok, 1 = parent missing / different persona.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                if let Some(parent) = parent_uuid {
                    let parent_ok: bool = conn.query_row(
                        "SELECT EXISTS (
                             SELECT 1 FROM dir
                              WHERE id = ?1 AND persona_id = ?2)",
                        params![parent, persona_uuid],
                        |row| row.get(0),
                    )?;
                    if !parent_ok {
                        return Ok(1);
                    }
                }
                conn.execute(
                    "INSERT INTO dir \
                        (id, persona_id, parent_id, name, position, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
                    params![uuid, persona_uuid, parent_uuid, name_owned, now_ms],
                )?;
                Ok(0)
            })
            .await
            .map_err(|err| map_name_conflict(err, &dir.name))?;
        if verdict == 1 {
            return Err(DomainError::Validation(
                "parent dir does not exist for this persona".into(),
            ));
        }
        Ok(dir)
    }

    async fn rename(
        &self,
        id: &DirId,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Dir, DomainError> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err(DomainError::Validation("Dir name must not be empty".into()));
        }
        let uuid = *id.as_uuid();
        let now_ms = datetime_to_ms(&now);
        let name_for_sql = trimmed.clone();
        let row = self
            .isle
            .call(move |conn| {
                let updated = conn.execute(
                    "UPDATE dir SET name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![name_for_sql, now_ms, uuid],
                )?;
                if updated == 0 {
                    return Ok(None);
                }
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM dir WHERE id = ?1",
                    DirRow::COLUMNS
                ))?;
                let row = stmt.query_row(params![uuid], DirRow::from_row)?;
                Ok(Some(row))
            })
            .await
            .map_err(|err| map_name_conflict(err, &trimmed))?;
        match row {
            Some(row) => row.into_domain(),
            None => Err(DomainError::not_found("dir", id)),
        }
    }

    async fn move_to(
        &self,
        id: &DirId,
        new_parent: Option<&DirId>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let parent_uuid = new_parent.map(|p| *p.as_uuid());
        if parent_uuid == Some(uuid) {
            return Err(StoreFault::Impossible("a dir cannot be moved into itself".into()).into());
        }
        let now_ms = datetime_to_ms(&now);
        // 0 = ok, 1 = parent missing / different persona, 2 = cycle,
        // 3 = missing dir.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                if let Some(parent) = parent_uuid {
                    let parent_ok: bool = conn.query_row(
                        "SELECT EXISTS (
                             SELECT 1 FROM dir p
                              JOIN dir me ON me.id = ?1
                             WHERE p.id = ?2
                               AND p.persona_id = me.persona_id)",
                        params![uuid, parent],
                        |row| row.get(0),
                    )?;
                    if !parent_ok {
                        return Ok(1);
                    }
                    // Cycle guard: the new parent must not sit inside
                    // the moved dir's own subtree.
                    let cycle: bool = conn.query_row(
                        "WITH RECURSIVE sub(id) AS (
                             SELECT ?1
                             UNION
                             SELECT d.id FROM dir d JOIN sub ON d.parent_id = sub.id
                         )
                         SELECT EXISTS (SELECT 1 FROM sub WHERE id = ?2)",
                        params![uuid, parent],
                        |row| row.get(0),
                    )?;
                    if cycle {
                        return Ok(2);
                    }
                }
                let updated = conn.execute(
                    "UPDATE dir SET parent_id = ?1, updated_at = ?2 WHERE id = ?3",
                    params![parent_uuid, now_ms, uuid],
                )?;
                Ok(if updated == 0 { 3 } else { 0 })
            })
            .await
            .map_err(|err| map_name_conflict(err, "(moved dir)"))?;
        match verdict {
            1 => Err(DomainError::Validation(
                "target parent dir does not exist for this persona".into(),
            )),
            2 => Err(StoreFault::blocked_by(
                "move rejected: the target parent sits inside this dir's own subtree",
                "move it out from under this dir first, and the same move then works",
            )
            .into()),
            3 => Err(DomainError::not_found("dir", id)),
            _ => Ok(()),
        }
    }

    async fn delete(&self, id: &DirId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        // 0 = ok, 1 = non-empty, 2 = missing.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                let occupied: bool = conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM dir    WHERE parent_id = ?1)
                         OR EXISTS (SELECT 1 FROM bucket WHERE dir_id    = ?1)",
                    params![uuid],
                    |row| row.get(0),
                )?;
                if occupied {
                    // The listing hides Groups whose persona is trashed,
                    // so a Dir can look empty and still refuse to go.
                    // Detect that case here to say so, instead of
                    // repeating "move its contents" about contents the
                    // user cannot see.
                    let hidden: bool = conn.query_row(
                        "SELECT EXISTS ( \
                             SELECT 1 FROM bucket \
                             WHERE dir_id = ?1 \
                               AND persona_id IN \
                                   (SELECT id FROM persona WHERE trashed_at IS NOT NULL) \
                         )",
                        params![uuid],
                        |row| row.get(0),
                    )?;
                    return Ok(if hidden { 3 } else { 1 });
                }
                let deleted = conn.execute("DELETE FROM dir WHERE id = ?1", params![uuid])?;
                Ok(if deleted == 0 { 2 } else { 0 })
            })
            .await
            .map_err(infra_err)?;
        match verdict {
            1 => Err(StoreFault::blocked_by(
                "dir is not empty",
                "move or delete its contents first",
            )
            .into()),
            2 => Err(DomainError::not_found("dir", id)),
            3 => Err(StoreFault::blocked_by(
                "dir looks empty because it holds Group(s) belonging to a persona \
                 in the trash",
                "restore or purge that persona first",
            )
            .into()),
            _ => Ok(()),
        }
    }

    async fn list(&self, persona_id: Option<&PersonaId>) -> Result<Vec<Dir>, DomainError> {
        let persona_uuid = persona_id.map(|p| *p.as_uuid());
        let rows = self
            .isle
            .call(move |conn| {
                // A Dir has no trash stamp of its own — trashing a
                // persona stamps its assets, not its organisation tree —
                // so a trashed persona's Dirs can only be excluded here.
                // Left in, they would sit in the sidebar looking like
                // ordinary empty folders right up until the retention
                // sweep destroyed them (same shape as the Sessions and
                // Groups fixes).
                let live_persona =
                    "persona_id IN (SELECT id FROM persona WHERE trashed_at IS NULL)";
                let (sql, params_vec): (String, Vec<rusqlite::types::Value>) = match persona_uuid {
                    Some(uuid) => (
                        format!(
                            "SELECT {} FROM dir WHERE persona_id = ?1 AND {live_persona} \
                                 ORDER BY position, name",
                            DirRow::COLUMNS
                        ),
                        vec![rusqlite::types::Value::Blob(uuid.as_bytes().to_vec())],
                    ),
                    None => (
                        format!(
                            "SELECT {} FROM dir WHERE {live_persona} ORDER BY position, name",
                            DirRow::COLUMNS
                        ),
                        Vec::new(),
                    ),
                };
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params_from_iter(params_vec.iter()),
                        DirRow::from_row,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(DirRow::into_domain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use crate::sqlite::repo::group::SqliteGroupRepository;
    use asterism_core::domain::repository::GroupRepository;
    use asterism_core::error::ConflictKind;

    async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
        let persona = PersonaId::new();
        let uuid = *persona.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, created_at, updated_at)
                 VALUES (?1, 'p', 0, 0)",
                params![uuid],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        persona
    }

    #[tokio::test]
    async fn dir_tree_create_move_cycle_and_delete_guard() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteDirRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();

        let root = repo.create(persona, None, "art".into(), now).await.unwrap();
        let child = repo
            .create(persona, Some(root.id), "2026".into(), now)
            .await
            .unwrap();

        // Sibling name collision at the root level.
        assert!(matches!(
            repo.create(persona, None, "art".into(), now).await,
            Err(DomainError::Conflict {
                kind: ConflictKind::Clashes,
                ..
            })
        ));

        // Moving the root under its own child must be rejected.
        assert!(matches!(
            repo.move_to(&root.id, Some(&child.id), now).await,
            Err(DomainError::Conflict { .. })
        ));

        // Deleting a non-empty dir must be rejected; emptying it
        // first makes the delete pass.
        assert!(matches!(
            repo.delete(&root.id).await,
            Err(DomainError::Conflict {
                kind: ConflictKind::Blocked,
                ..
            })
        ));
        repo.delete(&child.id).await.unwrap();
        repo.delete(&root.id).await.unwrap();
        assert!(repo.list(Some(&persona)).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn group_link_rejects_cycles_and_expands_via_links() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let dirs = SqliteDirRepository::new(isle.clone());
        let groups = SqliteGroupRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();

        let a = groups.create(persona, "a".into(), None, now).await.unwrap();
        let b = groups.create(persona, "b".into(), None, now).await.unwrap();
        let c = groups.create(persona, "c".into(), None, now).await.unwrap();

        groups.link(&a.id, &b.id, now).await.unwrap();
        groups.link(&b.id, &c.id, now).await.unwrap();
        // Idempotent re-link.
        groups.link(&a.id, &b.id, now).await.unwrap();

        // Self-link and transitive cycle are both rejected, and not with
        // the same answer: a group containing itself can never hold, so
        // it is the request that is wrong, while the transitive cycle is
        // another edge standing in the way and goes once that edge does.
        assert!(matches!(
            groups.link(&a.id, &a.id, now).await,
            Err(DomainError::Validation(_))
        ));
        assert!(matches!(
            groups.link(&c.id, &a.id, now).await,
            Err(DomainError::Conflict {
                kind: ConflictKind::Blocked,
                ..
            })
        ));

        let links = groups.links(Some(&persona)).await.unwrap();
        assert_eq!(links.len(), 2);

        // Dir filing on the organisation axis is orthogonal to the
        // link graph: filing `a` under a dir leaves the links alone.
        let d = dirs
            .create(persona, None, "shelf".into(), now)
            .await
            .unwrap();
        groups.set_dir(&a.id, Some(&d.id), now).await.unwrap();
        let listed = groups.list(Some(&persona)).await.unwrap();
        let a_row = listed.iter().find(|g| g.group.id == a.id).unwrap();
        assert_eq!(a_row.group.dir_id, Some(d.id));
        assert_eq!(groups.links(Some(&persona)).await.unwrap().len(), 2);

        driver.shutdown().await.unwrap();
    }
}
