//! A line's history: what it carries, and how it got there.
//!
//! ```text
//!   Genesis ──▶ ChangePoint ──▶ ChangePoint ──▶ …
//!      │             │                        ▲ head
//!      │             └ parent / from / table / act
//!      └ act
//! ```
//!
//! The history *is* the line's record. There is no second place that
//! says what is on it — [`History::states`] folds the chain every time
//! it is asked, and a kept copy would be a second thing to hold true.
//!
//! # One chain, never a fork
//!
//! A line exists so that there is one answer to what it carries, and
//! two chains would be two answers. [`History::land`] therefore
//! refuses any change point that does not name the current head as its
//! parent. That refusal is the whole of the rule — not a check
//! somewhere else that callers are asked to remember, and not a
//! resolution step that quietly picks one of the two.
//!
//! What a caller does with the refusal is rebuild against the new
//! head, which is where a collision with the work that got there first
//! becomes visible. That belongs to the act that produces change
//! points, and it is not in this module.
//!
//! # The chain is the order
//!
//! Which change point came first is which one took the other as its
//! parent. Nothing here reads a clock to decide that, and
//! [`ChangePoint::act`] is a record of when something happened rather
//! than an input to ordering — two nodes minted in the same
//! millisecond are still ordered, and a clock that steps backwards
//! changes nothing.
//!
//! # A genesis is not a change point
//!
//! [`Genesis`] carries no table, because there is nothing before it to
//! change, and comes from no work, because no work had a line to be on
//! yet. It is a separate type rather than a [`ChangePoint`] with empty
//! fields: modelled as one type, `parent`, `from` and `table` would
//! all be `Option`, all three would have to be empty together or
//! filled together, and a shape that has to be kept consistent by
//! agreement gets filled halfway.
//!
//! Its purpose is that a line has a head from the moment it exists.
//! Without it, "the head of a line nothing has landed on" would be an
//! absence every reader carries, and the first landing would be a
//! shape of its own.
//!
//! # It only grows
//!
//! Taking an entry off a line is a change point that says so. The
//! record stays, the name and content it had stay readable, and
//! nothing here removes a node — there is no method to, which is the
//! only way to mean it.

use std::collections::BTreeSet;

use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::table::{EntryStates, Table, states};
use crate::domain::forge::model::value::{ChangePointId, Name, PursuitId};

/// The node a line begins at.
///
/// It carries no table, because there is nothing before it to change,
/// and it comes from no work, because no work had a line to be on yet.
/// A separate type rather than a change point with empty fields: two
/// `Option`s that must be empty together, or filled together, are a
/// pair somebody eventually fills halfway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genesis {
    id: ChangePointId,
    act: Act,
}

impl Genesis {
    /// Mints the node a line begins at.
    pub fn new(act: Act) -> Self {
        Self {
            id: ChangePointId::new(),
            act,
        }
    }

    /// Which node.
    pub fn id(&self) -> ChangePointId {
        self.id
    }

    /// When the line began, and who began it.
    pub fn act(&self) -> &Act {
        &self.act
    }
}

/// One move of a line's history.
///
/// Always has a parent and always came out of some work: a change
/// point exists because a pursuit was satisfied, and it lands on
/// whatever the head was at that moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePoint {
    id: ChangePointId,
    parent: ChangePointId,
    from: PursuitId,
    table: Table,
    act: Act,
}

impl ChangePoint {
    /// Mints a change point landing `table` on top of `parent`.
    pub fn new(parent: ChangePointId, from: PursuitId, table: Table, act: Act) -> Self {
        Self {
            id: ChangePointId::new(),
            parent,
            from,
            table,
            act,
        }
    }

    /// Which node.
    pub fn id(&self) -> ChangePointId {
        self.id
    }

    /// The node this landed on.
    pub fn parent(&self) -> ChangePointId {
        self.parent
    }

    /// The work this came out of.
    pub fn from(&self) -> PursuitId {
        self.from
    }

    /// What it moved.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// When it landed, and who landed it.
    pub fn act(&self) -> &Act {
        &self.act
    }
}

/// The chain itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    genesis: Genesis,
    landed: Vec<ChangePoint>,
}

impl History {
    /// Begins a history. A line has one from the moment it exists, so
    /// there is no such thing as a line without a head.
    pub fn begin(act: Act) -> Self {
        Self {
            genesis: Genesis::new(act),
            landed: Vec::new(),
        }
    }

    /// The node a line begins at.
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    /// Everything that has landed, in the chain's order.
    pub fn landed(&self) -> &[ChangePoint] {
        &self.landed
    }

    /// The last node of the chain. Never absent — a history begins
    /// with its genesis, so an untouched line has a head like any
    /// other.
    pub fn head(&self) -> ChangePointId {
        self.landed
            .last()
            .map(ChangePoint::id)
            .unwrap_or_else(|| self.genesis.id())
    }

    /// Appends a change point.
    ///
    /// Two things are refused, and this is the only way a history
    /// changes — there is no counterpart that removes a node.
    ///
    /// **A node that does not land on the head.** It would start a
    /// second chain, and a line with two chains cannot answer what it
    /// carries.
    ///
    /// **A landing that would leave two live entries under one name.**
    /// A name is how a person reaches for what is on the line, so two
    /// of them is a line that cannot be addressed by the only handle
    /// anybody has. The whole table is applied first and the question
    /// is asked of the result, which is what makes a table that frees
    /// a name and takes it in the same breath legal: swapping which
    /// entry answers to "key visual" is one gesture, and judging the
    /// rows one at a time would refuse it for a collision that never
    /// exists.
    ///
    /// The check folds the chain, so it costs what a read costs. That
    /// is the honest price of keeping no second copy of what is on the
    /// line, and it is measured before it is optimised.
    pub fn land(&mut self, point: ChangePoint) -> Result<(), ForgeError> {
        if point.parent() != self.head() {
            return Err(ForgeError::NotOnHead);
        }
        let applied = states(
            self.landed
                .iter()
                .map(ChangePoint::table)
                .chain(std::iter::once(point.table())),
        );
        if let Some(name) = live_name_twice(&applied) {
            return Err(ForgeError::NameTaken(name));
        }
        self.landed.push(point);
        Ok(())
    }

    /// What the line carries: the tables of the chain, folded in
    /// order.
    pub fn states(&self) -> EntryStates {
        states(self.landed.iter().map(ChangePoint::table))
    }
}

/// A name two live entries would answer to, if there is one.
///
/// Entries that are off the line are skipped: a name that is not
/// answering to anything is available, which is the whole reason
/// taking an entry off frees its name. An entry on the line with no
/// name is not a duplicate of anything — whether such a row should
/// exist at all is a question about which rows are legal against a
/// head, and that belongs to the step that judges a table before it
/// lands.
fn live_name_twice(states: &EntryStates) -> Option<Name> {
    let mut seen: BTreeSet<&Name> = BTreeSet::new();
    for state in states.values() {
        if !state.alive {
            continue;
        }
        if let Some(name) = state.name.as_ref()
            && !seen.insert(name)
        {
            return Some(name.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    // SHARED KERNEL: attribution is a boundary type.
    use crate::domain::attribution::AttributionContext;
    use crate::domain::forge::model::table::Row;
    use crate::domain::forge::model::value::{Content, EntryId, Name};
    use chrono::{DateTime, TimeZone, Utc};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), &AttributionContext::owner_surface())
    }

    fn added(entry: EntryId, called: &str) -> Table {
        Table::one(
            entry,
            Row::added(
                Content::from_uuid(Uuid::now_v7()),
                Name::new(called).unwrap(),
            ),
        )
    }

    #[test]
    fn a_new_history_is_its_genesis_and_carries_nothing() {
        let history = History::begin(act(0));

        assert_eq!(history.head(), history.genesis().id());
        assert!(history.landed().is_empty());
        assert!(history.states().is_empty());
    }

    /// Two lines opened by the same act are still two lines. The node
    /// a history begins at is minted, not derived from when or by
    /// whom, so nothing about the opening can make two of them the
    /// same node.
    #[test]
    fn every_history_begins_at_a_node_of_its_own() {
        let opened = act(0);
        let one = History::begin(opened.clone());
        let other = History::begin(opened.clone());

        assert_ne!(one.genesis().id(), other.genesis().id());
        assert_eq!(one.genesis().act(), &opened);
    }

    #[test]
    fn landing_moves_the_head() {
        let mut history = History::begin(act(0));
        let entry = EntryId::new();
        let point = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            added(entry, "key visual"),
            act(1),
        );
        let landed = point.id();

        history.land(point).unwrap();

        assert_eq!(history.head(), landed);
        assert!(history.states().get(&entry).unwrap().alive);
    }

    #[test]
    fn a_change_point_that_names_another_parent_is_refused() {
        let mut history = History::begin(act(0));
        let first = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            added(EntryId::new(), "key visual"),
            act(1),
        );
        let stale_head = history.head();
        history.land(first).unwrap();

        let second = ChangePoint::new(
            stale_head,
            PursuitId::new(),
            added(EntryId::new(), "alternate"),
            act(2),
        );
        let refused = history.land(second);

        assert_eq!(refused, Err(ForgeError::NotOnHead));
        assert_eq!(history.landed().len(), 1);
    }

    #[test]
    fn a_second_live_entry_under_one_name_is_refused() {
        let mut history = History::begin(act(0));
        let first = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            added(EntryId::new(), "key visual"),
            act(1),
        );
        history.land(first).unwrap();

        let twin = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            added(EntryId::new(), "key visual"),
            act(2),
        );
        let refused = history.land(twin);

        assert_eq!(
            refused,
            Err(ForgeError::NameTaken(Name::new("key visual").unwrap()))
        );
        assert_eq!(history.landed().len(), 1);
    }

    /// The whole table is applied before the question is asked, so
    /// handing a name from one entry to another is one gesture rather
    /// than a collision.
    #[test]
    fn one_change_point_can_hand_a_name_from_one_entry_to_another() {
        let mut history = History::begin(act(0));
        let leaving = EntryId::new();
        history
            .land(ChangePoint::new(
                history.head(),
                PursuitId::new(),
                added(leaving, "key visual"),
                act(1),
            ))
            .unwrap();

        let arriving = EntryId::new();
        let swap = Table::of(BTreeMap::from([
            (leaving, Row::removed()),
            (
                arriving,
                Row::added(
                    Content::from_uuid(Uuid::now_v7()),
                    Name::new("key visual").unwrap(),
                ),
            ),
        ]))
        .unwrap();

        history
            .land(ChangePoint::new(
                history.head(),
                PursuitId::new(),
                swap,
                act(2),
            ))
            .unwrap();

        let states = history.states();
        assert!(!states.get(&leaving).unwrap().alive);
        assert!(states.get(&arriving).unwrap().alive);
    }

    /// A name an entry no longer answers to is free, which is what
    /// makes taking something off the line mean anything.
    #[test]
    fn a_name_freed_earlier_can_be_taken_later() {
        let mut history = History::begin(act(0));
        let first = EntryId::new();
        history
            .land(ChangePoint::new(
                history.head(),
                PursuitId::new(),
                added(first, "key visual"),
                act(1),
            ))
            .unwrap();
        history
            .land(ChangePoint::new(
                history.head(),
                PursuitId::new(),
                Table::one(first, Row::removed()),
                act(2),
            ))
            .unwrap();

        let second = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            added(EntryId::new(), "key visual"),
            act(3),
        );

        assert!(history.land(second).is_ok());
    }

    /// The chain orders the history, not the clock. A node minted a
    /// minute *earlier* than the one it lands on still lands after it,
    /// and the fold answers by the chain.
    #[test]
    fn the_chain_orders_the_history_and_not_the_clock() {
        let mut history = History::begin(act(10));
        let entry = EntryId::new();
        let arrival = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            added(entry, "key visual"),
            act(9),
        );
        history.land(arrival).unwrap();
        let departure = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            Table::one(entry, Row::removed()),
            act(8),
        );
        history.land(departure).unwrap();

        assert!(!history.states().get(&entry).unwrap().alive);
    }
}
