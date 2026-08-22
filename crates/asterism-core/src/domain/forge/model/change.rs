//! Putting work on a line — the one place both logs are read at once.
//!
//! ```text
//!   what work asks for  ──normalise──▶  what would change
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
//! already moved since the work was cut. That is the whole of the
//! definition — there is no second clause, and nothing about whether
//! anybody looked.
//!
//! **What clears one is the work saying something different.** Not a
//! record of having read, which is a claim about the reader and can be
//! written without changing anything; not a flag, which is a second
//! thing to keep true. When the work's value for an axis becomes what
//! the line already says, normalising drops it, and there is nothing
//! left to collide.
//!
//! So resolving is ordinary work: the operations somebody would have
//! written by hand. If the line moves that axis again afterwards, it
//! collides again, and is resolved again — one at a time, which is
//! what resolving against a moving line means.
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
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::op::Rows;
use crate::domain::forge::model::pursuit::Pursuit;
use crate::domain::forge::model::table::{EntryStates, Row, states};
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

/// The axes this work would move that the line moved after the work
/// was cut from it.
///
/// ```text
///   an axis (entry, axis) collides with a change point C
///   ⟺  the work's request would move it
///       — that is, (entry, axis) survives normalising against the head
///   ∧  C is on the line after the node the work was cut from
///   ∧  C moved that same axis
/// ```
///
/// That is the whole definition. Nothing in it mentions what anybody
/// looked at, because there is nothing to mention: the model has no
/// record of reading, on purpose. What clears a collision is the first
/// clause failing — the work asking for what the line already carries,
/// or asking nothing about that axis at all.
///
/// **So the only way to stop colliding over an axis is to stop
/// requesting it.** Asking for the same value a second time changes
/// nothing, because a fold keeps the last value and not the arguments
/// for it. That has consequences for what a resolution can be, and
/// they are worked out in
/// [`strategies`](crate::domain::forge::strategies).
///
/// Derived from the two logs every time it is asked. Nothing is kept,
/// so nothing can be stale, and there is no flag anybody has to clear.
/// Every change point that moved a contested axis is reported, not
/// only the latest: the question is what the line did, not what is
/// outstanding.
///
/// Fails with [`UnknownBase`](ForgeError::UnknownBase) if the work was
/// cut from a node this history does not have — answering "no
/// collisions" for a line the work has nothing to do with would be the
/// worst available way to say so.
pub fn collisions(line: &Line, work: &Pursuit) -> Result<Vec<Collision>, ForgeError> {
    let writes = write_set(&normalise(work.request(), &line.states()));

    let mut found = Vec::new();
    for point in since(line.history(), work.base())? {
        for (entry, row) in point.table().rows() {
            for axis in moved(row) {
                if writes.contains(&(*entry, axis)) {
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

/// What the line carried at `at`, folded from the chain up to and
/// including it.
///
/// What a rule needs to record where a divergence came from: an entry
/// forked out of a collision starts as the entry was when the work
/// began, and that value is nowhere else by then.
pub fn states_at(history: &History, at: ChangePointId) -> Result<EntryStates, ForgeError> {
    if history.genesis().id() == at {
        return Ok(EntryStates::new());
    }
    let upto = history
        .changes()
        .iter()
        .position(|point| point.id() == at)
        .ok_or(ForgeError::UnknownBase)?;
    Ok(states(
        history.changes()[..=upto].iter().map(ChangePoint::table),
    ))
}

/// Every change recorded after `base`, in the chain's order.
///
/// Public because it answers a question of its own: what has happened
/// to a line since work was cut from it. Reading that is how a reader
/// says what work has yet to look at, and recomputing the walk
/// elsewhere would be a second answer to a question this already has
/// one for.
pub fn since(history: &History, base: ChangePointId) -> Result<&[ChangePoint], ForgeError> {
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
    use crate::domain::forge::model::act::Act;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::closing::close;
    use crate::domain::forge::model::op::{Op, fold};
    use crate::domain::forge::model::pursuit::{Intent, Outcome, Round};
    use crate::domain::forge::model::table::Table;
    use crate::domain::forge::model::value::{
        ActorId, Content, Name, NodeId, PursuitId, StrategyId,
    };
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

    /// Records one table on a history and answers with the node it
    /// made.
    fn record(history: &mut History, table: Table, minute: u32) -> ChangePointId {
        let point = ChangePoint::new(
            history.head(),
            PursuitId::new(),
            NodeId::new(),
            table,
            act(minute),
        );
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

    // The collision tests build a line and work rather than a bare
    // history, because a collision is a statement about the pair and
    // there is no shorter way to say one truthfully.

    fn line() -> Line {
        Line::open(
            name(Line::ROOT),
            StrategyId::new("by-hand").unwrap(),
            act(0),
        )
    }

    fn work_on(line: &Line) -> Pursuit {
        Pursuit::open(line.id(), None, line.head(), Intent::default(), act(1))
    }

    /// Adds a round carrying `ops`.
    fn with_a_round(mut work: Pursuit, ops: Vec<Op>, minute: u32) -> Pursuit {
        work.push(Round::new(work.head(), ops, None, act(minute)).unwrap())
            .unwrap();
        work
    }

    /// Lands `ops` on the line through work of its own, and answers
    /// with the change point it made.
    fn landed(line: &mut Line, ops: Vec<Op>, minute: u32) -> ChangePointId {
        let mut work = with_a_round(work_on(line), ops, minute);
        let closing = close(line, &work, Outcome::Satisfied, None, act(minute)).unwrap();
        let moved = closing.point().expect("that work landed").id();
        closing.apply(line, &mut work).unwrap();
        moved
    }

    #[test]
    fn an_axis_the_line_moved_since_the_work_was_cut_collides() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        landed(&mut line, vec![arrival], 1);

        let work = with_a_round(work_on(&line), vec![Op::replace(entry, content())], 2);
        let theirs = landed(&mut line, vec![Op::replace(entry, content())], 3);

        let found = collisions(&line, &work).unwrap();

        assert_eq!(
            found,
            vec![Collision {
                entry,
                axis: Axis::Content,
                moved_in: theirs,
            }]
        );
    }

    /// What clears a collision is the work saying something else.
    /// Nothing it can record about having read clears one, because
    /// there is nothing it can record about that.
    #[test]
    fn saying_what_the_line_says_clears_the_collision() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        landed(&mut line, vec![arrival], 1);

        let work = with_a_round(work_on(&line), vec![Op::replace(entry, content())], 2);
        landed(&mut line, vec![Op::replace(entry, content())], 3);
        assert_eq!(collisions(&line, &work).unwrap().len(), 1);

        let theirs = line.states()[&entry].content.unwrap();
        let work = with_a_round(work, vec![Op::replace(entry, theirs)], 4);

        assert!(collisions(&line, &work).unwrap().is_empty());
    }

    /// And it is cleared against the line as it was, not for good. A
    /// line that moves the axis again collides again — resolving
    /// against something that keeps moving is done one at a time.
    #[test]
    fn a_later_move_of_the_same_axis_collides_again() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        landed(&mut line, vec![arrival], 1);

        let work = with_a_round(work_on(&line), vec![Op::replace(entry, content())], 2);
        landed(&mut line, vec![Op::replace(entry, content())], 3);
        let theirs = line.states()[&entry].content.unwrap();
        let work = with_a_round(work, vec![Op::replace(entry, theirs)], 4);
        assert!(collisions(&line, &work).unwrap().is_empty());

        let third = landed(&mut line, vec![Op::replace(entry, content())], 5);

        // Every change point that moved the axis is reported, the one
        // just now included: the definition asks what the line did,
        // not what is still outstanding.
        let found = collisions(&line, &work).unwrap();
        assert!(found.iter().any(|collision| collision.moved_in == third));
        assert!(
            found
                .iter()
                .all(|collision| collision.entry == entry && collision.axis == Axis::Content)
        );
    }

    #[test]
    fn asking_twice_gives_the_same_answer() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        landed(&mut line, vec![arrival], 1);
        let work = with_a_round(work_on(&line), vec![Op::replace(entry, content())], 2);
        landed(&mut line, vec![Op::replace(entry, content())], 3);

        assert_eq!(
            collisions(&line, &work).unwrap(),
            collisions(&line, &work).unwrap()
        );
    }

    /// Different axes of one entry are not a disagreement.
    #[test]
    fn work_on_another_axis_does_not_collide() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        landed(&mut line, vec![arrival], 1);

        let work = with_a_round(work_on(&line), vec![Op::rename(entry, name("hero"))], 2);
        landed(&mut line, vec![Op::replace(entry, content())], 3);

        assert!(collisions(&line, &work).unwrap().is_empty());
    }

    /// What changed before the work was cut is what the work started
    /// from, and is not something it collided with.
    #[test]
    fn what_changed_before_the_work_was_cut_is_not_a_collision() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        landed(&mut line, vec![arrival], 1);
        landed(&mut line, vec![Op::replace(entry, content())], 2);

        let work = with_a_round(work_on(&line), vec![Op::replace(entry, content())], 3);

        assert!(collisions(&line, &work).unwrap().is_empty());
    }

    /// What the line carried at a node, for a rule that has to record
    /// where a divergence came from.
    #[test]
    fn states_at_answers_the_line_as_it_was() {
        let mut line = line();
        let arrival = Op::add(content(), name("key visual"));
        let entry = arrival.entry();
        let held = line_content(&line, &arrival);
        let base = landed(&mut line, vec![arrival], 1);
        landed(&mut line, vec![Op::replace(entry, content())], 2);

        let was = states_at(line.history(), base).unwrap();

        assert_eq!(was[&entry].content, Some(held));
        assert_ne!(line.states()[&entry].content, Some(held));
    }

    #[test]
    fn a_node_this_history_does_not_have_is_refused() {
        let line = line();

        let refused = states_at(line.history(), ChangePointId::new());

        assert_eq!(refused.unwrap_err(), ForgeError::UnknownBase);
    }

    /// The content an arrival carries, without going through a fold.
    fn line_content(_line: &Line, arrival: &Op) -> Content {
        match arrival.kind() {
            crate::domain::forge::model::op::OpKind::Add { content, .. } => *content,
            _ => unreachable!("that operation is an arrival"),
        }
    }
}
