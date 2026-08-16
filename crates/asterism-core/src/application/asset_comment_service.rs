//! `AssetCommentService` — thread lifecycle on an Asset.
//!
//! Verbs:
//! - [`post`](AssetCommentService::post) — appends a new comment (User
//!   or Persona author).
//! - [`list`](AssetCommentService::list) — reads the thread in
//!   chronological order.
//! - [`edit`](AssetCommentService::edit) — rewrites the body of an
//!   existing comment (stamps `edited_at`).
//! - [`delete`](AssetCommentService::delete) — removes one row.
//!
//! MVP scope: no reactions, no threading (flat list), no @mention
//! parsing. The thread is a single flat stream — sufficient for
//! Asset-focused annotation.
//!
//! Every write here takes an [`AttributionContext`] it does not persist.
//! A comment records a
//! [`CommentAuthor`](crate::domain::asset_comment::CommentAuthor)
//! instead, whose `User` variant is the same "me" the attribution
//! `Owner` names and whose `Persona` variant is a register (a voice),
//! not a writer — so the row states who is speaking without stating who
//! is accountable. Closing that gap is a later wave;
//! this argument does not close it.

use std::sync::Arc;

use asterism_contract::command::{
    DeleteAssetCommentCommand, EditAssetCommentCommand, PostAssetCommentCommand,
};
use asterism_contract::dto::AssetCommentDto;
use chrono::Utc;

use crate::application::mapping::{
    asset_comment_to_dto, parse_asset_comment_id, parse_asset_id, parse_persona_id,
};
use crate::domain::asset_comment::{AssetComment, CommentAuthor};
use crate::domain::attribution::AttributionContext;
use crate::domain::job::JobKind;
use crate::domain::repository::{
    AssetCommentRepository, AssetRepository, JobQueue, PersonaRepository,
};
use crate::domain::value::AssetId;
use crate::error::DomainError;

/// Application-layer surface for `AssetComment`.
pub struct AssetCommentService {
    comments: Arc<dyn AssetCommentRepository>,
    assets: Arc<dyn AssetRepository>,
    personas: Arc<dyn PersonaRepository>,
    jobs: Arc<dyn JobQueue>,
    /// Body cache, write side — held only to clear a composition stamp
    /// when a re-index cannot be queued, so the backfill walk picks the
    /// row up. See [`Self::reindex`].
    asset_bodies: Arc<dyn crate::domain::repository::AssetBodyRepository>,
}

impl AssetCommentService {
    /// Wires the service around its ports.
    pub fn new(
        comments: Arc<dyn AssetCommentRepository>,
        assets: Arc<dyn AssetRepository>,
        personas: Arc<dyn PersonaRepository>,
        jobs: Arc<dyn JobQueue>,
        asset_bodies: Arc<dyn crate::domain::repository::AssetBodyRepository>,
    ) -> Self {
        Self {
            comments,
            assets,
            personas,
            jobs,
            asset_bodies,
        }
    }

    /// Re-composes the target asset's search document after a write to
    /// its thread.
    ///
    /// A comment is a section of the asset's derived text
    /// ([`derive_text`](crate::domain::derived_text::derive_text)), so
    /// every verb here leaves the index describing a thread that no
    /// longer exists — and for a picture the thread may be the only
    /// text there is, which makes this the difference between findable
    /// and not.
    ///
    /// Enqueue failure does not fail the write — the comment is saved,
    /// and the caller asked to post one, not to index it — but it is
    /// reported and repaired rather than swallowed. The backfill walk
    /// does not cover this case on its own: it selects bodies composed
    /// by an older reading, and this asset's body was composed by the
    /// current one, so clearing the stamp is what makes the walk the
    /// recovery it is described as.
    async fn reindex(&self, asset_id: &AssetId) {
        let Err(err) = self
            .jobs
            .enqueue(
                JobKind::IndexRebuild,
                serde_json::json!({ "asset_id": asset_id.to_string() }),
            )
            .await
        else {
            return;
        };
        tracing::warn!(
            event = "diag.index.enqueue_failed",
            asset_id = %asset_id,
            error = %err,
            "could not queue a re-index after a comment write"
        );
        if let Err(err) = self.asset_bodies.unstamp(asset_id).await {
            tracing::warn!(
                event = "diag.index.unstamp_failed",
                asset_id = %asset_id,
                error = %err,
                "the asset keeps a document composed from a thread that has changed"
            );
        }
    }

    /// Appends a comment to the target Asset.
    pub async fn post(
        &self,
        command: PostAssetCommentCommand,
        _attribution: &AttributionContext,
    ) -> Result<AssetCommentDto, DomainError> {
        let asset_id = parse_asset_id(&command.asset_id)?;
        if self.assets.find(&asset_id).await?.is_none() {
            return Err(DomainError::AssetNotFound(asset_id));
        }
        let author = match command.author_kind.as_str() {
            "user" => CommentAuthor::User,
            "persona" => {
                let pid_str = command.author_persona_id.as_deref().ok_or_else(|| {
                    DomainError::Validation(
                        "author_kind = persona requires author_persona_id".into(),
                    )
                })?;
                let pid = parse_persona_id(pid_str)?;
                if self.personas.find(&pid).await?.is_none() {
                    return Err(DomainError::PersonaNotFound(pid));
                }
                CommentAuthor::Persona { persona_id: pid }
            }
            other => {
                return Err(DomainError::Validation(format!(
                    "unknown author_kind: {other:?}"
                )));
            }
        };
        let comment = AssetComment::new(asset_id, author, command.body, Utc::now())?;
        self.comments.save(&comment).await?;
        self.reindex(&asset_id).await;
        Ok(asset_comment_to_dto(&comment))
    }

    /// Fetches every comment on an Asset in chronological order.
    pub async fn list(&self, asset_id: &str) -> Result<Vec<AssetCommentDto>, DomainError> {
        let aid = parse_asset_id(asset_id)?;
        let rows = self.comments.list_by_asset(&aid).await?;
        Ok(rows.iter().map(asset_comment_to_dto).collect())
    }

    /// Rewrites the body of an existing comment. Stamps `edited_at`.
    pub async fn edit(
        &self,
        command: EditAssetCommentCommand,
        _attribution: &AttributionContext,
    ) -> Result<AssetCommentDto, DomainError> {
        let cid = parse_asset_comment_id(&command.comment_id)?;
        // Fetch → mutate → save. `list_by_asset` is the only read
        // surface today so we walk the asset's thread to find one
        // row; a fresh `find(id)` port could shortcut this in a
        // follow-up.
        let asset_id_str = command.asset_id.clone();
        let aid = parse_asset_id(&asset_id_str)?;
        let mut rows = self.comments.list_by_asset(&aid).await?;
        let mut hit = rows
            .drain(..)
            .find(|c| c.id == cid)
            .ok_or_else(|| DomainError::not_found("comment", &command.comment_id))?;
        let new_body = command.body.trim().to_string();
        if new_body.is_empty() {
            return Err(DomainError::Validation(
                "comment body must not be empty".into(),
            ));
        }
        hit.body = new_body;
        hit.edited_at = Some(Utc::now());
        self.comments.save(&hit).await?;
        self.reindex(&aid).await;
        Ok(asset_comment_to_dto(&hit))
    }

    /// Deletes a comment.
    pub async fn delete(
        &self,
        command: DeleteAssetCommentCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let cid = parse_asset_comment_id(&command.comment_id)?;
        // Read before destroying: the command carries the comment id
        // alone, and the asset whose document has to be re-composed is
        // only knowable while the row is still there. A comment that
        // was already gone is not an error (`delete` is idempotent) and
        // has no asset to re-index.
        let target = self.comments.find(&cid).await?.map(|c| c.asset_id);
        self.comments.delete(&cid).await?;
        if let Some(asset_id) = target {
            self.reindex(&asset_id).await;
        }
        Ok(())
    }
}
