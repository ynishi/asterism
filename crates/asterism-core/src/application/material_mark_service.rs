//! `MaterialMarkService` — the marks placed into an Asset's material.
//!
//! Verbs:
//! - [`post`](MaterialMarkService::post) — places a mark at a position
//!   in the material (User or Persona author).
//! - [`list_by_asset`](MaterialMarkService::list_by_asset) — reads the
//!   marks in the material's own order, which is not the order they
//!   were placed in.
//! - [`edit`](MaterialMarkService::edit) — rewrites the body of an
//!   existing mark (stamps `edited_at`).
//! - [`delete`](MaterialMarkService::delete) — removes one row.
//!
//! Moving a mark is not among them. Rewording a note and repositioning
//! it are different acts, and no surface asks for the second one yet;
//! adding it later is a verb here, not a change to any of these.
//!
//! Every write takes an [`AttributionContext`] it does not persist, for
//! the same reason
//! [`AssetCommentService`](crate::application::asset_comment_service)
//! does: a mark records a
//! [`CommentAuthor`](crate::domain::asset_comment::CommentAuthor), whose
//! `Persona` variant is a register (a voice) rather than a writer — so
//! the row states who is speaking without stating who is accountable.

use std::sync::Arc;

use asterism_contract::command::{
    DeleteMaterialMarkCommand, EditMaterialMarkCommand, PostMaterialMarkCommand,
};
use asterism_contract::dto::MaterialMarkDto;
use chrono::Utc;

use crate::application::mapping::{
    material_mark_to_dto, parse_asset_id, parse_material_mark_id, parse_persona_id,
};
use crate::domain::asset::Asset;
use crate::domain::asset_comment::CommentAuthor;
use crate::domain::attribution::AttributionContext;
use crate::domain::material_mark::{MaterialAnchor, MaterialMark, TimelineSpan};
use crate::domain::repository::{AssetRepository, MaterialMarkRepository, PersonaRepository};
use crate::error::DomainError;

/// Application-layer surface for [`MaterialMark`].
pub struct MaterialMarkService {
    marks: Arc<dyn MaterialMarkRepository>,
    assets: Arc<dyn AssetRepository>,
    personas: Arc<dyn PersonaRepository>,
}

impl MaterialMarkService {
    /// Wires the service around its ports.
    pub fn new(
        marks: Arc<dyn MaterialMarkRepository>,
        assets: Arc<dyn AssetRepository>,
        personas: Arc<dyn PersonaRepository>,
    ) -> Self {
        Self {
            marks,
            assets,
            personas,
        }
    }

    /// Places a mark in the target Asset's material.
    ///
    /// The Asset is fetched rather than merely checked for existence:
    /// whether a mark can be placed at all depends on what the material
    /// offers, and the temporal anchor's precondition
    /// (`asset.duration_ms`) is on the row.
    pub async fn post(
        &self,
        command: PostMaterialMarkCommand,
        _attribution: &AttributionContext,
    ) -> Result<MaterialMarkDto, DomainError> {
        let asset_id = parse_asset_id(&command.asset_id)?;
        let asset = self
            .assets
            .find(&asset_id)
            .await?
            .ok_or(DomainError::AssetNotFound(asset_id))?;
        let anchor = build_anchor(&asset, &command)?;
        let author = self.decode_author(&command).await?;
        let mark = MaterialMark::new(asset_id, anchor, author, command.body, Utc::now())?;
        self.marks.save(&mark).await?;
        Ok(material_mark_to_dto(&mark))
    }

    /// Reads every mark in an Asset's material, in the material's order
    /// (the port's contract: `start_ms` ascending, ties broken by id).
    pub async fn list_by_asset(&self, asset_id: &str) -> Result<Vec<MaterialMarkDto>, DomainError> {
        let aid = parse_asset_id(asset_id)?;
        let rows = self.marks.list_by_asset(&aid).await?;
        Ok(rows.iter().map(material_mark_to_dto).collect())
    }

    /// Rewrites the body of an existing mark. Stamps `edited_at`.
    ///
    /// The anchor is left alone — this verb rewords a mark, it does not
    /// move one.
    pub async fn edit(
        &self,
        command: EditMaterialMarkCommand,
        _attribution: &AttributionContext,
    ) -> Result<MaterialMarkDto, DomainError> {
        let mid = parse_material_mark_id(&command.mark_id)?;
        // Fetch → mutate → save, over the asset's listing: `list_by_asset`
        // is the port's only read verb, mirroring how
        // `AssetCommentService::edit` walks its thread. A `find(id)`
        // port would shortcut it in a follow-up.
        let aid = parse_asset_id(&command.asset_id)?;
        let mut rows = self.marks.list_by_asset(&aid).await?;
        let mut hit = rows
            .drain(..)
            .find(|m| m.id == mid)
            .ok_or_else(|| DomainError::not_found("material mark", &command.mark_id))?;
        let new_body = command.body.trim().to_string();
        if new_body.is_empty() {
            return Err(DomainError::Validation(
                "MaterialMark body must not be empty".into(),
            ));
        }
        hit.body = new_body;
        hit.edited_at = Some(Utc::now());
        self.marks.save(&hit).await?;
        Ok(material_mark_to_dto(&hit))
    }

    /// Deletes a mark.
    pub async fn delete(
        &self,
        command: DeleteMaterialMarkCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let mid = parse_material_mark_id(&command.mark_id)?;
        self.marks.delete(&mid).await
    }

    /// Decodes the `(author_kind, author_persona_id)` wire pair,
    /// checking that a named Persona exists.
    ///
    /// The same eighteen lines as
    /// `AssetCommentService::post`, deliberately: the author vocabulary
    /// is shared and is due to be lifted out into a sub-domain of its
    /// own, and two *different* spellings of it in the meantime would be
    /// the harder thing to lift.
    async fn decode_author(
        &self,
        command: &PostMaterialMarkCommand,
    ) -> Result<CommentAuthor, DomainError> {
        match command.author_kind.as_str() {
            "user" => Ok(CommentAuthor::User),
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
                Ok(CommentAuthor::Persona { persona_id: pid })
            }
            other => Err(DomainError::Validation(format!(
                "unknown author_kind: {other:?}"
            ))),
        }
    }
}

/// Builds the anchor a `post` asks for, refusing one the Asset's
/// material cannot carry.
///
/// One arm per coordinate space, so a variant added to
/// [`MaterialAnchor`] lands here as a missing arm rather than as a mark
/// placed on an axis nobody checked.
///
/// The `duration_ms` precondition sits **inside** the temporal arm
/// rather than ahead of the match. A rectangle on an image plane is the
/// next anchor kind, and an image has no duration — hoisting the check
/// would refuse every one of those for the reason that the *other* kind
/// needs a timeline. Today, with one arm, the two placements behave
/// identically; the difference is which one stays correct.
fn build_anchor(
    asset: &Asset,
    command: &PostMaterialMarkCommand,
) -> Result<MaterialAnchor, DomainError> {
    match command.anchor_kind.as_str() {
        "temporal" => {
            if asset.duration_ms.is_none() {
                return Err(DomainError::Validation(format!(
                    "asset {} has no duration, so its material has no timeline to mark",
                    asset.id
                )));
            }
            let start_ms = command.start_ms.ok_or_else(|| {
                DomainError::Validation("anchor_kind = temporal requires start_ms".into())
            })?;
            let start_ms = to_domain_ms(start_ms, "start_ms")?;
            let end_ms = command
                .end_ms
                .map(|value| to_domain_ms(value, "end_ms"))
                .transpose()?;
            // Emptiness, inversion and the storable range are
            // `TimelineSpan`'s to refuse, not restated here.
            Ok(MaterialAnchor::Temporal(TimelineSpan::new(
                start_ms, end_ms,
            )?))
        }
        other => Err(DomainError::Validation(format!(
            "unknown anchor_kind: {other:?}"
        ))),
    }
}

/// Lifts a wire millisecond onto the domain's unsigned axis.
///
/// The wire carries `i64` because that is what the storage column and
/// every other timestamp on the contract are; the axis starts at the
/// presentation origin and does not run backwards from it, so a negative
/// value is a caller error rather than a position before the start.
fn to_domain_ms(value: i64, field: &str) -> Result<u64, DomainError> {
    u64::try_from(value).map_err(|_| {
        DomainError::Validation(format!(
            "{field} = {value} is before the start of the timeline"
        ))
    })
}
