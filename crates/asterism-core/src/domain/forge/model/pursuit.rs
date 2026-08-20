//! One line of work: what it is trying to do, and every pass at it.
//!
//! ```text
//!   Pursuit ── of : →Line ── parent? : →Pursuit
//!    └ WorkLog
//!        Open ──▶ Round ──▶ Round ──▶ Close?
//!          │        │  └ ops / taken in
//!          │        └ parent
//!          └ base : the change point this was cut from
//! ```
//!
//! The shape mirrors a line's history deliberately: a root that is not
//! like the others, a chain that only grows, and everything the thing
//! currently says derived by folding it. What differs is what the
//! nodes carry — a line's node carries a table that has landed, a
//! work log's carries operations that have not.
//!
//! # A pass is the unit, not the pursuit
//!
//! [`Round`] is where work happens. A pursuit is the container that
//! says what the passes are for, and it holds nothing that a round
//! could hold instead.
//!
//! A round with no operations is ordinary — it is a round that took a
//! change point in, which is how work says "I have seen what landed".
//! What is refused is a round that says neither: a node that carries
//! no operations and takes nothing in records that nothing happened,
//! and nothing happening is not an event.
//!
//! # The base does not move
//!
//! [`Open`] names the change point the work was cut from, and that is
//! the only thing the base ever is. Later rounds may take newer change
//! points in, and doing so does **not** move it.
//!
//! The distinction is what makes a collision visible. The base says
//! where this work started from; a round's `taken_in` says what it has
//! since seen. If taking something in moved the base, then work could
//! walk past a landing it never looked at, and the step that compares
//! this work against the line would find nothing to compare.
//!
//! # Closing is terminal
//!
//! [`Close`] carries which kind of ending it was, and nothing can be
//! pushed after it. Satisfied means the intent was met — the act that
//! turns that into a change point spans both logs and is not here.
//! Abandoned means it was not, and that is a record rather than a
//! deletion: the pass that was dropped stays readable, which is the
//! only way "we tried this and stopped" survives.
//!
//! Reopening is not a verb. Picking work back up is a new pursuit
//! with the same parent, and that reads as what happened.
//!
//! # Parent is where work belongs
//!
//! [`Pursuit::parent`] says which larger piece of work this is part
//! of. It is fixed when the pursuit is opened, because a link that can
//! be corrected is a link that has to be checked for cycles; one that
//! can only point at something already open cannot form one.
//!
//! Nothing else about the relationship is stored. Which pursuits are
//! under a parent, and what they landed, is a question answered by
//! looking — and a stored answer would be a second copy of it.

use crate::domain::forge::model::act::{Act, Meta};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::op::{Op, Rows, fold};
use crate::domain::forge::model::value::{ChangePointId, LineId, Name, NodeId, PursuitId};

/// Why a pursuit was opened, in the words of whoever opened it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Intent {
    /// A short name for the work, if it was given one.
    pub title: Option<Name>,
    /// Anything else worth saying about why.
    pub note: Option<String>,
}

/// How a pursuit ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// What the work set out to do is done.
    Satisfied,
    /// It is not, and nobody is going to.
    Abandoned,
}

/// The node work begins at.
///
/// A separate type from [`Round`] for the reason a line's genesis is
/// separate from a change point: it has no parent and carries no
/// operations, and modelling that as a round with everything optional
/// makes a shape that has to be kept consistent by agreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    id: NodeId,
    base: ChangePointId,
    intent: Intent,
    act: Act,
}

impl Open {
    /// Cuts work from a change point.
    pub fn new(base: ChangePointId, intent: Intent, act: Act) -> Self {
        Self {
            id: NodeId::new(),
            base,
            intent,
            act,
        }
    }

    /// Which node.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The change point this work was cut from.
    pub fn base(&self) -> ChangePointId {
        self.base
    }

    /// Why the work exists.
    pub fn intent(&self) -> &Intent {
        &self.intent
    }

    /// When it was opened, and by whom.
    pub fn act(&self) -> &Act {
        &self.act
    }
}

/// One pass at the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    id: NodeId,
    parent: NodeId,
    taken_in: Option<ChangePointId>,
    ops: Vec<Op>,
    note: Option<String>,
    act: Act,
}

impl Round {
    /// Records a pass.
    ///
    /// Refuses a pass that neither writes anything nor takes anything
    /// in: a node that says nothing is a record that nothing happened,
    /// and the work log is a record of what did.
    pub fn new(
        parent: NodeId,
        ops: Vec<Op>,
        taken_in: Option<ChangePointId>,
        note: Option<String>,
        act: Act,
    ) -> Result<Self, ForgeError> {
        if ops.is_empty() && taken_in.is_none() {
            return Err(ForgeError::EmptyRound);
        }
        Ok(Self {
            id: NodeId::new(),
            parent,
            taken_in,
            ops,
            note,
            act,
        })
    }

    /// Which node.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The node before it.
    pub fn parent(&self) -> NodeId {
        self.parent
    }

    /// The change point this pass looked at, if it looked at one.
    /// Evidence of having seen it — never a move of the base.
    pub fn taken_in(&self) -> Option<ChangePointId> {
        self.taken_in
    }

    /// What this pass wrote, in the order it wrote it.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Anything the pass said about itself.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// When it happened, and who did it.
    pub fn act(&self) -> &Act {
        &self.act
    }
}

/// The node work ends at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Close {
    id: NodeId,
    parent: NodeId,
    outcome: Outcome,
    note: Option<String>,
    act: Act,
}

impl Close {
    /// Ends the work.
    pub fn new(parent: NodeId, outcome: Outcome, note: Option<String>, act: Act) -> Self {
        Self {
            id: NodeId::new(),
            parent,
            outcome,
            note,
            act,
        }
    }

    /// Which node.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The node before it.
    pub fn parent(&self) -> NodeId {
        self.parent
    }

    /// Which kind of ending.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Anything said about the ending.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// When it ended, and who ended it.
    pub fn act(&self) -> &Act {
        &self.act
    }
}

/// The chain of passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkLog {
    open: Open,
    rounds: Vec<Round>,
    close: Option<Close>,
}

impl WorkLog {
    /// Begins a work log at the node work was cut at.
    pub fn begin(open: Open) -> Self {
        Self {
            open,
            rounds: Vec::new(),
            close: None,
        }
    }

    /// The node work began at.
    pub fn open(&self) -> &Open {
        &self.open
    }

    /// Every pass, in order.
    pub fn rounds(&self) -> &[Round] {
        &self.rounds
    }

    /// How it ended, if it has.
    pub fn close(&self) -> Option<&Close> {
        self.close.as_ref()
    }

    /// The last node — what the next one takes as its parent.
    pub fn head(&self) -> NodeId {
        if let Some(close) = &self.close {
            return close.id();
        }
        self.rounds
            .last()
            .map(Round::id)
            .unwrap_or_else(|| self.open.id())
    }

    /// Appends a pass.
    ///
    /// Refused if the work has ended, or if the pass does not sit on
    /// the head. Ended work that could still be written to would make
    /// the ending a note rather than a fact, and every reader would
    /// have to ask whether what follows counts.
    pub fn push(&mut self, round: Round) -> Result<(), ForgeError> {
        if self.close.is_some() {
            return Err(ForgeError::AlreadyClosed);
        }
        if round.parent() != self.head() {
            return Err(ForgeError::NotOnHead);
        }
        self.rounds.push(round);
        Ok(())
    }

    /// Ends the work.
    ///
    /// Refused twice over for the same reason as a pass: work ends
    /// once, and the ending sits on the head.
    pub fn end(&mut self, close: Close) -> Result<(), ForgeError> {
        if self.close.is_some() {
            return Err(ForgeError::AlreadyClosed);
        }
        if close.parent() != self.head() {
            return Err(ForgeError::NotOnHead);
        }
        self.close = Some(close);
        Ok(())
    }

    /// What this work says, folded across every pass.
    ///
    /// The line is not an input — see
    /// [`fold`](crate::domain::forge::model::op::fold).
    pub fn says(&self) -> Rows {
        let ops: Vec<Op> = self
            .rounds
            .iter()
            .flat_map(|round| round.ops().iter().cloned())
            .collect();
        fold(&ops)
    }
}

/// One line of work against one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pursuit {
    id: PursuitId,
    of: LineId,
    parent: Option<PursuitId>,
    log: WorkLog,
    meta: Meta,
}

impl Pursuit {
    /// Opens work against a line, cut from a change point on it.
    pub fn open(
        of: LineId,
        parent: Option<PursuitId>,
        base: ChangePointId,
        intent: Intent,
        act: Act,
    ) -> Self {
        Self {
            id: PursuitId::new(),
            of,
            parent,
            log: WorkLog::begin(Open::new(base, intent, act.clone())),
            meta: Meta::opened(act),
        }
    }

    /// Which pursuit.
    pub fn id(&self) -> PursuitId {
        self.id
    }

    /// The line this work is against. Declared when the work opens,
    /// and never derived from what it happens to touch.
    pub fn of(&self) -> LineId {
        self.of
    }

    /// The larger piece of work this belongs to, if any.
    pub fn parent(&self) -> Option<PursuitId> {
        self.parent
    }

    /// The change point this work was cut from. Fixed.
    pub fn base(&self) -> ChangePointId {
        self.log.open().base()
    }

    /// The passes.
    pub fn log(&self) -> &WorkLog {
        &self.log
    }

    /// When the work was opened, and the last time its own
    /// description moved.
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// Whether the work has ended, and how.
    pub fn outcome(&self) -> Option<Outcome> {
        self.log.close().map(Close::outcome)
    }

    /// What this work says, folded across every pass.
    pub fn says(&self) -> Rows {
        self.log.says()
    }

    /// Records a pass.
    pub fn push(&mut self, round: Round) -> Result<(), ForgeError> {
        self.log.push(round)
    }

    /// Ends the work.
    pub fn end(&mut self, close: Close) -> Result<(), ForgeError> {
        self.log.end(close)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // SHARED KERNEL: attribution is a boundary type.
    use crate::domain::attribution::AttributionContext;
    use crate::domain::forge::model::table::Row;
    use crate::domain::forge::model::value::Content;
    use chrono::{DateTime, TimeZone, Utc};
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), &AttributionContext::owner_surface())
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn name(value: &str) -> Name {
        Name::new(value).unwrap()
    }

    fn opened() -> Pursuit {
        Pursuit::open(
            LineId::new(),
            None,
            ChangePointId::new(),
            Intent::default(),
            act(0),
        )
    }

    fn round(parent: NodeId, ops: Vec<Op>, minute: u32) -> Round {
        Round::new(parent, ops, None, None, act(minute)).unwrap()
    }

    #[test]
    fn fresh_work_has_said_nothing_and_has_not_ended() {
        let pursuit = opened();

        assert!(pursuit.says().is_empty());
        assert_eq!(pursuit.outcome(), None);
        assert_eq!(pursuit.log().head(), pursuit.log().open().id());
        assert!(pursuit.log().rounds().is_empty());
    }

    #[test]
    fn passes_accumulate_into_what_the_work_says() {
        let mut pursuit = opened();
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();
        let held = content();

        pursuit
            .push(round(pursuit.log().head(), vec![added], 1))
            .unwrap();
        pursuit
            .push(round(
                pursuit.log().head(),
                vec![Op::replace(entry, held)],
                2,
            ))
            .unwrap();

        assert_eq!(pursuit.says()[&entry], Row::added(held, name("key visual")));
        assert_eq!(pursuit.log().rounds().len(), 2);
    }

    /// A pass that takes a change point in without writing anything is
    /// work saying it has looked. It is a node like any other.
    #[test]
    fn a_pass_may_take_something_in_and_write_nothing() {
        let mut pursuit = opened();
        let seen = ChangePointId::new();

        let looked =
            Round::new(pursuit.log().head(), Vec::new(), Some(seen), None, act(1)).unwrap();
        pursuit.push(looked).unwrap();

        assert_eq!(pursuit.log().rounds()[0].taken_in(), Some(seen));
        assert!(pursuit.says().is_empty());
    }

    #[test]
    fn a_pass_that_neither_writes_nor_looks_is_refused() {
        let pursuit = opened();

        let refused = Round::new(pursuit.log().head(), Vec::new(), None, None, act(1));

        assert_eq!(refused.unwrap_err(), ForgeError::EmptyRound);
    }

    /// Taking a newer change point in is evidence, not a move: the
    /// base is where the work was cut, and stays there.
    #[test]
    fn taking_something_in_does_not_move_the_base() {
        let mut pursuit = opened();
        let base = pursuit.base();

        let looked = Round::new(
            pursuit.log().head(),
            Vec::new(),
            Some(ChangePointId::new()),
            None,
            act(1),
        )
        .unwrap();
        pursuit.push(looked).unwrap();

        assert_eq!(pursuit.base(), base);
    }

    #[test]
    fn work_ends_once_and_nothing_follows_it() {
        let mut pursuit = opened();
        pursuit
            .push(round(
                pursuit.log().head(),
                vec![Op::add(content(), name("key visual"))],
                1,
            ))
            .unwrap();

        pursuit
            .end(Close::new(
                pursuit.log().head(),
                Outcome::Satisfied,
                None,
                act(2),
            ))
            .unwrap();

        let after = round(
            pursuit.log().head(),
            vec![Op::add(content(), name("late"))],
            3,
        );
        assert_eq!(pursuit.push(after).unwrap_err(), ForgeError::AlreadyClosed);
        let twice = Close::new(pursuit.log().head(), Outcome::Abandoned, None, act(4));
        assert_eq!(pursuit.end(twice).unwrap_err(), ForgeError::AlreadyClosed);
        assert_eq!(pursuit.outcome(), Some(Outcome::Satisfied));
    }

    #[test]
    fn a_pass_that_does_not_sit_on_the_head_is_refused() {
        let mut pursuit = opened();
        let stale = pursuit.log().head();
        pursuit
            .push(round(
                stale,
                vec![Op::add(content(), name("key visual"))],
                1,
            ))
            .unwrap();

        let refused = pursuit.push(round(stale, vec![Op::add(content(), name("alternate"))], 2));

        assert_eq!(refused.unwrap_err(), ForgeError::NotOnHead);
        assert_eq!(pursuit.log().rounds().len(), 1);
    }

    /// Abandoned work keeps everything it wrote. That is the whole
    /// point of recording an ending rather than removing the work.
    #[test]
    fn abandoned_work_still_says_what_it_said() {
        let mut pursuit = opened();
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();
        pursuit
            .push(round(pursuit.log().head(), vec![added], 1))
            .unwrap();

        pursuit
            .end(Close::new(
                pursuit.log().head(),
                Outcome::Abandoned,
                Some("the client went another way".to_string()),
                act(2),
            ))
            .unwrap();

        assert_eq!(pursuit.outcome(), Some(Outcome::Abandoned));
        assert!(pursuit.says().contains_key(&entry));
        assert_eq!(
            pursuit.log().close().unwrap().note(),
            Some("the client went another way")
        );
    }

    #[test]
    fn work_belongs_to_the_line_it_declares_and_to_its_parent() {
        let line = LineId::new();
        let epic = PursuitId::new();

        let pursuit = Pursuit::open(
            line,
            Some(epic),
            ChangePointId::new(),
            Intent {
                title: Some(name("the album cover")),
                note: None,
            },
            act(0),
        );

        assert_eq!(pursuit.of(), line);
        assert_eq!(pursuit.parent(), Some(epic));
        assert_eq!(
            pursuit.log().open().intent().title.as_ref(),
            Some(&name("the album cover"))
        );
    }
}
