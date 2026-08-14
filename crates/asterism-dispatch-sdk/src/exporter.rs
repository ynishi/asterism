//! `Exporter` — the trait every outbound adapter implements.
//!
//! An exporter is stateless from the SDK's point of view: the core
//! passes it a [`DispatchContext`] on every method and stores anything
//! the exporter wants to remember about the in-flight job as an opaque
//! [`Handle`]. The three methods split the lifecycle so backends with
//! very different rhythms (single-shot HTTP, long-poll, watchdog on a
//! filesystem drop dir) all fit the same shape:
//!
//! 1. [`Exporter::dispatch`] — send the job to the backend. Returns a
//!    `Handle` the core persists so subsequent calls survive restart.
//! 2. [`Exporter::poll`] — check the backend for progress. Called by
//!    the apalis `DispatchRun` job on a re-enqueue loop until the
//!    state is terminal.
//! 3. [`Exporter::harvest`] — once `poll` returns `Done`, ask the
//!    exporter to collect the produced artefacts and describe them as
//!    [`Derived`]s. The core reifies each Derived into a new `Asset`
//!    with `parent_ids` = the Selection's inputs, `session_id` =
//!    `DispatchJob.id`, and `source_kind` = `format!("dispatch:{}",
//!    exporter_slug)`.

use async_trait::async_trait;
use serde_json::Value;

use crate::derived::Derived;
use crate::handle::Handle;
use crate::state::DispatchState;

/// Errors returned by an exporter. Everything not covered by a
/// dedicated variant flows through [`ExporterError::Other`].
#[derive(Debug, thiserror::Error)]
pub enum ExporterError {
    /// The action string is not one this exporter handles. Reported
    /// during [`Exporter::dispatch`] pre-flight so the core can pick
    /// a different registered adapter (or fail fast when none match).
    #[error("exporter {exporter_slug:?} does not accept action {action:?}")]
    UnsupportedAction {
        /// Slug of the exporter that rejected the action.
        exporter_slug: String,
        /// The action string the core tried to dispatch.
        action: String,
    },
    /// The handle passed to [`Exporter::poll`] or [`Exporter::harvest`]
    /// was issued by a different exporter (programmer error — the
    /// core should route by `DispatchJob.exporter_slug`).
    #[error("handle kind {handle_kind:?} does not belong to exporter {exporter_slug:?}")]
    HandleMismatch {
        /// Slug of the exporter that received the mismatched handle.
        exporter_slug: String,
        /// The `Handle::kind` slug the caller supplied.
        handle_kind: String,
    },
    /// The backend rejected the request (bad params, workflow not
    /// found, auth failure). The exporter translates the underlying
    /// error into a short message; the core records it on the job
    /// row.
    #[error("backend rejected the request: {0}")]
    BackendRejected(String),
    /// Anything else — network I/O, timeout, malformed response,
    /// missing file after harvest.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The Selection-plus-context bundle the core hands the exporter on
/// every method call.
///
/// Carries **references only** — the core owns the entity storage
/// and lets exporters borrow read-only slices for the duration of
/// the call. `inputs` is materialised from `Selection::asset_ids`
/// (which is why an exporter never has to talk to the AssetRepo
/// itself).
///
/// `Copy` because every field is a shared borrow — the runner
/// hands the same context to `dispatch` → `poll` → `harvest` on
/// separate ticks without extra allocation.
///
/// `inputs` carries [`asterism_contract::dto::AssetCardDto`] —
/// the same wire shape the Tauri UI already receives, so backends
/// consuming the SDK see the exact card the human sees in the
/// grid. No SDK-local projection type is invented; the contract
/// crate is the single source of truth for both the frontend and
/// every outbound adapter.
#[derive(Copy, Clone)]
pub struct DispatchContext<'a> {
    /// Assets the user selected. Guaranteed non-empty and all owned by
    /// `persona_id`.
    pub inputs: &'a [asterism_contract::dto::AssetCardDto],
    /// Stable id of the Selection this dispatch was issued from. Same
    /// Selection may be dispatched multiple times to different
    /// exporters; siblings share this id and can be traced back to a
    /// single curation act.
    pub selection_id: &'a str,
    /// Stable id of *this* dispatch. Reified derived Assets carry it
    /// as `session_id` so the grid clusters per-dispatch siblings.
    pub dispatch_id: &'a str,
    /// Pursuit this round is filed under (#29), when the job carries
    /// the stamp — exporters that write a sidecar copy it out beside
    /// `dispatch_id`, so a returning artefact can name its line of
    /// work even where the dispatch row join is unavailable. `None`
    /// on rows that predate the stamp's backfill invariant.
    pub pursuit_id: Option<&'a str>,
    /// Exporter action (`"img2img"`, `"txt2img"`, `"lora_bake"`,
    /// `"multimodal_chat"`, …). Open slug space — new actions are
    /// added as data changes, exporters advertise which they support
    /// via [`Exporter::accepts`].
    pub action: &'a str,
    /// Exporter-specific parameters (workflow ref, prompt, steps,
    /// sampler, …). Opaque to the core; the exporter's schema is
    /// documented in its own crate.
    pub params: &'a Value,
    /// Persona bucket the whole dispatch belongs to.
    pub persona_id: &'a str,
}

/// The single trait every outbound adapter implements.
///
/// Implementations live outside this crate — see the sibling
/// `asterism-exporter-comfy` for the first real one.
#[async_trait]
pub trait Exporter: Send + Sync {
    /// Stable slug that identifies this exporter (e.g. `"comfy"`,
    /// `"gemini"`, `"vdsl"`, `"alc-sd-bake"`). Persisted on
    /// `DispatchJob.exporter_slug` and used as the routing key in the
    /// server-side registry.
    ///
    /// Must stay the same across releases — renaming it looks to the
    /// core like a brand-new exporter and orphans every in-flight job
    /// pointing at the old slug.
    fn slug(&self) -> &str;

    /// Cheap pre-flight check: does this exporter know how to run the
    /// given action string? The core calls this before enqueueing the
    /// `DispatchRun` job so misrouted requests fail fast.
    fn accepts(&self, action: &str) -> bool;

    /// Send the job to the backend. Returns a persistable handle the
    /// core will feed back on subsequent polls and the final harvest.
    ///
    /// Implementations should be *idempotent by [`Handle`]*: if the
    /// core restarts and re-invokes `dispatch` before it saw a handle
    /// (rare, but possible during a crash between backend accept and
    /// handle persist), the backend should not double-queue. In
    /// practice most exporters generate a client-side unique id
    /// derived from `ctx.dispatch_id` and let the backend dedupe on
    /// that.
    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError>;

    /// Ask the backend how the job is doing. Called on a periodic
    /// re-enqueue loop; the returned state's `is_terminal()` result
    /// drives whether the loop stops.
    async fn poll(
        &self,
        ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<DispatchState, ExporterError>;

    /// Collect the produced artefacts from a terminal-`Done` handle.
    /// Called at most once per dispatch. The returned Deriveds are
    /// mapped straight into new Assets by the core.
    async fn harvest(
        &self,
        ctx: DispatchContext<'_>,
        handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError>;
}
