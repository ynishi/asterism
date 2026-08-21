//! What work writes, and what it folds into.
//!
//! ```text
//!   Op : Entry ──▶ Add | Replace | Rename | Remove
//!
//!   [Op] ──fold──▶ Entry ──▶ Row
//! ```
//!
//! Four verbs, and every one of them names an entry. That is the whole
//! vocabulary of work: there is nothing to say about a line that is
//! not about one of the things on it.
//!
//! # An add mints its own entry
//!
//! [`Op::add`] mints the [`EntryId`] on the spot, before any change
//! has been recorded. Work refers to what it proposed by that id, so a
//! later round can rename or replace something no history has heard of
//! — and when it does reach the line, it reaches it as the thing that
//! was being talked about all along rather than as a new arrival.
//!
//! Nothing has to agree to a mint. An id is a surrogate, so there is
//! no shared counter to contend for, and proposing costs nothing that
//! has to be taken back if the work is abandoned.
//!
//! # Nothing here asks whether two contents are the same
//!
//! An add is taken at its word. Somebody meant to put something on the
//! line, so an entry arrives — and if what it holds is byte-identical
//! to what another entry already holds, that is still two entries and
//! both are on the line. No add is refused, folded into an existing
//! entry, or turned into a change to one, on the grounds of what it
//! points at.
//!
//! This is a boundary rather than an omission. Whether two things are
//! *the same thing* is a question about bytes, and the layer that
//! holds them answers it — with a fingerprint over the original, an
//! `identical_to` edge recording the fact, and a queue of the
//! questions that fact raises. The forge sees the outcome and nothing
//! else: when that layer decides two things are one, the same
//! [`Content`] comes back, and sameness shows up here as two rows
//! agreeing rather than as a judgement the forge made.
//!
//! Running the question this way round is what keeps the two kinds of
//! statement apart. A forge that folded adds by content would be
//! deciding what somebody's selection meant on the strength of a
//! digest, and the record of what was chosen out of what would quietly
//! become a record of what survived deduplication.
//!
//! Where the forge could eventually use that answer — showing whoever
//! is adding that the line already holds these bytes, or letting a
//! collision settle onto one entry instead of two — it uses it. It
//! does not compute it.
//!
//! # The fold reads work and nothing else
//!
//! [`fold`] takes operations and returns rows. **It does not take the
//! line.** Given the head it would produce a different answer at
//! different moments, and "what this work asks for" would stop being a
//! property of the work — the same operations would mean one thing
//! before somebody else changed the line and another thing after.
//!
//! Comparing that answer against a line is a later step, and a
//! separate one. Here the rule is only: per axis, the last operation
//! to write it wins.
//!
//! # Existence absorbs, or stands alone
//!
//! An entry being put on the line takes the winning content and name
//! with it, because that is one arrival rather than three statements.
//! An entry being taken off keeps nothing else: renaming something on
//! its way off says nothing anybody can read back.
//!
//! An entry no operation puts anywhere keeps whatever axes were
//! written — replacing the content of something already on the line
//! says nothing about its existence, and should not.

use std::collections::BTreeMap;

use crate::domain::forge::model::table::Row;
use crate::domain::forge::model::value::{Content, EntryId, Name};

/// One thing work asks for about one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    entry: EntryId,
    kind: OpKind,
}

/// The four verbs.
///
/// An enum with the payload on it rather than a kind beside a bag of
/// options, so an add without a name or a removal carrying content
/// cannot be written down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    /// Put an entry on the line, named and filled.
    Add {
        /// What it holds.
        content: Content,
        /// What it answers to.
        name: Name,
    },
    /// Move what an entry holds. Identity and name stay.
    Replace {
        /// What it holds from here on.
        content: Content,
    },
    /// Move what an entry answers to. Identity and content stay.
    Rename {
        /// What it answers to from here on.
        name: Name,
    },
    /// Take an entry off the line. What it held stays exactly as live
    /// as it was — this is a statement about the line.
    Remove,
}

impl Op {
    /// Proposes a new entry, minting the id it will be known by.
    pub fn add(content: Content, name: Name) -> Self {
        Self {
            entry: EntryId::new(),
            kind: OpKind::Add { content, name },
        }
    }

    /// Puts an entry that already has an id back on the line.
    ///
    /// The same verb as [`add`](Self::add), aimed rather than minting:
    /// undoing a removal is adding the entry that was removed, and it
    /// has to be *that* entry or the line gains a second thing where a
    /// person meant to bring one back. Reusing a name would not say
    /// it — an entry is what it is by id, and names move.
    pub fn add_to(entry: EntryId, content: Content, name: Name) -> Self {
        Self {
            entry,
            kind: OpKind::Add { content, name },
        }
    }

    /// Moves what an entry holds.
    pub fn replace(entry: EntryId, content: Content) -> Self {
        Self {
            entry,
            kind: OpKind::Replace { content },
        }
    }

    /// Moves what an entry answers to.
    pub fn rename(entry: EntryId, name: Name) -> Self {
        Self {
            entry,
            kind: OpKind::Rename { name },
        }
    }

    /// Takes an entry off the line.
    pub fn remove(entry: EntryId) -> Self {
        Self {
            entry,
            kind: OpKind::Remove,
        }
    }

    /// Which entry this is about.
    pub fn entry(&self) -> EntryId {
        self.entry
    }

    /// What it says.
    pub fn kind(&self) -> &OpKind {
        &self.kind
    }
}

/// What the winning operations said, per entry.
///
/// The rows a fold produces, before anything has been compared to a
/// line. Not a [`Table`](crate::domain::forge::model::table::Table),
/// because a table is what a change point carries and carrying nothing
/// is refused there — work that says nothing yet is ordinary.
pub type Rows = BTreeMap<EntryId, Row>;

/// Folds operations into rows: per axis, the last one to write it
/// wins.
///
/// Order is the order they are given, which is the order they were
/// written — within a node and across the nodes of one work log.
pub fn fold(ops: &[Op]) -> Rows {
    let mut existence: BTreeMap<EntryId, bool> = BTreeMap::new();
    let mut content: BTreeMap<EntryId, Content> = BTreeMap::new();
    let mut name: BTreeMap<EntryId, Name> = BTreeMap::new();

    for op in ops {
        match op.kind() {
            OpKind::Add {
                content: held,
                name: called,
            } => {
                existence.insert(op.entry(), true);
                content.insert(op.entry(), *held);
                name.insert(op.entry(), called.clone());
            }
            OpKind::Replace { content: held } => {
                content.insert(op.entry(), *held);
            }
            OpKind::Rename { name: called } => {
                name.insert(op.entry(), called.clone());
            }
            OpKind::Remove => {
                existence.insert(op.entry(), false);
            }
        }
    }

    let mut rows = Rows::new();
    for op in ops {
        let entry = op.entry();
        if rows.contains_key(&entry) {
            continue;
        }
        let row = match existence.get(&entry) {
            // Taken off: the row says that and nothing else.
            Some(false) => Row::removed(),
            // Put on: one arrival, carrying the axes that won.
            Some(true) => Row::added(
                content[&entry],
                name.get(&entry).expect("an add writes a name").clone(),
            ),
            // Nothing said about where it is; whatever was written
            // stands on its own.
            None => Row::new(
                None,
                content.get(&entry).copied(),
                name.get(&entry).cloned(),
            )
            .expect("an entry with an operation has an axis"),
        };
        rows.insert(entry, row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::table::Table;
    use crate::domain::forge::model::table::states;
    use uuid::Uuid;

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn name(value: &str) -> Name {
        Name::new(value).unwrap()
    }

    #[test]
    fn an_add_mints_an_entry_and_carries_both_axes() {
        let added = Op::add(content(), name("key visual"));

        let rows = fold(std::slice::from_ref(&added));

        assert_eq!(
            rows.get(&added.entry()),
            Some(&Row::added(
                match added.kind() {
                    OpKind::Add { content, .. } => *content,
                    _ => unreachable!(),
                },
                name("key visual")
            ))
        );
    }

    #[test]
    fn two_adds_are_two_entries() {
        let one = Op::add(content(), name("key visual"));
        let other = Op::add(content(), name("alternate"));

        assert_ne!(one.entry(), other.entry());
        assert_eq!(fold(&[one, other]).len(), 2);
    }

    #[test]
    fn the_last_operation_on_an_axis_wins() {
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();
        let last = content();

        let rows = fold(&[
            added,
            Op::replace(entry, content()),
            Op::replace(entry, last),
            Op::rename(entry, name("hero")),
        ]);

        assert_eq!(rows[&entry], Row::added(last, name("hero")));
    }

    #[test]
    fn taking_an_entry_off_leaves_the_row_saying_only_that() {
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();

        let rows = fold(&[added, Op::rename(entry, name("hero")), Op::remove(entry)]);

        assert_eq!(rows[&entry], Row::removed());
    }

    /// An entry put back after being taken off is an arrival again,
    /// carrying the axes that won.
    #[test]
    fn an_entry_put_back_arrives_with_what_was_written() {
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();
        let returned = content();

        let rows = fold(&[
            added,
            Op::remove(entry),
            Op::replace(entry, returned),
            Op::add_to(entry, returned, name("key visual")),
        ]);

        assert_eq!(rows[&entry], Row::added(returned, name("key visual")));
    }

    /// Work that only replaces says nothing about existence — the
    /// entry it names is already somewhere, and the fold does not
    /// guess where.
    #[test]
    fn replacing_alone_says_nothing_about_where_an_entry_is() {
        let entry = EntryId::new();
        let held = content();

        let rows = fold(&[Op::replace(entry, held)]);

        assert_eq!(rows[&entry], Row::replaced(held));
    }

    #[test]
    fn work_that_has_said_nothing_folds_to_nothing() {
        assert!(fold(&[]).is_empty());
    }

    /// The fold is a property of the operations. Nothing else is an
    /// input, so the answer cannot move under a caller.
    #[test]
    fn the_same_operations_fold_the_same_way() {
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();
        let ops = vec![added, Op::rename(entry, name("hero"))];

        assert_eq!(fold(&ops), fold(&ops));
    }

    /// What work folds to is the shape a change point carries, so the
    /// rows go straight into a table and read back the same way.
    #[test]
    fn the_rows_are_what_a_change_point_carries() {
        let added = Op::add(content(), name("key visual"));
        let entry = added.entry();

        let table = Table::of(fold(std::slice::from_ref(&added))).unwrap();

        assert!(states([&table])[&entry].alive);
    }
}
