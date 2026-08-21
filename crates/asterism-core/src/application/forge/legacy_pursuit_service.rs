//! `LegacyPursuitService` — the lifecycle verbs of the minted unit of work
//! (#29): open, close, reopen, and the reads that derive standing.
//!
//! **This is on its way out, and the name says so.** The model it
//! serves — `PursuitEvent`, `PursuitTx`, the standing derived from
//! them — is superseded by [`model`](crate::domain::forge::model),
//! where a line's history is a chain of change points and work is a
//! log of passes. Nothing here is being extended.
//!
//! It is still the one wired to transport, so it stays until the
//! service that replaces it can answer the same surface. That service
//! is `PursuitService`, which took this one's name: the code that is
//! leaving carries the awkward one, so that a reader never has to work
//! out which of two same-named services is current.
//!
//! When the replacement is wired, this file and the model under it go
//! in one deletion. Until then, a change here is a change to something
//! scheduled for removal, and is worth questioning on those grounds
//! alone.
//!
//! The open creates one (naming intent up front), the one-way
//! lifecycle facts (close / reopen) are recorded rather than written
//! as a status, and the ledger takes the membership gestures.
//! Transport routes land in the next slice of #29; until then the
//! service fronts the e2e surface through `CoreCtx`, the same way
//! every service is reachable there.
//!
//! The close writes one row and materialises nothing: it records that
//! a line of work ended, and what the line of work was on stays
//! derivable from the ledger it leaves alone.

use std::sync::Arc;

use asterism_contract::command::{
    ClosePursuitCommand, OpenPursuitCommand, RecordPursuitTxCommand, ReopenPursuitCommand,
};
use asterism_contract::dto::{PursuitDto, PursuitEventDto, PursuitTxDto};
use chrono::Utc;

use crate::application::forge::mapping::{parse_project_id, parse_pursuit_id};
use crate::application::mapping::{parse_asset_id, parse_persona_id};
use crate::domain::attribution::AttributionContext;
use crate::domain::forge::pursuit::{Pursuit, PursuitEvent, PursuitEventKind, standing};
use crate::domain::forge::repository::{ProjectRepository, PursuitRepository};
use crate::domain::forge::tx::{PursuitTx, PursuitTxKind, TxOrigin, ledger};
use crate::domain::repository::{AssetRepository, PersonaRepository};
use crate::error::DomainError;

/// Pursuit lifecycle use-case service.
pub struct LegacyPursuitService {
    pursuits: Arc<dyn PursuitRepository>,
    /// Filing is checked against the project it names — existence, and
    /// that it belongs to the same persona, which no foreign key can
    /// say.
    projects: Arc<dyn ProjectRepository>,
    personas: Arc<dyn PersonaRepository>,
    /// The ledger's `in` names an asset row; the existence and persona
    /// checks read it here.
    assets: Arc<dyn AssetRepository>,
}

impl LegacyPursuitService {
    /// Wires the service around its ports.
    pub fn new(
        pursuits: Arc<dyn PursuitRepository>,
        projects: Arc<dyn ProjectRepository>,
        personas: Arc<dyn PersonaRepository>,
        assets: Arc<dyn AssetRepository>,
    ) -> Self {
        Self {
            pursuits,
            projects,
            personas,
            assets,
        }
    }

    /// Opens a pursuit — the only way one comes into being. A caller
    /// that wants work filed under it opens it first and names the id
    /// on each gesture; a pursuit that never receives one is an honest
    /// record.
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
    /// Both outcomes record the ending and apply nothing else. The
    /// event's `snapshot_id` is always `None`: the close no longer
    /// selects among the pursuit's members, so there is no set for it
    /// to freeze, and `satisfied` differs from `abandoned` in what it
    /// says about how the line of work ended rather than in what it
    /// writes. What the pursuit worked on stays where it already is —
    /// derivable from the ledger, which the close leaves untouched.
    ///
    /// `snapshot_id` is still read on the way out, because rows
    /// written before this change carry one; nothing writes it now.
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
        let event = PursuitEvent::new(
            pursuit.id,
            pursuit.persona_id,
            kind,
            None,
            command.note,
            Utc::now(),
            attribution,
        )?;
        self.pursuits.append_close(&event).await?;
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

    /// One pursuit, opened up: the row, its events, its ledger — every
    /// piece an indexed read (the event log and the ledger by their
    /// pursuit indexes), so the view's cost tracks the pursuit's own
    /// size and not the library's.
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
        let txs = self.pursuits.txs_of(&pursuit_id).await?;
        Ok(asterism_contract::dto::PursuitViewDto {
            pursuit: pursuit_to_dto(&pursuit, standing(&events).slug()),
            events: events.iter().map(event_to_dto).collect(),
            txs: txs.iter().map(tx_to_dto).collect(),
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
