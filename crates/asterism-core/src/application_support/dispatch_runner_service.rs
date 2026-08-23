//! `DispatchRunnerService` — the runner-side half of the outbound
//! dispatch lifecycle.
//!
//! **Driven by the `DispatchRun` job (`asterism_infra::dispatch::runtime`),
//! and by nothing else.** No Tauri command and no HTTP route fronts
//! these three verbs, and none should: they are the state machine's
//! own transitions. A handler that could call
//! [`save_state`](DispatchRunnerService::save_state) could park a
//! dispatch in `Done` without an exporter ever having run, and one
//! that could call [`reify`](DispatchRunnerService::reify) could mint
//! assets from a payload the wire supplied rather than from what the
//! exporter actually produced. What the transports *do* front lives on
//! [`DispatchService`](crate::application::DispatchService): create /
//! run / redispatch (start one) and get / list (read one).
//!
//! - [`save_state`](DispatchRunnerService::save_state) /
//!   [`save_handle`](DispatchRunnerService::save_handle) — runner-side
//!   updates during the `dispatch → poll` loop. The handle is
//!   persisted immediately after `Exporter::dispatch` returns so a
//!   restart mid-poll rehydrates the exact same reference.
//! - [`save_attempt`](DispatchRunnerService::save_attempt) — the record
//!   of the call the exporter just made, which is the only thing a
//!   refused submit leaves behind: it produces no handle, and without
//!   this the request as sent and the backend's answer go out with the
//!   error.
//! - [`reify`](DispatchRunnerService::reify) — turn the
//!   `Vec<Derived>` the exporter produced into new `Asset` rows whose
//!   `parent_ids` point at the Snapshot's members via
//!   `ConstellationEdge { kind: DerivedFrom }`, and ask for each new
//!   row's bytes to be fingerprinted.
//!
//! `reify` is the boundary where the SDK's `Derived` shape crosses
//! back into the domain — the only place in the workspace that speaks
//! both dialects.
//!
//! It is also the one write path that takes no `AttributionContext`
//! from its caller (the *restore* class). The caller is the
//! job runtime in `asterism-infra`, which could only ever assert one;
//! the honest answer was recorded on the dispatch row when the request
//! arrived, so this service reads it back
//! ([`DispatchJob::persisted_attribution`]) and carries it onto the
//! assets it mints.

use std::sync::Arc;

use asterism_contract::dto::DerivedDto as Derived;
use chrono::{DateTime, Utc};

use crate::domain::asset::Asset;
use crate::domain::dispatch::{DispatchJob, DispatchState};
use crate::domain::edge::{ConstellationEdge, EdgeKind};
use crate::domain::job::JobKind;
use crate::domain::repository::{
    AssetRepository, DispatchRepository, EdgeRepository, JobQueue, PersonaRepository,
    SnapshotRepository,
};
use crate::domain::value::{
    AssetId, BundleId, CoverText, DispatchId, Label, Modality, PersonaId, RegisterNote, SnapshotId,
    SourceKind, SourceRef, dedup_labels,
};
use crate::error::DomainError;

/// Max cover / register text length copied over from `Derived`
/// (kept identical to
/// `asterism_contract::dto::DERIVED_COVER_MAX_CHARS`).
const COVER_MAX_CHARS: usize = 200;
const REGISTER_MAX_CHARS: usize = 80;

/// Runner-side dispatch service. Held by `CoreCtx`'s support bundle
/// and handed to the runner through `DispatchRunEnv`; it is not
/// reachable from `ServerCtx` / `AppState`.
pub struct DispatchRunnerService {
    dispatches: Arc<dyn DispatchRepository>,
    snapshots: Arc<dyn SnapshotRepository>,
    assets: Arc<dyn AssetRepository>,
    edges: Arc<dyn EdgeRepository>,
    /// Persona port — `reify` is the one place outside `AssetService`
    /// that mints assets, so it needs the same trashed-persona guard.
    /// A dispatch that was in flight when its persona went to the trash
    /// would otherwise land live rows under it: invisible in the grid,
    /// unreachable from the trash view, and destroyed without a trash
    /// stage by the retention sweep's persona cascade.
    personas: Arc<dyn PersonaRepository>,
    /// Composed rather than reimplemented, the same way
    /// `QueryGroupRefreshService` wraps `QueryGroupService`: there is
    /// exactly one provenance-resolution pipeline, and the post-reify
    /// repair pass is a loop over it. Only
    /// [`reresolve_unresolved`](crate::application::AssetService::reresolve_unresolved)
    /// is called through this handle.
    asset_service: Arc<crate::application::AssetService>,
    /// Queue port — [`reify`](Self::reify) mints assets, and a minted
    /// asset needs its bytes fingerprinted.
    ///
    /// **Injected here rather than left to the job handler that drives
    /// the runner**, for the same reason `AssetService` holds one: this
    /// is a place where assets come into existence, and every such
    /// place owes the new row a `material_hash`. The handler side would
    /// work — it can read `output_asset_ids` off the job it gets back —
    /// but it would make "mint a row" and "fingerprint it" two facts
    /// kept in step by whoever wired the caller, and `reify` has four
    /// other callers already (the e2e binaries drive it directly). Each
    /// of those would be a dispatch whose outputs have no fingerprint
    /// until a restart's backfill walk reaches them, which is precisely
    /// the gap this injection closes.
    jobs: Arc<dyn JobQueue>,
}

impl DispatchRunnerService {
    /// Wires the runner around its ports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dispatches: Arc<dyn DispatchRepository>,
        snapshots: Arc<dyn SnapshotRepository>,
        assets: Arc<dyn AssetRepository>,
        edges: Arc<dyn EdgeRepository>,
        personas: Arc<dyn PersonaRepository>,
        asset_service: Arc<crate::application::AssetService>,
        jobs: Arc<dyn JobQueue>,
    ) -> Self {
        Self {
            dispatches,
            snapshots,
            assets,
            edges,
            personas,
            asset_service,
            jobs,
        }
    }

    /// Runner-side atomic update: persist a new state (progress /
    /// failure / cancel signals).
    ///
    /// Updates `updated_at` (and `completed_at` if the new state is
    /// terminal) at the same time — the caller does not need to stamp
    /// timestamps.
    pub async fn save_state(
        &self,
        id: &DispatchId,
        new_state: DispatchState,
    ) -> Result<DispatchJob, DomainError> {
        let mut job = self
            .dispatches
            .find(id)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", id))?;
        let now = Utc::now();
        let becoming_terminal = new_state.is_terminal();
        job.state = new_state;
        job.updated_at = now;
        if becoming_terminal && job.completed_at.is_none() {
            job.completed_at = Some(now);
        }
        self.dispatches.save(&job).await?;
        Ok(job)
    }

    /// Runner-side handle persistence — called immediately after
    /// `Exporter::dispatch` returns, so a restart mid-poll can
    /// rehydrate the exact same reference.
    pub async fn save_handle(
        &self,
        id: &DispatchId,
        handle_kind: String,
        handle_payload: serde_json::Value,
    ) -> Result<(), DomainError> {
        let mut job = self
            .dispatches
            .find(id)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", id))?;
        job.handle_kind = Some(handle_kind);
        job.handle = Some(handle_payload);
        job.updated_at = Utc::now();
        self.dispatches.save(&job).await?;
        Ok(())
    }

    /// Runner-side attempt persistence — called after an exporter call
    /// returns, whichever arm it returned on, with whatever the exporter
    /// recorded about the call it just made.
    ///
    /// Separate from [`save_handle`](Self::save_handle) because the case
    /// it serves is the one with no handle to save: a submit the backend
    /// refused ends the job with an error, and the request as sent and
    /// the backend's answer would otherwise leave with it. The runner
    /// records the attempt *before* it saves the failure, so the row
    /// carries the evidence by the time it carries the verdict.
    ///
    /// The payload is opaque here in the way the handle is: its shape
    /// belongs to the exporter named by `attempt_kind`.
    pub async fn save_attempt(
        &self,
        id: &DispatchId,
        attempt_kind: String,
        attempt_payload: serde_json::Value,
    ) -> Result<(), DomainError> {
        let mut job = self
            .dispatches
            .find(id)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", id))?;
        job.attempt_kind = Some(attempt_kind);
        job.attempt = Some(attempt_payload);
        job.updated_at = Utc::now();
        self.dispatches.save(&job).await?;
        Ok(())
    }

    /// Reifies the exporter's `Vec<Derived>` into new `Asset` rows,
    /// records `DerivedFrom` constellation edges, enqueues each new
    /// row's fingerprint ([`enqueue_fingerprint`](Self::enqueue_fingerprint)),
    /// and closes out the dispatch with state `Done` + populated
    /// `output_asset_ids`.
    ///
    /// This is the point where the SDK's dialect (`Derived`) crosses
    /// back into the domain's dialect (`Asset` + `ConstellationEdge`).
    /// Everything else about the outbound flow is expressed in one or
    /// the other — this function is the seam.
    pub async fn reify(
        &self,
        id: &DispatchId,
        derived: Vec<Derived>,
    ) -> Result<DispatchJob, DomainError> {
        let mut job = self
            .dispatches
            .find(id)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", id))?;
        let snapshot = self
            .snapshots
            .find(&job.snapshot_id)
            .await?
            .ok_or_else(|| {
                DomainError::Validation(format!(
                    "snapshot {} vanished during reify of dispatch {id}",
                    job.snapshot_id
                ))
            })?;

        // Same guard as `AssetService::add`, and needed for the same
        // reason: a trashed persona is invisible only because its
        // then-live assets carry its stamp, so a freshly minted one
        // would be live under a persona the user believes is gone —
        // and would be destroyed by the retention sweep's cascade
        // without ever appearing in the trash. Dispatches are polled in
        // the background, so one can easily still be in flight when the
        // persona goes.
        if let Some(persona) = self.personas.find(&job.persona_id).await?
            && persona.trashed_at.is_some()
        {
            return Err(DomainError::blocked(format!(
                "persona {} is in the trash; restore it before reifying dispatch {id}",
                job.persona_id
            )));
        }

        let now = Utc::now();
        let session_id_str = job.id.to_string();
        // Route the SourceKind through the domain-owned factory so
        // the "dispatch-<slug>" convention lives in one place — the
        // 2026-07-19 smoke Test Red on `dispatch:file` was a
        // hand-formatted call site drifting past SourceKind's
        // grammar. The factory rejects an invalid exporter slug
        // here instead of at Asset save time.
        let source_kind = SourceKind::for_dispatch(&job.exporter_slug).map_err(|e| {
            DomainError::Validation(format!(
                "exporter slug {:?} does not form a valid SourceKind: {e}",
                job.exporter_slug
            ))
        })?;
        let source_kind_slug = source_kind.as_str().to_string();

        // The restore path (third class of write): this
        // service receives no `AttributionContext` from its caller —
        // there is none to receive. The runner is driven by a background
        // job, minutes or hours after the request that started the run,
        // and the honest answer to "who asked for this" was written on
        // the job row at that moment. Reading it back is the only
        // attribution source here; a context argument would either
        // duplicate it or let the job runtime assert something.
        let attribution = crate::domain::attribution::AttributionContext::from_persisted(
            job.persisted_attribution(),
        );

        let mut output_ids: Vec<AssetId> = Vec::with_capacity(derived.len());
        for d in derived {
            let asset = reify_one(
                &d,
                job.persona_id,
                &source_kind,
                &source_kind_slug,
                &job.exporter_slug,
                &job.id,
                &snapshot.id,
                &session_id_str,
                &attribution,
            )?;
            let asset_id = asset.id;
            self.assets.save(&asset).await?;
            // Write derived_from edges pointing at each Snapshot
            // member. Skip the (rare) case where the derivation's own
            // id collides with a source id — the edge constructor
            // rejects self-loops, and reify_derived should never
            // produce them under normal circumstances.
            let mut edges: Vec<ConstellationEdge> = Vec::with_capacity(snapshot.asset_ids.len());
            for parent_id in &snapshot.asset_ids {
                if *parent_id == asset_id {
                    continue;
                }
                let mut edge = ConstellationEdge::new(asset_id, *parent_id, EdgeKind::DerivedFrom)?;
                // Reuse the factory-produced source_kind slug so the
                // edge label matches the Asset's `source_kind` verbatim
                // — one convention, one construction site (mirror of
                // the SourceKind::for_dispatch policy).
                edge.label = Some(source_kind_slug.clone());
                edge.weight = Some(1.0);
                edges.push(edge);
            }
            // `add_edges` is the assertion port: provenance
            // accumulates and survives the `edge_rebuild` pass that
            // recomputes this asset's synth edges later.
            self.edges.add_edges(edges).await?;
            self.enqueue_fingerprint(&asset_id).await;
            output_ids.push(asset_id);
        }

        job.output_asset_ids = output_ids;
        job.state = DispatchState::Done;
        job.updated_at = now;
        job.completed_at = Some(now);
        self.dispatches.save(&job).await?;

        // New outputs are exactly what a pending provenance claim has
        // been waiting for — an artefact ingested with
        // `dispatch:<this id>` before the export finished was recorded
        // unresolved, and this is the moment the answer changed. The
        // sweep is best-effort: the reify itself has already landed,
        // and a failed repair pass leaves the claims recorded and
        // retryable, so it must not turn a successful reify into an
        // error.
        if let Err(err) = self.asset_service.reresolve_unresolved().await {
            tracing::warn!(
                event = "diag.dispatch.reresolve_failed",
                dispatch_id = %id,
                error = %err,
                "post-reify provenance re-resolve failed"
            );
        }
        Ok(job)
    }

    /// Asks for one reified artefact's bytes to be fingerprinted.
    ///
    /// # Why after the edges, not after the save
    ///
    /// The fingerprint is what raises a duplicate conflict, and what
    /// decides whether that conflict is folded without asking is, among
    /// other things, whether the two rows are one lineage
    /// (`asterism_core::application_support::duplicate_detection`). The
    /// lineage of this row is the `derived_from` edges written a few
    /// lines up. Enqueueing before them leaves a window in which a
    /// worker can fingerprint an asset that does not yet appear to
    /// descend from anything — and an exporter in copy mode writes its
    /// input's bytes verbatim, so what is on the other side of that
    /// window is the input itself. The whole point of shipping this
    /// enqueue with the exclusion rules is that neither is safe without
    /// the other; the ordering here is the same requirement one line
    /// smaller.
    ///
    /// # Why a failure is not the dispatch's failure
    ///
    /// A missing fingerprint is recoverable — the backfill walk finds
    /// work by `content_hash IS NULL` and will reach this row on a
    /// later pass. The export it belongs to is not: the bytes are
    /// written, the asset is saved, and failing the reify over a queue
    /// push would discard a run's output to avoid deferring a hash. It
    /// is logged rather than swallowed, because "the hash arrived a
    /// restart late" is otherwise indistinguishable from "the enqueue
    /// was never wired", which is the state this method exists to leave
    /// behind.
    ///
    /// Below the default priority, as on the ingest path: fingerprinting
    /// reads every byte of the artefact, and a worker slot held by a
    /// 4 GB video is a slot not painting the grid the user is watching.
    async fn enqueue_fingerprint(&self, asset_id: &AssetId) {
        if let Err(err) = self
            .jobs
            .enqueue_with_priority(
                JobKind::MaterialHash,
                serde_json::json!({ "asset_id": asset_id.to_string() }),
                -10,
            )
            .await
        {
            tracing::warn!(
                event = "diag.dispatch.hash_enqueue_failed",
                asset_id = %asset_id,
                error = %err,
                "could not enqueue the fingerprint for a reified artefact; \
                 the backfill walk will pick it up"
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn reify_one(
    derived: &Derived,
    persona_id: PersonaId,
    source_kind: &SourceKind,
    source_kind_slug: &str,
    exporter_slug: &str,
    _dispatch_id: &DispatchId,
    snapshot_id: &SnapshotId,
    session_id_str: &str,
    attribution: &crate::domain::attribution::AttributionContext,
) -> Result<Asset, DomainError> {
    // The exporter's declared modality rides along as a user slug for
    // now (asset-model v4 carry: exporters should eventually declare
    // format via the material layer instead).
    let modality = Some(Modality::new(derived.modality.clone())?);
    let mut source = SourceRef::new(source_kind.clone(), derived.locator.clone())?;
    source.file_size_bytes = derived.file_size_bytes;
    let _ = source_kind_slug; // reserved for future audit trail
    // The dispatch-time attribution, carried forward whole: whoever
    // asked for the run is who this output is by, through whichever
    // agent they used, and `via` keeps its dispatch-time meaning — the
    // channel the *request to start the run* came in on, not the
    // background job that finished it.
    let mut asset = Asset::new(
        persona_id,
        source,
        modality,
        derived.occurred_at,
        attribution,
    );
    // Physical layer for the reified artefact (asset-model v4).
    asset.attach_material(crate::domain::material::Material::primary(
        asset.source.locator.clone(),
        asset.source.file_size_bytes,
        asset.created_at,
    ))?;
    // Dispatch siblings cluster on the grid by dispatch id. Session is
    // now the Dialog-modality 1st-class entity (V27 adds `session_id IS
    // NULL OR modality = 'dialogue'` + `REFERENCES session(id)`), and
    // a dispatch id does not correspond to a Session row — so route
    // every reified derivation through `bundle_id`, the
    // modality-agnostic constellation grouping slot introduced in V24.
    // Same identifier either way, grid clustering behaves as before
    // (regardless of the exporter's chosen modality).
    asset.bundle_id = Some(BundleId::new(session_id_str.to_string())?);
    // Prepend the exporter slug to the label chip list so cards
    // immediately identify their origin without a follow-up query.
    // The prepend is why this list needs the guard more than most: an
    // exporter that already put `exporter:<slug>` in its own derived
    // labels would state it twice, and nothing stops one derivation
    // from repeating a label within itself either. The slug stays at
    // the head because `dedup_labels` keeps the first occurrence.
    let mut labels: Vec<Label> = Vec::with_capacity(derived.labels.len() + 1);
    labels.push(Label::new(format!("exporter:{exporter_slug}"))?);
    for l in &derived.labels {
        labels.push(Label::new(l.clone())?);
    }
    asset.labels = dedup_labels(labels);
    asset.cover = derived
        .cover_hint
        .as_deref()
        .map(|s| truncate_chars(s.trim(), COVER_MAX_CHARS))
        .filter(|s| !s.is_empty())
        .map(CoverText::new)
        .transpose()?;
    asset.register_note = derived
        .register_note
        .as_deref()
        .map(|s| truncate_chars(s.trim(), REGISTER_MAX_CHARS))
        .filter(|s| !s.is_empty())
        .map(RegisterNote::new)
        .transpose()?;
    asset.duration_ms = derived.duration_ms;
    let operator_ai = attribution.operator_ai();

    // Embed dispatch traceability in `Asset::extra` so downstream
    // queries can go "which Snapshot produced this asset?" in one
    // JSON extract without touching the edges table. Merge with the
    // exporter's own `extra` payload (exporter fields at the top
    // level, dispatch trace nested under `_dispatch`). The JSON key
    // stays `selection_id` for wire stability (downstream readers /
    // W5 cleanup) even though it now carries the snapshot id.
    let mut extra = derived.extra.clone();
    let mut trace = serde_json::json!({
        "selection_id": snapshot_id.to_string(),
        "dispatch_id": session_id_str,
        "exporter_slug": exporter_slug,
    });
    // The operator is on the asset's own column too; it is repeated on
    // the note because the note is what a reader of the *run* looks at,
    // and a dispatch with no outputs left would otherwise carry no
    // record of who started it. Omitted entirely when unrecorded —
    // never `null`, which reads as a value someone wrote.
    if let Some(operator) = operator_ai {
        trace["operator"] = serde_json::json!(operator.as_str());
    }
    match &mut extra {
        serde_json::Value::Object(map) => {
            map.insert(crate::domain::dispatch::DISPATCH_TRACE_KEY.into(), trace);
        }
        serde_json::Value::Null => {
            extra = serde_json::json!({ crate::domain::dispatch::DISPATCH_TRACE_KEY: trace });
        }
        _ => {
            extra = serde_json::json!({
                "_derived": extra,
                crate::domain::dispatch::DISPATCH_TRACE_KEY: trace,
            });
        }
    }
    asset.extra = extra;

    // Occurrence-time-driven `created_at` would shadow the ingest
    // clock; leave `created_at` at `Utc::now()` (set by `Asset::new`)
    // so grid "Added" sort surfaces the derivation in arrival order.
    let _ = DateTime::<Utc>::from_timestamp(0, 0);
    Ok(asset)
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
