//! `DispatchJob` — one exporter invocation against a Snapshot.
//!
//! Aggregate root for a single outbound job's whole lifecycle
//! (Pending → Running → Done/Failed/Cancelled). Everything the apalis
//! `DispatchRun` runner needs to resume after restart is on this row:
//!
//! - `snapshot_id` — points back at the input Snapshot.
//! - `exporter_slug` + `action` + `params_json` — how to reach the
//!   backend.
//! - `handle_json` — opaque bytes the exporter's `Handle` serialised
//!   into; the runner rehydrates this to call `poll` / `harvest`
//!   again.
//! - `state_slug` — the lifecycle state (mirrors
//!   `asterism_dispatch_sdk::DispatchState::slug()`).
//! - `output_asset_ids` — populated during `harvest` so callers can
//!   go "show me what this dispatch produced" without a follow-up
//!   query.
//!
//! # Invariants
//!
//! 1. `persona_id` is the persona the dispatch belongs to; the
//!    Snapshot carries the same persona (application service
//!    enforces).
//! 2. `state` transitions are one-way: Pending → Running →
//!    (Done | Failed | Cancelled). Backwards transitions are rejected
//!    at the service layer.
//! 3. `output_asset_ids` is empty until state is `Done`; setting it
//!    is atomic with the transition to `Done` (the reify path writes
//!    both in one save).

use chrono::{DateTime, Utc};

use crate::domain::value::{AssetId, DispatchId, PersonaId, SnapshotId};
use crate::error::DomainError;

/// Lifecycle state persisted with the dispatch job.
///
/// Kept as a domain enum so callers can pattern-match without having
/// to depend on `asterism-dispatch-sdk` (the runner does the
/// SDK-side ↔ domain-side mapping).
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchState {
    /// Not yet handed to the backend.
    Pending,
    /// Backend accepted the job and is working on it. The optional
    /// progress hint fields are opportunistic — most backends do
    /// not populate them.
    Running {
        /// Discrete step (backend-defined units).
        current: Option<u64>,
        /// Expected total steps.
        total: Option<u64>,
        /// Human-readable status message.
        message: Option<String>,
    },
    /// Backend finished successfully and the runner has reified the
    /// output (`DispatchJob::output_asset_ids` is populated).
    Done,
    /// Backend reported a permanent failure. `message` is the
    /// exporter-translated short reason.
    Failed {
        /// Short human-readable reason.
        message: String,
    },
    /// Cancelled by the user or during shutdown.
    Cancelled {
        /// Optional short reason (`"user"`, `"shutdown"`, …).
        reason: Option<String>,
    },
}

impl DispatchState {
    /// Slug used on the persisted `state_slug` column. Kept identical
    /// to `asterism_dispatch_sdk::DispatchState::slug` for one-hop
    /// round-tripping.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running { .. } => "running",
            Self::Done => "done",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
        }
    }

    /// True when the state signals the runner should stop polling.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

/// Key under which a reified artefact records the run that made it,
/// inside `Asset::extra`.
///
/// A named constant because two sides depend on the spelling and only
/// one of them writes it: `reify_one` puts the trace there, and anything
/// asking "did this library produce this file, and under which run"
/// reads it. The object holds `selection_id`, `dispatch_id`,
/// `exporter_slug`, and the operator when one was recorded.
pub const DISPATCH_TRACE_KEY: &str = "_dispatch";

/// One dispatch invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchJob {
    /// Surrogate id (UUID v7).
    pub id: DispatchId,
    /// Snapshot whose asset_ids seed this dispatch.
    pub snapshot_id: SnapshotId,
    /// Persona bucket (redundant with the Snapshot's persona, kept
    /// on the row for cheap persona-scoped queries).
    pub persona_id: PersonaId,
    /// Registered exporter slug that will run this job
    /// (`"comfy"`, `"gemini"`, `"vdsl"`, `"alc-sd-bake"`).
    pub exporter_slug: String,
    /// Action string handed to `Exporter::dispatch`
    /// (`"img2img"`, `"txt2img"`, `"lora_bake"`, …).
    pub action: String,
    /// Exporter-specific parameters (opaque JSON).
    pub params: serde_json::Value,
    /// Lifecycle state.
    pub state: DispatchState,
    /// Opaque handle payload returned by `Exporter::dispatch` and
    /// consumed by subsequent `poll` / `harvest` calls. `None`
    /// while the job is `Pending` (nothing to reference yet).
    pub handle: Option<serde_json::Value>,
    /// Handle kind slug (mirrors `Handle::kind`). Persisted alongside
    /// the payload so the runner can double-check it matches
    /// `exporter_slug` on rehydrate.
    pub handle_kind: Option<String>,
    /// Opaque record of the exporter's latest call — what it sent and
    /// what came back (`AttemptRecord::payload` in the dispatch SDK).
    ///
    /// Beside [`handle`](Self::handle) rather than inside it, because
    /// the case it exists for is the one with no handle: a submit the
    /// backend refused returns an error, and everything a reader would
    /// ask about it — which endpoint, with which body, what the backend
    /// said — used to leave with that error. The handle stays what it
    /// is, the exporter's reference to a job that exists.
    ///
    /// One record per row: the latest attempt replaces the one before
    /// it. A re-run after a refusal is a fresh row
    /// ([`DispatchService::redispatch`](crate::application::DispatchService::redispatch)),
    /// so the history a reader wants is already a sequence of rows.
    pub attempt: Option<serde_json::Value>,
    /// Kind slug for [`attempt`](Self::attempt) (mirrors
    /// `AttemptRecord::kind`) — which exporter's grammar the record is
    /// written in.
    pub attempt_kind: Option<String>,
    /// Reified-derived Asset ids. Populated atomically with the
    /// transition to `Done`.
    pub output_asset_ids: Vec<AssetId>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last-updated time (each state transition or handle update
    /// bumps this).
    pub updated_at: DateTime<Utc>,
    /// Wall-clock time the job reached a terminal state, or `None`
    /// if it is still pending / running.
    pub completed_at: Option<DateTime<Utc>>,
    /// Group that was frozen into the snapshot at dispatch time
    /// (snapshot provenance) — `None` for a
    /// direct dispatch of a volatile grid selection. Enables the P4
    /// "then vs now" comparison.
    pub source_group_id: Option<crate::domain::value::GroupId>,
    /// The query rule frozen alongside a query-group dispatch —
    /// reproduction material for the freeze. `None` for manual groups
    /// and volatile selections.
    pub source_query_json: Option<String>,
    /// Agent that started this dispatch (`claude-code`, `codex`,
    /// `asterism-ui`, …) — see
    /// [`attribution`](crate::domain::attribution).
    ///
    /// Persisted rather than passed along because the run outlives its
    /// caller: the exporter is polled by a background job, and the
    /// moment the answer is needed (stamping the reified outputs) is
    /// minutes or hours after the request that supplied it. `None` =
    /// unrecorded.
    ///
    /// **Private, with the two other attribution fields.** They travel
    /// as one triple — set by [`DispatchJob::new`] from the
    /// [`AttributionContext`](crate::domain::attribution::AttributionContext)
    /// the caller chose, restored by [`DispatchJob::from_persisted`],
    /// and handed back out whole by
    /// [`DispatchJob::persisted_attribution`]. Read this one through
    /// [`DispatchJob::operator_ai`](Self::operator_ai).
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    /// Subject that requested this dispatch, persisted for the same
    /// reason the operator is: the reified outputs are stamped long
    /// after the request is gone, and an attribution kept only in memory
    /// would not survive a restart. `None` = unrecorded. Read it through
    /// [`DispatchJob::author`](Self::author).
    author: Option<crate::domain::attribution::Author>,
    /// Channel the pair above arrived through. On a reified output this
    /// keeps its dispatch-time meaning: the channel the *request to
    /// start the run* came in on, not the background job that finished
    /// it. `None` = unrecorded (and on rows written before the column,
    /// an operator with no channel is the legacy shape). Read it through
    /// [`DispatchJob::attributed_via`](Self::attributed_via).
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl DispatchJob {
    /// Builds a fresh Pending job.
    ///
    /// The attribution is a required argument for the same reason
    /// [`Asset::new`](crate::domain::asset::Asset::new)'s is: the caller
    /// has to name the entry point it is, and no shape lets it hold a
    /// context and start a run without it. There used to be a
    /// `with_operator` builder here; it was removed because a stamp
    /// applied *after* construction can carry an operator with no
    /// channel — the exact row shape the write guard exists to stop.
    pub fn new(
        snapshot_id: SnapshotId,
        persona_id: PersonaId,
        exporter_slug: impl Into<String>,
        action: impl Into<String>,
        params: serde_json::Value,
        now: DateTime<Utc>,
        attribution: &crate::domain::attribution::AttributionContext,
    ) -> Result<Self, DomainError> {
        let exporter_slug = exporter_slug.into();
        let action = action.into();
        if exporter_slug.trim().is_empty() {
            return Err(DomainError::Validation(
                "DispatchJob.exporter_slug must not be empty".into(),
            ));
        }
        if action.trim().is_empty() {
            return Err(DomainError::Validation(
                "DispatchJob.action must not be empty".into(),
            ));
        }
        Ok(Self {
            id: DispatchId::new(),
            snapshot_id,
            persona_id,
            exporter_slug,
            action,
            params,
            state: DispatchState::Pending,
            handle: None,
            handle_kind: None,
            attempt: None,
            attempt_kind: None,
            output_asset_ids: Vec::new(),
            source_group_id: None,
            source_query_json: None,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
            created_at: now,
            updated_at: now,
            completed_at: None,
        })
    }

    /// Seed constructor for the read path — the
    /// [`Asset::from_persisted`](crate::domain::asset::Asset::from_persisted)
    /// twin. Restores identity, timestamps and attribution; the
    /// lifecycle columns (`state`, `handle`, outputs, `source_*`) are
    /// assigned by the adapter afterwards.
    ///
    /// Infallible where [`new`](Self::new) validates: the slug and the
    /// action were checked when the row was written, and a stored row is
    /// a fact rather than a request to accept.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: DispatchId,
        snapshot_id: SnapshotId,
        persona_id: PersonaId,
        exporter_slug: String,
        action: String,
        params: serde_json::Value,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        attribution: crate::domain::attribution::PersistedAttribution,
    ) -> Self {
        Self {
            id,
            snapshot_id,
            persona_id,
            exporter_slug,
            action,
            params,
            state: DispatchState::Pending,
            handle: None,
            handle_kind: None,
            attempt: None,
            attempt_kind: None,
            output_asset_ids: Vec::new(),
            source_group_id: None,
            source_query_json: None,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
            created_at,
            updated_at,
            completed_at: None,
        }
    }

    /// Subject that requested this dispatch (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that started this dispatch (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded, or a
    /// pre-column row).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the whole triple back out — the outlet the reify path
    /// reads so it can carry the dispatch-time attribution onto the
    /// assets it mints, minutes or hours after the request that
    /// supplied it is gone.
    ///
    /// Returns [`PersistedAttribution`](crate::domain::attribution::PersistedAttribution)
    /// rather than three loose values because that is the only type
    /// [`AttributionContext::from_persisted`](crate::domain::attribution::AttributionContext::from_persisted)
    /// accepts, and it cannot be assembled by a caller — so this is an
    /// outlet for a recorded fact, not a way to mint an arbitrary
    /// channel.
    pub fn persisted_attribution(&self) -> crate::domain::attribution::PersistedAttribution {
        crate::domain::attribution::PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }

    /// Stamps the dispatch-time provenance: the source Group
    /// (when the input was a Group rather than a volatile selection)
    /// and, for query groups, the rule that produced the freeze.
    pub fn with_source(
        mut self,
        source_group_id: Option<crate::domain::value::GroupId>,
        source_query_json: Option<String>,
    ) -> Self {
        self.source_group_id = source_group_id;
        self.source_query_json = source_query_json;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_slugs_are_stable() {
        assert_eq!(DispatchState::Pending.slug(), "pending");
        assert_eq!(
            DispatchState::Running {
                current: None,
                total: None,
                message: None,
            }
            .slug(),
            "running"
        );
        assert_eq!(DispatchState::Done.slug(), "done");
        assert_eq!(
            DispatchState::Failed {
                message: "boom".into()
            }
            .slug(),
            "failed"
        );
        assert_eq!(
            DispatchState::Cancelled { reason: None }.slug(),
            "cancelled"
        );
    }

    #[test]
    fn only_terminal_states_are_terminal() {
        assert!(!DispatchState::Pending.is_terminal());
        assert!(
            !DispatchState::Running {
                current: None,
                total: None,
                message: None,
            }
            .is_terminal()
        );
        assert!(DispatchState::Done.is_terminal());
        assert!(DispatchState::Failed { message: "".into() }.is_terminal());
        assert!(DispatchState::Cancelled { reason: None }.is_terminal());
    }

    #[test]
    fn new_rejects_empty_slugs() {
        let now = Utc::now();
        let snap = SnapshotId::new();
        let persona = PersonaId::new();
        let ctx = crate::domain::attribution::AttributionContext::unrecorded();
        assert!(
            DispatchJob::new(
                snap,
                persona,
                "",
                "img2img",
                serde_json::json!({}),
                now,
                &ctx
            )
            .is_err()
        );
        assert!(
            DispatchJob::new(
                snap,
                persona,
                "comfy",
                "   ",
                serde_json::json!({}),
                now,
                &ctx
            )
            .is_err()
        );
        assert!(
            DispatchJob::new(
                snap,
                persona,
                "comfy",
                "img2img",
                serde_json::json!({}),
                now,
                &ctx
            )
            .is_ok()
        );
    }

    #[test]
    fn the_requested_attribution_survives_the_round_trip_the_run_outlives() {
        use crate::domain::attribution::{
            AttributionChannel, AttributionContext, Author, PersistedAttribution,
        };

        let job = DispatchJob::new(
            SnapshotId::new(),
            PersonaId::new(),
            "comfy",
            "img2img",
            serde_json::json!({}),
            Utc::now(),
            &AttributionContext::owner_surface(),
        )
        .unwrap();
        assert_eq!(job.author(), Some(&Author::Owner));
        assert_eq!(job.attributed_via(), Some(AttributionChannel::OwnerSurface));

        // The outlet is what reify reads: the same triple, in the one
        // shape `AttributionContext::from_persisted` will take.
        let restored = AttributionContext::from_persisted(job.persisted_attribution());
        assert_eq!(restored.author(), Some(&Author::Owner));
        assert_eq!(
            restored.attributed_via(),
            Some(AttributionChannel::OwnerSurface),
            "the owner survives an outlet no constructor could have produced"
        );

        // And a stored V48-era row reads back as what it is — an
        // operator with no channel — rather than being refused.
        let legacy = DispatchJob::from_persisted(
            DispatchId::new(),
            SnapshotId::new(),
            PersonaId::new(),
            "comfy".into(),
            "img2img".into(),
            serde_json::json!({}),
            Utc::now(),
            Utc::now(),
            PersistedAttribution::from_columns(None, None, Some("claude-code"), None).unwrap(),
        );
        assert_eq!(
            legacy.operator_ai().map(|o| o.as_str()),
            Some("claude-code")
        );
        assert_eq!(legacy.attributed_via(), None);
    }
}
