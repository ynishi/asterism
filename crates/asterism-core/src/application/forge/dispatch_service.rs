//! `DispatchService` — the transport-fronted half of the outbound
//! dispatch lifecycle.
//!
//! Every verb here is behind a Tauri command, an HTTP route, or both:
//!
//! - [`create`](DispatchService::create) — start a new dispatch from
//!   an existing Snapshot, enqueueing an apalis `DispatchRun` job.
//! - [`run`](DispatchService::run) / [`redispatch`](DispatchService::redispatch)
//!   — freeze a live source (Group or volatile pick) and start it, or
//!   re-run a previous freeze unchanged.
//! - [`get`](DispatchService::get) / [`list`](DispatchService::list)
//!   — read the persisted state (used by the UI polling endpoint and
//!   an MCP tool).
//!
//! Each of the three start verbs takes an
//! [`AttributionContext`](crate::domain::attribution::AttributionContext)
//! and stamps it on the job row, because the run outlives its caller:
//! the exporter is polled by a background job, and the moment the answer
//! is needed (stamping the reified outputs) is minutes or hours after
//! the request that supplied it.
//!
//! What the runner does to a dispatch in flight — `save_state` /
//! `save_handle` / `reify` — is not here. Those live on
//! [`DispatchRunnerService`](crate::application_support::DispatchRunnerService),
//! reachable from the `DispatchRun` job's environment and from no
//! transport context, so no handler can park a job in `Done` or mint
//! assets from a wire payload.

use std::sync::Arc;

use asterism_contract::command::CreateDispatchCommand;
use asterism_contract::dto::DispatchDto;
use chrono::Utc;

use crate::application::attribution_intake::refuse_assertion_from_owner_surface;
use crate::application::mapping::{
    dispatch_to_dto, parse_dispatch_id, parse_pursuit_id, parse_snapshot_id,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::dispatch::DispatchJob;
use crate::domain::forge::pursuit::Pursuit;
use crate::domain::forge::repository::PursuitRepository;
use crate::domain::forge::value::PursuitId;
use crate::domain::job::JobKind;
use crate::domain::repository::{DispatchRepository, JobQueue, SnapshotRepository};
use crate::domain::value::PersonaId;
use crate::error::DomainError;

/// Outbound-dispatch use-case service.
pub struct DispatchService {
    snapshots: Arc<dyn SnapshotRepository>,
    dispatches: Arc<dyn DispatchRepository>,
    jobs: Arc<dyn JobQueue>,
    /// Group ports for the live-source dispatch path: resolve a
    /// Group's kind / rule, refresh a query group before freezing, and
    /// read the membership in `position` order.
    groups: Arc<dyn crate::domain::repository::GroupRepository>,
    query_groups: Arc<dyn crate::domain::repository::QueryGroupRepository>,
    query_group_service: Arc<crate::application::query_group_service::QueryGroupService>,
    /// The mint half of always-mint (#29): every start verb stamps a
    /// pursuit, minting one when the caller supplied none.
    pursuits: Arc<dyn PursuitRepository>,
}

impl DispatchService {
    /// Wires the service around its ports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        snapshots: Arc<dyn SnapshotRepository>,
        dispatches: Arc<dyn DispatchRepository>,
        jobs: Arc<dyn JobQueue>,
        groups: Arc<dyn crate::domain::repository::GroupRepository>,
        query_groups: Arc<dyn crate::domain::repository::QueryGroupRepository>,
        query_group_service: Arc<crate::application::query_group_service::QueryGroupService>,
        pursuits: Arc<dyn PursuitRepository>,
    ) -> Self {
        Self {
            snapshots,
            dispatches,
            jobs,
            groups,
            query_groups,
            query_group_service,
            pursuits,
        }
    }

    /// Resolves the pursuit a new round files under: the supplied id
    /// (validated to exist in the caller's persona — continuation is
    /// explicit, never inferred, and never crosses personas), or a
    /// fresh anonymous pursuit minted here (always-mint: work cannot
    /// happen outside a pursuit, there is no detached state).
    ///
    /// The mint and the dispatch write are two repository calls, not
    /// one transaction — deliberately. If the dispatch write fails
    /// after the mint, what remains is an empty anonymous pursuit,
    /// which is exactly the pre-created / stranded state the model
    /// already defines as an honest record (repairable by restamp,
    /// closable as abandoned). A cross-aggregate transaction would buy
    /// nothing but coupling.
    async fn resolve_pursuit(
        &self,
        supplied: Option<&str>,
        persona_id: PersonaId,
        now: chrono::DateTime<chrono::Utc>,
        attribution: &AttributionContext,
    ) -> Result<PursuitId, DomainError> {
        match supplied {
            Some(wire) => {
                let id = parse_pursuit_id(wire)?;
                let pursuit = self
                    .pursuits
                    .find(&id)
                    .await?
                    .ok_or_else(|| DomainError::not_found("pursuit", wire))?;
                if pursuit.persona_id != persona_id {
                    return Err(DomainError::Validation(
                        "pursuit belongs to a different persona".into(),
                    ));
                }
                Ok(id)
            }
            None => {
                let minted = Pursuit::new(persona_id, None, None, None, None, now, attribution);
                self.pursuits.create(&minted).await?;
                Ok(minted.id)
            }
        }
    }

    /// Persists a fresh `Pending` job and enqueues its apalis run —
    /// the shared tail of `create` / `run` / `redispatch`.
    async fn save_and_enqueue(&self, job: &DispatchJob) -> Result<(), DomainError> {
        self.dispatches.save(job).await?;
        // Fire-and-forget enqueue — the runner picks it up on the next
        // tick. A queue failure is not fatal for the persisted row; it
        // sits in `pending` until manually retried.
        let _ = self
            .jobs
            .enqueue(
                JobKind::DispatchRun,
                serde_json::json!({ "dispatch_id": job.id.to_string() }),
            )
            .await;
        Ok(())
    }

    /// Live-source dispatch (`dispatch_run`): freeze a
    /// Group (query groups are refreshed synchronously first, so the
    /// freeze is always fresh) or a volatile grid selection into a
    /// content-hash-deduped Snapshot, stamp the provenance, and enqueue
    /// the run.
    pub async fn run(
        &self,
        command: asterism_contract::command::DispatchRunCommand,
        attribution: &AttributionContext,
    ) -> Result<DispatchDto, DomainError> {
        let persona_id = crate::application::mapping::parse_persona_id(&command.persona_id)?;
        // The command's `operator_ai` is the remote adapters' assertion
        // carrier; the adapter translated it into the context above, so
        // the only thing left to do with the field here is refuse the
        // contradiction of it arriving on the owner's own surface.
        refuse_assertion_from_owner_surface(
            attribution,
            &[("operator_ai", command.operator_ai.is_some())],
        )?;
        let params: serde_json::Value = if command.params_json.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&command.params_json)
                .map_err(|e| DomainError::Validation(format!("invalid params_json: {e}")))?
        };

        // Resolve the input members + provenance.
        let (member_ids, source_group_id, source_query_json) =
            match (command.group_id.as_deref(), command.asset_ids.is_empty()) {
                (Some(gid), true) => {
                    let group_id = crate::application::mapping::parse_group_id(gid)?;
                    let group = self
                        .groups
                        .find(&group_id)
                        .await?
                        .ok_or_else(|| DomainError::not_found("group", gid))?;
                    if group.persona_id != persona_id {
                        return Err(DomainError::Validation(
                            "group belongs to a different persona".into(),
                        ));
                    }
                    // A query group gets one synchronous refresh so
                    // "freeze the evaluated result" matches the promote
                    // path's semantics; failure aborts the dispatch loudly.
                    if group.kind == crate::domain::group::GroupKind::Query {
                        let rule = group.query_json.clone().ok_or_else(|| {
                            DomainError::Validation("query group carries no rule".into())
                        })?;
                        self.query_group_service
                            .evaluate_and_materialize(&rule, &persona_id, &group_id)
                            .await?;
                    }
                    let members = self.query_groups.member_ids(&group_id).await?;
                    (members, Some(group_id), group.query_json)
                }
                (None, false) => {
                    let members = command
                        .asset_ids
                        .iter()
                        .map(|s| crate::application::mapping::parse_asset_id(s))
                        .collect::<Result<Vec<_>, _>>()?;
                    (members, None, None)
                }
                (Some(_), false) => {
                    return Err(DomainError::Validation(
                        "dispatch_run takes either group_id or asset_ids, not both".into(),
                    ));
                }
                (None, true) => {
                    return Err(DomainError::Validation(
                        "dispatch_run needs a group_id or a non-empty asset_ids".into(),
                    ));
                }
            };

        let now = Utc::now();
        let snapshot = crate::domain::snapshot::Snapshot::new(persona_id, member_ids, now)?;
        let stored = self.snapshots.create_or_reuse(&snapshot).await?;
        let mut job = DispatchJob::new(
            stored.id,
            persona_id,
            command.exporter_slug,
            command.action,
            params,
            now,
            attribution,
        )?
        .with_source(source_group_id, source_query_json);
        // Minted only after the job passed its own validation: a bad
        // slug is a refused request, and a refused request should not
        // strand an empty pursuit (a strand is legal, but it is the
        // price of a *failed write*, not of a typo).
        job.pursuit_id = Some(
            self.resolve_pursuit(command.pursuit_id.as_deref(), persona_id, now, attribution)
                .await?
                .as_correlation(),
        );
        self.save_and_enqueue(&job).await?;
        Ok(dispatch_to_dto(&job))
    }

    /// Re-runs a dispatch with the same frozen input / exporter /
    /// action / params (P2). The snapshot row is shared; only a
    /// new job row is written.
    pub async fn redispatch(
        &self,
        command: asterism_contract::command::RedispatchCommand,
        attribution: &AttributionContext,
    ) -> Result<DispatchDto, DomainError> {
        let did = parse_dispatch_id(&command.dispatch_id)?;
        let prior = self
            .dispatches
            .find(&did)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", &command.dispatch_id))?;
        let now = Utc::now();
        let mut job = DispatchJob::new(
            prior.snapshot_id,
            prior.persona_id,
            prior.exporter_slug.clone(),
            prior.action.clone(),
            prior.params.clone(),
            now,
            // The frozen input, exporter, action and params are what the
            // re-run repeats; the earlier run's attribution is not among
            // them. Whoever started that run did not start this one, and
            // copying their answer forward would put an assertion nobody
            // made on a fresh row — so this row records *this* request's
            // channel, exactly like a first run.
            attribution,
        )?
        .with_source(prior.source_group_id, prior.source_query_json.clone());
        // The pursuit *is* inherited where the attribution above is
        // not: the caller named the prior round literally, and a re-run
        // is a new round of the same line of work (a new patchset on
        // the same change) — an explicit reference, not the
        // membership-overlap inference the model forbids. A prior from
        // before the stamp invariant can still be NULL; then the
        // re-run mints, as any unstamped work does.
        job.pursuit_id = match command.pursuit_id.as_deref() {
            Some(wire) => Some(
                self.resolve_pursuit(Some(wire), prior.persona_id, now, attribution)
                    .await?
                    .as_correlation(),
            ),
            None => match prior.pursuit_id {
                Some(inherited) => Some(inherited),
                None => Some(
                    self.resolve_pursuit(None, prior.persona_id, now, attribution)
                        .await?
                        .as_correlation(),
                ),
            },
        };
        self.save_and_enqueue(&job).await?;
        Ok(dispatch_to_dto(&job))
    }

    /// Creates a new `DispatchJob` in `Pending` state and enqueues an
    /// apalis `DispatchRun` task for the runner to drive.
    ///
    /// The exporter is **not** invoked here — this method returns
    /// immediately so the caller can start polling. Registry lookup
    /// happens on the runner side; passing an unregistered
    /// `exporter_slug` fails there rather than at create time (the
    /// tradeoff is deliberate: it keeps this service dependency-free
    /// of the exporter registry, which lives in the server/infra
    /// layer).
    pub async fn create(
        &self,
        command: CreateDispatchCommand,
        attribution: &AttributionContext,
    ) -> Result<DispatchDto, DomainError> {
        let snapshot_id = parse_snapshot_id(&command.snapshot_id)?;
        refuse_assertion_from_owner_surface(
            attribution,
            &[("operator_ai", command.operator_ai.is_some())],
        )?;
        let snapshot = self
            .snapshots
            .find(&snapshot_id)
            .await?
            .ok_or_else(|| DomainError::not_found("snapshot", &command.snapshot_id))?;

        let params: serde_json::Value = if command.params_json.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&command.params_json)
                .map_err(|e| DomainError::Validation(format!("invalid params_json: {e}")))?
        };

        let now = Utc::now();
        let mut job = DispatchJob::new(
            snapshot.id,
            snapshot.persona_id,
            command.exporter_slug,
            command.action,
            params,
            now,
            attribution,
        )?;
        // Post-validation mint, as in `run`: a typo does not strand.
        job.pursuit_id = Some(
            self.resolve_pursuit(
                command.pursuit_id.as_deref(),
                snapshot.persona_id,
                now,
                attribution,
            )
            .await?
            .as_correlation(),
        );
        self.save_and_enqueue(&job).await?;
        Ok(dispatch_to_dto(&job))
    }

    /// Fetches one dispatch by wire id.
    pub async fn get(&self, id: &str) -> Result<DispatchDto, DomainError> {
        let did = parse_dispatch_id(id)?;
        let job = self
            .dispatches
            .find(&did)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", id))?;
        Ok(dispatch_to_dto(&job))
    }

    /// Lists dispatch jobs with the same predicate surface as the
    /// underlying `DispatchRepository::list`.
    pub async fn list(
        &self,
        persona_id: Option<&str>,
        snapshot_id: Option<&str>,
        state_slug: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DispatchDto>, DomainError> {
        let persona_uuid = persona_id
            .map(crate::application::mapping::parse_persona_id)
            .transpose()?;
        let snapshot_uuid = snapshot_id.map(parse_snapshot_id).transpose()?;
        let rows = self
            .dispatches
            .list(
                persona_uuid.as_ref(),
                snapshot_uuid.as_ref(),
                state_slug,
                limit,
            )
            .await?;
        Ok(rows.iter().map(dispatch_to_dto).collect())
    }
}
