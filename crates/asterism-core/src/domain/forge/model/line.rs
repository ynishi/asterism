//! The line — the forge's top entity.
//!
//! A line is a repository: one canonical history, and everything that
//! is on it derives from that history.
//!
//! ```text
//!   Line ── history ──▶ what the line carries   (a chain, folded)
//!    │
//!    └──── meta ──────▶ what the line is        (created / renamed)
//! ```
//!
//! # The top of the forge
//!
//! Every rule the forge states is stated per line: what a name is
//! unique among, where a pursuit files, how collisions settle, what
//! the canonical set is. A line is therefore the largest thing the
//! forge has an opinion about.
//!
//! Grouping and ownership are outside it, with the model of teams and
//! members that holds them, and the forge answers nothing about whose
//! line this is.
//!
//! # The name, and what it does not claim
//!
//! A line keeps an identifier, because a line has to be callable by
//! something a person chose, and reaching outside for a string on
//! every read would put the display of a line somewhere other than the
//! line.
//!
//! What it does not keep is any claim about that name. "Unique among
//! what?" needs an owner to answer — a person's lines, a team's lines
//! — and the owner is outside, so uniqueness is enforced where the
//! namespace lives. [`Name`] promises exactly one thing: it is not
//! blank. Where there is only ever one line, it is [`Line::ROOT`].
//!
//! # Two records, and neither moves the other
//!
//! [`Line::record`] moves the history. [`Line::rename`] and
//! [`Line::set_strategy`] move the description. A rename is not a
//! change point — the history says what happened to what the line
//! carries, and a rename did not — and recording one does not touch
//! [`Meta`], because "the line moved" and "the line is described
//! differently" would otherwise collapse into one value that answers
//! neither question.
//!
//! # Standing, and why it is not in the history
//!
//! ```text
//!   Open ──archive──▶ Archived ──(drop)──▶ gone
//!     ▲                   │                  │
//!     └──── reopen ───────┘        what it held is released
//!   takes change points   readable, holds
//!                         everything still
//! ```
//!
//! [`Standing`] sits beside the name and the strategy rather than in
//! the chain, for the same reason a rename is not a change point: the
//! history says what happened to what the line *carries*, and "this
//! line is finished with" is a statement about the line. It moves
//! [`Meta`] and nothing else.
//!
//! **Dropping is reachable only through the archive**, as purging is
//! reachable only through the trash everywhere else here. Two steps,
//! because the second one is irreversible and takes the history with
//! it.
//!
//! # What a line holds, and why anything cares
//!
//! [`Line::holds`] is every [`Content`] any change point on the line
//! has ever named. It is not a cache and not a second record — it is a
//! fold, like everything else about a line — but it is the one fold
//! something outside the forge has to act on: **while a line holds a
//! content, the layer that keeps the bytes may not let it go.**
//!
//! An entry taken off the line does not release what it held. The
//! change point that put it there is still in the chain and still
//! names it, and the chain is not rewritten. So the set only ever
//! grows, and the only thing that shrinks it is dropping the line.
//!
//! That is deliberate rather than unfortunate. A line says what is on
//! it *now*: `alive`, under this name, at this content. A line saying
//! that about bytes somebody deleted is a line telling a lie about the
//! present, which is a different thing from a log of past events —
//! those stay true whatever happens to what they name, which is why
//! the ledger this model replaced could name an asset without holding
//! it.
//!
//! # Rewriting is not a verb here
//!
//! There is no filter, no rebase, no editing a change point after the
//! fact. Wanting one is usually wanting to release a content without
//! dropping everything, and the answer is that the same result is
//! reachable with the verbs that already exist: open a new line, and
//! put on it what should have been there. That is a new history rather
//! than an edited one, which is what it honestly is — the change
//! points of a filtered line could not name the work they came out
//! of, because that work asked for something else.
//!
//! What the old line then needs is to be archived and dropped, which
//! is the pair above.

use std::collections::BTreeSet;

use crate::domain::forge::model::act::{Act, Meta};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::{ChangePoint, History};
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{ChangePointId, Content, LineId, Name, StrategyId};

/// Whether a line is still being worked on.
///
/// Two values and not three: "dropped" is absence, not a state. A row
/// saying a line is gone would be a line that is still there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Standing {
    /// Takes change points, and work may be cut from it.
    #[default]
    Open,
    /// Finished with. Readable in full, holds everything it held, and
    /// takes nothing new until it is reopened.
    Archived,
}

/// One repository: an identifier, a history, and how it settles
/// collisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    id: LineId,
    name: Name,
    strategy: StrategyId,
    standing: Standing,
    history: History,
    meta: Meta,
}

impl Line {
    /// The name a line carries where there is only ever one.
    pub const ROOT: &'static str = "ROOT";

    /// Opens a line. It begins with its genesis, so it has a head from
    /// the moment it exists and the first pursuit cuts from it like
    /// every later one.
    ///
    /// `strategy` names the rule it settles collisions by. There is no
    /// default to fall back on: which rule is right depends on what
    /// the line is for, and a line that quietly got somebody else's
    /// answer would settle collisions in a way nobody chose.
    pub fn open(name: Name, strategy: StrategyId, act: Act) -> Self {
        Self {
            id: LineId::new(),
            name,
            strategy,
            standing: Standing::Open,
            history: History::begin(act),
            meta: Meta::opened(act),
        }
    }

    /// Rebuilds a line under the id it was kept with, around a
    /// history put back node by node.
    ///
    /// Visible to the model and no further — see
    /// [`restore`](super::restore).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn restored(
        id: LineId,
        name: Name,
        strategy: StrategyId,
        standing: Standing,
        history: History,
        meta: Meta,
    ) -> Self {
        Self {
            id,
            name,
            strategy,
            standing,
            history,
            meta,
        }
    }

    /// Which line.
    pub fn id(&self) -> LineId {
        self.id
    }

    /// What it is called. The forge stores this and claims nothing
    /// about it.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// Which rule it settles collisions by.
    ///
    /// A name, not a rule: what it resolves to is decided by whoever
    /// runs the forge, and the model never holds one.
    pub fn strategy(&self) -> &StrategyId {
        &self.strategy
    }

    /// Its history, which is the only record of what it carries.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// When the line was opened, and the last time its own
    /// description moved.
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// The head of the history — the node a change has to name as its
    /// parent.
    pub fn head(&self) -> ChangePointId {
        self.history.head()
    }

    /// What is on the line: the history, folded.
    ///
    /// Derived on every call rather than kept beside the history,
    /// because a kept copy is a second thing that has to stay true.
    pub fn states(&self) -> EntryStates {
        self.history.states()
    }

    /// Records a change. The only way the line moves.
    pub fn record(&mut self, point: ChangePoint) -> Result<(), ForgeError> {
        if self.standing == Standing::Archived {
            return Err(ForgeError::Archived);
        }
        self.history.record(point)
    }

    /// Renames the line. Its own description moving is not something
    /// the history records — a change point says what happened to what
    /// the line carries, and this did not.
    pub fn rename(&mut self, name: Name, act: Act) {
        self.name = name;
        self.meta.touched(act);
    }

    /// Points the line at a different rule.
    pub fn set_strategy(&mut self, strategy: StrategyId, act: Act) {
        self.strategy = strategy;
        self.meta.touched(act);
    }

    /// Whether the line is still being worked on.
    pub fn standing(&self) -> Standing {
        self.standing
    }

    /// Finishes with the line. Idempotent.
    ///
    /// Nothing is lost and nothing is released: the history is intact,
    /// readable, and still holding every content it named. What stops
    /// is movement — [`record`](Self::record) refuses from here, so
    /// nothing lands on an archived line and no work closes onto one.
    ///
    /// Not a change point, for the reason a rename is not one: the
    /// chain says what happened to what the line carries, and this did
    /// not happen to any of that.
    pub fn archive(&mut self, act: Act) {
        self.standing = Standing::Archived;
        self.meta.touched(act);
    }

    /// Takes it back out of the archive. Idempotent.
    pub fn reopen(&mut self, act: Act) {
        self.standing = Standing::Open;
        self.meta.touched(act);
    }

    /// Every content this line has ever named.
    ///
    /// The set the layer holding the bytes may not let go of while
    /// this line exists. Folded from the chain like everything else,
    /// and it only grows: an entry taken off the line does not release
    /// what it held, because the change point that put it there is
    /// still in the chain and still names it.
    ///
    /// Read the module docs for why a line holds rather than merely
    /// mentions.
    pub fn holds(&self) -> BTreeSet<Content> {
        self.history
            .changes()
            .iter()
            .flat_map(|point| point.table().rows().values())
            .filter_map(|row| row.content())
            .collect()
    }

    /// Whether this line may be dropped, and what is in the way.
    ///
    /// Dropping is not a method here, because a value cannot delete
    /// itself and a `Line` that returned "I am gone" would be a line
    /// that is still there. What the model owns is the rule; whatever
    /// does the deleting asks this first.
    ///
    /// `open_work` is how many pursuits against this line have not
    /// ended. The caller counts them because the line does not hold
    /// its work — the two logs are separate, and a line that kept a
    /// list of its pursuits would be keeping a second answer to a
    /// question the pursuits already answer.
    ///
    /// # Refusals
    ///
    /// - [`NotArchived`](ForgeError::NotArchived) — the line is still
    ///   open. Dropping is reachable only through the archive.
    /// - [`WorkStillOpen`](ForgeError::WorkStillOpen) — work is open
    ///   against it, and dropping would leave a log cut from a history
    ///   that no longer exists.
    pub fn may_drop(&self, open_work: usize) -> Result<(), ForgeError> {
        if self.standing != Standing::Archived {
            return Err(ForgeError::NotArchived);
        }
        if open_work > 0 {
            return Err(ForgeError::WorkStillOpen(open_work));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::table::{Row, Table};
    use crate::domain::forge::model::value::ActorId;
    use crate::domain::forge::model::value::{Content, EntryId, NodeId, PursuitId};
    use chrono::{DateTime, TimeZone, Utc};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), Actor::User(ActorId::new()))
    }

    fn root() -> Line {
        Line::open(
            Name::new(Line::ROOT).unwrap(),
            StrategyId::new("mainline-first").unwrap(),
            act(0),
        )
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    /// A change point sitting on the line's current head, carrying
    /// whatever the caller wants it to.
    fn point_carrying(line: &Line, table: Table) -> ChangePoint {
        ChangePoint::new(line.head(), PursuitId::new(), NodeId::new(), table, act(1))
    }

    fn a_point(line: &Line) -> ChangePoint {
        point_carrying(
            line,
            Table::one(EntryId::new(), Row::added(content(), name("one"))),
        )
    }

    #[test]
    fn a_new_line_carries_nothing_and_still_has_a_head() {
        let line = root();

        assert_eq!(line.head(), line.history().genesis().id());
        assert!(line.states().is_empty());
        assert_eq!(line.strategy().as_str(), "mainline-first");
    }

    #[test]
    fn recording_a_change_puts_an_entry_on_the_line() {
        let mut line = root();
        let entry = EntryId::new();
        let content = Content::from_uuid(Uuid::now_v7());
        let point = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            NodeId::new(),
            Table::one(entry, Row::added(content, Name::new("key visual").unwrap())),
            act(1),
        );

        line.record(point).unwrap();

        let states = line.states();
        assert!(states.get(&entry).unwrap().alive);
        assert_eq!(states.get(&entry).unwrap().content, Some(content));
    }

    /// The line's own description and what it carries are different
    /// records, and neither moves the other.
    #[test]
    fn renaming_moves_the_description_and_not_the_history() {
        let mut line = root();
        let head_before = line.head();

        line.rename(Name::new("alternate").unwrap(), act(5));

        assert_eq!(line.name().as_str(), "alternate");
        assert_eq!(line.meta().updated().at(), at(5));
        assert_eq!(line.meta().created().at(), at(0));
        assert_eq!(line.head(), head_before);
    }

    #[test]
    fn recording_a_change_does_not_touch_the_description() {
        let mut line = root();
        let point = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            NodeId::new(),
            Table::one(
                EntryId::new(),
                Row::added(
                    Content::from_uuid(Uuid::now_v7()),
                    Name::new("key visual").unwrap(),
                ),
            ),
            act(3),
        );

        line.record(point).unwrap();

        assert_eq!(line.meta().updated().at(), at(0));
    }

    /// The rules the history holds are the line's rules too — the line
    /// does not get a second, laxer way in.
    #[test]
    fn the_line_refuses_what_its_history_refuses() {
        let mut line = root();
        let taken = Name::new("key visual").unwrap();
        let first = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            NodeId::new(),
            Table::one(
                EntryId::new(),
                Row::added(Content::from_uuid(Uuid::now_v7()), taken.clone()),
            ),
            act(1),
        );
        line.record(first).unwrap();

        let twin = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            NodeId::new(),
            Table::one(
                EntryId::new(),
                Row::added(Content::from_uuid(Uuid::now_v7()), taken.clone()),
            ),
            act(2),
        );
        let refused = line.record(twin);

        assert_eq!(refused, Err(ForgeError::NameTaken(taken)));
        assert_eq!(line.history().changes().len(), 1);
    }

    #[test]
    fn a_line_turns_automatic_divergence_off() {
        let mut line = root();

        line.set_strategy(StrategyId::new("by-hand").unwrap(), act(2));

        assert_eq!(line.strategy().as_str(), "by-hand");
        assert_eq!(line.meta().updated().at(), at(2));
    }

    #[test]
    fn an_archived_line_takes_nothing_and_reopening_lets_it_move_again() {
        let mut line = root();
        let point = a_point(&line);
        line.archive(act(1));
        assert_eq!(line.standing(), Standing::Archived);

        let refused = line.record(point.clone());
        assert!(matches!(refused, Err(ForgeError::Archived)), "{refused:?}");
        assert!(
            line.history().changes().is_empty(),
            "a refused record left nothing behind"
        );

        line.reopen(act(2));
        assert_eq!(line.standing(), Standing::Open);
        line.record(point).expect("it moves again");
        assert_eq!(line.history().changes().len(), 1);
    }

    #[test]
    fn archiving_moves_the_description_and_not_the_history() {
        let mut line = root();
        let head = line.head();
        line.archive(act(5));

        assert_eq!(line.head(), head, "the chain did not move");
        assert_eq!(line.meta().updated().at(), at(5), "the description did");
        assert_eq!(
            line.meta().created().at(),
            at(0),
            "and the line was still made when it was made"
        );
    }

    #[test]
    fn archiving_and_reopening_are_idempotent() {
        let mut line = root();
        line.archive(act(1));
        line.archive(act(2));
        assert_eq!(line.standing(), Standing::Archived);
        line.reopen(act(3));
        line.reopen(act(4));
        assert_eq!(line.standing(), Standing::Open);
    }

    #[test]
    fn a_line_holds_what_it_has_ever_named_and_taking_an_entry_off_releases_nothing() {
        let mut line = root();
        let held = content();
        let entry = EntryId::new();

        let mut put = BTreeMap::new();
        put.insert(entry, Row::added(held, name("one")));
        line.record(point_carrying(&line, Table::of(put).unwrap()))
            .unwrap();
        assert!(line.holds().contains(&held));

        let mut off = BTreeMap::new();
        off.insert(entry, Row::removed());
        line.record(point_carrying(&line, Table::of(off).unwrap()))
            .unwrap();

        assert!(!line.states()[&entry].alive, "the entry is off the line");
        assert!(
            line.holds().contains(&held),
            "and the line still holds what it named: the change point that put it \
             there is still in the chain"
        );
    }

    #[test]
    fn a_line_is_dropped_from_the_archive_and_not_before() {
        let mut line = root();
        assert!(matches!(line.may_drop(0), Err(ForgeError::NotArchived)));

        line.archive(act(1));
        assert!(line.may_drop(0).is_ok());
    }

    #[test]
    fn a_line_with_work_still_open_is_not_dropped() {
        let mut line = root();
        line.archive(act(1));

        let refused = line.may_drop(2);
        assert!(
            matches!(refused, Err(ForgeError::WorkStillOpen(2))),
            "the refusal says how many: {refused:?}"
        );
    }
}
