//! SQLite adapter for the [`SessionRepository`] port.
//!
//! session-model v2 (shape updated by asset-model v4): a Session is a
//! **composite Asset** (`asset` row with `role = 'collection'`,
//! `modality` NULL), not a row in the legacy `session`
//! table. Its members are the assets pointing at it via
//! `asset.container_id`. This adapter therefore reads/writes the
//! `asset` table and projects composite rows into the [`Session`]
//! domain shape, deriving the aggregates
//! (`message_count` / `started_at_ms` / `ended_at_ms`) at query time
//! from the members — so there is no cached-count drift (the class of
//! bug the old stored `session.message_count` column produced).
//!
//! Metadata mapping composite Asset ↔ Session: `title` ↔ `title`,
//! `register_note` ↔ `note`, `cover` ↔ `cover_hint`, `external_key` ↔
//! `external_key`. The composite's own `occurred_at` seeds the time
//! window when it has no members yet.
//!
//! The delete guard (`delete_if_empty`) is implemented server-side
//! (SQLite `COUNT` over `container_id` inside the same isle closure
//! that issues the composite DELETE) so a race between an
//! `AssetRepository::save` writing `container_id = <id>` and a caller
//! trying to delete the composite cannot slip an orphan through — both
//! writes hit the same writer isle serially.
//!
//! `create` is single-valued by the same mechanism. It is a
//! find-or-create — the lookup on `(persona_id, external_key)` and the
//! composite `INSERT` run inside **one** isle closure, so no second
//! writer can interleave between them. Until V62 the atomicity was
//! borrowed from a UNIQUE index instead: `create` inserted blindly, read
//! the violation back out of the error text, and the caller re-queried.
//! That index is gone — `external_key` is a Prop, an external record
//! legitimately arrives twice, and ids from two platforms collide — so
//! the serialisation point had to become explicit. It always was the thing
//! doing the work; the constraint was only where it happened to live.

use asterism_core::domain::repository::SessionRepository;
use asterism_core::domain::session::{Session, SessionMetadata, SessionMetadataPatch};
use asterism_core::domain::value::{ExternalSessionKey, PersonaId, SessionId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;
use crate::sqlite::repo::asset::MEMBER_POPULATION;

/// Primitive row built inside the isle closure — a composite Asset
/// (`role = 'collection'`) projected into the Session shape with its
/// member aggregates derived.
struct SessionRow {
    id: Uuid,
    persona_id: Uuid,
    external_key: String,
    title: Option<String>,
    note: Option<String>,
    cover_hint: Option<String>,
    started_at_ms: i64,
    ended_at_ms: i64,
    message_count: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl SessionRow {
    /// Projects a composite Asset (aliased `a`) into the Session
    /// columns, deriving the member aggregates via correlated
    /// subqueries over `asset.container_id`. The empty-composite
    /// fallback for the time window is the composite's own
    /// `occurred_at` (seeded from the run start). Column order matches
    /// [`from_row`](Self::from_row).
    ///
    /// The aggregates count [`MEMBER_POPULATION`], which is what makes
    /// `GET /sessions/{id}` agree with the Sessions listing. Until this
    /// carried the predicate the two disagreed by construction: the
    /// listing (`repo::asset::list_sessions`) filtered its members and
    /// this did not, so one session had two `message_count`s depending
    /// on which route the caller took, and `rename` / `PATCH` handed
    /// back the larger one right after the list had shown the smaller.
    /// Making the number drop here is not a change of behaviour so much
    /// as the end of a disagreement.
    ///
    /// A function rather than a `const` for the reason
    /// [`CardRow::columns`](crate::sqlite::repo::asset) is: a `const
    /// &str` cannot be concatenated into another one, and re-typing the
    /// predicate is what produced the disagreement in the first place.
    fn select() -> String {
        format!(
            "\
        SELECT a.id, a.persona_id, a.external_key, a.title, a.register_note, a.cover, \
               COALESCE((SELECT MIN(m.occurred_at) FROM asset m WHERE m.container_id = a.id AND {MEMBER_POPULATION}), a.occurred_at) AS started_at_ms, \
               COALESCE((SELECT MAX(m.occurred_at) FROM asset m WHERE m.container_id = a.id AND {MEMBER_POPULATION}), a.occurred_at) AS ended_at_ms, \
               (SELECT COUNT(*) FROM asset m WHERE m.container_id = a.id AND {MEMBER_POPULATION}) AS message_count, \
               a.created_at, a.updated_at \
          FROM asset a"
        )
    }

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            external_key: row.get(2)?,
            title: row.get(3)?,
            note: row.get(4)?,
            cover_hint: row.get(5)?,
            started_at_ms: row.get(6)?,
            ended_at_ms: row.get(7)?,
            message_count: row.get(8)?,
            created_at_ms: row.get(9)?,
            updated_at_ms: row.get(10)?,
        })
    }

    fn into_domain(self) -> Result<Session, DomainError> {
        Ok(Session {
            id: SessionId::new(self.id.to_string())?,
            persona_id: PersonaId::from_uuid(self.persona_id),
            external_key: ExternalSessionKey::new(self.external_key)?,
            metadata: SessionMetadata {
                title: self.title,
                note: self.note,
                cover_hint: self.cover_hint,
            },
            started_at_ms: self.started_at_ms,
            ended_at_ms: self.ended_at_ms,
            message_count: u64::try_from(self.message_count.max(0)).unwrap_or(0),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        })
    }
}

/// SQLite adapter for [`SessionRepository`] (uses a writer isle).
#[derive(Clone)]
pub struct SqliteSessionRepository {
    isle: AsyncIsle,
}

impl SqliteSessionRepository {
    /// Wraps a writer `AsyncIsle` (schema / pragma initialisation is
    /// done by `sqlite::open`).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// Parses a `SessionId` (hyphenated UUID text) into the 16-byte
    /// BLOB used by `asset.id`. The composite Asset reuses the
    /// Session's UUID as its primary key, so this is the bridge between
    /// the text-shaped domain id and the BLOB column.
    fn id_bytes(id: &SessionId) -> Result<Vec<u8>, DomainError> {
        Uuid::parse_str(id.as_str())
            .map(|u| u.as_bytes().to_vec())
            .map_err(|e| {
                DomainError::Validation(format!("invalid session id {:?}: {e}", id.as_str()))
            })
    }

    async fn fetch_row(&self, id_bytes: Vec<u8>) -> Result<Option<SessionRow>, DomainError> {
        self.isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "{} WHERE a.id = ?1 AND a.role = 'collection'",
                        SessionRow::select()
                    ),
                    params![id_bytes],
                    SessionRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)
    }
}

#[async_trait]
impl SessionRepository for SqliteSessionRepository {
    async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, DomainError> {
        let bytes = Self::id_bytes(id)?;
        let row = self.fetch_row(bytes).await?;
        row.map(SessionRow::into_domain).transpose()
    }

    async fn find_by_external_key(
        &self,
        persona_id: &PersonaId,
        external_key: &ExternalSessionKey,
    ) -> Result<Option<Session>, DomainError> {
        let persona_uuid = *persona_id.as_uuid();
        let key = external_key.as_str().to_string();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "{} WHERE a.persona_id = ?1 AND a.external_key = ?2 \
                           AND a.role = 'collection'",
                        SessionRow::select()
                    ),
                    params![persona_uuid, key],
                    SessionRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(SessionRow::into_domain).transpose()
    }

    async fn create(&self, session: &Session) -> Result<Session, DomainError> {
        let id_bytes = Self::id_bytes(&session.id)?;
        // A composite has no bytes anywhere, so its locator is a name
        // rather than an address, and the Session's own UUID is the name
        // it already has. It is not minted to keep an index happy — that
        // was the reason until V61 demoted the Source pair to a lookup,
        // and it is gone. The column is NOT NULL and a Source value is a
        // Prop every row carries, so what stands here is the plain
        // answer to "where is it": nowhere on disk, under this name.
        let locator = session.id.as_str().to_string();
        let persona_id = *session.persona_id.as_uuid();
        let external_key = session.external_key.as_str().to_string();
        let title = session.metadata.title.clone();
        let note = session.metadata.note.clone();
        let cover_hint = session.metadata.cover_hint.clone();
        // The composite's own occurred_at seeds the run start for the
        // empty (no-member) window; real aggregates derive on read.
        let occurred = session.started_at_ms;
        let created_at_ms = session.created_at_ms;
        let updated_at_ms = session.updated_at_ms;
        let key_for_lookup = external_key.clone();
        // Lookup and insert in **one** closure. The writer isle
        // serialises its callers, so select-then-insert is atomic here
        // without any index asserting it — the same mechanism the delete
        // guard below uses to keep a member from slipping past its
        // COUNT. `idx_asset_external_key` is a plain index from V62
        // onwards and refuses nothing; what makes this single-valued is
        // that no other writer can interleave between the two
        // statements.
        let outcome = self
            .isle
            .call(move |conn| {
                if let Some(row) = conn
                    .query_row(
                        &format!(
                            "{} WHERE a.persona_id = ?1 AND a.external_key = ?2 \
                               AND a.role = 'collection'",
                            SessionRow::select()
                        ),
                        params![persona_id, key_for_lookup],
                        SessionRow::from_row,
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?
                {
                    return Ok(Some(row));
                }
                conn.execute(
                    // Two axes, both set. `role = 'collection'` is the
                    // structure: this row is a container and never
                    // carries a `material` of its own. `modality =
                    // 'session'` is the meaning: what it holds is a run
                    // of exchanges. Leaving the second one NULL (as v4
                    // did, on the grounds that "is a container" is not a
                    // classification) was the mistake — it is not, but
                    // "is a conversation" is, and without it the row had
                    // no badge, no facet and no name (V42).
                    "INSERT INTO asset \
                         (id, persona_id, source_kind, source_locator, occurred_at, \
                          title, cover, register_note, external_key, created_at, updated_at, \
                          role, modality) \
                     VALUES (?1, ?2, 'session', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
                             'collection', 'session')",
                    params![
                        id_bytes,
                        persona_id,
                        locator,
                        occurred,
                        title,
                        cover_hint,
                        note,
                        external_key,
                        created_at_ms,
                        updated_at_ms,
                    ],
                )?;
                Ok(None)
            })
            .await
            .map_err(infra_err)?;
        match outcome {
            Some(row) => row.into_domain(),
            None => Ok(session.clone()),
        }
    }

    async fn update_metadata(
        &self,
        id: &SessionId,
        patch: &SessionMetadataPatch,
        now: DateTime<Utc>,
    ) -> Result<Session, DomainError> {
        let id_bytes = Self::id_bytes(id)?;
        let id_for_update = id_bytes.clone();
        let id_for_err = id.as_str().to_string();
        let title = patch.title.clone();
        let note = patch.note.clone();
        let cover_hint = patch.cover_hint.clone();
        let now_ms = now.timestamp_millis();
        // COALESCE(?, col) — NULL parameters preserve the current value,
        // non-NULL overwrite (the SessionMetadataPatch partial-update
        // contract). Metadata maps title↔title, note↔register_note,
        // cover_hint↔cover on the composite Asset.
        let updated = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset SET \
                         title = COALESCE(?2, title), \
                         register_note = COALESCE(?3, register_note), \
                         cover = COALESCE(?4, cover), \
                         updated_at = ?5 \
                     WHERE id = ?1 AND role = 'collection'",
                    params![id_for_update, title, note, cover_hint, now_ms],
                )
            })
            .await
            .map_err(infra_err)?;
        if updated == 0 {
            return Err(DomainError::not_found("session", &id_for_err));
        }
        let row = self.fetch_row(id_bytes).await?;
        row.expect("composite row was updated one tick ago")
            .into_domain()
    }

    async fn rename(
        &self,
        id: &SessionId,
        new_title: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Session, DomainError> {
        let id_bytes = Self::id_bytes(id)?;
        let id_for_update = id_bytes.clone();
        let id_for_err = id.as_str().to_string();
        let title = new_title;
        let now_ms = now.timestamp_millis();
        // Unconditional assignment (NULL clears the title): rename is
        // the canonical "back to untitled" path, deliberately not
        // COALESCE.
        let updated = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset SET title = ?2, updated_at = ?3 \
                     WHERE id = ?1 AND role = 'collection'",
                    params![id_for_update, title, now_ms],
                )
            })
            .await
            .map_err(infra_err)?;
        if updated == 0 {
            return Err(DomainError::not_found("session", &id_for_err));
        }
        let row = self.fetch_row(id_bytes).await?;
        row.expect("composite row was updated one tick ago")
            .into_domain()
    }

    async fn delete_if_empty(&self, id: &SessionId) -> Result<(), DomainError> {
        let id_bytes = Self::id_bytes(id)?;
        let id_for_err = id.as_str().to_string();
        // Both the guard COUNT (over container_id members) and the
        // composite DELETE run inside the same isle closure, so a
        // concurrent `AssetRepository::save` writing container_id
        // cannot slip a member past the guard on the same writer
        // connection.
        //
        // The COUNT deliberately has no `trashed_at` filter: a trashed
        // member still points here, so deleting the composite would
        // orphan it, and it would come back from the trash detached from
        // its conversation. The Sessions listing answers the
        // display-side question and does filter — the split below turns
        // that asymmetry into an explanation instead of a bare refusal.
        let outcome = self
            .isle
            .call(move |conn| {
                let attached: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM asset WHERE container_id = ?1",
                    params![id_bytes],
                    |row| row.get(0),
                )?;
                if attached > 0 {
                    // Split out the trashed share so the refusal can say
                    // *why* a Session that looks empty will not go: the
                    // Sessions view counts live members only, so a Session
                    // whose every message is in the trash shows 0 and
                    // then refuses to be deleted.
                    let trashed: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM asset \
                         WHERE container_id = ?1 AND trashed_at IS NOT NULL",
                        params![id_bytes],
                        |row| row.get(0),
                    )?;
                    return Ok(DeleteOutcome::HasAssets(attached, trashed));
                }
                let deleted = conn.execute(
                    "DELETE FROM asset WHERE id = ?1 AND role = 'collection'",
                    params![id_bytes],
                )?;
                Ok(if deleted == 0 {
                    DeleteOutcome::NotFound
                } else {
                    DeleteOutcome::Deleted
                })
            })
            .await
            .map_err(infra_err)?;
        match outcome {
            DeleteOutcome::Deleted => Ok(()),
            DeleteOutcome::NotFound => Err(DomainError::not_found("session", &id_for_err)),
            DeleteOutcome::HasAssets(count, trashed) if trashed == count => {
                Err(DomainError::blocked(format!(
                    "session {id_for_err} looks empty because all {count} of its \
                     asset(s) are in the trash; restore or purge them first"
                )))
            }
            DeleteOutcome::HasAssets(count, 0) => Err(DomainError::blocked(format!(
                "session {id_for_err} still has {count} attached asset(s); \
                 detach them first"
            ))),
            DeleteOutcome::HasAssets(count, trashed) => Err(DomainError::blocked(format!(
                "session {id_for_err} still has {count} attached asset(s) \
                 ({trashed} of them in the trash); detach them first, and \
                 restore or purge the trashed ones"
            ))),
        }
    }

    async fn list_by_persona(&self, persona_id: &PersonaId) -> Result<Vec<Session>, DomainError> {
        let persona_uuid = *persona_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "{} WHERE a.persona_id = ?1 AND a.role = 'collection' \
                       ORDER BY started_at_ms DESC, a.id",
                    SessionRow::select()
                ))?;
                stmt.query_map(params![persona_uuid], SessionRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(SessionRow::into_domain).collect()
    }
}

/// Delete-guard outcome so the isle closure can distinguish "row
/// missing" from "row deleted" from "row still has members" without
/// leaking three separate `Result` shapes back through `call`.
enum DeleteOutcome {
    Deleted,
    NotFound,
    /// `(attached, of_which_trashed)` — the split is what lets the
    /// refusal explain a Session the UI shows as empty.
    HasAssets(i64, i64),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::error::ConflictKind;

    async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
        let id = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, 'P', 0, 0, 0, 0)",
                params![id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        PersonaId::from_uuid(id)
    }

    /// Attaches a dialogue member to a composite via container_id
    /// (mirrors what ingest does at runtime). Returns the member id.
    async fn attach_member(
        isle: &AsyncIsle,
        persona: PersonaId,
        composite: &SessionId,
        occurred: i64,
    ) -> Uuid {
        let member = Uuid::now_v7();
        let persona_bytes = *persona.as_uuid();
        let composite_bytes = SqliteSessionRepository::id_bytes(composite).unwrap();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, occurred_at, container_id, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, 'dialogue', ?4, ?5, 0, 0)",
                params![
                    member,
                    persona_bytes,
                    format!("m-{member}.md"),
                    occurred,
                    composite_bytes,
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        member
    }

    fn key(v: &str) -> ExternalSessionKey {
        ExternalSessionKey::new(v).unwrap()
    }

    fn fresh_session_id() -> SessionId {
        SessionId::new(Uuid::now_v7().to_string()).unwrap()
    }

    /// A composite with no members yet: its derived aggregates are
    /// `started == ended == occurred_at` and `message_count == 0`, so
    /// seeding with `(t, t, 0)` makes the create→find round-trip an
    /// exact identity.
    fn seed_session(persona: PersonaId, external: &str, now: DateTime<Utc>) -> Session {
        Session::new(fresh_session_id(), persona, key(external), 10, 10, 0, now).unwrap()
    }

    #[tokio::test]
    async fn create_find_round_trips_by_id_and_external_key() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();
        let session = seed_session(persona, "cc.session.42", now);

        repo.create(&session).await.unwrap();

        let by_id = repo.find_by_id(&session.id).await.unwrap().unwrap();
        assert_eq!(by_id, session);

        let by_key = repo
            .find_by_external_key(&persona, &session.external_key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_key, session);

        assert!(
            repo.find_by_id(&fresh_session_id())
                .await
                .unwrap()
                .is_none(),
            "unknown id yields None (not an error)"
        );
        assert!(
            repo.find_by_external_key(&persona, &key("nope"))
                .await
                .unwrap()
                .is_none()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn aggregates_derive_live_from_members() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let session = seed_session(persona, "cc.session.live", Utc::now());
        repo.create(&session).await.unwrap();

        // 0 members → count 0.
        assert_eq!(
            repo.find_by_id(&session.id)
                .await
                .unwrap()
                .unwrap()
                .message_count,
            0
        );

        // Attach two members with distinct occurred_at → count 2,
        // window derives from the members (no cached column to drift).
        attach_member(&isle, persona, &session.id, 100).await;
        attach_member(&isle, persona, &session.id, 300).await;

        let loaded = repo.find_by_id(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.message_count, 2);
        assert_eq!(loaded.started_at_ms, 100, "MIN(member.occurred_at)");
        assert_eq!(loaded.ended_at_ms, 300, "MAX(member.occurred_at)");

        driver.shutdown().await.unwrap();
    }

    /// **`create` is single-valued, and nothing in the schema makes it
    /// so.** Since V62 `idx_asset_external_key` refuses nothing, so a
    /// second create with one key would insert a second composite unless
    /// the adapter looks first — inside the same isle closure.
    ///
    /// The fixture disagrees with the default in both directions: the
    /// second call carries a *different* `SessionId` and a *different*
    /// seed window, so an implementation that inserted blindly would
    /// leave two rows, and one that overwrote would move the window. The
    /// assertion is that neither happened and the id handed back is the
    /// first one.
    #[tokio::test]
    async fn create_is_find_or_create_and_stays_single_valued() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();
        let first = seed_session(persona, "cc.session.dup", now);
        let landed = repo.create(&first).await.unwrap();
        assert_eq!(
            landed.id, first.id,
            "a fresh key mints the row it was given"
        );

        // Same key, another id, another window — a caller that lost the
        // race and is about to be handed the row that won.
        let second = Session::new(
            fresh_session_id(),
            persona,
            key("cc.session.dup"),
            999,
            999,
            0,
            now,
        )
        .unwrap();
        let resolved = repo.create(&second).await.unwrap();
        assert_eq!(
            resolved.id, first.id,
            "the row that holds the key comes back — not the one this call proposed"
        );
        assert_eq!(
            resolved.started_at_ms, first.started_at_ms,
            "and it comes back as it stands: a find-or-create that hands the loser's \
             window back would have rewritten a stored value on a read-shaped call"
        );

        let composites: i64 = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM asset WHERE role = 'collection' AND external_key = ?1",
                    params!["cc.session.dup"],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(composites, 1, "two creates, one row");

        driver.shutdown().await.unwrap();
    }

    /// The same, driven concurrently. Two `create` calls with one key
    /// are issued together and both are awaited; the isle serialises
    /// them, so whichever runs second finds the row the first inserted.
    ///
    /// Without the lookup and the insert sharing one closure this is the
    /// interleaving that produces two rows, and since V62 no index would
    /// stop it.
    #[tokio::test]
    async fn concurrent_creates_with_one_key_converge_on_one_row() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();

        let a = seed_session(persona, "cc.session.race", now);
        let b = seed_session(persona, "cc.session.race", now);
        assert_ne!(a.id, b.id, "two callers, two proposed ids");

        let (ra, rb) = tokio::join!(repo.create(&a), repo.create(&b));
        let ra = ra.unwrap();
        let rb = rb.unwrap();
        assert_eq!(
            ra.id, rb.id,
            "both callers must be handed the row that landed"
        );

        let composites: i64 = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM asset WHERE role = 'collection' AND external_key = ?1",
                    params!["cc.session.race"],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(composites, 1, "a race must not leave a second composite");

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn update_metadata_stamps_now_and_leaves_aggregates_untouched() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();
        let session = seed_session(persona, "cc.session.metadata", now);
        repo.create(&session).await.unwrap();

        let stamp_one = Utc::now();
        let full = repo
            .update_metadata(
                &session.id,
                &SessionMetadataPatch {
                    title: Some("Weekly planning".into()),
                    note: Some("Kick-off notes".into()),
                    cover_hint: Some("cover hint one".into()),
                },
                stamp_one,
            )
            .await
            .unwrap();
        assert_eq!(full.metadata.title.as_deref(), Some("Weekly planning"));
        assert_eq!(full.metadata.note.as_deref(), Some("Kick-off notes"));
        assert_eq!(full.metadata.cover_hint.as_deref(), Some("cover hint one"));

        // Partial patch: overwrite title only; note + cover_hint stay
        // (the COALESCE contract).
        let stamp_two = Utc::now();
        let partial = repo
            .update_metadata(
                &session.id,
                &SessionMetadataPatch {
                    title: Some("Renamed run".into()),
                    note: None,
                    cover_hint: None,
                },
                stamp_two,
            )
            .await
            .unwrap();
        assert_eq!(partial.metadata.title.as_deref(), Some("Renamed run"));
        assert_eq!(
            partial.metadata.note.as_deref(),
            Some("Kick-off notes"),
            "None on patch.note must preserve the existing value"
        );
        assert_eq!(
            partial.metadata.cover_hint.as_deref(),
            Some("cover hint one"),
            "None on patch.cover_hint must preserve the existing value"
        );
        // Aggregates derive from members (none attached) → unchanged by
        // a metadata edit.
        assert_eq!(partial.started_at_ms, session.started_at_ms);
        assert_eq!(partial.ended_at_ms, session.ended_at_ms);
        assert_eq!(partial.message_count, 0);
        assert_eq!(partial.updated_at_ms, stamp_two.timestamp_millis());

        let err = repo
            .update_metadata(
                &fresh_session_id(),
                &SessionMetadataPatch::default(),
                stamp_two,
            )
            .await;
        assert!(matches!(err, Err(DomainError::NotFound { .. })));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rename_can_clear_title_back_to_null() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();
        let session = seed_session(persona, "cc.session.rename", now);
        repo.create(&session).await.unwrap();

        repo.update_metadata(
            &session.id,
            &SessionMetadataPatch {
                title: Some("Draft title".into()),
                note: Some("keep me".into()),
                cover_hint: None,
            },
            Utc::now(),
        )
        .await
        .unwrap();

        let stamp = Utc::now();
        let renamed = repo
            .rename(&session.id, Some("Final title".into()), stamp)
            .await
            .unwrap();
        assert_eq!(renamed.metadata.title.as_deref(), Some("Final title"));
        assert_eq!(renamed.metadata.note.as_deref(), Some("keep me"));
        assert_eq!(renamed.updated_at_ms, stamp.timestamp_millis());

        let stamp = Utc::now();
        let cleared = repo.rename(&session.id, None, stamp).await.unwrap();
        assert_eq!(cleared.metadata.title, None);
        assert_eq!(cleared.metadata.note.as_deref(), Some("keep me"));

        let err = repo.rename(&fresh_session_id(), None, Utc::now()).await;
        assert!(matches!(err, Err(DomainError::NotFound { .. })));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_if_empty_rejects_when_member_still_attached() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let now = Utc::now();
        let session = seed_session(persona, "cc.session.attached", now);
        repo.create(&session).await.unwrap();

        // Attach a member via container_id (what ingest does at runtime).
        attach_member(&isle, persona, &session.id, 50).await;

        let err = repo.delete_if_empty(&session.id).await;
        assert!(
            matches!(
                err,
                Err(DomainError::Conflict {
                    kind: ConflictKind::Blocked,
                    ..
                })
            ),
            "delete must be refused while a member points at the composite"
        );
        assert!(repo.find_by_id(&session.id).await.unwrap().is_some());

        // The awkward case: every member is in the trash, so the
        // Sessions view shows `message_count = 0` and the Session still
        // refuses to go. The refusal has to say why, or the user is told
        // "not empty" about a thing the UI draws as empty.
        let trash_bytes = SqliteSessionRepository::id_bytes(&session.id).unwrap();
        isle.call(move |conn| {
            conn.execute(
                "UPDATE asset SET trashed_at = 1000 WHERE container_id = ?1",
                params![trash_bytes],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        match repo.delete_if_empty(&session.id).await {
            Err(DomainError::Conflict { message, .. }) => assert!(
                message.contains("looks empty") && message.contains("trash"),
                "the refusal must name the trash, got: {message}"
            ),
            other => panic!("expected a Conflict naming the trash, got {other:?}"),
        }

        // Detach the member — now the composite deletes cleanly.
        let composite_bytes = SqliteSessionRepository::id_bytes(&session.id).unwrap();
        isle.call(move |conn| {
            conn.execute(
                "UPDATE asset SET container_id = NULL WHERE container_id = ?1",
                params![composite_bytes],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        repo.delete_if_empty(&session.id).await.unwrap();
        assert!(repo.find_by_id(&session.id).await.unwrap().is_none());

        // Second delete of the same id → NotFound.
        assert!(matches!(
            repo.delete_if_empty(&session.id).await,
            Err(DomainError::NotFound { .. })
        ));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn list_by_persona_returns_freshest_first() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let other = seed_persona(&isle).await;
        let now = Utc::now();

        // Composites seed their own occurred_at from started_at_ms, so
        // with no members the derived started_at_ms == occurred_at and
        // ordering is deterministic.
        let early =
            Session::new(fresh_session_id(), persona, key("early"), 100, 100, 0, now).unwrap();
        let late =
            Session::new(fresh_session_id(), persona, key("late"), 1000, 1000, 0, now).unwrap();
        let stranger =
            Session::new(fresh_session_id(), other, key("stranger"), 500, 500, 0, now).unwrap();

        repo.create(&early).await.unwrap();
        repo.create(&late).await.unwrap();
        repo.create(&stranger).await.unwrap();

        let list = repo.list_by_persona(&persona).await.unwrap();
        assert_eq!(list.len(), 2, "other-persona composite must not leak");
        assert_eq!(list[0].id, late.id, "freshest started_at first");
        assert_eq!(list[1].id, early.id);

        driver.shutdown().await.unwrap();
    }
}
