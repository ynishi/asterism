//! Tauri command handlers — a thin translation layer. They pass DTOs
//! through to the application services in `asterism-core` and convert
//! `DomainError` into `UiError`. No business logic lives here.
//!
//! Every mutation here names its attribution channel explicitly —
//! [`AttributionContext::owner_surface`]. This is the owner's own
//! operation surface (the desktop app's IPC), so the owner-ness is a
//! property of the surface rather than a guess about the caller, and the
//! commands carry no attribution fields for it to read. The argument is
//! required by the service signatures, so a new mutation cannot be added
//! here without choosing.

use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AddAssetToGroupCommand,
    AppendMessageCommand, ArchivePersonaCommand, ArchiveThreadCommand, AttachTagBatchCommand,
    AttachTagBatchResult, AttachTagCommand, BatchGroupMembershipCommand, ClosePursuitCommand,
    CreateDirCommand, CreateDispatchCommand, CreateGroupCommand, CreateMaterialLayerCommand,
    CreateModalityCommand, CreateQueryGroupCommand, CreateSnapshotCommand, CreateThreadCommand,
    DeleteAssetCommentCommand, DeleteChapterMarkCommand, DeleteDirCommand,
    DeleteMaterialLayerCommand, DeleteMaterialMarkCommand, DeleteMessageCommand,
    DeleteModalityCommand, DeletePersonaProfileCommand, DeletePersonaThemeCommand,
    DeleteSessionCommand, DeleteThreadCommand, DetachTagBatchCommand, DetachTagBatchResult,
    DetachTagCommand, DispatchRunCommand, EditAssetCommentCommand, EditChapterMarkCommand,
    EditMaterialMarkCommand, EmptyTrashCommand, EmptyTrashResult, LinkGroupCommand,
    MergeAssetsCommand, MergeGroupsCommand, MoveDirCommand, MoveGroupToDirCommand,
    OpenPursuitCommand, PasteImageImportCommand, PatchSessionMetadataCommand,
    PostAssetCommentCommand, PostChapterMarkCommand, PostMaterialMarkCommand,
    PromoteSnapshotToGroupCommand, PromoteSnapshotToGroupResult, PromoteTagToGroupCommand,
    PromoteTagToGroupResult, PromoteVolatileSelectionCommand, PurgeAssetCommand, PurgeGroupCommand,
    PurgePersonaCommand, RedispatchCommand, RegisterPersonaCommand, RemoveAssetFromGroupCommand,
    RenameDirCommand, RenameGroupCommand, RenameSessionCommand, ReopenPursuitCommand,
    ReorderGroupAssetsCommand, ReorderGroupChildrenCommand, ReorderPersonasCommand,
    ResetSettingCommand, ResolveDuplicateConflictCommand, RestoreAssetCommand, RestoreGroupCommand,
    RestorePersonaCommand, SetDefaultMaterialLayerCommand, SetPersonaProfileCommand,
    SetPersonaThemeCommand, SetSettingCommand, TrashAssetCommand, TrashGroupCommand,
    TrashPersonaCommand, UnlinkGroupCommand, UpdateAssetMetaBatchCommand,
    UpdateAssetMetaBatchResult, UpdateAssetMetaCommand, UpdateModalityCommand,
    UpdateQueryGroupQueryCommand,
};
use asterism_contract::dto::{
    AssetCardDto, AssetCommentDto, AssetCountEntryDto, AssetDetailDto, AssetDto, AssetPageDto,
    AssetTextDto, ChapterMarkDto, ConstellationItemDto, DirDto, DispatchDto, DuplicateConflictDto,
    DuplicateReportDto, DuplicateResolutionDto, EdgeDto, GroupDto, GroupLinkDto, GroupSummaryDto,
    MaterialLayerDto, MaterialLayerViewDto, MaterialMarkDto, MergeAssetsDto, MessageDto,
    ModalityDefDto, PersonaDto, PersonaProfileDto, PersonaThemeDto, PursuitDto, PursuitEventDto,
    PursuitViewDto, RetrievedPageDto, SessionDto, SessionPageDto, SettingDto, SnapshotDto,
    TagCountDto, TagDto, ThreadDto,
};
use asterism_contract::query::{GetAssetDetailQuery, ListAssetsQuery, SearchAssetsQuery};
use asterism_core::domain::attribution::AttributionContext;
use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

/// Returns the active local data profile for persistent UI chrome.
#[tauri::command]
pub fn active_profile() -> Result<String, UiError> {
    asterism_infra::paths::active_profile()
        .map(|profile| profile.as_str().to_string())
        .map_err(UiError::from)
}

/// Lists every persona (used to render the sidebar).
#[tauri::command]
pub async fn list_personas(state: State<'_, AppState>) -> Result<Vec<PersonaDto>, UiError> {
    Ok(state.persona_service.list().await?)
}

/// Sidebar Persona counts — `(persona_id, asset_count)` per persona
/// that owns at least one asset. Ordered by count DESC then uuid ASC.
#[tauri::command]
pub async fn list_persona_asset_counts(
    state: State<'_, AppState>,
    trash: Option<String>,
) -> Result<Vec<AssetCountEntryDto>, UiError> {
    Ok(state
        .asset_service
        .list_persona_asset_counts(trash.as_deref())
        .await?)
}

/// Sidebar Modality counts — `(modality_slug, asset_count)`, optionally
/// scoped to one persona (`None` = cross-persona total).
#[tauri::command]
pub async fn list_modality_asset_counts(
    state: State<'_, AppState>,
    persona_id: Option<String>,
    trash: Option<String>,
) -> Result<Vec<AssetCountEntryDto>, UiError> {
    Ok(state
        .asset_service
        .list_modality_asset_counts(persona_id.as_deref(), trash.as_deref())
        .await?)
}

/// Sidebar FORMAT facet counts (asset-model v4) — `(format, count)`
/// per mime top-level type (`image` / `video` / `audio` / `text`) on
/// top-level assets' primary materials, optionally persona-scoped.
#[tauri::command]
pub async fn list_format_asset_counts(
    state: State<'_, AppState>,
    persona_id: Option<String>,
    trash: Option<String>,
) -> Result<Vec<AssetCountEntryDto>, UiError> {
    Ok(state
        .asset_service
        .list_format_asset_counts(persona_id.as_deref(), trash.as_deref())
        .await?)
}

/// Sidebar COLOR facet counts — `(bucket, count)` per palette swatch
/// (`red` / `blue` / `white` / …) carried by a top-level asset, in
/// swatch order, optionally persona-scoped. Swatches nothing carries
/// are omitted.
#[tauri::command]
pub async fn list_color_asset_counts(
    state: State<'_, AppState>,
    persona_id: Option<String>,
    trash: Option<String>,
) -> Result<Vec<AssetCountEntryDto>, UiError> {
    Ok(state
        .asset_service
        .list_color_asset_counts(persona_id.as_deref(), trash.as_deref())
        .await?)
}

/// Duplicate report — sets of live assets sharing a fingerprint on
/// `axis` (`"artefact"` by default, `"content"` for the bytes that
/// decide the decoded result), optionally persona-scoped.
///
/// The counts ride along so the panel never has to present an empty
/// report as a clean bill of health: `unhashed_count` is "still
/// looking", `unreadable_count` is "these originals could not be read
/// when the pass tried, and the number will not move until the files
/// come back", `unwalked_count` is "the content axis has no reading of
/// these files, because the migration that fills the column in could not
/// open their originals".
#[tauri::command]
pub async fn list_duplicate_groups(
    state: State<'_, AppState>,
    persona_id: Option<String>,
    axis: Option<String>,
    limit: Option<u32>,
) -> Result<DuplicateReportDto, UiError> {
    Ok(state
        .asset_service
        .list_duplicate_groups(persona_id.as_deref(), axis.as_deref(), limit)
        .await?)
}

/// The duplicate questions still waiting on a person, newest first,
/// both sides hydrated as cards.
///
/// Distinct from `list_duplicate_groups`, which keeps reporting a pair
/// that has been ruled to be two separate things: this is the panel's
/// work list.
#[tauri::command]
pub async fn list_duplicate_conflicts(
    state: State<'_, AppState>,
    persona_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<DuplicateConflictDto>, UiError> {
    Ok(state
        .asset_service
        .list_duplicate_conflicts(persona_id.as_deref(), limit)
        .await?)
}

/// Answers one duplicate question — `folded` (queues the fold onto
/// `keeper_id`, which must be one of the pair) or `kept` (both rows
/// stay). Refused when the row was already answered or when either
/// side has since been folded away or trashed.
#[tauri::command]
pub async fn resolve_duplicate_conflict(
    state: State<'_, AppState>,
    command: ResolveDuplicateConflictCommand,
) -> Result<DuplicateResolutionDto, UiError> {
    Ok(state
        .asset_service
        .resolve_duplicate_conflict(command, &AttributionContext::owner_surface())
        .await?)
}

/// The manual merge verb: a person's ruling that a set of rows is one
/// thing, carried out. Not scoped to the queue — the caller declares
/// the whole set (keeper + discards + the members they saw on screen)
/// and the verb folds the discards into the keeper.
///
/// `command.dry_run: true` returns a preview and writes nothing;
/// `false` commits the fold. Both branches return the same
/// [`MergeAssetsDto`] shape (a run following a preview reads the
/// answer back on it); [`MergeAssetsDto::committed`] tells them apart.
/// `refusals` and `warnings` ride on the response — a refused pair is
/// not an error but a decision the caller has to re-make, and a
/// warning is what a rule was protecting before the caller confirms.
#[tauri::command]
pub async fn merge_assets(
    state: State<'_, AppState>,
    command: MergeAssetsCommand,
) -> Result<MergeAssetsDto, UiError> {
    Ok(state
        .asset_service
        .merge_assets(command, &AttributionContext::owner_surface())
        .await?)
}

/// Modality master listing — one row per registered modality (hidden
/// included), ordered by `sort_order` then `slug`, each carrying its
/// live asset count. Drives the sidebar axis + `slug → kind` catalog.
#[tauri::command]
pub async fn list_modalities(state: State<'_, AppState>) -> Result<Vec<ModalityDefDto>, UiError> {
    Ok(state.modality_service.list().await?)
}

/// Registers a new modality master row.
#[tauri::command]
pub async fn create_modality(
    state: State<'_, AppState>,
    command: CreateModalityCommand,
) -> Result<ModalityDefDto, UiError> {
    Ok(state
        .modality_service
        .create(command, &AttributionContext::owner_surface())
        .await?)
}

/// Partially updates a modality master row (each omitted field is left
/// unchanged). Used by drag-reorder (`sort_order`) and the settings UI.
#[tauri::command]
pub async fn update_modality(
    state: State<'_, AppState>,
    command: UpdateModalityCommand,
) -> Result<ModalityDefDto, UiError> {
    Ok(state
        .modality_service
        .update(command, &AttributionContext::owner_surface())
        .await?)
}

/// Deletes a modality master row — only when no asset carries the slug
/// (`409 Conflict` otherwise; hide it instead).
#[tauri::command]
pub async fn delete_modality(
    state: State<'_, AppState>,
    command: DeleteModalityCommand,
) -> Result<(), UiError> {
    state
        .modality_service
        .delete(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Every known application setting, resolved through code default →
/// environment variable → stored row (last wins). Each row reports the
/// layer that supplied it (`source`), the whole chain it came from
/// (`layers`), and the registry metadata, so the settings panel can
/// render a control and show where its value came from without a
/// second call.
#[tauri::command]
pub async fn list_settings(state: State<'_, AppState>) -> Result<Vec<SettingDto>, UiError> {
    Ok(state.app_setting_service.list().await?)
}

/// Stores one setting override and returns the value that now applies.
///
/// A successful write always resolves to `source: "stored"` — nothing
/// outranks the user's choice. The resolved row is returned rather than
/// the input so the caller also gets the refreshed `layers` chain.
/// Rejects a value that does not match the key's declared kind or falls
/// outside its range, and an unknown key.
#[tauri::command]
pub async fn set_setting(
    state: State<'_, AppState>,
    command: SetSettingCommand,
) -> Result<SettingDto, UiError> {
    Ok(state
        .app_setting_service
        .set(command, &AttributionContext::owner_surface())
        .await?)
}

/// Clears one setting override and returns the value that now applies.
/// Idempotent — resetting a key that was never overridden succeeds.
///
/// Like [`set_setting`], the response is the *resolved* value — here,
/// the layer directly beneath the row just removed: `"env"` when the
/// key's variable is exported and usable, `"default"` otherwise.
/// Clearing the stored row does not clear the environment.
#[tauri::command]
pub async fn reset_setting(
    state: State<'_, AppState>,
    command: ResetSettingCommand,
) -> Result<SettingDto, UiError> {
    Ok(state
        .app_setting_service
        .reset(command, &AttributionContext::owner_surface())
        .await?)
}

/// Registers a new persona.
#[tauri::command]
pub async fn register_persona(
    state: State<'_, AppState>,
    command: RegisterPersonaCommand,
) -> Result<PersonaDto, UiError> {
    Ok(state
        .persona_service
        .register(command, &AttributionContext::owner_surface())
        .await?)
}

/// Toggles a persona's archive flag.
#[tauri::command]
pub async fn archive_persona(
    state: State<'_, AppState>,
    command: ArchivePersonaCommand,
) -> Result<PersonaDto, UiError> {
    Ok(state
        .persona_service
        .set_archived(command, &AttributionContext::owner_surface())
        .await?)
}

/// Rewrites `display_order` across a persona slice.
#[tauri::command]
pub async fn reorder_personas(
    state: State<'_, AppState>,
    command: ReorderPersonasCommand,
) -> Result<(), UiError> {
    state
        .persona_service
        .reorder(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Moves a persona and every asset it holds to the trash (reversible).
#[tauri::command]
pub async fn trash_persona(
    state: State<'_, AppState>,
    command: TrashPersonaCommand,
) -> Result<(), UiError> {
    Ok(state
        .persona_service
        .trash(command, &AttributionContext::owner_surface())
        .await?)
}

/// Returns a trashed persona and the assets that went with it.
#[tauri::command]
pub async fn restore_persona(
    state: State<'_, AppState>,
    command: RestorePersonaCommand,
) -> Result<(), UiError> {
    Ok(state
        .persona_service
        .restore(command, &AttributionContext::owner_surface())
        .await?)
}

/// Permanently deletes an already-trashed persona and everything it
/// holds. Conflicts when the persona is still live.
#[tauri::command]
pub async fn purge_persona(
    state: State<'_, AppState>,
    command: PurgePersonaCommand,
) -> Result<(), UiError> {
    Ok(state
        .persona_service
        .purge(command, &AttributionContext::owner_surface())
        .await?)
}

/// Fetches the persona's UI chrome (wallpaper reference). `None`
/// means no custom theme is set.
#[tauri::command]
pub async fn get_persona_theme(
    state: State<'_, AppState>,
    persona_id: String,
) -> Result<Option<PersonaThemeDto>, UiError> {
    Ok(state.persona_service.get_theme(&persona_id).await?)
}

/// Sets (or clears) the wallpaper for a persona.
#[tauri::command]
pub async fn set_persona_theme(
    state: State<'_, AppState>,
    command: SetPersonaThemeCommand,
) -> Result<PersonaThemeDto, UiError> {
    Ok(state
        .persona_service
        .set_theme(command, &AttributionContext::owner_surface())
        .await?)
}

/// Removes the persona theme row entirely (reverts to defaults).
#[tauri::command]
pub async fn delete_persona_theme(
    state: State<'_, AppState>,
    command: DeletePersonaThemeCommand,
) -> Result<(), UiError> {
    state
        .persona_service
        .delete_theme(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Fetches the persona's identity signal (avatar / bio / role).
/// `None` means no profile row is set.
#[tauri::command]
pub async fn get_persona_profile(
    state: State<'_, AppState>,
    persona_id: String,
) -> Result<Option<PersonaProfileDto>, UiError> {
    Ok(state.persona_service.get_profile(&persona_id).await?)
}

/// Upserts the persona's identity signal.
#[tauri::command]
pub async fn set_persona_profile(
    state: State<'_, AppState>,
    command: SetPersonaProfileCommand,
) -> Result<PersonaProfileDto, UiError> {
    Ok(state
        .persona_service
        .set_profile(command, &AttributionContext::owner_surface())
        .await?)
}

/// Removes the persona profile row entirely.
#[tauri::command]
pub async fn delete_persona_profile(
    state: State<'_, AppState>,
    command: DeletePersonaProfileCommand,
) -> Result<(), UiError> {
    state
        .persona_service
        .delete_profile(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Ingests an asset (entry point for the asset-add pipeline).
#[tauri::command]
pub async fn add_asset(
    state: State<'_, AppState>,
    command: AddAssetCommand,
) -> Result<AssetDto, UiError> {
    Ok(state
        .asset_service
        .add(command, &AttributionContext::owner_surface())
        .await?)
}

/// Rehomes a dropped path into `~/Pictures/Asterism/dropped/`
/// when it lives under a volatile TEMP dir (macOS screenshot
/// stash lands in `/private/var/folders/…/TemporaryItems/`, and
/// screenshots dragged out of the preview thumbnail vanish once
/// the preview closes — a stale locator would then break every
/// downstream read). The copy also drops the path into the Tauri
/// `assetProtocol` scope so `convertFileSrc` works for full-window
/// image playback. Returns the durable path; the frontend feeds it
/// straight into `add_asset`.
#[tauri::command]
pub async fn rehome_dropped_path(source: String) -> Result<String, UiError> {
    fn is_temp(p: &str) -> bool {
        let heads = [
            "/private/tmp/",
            "/tmp/",
            "/private/var/folders/",
            "/var/folders/",
        ];
        heads.iter().any(|h| p.starts_with(h)) || p.contains("/TemporaryItems/")
    }
    if !is_temp(&source) {
        return Ok(source);
    }
    let src_path = std::path::PathBuf::from(&source);
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let stem = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("drop")
        .to_string();
    let home = std::env::var("HOME").map_err(|e| {
        UiError::from(asterism_core::error::DomainError::Validation(format!(
            "HOME env not set: {e}"
        )))
    })?;
    let base = std::path::PathBuf::from(&home).join("asterism/dropped");
    tokio::fs::create_dir_all(&base).await.map_err(|e| {
        UiError::from(asterism_core::error::DomainError::Validation(format!(
            "cannot create drop dir: {e}"
        )))
    })?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f");
    let target = base.join(format!("{stem}-{ts}.{ext}"));
    tokio::fs::copy(&src_path, &target).await.map_err(|e| {
        UiError::from(asterism_core::error::DomainError::Validation(format!(
            "cannot copy drop file {source}: {e}"
        )))
    })?;
    Ok(target.to_string_lossy().into_owned())
}

/// Writes a clipboard-pasted image blob to
/// `~/Pictures/Asterism/pasted/paste-<ts>.<ext>` and dispatches
/// `add_asset` for it. The MIME-type hint picks the extension; the
/// downstream pipeline still sniffs the actual container so a
/// slightly-wrong `image/webp` label does not stop cover_gen /
/// thumb_gen from producing previews.
#[tauri::command]
pub async fn paste_image_import(
    state: State<'_, AppState>,
    command: PasteImageImportCommand,
) -> Result<AssetDto, UiError> {
    use asterism_contract::command::AddAssetCommand;

    let ext = match command.mime_type.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/heic" => "heic",
        "image/tiff" => "tiff",
        "image/bmp" => "bmp",
        _ => "png",
    };

    let home = std::env::var("HOME").map_err(|e| {
        UiError::from(asterism_core::error::DomainError::Validation(format!(
            "HOME env not set: {e}"
        )))
    })?;
    let base = std::path::PathBuf::from(home).join("asterism/pasted");
    tokio::fs::create_dir_all(&base).await.map_err(|e| {
        UiError::from(asterism_core::error::DomainError::Validation(format!(
            "cannot create paste dir: {e}"
        )))
    })?;

    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%d-%H%M%S-%3f");
    let path = base.join(format!("paste-{ts}.{ext}"));
    let size = command.bytes.len() as u64;
    tokio::fs::write(&path, &command.bytes).await.map_err(|e| {
        UiError::from(asterism_core::error::DomainError::Validation(format!(
            "cannot write paste file: {e}"
        )))
    })?;

    let add = AddAssetCommand {
        persona_id: command.persona_id,
        source_kind: "fs".into(),
        locator: path.to_string_lossy().into_owned(),
        // Unclassified (asset-model v4): "is an image" is a data
        // format, captured on the material layer from the file
        // extension — not a semantic classification.
        modality: None,
        occurred_at_ms: now.timestamp_millis(),
        session_id: None,
        // `external_session_key` was added alongside `session_id`;
        // pasted images are ad-hoc (no importer key) so both stay
        // None.
        external_session_key: None,
        external_key: None,
        bundle_id: None,
        labels: vec!["pasted".into()],
        register_note: None,
        platform: None,
        file_size_bytes: Some(size),
        duration_ms: None,
        // Unmeasured: this command holds the bytes but never decodes
        // them (the extension comes from the clipboard MIME hint), so it
        // has nothing to state. The importer pipeline sniffs the stored
        // file afterwards, which is the road that measures.
        width_px: None,
        height_px: None,
        extra_json: None,
        cover_hint: None,
        auto_organize_base_dir: None,
        // A clipboard paste has no declarable origin: whatever the
        // image came from, the clipboard did not carry it.
        derived_from: None,
        // The three assertion fields stay empty on purpose: they are how
        // a *remote* caller states an attribution, and this request did
        // not come from one. The answer is carried by the context below
        // instead — the paste arrived through the owner's own surface,
        // which is a fact about the entry point rather than a guess
        // about who typed it. Setting these here as well would be
        // refused (an assertion cannot arrive on the owner's surface).
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        // Undeclared, on the same grounds: `PasteImageImportCommand`
        // carries no strategy, so choosing one here would be the server
        // deciding what the caller meant. `add_asset` above needs no
        // such line — it forwards the whole command, so a UI that grows
        // a picker reaches the field without this arm changing.
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    };
    Ok(state
        .asset_service
        .add(add, &AttributionContext::owner_surface())
        .await?)
}

/// Ingests a batch of assets (bulk form of `add_asset`).
#[tauri::command]
pub async fn add_asset_batch(
    state: State<'_, AppState>,
    command: AddAssetBatchCommand,
) -> Result<AddAssetBatchResult, UiError> {
    Ok(state
        .asset_service
        .add_batch(command, &AttributionContext::owner_surface())
        .await?)
}

/// Partially updates an asset's metadata.
#[tauri::command]
pub async fn update_asset_meta(
    state: State<'_, AppState>,
    command: UpdateAssetMetaCommand,
) -> Result<AssetDto, UiError> {
    Ok(state
        .asset_service
        .update_meta(command, &AttributionContext::owner_surface())
        .await?)
}

/// Partially updates metadata for multiple assets in one call.
#[tauri::command]
pub async fn update_asset_meta_batch(
    state: State<'_, AppState>,
    command: UpdateAssetMetaBatchCommand,
) -> Result<UpdateAssetMetaBatchResult, UiError> {
    Ok(state
        .asset_service
        .update_meta_batch(command, &AttributionContext::owner_surface())
        .await?)
}

/// Moves an asset to the trash (reversible).
#[tauri::command]
pub async fn trash_asset(
    state: State<'_, AppState>,
    command: TrashAssetCommand,
) -> Result<(), UiError> {
    Ok(state
        .asset_service
        .trash(command, &AttributionContext::owner_surface())
        .await?)
}

/// Returns a trashed asset to the live set.
#[tauri::command]
pub async fn restore_asset(
    state: State<'_, AppState>,
    command: RestoreAssetCommand,
) -> Result<(), UiError> {
    Ok(state
        .asset_service
        .restore(command, &AttributionContext::owner_surface())
        .await?)
}

/// Permanently deletes an already-trashed asset. Conflicts when the
/// asset is still live — trash it first.
#[tauri::command]
pub async fn purge_asset(
    state: State<'_, AppState>,
    command: PurgeAssetCommand,
) -> Result<(), UiError> {
    Ok(state
        .asset_service
        .purge(command, &AttributionContext::owner_surface())
        .await?)
}

/// Permanently deletes every asset in the trash. Irreversible, and
/// deliberately unfiltered — see `EmptyTrashCommand`.
#[tauri::command]
pub async fn empty_trash(
    state: State<'_, AppState>,
    command: EmptyTrashCommand,
) -> Result<EmptyTrashResult, UiError> {
    Ok(state
        .asset_service
        .empty_trash(command, &AttributionContext::owner_surface())
        .await?)
}

/// Lists assets for the grid (returns `AssetCard` projections).
#[tauri::command]
pub async fn list_assets(
    state: State<'_, AppState>,
    query: ListAssetsQuery,
) -> Result<AssetPageDto, UiError> {
    Ok(state.asset_service.list(query).await?)
}

/// Index-only grid listing for 6-figure result sets. Returns
/// `AssetIndex` projections — no cover text / source locator /
/// file size — so the frontend can eager-load the full ordering
/// and virtualised scroll without paying the card-serialisation
/// cost. The visible viewport is hydrated separately via
/// `hydrate_cards`.
#[tauri::command]
pub async fn list_asset_index(
    state: State<'_, AppState>,
    query: ListAssetsQuery,
) -> Result<asterism_contract::dto::AssetIndexPageDto, UiError> {
    Ok(state.asset_service.list_index(query).await?)
}

/// Batch-hydrates cards by id. Companion to `list_asset_index` —
/// the frontend calls this for the ~40 rows the VList is about
/// to paint plus a small prefetch window. Ids that don't exist
/// or are hidden from the viewer drop out of the response.
#[tauri::command]
pub async fn hydrate_cards(
    state: State<'_, AppState>,
    ids: Vec<String>,
    viewer_subject: Option<String>,
) -> Result<Vec<asterism_contract::dto::AssetCardDto>, UiError> {
    Ok(state
        .asset_service
        .hydrate_cards(ids, viewer_subject)
        .await?)
}

/// Full-text / fuzzy search.
#[tauri::command]
pub async fn search_assets(
    state: State<'_, AppState>,
    query: SearchAssetsQuery,
) -> Result<RetrievedPageDto, UiError> {
    Ok(state.asset_service.search(query).await?)
}

/// The same retrieval as `search_assets`, reduced to the rank order.
///
/// Feeds the grid's `✦ Relevance` sort: the page itself comes from the
/// exact (Query-side) read, and this only says which of those rows the
/// retriever considers the better match. The ids are an ordering hint,
/// not a membership answer.
///
/// **Not exposed over MCP on purpose.** An agent asking about relevance
/// already has `asset_search`, which hands back ranked cards; an
/// ids-only reply would only be useful to something that is already
/// painting a grid it fetched by other means.
#[tauri::command]
pub async fn search_asset_ids(
    state: State<'_, AppState>,
    query: SearchAssetsQuery,
) -> Result<asterism_contract::dto::RetrievedIdsDto, UiError> {
    Ok(state.asset_service.search_ids(query).await?)
}

/// A random handful out of the current filter — the sidebar's
/// "🎲 Random".
///
/// The grid shows the picks in place of its listing, so nothing about
/// the answer is a page: it cannot be scrolled past `k`, and asking
/// again answers differently. That is the feature.
///
/// **Not exposed over MCP on purpose.** The picks exist to give a person
/// something to look at when they do not know what they are looking for;
/// an agent asking for assets under a filter wants `asset_list`, which
/// enumerates, counts, and can be paged.
#[tauri::command]
pub async fn random_assets(
    state: State<'_, AppState>,
    query: asterism_contract::query::RandomAssetsQuery,
) -> Result<asterism_contract::dto::SampledPageDto, UiError> {
    Ok(state.asset_service.sample(query).await?)
}

/// Detail view (asset + attached tags + constellation edges).
#[tauri::command]
pub async fn asset_detail(
    state: State<'_, AppState>,
    query: GetAssetDetailQuery,
) -> Result<AssetDetailDto, UiError> {
    Ok(state.asset_service.detail(query).await?)
}

/// Returns the top-`limit` edges (by weight) for hover-burst
/// rendering.
#[tauri::command]
pub async fn asset_edges(
    state: State<'_, AppState>,
    asset_id: String,
    kind: Option<String>,
    limit: u32,
) -> Result<Vec<EdgeDto>, UiError> {
    Ok(state
        .asset_service
        .edges_of(&asset_id, kind.as_deref(), limit)
        .await?)
}

/// Returns the fully-resolved hover-burst payload — each edge with
/// the card it lands on, already filtered by viewer visibility.
#[tauri::command]
pub async fn asset_constellation(
    state: State<'_, AppState>,
    asset_id: String,
    viewer_subject: Option<String>,
    limit: u32,
) -> Result<Vec<ConstellationItemDto>, UiError> {
    Ok(state
        .asset_service
        .constellation_of(&asset_id, viewer_subject.as_deref(), limit)
        .await?)
}

/// Returns the 1-hop `derived_from` lineage around the asset —
/// ancestors (Selection inputs this asset was derived from) and
/// descendants (assets later derived from this one). Powers the
/// detail-pane Provenance section.
#[tauri::command]
pub async fn asset_provenance(
    state: State<'_, AppState>,
    asset_id: String,
    viewer_subject: Option<String>,
    limit: u32,
) -> Result<asterism_contract::dto::ProvenanceViewDto, UiError> {
    Ok(state
        .asset_service
        .provenance_of(&asset_id, viewer_subject.as_deref(), limit)
        .await?)
}

/// Walks the whole `derived_from` chain around the asset, not just
/// its immediate neighbours — the read side of correlation ingest.
///
/// `depth` is hops in each direction (clamped server-side); the
/// response says whether it had to stop early, and carries the
/// dispatch ids the chain passed through as its backbone.
#[tauri::command]
pub async fn asset_lineage(
    state: State<'_, AppState>,
    asset_id: String,
    viewer_subject: Option<String>,
    depth: u32,
) -> Result<asterism_contract::dto::LineageViewDto, UiError> {
    Ok(state
        .asset_service
        .lineage_of(&asset_id, viewer_subject.as_deref(), depth)
        .await?)
}

/// Declares (or repairs) an asset's origin after the fact — the
/// command twin of `POST /asterism/assets/{id}/provenance`. Same
/// `derived_from` vocabulary as ingest; an unresolvable claim is
/// recorded on `extra._trace`, not rejected.
#[tauri::command]
pub async fn asset_declare_provenance(
    state: State<'_, AppState>,
    command: asterism_contract::command::DeclareProvenanceCommand,
) -> Result<asterism_contract::dto::AssetDto, UiError> {
    Ok(state
        .asset_service
        .declare_provenance(command, &AttributionContext::owner_surface())
        .await?)
}

/// Records — or removes — one AlbumMeta statement on an asset: the
/// command twin of `POST /asterism/assets/{id}/album-meta`.
///
/// The statement's own `operator_ai` travels on the command and is a
/// different subject from who the asset is by, which is why the
/// attribution handed to the service is still the owner surface. Saying
/// something about a row is not authoring it.
#[tauri::command]
pub async fn asset_declare_meta(
    state: State<'_, AppState>,
    command: asterism_contract::command::DeclareAssetMetaCommand,
) -> Result<asterism_contract::dto::AssetDto, UiError> {
    Ok(state
        .asset_service
        .declare_asset_meta(command, &AttributionContext::owner_surface())
        .await?)
}

/// Where a video's transcoded preview rendition stands — the command
/// twin of `GET /asterism/assets/{id}/video-preview`. The first call
/// for a missing rendition enqueues the transcode; the pane polls
/// while `pending`.
#[tauri::command]
pub async fn asset_video_preview(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<asterism_contract::dto::VideoPreviewDto, UiError> {
    // The desktop pane is the owner's own window — no viewer filtering.
    Ok(state.asset_service.video_preview(&asset_id, None).await?)
}

/// Enqueues an incremental constellation-edge rebuild for the asset.
/// Returns the engine's task id.
#[tauri::command]
pub async fn rebuild_edges(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<String, UiError> {
    Ok(state.asset_service.rebuild_edges(&asset_id).await?)
}

/// Sidebar Tags section — every tag paired with the number of
/// distinct assets attached to it. `persona_id` restricts the count
/// to that persona's assets so the sidebar tracks the active persona
/// filter; pass `None` for a global count.
#[tauri::command]
pub async fn list_tag_counts(
    state: State<'_, AppState>,
    persona_id: Option<String>,
) -> Result<Vec<TagCountDto>, UiError> {
    Ok(state
        .asset_service
        .list_tag_counts(persona_id.as_deref())
        .await?)
}

/// Sessions view — one row per `session_id` in the query scope.
/// Same `ListAssetsQuery` shape as the Messages view; the UI wires
/// its view-mode toggle to swap `list_assets` ↔ `list_sessions`.
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
    query: asterism_contract::query::ListAssetsQuery,
) -> Result<SessionPageDto, UiError> {
    Ok(state.asset_service.list_sessions(query).await?)
}

/// Enqueues a `SessionRebuild` job. The precomputed rkyv snapshot
/// was retired in the Session 1st-class migration, so the handler
/// is a no-op today; the tauri command stays for wire compatibility
/// so a future reconciliation pass has a caller-visible entry
/// point (progress emits via the shared `job:progress:{id}` channel).
#[tauri::command]
pub async fn rebuild_sessions(state: State<'_, AppState>) -> Result<String, UiError> {
    Ok(state.asset_service.rebuild_sessions().await?)
}

/// Fetches one Session by surrogate id (`None` when absent). Used
/// by SessionsView after an inline edit to re-hydrate the tile
/// without a full `list_sessions` round-trip. Contract:
/// [`SessionService::get`](asterism_core::application::SessionService::get).
#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<SessionDto>, UiError> {
    Ok(state.session_service.get(&id).await?)
}

/// Renames a Session (title-only write). Passing `title: null`
/// clears the title back to the untitled state — this is the sole
/// path that expresses NULL because
/// [`patch_session_metadata`] treats per-field `null` as "leave
/// unchanged".
#[tauri::command]
pub async fn rename_session(
    state: State<'_, AppState>,
    command: RenameSessionCommand,
) -> Result<SessionDto, UiError> {
    Ok(state
        .session_service
        .rename(command, &AttributionContext::owner_surface())
        .await?)
}

/// Partially updates a Session's metadata (`title` / `note` /
/// `cover_hint`). Each `null` field is left unchanged. Use
/// [`rename_session`] to clear the title back to null.
#[tauri::command]
pub async fn patch_session_metadata(
    state: State<'_, AppState>,
    command: PatchSessionMetadataCommand,
) -> Result<SessionDto, UiError> {
    Ok(state
        .session_service
        .patch_metadata(command, &AttributionContext::owner_surface())
        .await?)
}

/// Deletes a Session — only when no `asset` row still references
/// it (`409 Conflict` otherwise). Mirror of the Modality delete
/// guard; the UI disables the tile's ✕ button when
/// `message_count > 0` so this error only surfaces under a race.
#[tauri::command]
pub async fn delete_session(
    state: State<'_, AppState>,
    command: DeleteSessionCommand,
) -> Result<(), UiError> {
    state
        .session_service
        .delete_if_empty(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Sidebar Groups section.
#[tauri::command]
pub async fn list_groups(
    state: State<'_, AppState>,
    persona_id: Option<String>,
) -> Result<Vec<GroupSummaryDto>, UiError> {
    Ok(state
        .asset_service
        .list_groups(persona_id.as_deref())
        .await?)
}

/// Creates a Group under the given persona.
#[tauri::command]
pub async fn create_group(
    state: State<'_, AppState>,
    command: CreateGroupCommand,
) -> Result<GroupDto, UiError> {
    Ok(state
        .asset_service
        .create_group(command, &AttributionContext::owner_surface())
        .await?)
}

/// Moves a Group to the trash (reversible; membership and drag order
/// survive, member assets are untouched).
#[tauri::command]
pub async fn trash_group(
    state: State<'_, AppState>,
    command: TrashGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .trash_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Returns a trashed Group to the sidebar.
#[tauri::command]
pub async fn restore_group(
    state: State<'_, AppState>,
    command: RestoreGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .restore_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Permanently deletes an already-trashed Group (cascades the m:n
/// rows). Conflicts when the Group is still live.
#[tauri::command]
pub async fn purge_group(
    state: State<'_, AppState>,
    command: PurgeGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .purge_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Idempotent add of an asset to a Group.
#[tauri::command]
pub async fn add_asset_to_group(
    state: State<'_, AppState>,
    command: AddAssetToGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .add_asset_to_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Idempotent remove of an asset from a Group.
#[tauri::command]
pub async fn remove_asset_from_group(
    state: State<'_, AppState>,
    command: RemoveAssetFromGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .remove_asset_from_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Bulk attach / detach of asset↔group pairs. Returns
/// `[attached, detached]` written-row counts.
#[tauri::command]
pub async fn batch_group_membership(
    state: State<'_, AppState>,
    command: BatchGroupMembershipCommand,
) -> Result<(u64, u64), UiError> {
    Ok(state
        .asset_service
        .batch_group_membership(command, &AttributionContext::owner_surface())
        .await?)
}

/// Merges one manual group into another and deletes the source
/// (duplicate-group consolidation). Returns the number of
/// members moved.
#[tauri::command]
pub async fn merge_groups(
    state: State<'_, AppState>,
    command: MergeGroupsCommand,
) -> Result<u64, UiError> {
    Ok(state
        .asset_service
        .merge_groups(command, &AttributionContext::owner_surface())
        .await?)
}

/// Attaches a tag to an asset by name (creates the tag row on first
/// use). Idempotent on both the row and the m:n link.
#[tauri::command]
pub async fn attach_tag(
    state: State<'_, AppState>,
    command: AttachTagCommand,
) -> Result<TagDto, UiError> {
    Ok(state
        .asset_service
        .attach_tag(command, &AttributionContext::owner_surface())
        .await?)
}

/// Removes a tag from an asset. Idempotent — a missing link is a
/// no-op; the tag row is left in place for other assets.
#[tauri::command]
pub async fn detach_tag(
    state: State<'_, AppState>,
    command: DetachTagCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .detach_tag(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Attaches one tag to many assets in one call (grid multi-select).
#[tauri::command]
pub async fn attach_tag_batch(
    state: State<'_, AppState>,
    command: AttachTagBatchCommand,
) -> Result<AttachTagBatchResult, UiError> {
    Ok(state
        .asset_service
        .attach_tag_batch(command, &AttributionContext::owner_surface())
        .await?)
}

/// Detaches one tag from many assets in one call.
#[tauri::command]
pub async fn detach_tag_batch(
    state: State<'_, AppState>,
    command: DetachTagBatchCommand,
) -> Result<DetachTagBatchResult, UiError> {
    Ok(state
        .asset_service
        .detach_tag_batch(command, &AttributionContext::owner_surface())
        .await?)
}

/// Snapshots every asset carrying a tag into a newly-created Group.
/// The tag itself is left alone; the promotion is a one-shot copy
/// (see `AssetService::promote_tag_to_group` for the rationale).
#[tauri::command]
pub async fn promote_tag_to_group(
    state: State<'_, AppState>,
    command: PromoteTagToGroupCommand,
) -> Result<PromoteTagToGroupResult, UiError> {
    Ok(state
        .asset_service
        .promote_tag_to_group(command, &AttributionContext::owner_surface())
        .await?)
}

/// Rewrites the front-to-back order of a Group's assets after a drag.
#[tauri::command]
pub async fn reorder_group_assets(
    state: State<'_, AppState>,
    command: ReorderGroupAssetsCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .reorder_group_assets(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Renames a Group.
#[tauri::command]
pub async fn rename_group(
    state: State<'_, AppState>,
    command: RenameGroupCommand,
) -> Result<GroupDto, UiError> {
    Ok(state
        .asset_service
        .rename_group(command, &AttributionContext::owner_surface())
        .await?)
}

/// Files a Group under a Dir (`None` = back to the root).
#[tauri::command]
pub async fn move_group_to_dir(
    state: State<'_, AppState>,
    command: MoveGroupToDirCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .move_group_to_dir(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Connects a Group into another Group (cycle- / persona-guarded).
#[tauri::command]
pub async fn link_group(
    state: State<'_, AppState>,
    command: LinkGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .link_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Removes a Group-in-Group connection.
#[tauri::command]
pub async fn unlink_group(
    state: State<'_, AppState>,
    command: UnlinkGroupCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .unlink_group(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Every Group-in-Group connection in scope — the UI builds the
/// nesting graph (child bands, descendant filter expansion) from it.
#[tauri::command]
pub async fn list_group_links(
    state: State<'_, AppState>,
    persona_id: Option<String>,
) -> Result<Vec<GroupLinkDto>, UiError> {
    Ok(state
        .asset_service
        .list_group_links(persona_id.as_deref())
        .await?)
}

/// Rewrites the order of a Group's child groups.
#[tauri::command]
pub async fn reorder_group_children(
    state: State<'_, AppState>,
    command: ReorderGroupChildrenCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .reorder_group_children(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Sidebar Dir tree (flat `parent_id` list; the UI assembles it).
#[tauri::command]
pub async fn list_dirs(
    state: State<'_, AppState>,
    persona_id: Option<String>,
) -> Result<Vec<DirDto>, UiError> {
    Ok(state.asset_service.list_dirs(persona_id.as_deref()).await?)
}

/// Creates a Dir under the given persona.
#[tauri::command]
pub async fn create_dir(
    state: State<'_, AppState>,
    command: CreateDirCommand,
) -> Result<DirDto, UiError> {
    Ok(state
        .asset_service
        .create_dir(command, &AttributionContext::owner_surface())
        .await?)
}

/// Renames a Dir.
#[tauri::command]
pub async fn rename_dir(
    state: State<'_, AppState>,
    command: RenameDirCommand,
) -> Result<DirDto, UiError> {
    Ok(state
        .asset_service
        .rename_dir(command, &AttributionContext::owner_surface())
        .await?)
}

/// Re-parents a Dir (`None` = to the root); cycle-guarded.
#[tauri::command]
pub async fn move_dir(state: State<'_, AppState>, command: MoveDirCommand) -> Result<(), UiError> {
    state
        .asset_service
        .move_dir(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Deletes an **empty** Dir.
#[tauri::command]
pub async fn delete_dir(
    state: State<'_, AppState>,
    command: DeleteDirCommand,
) -> Result<(), UiError> {
    state
        .asset_service
        .delete_dir(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Which Groups the asset already sits in — powers the "already
/// added" state on the detail-overlay dropdown.
#[tauri::command]
pub async fn groups_of_asset(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<Vec<GroupDto>, UiError> {
    Ok(state.asset_service.groups_of_asset(&asset_id).await?)
}

/// Resolves the full source text of each asset (session Reader
/// view). Unreadable sources come back as `text = None` so the UI
/// can fall back to the stored cover snippet.
#[tauri::command]
pub async fn asset_texts(
    state: State<'_, AppState>,
    asset_ids: Vec<String>,
) -> Result<Vec<AssetTextDto>, UiError> {
    Ok(state.asset_service.asset_texts(&asset_ids, None).await?)
}

/// Snapshot of the apalis `Jobs` table used by the UI progress
/// banner. Polled every few seconds by the front-end. Mapped to the
/// contract wire form so the shape reaches `bindings.ts` through
/// schema-bridge (the infra type stays off the contract layer).
#[tauri::command]
pub async fn jobs_stats(
    state: State<'_, AppState>,
) -> Result<asterism_contract::dto::JobsSnapshotDto, UiError> {
    let snap = asterism_infra::jobs::jobs_snapshot(&state.jobs_pool)
        .await
        .map_err(UiError::from)?;
    Ok(asterism_contract::dto::JobsSnapshotDto {
        total: snap.total,
        done: snap.done,
        pending: snap.pending,
        running: snap.running,
        failed: snap.failed,
        by_kind: snap
            .by_kind
            .into_iter()
            .map(|(kind, k)| {
                (
                    kind,
                    asterism_contract::dto::JobKindSnapshotDto {
                        total: k.total,
                        done: k.done,
                        pending: k.pending,
                        running: k.running,
                        failed: k.failed,
                    },
                )
            })
            .collect(),
    })
}

/// Appends one telemetry event to the local `event_log`. Fire-and-
/// forget on the UI side (a lost event is preferable to a blocked
/// interaction).
#[tauri::command]
pub async fn record_event(
    state: State<'_, AppState>,
    command: asterism_contract::command::RecordEventCommand,
) -> Result<(), UiError> {
    Ok(state.telemetry.record(command).await?)
}

/// Appends one webview-origin diagnostic to `diag_log` — the capture
/// half lives in `lib/diag.ts` (console/error hooks), the persistence
/// half in the shared tracing subscriber. Fire-and-forget on the UI
/// side, like `record_event`. Stateless: the re-emit helper writes
/// through the process-global subscriber, not a service handle.
#[tauri::command]
pub async fn record_diag(
    command: asterism_contract::command::RecordDiagCommand,
) -> Result<(), UiError> {
    Ok(asterism_server::http::record_webview_diag(&command)?)
}

/// Newest-first telemetry listing (kind / time-window filters). Feeds
/// local usage summaries; the HTTP twin serves agent-side aggregation.
#[tauri::command]
pub async fn list_events(
    state: State<'_, AppState>,
    query: asterism_contract::query::ListEventsQuery,
) -> Result<Vec<asterism_contract::dto::EventDto>, UiError> {
    Ok(state.telemetry.list(query).await?)
}

/// Returns the cached JPEG bytes of a thumbnail for `asset_id` at
/// `size_px`, or `None` when the pair is not cached yet (`cover_gen`
/// or the importer has not written one). The Svelte side wraps the
/// bytes in a `Blob` and hands the URL to `<img src>`.
#[tauri::command]
pub async fn get_asset_thumb(
    state: State<'_, AppState>,
    asset_id: String,
    size_px: u32,
) -> Result<Option<Vec<u8>>, UiError> {
    let cached = state.thumb_service.get(&asset_id, size_px).await?;
    if cached.is_none() {
        // Cache miss — kick a high-priority `thumb_gen` off so the
        // ImageIO worker materialises the blob on the next tick.
        // The UI polls (or re-invokes on the next paint) until the
        // fetch flips to `Some`, then swaps in the sharp preview.
        let _ = state
            .asset_service
            .enqueue_thumb_gen(&asset_id, size_px)
            .await;
    }
    Ok(cached)
}

/// The same thing for a whole screenful: cached JPEG bytes for each of
/// `asset_ids` at `size_px`, **in the order asked**, `None` where the
/// pair is not cached yet.
///
/// The grid paints tens of tiles per scroll, and asking per tile made
/// that tens of IPC round trips per scroll — measured at 8,263 single
/// fetches across 1,000 jumps, with p95 reaching 10.4 s once the blob
/// cache stopped absorbing the repeats [measured 2026-08-05,
/// bench-scroll-v3]. One call per screenful is the shape the caller
/// actually has.
///
/// Misses still enqueue `thumb_gen`, one job per missing pair — the
/// jobs are per asset by nature, so batching the read does not batch
/// those.
#[tauri::command]
pub async fn get_asset_thumbs(
    state: State<'_, AppState>,
    asset_ids: Vec<String>,
    size_px: u32,
) -> Result<Vec<Option<Vec<u8>>>, UiError> {
    let cached = state.thumb_service.get_many(&asset_ids, size_px).await?;
    for (asset_id, slot) in asset_ids.iter().zip(cached.iter()) {
        if slot.is_none() {
            let _ = state
                .asset_service
                .enqueue_thumb_gen(asset_id, size_px)
                .await;
        }
    }
    Ok(cached)
}

// ---------------------------------------------------------------------------
// Selector / Outbound Dispatch commands.
// ---------------------------------------------------------------------------

// The Selection CRUD commands (get / list / list_recent) were removed
// in the W3a Snapshot transmigration: a Snapshot is a system-generated
// content object with no public list / rename / delete surface.
// The W3c command surface (`snapshot_get` / `snapshot_members` /
// redispatch) supersedes the removed reads. `create` came back —
// freezing a pick is the one Snapshot verb a caller has a reason to
// ask for by itself.

/// Freezes a picked asset list into a Snapshot and stops there
/// (content-hash deduped).
///
/// The freeze half of `promote_volatile_selection` without the promote.
/// Both other Snapshot-consuming commands (`create_dispatch`,
/// `get_snapshot`) take a snapshot id, and until this existed the only
/// way to obtain one was to also create a Group — so "keep this exact
/// set for now" had to go through a side effect nobody asked for.
#[tauri::command]
pub async fn create_snapshot(
    state: State<'_, AppState>,
    command: CreateSnapshotCommand,
) -> Result<SnapshotDto, UiError> {
    Ok(state
        .snapshot_service
        .create(command, &AttributionContext::owner_surface())
        .await?)
}

/// Reverse lookup — every Snapshot whose asset_ids list contains this
/// asset (P5). Renders on the detail panel as "included in these
/// freezes" chips.
#[tauri::command]
pub async fn list_snapshots_containing(
    state: State<'_, AppState>,
    asset_id: String,
    limit: u32,
) -> Result<Vec<SnapshotDto>, UiError> {
    Ok(state
        .snapshot_service
        .list_containing(&asset_id, limit)
        .await?)
}

/// Promotes a Snapshot into a hand-owned Group (mirror of
/// `PromoteTagToGroup`): bulk-attaches every member in frozen order
/// and stamps `origin_snapshot_id` as the Group's birth record.
#[tauri::command]
pub async fn promote_snapshot_to_group(
    state: State<'_, AppState>,
    command: PromoteSnapshotToGroupCommand,
) -> Result<PromoteSnapshotToGroupResult, UiError> {
    Ok(state
        .snapshot_service
        .promote_to_group(command, &AttributionContext::owner_surface())
        .await?)
}

/// Fuses freeze + promote for the grid's volatile pick (W5-d):
/// mints a Snapshot from the picked ids (content-hash deduped)
/// and promotes it into a hand-owned Group in one step — the
/// right-click "Group-ify selection" entry, no pre-existing Snapshot
/// needed.
#[tauri::command]
pub async fn promote_volatile_selection(
    state: State<'_, AppState>,
    command: PromoteVolatileSelectionCommand,
) -> Result<PromoteSnapshotToGroupResult, UiError> {
    Ok(state
        .snapshot_service
        .promote_volatile_selection(command, &AttributionContext::owner_surface())
        .await?)
}

/// Snapshot view metadata (`snapshot_get`): the freeze's id,
/// content hash, and frozen member ids.
#[tauri::command]
pub async fn get_snapshot(state: State<'_, AppState>, id: String) -> Result<SnapshotDto, UiError> {
    Ok(state.snapshot_service.get_snapshot(&id).await?)
}

/// Snapshot view members (`snapshot_members`): renderable cards
/// in frozen `position` order (later-deleted assets are absent).
#[tauri::command]
pub async fn snapshot_members(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<AssetCardDto>, UiError> {
    Ok(state.snapshot_service.snapshot_members(&id).await?)
}

/// Live-source dispatch (`dispatch_run`): freezes a Group (query
/// groups are refreshed first) or a volatile selection into a deduped
/// Snapshot, stamps the provenance, and enqueues the run.
#[tauri::command]
pub async fn dispatch_run(
    state: State<'_, AppState>,
    command: DispatchRunCommand,
) -> Result<DispatchDto, UiError> {
    if let Some(exp) = state.exporter_registry.get(&command.exporter_slug) {
        if !exp.accepts(&command.action) {
            return Err(UiError::from(asterism_core::DomainError::Validation(
                format!(
                    "exporter {:?} does not accept action {:?}",
                    command.exporter_slug, command.action
                ),
            )));
        }
    } else if !state.exporter_registry.slugs().is_empty() {
        // Registry has entries but not this one — fail fast before a
        // snapshot is frozen for a job that can never run.
        return Err(UiError::from(asterism_core::DomainError::Validation(
            format!("exporter not registered: {:?}", command.exporter_slug),
        )));
    }
    Ok(state
        .dispatch_service
        .run(command, &AttributionContext::owner_surface())
        .await?)
}

/// Re-runs a finished dispatch with the same frozen input (P2).
#[tauri::command]
pub async fn redispatch(
    state: State<'_, AppState>,
    command: RedispatchCommand,
) -> Result<DispatchDto, UiError> {
    Ok(state
        .dispatch_service
        .redispatch(command, &AttributionContext::owner_surface())
        .await?)
}

// ---------------------------------------------------------------------------
// Pursuit — the unit of work assets are filed under (#29).
//
// Attribution is `owner_surface` throughout, like every other command in
// this file: a write arriving here came from the person sitting in front
// of the window. The commands carry an `operator_ai` field for the
// remote surfaces, and it is deliberately not read here — an agent slug
// stated to the desktop app would be a claim about somebody else.
// ---------------------------------------------------------------------------

/// Opens a pursuit and names what it is for, ahead of any work in it.
///
/// This is the "start a new line of work" affordance, and the path that
/// creates a pursuit at an id it was known by elsewhere.
#[tauri::command]
pub async fn open_pursuit(
    state: State<'_, AppState>,
    command: OpenPursuitCommand,
) -> Result<PursuitDto, UiError> {
    Ok(state
        .pursuit_service
        .open(command, &AttributionContext::owner_surface())
        .await?)
}

/// Records that a pursuit concluded — `satisfied` (freezing the kept set
/// into a snapshot the event references) or `abandoned`.
///
/// An event, not a status write: closing twice records two facts and
/// standing derives from the later one, and nothing happens to the
/// assets themselves.
#[tauri::command]
pub async fn close_pursuit(
    state: State<'_, AppState>,
    command: ClosePursuitCommand,
) -> Result<PursuitEventDto, UiError> {
    Ok(state
        .pursuit_service
        .close(command, &AttributionContext::owner_surface())
        .await?)
}

/// Records that a pursuit carried on after a close. Legal on one that is
/// already open, where it leaves a fact and changes no standing.
#[tauri::command]
pub async fn reopen_pursuit(
    state: State<'_, AppState>,
    command: ReopenPursuitCommand,
) -> Result<PursuitEventDto, UiError> {
    Ok(state
        .pursuit_service
        .reopen(command, &AttributionContext::owner_surface())
        .await?)
}

/// One pursuit with its standing derived from the latest event.
#[tauri::command]
pub async fn get_pursuit(state: State<'_, AppState>, id: String) -> Result<PursuitDto, UiError> {
    Ok(state.pursuit_service.get(&id).await?)
}

/// A persona's pursuits, most-recent first, each with its standing —
/// the multi-pursuit overview's read.
#[tauri::command]
pub async fn list_pursuits(
    state: State<'_, AppState>,
    persona_id: String,
    limit: u32,
) -> Result<Vec<PursuitDto>, UiError> {
    Ok(state.pursuit_service.list(&persona_id, limit).await?)
}

/// A pursuit's lifecycle facts, oldest first.
#[tauri::command]
pub async fn pursuit_events(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<PursuitEventDto>, UiError> {
    Ok(state.pursuit_service.events(&id).await?)
}

/// One pursuit opened up: the row and its standing, its events, and its
/// ledger.
#[tauri::command]
pub async fn pursuit_view(
    state: State<'_, AppState>,
    id: String,
) -> Result<PursuitViewDto, UiError> {
    Ok(state.pursuit_service.view(&id).await?)
}

// ---------------------------------------------------------------------------
// Query groups — Groups whose membership is a materialised rule.
// The SavedQuery commands that lived here were absorbed into these (the
// V19 migration transcribed every row; the concept is gone).
// ---------------------------------------------------------------------------

/// "Save as Group": mints a `kind='query'` Group from a `query_json`
/// v1 rule and evaluates it synchronously once.
#[tauri::command]
pub async fn create_query_group(
    state: State<'_, AppState>,
    command: CreateQueryGroupCommand,
) -> Result<GroupDto, UiError> {
    Ok(state
        .query_group_service
        .create_query_group(command, &AttributionContext::owner_surface())
        .await?)
}

/// "Update query": validates + persists a replacement rule (rejecting
/// dependency cycles) and re-evaluates the membership.
#[tauri::command]
pub async fn update_query_group_query(
    state: State<'_, AppState>,
    command: UpdateQueryGroupQueryCommand,
) -> Result<GroupDto, UiError> {
    Ok(state
        .query_group_service
        .update_query(command, &AttributionContext::owner_surface())
        .await?)
}

/// Kicks off one exporter run against a Selection. The apalis
/// `DispatchRun` job picks it up on the next tick; the UI polls
/// [`get_dispatch`] until the state is terminal.
#[tauri::command]
pub async fn create_dispatch(
    state: State<'_, AppState>,
    command: CreateDispatchCommand,
) -> Result<DispatchDto, UiError> {
    // Pre-flight registry check so mis-routed requests fail fast at
    // the wire boundary (mirror of the HTTP server pre-flight).
    if let Some(exp) = state.exporter_registry.get(&command.exporter_slug) {
        if !exp.accepts(&command.action) {
            return Err(UiError::from(asterism_core::DomainError::Validation(
                format!(
                    "exporter {:?} does not accept action {:?}",
                    command.exporter_slug, command.action
                ),
            )));
        }
    } else if !state.exporter_registry.slugs().is_empty() {
        return Err(UiError::from(asterism_core::DomainError::Validation(
            format!("exporter not registered: {:?}", command.exporter_slug),
        )));
    }
    Ok(state
        .dispatch_service
        .create(command, &AttributionContext::owner_surface())
        .await?)
}

/// Fetches a dispatch job by id — used by the poll loop that drives
/// the progress badge.
#[tauri::command]
pub async fn get_dispatch(state: State<'_, AppState>, id: String) -> Result<DispatchDto, UiError> {
    Ok(state.dispatch_service.get(&id).await?)
}

/// Lists dispatch jobs with the same predicate surface as the HTTP
/// list endpoint — the UI renders a per-persona history panel.
#[tauri::command]
pub async fn list_dispatch(
    state: State<'_, AppState>,
    persona_id: Option<String>,
    snapshot_id: Option<String>,
    state_slug: Option<String>,
    limit: u32,
) -> Result<Vec<DispatchDto>, UiError> {
    Ok(state
        .dispatch_service
        .list(
            persona_id.as_deref(),
            snapshot_id.as_deref(),
            state_slug.as_deref(),
            limit,
        )
        .await?)
}

/// Registered exporter slugs — the action bar renders one row per
/// entry. Empty vector means the current build shipped no exporters.
#[tauri::command]
pub async fn list_exporters(state: State<'_, AppState>) -> Result<Vec<String>, UiError> {
    Ok(state.exporter_registry.slugs())
}

// ---------------------------------------------------------------------------
// Asset comment thread.
// ---------------------------------------------------------------------------

/// Lists every comment on `asset_id` in chronological order.
#[tauri::command]
pub async fn list_asset_comments(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<Vec<AssetCommentDto>, UiError> {
    Ok(state.asset_comment_service.list(&asset_id).await?)
}

/// Posts a new comment. See [`PostAssetCommentCommand`] for the
/// author_kind / author_persona_id semantics.
#[tauri::command]
pub async fn post_asset_comment(
    state: State<'_, AppState>,
    command: PostAssetCommentCommand,
) -> Result<AssetCommentDto, UiError> {
    Ok(state
        .asset_comment_service
        .post(command, &AttributionContext::owner_surface())
        .await?)
}

/// Rewrites the body of an existing comment (stamps `edited_at`).
#[tauri::command]
pub async fn edit_asset_comment(
    state: State<'_, AppState>,
    command: EditAssetCommentCommand,
) -> Result<AssetCommentDto, UiError> {
    Ok(state
        .asset_comment_service
        .edit(command, &AttributionContext::owner_surface())
        .await?)
}

/// Deletes a comment. Idempotent.
#[tauri::command]
pub async fn delete_asset_comment(
    state: State<'_, AppState>,
    command: DeleteAssetCommentCommand,
) -> Result<(), UiError> {
    state
        .asset_comment_service
        .delete(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Marks in an Asset's material.
//
// Sibling of the comment thread above on a narrower anchor: a comment is
// about the asset, a mark is at a position inside what the asset holds
// (today a point or interval on its playback timeline). The four verbs
// are the same four; the ordering is the material's, not the thread's.
// ---------------------------------------------------------------------------

/// Lists every mark in `asset_id`'s material, in the material's own
/// order (`start_ms` ascending, ties broken by id).
#[tauri::command]
pub async fn list_material_marks(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<Vec<MaterialMarkDto>, UiError> {
    Ok(state.material_mark_service.list_by_asset(&asset_id).await?)
}

/// Places a new mark. See [`PostMaterialMarkCommand`] for the anchor and
/// author_kind / author_persona_id semantics.
#[tauri::command]
pub async fn post_material_mark(
    state: State<'_, AppState>,
    command: PostMaterialMarkCommand,
) -> Result<MaterialMarkDto, UiError> {
    Ok(state
        .material_mark_service
        .post(command, &AttributionContext::owner_surface())
        .await?)
}

/// Rewrites the body of an existing mark (stamps `edited_at`). The
/// anchor is left where it is — this verb rewords a mark, it does not
/// move one.
#[tauri::command]
pub async fn edit_material_mark(
    state: State<'_, AppState>,
    command: EditMaterialMarkCommand,
) -> Result<MaterialMarkDto, UiError> {
    Ok(state
        .material_mark_service
        .edit(command, &AttributionContext::owner_surface())
        .await?)
}

/// Deletes a mark. Idempotent.
#[tauri::command]
pub async fn delete_material_mark(
    state: State<'_, AppState>,
    command: DeleteMaterialMarkCommand,
) -> Result<(), UiError> {
    state
        .material_mark_service
        .delete(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Bands over an Asset's material, and the chapters in one.
//
// A layer says where a set of marks came from: the material's own
// declaration, a person's reading of it, or a job's. The panel that shows
// a chapter list reads them all at once and edits exactly one of them —
// the person's — because the other two are reproduced by running their
// producer again, so an edit into either is lost at the next run.
// ---------------------------------------------------------------------------

/// Lists every band over `asset_id`'s material, each with the chapters
/// in it, in display order.
///
/// One call rather than one per band: the panel needs the bands to choose
/// between and the contents of the chosen one at the same moment. An
/// annotation band's `chapters` is always empty — those hold notes, read
/// through `list_material_marks`.
#[tauri::command]
pub async fn list_material_layers(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<Vec<MaterialLayerViewDto>, UiError> {
    Ok(state.material_layer_service.list_views(&asset_id).await?)
}

/// Lists the sections in one band, in the reading order the band states
/// (which need not be the timeline's). What a panel re-reads after
/// editing a single band.
#[tauri::command]
pub async fn list_chapter_marks(
    state: State<'_, AppState>,
    layer_id: String,
) -> Result<Vec<ChapterMarkDto>, UiError> {
    Ok(state
        .material_layer_service
        .list_chapter_marks(&layer_id)
        .await?)
}

/// Opens a band the person owns. Never the default — see
/// [`SetDefaultMaterialLayerCommand`] for moving that flag.
#[tauri::command]
pub async fn create_material_layer(
    state: State<'_, AppState>,
    command: CreateMaterialLayerCommand,
) -> Result<MaterialLayerDto, UiError> {
    Ok(state
        .material_layer_service
        .create_layer(command, &AttributionContext::owner_surface())
        .await?)
}

/// Chooses the band the panel shows, and the one a new mark lands in.
///
/// Returns nothing: the flag moves off whichever band held it, so the
/// caller re-reads the asset's bands rather than patching one entry.
#[tauri::command]
pub async fn set_default_material_layer(
    state: State<'_, AppState>,
    command: SetDefaultMaterialLayerCommand,
) -> Result<(), UiError> {
    state
        .material_layer_service
        .set_default_layer(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Deletes a band the person owns, with everything in it. Refuses an
/// imported or machine band.
#[tauri::command]
pub async fn delete_material_layer(
    state: State<'_, AppState>,
    command: DeleteMaterialLayerCommand,
) -> Result<(), UiError> {
    state
        .material_layer_service
        .delete_layer(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Adds a section to a band the person owns.
#[tauri::command]
pub async fn post_chapter_mark(
    state: State<'_, AppState>,
    command: PostChapterMarkCommand,
) -> Result<ChapterMarkDto, UiError> {
    Ok(state
        .material_layer_service
        .post_chapter_mark(command, &AttributionContext::owner_surface())
        .await?)
}

/// Retitles a section and, unlike the mark face, may move it: the reason
/// a person keeps a band of their own is usually that the file's
/// divisions are in the wrong places.
#[tauri::command]
pub async fn edit_chapter_mark(
    state: State<'_, AppState>,
    command: EditChapterMarkCommand,
) -> Result<ChapterMarkDto, UiError> {
    Ok(state
        .material_layer_service
        .edit_chapter_mark(command, &AttributionContext::owner_surface())
        .await?)
}

/// Removes one section. **Not** idempotent, unlike deleting a mark: a
/// chapter is named by `(layer_id, chapter_id)`, so an id that is not in
/// that band is a refusal rather than a no-op.
#[tauri::command]
pub async fn delete_chapter_mark(
    state: State<'_, AppState>,
    command: DeleteChapterMarkCommand,
) -> Result<(), UiError> {
    state
        .material_layer_service
        .delete_chapter_mark(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

// -----------------------------------------------------------------
// App-level Threads
// -----------------------------------------------------------------

/// Lists Threads under the given anchor, freshest first. Archived
/// Threads are excluded unless `include_archived` is `true`.
#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    anchor_kind: String,
    anchor_id: Option<String>,
    include_archived: Option<bool>,
) -> Result<Vec<ThreadDto>, UiError> {
    Ok(state
        .thread_service
        .list(
            &anchor_kind,
            anchor_id.as_deref(),
            include_archived.unwrap_or(false),
        )
        .await?)
}

/// Fetches one Thread by id.
#[tauri::command]
pub async fn get_thread(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<Option<ThreadDto>, UiError> {
    Ok(state.thread_service.find(&thread_id).await?)
}

/// Creates a Thread.
#[tauri::command]
pub async fn create_thread(
    state: State<'_, AppState>,
    command: CreateThreadCommand,
) -> Result<ThreadDto, UiError> {
    Ok(state
        .thread_service
        .create(command, &AttributionContext::owner_surface())
        .await?)
}

/// Toggles the archived flag.
#[tauri::command]
pub async fn archive_thread(
    state: State<'_, AppState>,
    command: ArchiveThreadCommand,
) -> Result<ThreadDto, UiError> {
    Ok(state
        .thread_service
        .archive(command, &AttributionContext::owner_surface())
        .await?)
}

/// Deletes a Thread (cascades to messages).
#[tauri::command]
pub async fn delete_thread(
    state: State<'_, AppState>,
    command: DeleteThreadCommand,
) -> Result<(), UiError> {
    state
        .thread_service
        .delete(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Lists the Messages of a Thread.
#[tauri::command]
pub async fn list_thread_messages(
    state: State<'_, AppState>,
    thread_id: String,
    since_ms: Option<i64>,
) -> Result<Vec<MessageDto>, UiError> {
    Ok(state
        .thread_service
        .list_messages(&thread_id, since_ms)
        .await?)
}

/// Appends one Message. UI-side callers pass `author_kind = "human"`.
#[tauri::command]
pub async fn append_thread_message(
    state: State<'_, AppState>,
    command: AppendMessageCommand,
) -> Result<MessageDto, UiError> {
    Ok(state
        .thread_service
        .append_message(command, &AttributionContext::owner_surface())
        .await?)
}

/// Deletes one Message (misfire correction).
#[tauri::command]
pub async fn delete_thread_message(
    state: State<'_, AppState>,
    command: DeleteMessageCommand,
) -> Result<(), UiError> {
    state
        .thread_service
        .delete_message(command, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}
