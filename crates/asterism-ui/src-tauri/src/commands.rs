//! Tauri command handlers — a thin translation layer. They pass DTOs
//! through to the application services in `asterism-core` and convert
//! `DomainError` into `UiError`. No business logic lives here.
//!
//! Every mutation that writes to *this machine* names its attribution
//! channel explicitly — [`AttributionContext::owner_surface`]. This is
//! the owner's own operation surface (the desktop app's IPC), so the
//! owner-ness is a property of the surface rather than a guess about the
//! caller, and the commands carry no attribution fields for it to read.
//! The argument is required by the service signatures, so a new
//! mutation cannot be added here without choosing.
//!
//! The qualifier is #153's and covers exactly one block, at the end of
//! this file: the verbs that write to a **team**. There the author is
//! the authenticated member and the team's server stamps it, so a
//! context stated here would be a second answer to a settled question.
//! Two of those verbs — connecting and publishing — write nothing
//! through a service that takes a context, and `publish_line_to_team`
//! additionally writes local relation rows, which carry no actor at
//! all. The block says all of this where it sits.
//!
//! # This surface and the HTTP one are mirrors
//!
//! Not "mostly overlapping" — a verb on one belongs on the other, in the
//! same change. `asterism-server`'s `http` module doc states the rule
//! and the two differences that are by design: attribution, and where
//! the id comes from.
//!
//! MCP is not part of that obligation. It is curated on purpose, which
//! its own module doc explains.
//!
//! # Why they are mirrors and not one path
//!
//! This process serves that HTTP surface itself — `lib.rs` spawns the
//! loopback listener over `asterism_server::http::router`, and
//! `state.rs` builds its context from the same `CoreCtx` these commands
//! hold. So a command *could* answer by calling it, and the service
//! call each verb carries would then be written once rather than on
//! both sides.
//!
//! It does not, and the reason is what the second path buys back. A
//! command reaching the loopback surface would serialise its arguments,
//! cross a socket, and deserialise a response the process already holds
//! in memory, on every call the window makes — including the ones a
//! scroll makes tens of at a time. Commands take the contract's types
//! and call the application services directly instead.
//!
//! What that costs is the thing the rule above exists to bound: two
//! transports over one service graph, either of which can gain a verb
//! the other does not. The obligation is that a person can do the same
//! work over HTTP — not that both take the same route to the service,
//! which they deliberately do not. `asterism-server`'s
//! `tests/transport_parity.rs` is what holds the two lists to each
//! other.

use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AddAssetToGroupCommand,
    AppendMessageCommand, ArchivePersonaCommand, ArchiveThreadCommand, AttachTagBatchCommand,
    AttachTagBatchResult, AttachTagCommand, BatchGroupMembershipCommand, CreateDirCommand,
    CreateDispatchCommand, CreateGroupCommand, CreateMaterialLayerCommand, CreateModalityCommand,
    CreateQueryGroupCommand, CreateSeriesStrategyCommand, CreateSnapshotCommand,
    CreateThreadCommand, DeleteAssetCommentCommand, DeleteChapterMarkCommand, DeleteDirCommand,
    DeleteMaterialLayerCommand, DeleteMaterialMarkCommand, DeleteMessageCommand,
    DeleteModalityCommand, DeletePersonaProfileCommand, DeletePersonaThemeCommand,
    DeleteSeriesStrategyCommand, DeleteSessionCommand, DeleteTagCommand, DeleteTagResult,
    DeleteThreadCommand, DetachTagBatchCommand, DetachTagBatchResult, DetachTagCommand,
    DispatchRunCommand, EditAssetCommentCommand, EditChapterMarkCommand, EditMaterialMarkCommand,
    EmptyTrashCommand, EmptyTrashResult, LinkGroupCommand, MergeAssetsCommand, MergeGroupsCommand,
    MergeTagsCommand, MergeTagsResult, MoveDirCommand, MoveGroupToDirCommand,
    OrganizeByLocationCommand, OrganizeByLocationResult, PasteImageImportCommand,
    PatchSessionMetadataCommand, PostAssetCommentCommand, PostChapterMarkCommand,
    PostMaterialMarkCommand, PromoteSnapshotToGroupCommand, PromoteSnapshotToGroupResult,
    PromoteTagToGroupCommand, PromoteTagToGroupResult, PromoteVolatileSelectionCommand,
    PurgeAssetCommand, PurgeGroupCommand, PurgePersonaCommand, RedispatchCommand,
    RegisterPersonaCommand, RemoveAssetFromGroupCommand, RenameDirCommand, RenameGroupCommand,
    RenameSessionCommand, RenameTagCommand, ReorderGroupAssetsCommand, ReorderGroupChildrenCommand,
    ReorderPersonasCommand, ResetSettingCommand, ResolveDuplicateConflictCommand,
    RestoreAssetCommand, RestoreGroupCommand, RestorePersonaCommand,
    SetDefaultMaterialLayerCommand, SetPersonaProfileCommand, SetPersonaThemeCommand,
    SetSettingCommand, TrashAssetCommand, TrashGroupCommand, TrashPersonaCommand,
    UnlinkGroupCommand, UpdateAssetMetaBatchCommand, UpdateAssetMetaBatchResult,
    UpdateAssetMetaCommand, UpdateModalityCommand, UpdateQueryGroupQueryCommand,
    UpdateSeriesStrategyCommand,
};
use asterism_contract::dto::{
    AssetCardDto, AssetCommentDto, AssetCountEntryDto, AssetDetailDto, AssetDto, AssetPageDto,
    AssetTextDto, ChapterMarkDto, ConstellationItemDto, DirDto, DispatchDto, DuplicateConflictDto,
    DuplicateReportDto, DuplicateResolutionDto, EdgeDto, GroupDto, GroupLinkDto, GroupSummaryDto,
    MaterialLayerDto, MaterialLayerViewDto, MaterialMarkDto, MergeAssetsDto, MessageDto,
    ModalityDefDto, ObservationDto, PersonaDto, PersonaProfileDto, PersonaThemeDto,
    RetrievedPageDto, SeriesStrategyDto, SessionDto, SessionPageDto, SettingDto, SnapshotDto,
    TagCountDto, TagDto, ThreadDto,
};
use asterism_contract::forge::{
    AmendForgeMessageCommand, CloseForgePursuitCommand, ForgeCollisionDto, ForgeDiscardedDto,
    ForgeEntryStateDto, ForgeLineDto, ForgeLineHistoryDto, ForgeMessageDto, ForgeOpDto,
    ForgePursuitDto, ForgeResolvedDto, ForgeRevisionDto, ForgeStrategyDto, ForgeThreadDto,
    OpenForgeLineCommand, OpenForgePursuitCommand, OpenForgeThreadCommand, PushForgeRoundCommand,
    RenameForgeLineCommand, RenameForgeThreadCommand, SayInForgeThreadCommand,
    SetForgeLineStrategyCommand,
};
use asterism_contract::query::{
    GetAssetDetailQuery, ListAssetsQuery, ListObservationsQuery, SearchAssetsQuery,
};
use asterism_contract::teams::{
    MyTeamDto, MyTeamsDto, PromotedAssetDto, StoredTeamConnectDto, StoredTeamConnectOutcome,
    StoredTeamConnectionDto, TeamCreatedDto, TeamDeviceTokenDto, TeamDeviceTokensDto,
    TeamLedgerEventDto, TeamLedgerPageDto, TeamProviderDto, TeamRosterDto, TeamRosterMemberDto,
    TeamRosterViewerDto, TeamSubjectRefDto,
};
use asterism_core::DomainError;
use asterism_core::application::mapping::{
    asset_to_dto, forge_anchored, forge_body, forge_collisions_to_dto, forge_discarded_to_dto,
    forge_history_to_dto, forge_line_id, forge_line_to_dto, forge_message_id, forge_message_to_dto,
    forge_name, forge_op, forge_outcome, forge_pursuit_id, forge_pursuit_to_dto,
    forge_revision_to_dto, forge_round_to_dto, forge_states_to_dto, forge_strategy_id,
    forge_strategy_to_dto, forge_thread_id, forge_thread_to_dto, parse_asset_id, parse_persona_id,
};
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::forge::model::pursuit::Intent;
use asterism_core::domain::forge::model::value::{LineId, PursuitId, ThreadId};
use asterism_core::domain::repository::SourceLookupScope;
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::domain::team_link::TeamScopedId;
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind};
use tauri::State;

use crate::error::UiError;
use crate::state::{AppState, TeamsConnection};

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

/// Every registered series rule, oldest first, seeded and user-written
/// alike — the command twin of `GET /asterism/series-strategies`.
/// Rules, not groups: what a rule put on which key is a different
/// question (see the route's doc).
#[tauri::command]
pub async fn list_series_strategies(
    state: State<'_, AppState>,
) -> Result<Vec<SeriesStrategyDto>, UiError> {
    Ok(state.series_strategy_service.list().await?)
}

/// Registers a series rule and asks for the keys it implies — the
/// command twin of `POST /asterism/series-strategies`. A rule this
/// build could not carry out is refused before the row is written.
#[tauri::command]
pub async fn create_series_strategy(
    state: State<'_, AppState>,
    command: CreateSeriesStrategyCommand,
) -> Result<SeriesStrategyDto, UiError> {
    Ok(state
        .series_strategy_service
        .create(command, &AttributionContext::owner_surface())
        .await?)
}

/// Partially updates a series rule (each omitted field is left
/// unchanged) — the command twin of
/// `PATCH /asterism/series-strategies/{id}`. The `id` argument names
/// the target and overwrites the body's copy, the same arbitration the
/// route applies to its path segment.
#[tauri::command]
pub async fn update_series_strategy(
    state: State<'_, AppState>,
    id: String,
    mut command: UpdateSeriesStrategyCommand,
) -> Result<SeriesStrategyDto, UiError> {
    command.id = id;
    Ok(state
        .series_strategy_service
        .update(command, &AttributionContext::owner_surface())
        .await?)
}

/// Removes a series rule and, by the schema's cascade, every key
/// derived under it — the command twin of
/// `DELETE /asterism/series-strategies/{id}`. No guard, unlike
/// [`delete_modality`]: a series key is recomputed from rows already
/// in hand.
#[tauri::command]
pub async fn delete_series_strategy(state: State<'_, AppState>, id: String) -> Result<(), UiError> {
    state
        .series_strategy_service
        .delete(
            DeleteSeriesStrategyCommand { id },
            &AttributionContext::owner_surface(),
        )
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

/// Rehomes a dropped path into `$HOME/asterism/dropped/`
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
/// `$HOME/asterism/pasted/paste-<ts>.<ext>` and dispatches
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

/// Asserts — or retracts, via an absent `source_type` — the asset's
/// digital source type by hand: the command twin of
/// `POST /asterism/assets/{id}/source-type`. A term the IPTC
/// vocabulary does not define is refused at the door.
///
/// Like [`asset_declare_meta`], the statement's own `operator_ai`
/// travels on the command and is a different subject from who the
/// asset is by, which is why the attribution handed to the service is
/// still the owner surface.
#[tauri::command]
pub async fn asset_declare_source_type(
    state: State<'_, AppState>,
    command: asterism_contract::command::DeclareSourceTypeCommand,
) -> Result<asterism_contract::dto::AssetDto, UiError> {
    Ok(state
        .asset_service
        .declare_source_type(command, &AttributionContext::owner_surface())
        .await?)
}

/// What the asset's source type currently rests on — the read twin of
/// `GET /asterism/assets/{id}/source-type`: the container's evidence
/// and the person's assertion, each on its own, with "not yet
/// fingerprinted" kept distinct from "declares nothing".
#[tauri::command]
pub async fn asset_source_type(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<asterism_contract::dto::AssetSourceTypeDto, UiError> {
    // The desktop pane is the owner's own window — no viewer filtering.
    Ok(state.asset_service.source_type_of(&asset_id, None).await?)
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

/// Enqueues a batch `IndexRebuild` job and returns its task id — the
/// command twin of `POST /asterism/index/rebuild`. Idempotent on an
/// already-indexed DB; progress reaches the jobs ticker.
#[tauri::command]
pub async fn rebuild_index(state: State<'_, AppState>) -> Result<String, UiError> {
    Ok(state.asset_service.rebuild_index().await?)
}

/// Re-derives duplicate conflicts from fingerprints already on the
/// rows — the command twin of `POST /asterism/duplicates/rescan`.
/// Takes no input: the pass is idempotent, never folds, and always
/// walks the whole library (the route's doc has the reasoning).
/// Returns the enqueued job's task id.
#[tauri::command]
pub async fn rescan_duplicates(state: State<'_, AppState>) -> Result<String, UiError> {
    Ok(state.asset_service.rescan_duplicates().await?)
}

/// Re-reads artefacts and rewrites `width_px` / `height_px` — the
/// command twin of `POST /asterism/assets/remeasure`, with the same
/// two shapes and the same refusal to accept both: `asset_ids` is "I
/// put the right file behind these, read them again" (overwrites);
/// `scope` is `unlooked` / `unmeasured` / `all`, and only `all`
/// replaces existing answers. Returns the enqueued task ids (one job
/// per asset on the id shape, one batch job on the scope shape).
#[tauri::command]
pub async fn remeasure_dims(
    state: State<'_, AppState>,
    asset_ids: Vec<String>,
    scope: Option<String>,
) -> Result<Vec<String>, UiError> {
    match (asset_ids.is_empty(), scope.as_deref()) {
        (false, None) => {
            let ids = asset_ids
                .iter()
                .map(|id| parse_asset_id(id))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(state.asset_service.remeasure_dims(&ids).await?)
        }
        (true, Some(scope)) => Ok(vec![state.asset_service.remeasure_dims_batch(scope).await?]),
        (true, None) => {
            Err(DomainError::Validation("name either asset_ids or a scope".into()).into())
        }
        (false, Some(_)) => Err(DomainError::Validation(
            "asset_ids and scope are different requests; name one".into(),
        )
        .into()),
    }
}

/// Backfill: auto-organises existing assets under a Dir tree derived
/// from `source_locator` — the command twin of
/// `POST /asterism/organize/by-location`. Synchronous rather than a
/// job: the summary comes back when the pass finishes, and a large
/// library is a multi-minute wait (the service doc carries the cost).
#[tauri::command]
pub async fn organize_by_location(
    state: State<'_, AppState>,
    command: OrganizeByLocationCommand,
) -> Result<OrganizeByLocationResult, UiError> {
    Ok(state
        .asset_service
        .organize_by_location(command, &AttributionContext::owner_surface())
        .await?)
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

/// Lists what the bound visual model proposed for one asset (#112),
/// score-descending, rulings included. Empty without a model.
#[tauri::command]
pub async fn list_tag_suggestions(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<Vec<asterism_contract::dto::TagSuggestionDto>, UiError> {
    Ok(state.asset_service.tag_suggestions_of(&asset_id).await?)
}

/// Accepts one tag suggestion (#112): the ruling lands on the
/// evidence row and the tag is linked in `asset_tag`.
#[tauri::command]
pub async fn accept_tag_suggestion(
    state: State<'_, AppState>,
    asset_id: String,
    tag_id: String,
) -> Result<(), UiError> {
    state
        .asset_service
        .accept_tag_suggestion(&asset_id, &tag_id, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Rejects one tag suggestion (#112); this model never proposes the
/// pair again.
#[tauri::command]
pub async fn reject_tag_suggestion(
    state: State<'_, AppState>,
    asset_id: String,
    tag_id: String,
) -> Result<(), UiError> {
    state
        .asset_service
        .reject_tag_suggestion(&asset_id, &tag_id, &AttributionContext::owner_surface())
        .await?;
    Ok(())
}

/// Which visual model this process bound, if any (#112).
#[tauri::command]
pub async fn visual_model_status(
    state: State<'_, AppState>,
) -> Result<asterism_contract::dto::VisualModelStatusDto, UiError> {
    Ok(state.asset_service.visual_model_status().await)
}

/// Which trained head scores tags, and what a next training run would
/// learn from — the command twin of `GET /asterism/heads/status`
/// (#130).
#[tauri::command]
pub async fn head_status(
    state: State<'_, AppState>,
) -> Result<asterism_contract::dto::HeadStatusDto, UiError> {
    Ok(state.asset_service.head_status().await?)
}

/// Enqueues a `HeadTrain` run over the rulings under the bound
/// encoder — the command twin of `POST /asterism/heads/train` (#132).
/// No input: the corpus is whatever rulings exist, so there is
/// nothing to scope. Returns the job's task id; the completion
/// message carries the held-out verdict and whether promotion
/// happened. #130's model panel is where a screen invokes this.
#[tauri::command]
pub async fn train_tag_head(state: State<'_, AppState>) -> Result<String, UiError> {
    Ok(state.asset_service.train_head().await?)
}

/// Enqueues a `HeadPull` install of a fetched head artifact — the
/// command twin of `POST /asterism/heads/pull` (#132 phase 3). The
/// caller fetches the artifact from the instance's registry (which
/// sits behind its own session gate) and hands it over; verification
/// — encoder identity, shapes, the startup-bind checks — is the
/// job's, and installing promotes on the next launch.
#[tauri::command]
pub async fn pull_tag_head(
    state: State<'_, AppState>,
    artifact: serde_json::Value,
) -> Result<String, UiError> {
    Ok(state.asset_service.pull_head(artifact).await?)
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

/// Renames a tag channel in place — the command twin of
/// `POST /asterism/tags/rename`. Rejected when the name belongs to
/// another tag: rename never merges (that is [`merge_tags`]).
#[tauri::command]
pub async fn rename_tag(
    state: State<'_, AppState>,
    command: RenameTagCommand,
) -> Result<TagDto, UiError> {
    Ok(state
        .asset_service
        .rename_tag(command, &AttributionContext::owner_surface())
        .await?)
}

/// Drops a tag channel and every link to it — the command twin of
/// `POST /asterism/tags/delete`. Unlike [`detach_tag`], this removes
/// the channel itself; there is no trash for tags.
#[tauri::command]
pub async fn delete_tag(
    state: State<'_, AppState>,
    command: DeleteTagCommand,
) -> Result<DeleteTagResult, UiError> {
    Ok(state
        .asset_service
        .delete_tag(command, &AttributionContext::owner_surface())
        .await?)
}

/// Folds one tag channel into another and deletes the source — the
/// command twin of `POST /asterism/tags/merge`. `command.dry_run`
/// reports the same numbers without writing; merge is not undoable.
#[tauri::command]
pub async fn merge_tags(
    state: State<'_, AppState>,
    command: MergeTagsCommand,
) -> Result<MergeTagsResult, UiError> {
    Ok(state
        .asset_service
        .merge_tags(command, &AttributionContext::owner_surface())
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

/// Every observation stream on one timeline, newest first — the
/// command twin of `GET /asterism/observations`. Carries the shared
/// envelope only; a stream's own columns stay with that stream's own
/// socket endpoint (diagnostics the desktop does not read).
#[tauri::command]
pub async fn list_observations(
    state: State<'_, AppState>,
    query: ListObservationsQuery,
) -> Result<Vec<ObservationDto>, UiError> {
    Ok(state.observations.all(query).await?)
}

/// The stream names [`list_observations`]'s `stream` filter accepts —
/// the command twin of `GET /asterism/observations/streams`. Published
/// because the set is closed, and a caller guessing at it is how a
/// filter ends up silently doing nothing.
#[tauri::command]
pub async fn list_streams() -> Result<Vec<String>, UiError> {
    Ok(asterism_core::domain::observation::Stream::ALL
        .iter()
        .map(|s| s.as_str().to_string())
        .collect())
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

// -----------------------------------------------------------------
// The forge — a line
//
// The services take typed ids and IPC carries strings, so the parsing
// is here, exactly as it is in the HTTP adapter: an id that is not a
// UUID is malformed input and says so, rather than reaching a service
// that would have to describe a shape it does not accept.
//
// Each verb takes its target as an argument of its own. The commands
// in `asterism-contract` carry that id as a field too, and HTTP fills
// the field in from the path because a path and a body can disagree.
// There is no path here and nothing to disagree with, so the argument
// is the whole answer and the field is left alone. Four verbs whose
// entire input was the id and who is asking — archive, reopen and
// discard on a line, and `resolve` on the pursuit side — take no
// command struct at all: attribution comes from the surface, so
// nothing would be left in it to read.
// -----------------------------------------------------------------

/// Reads a line back after a write, so the caller sees what it now is.
///
/// A write on a line answers with the line. `line_now` in
/// `asterism-server`'s `http` module records why, and why `discard` is
/// the one that answers with something else.
async fn line_now(state: &AppState, id: &LineId) -> Result<ForgeLineDto, UiError> {
    Ok(forge_line_to_dto(&state.line_service.get(id).await?))
}

/// Opens a line.
#[tauri::command]
pub async fn open_forge_line(
    state: State<'_, AppState>,
    command: OpenForgeLineCommand,
) -> Result<ForgeLineDto, UiError> {
    let line = state
        .line_service
        .open(
            forge_name(command.name)?,
            forge_strategy_id(command.strategy_id)?,
            &AttributionContext::owner_surface(),
        )
        .await?;
    Ok(forge_line_to_dto(&line))
}

/// Every line, without its history.
#[tauri::command]
pub async fn list_forge_lines(state: State<'_, AppState>) -> Result<Vec<ForgeLineDto>, UiError> {
    let lines = state.line_service.list().await?;
    Ok(lines.iter().map(forge_line_to_dto).collect())
}

/// The line and its whole history.
///
/// This grows with the line. A surface showing what is *on* a line
/// wants [`get_forge_line_states`] instead; this one is for showing how
/// it got there.
#[tauri::command]
pub async fn get_forge_line(
    state: State<'_, AppState>,
    line_id: String,
) -> Result<ForgeLineHistoryDto, UiError> {
    let line = state
        .line_service
        .get(&forge_line_id(&line_id, "line id")?)
        .await?;
    Ok(forge_history_to_dto(&line))
}

/// What is on the line, folded from the chain.
#[tauri::command]
pub async fn get_forge_line_states(
    state: State<'_, AppState>,
    line_id: String,
) -> Result<Vec<ForgeEntryStateDto>, UiError> {
    let states = state
        .line_service
        .states(&forge_line_id(&line_id, "line id")?)
        .await?;
    Ok(forge_states_to_dto(&states))
}

/// Moves the line's own description. Not a landing: nothing goes on the
/// chain.
#[tauri::command]
pub async fn rename_forge_line(
    state: State<'_, AppState>,
    line_id: String,
    command: RenameForgeLineCommand,
) -> Result<ForgeLineDto, UiError> {
    let id = forge_line_id(&line_id, "line id")?;
    state
        .line_service
        .rename(
            &id,
            &forge_name(command.name)?,
            &AttributionContext::owner_surface(),
        )
        .await?;
    line_now(&state, &id).await
}

/// Points the line at a different rule, from here on.
#[tauri::command]
pub async fn set_forge_line_strategy(
    state: State<'_, AppState>,
    line_id: String,
    command: SetForgeLineStrategyCommand,
) -> Result<ForgeLineDto, UiError> {
    let id = forge_line_id(&line_id, "line id")?;
    state
        .line_service
        .set_strategy(
            &id,
            &forge_strategy_id(command.strategy_id)?,
            &AttributionContext::owner_surface(),
        )
        .await?;
    line_now(&state, &id).await
}

/// Finished with. Takes no landing until it is reopened, and is the
/// only state a drop can be reached from.
#[tauri::command]
pub async fn archive_forge_line(
    state: State<'_, AppState>,
    line_id: String,
) -> Result<ForgeLineDto, UiError> {
    let id = forge_line_id(&line_id, "line id")?;
    state
        .line_service
        .archive(&id, &AttributionContext::owner_surface())
        .await?;
    line_now(&state, &id).await
}

/// Takes it back out.
#[tauri::command]
pub async fn reopen_forge_line(
    state: State<'_, AppState>,
    line_id: String,
) -> Result<ForgeLineDto, UiError> {
    let id = forge_line_id(&line_id, "line id")?;
    state
        .line_service
        .reopen(&id, &AttributionContext::owner_surface())
        .await?;
    line_now(&state, &id).await
}

/// Takes the line, its history and every piece of work against it.
///
/// **The answer is the point.** It names the assets the drop released,
/// and after this write there is no record left to derive them from, so
/// a caller that ignores it has lost the only answer there will be.
#[tauri::command]
pub async fn discard_forge_line(
    state: State<'_, AppState>,
    line_id: String,
) -> Result<ForgeDiscardedDto, UiError> {
    let id = forge_line_id(&line_id, "line id")?;
    let released = state
        .line_service
        .discard(&id, &AttributionContext::owner_surface())
        .await?;
    Ok(forge_discarded_to_dto(id, &released))
}

/// Every rule a line can be pointed at, built from the rules this
/// deployment carries.
#[tauri::command]
pub async fn list_forge_strategies(
    state: State<'_, AppState>,
) -> Result<Vec<ForgeStrategyDto>, UiError> {
    let rules = state.line_service.strategies().await;
    Ok(rules
        .iter()
        .map(|(id, about)| forge_strategy_to_dto(id, about))
        .collect())
}

// -----------------------------------------------------------------
// The forge — work against a line
// -----------------------------------------------------------------

/// Reads work back after a write, so the caller sees what it now is.
///
/// `push` and `close` answer with the pursuit whole, and `resolve` does
/// not; `pursuit_now` in `asterism-server`'s `http` module records why
/// that is not an exception.
async fn pursuit_now(state: &AppState, id: &PursuitId) -> Result<ForgePursuitDto, UiError> {
    Ok(forge_pursuit_to_dto(&state.pursuit_service.get(id).await?))
}

/// Opens work against a line.
///
/// The line is named in the command rather than as an argument, because
/// this is the one verb here that does not have a pursuit to name yet.
/// A caller asking "what work is against this line" wants
/// [`list_forge_pursuits_of_line`].
#[tauri::command]
pub async fn open_forge_pursuit(
    state: State<'_, AppState>,
    command: OpenForgePursuitCommand,
) -> Result<ForgePursuitDto, UiError> {
    let line = forge_line_id(&command.line_id, "line id")?;
    let parent = command
        .parent_id
        .as_deref()
        .map(|raw| forge_pursuit_id(raw, "pursuit id"))
        .transpose()?;
    let intent = Intent {
        title: command.title.map(forge_name).transpose()?,
        note: command.note,
    };
    let pursuit = state
        .pursuit_service
        .open(&line, parent, intent, &AttributionContext::owner_surface())
        .await?;
    Ok(forge_pursuit_to_dto(&pursuit))
}

/// The work, whole — one read rather than the line's two.
#[tauri::command]
pub async fn get_forge_pursuit(
    state: State<'_, AppState>,
    pursuit_id: String,
) -> Result<ForgePursuitDto, UiError> {
    let id = forge_pursuit_id(&pursuit_id, "pursuit id")?;
    pursuit_now(&state, &id).await
}

/// Writes a round.
///
/// The line is not read, which is what makes this the operation that
/// can run all day without touching anything anybody else is using.
/// What it does check is that the content each operation names exists.
#[tauri::command]
pub async fn push_forge_round(
    state: State<'_, AppState>,
    pursuit_id: String,
    command: PushForgeRoundCommand,
) -> Result<ForgePursuitDto, UiError> {
    let id = forge_pursuit_id(&pursuit_id, "pursuit id")?;
    let ops = command
        .ops
        .iter()
        .map(forge_op)
        .collect::<Result<Vec<_>, _>>()?;
    state
        .pursuit_service
        .push(&id, ops, command.note, &AttributionContext::owner_surface())
        .await?;
    pursuit_now(&state, &id).await
}

/// Lets the line's rule answer whatever this work collides with.
///
/// **It succeeds whether or not a round was written, and the answer
/// says which.** A rule that leaves collisions to a person writes
/// nothing; that is an outcome, not a failure, so `round` is absent
/// rather than this being an error, and `collisions` carries what is
/// left either way.
#[tauri::command]
pub async fn resolve_forge_pursuit(
    state: State<'_, AppState>,
    pursuit_id: String,
) -> Result<ForgeResolvedDto, UiError> {
    let id = forge_pursuit_id(&pursuit_id, "pursuit id")?;
    let round = state
        .pursuit_service
        .resolve(&id, &AttributionContext::owner_surface())
        .await?;
    let collisions = state.pursuit_service.collisions(&id).await?;
    Ok(ForgeResolvedDto {
        round: round.as_ref().map(forge_round_to_dto),
        collisions: forge_collisions_to_dto(&collisions),
    })
}

/// Ends the work, and puts what it says on the line if it says
/// anything.
///
/// **A [`UiError::Conflict`] from here is not always worth retrying,
/// and its `reason` says which kind it is:** `"blocked"` wants
/// something done first and the message says what, `"raced"` will
/// usually win on a retry, `"settled"` never helps, and `"clashes"` has
/// to ask for something different. They are spelled out on
/// `close_forge_pursuit` in `asterism-server`'s `http` module,
/// including why `"blocked"` arrives for two different reasons.
#[tauri::command]
pub async fn close_forge_pursuit(
    state: State<'_, AppState>,
    pursuit_id: String,
    command: CloseForgePursuitCommand,
) -> Result<ForgePursuitDto, UiError> {
    let id = forge_pursuit_id(&pursuit_id, "pursuit id")?;
    let outcome = forge_outcome(&command.outcome)?;
    state
        .pursuit_service
        .close(
            &id,
            outcome,
            command.note,
            &AttributionContext::owner_surface(),
        )
        .await?;
    pursuit_now(&state, &id).await
}

/// What this work still asks for that the line has moved since.
///
/// Derived from both logs on every call, so it cannot go stale. This is
/// what a screen shows before offering [`resolve_forge_pursuit`].
#[tauri::command]
pub async fn get_forge_pursuit_collisions(
    state: State<'_, AppState>,
    pursuit_id: String,
) -> Result<Vec<ForgeCollisionDto>, UiError> {
    let found = state
        .pursuit_service
        .collisions(&forge_pursuit_id(&pursuit_id, "pursuit id")?)
        .await?;
    Ok(forge_collisions_to_dto(&found))
}

/// The landings this work has not seen, oldest first.
///
/// How far behind rather than what collides: a landing here may touch
/// nothing this work asks for. A screen reads both — this one to say
/// the line has moved, [`get_forge_pursuit_collisions`] to say whether
/// it matters.
#[tauri::command]
pub async fn get_forge_pursuit_behind(
    state: State<'_, AppState>,
    pursuit_id: String,
) -> Result<Vec<String>, UiError> {
    let behind = state
        .pursuit_service
        .behind(&forge_pursuit_id(&pursuit_id, "pursuit id")?)
        .await?;
    Ok(behind.iter().map(|id| id.to_string()).collect())
}

/// Work opened from this work.
#[tauri::command]
pub async fn list_forge_pursuit_children(
    state: State<'_, AppState>,
    pursuit_id: String,
) -> Result<Vec<ForgePursuitDto>, UiError> {
    let found = state
        .pursuit_service
        .children(&forge_pursuit_id(&pursuit_id, "pursuit id")?)
        .await?;
    Ok(found.iter().map(forge_pursuit_to_dto).collect())
}

/// Every piece of work against a line, open and ended alike.
#[tauri::command]
pub async fn list_forge_pursuits_of_line(
    state: State<'_, AppState>,
    line_id: String,
) -> Result<Vec<ForgePursuitDto>, UiError> {
    let found = state
        .pursuit_service
        .of_line(&forge_line_id(&line_id, "line id")?)
        .await?;
    Ok(found.iter().map(forge_pursuit_to_dto).collect())
}

// -----------------------------------------------------------------
// The forge — what was said about work
// -----------------------------------------------------------------

/// Reads a conversation back after a write.
///
/// `say` and `amend` do not use this: what each of them wrote is the
/// answer, which `thread_now` in `asterism-server`'s `http` module
/// records as the point rather than an economy.
async fn thread_now(state: &AppState, id: &ThreadId) -> Result<ForgeThreadDto, UiError> {
    Ok(forge_thread_to_dto(
        &state.forge_thread_service.get(id).await?,
    ))
}

/// Opens a conversation about something in the forge.
///
/// **The anchor is resolved, not accepted.** The command names ids and
/// a kind; the service reads the pursuit or the line and the model
/// builds the anchor from what it finds, so a pursuit nobody opened and
/// an entry the round never touched are two different refusals.
#[tauri::command]
pub async fn open_forge_thread(
    state: State<'_, AppState>,
    command: OpenForgeThreadCommand,
) -> Result<ForgeThreadDto, UiError> {
    let about = forge_anchored(
        &command.anchor_kind,
        command.pursuit_id.as_deref(),
        command.line_id.as_deref(),
        command.node_id.as_deref(),
        command.entry_id.as_deref(),
        command.change_point_id.as_deref(),
    )?;
    let title = command.title.map(forge_name).transpose()?;
    let thread = state
        .forge_thread_service
        .open(
            about,
            title,
            forge_body(command.said)?,
            &AttributionContext::owner_surface(),
        )
        .await?;
    Ok(forge_thread_to_dto(&thread))
}

/// The conversation, whole — every message and every correction to
/// each.
///
/// A payload carrying only what each message says now would leave a
/// withdrawn sentence attributed to the person who withdrew it.
#[tauri::command]
pub async fn get_forge_thread(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<ForgeThreadDto, UiError> {
    let id = forge_thread_id(&thread_id, "thread id")?;
    thread_now(&state, &id).await
}

/// Says something.
///
/// Answers with the message it wrote. A reply naming a message of
/// another conversation is refused as invalid input: the caller
/// addressed one conversation and named something that is not in it,
/// which no change of state makes true.
#[tauri::command]
pub async fn say_in_forge_thread(
    state: State<'_, AppState>,
    thread_id: String,
    command: SayInForgeThreadCommand,
) -> Result<ForgeMessageDto, UiError> {
    let id = forge_thread_id(&thread_id, "thread id")?;
    let replying_to = command
        .replying_to
        .as_deref()
        .map(|raw| forge_message_id(raw, "replying_to"))
        .transpose()?;
    let said = state
        .forge_thread_service
        .say(
            &id,
            replying_to,
            forge_body(command.said)?,
            &AttributionContext::owner_surface(),
        )
        .await?;
    Ok(forge_message_to_dto(&said))
}

/// Corrects something said.
///
/// **Answers with the correction, not with the message as it now
/// reads.** The model keeps both and the distinction is the model's:
/// what was said first is still there, and an answer shaped as "the
/// message, updated" would quietly be the shape that loses it.
#[tauri::command]
pub async fn amend_forge_message(
    state: State<'_, AppState>,
    thread_id: String,
    command: AmendForgeMessageCommand,
) -> Result<ForgeRevisionDto, UiError> {
    let id = forge_thread_id(&thread_id, "thread id")?;
    let message = forge_message_id(&command.message_id, "message id")?;
    let revision = state
        .forge_thread_service
        .amend(
            &id,
            &message,
            forge_body(command.said)?,
            &AttributionContext::owner_surface(),
        )
        .await?;
    Ok(forge_revision_to_dto(&revision))
}

/// Names the conversation, or takes its name off.
///
/// A title is a label on the conversation rather than something said in
/// it, so this writes no message. `title` absent takes the name off.
#[tauri::command]
pub async fn rename_forge_thread(
    state: State<'_, AppState>,
    thread_id: String,
    command: RenameForgeThreadCommand,
) -> Result<ForgeThreadDto, UiError> {
    let id = forge_thread_id(&thread_id, "thread id")?;
    let title = command.title.map(forge_name).transpose()?;
    state
        .forge_thread_service
        .rename(&id, title.as_ref(), &AttributionContext::owner_surface())
        .await?;
    thread_now(&state, &id).await
}

/// Conversations about one thing in the forge — the work as a whole,
/// one round of it, one entry as that round had it, or what landed on a
/// line.
///
/// **One command where HTTP has four routes.** There are four there
/// because a path cannot express a wrong combination of ids, and that
/// property is the route's rather than the question's: IPC has no path,
/// so the combination has to be checked wherever it arrives. It is
/// checked here, by the same resolver [`open_forge_thread`] names its
/// anchor through — which is the shape the forge already uses when
/// there is no path to lean on, and the shape [`list_threads`] uses for
/// the same job on the app-level Threads. `anchor_kind` says which ids
/// are wanted: `"pursuit"` wants `pursuit_id`, `"round"` adds
/// `node_id`, `"entry"` adds `entry_id` to that, and `"change"` wants
/// `line_id` with `change_point_id`.
///
/// Both directions are refused, and the second is the one that matters:
/// a missing id is obvious, while an id the kind has no use for would
/// otherwise be ignored — `"round"` carrying an `entry_id` would answer
/// about the round for a caller that asked about the entry, which is
/// exactly what the four routes exist to make impossible.
///
/// More than one conversation can hang off the same thing — two people
/// starting separate ones about a round is not a mistake to merge — so
/// this answers a list.
#[tauri::command]
pub async fn list_forge_threads_about(
    state: State<'_, AppState>,
    anchor_kind: String,
    pursuit_id: Option<String>,
    line_id: Option<String>,
    node_id: Option<String>,
    entry_id: Option<String>,
    change_point_id: Option<String>,
) -> Result<Vec<ForgeThreadDto>, UiError> {
    let about = forge_anchored(
        &anchor_kind,
        pursuit_id.as_deref(),
        line_id.as_deref(),
        node_id.as_deref(),
        entry_id.as_deref(),
        change_point_id.as_deref(),
    )?;
    let found = state.forge_thread_service.about(about).await?;
    Ok(found.iter().map(forge_thread_to_dto).collect())
}

// -----------------------------------------------------------------
// Lines a team hosts (#148 decisions 10, 11 and 16).
//
// Everything below reaches a team's server, which is the difference
// from the block above. A shared line is served through rather than
// mirrored, so each of these reads is a request and there is no local
// copy to be out of date with — which is also why they answer the same
// DTOs the local verbs do. Two sources, one vocabulary, and a panel
// that keeps them apart.
//
// Two of them also write here, and it is worth saying which, because
// "goes to a server" would otherwise read as "touches nothing local".
// `clone_shared_entry` records an asset on this machine — that is what
// a clone is. `publish_line_to_team` writes the relation rows a
// promotion leaves at home (#148 decision 8). Neither of those is a
// reason for the block to sit anywhere else; the reads and the writes
// share a connection, and splitting them would put the connection in
// two places.
//
// The attribution question has a different answer here too. A write in
// the block above is the owner's, because the surface says so. A write
// that lands on a team is the authenticated member's, because the team
// says so — the client states no author at all and the server stamps
// the session. So `owner_surface` appears below exactly once, on the
// clone, and it is right there for the reason it is right anywhere:
// the clone's write is a local import, on the owner's own machine,
// through the owner's own window.
// -----------------------------------------------------------------

/// Turns what went wrong with a team into what the window shows.
///
/// A refusal the server wrote keeps its own words — the panel is
/// telling somebody why the team said no, and "internal error" is not
/// that. Everything else the client can fail at is this machine's
/// business and reads as one.
fn teams_error(err: asterism_teams_client::TeamsClientError) -> UiError {
    use asterism_teams_client::TeamsClientError as E;
    match err {
        E::Local(domain) => UiError::from(domain),
        E::Refused {
            status: 404,
            message,
            ..
        } => UiError::from(DomainError::not_found("on the team server", message)),
        E::Refused {
            status, message, ..
        } if (400..500).contains(&status) => UiError::from(DomainError::Validation(message)),
        other => UiError::from(DomainError::Infra(anyhow::anyhow!("{other}"))),
    }
}

/// The team connection this window is holding, or a refusal saying it
/// is holding none.
///
/// The pair travels with the session because some verbs have to decide
/// whether what this machine remembers is what it is talking to —
/// [`stored_connection`](crate::stored_connection)'s "which connection
/// a verb acts on". Most verbs want only the session and take
/// [`teams_client`].
async fn teams_connection(state: &AppState) -> Result<TeamsConnection, UiError> {
    state.teams.lock().await.clone().ok_or_else(|| {
        UiError::from(DomainError::Validation(
            "this window is not connected to a team server; connect to one first".into(),
        ))
    })
}

/// The team server this window is talking to, or a refusal saying it is
/// talking to none.
async fn teams_client(state: &AppState) -> Result<asterism_teams_client::TeamsClient, UiError> {
    Ok(teams_connection(state).await?.client)
}

fn team_id(raw: &str, what: &'static str) -> Result<TeamScopedId, UiError> {
    Ok(TeamScopedId::parse(raw, what)?)
}

/// Whose account a server just accepted, or a refusal saying it named
/// nobody.
///
/// Both login arms ask this, and neither has anything to do with an
/// answer that authenticated somebody the client cannot name.
fn accepted_as(client: &asterism_teams_client::TeamsClient) -> Result<String, UiError> {
    client.user_id().map(ToString::to_string).ok_or_else(|| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "the server accepted the login and named nobody"
        )))
    })
}

/// Whether a refusal was a `401`.
///
/// The single fact the stored-credential path turns on: what this
/// machine does is the same for every reason the body carries — forget
/// the token — because a credential the server has said no to is not
/// one to present again. Which reason it was travels separately, on
/// `StoredTeamConnectDto::reason`, for the drawer to tell the person.
fn was_unauthorized(err: &asterism_teams_client::TeamsClientError) -> bool {
    matches!(
        err,
        asterism_teams_client::TeamsClientError::Refused { status: 401, .. }
    )
}

/// The instance id a session said, or `None` from a server too old to
/// say one — which is what lets `stored_connection`'s comparison fall
/// back to the URL for it.
fn known_instance(id: &str) -> Option<&str> {
    (!id.is_empty()).then_some(id)
}

/// Who a session says it is for and on which instance, once a client
/// holds one.
fn session_identity(
    client: &asterism_teams_client::TeamsClient,
) -> Result<(String, String), UiError> {
    let session = client.session().ok_or_else(|| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "the server accepted the sign-in and handed back no session"
        )))
    })?;
    Ok((session.login.clone(), session.instance_id.clone()))
}

/// Logs this window in to a team server and holds the session.
///
/// `remember` is the mint: with it, the session this login opened is
/// used once more to ask the server for a device token, which goes in
/// this machine's keychain so the next window reconnects without a
/// password. What that stores and where is
/// [`stored_connection`](crate::stored_connection); the password is
/// not part of it under any key.
///
/// Being remembered is not a condition of connecting. A mint that
/// fails leaves a window logged in and a box that will have to be
/// ticked again, which is the honest outcome — refusing the connection
/// over it would take away the thing that did work.
///
/// That is why the session is installed **before** the mint is
/// attempted rather than after it, and why the mint's refusal does not
/// leave this function: a locked keychain, a denied prompt and a
/// server that would not mint all end here as a warning in the diag
/// log and a connected window. Nothing tells the person, which is a
/// real cost and the smaller one — the box is unticked next time they
/// look, and a modal saying a credential was not saved is a modal
/// about something they can simply do again. Saying so on the return
/// shape is the alternative, and it puts a field across the wire, the
/// bindings and the store to carry a fact nothing acts on.
#[tauri::command]
pub async fn connect_team_server(
    state: State<'_, AppState>,
    base_url: String,
    login: String,
    password: String,
    remember: bool,
) -> Result<String, UiError> {
    let mut client = asterism_teams_client::TeamsClient::new(base_url.clone());
    client.login(&login, &password).await.map_err(teams_error)?;
    let user = accepted_as(&client)?;
    let (_, instance_id) = session_identity(&client)?;
    *state.teams.lock().await = Some(TeamsConnection {
        client: client.clone(),
        base_url: base_url.clone(),
        login: login.clone(),
        instance_id: instance_id.clone(),
    });
    if remember
        && let Err(err) = remember_this_device(&client, &base_url, &login, &instance_id).await
    {
        tracing::warn!(
            error = %err,
            "the connection stands; this machine was not able to remember it",
        );
    }
    Ok(user)
}

/// What a team server offers besides a password (#163), or `None`.
///
/// Asked of a server nobody is signed in to, which is the point: the
/// connect form reads it to know whether to show the provider's button
/// beside the password fields. A server that cannot be reached is an
/// error here rather than `None`, because this command answers a fact
/// and "no answer" is not "no provider"; the drawer's store folds the
/// two into "no button", and says why there.
#[tauri::command]
pub async fn team_auth_provider(base_url: String) -> Result<Option<TeamProviderDto>, UiError> {
    let client = asterism_teams_client::TeamsClient::new(base_url);
    let providers = client.auth_providers().await.map_err(teams_error)?;
    Ok(providers.oidc.map(|provider| TeamProviderDto {
        name: provider.name,
    }))
}

/// Signs this window in through the team server's identity provider
/// (#163), and holds the session.
///
/// The app's end of the shape `provider_sign_in` describes: a listener
/// on loopback, an attempt started with a secret's hash and the port,
/// the system browser opened at the server's page, and the session
/// collected once the browser has come back with the grant. The person
/// never types a login here — the session says it — and from the
/// session on this is [`connect_team_server`]: the same connection,
/// the same mint when `remember` is ticked, the same keychain.
///
/// Waits about as long as the attempt is good for, and the listener
/// goes with the wait. A browser tab closed without finishing is a
/// wait that ends with a refusal saying so; a provider that refused is
/// a refusal saying that; both leave the window where it was.
#[tauri::command]
pub async fn connect_team_server_provider(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    base_url: String,
    remember: bool,
) -> Result<String, UiError> {
    use tauri_plugin_opener::OpenerExt as _;

    let mut client = asterism_teams_client::TeamsClient::new(base_url.clone());
    let provider = client
        .auth_providers()
        .await
        .map_err(teams_error)?
        .oidc
        .ok_or_else(|| {
            UiError::from(DomainError::Validation(
                "this team server signs people in with passwords only".into(),
            ))
        })?;
    let listener = crate::provider_sign_in::Listener::bind()
        .await
        .map_err(|err| {
            UiError::from(DomainError::Infra(anyhow::anyhow!(
                "could not listen on loopback for the browser to come back: {err}"
            )))
        })?;
    let secret = crate::provider_sign_in::Secret::new().map_err(|err| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "could not make a secret for the sign-in: {err}"
        )))
    })?;
    let attempt = client
        .start_oidc_attempt(
            &secret.collector(),
            &crate::stored_connection::device_label(),
            listener.port(),
        )
        .await
        .map_err(teams_error)?;
    let done_url = format!("{}/done", attempt.start_url);
    let waiting = listener.serve(attempt.attempt_id.clone(), done_url);
    app.opener()
        .open_url(&attempt.start_url, None::<&str>)
        .map_err(|err| {
            UiError::from(DomainError::Infra(anyhow::anyhow!(
                "could not open the browser for the sign-in: {err}"
            )))
        })?;
    // How long to wait is the attempt's own life, which the server
    // states as an instant on its clock. Read on this machine's clock
    // that can come out as nothing at all when the two disagree, so
    // the wait is held to at least a minute and at most a quarter of
    // an hour: a person who finishes inside a minute on a fast clock
    // is still collected, and a server that has already expired the
    // attempt answers the collect with the refusal that says so.
    let remaining = attempt
        .expires_at_ms
        .saturating_sub(chrono::Utc::now().timestamp_millis())
        .clamp(60_000, 15 * 60_000) as u64;
    let outcome = tokio::time::timeout(std::time::Duration::from_millis(remaining), waiting)
        .await
        .map_err(|_| {
            UiError::from(DomainError::Validation(format!(
                "the sign-in through {} was not finished in time; try again",
                provider.name
            )))
        })?
        .map_err(|_| {
            UiError::from(DomainError::Infra(anyhow::anyhow!(
                "the loopback listener went away before the browser came back"
            )))
        })?;
    let grant = match outcome {
        crate::provider_sign_in::Outcome::Granted(grant) => grant,
        // One refusal on the wire for three causes — the provider sent
        // no code, the token did not verify, the person is nobody this
        // instance knows — and the app cannot tell them apart, which
        // is the server's one-armed answer doing its job. The sentence
        // names the cause a person can act on and hedges the rest.
        crate::provider_sign_in::Outcome::Refused => {
            return Err(UiError::from(DomainError::Validation(format!(
                "the team server did not accept the sign-in through {}; if you expected to \
                 be let in, ask whoever runs it to check that your address is bound to your \
                 account",
                provider.name
            ))));
        }
    };
    client
        .collect_oidc_attempt(&attempt.attempt_id, secret.value(), &grant)
        .await
        .map_err(teams_error)?;
    let user = accepted_as(&client)?;
    let (login, instance_id) = session_identity(&client)?;
    *state.teams.lock().await = Some(TeamsConnection {
        client: client.clone(),
        base_url: base_url.clone(),
        login: login.clone(),
        instance_id: instance_id.clone(),
    });
    if remember
        && let Err(err) = remember_this_device(&client, &base_url, &login, &instance_id).await
    {
        tracing::warn!(
            error = %err,
            "the connection stands; this machine was not able to remember it",
        );
    }
    Ok(user)
}

/// Mints a device token on a live session and stores the pair, in the
/// order `stored_connection`'s "what a remember retires, and in what
/// order" fixes: the displaced entry first, the file second, the entry
/// the file has just named last. What a crash at each step leaves, and
/// why the order was once the other one, are argued there and not
/// here.
///
/// What is this site's: which pair is being displaced can only be read
/// before the write, so the old metadata is read at the top and
/// carried across all three writes.
/// [`retire_replaced`](crate::stored_connection::retire_replaced)
/// takes the old keychain entry when the write goes under another key,
/// and [`superseded_row`](crate::stored_connection::superseded_row)
/// names the server row left over when the pair is this connection's
/// — revoked here on the live client, which is the server that issued
/// it in exactly that case. That revoke is best effort in
/// [`disconnect_team_server`]'s sense: a remember that stored the
/// credential did the thing that was asked for, and a stale row must
/// not turn it into a refusal, so a failure is logged and the row is
/// left to its expiry.
async fn remember_this_device(
    client: &asterism_teams_client::TeamsClient,
    base_url: &str,
    login: &str,
    instance_id: &str,
) -> Result<(), UiError> {
    let label = crate::stored_connection::device_label();
    let instance = known_instance(instance_id);
    let displaced = crate::stored_connection::read_metadata().await;
    let minted = client
        .mint_device_token(&label)
        .await
        .map_err(teams_error)?;
    let superseded =
        crate::stored_connection::superseded_row(displaced.as_ref(), base_url, login, instance);
    crate::stored_connection::retire_replaced(displaced.as_ref(), base_url, login, instance).await;
    let stored = crate::stored_connection::StoredConnection {
        base_url: base_url.to_string(),
        login: login.to_string(),
        token_id: minted.id,
        label,
        instance_id: instance.map(str::to_string),
    };
    crate::stored_connection::write_metadata(&stored).await?;
    crate::stored_connection::write_token(&stored.key(), &minted.token).await?;
    if let Some(superseded) = superseded
        && let Err(err) = client.revoke_device_token(superseded).await
    {
        tracing::warn!(
            error = %err,
            "this pair was remembered again; the row the new token replaced \
             is left to expire",
        );
    }
    Ok(())
}

/// What the connect form pre-fills from, or `None` when this machine
/// remembers no server.
///
/// Every field is safe to hand a window, which
/// [`StoredConnection`](crate::stored_connection::StoredConnection) is
/// where it is argued: the token has no path into that shape.
#[tauri::command]
pub async fn stored_team_connection() -> Result<Option<StoredTeamConnectionDto>, UiError> {
    Ok(crate::stored_connection::read_metadata()
        .await
        .map(|stored| StoredTeamConnectionDto {
            base_url: stored.base_url,
            login: stored.login,
            token_id: stored.token_id,
            label: stored.label,
        }))
}

/// Reconnects from the device token this machine holds, without asking
/// anybody anything.
///
/// The silent half of #204: a restart reaches its team from the
/// keychain, and the password form is what a rejection falls back to.
/// What lands in [`AppState::teams`](crate::state::AppState::teams) is
/// an ordinary session — the same one
/// [`connect_team_server`] hands over — so nothing downstream knows
/// which arm was used.
///
/// A `401` forgets both halves before answering. Keeping a token the
/// server has said no to buys a second silent failure on the next
/// launch and a listing row nobody can match to a machine.
///
/// Which of the three ends this reached, and why none of them is an
/// error, is on [`StoredTeamConnectOutcome`].
#[tauri::command]
pub async fn connect_team_server_stored(
    state: State<'_, AppState>,
) -> Result<StoredTeamConnectDto, UiError> {
    let nothing = StoredTeamConnectDto {
        outcome: StoredTeamConnectOutcome::Nothing,
        user: None,
        reason: None,
    };
    let Some(stored) = crate::stored_connection::read_metadata().await else {
        return Ok(nothing);
    };
    let token = match crate::stored_connection::read_token(&stored.key()).await {
        crate::stored_connection::StoredToken::Held(token) => token,
        // The keychain answered, and its answer is that there is no
        // entry: the file is describing a connection that cannot be
        // made, and it is the sole index of the keychain, so keeping
        // it buys a form that pre-fills into nothing.
        crate::stored_connection::StoredToken::NoEntry => {
            crate::stored_connection::delete_metadata().await;
            return Ok(nothing);
        }
        // The keychain would not answer — locked, a prompt dismissed,
        // no store configured. **The file stays.** The entry is very
        // likely still there, and deleting the only thing that names
        // it over one denied prompt is how a live credential becomes
        // one nothing knows how to revoke. This launch shows the
        // password form; the next one asks the keychain again.
        crate::stored_connection::StoredToken::Unavailable => {
            tracing::warn!(
                "this machine's keychain would not answer for the stored \
                 connection; it is kept and will be tried again",
            );
            return Ok(nothing);
        }
    };

    let mut client = asterism_teams_client::TeamsClient::new(stored.base_url.clone());
    match client.login_with_device_token(&token).await {
        Ok(_) => {
            let user = accepted_as(&client)?;
            let (_, instance_id) = session_identity(&client)?;
            // An entry from before the instance id existed is moved
            // onto the key it is named by from now on, the first time
            // a session says the id (#163). `stored_connection`'s
            // `rekey` is where the order of the three writes is
            // argued.
            let stored = match (&stored.instance_id, known_instance(&instance_id)) {
                (None, Some(id)) => crate::stored_connection::rekey(&stored, id, &token).await,
                _ => stored,
            };
            *state.teams.lock().await = Some(TeamsConnection {
                client,
                base_url: stored.base_url,
                login: stored.login,
                instance_id,
            });
            Ok(StoredTeamConnectDto {
                outcome: StoredTeamConnectOutcome::Connected,
                user: Some(user),
                reason: None,
            })
        }
        Err(err) if was_unauthorized(&err) => {
            crate::stored_connection::forget(&stored).await;
            let reason = match &err {
                asterism_teams_client::TeamsClientError::Refused { reason, .. } => reason.clone(),
                _ => None,
            };
            Ok(StoredTeamConnectDto {
                outcome: StoredTeamConnectOutcome::Rejected,
                user: None,
                reason,
            })
        }
        Err(err) => Err(teams_error(err)),
    }
}

/// Drops the session, and drops what this machine was keeping.
///
/// **Signing out revokes the device token; closing the window does
/// not.** The issue leaves this open and this is the answer: pressing
/// disconnect is somebody saying they are done here, and a stored
/// credential outliving that would make the button a lie on every
/// machine but the server's listing. A window closing says nothing of
/// the kind — nothing in this process runs on the way out, which is
/// what makes "close the window" the gesture that keeps the
/// connection.
///
/// The revoke is best effort in the same sense the logout already is:
/// a server that cannot be reached to be told must not be able to hold
/// somebody in a session, so the local half happens either way. What
/// that costs is a row left in the owner's listing when the server was
/// down, revocable from the panel the next time it answers, and the
/// token behind it is gone from this machine regardless.
///
/// **What is revoked and forgotten is the connection being ended, not
/// whatever the file happens to name.** The two come apart on a path
/// that is reached rather than theoretical — a remembered server that
/// was down leaves its metadata in place and its login in the connect
/// form, which is editable over — and a verb that followed the file
/// there would send one server's handle to another and wipe the first
/// one's entry while its row stayed live. So the stored half is
/// touched only when it names this connection, and otherwise this is a
/// logout and nothing more:
/// [`stored_connection`](crate::stored_connection)'s "which connection
/// a verb acts on" is where that rule is stated for all three verbs
/// that ask it.
#[tauri::command]
pub async fn disconnect_team_server(state: State<'_, AppState>) -> Result<(), UiError> {
    // Taken once, before either call to the server. The session is
    // gone from this window the moment somebody presses the button —
    // what follows is telling the server about it — and holding the
    // lock across two network round trips would make every other
    // command wait on a server that may be the reason this is being
    // pressed.
    let Some(connection) = state.teams.lock().await.take() else {
        return Ok(());
    };
    if let Some(stored) = crate::stored_connection::read_metadata().await
        && stored.names(
            &connection.base_url,
            &connection.login,
            known_instance(&connection.instance_id),
        )
    {
        let _ = connection
            .client
            .revoke_device_token(&stored.token_id)
            .await;
        crate::stored_connection::forget(&stored).await;
    }
    // Best effort: the local session is already gone, and a server
    // that cannot be reached to be told will expire it.
    let mut client = connection.client;
    let _ = client.logout().await;
    Ok(())
}

/// The device tokens this account holds, on whatever machines.
///
/// Owner-scoped by the route rather than by anything here, which the
/// wire's `DeviceTokensDto` states where it is defined. Mapped to the
/// contract for the reason the roster's and the ledger's are — the
/// wire is what a member's client and a team server say to each other,
/// and a screen holds one vocabulary.
#[tauri::command]
pub async fn list_team_device_tokens(
    state: State<'_, AppState>,
) -> Result<TeamDeviceTokensDto, UiError> {
    let client = teams_client(&state).await?;
    let held = client.list_device_tokens().await.map_err(teams_error)?;
    Ok(TeamDeviceTokensDto {
        tokens: held
            .tokens
            .into_iter()
            .map(|token| TeamDeviceTokenDto {
                id: token.id,
                label: token.label,
                created_at_ms: token.created_at_ms,
                last_used_at_ms: token.last_used_at_ms,
                expires_at_ms: token.expires_at_ms,
            })
            .collect(),
    })
}

/// Revokes one of this account's device tokens.
///
/// Revoking the row this machine is holding forgets it here too, so
/// the next launch shows the password form rather than presenting a
/// token the server has already dropped. Revoking another machine's
/// row leaves this one alone — that is the point of the listing.
///
/// The revoke itself needs no identity rule: it goes down the live
/// client, so the connection it acts on is the connection in hand by
/// construction. The rule applies to the second half, where the file
/// is consulted — a handle is unique to the server that issued it, so
/// the stored pair has to name this connection before a matching
/// `token_id` means the row just dropped was the one this machine
/// holds. That is
/// [`stored_connection`](crate::stored_connection)'s "which connection
/// a verb acts on", third case.
#[tauri::command]
pub async fn revoke_team_device_token(
    state: State<'_, AppState>,
    token_id: String,
) -> Result<(), UiError> {
    let connection = teams_connection(&state).await?;
    connection
        .client
        .revoke_device_token(&token_id)
        .await
        .map_err(teams_error)?;
    if let Some(stored) = crate::stored_connection::read_metadata().await
        && stored.names(
            &connection.base_url,
            &connection.login,
            known_instance(&connection.instance_id),
        )
        && stored.token_id == token_id
    {
        crate::stored_connection::forget(&stored).await;
    }
    Ok(())
}

/// Whether this window is talking to a team server.
#[tauri::command]
pub async fn team_server_session(state: State<'_, AppState>) -> Result<Option<String>, UiError> {
    Ok(state
        .teams
        .lock()
        .await
        .as_ref()
        .and_then(|held| held.client.user_id().map(ToString::to_string)))
}

/// Every line a team hosts, without its history.
///
/// The panel these go in is its own, separate from the local lines —
/// which is what having two sources honestly looks like (decision 16).
#[tauri::command]
pub async fn list_shared_lines(
    state: State<'_, AppState>,
    team_id_raw: String,
) -> Result<Vec<ForgeLineDto>, UiError> {
    let client = teams_client(&state).await?;
    client
        .lines(team_id(&team_id_raw, "team id")?)
        .await
        .map_err(teams_error)
}

/// The teams this window's account belongs to.
///
/// The read a picker is over, and one a caller makes before they have
/// a team to name. Mapped to the contract for the reason the roster's
/// and the ledger's are — the wire is what a member's client and a
/// team server say to each other, and a screen holds one vocabulary.
///
/// What it answers, and what a screen must not conclude from an empty
/// list, is on `MyTeamsDto`.
#[tauri::command]
pub async fn my_teams(state: State<'_, AppState>) -> Result<MyTeamsDto, UiError> {
    let client = teams_client(&state).await?;
    let mine = client.my_teams().await.map_err(teams_error)?;
    Ok(MyTeamsDto {
        teams: mine
            .teams
            .into_iter()
            .map(|team| MyTeamDto {
                team_id: team.team_id,
                role: team.role,
                created_at_ms: team.created_at_ms,
            })
            .collect(),
    })
}

/// Who is in a team, in what role, and what the reader may do there.
///
/// Ids rather than names, because that is what a membership row holds
/// — see [`TeamRosterMemberDto`]. The mapping is here for the reason
/// the ledger's is: the wire is what a member's client and a team
/// server say to each other, and a screen holds the contract's
/// vocabulary alone.
#[tauri::command]
pub async fn team_roster(
    state: State<'_, AppState>,
    team_id_raw: String,
) -> Result<TeamRosterDto, UiError> {
    let client = teams_client(&state).await?;
    let roster = client
        .roster(team_id(&team_id_raw, "team id")?)
        .await
        .map_err(teams_error)?;
    Ok(TeamRosterDto {
        team_id: roster.team_id,
        members: roster
            .members
            .into_iter()
            .map(|member| TeamRosterMemberDto {
                user_id: member.user_id,
                role: member.role,
            })
            .collect(),
        viewer: TeamRosterViewerDto {
            role: roster.viewer.role,
            admin: roster.viewer.admin,
        },
    })
}

/// Founds a team on the connected server, owned by the signed-in
/// account.
///
/// Answers with the id alone. The wire also returns the ledger event
/// the creation appended, and a screen reads that where every other
/// act of the team's is read rather than out of a write's response.
#[tauri::command]
pub async fn create_team(state: State<'_, AppState>) -> Result<TeamCreatedDto, UiError> {
    let client = teams_client(&state).await?;
    // The owner is named rather than left implicit, and that is the
    // one form that works for whoever is signed in. The server refuses
    // an implicit owner from an admin session outright — an admin is
    // never implicitly a member (#83 §1) — and permits any account to
    // name itself. Passing `None` would work for a member and fail for
    // an admin with a message about a field this surface has no way to
    // fill.
    let owner = client.user_id().map(str::to_string);
    let created = client
        .create_team(owner.as_deref())
        .await
        .map_err(teams_error)?;
    Ok(TeamCreatedDto {
        team_id: created.team_id,
    })
}

// The roster writes (#210). None of them answers with anything, and
// that is the same decision `create_team` made for the half of its
// response it drops: what the server hands back is the ledger event
// the write appended, and a screen reads the team's acts in the ledger
// tab rather than out of a write. What a caller wants after one of
// these is the roster, which it asks for.
//
// No contract type is added by any of them, so the boundary these
// commands sit on carries nothing new across.

/// Invites an account into the team, in the role named.
///
/// The invitee must already hold an account on the instance; an id
/// nobody holds is refused as a malformed request rather than
/// provisioning anybody.
#[tauri::command]
pub async fn invite_team_member(
    state: State<'_, AppState>,
    team_id_raw: String,
    user_id: String,
    role: String,
) -> Result<(), UiError> {
    let client = teams_client(&state).await?;
    client
        .invite_member(team_id(&team_id_raw, "team id")?, &user_id, &role)
        .await
        .map_err(teams_error)?;
    Ok(())
}

/// Removes a member from the team.
///
/// Removing the last owner is refused with a `409`, which is a
/// different thing from a malformed request and arrives here as the
/// same `UiError` anyway: [`teams_error`] answers a `404` on its own
/// and turns every other 4xx into a validation error, dropping the
/// token that told a conflict from a bad field. So the screen can say
/// what happened only as far as the server's own sentence carries it.
/// That flattening is #211's, not this command's.
#[tauri::command]
pub async fn remove_team_member(
    state: State<'_, AppState>,
    team_id_raw: String,
    user_id: String,
) -> Result<(), UiError> {
    let client = teams_client(&state).await?;
    client
        .remove_member(team_id(&team_id_raw, "team id")?, &user_id)
        .await
        .map_err(teams_error)?;
    Ok(())
}

/// Makes a member an owner.
#[tauri::command]
pub async fn grant_team_owner(
    state: State<'_, AppState>,
    team_id_raw: String,
    user_id: String,
) -> Result<(), UiError> {
    let client = teams_client(&state).await?;
    client
        .grant_owner(team_id(&team_id_raw, "team id")?, &user_id)
        .await
        .map_err(teams_error)?;
    Ok(())
}

/// Puts an owner back to being a member.
///
/// The last owner cannot be demoted, including by themself, and that
/// refusal reaches a screen the way [`remove_team_member`]'s does.
#[tauri::command]
pub async fn revoke_team_owner(
    state: State<'_, AppState>,
    team_id_raw: String,
    user_id: String,
) -> Result<(), UiError> {
    let client = teams_client(&state).await?;
    client
        .revoke_owner(team_id(&team_id_raw, "team id")?, &user_id)
        .await
        .map_err(teams_error)?;
    Ok(())
}

/// Takes the signed-in account out of the team.
///
/// Reachable by any member, because leaving asks no authority over
/// anybody. A caller holding no membership row has nothing to leave
/// and is refused; the last owner is refused too, and that refusal
/// reaches a screen the way [`remove_team_member`]'s does.
#[tauri::command]
pub async fn leave_team(state: State<'_, AppState>, team_id_raw: String) -> Result<(), UiError> {
    let client = teams_client(&state).await?;
    client
        .leave_team(team_id(&team_id_raw, "team id")?)
        .await
        .map_err(teams_error)?;
    Ok(())
}

/// Deletes the team, which an owner may do and so may an instance
/// admin: standing outside the roster is enough here, which the client
/// verb it calls says where the routes are named.
///
/// It takes the team's ledger with it. The confirmation that says so
/// belongs on the screen offering the act, not here.
#[tauri::command]
pub async fn delete_team(state: State<'_, AppState>, team_id_raw: String) -> Result<(), UiError> {
    let client = teams_client(&state).await?;
    client
        .delete_team(team_id(&team_id_raw, "team id")?)
        .await
        .map_err(teams_error)?;
    Ok(())
}

/// One page of a team's ledger, seq ascending (#148 decision 18).
///
/// `after` is the `next_after` of the page before, or nothing for the
/// first. What comes back can carry a cursor even when it happened to
/// end on the last event there is — [`TeamLedgerPageDto::next_after`]
/// says why, and a caller reading a null cursor as "that was
/// everything" is claiming something this read cannot answer.
///
/// The page size is the caller's: a screen paging by hand and one
/// following the stream want different ones, and the server's own
/// maximum is the only ceiling either has.
///
/// **This is where the two vocabularies meet.** The wire's shapes are
/// what a member's client and a team server say to each other; the
/// contract's are what this app says to its own frontend, and
/// `bindings.ts` is a projection of the contract alone. So the mapping
/// happens here rather than by handing a wire type to a screen, which
/// would give every caller of this command a second vocabulary to
/// know. `asterism-contract::teams` argues the same boundary from its
/// own side, and `tests/boundary.rs` holds it.
#[tauri::command]
pub async fn team_ledger_page(
    state: State<'_, AppState>,
    team_id_raw: String,
    after: Option<i64>,
    limit: Option<u32>,
) -> Result<TeamLedgerPageDto, UiError> {
    let client = teams_client(&state).await?;
    let page = client
        .events(team_id(&team_id_raw, "team id")?, after, limit)
        .await
        .map_err(teams_error)?;
    Ok(TeamLedgerPageDto {
        events: page
            .events
            .into_iter()
            .map(|event| TeamLedgerEventDto {
                seq: event.seq,
                event_id: event.event_id,
                team_id: event.team_id,
                actor_kind: event.actor_kind,
                actor_user_id: event.actor_user_id,
                actor_display_name: event.actor_display_name,
                occurred_at_ms: event.occurred_at_ms,
                kind: event.kind,
                subjects: event
                    .subjects
                    .into_iter()
                    .map(|subject| TeamSubjectRefDto {
                        ref_type: subject.ref_type,
                        value: subject.value,
                    })
                    .collect(),
                payload_json: event.payload_json,
            })
            .collect(),
        next_after: page.next_after,
    })
}

/// The work against a shared line, open and ended alike.
///
/// No mapping, unlike the ledger's and the roster's: the forge is the
/// same on both planes, so a shared line answers with the contract's
/// own `ForgePursuitDto` and there is no second vocabulary to cross.
/// That is #148 decision 19 showing through — the mirror is path for
/// path and shape for shape.
#[tauri::command]
pub async fn shared_line_pursuits(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
) -> Result<Vec<ForgePursuitDto>, UiError> {
    let client = teams_client(&state).await?;
    client
        .pursuits_of_line(
            team_id(&team_id_raw, "team id")?,
            team_id(&line_id, "line id")?,
        )
        .await
        .map_err(teams_error)
}

/// One piece of work on a shared line, as it stands.
#[tauri::command]
pub async fn shared_pursuit(
    state: State<'_, AppState>,
    team_id_raw: String,
    pursuit_id: String,
) -> Result<ForgePursuitDto, UiError> {
    let client = teams_client(&state).await?;
    client
        .pursuit(
            team_id(&team_id_raw, "team id")?,
            team_id(&pursuit_id, "pursuit id")?,
        )
        .await
        .map_err(teams_error)
}

/// Opens work against a shared line.
///
/// Decision 10 is why nothing is copied first: working on a shared
/// line needs no clone, so this is the verb a member reaches for
/// rather than the one that takes a copy home.
#[tauri::command]
pub async fn open_shared_pursuit(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
    title: Option<String>,
    note: Option<String>,
) -> Result<ForgePursuitDto, UiError> {
    let client = teams_client(&state).await?;
    client
        .open_pursuit(
            team_id(&team_id_raw, "team id")?,
            team_id(&line_id, "line id")?,
            title.as_deref(),
            note.as_deref(),
        )
        .await
        .map_err(teams_error)
}

/// Writes a round into open work on a shared line.
///
/// A round is a request rather than a landing: `push` does not read
/// the line, which is what lets two members work against one line
/// without contending, and nothing reaches the line until a close with
/// `satisfied`.
///
/// The projections a round may carry ride beside content entering the
/// team (#148 decision 12), and this surface has no verb that brings
/// any in — so it passes an empty set. The two callers that fill the
/// argument are the ones that do: the promotion, and publishing a line.
#[tauri::command]
pub async fn push_shared_round(
    state: State<'_, AppState>,
    team_id_raw: String,
    pursuit_id: String,
    ops: Vec<ForgeOpDto>,
    note: Option<String>,
) -> Result<ForgePursuitDto, UiError> {
    let client = teams_client(&state).await?;
    client
        .push_round(
            team_id(&team_id_raw, "team id")?,
            team_id(&pursuit_id, "pursuit id")?,
            ops,
            note.as_deref(),
            Vec::new(),
        )
        .await
        .map_err(teams_error)
}

/// Ends work on a shared line, landing it or abandoning it.
///
/// `outcome` is the forge's word — a satisfied close is the only
/// moment anything a round asked for reaches the line.
#[tauri::command]
pub async fn close_shared_pursuit(
    state: State<'_, AppState>,
    team_id_raw: String,
    pursuit_id: String,
    outcome: String,
    note: Option<String>,
) -> Result<ForgePursuitDto, UiError> {
    let client = teams_client(&state).await?;
    client
        .close_pursuit(
            team_id(&team_id_raw, "team id")?,
            team_id(&pursuit_id, "pursuit id")?,
            &outcome,
            note.as_deref(),
        )
        .await
        .map_err(teams_error)
}

/// What is on a shared line, folded from its chain by the server.
#[tauri::command]
pub async fn shared_line_states(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
) -> Result<Vec<ForgeEntryStateDto>, UiError> {
    let client = teams_client(&state).await?;
    client
        .line_states(
            team_id(&team_id_raw, "team id")?,
            team_id(&line_id, "line id")?,
        )
        .await
        .map_err(teams_error)
}

/// A shared line and its whole history.
#[tauri::command]
pub async fn shared_line_history(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
) -> Result<ForgeLineHistoryDto, UiError> {
    let client = teams_client(&state).await?;
    client
        .line_history(
            team_id(&team_id_raw, "team id")?,
            team_id(&line_id, "line id")?,
        )
        .await
        .map_err(teams_error)
}

/// Takes a copy of one entry of a shared line (#148 decision 10).
///
/// An import, and it lands through the same door every other import
/// lands through — which is why the answer is an ordinary `AssetDto`
/// and why the duplicate machinery recognises the second ask.
#[tauri::command]
pub async fn clone_shared_entry(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
    entry_id: String,
    persona_id: String,
) -> Result<AssetDto, UiError> {
    let client = teams_client(&state).await?;
    let persona = parse_persona_id(&persona_id)?;
    let root = clones_dir()?;
    // The channel is chosen here rather than inside the port below,
    // and that is not a detail. A mutation on this surface names
    // `AttributionContext::owner_surface()` in its own body, which is
    // both how the service learns who is asking and how the guard in
    // `tests/mutation_surface.rs` can see that the write surface grew.
    // Passed down rather than constructed there, so the choosing stays
    // where the command is.
    let library = LocalLibrary {
        state: &state,
        persona,
        by: AttributionContext::owner_surface(),
    };
    let cloned = asterism_teams_client::clone::clone_entry(
        &client,
        &library,
        asterism_teams_client::clone::CloneRequest {
            team_id: team_id(&team_id_raw, "team id")?,
            line_id: team_id(&line_id, "line id")?,
            entry_id: team_id(&entry_id, "entry id")?,
            persona_id: &persona,
            root: &root,
        },
        chrono::Utc::now(),
    )
    .await
    .map_err(teams_error)?;
    let held = state
        .assets
        .find(&cloned.asset_id)
        .await?
        .ok_or_else(|| DomainError::not_found("asset", cloned.asset_id))?;
    Ok(asset_to_dto(&held))
}

/// Seeds a team's line from a local one (#148 decision 11).
///
/// `reenact` is the option at init and nowhere else. It replays the
/// chain — one pursuit and one close per change point — and it is a
/// **re-enactment**: the acts are restamped to whoever published, so
/// the team's line does not record who did the work upstream. It also
/// sends every content the line ever named, which is usually far more
/// than what the line holds now. The panel says both of those things
/// before it offers the choice.
#[tauri::command]
pub async fn publish_line_to_team(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
    name: String,
    strategy_id: String,
    reenact: bool,
) -> Result<ForgeLineDto, UiError> {
    let client = teams_client(&state).await?;
    let line = state
        .line_service
        .get(&forge_line_id(&line_id, "line id")?)
        .await?;
    let holdings = LocalHoldings { state: &state };
    let published = asterism_teams_client::publish::publish(
        &client,
        state.asset_links.as_ref(),
        &holdings,
        asterism_teams_client::publish::Publication {
            team_id: team_id(&team_id_raw, "team id")?,
            line: &line,
            named: &name,
            strategy_id: &strategy_id,
            seeding: if reenact {
                asterism_teams_client::publish::Seeding::Reenactment
            } else {
                asterism_teams_client::publish::Seeding::CurrentState
            },
        },
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(teams_error)?;
    client
        .lines(team_id(&team_id_raw, "team id")?)
        .await
        .map_err(teams_error)?
        .into_iter()
        .find(|line| line.id == published.line_id.to_string())
        .ok_or_else(|| {
            UiError::from(DomainError::Infra(anyhow::anyhow!(
                "the line was seeded and the team does not list it"
            )))
        })
}

/// Where copies of what a team holds are kept.
///
/// Beside the other two places the desktop puts bytes it was handed
/// rather than pointed at — `rehome_dropped_path` and
/// `paste_image_import` — and for the same reason: a locator has to go
/// on outliving the gesture that produced it.
fn clones_dir() -> Result<std::path::PathBuf, UiError> {
    let home = std::env::var("HOME")
        .map_err(|e| UiError::from(DomainError::Validation(format!("HOME env not set: {e}"))))?;
    Ok(std::path::PathBuf::from(home).join("asterism/cloned"))
}

/// The local library a clone lands in.
struct LocalLibrary<'a> {
    state: &'a AppState,
    persona: PersonaId,
    /// Chosen by the command, not here — see `clone_shared_entry`.
    by: AttributionContext,
}

#[async_trait::async_trait]
impl asterism_teams_client::clone::Imports for LocalLibrary<'_> {
    async fn held(&self, source_kind: &str, locator: &str) -> Result<Option<AssetId>, DomainError> {
        Ok(self
            .state
            .assets
            .find_by_source(
                &self.persona,
                &SourceKind::new(source_kind)?,
                &SourceLocator::from_wire(locator)?,
                SourceLookupScope::Live,
            )
            .await?
            .map(|asset| asset.id))
    }

    async fn record(
        &self,
        arrival: asterism_teams_client::clone::Arrival<'_>,
    ) -> Result<AssetId, DomainError> {
        let added = self
            .state
            .asset_service
            .add(
                AddAssetCommand {
                    persona_id: self.persona.to_string(),
                    source_kind: arrival.source_kind.to_string(),
                    locator: arrival.locator.to_string(),
                    // Unclassified, as a paste is: "is an image" is a
                    // data format and the material layer answers it
                    // from the file.
                    modality: None,
                    occurred_at_ms: arrival.occurred_at.timestamp_millis(),
                    session_id: None,
                    external_session_key: None,
                    external_key: None,
                    bundle_id: None,
                    labels: vec!["cloned".into()],
                    register_note: None,
                    platform: None,
                    file_size_bytes: Some(arrival.bytes),
                    duration_ms: None,
                    width_px: None,
                    height_px: None,
                    extra_json: None,
                    // What the promoter called it, which is the one
                    // thing an ingest has a slot for saying.
                    cover_hint: arrival.cover_hint.map(ToString::to_string),
                    auto_organize_base_dir: None,
                    // A copy declares no origin on this axis: where it
                    // came from is `source_kind` and the locator, and
                    // `derived_from` names a local asset this was made
                    // out of, which nothing here is.
                    derived_from: None,
                    // The three assertion fields stay empty for the
                    // reason `paste_image_import` states: this arrived
                    // through the owner's own surface, which the
                    // context below says and an assertion may not.
                    author_kind: None,
                    author_subject: None,
                    operator_ai: None,
                    on_duplicate: None,
                    // A logical locator would be refused with one, and
                    // this one is a path — but the digest the team
                    // verified is about the bytes as the team holds
                    // them, and what lands here is what arrived. The
                    // hash job reads the file and answers for itself.
                    declared_content_hash: None,
                    album_meta: Default::default(),
                },
                &self.by,
            )
            .await?;
        parse_asset_id(&added.id)
    }
}

/// Hands one of this library's assets to a team, against open work.
///
/// The act #66 exists for, and the one entry point content has (#148
/// decision 5): the pursuit is named because a team never holds an
/// asset that is not attached to work.
///
/// **What travels is decision 4's and is decided below this command**,
/// in `PromotedMark::gather`, which is the only thing that can say
/// which marks qualify: the filter is over the layer a mark sits in,
/// and `MaterialMarkDto` drops the `layer_id` that join is made of.
/// So the marks are read through their repository rather than their
/// service, which is the same arrangement `publish` makes one screen
/// over and for the same reason.
///
/// **Not safe to press twice, and the answer says so rather than the
/// command refusing.** Decision 7 mints a `TeamAsset` per promotion.
/// What stops a second one is a relation row this machine wrote, which
/// the client reads before anything is sent — so a repeat is answered
/// from here with `already_promoted` and nothing crosses the wire.
/// That is a fact about this machine and not about the team: another
/// member promoting the same bytes gets their own `TeamAsset`, and
/// nothing here can see it.
///
/// `named` is the caller's rather than the asset's title, and
/// `Promotion::named` says why.
#[tauri::command]
pub async fn promote_asset_to_team(
    state: State<'_, AppState>,
    team_id_raw: String,
    line_id: String,
    pursuit_id: String,
    asset_id: String,
    named: String,
) -> Result<PromotedAssetDto, UiError> {
    let client = teams_client(&state).await?;
    let asset_id = parse_asset_id(&asset_id)?;
    let asset = state
        .assets
        .find(&asset_id)
        .await?
        .ok_or_else(|| DomainError::not_found("asset", asset_id))?;
    let layers = state
        .material_layer_service
        .list_by_asset(&asset_id)
        .await?;
    let marks = state.material_marks.list_by_asset(&asset_id).await?;
    let user_marks = asterism_teams_client::PromotedMark::gather(&layers, &marks);

    let outcome = asterism_teams_client::promotion::promote(
        &client,
        state.asset_links.as_ref(),
        asterism_teams_client::promotion::Promotion {
            team_id: team_id(&team_id_raw, "team id")?,
            line_id: team_id(&line_id, "line id")?,
            pursuit_id: team_id(&pursuit_id, "pursuit id")?,
            subject: asterism_teams_client::mapper::LocalSubject {
                asset: &asset,
                user_marks: &user_marks,
            },
            named: &named,
        },
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(teams_error)?;

    Ok(PromotedAssetDto {
        entry_id: outcome.key.entry_id.to_string(),
        team_asset_id: outcome.team_asset_id.map(|id| id.to_string()),
        digest: outcome.digest,
        bytes_already_held: outcome.bytes_already_held,
        already_promoted: outcome.already_promoted,
    })
}

/// What a line's content is, on this machine.
struct LocalHoldings<'a> {
    state: &'a AppState,
}

#[async_trait::async_trait]
impl asterism_teams_client::publish::Holdings for LocalHoldings<'_> {
    async fn subject(
        &self,
        content: AssetId,
    ) -> Result<asterism_teams_client::publish::HeldSubject, DomainError> {
        let asset = self
            .state
            .assets
            .find(&content)
            .await?
            .ok_or_else(|| DomainError::not_found("asset", content))?;
        // The origin filter is the domain's, over the domain's values:
        // a mark travels when the band it sits in was written by a
        // person (#148 decision 4), and `gather` is the only thing that
        // can say so. The bands come through their service, which hands
        // back the domain value; the marks cannot, because the DTO
        // drops the `layer_id` the join is made of.
        let layers = self
            .state
            .material_layer_service
            .list_by_asset(&content)
            .await?;
        let marks = self.state.material_marks.list_by_asset(&content).await?;
        Ok(asterism_teams_client::publish::HeldSubject {
            user_marks: asterism_teams_client::PromotedMark::gather(&layers, &marks),
            asset,
        })
    }
}
