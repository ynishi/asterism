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
use crate::domain::repository::{AssetCommentRepository, AssetRepository, PersonaRepository};
use crate::error::DomainError;

/// Application-layer surface for `AssetComment`.
pub struct AssetCommentService {
    comments: Arc<dyn AssetCommentRepository>,
    assets: Arc<dyn AssetRepository>,
    personas: Arc<dyn PersonaRepository>,
}

impl AssetCommentService {
    /// Wires the service around its ports.
    pub fn new(
        comments: Arc<dyn AssetCommentRepository>,
        assets: Arc<dyn AssetRepository>,
        personas: Arc<dyn PersonaRepository>,
    ) -> Self {
        Self {
            comments,
            assets,
            personas,
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
        Ok(asset_comment_to_dto(&hit))
    }

    /// Deletes a comment.
    pub async fn delete(
        &self,
        command: DeleteAssetCommentCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let cid = parse_asset_comment_id(&command.comment_id)?;
        self.comments.delete(&cid).await
    }
}
