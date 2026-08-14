//! Shared backend initialisation for both the Tauri UI and the standalone
//! server.
//!
//! The two processes assemble the exact same service graph. The only
//! differences are (1) the progress emitter (`TauriEmitter` in the UI, the
//! stderr [`LogEmitter`] in the server), (2) whether the Tantivy index is
//! opened read-write or read-only, and (3) whether a job-worker `Monitor`
//! is spawned. Those three axes are captured by [`CoreMode`]; everything
//! else lives here so the ~160 lines of DI wiring are written once.
//!
//! Callers wrap the returned [`CoreCtx`] into their own context struct
//! (`ServerCtx` / `AppState`) and add nothing to it. A service assembled
//! in one wrapper instead of here would be reachable from that
//! transport alone, which is how the Asset comment thread ended up with
//! four Tauri commands and no HTTP route.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use asterism_core::DomainError;
use asterism_core::application::provenance_service::ProvenanceService;
use asterism_core::application::query_group_invalidation::QueryGroupInvalidator;
use asterism_core::application::{
    AppSettingService, AssetCommentService, AssetService, DispatchService, MaterialLayerService,
    MaterialMarkService, ModalityService, PersonaService, QueryGroupService, SeriesStrategyService,
    SessionService, SnapshotService, ThreadService, ThumbService,
};
use asterism_core::application_support::{
    DispatchRunnerService, QueryGroupRefreshService, RetentionService, SupportServices,
};
use asterism_core::domain::disclosure::PromptDisclosure;
use asterism_core::domain::repository::ProgressEmitter;
use asterism_core::domain::value::Progress;
use asterism_dispatch_sdk::Exporter;
use asterism_exporter_comfy::ComfyHttpExporter;
use asterism_exporter_file::FileExporter;
use asterism_exporter_http::HttpExporter;
use asterism_infra::dispatch::{DispatchRunEnv, ExporterRegistry, QueueReEnqueue, ReEnqueue};
use asterism_infra::jobs::{self, JobDeps};
// Named for what it is on this side of the boundary: the core's port is
// also called `ProvenanceWriter`, and both are in scope here.
use asterism_infra::provenance::ProvenanceWriter as InfraProvenanceWriter;
use asterism_infra::search::TantivyIndex;
use asterism_infra::source_text::FsSourceTextReader;
use asterism_infra::sqlite;
use async_trait::async_trait;

/// Selects how the shared core is opened for the calling process.
pub enum CoreMode {
    /// Full read-write process: opens the Tantivy index read-write
    /// (acquiring the exclusive writer lock) and spawns the job-worker
    /// `Monitor`. Used by the Tauri UI, the single writer.
    Full,
    /// Read-only / enqueue-only process: opens the Tantivy index without
    /// the writer lock and opens the job queue without spawning a worker.
    /// Used by the standalone server, which shares the DB with a `Full`
    /// UI process that drains the queue and holds the writer lock.
    ReadOnly,
}

/// Default [`ProgressEmitter`] for processes without a UI event bus (the
/// standalone server). Logs each progress payload to stderr; `broadcast`
/// falls back to the trait's no-op default.
pub struct LogEmitter;

#[async_trait]
impl ProgressEmitter for LogEmitter {
    async fn emit(&self, job_id: &str, progress: Progress) -> Result<(), DomainError> {
        let total = progress
            .total
            .map(|t| t.to_string())
            .unwrap_or_else(|| "?".to_string());
        eprintln!(
            "job {job_id}: {}/{total} {}",
            progress.current,
            progress.message.as_deref().unwrap_or("")
        );
        Ok(())
    }
}

/// How long a trashed asset / Group survives before the retention sweep
/// may purge it.
///
/// Lives here — at the composition root — rather than as a constant
/// inside the service, so the value can be swapped for configuration
/// (environment → config → this call site) without the domain or the
/// application layer learning about it. Two weeks is the interval the
/// Finder / Photos trash conditions users to expect, and the cost of
/// being generous is bounded: a trashed asset occupies one row plus the
/// dependent rows it already had.
///
/// Overridable through `ASTERISM_TRASH_RETENTION_DAYS`, matching the
/// `ASTERISM_HOME` / `ASTERISM_PROFILE` convention the rest of the app
/// already uses. There is no config *file* layer to route this through
/// yet, and inventing one for a single duration would let that one
/// duration dictate its shape — the env var is the whole mechanism the
/// value needs today, and `AssetService` takes the period as an
/// argument, so a future config layer only has to change this function.
///
/// Two weeks by default: the interval Finder / Photos condition users to
/// expect. The cost of being generous is bounded — a trashed asset is
/// one row plus the dependent rows it already had.
///
/// A malformed or non-positive value is refused rather than silently
/// replaced. This number decides when data stops being recoverable, so
/// "it quietly meant 14 days" and "it quietly meant purge on sight" are
/// both worse than failing to start.
fn trash_retention() -> Result<chrono::Duration, DomainError> {
    match std::env::var(TRASH_RETENTION_ENV) {
        Ok(raw) => parse_trash_retention(Some(&raw)),
        Err(std::env::VarError::NotPresent) => parse_trash_retention(None),
        Err(err) => Err(DomainError::Infra(anyhow::anyhow!(
            "cannot read {TRASH_RETENTION_ENV}: {err}"
        ))),
    }
}

const TRASH_RETENTION_ENV: &str = "ASTERISM_TRASH_RETENTION_DAYS";
const TRASH_RETENTION_DEFAULT_DAYS: i64 = 14;

/// Pure half of [`trash_retention`], split out so the rules can be
/// tested without mutating process-global environment state.
fn parse_trash_retention(raw: Option<&str>) -> Result<chrono::Duration, DomainError> {
    let Some(raw) = raw else {
        return Ok(chrono::Duration::days(TRASH_RETENTION_DEFAULT_DAYS));
    };
    let days: i64 = raw.trim().parse().map_err(|_| {
        DomainError::Infra(anyhow::anyhow!(
            "invalid {TRASH_RETENTION_ENV}={raw:?}; \
             expected a positive whole number of days"
        ))
    })?;
    if days <= 0 {
        return Err(DomainError::Infra(anyhow::anyhow!(
            "invalid {TRASH_RETENTION_ENV}={days}; retention must be at least 1 day \
             (0 would purge trashed items on the next sweep)"
        )));
    }
    Ok(chrono::Duration::days(days))
}

/// Shared service graph assembled by [`init_core`].
///
/// Both the Tauri UI (`AppState`) and the standalone server (`ServerCtx`)
/// wrap this bundle. Every service is assembled here rather than in
/// either wrapper: a service built on one side only is reachable from
/// one transport only, which is how the comment thread ended up with
/// four Tauri commands and no HTTP route.
pub struct CoreCtx {
    /// Persona use case.
    pub persona_service: Arc<PersonaService>,
    /// Asset use case.
    pub asset_service: Arc<AssetService>,
    /// Thumbnail cache use case (grid rendering).
    pub thumb_service: Arc<ThumbService>,
    /// Immutable content-addressed snapshot lifecycle (seeds outbound
    /// dispatch).
    pub snapshot_service: Arc<SnapshotService>,
    /// Outbound dispatch lifecycle.
    pub dispatch_service: Arc<DispatchService>,
    /// Query Group evaluate-and-materialize pipeline: startup refresh,
    /// the create / update-rule commands, and (W4) the refresh job.
    pub query_group_service: Arc<QueryGroupService>,
    /// Modality master lifecycle (list / create / update / delete of
    /// the `modality` table).
    pub modality_service: Arc<ModalityService>,
    /// Series Strategy lifecycle — the rules the series axis derives
    /// keys under, registered from outside this process as data (see
    /// [`SeriesStrategyService`]'s module doc for why it has to be
    /// data).
    pub series_strategy_service: Arc<SeriesStrategyService>,
    /// Application settings, resolved as code default → stored
    /// `app_setting` row → environment variable.
    pub app_setting_service: Arc<AppSettingService>,
    /// Session 1st-class entity lifecycle — SessionsView
    /// list source in P1b, HTTP CRUD backend in P2, importer
    /// find-or-create in P3.
    pub session_service: Arc<SessionService>,
    /// Comment thread on an Asset (list / post / edit / delete).
    pub asset_comment_service: Arc<AssetCommentService>,
    /// Marks placed into an Asset's material — a position inside the
    /// content (today a point or interval on its playback timeline),
    /// as opposed to the comment thread on the Asset as a whole.
    pub material_mark_service: Arc<MaterialMarkService>,
    /// The bands of marks over an Asset's material — which reading of
    /// the content a surface shows, which one a person may edit, and
    /// the chapters inside a structure band. Selected by both
    /// `ServerCtx::from_core` (HTTP + MCP) and `AppState` (Tauri IPC),
    /// so all three doors reach the same rows.
    pub material_layer_service: Arc<MaterialLayerService>,
    /// App-level Threads container (both UI and HTTP writes land on
    /// the same rows through this service).
    pub thread_service: Arc<ThreadService>,
    /// Registered exporters (`comfy` / `file` / `http`).
    pub exporter_registry: ExporterRegistry,
    /// Read-only handle to the apalis job DB pool (job-stats endpoints).
    pub jobs_pool: asterism_infra::jobs::SqlitePool,
    /// Local telemetry append/read handle (`event_log`).
    pub telemetry: asterism_infra::telemetry::Telemetry,
    /// Read + expiry handle over the four observation streams. Cannot
    /// append: records arrive through the `tracing` subscriber and
    /// nowhere else.
    pub observations: asterism_infra::observe::ObservationStore,
    /// Services nothing on the wire fronts — the retention sweep, the
    /// bulk Query Group refresh, and the runner-side dispatch
    /// transitions (`asterism_core::application_support`).
    ///
    /// Assembled here with everything else, then handed to the job
    /// worker and the dispatch runner. `ServerCtx` / `AppState`
    /// deliberately have **no** field for it: a Tauri command or an
    /// HTTP handler holds no object these methods can be called on, so
    /// the "worker-driven only" property is checked by the compiler
    /// rather than asserted in a doc comment.
    pub support: SupportServices,
}

/// Initialises the shared backend and returns the assembled [`CoreCtx`].
///
/// `db_path` is the SQLite file shared by the UI and the server; `emitter`
/// is the process-specific progress sink; `mode` selects the read-write
/// ([`CoreMode::Full`]) versus read-only / enqueue-only
/// ([`CoreMode::ReadOnly`]) shape. The startup drift check (rebuild the
/// Session snapshot when stale) runs at the end in both modes — in
/// `ReadOnly` the resulting `SessionRebuild` no-op still enqueues
/// but the `Full` worker consumes it.
pub async fn init_core(
    db_path: &Path,
    emitter: Arc<dyn ProgressEmitter>,
    mode: CoreMode,
) -> anyhow::Result<CoreCtx> {
    init_core_with(db_path, emitter, mode, None).await
}

/// [`init_core`] with an explicit override for the Tantivy index dir.
///
/// The production entry point resolves the index location from the
/// active profile via [`asterism_infra::paths::tantivy_index_dir`],
/// which is exactly what the running app should do. Tests are the
/// other caller: they hand in a tempdir path so an `init_core` inside
/// a test binary does not open — let alone write into — the User's
/// profile-scoped index (which is the DB the running app is using).
///
/// Kept as an override rather than a required argument so every
/// production call site stays a one-liner and the profile lookup
/// remains the single source of truth for "where does the app's index
/// live"; `Some(path)` is the escape hatch, not the norm.
pub async fn init_core_with(
    db_path: &Path,
    emitter: Arc<dyn ProgressEmitter>,
    mode: CoreMode,
    tantivy_index_dir: Option<&Path>,
) -> anyhow::Result<CoreCtx> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Video preview renditions live beside the database so a test
    // that sandboxes the db sandboxes them too. Stale `.part` staging
    // files are a crash's leftovers — swept here so the status
    // endpoint's "a `.part` means a transcode is running" reading
    // cannot stick at pending forever.
    let previews_dir = db_path
        .parent()
        .map(|p| p.join("previews"))
        .unwrap_or_else(|| PathBuf::from("previews"));
    std::fs::create_dir_all(&previews_dir)?;
    if let Ok(entries) = std::fs::read_dir(&previews_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "part") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    // Writer isle. `open_and_migrate` also applies any pending schema
    // migrations so the first launch on a fresh install still works.
    // Keep the driver alive for the whole process (graceful shutdown is a
    // future addition).
    let (isle, driver) = sqlite::open_and_migrate(db_path).await?;
    std::mem::forget(driver);
    // The subscriber has been running since `main`; give it somewhere
    // durable to put records now that the database exists. Anything
    // logged during startup (including a failure in the lines above)
    // was queued and lands here.
    asterism_infra::observe::attach(isle.clone());

    let personas = Arc::new(sqlite::repo::SqlitePersonaRepository::new(isle.clone()));
    let persona_themes = Arc::new(sqlite::repo::SqlitePersonaThemeRepository::new(
        isle.clone(),
    ));
    let persona_profiles = Arc::new(sqlite::repo::SqlitePersonaProfileRepository::new(
        isle.clone(),
    ));
    let assets = sqlite::repo::SqliteAssetRepository::new(isle.clone());
    // Session 1st-class entity (P1a) — separate writer-isle handle so
    // the SessionService can be exposed alongside AssetService
    // without threading a shared repo wrapper through every DI seam.
    let sessions_repo = Arc::new(sqlite::repo::SqliteSessionRepository::new(isle.clone()));
    let tags = sqlite::repo::SqliteTagRepository::new(isle.clone());
    let groups = sqlite::repo::group::SqliteGroupRepository::new(isle.clone());
    let dirs = sqlite::repo::SqliteDirRepository::new(isle.clone());
    let edges = sqlite::repo::SqliteEdgeRepository::new(isle.clone());
    let thumbs = sqlite::repo::SqliteThumbRepository::new(isle.clone());
    let modalities = sqlite::repo::SqliteModalityRepository::new(isle.clone());
    let app_settings = Arc::new(sqlite::repo::SqliteAppSettingRepository::new(isle.clone()));
    let app_setting_service = Arc::new(AppSettingService::new(app_settings));
    let asset_bodies = sqlite::repo::SqliteAssetBodyRepository::new(isle.clone());
    let snapshots = Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone()));
    let telemetry = asterism_infra::telemetry::Telemetry::new(isle.clone());
    let observations = asterism_infra::observe::ObservationStore::new(isle.clone());
    let dispatches = Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone()));
    let pursuits = Arc::new(sqlite::repo::SqlitePursuitRepository::new(isle.clone()));
    let query_groups = Arc::new(sqlite::repo::query_group::SqliteQueryGroupRepository::new(
        isle.clone(),
    ));
    let asset_comments = Arc::new(sqlite::repo::SqliteAssetCommentRepository::new(
        isle.clone(),
    ));
    let material_marks = Arc::new(sqlite::repo::SqliteMaterialMarkRepository::new(
        isle.clone(),
    ));
    // The bands those marks belong to. Held by two services: the layer
    // service, which is what a person opens and chooses between, and
    // the mark service, which resolves the default band a post lands in
    // without the caller naming one. Both over the same isle, which is
    // what makes "the band this post went into" and "the bands this
    // asset has" the same answer.
    let material_layers = Arc::new(sqlite::repo::SqliteMaterialLayerRepository::new(
        isle.clone(),
    ));
    let chapter_marks = Arc::new(sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()));
    // Series axis. Two readers now: the job engine derives keys through
    // `JobDeps`, and `SeriesStrategyService` registers and edits the
    // rules it derives them under. Both hold the same adapter over the
    // same isle, which is what makes an edit and the walk it enqueues
    // agree about which rules exist.
    let series = sqlite::repo::SqliteSeriesRepository::new(isle.clone());
    let threads = Arc::new(sqlite::repo::SqliteThreadRepository::new(isle.clone()));
    let source_texts = Arc::new(FsSourceTextReader::new());

    // Tantivy on-disk index. Production resolves the location from the
    // active profile so the running app, the CLI, and the headless
    // server all agree on one directory. A caller may hand in a
    // tempdir instead (tests do this — see `init_core_with`) so a
    // spun-up core does not collide with the profile-global index the
    // running app has opened. `Full` takes the exclusive writer lock;
    // `ReadOnly` skips it (the read path is identical either way).
    let tantivy_dir = match tantivy_index_dir {
        Some(explicit) => {
            std::fs::create_dir_all(explicit)?;
            explicit.to_path_buf()
        }
        None => asterism_infra::paths::tantivy_index_dir()?,
    };
    let search_index = Arc::new(
        match mode {
            CoreMode::Full => TantivyIndex::open(tantivy_dir),
            CoreMode::ReadOnly => TantivyIndex::open_read_only(tantivy_dir),
        }
        .map_err(|e| anyhow::anyhow!("open tantivy index: {e}"))?,
    );
    // An asset's body feeds two indexes with two different jobs: the
    // SQL trigram index answers the Query-side `text_match` predicate
    // (an exact set), Tantivy answers Retrieval (a ranked shortlist).
    // Fanned out behind one `AssetIndexer` so every path that already
    // maintains an index maintains both — see `search::fan_out`.
    let indexer: Arc<dyn asterism_core::domain::repository::AssetIndexer> =
        Arc::new(asterism_infra::search::FanOutIndexer::new(vec![
            Arc::new(sqlite::repo::SqliteAssetTextIndex::new(isle.clone())),
            search_index.clone(),
        ]));

    // Arc-wrap the repositories that both the job engine and the
    // application services need. Ordering: services that both the
    // JobDeps and the invalidator depend on must exist before
    // `jobs::start` — the `query_group_refresh` handler needs the
    // service to be reachable from JobDeps, and the invalidator
    // wraps the queue returned by `jobs::start`, so the service is
    // built first (its own deps are independent of the queue).
    let assets_arc: Arc<sqlite::repo::SqliteAssetRepository> = Arc::new(assets.clone());
    let edges_arc: Arc<sqlite::repo::SqliteEdgeRepository> = Arc::new(edges.clone());
    let tags_arc: Arc<sqlite::repo::SqliteTagRepository> = Arc::new(tags.clone());
    let groups_arc = Arc::new(groups.clone());
    let dirs_arc = Arc::new(dirs.clone());

    // Query Group evaluate-and-materialize service — needed by:
    //  * DispatchService (dispatch-time refresh),
    //  * the create / update-rule commands (both transports),
    //  * the refresh sweep below, which drives it per group.
    let query_group_service = Arc::new(QueryGroupService::new(
        query_groups.clone(),
        personas.clone(),
        assets_arc.clone(),
        groups_arc.clone(),
    ));

    // Support services (`application_support`) — driven by the job
    // worker, the dispatch runner, or startup, and by no transport.
    // Assembled here like everything else; what differs is where they
    // end up: the JobDeps / DispatchRunEnv seams below and the
    // `CoreCtx.support` field, never `ServerCtx` / `AppState`.
    //
    // The bulk refresh wraps the evaluator above rather than owning
    // it: sweeping every group is the part nothing on the wire asks
    // for, while evaluating one group is what `create_query_group`,
    // `update_query` and `DispatchService::run` all do.
    let query_group_refresh = Arc::new(QueryGroupRefreshService::new(
        query_groups.clone(),
        query_group_service.clone(),
    ));
    // The retention period reaches the sweep and nothing else: a
    // policy number for scheduled destruction has no business on a
    // service a handler can call.
    let retention_service = Arc::new(RetentionService::new(
        assets_arc.clone(),
        groups_arc.clone(),
        personas.clone(),
        indexer.clone(),
        trash_retention()?,
    ));
    // `DispatchRunnerService` is built later than its support
    // siblings: its post-reify provenance repair pass composes
    // `AssetService`, which cannot exist until the job queue does.

    // Job engine. `Full` starts the worker `Monitor`; `ReadOnly` only opens
    // the queue (enqueue-only — jobs stay Pending until the `Full` worker
    // drains the shared apalis DB).
    let pool = jobs::open_job_pool(db_path).await?;
    let jobs_pool = pool.clone();
    // Late-bound dispatch runtime cell — filled after the queue exists
    // (the runtime references the queue as its re-enqueue port, so it
    // cannot be constructed before start).
    let dispatch_cell: Arc<OnceLock<Arc<DispatchRunEnv>>> = Arc::new(OnceLock::new());
    // The disclosure writer the `provenance_stamp` handler drives. A
    // cell like the two below, but not for their reason: nothing here
    // is chicken-and-egg, the cell is what lets a build leave stamping
    // unwired and have the handler skip rather than fail.
    let provenance_cell: Arc<OnceLock<Arc<ProvenanceService>>> = Arc::new(OnceLock::new());
    // Late-bound invalidator cell (same chicken-and-egg): handler-chain
    // writes (auto_tag / cover_gen / index_rebuild) notify query-group
    // refreshes through it (W4-a).
    let invalidator_cell: Arc<OnceLock<QueryGroupInvalidator>> = Arc::new(OnceLock::new());
    // Retention-sweep cell: the `trash_purge` handler drives the sweep
    // through the support service so the retention period and the
    // search-document lifecycle are not reimplemented on raw
    // repositories. Filled immediately — unlike the two cells above it
    // has no queue dependency; the cell survives because an unbound
    // one is how a test / preview harness says "no sweep".
    let retention_cell: Arc<OnceLock<Arc<RetentionService>>> = Arc::new(OnceLock::new());
    let _ = retention_cell.set(retention_service.clone());
    let job_queue = match mode {
        CoreMode::Full => {
            // Worker parallelism comes from the `jobs.concurrency`
            // setting, resolved as `default → env → stored`:
            // `ASTERISM_JOB_CONCURRENCY` applies while nothing is
            // stored, and a value set in the settings screen wins once
            // there is one. `0` (the registry default) means "follow
            // the machine", which `jobs::start` expresses as `None`. A
            // failed read lands in that same fallback rather than
            // aborting startup — a tuning knob must not be able to stop
            // the app — so it is logged instead of swallowed.
            //
            // Resolved once, at startup: apalis fixes the worker count
            // when the `Monitor` is built, so a later change takes
            // effect on the next launch. A stored value that turns out
            // to be unusable is recoverable from the settings screen
            // (Reset clears the row and hands the key back to the env
            // layer, or to the default) — the registry range is what
            // keeps it from being unusable in the first place.
            //
            // Resolved inside this arm because ReadOnly never starts
            // workers and would otherwise pay the query for nothing.
            let job_concurrency = match app_setting_service.get("jobs.concurrency").await {
                Ok(dto) => serde_json::from_str::<i64>(&dto.value_json)
                    .ok()
                    .filter(|n| *n > 0)
                    .and_then(|n| usize::try_from(n).ok()),
                Err(err) => {
                    tracing::warn!(
                        event = "diag.setting.read_failed",
                        error = %err,
                        "jobs.concurrency unreadable; following available_parallelism"
                    );
                    None
                }
            };
            jobs::start(
                pool,
                JobDeps {
                    emitter,
                    assets: assets.clone(),
                    tags: tags.clone(),
                    edges: edges.clone(),
                    thumbs: thumbs.clone(),
                    modalities: modalities.clone(),
                    asset_bodies: asset_bodies.clone(),
                    search_index: indexer.clone(),
                    source_texts: source_texts.clone(),
                    dispatch: dispatch_cell.clone(),
                    query_group_refresh: query_group_refresh.clone(),
                    query_group_invalidator: invalidator_cell.clone(),
                    retention_service: retention_cell.clone(),
                    series: series.clone(),
                    observations: observations.clone(),
                    // The same two adapters the layer service holds, so
                    // a band a job writes and a band a person opens are
                    // rows in one table rather than two readings of it.
                    material_layers: (*material_layers).clone(),
                    chapter_marks: (*chapter_marks).clone(),
                    previews_dir: previews_dir.clone(),
                    provenance: provenance_cell.clone(),
                },
                job_concurrency,
            )
            .await?
        }
        CoreMode::ReadOnly => jobs::open_queue(pool).await?,
    };
    let job_queue_arc = Arc::new(job_queue);

    let query_group_invalidator = QueryGroupInvalidator::new(
        job_queue_arc.clone() as Arc<dyn asterism_core::domain::repository::JobQueue>
    );
    // Fill the handler-side cell now that the queue-backed invalidator
    // exists. `set` cannot fail here (nothing else writes the cell).
    let _ = invalidator_cell.set(query_group_invalidator.clone());
    // Session resolver (P3): `AssetService::add` routes
    // `AddAssetCommand::external_session_key` through this service so
    // re-imports of the same JSONL / harvest bundle converge onto the
    // same Session row. Built here so the `Arc` can be shared with
    // both the AssetService and the CoreCtx SessionService seat.
    let session_service = Arc::new(SessionService::new(sessions_repo.clone()));
    let asset_service = Arc::new(AssetService::new(
        assets_arc.clone(),
        personas.clone(),
        tags_arc.clone(),
        groups_arc.clone(),
        dirs_arc.clone(),
        edges_arc.clone(),
        snapshots.clone(),
        dispatches.clone(),
        source_texts.clone(),
        job_queue_arc.clone(),
        // Read side is Tantivy alone (it is what ranks); write side
        // is the fan-out, so a trashed asset leaves both indexes.
        search_index.clone(),
        indexer.clone(),
        query_groups.clone(),
        query_group_invalidator.clone(),
        session_service.clone(),
        previews_dir.clone(),
    ));
    let dispatch_runner_service = Arc::new(DispatchRunnerService::new(
        dispatches.clone(),
        snapshots.clone(),
        assets_arc.clone(),
        edges_arc.clone(),
        personas.clone(),
        asset_service.clone(),
        job_queue_arc.clone(),
    ));
    // Kick one retention sweep per `Full` startup. The sweep is
    // self-chaining while pages come back full, so this single enqueue
    // drains whatever aged past retention while the app was closed —
    // which is the realistic case for a desktop app that is not running
    // when the clock passes a trash stamp's expiry.
    //
    // The observation sweep rides the same trigger for the same
    // reason: both expire rows on a clock the application does not
    // otherwise consult, and startup is the one moment a desktop app
    // reliably reaches.
    if matches!(mode, CoreMode::Full) {
        use asterism_core::domain::job::JobKind;
        use asterism_core::domain::repository::JobQueue as _;
        for kind in [JobKind::TrashPurge, JobKind::ObservationSweep] {
            if let Err(err) = job_queue_arc.enqueue(kind, serde_json::json!({})).await {
                tracing::warn!(
                    event = "diag.retention.enqueue_failed",
                    job_kind = kind.as_str(),
                    error = %err,
                    "could not enqueue a startup retention sweep"
                );
            }
        }
        // The content-hash backfill rides the same trigger, for a
        // different reason: every asset imported before the column
        // existed has no fingerprint, and without a pass over them the
        // duplicate report would only ever see newly-imported files —
        // an empty answer that looks like a clean library. Like the
        // sweeps above it is self-chaining and idempotent: a page with
        // nothing left to hash simply stops, so a startup on an
        // already-hashed library costs one query.
        //
        // Dedupe against the durable queue first: a walk page chained
        // by the previous run survives the restart as a Pending row,
        // and enqueueing a fresh `cursor: null` walk on top of it runs
        // both in parallel — every unhashed file read twice for one
        // answer. A probe failure falls through to the enqueue: the
        // walk is idempotent, so the double-walk it risks is the
        // cheaper error next to silently skipping the backfill.
        let walk_already_queued = job_queue_arc
            .has_pending_batch(JobKind::MaterialHash)
            .await
            .unwrap_or(false);
        if walk_already_queued {
            tracing::info!(
                event = "diag.material_hash.startup_skipped",
                "content-hash backfill walk already queued; not starting a second one"
            );
        } else if let Err(err) = job_queue_arc
            .enqueue(
                JobKind::MaterialHash,
                serde_json::json!({ "batch": true, "cursor": null }),
            )
            .await
        {
            tracing::warn!(
                event = "diag.material_hash.enqueue_failed",
                error = %err,
                "could not enqueue the startup content-hash backfill"
            );
        }
        // The dimension backfill rides the same trigger for the same
        // shape of reason: every asset imported before schema V69 has no
        // width / height, and without a pass over them the resolution
        // facet answers only about newly-imported files — a nearly empty
        // grid that looks like a library of unmeasurable material.
        //
        // Cheaper to repeat than the hash walk, and it stops for good
        // rather than merely stopping early: its predicate is
        // `dims_probed_at IS NULL`, so a row is offered once whatever
        // the answer, and a startup on an already-measured library costs
        // one query. The hash walk's equivalent claim rests on a
        // sentinel written into the answer; this one rests on V71's
        // separate column, because every integer `width_px` can hold is
        // a real measurement.
        //
        // Same dedupe against the durable queue, for the same reason:
        // a chained page that survived the restart plus a fresh
        // `cursor: null` walk would read every unmeasured file twice.
        let dims_walk_already_queued = job_queue_arc
            .has_pending_batch(JobKind::AssetDims)
            .await
            .unwrap_or(false);
        if dims_walk_already_queued {
            tracing::info!(
                event = "diag.asset_dims.startup_skipped",
                "dimension backfill walk already queued; not starting a second one"
            );
        } else if let Err(err) = job_queue_arc
            .enqueue(
                JobKind::AssetDims,
                serde_json::json!({ "batch": true, "cursor": null }),
            )
            .await
        {
            tracing::warn!(
                event = "diag.asset_dims.enqueue_failed",
                error = %err,
                "could not enqueue the startup dimension backfill"
            );
        }
        // The chapter walk rides the same trigger, for the reason the
        // two above it do: every video and audio file imported before
        // `material_layer` existed carries a chapter list nothing has
        // read, and the ingest fan-out only fires for files arriving
        // from now on. Without a pass over the rest, a library that is
        // simply sitting there shows chapters on nothing.
        //
        // It stops the way the dimension walk stops rather than the way
        // the hash walk does: its predicate is "no imported structure
        // band", and a completed reading always leaves one — including
        // for a file that declares no chapters, which is filed as an
        // empty band. So a start on an already-read library costs one
        // query.
        //
        // Same dedupe against the durable queue, for the same reason:
        // a chained page that survived the restart plus a fresh
        // `cursor: null` walk would spawn an ffmpeg per file twice.
        let chapter_walk_already_queued = job_queue_arc
            .has_pending_batch(JobKind::ChapterScan)
            .await
            .unwrap_or(false);
        if chapter_walk_already_queued {
            tracing::info!(
                event = "diag.chapter_scan.startup_skipped",
                "chapter backfill walk already queued; not starting a second one"
            );
        } else if let Err(err) = job_queue_arc
            .enqueue(
                JobKind::ChapterScan,
                serde_json::json!({ "batch": true, "cursor": null }),
            )
            .await
        {
            tracing::warn!(
                event = "diag.chapter_scan.enqueue_failed",
                error = %err,
                "could not enqueue the startup chapter backfill"
            );
        }
        // The series walk rides the same trigger, and it is the one that
        // has to be here rather than merely benefits from it: its
        // population is `(material, rule)` pairs with no row, and a rule
        // registered while the app was closed — or one shipped by a
        // migration this start just applied — leaves *every* material
        // unanswered under it. Nothing else notices. The per-asset
        // enqueue only fires when a fingerprint writes `meta_kv`, which
        // has already happened for a library that is sitting there.
        //
        // Cheap to repeat and it stops for good, on the terms V71's
        // column buys the dimension walk: a pair leaves the population by
        // acquiring a row, and all three answers are rows — so a start on
        // an already-derived library costs one query.
        //
        // Same dedupe against the durable queue, for the same reason: a
        // page chained by the previous run survives the restart as a
        // Pending row, and a fresh `cursor: null` walk on top of it would
        // derive the same pairs twice.
        let series_walk_already_queued = job_queue_arc
            .has_pending_batch(JobKind::SeriesDerive)
            .await
            .unwrap_or(false);
        if series_walk_already_queued {
            tracing::info!(
                event = "diag.series_derive.startup_skipped",
                "series derivation walk already queued; not starting a second one"
            );
        } else if let Err(err) = job_queue_arc
            .enqueue(
                JobKind::SeriesDerive,
                serde_json::json!({ "batch": true, "cursor": null }),
            )
            .await
        {
            tracing::warn!(
                event = "diag.series_derive.enqueue_failed",
                error = %err,
                "could not enqueue the startup series derivation walk"
            );
        }
    }

    let snapshot_service = Arc::new(SnapshotService::new(
        snapshots.clone(),
        personas.clone(),
        assets_arc.clone(),
        groups_arc.clone(),
        query_group_invalidator.clone(),
    ));
    // `query_group_service` was built up-front so the JobDeps entry
    // for the `query_group_refresh` handler could share the same
    // instance; the live-source dispatch path reuses it here.
    let dispatch_service = Arc::new(DispatchService::new(
        snapshots.clone(),
        dispatches.clone(),
        job_queue_arc.clone(),
        groups_arc.clone(),
        query_groups.clone(),
        query_group_service.clone(),
        pursuits.clone(),
    ));

    // Register the built-in exporters (`comfy` / `file` / `http`).
    let comfy: Arc<dyn Exporter> = Arc::new(ComfyHttpExporter::new());
    let file: Arc<dyn Exporter> = Arc::new(FileExporter::new());
    let http: Arc<dyn Exporter> = Arc::new(HttpExporter::new());
    let mut exporters: HashMap<String, Arc<dyn Exporter>> = HashMap::new();
    exporters.insert(comfy.slug().to_string(), comfy);
    exporters.insert(file.slug().to_string(), file);
    exporters.insert(http.slug().to_string(), http);
    let exporter_registry = ExporterRegistry::new(exporters);

    // AI-disclosure provenance for what a dispatch mints.
    //
    // Unsigned, because this repository ships no certificate and there
    // is no configuration surface that supplies one yet. That is the
    // supported state rather than a degraded one: the writer emits the
    // IPTC/XMP half — the half platforms read most widely, and the one
    // that needs no key material — and reports the manifest half as
    // skipped. An untrusted manifest would be worse than none.
    //
    // `Withhold` is the documented recommendation, and this is the
    // place the recommendation was written for. The prompt field
    // receives whatever the generator wrote as one blob, which for the
    // one family that supplies it carries the model name and every
    // LoRA hash beside the text somebody typed; IPTC scopes the
    // property to what was given "as prompt(s)", the AI Act asks only
    // that synthetic origin be detectable, and publication is the one
    // operation here that cannot be undone. Turning it on is a
    // deployment's call, and it does not have a way to make it yet —
    // which is why this is a literal rather than a setting lookup.
    let _ = provenance_cell.set(Arc::new(ProvenanceService::new(
        assets_arc.clone(),
        edges_arc.clone(),
        Arc::new(InfraProvenanceWriter::unsigned()),
        PromptDisclosure::Withhold,
    )));

    // Fill the late-bound dispatch cell so apalis `DispatchRun` jobs route
    // into a live runtime.
    let reenqueue: Arc<dyn ReEnqueue> = Arc::new(QueueReEnqueue {
        queue: job_queue_arc.clone(),
    });
    let _ = dispatch_cell.set(Arc::new(DispatchRunEnv {
        registry: exporter_registry.clone(),
        service: dispatch_runner_service.clone(),
        snapshots: snapshots.clone(),
        dispatches: dispatches.clone(),
        assets: assets_arc.clone(),
        reenqueue,
    }));

    // (Session snapshot drift check retired with the rkyv store —
    // list_sessions now derives aggregates at query time; the
    // SessionRebuild handler is a no-op kept for wire compatibility.)

    // Query Group refresh — the startup home of the "initial evaluation"
    // the V19 migration cannot perform itself (no isle / Tantivy at raw-
    // connection migrate time; see `migrations::v19_selection_model`).
    // Runs before the context is handed to any boundary, so a query
    // group is never served with stale-or-empty members after a schema
    // wave — the refresh leaves no window open. `Full` mode only: the
    // read-only process must not write memberships. Failures are loud
    // but non-fatal: one corrupt rule does not block the app.
    if matches!(mode, CoreMode::Full) {
        let outcome = query_group_refresh.refresh_all().await;
        for (bucket, err) in &outcome.failures {
            tracing::error!(
                event = "diag.query_group.refresh_failed",
                // Rendered, not `Debug`ged: the sibling site in the job
                // handler writes the same field, and `GroupId`'s derived
                // `Debug` would make one of the two unreadable by the
                // same `json_extract`.
                bucket = ?bucket.as_ref().map(|b| b.to_string()),
                error = %err,
                "query group startup refresh failed"
            );
        }
        if outcome.refreshed > 0 {
            tracing::info!(
                event = "diag.query_group.refreshed",
                refreshed = outcome.refreshed,
                "query groups refreshed at startup"
            );
        }
    }

    Ok(CoreCtx {
        persona_service: Arc::new(PersonaService::new(
            personas.clone(),
            persona_themes,
            persona_profiles,
            assets_arc.clone(),
            indexer.clone(),
            job_queue_arc.clone(),
        )),
        asset_service,
        thumb_service: Arc::new(ThumbService::new(Arc::new(thumbs))),
        snapshot_service,
        dispatch_service,
        query_group_service,
        modality_service: Arc::new(ModalityService::new(Arc::new(modalities))),
        series_strategy_service: Arc::new(SeriesStrategyService::new(
            Arc::new(series),
            job_queue_arc.clone(),
        )),
        app_setting_service,
        session_service,
        asset_comment_service: Arc::new(AssetCommentService::new(
            asset_comments,
            assets_arc.clone(),
            personas.clone(),
        )),
        material_mark_service: Arc::new(MaterialMarkService::new(
            material_marks,
            material_layers.clone(),
            assets_arc.clone(),
            personas.clone(),
        )),
        material_layer_service: Arc::new(MaterialLayerService::new(
            material_layers,
            chapter_marks,
            assets_arc,
        )),
        thread_service: Arc::new(ThreadService::new(threads, personas)),
        exporter_registry,
        jobs_pool,
        telemetry,
        observations,
        support: SupportServices {
            retention: retention_service,
            query_group_refresh,
            dispatch_runner: dispatch_runner_service,
        },
    })
}

#[cfg(test)]
mod trash_retention_tests {
    use super::*;

    #[test]
    fn absent_env_falls_back_to_the_default_window() {
        assert_eq!(
            parse_trash_retention(None).unwrap(),
            chrono::Duration::days(TRASH_RETENTION_DEFAULT_DAYS)
        );
    }

    #[test]
    fn a_valid_value_wins_and_tolerates_surrounding_space() {
        assert_eq!(
            parse_trash_retention(Some("30")).unwrap(),
            chrono::Duration::days(30)
        );
        assert_eq!(
            parse_trash_retention(Some("  7\n")).unwrap(),
            chrono::Duration::days(7)
        );
        assert_eq!(
            parse_trash_retention(Some("1")).unwrap(),
            chrono::Duration::days(1),
            "one day is the shortest honest retention"
        );
    }

    /// A bad value must stop the process, not pick a number for the
    /// user. This setting decides when data stops being recoverable, so
    /// both silent fallbacks are wrong: "quietly 14 days" hides a typo,
    /// and "quietly 0" purges on the next sweep.
    #[test]
    fn malformed_or_non_positive_values_are_refused() {
        for bad in ["", "abc", "7d", "3.5", "0", "-1"] {
            let err = parse_trash_retention(Some(bad));
            assert!(
                err.is_err(),
                "{bad:?} must be refused, got {:?}",
                err.map(|d| d.num_days())
            );
        }
    }
}
