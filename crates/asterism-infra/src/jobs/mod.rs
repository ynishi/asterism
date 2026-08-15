//! Job engine — apalis with the `apalis-sql` SQLite backend.
//!
//! ## Contents
//!
//! - [`AsterismJob`] — the queue payload. A single typed queue holds every
//!   kind of job; the kind is carried in a slug field.
//! - [`SqliteJobQueue`] — implementation of the `JobQueue` port from
//!   `asterism-core`.
//! - [`start`] — runs `SqliteStorage::setup` and spawns the worker
//!   `Monitor` on a tokio task.
//!
//! ## Coexistence with `rusqlite-isle`
//!
//! `apalis-sql` reaches SQLite through `sqlx 0.8` (which uses
//! `libsqlite3-sys 0.30`). The workspace pins `rusqlite-isle` to the
//! release line built against the same cluster, so both stacks link
//! against a single copy of `sqlite3`. Job persistence tables (`Jobs`,
//! and so on) are created by `SqliteStorage::setup` inside the same DB
//! file as the domain tables — they are owned by apalis, and the domain
//! never mirrors them.
//!
//! ## Handlers
//!
//! [`handlers`] holds the per-kind implementations: `cover_gen`
//! (modality-specific heuristic), `auto_tag` (keyword extraction + tag
//! materialisation), and `edge_rebuild` (windowed incremental rebuild via
//! `plan_edges`). `asset_add`, `persona_import`, and `index_rebuild` are
//! future work.

pub mod chapter_ffmetadata;
pub mod handlers;
pub mod preview_ffmpeg;
pub mod thumb_ffmpeg;
#[cfg(target_os = "macos")]
pub mod thumb_macos;
#[cfg(target_os = "macos")]
pub mod thumb_video;

use std::sync::Arc;

use apalis::prelude::*;
use apalis_sql::sqlite::{SqlitePool as InternalSqlitePool, SqliteStorage};
use apalis_sql::sqlx;

/// Re-export the sqlx-side pool type so downstream crates (server /
/// Tauri UI) can carry a handle without pulling `apalis_sql::sqlite`
/// directly.
pub type SqlitePool = InternalSqlitePool;
use asterism_core::domain::job::JobKind;
use asterism_core::domain::repository::{JobQueue, ProgressEmitter};
use asterism_core::domain::value::Progress;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::dispatch::run_dispatch_run;
use crate::sqlite::repo::{
    SqliteAssetBodyRepository, SqliteAssetCommentRepository, SqliteAssetRepository,
    SqliteEdgeRepository, SqliteModalityRepository, SqliteTagRepository, SqliteThumbRepository,
};

/// Queue payload — the job-kind slug (see [`JobKind::as_str`]) plus a
/// kind-specific JSON payload.
///
/// Apalis' typed storage holds one Rust type per queue, so the kind is a
/// slug field rather than an enum variant. This maps 1:1 to the
/// `(JobKind, Value)` input of the [`JobQueue`] port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsterismJob {
    /// Job-kind slug (`asset_add`, and so on).
    pub kind: String,
    /// Kind-specific parameters (handlers know how to interpret them).
    pub payload: serde_json::Value,
}

/// Dependencies passed in by the caller (`asterism-ui` /
/// `asterism-server`).
pub struct JobDeps {
    /// Progress emitter — usually a Tauri emitter in the UI, or a log
    /// emitter in the standalone server.
    pub emitter: Arc<dyn ProgressEmitter>,
    /// Asset repository. Held by concrete type because handlers use the
    /// non-port helpers (`candidates_near`, `set_cover`, `set_keywords`).
    pub assets: SqliteAssetRepository,
    /// Tag repository (used to materialise keywords into channel tags).
    pub tags: SqliteTagRepository,
    /// Edge repository (used to replace constellation edges).
    pub edges: SqliteEdgeRepository,
    /// Thumb cache repository — `thumb_gen` writes resized JPEG
    /// bytes here for a given (asset_id, size_px).
    pub thumbs: SqliteThumbRepository,
    /// Modality master repository — `thumb_gen` / `cover_gen` resolve
    /// `asset.modality` → the master row → its `ContentKind` to decide
    /// thumbnail eligibility and the cover template (kind-driven, no
    /// slug-literal branching).
    pub modalities: SqliteModalityRepository,
    /// Body cache repository (SQLite `asset_body` table) — write
    /// target of the `index_rebuild` handler; source of truth for
    /// the Tantivy projection.
    pub asset_bodies: SqliteAssetBodyRepository,
    /// Comment thread repository — read by `index_rebuild`, which
    /// composes the thread into the asset's derived text (a note left
    /// on a picture is one of the few sentences the library holds
    /// about it). Read-only from here: the write verbs belong to
    /// `AssetCommentService`, which enqueues this job after each one.
    pub comments: SqliteAssetCommentRepository,
    /// Write side of every text index an asset's body feeds — the SQL
    /// trigram index behind the Query-side `text_match` predicate and
    /// the Tantivy index behind Retrieval. Held as the port, not as
    /// `TantivyIndex`, so `index_rebuild` maintains both through one
    /// call and a new index is added at the composition root rather
    /// than in this handler (see `search::fan_out`).
    pub search_index: Arc<dyn asterism_core::domain::repository::AssetIndexer>,
    /// Source-text reader used by `index_rebuild` to resolve the
    /// full body from an asset locator (`<path>` or `<path>#<frag>`).
    pub source_texts: Arc<crate::source_text::FsSourceTextReader>,
    /// Late-bound dispatch runtime — carries the exporter registry
    /// and the `DispatchService`. The cell is initialised **after**
    /// [`start`] returns the queue handle, because the runtime needs
    /// the queue as its re-enqueue port (chicken-and-egg: the queue
    /// exists only after `start`, but `start` needs `JobDeps`). Until
    /// the cell is set, `DispatchRun` jobs skip with a "no runtime"
    /// message. Tests / preview tooling can leave the cell empty.
    pub dispatch: Arc<std::sync::OnceLock<Arc<crate::dispatch::DispatchRunEnv>>>,
    /// Query Group bulk-refresh service — the `query_group_refresh`
    /// handler calls `refresh_for_persona` on it (W4 invalidation).
    /// A support service: the per-group evaluator it drives is
    /// transport-fronted, the sweep is not, so this handle exists here
    /// and on `CoreCtx` and nowhere a request handler can see it.
    pub query_group_refresh: Arc<asterism_core::application_support::QueryGroupRefreshService>,
    /// Query Group invalidator cell — late-bound like [`dispatch`]
    /// (chicken-and-egg: the invalidator enqueues into the queue that
    /// only exists once [`start`] returns). Handlers whose writes
    /// change rule inputs (`auto_tag` / `cover_gen` / `index_rebuild`,
    /// W4-a) notify through it; an empty cell (tests / preview
    /// tooling) degrades to "no refresh", the pre-W4-a behaviour.
    /// The `query_group_refresh` handler itself must NOT notify —
    /// its materialise write is the "job-derived writes are excluded
    /// from the hook" case and notifying would self-trigger a
    /// refresh loop.
    pub query_group_invalidator: Arc<
        std::sync::OnceLock<
            asterism_core::application::query_group_invalidation::QueryGroupInvalidator,
        >,
    >,
    /// Retention sweep cell — the `trash_purge` handler drives
    /// `purge_expired` through it rather than reimplementing the sweep
    /// on the raw repositories, so the retention period and the
    /// search-document lifecycle stay in one place. A support service:
    /// scheduled destruction of aged rows has no wire surface, and the
    /// user-facing single-row `AssetService::purge` (which refuses
    /// anything not already trashed) is the verb the transports get
    /// instead. Still a cell so a test / preview harness can leave it
    /// unbound, which degrades to "no sweep" — the queue dependency
    /// that originally forced the late binding is gone.
    pub retention_service:
        Arc<std::sync::OnceLock<Arc<asterism_core::application_support::RetentionService>>>,
    /// Series axis — the registered rules and the keys derived under
    /// them. `series_derive` reads both halves through this and writes
    /// one; nothing else in the job engine touches it.
    ///
    /// Held by concrete type like the repositories above rather than as
    /// `Arc<dyn SeriesRepository>`: the handler needs no other
    /// implementation, and the port exists for the direction of the
    /// dependency, not to make this field polymorphic.
    pub series: crate::sqlite::repo::SqliteSeriesRepository,
    /// Observation streams — the `observation_sweep` handler expires
    /// rows past their stream's retention through this. Not a cell:
    /// unlike the services above it needs nothing but the isle, which
    /// exists before the queue does.
    pub observations: crate::observe::ObservationStore,
    /// The bands of marks over a material, and the chapters inside one
    /// — the two ports `chapter_scan` hands to
    /// `application_support::replace_imported_chapters`.
    ///
    /// Both, because that function resolves the imported band through
    /// the first and replaces its contents through the second, and
    /// splitting the pair would let a caller hold one without the other
    /// — which is a caller that can name a band it cannot fill.
    ///
    /// Held by concrete type like the repositories above, for the same
    /// reason: no other implementation is wanted here, and the port
    /// exists for the direction of the dependency rather than to make
    /// this field polymorphic.
    pub material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository,
    /// See [`material_layers`](Self::material_layers).
    pub chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository,
    /// Where `preview_gen` writes video preview renditions
    /// (`<profile>/previews/`). Passed explicitly rather than resolved
    /// from the active profile so a test that sandboxes the database
    /// sandboxes the renditions with it (the same reasoning as the
    /// Tantivy index override).
    pub previews_dir: std::path::PathBuf,
    /// Writes the AI disclosure into a file this library produced — the
    /// one thing `disclosure_stamp` does.
    ///
    /// A cell, and empty is a supported state: a build that has not
    /// decided whether it wants its exports rewritten leaves it unset,
    /// and the handler skips with a message rather than failing. That
    /// is the same shape [`dispatch`](Self::dispatch) uses, for a
    /// different reason — this one is not late-bound, it is optional.
    pub disclosure: Arc<
        std::sync::OnceLock<Arc<asterism_core::application::disclosure_service::DisclosureService>>,
    >,
}

/// Execution environment handed to worker handlers via apalis' `Data`
/// extractor. Built by [`start`] from the caller-supplied [`JobDeps`] and
/// a queue handle used for chain-enqueue.
pub struct JobEnv {
    /// Caller-supplied dependencies.
    pub deps: JobDeps,
    /// Queue handle used to chain jobs (for example `auto_tag` enqueues
    /// `edge_rebuild` after its keywords land).
    pub queue: SqliteJobQueue,
}

/// Implementation of `JobQueue` on top of apalis' `SqliteStorage`.
#[derive(Clone)]
pub struct SqliteJobQueue {
    storage: SqliteStorage<AsterismJob>,
}

#[async_trait]
impl JobQueue for SqliteJobQueue {
    async fn enqueue(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
    ) -> Result<String, DomainError> {
        self.enqueue_with_priority(kind, payload, 0).await
    }

    async fn enqueue_with_priority(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
        priority: i32,
    ) -> Result<String, DomainError> {
        // `SqliteStorage` is pool-backed and cheap to clone; `push` takes
        // `&mut self`, so we clone the handle locally.
        let mut storage = self.storage.clone();
        let parts = storage
            .push(AsterismJob {
                kind: kind.as_str().to_string(),
                payload,
            })
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("job push failed: {e}")))?;
        let task_id = parts.task_id.to_string();
        // apalis' typed API does not expose a "push at priority"
        // shortcut, so we bump the `priority` column directly. Higher
        // = popped first (the fetch query orders by priority DESC).
        // Skipping the update when priority is 0 avoids a round-trip
        // for the common case.
        if priority != 0 {
            let pool = storage.pool().clone();
            let task_id_owned = task_id.clone();
            if let Err(err) = sqlx::query("UPDATE Jobs SET priority = ? WHERE id = ?")
                .bind(priority as i64)
                .bind(&task_id_owned)
                .execute(&pool)
                .await
            {
                // Enqueue itself succeeded; downgrading priority to a
                // warning keeps the job runnable at normal FIFO.
                tracing::warn!(
                    event = "diag.jobs.priority_bump_failed",
                    task_id = %task_id_owned,
                    priority,
                    error = %err,
                    "priority bump failed"
                );
            }
        }
        Ok(task_id)
    }

    async fn has_pending_batch(&self, kind: JobKind) -> Result<bool, DomainError> {
        // `Pending` only, per the trait contract: a `Running` row
        // orphaned by a crash is never fetched again (the worker fetch
        // targets Pending / Failed / Retry and no orphan-reenqueue is
        // configured), so counting it would suppress the startup
        // backfill forever after one crash. Served by
        // `idx_jobs_kind_status` (kind-expression prefix).
        let storage = self.storage.clone();
        let pool = storage.pool().clone();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM Jobs \
              WHERE json_extract(job, '$.kind') = ? \
                AND status = 'Pending' \
                AND json_extract(job, '$.payload.batch') = 1",
        )
        .bind(kind.as_str())
        .fetch_one(&pool)
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("pending batch probe failed: {e}")))?;
        Ok(count > 0)
    }
}

/// How a run ended. Four outcomes, not two: a kind with no handler
/// neither succeeded nor failed, and a run that unwound produced no
/// verdict at all. Collapsing either into success or failure would
/// make "how often does this fail" unanswerable — the question
/// `JobLog` exists to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobOutcome {
    /// The handler ran and returned.
    Completed,
    /// The handler ran and returned an error.
    Failed,
    /// No handler ran: the kind is unimplemented, unknown, or its
    /// runtime was not configured.
    Skipped,
    /// The run did not reach its verdict — the handler panicked, or
    /// the task was dropped mid-flight. Recorded by [`RunRecord`]'s
    /// `Drop`, which is the only path that survives an unwind.
    Panicked,
}

impl JobOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            JobOutcome::Completed => "completed",
            JobOutcome::Failed => "failed",
            JobOutcome::Skipped => "skipped",
            JobOutcome::Panicked => "panicked",
        }
    }

    /// Classifies one run that reached its end.
    ///
    /// `skipped` is decided by the dispatch arm rather than inferred
    /// from the returned message: the no-handler arms return `Ok` so
    /// the queue row settles, and reading their prose to tell them
    /// apart would make the classification a function of wording.
    const fn of(handler_failed: bool, skipped: bool) -> Self {
        match (skipped, handler_failed) {
            (true, _) => JobOutcome::Skipped,
            (false, true) => JobOutcome::Failed,
            (false, false) => JobOutcome::Completed,
        }
    }
}

/// Emits the `JobLog` row for one run, from `Drop`.
///
/// A plain call after the handler would miss exactly the runs most
/// worth recording: a panicking handler unwinds past it, leaving no
/// trace anywhere — apalis 0.7 has no panic-catching layer wired here,
/// and the `Jobs` row it abandons stays `Running` indefinitely because
/// this process keeps its worker heartbeat alive, so the orphan
/// re-enqueue sweep never claims it. `Drop` runs on the unwind, so the
/// row exists either way; the panic itself still propagates, because
/// swallowing it here would hide a bug rather than record one.
struct RunRecord {
    task_id: String,
    job_kind: String,
    attempt: i64,
    started: std::time::Instant,
    /// Set on the normal path. Still `None` at drop time means the run
    /// never reached a verdict.
    outcome: Option<JobOutcome>,
    detail: String,
}

impl Drop for RunRecord {
    fn drop(&mut self) {
        let outcome = self.outcome.unwrap_or(JobOutcome::Panicked);
        // `{stream}.{object}.{action}` — the name identifies the
        // record's type, and the columns beside it are what queries
        // group by. Both come from these two fields, so the name and
        // the columns cannot disagree.
        let event = format!("job.{}.{}", self.job_kind, outcome.as_str());
        tracing::info!(
            event = %event,
            task_id = %self.task_id,
            job_kind = %self.job_kind,
            outcome = outcome.as_str(),
            attempt = self.attempt,
            duration_ms = self.started.elapsed().as_millis() as i64,
            detail = %self.detail,
            "job run finished"
        );
    }
}

/// Worker entry point — dispatches on the kind slug to the matching
/// handler in [`handlers`].
///
/// Handler failures surface as a progress message; the job itself is
/// still reported as complete. v1 has no retry policy; every handler is
/// designed to be recoverable from persisted state (for example, a null
/// cover can be re-enqueued).
///
/// Every run that reaches this function appends one `JobLog` row (see
/// [`RunRecord`], which emits from `Drop` so an unwinding run is still
/// recorded). This is the only place every job passes through, and
/// apalis' `Jobs` table keeps state rather than history — so without
/// this row, "how long did `cover_gen` take yesterday" and "how often
/// does `auto_tag` fail" have no answer.
///
/// Two limits on that coverage, stated plainly because a log people
/// trust to be complete is worse than one they know the edges of:
///
/// - A run whose process is killed mid-flight leaves no row. `Drop`
///   needs an unwind, and `SIGKILL` does not give one.
/// - A handler that returns `Ok` after doing nothing (its subject was
///   already gone, its cell was unbound) is recorded as `completed`,
///   because that is what it reported. Distinguishing those would mean
///   handlers returning an outcome rather than a message.
///
/// The record goes out through `tracing`. That keeps the durable write
/// off this task, though the stderr layer on the same event is still a
/// synchronous write here.
async fn handle_asterism_job(
    job: AsterismJob,
    task_id: TaskId,
    attempt: Attempt,
    env: Data<Arc<JobEnv>>,
) {
    // Emits on the way out, whichever way that is.
    let mut record = RunRecord {
        task_id: task_id.to_string(),
        job_kind: job.kind.clone(),
        // apalis increments before calling us, so this is 1 on a first
        // run. It counts *claims*, not retries: nothing here re-queues
        // on failure, so a value above 1 means the previous claim was
        // reclaimed by the orphan sweep — the one trace an interrupted
        // run leaves behind.
        attempt: attempt.current() as i64,
        started: std::time::Instant::now(),
        outcome: None,
        detail: String::new(),
    };
    // `Skipped` is decided here rather than inferred from the message:
    // these arms return `Ok` so the queue row settles, but nothing ran.
    let (result, skipped) = match JobKind::parse(&job.kind) {
        Ok(JobKind::CoverGen) => (handlers::cover_gen(&env, &job.payload).await, false),
        Ok(JobKind::ThumbGen) => (handlers::thumb_gen(&env, &job.payload).await, false),
        Ok(JobKind::AutoTag) => (handlers::auto_tag(&env, &job.payload).await, false),
        Ok(JobKind::EdgeRebuild) => (handlers::edge_rebuild(&env, &job.payload).await, false),
        Ok(JobKind::SessionRebuild) => (handlers::session_rebuild(&env, &job.payload).await, false),
        Ok(JobKind::IndexRebuild) => (handlers::index_rebuild(&env, &job.payload).await, false),
        Ok(JobKind::MaterialText) => (handlers::material_text(&env, &job.payload).await, false),
        Ok(JobKind::DispatchRun) => match env.deps.dispatch.get() {
            Some(rt) => (run_dispatch_run(rt, &job.payload).await, false),
            None => (
                Ok("dispatch_run skipped: no runtime configured".to_string()),
                true,
            ),
        },
        Ok(JobKind::QueryGroupRefresh) => (
            handlers::query_group_refresh(&env, &job.payload).await,
            false,
        ),
        Ok(JobKind::ObservationSweep) => {
            (handlers::observation_sweep(&env, &job.payload).await, false)
        }
        Ok(JobKind::MaterialHash) => (handlers::material_hash(&env, &job.payload).await, false),
        Ok(JobKind::AssetDims) => (handlers::asset_dims(&env, &job.payload).await, false),
        Ok(JobKind::DuplicateScan) => (handlers::duplicate_scan(&env, &job.payload).await, false),
        Ok(JobKind::SeriesDerive) => (handlers::series_derive(&env, &job.payload).await, false),
        Ok(JobKind::AssetFold) => (handlers::asset_fold(&env, &job.payload).await, false),
        Ok(JobKind::PreviewGen) => (handlers::preview_gen(&env, &job.payload).await, false),
        Ok(JobKind::ChapterScan) => (handlers::chapter_scan(&env, &job.payload).await, false),
        Ok(JobKind::DisclosureStamp) => match env.deps.disclosure.get() {
            Some(_) => (handlers::disclosure_stamp(&env, &job.payload).await, false),
            // Same shape as `DispatchRun` above: an unbound cell means
            // nothing was stamped, so it classifies as skipped rather
            // than as a run that did its work.
            None => (
                Ok("disclosure_stamp skipped: no writer configured".to_string()),
                true,
            ),
        },
        Ok(JobKind::TrashPurge) => match env.deps.retention_service.get() {
            Some(_) => (handlers::trash_purge(&env, &job.payload).await, false),
            // Same shape as `DispatchRun` above — an unbound cell means
            // no sweep ran, so it must classify the same way.
            None => (handlers::trash_purge(&env, &job.payload).await, true),
        },
        // Batched `asset_add` and `persona_import` are future work.
        Ok(other) => (
            Ok(format!("{} handler not implemented yet", other.as_str())),
            true,
        ),
        Err(_) => (Ok(format!("unknown job kind skipped: {}", job.kind)), true),
    };
    let ok = result.is_ok();
    record.outcome = Some(JobOutcome::of(result.is_err(), skipped));
    record.detail = match &result {
        Ok(message) => message.clone(),
        Err(err) => err.to_string(),
    };
    let message = match result {
        Ok(message) => format!("{}: {message}", job.kind),
        Err(err) => format!("{} failed: {err}", job.kind),
    };
    // Emitter failures must not tear the job down — progress delivery is
    // best-effort (see the `ProgressEmitter` doc).
    let _ = env
        .deps
        .emitter
        .emit(
            &task_id.to_string(),
            Progress {
                current: 1,
                total: Some(1),
                message: Some(message),
            },
        )
        .await;
    // Also fire a compact per-kind broadcast so the UI can drive a
    // live "N cover_gen done, M edge_rebuild done" ticker without
    // subscribing to every individual task id. Silent on emit
    // failure — the per-job event above already ran.
    let _ = env
        .deps
        .emitter
        .broadcast(
            "jobs:tick",
            serde_json::json!({
                "kind": job.kind,
                "ok": ok,
            }),
        )
        .await;
}

/// Per-kind slice of the apalis `Jobs` table.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobKindSnapshot {
    /// Every persisted row for this kind.
    pub total: u64,
    /// Rows in `Done` state.
    pub done: u64,
    /// Rows in `Pending` state (queued, not picked up yet).
    pub pending: u64,
    /// Rows in `Running` state (actively worked on).
    pub running: u64,
    /// Rows in `Failed` state.
    pub failed: u64,
}

/// Snapshot of the apalis `Jobs` table used by the UI progress banner.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobsSnapshot {
    /// Every persisted row, regardless of status.
    pub total: u64,
    /// Rows in `Done` state.
    pub done: u64,
    /// Rows in `Pending` state (queued, not picked up yet).
    pub pending: u64,
    /// Rows in `Running` state (actively worked on).
    pub running: u64,
    /// Rows in `Failed` state (retry / dead-letter path).
    pub failed: u64,
    /// Per-kind breakdown so the banner can show
    /// `cover_gen 512/512, edge_rebuild 200/500 pending 300`.
    pub by_kind: std::collections::BTreeMap<String, JobKindSnapshot>,
}

/// Queue depth by status — the poll-cheap half of [`jobs_snapshot`].
///
/// No `by_kind`, and so no `json_extract` over every row: this is one
/// indexed `GROUP BY status` (`idx_jobs_status`) and nothing else.
/// [`jobs_snapshot`] pays for the kind × status pass whenever anything
/// is in flight, which is 1.15-1.27 s per call at 368 k accumulated
/// rows [measured 2026-07-21, sqlx slow-statement WARN] — affordable for a
/// 3-second banner poll that only runs while a person is watching, and
/// not affordable for a bench driver polling a 5,000-file import to
/// completion. Kept as a second function
/// rather than as a flag on the first so the expensive path cannot be
/// reached by forgetting an argument.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct JobsDepth {
    /// Rows in `Pending` state (queued, not picked up yet).
    pub pending: u64,
    /// Rows in `Running` state (actively worked on).
    pub running: u64,
    /// Rows in `Done` state.
    pub done: u64,
    /// Rows in `Failed` state (retry / dead-letter path).
    pub failed: u64,
}

/// Reads [`JobsDepth`] off the apalis `Jobs` table.
///
/// `Killed` / `Retry` rows count towards none of the four fields, the
/// same way [`jobs_snapshot`] leaves them out of its four counters. A
/// drain check therefore asks `pending + running == 0`: those are the
/// two states that still owe work.
pub async fn jobs_depth(pool: &SqlitePool) -> Result<JobsDepth, DomainError> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM Jobs GROUP BY status")
            .fetch_all(pool)
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("jobs_depth: {e}")))?;
    let mut depth = JobsDepth::default();
    for (status, count) in rows {
        let n = count.max(0) as u64;
        match status.as_str() {
            "Pending" => depth.pending += n,
            "Running" => depth.running += n,
            "Done" => depth.done += n,
            "Failed" => depth.failed += n,
            _ => {}
        }
    }
    Ok(depth)
}

/// Returns a compact snapshot of the apalis `Jobs` table. Groups by
/// status and by kind so the UI banner can render a "N total, K
/// done" gauge and — if space allows — per-kind chips.
pub async fn jobs_snapshot(pool: &SqlitePool) -> Result<JobsSnapshot, DomainError> {
    let mut snap = JobsSnapshot::default();

    // Status roll-up. `Jobs.status` is a text column; the values
    // apalis 0.7 writes are `Pending`, `Running`, `Done`, `Failed`,
    // `Killed`, `Retry`. Everything else falls into `total` only.
    let rows: Vec<(String, i64)> =
        sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM Jobs GROUP BY status")
            .fetch_all(pool)
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("jobs_snapshot status: {e}")))?;
    for (status, count) in rows {
        let n = count.max(0) as u64;
        snap.total += n;
        match status.as_str() {
            "Done" => snap.done += n,
            "Pending" => snap.pending += n,
            "Running" => snap.running += n,
            "Failed" => snap.failed += n,
            _ => {}
        }
    }

    // Idle fast-path: with no pending / running / failed rows there is
    // no active kind for the UI banner to render (its gauge filter is
    // exactly `pending + running + failed > 0`), so skip the kind ×
    // status pass entirely. This is the steady state outside import
    // bursts — the poll then costs one indexed status roll-up only.
    if snap.pending + snap.running + snap.failed == 0 {
        return Ok(snap);
    }

    // Kind × status roll-up. `Jobs.job` is a JSON blob whose top-
    // level `kind` field carries the slug (`cover_gen`, `thumb_gen`,
    // …). Grouping by both dims once here saves the UI from
    // building the picture out of the flat status roll-up. Served
    // index-only by `idx_jobs_kind_status` (created in [`start`]).
    let kind_rows: Vec<(Option<String>, String, i64)> =
        sqlx::query_as::<_, (Option<String>, String, i64)>(
            "SELECT json_extract(job, '$.kind'), status, COUNT(*) \
             FROM Jobs GROUP BY json_extract(job, '$.kind'), status",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("jobs_snapshot kind: {e}")))?;
    for (kind, status, count) in kind_rows {
        let Some(k) = kind else { continue };
        let entry = snap.by_kind.entry(k).or_default();
        let n = count.max(0) as u64;
        entry.total += n;
        match status.as_str() {
            "Done" => entry.done += n,
            "Pending" => entry.pending += n,
            "Running" => entry.running += n,
            "Failed" => entry.failed += n,
            _ => {}
        }
    }
    Ok(snap)
}

/// Opens the sqlx pool used by the job engine. Provided as a helper so
/// consumers (the Tauri UI, the standalone server) do not need to import
/// sqlx directly. The pool shares the on-disk file with the isle backend
/// (they run on separate connection stacks under WAL).
pub async fn open_job_pool(db_path: &std::path::Path) -> Result<SqlitePool, DomainError> {
    SqlitePool::connect(&format!("sqlite://{}?mode=rwc", db_path.display()))
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("job pool open failed: {e}")))
}

/// Opens the job queue: runs `SqliteStorage::setup`, creates the
/// snapshot-poll indexes, and wraps the storage in a [`SqliteJobQueue`].
///
/// This is the enqueue-only half of [`start`] — no worker `Monitor` is
/// spawned, so the returned queue can push jobs but nothing in this
/// process consumes them. Used by read-only consumers (the standalone
/// server) that share the apalis DB with a worker-running process (the
/// Tauri UI) which drains the queue.
pub async fn open_queue(pool: SqlitePool) -> Result<SqliteJobQueue, DomainError> {
    SqliteStorage::setup(&pool)
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("job storage setup failed: {e}")))?;
    // Snapshot-poll indexes on the apalis-owned `Jobs` table (see
    // [`jobs_snapshot`]): a plain status index for the roll-up and an
    // expression index matching the exact `json_extract(job, '$.kind')`
    // the kind×status GROUP BY uses. Without them the 3-second UI poll
    // full-scanned the table — 1.15-1.27 s/poll at 368 k accumulated
    // Done rows [measured 2026-07-21, sqlx slow-statement WARN]. Created
    // here every boot (idempotent) rather than in the schema migration
    // chain: `Jobs` only exists after `SqliteStorage::setup` above, and
    // an apalis upgrade that recreates the table would silently drop
    // migration-made indexes.
    // The third index serves apalis' own worker fetch poll
    // (`status='Pending' OR status='Failed' … AND job_type=? AND
    // run_at<? ORDER BY priority`): its stock JTIdx(job_type) has
    // zero selectivity here (every row shares one job_type), so the
    // poll degraded to a 1.3-2.9 s full scan at 368 k rows [measured
    // 2026-07-21]. With the composite index SQLite picks a
    // MULTI-INDEX OR plan (one seek per status branch) — ~0 ms.
    for ddl in [
        "CREATE INDEX IF NOT EXISTS idx_jobs_status ON Jobs(status)",
        "CREATE INDEX IF NOT EXISTS idx_jobs_kind_status \
             ON Jobs(json_extract(job, '$.kind'), status)",
        "CREATE INDEX IF NOT EXISTS idx_jobs_fetch \
             ON Jobs(status, job_type, run_at, priority)",
    ] {
        sqlx::query(ddl)
            .execute(&pool)
            .await
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("jobs index setup failed: {e}")))?;
    }
    let storage = SqliteStorage::<AsterismJob>::new(pool);
    Ok(SqliteJobQueue { storage })
}

/// Spawns the worker `Monitor` for an already-opened queue on a tokio
/// task. The monitor runs for the lifetime of the process; graceful
/// shutdown is a future addition.
///
/// Split out of [`start`] so consumers can choose enqueue-only
/// ([`open_queue`] alone) versus enqueue + consume ([`open_queue`] +
/// this).
pub fn start_workers(queue: SqliteJobQueue, deps: JobDeps, concurrency: Option<usize>) {
    let storage = queue.storage.clone();
    let env = JobEnv {
        deps,
        queue: queue.clone(),
    };
    // Parallelism: the caller resolves the requested worker count (the
    // `jobs.concurrency` setting, whose layer stack the settings
    // service owns). `None` means "you decide" — pick
    // `available_parallelism`, falling back to 4 when the OS refuses to
    // answer.
    //
    // Reading the environment here directly is deliberately gone: two
    // layers reading the same knob would disagree the moment a value
    // was stored in the database.
    let concurrency = concurrency.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });
    let worker = WorkerBuilder::new("asterism-jobs")
        .concurrency(concurrency)
        .data(Arc::new(env))
        .backend(storage)
        .build_fn(handle_asterism_job);
    tokio::spawn(Monitor::new().register(worker).run());
}

/// Starts the job engine: opens the queue ([`open_queue`]) and spawns a
/// worker `Monitor` ([`start_workers`]).
///
/// `concurrency` is the caller-resolved worker count; `None` defers to
/// `available_parallelism`. The composition root reads it from the
/// `jobs.concurrency` setting, so the value survives a restart and
/// `ASTERISM_JOB_CONCURRENCY` still applies while nothing is stored —
/// both arrive through the one resolution stack rather than through a
/// second env read here.
///
/// The returned [`SqliteJobQueue`] is what application services hold on
/// to for enqueueing. The monitor runs for the lifetime of the process;
/// graceful shutdown is a future addition.
pub async fn start(
    pool: SqlitePool,
    deps: JobDeps,
    concurrency: Option<usize>,
) -> Result<SqliteJobQueue, DomainError> {
    let queue = open_queue(pool).await?;
    start_workers(queue.clone(), deps, concurrency);
    Ok(queue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    // The two layer ports, in scope so the chapter tests can read back
    // what a handler wrote through them.
    use asterism_core::domain::repository::{ChapterMarkRepository, MaterialLayerRepository};

    /// Test emitter that records every payload it sees.
    struct RecordingEmitter {
        records: Mutex<Vec<(String, Progress)>>,
        notify: tokio::sync::Notify,
    }

    #[async_trait]
    impl ProgressEmitter for RecordingEmitter {
        async fn emit(&self, job_id: &str, progress: Progress) -> Result<(), DomainError> {
            self.records
                .lock()
                .unwrap()
                .push((job_id.to_string(), progress));
            self.notify.notify_one();
            Ok(())
        }
    }

    /// A 1×1 PNG, built here so the fixture depends on no other
    /// crate's test module.
    fn png_bytes() -> Vec<u8> {
        fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(kind);
            hasher.update(payload);
            out.extend_from_slice(&hasher.finalize().to_be_bytes());
            out
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        png.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x02]));
        png.extend_from_slice(&chunk(b"IEND", &[]));
        png
    }

    /// The XMP packet a file on disk carries, if any.
    fn packet_of(path: &std::path::Path) -> Option<String> {
        asterism_disclosure_format::embed::read_xmp(&std::fs::read(path).unwrap()).unwrap()
    }

    /// **A dispatch's output comes back marked.**
    ///
    /// The acceptance criterion, asserted where it actually happens.
    /// The chain is `reify` → `material_hash` → `disclosure_stamp`, and
    /// this drives the last link over the row state the middle one
    /// leaves behind: an artefact carrying the dispatch trace, a
    /// material whose `meta_kv` names a generator, and a real file on
    /// disk.
    ///
    /// It is the last link that had to be proven separately, because
    /// the first attempt at this feature stamped at export time —
    /// before any fingerprint existed — where the evidence set is empty
    /// and the writer correctly does nothing. A test that only checked
    /// "the export succeeded" was green for a build that marked no file
    /// at all.
    #[tokio::test]
    async fn a_dispatch_output_comes_back_marked() {
        use asterism_core::domain::asset::Asset;
        use asterism_core::domain::attribution::AttributionContext;
        use asterism_core::domain::disclosure::PromptDisclosure;
        use asterism_core::domain::material::Material;
        use asterism_core::domain::persona::Persona;
        use asterism_core::domain::repository::{
            AssetRepository, MaterialFingerprint, PersonaRepository,
        };
        use asterism_core::domain::value::{Modality, SourceKind, SourceRef};

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let assets = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let edges = Arc::new(SqliteEdgeRepository::new(isle.clone()));
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));

        let unattributed = AttributionContext::asserted(None, None).unwrap();
        let persona = Persona::new("P", None).unwrap();
        personas.save(&persona).await.unwrap();

        // The file a dispatch left in its outbox.
        let dir = tempfile::tempdir().unwrap();
        let exported = dir.path().join("out.png");
        std::fs::write(&exported, png_bytes()).unwrap();
        assert_eq!(
            packet_of(&exported),
            None,
            "the fixture starts with nothing to read"
        );

        let source = SourceRef::new(
            SourceKind::new(SourceKind::FS).unwrap(),
            exported.to_string_lossy(),
        )
        .unwrap();
        let mut asset = Asset::new(
            persona.id,
            source.clone(),
            Some(Modality::new("image").unwrap()),
            chrono::Utc::now(),
            &unattributed,
        );
        asset
            .attach_material(Material::primary(
                asset.source.locator.clone(),
                None,
                asset.created_at,
            ))
            .unwrap();
        // What `reify_one` writes, and the only thing that lets this
        // artefact be stamped at all.
        asset.extra = serde_json::json!({
            "_dispatch": {
                "selection_id": "01930000-0000-7000-8000-000000000001",
                "dispatch_id": "01930000-0000-7000-8000-000000000002",
                "exporter_slug": "file",
            }
        });
        assets.save(&asset).await.unwrap();
        // What `material_hash` writes, and the reason the stamp cannot
        // happen before it: without this the evidence set is empty and
        // the mapping establishes nothing.
        assets
            .set_material_fingerprint(
                &asset.id,
                0,
                &MaterialFingerprint {
                    file: "unhashable:no-bytes".into(),
                    content: "unhashable:no-bytes".into(),
                    meta: "m1-sha256:0".into(),
                    meta_kv: Some(
                        serde_json::json!({ "Software": "ComfyUI", "workflow": "{}" }).to_string(),
                    ),
                    meta_raw: None,
                    meta_text: None,
                },
            )
            .await
            .unwrap();

        let disclosure = Arc::new(std::sync::OnceLock::new());
        let _ = disclosure.set(Arc::new(
            asterism_core::application::disclosure_service::DisclosureService::new(
                assets.clone(),
                edges.clone(),
                Arc::new(crate::disclosure::DisclosureWriter::unsigned()),
                // The composition root's answer, so the fixture exercises
                // what a deployment runs rather than a friendlier setting.
                PromptDisclosure::Withhold,
            ),
        ));

        let env = JobEnv {
            deps: test_deps(&isle, disclosure.clone()).await,
            queue: open_queue(pool).await.unwrap(),
        };
        let outcome = handlers::disclosure_stamp(
            &env,
            &serde_json::json!({ "asset_id": asset.id.to_string() }),
        )
        .await
        .expect("a stamp is never a failed job");
        assert_eq!(outcome, "disclosed=true failures=0", "{outcome}");

        let packet =
            packet_of(&exported).expect("the file carries a packet it did not have before");
        assert!(
            packet.contains("trainedAlgorithmicMedia"),
            "and the packet says what the row established: {packet}"
        );

        // And the row says so. A mark lives in the file's bytes, which
        // a downstream conversion can strip; without this note there is
        // nothing to ask about what happened and nothing to re-apply
        // from.
        let stored = assets.find(&asset.id).await.unwrap().unwrap();
        let note = stored.extra["_trace"]["disclosure"].clone();
        assert_eq!(note["discloses"], serde_json::json!(true), "{note}");
        assert_eq!(
            note["xmp"],
            serde_json::json!({ "state": "written" }),
            "{note}"
        );
        assert_eq!(
            note["manifest"],
            serde_json::json!({ "state": "skipped", "reason": "no_signing_identity" }),
            "an unsigned build says why, rather than reporting a failure: {note}"
        );
        assert!(note["at"].is_i64(), "{note}");

        // The dispatch trace it was written beside is still there: the
        // narrow write replaces one field of a shared bag, and the key
        // that decides whether this artefact may be stamped at all is
        // one of its neighbours.
        assert!(
            stored.extra["_dispatch"]["dispatch_id"].is_string(),
            "{}",
            stored.extra
        );
    }

    /// **Two writers of `_trace` do not evict each other.**
    ///
    /// The property the narrow write acquired when it stopped being
    /// about one key. `_trace` is a shared bag whose writers do not
    /// know about each other — the declared-hash verdict, the
    /// disclosure note, a fold, an absorption — and the whole reason
    /// the merge reads before it writes is that each of them must find
    /// its neighbours intact afterwards. Nothing asserted that until
    /// there were two keys to assert it with.
    #[tokio::test]
    async fn one_trace_writer_does_not_evict_another() {
        use asterism_core::domain::asset::Asset;
        use asterism_core::domain::attribution::AttributionContext;
        use asterism_core::domain::content_hash::DECLARED_HASH_NOTE_KEY;
        use asterism_core::domain::disclosure::DISCLOSURE_NOTE_KEY;
        use asterism_core::domain::persona::Persona;
        use asterism_core::domain::repository::{AssetRepository, PersonaRepository};
        use asterism_core::domain::value::{SourceKind, SourceRef};

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let assets = SqliteAssetRepository::new(isle.clone());
        let personas = crate::sqlite::repo::SqlitePersonaRepository::new(isle.clone());

        let persona = Persona::new("P", None).unwrap();
        personas.save(&persona).await.unwrap();
        let asset = Asset::new(
            persona.id,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/tmp/x.png").unwrap(),
            None,
            chrono::Utc::now(),
            &AttributionContext::asserted(None, None).unwrap(),
        );
        assets.save(&asset).await.unwrap();

        assert!(
            assets
                .note_trace_field(
                    &asset.id,
                    DECLARED_HASH_NOTE_KEY,
                    serde_json::json!({ "agreed": true }),
                )
                .await
                .unwrap()
        );
        assert!(
            assets
                .note_trace_field(
                    &asset.id,
                    DISCLOSURE_NOTE_KEY,
                    serde_json::json!({ "discloses": true }),
                )
                .await
                .unwrap()
        );

        let stored = assets.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(
            stored.extra["_trace"][DECLARED_HASH_NOTE_KEY],
            serde_json::json!({ "agreed": true }),
            "the first writer's key survived the second: {}",
            stored.extra
        );
        assert_eq!(
            stored.extra["_trace"][DISCLOSURE_NOTE_KEY],
            serde_json::json!({ "discloses": true }),
            "{}",
            stored.extra
        );
    }

    /// With no writer configured the handler skips, and the file it
    /// would have rewritten is left exactly as it was.
    #[tokio::test]
    async fn an_unconfigured_build_stamps_nothing() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let env = JobEnv {
            deps: test_deps(&isle, Arc::new(std::sync::OnceLock::new())).await,
            queue: open_queue(pool).await.unwrap(),
        };
        let outcome = handlers::disclosure_stamp(
            &env,
            &serde_json::json!({ "asset_id": uuid::Uuid::now_v7().to_string() }),
        )
        .await
        .unwrap();
        assert_eq!(outcome, "no writer configured, skipped");
    }

    /// The dependency bundle these two fixtures need, with everything
    /// they do not touch left inert.
    async fn test_deps(
        isle: &rusqlite_isle::AsyncIsle,
        disclosure: Arc<
            std::sync::OnceLock<
                Arc<asterism_core::application::disclosure_service::DisclosureService>,
            >,
        >,
    ) -> JobDeps {
        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        std::mem::forget(tantivy_dir);
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let query_group_service = Arc::new(asterism_core::application::QueryGroupService::new(
            query_groups.clone(),
            personas.clone(),
            assets_shared.clone(),
            groups_shared.clone(),
        ));
        JobDeps {
            emitter: Arc::new(RecordingEmitter {
                records: Mutex::new(Vec::new()),
                notify: tokio::sync::Notify::new(),
            }),
            assets: SqliteAssetRepository::new(isle.clone()),
            tags: SqliteTagRepository::new(isle.clone()),
            edges: SqliteEdgeRepository::new(isle.clone()),
            thumbs: SqliteThumbRepository::new(isle.clone()),
            modalities: SqliteModalityRepository::new(isle.clone()),
            asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
            comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
            search_index,
            source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
            dispatch: Arc::new(std::sync::OnceLock::new()),
            query_group_refresh: Arc::new(
                asterism_core::application_support::QueryGroupRefreshService::new(
                    query_groups.clone(),
                    query_group_service,
                ),
            ),
            query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
            retention_service: Arc::new(std::sync::OnceLock::new()),
            series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
            observations: crate::observe::ObservationStore::new(isle.clone()),
            material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(isle.clone()),
            chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
            previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
            disclosure,
        }
    }

    #[tokio::test]
    async fn enqueue_runs_handler_and_emits_progress() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let emitter = Arc::new(RecordingEmitter {
            records: Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        });
        // Full-text search stubs — tantivy index goes in a temp
        // dir so the harness never touches the real
        // `~/.asterism/tantivy/` state.
        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        // Query-Group refresh service — the CoverGen path this test
        // exercises never touches it, but JobDeps requires the field.
        // Wire it against the same isle so a future test can enqueue a
        // `query_group_refresh` without extra scaffolding.
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let query_group_service = Arc::new(asterism_core::application::QueryGroupService::new(
            query_groups.clone(),
            personas.clone(),
            assets_shared.clone(),
            groups_shared.clone(),
        ));
        let query_group_refresh = Arc::new(
            asterism_core::application_support::QueryGroupRefreshService::new(
                query_groups.clone(),
                query_group_service,
            ),
        );
        let queue = start(
            pool,
            JobDeps {
                emitter: emitter.clone(),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index,
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh,
                query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
                // Left empty: these tests never exercise the retention
                // sweep, and an unbound cell degrades to "no sweep".
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle),
                // Inert here — no preview job runs in this test.
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            None,
        )
        .await
        .unwrap();

        let task_id = queue
            .enqueue(JobKind::CoverGen, serde_json::json!({"asset_id": "x"}))
            .await
            .unwrap();
        assert!(!task_id.is_empty());

        // Wait for the worker to poll, dispatch the handler, and emit.
        tokio::time::timeout(Duration::from_secs(10), emitter.notify.notified())
            .await
            .expect("handler should emit progress within 10s");
        let records = emitter.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].1.current, 1);
    }

    /// The startup dedupe probe distinguishes a queued backfill walk
    /// (`batch: true`) from single-asset jobs of the same kind and
    /// from other kinds' walks. `open_queue` spawns no workers, so
    /// everything enqueued here stays `Pending` — exactly the state
    /// the probe is defined over.
    #[tokio::test]
    async fn pending_batch_probe_sees_only_queued_batch_walks_of_its_kind() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let queue = open_queue(pool).await.unwrap();

        assert!(
            !queue
                .has_pending_batch(JobKind::MaterialHash)
                .await
                .unwrap(),
            "an empty queue has no walk"
        );

        // The ingest fan-out shape: same kind, no `batch` flag.
        queue
            .enqueue(JobKind::MaterialHash, serde_json::json!({"asset_id": "x"}))
            .await
            .unwrap();
        assert!(
            !queue
                .has_pending_batch(JobKind::MaterialHash)
                .await
                .unwrap(),
            "a single-asset job is not a walk"
        );

        queue
            .enqueue(
                JobKind::MaterialHash,
                serde_json::json!({"batch": true, "cursor": null}),
            )
            .await
            .unwrap();
        assert!(
            queue
                .has_pending_batch(JobKind::MaterialHash)
                .await
                .unwrap(),
            "a queued walk page is seen"
        );
        assert!(
            !queue.has_pending_batch(JobKind::CoverGen).await.unwrap(),
            "kinds do not bleed into each other"
        );
    }

    /// The bench driver's drain poll reads this, so what it has to get
    /// right is the counting: an empty queue must read as drained, and
    /// queued work must read as pending rather than as nothing.
    /// `open_queue` spawns no workers, so everything enqueued here
    /// stays `Pending` — the state a mid-import poll is defined over.
    #[tokio::test]
    async fn depth_counts_queued_work_by_status() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let queue = open_queue(pool.clone()).await.unwrap();

        let empty = jobs_depth(&pool).await.unwrap();
        assert_eq!(empty, JobsDepth::default());
        assert_eq!(
            empty.pending + empty.running,
            0,
            "an empty queue is drained"
        );

        for asset in ["x", "y", "z"] {
            queue
                .enqueue(
                    JobKind::ThumbGen,
                    serde_json::json!({"asset_id": asset, "size_px": 256}),
                )
                .await
                .unwrap();
        }
        queue
            .enqueue(JobKind::MaterialHash, serde_json::json!({"asset_id": "x"}))
            .await
            .unwrap();

        let depth = jobs_depth(&pool).await.unwrap();
        // Kinds are deliberately not a dimension here — that is the
        // `json_extract` pass this endpoint exists to avoid.
        assert_eq!(depth.pending, 4);
        assert_eq!(depth.running, 0);
        assert_eq!(depth.done, 0);
        assert_eq!(depth.failed, 0);

        // A row apalis finished leaves `pending` and lands in `done`,
        // which is what makes the poll terminate.
        sqlx::query("UPDATE Jobs SET status = 'Done' WHERE id = (SELECT id FROM Jobs LIMIT 1)")
            .execute(&pool)
            .await
            .unwrap();
        let depth = jobs_depth(&pool).await.unwrap();
        assert_eq!((depth.pending, depth.done), (3, 1));
    }

    /// Captures what `RunRecord` emits, without a global subscriber.
    fn recorded_outcome(f: impl FnOnce(&mut RunRecord) + std::panic::UnwindSafe) -> String {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Default)]
        struct Capture(Arc<Mutex<Vec<String>>>);
        impl tracing::field::Visit for Capture {
            fn record_debug(&mut self, _f: &tracing::field::Field, _v: &dyn std::fmt::Debug) {}
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "outcome" {
                    self.0.lock().unwrap().push(value.to_string());
                }
            }
        }
        struct L(Arc<Mutex<Vec<String>>>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for L {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                event.record(&mut Capture(self.0.clone()));
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(L(seen.clone()));
        tracing::subscriber::with_default(subscriber, || {
            // The panic is swallowed here only so the test can assert
            // on what was recorded; production lets it propagate.
            let _ = std::panic::catch_unwind(|| {
                let mut record = RunRecord {
                    task_id: "t-1".into(),
                    job_kind: "cover_gen".into(),
                    attempt: 1,
                    started: std::time::Instant::now(),
                    outcome: None,
                    detail: String::new(),
                };
                f(&mut record);
            });
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "exactly one row per run");
        seen[0].clone()
    }

    #[test]
    fn a_run_that_unwinds_is_still_recorded() {
        // The runs most worth having are the ones that did not reach a
        // verdict: apalis 0.7 has no panic-catching layer wired here, so
        // without the `Drop` emit a panicking handler would leave no
        // trace anywhere and its `Jobs` row would sit `Running` forever.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = recorded_outcome(|_record| panic!("handler exploded"));
        std::panic::set_hook(previous);
        assert_eq!(outcome, "panicked");
    }

    #[test]
    fn a_run_that_reaches_its_verdict_records_that_verdict() {
        let outcome = recorded_outcome(|record| {
            record.outcome = Some(JobOutcome::Failed);
            record.detail = "boom".into();
        });
        assert_eq!(outcome, "failed");
    }

    #[test]
    fn a_kind_with_no_handler_is_skipped_rather_than_completed() {
        // The no-handler arms return `Ok` so the queue row settles.
        // Recording that as success would make "how often does this
        // fail" — and "did this ever actually run" — unanswerable,
        // which is the question `JobLog` exists for.
        assert_eq!(JobOutcome::of(false, true), JobOutcome::Skipped);
        assert_eq!(JobOutcome::of(true, true), JobOutcome::Skipped);
        assert_eq!(JobOutcome::of(false, false), JobOutcome::Completed);
        assert_eq!(JobOutcome::of(true, false), JobOutcome::Failed);
    }

    #[tokio::test]
    async fn open_queue_enqueues_without_worker() {
        // `open_queue` is the enqueue-only half of `start`: it must set
        // up the storage + indexes and hand back a usable queue with no
        // worker `Monitor` spawned.
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let queue = open_queue(pool).await.unwrap();
        let task_id = queue
            .enqueue(JobKind::CoverGen, serde_json::json!({"asset_id": "x"}))
            .await
            .unwrap();
        assert!(!task_id.is_empty());
    }

    /// Queue stub behind the invalidator — records what the debounced
    /// hook enqueues so the test can pin the W4-a contract.
    struct RecordingQueue {
        jobs: Mutex<Vec<(JobKind, serde_json::Value)>>,
        notify: tokio::sync::Notify,
    }

    #[async_trait]
    impl asterism_core::domain::repository::JobQueue for RecordingQueue {
        async fn enqueue(
            &self,
            kind: JobKind,
            payload: serde_json::Value,
        ) -> Result<String, DomainError> {
            self.jobs.lock().unwrap().push((kind, payload));
            self.notify.notify_one();
            Ok("recorded".into())
        }
    }

    #[tokio::test]
    async fn auto_tag_handler_fires_query_group_invalidator() {
        use asterism_core::application::query_group_invalidation::QueryGroupInvalidator;

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        // Seed a persona + one labelled asset the handler can tag.
        let pid = uuid::Uuid::now_v7();
        let aid = uuid::Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, labels, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[\"alpha\"]', 0, 0, 0)",
                rusqlite::params![
                    aid,
                    pid,
                    crate::sqlite::stored_locator(&format!("a-{aid}.md"))
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // Invalidator cell wired to a recording queue — the handler
        // chain path under test (W4-a).
        let recording = Arc::new(RecordingQueue {
            jobs: Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        });
        let cell = Arc::new(std::sync::OnceLock::new());
        cell.set(QueryGroupInvalidator::new(
            recording.clone() as Arc<dyn asterism_core::domain::repository::JobQueue>
        ))
        .ok();

        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: cell,
                // Left empty: this test never exercises the retention
                // sweep, and an unbound cell degrades to "no sweep".
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                // Inert here — no preview job runs in this test.
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        };

        let msg = handlers::auto_tag(&env, &serde_json::json!({"asset_id": aid.to_string()}))
            .await
            .unwrap();
        assert!(msg.contains("tagged"), "handler ran: {msg}");

        // The invalidator debounces (200 ms) before enqueueing — wait
        // for the recorded enqueue rather than sleeping a fixed time.
        tokio::time::timeout(Duration::from_secs(5), recording.notify.notified())
            .await
            .expect("debounced QueryGroupRefresh should be enqueued");
        let jobs = recording.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1, "one collapsed refresh for the persona");
        assert!(matches!(jobs[0].0, JobKind::QueryGroupRefresh));
        assert_eq!(jobs[0].1["persona_id"], pid.to_string());
    }

    /// The single-doc indexing path enforces the fold rule in Rust,
    /// not in SQL, for the same reason it enforces the trash rule
    /// there: the queue is asynchronous, so the row can be folded
    /// *after* its `index_rebuild` job was enqueued — which is the
    /// normal order of events, since the fold is downstream of the
    /// hash job that ingest enqueues alongside this one.
    ///
    /// The keeper is indexed in the same test with the same fixture,
    /// so "skipped" is a decision about the folded row rather than a
    /// harness that could not index anything.
    #[tokio::test]
    async fn index_rebuild_skips_a_folded_row_and_still_indexes_the_keeper() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();

        // Real files, so the live case actually reaches Tantivy.
        let corpus = tempfile::tempdir().unwrap();
        let keeper_path = corpus.path().join("keeper.md");
        let headstone_path = corpus.path().join("copy.md");
        std::fs::write(&keeper_path, "a body worth indexing").unwrap();
        std::fs::write(&headstone_path, "the same body, second copy").unwrap();

        let pid = uuid::Uuid::now_v7();
        let keeper = uuid::Uuid::now_v7();
        let headstone = uuid::Uuid::now_v7();
        let (keeper_loc, headstone_loc) = (
            keeper_path.to_string_lossy().to_string(),
            headstone_path.to_string_lossy().to_string(),
        );
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            for (id, locator) in [(keeper, &keeper_loc), (headstone, &headstone_loc)] {
                let stored = crate::sqlite::stored_locator(locator);
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, labels, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                    rusqlite::params![id, pid, stored],
                )?;
                // Ingest always writes one, and indexing now reads it:
                // the body cache is for text, and the format is where
                // that is established. An asset with no material row
                // is not a shape `AssetService::add` can produce.
                conn.execute(
                    "INSERT INTO material (asset_id, ord, locator, mime,
                                           created_at, updated_at)
                     VALUES (?1, 0, ?2, 'text/plain', 0, 0)",
                    rusqlite::params![id, stored],
                )?;
            }
            let marked = conn.execute(
                "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
                rusqlite::params![headstone, keeper],
            )?;
            assert_eq!(marked, 1, "the fixture must actually stand a headstone");
            Ok(())
        })
        .await
        .unwrap();

        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        };

        let kept =
            handlers::index_rebuild(&env, &serde_json::json!({"asset_id": keeper.to_string()}))
                .await
                .unwrap();
        assert!(
            kept.starts_with("indexed asset"),
            "the keeper carries the text: {kept}"
        );

        let skipped = handlers::index_rebuild(
            &env,
            &serde_json::json!({"asset_id": headstone.to_string()}),
        )
        .await
        .unwrap();
        assert!(
            skipped.contains("was folded into"),
            "a headstone must not become a search hit: {skipped}"
        );
        assert!(
            !skipped.starts_with("indexed asset"),
            "and must not be reported as indexed: {skipped}"
        );

        // The body cache is the durable half of the same decision: a
        // row that never gets indexed must not leave a cached body
        // behind either.
        use asterism_core::domain::repository::AssetBodyRepository;
        let cached = env
            .deps
            .asset_bodies
            .get(&asterism_core::domain::value::AssetId::from_uuid(headstone))
            .await
            .unwrap();
        assert_eq!(cached, None, "no body cached for a folded row");
    }

    /// Teeth for the defect this gate exists for: a picture's bytes
    /// must not become a search document.
    ///
    /// The fixture points both assets at **the same readable text
    /// file** and differs only in the `material.mime` column. So the
    /// one filed as a picture is skipped while the one filed as text is
    /// indexed — from identical bytes, which is the only way to show
    /// that the decision is the declared format rather than anything
    /// about the content. Reversing it (asserting on a real PNG) would
    /// pass just as well if the reader had simply failed to decode.
    ///
    /// What it guards: `index_rebuild` used to read every asset's
    /// original as lossy UTF-8 under a 4 MiB cap, so a 5,000-file PNG
    /// corpus went into `asset_body` and Tantivy whole — tokenised,
    /// position-indexed, and stored [measured 2026-08-05, bench S preset].
    #[tokio::test]
    async fn a_picture_is_not_read_into_the_body_cache_or_the_index() {
        use asterism_core::domain::repository::AssetBodyRepository;
        use asterism_core::domain::value::AssetId;

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();

        // Two files carrying byte-identical, perfectly readable text.
        // Separate paths because `(source_kind, source_locator)` is
        // unique; the bytes are what has to match, and they do.
        const BODY: &str = "bytes that read perfectly well as text";
        // The picture's only words: a cover somebody's job wrote. It
        // shares no term with `BODY`, so "did the reader run" is a
        // decidable question about the cached body.
        const CAPTION: &str = "a caption naming what the picture shows";
        // A second surface with no column of its own on `asset`: the
        // thread. It has to reach the document through the repository
        // the handler holds, not through the row it loaded.
        const NOTE: &str = "the one we printed for the hallway";
        let corpus = tempfile::tempdir().unwrap();
        let picture_path = corpus.path().join("filed-as-a-picture.png");
        let text_path = corpus.path().join("filed-as-text.txt");
        std::fs::write(&picture_path, BODY).unwrap();
        std::fs::write(&text_path, BODY).unwrap();

        let pid = uuid::Uuid::now_v7();
        let as_picture = uuid::Uuid::now_v7();
        let as_text = uuid::Uuid::now_v7();
        let rows = [
            (
                as_picture,
                picture_path.to_string_lossy().to_string(),
                "image/png",
                Some(CAPTION),
            ),
            (
                as_text,
                text_path.to_string_lossy().to_string(),
                "text/plain",
                None,
            ),
        ];
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            for (id, locator, mime, cover) in &rows {
                let stored = crate::sqlite::stored_locator(locator);
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, labels, cover, occurred_at,
                                        created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', ?4, 0, 0, 0)",
                    rusqlite::params![id, pid, stored, cover],
                )?;
                conn.execute(
                    "INSERT INTO material (asset_id, ord, locator, mime,
                                           created_at, updated_at)
                     VALUES (?1, 0, ?2, ?3, 0, 0)",
                    rusqlite::params![id, stored, mime],
                )?;
            }
            // One note on the picture's thread — for a picture the
            // thread is often the only prose the library holds, so it
            // is part of the document or the picture is unfindable.
            conn.execute(
                "INSERT INTO asset_comment (id, asset_id, author_kind, body, created_at)
                 VALUES (?1, ?2, 'user', ?3, 0)",
                rusqlite::params![uuid::Uuid::now_v7(), as_picture, NOTE],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        };

        let picture = handlers::index_rebuild(
            &env,
            &serde_json::json!({"asset_id": as_picture.to_string()}),
        )
        .await
        .unwrap();
        assert!(
            picture.starts_with("indexed asset"),
            "a picture with words about it is a document: {picture}"
        );

        // The same bytes, filed as text, are read — so what the
        // picture's document is missing is a decision about the format,
        // not a reader that could not have succeeded.
        let text =
            handlers::index_rebuild(&env, &serde_json::json!({"asset_id": as_text.to_string()}))
                .await
                .unwrap();
        assert!(
            text.starts_with("indexed asset"),
            "identical bytes filed as text are indexable: {text}"
        );

        // The durable half, and the actual claim: the picture's cached
        // body is what was said about it — the caption on the row and
        // the note on its thread — and the file it points at was never
        // opened, though it would have read perfectly.
        let picture_body = env
            .deps
            .asset_bodies
            .get(&AssetId::from_uuid(as_picture))
            .await
            .unwrap()
            .expect("a picture with words about it caches a body");
        assert!(
            picture_body.contains(CAPTION),
            "the row's own words are in the document: {picture_body}"
        );
        assert!(
            picture_body.contains(NOTE),
            "the thread reaches the document too: {picture_body}"
        );
        assert!(
            !picture_body.contains(BODY),
            "and the file was never opened: {picture_body}"
        );
        let text_body = env
            .deps
            .asset_bodies
            .get(&AssetId::from_uuid(as_text))
            .await
            .unwrap()
            .expect("the text row is the control: it must actually cache");
        assert!(
            text_body.contains(BODY),
            "identical bytes filed as text are read: {text_body}"
        );
    }

    /// The whole point of the axis, end to end: a prompt a generator
    /// compressed into the file is findable.
    ///
    /// Every earlier test in this change covers one joint — the walk
    /// recovers the chunk, the column round-trips, the composition reads
    /// both metadata columns. None of them fails if the joints are wired
    /// to each other wrongly, and that is exactly the state this work
    /// started from: `hash_artefact` computed the recovered text and the
    /// repository dropped it on the floor, so every unit test passed and
    /// nothing reached a document.
    ///
    /// So this runs the real jobs in the real order —
    /// `material_hash` (reads the bytes, writes the columns) then
    /// `index_rebuild` (composes the document) — over a real PNG on
    /// disk, and asserts on the cached body a search would be built
    /// from.
    ///
    /// `zTXt` deliberately: the meta digest walk excludes compressed
    /// chunks by definition, so a prompt stored this way exists in no
    /// other column. If the recovery is not wired through, there is
    /// nowhere else these words could come from and the assertion is
    /// unambiguous.
    #[tokio::test]
    async fn a_compressed_prompt_reaches_the_body_cache_through_the_real_jobs() {
        use asterism_core::domain::repository::{AssetBodyRepository, AssetRepository};
        use asterism_core::domain::value::AssetId;

        const PROMPT: &str = "a lighthouse at dusk, long exposure";

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();

        let corpus = tempfile::tempdir().unwrap();
        let picture_path = corpus.path().join("generated.png");
        std::fs::write(
            &picture_path,
            pngmeta::test_util::PngBuilder::new()
                .ztxt("parameters", PROMPT)
                .build(),
        )
        .unwrap();

        let pid = uuid::Uuid::now_v7();
        let asset_uuid = uuid::Uuid::now_v7();
        let stored = crate::sqlite::stored_locator(&picture_path.to_string_lossy());
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                rusqlite::params![asset_uuid, pid, stored],
            )?;
            // No hashes on the row: the hashing pass is what this test
            // is asking to run, and a row that already had them would
            // be skipped by it.
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at) \
                 VALUES (?1, 0, ?2, 'image/png', 0, 0)",
                rusqlite::params![asset_uuid, stored],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        };

        let asset_id = AssetId::from_uuid(asset_uuid);
        let hashed = handlers::material_hash(
            &env,
            &serde_json::json!({"asset_id": asset_uuid.to_string()}),
        )
        .await
        .unwrap();
        assert!(
            hashed.contains("hashed=1"),
            "the pass has to read the bytes for anything downstream to exist: {hashed}"
        );

        // The joint that was missing: the reading survives as a column.
        let stored_row = env
            .deps
            .assets
            .find(&asset_id)
            .await
            .unwrap()
            .expect("the row is there");
        let recovered = stored_row.materials[0]
            .meta_text
            .as_deref()
            .expect("the hashing pass writes what it recovered");
        assert!(
            recovered.contains(PROMPT),
            "the compressed prompt is on the row: {recovered}"
        );
        assert_eq!(
            stored_row.materials[0].meta_kv, None,
            "and the digest's own column is empty, because the meta axis does not read zTXt \
             — so the words below can only have come from the recovery"
        );

        let indexed = handlers::index_rebuild(
            &env,
            &serde_json::json!({"asset_id": asset_uuid.to_string()}),
        )
        .await
        .unwrap();
        assert!(
            indexed.starts_with("indexed asset"),
            "a picture whose file carries a prompt is a document: {indexed}"
        );

        let body = env
            .deps
            .asset_bodies
            .get(&asset_id)
            .await
            .unwrap()
            .expect("the composed document is cached");
        assert!(
            body.contains(PROMPT),
            "the prompt somebody typed is what search is built from: {body}"
        );
    }

    /// The two effects a fold has outside its transaction: the
    /// headstone leaves the search index, and the Query Groups of its
    /// persona are told to re-evaluate (its Group membership and tag
    /// links just moved, and it left the live population entirely).
    ///
    /// Both are asserted against real machinery rather than a spy on
    /// the handler — a fresh read-only handle on the same index
    /// directory sees exactly what was committed, which is what the
    /// search path would see.
    #[tokio::test]
    async fn folding_takes_the_headstone_out_of_search_and_tells_the_query_groups() {
        use asterism_core::application::query_group_invalidation::QueryGroupInvalidator;
        use asterism_core::domain::repository::{
            AssetIndexer, AssetRetriever, IndexDoc, RetrievalIntent, RetrievalQuery,
        };
        use asterism_core::domain::value::{AssetId, PersonaId};

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();

        let pid = uuid::Uuid::now_v7();
        let keeper = uuid::Uuid::now_v7();
        let headstone = uuid::Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            for (id, locator) in [(keeper, "/pics/keeper.png"), (headstone, "/pics/copy.png")] {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, labels, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                    rusqlite::params![id, pid, crate::sqlite::stored_locator(locator)],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        // The headstone is in the index before the fold — without this
        // the assertion afterwards would hold over a document that
        // never existed.
        search_index
            .upsert(&IndexDoc {
                asset_id: AssetId::from_uuid(headstone),
                persona_id: PersonaId::from_uuid(pid),
                text: Some("heliotrope marginalia".to_string()),
            })
            .await
            .unwrap();
        search_index.flush().await.unwrap();
        let probe = |dir: std::path::PathBuf| async move {
            crate::search::TantivyIndex::open_read_only(dir)
                .unwrap()
                .retrieve(&RetrievalQuery {
                    intent: RetrievalIntent::Text("heliotrope".to_string()),
                    scope: None,
                    k: 10,
                })
                .await
                .unwrap()
        };
        let before = probe(tantivy_dir.path().to_path_buf()).await;
        assert_eq!(
            before
                .candidates
                .iter()
                .map(|c| c.asset_id)
                .collect::<Vec<_>>(),
            vec![AssetId::from_uuid(headstone)],
            "the fixture must actually put the row in the index"
        );

        let recording = Arc::new(RecordingQueue {
            jobs: Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        });
        let cell = Arc::new(std::sync::OnceLock::new());
        cell.set(QueryGroupInvalidator::new(
            recording.clone() as Arc<dyn asterism_core::domain::repository::JobQueue>
        ))
        .ok();

        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: cell,
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool.clone()).await.unwrap(),
        };

        let message = handlers::asset_fold(
            &env,
            &serde_json::json!({
                "asset_id": headstone.to_string(),
                "keeper_id": keeper.to_string(),
            }),
        )
        .await
        .unwrap();
        assert!(
            message.contains("unindexed=true"),
            "the handler reports what it did to the index: {message}"
        );

        // The other half of a fold, and the half the handler's message
        // does not report: the keeper now holds the headstone's
        // keywords, labels and comment thread, so its document was
        // composed from less than the row says. Asserted here because
        // nothing else can — `fold_symmetry_e2e` counts the queue at
        // the instant of each service call, which is before this job
        // has run, and the manual path's own enqueue is the one it
        // sees. Delete the enqueue in `handlers::asset_fold` and this
        // is the test that fails.
        let recomposed: Vec<String> = sqlx::query_scalar(
            "SELECT json_extract(job, '$.payload.asset_id') FROM Jobs \
              WHERE json_extract(job, '$.kind') = 'index_rebuild'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            recomposed,
            vec![keeper.to_string()],
            "the keeper of an automatic fold is queued for re-composition, \
             and the headstone is not"
        );

        let after = probe(tantivy_dir.path().to_path_buf()).await;
        assert!(
            after.candidates.is_empty(),
            "a headstone must not stay retrievable: {after:?}"
        );

        // The invalidator debounces per persona, so wait for the
        // enqueue rather than sleeping a fixed time.
        tokio::time::timeout(Duration::from_secs(5), recording.notify.notified())
            .await
            .expect("a fold must invalidate the persona's query groups");
        let jobs = recording.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1, "one collapsed refresh for the persona");
        assert!(matches!(jobs[0].0, JobKind::QueryGroupRefresh));
        assert_eq!(jobs[0].1["persona_id"], pid.to_string());
    }

    /// **A fold job over a row that is already a headstone still takes
    /// it out of search.**
    ///
    /// This is the branch the manual merge depends on. `merge_into`
    /// folds inside its own transaction and then `merge_assets` enqueues
    /// this job for the half that lives outside one, so the job always
    /// arrives at a row that `fold_into` will refuse with
    /// `AlreadyFolded`. Skipping on that refusal — which is what the
    /// handler used to do — leaves the manual path's enqueue doing
    /// nothing at all.
    ///
    /// The fixture folds through the **real** `merge_into` rather than
    /// writing `folded_into` by hand: the state under test is the one a
    /// merge leaves behind, and a hand-written column could be a state
    /// no verb produces.
    #[tokio::test]
    async fn a_fold_job_over_an_already_folded_row_still_retires_it() {
        use asterism_core::application::query_group_invalidation::QueryGroupInvalidator;
        use asterism_core::domain::merge_plan::MergePlan;
        use asterism_core::domain::repository::{
            AssetIndexer, AssetRepository, AssetRetriever, IndexDoc, RetrievalIntent,
            RetrievalQuery,
        };
        use asterism_core::domain::value::{AssetId, PersonaId};

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();

        let pid = uuid::Uuid::now_v7();
        let keeper = uuid::Uuid::now_v7();
        let headstone = uuid::Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            for (id, locator) in [(keeper, "/pics/keeper.png"), (headstone, "/pics/copy.png")] {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, labels, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                    rusqlite::params![id, pid, crate::sqlite::stored_locator(locator)],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        search_index
            .upsert(&IndexDoc {
                asset_id: AssetId::from_uuid(headstone),
                persona_id: PersonaId::from_uuid(pid),
                text: Some("heliotrope marginalia".to_string()),
            })
            .await
            .unwrap();
        search_index.flush().await.unwrap();
        let probe = |dir: std::path::PathBuf| async move {
            crate::search::TantivyIndex::open_read_only(dir)
                .unwrap()
                .retrieve(&RetrievalQuery {
                    intent: RetrievalIntent::Text("heliotrope".to_string()),
                    scope: None,
                    k: 10,
                })
                .await
                .unwrap()
        };

        // The merge does the fold, exactly as `merge_assets` would, and
        // leaves the document behind — it has no index port of its own.
        let assets = SqliteAssetRepository::new(isle.clone());
        let keeper_id = AssetId::from_uuid(keeper);
        let headstone_id = AssetId::from_uuid(headstone);
        let plan =
            MergePlan::declare(keeper_id, vec![headstone_id], &[keeper_id, headstone_id]).unwrap();
        let outcome = assets.merge_into(&plan, false).await.unwrap();
        assert!(
            outcome.committed && outcome.folded == vec![headstone_id],
            "the fixture must actually fold through the merge: {outcome:?}"
        );
        let before = probe(tantivy_dir.path().to_path_buf()).await;
        assert_eq!(
            before
                .candidates
                .iter()
                .map(|c| c.asset_id)
                .collect::<Vec<_>>(),
            vec![headstone_id],
            "a merge leaves the document standing — that is what this job is for"
        );

        let recording = Arc::new(RecordingQueue {
            jobs: Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        });
        let cell = Arc::new(std::sync::OnceLock::new());
        cell.set(QueryGroupInvalidator::new(
            recording.clone() as Arc<dyn asterism_core::domain::repository::JobQueue>
        ))
        .ok();

        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: cell,
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        };

        let message = handlers::asset_fold(
            &env,
            &serde_json::json!({
                "asset_id": headstone.to_string(),
                "keeper_id": keeper.to_string(),
            }),
        )
        .await
        .unwrap();
        assert!(
            message.contains("already folded") && message.contains("unindexed=true"),
            "the job says it found the row standing and stood it down anyway: {message}"
        );

        let after = probe(tantivy_dir.path().to_path_buf()).await;
        assert!(
            after.candidates.is_empty(),
            "a headstone must not stay retrievable, whoever folded it: {after:?}"
        );

        // The other half of the outside-the-transaction work, owed for
        // the same reason: the row left the live population.
        tokio::time::timeout(Duration::from_secs(5), recording.notify.notified())
            .await
            .expect("the persona's query groups must be told");
        let jobs = recording.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(jobs[0].0, JobKind::QueryGroupRefresh));
        assert_eq!(jobs[0].1["persona_id"], pid.to_string());
    }

    /// A refused fold is reported, not raised as an error: the job
    /// engine has no retries, so an error here would be a dead row in
    /// the queue for a question that is already answered.
    #[tokio::test]
    async fn a_fold_of_a_row_that_is_gone_is_reported_rather_than_failed() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let tantivy_dir = tempfile::tempdir().unwrap();
        let search_index =
            Arc::new(crate::search::TantivyIndex::open(tantivy_dir.path().to_path_buf()).unwrap());
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        let env = JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: search_index.clone(),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        };

        let message = handlers::asset_fold(
            &env,
            &serde_json::json!({
                "asset_id": uuid::Uuid::now_v7().to_string(),
                "keeper_id": uuid::Uuid::now_v7().to_string(),
            }),
        )
        .await
        .unwrap();
        assert!(
            message.contains("skipped") && message.contains("no such asset"),
            "the refusal says which half of the re-read failed: {message}"
        );

        // A missing payload half is a different matter — that is a
        // malformed job, and it fails.
        let err = handlers::asset_fold(
            &env,
            &serde_json::json!({"asset_id": uuid::Uuid::now_v7().to_string()}),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("keeper_id"), "{err}");
    }

    // ---- the series axis -------------------------------------------

    /// A `JobEnv` over one in-memory database. The caller owns the
    /// search directory so it outlives the index handle.
    async fn series_job_env(
        isle: &rusqlite_isle::AsyncIsle,
        pool: SqlitePool,
        search_dir: &std::path::Path,
    ) -> JobEnv {
        let query_groups = Arc::new(
            crate::sqlite::repo::query_group::SqliteQueryGroupRepository::new(isle.clone()),
        );
        let personas = Arc::new(crate::sqlite::repo::SqlitePersonaRepository::new(
            isle.clone(),
        ));
        let assets_shared = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let groups_shared = Arc::new(crate::sqlite::repo::group::SqliteGroupRepository::new(
            isle.clone(),
        ));
        JobEnv {
            deps: JobDeps {
                emitter: Arc::new(RecordingEmitter {
                    records: Mutex::new(Vec::new()),
                    notify: tokio::sync::Notify::new(),
                }),
                assets: SqliteAssetRepository::new(isle.clone()),
                tags: SqliteTagRepository::new(isle.clone()),
                edges: SqliteEdgeRepository::new(isle.clone()),
                thumbs: SqliteThumbRepository::new(isle.clone()),
                modalities: SqliteModalityRepository::new(isle.clone()),
                asset_bodies: crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone()),
                comments: crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone()),
                search_index: Arc::new(
                    crate::search::TantivyIndex::open(search_dir.to_path_buf()).unwrap(),
                ),
                source_texts: Arc::new(crate::source_text::FsSourceTextReader::new()),
                dispatch: Arc::new(std::sync::OnceLock::new()),
                query_group_refresh: Arc::new(
                    asterism_core::application_support::QueryGroupRefreshService::new(
                        query_groups.clone(),
                        Arc::new(asterism_core::application::QueryGroupService::new(
                            query_groups,
                            personas,
                            assets_shared,
                            groups_shared,
                        )),
                    ),
                ),
                query_group_invalidator: Arc::new(std::sync::OnceLock::new()),
                retention_service: Arc::new(std::sync::OnceLock::new()),
                series: crate::sqlite::repo::SqliteSeriesRepository::new(isle.clone()),
                observations: crate::observe::ObservationStore::new(isle.clone()),
                material_layers: crate::sqlite::repo::SqliteMaterialLayerRepository::new(
                    isle.clone(),
                ),
                chapter_marks: crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone()),
                previews_dir: std::env::temp_dir().join("asterism-jobs-test-previews"),
                disclosure: Arc::new(std::sync::OnceLock::new()),
            },
            queue: open_queue(pool).await.unwrap(),
        }
    }

    /// One persona to hang the fixtures off.
    async fn seed_persona(isle: &rusqlite_isle::AsyncIsle) -> uuid::Uuid {
        let pid = uuid::Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'p', 'P', 0, 0)",
                rusqlite::params![pid],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        pid
    }

    /// An asset whose primary material carries the metadata a walk would
    /// have written — the state `series_derive` finds a library in.
    ///
    /// `meta_hash` is filled beside `meta_kv` because the two travel
    /// together (`Material::meta_kv` is `Some` exactly when the digest
    /// is one); nothing under test reads it, and a fixture that split
    /// the pair would be a state no walk produces.
    async fn seed_walked_material(
        isle: &rusqlite_isle::AsyncIsle,
        persona: uuid::Uuid,
        mime: &str,
        meta_kv: Option<String>,
    ) -> uuid::Uuid {
        let aid = uuid::Uuid::now_v7();
        let mime = mime.to_string();
        isle.call(move |conn| {
            let stored = crate::sqlite::stored_locator(&format!("/pics/{aid}.png"));
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, labels, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                rusqlite::params![aid, persona, stored],
            )?;
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, meta_hash, meta_kv,
                                       created_at, updated_at)
                 VALUES (?1, 0, ?2, ?3, ?4, ?5, 0, 0)",
                rusqlite::params![
                    aid,
                    stored,
                    mime,
                    meta_kv
                        .as_ref()
                        .map(|_| format!("m1-sha256:{}", "e".repeat(64))),
                    meta_kv
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        aid
    }

    /// A secondary original beside the primary — `ord = 1`, its own
    /// container and its own metadata. See
    /// `a_secondary_original_is_its_own_pair_and_gets_its_own_key` in the
    /// adapter for why the fixture has to exist somewhere.
    async fn attach_secondary_walked_material(
        isle: &rusqlite_isle::AsyncIsle,
        asset: uuid::Uuid,
        mime: &str,
        meta_kv: String,
    ) {
        let mime = mime.to_string();
        isle.call(move |conn| {
            let stored = crate::sqlite::stored_locator(&format!("/pics/{asset}.raw"));
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, meta_hash, meta_kv,
                                       created_at, updated_at)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5, 0, 0)",
                rusqlite::params![
                    asset,
                    stored,
                    mime,
                    format!("m1-sha256:{}", "f".repeat(64)),
                    meta_kv
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// `count` assets, each with one walked original, in one call — the
    /// shape a page-boundary fixture needs without `count` round trips.
    async fn seed_many_walked_materials(
        isle: &rusqlite_isle::AsyncIsle,
        persona: uuid::Uuid,
        count: usize,
    ) {
        isle.call(move |conn| {
            for seed in 0..count {
                let aid = uuid::Uuid::now_v7();
                let stored = crate::sqlite::stored_locator(&format!("/pics/{aid}.png"));
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, labels, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                    rusqlite::params![aid, persona, stored],
                )?;
                conn.execute(
                    "INSERT INTO material (asset_id, ord, locator, mime, meta_hash, meta_kv,
                                           created_at, updated_at)
                     VALUES (?1, 0, ?2, 'image/png', ?3, ?4, 0, 0)",
                    rusqlite::params![
                        aid,
                        stored,
                        format!("m1-sha256:{}", "e".repeat(64)),
                        vdsl_container(Some("phase8_hires.lua"), seed as u64)
                    ],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A PNG container as the walk records it: the generator's chunk,
    /// and a `prompt` chunk that differs per image.
    ///
    /// The `prompt` is what makes the grouping assertions mean anything
    /// — it is why all eleven measured images are distinct under
    /// every digest and every exclusion, and a fixture holding it
    /// constant would let a handler that groups by accident pass.
    /// `script: None` is the generator before it wrote one (v0.3.0):
    /// the keyword is there and the path resolves nowhere.
    fn vdsl_container(script: Option<&str>, seed: u64) -> String {
        let chunk = |value: serde_json::Value| {
            serde_json::Value::String(
                serde_json::to_string(&value).expect("built by the serialiser"),
            )
        };
        let vdsl = match script {
            Some(script) => serde_json::json!({
                "script": script,
                "timestamp": "2026-04-26T15:48:29.514778+09:00",
                "version": "0.4.0",
            }),
            None => serde_json::json!({
                "timestamp": "2026-04-26T15:48:29.514778+09:00",
                "version": "0.3.0",
            }),
        };
        serde_json::to_string(&serde_json::json!({
            "prompt": chunk(serde_json::json!({"3": {"inputs": {"seed": seed}}})),
            "vdsl": chunk(vdsl),
        }))
        .expect("built by the serialiser")
    }

    /// Every derived row, as `(outcome, key)`.
    async fn filed_series(isle: &rusqlite_isle::AsyncIsle) -> Vec<(String, Option<String>)> {
        isle.call(|conn| {
            let mut stmt =
                conn.prepare("SELECT outcome, key FROM material_series ORDER BY outcome, key")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .unwrap()
    }

    /// **The walk ends**, and it ends because every pair gets a row —
    /// including the two answers that are not keys.
    ///
    /// The fixture is one material per outcome plus one that is not a
    /// pair at all:
    ///
    /// - two PNGs off one run and one off another → `derived`;
    /// - a PNG whose generator predates the `script` field → the keyword
    ///   is there and the path resolves nowhere, `nothing_to_select`;
    /// - **a JPEG carrying the same chunk** → the rule is written against
    ///   PNG, `not_applicable`. This is the row the termination claim
    ///   rests on: it can never become a key, so if declining did not
    ///   file a row it would come back on every page forever;
    /// - a material nothing has walked (`meta_kv IS NULL`) → no pair, and
    ///   no row.
    ///
    /// Checked by mutation on 2026-08-10 by filing only `Derived` (an
    /// early `return` in `DerivedTally::record` for the other two arms).
    /// The second pass then reported *"series_derive pass: derived=0
    /// empty=1 not_applicable=1 …"* instead of "nothing left to derive",
    /// and a third pass reported the same again — the walk had stopped
    /// shrinking, and a full page would have chain-enqueued itself for
    /// the life of the process. Restored, the second pass is empty.
    #[tokio::test]
    async fn the_series_walk_terminates_because_a_declined_pair_is_still_answered() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool, search_dir.path()).await;
        let persona = seed_persona(&isle).await;

        for (mime, meta_kv) in [
            (
                "image/png",
                Some(vdsl_container(Some("phase8_hires.lua"), 1)),
            ),
            (
                "image/png",
                Some(vdsl_container(Some("phase8_hires.lua"), 2)),
            ),
            (
                "image/png",
                Some(vdsl_container(Some("phase9_portrait.lua"), 3)),
            ),
            ("image/png", Some(vdsl_container(None, 4))),
            (
                "image/jpeg",
                Some(vdsl_container(Some("phase8_hires.lua"), 5)),
            ),
            ("image/png", None),
        ] {
            seed_walked_material(&isle, persona, mime, meta_kv).await;
        }

        let first = handlers::series_derive(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert!(
            first.contains("derived=3 empty=1 not_applicable=1 failed=0"),
            "one pass answers every pair the one seeded rule is offered: {first}"
        );

        let second = handlers::series_derive(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert_eq!(
            second, "series_derive pass: nothing left to derive",
            "the walk has to empty — every pair was answered"
        );

        // The durable half, so "answered" is not just what the message
        // said: five rows, and the two silences are filed as themselves.
        let rows = filed_series(&isle).await;
        assert_eq!(
            rows.len(),
            5,
            "one row per pair, and none for the unwalked material: {rows:#?}"
        );
        assert_eq!(
            rows.iter()
                .filter(|(outcome, _)| outcome == "not_applicable")
                .count(),
            1,
            "the JPEG's declined pair is a row: {rows:#?}"
        );
        assert_eq!(
            rows.iter()
                .filter(|(outcome, _)| outcome == "nothing_to_select")
                .count(),
            1,
            "the rule ran on the older generator's chunk and found nothing: {rows:#?}"
        );
        assert!(
            rows.iter()
                .filter(|(outcome, _)| outcome == "derived")
                .all(|(_, key)| key.as_deref().is_some_and(|k| k.starts_with("sk1-sha256:"))),
            "{rows:#?}"
        );
    }

    /// The VDSL measurement, reproduced **through the database**:
    /// eleven images off two runs land on two keys, five and six.
    ///
    /// `domain/series.rs` freezes this over the pure function. What is
    /// added here is everything between: the rule as V73 seeded it and
    /// the adapter read it back, the containers as `meta_kv` columns, the
    /// walk that pairs them, and the keys as `material_series` rows. A
    /// path list flattened in storage, a rule the walk never offered, a
    /// key written under the wrong pair — none of those can be seen from
    /// the pure side, and each of them lands here as a group count.
    ///
    /// The split is asserted rather than the number of groups: "two
    /// groups" is also what a handler that grouped by the `timestamp`
    /// would produce, and `[5, 6]` is not.
    ///
    /// Checked by mutation on 2026-08-10 by seeding the rule with an
    /// empty `include` — the whole container, which is what the
    /// measurement tried first. This failed with *left `[1, 1, 1, 1, 1,
    /// 1, 1, 1, 1, 1, 1]`, right `[5, 6]`*: the eleven images separate
    /// because the per-image `prompt` chunk is still in the selection,
    /// which is the finding that turned this axis from a denylist into a
    /// selection, reproduced here through the column and the walk.
    #[tokio::test]
    async fn the_walk_reproduces_the_measured_five_and_six_through_the_database() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool, search_dir.path()).await;
        let persona = seed_persona(&isle).await;

        for seed in 0..5 {
            let meta = vdsl_container(Some("phase8_hires.lua"), 1_000 + seed);
            seed_walked_material(&isle, persona, "image/png", Some(meta)).await;
        }
        for seed in 0..6 {
            let meta = vdsl_container(Some("phase9_portrait.lua"), 2_000 + seed);
            seed_walked_material(&isle, persona, "image/png", Some(meta)).await;
        }

        let message = handlers::series_derive(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert!(
            message.contains("derived=11"),
            "every image is a key under this rule: {message}"
        );

        let mut sizes: Vec<i64> = isle
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM material_series \
                      WHERE outcome = 'derived' GROUP BY key",
                )?;
                stmt.query_map([], |r| r.get(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .unwrap();
        sizes.sort_unstable();
        assert_eq!(
            sizes,
            vec![5, 6],
            "selecting the recipe recovers the two runs — through the column, \
             the rule as stored, and the walk"
        );
    }

    /// The per-asset pass answers **every** original of the asset, not
    /// only the primary.
    ///
    /// `series_derive`'s per-asset arm loops the materials and the walk's
    /// population is materials, so both have to hold; this is the
    /// handler-side half of the adapter's
    /// `a_secondary_original_is_its_own_pair_and_gets_its_own_key`.
    /// Without it an `if material.ord != 0 { continue }` — which reads as
    /// harmless while every asset has one original — would pass.
    ///
    /// The two originals carry different recipes, so "both answered" is
    /// two different keys rather than two rows.
    #[tokio::test]
    async fn the_per_asset_pass_answers_every_original_not_only_the_primary() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool, search_dir.path()).await;
        let persona = seed_persona(&isle).await;

        let asset = seed_walked_material(
            &isle,
            persona,
            "image/png",
            Some(vdsl_container(Some("phase8_hires.lua"), 1)),
        )
        .await;
        attach_secondary_walked_material(
            &isle,
            asset,
            "image/png",
            vdsl_container(Some("phase9_portrait.lua"), 2),
        )
        .await;

        let message =
            handlers::series_derive(&env, &serde_json::json!({"asset_id": asset.to_string()}))
                .await
                .unwrap();
        assert!(
            message.contains("derived=2"),
            "one rule, two originals, two answers: {message}"
        );

        let filed: Vec<(i64, Option<String>)> = isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT ord, key FROM material_series WHERE asset_id = ?1 ORDER BY ord",
                )?;
                stmt.query_map(rusqlite::params![asset], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .unwrap();
        assert_eq!(
            filed.iter().map(|(ord, _)| *ord).collect::<Vec<_>>(),
            vec![0, 1],
            "an answer per original: {filed:#?}"
        );
        assert_ne!(
            filed[0].1, filed[1].1,
            "each original was read as itself — the recipes differ, so the keys do"
        );
        // And the walk agrees there is nothing left, so the per-asset
        // pass and the walk have the same idea of what a pair is.
        let swept = handlers::series_derive(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert_eq!(swept, "series_derive pass: nothing left to derive");
    }

    /// A full page chains, and **the cursor it enqueues is the one it
    /// parses**.
    ///
    /// Everything else here runs against a handful of pairs, so `full` is
    /// always false and the chain branch — with the payload it writes and
    /// the parse that reads it back — never runs at all. The fixture is
    /// `SERIES_DERIVE_PAGE + 1` pairs, the real page size rather than a
    /// test-only knob, because a knob production never turns is how the
    /// real value goes untested.
    ///
    /// The assertion is on the enqueued payload rather than only on the
    /// second page's count, and that is the point: a chained cursor that
    /// fails to parse falls to `None`, which restarts the walk from the
    /// beginning — and since the answered pairs have rows, the *answers*
    /// come out identical while the walk goes quadratic in the number of
    /// pages. Nothing about the outcome can see that. The wire shape can.
    ///
    /// Checked by mutation on 2026-08-10 by renaming the enqueued
    /// `"strategy_id"` key to `"strategy"`: *left `Null`, right
    /// `"019fe8f8-1400-7000-8000-000000000001"`*. Every assertion about
    /// the *answers* still passed under that edit, which is the reason
    /// this test reads the payload.
    #[tokio::test]
    async fn a_full_page_chains_and_the_cursor_it_writes_is_the_one_it_reads() {
        use asterism_core::domain::repository::SeriesRepository as _;

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool.clone(), search_dir.path()).await;
        let persona = seed_persona(&isle).await;

        let page_size = handlers::SERIES_DERIVE_PAGE;
        seed_many_walked_materials(&isle, persona, page_size as usize + 1).await;

        // Where the page ends, read through the same statement the
        // handler runs — so the expectation is the walk's own ordering
        // rather than a second guess at it.
        let first_page = env
            .deps
            .series
            .scan_underived(None, page_size)
            .await
            .unwrap();
        assert_eq!(first_page.len(), page_size as usize, "the page fills");
        let last = first_page.last().expect("page is non-empty");
        let (last_asset, last_ord, last_rule) = (last.asset_id, last.ord, last.strategy.id);

        let first = handlers::series_derive(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert!(
            first.contains(&format!("derived={page_size}")) && first.contains("more=true"),
            "a full page reports itself full: {first}"
        );

        let queued: Vec<String> = sqlx::query_scalar(
            "SELECT json_extract(job, '$.payload') FROM Jobs \
              WHERE json_extract(job, '$.kind') = 'series_derive'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(queued.len(), 1, "one chained page, not none and not two");
        let chained: serde_json::Value = serde_json::from_str(&queued[0]).unwrap();
        assert_eq!(chained["batch"], true);
        assert_eq!(chained["cursor"]["asset_id"], last_asset.to_string());
        assert_eq!(chained["cursor"]["ord"], last_ord);
        assert_eq!(
            chained["cursor"]["strategy_id"],
            last_rule.to_string(),
            "the cursor names the pair the page ended on, in the shape the parse reads"
        );

        // Running exactly what it enqueued finishes the walk, and stops.
        let second = handlers::series_derive(&env, &chained).await.unwrap();
        assert!(
            second.contains("derived=1") && second.contains("more=false"),
            "the chained page picks up where the first left off: {second}"
        );
        let queued_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM Jobs WHERE json_extract(job, '$.kind') = 'series_derive'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(queued_after, 1, "a short page chains nothing");

        let third = handlers::series_derive(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert_eq!(third, "series_derive pass: nothing left to derive");
    }

    /// The per-asset entry point, end to end: the fingerprint pass writes
    /// a container's metadata and asks for its keys in the same breath.
    ///
    /// Without this an imported file carries no key until the next start,
    /// which is the whole difference between an axis that is derived and
    /// one that is derived eventually. The enqueue is best-effort and
    /// swallowed, so the assertion is on the durable queue row rather
    /// than on the handler's message — a lost enqueue is silent by
    /// design, and only the row proves it happened.
    ///
    /// Checked by mutation on 2026-08-10 by removing the
    /// `derive_series_after_hash` call from `hash_material`: the queue
    /// held no `series_derive` row, and the assertion failed at
    /// *"the fingerprint pass must ask for the keys of the metadata it
    /// just wrote"*.
    #[tokio::test]
    async fn the_fingerprint_pass_asks_for_the_keys_of_the_metadata_it_just_wrote() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool.clone(), search_dir.path()).await;
        let persona = seed_persona(&isle).await;

        // A real PNG carrying a real `vdsl` chunk, so the walker writes
        // `meta_kv` rather than the test writing it.
        let corpus = tempfile::tempdir().unwrap();
        let chunk = serde_json::to_string(&serde_json::json!({
            "script": "phase8_hires.lua",
            "version": "0.4.0",
        }))
        .unwrap();
        let mut text = b"vdsl\0".to_vec();
        text.extend_from_slice(chunk.as_bytes());
        let bytes = pngmeta::test_util::PngBuilder::new()
            .raw_chunk(*b"IDAT", 8, b"pixels!!")
            .raw_chunk(*b"tEXt", text.len() as u32, &text)
            .build();
        let path = corpus.path().join("run.png");
        std::fs::write(&path, &bytes).unwrap();

        let aid = uuid::Uuid::now_v7();
        let locator = path.to_string_lossy().to_string();
        isle.call(move |conn| {
            let stored = crate::sqlite::stored_locator(&locator);
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, labels, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                rusqlite::params![aid, persona, stored],
            )?;
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at)
                 VALUES (?1, 0, ?2, 'image/png', 0, 0)",
                rusqlite::params![aid, stored],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let hashed =
            handlers::material_hash(&env, &serde_json::json!({"asset_id": aid.to_string()}))
                .await
                .unwrap();
        assert!(
            hashed.contains("hashed=1"),
            "the fixture must hash: {hashed}"
        );

        let asked: Vec<String> = sqlx::query_scalar(
            "SELECT json_extract(job, '$.payload.asset_id') FROM Jobs \
              WHERE json_extract(job, '$.kind') = 'series_derive'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            asked,
            vec![aid.to_string()],
            "the fingerprint pass must ask for the keys of the metadata it just wrote"
        );

        // And running what it asked for derives a key off the container
        // the walker read — no second read of the file.
        let derived =
            handlers::series_derive(&env, &serde_json::json!({"asset_id": aid.to_string()}))
                .await
                .unwrap();
        assert!(derived.contains("derived=1"), "{derived}");
        let rows = filed_series(&isle).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "derived");
        assert!(
            rows[0]
                .1
                .as_deref()
                .is_some_and(|k| k.starts_with("sk1-sha256:"))
        );
    }

    // ---- chapter bands ---------------------------------------------

    /// A material pointing at a real file on disk, at `ord = 0`.
    ///
    /// Seeded through SQL rather than through `AssetService` for the
    /// reason its neighbours are: what is under test is a handler over
    /// stored rows, and routing the fixture through the ingest path
    /// would put that path's own enqueues in the queue this test reads.
    async fn seed_material_at(
        isle: &rusqlite_isle::AsyncIsle,
        persona: uuid::Uuid,
        path: &std::path::Path,
        mime: &str,
    ) -> uuid::Uuid {
        let aid = uuid::Uuid::now_v7();
        let path = path.to_string_lossy().to_string();
        let mime = mime.to_string();
        isle.call(move |conn| {
            let stored = crate::sqlite::stored_locator(&path);
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, labels, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'tape', '[]', 0, 0, 0)",
                rusqlite::params![aid, persona, stored],
            )?;
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at)
                 VALUES (?1, 0, ?2, ?3, 0, 0)",
                rusqlite::params![aid, stored, mime],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        aid
    }

    /// The handler end to end: a real container on disk, read by a real
    /// ffmpeg, filed into the imported band by the ports `JobDeps`
    /// carries.
    ///
    /// The reading itself is covered against the same fixture in
    /// `tests/chapter_scan_job.rs`; what only this test can reach is the
    /// wiring — that the handler resolves the two layer ports off
    /// `JobDeps`, hands them to the intake, and reports what landed.
    /// Wiring one of those fields to the wrong isle would leave every
    /// assertion in that file passing.
    #[tokio::test]
    async fn chapter_scan_files_the_sections_a_real_container_declares() {
        let bin = crate::jobs::thumb_ffmpeg::ffmpeg_binary();
        assert!(
            bin.is_some(),
            "ffmpeg is required for this test: brew install ffmpeg (or set $ASTERISM_FFMPEG)"
        );
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../asterism-importer-video/tests/fixtures/chaptered.mkv");
        assert!(
            fixture.is_file(),
            "{} is missing — run `python3 scripts/gen-test-fixtures.py`",
            fixture.display()
        );

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool, search_dir.path()).await;
        let persona = seed_persona(&isle).await;
        let aid = seed_material_at(&isle, persona, &fixture, "video/x-matroska").await;

        let message =
            handlers::chapter_scan(&env, &serde_json::json!({"asset_id": aid.to_string()}))
                .await
                .unwrap();
        assert!(
            message.contains("filed=1") && message.contains("sections=3"),
            "the message states what the file declared: {message}"
        );
        assert!(
            message.contains("refused=0") && message.contains("unreadable=0"),
            "nothing in the fixture is unrepresentable: {message}"
        );

        let layers = crate::sqlite::repo::SqliteMaterialLayerRepository::new(isle.clone());
        let chapters = crate::sqlite::repo::SqliteChapterMarkRepository::new(isle.clone());
        let bands = layers
            .list_by_asset(&asterism_core::domain::value::AssetId::from_uuid(aid))
            .await
            .unwrap();
        let band = bands
            .iter()
            .find(|l| {
                l.origin == asterism_core::domain::material_layer::LayerOrigin::Imported
                    && l.role == asterism_core::domain::material_layer::LayerRole::Structure
            })
            .expect("the reading opened an imported structure band");
        assert!(
            band.is_default,
            "the file's own division is the best answer until a person says otherwise"
        );
        let stored = chapters.list_by_layer(&band.id).await.unwrap();
        assert_eq!(
            stored
                .iter()
                .map(|c| (c.span.start_ms(), c.label.as_str()))
                .collect::<Vec<_>>(),
            [(0, "Opening"), (2_000, ""), (4_000, "Finale")],
            "the nanosecond timestamps the container declares arrive as milliseconds"
        );
    }

    /// A material with no timeline never reaches an ffmpeg, and gets no
    /// band.
    ///
    /// The eligibility rule is stated in three places — the ingest
    /// enqueue, the backfill's SQL, and the handler — and this is the
    /// one that has to hold whatever the other two let through: a
    /// `chapter_scan` aimed at a still by hand is still a `chapter_scan`
    /// that must not open the file.
    #[tokio::test]
    async fn chapter_scan_leaves_a_material_with_no_timeline_alone() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool, search_dir.path()).await;
        let persona = seed_persona(&isle).await;
        let aid = seed_material_at(
            &isle,
            persona,
            std::path::Path::new("/library/plate.png"),
            "image/png",
        )
        .await;

        let message =
            handlers::chapter_scan(&env, &serde_json::json!({"asset_id": aid.to_string()}))
                .await
                .unwrap();
        assert!(
            message.contains("not_timed=1") && message.contains("filed=0"),
            "a still is declined rather than read: {message}"
        );
        let layers = crate::sqlite::repo::SqliteMaterialLayerRepository::new(isle.clone());
        assert!(
            layers
                .list_by_asset(&asterism_core::domain::value::AssetId::from_uuid(aid))
                .await
                .unwrap()
                .is_empty(),
            "declining to read is not a reading, so it opens no band"
        );
    }

    /// An asset that is gone between enqueue and worker is a message,
    /// not an error — the same judgement every other per-asset handler
    /// records, and the reason a purge during an import wave does not
    /// fill the queue with failures.
    #[tokio::test]
    async fn chapter_scan_answers_a_vanished_asset_with_a_message() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool, search_dir.path()).await;

        let message = handlers::chapter_scan(
            &env,
            &serde_json::json!({"asset_id": uuid::Uuid::now_v7().to_string()}),
        )
        .await
        .unwrap();
        assert!(message.contains("gone"), "{message}");
    }

    /// A backfill page over an empty library ends the walk instead of
    /// chaining, which is what keeps a start on a read library at one
    /// query.
    #[tokio::test]
    async fn a_chapter_backfill_with_nothing_left_does_not_chain() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let search_dir = tempfile::tempdir().unwrap();
        let env = series_job_env(&isle, pool.clone(), search_dir.path()).await;

        let message = handlers::chapter_scan(&env, &serde_json::json!({"batch": true}))
            .await
            .unwrap();
        assert!(message.contains("nothing left to read"), "{message}");
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM Jobs WHERE json_extract(job, '$.kind') = 'chapter_scan'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(queued, 0, "a short page is the end of the walk");
    }
}
