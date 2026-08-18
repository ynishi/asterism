//! `PursuitService` — the lifecycle verbs of the minted unit of work
//! (#29): open, close, reopen, restamp, and the reads that derive
//! standing.
//!
//! Always-mint lives in
//! [`DispatchService`](super::dispatch_service::DispatchService) —
//! a dispatch arriving unstamped mints its own pursuit there. This
//! service is everything else: the explicit pre-create (naming intent
//! up front), the one-way lifecycle facts (close / reopen — recorded,
//! never a status write), the close's single deliberate
//! materialisation (the kept set frozen into a snapshot), and the
//! restamp repair verb. Transport routes land in the next slice of
//! #29; until then the service fronts the e2e surface through
//! `CoreCtx`, the same way every service is reachable there.
//!
//! # The close freeze is a forge-side calling convention
//!
//! The core treats snapshot member order as part of snapshot identity
//! (caller order, nothing sorts). The close path sorts the kept set
//! ascending *itself* before freezing, so identical kept sets dedupe
//! across closes; a close snapshot consequently does not dedupe with a
//! pick-ordered input snapshot over the same members — correct, they
//! are different statements.

use std::sync::Arc;

use asterism_contract::command::{
    ClosePursuitCommand, OpenPursuitCommand, RecordPursuitTxCommand, ReopenPursuitCommand,
    RestampDispatchCommand,
};
use asterism_contract::dto::{
    AssetCullDto, CullDto, CullMemberDto, DispatchDto, PursuitDto, PursuitEventDto, PursuitTxDto,
};
use chrono::Utc;

use crate::application::mapping::{
    dispatch_to_dto, parse_asset_id, parse_dispatch_id, parse_persona_id, parse_project_id,
    parse_pursuit_id, parse_snapshot_id,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::forge::cull::{
    Cull, CullMember, CullVerdict, RequestedVerdict, resolve_verdicts,
};
use crate::domain::forge::pursuit::{
    Pursuit, PursuitEvent, PursuitEventKind, PursuitRestamp, RestampSubject, standing,
};
use crate::domain::forge::tx::{PursuitTx, PursuitTxKind, TxOrigin, ledger};
use crate::domain::forge::value::PursuitId;
use crate::domain::repository::{
    AssetRepository, DispatchRepository, PersonaRepository, ProjectRepository, PursuitRepository,
};
use crate::domain::value::{AssetId, PersonaId, SnapshotId};
use crate::error::DomainError;

/// Pursuit lifecycle use-case service.
pub struct PursuitService {
    pursuits: Arc<dyn PursuitRepository>,
    /// Filing is checked against the project it names — existence, and
    /// that it belongs to the same persona, which no foreign key can
    /// say.
    projects: Arc<dyn ProjectRepository>,
    personas: Arc<dyn PersonaRepository>,
    dispatches: Arc<dyn DispatchRepository>,
    /// The ledger's `in` names an asset row; the existence and persona
    /// checks read it here.
    assets: Arc<dyn AssetRepository>,
    /// The close freezes go through the snapshot service rather than
    /// the repository so the frozen ids get the same existence /
    /// persona / fold-redirect hydration every other freeze gets.
    snapshots: Arc<crate::application::SnapshotService>,
}

impl PursuitService {
    /// Wires the service around its ports.
    pub fn new(
        pursuits: Arc<dyn PursuitRepository>,
        projects: Arc<dyn ProjectRepository>,
        personas: Arc<dyn PersonaRepository>,
        dispatches: Arc<dyn DispatchRepository>,
        assets: Arc<dyn AssetRepository>,
        snapshots: Arc<crate::application::SnapshotService>,
    ) -> Self {
        Self {
            pursuits,
            projects,
            personas,
            dispatches,
            assets,
            snapshots,
        }
    }

    /// Opens a pursuit explicitly. The always-mint rule makes this
    /// optional; pre-creating lets a caller name intent before the
    /// first round, and a pre-created pursuit that never receives work
    /// is an honest record.
    pub async fn open(
        &self,
        command: OpenPursuitCommand,
        attribution: &AttributionContext,
    ) -> Result<PursuitDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        self.personas
            .find(&persona_id)
            .await?
            .ok_or_else(|| DomainError::not_found("persona", &command.persona_id))?;
        let parent_id = match command.parent_pursuit_id.as_deref() {
            None => None,
            Some(wire) => {
                let id = parse_pursuit_id(wire)?;
                let parent = self
                    .pursuits
                    .find(&id)
                    .await?
                    .ok_or_else(|| DomainError::not_found("pursuit", wire))?;
                // `parent_id` never crosses personas — the invariant is
                // enforced here because only this layer sees both rows.
                if parent.persona_id != persona_id {
                    return Err(DomainError::Validation(
                        "parent pursuit belongs to a different persona".into(),
                    ));
                }
                Some(id)
            }
        };
        // Filing crosses no personas either, and the foreign key cannot
        // hold that: `project` carries its own `persona_id` and
        // `pursuit.project_id` references only `project(id)`, so a
        // pursuit filed under someone else's project would be an
        // ordinary-looking row. Checked here, where both are visible,
        // exactly like `parent_id` above.
        //
        // A parent's filing is deliberately not inherited: a child says
        // where it files or files nowhere. Inheritance would make the
        // filing of a whole subtree move with a decision taken once at
        // its root, which is a different verb from the one this is.
        let project_id = match command.project_id.as_deref() {
            None => None,
            Some(wire) => {
                let id = parse_project_id(wire)?;
                let project = self
                    .projects
                    .find(&id)
                    .await?
                    .ok_or_else(|| DomainError::not_found("project", wire))?;
                if project.persona_id != persona_id {
                    return Err(DomainError::Validation(
                        "project belongs to a different persona".into(),
                    ));
                }
                Some(id)
            }
        };
        // A caller-chosen id is the repair path (a returning artefact
        // claimed a pursuit that has no row here); absent, the id is
        // minted like every other open. Either way the create is a
        // create — an id already taken collides at the repository
        // rather than adopting the row that is there.
        let pursuit = match command.pursuit_id.as_deref() {
            None => Pursuit::new(
                persona_id,
                project_id,
                parent_id,
                command.title,
                command.note,
                Utc::now(),
                attribution,
            ),
            Some(wire) => Pursuit::new_at(
                parse_pursuit_id(wire)?,
                persona_id,
                project_id,
                parent_id,
                command.title,
                command.note,
                Utc::now(),
                attribution,
            ),
        };
        self.pursuits.create(&pursuit).await?;
        // Fresh row, no events yet: standing is open by definition.
        Ok(pursuit_to_dto(&pursuit, "open"))
    }

    /// Records a close. One-way and repeatable: a second close is a
    /// new fact and standing re-derives; nothing is edited.
    ///
    /// `satisfied` is where the cull is recorded (#22, model on #63).
    /// The candidate set is derived from the pursuit's own ledger —
    /// never supplied by the caller — and frozen into a snapshot; the
    /// command's verdicts are resolved against it (removed and
    /// unspoken culls as `reject`, untouched and unspoken gets no
    /// row); and the kept set the event freezes is exactly the `keep`
    /// verdicts (ascending, see the module doc) — the merge-product
    /// analogue, immediately usable as the input of a next dispatch or
    /// a child pursuit. No keeps records `None`: "concluded with
    /// nothing kept" is a defined state, because an empty snapshot is
    /// domain-rejected. The rejected side is deliberately not
    /// snapshotted — its rows stay live and restorable, judged in the
    /// cull. A close with no verdicts to record writes no cull row.
    ///
    /// The event and the cull land in one repository transaction; the
    /// two freezes happen first and are content-addressed, so a close
    /// that fails between them leaves only unreferenced snapshots
    /// behind — harmless, shared with any later freeze of the same
    /// sets.
    pub async fn close(
        &self,
        command: ClosePursuitCommand,
        attribution: &AttributionContext,
    ) -> Result<PursuitEventDto, DomainError> {
        let pursuit_id = parse_pursuit_id(&command.pursuit_id)?;
        let pursuit = self
            .pursuits
            .find(&pursuit_id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", &command.pursuit_id))?;
        let kind = match command.outcome.as_str() {
            "satisfied" => PursuitEventKind::ClosedSatisfied,
            "abandoned" => PursuitEventKind::ClosedAbandoned,
            other => {
                return Err(DomainError::Validation(format!(
                    "unknown close outcome {other:?}: expected \"satisfied\" or \"abandoned\""
                )));
            }
        };
        if kind == PursuitEventKind::ClosedAbandoned {
            // Applies nothing — no cull, no freeze (#63): the
            // GitHub-shaped close-without-merging.
            if !command.verdicts.is_empty() {
                return Err(DomainError::Validation(
                    "an abandoned close decides nothing; verdicts must be empty".into(),
                ));
            }
            let event = PursuitEvent::new(
                pursuit.id,
                pursuit.persona_id,
                kind,
                None,
                command.note,
                Utc::now(),
                attribution,
            )?;
            self.pursuits.append_close(&event, None).await?;
            return Ok(event_to_dto(&event));
        }
        let requested = command
            .verdicts
            .iter()
            .map(|entry| {
                Ok(RequestedVerdict {
                    asset_id: parse_asset_id(&entry.asset_id)?,
                    verdict: CullVerdict::parse(&entry.verdict)?,
                    note: entry.note.clone(),
                })
            })
            .collect::<Result<Vec<_>, DomainError>>()?;
        let txs = self.pursuits.txs_of(&pursuit.id).await?;
        let state = ledger(&txs);
        let resolved = resolve_verdicts(&state, &requested)?;
        // The ledger is history and history outlives the asset (its
        // rows carry no FK) — but a freeze cannot hold what no longer
        // exists, and the snapshot service refuses dead ids. So
        // existence is read once per member: a purged member can
        // still be *rejected* (the verdict row outlives the asset
        // too), a `keep` of one is refused — a kept set that silently
        // dropped a keep would misstate "kept = the keep verdicts" —
        // and the freezes hold the surviving members.
        let mut existing: std::collections::BTreeSet<AssetId> = std::collections::BTreeSet::new();
        for asset_id in state.keys() {
            if self.assets.find(asset_id).await?.is_some() {
                existing.insert(*asset_id);
            }
        }
        for verdict in &resolved {
            if verdict.verdict == CullVerdict::Keep && !existing.contains(&verdict.asset_id) {
                return Err(DomainError::Validation(format!(
                    "keep of {}: the asset no longer exists; a purged member \
                     can only be rejected",
                    verdict.asset_id
                )));
            }
        }
        let kept: Vec<AssetId> = resolved
            .iter()
            .filter(|r| r.verdict == CullVerdict::Keep)
            .map(|r| r.asset_id)
            .collect();
        let kept_snapshot = if kept.is_empty() {
            None
        } else {
            Some(
                self.freeze_canonical(pursuit.persona_id, kept, attribution)
                    .await?,
            )
        };
        let now = Utc::now();
        let event = PursuitEvent::new(
            pursuit.id,
            pursuit.persona_id,
            kind,
            kept_snapshot,
            command.note,
            now,
            attribution,
        )?;
        // The candidate set is everything the ledger admitted that
        // still exists — removed members included (removal is a
        // verdict input, not an exit), purged members excluded (see
        // above; their verdict rows still name them). A close whose
        // surviving candidate set is empty writes no cull: there is
        // no set left to say "out of".
        let candidates: Vec<AssetId> = state
            .keys()
            .filter(|id| existing.contains(id))
            .copied()
            .collect();
        let cull_payload = if resolved.is_empty() || candidates.is_empty() {
            None
        } else {
            let candidate_snapshot = self
                .freeze_canonical(pursuit.persona_id, candidates, attribution)
                .await?;
            let cull = Cull::new(
                pursuit.id,
                pursuit.persona_id,
                event.id,
                candidate_snapshot,
                command.cull_note,
                now,
                attribution,
            );
            let members: Vec<CullMember> = resolved
                .into_iter()
                .map(|r| CullMember {
                    cull_id: cull.id,
                    asset_id: r.asset_id,
                    verdict: r.verdict,
                    note: r.note,
                })
                .collect();
            Some((cull, members))
        };
        self.pursuits
            .append_close(
                &event,
                cull_payload
                    .as_ref()
                    .map(|(cull, members)| (cull, members.as_slice())),
            )
            .await?;
        Ok(event_to_dto(&event))
    }

    /// Appends one membership gesture to a pursuit's ledger (#22).
    /// Refusals, not repairs: an `in` of a present member (`unremove`
    /// is the re-entry verb for a removed one), a `remove` of a
    /// non-member, an `unremove` of an unremoved one — each names a
    /// gesture that misunderstands the ledger, and recording it would
    /// record the misunderstanding.
    pub async fn record_tx(
        &self,
        command: RecordPursuitTxCommand,
        attribution: &AttributionContext,
    ) -> Result<PursuitTxDto, DomainError> {
        let pursuit_id = parse_pursuit_id(&command.pursuit_id)?;
        let pursuit = self
            .pursuits
            .find(&pursuit_id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", &command.pursuit_id))?;
        let asset_id = parse_asset_id(&command.asset_id)?;
        let kind = match (command.kind.as_str(), command.origin.as_deref()) {
            // Aim and scope stay unset here: this command cannot yet
            // carry them, and a targeted IN is the filing verb's to
            // record (#63 decision 4) once a pursuit has a project
            // whose entries it could be aiming at.
            ("in", Some(origin)) => PursuitTxKind::In {
                origin: TxOrigin::parse(origin)?,
                target: None,
                out_of_scope: false,
            },
            ("in", None) => {
                return Err(DomainError::Validation(
                    "an 'in' names its origin: generated, imported, or existing".into(),
                ));
            }
            ("update", _) => {
                return Err(DomainError::Validation(
                    "'update' is the reserved round-trip verb (#63); nothing records it yet".into(),
                ));
            }
            (_, Some(_)) => {
                return Err(DomainError::Validation(
                    "origin rides 'in' and nothing else".into(),
                ));
            }
            ("remove", None) => PursuitTxKind::Remove,
            ("unremove", None) => PursuitTxKind::Unremove,
            (other, None) => {
                return Err(DomainError::Validation(format!(
                    "unknown pursuit tx kind {other:?}: expected \"in\", \"remove\" or \
                     \"unremove\""
                )));
            }
        };
        let txs = self.pursuits.txs_of(&pursuit.id).await?;
        let state = ledger(&txs);
        match kind {
            PursuitTxKind::In { .. } => {
                let asset = self
                    .assets
                    .find(&asset_id)
                    .await?
                    .ok_or_else(|| DomainError::not_found("asset", &command.asset_id))?;
                if asset.persona_id != pursuit.persona_id {
                    return Err(DomainError::Validation(
                        "asset belongs to a different persona than the pursuit".into(),
                    ));
                }
                match state.get(&asset_id) {
                    None => {}
                    Some(member) if member.removed => {
                        return Err(DomainError::Validation(
                            "a removed member re-enters by 'unremove', not a second 'in'".into(),
                        ));
                    }
                    Some(_) => {
                        return Err(DomainError::Conflict(format!(
                            "asset {asset_id} is already a member of this pursuit"
                        )));
                    }
                }
            }
            PursuitTxKind::Remove => match state.get(&asset_id) {
                Some(member) if !member.removed => {}
                Some(_) => {
                    return Err(DomainError::Validation(format!(
                        "asset {asset_id} is already removed"
                    )));
                }
                None => {
                    return Err(DomainError::Validation(format!(
                        "asset {asset_id} is not a member of this pursuit"
                    )));
                }
            },
            PursuitTxKind::Unremove => match state.get(&asset_id) {
                Some(member) if member.removed => {}
                Some(_) => {
                    return Err(DomainError::Validation(format!(
                        "asset {asset_id} is not removed"
                    )));
                }
                None => {
                    return Err(DomainError::Validation(format!(
                        "asset {asset_id} is not a member of this pursuit"
                    )));
                }
            },
            PursuitTxKind::Update { .. } => unreachable!("refused above"),
        }
        let tx = PursuitTx::new(
            pursuit.id,
            pursuit.persona_id,
            kind,
            asset_id,
            command.note,
            Utc::now(),
            attribution,
        )?;
        self.pursuits.append_tx(&tx).await?;
        Ok(tx_to_dto(&tx))
    }

    /// Every verdict ever recorded about one asset, most-recent first
    /// — the acceptance read of #22: who decided to keep or drop it,
    /// out of which set, in which line of work.
    pub async fn asset_culls(
        &self,
        asset_id: &str,
        limit: u32,
    ) -> Result<Vec<AssetCullDto>, DomainError> {
        let asset = parse_asset_id(asset_id)?;
        let rows = self.pursuits.culls_for_asset(&asset, limit).await?;
        Ok(rows
            .into_iter()
            .map(|(cull, member)| AssetCullDto {
                cull_id: cull.id.to_string(),
                pursuit_id: cull.pursuit_id.to_string(),
                candidate_snapshot_id: cull.candidate_snapshot_id.to_string(),
                verdict: member.verdict.slug().to_string(),
                note: member.note,
                author_kind: cull.author().map(|a| a.kind_slug().to_string()),
                operator_ai: cull.operator_ai().map(|o| o.as_str().to_string()),
                created_at_ms: cull.created_at.timestamp_millis(),
            })
            .collect())
    }

    /// Freezes a canonical (ascending, deduplicated) set through the
    /// snapshot service, re-freezing once when fold-redirect changed
    /// the shape — see the comment inside; the first freeze's row
    /// stays as an unreferenced content-addressed snapshot, harmless.
    async fn freeze_canonical(
        &self,
        persona_id: PersonaId,
        mut ids: Vec<AssetId>,
        attribution: &AttributionContext,
    ) -> Result<SnapshotId, DomainError> {
        // Sort by parsed id, not by string: the convention is "asset
        // id ascending", and the id is the contract. Dedup after
        // sorting — the same asset named twice is one membership.
        ids.sort();
        ids.dedup();
        let mut frozen = self
            .snapshots
            .create(
                asterism_contract::command::CreateSnapshotCommand {
                    persona_id: persona_id.to_string(),
                    asset_ids: ids.iter().map(|a| a.to_string()).collect(),
                },
                attribution,
            )
            .await?;
        // The freeze redirects fold headstones to their keepers in
        // place, preserving position — which can un-sort the set this
        // path just sorted (and two headstones can collapse onto one
        // keeper, duplicating a member). The convention is over the
        // *effective* members, so when redirection changed the shape,
        // re-freeze once over the post-redirect ids, canonicalised.
        let post: Vec<_> = frozen
            .asset_ids
            .iter()
            .map(|s| parse_asset_id(s))
            .collect::<Result<Vec<_>, _>>()?;
        let mut canonical = post.clone();
        canonical.sort();
        canonical.dedup();
        if canonical != post {
            frozen = self
                .snapshots
                .create(
                    asterism_contract::command::CreateSnapshotCommand {
                        persona_id: persona_id.to_string(),
                        asset_ids: canonical.iter().map(|a| a.to_string()).collect(),
                    },
                    attribution,
                )
                .await?;
        }
        parse_snapshot_id(&frozen.id)
    }

    /// Records a reopen. Legal on an already-open pursuit — the fact
    /// is recorded and standing does not change.
    pub async fn reopen(
        &self,
        command: ReopenPursuitCommand,
        attribution: &AttributionContext,
    ) -> Result<PursuitEventDto, DomainError> {
        let pursuit_id = parse_pursuit_id(&command.pursuit_id)?;
        let pursuit = self
            .pursuits
            .find(&pursuit_id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", &command.pursuit_id))?;
        let event = PursuitEvent::new(
            pursuit.id,
            pursuit.persona_id,
            PursuitEventKind::Reopened,
            None,
            command.note,
            Utc::now(),
            attribution,
        )?;
        self.pursuits.append_event(&event).await?;
        Ok(event_to_dto(&event))
    }

    /// Moves a dispatch round to another pursuit — the recorded repair
    /// verb. The `from` is read from the row here and re-checked
    /// inside the adapter's transaction, so a restamp that raced in
    /// between is refused rather than guessed over; the adapter also
    /// refuses a cross-persona target (one crossing row would make the
    /// pointed persona permanently unpurgeable).
    pub async fn restamp_dispatch(
        &self,
        command: RestampDispatchCommand,
        attribution: &AttributionContext,
    ) -> Result<DispatchDto, DomainError> {
        let dispatch_id = parse_dispatch_id(&command.dispatch_id)?;
        let job = self
            .dispatches
            .find(&dispatch_id)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", &command.dispatch_id))?;
        let to = parse_pursuit_id(&command.to_pursuit_id)?;
        let restamp = PursuitRestamp::new(
            RestampSubject::Dispatch(dispatch_id),
            job.pursuit_id.map(PursuitId::from_correlation),
            to,
            Utc::now(),
            attribution,
        )?;
        self.pursuits.restamp(&restamp).await?;
        // Re-read rather than patching the in-memory job: the row is
        // the fact, and this answer is what the caller files under.
        let moved = self
            .dispatches
            .find(&dispatch_id)
            .await?
            .ok_or_else(|| DomainError::not_found("dispatch", &command.dispatch_id))?;
        Ok(dispatch_to_dto(&moved))
    }

    /// Fetches one pursuit with its derived standing.
    pub async fn get(&self, id: &str) -> Result<PursuitDto, DomainError> {
        let pursuit_id = parse_pursuit_id(id)?;
        let pursuit = self
            .pursuits
            .find(&pursuit_id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", id))?;
        let events = self.pursuits.events_of(&pursuit_id).await?;
        Ok(pursuit_to_dto(&pursuit, standing(&events).slug()))
    }

    /// A pursuit's lifecycle facts, oldest first.
    pub async fn events(&self, id: &str) -> Result<Vec<PursuitEventDto>, DomainError> {
        let pursuit_id = parse_pursuit_id(id)?;
        // Existence surfaced explicitly: an unknown pursuit and a
        // pursuit with no events must not read the same.
        self.pursuits
            .find(&pursuit_id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", id))?;
        let events = self.pursuits.events_of(&pursuit_id).await?;
        Ok(events.iter().map(event_to_dto).collect())
    }

    /// Lists a persona's pursuits, most-recent first. Standing comes
    /// from one latest-event window query over the whole persona
    /// rather than one events read per row — the listing is a surface
    /// that opens constantly, and its cost has to stay flat in the
    /// number of events.
    pub async fn list(&self, persona_id: &str, limit: u32) -> Result<Vec<PursuitDto>, DomainError> {
        let persona = parse_persona_id(persona_id)?;
        let pursuits = self.pursuits.list(&persona, limit).await?;
        let latest: std::collections::HashMap<_, _> = self
            .pursuits
            .latest_event_kinds(&persona)
            .await?
            .into_iter()
            .collect();
        Ok(pursuits
            .iter()
            .map(|pursuit| {
                let standing = crate::domain::forge::pursuit::PursuitStanding::from_latest(
                    latest.get(&pursuit.id).copied(),
                );
                pursuit_to_dto(pursuit, standing.slug())
            })
            .collect())
    }

    /// One pursuit, opened up: the row, its rounds, its returns, its
    /// events — every piece an indexed read (`list_rounds` and the
    /// event log by their pursuit indexes, returns by the V80 lookup
    /// columns), so the view's cost tracks the pursuit's own size and
    /// not the library's.
    pub async fn view(
        &self,
        id: &str,
    ) -> Result<asterism_contract::dto::PursuitViewDto, DomainError> {
        let pursuit_id = parse_pursuit_id(id)?;
        let pursuit = self
            .pursuits
            .find(&pursuit_id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", id))?;
        let events = self.pursuits.events_of(&pursuit_id).await?;
        let rounds = self
            .dispatches
            .list_rounds(&pursuit_id.as_correlation())
            .await?;
        let returns = self.pursuits.returns_of(&pursuit_id).await?;
        let txs = self.pursuits.txs_of(&pursuit_id).await?;
        let culls = self.pursuits.culls_of(&pursuit_id).await?;
        Ok(asterism_contract::dto::PursuitViewDto {
            pursuit: pursuit_to_dto(&pursuit, standing(&events).slug()),
            rounds: rounds.iter().map(dispatch_to_dto).collect(),
            returns: returns.iter().map(|a| a.to_string()).collect(),
            events: events.iter().map(event_to_dto).collect(),
            txs: txs.iter().map(tx_to_dto).collect(),
            culls: culls.iter().map(cull_to_dto).collect(),
        })
    }
}

/// Projects a pursuit row plus its derived standing into the wire
/// shape.
fn pursuit_to_dto(pursuit: &Pursuit, standing: &str) -> PursuitDto {
    PursuitDto {
        id: pursuit.id.to_string(),
        persona_id: pursuit.persona_id.to_string(),
        project_id: pursuit.project_id.map(|p| p.to_string()),
        parent_id: pursuit.parent_id.map(|p| p.to_string()),
        title: pursuit.title.clone(),
        note: pursuit.note.clone(),
        standing: standing.to_string(),
        created_at_ms: pursuit.created_at.timestamp_millis(),
    }
}

/// Projects one lifecycle fact into the wire shape.
fn event_to_dto(event: &PursuitEvent) -> PursuitEventDto {
    PursuitEventDto {
        id: event.id.to_string(),
        pursuit_id: event.pursuit_id.to_string(),
        kind: event.kind.slug().to_string(),
        snapshot_id: event.snapshot_id.map(|s| s.to_string()),
        note: event.note.clone(),
        created_at_ms: event.created_at.timestamp_millis(),
    }
}

/// Projects one ledger gesture into the wire shape — attribution
/// included, because a gesture is a statement somebody made.
fn tx_to_dto(tx: &PursuitTx) -> PursuitTxDto {
    PursuitTxDto {
        id: tx.id.to_string(),
        pursuit_id: tx.pursuit_id.to_string(),
        kind: tx.kind.kind_slug().to_string(),
        origin: tx.kind.origin_slug().map(str::to_string),
        asset_id: tx.asset_id.to_string(),
        note: tx.note.clone(),
        author_kind: tx.author().map(|a| a.kind_slug().to_string()),
        operator_ai: tx.operator_ai().map(|o| o.as_str().to_string()),
        created_at_ms: tx.created_at.timestamp_millis(),
    }
}

/// Projects one cull with its member verdicts into the wire shape.
/// The attribution is the "who decided" of #22's acceptance question,
/// so it rides every read of the act.
fn cull_to_dto((cull, members): &(Cull, Vec<CullMember>)) -> CullDto {
    CullDto {
        id: cull.id.to_string(),
        pursuit_id: cull.pursuit_id.to_string(),
        pursuit_event_id: cull.pursuit_event_id.to_string(),
        candidate_snapshot_id: cull.candidate_snapshot_id.to_string(),
        note: cull.note.clone(),
        author_kind: cull.author().map(|a| a.kind_slug().to_string()),
        operator_ai: cull.operator_ai().map(|o| o.as_str().to_string()),
        created_at_ms: cull.created_at.timestamp_millis(),
        members: members
            .iter()
            .map(|member| CullMemberDto {
                asset_id: member.asset_id.to_string(),
                verdict: member.verdict.slug().to_string(),
                note: member.note.clone(),
            })
            .collect(),
    }
}
