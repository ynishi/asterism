//! Application **support** layer — use cases no transport adapter
//! fronts.
//!
//! Sibling of [`application`](crate::application), split off it on one
//! distinction: a service here is driven by the job worker, the
//! dispatch runner, or process startup, and by nothing else. There is
//! no Tauri command and no HTTP route behind any of these methods, and
//! the wiring makes that structural rather than advisory — the support
//! bundle is assembled into `CoreCtx` alone, and neither `ServerCtx`
//! nor `AppState` (the two structs a handler is handed) carries a
//! field for it. A handler that wants `purge_expired` does not have an
//! object to call it on.
//!
//! - [`retention_service`] — the trash retention sweep
//!   (`trash_purge` job).
//! - [`query_group_refresh_service`] — bulk Query Group
//!   re-evaluation (`query_group_refresh` job + the startup drift
//!   check).
//! - [`dispatch_runner_service`] — the runner-side half of the
//!   outbound dispatch state machine (`DispatchRun` job).
//! - [`duplicate_detection`] — what a freshly written fingerprint means
//!   for the corpus (`material_hash` job). Functions rather than a
//!   service, for the reason its own module doc gives: the handler
//!   already holds every port it names.
//!
//! These are new types, not relocated `impl` blocks: an inherent
//! `impl` moved to another module is still reachable from everywhere
//! the type is, so moving code would have changed nothing about who
//! can call it. What changed is which handle owns the verb.
//!
//! Support services are free to sit *on top of* `application`
//! services (the refresh service drives
//! [`QueryGroupService::evaluate_and_materialize`](crate::application::QueryGroupService::evaluate_and_materialize),
//! which has transport-fronted callers of its own). The dependency
//! runs support → application and never back.

pub mod dispatch_runner_service;
pub mod duplicate_detection;
pub mod query_group_refresh_service;
pub mod retention_service;

use std::sync::Arc;

pub use dispatch_runner_service::DispatchRunnerService;
pub use duplicate_detection::{
    Detection, DetectionOrigin, DetectionPorts, detect_duplicate, fold_excluded_by,
    resolve_strategy,
};
pub use query_group_refresh_service::{QueryGroupRefreshService, RefreshAllOutcome};
pub use retention_service::{RetentionService, Sweep};

/// Every support service, assembled once at the composition root.
///
/// Carried as a single `CoreCtx` field so the transport wrappers have
/// one thing to *not* copy: `ServerCtx::from_core` / `AppState` select
/// the HTTP- and Tauri-facing services out of `CoreCtx` field by
/// field, and this bundle is the field they skip. Adding a support
/// service therefore costs nothing at the transport boundary, and
/// exposing one is a visible edit to a wrapper rather than an
/// accident.
pub struct SupportServices {
    /// Trash retention sweep — driven by the `trash_purge` job.
    pub retention: Arc<RetentionService>,
    /// Bulk Query Group re-evaluation — driven by the
    /// `query_group_refresh` job and by startup.
    pub query_group_refresh: Arc<QueryGroupRefreshService>,
    /// Runner-side dispatch state machine — driven by the
    /// `DispatchRun` job.
    pub dispatch_runner: Arc<DispatchRunnerService>,
}
