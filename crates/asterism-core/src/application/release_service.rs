//! `ReleaseService` — writing out what a change point carries.
//!
//! One verb: [`release`](ReleaseService::release) freezes the change
//! point's folded state, starts a `file` dispatch in `copy` mode over
//! it, and records that it happened.
//!
//! Stamping the copies is the other half and is not here. Only the
//! runner drives it, so it sits in
//! [`application_support::outbound_stamp`](crate::application_support::outbound_stamp)
//! where no transport can reach it — the placement rule this module's
//! own doc states.
//!
//! # The freeze is driven from here, not from inside the forge
//!
//! [`boundary`](crate::domain::forge::boundary) says which questions the
//! forge asks downward, and [`Release`] says why the record that names a
//! snapshot and a dispatch is not one of them. The consequence for this
//! service is the shape of [`release`](ReleaseService::release): it
//! reads the line, folds the chain and calls the services that freeze
//! and dispatch, the way `asterism-teams-client::publish` hands a line's
//! state to a receiver the forge does not control.
//!
//! Nothing here writes a forge word onto a core row, and nothing writes
//! a core id onto a forge one.
//!
//! # A release with nothing live is refused
//!
//! A change point whose folded state has no live entry has nothing to
//! write out. Recording one would produce a release naming a snapshot
//! that cannot exist — [`Snapshot::new`](crate::domain::snapshot::Snapshot::new)
//! rejects an empty membership — so the choice is between refusing and
//! inventing a record of an export that never happened. It is refused,
//! as [`Blocked`](crate::error::ConflictKind::Blocked): put something on
//! the line and the same request works.

use std::sync::Arc;

use crate::application::dispatch_service::DispatchService;
use crate::application::snapshot_service::SnapshotService;
use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::actors::Actors;
use crate::domain::forge::clock::Clock;
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::history::ChangePoint;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::table::states;
use crate::domain::forge::model::value::{ChangePointId, LineId};
use crate::domain::release::Release;
use crate::domain::repository::ReleaseRepository;
use crate::domain::value::{AssetId, PersonaId, ReleaseId};
use crate::error::DomainError;

/// The exporter a release writes through.
///
/// Named here rather than taken from the caller. A release is "the set
/// is written out as files"; which exporter does that is this service's
/// answer, and letting a caller choose would make "release" mean
/// whatever the caller passed.
const FILE_EXPORTER: &str = "file";

/// The action that exporter takes.
const WRITE_ACTION: &str = "write";

/// Recording a release.
pub struct ReleaseService {
    lines: Arc<dyn Lines>,
    releases: Arc<dyn ReleaseRepository>,
    snapshots: Arc<SnapshotService>,
    dispatches: Arc<DispatchService>,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl ReleaseService {
    /// Wires the service around its ports.
    pub fn new(
        lines: Arc<dyn Lines>,
        releases: Arc<dyn ReleaseRepository>,
        snapshots: Arc<SnapshotService>,
        dispatches: Arc<DispatchService>,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            lines,
            releases,
            snapshots,
            dispatches,
            actors,
            clock,
        }
    }

    /// Releases a change point into `output_dir`.
    ///
    /// The state the change point left the line in is frozen as a
    /// snapshot, a `file` dispatch in `copy` mode is started over it,
    /// and the release records both ids beside its own act. The chain is
    /// untouched: nothing here calls [`Line::record`], and there is no
    /// path from this service to one.
    ///
    /// `output_dir` is a path and nothing else. What an agency's form
    /// asks for is the contributor's step, and a destination vocabulary
    /// here would encode somebody else's policy on somebody else's
    /// schedule.
    ///
    /// # Refusals
    ///
    /// - [`NotFound`](DomainError::NotFound) — no such line, or the
    ///   change point is not on it. A node of another line's history is
    ///   the same answer as one that does not exist: this line does not
    ///   have it.
    /// - [`Conflict`](DomainError::Conflict), [`Blocked`](crate::error::ConflictKind::Blocked)
    ///   — the folded state has no live entry, so there is nothing to
    ///   write out.
    /// - Whatever freezing and dispatching refuse: an asset that is not
    ///   the persona's, a snapshot that cannot be minted.
    pub async fn release(
        &self,
        line: &LineId,
        change_point: &ChangePointId,
        persona: &PersonaId,
        output_dir: &str,
        by: &AttributionContext,
    ) -> Result<Release, DomainError> {
        let held = self
            .lines
            .get(line)
            .await?
            .ok_or_else(|| DomainError::not_found("forge line", line))?;
        let members = released_members(&held, change_point)?;

        let snapshot = self
            .snapshots
            .create(
                asterism_contract::command::CreateSnapshotCommand {
                    persona_id: persona.to_string(),
                    asset_ids: members.iter().map(AssetId::to_string).collect(),
                },
                by,
            )
            .await?;

        let dispatch = self
            .dispatches
            .create(
                asterism_contract::command::CreateDispatchCommand {
                    snapshot_id: snapshot.id.clone(),
                    exporter_slug: FILE_EXPORTER.into(),
                    action: WRITE_ACTION.into(),
                    params_json: serde_json::json!({
                        "output_dir": output_dir,
                        "mode": "copy",
                    })
                    .to_string(),
                    operator_ai: by.operator_ai().map(|op| op.as_str().to_string()),
                },
                by,
            )
            .await?;

        let release = Release::new(
            held.id(),
            *change_point,
            crate::application::mapping::parse_snapshot_id(&snapshot.id)?,
            crate::application::mapping::parse_dispatch_id(&dispatch.id)?,
            self.act(by).await?,
        );
        self.releases.record(&release).await?;
        Ok(release)
    }

    /// Reads one release back, its file stamps included.
    pub async fn get(&self, id: &ReleaseId) -> Result<Release, DomainError> {
        self.releases
            .find(id)
            .await?
            .ok_or_else(|| DomainError::not_found("release", id))
    }

    /// Every release of one change point on one line, most recent
    /// first.
    ///
    /// The line is read and the node checked against it rather than
    /// taken as decoration. A caller that named a change point of
    /// another line and got the answer anyway would be told a line
    /// holds something it does not — the same refusal
    /// [`release`](Self::release) makes at the same address, which is
    /// what makes reading and writing there answer for the same thing.
    pub async fn of_change_point(
        &self,
        line: &LineId,
        change_point: &ChangePointId,
    ) -> Result<Vec<Release>, DomainError> {
        let held = self
            .lines
            .get(line)
            .await?
            .ok_or_else(|| DomainError::not_found("forge line", line))?;
        crate::application_support::outbound_stamp::point_on(&held, change_point)?;
        self.releases.of_change_point(change_point).await
    }

    /// Stamps an act: now, by whoever this write is from.
    async fn act(&self, by: &AttributionContext) -> Result<Act, DomainError> {
        Ok(Act::new(
            self.clock.now(),
            Actor::User(self.actors.resolve(by).await?),
        ))
    }
}

/// What the line carried when `change_point` landed, as the contents of
/// its live entries.
///
/// The chain is folded up to and including that node, which is what
/// makes a release of an older change point mean what it says: the state
/// then, not the state now.
///
/// The genesis is not among the nodes this looks at, and that is the
/// answer for it rather than an oversight — a line's genesis carries no
/// table, so the state at it is empty and releasing it is the refusal
/// below.
fn released_members(
    line: &Line,
    change_point: &ChangePointId,
) -> Result<Vec<AssetId>, DomainError> {
    let chain = line.history().changes();
    let upto = chain
        .iter()
        .position(|point| point.id() == *change_point)
        .ok_or_else(|| DomainError::not_found("forge change point", change_point))?;
    let members: Vec<AssetId> = states(chain[..=upto].iter().map(ChangePoint::table))
        .into_values()
        .filter(|state| state.alive)
        .filter_map(|state| state.content)
        .map(|content| AssetId::from_uuid(*content.as_uuid()))
        .collect();
    if members.is_empty() {
        return Err(DomainError::blocked(format!(
            "change point {change_point} carries no live entry, so a release of it \
             would name a freeze with no members; put something on the line first"
        )));
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::closing::close;
    use crate::domain::forge::model::op::Op;
    use crate::domain::forge::model::pursuit::{Intent, Outcome, Pursuit};
    use crate::domain::forge::model::value::{ActorId, Content, Name, StrategyId};
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn act(minute: u32) -> Act {
        Act::new(
            Utc.with_ymd_and_hms(2026, 9, 11, 12, minute, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    /// A line with one change point on it, and what that change point
    /// put there.
    fn a_line_carrying(rows: Vec<(Content, &str)>) -> (Line, ChangePointId) {
        let mut line = Line::open(
            Name::new(Line::ROOT).unwrap(),
            StrategyId::new("by-hand").unwrap(),
            act(0),
        );
        let mut work = Pursuit::open(line.id(), None, line.head(), Intent::default(), act(1));
        let ops: Vec<Op> = rows
            .iter()
            .map(|(content, called)| Op::add(*content, Name::new(*called).unwrap()))
            .collect();
        let round =
            crate::domain::forge::model::pursuit::Round::new(work.head(), ops, None, act(2))
                .unwrap();
        work.push(round).unwrap();
        let closing = close(&line, &work, Outcome::Satisfied, None, act(3)).unwrap();
        let id = closing.point().expect("a satisfied close lands one").id();
        closing.apply(&mut line, &mut work).unwrap();
        (line, id)
    }

    #[test]
    fn the_members_are_the_contents_of_the_live_entries() {
        let first = Content::from_uuid(Uuid::now_v7());
        let second = Content::from_uuid(Uuid::now_v7());
        let (line, point) = a_line_carrying(vec![(first, "key visual"), (second, "alternate")]);

        let members = released_members(&line, &point).unwrap();

        assert_eq!(members.len(), 2);
        for content in [first, second] {
            assert!(members.contains(&AssetId::from_uuid(*content.as_uuid())));
        }
    }

    #[test]
    fn a_node_of_another_line_is_not_on_this_one() {
        let (line, _) = a_line_carrying(vec![(Content::from_uuid(Uuid::now_v7()), "key visual")]);

        let refused = released_members(&line, &ChangePointId::new());

        assert!(matches!(refused, Err(DomainError::NotFound { .. })));
    }

    /// A change point that took the only entry off the line has nothing
    /// to write out, and saying so is the honest answer — see the module
    /// docs.
    #[test]
    fn a_change_point_carrying_nothing_live_is_refused() {
        let content = Content::from_uuid(Uuid::now_v7());
        let (mut line, _) = a_line_carrying(vec![(content, "key visual")]);
        let entry = *line.states().keys().next().unwrap();
        let mut work = Pursuit::open(line.id(), None, line.head(), Intent::default(), act(4));
        let round = crate::domain::forge::model::pursuit::Round::new(
            work.head(),
            vec![Op::remove(entry)],
            None,
            act(5),
        )
        .unwrap();
        work.push(round).unwrap();
        let closing = close(&line, &work, Outcome::Satisfied, None, act(6)).unwrap();
        let id = closing.point().expect("a satisfied close lands one").id();
        closing.apply(&mut line, &mut work).unwrap();

        let refused = released_members(&line, &id);

        assert!(matches!(
            refused,
            Err(DomainError::Conflict {
                kind: crate::error::ConflictKind::Blocked,
                ..
            })
        ));
    }
}
