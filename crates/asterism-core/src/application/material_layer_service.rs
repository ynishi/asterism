//! `MaterialLayerService` — the bands of marks over an Asset's
//! material, and the one rule that makes them worth having: **a person
//! edits their own band and nothing else**.
//!
//! Verbs:
//! - [`list_by_asset`](MaterialLayerService::list_by_asset) — every
//!   band over an asset, in display order.
//! - [`create_user_layer`](MaterialLayerService::create_user_layer) —
//!   opens an empty band the person owns.
//! - [`set_default`](MaterialLayerService::set_default) — chooses which
//!   band a surface shows, and which one a new note lands in.
//! - [`delete_user_layer`](MaterialLayerService::delete_user_layer) —
//!   removes a band the person owns, with its contents.
//! - [`list_chapters`](MaterialLayerService::list_chapters) /
//!   [`post_chapter`](MaterialLayerService::post_chapter) /
//!   [`edit_chapter`](MaterialLayerService::edit_chapter) /
//!   [`delete_chapter`](MaterialLayerService::delete_chapter) — the
//!   sections inside one band.
//!
//! # Where the immutability rule lives, and why here
//!
//! An `Imported` band is the file's own statement about itself, and a
//! `Machine` band is a job's. Both are reproduced by running their
//! producer again, and neither has an author to ask about a conflict —
//! so a hand edit into one is either lost at the next re-read or
//! silently promoted into a claim the file never made. The guard is
//! therefore: **the four writing verbs above accept `LayerOrigin::User`
//! and refuse the rest.**
//!
//! It is not in the entity, because the entity cannot see who is
//! calling: the re-probe path writes into an imported band *by design*
//! (that is what re-reading a file means), and a rule that refused
//! every write to one would refuse the only legitimate writer along
//! with the illegitimate ones. It is not in the schema either, for the
//! same reason — a `CHECK` cannot read the caller. What the schema does
//! hold is the half that is about rows rather than callers: one default
//! per `(asset, material, role)`, and a default annotation band that
//! belongs to the user.
//!
//! The machine-side counterpart is
//! [`chapter_intake`](crate::application_support::chapter_intake),
//! in the support layer because a job drives it and no transport does.
//! Both routes reach the same two ports, and this is the only door with
//! a person on the other side of it.
//!
//! # Attribution
//!
//! Every write takes an [`AttributionContext`] it does not persist, for
//! the reason
//! [`MaterialMarkService`](crate::application::MaterialMarkService)
//! gives: a layer has no author column at all — it is a container, and
//! what it contains is what carries a voice. Receiving the argument is
//! still the point, since it is what makes a new caller of one of these
//! verbs name the channel it arrived through before it compiles.
//!
//! # Two faces, and which one to call
//!
//! The verbs above take and return **domain types**, unlike every
//! sibling in this module. Below them is a second `impl` block spelling
//! the same eight acts in **contract types** — commands in, DTOs out —
//! and that is the one the three adapters (HTTP, MCP, Tauri IPC) call.
//!
//! The split is not a preference. Each adapter would otherwise parse the
//! wire ids and shape the DTOs itself, including the per-band assembly
//! [`MaterialLayerService::list_views`] does, and three copies of that
//! is how a surface ends up answering a question differently depending
//! on which door it was asked through. Keeping it here also keeps the
//! domain face, which is what the storage-level tests drive and what an
//! in-process caller holding a [`MaterialLayerId`] already has in hand:
//! neither of those should have to spell an id back into a string to ask
//! this service something.
//!
//! When a verb is added, it belongs on both faces or on neither — a
//! domain verb no adapter can reach is the state this module was in
//! before the adapters landed, and it is not a state anything checks.

use std::sync::Arc;

use asterism_contract::command::{
    CreateMaterialLayerCommand, DeleteChapterMarkCommand, DeleteMaterialLayerCommand,
    EditChapterMarkCommand, PostChapterMarkCommand, SetDefaultMaterialLayerCommand,
};
use asterism_contract::dto::{ChapterMarkDto, MaterialLayerDto, MaterialLayerViewDto};

use crate::application::mapping::{
    chapter_mark_to_dto, material_layer_to_dto, parse_asset_id, parse_chapter_mark_id,
    parse_layer_role, parse_material_layer_id, parse_timeline_span,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::chapter_mark::ChapterMark;
use crate::domain::material_layer::{LayerOrigin, LayerRole, MaterialLayer, PRIMARY_MATERIAL_ORD};
use crate::domain::material_mark::TimelineSpan;
use crate::domain::repository::{
    AssetRepository, ChapterMarkRepository, LayerScope, MaterialLayerRepository,
};
use crate::domain::value::{AssetId, ChapterMarkId, MaterialLayerId};
use crate::error::DomainError;

/// Application-layer surface for [`MaterialLayer`] and the chapters
/// inside one.
pub struct MaterialLayerService {
    layers: Arc<dyn MaterialLayerRepository>,
    chapters: Arc<dyn ChapterMarkRepository>,
    assets: Arc<dyn AssetRepository>,
}

impl MaterialLayerService {
    /// Wires the service around its ports.
    pub fn new(
        layers: Arc<dyn MaterialLayerRepository>,
        chapters: Arc<dyn ChapterMarkRepository>,
        assets: Arc<dyn AssetRepository>,
    ) -> Self {
        Self {
            layers,
            chapters,
            assets,
        }
    }

    /// Every band over `asset_id`'s materials, in display order.
    pub async fn list_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<MaterialLayer>, DomainError> {
        self.layers.list_by_asset(asset_id).await
    }

    /// Opens an empty band the person owns.
    ///
    /// The asset is fetched rather than assumed: without the check the
    /// missing row surfaces as a foreign-key abort from the adapter,
    /// which reads as `Infra` — an infrastructure failure — when what
    /// happened is that the caller named an asset that is not there.
    ///
    /// The new band is never the default. Making it one would move the
    /// flag as a side effect of creation, and the caller that wants
    /// that has [`Self::set_default`] to say so with.
    pub async fn create_user_layer(
        &self,
        scope: LayerScope,
        ord: u32,
        _attribution: &AttributionContext,
    ) -> Result<MaterialLayer, DomainError> {
        self.require_asset(&scope.asset_id).await?;
        let layer = MaterialLayer::new(
            scope.asset_id,
            scope.material_ord,
            LayerOrigin::User,
            scope.role,
            false,
            ord,
        )?;
        self.layers.save(&layer).await?;
        Ok(layer)
    }

    /// Chooses the band a surface shows, and the one a new note lands
    /// in.
    ///
    /// Open to every origin, unlike the writing verbs: choosing to read
    /// the file's own chapter list rather than one's own is not an edit
    /// to either. The refusal that does apply is the entity's — an
    /// annotation band that is not the user's cannot be the default,
    /// because notes land there (see
    /// [`MaterialLayer::validate`](crate::domain::material_layer::MaterialLayer::validate)).
    pub async fn set_default(
        &self,
        layer_id: &MaterialLayerId,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let layer = self.require_layer(layer_id).await?;
        // Checked here as well as in the adapter's write, because the
        // adapter's is an `UPDATE` against a row it does not construct
        // — the entity's rule has no door on that path.
        let promoted = MaterialLayer {
            is_default: true,
            ..layer
        };
        promoted.validate()?;
        self.layers.set_default(layer_id).await
    }

    /// Removes a band the person owns, with everything in it.
    ///
    /// Refuses an imported or machine band: those are reproduced by
    /// running their producer again, so deleting one means the next
    /// re-probe silently recreates it — a verb whose effect lasts until
    /// something unrelated happens is worse than no verb.
    pub async fn delete_user_layer(
        &self,
        layer_id: &MaterialLayerId,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let layer = self.require_layer(layer_id).await?;
        require_user_owned(&layer, "delete")?;
        self.layers.delete(layer_id).await
    }

    /// The chapters in one band, in reading order.
    ///
    /// Open to every origin — reading the file's own list is the
    /// ordinary case.
    pub async fn list_chapters(
        &self,
        layer_id: &MaterialLayerId,
    ) -> Result<Vec<ChapterMark>, DomainError> {
        self.require_layer(layer_id).await?;
        self.chapters.list_by_layer(layer_id).await
    }

    /// Adds a section to a band the person owns.
    pub async fn post_chapter(
        &self,
        layer_id: &MaterialLayerId,
        span: TimelineSpan,
        label: impl Into<String> + Send,
        ord: u32,
        _attribution: &AttributionContext,
    ) -> Result<ChapterMark, DomainError> {
        let layer = self.require_layer(layer_id).await?;
        require_structure(&layer)?;
        require_user_owned(&layer, "write a chapter into")?;
        let chapter = ChapterMark::new(*layer_id, span, label, ord)?;
        self.chapters.save(&chapter).await?;
        Ok(chapter)
    }

    /// Rewrites one section of a band the person owns.
    ///
    /// `span` and `ord` are `Option`: `None` leaves the stored value
    /// alone. Unlike
    /// [`MaterialMarkService::edit`](crate::application::MaterialMarkService::edit),
    /// which rewords without moving, a chapter's position *is* part of
    /// what a person corrects — the whole reason for a user structure
    /// band is that the file's divisions are in the wrong places.
    ///
    /// The chapter is named by `(layer_id, chapter_id)` rather than by
    /// id alone, matching `EditMaterialMarkCommand`'s
    /// `(asset_id, mark_id)`: the guard is a fact about the parent, so
    /// the parent has to be named for the guard to be checkable in one
    /// read.
    pub async fn edit_chapter(
        &self,
        layer_id: &MaterialLayerId,
        chapter_id: &ChapterMarkId,
        label: impl Into<String> + Send,
        span: Option<TimelineSpan>,
        ord: Option<u32>,
        _attribution: &AttributionContext,
    ) -> Result<ChapterMark, DomainError> {
        let layer = self.require_layer(layer_id).await?;
        require_structure(&layer)?;
        require_user_owned(&layer, "edit a chapter in")?;
        let mut hit = self.require_chapter(layer_id, chapter_id).await?;
        hit.label = label.into();
        if let Some(span) = span {
            hit.span = span;
        }
        if let Some(ord) = ord {
            hit.ord = ord;
        }
        self.chapters.save(&hit).await?;
        Ok(hit)
    }

    /// Removes one section from a band the person owns.
    pub async fn delete_chapter(
        &self,
        layer_id: &MaterialLayerId,
        chapter_id: &ChapterMarkId,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let layer = self.require_layer(layer_id).await?;
        require_structure(&layer)?;
        require_user_owned(&layer, "delete a chapter from")?;
        // Fetched before the delete so that an id belonging to another
        // band is `NotFound` rather than a silent no-op: `delete` is
        // idempotent by contract, so without this the caller cannot
        // tell "removed" from "was never yours".
        self.require_chapter(layer_id, chapter_id).await?;
        self.chapters.delete(chapter_id).await
    }

    async fn require_asset(&self, asset_id: &AssetId) -> Result<(), DomainError> {
        if self.assets.find(asset_id).await?.is_none() {
            return Err(DomainError::AssetNotFound(*asset_id));
        }
        Ok(())
    }

    async fn require_layer(
        &self,
        layer_id: &MaterialLayerId,
    ) -> Result<MaterialLayer, DomainError> {
        self.layers
            .find(layer_id)
            .await?
            .ok_or_else(|| DomainError::not_found("material layer", layer_id))
    }

    /// One chapter of `layer_id`, over the port's listing verb.
    ///
    /// The listing is the port's only read of chapters, mirroring how
    /// [`MaterialMarkService::edit`](crate::application::MaterialMarkService::edit)
    /// walks its asset's marks. It also does the containment check for
    /// free: an id that belongs to another band is simply not in this
    /// answer.
    async fn require_chapter(
        &self,
        layer_id: &MaterialLayerId,
        chapter_id: &ChapterMarkId,
    ) -> Result<ChapterMark, DomainError> {
        self.chapters
            .list_by_layer(layer_id)
            .await?
            .into_iter()
            .find(|c| c.id == *chapter_id)
            .ok_or_else(|| DomainError::not_found("chapter mark", chapter_id))
    }
}

/// The wire face — the same eight acts, in the contract's vocabulary.
///
/// Every method here parses its ids, delegates to the domain verb of the
/// same name above, and shapes the answer into a DTO. They hold no rule
/// of their own: a refusal a caller sees through HTTP is the same
/// refusal, raised in the same place, as one an in-process caller sees.
impl MaterialLayerService {
    /// Every band over an asset's material, each with the sections in
    /// it, in display order.
    ///
    /// An annotation band's `chapters` is empty **without a query being
    /// issued for it**: that role holds notes, which are read through
    /// the material-marks route, so asking its chapter list would be
    /// asking a question whose answer is known from the row.
    ///
    /// One read per structure band rather than a single joined one. An
    /// asset carries single-digit bands, so what a join would buy is
    /// unmeasurable, and it would buy it by adding a second read path
    /// for chapters — one that could disagree with
    /// [`Self::list_chapters`] about ordering.
    pub async fn list_views(
        &self,
        asset_id: &str,
    ) -> Result<Vec<MaterialLayerViewDto>, DomainError> {
        let asset_id = parse_asset_id(asset_id)?;
        let bands = self.layers.list_by_asset(&asset_id).await?;
        let mut views = Vec::with_capacity(bands.len());
        for band in &bands {
            let chapters = match band.role {
                LayerRole::Structure => self.chapters.list_by_layer(&band.id).await?,
                LayerRole::Annotation => Vec::new(),
            };
            views.push(MaterialLayerViewDto {
                layer: material_layer_to_dto(band),
                chapters: chapters.iter().map(chapter_mark_to_dto).collect(),
            });
        }
        Ok(views)
    }

    /// The sections in one band, in reading order.
    pub async fn list_chapter_marks(
        &self,
        layer_id: &str,
    ) -> Result<Vec<ChapterMarkDto>, DomainError> {
        let layer_id = parse_material_layer_id(layer_id)?;
        let rows = self.list_chapters(&layer_id).await?;
        Ok(rows.iter().map(chapter_mark_to_dto).collect())
    }

    /// Opens an empty band the person owns.
    ///
    /// A command that names no `material_ord` means the primary
    /// original, the axis every surface marks today — the same
    /// convention the mark path resolves by, spelled here as a default
    /// rather than as an assumption the caller cannot override.
    pub async fn create_layer(
        &self,
        command: CreateMaterialLayerCommand,
        attribution: &AttributionContext,
    ) -> Result<MaterialLayerDto, DomainError> {
        let scope = LayerScope {
            asset_id: parse_asset_id(&command.asset_id)?,
            material_ord: command.material_ord.unwrap_or(PRIMARY_MATERIAL_ORD),
            role: parse_layer_role(&command.role)?,
        };
        let layer = self
            .create_user_layer(scope, command.ord, attribution)
            .await?;
        Ok(material_layer_to_dto(&layer))
    }

    /// Chooses the band a surface shows.
    ///
    /// Returns nothing on purpose: the flag moves off whichever band
    /// held it, so the row a caller would be handed back is only half of
    /// what changed, and a surface that patched that half would go on
    /// showing two defaults.
    pub async fn set_default_layer(
        &self,
        command: SetDefaultMaterialLayerCommand,
        attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let layer_id = parse_material_layer_id(&command.layer_id)?;
        self.set_default(&layer_id, attribution).await
    }

    /// Removes a band the person owns, with everything in it.
    pub async fn delete_layer(
        &self,
        command: DeleteMaterialLayerCommand,
        attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let layer_id = parse_material_layer_id(&command.layer_id)?;
        self.delete_user_layer(&layer_id, attribution).await
    }

    /// Adds a section to a band the person owns.
    pub async fn post_chapter_mark(
        &self,
        command: PostChapterMarkCommand,
        attribution: &AttributionContext,
    ) -> Result<ChapterMarkDto, DomainError> {
        let layer_id = parse_material_layer_id(&command.layer_id)?;
        let span = parse_timeline_span(command.start_ms, command.end_ms)?;
        let chapter = self
            .post_chapter(&layer_id, span, command.label, command.ord, attribution)
            .await?;
        Ok(chapter_mark_to_dto(&chapter))
    }

    /// Rewrites one section of a band the person owns.
    ///
    /// `end_ms` is read only when `start_ms` is present, which is what
    /// keeps "leave the section where it is" (`start_ms: None`) apart
    /// from "move it and give it no stated end" (`start_ms: Some`,
    /// `end_ms: None`). Two absent fields would otherwise be one
    /// request.
    pub async fn edit_chapter_mark(
        &self,
        command: EditChapterMarkCommand,
        attribution: &AttributionContext,
    ) -> Result<ChapterMarkDto, DomainError> {
        let layer_id = parse_material_layer_id(&command.layer_id)?;
        let chapter_id = parse_chapter_mark_id(&command.chapter_id)?;
        let span = command
            .start_ms
            .map(|start_ms| parse_timeline_span(start_ms, command.end_ms))
            .transpose()?;
        let chapter = self
            .edit_chapter(
                &layer_id,
                &chapter_id,
                command.label,
                span,
                command.ord,
                attribution,
            )
            .await?;
        Ok(chapter_mark_to_dto(&chapter))
    }

    /// Removes one section from a band the person owns.
    pub async fn delete_chapter_mark(
        &self,
        command: DeleteChapterMarkCommand,
        attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let layer_id = parse_material_layer_id(&command.layer_id)?;
        let chapter_id = parse_chapter_mark_id(&command.chapter_id)?;
        self.delete_chapter(&layer_id, &chapter_id, attribution)
            .await
    }
}

/// Refuses a write into a band the person does not own.
///
/// The message names the origin and the act, because the caller's next
/// question is always "then where do I put it" and the answer depends
/// on which of the two it is: an imported band is replaced by re-reading
/// the file, a machine band by re-running the job.
fn require_user_owned(layer: &MaterialLayer, act: &str) -> Result<(), DomainError> {
    match layer.origin {
        LayerOrigin::User => Ok(()),
        LayerOrigin::Imported => Err(DomainError::Validation(format!(
            "layer {} was read out of the material, so a caller may not {act} it; \
             its contents are replaced by reading the material again",
            layer.id
        ))),
        LayerOrigin::Machine => Err(DomainError::Validation(format!(
            "layer {} was derived by a job, so a caller may not {act} it; \
             its contents are replaced by running that job again",
            layer.id
        ))),
    }
}

/// Refuses a chapter verb aimed at an annotation band.
///
/// The two roles hold different aggregates, so this is not a
/// preference: `chapter_mark` rows in an annotation band would be
/// invisible to every reader of that band, which reads
/// `material_mark`.
fn require_structure(layer: &MaterialLayer) -> Result<(), DomainError> {
    if layer.role != LayerRole::Structure {
        return Err(DomainError::Validation(format!(
            "layer {} holds notes, not chapters — post a material mark instead",
            layer.id
        )));
    }
    Ok(())
}

/// The band a note lands in when the caller names none: the default
/// annotation layer of `(asset_id, material_ord)`, created if the asset
/// has none yet.
///
/// Placing a mark does not choose a band — a person clicking a timeline
/// is answering "where", not "in which of my passes over this file" —
/// so somebody has to resolve one, and an asset that has never been
/// marked has no bands at all. Creating it lazily here rather than at
/// ingest means the row exists for the assets that have marks and for
/// no others: a library of a hundred thousand images does not carry a
/// hundred thousand empty bands to make one code path shorter.
///
/// # The race, and why it ends in a re-read rather than an error
///
/// Two posts against an unmarked asset can both find no default and
/// both try to create one. The schema's partial unique index refuses
/// the second insert, which is what keeps two defaults from existing —
/// but the caller that lost is a person who clicked twice, and the
/// second note is not less real than the first. So the loser re-reads
/// and uses the band the winner made. The re-read is the only thing
/// that can answer it: the losing writer holds an id that is not in the
/// table.
pub async fn default_annotation_layer(
    layers: &dyn MaterialLayerRepository,
    asset_id: &AssetId,
    material_ord: u32,
) -> Result<MaterialLayer, DomainError> {
    let scope = LayerScope {
        asset_id: *asset_id,
        material_ord,
        role: LayerRole::Annotation,
    };
    if let Some(existing) = find_default(layers, scope).await? {
        return Ok(existing);
    }
    let fresh = MaterialLayer::new(
        *asset_id,
        material_ord,
        LayerOrigin::User,
        LayerRole::Annotation,
        true,
        0,
    )?;
    match layers.save(&fresh).await {
        Ok(()) => Ok(fresh),
        Err(lost) => find_default(layers, scope).await?.ok_or(lost),
    }
}

/// The band a re-read of the material writes into: the imported
/// structure layer of `(asset_id, material_ord)`, created if there is
/// none.
///
/// Keyed on the origin rather than on the default flag, which is the
/// difference from [`default_annotation_layer`]: there is exactly one
/// band that *is* the file's own chapter list, whether or not it is the
/// one currently displayed. Choosing it by "whichever structure band is
/// default" would make a re-probe overwrite the person's own chapters
/// as soon as they had selected them.
///
/// The first imported band an asset gets is also made the default,
/// since a file that declares chapters is the best available answer
/// until a person says otherwise; a later one is not, because by then
/// something else already holds the flag and moving it is
/// [`MaterialLayerService::set_default`]'s job.
pub async fn imported_structure_layer(
    layers: &dyn MaterialLayerRepository,
    asset_id: &AssetId,
    material_ord: u32,
) -> Result<MaterialLayer, DomainError> {
    let scope = LayerScope {
        asset_id: *asset_id,
        material_ord,
        role: LayerRole::Structure,
    };
    let bands = in_scope(layers, scope).await?;
    if let Some(existing) = bands.iter().find(|l| l.origin == LayerOrigin::Imported) {
        return Ok(existing.clone());
    }
    let claim_default = !bands.iter().any(|l| l.is_default);
    let fresh = MaterialLayer::new(
        *asset_id,
        material_ord,
        LayerOrigin::Imported,
        LayerRole::Structure,
        claim_default,
        0,
    )?;
    match layers.save(&fresh).await {
        Ok(()) => Ok(fresh),
        Err(lost) => in_scope(layers, scope)
            .await?
            .into_iter()
            .find(|l| l.origin == LayerOrigin::Imported)
            .ok_or(lost),
    }
}

/// The layers of one `(asset, material, role)` triple.
async fn in_scope(
    layers: &dyn MaterialLayerRepository,
    scope: LayerScope,
) -> Result<Vec<MaterialLayer>, DomainError> {
    Ok(layers
        .list_by_asset(&scope.asset_id)
        .await?
        .into_iter()
        .filter(|l| l.material_ord == scope.material_ord && l.role == scope.role)
        .collect())
}

/// The default band of one triple, if the asset has one.
async fn find_default(
    layers: &dyn MaterialLayerRepository,
    scope: LayerScope,
) -> Result<Option<MaterialLayer>, DomainError> {
    Ok(in_scope(layers, scope)
        .await?
        .into_iter()
        .find(|l| l.is_default))
}

/// The material a timeline mark addresses.
///
/// Every current surface marks the primary original, so this is the
/// ordinal the mark path resolves layers against. Written once here so
/// that the day a surface marks a second original, the call sites that
/// have to grow an argument are a compile error rather than a search.
pub const MARKED_MATERIAL_ORD: u32 = PRIMARY_MATERIAL_ORD;
