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
//! [`Line::land`] moves the history. [`Line::rename`] and
//! [`Line::set_strategy`] move the description. A rename is not a
//! change point — the history says what happened to what the line
//! carries, and a rename did not — and landing does not touch
//! [`Meta`], because "the line moved" and "the line is described
//! differently" would otherwise collapse into one value that answers
//! neither question.

use crate::domain::forge::model::act::{Act, Meta};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::{ChangePoint, History};
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{ChangePointId, LineId, Name};

/// What a line does when a landing collides with one that came before
/// it.
///
/// What diverges is the **entry**, never the chain: the collision is
/// settled by putting a second entry on the line beside the first, so
/// both candidates survive and the history stays one chain. Whether
/// that happens without asking is the setting, and it belongs to the
/// line because the history does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Turn a collision into divergence without asking. The default.
    Auto,
    /// Never do that on this line.
    NoAuto,
}

/// One repository: an identifier, a history, and how it settles
/// collisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    id: LineId,
    name: Name,
    strategy: Strategy,
    history: History,
    meta: Meta,
}

impl Line {
    /// The name a line carries where there is only ever one.
    pub const ROOT: &'static str = "ROOT";

    /// Opens a line. It begins with its genesis, so it has a head from
    /// the moment it exists and the first pursuit cuts from it like
    /// every later one.
    pub fn open(name: Name, act: Act) -> Self {
        Self {
            id: LineId::new(),
            name,
            strategy: Strategy::Auto,
            history: History::begin(act.clone()),
            meta: Meta::opened(act),
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

    /// How it settles collisions.
    pub fn strategy(&self) -> Strategy {
        self.strategy
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

    /// The head of the history — the node a landing has to name as its
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

    /// Lands a change point. The only way the line moves.
    pub fn land(&mut self, point: ChangePoint) -> Result<(), ForgeError> {
        self.history.land(point)
    }

    /// Renames the line. Its own description moving is not something
    /// the history records — a change point says what happened to what
    /// the line carries, and this did not.
    pub fn rename(&mut self, name: Name, act: Act) {
        self.name = name;
        self.meta.touched(act);
    }

    /// Changes how collisions settle.
    pub fn set_strategy(&mut self, strategy: Strategy, act: Act) {
        self.strategy = strategy;
        self.meta.touched(act);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // SHARED KERNEL: attribution is a boundary type.
    use crate::domain::attribution::AttributionContext;
    use crate::domain::forge::model::table::{Row, Table};
    use crate::domain::forge::model::value::{Content, EntryId, PursuitId};
    use chrono::{DateTime, TimeZone, Utc};
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), &AttributionContext::owner_surface())
    }

    fn root() -> Line {
        Line::open(Name::new(Line::ROOT).unwrap(), act(0))
    }

    #[test]
    fn a_new_line_carries_nothing_and_still_has_a_head() {
        let line = root();

        assert_eq!(line.head(), line.history().genesis().id());
        assert!(line.states().is_empty());
        assert_eq!(line.strategy(), Strategy::Auto);
    }

    #[test]
    fn landing_puts_an_entry_on_the_line() {
        let mut line = root();
        let entry = EntryId::new();
        let content = Content::from_uuid(Uuid::now_v7());
        let point = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            Table::one(entry, Row::added(content, Name::new("key visual").unwrap())),
            act(1),
        );

        line.land(point).unwrap();

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
    fn landing_does_not_touch_the_description() {
        let mut line = root();
        let point = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            Table::one(
                EntryId::new(),
                Row::added(
                    Content::from_uuid(Uuid::now_v7()),
                    Name::new("key visual").unwrap(),
                ),
            ),
            act(3),
        );

        line.land(point).unwrap();

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
            Table::one(
                EntryId::new(),
                Row::added(Content::from_uuid(Uuid::now_v7()), taken.clone()),
            ),
            act(1),
        );
        line.land(first).unwrap();

        let twin = ChangePoint::new(
            line.head(),
            PursuitId::new(),
            Table::one(
                EntryId::new(),
                Row::added(Content::from_uuid(Uuid::now_v7()), taken.clone()),
            ),
            act(2),
        );
        let refused = line.land(twin);

        assert_eq!(refused, Err(ForgeError::NameTaken(taken)));
        assert_eq!(line.history().landed().len(), 1);
    }

    #[test]
    fn a_line_turns_automatic_divergence_off() {
        let mut line = root();

        line.set_strategy(Strategy::NoAuto, act(2));

        assert_eq!(line.strategy(), Strategy::NoAuto);
        assert_eq!(line.meta().updated().at(), at(2));
    }
}
