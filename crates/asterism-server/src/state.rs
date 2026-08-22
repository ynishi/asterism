//! Backend context for the standalone server. Thin wrapper over the
//! shared [`crate::core_init::init_core`] (`ReadOnly` mode): assembles a
//! [`ServerCtx`] from the returned `CoreCtx`.
//!
//! The server shares the SQLite file with the Tauri UI process under
//! WAL; the `busy_timeout = 5000` pragma is applied by `sqlite::open`.
//! Progress updates go to stderr via `LogEmitter` — there is no UI event
//! bus in this process.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use asterism_core::application::DispatchService;
use asterism_core::application::{
    AppSettingService, AssetCommentService, AssetService, MaterialLayerService,
    MaterialMarkService, ModalityService, PersonaService, QueryGroupService, SeriesStrategyService,
    SessionService, SnapshotService, ThreadService, ThumbService,
};
use asterism_infra::dispatch::ExporterRegistry;

use crate::core_init::{CoreMode, LogEmitter, init_core};

/// Bundle of services that HTTP handlers share via `axum` state.
///
/// Note: no isle handle here. The precomputed session snapshot is
/// rebuilt through `AssetService::rebuild_sessions` (which enqueues
/// a `SessionRebuild` job); the store's SQLite handle lives inside
/// the store itself.
pub struct ServerCtx {
    /// Persona use case.
    pub persona_service: Arc<PersonaService>,
    /// Asset use case.
    pub asset_service: Arc<AssetService>,
    /// Thumbnail cache use case (grid rendering).
    pub thumb_service: Arc<ThumbService>,
    /// Immutable content-addressed snapshots (seeds for outbound
    /// dispatch).
    pub snapshot_service: Arc<SnapshotService>,
    /// Outbound dispatch lifecycle (creates jobs, reifies derived
    /// output, exposes list/detail).
    pub dispatch_service: Arc<DispatchService>,
    /// Query Group lifecycle (create / update-rule commands).
    pub query_group_service: Arc<QueryGroupService>,
    /// Modality master lifecycle (list / create / update / delete).
    pub modality_service: Arc<ModalityService>,
    /// Series Strategy lifecycle (list / create / update / delete) —
    /// the door a rule crosses into this process through.
    pub series_strategy_service: Arc<SeriesStrategyService>,
    /// Application settings (default → env var → stored row).
    pub app_setting_service: Arc<AppSettingService>,
    /// Session 1st-class entity lifecycle. Backs the P2 HTTP
    /// CRUD (rename / metadata / delete) once those routes land; the
    /// SessionsView list path currently continues to flow through
    /// `AssetService::list_sessions`.
    pub session_service: Arc<SessionService>,
    /// Comment thread on an Asset — same rows the UI's four Tauri
    /// commands write.
    pub asset_comment_service: Arc<AssetCommentService>,
    /// Marks placed into an Asset's material — a position inside the
    /// content rather than a note on the asset row. Same rows the
    /// UI's four Tauri commands write.
    pub material_mark_service: Arc<MaterialMarkService>,
    /// The bands of marks over an Asset's material — which reading of
    /// the content a surface shows, which one a person may edit, and
    /// the chapters inside a structure band. Same rows the UI's eight
    /// Tauri commands write.
    pub material_layer_service: Arc<MaterialLayerService>,
    /// App-level Threads container — receives both UI (human) and
    /// HTTP (Claude Code / agents) writes on the same rows.
    pub thread_service: Arc<ThreadService>,
    /// A line of work in the forge: opening one, reading what is on
    /// it, and its lifecycle. Named as `CoreCtx` names it — the forge's
    /// conversations are `forge_thread_service` there, because
    /// `thread_service` above was taken first by the annotation
    /// surface on the raw layer.
    pub line_service: Arc<asterism_core::application::forge::LineService>,
    /// Registered exporters — surfaces which backends the server can
    /// dispatch to.
    pub exporter_registry: ExporterRegistry,
    /// Read-only handle to the apalis job DB pool. Only used by
    /// the `/asterism/jobs/stats` endpoint so far; keeping it on
    /// the ctx lets the HTTP handler count jobs without wiring a
    /// separate connection stack.
    pub jobs_pool: asterism_infra::jobs::SqlitePool,
    /// Local telemetry append/read handle (`event_log`) — serves
    /// `/asterism/events` so an agent can record and aggregate usage.
    pub telemetry: asterism_infra::telemetry::Telemetry,
    /// Read handle over the four observation streams — serves
    /// `/asterism/diag`, `/asterism/perf`, `/asterism/jobs/log` and
    /// `/asterism/observations`, the only way to read what the
    /// application recorded about itself without a `sqlite3` session.
    pub observations: asterism_infra::observe::ObservationStore,
}

/// Default DB path: active local data profile (override via
/// `$ASTERISM_HOME`). Shared with `asterism-ui` through
/// `asterism_infra::paths`.
pub fn default_db_path() -> anyhow::Result<PathBuf> {
    Ok(asterism_infra::paths::default_db_path()?)
}

impl ServerCtx {
    /// Selects the HTTP-facing services out of an assembled [`CoreCtx`].
    ///
    /// The single place the field list is written. The Tauri process
    /// builds its own `ServerCtx` from the same core (it serves the
    /// loopback router itself), and the route tests build one over a
    /// tempdir core — routing all three through here means a service
    /// added to `CoreCtx` cannot reach one caller and miss another.
    pub fn from_core(core: &crate::core_init::CoreCtx) -> Arc<Self> {
        Arc::new(Self {
            persona_service: core.persona_service.clone(),
            asset_service: core.asset_service.clone(),
            thumb_service: core.thumb_service.clone(),
            snapshot_service: core.snapshot_service.clone(),
            dispatch_service: core.dispatch_service.clone(),
            query_group_service: core.query_group_service.clone(),
            modality_service: core.modality_service.clone(),
            series_strategy_service: core.series_strategy_service.clone(),
            app_setting_service: core.app_setting_service.clone(),
            session_service: core.session_service.clone(),
            asset_comment_service: core.asset_comment_service.clone(),
            material_mark_service: core.material_mark_service.clone(),
            material_layer_service: core.material_layer_service.clone(),
            thread_service: core.thread_service.clone(),
            line_service: core.line_service.clone(),
            exporter_registry: core.exporter_registry.clone(),
            jobs_pool: core.jobs_pool.clone(),
            telemetry: core.telemetry.clone(),
            observations: core.observations.clone(),
        })
    }
}

/// Initialises the backend in read-only mode and returns the shared
/// context. The tantivy writer lock and the job worker stay with the
/// Tauri UI process; the server only enqueues jobs and serves reads.
pub async fn init(db_path: &Path) -> anyhow::Result<Arc<ServerCtx>> {
    let core = init_core(db_path, Arc::new(LogEmitter), CoreMode::ReadOnly).await?;
    Ok(ServerCtx::from_core(&core))
}
