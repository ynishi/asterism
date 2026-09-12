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
use asterism_core::application_support::{DispatchRunnerService, OutboundFile, OutboundStamping};
use asterism_core::domain::asset::AssetCard;
use asterism_core::domain::dispatch::DispatchState;
use asterism_core::domain::job::JobKind;
use asterism_core::domain::repository::{
    AssetRepository, DispatchRepository, JobQueue, SnapshotRepository,
};
use asterism_core::domain::value::{DispatchId, Viewer};
use asterism_core::error::DomainError;
use asterism_dispatch_sdk::{
    AttemptRecord, AttemptRecorder, DispatchContext, DispatchState as SdkState, Exporter,
    ExporterError, Handle,
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
    /// What gets asked to stamp the files a run wrote, before the run
    /// reports done.
    ///
    /// A port rather than a service, because the runner has no business
    /// knowing what a release is: it hands over the files and their
    /// sources, and the far side decides whether this run was one. An
    /// absent hook is a build that does not stamp on the way out, which
    /// is what every build was until releases existed.
    pub outbound: Option<Arc<dyn OutboundStamping>>,
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
    let named = asterism_core::application::fold_redirect::hydrate_named(
        env.assets.as_ref(),
        &snapshot.asset_ids,
        &Viewer::Owner,
    )
    .await?;
    // Re-projected onto the ids rather than taken as it came back.
    // `NamedCards.cards` is a set — the repository answers a
    // `WHERE id IN (…)` and SQLite may serve it from the id index, so
    // the order is the ids' own and not the freeze's. An exporter whose
    // params say "this node takes member 1" is reading a position, and
    // a position is only a fact once something establishes the order;
    // the freeze is what does, so the order the snapshot was named in
    // is the order the exporter sees. Members the viewer cannot see are
    // dropped here as they are there, which shortens the list rather
    // than shifting one member into another's place.
    let cards: Vec<AssetCard> = named
        .ids
        .iter()
        .filter_map(|id| named.cards.iter().find(|c| &c.id == id).cloned())
        .collect();
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
    let inputs: Vec<asterism_contract::dto::AssetCardDto> = cards
        .iter()
        .map(|c| {
            let mut dto = card_to_dto(c);
            dto.persona_id = persona_str.clone();
            dto
        })
        .collect();
    // Handed to the exporter on every call below, and drained after each
    // one. It is what makes a refused submit legible: `dispatch` can
    // only return an error, so anything it wants remembered about the
    // call has to have been written down before it returned.
    let recorder = CollectAttempt::default();
    let ctx = DispatchContext {
        inputs: &inputs,
        selection_id: &selection_str,
        dispatch_id: &dispatch_str,
        action: &job.action,
        params: &job.params,
        persona_id: &persona_str,
        attempt: &recorder,
    };

    match &job.state {
        DispatchState::Pending => {
            let submitted = exporter.dispatch(ctx).await;
            // Before the verdict, on both arms: the record of a refused
            // submit is the whole point, and the record of an accepted
            // one is what makes the two read alike.
            record_attempt(env, &dispatch_id, &recorder).await;
            let handle = match submitted {
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
            let polled = exporter.poll(ctx, &handle).await;
            record_attempt(env, &dispatch_id, &recorder).await;
            match polled {
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
                Ok(SdkState::Done) => {
                    let harvested = exporter.harvest(ctx, &handle).await;
                    record_attempt(env, &dispatch_id, &recorder).await;
                    match harvested {
                        Ok(derived) => {
                            let n = derived.len();
                            // Before the reify, which is what parks the
                            // row in `Done`: a file that leaves has to
                            // carry its disclosure by the time anything
                            // says the run finished.
                            stamp_outbound(env, &dispatch_id, &inputs, &derived).await;
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
                    }
                }
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

/// Asks the outbound hook to stamp what this run wrote.
///
/// # Why a failure here does not fail the tick
///
/// This is the state machine's rule rather than the port's. Nothing
/// re-queues a failed handler and v1 has no retry policy — the
/// dispatcher in [`crate::jobs`] is where that is decided — so returning
/// `Err` from this tick would leave the dispatch row in `Running` with
/// nothing coming to move it, over a mark that can be applied again from
/// stored rows. What a failure *means* is the port's own doc; what
/// happens to it here is that it is said out loud, on the same terms as
/// the attempt record above.
///
/// A run whose outputs cannot be paired with its inputs at all is said
/// out loud too ([`paired`]). Nothing is offered for stamping in that
/// case, so a release would otherwise end with no file rows and no line
/// anywhere saying why. A run whose outputs paired and copied nothing is
/// not that case and says nothing: a `reference`-mode export reports the
/// library's own files as its outputs, and there is no copy for a stamp
/// to go into.
async fn stamp_outbound(
    env: &DispatchRunEnv,
    id: &DispatchId,
    inputs: &[asterism_contract::dto::AssetCardDto],
    derived: &[asterism_dispatch_sdk::Derived],
) {
    let Some(outbound) = &env.outbound else {
        return;
    };
    let files = copies(inputs, derived);
    if files.is_empty() {
        if !derived.is_empty() && !paired(inputs, derived) {
            tracing::warn!(
                event = "diag.dispatch.outbound_unpaired",
                dispatch_id = %id,
                inputs = inputs.len(),
                derived = derived.len(),
                "this run's outputs could not be paired with its inputs, so nothing \
                 was offered for stamping"
            );
        }
        return;
    }
    if let Err(err) = outbound.stamp(id, &files).await {
        tracing::warn!(
            event = "diag.dispatch.outbound_stamp_failed",
            dispatch_id = %id,
            error = %err,
            "the files left and something they were owed did not land"
        );
    }
}

/// Whether this run's outputs line up with its inputs at all.
///
/// **One output per input, in input order.** That is what copying a
/// frozen set is, and it is the shape the `file` exporter's own loop
/// produces. A run that produced a different number of outputs made
/// something that is not a copy of one member — a single instruction
/// file, a batch of generations — and nothing here could say which
/// member a given file came out of.
///
/// Separate from [`copies`] because the two answer different questions,
/// and one of them was being read as the other: a run can pair perfectly
/// and yield no copies, which is every `reference`-mode export, and
/// reporting that as an unpaired run said something false about the
/// commonest export there is.
fn paired(
    inputs: &[asterism_contract::dto::AssetCardDto],
    derived: &[asterism_dispatch_sdk::Derived],
) -> bool {
    derived.len() == inputs.len()
}

/// The files this run wrote, paired with the library rows they are
/// copies of.
///
/// Empty for a run [`paired`] refuses, and empty for one that paired and
/// copied nothing.
///
/// **An output that is its own input is not a copy**, and is excluded.
/// A reference-mode run reports the library's own file as its output,
/// and stamping that would rewrite the original because an export
/// happened to walk past it — which is the distinction `stamp_after_hash`
/// draws for the same reason on the other path.
fn copies(
    inputs: &[asterism_contract::dto::AssetCardDto],
    derived: &[asterism_dispatch_sdk::Derived],
) -> Vec<OutboundFile> {
    if !paired(inputs, derived) {
        return Vec::new();
    }
    inputs
        .iter()
        .zip(derived)
        .filter(|(input, output)| output.locator != input.source_locator)
        .filter_map(|(input, output)| {
            let asset = uuid::Uuid::parse_str(&input.id).ok()?;
            Some(OutboundFile {
                asset: asterism_core::domain::value::AssetId::from_uuid(asset),
                path: std::path::PathBuf::from(&output.locator),
            })
        })
        .collect()
}

/// The runner's [`AttemptRecorder`]: a slot the exporter writes into
/// during a call and the runner empties once the call has returned.
///
/// Keeps the latest record only. Within one tick an exporter makes one
/// call worth recording, and across ticks the row's own column holds one
/// record by design — the sequence a reader wants is the sequence of
/// dispatch rows, not of retries inside a tick.
#[derive(Default)]
struct CollectAttempt(std::sync::Mutex<Option<AttemptRecord>>);

impl CollectAttempt {
    /// Empties the slot, so a call that recorded nothing is
    /// distinguishable from the previous call's record still sitting
    /// there.
    fn take(&self) -> Option<AttemptRecord> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl AttemptRecorder for CollectAttempt {
    fn record(&self, record: AttemptRecord) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(record);
    }
}

/// Persists whatever the exporter recorded about the call that just
/// returned. A call that recorded nothing writes nothing.
///
/// Logged rather than propagated, for the reason the reify path's
/// fingerprint enqueue is: the tick's own outcome — a handle to save, a
/// failure to record — is the thing the row must end up carrying, and an
/// error here would return `Err` from the handler and hand the job back
/// to the queue. The retry would find the row still `Pending` and submit
/// to the backend a second time, which is a duplicate job in exchange
/// for a note about the first one.
async fn record_attempt(env: &DispatchRunEnv, id: &DispatchId, recorder: &CollectAttempt) {
    let Some(record) = recorder.take() else {
        return;
    };
    if let Err(err) = env
        .service
        .save_attempt(id, record.kind, record.payload)
        .await
    {
        tracing::warn!(
            event = "diag.dispatch.attempt_save_failed",
            dispatch_id = %id,
            error = %err,
            "could not record what the exporter's call sent and received"
        );
    }
}

fn build_handle(job: &asterism_core::domain::dispatch::DispatchJob) -> Option<Handle> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_contract::dto::{AssetCardDto, DerivedDto};

    fn input(id: &str, source: &str) -> AssetCardDto {
        AssetCardDto {
            id: id.into(),
            persona_id: "p1".into(),
            modality: Some("image".into()),
            mime: Some("image/png".into()),
            media: "image".into(),
            occurred_at_ms: 0,
            occurred_source: "unknown".into(),
            time_zone: None,
            cover: None,
            labels: vec![],
            file_size_bytes: None,
            duration_ms: None,
            pixel_count: None,
            source_locator: source.into(),
            group_ids: vec![],
            primary_group_position: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            rating: None,
            palette: None,
            has_note: false,
            has_thread: false,
            role: "item".into(),
            title: None,
            member_count: 0,
            score: None,
            snippet: None,
            found_by: None,
            author_kind: None,
            author_subject: None,
            operator_ai: None,
        }
    }

    fn output(locator: &str) -> DerivedDto {
        DerivedDto {
            modality: "image".into(),
            locator: locator.into(),
            occurred_at: chrono::Utc::now(),
            cover_hint: None,
            register_note: None,
            labels: vec![],
            file_size_bytes: None,
            duration_ms: None,
            extra: serde_json::Value::Null,
            batch_hint: None,
        }
    }

    const ONE: &str = "0198c1c2-0000-7000-8000-000000000001";
    const TWO: &str = "0198c1c2-0000-7000-8000-000000000002";

    #[test]
    fn a_copy_is_paired_with_the_row_it_was_made_from() {
        let inputs = [input(ONE, "/lib/a.png"), input(TWO, "/lib/b.png")];
        let derived = [output("/out/a.png"), output("/out/b.png")];

        assert!(paired(&inputs, &derived));
        let files = copies(&inputs, &derived);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, std::path::PathBuf::from("/out/a.png"));
        assert_eq!(files[1].asset.to_string(), TWO);
    }

    /// A `reference`-mode export reports the library's own files as its
    /// outputs. Those outputs paired; there is simply no copy for a
    /// stamp to go into — and calling that an unpaired run said
    /// something false about the commonest export there is.
    #[test]
    fn a_reference_export_pairs_and_yields_no_copy() {
        let inputs = [input(ONE, "/lib/a.png"), input(TWO, "/lib/b.png")];
        let derived = [output("/lib/a.png"), output("/lib/b.png")];

        assert!(
            paired(&inputs, &derived),
            "the outputs line up with the inputs one for one"
        );
        assert!(
            copies(&inputs, &derived).is_empty(),
            "and none of them is a copy the library does not already hold"
        );
    }

    /// A run that produced a different number of outputs made something
    /// that is not a copy of one member, and nothing here can say which
    /// member any of it came out of.
    #[test]
    fn a_run_whose_outputs_do_not_line_up_is_unpaired() {
        let inputs = [input(ONE, "/lib/a.png"), input(TWO, "/lib/b.png")];
        let instruction = [output("/out/dispatch.json")];

        assert!(!paired(&inputs, &instruction));
        assert!(copies(&inputs, &instruction).is_empty());
    }

    /// An id the library could not have written is skipped rather than
    /// guessed at: a stamp needs the row the copy was made from.
    #[test]
    fn an_output_whose_input_carries_no_readable_id_is_left_out() {
        let inputs = [input("not-a-uuid", "/lib/a.png")];
        let derived = [output("/out/a.png")];

        assert!(paired(&inputs, &derived));
        assert!(copies(&inputs, &derived).is_empty());
    }
}
