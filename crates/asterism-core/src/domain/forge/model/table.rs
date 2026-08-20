//! What a change point carries, and what folding a sequence of them
//! answers.
//!
//! ```text
//!   Table : Entry ──▶ Row
//!                      ├ existence?   on the line / off it
//!                      ├ content?     what it holds
//!                      └ name?        what it answers to
//! ```
//!
//! # Three axes rather than a verb
//!
//! A person writes verbs — add this, rename that — but by the time a
//! table reaches a line, the question is per axis, and a verb set cannot
//! spell "says nothing about the name" except as another verb. So a
//! row states only the axes it moves, and [`Row::added`],
//! [`Row::replaced`], [`Row::renamed`] and [`Row::removed`] are the
//! four verbs written as the rows they mean.
//!
//! The gain is that disagreement becomes visible without comparison:
//! two change points that touched different axes of one entry did not
//! disagree, and nothing has to work that out after the fact.
//!
//! # Two shapes a row must not take
//!
//! [`Row::new`] refuses both, because neither can be read back:
//!
//! - **A row that states no axis** puts an entry in a table that never
//!   moved it, which is exactly the claim a reader takes from finding
//!   it there.
//! - **A row that takes an entry off while naming or filling it** gives
//!   one state two spellings. Removing and renaming across two change
//!   points reaches the same place, and the fold cannot tell the two
//!   apart afterwards — so allowing both would stop a table being a
//!   description of what its change point did.
//!
//! A row that states existence alone is legal, and is what a revival
//! looks like once the axes already matching the head fall away.
//! Which rows make sense *against a particular head* is a different
//! question, and it belongs to the step that judges a table before it
//! is recorded rather than to the row.
//!
//! An empty [`Table`] is refused for the same kind of reason: a change
//! point carrying nothing is a line advancing to say nothing, and
//! there is no reading of the history under which that means anything.
//!
//! # The fold
//!
//! [`states`] takes tables **in the chain's order** and lets later
//! ones win per axis, on the axes they state. The three axes derive
//! independently, which is why taking an entry off does not erase what
//! it was called: a name that is off the line is still readable, and
//! merely available again.
//!
//! An entry appears in the result as soon as any table names it, on
//! the line or off it — "was taken off" and "was never here" are
//! different answers, and a caller can tell them apart.

use std::collections::BTreeMap;

use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::value::{Content, EntryId, Existence, Name};

/// One entry's line in a table: what is said about it, on the axes
/// anything is said about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    existence: Option<Existence>,
    content: Option<Content>,
    name: Option<Name>,
}

impl Row {
    /// Builds a row, refusing the two shapes that could not be read
    /// back.
    ///
    /// A row that says nothing at all is one: it would put an entry in
    /// a table that never moved it, which is exactly the claim a
    /// reader would take from finding it there. A row that takes an
    /// entry off while also naming or filling it is the other — the
    /// same state is reachable by removing and renaming across two
    /// change points, so allowing both spellings would leave a table
    /// no longer describing what its change point did.
    pub fn new(
        existence: Option<Existence>,
        content: Option<Content>,
        name: Option<Name>,
    ) -> Result<Self, ForgeError> {
        if existence.is_none() && content.is_none() && name.is_none() {
            return Err(ForgeError::EmptyRow);
        }
        if existence == Some(Existence::Absent) && (content.is_some() || name.is_some()) {
            return Err(ForgeError::RemovalMovesAnotherAxis);
        }
        Ok(Self {
            existence,
            content,
            name,
        })
    }

    /// Puts an entry on the line, named and filled.
    pub fn added(content: Content, name: Name) -> Self {
        Self {
            existence: Some(Existence::Present),
            content: Some(content),
            name: Some(name),
        }
    }

    /// Moves what an entry holds.
    pub fn replaced(content: Content) -> Self {
        Self {
            existence: None,
            content: Some(content),
            name: None,
        }
    }

    /// Moves what an entry answers to.
    pub fn renamed(name: Name) -> Self {
        Self {
            existence: None,
            content: None,
            name: Some(name),
        }
    }

    /// Takes an entry off the line.
    pub fn removed() -> Self {
        Self {
            existence: Some(Existence::Absent),
            content: None,
            name: None,
        }
    }

    /// On or off the line from here on, or `None` for "does not say".
    pub fn existence(&self) -> Option<Existence> {
        self.existence
    }

    /// What the entry holds from here on, or `None`.
    pub fn content(&self) -> Option<Content> {
        self.content
    }

    /// What it answers to from here on, or `None`.
    pub fn name(&self) -> Option<&Name> {
        self.name.as_ref()
    }
}

/// What one change point carries: one row per entry it moves.
///
/// Never empty. A table with no rows is a change point that moved
/// nothing — a line advancing to say nothing — and there is no reading
/// of the history under which that means anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table(BTreeMap<EntryId, Row>);

impl Table {
    /// Builds a table, refusing an empty one.
    pub fn of(rows: BTreeMap<EntryId, Row>) -> Result<Self, ForgeError> {
        if rows.is_empty() {
            return Err(ForgeError::EmptyTable);
        }
        Ok(Self(rows))
    }

    /// Builds a table from one row, which is the common case.
    pub fn one(entry: EntryId, row: Row) -> Self {
        Self(BTreeMap::from([(entry, row)]))
    }

    /// The rows, by entry.
    pub fn rows(&self) -> &BTreeMap<EntryId, Row> {
        &self.0
    }
}

/// One entry's position: whether it is on the line, and what it
/// answers to and holds.
///
/// The three axes derive independently, so taking an entry off does
/// not erase what it was called. A name that is off the line is still
/// readable — it is merely available again, which is a question about
/// what a new entry may be called rather than about what this one was.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryState {
    /// Whether the latest existence axis leaves it on the line. An
    /// entry no table has ever spoken about is not on it.
    pub alive: bool,
    /// The latest name stated, if any table stated one.
    pub name: Option<Name>,
    /// The latest content stated, if any table stated one.
    pub content: Option<Content>,
}

/// Every entry a line has heard of, and where each one stands.
pub type EntryStates = BTreeMap<EntryId, EntryState>;

/// Folds tables into what is on the line.
///
/// **The tables must arrive in the chain's order** — the genesis end
/// first. A change point's place is which node took it as a parent,
/// not its clock reading, so the chain is the only thing that answers
/// "which came first"; reading a time instead would be a second answer
/// to a question already answered, and the two would disagree the
/// first time a clock stepped backwards.
///
/// Later tables win per axis, and only on the axes they state. An
/// entry appears as soon as any table names it, on or off the line —
/// "was taken off" and "was never here" are different answers, and the
/// caller can tell them apart.
pub fn states<'a, I>(tables: I) -> EntryStates
where
    I: IntoIterator<Item = &'a Table>,
{
    let mut states: EntryStates = BTreeMap::new();
    for table in tables {
        for (entry, row) in table.rows() {
            let state = states.entry(*entry).or_default();
            if let Some(existence) = row.existence() {
                state.alive = existence == Existence::Present;
            }
            if let Some(content) = row.content() {
                state.content = Some(content);
            }
            if let Some(name) = row.name() {
                state.name = Some(name.clone());
            }
        }
    }
    states
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn entry() -> EntryId {
        EntryId::new()
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn name(value: &str) -> Name {
        Name::new(value).unwrap()
    }

    #[test]
    fn an_added_entry_is_on_the_line_named_and_filled() {
        let e = entry();
        let c = content();
        let table = Table::one(e, Row::added(c, name("key visual")));

        let states = states([&table]);

        assert_eq!(
            states.get(&e),
            Some(&EntryState {
                alive: true,
                name: Some(name("key visual")),
                content: Some(c),
            })
        );
    }

    #[test]
    fn a_later_table_wins_only_on_the_axes_it_states() {
        let e = entry();
        let replaced = content();
        let tables = vec![
            Table::one(e, Row::added(content(), name("key visual"))),
            Table::one(e, Row::replaced(replaced)),
            Table::one(e, Row::renamed(name("hero"))),
        ];

        let states = states(&tables);

        assert_eq!(
            states.get(&e),
            Some(&EntryState {
                alive: true,
                name: Some(name("hero")),
                content: Some(replaced),
            })
        );
    }

    #[test]
    fn taking_an_entry_off_leaves_its_name_and_content_readable() {
        let e = entry();
        let c = content();
        let tables = vec![
            Table::one(e, Row::added(c, name("key visual"))),
            Table::one(e, Row::removed()),
        ];

        let states = states(&tables);

        assert_eq!(
            states.get(&e),
            Some(&EntryState {
                alive: false,
                name: Some(name("key visual")),
                content: Some(c),
            })
        );
    }

    #[test]
    fn an_entry_can_come_back_under_the_name_it_had() {
        let e = entry();
        let returned = content();
        let tables = vec![
            Table::one(e, Row::added(content(), name("key visual"))),
            Table::one(e, Row::removed()),
            Table::one(e, Row::added(returned, name("key visual"))),
        ];

        let states = states(&tables);

        assert_eq!(
            states.get(&e),
            Some(&EntryState {
                alive: true,
                name: Some(name("key visual")),
                content: Some(returned),
            })
        );
    }

    #[test]
    fn entries_derive_independently_of_each_other() {
        let kept = entry();
        let dropped = entry();
        let kept_content = content();
        let tables = vec![
            Table::of(BTreeMap::from([
                (kept, Row::added(kept_content, name("key visual"))),
                (dropped, Row::added(content(), name("alternate"))),
            ]))
            .unwrap(),
            Table::one(dropped, Row::removed()),
        ];

        let states = states(&tables);

        assert!(states.get(&kept).unwrap().alive);
        assert!(!states.get(&dropped).unwrap().alive);
        assert_eq!(states.get(&kept).unwrap().content, Some(kept_content));
    }

    #[test]
    fn nothing_is_on_a_line_no_table_has_spoken_about() {
        assert!(states([]).is_empty());
    }

    /// An entry a table only fills or names is known without being on
    /// the line. Existence is its own axis, and nothing else implies
    /// it — which is what lets work refer to an entry before anything
    /// has put it anywhere.
    #[test]
    fn an_entry_named_without_being_added_is_known_and_not_on_the_line() {
        let e = entry();
        let c = content();
        let table = Table::one(e, Row::replaced(c));

        let states = states([&table]);

        assert_eq!(
            states.get(&e),
            Some(&EntryState {
                alive: false,
                name: None,
                content: Some(c),
            })
        );
    }

    /// The fold is a function of the sequence and nothing else: same
    /// tables, same answer, however many times it is asked.
    #[test]
    fn the_same_tables_fold_to_the_same_answer() {
        let e = entry();
        let tables = vec![
            Table::one(e, Row::added(content(), name("key visual"))),
            Table::one(e, Row::renamed(name("hero"))),
        ];

        assert_eq!(states(&tables), states(&tables));
    }

    #[test]
    fn a_row_that_states_no_axis_is_refused() {
        assert!(Row::new(None, None, None).is_err());
    }

    #[test]
    fn a_removing_row_carrying_a_name_or_a_content_is_refused() {
        assert!(Row::new(Some(Existence::Absent), None, Some(name("hero"))).is_err());
        assert!(Row::new(Some(Existence::Absent), Some(content()), None).is_err());
    }

    #[test]
    fn an_empty_table_is_refused() {
        assert!(Table::of(BTreeMap::new()).is_err());
    }
}
