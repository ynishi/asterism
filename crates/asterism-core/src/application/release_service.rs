//! `ReleaseService` — writing out what a change point carries, and
//! stamping what leaves.
//!
//! One verb and one hook:
//!
//! - [`release`](ReleaseService::release) — freeze the change point's
//!   folded state, start a `file` dispatch in `copy` mode over it, and
//!   record that it happened.
//! - [`OutboundStamping`] — what the run calls back when it has written
//!   the files, so that each copy carries the disclosure and the history
//!   that chose it before the dispatch reports done.
//!
//! # The freeze is driven from here, not from inside the forge
//!
//! [`boundary::Store`](crate::domain::forge::boundary::store::Store)
//! asks the layer below exactly one question, and its module doc says
//! the rest — freezing a set among them — waits for the work that needs
//! it. This is that work, and the answer is that the store does not grow
//! a `freeze` method.
//!
//! Two reasons, and the second is the one that decides it.
//!
//! **The forge may not name what a freeze produces.** A `SnapshotId` and
//! a `DispatchId` are core words, `tests/forge_boundary.rs` holds the
//! list of words a contract across that boundary may be written in, and
//! `freeze(change_point) -> SnapshotId` would put two more on it. That
//! list is a statement about what the forge would have to carry when it
//! is lifted into a crate of its own, and paying two entries of it for a
//! record that does not have to live inside the forge is the reversal
//! #254 asked to be argued before it was written.
//!
//! **The same walk already happens outward, from outside.**
//! `asterism-teams-client::publish` hands a line's current state to a
//! receiver the forge does not control, and it does that by reading the
//! line and calling the far side — not by asking the forge to hand
//! anything down. A release to a filesystem is that walk with a
//! different transport, so it is driven the same way: read the line,
//! fold the chain, and call the services that freeze and dispatch.
//!
//! What the forge keeps is what it kept before: it decides, and the raw
//! layer carries. Nothing here writes a forge word onto a core row, and
//! nothing writes a core id onto a forge one — the release is a row of
//! its own that names both ([`Release`]).
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
//!
//! # Stamping is the same writer, pointed at the copy
//!
//! [`DisclosureService::apply_to`] is what the `disclosure_stamp` job
//! already calls on the library's own artefacts. A release calls it on
//! the *copies* the exporter wrote, and never on the library's own
//! file — the record is derived from stored rows either way, so what
//! differs is only which path is handed in.
//!
//! The manifest half being [`Skipped`](crate::domain::disclosure::Skipped)
//! on a build with no certificate is a supported state and not a
//! failure, so it is recorded per file and reported rather than raised.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::application::disclosure_service::DisclosureService;
use crate::application::dispatch_service::DispatchService;
use crate::application::snapshot_service::SnapshotService;
use crate::domain::attribution::AttributionContext;
use crate::domain::disclosure::{Hand, ReleaseAct, ReleaseDisclosure};
use crate::domain::forge::boundary::actors::Actors;
use crate::domain::forge::clock::Clock;
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::history::ChangePoint;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::table::states;
use crate::domain::forge::model::value::{ChangePointId, LineId};
use crate::domain::forge::pursuits::Pursuits;
use crate::domain::release::{FileStamp, Release};
use crate::domain::repository::ReleaseRepository;
use crate::domain::value::{AssetId, DispatchId, PersonaId, ReleaseId};
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

/// One file a run wrote, and the library row it is a copy of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundFile {
    /// The row the copy was made from.
    pub asset: AssetId,
    /// Where the copy landed.
    pub path: PathBuf,
}

/// What a dispatch calls when it has written its files and has not yet
/// reported done.
///
/// A port, declared beside the service that owns the concept for the
/// same reason
/// [`DisclosureWriter`](crate::application::disclosure_service::DisclosureWriter)
/// is: the caller is the runner in `asterism-infra`, and a runner that
/// named this service would have to know what a release is in order to
/// finish a dispatch that is not one.
///
/// # Why nothing here fails a dispatch
///
/// The bytes are written by the time this is called. A stamp that does
/// not land leaves a file that exists and is not marked, and the mark
/// can be made again from stored rows; failing the run instead would
/// discard an export that produced exactly what it was asked for. So the
/// error channel carries what went wrong for the caller to log, and the
/// caller logs it.
#[async_trait]
pub trait OutboundStamping: Send + Sync {
    /// Stamps the files one dispatch wrote.
    ///
    /// A dispatch that is not a release is the ordinary case, and the
    /// answer for it is that nothing happens.
    async fn stamp(&self, dispatch: &DispatchId, files: &[OutboundFile])
    -> Result<(), DomainError>;
}

/// Releasing a change point, and stamping what leaves.
pub struct ReleaseService {
    lines: Arc<dyn Lines>,
    pursuits: Arc<dyn Pursuits>,
    releases: Arc<dyn ReleaseRepository>,
    snapshots: Arc<SnapshotService>,
    dispatches: Arc<DispatchService>,
    /// The disclosure service, late-bound.
    ///
    /// A cell rather than an `Arc` because that service is built after
    /// this one in the composition root and may not be built at all —
    /// the same handle the job runtime holds, for the same reason. An
    /// unbound cell means this build does not stamp, which is a
    /// configuration rather than a fault.
    disclosure: Arc<OnceLock<Arc<DisclosureService>>>,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl ReleaseService {
    /// Wires the service around its ports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lines: Arc<dyn Lines>,
        pursuits: Arc<dyn Pursuits>,
        releases: Arc<dyn ReleaseRepository>,
        snapshots: Arc<SnapshotService>,
        dispatches: Arc<DispatchService>,
        disclosure: Arc<OnceLock<Arc<DisclosureService>>>,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            lines,
            pursuits,
            releases,
            snapshots,
            dispatches,
            disclosure,
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

    /// Every release of one change point, most recent first.
    pub async fn of_change_point(
        &self,
        change_point: &ChangePointId,
    ) -> Result<Vec<Release>, DomainError> {
        self.releases.of_change_point(change_point).await
    }

    /// The shape of the work a released change point came out of, as the
    /// files will state it.
    ///
    /// The translation from the forge's vocabulary into the
    /// disclosure's happens here and nowhere else — see
    /// [`ReleaseDisclosure`] for why the renderer is not given a
    /// `Pursuit` to walk.
    async fn shape_of(&self, release: &Release) -> Result<ReleaseDisclosure, DomainError> {
        let held = self
            .lines
            .get(&release.line())
            .await?
            .ok_or_else(|| DomainError::not_found("forge line", release.line()))?;
        let point = point_on(&held, &release.change_point())?;
        let work = self
            .pursuits
            .get(&point.from())
            .await?
            .ok_or_else(|| DomainError::not_found("forge pursuit", point.from()))?;
        // A change point exists because a pursuit was satisfied, so the
        // close is there. `by()` names it, and reading the act off the
        // pursuit's own ending rather than off the change point is what
        // keeps the two logs' answers from being conflated: they are
        // stamped together today and are still two records.
        let closed = work
            .close()
            .map(|close| act_of(close.act()))
            .ok_or_else(|| {
                DomainError::Validation(format!(
                    "the work a released change point came out of has no ending: {}",
                    point.from()
                ))
            })?;
        Ok(ReleaseDisclosure::new(
            act_of(release.act()),
            point.from().to_string(),
            work.opening()
                .intent()
                .title
                .as_ref()
                .map(|title| title.as_str().to_string()),
            work.rounds().len() as u32,
            closed,
        ))
    }

    /// Stamps an act: now, by whoever this write is from.
    async fn act(&self, by: &AttributionContext) -> Result<Act, DomainError> {
        Ok(Act::new(
            self.clock.now(),
            Actor::User(self.actors.resolve(by).await?),
        ))
    }
}

#[async_trait]
impl OutboundStamping for ReleaseService {
    async fn stamp(
        &self,
        dispatch: &DispatchId,
        files: &[OutboundFile],
    ) -> Result<(), DomainError> {
        let Some(release) = self.releases.by_dispatch(dispatch).await? else {
            // Most dispatches are not releases. Nothing to say.
            return Ok(());
        };
        let Some(disclosure) = self.disclosure.get() else {
            // No writer configured. The files are written and carry no
            // mark, which is the state a build that has not asked for
            // stamping is in — and the release says so by holding no
            // stamps rather than by holding failures.
            return Ok(());
        };
        let shape = self.shape_of(&release).await?;
        let carried = dispatch.to_string();
        let mut stamps = Vec::with_capacity(files.len());
        for file in files {
            let outcome = disclosure
                .apply_to(&file.asset, &file.path, Some(&carried), Some(&shape))
                .await?;
            stamps.push(FileStamp {
                asset: file.asset,
                path: file.path.display().to_string(),
                outcome,
            });
        }
        self.releases.note_files(&release.id(), &stamps).await
    }
}

/// One act, in the words a file may state it in.
fn act_of(act: &Act) -> ReleaseAct {
    ReleaseAct::new(
        act.at(),
        if act.by().is_system() {
            Hand::Rule
        } else {
            Hand::Person
        },
    )
}

/// The change point, if this line's history has it.
fn point_on<'a>(
    line: &'a Line,
    change_point: &ChangePointId,
) -> Result<&'a ChangePoint, DomainError> {
    line.history()
        .changes()
        .iter()
        .find(|point| point.id() == *change_point)
        .ok_or_else(|| DomainError::not_found("forge change point", change_point))
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

    #[test]
    fn a_rule_and_a_person_read_differently_in_the_assertion() {
        assert_eq!(
            act_of(&Act::new(act(0).at(), Actor::System(ActorId::new()))).by,
            Hand::Rule
        );
        assert_eq!(act_of(&act(0)).by, Hand::Person);
    }
}
