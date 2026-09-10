//! Stamping what a dispatch wrote, before the run reports done.
//!
//! [`OutboundStamping`] is what the runner calls once the files exist
//! and before [`reify`](super::DispatchRunnerService::reify) parks the
//! row in `Done`. An implementation answers whether the run was a
//! release and, when it was, writes the disclosure and the history that
//! chose the set into every copy; [`ReleaseStamping`] is that answer.
//!
//! # Why this is here and not beside `ReleaseService`
//!
//! Only the runner drives it. [`application`](crate::application) says a
//! verb that grows a worker-only counterpart moves across rather than
//! sitting next to its transport-fronted sibling, and this is that
//! counterpart. Recording a release is the other half and stays where a
//! caller can ask for it
//! ([`ReleaseService`](crate::application::ReleaseService)).
//!
//! Two things in the process hold one and they are the same object: the
//! runner's `DispatchRunEnv`, which calls it, and
//! [`SupportServices`](super::SupportServices), which is how a test
//! reaches the one the composition root built. Neither `ServerCtx` nor
//! `AppState` carries either — the support bundle is the field the
//! transport wrappers skip, so no route and no command has an object to
//! call this on.
//!
//! # What a release records when a stamp does not land
//!
//! A row per copy, once a pass runs at all. The alternative was to stop
//! at the first failure, and it made "this release has no file rows"
//! mean a third thing — beside "the run has not finished" and "this
//! build does not stamp" — that nothing could tell from the other two.
//! So a file whose stamp could not be attempted is recorded as a failure
//! with the reason in it, the pass continues, and the first error is
//! returned afterwards so the runner still says so out loud.
//!
//! The two states that write nothing are the two that leave: a dispatch
//! that is not a release, and a build with no disclosure service to
//! stamp with. Neither reaches a file.
//!
//! That is the same reading [`Stamped`] already asks for on its two
//! halves: an outcome that says nothing happened, and does not say why,
//! is the value the type was reshaped to remove.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::application::disclosure_service::DisclosureService;
use crate::domain::disclosure::{Half, Hand, ReleaseAct, ReleaseDisclosure, Stamped};
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::history::ChangePoint;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::pursuit::Pursuit;
use crate::domain::forge::model::value::ChangePointId;
use crate::domain::forge::pursuits::Pursuits;
use crate::domain::release::{FileStamp, Release};
use crate::domain::repository::ReleaseRepository;
use crate::domain::value::{AssetId, DispatchId};
use crate::error::DomainError;

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
/// A port because the runner has no business knowing what a release is:
/// it hands over the copies and the rows they were made from, and the
/// far side decides whether this run was one. A runner that named the
/// implementation would have to hold it in order to finish a dispatch
/// that is not a release.
#[async_trait]
pub trait OutboundStamping: Send + Sync {
    /// Stamps the files one dispatch wrote.
    ///
    /// A dispatch that is not a release is the ordinary case, and the
    /// answer for it is that nothing happens.
    ///
    /// # What `Err` means here
    ///
    /// That something a file was owed did not land, and what. It is not
    /// a verdict on the run: the bytes are written by the time this is
    /// called. What the caller does with it is the caller's, and the
    /// runner's own answer is beside the call it makes.
    async fn stamp(&self, dispatch: &DispatchId, files: &[OutboundFile])
    -> Result<(), DomainError>;
}

/// Stamps a release's copies, and answers for nothing else.
pub struct ReleaseStamping {
    releases: Arc<dyn ReleaseRepository>,
    lines: Arc<dyn Lines>,
    pursuits: Arc<dyn Pursuits>,
    /// The disclosure service, late-bound: a cell because that service
    /// is built after this one in the composition root.
    ///
    /// `asterism_infra::jobs::JobDeps` holds the same handle for a
    /// different reason — there it is optional — and an unbound cell
    /// reads the same way on both sides: there is nothing to stamp
    /// with, which is an answer rather than a fault.
    disclosure: Arc<OnceLock<Arc<DisclosureService>>>,
}

impl ReleaseStamping {
    /// Wires the stamping pass around its ports.
    pub fn new(
        releases: Arc<dyn ReleaseRepository>,
        lines: Arc<dyn Lines>,
        pursuits: Arc<dyn Pursuits>,
        disclosure: Arc<OnceLock<Arc<DisclosureService>>>,
    ) -> Self {
        Self {
            releases,
            lines,
            pursuits,
            disclosure,
        }
    }

    /// The shape of the work a released change point came out of, read
    /// back through the forge's ports.
    async fn shape_for(&self, release: &Release) -> Result<ReleaseDisclosure, DomainError> {
        let held = self
            .lines
            .get(&release.line())
            .await?
            .ok_or_else(|| DomainError::not_found("forge line", release.line()))?;
        let from = point_on(&held, &release.change_point())?.from();
        let work = self
            .pursuits
            .get(&from)
            .await?
            .ok_or_else(|| DomainError::not_found("forge pursuit", from))?;
        shape_of(release, &held, &work)
    }
}

#[async_trait]
impl OutboundStamping for ReleaseStamping {
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
            // Nothing to stamp with. The files are written and carry no
            // mark, and the release says so by holding no stamps rather
            // than by holding failures.
            return Ok(());
        };

        // The shape is the release's rather than any one file's, so a
        // failure to read it is a failure for every copy — recorded on
        // each of them rather than left as an absence.
        let shape = match self.shape_for(&release).await {
            Ok(shape) => shape,
            Err(err) => {
                let stamps = files
                    .iter()
                    .map(|file| refused(file, &err))
                    .collect::<Vec<_>>();
                self.releases.note_files(&release.id(), &stamps).await?;
                return Err(err);
            }
        };

        let carried = dispatch.to_string();
        let mut stamps = Vec::with_capacity(files.len());
        let mut first: Option<DomainError> = None;
        for file in files {
            match disclosure
                .apply_to(&file.asset, &file.path, Some(&carried), Some(&shape))
                .await
            {
                Ok(outcome) => stamps.push(FileStamp {
                    asset: file.asset,
                    path: file.path.display().to_string(),
                    outcome,
                }),
                Err(err) => {
                    stamps.push(refused(file, &err));
                    if first.is_none() {
                        first = Some(err);
                    }
                }
            }
        }
        self.releases.note_files(&release.id(), &stamps).await?;
        match first {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

/// One file the disclosure could not be built or written for.
///
/// Both halves [`Failed`](Half::Failed), carrying the reason. Neither
/// was attempted, and [`Skipped`](crate::domain::disclosure::Skipped) is
/// the wrong word for it — every variant of that is a state the
/// application is meant to be able to be in, and a record that cannot be
/// assembled is not one of them.
fn refused(file: &OutboundFile, why: &DomainError) -> FileStamp {
    let cause = why.to_string();
    FileStamp {
        asset: file.asset,
        path: file.path.display().to_string(),
        outcome: Stamped::new(Half::Failed(cause.clone()), Half::Failed(cause)),
    }
}

/// The change point, if this line's history has it.
pub(crate) fn point_on<'a>(
    line: &'a Line,
    change_point: &ChangePointId,
) -> Result<&'a ChangePoint, DomainError> {
    line.history()
        .changes()
        .iter()
        .find(|point| point.id() == *change_point)
        .ok_or_else(|| DomainError::not_found("forge change point", change_point))
}

/// What a released file says about the work that reached it.
///
/// A function of three values rather than a method, so that what it
/// decides can be asked without a store: the pursuit id, the intent
/// title, how many rounds there were, and whose hand each of the two
/// acts was.
///
/// The close's act is read off the pursuit's own ending rather than off
/// the change point. The two are stamped together, and they are still
/// two records — reading the one the sentence is about is what keeps
/// them from being conflated the day they stop agreeing.
///
/// # Refusals
///
/// - [`NotFound`](DomainError::NotFound) — `line` does not have the node
///   the release names.
/// - [`Validation`](DomainError::Validation) — `work` is not the work
///   that change point came out of, or it has no ending. A change point
///   exists because a pursuit was satisfied, so both are states a store
///   would have to be wrong to produce.
fn shape_of(
    release: &Release,
    line: &Line,
    work: &Pursuit,
) -> Result<ReleaseDisclosure, DomainError> {
    let from = point_on(line, &release.change_point())?.from();
    if work.id() != from {
        return Err(DomainError::Validation(format!(
            "a release of change point {} names the work {from}, and the work read \
             back is {}",
            release.change_point(),
            work.id()
        )));
    }
    let closed = work
        .close()
        .map(|close| act_of(close.act()))
        .ok_or_else(|| {
            DomainError::Validation(format!(
                "the work a released change point came out of has no ending: {from}"
            ))
        })?;
    Ok(ReleaseDisclosure::new(
        act_of(release.act()),
        from.to_string(),
        work.opening()
            .intent()
            .title
            .as_ref()
            .map(|title| title.as_str().to_string()),
        work.rounds().len() as u32,
        closed,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::closing::close;
    use crate::domain::forge::model::op::Op;
    use crate::domain::forge::model::pursuit::{Intent, Outcome, Round};
    use crate::domain::forge::model::value::{ActorId, Content, Name, StrategyId};
    use crate::domain::value::{DispatchId, SnapshotId};
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn at(minute: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 11, 12, minute, 0).unwrap()
    }

    fn by_a_person(minute: u32) -> Act {
        Act::new(at(minute), Actor::User(ActorId::new()))
    }

    fn by_a_rule(minute: u32) -> Act {
        Act::new(at(minute), Actor::System(ActorId::new()))
    }

    /// A line with one change point on it, and the work that landed it.
    fn a_landed_line(title: Option<&str>, rounds: usize, closed_by: Act) -> (Line, Pursuit) {
        let mut line = Line::open(
            Name::new(Line::ROOT).unwrap(),
            StrategyId::new("by-hand").unwrap(),
            by_a_person(0),
        );
        let intent = Intent {
            title: title.map(|text| Name::new(text).unwrap()),
            note: None,
        };
        let mut work = Pursuit::open(line.id(), None, line.head(), intent, by_a_person(1));
        for nth in 0..rounds {
            let op = Op::add(
                Content::from_uuid(Uuid::now_v7()),
                Name::new(format!("cut-{nth}")).unwrap(),
            );
            let round = Round::new(work.head(), vec![op], None, by_a_person(2)).unwrap();
            work.push(round).unwrap();
        }
        let closing = close(&line, &work, Outcome::Satisfied, None, closed_by).unwrap();
        closing.apply(&mut line, &mut work).unwrap();
        (line, work)
    }

    fn a_release(line: &Line, act: Act) -> Release {
        Release::new(
            line.id(),
            line.history().changes().last().unwrap().id(),
            SnapshotId::new(),
            DispatchId::new(),
            act,
        )
    }

    #[test]
    fn the_shape_carries_the_pursuit_the_title_the_rounds_and_both_acts() {
        let (line, work) = a_landed_line(Some("the key visual"), 2, by_a_rule(3));
        let release = a_release(&line, by_a_person(30));

        let shape = shape_of(&release, &line, &work).unwrap();

        assert_eq!(shape.pursuit_id, work.id().to_string());
        assert_eq!(shape.intent_title.as_deref(), Some("the key visual"));
        assert_eq!(shape.rounds, 2);
        assert_eq!(shape.released.at, at(30));
        assert_eq!(shape.released.by, Hand::Person);
        assert_eq!(shape.closed.at, at(3));
        assert_eq!(
            shape.closed.by,
            Hand::Rule,
            "a rule wrote this one is a different answer from a person chose it"
        );
    }

    /// Work nobody named contributes no title, rather than an empty one.
    #[test]
    fn an_unnamed_pursuit_leaves_the_title_absent() {
        let (line, work) = a_landed_line(None, 1, by_a_person(3));

        let shape = shape_of(&a_release(&line, by_a_person(30)), &line, &work).unwrap();

        assert_eq!(shape.intent_title, None);
        assert_eq!(shape.rounds, 1);
    }

    #[test]
    fn a_release_of_a_node_this_line_does_not_have_is_refused() {
        let (line, work) = a_landed_line(None, 1, by_a_person(3));
        let elsewhere = Release::new(
            line.id(),
            ChangePointId::new(),
            SnapshotId::new(),
            DispatchId::new(),
            by_a_person(30),
        );

        let refused = shape_of(&elsewhere, &line, &work);

        assert!(matches!(refused, Err(DomainError::NotFound { .. })));
    }

    /// The work read back has to be the work the change point names, or
    /// the file would state the shape of somebody else's deliberation.
    #[test]
    fn work_the_change_point_did_not_come_out_of_is_refused() {
        let (line, _) = a_landed_line(None, 1, by_a_person(3));
        let (_, other) = a_landed_line(None, 1, by_a_person(3));

        let refused = shape_of(&a_release(&line, by_a_person(30)), &line, &other);

        assert!(matches!(refused, Err(DomainError::Validation(_))));
    }

    /// A file nothing could be assembled for is recorded as a failure
    /// with the reason in it, on both halves — never as an absence.
    #[test]
    fn a_refused_file_records_the_reason_on_both_halves() {
        let file = OutboundFile {
            asset: AssetId::new(),
            path: PathBuf::from("/out/key-visual.png"),
        };

        let stamp = refused(&file, &DomainError::blocked("the fingerprint has not run"));

        assert_eq!(stamp.asset, file.asset);
        assert_eq!(stamp.path, "/out/key-visual.png");
        assert!(!stamp.outcome.discloses());
        assert_eq!(stamp.outcome.failures().len(), 2);
        assert!(stamp.outcome.failures()[0].contains("the fingerprint has not run"));
    }
}
