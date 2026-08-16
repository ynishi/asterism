//! `DispatchRun` handler + `ExporterRegistry`.
//!
//! The handler is intentionally boring: fetch the job row → look up
//! the matching Exporter → advance one step of the state machine →
//! persist → optionally re-enqueue. All Exporter interaction goes
//! through the SDK trait so this file has no knowledge of any
//! specific backend.

use std::collections::HashMap;
use std::sync::Arc;

use asterism_core::application::mapping::card_to_dto;
use asterism_core::application_support::DispatchRunnerService;
use asterism_core::domain::asset::AssetCard;
use asterism_core::domain::forge::dispatch::DispatchState;
use asterism_core::domain::job::JobKind;
use asterism_core::domain::repository::{
    AssetRepository, DispatchRepository, JobQueue, SnapshotRepository,
};
use asterism_core::domain::value::{DispatchId, Viewer};
use asterism_core::error::DomainError;
use asterism_dispatch_sdk::{
    DispatchContext, DispatchState as SdkState, Exporter, ExporterError, Handle,
};

/// Registry of exporters keyed by their `Exporter::slug()`, plus any
/// retired slugs that still resolve.
///
/// Cheap to `Clone` (behind an `Arc`).
#[derive(Clone, Default)]
pub struct ExporterRegistry {
    inner: Arc<HashMap<String, Arc<dyn Exporter>>>,
    /// Retired slug → the slug that answers for it now.
    ///
    /// Separate from `inner` on purpose. An alias has to *resolve*,
    /// because it is written on dispatch rows and on the assets those
    /// produced, and it must not be *offered*, because offering it lets
    /// a new dispatch be created under a name the codebase merged away.
    /// One map would have to be both.
    aliases: Arc<HashMap<String, String>>,
}

impl ExporterRegistry {
    /// Builds a registry from a slug → impl map. Duplicates in the
    /// input are last-write-wins; callers usually iterate over a
    /// static list at boot time so this rarely matters.
    pub fn new(entries: HashMap<String, Arc<dyn Exporter>>) -> Self {
        Self {
            inner: Arc::new(entries),
            aliases: Arc::new(HashMap::new()),
        }
    }

    /// Adds retired slugs that resolve to a registered one.
    ///
    /// For a slug that outlived the crate that minted it: rows created
    /// under it keep running, and it does not appear in [`slugs`] — so
    /// nothing new can be created under it.
    ///
    /// [`slugs`]: Self::slugs
    pub fn with_aliases(mut self, aliases: HashMap<String, String>) -> Self {
        self.aliases = Arc::new(aliases);
        self
    }

    /// Convenience builder for the common case of a single
    /// registered exporter.
    pub fn single(exporter: Arc<dyn Exporter>) -> Self {
        let mut map = HashMap::new();
        map.insert(exporter.slug().to_string(), exporter);
        Self::new(map)
    }

    /// Looks up an exporter by slug, following one alias hop.
    pub fn get(&self, slug: &str) -> Option<Arc<dyn Exporter>> {
        if let Some(found) = self.inner.get(slug) {
            return Some(Arc::clone(found));
        }
        let target = self.aliases.get(slug)?;
        self.inner.get(target).cloned()
    }

    /// Returns every registered slug (used by the server's
    /// `/exporters` endpoint for UI discovery).
    ///
    /// Aliases are deliberately absent: this list is what a caller may
    /// choose from, and a retired slug is something old rows carry
    /// rather than something anybody should pick.
    pub fn slugs(&self) -> Vec<String> {
        let mut out: Vec<String> = self.inner.keys().cloned().collect();
        out.sort();
        out
    }
}

/// Bundle of dependencies the runner needs on every tick.
pub struct DispatchRunEnv {
    /// Registry of installed exporters.
    pub registry: ExporterRegistry,
    /// Runner-side service surface: state saves, handle persistence,
    /// and reify. A support service
    /// (`asterism_core::application_support`) rather than an
    /// application one — these are the state machine's own
    /// transitions, and this environment is the only place in the
    /// process that holds a handle to them. The transport-fronted half
    /// (create / run / redispatch / get / list) stays on
    /// `DispatchService` and is not reachable from here.
    pub service: Arc<DispatchRunnerService>,
    /// Read-side Snapshot port (used to materialise the input Assets).
    pub snapshots: Arc<dyn SnapshotRepository>,
    /// Read-side DispatchJob port (used to fetch the job row + state
    /// on every tick).
    pub dispatches: Arc<dyn DispatchRepository>,
    /// Read-side Asset port (used to hydrate `AssetCard`s for the
    /// `DispatchContext.inputs` slice).
    pub assets: Arc<dyn AssetRepository>,
    /// A queue handle used to re-enqueue the next poll tick without
    /// pulling in a full `JobQueue` trait object at this call site.
    pub reenqueue: Arc<dyn ReEnqueue>,
}

/// Small port around "put this dispatch id back on the queue for
/// another tick". Kept as a trait so the runner does not have to know
/// about apalis specifics — the caller wires it up to
/// [`crate::jobs::SqliteJobQueue::enqueue`] with
/// [`JobKind::DispatchRun`], or a test fake in unit tests.
#[async_trait::async_trait]
pub trait ReEnqueue: Send + Sync {
    /// Push another `DispatchRun` tick for the given id.
    async fn reenqueue(&self, dispatch_id: &DispatchId) -> Result<(), DomainError>;
}

/// [`ReEnqueue`] impl that pushes another `DispatchRun` job through
/// the shared apalis-backed queue.
///
/// The runner uses this to bridge the poll-loop's "check again in a
/// moment" back into the same queue every other job flows through, so
/// dispatch progress lands in the same monitoring surface as the rest
/// of the pipeline.
pub struct QueueReEnqueue<Q: JobQueue + Send + Sync + 'static> {
    /// Underlying queue handle.
    pub queue: Arc<Q>,
}

#[async_trait::async_trait]
impl<Q: JobQueue + Send + Sync + 'static> ReEnqueue for QueueReEnqueue<Q> {
    async fn reenqueue(&self, dispatch_id: &DispatchId) -> Result<(), DomainError> {
        self.queue
            .enqueue(
                JobKind::DispatchRun,
                serde_json::json!({ "dispatch_id": dispatch_id.to_string() }),
            )
            .await
            .map(|_| ())
    }
}

/// Executes one step of the dispatch state machine.
///
/// Returns a short human-readable status for the progress emitter.
/// Structured to leave the poll loop safe against:
///   - duplicate ticks: terminal-state guard exits fast.
///   - lost handles: `Pending` reruns `dispatch` because `handle` is `None`.
///   - dead exporters: the runner marks the job `Failed` and stops.
pub async fn run_dispatch_run(
    env: &DispatchRunEnv,
    payload: &serde_json::Value,
) -> Result<String, DomainError> {
    let dispatch_id_str = payload
        .get("dispatch_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DomainError::Validation("DispatchRun payload missing dispatch_id".into()))?;
    let dispatch_uuid = uuid::Uuid::parse_str(dispatch_id_str).map_err(|_| {
        DomainError::Validation(format!("invalid dispatch_id: {dispatch_id_str:?}"))
    })?;
    let dispatch_id = DispatchId::from_uuid(dispatch_uuid);

    let job = env
        .dispatches
        .find(&dispatch_id)
        .await?
        .ok_or_else(|| DomainError::not_found("dispatch", dispatch_id_str))?;
    if job.state.is_terminal() {
        return Ok(format!(
            "dispatch {} already in terminal state {}",
            dispatch_id_str,
            job.state.slug()
        ));
    }
    let Some(exporter) = env.registry.get(&job.exporter_slug) else {
        // Unknown exporter — persist the failure so the caller sees
        // the reason on the row instead of the job silently
        // disappearing.
        let _ = env
            .service
            .save_state(
                &dispatch_id,
                DispatchState::Failed {
                    message: format!("exporter not registered: {}", job.exporter_slug),
                },
            )
            .await?;
        return Ok(format!(
            "dispatch {} failed: exporter {:?} not registered",
            dispatch_id_str, job.exporter_slug
        ));
    };

    // Materialise the InputAsset slice from the Snapshot's ids. The
    // Snapshot may reference deleted assets; those simply drop out
    // of the input (the exporter sees an empty slice as
    // "everything was collected already" and can respond with
    // BackendRejected if that is unacceptable).
    //
    // A member folded since the freeze was minted is exported as its
    // keeper, and collapses onto a keeper the freeze already held —
    // without that, a freeze holding both sides of a later merge hands
    // the exporter the same artefact twice, under two ids that name one
    // row (`asterism_core::application::fold_redirect`).
    let snapshot = env.snapshots.find(&job.snapshot_id).await?.ok_or_else(|| {
        DomainError::Validation(format!("snapshot vanished: {}", job.snapshot_id))
    })?;
    let cards: Vec<AssetCard> = asterism_core::application::fold_redirect::hydrate_named(
        env.assets.as_ref(),
        &snapshot.asset_ids,
        &Viewer::Owner,
    )
    .await?
    .cards;
    // Project every AssetCard through the contract-owned
    // AssetCardDto so the SDK sees the same wire shape the Tauri UI
    // already receives. Persona id on the DispatchJob wins over the
    // per-card persona id (they should agree, but the job is the
    // authoritative bucket at dispatch time).
    let persona_str = job.persona_id.to_string();
    // `DispatchContext.selection_id` is the SDK field name (kept); it
    // carries the snapshot id.
    let selection_str = job.snapshot_id.to_string();
    let dispatch_str = job.id.to_string();
    let pursuit_str = job.pursuit_id.map(|p| p.to_string());
    let inputs: Vec<asterism_contract::dto::AssetCardDto> = cards
        .iter()
        .map(|c| {
            let mut dto = card_to_dto(c);
            dto.persona_id = persona_str.clone();
            dto
        })
        .collect();
    let ctx = DispatchContext {
        inputs: &inputs,
        selection_id: &selection_str,
        dispatch_id: &dispatch_str,
        pursuit_id: pursuit_str.as_deref(),
        action: &job.action,
        params: &job.params,
        persona_id: &persona_str,
    };

    match &job.state {
        DispatchState::Pending => {
            let handle = match exporter.dispatch(ctx).await {
                Ok(h) => h,
                Err(err) => {
                    let message = describe(&err);
                    env.service
                        .save_state(
                            &dispatch_id,
                            DispatchState::Failed {
                                message: message.clone(),
                            },
                        )
                        .await?;
                    return Ok(format!("dispatch {} rejected: {message}", dispatch_id_str));
                }
            };
            let handle_kind = handle.kind.clone();
            env.service
                .save_handle(&dispatch_id, handle_kind, handle.payload.clone())
                .await?;
            env.service
                .save_state(
                    &dispatch_id,
                    DispatchState::Running {
                        current: None,
                        total: None,
                        message: Some("dispatched".into()),
                    },
                )
                .await?;
            env.reenqueue.reenqueue(&dispatch_id).await?;
            Ok(format!("dispatch {} sent to exporter", dispatch_id_str))
        }
        DispatchState::Running { .. } => {
            let Some(handle) = build_handle(&job) else {
                let message = "running dispatch has no handle payload".to_string();
                env.service
                    .save_state(
                        &dispatch_id,
                        DispatchState::Failed {
                            message: message.clone(),
                        },
                    )
                    .await?;
                return Ok(format!("dispatch {} failed: {message}", dispatch_id_str));
            };
            match exporter.poll(ctx, &handle).await {
                Ok(SdkState::Running(hint)) => {
                    env.service
                        .save_state(
                            &dispatch_id,
                            DispatchState::Running {
                                current: hint.current,
                                total: hint.total,
                                message: hint.message,
                            },
                        )
                        .await?;
                    env.reenqueue.reenqueue(&dispatch_id).await?;
                    Ok(format!("dispatch {} still running", dispatch_id_str))
                }
                Ok(SdkState::Done) => match exporter.harvest(ctx, &handle).await {
                    Ok(derived) => {
                        let n = derived.len();
                        env.service.reify(&dispatch_id, derived).await?;
                        Ok(format!(
                            "dispatch {} harvested {} derived",
                            dispatch_id_str, n
                        ))
                    }
                    Err(err) => {
                        let message = describe(&err);
                        env.service
                            .save_state(
                                &dispatch_id,
                                DispatchState::Failed {
                                    message: message.clone(),
                                },
                            )
                            .await?;
                        Ok(format!(
                            "dispatch {} harvest failed: {message}",
                            dispatch_id_str
                        ))
                    }
                },
                Ok(SdkState::Failed { message }) => {
                    env.service
                        .save_state(
                            &dispatch_id,
                            DispatchState::Failed {
                                message: message.clone(),
                            },
                        )
                        .await?;
                    Ok(format!("dispatch {} failed: {message}", dispatch_id_str))
                }
                Ok(SdkState::Cancelled { reason }) => {
                    env.service
                        .save_state(&dispatch_id, DispatchState::Cancelled { reason })
                        .await?;
                    Ok(format!("dispatch {} cancelled", dispatch_id_str))
                }
                Ok(SdkState::Pending) => {
                    env.reenqueue.reenqueue(&dispatch_id).await?;
                    Ok(format!(
                        "dispatch {} still pending on backend",
                        dispatch_id_str
                    ))
                }
                Err(err) => {
                    let message = describe(&err);
                    env.service
                        .save_state(
                            &dispatch_id,
                            DispatchState::Failed {
                                message: message.clone(),
                            },
                        )
                        .await?;
                    Ok(format!(
                        "dispatch {} poll failed: {message}",
                        dispatch_id_str
                    ))
                }
            }
        }
        DispatchState::Done | DispatchState::Failed { .. } | DispatchState::Cancelled { .. } => {
            // Guarded by the early `is_terminal` return above; here
            // as a defensive branch.
            Ok(format!(
                "dispatch {} already terminal ({}), skipped",
                dispatch_id_str,
                job.state.slug()
            ))
        }
    }
}

fn build_handle(job: &asterism_core::domain::forge::dispatch::DispatchJob) -> Option<Handle> {
    match (&job.handle_kind, &job.handle) {
        (Some(kind), Some(payload)) => Some(Handle {
            kind: kind.clone(),
            payload: payload.clone(),
        }),
        _ => None,
    }
}

fn describe(err: &ExporterError) -> String {
    err.to_string()
}
