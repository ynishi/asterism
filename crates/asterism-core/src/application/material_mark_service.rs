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
    parse_timeline_span,
};
use crate::application::material_layer_service::{MARKED_MATERIAL_ORD, default_annotation_layer};
use crate::domain::asset::Asset;
use crate::domain::asset_comment::CommentAuthor;
use crate::domain::attribution::AttributionContext;
use crate::domain::material_mark::{MaterialAnchor, MaterialMark};
use crate::domain::repository::{
    AssetRepository, MaterialLayerRepository, MaterialMarkRepository, PersonaRepository,
};
use crate::error::DomainError;

/// Application-layer surface for [`MaterialMark`].
pub struct MaterialMarkService {
    marks: Arc<dyn MaterialMarkRepository>,
    layers: Arc<dyn MaterialLayerRepository>,
    assets: Arc<dyn AssetRepository>,
    personas: Arc<dyn PersonaRepository>,
}

impl MaterialMarkService {
    /// Wires the service around its ports.
    pub fn new(
        marks: Arc<dyn MaterialMarkRepository>,
        layers: Arc<dyn MaterialLayerRepository>,
        assets: Arc<dyn AssetRepository>,
        personas: Arc<dyn PersonaRepository>,
    ) -> Self {
        Self {
            marks,
            layers,
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
    ///
    /// The command names no layer, and is not expected to: a person
    /// clicking a position on a timeline is answering "where", not "in
    /// which of my passes over this material". So the band is resolved
    /// here — the asset's default annotation layer, created on the
    /// first mark it ever receives
    /// ([`default_annotation_layer`]). The mark's `layer_id` is
    /// mandatory, so this is not an optional enrichment step: a post
    /// that could not resolve a band would have nothing to store.
    ///
    /// Resolved **after** the anchor and the author are checked, so a
    /// post that is going to be refused does not leave a band behind on
    /// an asset that still has no marks.
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
        let layer =
            default_annotation_layer(self.layers.as_ref(), &asset_id, MARKED_MATERIAL_ORD).await?;
        let mark = MaterialMark::new(asset_id, layer.id, anchor, author, command.body, Utc::now())?;
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
    ///
    /// # No origin guard, and what has to change before one is owed
    ///
    /// The chapter verbs next door run
    /// [`require_user_owned`](crate::application::material_layer_service)
    /// on the band before writing; this one does not, and the reason is
    /// that today it cannot reach a band the check would refuse.
    /// [`Self::post`] is the only writer of `material_mark`, it always
    /// resolves the band through
    /// [`default_annotation_layer`], and that function mints
    /// `origin = User`. The schema holds the same fact independently —
    /// `CHECK (role <> 'annotation' OR is_default = 0 OR origin =
    /// 'user')` — so a default annotation band that is *not* the user's
    /// cannot be stored at all, by this path or any other. Every mark
    /// therefore sits in a user-owned band, and a guard here would be a
    /// branch no input reaches, which is a branch no test can cover.
    ///
    /// It stops being unreachable the moment a mark can land somewhere
    /// else: a command that names its band, an importer that reads
    /// notes out of a file into an `Imported` one, a job deriving them
    /// into a `Machine` one. Any of those makes an imported note
    /// editable in place — and an imported band's contents are replaced
    /// by re-reading the material, so the edit would survive until the
    /// next probe and then vanish without the person being told. Add
    /// the `require_user_owned` call to this verb and to
    /// [`Self::delete`] in the same change that adds the second writer,
    /// not after.
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
    ///
    /// No origin guard, for the reason [`Self::edit`] sets out at
    /// length: every mark is in the asset's default annotation band,
    /// which both `default_annotation_layer` and the schema's
    /// `CHECK` force to be the user's. This verb is the weaker case of
    /// the two — it does not even read the mark, so a guard would cost
    /// a fetch to check a condition nothing can currently fail — but it
    /// owes the same call as soon as a mark can belong to a band the
    /// person does not own.
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
            // The lift onto the domain's unsigned axis — and the
            // refusals `TimelineSpan` owns — are `parse_timeline_span`'s,
            // shared with the chapter face so the two callers that place
            // something on this timeline cannot disagree about what a
            // wire millisecond means.
            Ok(MaterialAnchor::Temporal(parse_timeline_span(
                start_ms,
                command.end_ms,
            )?))
        }
        other => Err(DomainError::Validation(format!(
            "unknown anchor_kind: {other:?}"
        ))),
    }
}
