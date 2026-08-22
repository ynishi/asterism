//! Building the model back from what a store kept.
//!
//! ```text
//!   stored rows ──► restore::line / restore::pursuit
//!                        │
//!                        ├─ the ids come from outside, here and
//!                        │  nowhere else in the model
//!                        │
//!                        └─ the nodes go back through record / push /
//!                           end — the same refusals a fresh write meets
//! ```
//!
//! # Why this is one module rather than a constructor per type
//!
//! Every other constructor in [`model`](super) mints. A line mints its
//! id and its genesis, work mints its id and its opening node, a pass
//! mints a node id — which means that until this module existed, no
//! value here could be built holding an id somebody else chose. That
//! was the property, and it is the reason the read half of the ports
//! had no implementation but a fake for as long as it did.
//!
//! A rehydration constructor takes every field including the id, so it
//! is the one door through which a stored row can contradict a rule
//! this module holds. Spreading it across the types would put a piece
//! of that door on each of them and leave nothing that could be read
//! as the whole. Here it is one file, and what it may and may not do
//! is stated once.
//!
//! # What it does not do
//!
//! **It does not skip the model's questions.** The nodes are handed
//! back one at a time to [`History::record`], [`Pursuit::push`] and
//! [`Pursuit::end`] — the same calls a fresh write goes through, and
//! the same refusals. A chain whose parents do not line up, a table
//! that would leave two live entries under one name, a pass on a log
//! that has ended: each of those is a stored row that cannot become a
//! value here, and the read fails rather than handing back something
//! the model would not have written.
//!
//! That is the cost as well as the point. The check that reads a name
//! twice folds the chain, so putting back a history of *n* change
//! points costs what *n* reads cost, and a store that keeps something
//! it cannot read back is a store that has to be repaired rather than
//! opened. Both are consequences somebody may want to price
//! differently later; neither is a thing to discover by accident.
//!
//! **It does not put the chain in order.** A change point carries its
//! parent, so [`line()`] takes them in whatever order a store hands them
//! over and walks the links from the genesis. Nothing has to keep a
//! sequence number beside the chain, and a store that got the order
//! wrong is caught by `record` rather than believed.

use std::collections::HashMap;

use crate::domain::forge::model::act::{Act, Meta};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::{ChangePoint, Genesis, History};
use crate::domain::forge::model::line::{Line, Standing};
use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::pursuit::{Close, Intent, Open, Outcome, Pursuit, Round};
use crate::domain::forge::model::table::Table;
use crate::domain::forge::model::value::{
    ChangePointId, LineId, Name, NodeId, PursuitId, StrategyId,
};

/// The two stamps a store kept for a thing's description.
pub fn meta(created: Act, updated: Act) -> Meta {
    Meta::restored(created, updated)
}

/// The node a line began at.
pub fn genesis(id: ChangePointId, act: Act) -> Genesis {
    Genesis::restored(id, act)
}

/// One change point, as it was kept.
///
/// Nothing is checked here. What makes it legal is where it goes —
/// [`line()`] hands it to [`History::record`], which asks a stored node
/// exactly what it asks a new one.
pub fn change_point(
    id: ChangePointId,
    parent: ChangePointId,
    from: PursuitId,
    by: NodeId,
    table: Table,
    act: Act,
) -> ChangePoint {
    ChangePoint::restored(id, parent, from, by, table, act)
}

/// A whole line, from its genesis and the change points on it.
///
/// `points` may arrive in any order: each carries its parent, so the
/// chain is walked from the genesis and recorded in that order.
///
/// # Refusals
///
/// - [`NotOnHead`](ForgeError::NotOnHead) — the points do not form one
///   chain from this genesis. A parent naming a node that is not here,
///   two points claiming the same parent, or a cycle all arrive as
///   this: what is left over is not reachable, and a history is not
///   something to assemble out of the reachable part.
/// - [`NameTaken`](ForgeError::NameTaken) — somewhere along the chain,
///   a change point would leave two live entries under one name. The
///   line could not have been written that way and is not read back
///   that way either.
#[allow(clippy::too_many_arguments)]
pub fn line(
    id: LineId,
    name: Name,
    strategy: StrategyId,
    standing: Standing,
    meta: Meta,
    genesis: Genesis,
    points: Vec<ChangePoint>,
) -> Result<Line, ForgeError> {
    // The chain goes back through `History::record` rather than
    // `Line::record`, which is the one place the two differ on
    // purpose: `Line::record` refuses an archived line, and an
    // archived line's history is exactly what this is putting back.
    // Reading is not moving.
    let mut history = History::restored(genesis);
    for point in chain(history.head(), points)? {
        history.record(point)?;
    }
    Ok(Line::restored(id, name, strategy, standing, history, meta))
}

/// The node work opened at.
pub fn open(id: NodeId, base: ChangePointId, intent: Intent, act: Act) -> Open {
    Open::restored(id, base, intent, act)
}

/// One pass, as it was kept.
///
/// Refuses one carrying no operations, for the reason
/// [`Round::new`](crate::domain::forge::model::pursuit::Round::new)
/// does.
pub fn round(
    id: NodeId,
    parent: NodeId,
    ops: Vec<Op>,
    note: Option<String>,
    act: Act,
) -> Result<Round, ForgeError> {
    Round::restored(id, parent, ops, note, act)
}

/// The node work ended at.
pub fn close(
    id: NodeId,
    parent: NodeId,
    outcome: Outcome,
    note: Option<String>,
    act: Act,
) -> Close {
    Close::restored(id, parent, outcome, note, act)
}

/// A node of a pursuit after the one it opened at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A pass.
    Round(Round),
    /// The ending.
    Close(Close),
}

/// A whole pursuit, from the node it opened at and what followed.
///
/// `nodes` are in the log's own order — a pursuit is a line, not a
/// tree, and a store that kept them shuffled is telling this function
/// something untrue rather than something it should sort out.
///
/// # Refusals
///
/// - [`NotOnHead`](ForgeError::NotOnHead) — a node does not sit on the
///   one before it.
/// - [`AlreadyClosed`](ForgeError::AlreadyClosed) — something follows
///   the ending. Work ends once, and a log that says otherwise is not
///   read back into one that could.
pub fn pursuit(
    id: PursuitId,
    of: LineId,
    parent: Option<PursuitId>,
    meta: Meta,
    open: Open,
    nodes: Vec<Node>,
) -> Result<Pursuit, ForgeError> {
    let mut work = Pursuit::restored(id, of, parent, open, meta);
    for node in nodes {
        match node {
            Node::Round(round) => work.push(round)?,
            Node::Close(close) => work.end(close)?,
        }
    }
    Ok(work)
}

/// Puts change points in the chain's order, starting from `head`.
///
/// Refuses anything that is not one chain covering every point given:
/// a leftover is a node the walk could not reach, which is a store
/// handing over a history with a hole in it or a second branch, and
/// neither is a thing to quietly read the reachable part of.
fn chain(head: ChangePointId, points: Vec<ChangePoint>) -> Result<Vec<ChangePoint>, ForgeError> {
    let total = points.len();
    let mut by_parent: HashMap<ChangePointId, ChangePoint> = HashMap::with_capacity(total);
    for point in points {
        if by_parent.insert(point.parent(), point).is_some() {
            // Two points on one parent: a fork, and a line has none.
            return Err(ForgeError::NotOnHead);
        }
    }

    let mut ordered = Vec::with_capacity(total);
    let mut at = head;
    while let Some(point) = by_parent.remove(&at) {
        at = point.id();
        ordered.push(point);
    }

    if !by_parent.is_empty() {
        return Err(ForgeError::NotOnHead);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::closing::close as end_work;
    use crate::domain::forge::model::op::Op;
    use crate::domain::forge::model::table::Row;
    use crate::domain::forge::model::value::{ActorId, Content, EntryId};
    use chrono::{DateTime, TimeZone, Utc};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, 12, minute, 0).unwrap()
    }

    fn act(minute: u32) -> Act {
        Act::new(at(minute), Actor::User(ActorId::new()))
    }

    fn name(text: &str) -> Name {
        Name::new(text).expect("a name")
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    /// A line with two change points on it, built the ordinary way, so
    /// the tests below have something a store could have kept.
    fn a_line_with_two_changes() -> Line {
        let mut line = Line::open(name("main"), StrategyId::new("by-hand").unwrap(), act(0));
        for (minute, label) in [(1, "one"), (2, "two")] {
            let mut work =
                Pursuit::open(line.id(), None, line.head(), Intent::default(), act(minute));
            work.push(
                Round::new(
                    work.head(),
                    vec![Op::add(content(), name(label))],
                    None,
                    act(minute),
                )
                .unwrap(),
            )
            .unwrap();
            let closing = end_work(&line, &work, Outcome::Satisfied, None, act(minute)).unwrap();
            closing.apply(&mut line, &mut work).unwrap();
        }
        line
    }

    /// Takes a line apart into the values a store would keep, and puts
    /// it back. The round trip is the whole contract.
    fn round_trip(kept: &Line) -> Result<Line, ForgeError> {
        let points: Vec<ChangePoint> = kept
            .history()
            .changes()
            .iter()
            .map(|point| {
                change_point(
                    point.id(),
                    point.parent(),
                    point.from(),
                    point.by(),
                    point.table().clone(),
                    *point.act(),
                )
            })
            .collect();
        line(
            kept.id(),
            kept.name().clone(),
            kept.strategy().clone(),
            kept.standing(),
            meta(*kept.meta().created(), *kept.meta().updated()),
            genesis(
                kept.history().genesis().id(),
                *kept.history().genesis().act(),
            ),
            points,
        )
    }

    #[test]
    fn a_line_comes_back_as_the_line_it_was() {
        let original = a_line_with_two_changes();
        let read_back = round_trip(&original).expect("a line a store kept is a line");
        assert_eq!(read_back, original);
    }

    #[test]
    fn the_order_a_store_hands_them_over_in_does_not_matter() {
        let original = a_line_with_two_changes();
        let mut points: Vec<ChangePoint> = original.history().changes().to_vec();
        points.reverse();

        let read_back = line(
            original.id(),
            original.name().clone(),
            original.strategy().clone(),
            original.standing(),
            meta(*original.meta().created(), *original.meta().updated()),
            genesis(
                original.history().genesis().id(),
                *original.history().genesis().act(),
            ),
            points,
        )
        .expect("the parent chain is the order");
        assert_eq!(read_back, original);
    }

    #[test]
    fn a_point_the_walk_cannot_reach_is_refused() {
        let original = a_line_with_two_changes();
        let mut points: Vec<ChangePoint> = original.history().changes().to_vec();
        // Cut the first link. The second point is now unreachable, and
        // a history with a hole is not one to read the front half of.
        points.remove(0);

        let refused = line(
            original.id(),
            original.name().clone(),
            original.strategy().clone(),
            original.standing(),
            meta(*original.meta().created(), *original.meta().updated()),
            genesis(
                original.history().genesis().id(),
                *original.history().genesis().act(),
            ),
            points,
        );
        assert!(matches!(refused, Err(ForgeError::NotOnHead)), "{refused:?}");
    }

    #[test]
    fn a_stored_table_that_names_one_thing_twice_does_not_come_back() {
        let taken = name("key visual");
        let mut line_a = Line::open(name("main"), StrategyId::new("by-hand").unwrap(), act(0));
        let genesis_id = line_a.history().genesis().id();

        // Two entries, both alive, both under one name — a table the
        // model would never have written, handed over as if it had.
        let mut rows = BTreeMap::new();
        rows.insert(EntryId::new(), Row::added(content(), taken.clone()));
        rows.insert(EntryId::new(), Row::added(content(), taken.clone()));
        let table = Table::of(rows).expect("a table of two adds");

        let forged = change_point(
            ChangePointId::new(),
            genesis_id,
            PursuitId::new(),
            NodeId::new(),
            table,
            act(1),
        );

        let refused = line(
            line_a.id(),
            line_a.name().clone(),
            line_a.strategy().clone(),
            line_a.standing(),
            meta(*line_a.meta().created(), *line_a.meta().updated()),
            genesis(genesis_id, *line_a.history().genesis().act()),
            vec![forged],
        );
        assert!(
            matches!(refused, Err(ForgeError::NameTaken(_))),
            "the store cannot argue the model out of its own rule: {refused:?}"
        );

        // And the line it was built from is untouched by the attempt.
        assert!(line_a.states().is_empty());
        line_a.rename(name("still here"), act(2));
    }

    #[test]
    fn work_comes_back_as_the_work_it_was() {
        let mut line_a = Line::open(name("main"), StrategyId::new("by-hand").unwrap(), act(0));
        let mut work = Pursuit::open(line_a.id(), None, line_a.head(), Intent::default(), act(1));
        work.push(
            Round::new(
                work.head(),
                vec![Op::add(content(), name("one"))],
                Some("a pass".into()),
                act(2),
            )
            .unwrap(),
        )
        .unwrap();
        let closing = end_work(&line_a, &work, Outcome::Satisfied, None, act(3)).unwrap();
        closing.apply(&mut line_a, &mut work).unwrap();

        let mut nodes: Vec<Node> = work
            .rounds()
            .iter()
            .map(|pass| {
                Node::Round(
                    round(
                        pass.id(),
                        pass.parent(),
                        pass.ops().to_vec(),
                        pass.note().map(str::to_owned),
                        *pass.act(),
                    )
                    .expect("a pass a store kept carries operations"),
                )
            })
            .collect();
        let ending = work.close().expect("it ended");
        nodes.push(Node::Close(close(
            ending.id(),
            ending.parent(),
            ending.outcome(),
            ending.note().map(str::to_owned),
            *ending.act(),
        )));

        let read_back = pursuit(
            work.id(),
            work.of(),
            work.parent(),
            meta(*work.meta().created(), *work.meta().updated()),
            open(
                work.opening().id(),
                work.base(),
                work.opening().intent().clone(),
                *work.opening().act(),
            ),
            nodes,
        )
        .expect("work a store kept is work");

        assert_eq!(read_back, work);
    }

    #[test]
    fn a_pass_after_the_ending_is_refused() {
        let line_a = Line::open(name("main"), StrategyId::new("by-hand").unwrap(), act(0));
        let opened = open(NodeId::new(), line_a.head(), Intent::default(), act(1));
        let ending = close(NodeId::new(), opened.id(), Outcome::Abandoned, None, act(2));
        let after = round(
            NodeId::new(),
            ending.id(),
            vec![Op::add(content(), name("late"))],
            None,
            act(3),
        )
        .unwrap();

        let refused = pursuit(
            PursuitId::new(),
            line_a.id(),
            None,
            meta(act(1), act(2)),
            opened,
            vec![Node::Close(ending), Node::Round(after)],
        );
        assert!(
            matches!(refused, Err(ForgeError::AlreadyClosed)),
            "{refused:?}"
        );
    }

    #[test]
    fn a_stored_pass_carrying_nothing_is_refused() {
        let refused = round(NodeId::new(), NodeId::new(), Vec::new(), None, act(1));
        assert!(
            matches!(refused, Err(ForgeError::EmptyRound)),
            "{refused:?}"
        );
    }
}
