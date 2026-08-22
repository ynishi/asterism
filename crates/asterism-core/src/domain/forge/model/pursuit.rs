//! One line of work: what it is trying to do, and every pass at it.
//!
//! ```text
//!   Pursuit ── of : →Line ── parent? : →Pursuit
//!    └ WorkLog
//!        Open ──▶ Round ──▶ Round ──▶ Close?
//!          │        │  └ ops
//!          │        └ parent
//!          └ base : the change point this was cut from
//! ```
//!
//! The shape mirrors a line's history deliberately: a root that is not
//! like the others, a chain that only grows, and everything the thing
//! currently asks for derived by folding it. What differs is what the
//! nodes carry — a line's node carries a table that has changed it, a
//! work log's carries operations that have not changed anything yet.
//!
//! # A pass is the unit, not the pursuit
//!
//! [`Round`] is where work happens. A pursuit is the container that
//! says what the passes are for, and it holds nothing that a round
//! could hold instead.
//!
//! A round that writes nothing is refused. Work is what a person does
//! to a request, and a node carrying no operations records that
//! nothing happened.
//!
//! # The base does not move
//!
//! [`Open`] names the change point the work was cut from, and that is
//! the only thing the base ever is. Nothing here moves it, and there
//! is no operation that could.
//!
//! It is what "since" is measured from. Everything the line recorded
//! after it is something this work may not account for, and comparing
//! the two is how that is found out — so a base that crept forward
//! would shrink the window every time somebody looked at it, and a
//! change nobody ever reconciled would come out clean.
//!
//! # A pass is a write, and nothing else
//!
//! There is no node here that records having looked at something.
//! Work stops colliding with a line by *saying something different*,
//! not by noting that it read. A note of that kind would be a claim
//! about the reader — writable without changing anything — and
//! whatever depended on it could be had for nothing.
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
//! under a parent, and what they changed, is a question answered by
//! looking — and a stored answer would be a second copy of it.

use std::collections::BTreeSet;

use crate::domain::forge::model::act::{Act, Meta};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::op::{Op, OpKind, Rows, fold};
use crate::domain::forge::model::value::{ChangePointId, Content, LineId, Name, NodeId, PursuitId};

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
    /// What the work set out to do is done, and the line carries it.
    ///
    /// This value is never the whole of the fact. A satisfied close
    /// and the change point it puts on the line are born together and
    /// neither exists without the other, so nothing outside the model
    /// can mint a close at all — the one function that ends work
    /// returns both or refuses.
    Satisfied,
    /// It is not, and nobody is going to. Nothing is put on the line,
    /// and everything the work wrote stays readable.
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

    /// Rebuilds the node work opened at, under the id it was kept
    /// with.
    ///
    /// Visible to the model and no further — see
    /// [`restore`](super::restore).
    pub(super) fn restored(id: NodeId, base: ChangePointId, intent: Intent, act: Act) -> Self {
        Self {
            id,
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
    ops: Vec<Op>,
    note: Option<String>,
    act: Act,
}

impl Round {
    /// Records a pass.
    ///
    /// Refuses one that writes nothing. A pass is what work does, and
    /// a node carrying no operations is a record that nothing
    /// happened — the work log is a record of what did.
    ///
    /// There is no pass that only looks at something. Looking is not
    /// an operation, and a node that claimed it would be a claim about
    /// the reader rather than a fact about the work: it could be
    /// written without changing anything, and anything a collision
    /// depended on it for could be had for free.
    pub fn new(
        parent: NodeId,
        ops: Vec<Op>,
        note: Option<String>,
        act: Act,
    ) -> Result<Self, ForgeError> {
        if ops.is_empty() {
            return Err(ForgeError::EmptyRound);
        }
        Ok(Self {
            id: NodeId::new(),
            parent,
            ops,
            note,
            act,
        })
    }

    /// Rebuilds a pass under the id it was kept with.
    ///
    /// Refuses an empty one for the reason [`new`](Self::new) does: a
    /// stored node carrying no operations is a record that nothing
    /// happened, and reading it back would let the log say what it was
    /// refused permission to say. Visible to the model and no further
    /// — see [`restore`](super::restore).
    pub(super) fn restored(
        id: NodeId,
        parent: NodeId,
        ops: Vec<Op>,
        note: Option<String>,
        act: Act,
    ) -> Result<Self, ForgeError> {
        if ops.is_empty() {
            return Err(ForgeError::EmptyRound);
        }
        Ok(Self {
            id,
            parent,
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
    ///
    /// Visible to the model and no further, for the reason stated on
    /// [`Outcome::Satisfied`]: a satisfied close and the change point
    /// it puts on the line are one act, and a constructor anybody
    /// could reach is a way to write half of it.
    pub(super) fn new(parent: NodeId, outcome: Outcome, note: Option<String>, act: Act) -> Self {
        Self {
            id: NodeId::new(),
            parent,
            outcome,
            note,
            act,
        }
    }

    /// Rebuilds an ending under the id it was kept with.
    ///
    /// Visible to the model and no further — see
    /// [`restore`](super::restore).
    pub(super) fn restored(
        id: NodeId,
        parent: NodeId,
        outcome: Outcome,
        note: Option<String>,
        act: Act,
    ) -> Self {
        Self {
            id,
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

    /// Every content this log has ever named.
    ///
    /// The line's [`holds`](crate::domain::forge::model::line::Line::holds)
    /// with the same meaning and the same reason: an operation naming
    /// content that somebody deleted is an operation that cannot be
    /// read, folded, or landed. Work that gave up still holds what it
    /// named — what it tried is the record, and a record pointing at
    /// nothing is not one.
    pub fn holds(&self) -> BTreeSet<Content> {
        self.rounds
            .iter()
            .flat_map(|round| round.ops())
            .filter_map(|op| match op.kind() {
                OpKind::Add { content, .. } | OpKind::Replace { content } => Some(*content),
                OpKind::Rename { .. } | OpKind::Remove => None,
            })
            .collect()
    }

    /// What this work is asking the line to carry, folded across every
    /// pass.
    ///
    /// The line is not an input — see [`fold`]. What the request means
    /// is only decided against a line, and that happens elsewhere.
    pub fn request(&self) -> Rows {
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
            log: WorkLog::begin(Open::new(base, intent, act)),
            meta: Meta::opened(act),
        }
    }

    /// Rebuilds work under the id it was kept with, around a log put
    /// back node by node.
    ///
    /// Visible to the model and no further — see
    /// [`restore`](super::restore).
    pub(super) fn restored(
        id: PursuitId,
        of: LineId,
        parent: Option<PursuitId>,
        log: WorkLog,
        meta: Meta,
    ) -> Self {
        Self {
            id,
            of,
            parent,
            log,
            meta,
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

    /// Every content this work's log has ever named — see
    /// [`WorkLog::holds`].
    pub fn holds(&self) -> BTreeSet<Content> {
        self.log.holds()
    }

    /// What this work is asking the line to carry.
    pub fn request(&self) -> Rows {
        self.log.request()
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
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::table::Row;
    use crate::domain::forge::model::value::ActorId;
    use crate::domain::forge::model::value::{Content, EntryId};
    use chrono::{DateTime, TimeZone, Utc};
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), Actor::User(ActorId::new()))
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
        Round::new(parent, ops, None, act(minute)).unwrap()
    }

    #[test]
    fn fresh_work_has_said_nothing_and_has_not_ended() {
        let pursuit = opened();

        assert!(pursuit.request().is_empty());
        assert_eq!(pursuit.outcome(), None);
        assert_eq!(pursuit.log().head(), pursuit.log().open().id());
        assert!(pursuit.log().rounds().is_empty());
    }

    #[test]
    fn passes_accumulate_into_what_the_work_request() {
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

        assert_eq!(
            pursuit.request()[&entry],
            Row::added(held, name("key visual"))
        );
        assert_eq!(pursuit.log().rounds().len(), 2);
    }

    #[test]
    fn a_pass_that_writes_nothing_is_refused() {
        let pursuit = opened();

        let refused = Round::new(pursuit.log().head(), Vec::new(), None, act(1));

        assert_eq!(refused.unwrap_err(), ForgeError::EmptyRound);
    }

    /// The base is where the work was cut, and no pass moves it —
    /// there is no operation that could, which is the only way to mean
    /// it.
    #[test]
    fn passes_do_not_move_the_base() {
        let mut pursuit = opened();
        let base = pursuit.base();

        for minute in 1..4 {
            let pass = round(
                pursuit.log().head(),
                vec![Op::add(
                    Content::from_uuid(Uuid::now_v7()),
                    name("key visual"),
                )],
                minute,
            );
            pursuit.push(pass).unwrap();
        }

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
        assert!(pursuit.request().contains_key(&entry));
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

    /// A work log holds what it named, for the same reason a line
    /// does: an operation pointing at bytes somebody deleted is one
    /// nothing can fold, land, or read back.
    #[test]
    fn a_work_log_holds_what_its_operations_named_including_the_one_it_took_off() {
        let mut work = opened();
        let added = content();
        let replaced = content();
        let entry = EntryId::new();

        work.push(
            Round::new(
                work.log().head(),
                vec![
                    Op::add_to(entry, added, name("one")),
                    Op::replace(entry, replaced),
                    Op::rename(entry, name("two")),
                    Op::remove(entry),
                ],
                None,
                act(1),
            )
            .unwrap(),
        )
        .unwrap();

        let held = work.holds();
        assert!(held.contains(&added), "what it first asked for");
        assert!(held.contains(&replaced), "and what it moved to");
        assert_eq!(held.len(), 2, "a rename and a removal name no content");
    }

    /// Giving up releases nothing. What was tried is the record, and a
    /// record pointing at nothing is not one.
    #[test]
    fn abandoned_work_still_holds_what_it_named() {
        let mut work = opened();
        let tried = content();
        work.push(
            Round::new(
                work.log().head(),
                vec![Op::add(tried, name("tried"))],
                None,
                act(1),
            )
            .unwrap(),
        )
        .unwrap();
        work.end(Close::new(
            work.log().head(),
            Outcome::Abandoned,
            None,
            act(2),
        ))
        .unwrap();

        assert_eq!(work.outcome(), Some(Outcome::Abandoned));
        assert!(work.holds().contains(&tried));
    }
}
