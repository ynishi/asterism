//! `SessionService` — use cases for the Session 1st-class entity.
//!
//! P1b + P2 scope: `get` (single-Session lookup for the detail path),
//! `find_or_create_by_external_key` (the importer's idempotent
//! re-entry point), plus the P2 CRUD trio `rename` /
//! `patch_metadata` / `delete_if_empty` (backing the HTTP CRUD
//! surface). The write methods return the resolved
//! [`SessionDto`] so the caller can echo the persisted state back
//! on the wire without a follow-up `get`.
//!
//! `list_by_persona` was removed: the SessionsView list path goes
//! through `AssetService::list_sessions`, so the service method had no
//! caller on either transport. The repository port of the same name
//! survives it (`SessionRepository::list_by_persona`).
//!
//! Shared as an `Arc` through Tauri state and server contexts, same
//! shape as [`ModalityService`](crate::application::ModalityService).
//!
//! Every write here takes an [`AttributionContext`] it does not persist:
//! the `session` table carries no attribution column, and none is being
//! added (see the [`application`](crate::application) module doc for why
//! the argument is required anyway).

use std::sync::Arc;

use asterism_contract::command::{
    DeleteSessionCommand, PatchSessionMetadataCommand, RenameSessionCommand,
};
use asterism_contract::dto::SessionDto;
use chrono::Utc;
use uuid::Uuid;

use crate::application::mapping::session_to_dto;
use crate::domain::attribution::AttributionContext;
use crate::domain::repository::SessionRepository;
use crate::domain::session::{Session, SessionMetadataPatch};
use crate::domain::value::{ExternalSessionKey, PersonaId, SessionId};
use crate::error::DomainError;

/// Session use-case service. Shared as an `Arc` through Tauri
/// state and server contexts.
pub struct SessionService {
    repo: Arc<dyn SessionRepository>,
}

impl SessionService {
    /// Constructs the service around the Session repository port.
    pub fn new(repo: Arc<dyn SessionRepository>) -> Self {
        Self { repo }
    }

    /// Fetches one Session by surrogate id. Returns `None` when the
    /// id is not registered (used by the future P2 detail handler to
    /// distinguish "unknown id" from a server error).
    pub async fn get(&self, id: &str) -> Result<Option<SessionDto>, DomainError> {
        let sid = SessionId::new(id.to_string())?;
        Ok(self
            .repo
            .find_by_id(&sid)
            .await?
            .as_ref()
            .map(session_to_dto))
    }

    /// Idempotent find-or-create keyed on the importer-supplied
    /// `(persona_id, external_key)` pair. Re-imports converge onto the
    /// same Session row.
    ///
    /// The convergence is the repository's, not a constraint's:
    /// [`SessionRepository::create`](crate::domain::repository::SessionRepository::create)
    /// is itself a find-or-create and returns the row that holds the
    /// key. The pre-check here saves a write round trip on the common
    /// re-import; it is not what makes the result single-valued, and it
    /// need not be — losing the race is answered one frame down.
    ///
    /// Seed timestamps (`seed_started_ms` / `seed_ended_ms`) are
    /// applied only when a new Session is minted — an existing row's
    /// derived aggregates are never touched here (the
    /// `SessionRebuild` reconciliation job owns them). The seed
    /// window must be monotonic; the constructor enforces that.
    ///
    /// Consumed by the P3 importer migration; a unit test in this
    /// file pins the idempotence contract.
    ///
    /// No importer calls this directly yet: the only production caller
    /// is [`AssetService::add`](crate::application::AssetService::add),
    /// which routes `AddAssetCommand::external_session_key` through it,
    /// so an importer reaches it only by filling that field. The
    /// direct-wiring path the doc above describes ("the importer's
    /// idempotent re-entry point") is still unbuilt — recorded here
    /// because the method reads as if it already had a caller of its
    /// own, and it does not.
    pub async fn find_or_create_by_external_key(
        &self,
        persona_id: &PersonaId,
        external_key: &str,
        seed_started_ms: i64,
        seed_ended_ms: i64,
        _attribution: &AttributionContext,
    ) -> Result<SessionDto, DomainError> {
        let key = ExternalSessionKey::new(external_key.to_string())?;
        if let Some(existing) = self.repo.find_by_external_key(persona_id, &key).await? {
            return Ok(session_to_dto(&existing));
        }
        let id = SessionId::new(Uuid::now_v7().to_string())?;
        let session = Session::new(
            id,
            *persona_id,
            key,
            seed_started_ms,
            seed_ended_ms,
            0,
            Utc::now(),
        )?;
        // `create` is itself a find-or-create and resolves the race on
        // its own: the row it returns is the row that holds the key,
        // whether this call minted it or another caller won. The branch
        // that used to sit here read a `Conflict` out of a UNIQUE
        // violation and re-queried; the constraint it read is gone (V62,
        // `external_key` is a Prop) and so is the error it produced.
        let landed = self.repo.create(&session).await?;
        Ok(session_to_dto(&landed))
    }

    /// Renames the Session (title-only write path). `None` on
    /// `command.title` clears the title back to untitled — the sole
    /// path that expresses NULL, per
    /// [`SessionRepository::rename`](crate::domain::repository::SessionRepository::rename).
    /// `note` / `cover_hint` are untouched.
    pub async fn rename(
        &self,
        command: RenameSessionCommand,
        _attribution: &AttributionContext,
    ) -> Result<SessionDto, DomainError> {
        let sid = SessionId::new(command.id)?;
        let session = self.repo.rename(&sid, command.title, Utc::now()).await?;
        Ok(session_to_dto(&session))
    }

    /// Partially updates the Session's metadata (`title` / `note` /
    /// `cover_hint`). Each `None` field on `command` leaves the
    /// existing value intact; `Some(v)` overwrites. To clear `title`
    /// back to NULL call [`rename`](Self::rename) with `title: None`
    /// — this path cannot express "clear" (see
    /// [`SessionMetadataPatch`](crate::domain::session::SessionMetadataPatch)
    /// doc comment).
    pub async fn patch_metadata(
        &self,
        command: PatchSessionMetadataCommand,
        _attribution: &AttributionContext,
    ) -> Result<SessionDto, DomainError> {
        let sid = SessionId::new(command.id)?;
        let patch = SessionMetadataPatch {
            title: command.title,
            note: command.note,
            cover_hint: command.cover_hint,
        };
        let session = self.repo.update_metadata(&sid, &patch, Utc::now()).await?;
        Ok(session_to_dto(&session))
    }

    /// Deletes the Session iff no `asset` row still references its
    /// id. Fails with `Conflict` when the guard trips (asset(s)
    /// still attached) or when the id is not registered — same
    /// contract as
    /// [`SessionRepository::delete_if_empty`](crate::domain::repository::SessionRepository::delete_if_empty).
    /// Mirror of the Modality delete guard (orphaning delete is
    /// forbidden — detach the participating assets first, then
    /// retry).
    pub async fn delete_if_empty(
        &self,
        command: DeleteSessionCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let sid = SessionId::new(command.id)?;
        self.repo.delete_if_empty(&sid).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::ExternalSessionKey;
    use crate::error::DomainError;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use std::sync::Mutex;

    // In-memory stub — mirrors the adapter contract enough to pin
    // (a) the find-or-create idempotence: `create` looks up
    // (persona_id, external_key) and hands back the row that holds it
    // rather than refusing; and
    // (b) the P2 CRUD semantics: `update_metadata` respects `None =
    // preserve`, `rename` writes NULL unconditionally, and
    // `delete_if_empty` is a bare row check (asset attachment is
    // exercised at the SQLite layer where a real `asset` table
    // exists).
    struct InMemoryRepo {
        rows: Mutex<Vec<Session>>,
    }

    impl InMemoryRepo {
        fn new() -> Self {
            Self {
                rows: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SessionRepository for InMemoryRepo {
        async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, DomainError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.id == *id)
                .cloned())
        }

        async fn find_by_external_key(
            &self,
            persona_id: &PersonaId,
            external_key: &ExternalSessionKey,
        ) -> Result<Option<Session>, DomainError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.persona_id == *persona_id && s.external_key == *external_key)
                .cloned())
        }

        async fn create(&self, session: &Session) -> Result<Session, DomainError> {
            // Lookup and insert under one lock — the stub's stand-in for
            // the adapter's single isle closure, and the reason it can
            // be single-valued without anything asserting uniqueness.
            let mut rows = self.rows.lock().unwrap();
            if let Some(held) = rows.iter().find(|s| {
                s.persona_id == session.persona_id && s.external_key == session.external_key
            }) {
                return Ok(held.clone());
            }
            rows.push(session.clone());
            Ok(session.clone())
        }

        async fn update_metadata(
            &self,
            id: &SessionId,
            patch: &SessionMetadataPatch,
            now: DateTime<Utc>,
        ) -> Result<Session, DomainError> {
            let mut rows = self.rows.lock().unwrap();
            let row = rows
                .iter_mut()
                .find(|s| s.id == *id)
                .ok_or_else(|| DomainError::not_found("session", id))?;
            // COALESCE semantic: None = keep existing, Some = write.
            if let Some(title) = patch.title.clone() {
                row.metadata.title = Some(title);
            }
            if let Some(note) = patch.note.clone() {
                row.metadata.note = Some(note);
            }
            if let Some(cover_hint) = patch.cover_hint.clone() {
                row.metadata.cover_hint = Some(cover_hint);
            }
            row.updated_at_ms = now.timestamp_millis();
            Ok(row.clone())
        }

        async fn rename(
            &self,
            id: &SessionId,
            new_title: Option<String>,
            now: DateTime<Utc>,
        ) -> Result<Session, DomainError> {
            let mut rows = self.rows.lock().unwrap();
            let row = rows
                .iter_mut()
                .find(|s| s.id == *id)
                .ok_or_else(|| DomainError::not_found("session", id))?;
            // Unconditional write (including None → clear) — this is
            // the sole path that expresses NULL on title.
            row.metadata.title = new_title;
            row.updated_at_ms = now.timestamp_millis();
            Ok(row.clone())
        }

        async fn delete_if_empty(&self, id: &SessionId) -> Result<(), DomainError> {
            let mut rows = self.rows.lock().unwrap();
            let idx = rows
                .iter()
                .position(|s| s.id == *id)
                .ok_or_else(|| DomainError::not_found("session", id))?;
            rows.remove(idx);
            Ok(())
        }

        async fn list_by_persona(
            &self,
            persona_id: &PersonaId,
        ) -> Result<Vec<Session>, DomainError> {
            let mut list: Vec<Session> = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.persona_id == *persona_id)
                .cloned()
                .collect();
            list.sort_by_key(|s| std::cmp::Reverse(s.started_at_ms));
            Ok(list)
        }
    }

    /// These tests are about the Session verbs, not about who asked for
    /// them: nothing here reads the context, so the fixtures pass the
    /// value a system write carries.
    fn anyone() -> AttributionContext {
        AttributionContext::unrecorded()
    }

    #[tokio::test]
    async fn find_or_create_is_idempotent_on_repeat_call() {
        let repo = Arc::new(InMemoryRepo::new());
        let service = SessionService::new(repo);
        let persona = PersonaId::new();

        let first = service
            .find_or_create_by_external_key(&persona, "cc.session.42", 10, 20, &anyone())
            .await
            .unwrap();
        let second = service
            .find_or_create_by_external_key(&persona, "cc.session.42", 999, 1_000, &anyone())
            .await
            .unwrap();

        assert_eq!(
            first.id, second.id,
            "same external_key must resolve to the same SessionId"
        );
        // Seed timestamps only apply on the first call — the second
        // call must not touch the derived aggregates.
        assert_eq!(second.started_at_ms, 10);
        assert_eq!(second.ended_at_ms, 20);
    }

    // Seeds one Session and returns its id so the CRUD tests below
    // do not each re-implement the setup dance.
    async fn seeded(service: &SessionService, persona: &PersonaId, key: &str) -> SessionId {
        let dto = service
            .find_or_create_by_external_key(persona, key, 10, 20, &anyone())
            .await
            .unwrap();
        SessionId::new(dto.id).unwrap()
    }

    #[tokio::test]
    async fn rename_sets_title_and_can_clear_back_to_none() {
        let repo = Arc::new(InMemoryRepo::new());
        let service = SessionService::new(repo);
        let persona = PersonaId::new();
        let id = seeded(&service, &persona, "cc.session.rename").await;

        // Some → set.
        let named = service
            .rename(
                RenameSessionCommand {
                    id: id.as_str().to_string(),
                    title: Some("Draft 1".into()),
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(named.title.as_deref(), Some("Draft 1"));

        // None → clear back to untitled (the canonical NULL path).
        let cleared = service
            .rename(
                RenameSessionCommand {
                    id: id.as_str().to_string(),
                    title: None,
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(cleared.title, None);
    }

    #[tokio::test]
    async fn patch_metadata_preserves_none_fields() {
        let repo = Arc::new(InMemoryRepo::new());
        let service = SessionService::new(repo);
        let persona = PersonaId::new();
        let id = seeded(&service, &persona, "cc.session.patch").await;

        // First: set all three fields so we have concrete state to
        // preserve in the follow-up partial patch below.
        let full = service
            .patch_metadata(
                PatchSessionMetadataCommand {
                    id: id.as_str().to_string(),
                    title: Some("First title".into()),
                    note: Some("first note".into()),
                    cover_hint: Some("first cover".into()),
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(full.title.as_deref(), Some("First title"));
        assert_eq!(full.note.as_deref(), Some("first note"));
        assert_eq!(full.cover_hint.as_deref(), Some("first cover"));

        // Partial: only note changes. Title + cover_hint stay put —
        // the None-preserves contract in action.
        let partial = service
            .patch_metadata(
                PatchSessionMetadataCommand {
                    id: id.as_str().to_string(),
                    title: None,
                    note: Some("updated note".into()),
                    cover_hint: None,
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(
            partial.title.as_deref(),
            Some("First title"),
            "title=None on patch must preserve the existing value"
        );
        assert_eq!(partial.note.as_deref(), Some("updated note"));
        assert_eq!(
            partial.cover_hint.as_deref(),
            Some("first cover"),
            "cover_hint=None on patch must preserve the existing value"
        );
    }

    #[tokio::test]
    async fn delete_if_empty_happy_path_removes_the_row() {
        let repo = Arc::new(InMemoryRepo::new());
        let service = SessionService::new(repo);
        let persona = PersonaId::new();
        let id = seeded(&service, &persona, "cc.session.delete").await;

        service
            .delete_if_empty(
                DeleteSessionCommand {
                    id: id.as_str().to_string(),
                },
                &anyone(),
            )
            .await
            .unwrap();

        // Second delete → NotFound (the row is gone; a missing target is
        // a 404, not a 409 — see `DomainError::NotFound`).
        let err = service
            .delete_if_empty(
                DeleteSessionCommand {
                    id: id.as_str().to_string(),
                },
                &anyone(),
            )
            .await;
        assert!(matches!(err, Err(DomainError::NotFound { .. })));

        // get() should also come up empty now.
        assert!(service.get(id.as_str()).await.unwrap().is_none());
    }

    // The "asset still attached → Conflict" leg of `delete_if_empty`
    // requires a real `asset` table (the guard is a SQLite `COUNT(*)
    // FROM asset` inside the writer isle); it is exercised in
    // `asterism-infra::sqlite::repo::session::tests::delete_if_empty_rejects_when_asset_still_attached`.

    // `list_by_persona_returns_freshest_started_at_first` went with the
    // service method it covered. The repository ordering it really
    // pinned is still covered by
    // `asterism-infra::sqlite::repo::session::tests::list_by_persona_returns_freshest_first`.
}
