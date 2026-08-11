//! `PersonaService` — use cases for the persona lifecycle.
//!
//! Weak CQRS split: reads (`list`) go through a projection; writes
//! (`register` / `set_archived` / `trash` / `restore` / `purge`) enforce
//! the invariants.
//!
//! Deleting a persona is a two-step verb like everywhere else in the
//! trash model: `trash` takes the persona and its live assets out of
//! sight (reversibly, keyed on a shared stamp), and only `purge` — which
//! refuses a live persona — lets the DB cascade do its irreversible
//! work. `archived` is a different thing entirely: a sidebar visibility
//! toggle over data that is still live.
//!
//! Every write here takes an [`AttributionContext`] it does not persist:
//! no persona / theme / profile column carries attribution, and none is
//! being added (see the [`application`](crate::application) module doc
//! for why the argument is required anyway).

use std::sync::Arc;

use asterism_contract::command::{
    ArchivePersonaCommand, DeletePersonaProfileCommand, DeletePersonaThemeCommand,
    PurgePersonaCommand, RegisterPersonaCommand, ReorderPersonasCommand, RestorePersonaCommand,
    SetPersonaProfileCommand, SetPersonaThemeCommand, TrashPersonaCommand,
};
use asterism_contract::dto::{PersonaDto, PersonaProfileDto, PersonaThemeDto};
use chrono::Utc;

use crate::application::mapping::{
    parse_asset_id, parse_persona_id, persona_profile_to_dto, persona_theme_to_dto, persona_to_dto,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::persona::Persona;
use crate::domain::persona_profile::PersonaProfile;
use crate::domain::persona_theme::PersonaTheme;
use crate::domain::repository::{
    PersonaProfileRepository, PersonaRepository, PersonaThemeRepository,
};
use crate::domain::value::PackId;
use crate::error::DomainError;

/// Persona use-case service. Shared as an `Arc` through Tauri state and
/// server contexts.
pub struct PersonaService {
    repo: Arc<dyn PersonaRepository>,
    theme_repo: Arc<dyn PersonaThemeRepository>,
    profile_repo: Arc<dyn PersonaProfileRepository>,
    /// Asset port — `trash` / `restore` move the persona's assets with
    /// it. Held here rather than reached through `AssetService` because
    /// the dependency would be circular, and the operation this needs is
    /// a single bulk stamp, not a use case.
    assets: Arc<dyn crate::domain::repository::AssetRepository>,
    /// Retrieval index, write side — a trashed persona's assets must
    /// stop turning up as candidates, exactly as an individually
    /// trashed asset does.
    indexer: Arc<dyn crate::domain::repository::AssetIndexer>,
    /// Job queue — `restore` re-indexes the assets it brings back
    /// through the same `IndexRebuild` job the single-asset restore
    /// uses, rather than duplicating body resolution here.
    jobs: Arc<dyn crate::domain::repository::JobQueue>,
}

impl PersonaService {
    /// Constructs the service around the persona + theme + profile
    /// repository ports. Theme and profile reads / writes go
    /// through separate ports so identity (`register` / `save`),
    /// chrome (`set_theme`), and identity signal (`set_profile`)
    /// stay on independent commit paths.
    pub fn new(
        repo: Arc<dyn PersonaRepository>,
        theme_repo: Arc<dyn PersonaThemeRepository>,
        profile_repo: Arc<dyn PersonaProfileRepository>,
        assets: Arc<dyn crate::domain::repository::AssetRepository>,
        indexer: Arc<dyn crate::domain::repository::AssetIndexer>,
        jobs: Arc<dyn crate::domain::repository::JobQueue>,
    ) -> Self {
        Self {
            repo,
            theme_repo,
            profile_repo,
            assets,
            indexer,
            jobs,
        }
    }

    /// Returns every persona, sorted for sidebar rendering.
    pub async fn list(&self) -> Result<Vec<PersonaDto>, DomainError> {
        Ok(self.repo.list().await?.iter().map(persona_to_dto).collect())
    }

    /// Registers a new persona. Enforces the "pack_id is unique" invariant.
    pub async fn register(
        &self,
        command: RegisterPersonaCommand,
        _attribution: &AttributionContext,
    ) -> Result<PersonaDto, DomainError> {
        let pack_id = command.pack_id.map(PackId::new).transpose()?;
        if let Some(pack) = &pack_id
            && self.repo.find_by_pack_id(pack).await?.is_some()
        {
            return Err(DomainError::DuplicatePersona(pack.clone()));
        }
        let persona = Persona::new(command.name, pack_id)?;
        self.repo.save(&persona).await?;
        Ok(persona_to_dto(&persona))
    }

    /// Toggles the archive flag (a soft-delete alternative).
    pub async fn set_archived(
        &self,
        command: ArchivePersonaCommand,
        _attribution: &AttributionContext,
    ) -> Result<PersonaDto, DomainError> {
        let id = parse_persona_id(&command.persona_id)?;
        let mut persona = self
            .repo
            .find(&id)
            .await?
            .ok_or(DomainError::PersonaNotFound(id))?;
        persona.archived = command.archived;
        persona.updated_at = Utc::now();
        self.repo.save(&persona).await?;
        Ok(persona_to_dto(&persona))
    }

    /// Moves a persona **and everything it holds** to the trash.
    ///
    /// This used to be a hard delete, and it was the one destructive
    /// path that skipped the trash entirely: `asset.persona_id` is
    /// `ON DELETE CASCADE`, so deleting the row physically removed every
    /// asset the persona ever held — ratings, comments, group filings,
    /// body text — irrecoverably, and left their search documents behind.
    /// A persona's assets go to the trash with it instead.
    ///
    /// The persona's live assets are stamped with the **same** timestamp
    /// as the persona, which is what lets [`restore`](Self::restore) put
    /// back exactly this set. Assets the user had already trashed by
    /// hand carry a different stamp and are left alone by both verbs.
    pub async fn trash(
        &self,
        command: TrashPersonaCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_persona_id(&command.persona_id)?;
        if self.repo.find(&id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(id));
        }
        // Persona first, because it *owns* the stamp: `trash` returns the
        // effective one (the original on a repeat), and handing that to
        // the asset side is what keeps the two halves on a single key. A
        // second `Utc::now()` here would strand the assets — restore
        // matches on the persona's stamp, and a re-run would mint one the
        // already-trashed assets do not carry.
        //
        // Partial failure is therefore repairable by re-running: the
        // persona keeps its original stamp, and `trash_by_persona` picks
        // up whatever is still live under it.
        let stamp = self.repo.trash(&id, Utc::now()).await?;
        let trashed = self.assets.trash_by_persona(&id, stamp).await?;
        // Immediately, not after any further write: a trashed asset with
        // a live search document comes back as a hit, and `search`
        // hydrates through the deliberately unfiltered `cards_by_ids`.
        self.drop_search_documents(&trashed).await;
        Ok(())
    }

    /// Returns a trashed persona and the assets that went down with it.
    ///
    /// Only assets carrying the persona's own trash stamp come back, so
    /// anything the user trashed separately stays in the trash. The
    /// restored assets are re-indexed through the same job the
    /// single-asset restore uses.
    pub async fn restore(
        &self,
        command: RestorePersonaCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_persona_id(&command.persona_id)?;
        if self.repo.find(&id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(id));
        }
        // Assets first, persona stamp last. The stamp is the only key to
        // this persona's half of the trash: clearing it before the assets
        // are back would make a failure here unrecoverable — a re-run
        // would read `None`, skip the asset restore, and report success
        // while every asset stayed trashed. In this order a failure
        // leaves the persona trashed, and re-running is exact.
        if let Some(stamp) = self.repo.trashed_at(&id).await? {
            let restored = self.assets.restore_by_persona(&id, stamp).await?;
            self.repo.restore(&id).await?;
            for asset_id in restored {
                if let Err(err) = self
                    .jobs
                    .enqueue(
                        crate::domain::job::JobKind::IndexRebuild,
                        serde_json::json!({ "asset_id": asset_id.to_string() }),
                    )
                    .await
                {
                    tracing::warn!(
                        event = "diag.reindex.enqueue_failed",
                        asset_id = %asset_id,
                        error = %err,
                        "could not enqueue reindex for restored asset"
                    );
                }
            }
        }
        Ok(())
    }

    /// Permanently deletes an **already-trashed** persona; the FK
    /// cascade takes its assets, groups, snapshots and dispatch history.
    /// `Conflict` when the persona is still live.
    pub async fn purge(
        &self,
        command: PurgePersonaCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_persona_id(&command.persona_id)?;
        // Collect the doomed assets before the cascade removes them —
        // afterwards there is no way to learn which documents to drop,
        // and a search index full of ids that resolve to nothing is what
        // this whole path was leaking before.
        //
        // `ids_by_persona`, not a listing: the card / index projections
        // clamp at their page ceiling, so a persona holding more assets
        // than that would silently keep the tail's documents forever —
        // exactly the leak this path exists to close.
        let doomed = self.assets.ids_by_persona(&id).await?;
        self.repo.purge(&id).await?;
        self.drop_search_documents(&doomed).await;
        Ok(())
    }

    /// Drops several assets' retrieval documents behind a single flush.
    /// Failures are logged, never propagated: the row write they follow
    /// has already happened, and a stale document is the recoverable
    /// direction.
    async fn drop_search_documents(&self, ids: &[crate::domain::value::AssetId]) {
        if ids.is_empty() {
            return;
        }
        let mut dropped = false;
        for id in ids {
            match self.indexer.remove(id).await {
                Ok(()) => dropped = true,
                Err(err) => {
                    tracing::warn!(
                        event = "diag.retrieval.remove_failed",
                        asset_id = %id,
                        error = %err,
                        "retrieval index remove failed"
                    )
                }
            }
        }
        if dropped && let Err(err) = self.indexer.flush().await {
            tracing::warn!(
                event = "diag.retrieval.flush_failed",
                dropped = ids.len(),
                error = %err,
                "retrieval index flush failed after dropping documents"
            );
        }
    }

    /// Fetches the theme for a persona. `None` when the persona has
    /// never had a theme set — the UI falls back to built-in defaults.
    pub async fn get_theme(
        &self,
        persona_id: &str,
    ) -> Result<Option<PersonaThemeDto>, DomainError> {
        let id = parse_persona_id(persona_id)?;
        Ok(self
            .theme_repo
            .get(&id)
            .await?
            .as_ref()
            .map(persona_theme_to_dto))
    }

    /// Sets (or clears) the wallpaper for a persona. Idempotent — the
    /// theme row is upserted keyed by `persona_id`.
    pub async fn set_theme(
        &self,
        command: SetPersonaThemeCommand,
        _attribution: &AttributionContext,
    ) -> Result<PersonaThemeDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        if self.repo.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        let wallpaper_asset_id = command
            .wallpaper_asset_id
            .as_deref()
            .map(parse_asset_id)
            .transpose()?;
        let theme = PersonaTheme {
            persona_id,
            wallpaper_asset_id,
            updated_at: Utc::now(),
        };
        self.theme_repo.upsert(&theme).await?;
        Ok(persona_theme_to_dto(&theme))
    }

    /// Removes the theme row entirely so the persona falls back to
    /// built-in UI defaults. Idempotent — a missing row is a no-op.
    pub async fn delete_theme(
        &self,
        command: DeletePersonaThemeCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_persona_id(&command.persona_id)?;
        self.theme_repo.delete(&id).await
    }

    /// Fetches the identity profile for a persona. `None` when
    /// the profile row has never been set — the sidebar falls back
    /// to the plain name + accent color card.
    pub async fn get_profile(
        &self,
        persona_id: &str,
    ) -> Result<Option<PersonaProfileDto>, DomainError> {
        let id = parse_persona_id(persona_id)?;
        Ok(self
            .profile_repo
            .get(&id)
            .await?
            .as_ref()
            .map(persona_profile_to_dto))
    }

    /// Upserts the identity profile. Idempotent — the profile row
    /// is keyed on `persona_id`. Every optional field is a full
    /// replace, so passing `None` clears the stored value.
    pub async fn set_profile(
        &self,
        command: SetPersonaProfileCommand,
        _attribution: &AttributionContext,
    ) -> Result<PersonaProfileDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        if self.repo.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        let avatar_asset_id = command
            .avatar_asset_id
            .as_deref()
            .map(parse_asset_id)
            .transpose()?;
        let bio_short = command.bio_short.and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        });
        let role_tag = command.role_tag.and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        });
        let profile = PersonaProfile {
            persona_id,
            avatar_asset_id,
            bio_short,
            role_tag,
            updated_at: Utc::now(),
        };
        self.profile_repo.upsert(&profile).await?;
        Ok(persona_profile_to_dto(&profile))
    }

    /// Removes the profile row entirely so the sidebar falls back
    /// to the plain name + accent color. Idempotent.
    pub async fn delete_profile(
        &self,
        command: DeletePersonaProfileCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_persona_id(&command.persona_id)?;
        self.profile_repo.delete(&id).await
    }

    /// Rewrites `display_order` across a persona slice so a sidebar
    /// drag-reorder survives across sessions. Ids not present in
    /// `ordered_ids` are left untouched — the caller can therefore
    /// send only the visible subset when the sidebar filters some
    /// personas out.
    pub async fn reorder(
        &self,
        command: ReorderPersonasCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        for (idx, raw_id) in command.ordered_ids.iter().enumerate() {
            let id = parse_persona_id(raw_id)?;
            let mut persona = self
                .repo
                .find(&id)
                .await?
                .ok_or(DomainError::PersonaNotFound(id))?;
            let next = idx as i64;
            if persona.display_order == next {
                continue;
            }
            persona.display_order = next;
            persona.updated_at = Utc::now();
            self.repo.save(&persona).await?;
        }
        Ok(())
    }
}
