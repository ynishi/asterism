//! Putting work on a line — the one place both logs are read at once.
//!
//! ```text
//!   what the work says  ──normalise──▶  what would change
//!         (rows)              ▲              (write set)
//!                             │                   │
//!                    the line's head              │
//!                                                 ▼
//!                                           collisions?
//! ```
//!
//! Everywhere else in the model, one log is read at a time. Here they
//! meet, and the order matters: what a proposal *means* is only
//! decided against the line it would change.
//!
//! # Normalising is what makes work survivable
//!
//! [`normalise`] drops what the line already says. An axis whose value
//! matches the head is not a change; an arrival for something already
//! on the line is a content-and-name change rather than an arrival; a
//! removal of something already off the line has nothing left to do.
//!
//! Without it, two people doing the same thing would leave the second
//! permanently unable to change anything — and worse, the second one's
//! unchanged rows would write themselves over a line that had moved
//! on, undoing whatever happened in between.
//!
//! # An empty write set is an outcome, not a failure
//!
//! When everything falls away, the work has nothing left to say —
//! usually because somebody else said it first. There is nothing to
//! record, because a change point carrying nothing is a line advancing
//! to say nothing. What was attempted stays readable either way.
//!
//! # Collision is derived, never stored
//!
//! A collision is an axis this work would write that the line has
//! already moved since the work was cut — **unless this work looked at
//! the change point that moved it**. Looking is what a pass records by
//! taking a change point in, and writing the axis afterwards is what
//! settles the question.
//!
//! That is why the base does not move when work takes something in. If
//! it did, the window this reads over would shrink every time work
//! looked, and a change that was never reconciled would come out
//! clean.
//!
//! Nothing here stores a collision. It is computed from the two logs
//! whenever it is asked, so it cannot go stale and there is no flag
//! for anybody to forget to clear.
//!
//! # This module reports; it does not settle
//!
//! What to do about a collision is a decision — take the line's side,
//! take the work's, or put both on the line under different names —
//! and it belongs to whoever set the line up rather than to the code
//! that noticed.

use std::collections::BTreeSet;

use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::{ChangePoint, History};
use crate::domain::forge::model::op::Rows;
use crate::domain::forge::model::table::{EntryStates, Row};
use crate::domain::forge::model::value::{ChangePointId, EntryId, Existence};

/// One thing a row can move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    /// Whether the entry is on the line.
    Existence,
    /// What it holds.
    Content,
    /// What it answers to.
    Name,
}

/// What a change would actually move.
pub type WriteSet = BTreeSet<(EntryId, Axis)>;

/// An axis the line moved first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Collision {
    /// The entry both moved.
    pub entry: EntryId,
    /// The axis both moved.
    pub axis: Axis,
    /// The change point that moved it, which this work has not seen.
    pub moved_in: ChangePointId,
}

/// Drops everything the line already says.
///
/// What survives is what would move the line, stated as the smallest
/// rows that would move it.
pub fn normalise(rows: Rows, head: &EntryStates) -> Rows {
    let mut kept = Rows::new();
    for (entry, row) in rows {
        let standing = head.get(&entry);
        let alive = standing.map(|state| state.alive).unwrap_or(false);

        if row.existence() == Some(Existence::Absent) {
            // Taking off something already off has nothing left to do.
            if alive {
                kept.insert(entry, row);
            }
            continue;
        }

        // An arrival for something already on the line is not an
        // arrival; what remains is whatever it moves.
        let existence = match row.existence() {
            Some(Existence::Present) if !alive => Some(Existence::Present),
            _ => None,
        };
        let content = row
            .content()
            .filter(|proposed| standing.and_then(|state| state.content) != Some(*proposed));
        let name = row
            .name()
            .filter(|proposed| standing.and_then(|state| state.name.as_ref()) != Some(*proposed))
            .cloned();

        // `Row::new` refuses a row with no axes left, which is exactly
        // the row whose every wish the line already grants.
        if let Ok(row) = Row::new(existence, content, name) {
            kept.insert(entry, row);
        }
    }
    kept
}

/// What a normalised set of rows would change, axis by axis.
pub fn write_set(rows: &Rows) -> WriteSet {
    let mut set = WriteSet::new();
    for (entry, row) in rows {
        for axis in moved(row) {
            set.insert((*entry, axis));
        }
    }
    set
}

/// The axes this work would move that the line moved first, and that
/// the work has not looked at.
///
/// `base` is the change point the work was cut from. `seen` is every
/// change point the work has taken in.
///
/// Fails with [`UnknownBase`](ForgeError::UnknownBase) if the base is
/// not a node of this history — work cut from one line cannot be
/// judged against another, and answering "no collisions" would be the
/// worst possible way to say so.
pub fn collisions(
    write_set: &WriteSet,
    history: &History,
    base: ChangePointId,
    seen: &BTreeSet<ChangePointId>,
) -> Result<Vec<Collision>, ForgeError> {
    let mut found = Vec::new();
    for point in since(history, base)? {
        if seen.contains(&point.id()) {
            continue;
        }
        for (entry, row) in point.table().rows() {
            for axis in moved(row) {
                if write_set.contains(&(*entry, axis)) {
                    found.push(Collision {
                        entry: *entry,
                        axis,
                        moved_in: point.id(),
                    });
                }
            }
        }
    }
    Ok(found)
}

/// Every change recorded after `base`, in the chain's order.
fn since(history: &History, base: ChangePointId) -> Result<&[ChangePoint], ForgeError> {
    if history.genesis().id() == base {
        return Ok(history.changes());
    }
    history
        .changes()
        .iter()
        .position(|point| point.id() == base)
        .map(|at| &history.changes()[at + 1..])
        .ok_or(ForgeError::UnknownBase)
}

/// The axes one row moves.
fn moved(row: &Row) -> Vec<Axis> {
    let mut axes = Vec::new();
    if row.existence().is_some() {
        axes.push(Axis::Existence);
    }
    if row.content().is_some() {
        axes.push(Axis::Content);
    }
    if row.name().is_some() {
        axes.push(Axis::Name);
    }
    axes
}

#[cfg(test)]
mod tests {
    use super::*;
    // SHARED KERNEL: attribution is a boundary type.
    use crate::domain::attribution::AttributionContext;
    use crate::domain::forge::model::act::Act;
    use crate::domain::forge::model::op::{Op, fold};
    use crate::domain::forge::model::table::Table;
    use crate::domain::forge::model::value::{Content, Name, PursuitId};
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

    /// Records one table on a history and answers with the node it
    /// made.
    fn record(history: &mut History, table: Table, minute: u32) -> ChangePointId {
        let point = ChangePoint::new(history.head(), PursuitId::new(), table, act(minute));
        let id = point.id();
        history.record(point).unwrap();
        id
    }

    #[test]
    fn work_the_line_already_agrees_with_falls_away_entirely() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let held = match arrival.kind() {
            crate::domain::forge::model::op::OpKind::Add { content, .. } => *content,
            _ => unreachable!(),
        };
        record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );

        // Somebody else proposes exactly what is already there.
        let same = fold(&[Op::add_to(entry, held, name("key visual"))]);
        let left = normalise(same, &history.states());

        assert!(left.is_empty());
        assert!(write_set(&left).is_empty());
    }

    /// The failure this whole step exists to prevent: a row that
    /// matches what the work was cut from, written over a line that
    /// has moved, would undo the move.
    #[test]
    fn a_value_the_line_has_moved_past_is_not_written_back() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let original = match arrival.kind() {
            crate::domain::forge::model::op::OpKind::Add { content, .. } => *content,
            _ => unreachable!(),
        };
        record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );
        let newer = content();
        record(&mut history, Table::one(entry, Row::replaced(newer)), 2);

        // Work that only renames, carrying the content it was cut with.
        let says = fold(&[
            Op::replace(entry, original),
            Op::rename(entry, name("hero")),
        ]);
        let left = normalise(says, &history.states());

        assert_eq!(left[&entry].content(), Some(original), "it still says it");
        let set = write_set(&left);
        assert!(set.contains(&(entry, Axis::Content)));
        assert!(set.contains(&(entry, Axis::Name)));
    }

    #[test]
    fn an_arrival_for_something_already_on_the_line_keeps_only_what_it_moves() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );

        let held = content();
        let says = fold(&[Op::add_to(entry, held, name("hero"))]);
        let left = normalise(says, &history.states());

        assert_eq!(
            left[&entry],
            Row::new(None, Some(held), Some(name("hero"))).unwrap()
        );
    }

    #[test]
    fn taking_off_something_already_off_falls_away() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );
        record(&mut history, Table::one(entry, Row::removed()), 2);

        let left = normalise(fold(&[Op::remove(entry)]), &history.states());

        assert!(left.is_empty());
    }

    #[test]
    fn an_axis_the_line_moved_since_the_work_was_cut_collides() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let base = record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );
        let theirs = record(&mut history, Table::one(entry, Row::replaced(content())), 2);

        let mine = normalise(fold(&[Op::replace(entry, content())]), &history.states());
        let found = collisions(&write_set(&mine), &history, base, &BTreeSet::new()).unwrap();

        assert_eq!(
            found,
            vec![Collision {
                entry,
                axis: Axis::Content,
                moved_in: theirs,
            }]
        );
    }

    /// Looking at what changed and then writing the axis anyway is what
    /// settling a collision *is*.
    #[test]
    fn an_axis_written_after_looking_does_not_collide() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let base = record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );
        let theirs = record(&mut history, Table::one(entry, Row::replaced(content())), 2);

        let mine = normalise(fold(&[Op::replace(entry, content())]), &history.states());
        let found =
            collisions(&write_set(&mine), &history, base, &BTreeSet::from([theirs])).unwrap();

        assert!(found.is_empty());
    }

    /// Different axes of one entry are not a disagreement.
    #[test]
    fn work_on_another_axis_does_not_collide() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let base = record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );
        record(&mut history, Table::one(entry, Row::replaced(content())), 2);

        let mine = normalise(fold(&[Op::rename(entry, name("hero"))]), &history.states());
        let found = collisions(&write_set(&mine), &history, base, &BTreeSet::new()).unwrap();

        assert!(found.is_empty());
    }

    /// What changed before the work was cut is what the work started
    /// from, and is not something it collided with.
    #[test]
    fn what_changed_before_the_work_was_cut_is_not_a_collision() {
        let mut history = History::begin(act(0));
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );
        let base = record(&mut history, Table::one(entry, Row::replaced(content())), 2);

        let mine = normalise(fold(&[Op::replace(entry, content())]), &history.states());
        let found = collisions(&write_set(&mine), &history, base, &BTreeSet::new()).unwrap();

        assert!(found.is_empty());
    }

    /// Work cut from an untouched line has the genesis as its base,
    /// and everything since is fair game.
    #[test]
    fn work_cut_at_the_genesis_sees_every_change_since() {
        let mut history = History::begin(act(0));
        let base = history.genesis().id();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let theirs = record(
            &mut history,
            Table::of(fold(std::slice::from_ref(&arrival))).unwrap(),
            1,
        );

        let mine = normalise(fold(&[Op::rename(entry, name("hero"))]), &history.states());
        let found = collisions(&write_set(&mine), &history, base, &BTreeSet::new()).unwrap();

        assert_eq!(found[0].moved_in, theirs);
        assert_eq!(found[0].axis, Axis::Name);
    }

    #[test]
    fn work_cut_from_another_line_is_refused_rather_than_cleared() {
        let history = History::begin(act(0));

        let answered = collisions(
            &WriteSet::new(),
            &history,
            ChangePointId::new(),
            &BTreeSet::new(),
        );

        assert_eq!(answered.unwrap_err(), ForgeError::UnknownBase);
    }
}
