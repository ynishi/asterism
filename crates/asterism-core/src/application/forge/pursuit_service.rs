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
    ClosePursuitCommand, OpenPursuitCommand, ReopenPursuitCommand, RestampDispatchCommand,
};
use asterism_contract::dto::{DispatchDto, PursuitDto, PursuitEventDto};
use chrono::Utc;

use crate::application::mapping::{
    dispatch_to_dto, parse_asset_id, parse_dispatch_id, parse_persona_id, parse_pursuit_id,
    parse_snapshot_id,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::forge::pursuit::{
    Pursuit, PursuitEvent, PursuitEventKind, PursuitRestamp, RestampSubject, standing,
};
use crate::domain::repository::{DispatchRepository, PersonaRepository, PursuitRepository};
use crate::error::DomainError;

/// Pursuit lifecycle use-case service.
pub struct PursuitService {
    pursuits: Arc<dyn PursuitRepository>,
    personas: Arc<dyn PersonaRepository>,
    dispatches: Arc<dyn DispatchRepository>,
    /// The close freeze goes through the snapshot service rather than
    /// the repository so the kept ids get the same existence / persona
    /// / fold-redirect hydration every other freeze gets.
    snapshots: Arc<crate::application::SnapshotService>,
}

impl PursuitService {
    /// Wires the service around its ports.
    pub fn new(
        pursuits: Arc<dyn PursuitRepository>,
        personas: Arc<dyn PersonaRepository>,
        dispatches: Arc<dyn DispatchRepository>,
        snapshots: Arc<crate::application::SnapshotService>,
    ) -> Self {
        Self {
            pursuits,
            personas,
            dispatches,
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
        // A caller-chosen id is the repair path (a returning artefact
        // claimed a pursuit that has no row here); absent, the id is
        // minted like every other open. Either way the create is a
        // create — an id already taken collides at the repository
        // rather than adopting the row that is there.
        let pursuit = match command.pursuit_id.as_deref() {
            None => Pursuit::new(
                persona_id,
                parent_id,
                command.title,
                command.note,
                Utc::now(),
                attribution,
            ),
            Some(wire) => Pursuit::new_at(
                parse_pursuit_id(wire)?,
                persona_id,
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
    /// `satisfied` freezes the kept set at this moment (ascending, see
    /// the module doc) into a snapshot the event references — the
    /// merge-product analogue, immediately usable as the input of a
    /// next dispatch or a child pursuit. An empty kept set records
    /// `None`: "concluded with nothing kept" is a defined state,
    /// because an empty snapshot is domain-rejected. The rejected side
    /// is deliberately not snapshotted — its rows stay live and
    /// restorable.
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
        if kind == PursuitEventKind::ClosedAbandoned && !command.kept_asset_ids.is_empty() {
            return Err(DomainError::Validation(
                "an abandoned close keeps nothing; kept_asset_ids must be empty".into(),
            ));
        }
        let snapshot_id = if command.kept_asset_ids.is_empty() {
            None
        } else {
            // Sort by parsed id, not by string: the convention is
            // "asset id ascending", and the wire strings happen to
            // sort the same for hyphenated UUIDs but the id is the
            // contract. Dedup after sorting — the same asset named
            // twice is one membership.
            let mut kept = command
                .kept_asset_ids
                .iter()
                .map(|s| parse_asset_id(s))
                .collect::<Result<Vec<_>, _>>()?;
            kept.sort();
            kept.dedup();
            let mut frozen = self
                .snapshots
                .create(
                    asterism_contract::command::CreateSnapshotCommand {
                        persona_id: pursuit.persona_id.to_string(),
                        asset_ids: kept.iter().map(|a| a.to_string()).collect(),
                    },
                    attribution,
                )
                .await?;
            // The freeze redirects fold headstones to their keepers in
            // place, preserving position — which can un-sort the set
            // this path just sorted (and two headstones can collapse
            // onto one keeper, duplicating a member). The convention
            // is over the *effective* members, so when redirection
            // changed the shape, re-freeze once over the post-redirect
            // ids, canonicalised. The first freeze's row stays as an
            // unreferenced content-addressed snapshot — harmless, and
            // shared with anything else that ever freezes that exact
            // sequence.
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
                            persona_id: pursuit.persona_id.to_string(),
                            asset_ids: canonical.iter().map(|a| a.to_string()).collect(),
                        },
                        attribution,
                    )
                    .await?;
            }
            Some(parse_snapshot_id(&frozen.id)?)
        };
        let event = PursuitEvent::new(
            pursuit.id,
            pursuit.persona_id,
            kind,
            snapshot_id,
            command.note,
            Utc::now(),
            attribution,
        )?;
        self.pursuits.append_event(&event).await?;
        Ok(event_to_dto(&event))
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
            job.pursuit_id,
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
        let rounds = self.dispatches.list_rounds(&pursuit_id).await?;
        let returns = self.pursuits.returns_of(&pursuit_id).await?;
        Ok(asterism_contract::dto::PursuitViewDto {
            pursuit: pursuit_to_dto(&pursuit, standing(&events).slug()),
            rounds: rounds.iter().map(dispatch_to_dto).collect(),
            returns: returns.iter().map(|a| a.to_string()).collect(),
            events: events.iter().map(event_to_dto).collect(),
        })
    }
}

/// Projects a pursuit row plus its derived standing into the wire
/// shape.
fn pursuit_to_dto(pursuit: &Pursuit, standing: &str) -> PursuitDto {
    PursuitDto {
        id: pursuit.id.to_string(),
        persona_id: pursuit.persona_id.to_string(),
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
