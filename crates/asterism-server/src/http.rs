//! HTTP transport — axum router.
//!
//! Route conventions: RPC-style endpoints that mirror the Tauri command
//! surface (for example `POST /asterism/assets/add`). Contract DTOs are
//! reused verbatim so the same shape flows through the HTTP body, the
//! Tauri IPC bridge, and the MCP tool schemas (`crate::mcp`, nested on
//! this router at `/mcp`).
//!
//! The server is bound to loopback in v1 and does not authenticate
//! requests.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AddAssetToGroupCommand,
    AppendMessageCommand, ArchivePersonaCommand, ArchiveThreadCommand, AttachTagBatchCommand,
    AttachTagBatchResult, AttachTagCommand, BatchGroupMembershipCommand, CreateDirCommand,
    CreateDispatchCommand, CreateGroupCommand, CreateMaterialLayerCommand, CreateModalityCommand,
    CreateQueryGroupCommand, CreateSeriesStrategyCommand, CreateSnapshotCommand,
    CreateThreadCommand, DeclareAssetMetaCommand, DeclareProvenanceCommand,
    DeclareSourceTypeCommand, DeleteAssetCommentCommand, DeleteChapterMarkCommand,
    DeleteDirCommand, DeleteMaterialLayerCommand, DeleteMaterialMarkCommand, DeleteMessageCommand,
    DeleteModalityCommand, DeletePersonaProfileCommand, DeletePersonaThemeCommand,
    DeleteSeriesStrategyCommand, DeleteSessionCommand, DeleteTagCommand, DeleteTagResult,
    DeleteThreadCommand, DetachTagBatchCommand, DetachTagBatchResult, DetachTagCommand,
    DispatchRunCommand, EditAssetCommentCommand, EditChapterMarkCommand, EditMaterialMarkCommand,
    EmptyTrashCommand, EmptyTrashResult, LinkGroupCommand, MergeAssetsCommand, MergeGroupsCommand,
    MergeTagsCommand, MergeTagsResult, MoveDirCommand, MoveGroupToDirCommand,
    OrganizeByLocationCommand, OrganizeByLocationResult, PatchSessionMetadataCommand,
    PostAssetCommentCommand, PostChapterMarkCommand, PostMaterialMarkCommand,
    PromoteSnapshotToGroupCommand, PromoteSnapshotToGroupResult, PromoteTagToGroupCommand,
    PromoteTagToGroupResult, PromoteVolatileSelectionCommand, PurgeAssetCommand, PurgeGroupCommand,
    PurgePersonaCommand, RecordDiagCommand, RecordEventCommand, RedispatchCommand,
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
    AssetCardDto, AssetCommentDto, AssetCountEntryDto, AssetDetailDto, AssetDto, AssetIndexPageDto,
    AssetPageDto, AssetTextDto, ChapterMarkDto, ConstellationItemDto, DiagDto, DirDto, DispatchDto,
    DuplicateConflictDto, DuplicateReportDto, DuplicateResolutionDto, EdgeDto, EventDto, GroupDto,
    GroupLinkDto, GroupSummaryDto, JobLogDto, LineageViewDto, MaterialLayerDto,
    MaterialLayerViewDto, MaterialMarkDto, MergeAssetsDto, MessageDto, ModalityDefDto,
    ObservationDto, PerfDto, PersonaDto, PersonaProfileDto, PersonaThemeDto, ProvenanceViewDto,
    RetrievedIdsDto, RetrievedPageDto, SampledPageDto, SeriesStrategyDto, SessionDto,
    SessionPageDto, SettingDto, SnapshotDto, TagCountDto, TagDto, TagSuggestionDto, ThreadDto,
    VideoPreviewDto, VisualModelStatusDto,
};
use asterism_contract::forge::{
    ForgeDiscardedDto, ForgeEntryStateDto, ForgeLineActCommand, ForgeLineDto, ForgeLineHistoryDto,
    ForgeStrategyDto, OpenForgeLineCommand, RenameForgeLineCommand, SetForgeLineStrategyCommand,
};
use asterism_contract::query::{
    DiagLevel, GetAssetDetailQuery, ListAssetsQuery, ListDiagQuery, ListEventsQuery,
    ListJobLogQuery, ListObservationsQuery, ListPerfQuery, RandomAssetsQuery, SearchAssetsQuery,
};
use asterism_core::DomainError;
use asterism_core::application::mapping::{
    forge_discarded_to_dto, forge_history_to_dto, forge_line_id, forge_line_to_dto, forge_name,
    forge_states_to_dto, forge_strategy_id, forge_strategy_to_dto,
};
use asterism_core::domain::forge::model::value::LineId;
use asterism_core::domain::observation::Stream;
use asterism_core::domain::value::MimeType;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_util::io::ReaderStream;

use crate::attribution::asserted;
use crate::state::ServerCtx;

/// HTTP-boundary error type. Same tagged shape as the Tauri `UiError`,
/// with an added HTTP status code.
struct ApiError(DomainError);

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, kind) = match &self.0 {
            DomainError::PersonaNotFound(_)
            | DomainError::AssetNotFound(_)
            | DomainError::NotFound { .. } => (StatusCode::NOT_FOUND, "NotFound"),
            DomainError::Validation(_) => (StatusCode::BAD_REQUEST, "Validation"),
            DomainError::DuplicatePersona(_) | DomainError::Conflict(_) => {
                (StatusCode::CONFLICT, "Conflict")
            }
            DomainError::Infra(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal"),
        };
        (
            status,
            Json(serde_json::json!({ "kind": kind, "message": self.0.to_string() })),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// Builds the router; the caller binds a listener and calls
/// `axum::serve`.
pub fn router(ctx: Arc<ServerCtx>) -> Router {
    // Pin the process start time now, not on the first `/asterism/health`
    // call — the value's job is to prove "a different process answered"
    // across a restart, so it must date the serve, not the probe.
    let _ = *STARTED_AT_MS;
    Router::new()
        // MCP transport, same services, same lifetime: nesting it here
        // means it exists wherever the HTTP API does (Tauri-embedded
        // serve and the standalone binary alike), with no second core.
        .nest_service("/mcp", crate::mcp::streamable_service(ctx.clone()))
        .route("/asterism/health", get(health))
        .route("/asterism/admin/shutdown", post(shutdown_process))
        .route("/asterism/personas", get(list_personas))
        .route("/asterism/personas/counts", get(list_persona_asset_counts))
        .route(
            "/asterism/modalities/counts",
            get(list_modality_asset_counts),
        )
        .route("/asterism/formats/counts", get(list_format_asset_counts))
        .route("/asterism/colors/counts", get(list_color_asset_counts))
        .route("/asterism/duplicates", get(list_duplicate_groups))
        .route(
            "/asterism/duplicates/conflicts",
            get(list_duplicate_conflicts),
        )
        .route(
            "/asterism/duplicates/conflicts/resolve",
            post(resolve_duplicate_conflict),
        )
        .route("/asterism/duplicates/merge", post(merge_assets))
        .route("/asterism/duplicates/rescan", post(rescan_duplicates))
        .route(
            "/asterism/modalities",
            get(list_modalities).post(create_modality),
        )
        .route(
            "/asterism/modalities/{slug}",
            patch(update_modality).delete(delete_modality),
        )
        // The series axis's rules. Registration is the whole point of
        // these four: an importer runs in its own process, so a rule it
        // wants applied crosses as data or not at all — the argument is
        // in `SeriesStrategyService`'s module doc. The MCP resource
        // `asterism://schemas/series-strategy` is where the agent
        // writing one reads how.
        .route(
            "/asterism/series-strategies",
            get(list_series_strategies).post(create_series_strategy),
        )
        .route(
            "/asterism/series-strategies/{id}",
            patch(update_series_strategy).delete(delete_series_strategy),
        )
        .route("/asterism/settings", get(list_settings))
        .route(
            "/asterism/settings/{key}",
            get(get_setting).put(set_setting).delete(reset_setting),
        )
        .route("/asterism/personas/register", post(register_persona))
        .route("/asterism/personas/reorder", post(reorder_personas))
        .route("/asterism/personas/archive", post(archive_persona))
        .route("/asterism/personas/trash", post(trash_persona))
        .route("/asterism/personas/restore", post(restore_persona))
        .route("/asterism/personas/purge", post(purge_persona))
        .route("/asterism/personas/{id}/theme", get(get_persona_theme))
        .route("/asterism/personas/theme/set", post(set_persona_theme))
        .route(
            "/asterism/personas/theme/delete",
            post(delete_persona_theme),
        )
        .route("/asterism/personas/{id}/profile", get(get_persona_profile))
        .route("/asterism/personas/profile/set", post(set_persona_profile))
        .route(
            "/asterism/personas/profile/delete",
            post(delete_persona_profile),
        )
        .route("/asterism/organize/by-location", post(organize_by_location))
        .route("/asterism/jobs/stats", get(jobs_stats))
        .route("/asterism/jobs/depth", get(jobs_depth))
        .route("/asterism/events", get(list_events).post(record_event))
        .route("/asterism/jobs/log", get(list_job_log))
        .route("/asterism/diag", get(list_diag).post(record_diag))
        .route("/asterism/diag/levels", get(list_diag_levels))
        .route("/asterism/perf", get(list_perf))
        .route("/asterism/observations", get(list_observations))
        .route("/asterism/observations/streams", get(list_streams))
        .route("/asterism/assets", get(list_assets))
        // The grid's own read pair: the full ordering as lightweight
        // index rows, then the ~40 cards the viewport is about to
        // paint. `list_assets` returns cards for the whole page, which
        // is the wrong shape for a caller reproducing what the grid
        // does — it pays card serialisation for rows it will not read.
        .route("/asterism/assets/index", get(list_asset_index))
        .route("/asterism/assets/hydrate", post(hydrate_cards))
        .route("/asterism/assets/add", post(add_asset))
        .route("/asterism/assets/add-batch", post(add_asset_batch))
        .route("/asterism/assets/update-meta", post(update_asset_meta))
        .route("/asterism/assets/remeasure", post(remeasure_dims))
        .route(
            "/asterism/assets/update-meta-batch",
            post(update_asset_meta_batch),
        )
        .route("/asterism/assets/trash", post(trash_asset))
        .route("/asterism/assets/restore", post(restore_asset))
        .route("/asterism/assets/purge", post(purge_asset))
        .route("/asterism/assets/empty-trash", post(empty_trash))
        .route("/asterism/assets/search", post(search_assets))
        .route("/asterism/assets/search-ids", post(search_asset_ids))
        .route("/asterism/assets/random", post(random_assets))
        .route("/asterism/assets/{id}", get(asset_detail))
        .route("/asterism/assets/{id}/edges", get(asset_edges))
        .route(
            "/asterism/assets/{id}/constellation",
            get(asset_constellation),
        )
        .route(
            "/asterism/assets/{id}/provenance",
            get(asset_provenance).post(declare_asset_provenance),
        )
        // `album-meta`, not `meta`: `update-meta` above already means
        // the asset's own columns (labels / cover / register note), and
        // two routes a reader has to tell apart by which noun follows
        // is how a caller ends up writing a person's statement into a
        // field the application acts on.
        .route(
            "/asterism/assets/{id}/album-meta",
            post(declare_asset_album_meta),
        )
        .route(
            "/asterism/assets/{id}/source-type",
            post(declare_asset_source_type),
        )
        .route("/asterism/assets/{id}/lineage", get(asset_lineage))
        .route("/asterism/assets/{id}/groups", get(groups_of_asset))
        .route(
            "/asterism/assets/{id}/snapshots",
            get(list_snapshots_containing),
        )
        // Comment thread on an Asset. Same rows as the UI's four Tauri
        // commands — author identity is a field on the command body, so
        // an agent posts as itself rather than impersonating the user.
        .route(
            "/asterism/assets/{id}/comments",
            get(list_asset_comments).post(post_asset_comment),
        )
        .route("/asterism/comments/edit", post(edit_asset_comment))
        .route("/asterism/comments/delete", post(delete_asset_comment))
        // Model-proposed tag suggestions (#112): a listing a person
        // rules on. Accept/reject are verbs on the pair, path-scoped
        // like the comment routes so the URL is authoritative.
        .route(
            "/asterism/assets/{id}/tag-suggestions",
            get(list_tag_suggestions),
        )
        .route(
            "/asterism/assets/{id}/tag-suggestions/{tag_id}/accept",
            post(accept_tag_suggestion),
        )
        .route(
            "/asterism/assets/{id}/tag-suggestions/{tag_id}/reject",
            post(reject_tag_suggestion),
        )
        // Which visual model this process bound, if any (#112).
        .route("/asterism/models/status", get(visual_model_status))
        // Install the package a registry entry describes (#126).
        .route("/asterism/models/fetch", post(fetch_visual_model))
        // Marks inside an Asset's material — the same four verbs on a
        // narrower anchor: a position in the content rather than a note
        // on the asset row.
        .route(
            "/asterism/assets/{id}/material-marks",
            get(list_material_marks).post(post_material_mark),
        )
        .route("/asterism/material-marks/edit", post(edit_material_mark))
        .route(
            "/asterism/material-marks/delete",
            post(delete_material_mark),
        )
        // The bands those marks sit in, and the chapters inside a
        // structure band. The asset-level GET carries each band's
        // chapters with it; the per-band GET is what a surface re-reads
        // after editing one, and neither is derivable from the other
        // without a round trip the caller did not ask for.
        .route(
            "/asterism/assets/{id}/material-layers",
            get(list_material_layers).post(create_material_layer),
        )
        .route(
            "/asterism/material-layers/set-default",
            post(set_default_material_layer),
        )
        .route(
            "/asterism/material-layers/delete",
            post(delete_material_layer),
        )
        .route(
            "/asterism/material-layers/{id}/chapter-marks",
            get(list_chapter_marks).post(post_chapter_mark),
        )
        .route("/asterism/chapter-marks/edit", post(edit_chapter_mark))
        .route("/asterism/chapter-marks/delete", post(delete_chapter_mark))
        .route("/asterism/assets/{id}/rebuild-edges", post(rebuild_edges))
        .route(
            "/asterism/assets/{id}/video-preview",
            get(asset_video_preview),
        )
        .route(
            "/asterism/assets/{id}/thumbs/{size_px}",
            get(get_thumb).put(put_thumb),
        )
        // The original bytes. Without this the only binary an off-machine
        // caller could obtain was a thumbnail — `locator` is a path on
        // *this* disk, so a remote agent held a name it could not open.
        .route("/asterism/assets/{id}/file", get(get_asset_file))
        .route("/asterism/assets/texts", post(asset_texts))
        .route("/asterism/tags/counts", get(list_tag_counts))
        .route("/asterism/tags/attach", post(attach_tag))
        .route("/asterism/tags/detach", post(detach_tag))
        .route("/asterism/tags/attach-batch", post(attach_tag_batch))
        .route("/asterism/tags/detach-batch", post(detach_tag_batch))
        // Channel administration, as opposed to per-asset attachment.
        // Without these three an automatic tagger's synonyms and
        // spelling variants accumulate with no way back: the surface
        // could create channels but never repair them.
        .route("/asterism/tags/rename", post(rename_tag))
        .route("/asterism/tags/delete", post(delete_tag))
        .route("/asterism/tags/merge", post(merge_tags))
        .route(
            "/asterism/tags/promote-to-group",
            post(promote_tag_to_group),
        )
        .route("/asterism/sessions", get(list_sessions))
        .route("/asterism/sessions/rebuild", post(rebuild_sessions))
        .route(
            "/asterism/sessions/{id}",
            get(get_session)
                .patch(patch_session_metadata)
                .delete(delete_session),
        )
        .route("/asterism/sessions/{id}/rename", post(rename_session))
        .route("/asterism/index/rebuild", post(rebuild_index))
        .route("/asterism/groups", get(list_groups))
        .route("/asterism/groups/create", post(create_group))
        .route("/asterism/groups/trash", post(trash_group))
        .route("/asterism/groups/restore", post(restore_group))
        .route("/asterism/groups/purge", post(purge_group))
        .route("/asterism/groups/add-asset", post(add_asset_to_group))
        .route(
            "/asterism/groups/remove-asset",
            post(remove_asset_from_group),
        )
        .route(
            "/asterism/groups/batch-membership",
            post(batch_group_membership),
        )
        .route("/asterism/groups/merge", post(merge_groups))
        .route("/asterism/groups/reorder", post(reorder_group_assets))
        .route("/asterism/groups/rename", post(rename_group))
        .route("/asterism/groups/move-to-dir", post(move_group_to_dir))
        .route("/asterism/groups/link", post(link_group))
        .route("/asterism/groups/unlink", post(unlink_group))
        .route("/asterism/groups/links", get(list_group_links))
        .route(
            "/asterism/groups/reorder-children",
            post(reorder_group_children),
        )
        .route("/asterism/dirs", get(list_dirs))
        .route("/asterism/dirs/create", post(create_dir))
        .route("/asterism/dirs/rename", post(rename_dir))
        .route("/asterism/dirs/move", post(move_dir))
        .route("/asterism/dirs/delete", post(delete_dir))
        .route("/asterism/query-groups/create", post(create_query_group))
        .route(
            "/asterism/query-groups/update-query",
            post(update_query_group_query),
        )
        .route("/asterism/dispatch", get(list_dispatch))
        .route("/asterism/dispatch/create", post(create_dispatch))
        .route("/asterism/dispatch/run", post(dispatch_run))
        .route("/asterism/dispatch/redispatch", post(redispatch))
        .route("/asterism/snapshots/create", post(create_snapshot))
        .route("/asterism/snapshots/{id}", get(get_snapshot))
        .route("/asterism/snapshots/{id}/members", get(snapshot_members))
        .route(
            "/asterism/snapshots/promote-volatile",
            post(promote_volatile_selection),
        )
        .route(
            "/asterism/snapshots/promote-to-group",
            post(promote_snapshot_to_group),
        )
        .route("/asterism/dispatch/{id}", get(get_dispatch))
        .route("/asterism/exporters", get(list_exporters))
        // App-level Threads primitive.
        // UI and Claude Code / agents both hit the same rows via
        // POST /asterism/threads/{id}/append. Author identity is a
        // header-supplied field on the command body — the server does
        // not infer it.
        .route(
            "/asterism/threads",
            get(list_threads_by_anchor).post(create_thread),
        )
        .route("/asterism/threads/archive", post(archive_thread))
        .route("/asterism/threads/delete", post(delete_thread))
        .route("/asterism/threads/{id}", get(get_thread))
        .route(
            "/asterism/threads/{id}/messages",
            get(list_thread_messages).post(append_thread_message),
        )
        .route("/asterism/messages/delete", post(delete_message))
        // The forge, under a prefix of its own. `/asterism/threads`
        // above is the annotation surface on the raw layer, which
        // anchors to snapshots and cards; the forge's conversations
        // anchor to work. Neither could carry the other's anchors, and
        // the same collision is why `CoreCtx` has `thread_service` and
        // `forge_thread_service` side by side. Prefixing all of the
        // forge keeps the pair impossible rather than resolved noun by
        // noun.
        //
        // The verbs are acts and are spelled as acts, which is the
        // form `/asterism/personas/archive` and
        // `/asterism/assets/{id}/source-type` already use here.
        .route(
            "/asterism/forge/lines",
            get(list_forge_lines).post(open_forge_line),
        )
        .route("/asterism/forge/lines/{id}", get(get_forge_line))
        .route(
            "/asterism/forge/lines/{id}/states",
            get(get_forge_line_states),
        )
        .route("/asterism/forge/lines/{id}/rename", post(rename_forge_line))
        .route(
            "/asterism/forge/lines/{id}/strategy",
            post(set_forge_line_strategy),
        )
        .route(
            "/asterism/forge/lines/{id}/archive",
            post(archive_forge_line),
        )
        .route("/asterism/forge/lines/{id}/reopen", post(reopen_forge_line))
        .route(
            "/asterism/forge/lines/{id}/discard",
            post(discard_forge_line),
        )
        .route("/asterism/forge/strategies", get(list_forge_strategies))
        .with_state(ctx)
}

/// Millisecond timestamp of process start (dereferenced once in
/// [`router`], so it dates the serve rather than the first probe).
static STARTED_AT_MS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
});

/// Health doubles as the build-identity probe: `git_sha` says which
/// commit the serving binary was built from, `pid` / `started_at_ms`
/// say which process is answering. Together they make "the restart
/// silently left the old build serving" detectable by any client
/// (the MCP proxy's `app_restart`, the `dogfood-restart` recipe, or a
/// bare curl) instead of being an article of faith.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "asterism-server",
        "version": env!("CARGO_PKG_VERSION"),
        "git_sha": env!("ASTERISM_GIT_SHA"),
        "pid": std::process::id(),
        "started_at_ms": *STARTED_AT_MS,
    }))
}

/// Terminates the serving process (loopback-only, like every route
/// here). This is the lifecycle counterpart to launching the app from
/// outside: the process must actually die for a relaunch to pick up a
/// new binary, so no graceful-drain dance — answer, then exit. The
/// Tauri-embedded serve exits the whole app by design; the caller
/// (MCP proxy `app_restart` / restart recipe) owns the relaunch.
async fn shutdown_process() -> Json<serde_json::Value> {
    tokio::spawn(async {
        // Long enough for this response to flush, nothing more.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::process::exit(0);
    });
    Json(serde_json::json!({
        "status": "shutting_down",
        "pid": std::process::id(),
        "git_sha": env!("ASTERISM_GIT_SHA"),
    }))
}

async fn list_personas(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<Vec<PersonaDto>> {
    Ok(Json(ctx.persona_service.list().await?))
}

async fn register_persona(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RegisterPersonaCommand>,
) -> ApiResult<PersonaDto> {
    Ok(Json(
        ctx.persona_service
            .register(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/personas/reorder` — rewrites `display_order` across a
/// persona slice. This is the sidebar's hand arrangement, which
/// `Sort: persona` + `Order: As arranged` reads back.
async fn reorder_personas(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<ReorderPersonasCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.persona_service
        .reorder(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "reordered": true })))
}

async fn archive_persona(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<ArchivePersonaCommand>,
) -> ApiResult<PersonaDto> {
    Ok(Json(
        ctx.persona_service
            .set_archived(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/personas/trash` — takes the persona's assets with it,
/// reversibly.
async fn trash_persona(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<TrashPersonaCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.persona_service
        .trash(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "trashed": true })))
}

/// `POST /asterism/personas/restore`.
async fn restore_persona(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RestorePersonaCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.persona_service
        .restore(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "restored": true })))
}

/// `POST /asterism/personas/purge` — the widest destructive verb in the
/// system. Irreversible, and 409 unless the persona is already trashed.
async fn purge_persona(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<PurgePersonaCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.persona_service
        .purge(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "purged": true })))
}

/// `GET /asterism/personas/{id}/theme` — returns the persona's UI
/// chrome (wallpaper reference). A `null` body means the persona
/// has no custom theme; the UI falls back to built-in defaults.
async fn get_persona_theme(
    State(ctx): State<Arc<ServerCtx>>,
    Path(persona_id): Path<String>,
) -> ApiResult<Option<PersonaThemeDto>> {
    Ok(Json(ctx.persona_service.get_theme(&persona_id).await?))
}

/// `POST /asterism/personas/theme/set` — upserts the theme row.
async fn set_persona_theme(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<SetPersonaThemeCommand>,
) -> ApiResult<PersonaThemeDto> {
    Ok(Json(
        ctx.persona_service
            .set_theme(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/personas/theme/delete` — drops the theme row so
/// the UI reverts to built-in defaults.
async fn delete_persona_theme(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeletePersonaThemeCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.persona_service
        .delete_theme(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /asterism/personas/{id}/profile` — returns the persona's
/// identity signal (avatar / bio / role). `null` means no profile
/// row yet.
async fn get_persona_profile(
    State(ctx): State<Arc<ServerCtx>>,
    Path(persona_id): Path<String>,
) -> ApiResult<Option<PersonaProfileDto>> {
    Ok(Json(ctx.persona_service.get_profile(&persona_id).await?))
}

/// `POST /asterism/personas/profile/set`.
async fn set_persona_profile(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<SetPersonaProfileCommand>,
) -> ApiResult<PersonaProfileDto> {
    Ok(Json(
        ctx.persona_service
            .set_profile(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/personas/profile/delete`.
async fn delete_persona_profile(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeletePersonaProfileCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.persona_service
        .delete_profile(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// Backfill endpoint: auto-organises existing assets under a Dir
/// tree derived from `source_locator`. See
/// [`OrganizeByLocationCommand`] for the shape.
async fn organize_by_location(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<OrganizeByLocationCommand>,
) -> ApiResult<OrganizeByLocationResult> {
    Ok(Json(
        ctx.asset_service
            .organize_by_location(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// Compact snapshot of the apalis `Jobs` table used by the UI
/// progress banner. Returns totals + per-kind counts so the ticker
/// can render "done / total" gauges alongside the live per-kind
/// chips.
async fn jobs_stats(
    State(ctx): State<Arc<ServerCtx>>,
) -> ApiResult<asterism_infra::jobs::JobsSnapshot> {
    Ok(Json(
        asterism_infra::jobs::jobs_snapshot(&ctx.jobs_pool)
            .await
            .map_err(ApiError)?,
    ))
}

/// Queue depth by status — `{ pending, running, done, failed }`.
///
/// The same table [`jobs_stats`] reads, without the per-kind
/// breakdown and therefore without its `json_extract` pass over every
/// row. It exists to be *polled*: a bench driver watching a 5,000-file
/// import to completion asks this every five seconds for as long as an
/// hour, which the snapshot's kind roll-up cannot afford (1.15-1.27 s
/// per call at 368 k rows [measured 2026-07-21]). "Drained" is
/// `pending + running == 0`.
async fn jobs_depth(
    State(ctx): State<Arc<ServerCtx>>,
) -> ApiResult<asterism_infra::jobs::JobsDepth> {
    Ok(Json(
        asterism_infra::jobs::jobs_depth(&ctx.jobs_pool)
            .await
            .map_err(ApiError)?,
    ))
}

/// `POST /asterism/events` — append one telemetry event (local
/// `event_log`; `occurred_at` is stamped server-side).
async fn record_event(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RecordEventCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.telemetry.record(command).await?;
    Ok(Json(serde_json::json!({ "recorded": true })))
}

/// `GET /asterism/events` — newest-first telemetry listing. Query
/// string carries the optional `kind` / `since_ms` / `until_ms` /
/// `limit` filters; agents aggregate usage summaries from this feed.
async fn list_events(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListEventsQuery>,
) -> ApiResult<Vec<EventDto>> {
    Ok(Json(ctx.telemetry.list(query).await?))
}

/// Re-emits one webview-origin diagnostic as a `tracing` event, which
/// the installed subscriber persists to `diag_log` like any native
/// record. Shared by the HTTP route below and the Tauri `record_diag`
/// command, so both adapters land byte-identical rows.
///
/// The target is the literal `asterism_webview`: it must carry the
/// `asterism` prefix or the subscriber's db filter (`asterism=info`,
/// `observe::install`) drops it, and it is distinct from every real
/// module path so `GET /asterism/diag?target=asterism_webview` is the
/// exact query for "what did the UI see".
///
/// Validation is deliberately strict on `event`: the `webview.`
/// namespace keeps a client payload out of the `perf` / `job` stream
/// routing (`Stream::of_event`), and an unknown `level` is an error
/// rather than a guessed severity.
pub fn record_webview_diag(command: &RecordDiagCommand) -> Result<(), DomainError> {
    let level = DiagLevel::parse(&command.level).map_err(DomainError::Validation)?;
    if !command.event.starts_with("webview.") {
        return Err(DomainError::Validation(format!(
            "RecordDiagCommand.event must start with 'webview.', got {:?}",
            command.event
        )));
    }
    if command.message.trim().is_empty() {
        return Err(DomainError::Validation(
            "RecordDiagCommand.message must not be empty".into(),
        ));
    }
    let event = command.event.as_str();
    let attrs = command.attrs_json.as_deref().unwrap_or("{}");
    let message = command.message.as_str();
    match level {
        DiagLevel::Error => {
            tracing::error!(target: "asterism_webview", event, attrs, "{message}")
        }
        DiagLevel::Warn => tracing::warn!(target: "asterism_webview", event, attrs, "{message}"),
        DiagLevel::Info => tracing::info!(target: "asterism_webview", event, attrs, "{message}"),
        // Below the diagnostics persistence floor — accepted (the
        // level set is honest) but only reaches stderr, not the table.
        DiagLevel::Debug => tracing::debug!(target: "asterism_webview", event, attrs, "{message}"),
        DiagLevel::Trace => tracing::trace!(target: "asterism_webview", event, attrs, "{message}"),
    }
    Ok(())
}

/// `POST /asterism/diag` — append one webview-origin diagnostic.
///
/// The write twin of the `GET`: the webview cannot `tracing::error!`,
/// so its captured console/error moments arrive as a command and are
/// re-emitted into the same subscriber → `diag_log` pipe
/// ([`record_webview_diag`]).
async fn record_diag(
    State(_ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RecordDiagCommand>,
) -> ApiResult<serde_json::Value> {
    record_webview_diag(&command)?;
    Ok(Json(serde_json::json!({ "recorded": true })))
}

/// `GET /asterism/diag` — the application's own diagnostics, newest
/// first.
///
/// Everything `tracing` persisted to `diag_log`: swallowed warnings,
/// startup decisions, and the `list_index` perf breakdown. This is the
/// answer to "it did something odd an hour ago and the terminal is
/// gone".
///
/// Filters: `min_level` (inclusive — `warn` also returns errors; an
/// unknown value is a `400` naming the accepted set, which
/// `/asterism/diag/levels` also publishes), `target` (a `LIKE` pattern
/// matched against the module path, so `%` and `_` are wildcards),
/// `since_ms` / `until_ms`, `limit`. Example:
/// `/asterism/diag?min_level=warn&target=app_setting&limit=20`.
///
/// Every filter is applied in the query, so the page is exact: fewer
/// than `limit` rows means the requested window held no more matches.
///
/// The *read* side is HTTP-only on purpose — no Tauri command and no
/// TypeScript binding. Diagnostics are for whoever is investigating
/// the application, not for the person using it. (The *write* side has
/// a Tauri command — `record_diag` — because the webview is itself a
/// diagnostic source; it still never reads the stream back.)
async fn list_diag(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListDiagQuery>,
) -> ApiResult<Vec<DiagDto>> {
    Ok(Json(ctx.observations.diag(query).await?))
}

/// `GET /asterism/perf` — timings, newest first.
///
/// Written in development only, so an empty listing under the dogfood
/// profile is the policy working rather than an absence of activity.
async fn list_perf(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListPerfQuery>,
) -> ApiResult<Vec<PerfDto>> {
    Ok(Json(ctx.observations.perf(query).await?))
}

/// `GET /asterism/jobs/log` — job runs, newest first.
///
/// The history the queue's own table does not keep: `/asterism/jobs/stats`
/// answers "what is queued right now", this answers "what happened".
async fn list_job_log(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListJobLogQuery>,
) -> ApiResult<Vec<JobLogDto>> {
    Ok(Json(ctx.observations.job_log(query).await?))
}

/// `GET /asterism/observations` — every stream on one timeline.
///
/// Carries the shared envelope only. A stream's own columns are the
/// reason it is a separate table, so asking for them means asking that
/// stream's own endpoint.
async fn list_observations(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListObservationsQuery>,
) -> ApiResult<Vec<ObservationDto>> {
    Ok(Json(ctx.observations.all(query).await?))
}

/// `GET /asterism/observations/streams` — the stream names `stream`
/// accepts.
///
/// Published for the same reason as `/asterism/diag/levels`: the set is
/// closed, and a caller guessing at it is how a filter ends up silently
/// doing nothing.
async fn list_streams() -> ApiResult<Vec<&'static str>> {
    Ok(Json(Stream::ALL.iter().map(|s| s.as_str()).collect()))
}

/// `GET /asterism/diag/levels` — the severity names `min_level`
/// accepts, ascending.
///
/// Published rather than left to be discovered by trial: the set is
/// closed, and a caller guessing at it is how a filter ends up silently
/// doing nothing. Reads from the same [`DiagLevel::ALL`] the filter
/// itself uses, so the two cannot drift.
async fn list_diag_levels() -> ApiResult<Vec<&'static str>> {
    Ok(Json(DiagLevel::ALL.iter().map(|l| l.as_str()).collect()))
}

async fn list_assets(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListAssetsQuery>,
) -> ApiResult<AssetPageDto> {
    Ok(Json(ctx.asset_service.list(query).await?))
}

/// `GET /asterism/assets/index` — the same filter and sort as
/// [`list_assets`], answered as `AssetIndex` projections (no cover text
/// / source locator / file size).
///
/// This is the read the grid itself performs: it takes the whole
/// ordering cheaply and hydrates only what it paints. A caller checking
/// "what order does this filter produce" wants this rather than
/// `list_assets`, which serialises a full card per row.
async fn list_asset_index(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListAssetsQuery>,
) -> ApiResult<AssetIndexPageDto> {
    Ok(Json(ctx.asset_service.list_index(query).await?))
}

/// Request body for `POST /asterism/assets/hydrate`.
#[derive(Deserialize)]
struct HydrateCardsBody {
    ids: Vec<String>,
    viewer_subject: Option<String>,
}

/// `POST /asterism/assets/hydrate` — batch-hydrates cards by id,
/// the companion to [`list_asset_index`]. Ids that do not exist or are
/// hidden from the viewer drop out of the response rather than erroring:
/// a viewport is a guess about what is still there.
///
/// POST because the id list is unbounded — the grid asks for a viewport
/// plus a prefetch window, which does not fit a query string.
async fn hydrate_cards(
    State(ctx): State<Arc<ServerCtx>>,
    Json(body): Json<HydrateCardsBody>,
) -> ApiResult<Vec<AssetCardDto>> {
    Ok(Json(
        ctx.asset_service
            .hydrate_cards(body.ids, body.viewer_subject)
            .await?,
    ))
}

async fn add_asset(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<AddAssetCommand>,
) -> ApiResult<AssetDto> {
    // The command's three attribution fields are this caller's statement
    // about itself; translating them into the context is the adapter's
    // job, and the service reads the context alone.
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    Ok(Json(ctx.asset_service.add(command, &attribution).await?))
}

async fn add_asset_batch(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<AddAssetBatchCommand>,
) -> ApiResult<AddAssetBatchResult> {
    // A batch is one request over one channel, so it carries one
    // attribution. The items each have the three fields (they are
    // `AddAssetCommand`s), so the batch is only translatable when they
    // agree — a batch stating two different authors is two requests,
    // and picking one of them would silently drop the other.
    let mut stated: Option<(Option<String>, Option<String>, Option<String>)> = None;
    for item in &command.items {
        let triple = (
            item.author_kind.clone(),
            item.author_subject.clone(),
            item.operator_ai.clone(),
        );
        match &stated {
            None => stated = Some(triple),
            Some(first) if *first == triple => {}
            Some(_) => {
                return Err(DomainError::Validation(
                    "a batch is one request and records one attribution; its items state \
                     different ones — split it, or leave the fields off"
                        .into(),
                )
                .into());
            }
        }
    }
    let stated = stated.unwrap_or((None, None, None));
    let attribution = asserted(
        stated.0.as_deref(),
        stated.1.as_deref(),
        stated.2.as_deref(),
    )?;
    Ok(Json(
        ctx.asset_service.add_batch(command, &attribution).await?,
    ))
}

async fn update_asset_meta(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<UpdateAssetMetaCommand>,
) -> ApiResult<AssetDto> {
    Ok(Json(
        ctx.asset_service
            .update_meta(command, &asserted(None, None, None)?)
            .await?,
    ))
}

async fn update_asset_meta_batch(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<UpdateAssetMetaBatchCommand>,
) -> ApiResult<UpdateAssetMetaBatchResult> {
    Ok(Json(
        ctx.asset_service
            .update_meta_batch(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/assets/trash` — reversible.
async fn trash_asset(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<TrashAssetCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .trash(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "trashed": true })))
}

/// `POST /asterism/assets/restore`.
async fn restore_asset(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RestoreAssetCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .restore(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "restored": true })))
}

/// `POST /asterism/assets/purge` — irreversible, and rejected with 409
/// unless the asset is already in the trash. Scripted cleanup therefore
/// cannot destroy anything in a single call.
async fn purge_asset(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<PurgeAssetCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .purge(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "purged": true })))
}

/// `POST /asterism/assets/empty-trash` — irreversible, and reaches
/// only rows that already carry a trash stamp. Takes no filter: the
/// trash is one place, not a view of the library.
async fn empty_trash(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<EmptyTrashCommand>,
) -> ApiResult<EmptyTrashResult> {
    Ok(Json(
        ctx.asset_service
            .empty_trash(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/assets/search` — relevance-ranked full text.
///
/// The body carries the list query as its `filter`, so `filter.sort` is
/// expressible; it is answered with `400` rather than dropped, because
/// the BM25 ranking is the order. Sorted listings go to
/// `POST /asterism/assets`, which takes the same filter surface.
async fn search_assets(
    State(ctx): State<Arc<ServerCtx>>,
    Json(query): Json<SearchAssetsQuery>,
) -> ApiResult<RetrievedPageDto> {
    Ok(Json(ctx.asset_service.search(query).await?))
}

/// `POST /asterism/assets/search-ids` — the same retrieval reduced to
/// its rank order.
///
/// Takes the identical body as `/search` and answers with ids only: the
/// caller pairs them with a page it fetched from `POST /asterism/assets`
/// and uses the sequence, not the membership. The same
/// `400`s apply — a `filter.sort` axis or the trash side is refused
/// rather than dropped.
async fn search_asset_ids(
    State(ctx): State<Arc<ServerCtx>>,
    Json(query): Json<SearchAssetsQuery>,
) -> ApiResult<RetrievedIdsDto> {
    Ok(Json(ctx.asset_service.search_ids(query).await?))
}

/// `POST /asterism/assets/random` — a random handful out of the filter.
///
/// The body is the list query as its `filter` plus an optional `k`.
/// Answers with the picks, how many came back, and the exact size of the
/// set they were drawn from. Nothing about it is stable: the same body
/// answers differently every time, which is the point.
/// `filter.sort` is a `400` — the order is the shuffle. The trash side
/// *is* honoured here, unlike on the search path.
async fn random_assets(
    State(ctx): State<Arc<ServerCtx>>,
    Json(query): Json<RandomAssetsQuery>,
) -> ApiResult<SampledPageDto> {
    Ok(Json(ctx.asset_service.sample(query).await?))
}

/// Query-string parameters for `GET /asterism/assets/{id}`.
#[derive(Debug, Default, Deserialize)]
struct DetailParams {
    viewer_subject: Option<String>,
}

async fn asset_detail(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<DetailParams>,
) -> ApiResult<AssetDetailDto> {
    Ok(Json(
        ctx.asset_service
            .detail(GetAssetDetailQuery {
                asset_id: id,
                viewer_subject: params.viewer_subject,
            })
            .await?,
    ))
}

/// Query-string parameters for `GET /asterism/assets/{id}/edges`.
#[derive(Debug, Deserialize)]
struct EdgeParams {
    kind: Option<String>,
    #[serde(default = "default_edge_limit")]
    limit: u32,
}

fn default_edge_limit() -> u32 {
    // Default hover-burst size (three related items).
    3
}

async fn asset_edges(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<EdgeParams>,
) -> ApiResult<Vec<EdgeDto>> {
    Ok(Json(
        ctx.asset_service
            .edges_of(&id, params.kind.as_deref(), params.limit)
            .await?,
    ))
}

/// Query-string parameters for
/// `GET /asterism/assets/{id}/constellation`.
#[derive(Debug, Deserialize)]
struct ConstellationParams {
    viewer_subject: Option<String>,
    #[serde(default = "default_edge_limit")]
    limit: u32,
}

async fn asset_constellation(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<ConstellationParams>,
) -> ApiResult<Vec<ConstellationItemDto>> {
    Ok(Json(
        ctx.asset_service
            .constellation_of(&id, params.viewer_subject.as_deref(), params.limit)
            .await?,
    ))
}

/// Query-string parameters for
/// `GET /asterism/assets/{id}/provenance`.
#[derive(Debug, Deserialize)]
struct ProvenanceParams {
    viewer_subject: Option<String>,
    #[serde(default = "default_provenance_limit")]
    limit: u32,
}

fn default_provenance_limit() -> u32 {
    // Detail-pane Provenance lane fits ~12 cards without wrapping.
    12
}

async fn asset_provenance(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<ProvenanceParams>,
) -> ApiResult<ProvenanceViewDto> {
    Ok(Json(
        ctx.asset_service
            .provenance_of(&id, params.viewer_subject.as_deref(), params.limit)
            .await?,
    ))
}

/// `POST /asterism/assets/{id}/provenance` — declares (or repairs)
/// the asset's origin after the fact.
///
/// The URL names the asset and wins over whatever the body claims,
/// the same arbitration rule as `POST /assets/{id}/comments`: two
/// callers disagreeing about the target must not fork silently.
async fn declare_asset_provenance(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<DeclareProvenanceCommand>,
) -> ApiResult<AssetDto> {
    command.asset_id = id;
    // The command's `operator_ai` is part of the provenance *claim*
    // (`_trace.operator`), not an attribution column, so it stays with
    // the command; this context is the request's own channel.
    Ok(Json(
        ctx.asset_service
            .declare_provenance(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/assets/{id}/album-meta` — records, or removes, one
/// AlbumMeta statement on the asset.
///
/// The URL names the asset and wins over the body, the same arbitration
/// the provenance route above uses. Removal has no separate route: the
/// command already spells it as an absent `value`, and a second spelling
/// would be a second thing to keep in step.
///
/// The command's `operator_ai` is the agent the *statement* came
/// through (`_trace.meta.<key>.operator`), not an attribution column, so
/// it stays on the command; the context below is the request's own
/// channel.
async fn declare_asset_album_meta(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<DeclareAssetMetaCommand>,
) -> ApiResult<AssetDto> {
    command.asset_id = id;
    Ok(Json(
        ctx.asset_service
            .declare_asset_meta(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/assets/{id}/source-type` — asserts, or retracts, the
/// asset's digital source type by hand.
///
/// The URL names the asset and wins over the body, the arbitration its
/// two sibling declare routes use. Removal is an absent `source_type`
/// in the body, the spelling `album-meta` gives its own removal. The
/// term is refused at the door when it is not one the IPTC vocabulary
/// defines — everything downstream signs this verbatim.
async fn declare_asset_source_type(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<DeclareSourceTypeCommand>,
) -> ApiResult<AssetDto> {
    command.asset_id = id;
    Ok(Json(
        ctx.asset_service
            .declare_source_type(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `GET /asterism/assets/{id}/video-preview` — where the transcoded
/// preview rendition stands (`ready` / `pending` / `not_needed` /
/// `failed`). The first call for a missing rendition enqueues the
/// transcode; the caller polls while `pending`.
///
/// Takes `?viewer_subject=` like the other single-asset reads: the
/// status alone confirms an asset exists (and `pending` starts a
/// transcode on the asker's behalf), so a restricted asset answers 404
/// for an outside viewer.
async fn asset_video_preview(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<ViewerParams>,
) -> ApiResult<VideoPreviewDto> {
    Ok(Json(
        ctx.asset_service
            .video_preview(&id, params.viewer_subject.as_deref())
            .await?,
    ))
}

/// `GET /asterism/assets/{id}/lineage`.
#[derive(Debug, Deserialize)]
struct LineageParams {
    viewer_subject: Option<String>,
    #[serde(default = "default_lineage_depth")]
    depth: u32,
}

fn default_lineage_depth() -> u32 {
    // Deep enough for the chains this exists for (out to a generator,
    // back, out again, back) without turning the default request into
    // a corpus walk. `LINEAGE_MAX_DEPTH` is the ceiling.
    4
}

async fn asset_lineage(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<LineageParams>,
) -> ApiResult<LineageViewDto> {
    Ok(Json(
        ctx.asset_service
            .lineage_of(&id, params.viewer_subject.as_deref(), params.depth)
            .await?,
    ))
}

async fn rebuild_edges(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let task_id = ctx.asset_service.rebuild_edges(&id).await?;
    Ok(Json(serde_json::json!({ "task_id": task_id })))
}

/// Query parameters for the sidebar Tags section — a persona filter
/// so the counts follow the active persona pane. Both fields are
/// optional; omitting `persona_id` counts across every persona.
#[derive(Debug, Deserialize, Default)]
struct TagCountsParams {
    #[serde(default)]
    persona_id: Option<String>,
    /// Which side of the trash to count — mirrors `ListAssetsQuery`'s
    /// selector so the sidebar chips can follow the grid. Omitted =
    /// live.
    #[serde(default)]
    trash: Option<String>,
}

/// `GET /asterism/personas/counts` — one entry per persona that
/// owns at least one asset (`key = persona uuid`, count DESC then
/// uuid ASC).
async fn list_persona_asset_counts(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>,
) -> ApiResult<Vec<AssetCountEntryDto>> {
    Ok(Json(
        ctx.asset_service
            .list_persona_asset_counts(params.trash.as_deref())
            .await?,
    ))
}

/// `GET /asterism/modalities/counts` — one entry per modality slug
/// present in the corpus (`key = modality slug`), optionally scoped
/// to one persona via `?persona_id=`.
async fn list_modality_asset_counts(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>,
) -> ApiResult<Vec<AssetCountEntryDto>> {
    Ok(Json(
        ctx.asset_service
            .list_modality_asset_counts(params.persona_id.as_deref(), params.trash.as_deref())
            .await?,
    ))
}

/// `GET /asterism/formats/counts` — sidebar FORMAT facet (asset-model
/// v4): one entry per mime top-level type (`key = "image" | "video" |
/// "audio" | "text" | …`) on top-level assets' primary materials,
/// optionally scoped via `?persona_id=`.
async fn list_format_asset_counts(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>,
) -> ApiResult<Vec<AssetCountEntryDto>> {
    Ok(Json(
        ctx.asset_service
            .list_format_asset_counts(params.persona_id.as_deref(), params.trash.as_deref())
            .await?,
    ))
}

/// `GET /asterism/colors/counts` — sidebar COLOR facet: one entry per
/// palette swatch (`key = "red" | "blue" | "white" | …`) carried by a
/// top-level asset, in swatch order, optionally scoped via
/// `?persona_id=`. Swatches nothing carries are omitted.
async fn list_color_asset_counts(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>,
) -> ApiResult<Vec<AssetCountEntryDto>> {
    Ok(Json(
        ctx.asset_service
            .list_color_asset_counts(params.persona_id.as_deref(), params.trash.as_deref())
            .await?,
    ))
}

/// Query parameters shared by the duplicate reads that take no axis.
#[derive(Debug, Deserialize)]
struct DuplicatesParams {
    /// Restrict to one persona's assets. Omitted = every persona.
    #[serde(default)]
    persona_id: Option<String>,
    /// Maximum number of *groups* (not assets). Omitted = the
    /// service's default work-list size.
    #[serde(default)]
    limit: Option<u32>,
}

/// Query parameters for the duplicate report, which does take one.
///
/// A separate struct rather than an `axis` added to the shared one. The
/// conflicts endpoint next door reads a queue whose rows each carry
/// their own axis, so there is nothing for the parameter to select
/// there — and a struct shared with this one would make
/// `?axis=content` a spelling that returns `200` and every axis, which
/// is the shape of "accepted and ignored" that a caller only finds out
/// about by comparing two answers by hand.
#[derive(Debug, Deserialize)]
struct DuplicateReportParams {
    /// Restrict to one persona's assets. Omitted = every persona.
    #[serde(default)]
    persona_id: Option<String>,
    /// Which fingerprint to group on: `artefact` (default), `content`
    /// or `meta`. An unknown value is a `400` rather than a silent fall
    /// back to the default — the axes are different questions, and
    /// answering the wrong one under a typo is worse than refusing.
    /// `file` was this axis's spelling before V64 and is now one of the
    /// unknown values, deliberately: a caller still sending it is
    /// telling this server something.
    #[serde(default)]
    axis: Option<String>,
    /// Maximum number of *groups* (not assets). Omitted = the
    /// service's default work-list size.
    #[serde(default)]
    limit: Option<u32>,
}

/// `GET /asterism/duplicates` — sets of live assets sharing a
/// fingerprint on `?axis=` (`artefact` by default, `content` for the
/// bytes that decide the decoded result), newest group first, each
/// group's members oldest first.
///
/// Three counts ride along so an empty report can be read correctly.
/// `unhashed_count` non-zero means the library has not finished being
/// fingerprinted; it converges to zero on its own. `unreadable_count`
/// non-zero means that many originals could not be read when the
/// fingerprint pass tried — the walk keeps retrying them, but the
/// number moves only when the files come back. `unwalked_count`
/// non-zero means the content axis has no reading of that many
/// materials — the migration that fills the column in could not open
/// their originals — so a content-axis answer is silent about them
/// until the files are back.
async fn list_duplicate_groups(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<DuplicateReportParams>,
) -> ApiResult<DuplicateReportDto> {
    Ok(Json(
        ctx.asset_service
            .list_duplicate_groups(
                params.persona_id.as_deref(),
                params.axis.as_deref(),
                params.limit,
            )
            .await?,
    ))
}

/// `GET /asterism/duplicates/conflicts` — the questions a person still
/// has to answer, newest first, both sides hydrated as cards.
///
/// Not the same list as `GET /asterism/duplicates`: that one groups on
/// the digest and keeps reporting a pair somebody has ruled to be two
/// separate things (both rows stay, deliberately). This one is what is
/// left to decide.
async fn list_duplicate_conflicts(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<DuplicatesParams>,
) -> ApiResult<Vec<DuplicateConflictDto>> {
    Ok(Json(
        ctx.asset_service
            .list_duplicate_conflicts(params.persona_id.as_deref(), params.limit)
            .await?,
    ))
}

/// `POST /asterism/duplicates/conflicts/resolve` — answers one question.
///
/// `folded` closes the row and queues the fold onto `keeper_id` (which
/// must be one of the pair); `kept` closes the row and leaves both rows
/// alone. `409` when the row was already answered or when either side
/// has since been folded away or thrown in the trash, `400` for a
/// keeper that is not part of the pair.
async fn resolve_duplicate_conflict(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<ResolveDuplicateConflictCommand>,
) -> ApiResult<DuplicateResolutionDto> {
    Ok(Json(
        ctx.asset_service
            .resolve_duplicate_conflict(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/duplicates/merge` — a person's ruling that a set of
/// rows is one thing, carried out.
///
/// `dry_run: true` returns a preview and writes nothing; `dry_run:
/// false` folds the discards into the keeper. The two branches return
/// the **same DTO shape**: a run following a preview reads the answer
/// back on it, and the only field that tells them apart is
/// [`MergeAssetsDto::committed`](asterism_contract::dto::MergeAssetsDto::committed).
/// The knob lives inside the command rather than on the query string
/// because the command already carries every other input the verb
/// reads, and a second one outside it would let a caller ask two things
/// with one call.
///
/// # Namespace choice
///
/// This route sits on `/asterism/duplicates/*` next to the two-row
/// queue verb, and that is a convenience of routing rather than a
/// claim about the domain. **The manual merge verb does not require a
/// detected duplicate**: it collapses whichever N rows the caller
/// declares are one thing. A reader browsing
/// the tree who took the namespace as a promise — "so this only works
/// on rows the fingerprint pass raised a question about" — would be
/// reading the URL wrong. Kept here for cohesion with the queue verb
/// the panel already reaches over the same prefix.
///
/// # Status codes
///
/// * `400` — [`MergePlan::declare`](asterism_core::domain::merge_plan::MergePlan)
///   refused the declaration (member set does not equal `{keeper} ∪
///   discards`, or an id failed to parse). Raised as
///   [`DomainError::Validation`](asterism_core::DomainError::Validation).
/// * `200 OK` with `refusals` **non-empty** and `committed: false` —
///   the fold could not touch a row (the keeper was in the trash, was
///   itself folded, an id names no row, and so on). Refusals are a
///   **response field**, not an HTTP error: one refusal abandons the
///   whole merge (all-or-nothing) and the caller
///   re-reads the panel to rule again, which is a decision to make on
///   the same shape the successful branch returns.
/// * `200 OK` with `warnings` non-empty — a rule (lineage, dispatch)
///   would have declined an *automatic* fold of a pair inside this
///   merge. It is not binding on a person's ruling and does not stop
///   the run; the field exists so the panel can say what the rule was
///   protecting before the caller confirms. Populated on `dry_run`
///   only; empty on the commit branch by design (the caller has
///   already seen them).
/// * `500` — infra failure inside the transaction.
async fn merge_assets(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<MergeAssetsCommand>,
) -> ApiResult<MergeAssetsDto> {
    Ok(Json(
        ctx.asset_service
            .merge_assets(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `GET /asterism/modalities` — the Modality master listing. One row
/// per registered modality (hidden included), ordered by `sort_order`
/// then `slug`, each carrying its live asset count.
async fn list_modalities(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<Vec<ModalityDefDto>> {
    Ok(Json(ctx.modality_service.list().await?))
}

/// `POST /asterism/modalities` — registers a new modality. `409` on a
/// duplicate slug, `400` on a bad slug grammar / unknown kind.
async fn create_modality(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateModalityCommand>,
) -> ApiResult<ModalityDefDto> {
    Ok(Json(
        ctx.modality_service
            .create(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `PATCH /asterism/modalities/{slug}` — partial update (each omitted
/// field is left unchanged). The path `slug` selects the target; any
/// `slug` in the body is ignored.
async fn update_modality(
    State(ctx): State<Arc<ServerCtx>>,
    Path(slug): Path<String>,
    Json(mut command): Json<UpdateModalityCommand>,
) -> ApiResult<ModalityDefDto> {
    command.slug = slug;
    Ok(Json(
        ctx.modality_service
            .update(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `DELETE /asterism/modalities/{slug}` — removes a modality, but only
/// when no asset still carries the slug (`409` otherwise; hide it
/// instead).
async fn delete_modality(
    State(ctx): State<Arc<ServerCtx>>,
    Path(slug): Path<String>,
) -> ApiResult<serde_json::Value> {
    ctx.modality_service
        .delete(DeleteModalityCommand { slug }, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /asterism/series-strategies` — every registered series rule,
/// oldest first, seeded and user-written alike.
///
/// **Rules, not groups.** What a rule put on which key is a different
/// question, and the shape of the statement that answers it follows from
/// a reader that does not exist yet (`SeriesRepository`'s doc records
/// what it will owe). A caller here is about to write a rule, and what
/// it needs is what is already registered.
async fn list_series_strategies(
    State(ctx): State<Arc<ServerCtx>>,
) -> ApiResult<Vec<SeriesStrategyDto>> {
    Ok(Json(ctx.series_strategy_service.list().await?))
}

/// `POST /asterism/series-strategies` — registers a rule and asks for
/// the keys it implies. `400` on a rule this build could not carry out.
///
/// The refusals are the point of the route rather than input hygiene:
/// one stored rule this build cannot read fails **every** page of the
/// derivation walk, so no material gets a key under any rule. Two of
/// them are `400` here (an unknown `decode`, a blank `applies_to`, an
/// empty path), and the third — a body whose `include` is not a list of
/// paths — never reaches the handler: `Json` refuses it as `422` before
/// any of this runs. Different codes, one property, which is the one the
/// blast radius needs: the row is not written.
async fn create_series_strategy(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateSeriesStrategyCommand>,
) -> ApiResult<SeriesStrategyDto> {
    Ok(Json(
        ctx.series_strategy_service
            .create(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `PATCH /asterism/series-strategies/{id}` — partial update (each
/// omitted field is left unchanged). The path `id` selects the target;
/// any `id` in the body is ignored.
///
/// Changing `applies_to` / `decode` / `include` / `exclude` throws away
/// every key derived under this rule and enqueues the walk that derives
/// them again; changing `name` alone does neither. A seeded rule is
/// editable like any other, and the response carries the `updated_at`
/// the edit moved — which is what a later corrective migration reads to
/// tell a pristine seed from one somebody took over.
async fn update_series_strategy(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<UpdateSeriesStrategyCommand>,
) -> ApiResult<SeriesStrategyDto> {
    command.id = id;
    Ok(Json(
        ctx.series_strategy_service
            .update(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `DELETE /asterism/series-strategies/{id}` — removes a rule and, by
/// the schema's cascade, every key derived under it. `404` when the id
/// names nothing.
///
/// No guard, unlike the modality delete: a series key is recomputed from
/// rows already in hand, so removing a rule costs a scan rather than an
/// orphaned asset.
async fn delete_series_strategy(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ctx.series_strategy_service
        .delete(
            DeleteSeriesStrategyCommand { id },
            &asserted(None, None, None)?,
        )
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /asterism/settings` — every known setting, resolved through
/// code default → environment variable → stored row (last wins, so a
/// value the user chose always beats an export).
///
/// Each row reports which layer supplied the value (`source`), the
/// whole chain it came from (`layers`, including any layer that was
/// rejected and why), plus the registry metadata (`kind` / `min` /
/// `max` / `env_var`) — enough to render the control and explain where
/// its value came from without a second request.
async fn list_settings(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<Vec<SettingDto>> {
    Ok(Json(ctx.app_setting_service.list().await?))
}

/// `GET /asterism/settings/{key}` — one resolved setting. `404` when
/// the key is not in the registry.
async fn get_setting(
    State(ctx): State<Arc<ServerCtx>>,
    Path(key): Path<String>,
) -> ApiResult<SettingDto> {
    Ok(Json(ctx.app_setting_service.get(&key).await?))
}

/// `PUT /asterism/settings/{key}` — stores an override. The path `key`
/// selects the target; the body's `key` must still be present (it is a
/// required field) but is overwritten by the path value. `400` when the
/// value does not match the key's declared kind, `404` for an unknown
/// key.
///
/// The response is the *resolved* value; a successful write always
/// makes it `source: "stored"`. It is returned rather than echoed so
/// the caller gets the refreshed `layers` chain in the same round trip.
async fn set_setting(
    State(ctx): State<Arc<ServerCtx>>,
    Path(key): Path<String>,
    Json(mut command): Json<SetSettingCommand>,
) -> ApiResult<SettingDto> {
    command.key = key;
    Ok(Json(
        ctx.app_setting_service
            .set(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `DELETE /asterism/settings/{key}` — clears the override and returns
/// the value that now applies. Idempotent: resetting a key that was
/// never overridden succeeds.
///
/// As with `PUT`, the response is the *resolved* value, which is the
/// layer directly beneath the row just removed — `"env"` when the key's
/// variable is exported and usable, `"default"` otherwise. Clearing the
/// stored row does not clear the environment.
async fn reset_setting(
    State(ctx): State<Arc<ServerCtx>>,
    Path(key): Path<String>,
) -> ApiResult<SettingDto> {
    Ok(Json(
        ctx.app_setting_service
            .reset(ResetSettingCommand { key }, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `GET /asterism/tags/counts` — returns every tag paired with the
/// number of distinct assets currently attached to it, ordered by
/// count descending (name ascending on ties). Dead channels (zero
/// count in the requested scope) are omitted.
async fn list_tag_counts(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>,
) -> ApiResult<Vec<TagCountDto>> {
    Ok(Json(
        ctx.asset_service
            .list_tag_counts(params.persona_id.as_deref())
            .await?,
    ))
}

/// `GET /asterism/sessions` — Sessions view. Returns one row per
/// `session_id` in the query scope, ordered by the session's latest
/// occurrence time (freshest first). Persona / modality / tag_ids
/// filters flow through the same `ListAssetsQuery` structure.
async fn list_sessions(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListAssetsQuery>,
) -> ApiResult<SessionPageDto> {
    Ok(Json(ctx.asset_service.list_sessions(query).await?))
}

/// `POST /asterism/duplicates/rescan` — re-derives conflicts from
/// fingerprints already on the rows.
///
/// Takes no body: there is nothing to scope. The pass is idempotent
/// (`UNIQUE (pair_lo, pair_hi, axis)` + `ON CONFLICT DO NOTHING`) and
/// never folds, so "which rows" is not a question a caller has to
/// answer — and a scoped variant would only be a way to look at less
/// than the whole library for no gain.
async fn rescan_duplicates(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<serde_json::Value> {
    let task_id = ctx.asset_service.rescan_duplicates().await?;
    Ok(Json(serde_json::json!({
        "enqueued": true,
        "task_id": task_id,
    })))
}

/// Body of `POST /asterism/assets/remeasure`.
///
/// Exactly one of the two is meant. Naming ids is the ordinary case —
/// "these ones" — and `scope` is the library-scale sibling.
#[derive(serde::Deserialize)]
struct RemeasureRequest {
    /// The assets to re-measure. Overwrites whatever is stored.
    #[serde(default)]
    asset_ids: Vec<String>,
    /// `unlooked` / `unmeasured` / `all`. Only `all` overwrites; see
    /// `asterism_core::domain::repository::DimsScope`.
    #[serde(default)]
    scope: Option<String>,
}

/// `POST /asterism/assets/remeasure` — re-reads artefacts and rewrites
/// `width_px` / `height_px`.
///
/// Two shapes, because two callers ask different questions:
///
/// - `{"asset_ids": ["…"]}` — "I put the right file behind these, read
///   them again." Overwrites.
/// - `{"scope": "unmeasured"}` — "the situation changed" (a volume is
///   mounted now, a parser learned a container). `"all"` is "the
///   *measurement* changed" and is the only scope that replaces answers.
///
/// Naming both is refused rather than silently preferring one: they are
/// different requests and a caller that sent both did not mean either.
async fn remeasure_dims(
    State(ctx): State<Arc<ServerCtx>>,
    Json(body): Json<RemeasureRequest>,
) -> ApiResult<serde_json::Value> {
    match (body.asset_ids.is_empty(), body.scope.as_deref()) {
        (false, None) => {
            let ids = body
                .asset_ids
                .iter()
                .map(|s| asterism_core::application::mapping::parse_asset_id(s))
                .collect::<Result<Vec<_>, _>>()?;
            let task_ids = ctx.asset_service.remeasure_dims(&ids).await?;
            Ok(Json(serde_json::json!({
                "enqueued": task_ids.len(),
                "task_ids": task_ids,
            })))
        }
        (true, Some(scope)) => {
            let task_id = ctx.asset_service.remeasure_dims_batch(scope).await?;
            Ok(Json(serde_json::json!({
                "enqueued": 1,
                "scope": scope,
                "task_id": task_id,
            })))
        }
        (true, None) => {
            Err(DomainError::Validation("name either asset_ids or a scope".into()).into())
        }
        (false, Some(_)) => Err(DomainError::Validation(
            "asset_ids and scope are different requests; name one".into(),
        )
        .into()),
    }
}

/// `POST /asterism/sessions/rebuild` — enqueues a `SessionRebuild`
/// job. The handler in `asterism_infra::jobs::handlers` calls
/// `SessionStore::rebuild()` on a worker thread; the returned task
/// id is the engine's job identifier (progress emits via the same
/// channel as the other jobs).
async fn rebuild_sessions(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<serde_json::Value> {
    let task_id = ctx.asset_service.rebuild_sessions().await?;
    Ok(Json(serde_json::json!({
        "enqueued": true,
        "task_id": task_id,
    })))
}

/// `GET /asterism/sessions/{id}` — one Session by surrogate id.
/// Returns `null` when the id is not registered (mirrors the
/// `get_thread` / `get_persona_theme` pattern; the UI's inline edit
/// path uses this to hydrate the tile it just wrote to).
async fn get_session(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Option<SessionDto>> {
    Ok(Json(ctx.session_service.get(&id).await?))
}

/// `POST /asterism/sessions/{id}/rename` — rewrites the Session's
/// title. `title = null` on the body clears the title back to
/// "untitled" (the canonical clear path — the PATCH endpoint
/// cannot express NULL because per-field `None` there means "leave
/// unchanged").
async fn rename_session(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<RenameSessionCommand>,
) -> ApiResult<SessionDto> {
    command.id = id;
    Ok(Json(
        ctx.session_service
            .rename(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `PATCH /asterism/sessions/{id}` — partial metadata update
/// (`title` / `note` / `cover_hint`). Every omitted / `null` field
/// is left unchanged. To clear `title` back to `null` use the
/// dedicated rename endpoint above.
async fn patch_session_metadata(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<PatchSessionMetadataCommand>,
) -> ApiResult<SessionDto> {
    command.id = id;
    Ok(Json(
        ctx.session_service
            .patch_metadata(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `DELETE /asterism/sessions/{id}` — deletes the Session, but
/// only when no `asset` row still references it (`409 Conflict`
/// otherwise; detach the participating assets first). Mirror of
/// the Modality delete guard.
async fn delete_session(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ctx.session_service
        .delete_if_empty(DeleteSessionCommand { id }, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `POST /asterism/index/rebuild` — enqueues a batch `IndexRebuild`
/// job. The handler in `asterism_infra::jobs::handlers::index_rebuild`
/// walks `asset LEFT JOIN asset_body IS NULL` in pages, resolves
/// each body via `SourceTextReader`, upserts `asset_body`, and adds
/// the doc to the Tantivy index. Chain-enqueues itself until the
/// scan runs out of rows — idempotent on an already-indexed DB.
async fn rebuild_index(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<serde_json::Value> {
    let task_id = ctx.asset_service.rebuild_index().await?;
    Ok(Json(serde_json::json!({
        "enqueued": true,
        "task_id": task_id,
    })))
}

/// `GET /asterism/groups?persona_id=<pid>` — sidebar Groups.
async fn list_groups(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>, // shape identical to tag_counts
) -> ApiResult<Vec<GroupSummaryDto>> {
    Ok(Json(
        ctx.asset_service
            .list_groups(params.persona_id.as_deref())
            .await?,
    ))
}

/// `POST /asterism/groups/create`.
async fn create_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateGroupCommand>,
) -> ApiResult<GroupDto> {
    Ok(Json(
        ctx.asset_service
            .create_group(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/groups/trash` — reversible; membership and drag
/// order survive, member assets are untouched.
async fn trash_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<TrashGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .trash_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "trashed": true })))
}

/// `POST /asterism/groups/restore`.
async fn restore_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RestoreGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .restore_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "restored": true })))
}

/// `POST /asterism/groups/purge` — irreversible, 409 unless already
/// trashed.
async fn purge_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<PurgeGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .purge_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "purged": true })))
}

/// `POST /asterism/groups/add-asset`.
async fn add_asset_to_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<AddAssetToGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .add_asset_to_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "added": true })))
}

/// `POST /asterism/groups/remove-asset`.
async fn remove_asset_from_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RemoveAssetFromGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .remove_asset_from_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "removed": true })))
}

/// `POST /asterism/groups/batch-membership` — bulk attach/detach
/// pairs in one call (AI / script membership cleanup).
async fn batch_group_membership(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<BatchGroupMembershipCommand>,
) -> ApiResult<serde_json::Value> {
    let (attached, detached) = ctx
        .asset_service
        .batch_group_membership(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(
        serde_json::json!({ "attached": attached, "detached": detached }),
    ))
}

/// `POST /asterism/groups/merge` — merge one manual group into
/// another and delete the source (duplicate-group consolidation).
async fn merge_groups(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<MergeGroupsCommand>,
) -> ApiResult<serde_json::Value> {
    let moved = ctx
        .asset_service
        .merge_groups(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(
        serde_json::json!({ "moved": moved, "deleted_from": true }),
    ))
}

/// `POST /asterism/tags/attach` — attach a tag to an asset by name.
async fn attach_tag(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<AttachTagCommand>,
) -> ApiResult<TagDto> {
    Ok(Json(
        ctx.asset_service
            .attach_tag(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/tags/detach` — remove a tag from an asset.
async fn detach_tag(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DetachTagCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .detach_tag(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "detached": true })))
}

/// `POST /asterism/tags/attach-batch` — attach one tag to many assets.
async fn attach_tag_batch(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<AttachTagBatchCommand>,
) -> ApiResult<AttachTagBatchResult> {
    Ok(Json(
        ctx.asset_service
            .attach_tag_batch(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/tags/detach-batch` — detach one tag from many assets.
async fn detach_tag_batch(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DetachTagBatchCommand>,
) -> ApiResult<DetachTagBatchResult> {
    Ok(Json(
        ctx.asset_service
            .detach_tag_batch(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/tags/rename` — rename a channel in place.
///
/// `409` when the name belongs to another tag: rename never merges.
async fn rename_tag(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RenameTagCommand>,
) -> ApiResult<TagDto> {
    Ok(Json(
        ctx.asset_service
            .rename_tag(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/tags/delete` — drop a channel and every link to
/// it (`404` on an unknown id).
async fn delete_tag(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteTagCommand>,
) -> ApiResult<DeleteTagResult> {
    Ok(Json(
        ctx.asset_service
            .delete_tag(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/tags/merge` — fold one channel into another and
/// delete the source. `dry_run` reports the same numbers without
/// writing.
async fn merge_tags(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<MergeTagsCommand>,
) -> ApiResult<MergeTagsResult> {
    Ok(Json(
        ctx.asset_service
            .merge_tags(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/tags/promote-to-group` — snapshot the tag's
/// current asset set into a new hand-curated Group.
async fn promote_tag_to_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<PromoteTagToGroupCommand>,
) -> ApiResult<PromoteTagToGroupResult> {
    Ok(Json(
        ctx.asset_service
            .promote_tag_to_group(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/groups/reorder`.
async fn reorder_group_assets(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<ReorderGroupAssetsCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .reorder_group_assets(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "reordered": true })))
}

/// Request body for `POST /asterism/assets/texts`.
#[derive(Deserialize)]
struct AssetTextsBody {
    asset_ids: Vec<String>,
    viewer_subject: Option<String>,
}

/// `POST /asterism/assets/texts` — resolves the full source text of
/// each asset (session Reader view).
async fn asset_texts(
    State(ctx): State<Arc<ServerCtx>>,
    Json(body): Json<AssetTextsBody>,
) -> ApiResult<Vec<AssetTextDto>> {
    Ok(Json(
        ctx.asset_service
            .asset_texts(&body.asset_ids, body.viewer_subject.as_deref())
            .await?,
    ))
}

/// `POST /asterism/groups/rename`.
async fn rename_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RenameGroupCommand>,
) -> ApiResult<GroupDto> {
    Ok(Json(
        ctx.asset_service
            .rename_group(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/groups/move-to-dir`.
async fn move_group_to_dir(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<MoveGroupToDirCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .move_group_to_dir(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "moved": true })))
}

/// `POST /asterism/groups/link`.
async fn link_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<LinkGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .link_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "linked": true })))
}

/// `POST /asterism/groups/unlink`.
async fn unlink_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<UnlinkGroupCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .unlink_group(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "unlinked": true })))
}

/// `GET /asterism/groups/links?persona_id=<pid>` — every
/// group-in-group connection in scope.
async fn list_group_links(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>, // shape identical to tag_counts
) -> ApiResult<Vec<GroupLinkDto>> {
    Ok(Json(
        ctx.asset_service
            .list_group_links(params.persona_id.as_deref())
            .await?,
    ))
}

/// `POST /asterism/groups/reorder-children`.
async fn reorder_group_children(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<ReorderGroupChildrenCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .reorder_group_children(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "reordered": true })))
}

/// `GET /asterism/dirs?persona_id=<pid>` — sidebar Dir tree (flat
/// `parent_id` list; the client assembles the tree).
async fn list_dirs(
    State(ctx): State<Arc<ServerCtx>>,
    Query(params): Query<TagCountsParams>, // shape identical to tag_counts
) -> ApiResult<Vec<DirDto>> {
    Ok(Json(
        ctx.asset_service
            .list_dirs(params.persona_id.as_deref())
            .await?,
    ))
}

/// `POST /asterism/dirs/create`.
async fn create_dir(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateDirCommand>,
) -> ApiResult<DirDto> {
    Ok(Json(
        ctx.asset_service
            .create_dir(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/dirs/rename`.
async fn rename_dir(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RenameDirCommand>,
) -> ApiResult<DirDto> {
    Ok(Json(
        ctx.asset_service
            .rename_dir(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/dirs/move`.
async fn move_dir(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<MoveDirCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .move_dir(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "moved": true })))
}

/// `POST /asterism/dirs/delete`.
async fn delete_dir(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteDirCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .delete_dir(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// Query-string parameters shared by the byte-serving asset reads
/// (`/file`, `/thumbs/{size_px}`).
#[derive(Debug, Default, Deserialize)]
struct ViewerParams {
    viewer_subject: Option<String>,
}

/// `GET /asterism/assets/{id}/thumbs/{size_px}` — returns the cached
/// thumbnail bytes with `Content-Type: image/jpeg` (the encoding the
/// importer produces).
///
/// Visibility is the same filtering contract as the detail / original
/// reads: an asset restricted away from `viewer_subject` — and an id
/// nothing is filed under — both answer **404**, indistinguishably.
/// A thumbnail is a rendition of the artefact, so it leaks exactly what
/// the original would.
async fn get_thumb(
    State(ctx): State<Arc<ServerCtx>>,
    Path((id, size_px)): Path<(String, u32)>,
    Query(params): Query<ViewerParams>,
) -> Result<Response, ApiError> {
    // The gate answers before the cache probe on purpose: the miss
    // branch below is a 202, so a gate placed inside the hit branch
    // would refuse the bytes yet confirm the asset's existence through
    // the 404/202 split — and enqueue work for a caller who may not
    // even name a real asset.
    ctx.asset_service
        .assert_visible(&id, params.viewer_subject.as_deref())
        .await?;
    match ctx.thumb_service.get(&id, size_px).await? {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "image/jpeg")], Bytes::from(bytes)).into_response())
        }
        None => {
            // On cache miss enqueue a high-priority `thumb_gen`
            // job so the ImageIO worker (HW JPEG decode on Apple
            // Silicon) materialises the blob on the next tick.
            // The client polls this endpoint until it turns 200 —
            // an interactive open normally settles under 100 ms
            // even at 512 px because the decoder is on-device.
            let _ = ctx.asset_service.enqueue_thumb_gen(&id, size_px).await;
            Ok((
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "kind": "Accepted",
                    "message": "thumb generation queued",
                })),
            )
                .into_response())
        }
    }
}

/// `GET /asterism/assets/{id}/file` — the asset's **original** bytes.
///
/// The thumbnail route serves a derived rendition; this serves the
/// artefact itself, which is what makes an off-machine caller able to
/// read the library at all. `locator` names a path on *this* disk, so
/// before this route existed a remote agent held an identifier it could
/// not open.
///
/// Streamed (`ReaderStream`), never buffered: an original can be a
/// multi-gigabyte video, and reading one into memory to answer a request
/// would be the largest allocation in the process.
///
/// Three answers other than the bytes, each about a different subject:
///
/// - **404** — no such asset, *or* an asset restricted away from
///   `viewer_subject`. Same conflation as `GET /asterism/assets/{id}`,
///   deliberately: a distinguishable 403 would confirm the asset exists.
///   Note `viewer_subject` is caller-asserted (no authentication on the
///   loopback port), so this is a filtering contract shared with the
///   rest of the read surface, not an authorization boundary — omitting
///   the parameter reads as the owner.
/// - **409** — the asset exists and is visible, but its original is not
///   a file on this disk (a record inside a container file, a remote
///   URL, a caller-minted logical name). A fact about the asset, not
///   about the request, so it is not a 404.
/// - **404, "asset original file not found"** — the row is here and its
///   locator is a path, but nothing is at that path. `410 Gone` states
///   this more precisely; it is not used because the status comes from
///   the shared `DomainError` → status table (see [`ApiError`]) and a
///   fourth mapping there would be a transport concern leaking into
///   every other route's error vocabulary. The message carries the
///   distinction the status drops.
///
/// **Range requests are not honoured** — the whole file comes back for
/// every call. Deliberate for the first cut, and silent (no
/// `Accept-Ranges`) because ignoring an unsupported `Range` is what HTTP
/// asks of a server that cannot serve one. Video scrubbing over HTTP is
/// what will need it.
async fn get_asset_file(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(params): Query<ViewerParams>,
) -> Result<Response, ApiError> {
    let original = ctx
        .asset_service
        .original_file(&id, params.viewer_subject.as_deref())
        .await?;
    let file = match tokio::fs::File::open(&original.path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(DomainError::NotFound {
                entity: "asset original file",
                id: original.locator,
            }
            .into());
        }
        Err(err) => {
            return Err(DomainError::Infra(anyhow::anyhow!(
                "asset original is unreadable ({}): {err}",
                original.locator
            ))
            .into());
        }
    };
    // Length from the open handle rather than a second `stat` on the
    // path: the two can disagree, and the one that must match the bytes
    // on the socket is the file already opened. The same metadata also
    // answers "is this a regular file" — `open(2)` succeeds on a
    // directory, and streaming one would be a `200` whose body dies on
    // the first read (`EISDIR`), so a non-file is refused up front with
    // the same answer a container-record locator gets.
    let meta = file.metadata().await.map_err(|err| {
        DomainError::Infra(anyhow::anyhow!(
            "asset original is unreadable ({}): {err}",
            original.locator
        ))
    })?;
    if !meta.is_file() {
        return Err(DomainError::Conflict(format!(
            "asset original is not a regular file: {}",
            original.locator
        ))
        .into());
    }
    let length = meta.len();
    // The stored token, whatever it was: a format this codebase does
    // not name still round-trips as its own `Content-Type` rather than
    // collapsing to the octet-stream default.
    let mime = original
        .mime
        .as_ref()
        .map_or("application/octet-stream", MimeType::as_str);

    // 64 KiB chunks, not `ReaderStream`'s 4 KiB default: the doc above
    // argues from the multi-gigabyte case, where 4 KiB means a million
    // read syscalls per file.
    let mut response = Response::new(Body::from_stream(ReaderStream::with_capacity(
        file,
        64 * 1024,
    )));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    // The bytes come back verbatim under a stored mime; `nosniff` keeps
    // a browser from second-guessing that type into something
    // executable on this (unauthenticated, loopback) origin.
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    Ok(response)
}

/// `PUT /asterism/assets/{id}/thumbs/{size_px}` — raw body upload;
/// the importer produces a JPEG-encoded resize and posts it here.
async fn put_thumb(
    State(ctx): State<Arc<ServerCtx>>,
    Path((id, size_px)): Path<(String, u32)>,
    body: Bytes,
) -> ApiResult<serde_json::Value> {
    ctx.thumb_service.put(&id, size_px, body.to_vec()).await?;
    Ok(Json(
        serde_json::json!({ "stored": true, "bytes": body.len() }),
    ))
}

// ---------------------------------------------------------------------------
// Outbound dispatch — dispatch jobs / registered exporters.
//
// The Selection CRUD endpoints (list / create / get) were removed in the
// W3a Snapshot transmigration: a Snapshot is a system-generated content
// object with no public list / rename / delete surface. The W3c
// command surface (`snapshot_get` / `snapshot_members` / dispatch_run /
// dispatch_list / redispatch) supersedes them.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Query groups — Groups whose membership is a materialised rule.
// The SavedQuery routes that lived here were absorbed into these (V19
// transcribed the rows; the concept is gone).
// ---------------------------------------------------------------------------

async fn create_query_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateQueryGroupCommand>,
) -> ApiResult<GroupDto> {
    Ok(Json(
        ctx.query_group_service
            .create_query_group(command, &asserted(None, None, None)?)
            .await?,
    ))
}

async fn update_query_group_query(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<UpdateQueryGroupQueryCommand>,
) -> ApiResult<GroupDto> {
    Ok(Json(
        ctx.query_group_service
            .update_query(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// Query params on `GET /asterism/dispatch`.
#[derive(Debug, Deserialize)]
struct ListDispatchQuery {
    #[serde(default)]
    persona_id: Option<String>,
    #[serde(default)]
    snapshot_id: Option<String>,
    /// Lifecycle state slug (`pending` / `running` / `done` / `failed`
    /// / `cancelled`). `None` = every state.
    #[serde(default)]
    state: Option<String>,
    #[serde(default = "default_dispatch_limit")]
    limit: u32,
}

fn default_dispatch_limit() -> u32 {
    50
}

async fn list_dispatch(
    State(ctx): State<Arc<ServerCtx>>,
    Query(q): Query<ListDispatchQuery>,
) -> ApiResult<Vec<DispatchDto>> {
    Ok(Json(
        ctx.dispatch_service
            .list(
                q.persona_id.as_deref(),
                q.snapshot_id.as_deref(),
                q.state.as_deref(),
                q.limit,
            )
            .await?,
    ))
}

async fn create_dispatch(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateDispatchCommand>,
) -> ApiResult<DispatchDto> {
    // Pre-flight registry check so an unregistered exporter slug
    // fails fast (400) rather than persisting a job that will never
    // run. `Exporter::accepts` is the exporter-side variant of this
    // check; the runner also runs it, but on a queue delay.
    if let Some(exp) = ctx.exporter_registry.get(&command.exporter_slug) {
        if !exp.accepts(&command.action) {
            return Err(ApiError::from(asterism_core::DomainError::Validation(
                format!(
                    "exporter {:?} does not accept action {:?}",
                    command.exporter_slug, command.action
                ),
            )));
        }
    } else if !ctx.exporter_registry.slugs().is_empty() {
        // Registry has entries but not this one — clearly a
        // misrouted request.
        return Err(ApiError::from(asterism_core::DomainError::Validation(
            format!("exporter not registered: {:?}", command.exporter_slug),
        )));
    }
    // Empty registry: still let the job land so ops can inspect the
    // row and register the exporter later; the runner marks it Failed
    // on the first tick.
    let attribution = asserted(None, None, command.operator_ai.as_deref())?;
    Ok(Json(
        ctx.dispatch_service.create(command, &attribution).await?,
    ))
}

async fn get_dispatch(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<DispatchDto> {
    Ok(Json(ctx.dispatch_service.get(&id).await?))
}

/// Live-source dispatch (`dispatch_run`): freezes a Group (query
/// groups refreshed first) or a volatile selection into a deduped
/// Snapshot and enqueues the run.
async fn dispatch_run(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DispatchRunCommand>,
) -> ApiResult<DispatchDto> {
    if let Some(exp) = ctx.exporter_registry.get(&command.exporter_slug) {
        if !exp.accepts(&command.action) {
            return Err(ApiError::from(asterism_core::DomainError::Validation(
                format!(
                    "exporter {:?} does not accept action {:?}",
                    command.exporter_slug, command.action
                ),
            )));
        }
    } else if !ctx.exporter_registry.slugs().is_empty() {
        return Err(ApiError::from(asterism_core::DomainError::Validation(
            format!("exporter not registered: {:?}", command.exporter_slug),
        )));
    }
    let attribution = asserted(None, None, command.operator_ai.as_deref())?;
    Ok(Json(ctx.dispatch_service.run(command, &attribution).await?))
}

/// Re-runs a finished dispatch with the same frozen input (P2).
async fn redispatch(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<RedispatchCommand>,
) -> ApiResult<DispatchDto> {
    // `RedispatchCommand` carries no operator field — a re-run states
    // nothing about itself, so the honest record is unrecorded.
    Ok(Json(
        ctx.dispatch_service
            .redispatch(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `GET /asterism/assets/{id}/groups` — which Groups the asset already
/// sits in. Drives the "already added" state on the UI's add-to-group
/// dropdown; for an agent it answers the same question before a write.
async fn groups_of_asset(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<GroupDto>> {
    Ok(Json(ctx.asset_service.groups_of_asset(&id).await?))
}

/// Query for `GET /asterism/assets/{id}/snapshots`.
#[derive(Deserialize)]
struct SnapshotsContainingQuery {
    /// Cap on returned rows. Defaults to
    /// [`DEFAULT_SNAPSHOTS_CONTAINING_LIMIT`] rather than being required,
    /// so the endpoint answers without a caller having to invent a bound.
    #[serde(default = "default_snapshots_containing_limit")]
    limit: u32,
}

/// Rows returned by `GET /asterism/assets/{id}/snapshots` when the
/// caller names no limit.
const DEFAULT_SNAPSHOTS_CONTAINING_LIMIT: u32 = 20;

fn default_snapshots_containing_limit() -> u32 {
    DEFAULT_SNAPSHOTS_CONTAINING_LIMIT
}

/// `GET /asterism/assets/{id}/snapshots` — reverse lookup: every
/// Snapshot whose frozen member list contains this asset.
async fn list_snapshots_containing(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(query): Query<SnapshotsContainingQuery>,
) -> ApiResult<Vec<SnapshotDto>> {
    Ok(Json(
        ctx.snapshot_service
            .list_containing(&id, query.limit)
            .await?,
    ))
}

/// `POST /asterism/snapshots/promote-to-group` — promotes an existing
/// Snapshot into a hand-owned Group, bulk-attaching every member in
/// frozen order. Sibling of `promote-volatile`, which mints the Snapshot
/// first; this one starts from a Snapshot that already exists.
async fn promote_snapshot_to_group(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<PromoteSnapshotToGroupCommand>,
) -> ApiResult<PromoteSnapshotToGroupResult> {
    Ok(Json(
        ctx.snapshot_service
            .promote_to_group(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/snapshots/create` — freeze a picked asset list into
/// a Snapshot and stop there.
///
/// The freeze half of `promote-volatile` on its own. Until now the only
/// way to mint a Snapshot from the outside was that fused call, which
/// also creates a Group — so "keep this exact set, decide later what to
/// do with it" had no route, even though `dispatch/create` and
/// `snapshots/{id}` both speak in snapshot ids the caller then had no
/// way to obtain.
async fn create_snapshot(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateSnapshotCommand>,
) -> ApiResult<SnapshotDto> {
    Ok(Json(
        ctx.snapshot_service
            .create(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// Snapshot view metadata (`snapshot_get`).
async fn get_snapshot(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<SnapshotDto> {
    Ok(Json(ctx.snapshot_service.get_snapshot(&id).await?))
}

/// Snapshot view members in frozen order (`snapshot_members`).
async fn snapshot_members(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<AssetCardDto>> {
    Ok(Json(ctx.snapshot_service.snapshot_members(&id).await?))
}

/// `POST /asterism/snapshots/promote-volatile` — freeze the caller's
/// volatile pick into a Snapshot and promote it into a hand-owned
/// Group in one step (W5-d).
async fn promote_volatile_selection(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<PromoteVolatileSelectionCommand>,
) -> ApiResult<PromoteSnapshotToGroupResult> {
    Ok(Json(
        ctx.snapshot_service
            .promote_volatile_selection(command, &asserted(None, None, None)?)
            .await?,
    ))
}

async fn list_exporters(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<Vec<String>> {
    Ok(Json(ctx.exporter_registry.slugs()))
}

// -----------------------------------------------------------------
// Comment thread on an Asset.
//
// Sibling of the app-level Threads primitive below, on a narrower
// anchor: these hang off one Asset, carry an author (`user` /
// `persona`), and are what the detail panel shows. The UI reaches them
// through four Tauri commands; these routes are the same four calls on
// the same service, so an agent can read and write the thread without
// driving the panel.
// -----------------------------------------------------------------

/// `GET /asterism/assets/{id}/comments` — the whole thread in
/// chronological order.
async fn list_asset_comments(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<AssetCommentDto>> {
    Ok(Json(ctx.asset_comment_service.list(&id).await?))
}

/// `GET /asterism/assets/{id}/tag-suggestions` — what the bound model
/// proposed for this asset (#112), score-descending, rulings included.
/// Empty on a build or profile without a model.
async fn list_tag_suggestions(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<TagSuggestionDto>> {
    Ok(Json(ctx.asset_service.tag_suggestions_of(&id).await?))
}

/// `POST /asterism/assets/{id}/tag-suggestions/{tag_id}/accept` — a
/// person takes the suggestion: the ruling lands on the evidence row
/// and the tag is linked in `asset_tag`.
async fn accept_tag_suggestion(
    State(ctx): State<Arc<ServerCtx>>,
    Path((id, tag_id)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .accept_tag_suggestion(&id, &tag_id, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "accepted": true })))
}

/// `POST /asterism/assets/{id}/tag-suggestions/{tag_id}/reject` — a
/// person refuses the suggestion; this model never proposes the pair
/// again.
async fn reject_tag_suggestion(
    State(ctx): State<Arc<ServerCtx>>,
    Path((id, tag_id)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_service
        .reject_tag_suggestion(&id, &tag_id, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "rejected": true })))
}

/// `GET /asterism/models/status` — which visual model this process
/// bound, if any (#112). All-null when the process runs without one.
async fn visual_model_status(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<VisualModelStatusDto> {
    Ok(Json(ctx.asset_service.visual_model_status().await))
}

/// Body of `POST /asterism/models/fetch` — a registry entry by
/// reference or by value, exactly one.
#[derive(serde::Deserialize)]
struct ModelFetchRequest {
    /// Where the entry lives (the instance's registry route, or
    /// wherever the provider put it).
    #[serde(default)]
    url: Option<String>,
    /// The entry itself — for a caller that already fetched it (the
    /// instance's route sits behind its own session, and this server
    /// holds no such credential).
    #[serde(default)]
    entry: Option<serde_json::Value>,
}

/// `POST /asterism/models/fetch` — enqueues a `ModelFetch` install
/// (#126). The shape is validated here so a caller hears the refusal
/// at request time rather than as a failed job run; everything deeper
/// — entry schema, digests, the replacement — is the handler's, where
/// the entry's own verification lives. Installation does not bind:
/// the completion message says to restart (#112's bind-once).
async fn fetch_visual_model(
    State(ctx): State<Arc<ServerCtx>>,
    Json(body): Json<ModelFetchRequest>,
) -> ApiResult<serde_json::Value> {
    let payload = match (body.url, body.entry) {
        (Some(url), None) => serde_json::json!({ "url": url }),
        (None, Some(entry)) => serde_json::json!({ "entry": entry }),
        (None, None) => {
            return Err(DomainError::Validation("name either url or entry".into()).into());
        }
        (Some(_), Some(_)) => {
            return Err(DomainError::Validation(
                "url and entry are different requests; name one".into(),
            )
            .into());
        }
    };
    let task_id = ctx.asset_service.fetch_model(payload).await?;
    Ok(Json(serde_json::json!({
        "enqueued": true,
        "task_id": task_id,
    })))
}

/// `POST /asterism/assets/{id}/comments` — appends one comment.
///
/// The URL path is authoritative for `asset_id`: a body naming a
/// different asset is overwritten rather than honoured, so the two can
/// never disagree about where the comment landed (same rule as
/// [`append_thread_message`]). Author identity stays a body field — the
/// server does not infer who is posting.
async fn post_asset_comment(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<PostAssetCommentCommand>,
) -> ApiResult<AssetCommentDto> {
    command.asset_id = id;
    Ok(Json(
        ctx.asset_comment_service
            .post(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/comments/edit` — rewrites a comment body (stamps
/// `edited_at`).
///
/// Not `PATCH /assets/{id}/comments/{comment_id}`: a comment id already
/// identifies the row on its own, and putting the asset in the path
/// would invite a mismatched pair the handler has to arbitrate.
async fn edit_asset_comment(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<EditAssetCommentCommand>,
) -> ApiResult<AssetCommentDto> {
    Ok(Json(
        ctx.asset_comment_service
            .edit(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/comments/delete` — removes one comment. Idempotent;
/// a missing id is not an error.
async fn delete_asset_comment(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteAssetCommentCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.asset_comment_service
        .delete(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// -----------------------------------------------------------------
// Marks in an Asset's material.
//
// Sibling of the comment thread above on a narrower anchor: a comment
// is about the asset, a mark is at a position inside what the asset
// holds (today a point or interval on its playback timeline). The four
// routes mirror the comment four, on the same four Tauri commands'
// service, so an agent can place and read marks without driving the
// panel.
// -----------------------------------------------------------------

/// `GET /asterism/assets/{id}/material-marks` — every mark in the
/// asset's material, in the material's own order (`start_ms` ascending,
/// ties broken by id) rather than the order they were placed in.
async fn list_material_marks(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<MaterialMarkDto>> {
    Ok(Json(ctx.material_mark_service.list_by_asset(&id).await?))
}

/// `POST /asterism/assets/{id}/material-marks` — places one mark.
///
/// The URL path is authoritative for `asset_id`, as on the comment
/// route: a body naming a different asset is overwritten rather than
/// honoured. Anchor and author identity stay body fields — the server
/// infers neither where the mark goes nor who is speaking.
async fn post_material_mark(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<PostMaterialMarkCommand>,
) -> ApiResult<MaterialMarkDto> {
    command.asset_id = id;
    Ok(Json(
        ctx.material_mark_service
            .post(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/material-marks/edit` — rewrites a mark's body
/// (stamps `edited_at`). The anchor is not editable here.
async fn edit_material_mark(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<EditMaterialMarkCommand>,
) -> ApiResult<MaterialMarkDto> {
    Ok(Json(
        ctx.material_mark_service
            .edit(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/material-marks/delete` — removes one mark.
/// Idempotent; a missing id is not an error.
async fn delete_material_mark(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteMaterialMarkCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.material_mark_service
        .delete(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// -----------------------------------------------------------------
// Bands over an Asset's material, and the chapters in one.
//
// A layer says where a set of marks came from: the material's own
// declaration, a person's reading of it, or a job's. The write routes
// below accept the person's bands and refuse the other two, because
// those are reproduced by running their producer again — an edit into
// one is lost at the next run, and a delete undoes itself.
// -----------------------------------------------------------------

/// `GET /asterism/assets/{id}/material-layers` — every band over the
/// asset's material, each carrying the chapters in it.
///
/// One call rather than one per band: the surface that shows a chapter
/// list needs the bands to choose between and the contents of the chosen
/// one at the same moment, and an asset carries single-digit bands. An
/// annotation band's `chapters` is always empty — those hold notes, read
/// through `/material-marks` above.
async fn list_material_layers(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<MaterialLayerViewDto>> {
    Ok(Json(ctx.material_layer_service.list_views(&id).await?))
}

/// `POST /asterism/assets/{id}/material-layers` — opens a band the
/// person owns.
///
/// The URL path is authoritative for `asset_id`, as on the mark route: a
/// body naming a different asset is overwritten rather than honoured.
async fn create_material_layer(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<CreateMaterialLayerCommand>,
) -> ApiResult<MaterialLayerDto> {
    command.asset_id = id;
    Ok(Json(
        ctx.material_layer_service
            .create_layer(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/material-layers/set-default` — chooses the band a
/// surface shows, and the one a new mark lands in.
///
/// Answers with a flag rather than a row because the call changes two of
/// them: the caller re-reads the asset's bands rather than patching the
/// one it named.
async fn set_default_material_layer(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<SetDefaultMaterialLayerCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.material_layer_service
        .set_default_layer(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "updated": true })))
}

/// `POST /asterism/material-layers/delete` — removes a band the person
/// owns, with everything in it. Refuses an imported or machine band.
async fn delete_material_layer(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteMaterialLayerCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.material_layer_service
        .delete_layer(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /asterism/material-layers/{id}/chapter-marks` — the sections in
/// one band, in the reading order the band states (which need not be the
/// timeline's).
async fn list_chapter_marks(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<ChapterMarkDto>> {
    Ok(Json(
        ctx.material_layer_service.list_chapter_marks(&id).await?,
    ))
}

/// `POST /asterism/material-layers/{id}/chapter-marks` — adds one
/// section. The URL path is authoritative for `layer_id`.
async fn post_chapter_mark(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<PostChapterMarkCommand>,
) -> ApiResult<ChapterMarkDto> {
    command.layer_id = id;
    Ok(Json(
        ctx.material_layer_service
            .post_chapter_mark(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/chapter-marks/edit` — retitles a section and, unlike
/// the mark face, may move it: a person opens a band of their own
/// because the file's divisions are in the wrong places.
async fn edit_chapter_mark(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<EditChapterMarkCommand>,
) -> ApiResult<ChapterMarkDto> {
    Ok(Json(
        ctx.material_layer_service
            .edit_chapter_mark(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/chapter-marks/delete` — removes one section. **Not**
/// idempotent, unlike the mark route: a chapter is named by
/// `(layer_id, chapter_id)`, so an id that is not in that band is a
/// refusal rather than a no-op.
async fn delete_chapter_mark(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteChapterMarkCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.material_layer_service
        .delete_chapter_mark(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// -----------------------------------------------------------------
// App-level Threads
// -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ListThreadsQuery {
    /// `"app_global"` / `"snapshot"` / `"query_group"` / `"card"`.
    anchor_kind: String,
    /// Anchor entity id (required for every kind except `app_global`).
    #[serde(default)]
    anchor_id: Option<String>,
    /// When `true`, archived Threads are included. Defaults to `false`.
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct ListMessagesQuery {
    /// Exclusive lower bound on `created_at_ms` — pass the greatest
    /// value the caller has seen to poll for new writes.
    #[serde(default)]
    since_ms: Option<i64>,
}

/// `GET /asterism/threads?anchor_kind=...&anchor_id=...` — Threads
/// under the given anchor, freshest first.
async fn list_threads_by_anchor(
    State(ctx): State<Arc<ServerCtx>>,
    Query(query): Query<ListThreadsQuery>,
) -> ApiResult<Vec<ThreadDto>> {
    Ok(Json(
        ctx.thread_service
            .list(
                &query.anchor_kind,
                query.anchor_id.as_deref(),
                query.include_archived,
            )
            .await?,
    ))
}

/// `POST /asterism/threads` — creates a Thread with a title + anchor.
async fn create_thread(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<CreateThreadCommand>,
) -> ApiResult<ThreadDto> {
    Ok(Json(
        ctx.thread_service
            .create(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/threads/archive` — toggles the archived flag.
async fn archive_thread(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<ArchiveThreadCommand>,
) -> ApiResult<ThreadDto> {
    Ok(Json(
        ctx.thread_service
            .archive(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/threads/delete` — hard delete (cascades to messages).
async fn delete_thread(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteThreadCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.thread_service
        .delete(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /asterism/threads/{id}` — one Thread by surrogate id (`null`
/// when absent).
async fn get_thread(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Option<ThreadDto>> {
    Ok(Json(ctx.thread_service.find(&id).await?))
}

/// `GET /asterism/threads/{id}/messages` — chronological list.
async fn list_thread_messages(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Query(query): Query<ListMessagesQuery>,
) -> ApiResult<Vec<MessageDto>> {
    Ok(Json(
        ctx.thread_service
            .list_messages(&id, query.since_ms)
            .await?,
    ))
}

/// `POST /asterism/threads/{id}/messages` — appends one Message.
/// UI (`author_kind="human"`) and Claude Code (`author_kind="claude_code"`)
/// share this endpoint. `thread_id` on the URL is authoritative; if the
/// body carries a mismatching id it is overwritten by the URL path so
/// the two never disagree.
async fn append_thread_message(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(mut command): Json<AppendMessageCommand>,
) -> ApiResult<MessageDto> {
    command.thread_id = id;
    // `command.author_kind` is the Message's own author (human /
    // claude_code / agent / persona) — a different question with a
    // different value domain from the asset-side `author_kind`, and not
    // an attribution assertion. It stays with the command; the context
    // says only that this write arrived over HTTP.
    Ok(Json(
        ctx.thread_service
            .append_message(command, &asserted(None, None, None)?)
            .await?,
    ))
}

/// `POST /asterism/messages/delete` — misfire correction (there is no
/// message edit verb; the UI overwrites by deleting + re-appending).
async fn delete_message(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<DeleteMessageCommand>,
) -> ApiResult<serde_json::Value> {
    ctx.thread_service
        .delete_message(command, &asserted(None, None, None)?)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ---------------------------------------------------------------
// The forge — a line
//
// The service takes typed ids and the wire carries strings, so the
// parsing is here. It is the adapter's job in both directions: a path
// segment that is not a UUID is malformed input and says so, rather
// than reaching a service that would have to describe a shape it does
// not accept.
// ---------------------------------------------------------------

/// Reads a line id out of a path segment.
fn line_id(raw: &str) -> Result<LineId, DomainError> {
    forge_line_id(raw, "line id")
}

/// `POST /asterism/forge/lines` — opens a line.
async fn open_forge_line(
    State(ctx): State<Arc<ServerCtx>>,
    Json(command): Json<OpenForgeLineCommand>,
) -> ApiResult<ForgeLineDto> {
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    let line = ctx
        .line_service
        .open(
            forge_name(command.name)?,
            forge_strategy_id(command.strategy_id)?,
            &attribution,
        )
        .await?;
    Ok(Json(forge_line_to_dto(&line)))
}

/// `GET /asterism/forge/lines` — every line, without its history.
async fn list_forge_lines(State(ctx): State<Arc<ServerCtx>>) -> ApiResult<Vec<ForgeLineDto>> {
    let lines = ctx.line_service.list().await?;
    Ok(Json(lines.iter().map(forge_line_to_dto).collect()))
}

/// `GET /asterism/forge/lines/{id}` — the line and its whole history.
///
/// This grows with the line. A surface showing what is *on* a line
/// wants `/states` instead; this one is for showing how it got there.
async fn get_forge_line(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<ForgeLineHistoryDto> {
    let line = ctx.line_service.get(&line_id(&id)?).await?;
    Ok(Json(forge_history_to_dto(&line)))
}

/// `GET /asterism/forge/lines/{id}/states` — what is on the line,
/// folded from the chain.
async fn get_forge_line_states(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<ForgeEntryStateDto>> {
    let states = ctx.line_service.states(&line_id(&id)?).await?;
    Ok(Json(forge_states_to_dto(&states)))
}

/// Reads a line back after a write, so the caller sees what it now is.
///
/// **A write verb on a line answers with the line, unless it has
/// something the line can no longer say.** `discard` is the one that
/// has: it answers with the assets the drop released, because after
/// that write there is no line left to read. The four that move a
/// line's description return nothing from the service — they are
/// `Result<(), _>` — and a caller told only "renamed" has to ask again
/// for the name, the standing and the stamp that moved. That second
/// request is the thing a screen would forget; `personas/archive`
/// answers with the persona for the same reason. The read costs one
/// more query and buys a surface where no write leaves the caller
/// holding a value it knows is stale.
async fn line_now(ctx: &ServerCtx, id: &LineId) -> Result<Json<ForgeLineDto>, ApiError> {
    Ok(Json(forge_line_to_dto(&ctx.line_service.get(id).await?)))
}

/// `POST /asterism/forge/lines/{id}/rename` — moves the line's own
/// description. Not a landing: nothing goes on the chain.
async fn rename_forge_line(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(command): Json<RenameForgeLineCommand>,
) -> ApiResult<ForgeLineDto> {
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    let id = line_id(&id)?;
    ctx.line_service
        .rename(&id, &forge_name(command.name)?, &attribution)
        .await?;
    line_now(&ctx, &id).await
}

/// `POST /asterism/forge/lines/{id}/strategy` — points the line at a
/// different rule, from here on.
async fn set_forge_line_strategy(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(command): Json<SetForgeLineStrategyCommand>,
) -> ApiResult<ForgeLineDto> {
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    let id = line_id(&id)?;
    ctx.line_service
        .set_strategy(&id, &forge_strategy_id(command.strategy_id)?, &attribution)
        .await?;
    line_now(&ctx, &id).await
}

/// `POST /asterism/forge/lines/{id}/archive` — finished with. Takes no
/// landing until it is reopened, and is the only state a drop can be
/// reached from.
async fn archive_forge_line(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(command): Json<ForgeLineActCommand>,
) -> ApiResult<ForgeLineDto> {
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    let id = line_id(&id)?;
    ctx.line_service.archive(&id, &attribution).await?;
    line_now(&ctx, &id).await
}

/// `POST /asterism/forge/lines/{id}/reopen` — takes it back out.
async fn reopen_forge_line(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(command): Json<ForgeLineActCommand>,
) -> ApiResult<ForgeLineDto> {
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    let id = line_id(&id)?;
    ctx.line_service.reopen(&id, &attribution).await?;
    line_now(&ctx, &id).await
}

/// `POST /asterism/forge/lines/{id}/discard` — takes the line, its
/// history and every piece of work against it.
///
/// **The response is the point.** It names the assets the forge was
/// holding and is not holding any more; after this write there is no
/// record left to derive them from, so a caller that ignores the body
/// has lost the only answer there will be.
async fn discard_forge_line(
    State(ctx): State<Arc<ServerCtx>>,
    Path(id): Path<String>,
    Json(command): Json<ForgeLineActCommand>,
) -> ApiResult<ForgeDiscardedDto> {
    let attribution = asserted(
        command.author_kind.as_deref(),
        command.author_subject.as_deref(),
        command.operator_ai.as_deref(),
    )?;
    let id = line_id(&id)?;
    let released = ctx.line_service.discard(&id, &attribution).await?;
    Ok(Json(forge_discarded_to_dto(id, &released)))
}

/// `GET /asterism/forge/strategies` — every rule a line can be pointed
/// at, built from the rules this deployment carries.
async fn list_forge_strategies(
    State(ctx): State<Arc<ServerCtx>>,
) -> ApiResult<Vec<ForgeStrategyDto>> {
    let rules = ctx.line_service.strategies().await;
    Ok(Json(
        rules
            .iter()
            .map(|(id, about)| forge_strategy_to_dto(id, about))
            .collect(),
    ))
}
