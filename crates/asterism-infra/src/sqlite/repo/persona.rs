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

use crate::fault::StoreFault;
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
        // The same is true of what the forge holds: see `Verdict`.
        let verdict = self
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
                //   dispatch_job → bucket.origin clear → snapshot →
                //   persona.
                //
                // `dispatch_job` leads for its `snapshot_id` alone.
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
                    None => return Ok(Verdict::Gone),
                    Some(true) => return Ok(Verdict::Live),
                    Some(false) => {}
                }

                // The forge holds what it names, and the schema says
                // so with a foreign key from `change_row.content` and
                // `pursuit_op.content`. Without this the delete below
                // still refuses — the cascade reaches `asset` and the
                // key stops it — but it refuses as a foreign-key
                // error, which names a column and tells nobody what to
                // do. Asked here, inside the same transaction as the
                // delete for the reason the live check is, so the
                // answer cannot go stale between saying it and acting
                // on it.
                let held: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM asset a \
                      WHERE a.persona_id = ?1 \
                        AND (EXISTS (SELECT 1 FROM change_row r WHERE r.content = a.id) \
                          OR EXISTS (SELECT 1 FROM pursuit_op o WHERE o.content = a.id))",
                    params![uuid],
                    |r| r.get(0),
                )?;
                if held > 0 {
                    let mut stmt = tx.prepare(
                        "SELECT DISTINCT l.name FROM line l \
                          JOIN pursuit w ON w.line_id = l.id \
                          JOIN pursuit_node n ON n.pursuit_id = w.id \
                          JOIN pursuit_op o ON o.node_id = n.id \
                          JOIN asset a ON a.id = o.content \
                         WHERE a.persona_id = ?1 \
                         UNION \
                        SELECT DISTINCT l.name FROM line l \
                          JOIN change_point p ON p.line_id = l.id \
                          JOIN change_row r ON r.point_id = p.id \
                          JOIN asset a ON a.id = r.content \
                         WHERE a.persona_id = ?1 \
                         ORDER BY 1",
                    )?;
                    let lines: Vec<String> = stmt
                        .query_map(params![uuid], |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    return Ok(Verdict::Held { held, lines });
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
                Ok(Verdict::Gone)
            })
            .await
            .map_err(infra_err)?;
        match verdict {
            Verdict::Gone => Ok(()),
            Verdict::Live => Err(StoreFault::blocked_by(
                format!("persona {id} is still live"),
                "trash it before purging",
            )
            .into()),
            Verdict::Held { held, lines } => Err(StoreFault::blocked_by(
                format!(
                    "persona {id} has {held} asset(s) the forge is holding, on {}. \
                     A line holds what it has ever named, and releases it when the line \
                     itself is dropped — taking an entry off does not, because bringing \
                     it back needs the content to still be there",
                    describe(&lines)
                ),
                // Three steps rather than one, and named as three:
                // a drop is decided against an archived line, and an
                // archived line refuses while work is open against it.
                // "Drop those lines" is one verb for a sequence the
                // caller has to walk, and a remedy that understates
                // what it costs is a remedy that fails once.
                "end the work against those lines, archive them and drop them, \
                 and the same purge then works",
            )
            .into()),
        }
    }
}

/// What the purge found in the way, so the answer can be one value
/// rather than a number the caller has to remember the meaning of.
enum Verdict {
    /// Purged, or never there.
    Gone,
    /// Still live: the trash comes first.
    Live,
    /// The forge is holding assets of this persona.
    Held { held: i64, lines: Vec<String> },
}

/// Names the lines in the way, and stops naming them before the
/// message stops being readable.
fn describe(lines: &[String]) -> String {
    const SHOWN: usize = 3;
    let named: Vec<String> = lines
        .iter()
        .take(SHOWN)
        .map(|name| format!("`{name}`"))
        .collect();
    match lines.len().checked_sub(SHOWN) {
        Some(0) | None => format!("line {}", named.join(", ")),
        Some(rest) => format!("line {} and {rest} more", named.join(", ")),
    }
}

#[cfg(test)]
mod delete_order_tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::repository::PersonaRepository;
    use asterism_core::error::ConflictKind;

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
        let bystander = Uuid::now_v7();
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
            // A bystander persona, so the sweep's scoping is exercised
            // rather than assumed: what it leaves standing has to be
            // somebody else's.
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived,
                                      created_at, updated_at)
                 VALUES (?1, 'q', NULL, 1, 0, 0, 0)",
                params![bystander],
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
            matches!(
                repo.purge(&id).await,
                Err(DomainError::Conflict {
                    kind: ConflictKind::Blocked,
                    ..
                })
            ),
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
            (1, 0, 0, 0, 0),
            "everything of the purged persona swept; the bystander persona left alone"
        );
        driver.shutdown().await.unwrap();
    }
    /// The forge holding an asset stops the purge, and the refusal
    /// says what is holding it.
    ///
    /// Without the guard the delete still refuses — the cascade
    /// reaches `asset` and the foreign key stops it — but it refuses
    /// with a column name and no move. This asserts the message
    /// instead: the count, the line, and what releases it.
    #[tokio::test]
    async fn a_persona_whose_asset_is_on_a_line_is_not_purged_and_is_told_why() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = Uuid::now_v7();
        let asset = Uuid::now_v7();
        let bystander_asset = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived, \
                                      created_at, updated_at) \
                 VALUES (?1, 'p', NULL, 0, 0, 0, 0)",
                params![persona],
            )?;
            for id in [asset, bystander_asset] {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                        modality, labels, occurred_at, created_at, updated_at) \
                     VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[]', 0, 0, 0)",
                    params![id, persona, format!("a-{id}.md")],
                )?;
            }

            // A line with one of them on it. Only the first is held;
            // the second is this persona's too and nothing names it.
            let line = Uuid::now_v7();
            let genesis = Uuid::now_v7();
            let actor = Uuid::now_v7();
            conn.execute(
                "INSERT INTO line \
                     (id, name, strategy, standing, genesis_id, genesis_at, genesis_by, \
                      genesis_kind, created_at, created_by, created_kind, \
                      updated_at, updated_by, updated_kind) \
                 VALUES (?1, 'ROOT', 'by-hand', 'open', ?2, 0, ?3, 'user', \
                         0, ?3, 'user', 0, ?3, 'user')",
                params![line, genesis, actor],
            )?;
            let point = Uuid::now_v7();
            conn.execute(
                "INSERT INTO change_point \
                     (id, line_id, parent_id, from_work, by_node, at, actor_id, actor_kind) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 'user')",
                params![point, line, genesis, Uuid::now_v7(), Uuid::now_v7(), actor],
            )?;
            conn.execute(
                "INSERT INTO change_row (point_id, entry_id, existence, content, name) \
                 VALUES (?1, ?2, 'present', ?3, 'one')",
                params![point, Uuid::now_v7(), asset],
            )
        })
        .await
        .unwrap();

        let repo = SqlitePersonaRepository::new(isle.clone());
        let id = PersonaId::from_uuid(persona);
        repo.trash(&id, chrono::Utc::now()).await.unwrap();

        let refused = repo.purge(&id).await.expect_err("the forge is holding one");
        let DomainError::Conflict { message: said, .. } = &refused else {
            panic!("a held asset is the state fighting back: {refused:?}");
        };
        assert!(
            said.contains("1 asset(s)"),
            "the count is the one that is held, not every asset it owns: {said}"
        );
        assert!(said.contains("`ROOT`"), "and it names the line: {said}");
        assert!(
            said.contains("dropped"),
            "and what releases it, or the refusal leaves nowhere to go: {said}"
        );

        // Nothing was taken on the way to refusing.
        let left: (i64, i64) = isle
            .call(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM persona", [], |r| r.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(left, (1, 2), "a refused purge is not a partial one");

        driver.shutdown().await.unwrap();
    }

    /// And a persona the forge has never heard of purges as it always
    /// did — the guard is a refusal for a state that exists, not a new
    /// hoop.
    #[tokio::test]
    async fn a_persona_the_forge_is_not_holding_still_purges() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived, \
                                      created_at, updated_at) \
                 VALUES (?1, 'p', NULL, 0, 0, 0, 0)",
                params![persona],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', 'a.md', 'dialogue', '[]', 0, 0, 0)",
                params![Uuid::now_v7(), persona],
            )
        })
        .await
        .unwrap();

        let repo = SqlitePersonaRepository::new(isle.clone());
        let id = PersonaId::from_uuid(persona);
        repo.trash(&id, chrono::Utc::now()).await.unwrap();
        repo.purge(&id).await.expect("nothing is holding anything");

        let left: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM persona", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(left, 0);

        driver.shutdown().await.unwrap();
    }
}
