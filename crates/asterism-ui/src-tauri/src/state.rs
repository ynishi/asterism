//! `AppState` — service DI + backend initialisation.
//!
//! Tauri's own `State` container gives us `Arc`-style sharing, so this
//! struct just holds `Arc<Service>` fields and does not wrap again. The
//! heavy lifting is delegated to the shared `asterism_server::core_init`
//! (`Full` mode: read-write tantivy index + job worker); this module only
//! adapts the returned `CoreCtx` into `AppState` and supplies the Tauri
//! progress emitter.

use std::sync::Arc;

use asterism_core::DomainError;
use asterism_core::application::DispatchService;
use asterism_core::application::{
    AppSettingService, AssetCommentService, AssetService, MaterialLayerService,
    MaterialMarkService, ModalityService, PersonaService, QueryGroupService, SeriesStrategyService,
    SessionService, SnapshotService, ThreadService, ThumbService,
};
use asterism_core::domain::repository::ProgressEmitter;
use asterism_core::domain::value::Progress;
use asterism_infra::dispatch::ExporterRegistry;
use asterism_server::core_init::{CoreMode, init_core};
use asterism_server::state::ServerCtx;
use async_trait::async_trait;
use tauri::{AppHandle, Emitter};

/// A live team session, and the pair that names it.
///
/// The client alone cannot answer "which connection is this" the way
/// the stored state asks it: `stored_connection` keys an entry by
/// server **and login**, and a session knows the server it was built
/// with and the `user_id` the server answered with — not the login as
/// somebody typed it. So the pair travels beside the session, set once
/// where the connection is made and read by every verb that has to
/// decide whether what this machine remembers is what it is holding.
///
/// That rule, and what goes wrong without it, is on
/// [`stored_connection`](crate::stored_connection)'s "which connection
/// a verb acts on".
#[derive(Clone)]
pub struct TeamsConnection {
    /// The session itself — what every read of a shared line goes
    /// down.
    pub client: asterism_teams_client::TeamsClient,
    /// The server as it was typed, before any trailing slash came off.
    pub base_url: String,
    /// The account it was typed for — or, for a sign-in through the
    /// provider, the login the session said (#163). Not the `user_id`:
    /// the keychain entry is keyed by the login, and that is what has
    /// to match.
    pub login: String,
    /// The instance's stable id, as the session said it (#163) — or
    /// empty, from a server too old to say one, which every reader
    /// turns into `None` before comparing. Why the keychain entry is
    /// keyed by it is `stored_connection`'s "what an entry is keyed
    /// by".
    pub instance_id: String,
}

/// Bundle of services registered as Tauri state.
///
/// Note: no isle handle here. The precomputed session snapshot is
/// rebuilt through `AssetService::rebuild_sessions` (which enqueues
/// a `SessionRebuild` job); the store's SQLite handle lives inside
/// the store itself.
pub struct AppState {
    /// Persona use case.
    pub persona_service: Arc<PersonaService>,
    /// Asset use case.
    pub asset_service: Arc<AssetService>,
    /// Thumbnail cache use case (image grid rendering).
    pub thumb_service: Arc<ThumbService>,
    /// Immutable content-addressed snapshot lifecycle (seeds outbound
    /// dispatch).
    pub snapshot_service: Arc<SnapshotService>,
    /// Outbound dispatch lifecycle (create → apalis → reify → new Asset
    /// via each registered `Exporter`).
    pub dispatch_service: Arc<DispatchService>,
    /// Query Group lifecycle (create / update-rule commands).
    pub query_group_service: Arc<QueryGroupService>,
    /// Modality master lifecycle (list / create / update / delete) —
    /// the backend-authoritative row set that drives the sidebar
    /// modality axis + the `slug → kind` resolution.
    pub modality_service: Arc<ModalityService>,
    /// Application settings (default → env var → stored row). Shared
    /// with the loopback HTTP surface so a headless core and the window
    /// resolve the same preference.
    pub app_setting_service: Arc<AppSettingService>,
    /// Session 1st-class entity lifecycle — exposed for the upcoming
    /// rename / metadata / delete Tauri commands. Currently not
    /// wired to any command; kept on the state so those do not have to
    /// thread another DI seam through the setup hook.
    pub session_service: Arc<SessionService>,
    /// Registered exporters — surfaces which backends the UI can
    /// dispatch to (rendered as the action-bar target menu).
    pub exporter_registry: ExporterRegistry,
    /// Asset comment thread lifecycle (User + Persona posts on a
    /// single Asset).
    pub asset_comment_service: Arc<AssetCommentService>,
    /// Marks placed into an Asset's material — the coordinate space its
    /// content carries (today a point or interval on the playback
    /// timeline), as opposed to the thread on the Asset as a whole.
    pub material_mark_service: Arc<MaterialMarkService>,
    /// The bands those marks sit in — which reading of the material a
    /// panel shows, which one the person may edit, and the chapters
    /// inside a structure band.
    pub material_layer_service: Arc<MaterialLayerService>,
    /// The marks behind their own port — what a publication joins to
    /// their bands to decide which of them a person wrote. See
    /// `CoreCtx::material_marks` for why the service above will not
    /// answer that, and why the bands need no equivalent.
    pub material_marks: Arc<dyn asterism_core::domain::repository::MaterialMarkRepository>,
    /// The asset rows behind their port — the two questions a clone
    /// asks that `AssetService` has no method for. See
    /// `CoreCtx::assets`.
    pub assets: Arc<dyn asterism_core::domain::repository::AssetRepository>,
    /// The relation a promotion leaves at home, which publishing a line
    /// writes into. See `CoreCtx::asset_links`.
    pub asset_links: Arc<dyn asterism_core::domain::repository::AssetLinkRepository>,
    /// The team server this window is talking to, when it is talking to
    /// one.
    ///
    /// A shared line is served through rather than mirrored (#148
    /// decision 16), so this is not a cache — it is the connection every
    /// read of one goes down, and when it is `None` the shared panel has
    /// nothing to show rather than something stale.
    ///
    /// Held in a lock because logging in mutates the client, and behind
    /// an `Option` because the desktop starts with no team and may end
    /// with none.
    ///
    /// **The session here is still the window's and dies with it.**
    /// What #204 added is not a longer-lived session but a way to open
    /// another one without a password: the OS keychain may hold a
    /// device token, and the profile home may hold the server URL and
    /// login that name it. Neither is a session and neither is a
    /// password — the invariant, and which store holds which half, is
    /// on [`stored_connection`](crate::stored_connection).
    ///
    /// So a window that reconnects silently arrives here exactly as one
    /// that was typed into does, and nothing below this field can tell
    /// which happened. This was written before that existed, when the
    /// sentence was that nothing was stored anywhere; the reason it
    /// said so — that a credential had no designed home — is what #204
    /// answered rather than what it worked around.
    pub teams: Arc<tokio::sync::Mutex<Option<TeamsConnection>>>,
    /// App-level Threads container — UI writes flow through this
    /// service; the HTTP surface (Claude Code / agents) writes to
    /// the same rows via `ServerCtx::thread_service`, since both
    /// contexts share one `Arc<ThreadService>`.
    pub thread_service: Arc<ThreadService>,
    /// A line of work in the forge: opening one, reading what is on it,
    /// and its lifecycle.
    pub line_service: Arc<asterism_core::application::forge::LineService>,
    /// Work against a line: opening it, writing rounds, letting the
    /// line's rule answer what it collides with, and ending it.
    pub pursuit_service: Arc<asterism_core::application::forge::PursuitService>,
    /// What was said about work. Named apart from
    /// [`thread_service`](Self::thread_service) for the reason `CoreCtx`
    /// names it apart: the forge's conversations hang off a forge node,
    /// and the other ones hang off a card.
    pub forge_thread_service: Arc<asterism_core::application::forge::ThreadService>,
    /// Read-only handle to the apalis job DB pool. Used by the
    /// `jobs_stats` Tauri command that drives the progress banner.
    pub jobs_pool: asterism_infra::jobs::SqlitePool,
    /// Local telemetry append/read handle (`event_log` — dogfooding
    /// metrics recorded by the UI, read back for summaries).
    pub telemetry: asterism_infra::telemetry::Telemetry,
    /// Series Strategy lifecycle — the registered derivation rules,
    /// reached by the four `*_series_strategy` commands.
    pub series_strategy_service: Arc<SeriesStrategyService>,
    /// Read handle over the observation streams (`observation` view) —
    /// the single-timeline listing `list_observations` serves.
    pub observations: asterism_infra::observe::ObservationStore,
}

/// Tauri implementation of `ProgressEmitter` — forwards each payload to
/// the `job:progress:{task_id}` event channel.
struct TauriEmitter {
    app: AppHandle,
}

#[async_trait]
impl ProgressEmitter for TauriEmitter {
    async fn emit(&self, job_id: &str, progress: Progress) -> Result<(), DomainError> {
        self.app
            .emit(
                &format!("job:progress:{job_id}"),
                serde_json::json!({
                    "current": progress.current,
                    "total": progress.total,
                    "message": progress.message,
                }),
            )
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("tauri emit failed: {e}")))
    }

    async fn broadcast(&self, event: &str, payload: serde_json::Value) -> Result<(), DomainError> {
        self.app
            .emit(event, payload)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("tauri broadcast failed: {e}")))
    }
}

/// Initialises the whole backend and returns both the Tauri `AppState`
/// and the HTTP [`ServerCtx`]. Invoked from the Tauri setup hook. Opens
/// the shared core in `Full` mode (this process is the single tantivy
/// writer and runs the job worker).
///
/// Both context structs are built from the same `CoreCtx` so the UI and
/// the loopback HTTP surface it serves share one service graph and one
/// job worker. Neither wrapper constructs a service of its own: one
/// built here rather than in `init_core` would be reachable from this
/// transport only, which is how the comment thread spent four commands
/// with no HTTP route. All fields are `Arc`/`Clone`, so the bundle is
/// assembled by cloning the handles before `AppState` takes ownership of
/// the originals.
pub async fn init(app: AppHandle) -> anyhow::Result<(AppState, Arc<ServerCtx>)> {
    // Data lives under the active isolated profile (or an explicit
    // `$ASTERISM_HOME`). Using `asterism_infra::paths` means the UI and
    // standalone server resolve to the same file on disk.
    let db_path = asterism_infra::paths::default_db_path()?;
    let emitter: Arc<dyn ProgressEmitter> = Arc::new(TauriEmitter { app });
    let core = init_core(&db_path, emitter, CoreMode::Full).await?;

    // HTTP context — a subset of the same core, wired into `axum` state
    // by `asterism_server::http::router`. Selected by `ServerCtx` itself
    // so this process cannot end up serving a different set of services
    // than the standalone server does.
    let server_ctx = ServerCtx::from_core(&core);

    let app_state = AppState {
        persona_service: core.persona_service,
        asset_service: core.asset_service,
        thumb_service: core.thumb_service,
        snapshot_service: core.snapshot_service,
        dispatch_service: core.dispatch_service,
        query_group_service: core.query_group_service,
        modality_service: core.modality_service,
        app_setting_service: core.app_setting_service,
        session_service: core.session_service,
        exporter_registry: core.exporter_registry,
        asset_comment_service: core.asset_comment_service,
        material_mark_service: core.material_mark_service,
        material_layer_service: core.material_layer_service,
        material_marks: core.material_marks,
        assets: core.assets,
        asset_links: core.asset_links,
        teams: Arc::new(tokio::sync::Mutex::new(None)),
        thread_service: core.thread_service,
        line_service: core.line_service,
        pursuit_service: core.pursuit_service,
        forge_thread_service: core.forge_thread_service,
        jobs_pool: core.jobs_pool,
        telemetry: core.telemetry,
        series_strategy_service: core.series_strategy_service,
        observations: core.observations,
    };

    Ok((app_state, server_ctx))
}
