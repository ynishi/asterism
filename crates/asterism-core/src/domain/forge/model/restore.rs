//! Building the model back from what a store kept.
//!
//! ```text
//!   stored rows ──► restore::line / restore::pursuit / restore::thread
//!                        │
//!                        ├─ the ids come from outside, here and
//!                        │  nowhere else in the model
//!                        │
//!                        └─ the nodes go back through record / push /
//!                           end / say — the same refusals a fresh
//!                           write meets
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

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::domain::forge::model::act::{Act, Meta};
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::history::{ChangePoint, Genesis, History};
use crate::domain::forge::model::line::{Line, Standing};
use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::pursuit::{Close, Intent, Open, Outcome, Pursuit, Round};
use crate::domain::forge::model::table::Table;
use crate::domain::forge::model::thread::{Anchor, Body, Message, Revision, Thread};
use crate::domain::forge::model::value::{
    ChangePointId, LineId, MessageId, Name, NodeId, PursuitId, StrategyId, ThreadId,
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

/// One thing said, as it was kept, with every correction to it.
pub fn message(
    id: MessageId,
    parent: Option<MessageId>,
    body: Body,
    act: Act,
    revisions: Vec<Revision>,
) -> Message {
    Message::restored(id, parent, body, act, revisions)
}

/// A whole conversation, from what it hangs off and what was said.
///
/// `messages` are in the order a store kept them, which is the order
/// they were said in, and it is kept — with one exception. A reply the
/// order given puts *before* the message it answers is moved to just
/// after it, because a conversation cannot be read back in an order
/// that has an answer preceding its question. Nothing else moves; this
/// is not a threading pass, and a conversation whose clock behaved
/// comes back exactly as it was given. See the module docs on
/// [`thread`](super::thread) for what orders what.
///
/// That exception is what keeps a clock that stepped backwards from
/// making a conversation unreadable rather than merely odd.
///
/// # Refusals
///
/// - [`NotInThatThread`](ForgeError::NotInThatThread) — a message
///   replies to one this thread does not hold. Handed back through
///   [`Thread::say`], so a stored reply meets the refusal a fresh one
///   meets. Replies that answer each other in a circle are held to the
///   same refusal: none of them can be put after its parent, so they
///   arrive as they were given and the first one meets it.
/// - [`EmptyThread`](ForgeError::EmptyThread) — nothing was said. A
///   thread with no messages is somebody having opened a conversation
///   and said nothing, which [`Thread::open`] refuses to make and this
///   refuses to read back.
pub fn thread(
    id: ThreadId,
    anchor: Anchor,
    title: Option<Name>,
    messages: Vec<Message>,
) -> Result<Thread, ForgeError> {
    if messages.is_empty() {
        return Err(ForgeError::EmptyThread);
    }
    let mut held = Thread::restored(id, anchor, title);
    for message in replies_after_parents(messages) {
        held.say(message)?;
    }
    Ok(held)
}

/// Moves a reply that arrived before the message it answers, and
/// nothing else.
///
/// The order given is the order a conversation was said in, and that
/// is the order it reads in — this is not a threading pass. The only
/// message that moves is one the order given puts before its parent,
/// and it moves the shortest distance that fixes it: everything the
/// clock already got right stays where it was. So four remarks said
/// 1, 2, 3, 4 with 3 answering 1 come back 1, 2, 3, 4, and only a
/// clock that stepped backwards makes this function do anything.
///
/// A reply whose parent is not here at all does not move, because that
/// is [`Thread::say`]'s refusal to give and not this function's. Nor
/// does a circle of replies: nothing in it can be put after its
/// parent, and what cannot be placed is handed over to meet the same
/// refusal rather than disappear from the conversation.
fn replies_after_parents(messages: Vec<Message>) -> Vec<Message> {
    let total = messages.len();
    let mut at: HashMap<MessageId, usize> = HashMap::with_capacity(total);
    for (index, message) in messages.iter().enumerate() {
        at.insert(message.id(), index);
    }

    // A reply whose parent is not here, and one that names itself, are
    // taken as answering nothing: both are refusals to hand to `say`,
    // and neither is an order to work out.
    let mut answers: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut waiting_on: Vec<Option<usize>> = vec![None; total];
    for (index, message) in messages.iter().enumerate() {
        if let Some(&parent) = message.parent().and_then(|parent| at.get(&parent))
            && parent != index
        {
            answers[parent].push(index);
            waiting_on[index] = Some(parent);
        }
    }

    // Earliest first among everything whose parent is already out,
    // which is what keeps the given order everywhere it was already
    // right: a message only waits while something it answers is still
    // to come.
    let mut ready: BinaryHeap<Reverse<usize>> = (0..total)
        .filter(|index| waiting_on[*index].is_none())
        .map(Reverse)
        .collect();
    let mut order = Vec::with_capacity(total);
    let mut taken = vec![false; total];
    while let Some(Reverse(index)) = ready.pop() {
        taken[index] = true;
        order.push(index);
        for answer in &answers[index] {
            ready.push(Reverse(*answer));
        }
    }

    // A circle answering itself: none of these ever became ready. They
    // go back in the order they came, to meet `say`.
    order.extend((0..total).filter(|index| !taken[*index]));

    let mut held: Vec<Option<Message>> = messages.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| held[index].take())
        .collect()
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

    fn body(said: &str) -> Body {
        Body::new(said).expect("something was said")
    }

    /// A conversation comes back whole: what was said, what it replied
    /// to, and every correction — with the ids the store kept.
    #[test]
    fn a_stored_thread_comes_back_with_its_ids_and_its_corrections() {
        let anchor = Anchor::Pursuit(PursuitId::new());
        let (first, second) = (MessageId::new(), MessageId::new());
        let held = ThreadId::new();

        let thread = thread(
            held,
            anchor,
            Some(name("about the second pass")),
            vec![
                message(
                    first,
                    None,
                    body("this reads oddly"),
                    act(1),
                    vec![Revision::new(body("this reads oddly to me"), act(2))],
                ),
                message(second, Some(first), body("agreed"), act(3), Vec::new()),
            ],
        )
        .expect("a store kept a conversation somebody had");

        assert_eq!(thread.id(), held);
        assert_eq!(thread.anchor(), anchor);
        assert_eq!(thread.messages().len(), 2);
        assert_eq!(thread.messages()[0].id(), first);
        assert_eq!(thread.messages()[1].parent(), Some(first));

        // The body now is the correction; what was said first is still
        // there, which is the whole reason amending appends.
        assert_eq!(
            thread.messages()[0].body().as_str(),
            "this reads oddly to me"
        );
        assert_eq!(thread.messages()[0].said().as_str(), "this reads oddly");
    }

    /// A reply the store handed over before the message it answers
    /// comes back, after it. The order given is a clock's, and a clock
    /// that stepped backwards is not a conversation to lose: the reply
    /// says what it answers, and that is where it goes.
    #[test]
    fn a_stored_reply_kept_before_its_parent_still_comes_back() {
        let (first, second) = (MessageId::new(), MessageId::new());
        let thread = thread(
            ThreadId::new(),
            Anchor::Pursuit(PursuitId::new()),
            None,
            vec![
                message(second, Some(first), body("agreed"), act(1), Vec::new()),
                message(first, None, body("this reads oddly"), act(3), Vec::new()),
            ],
        )
        .expect("a conversation written by a clock that stepped back");

        assert_eq!(thread.messages().len(), 2);
        assert_eq!(thread.messages()[0].id(), first);
        assert_eq!(thread.messages()[1].id(), second);
    }

    /// A conversation the clock got right comes back exactly as it was
    /// said. A reply to something further up is not pulled next to
    /// what it answers: reading a thread is not threading it.
    #[test]
    fn a_conversation_said_in_order_comes_back_in_that_order() {
        let (one, two, three, four) = (
            MessageId::new(),
            MessageId::new(),
            MessageId::new(),
            MessageId::new(),
        );
        let thread = thread(
            ThreadId::new(),
            Anchor::Pursuit(PursuitId::new()),
            None,
            vec![
                message(one, None, body("this reads oddly"), act(1), Vec::new()),
                message(two, None, body("the next one too"), act(2), Vec::new()),
                // Answers the first and was said third. A threading
                // pass would put it second.
                message(three, Some(one), body("agreed"), act(3), Vec::new()),
                message(four, None, body("I will take both"), act(4), Vec::new()),
            ],
        )
        .expect("a store kept a conversation somebody had");

        let said: Vec<MessageId> = thread.messages().iter().map(|held| held.id()).collect();
        assert_eq!(said, vec![one, two, three, four]);
    }

    /// And a reply that arrived early moves the shortest distance that
    /// makes it readable: after the message it answers, not to the end
    /// and not into a tree.
    #[test]
    fn a_reply_that_arrived_early_moves_only_past_what_it_answers() {
        let (one, two, three) = (MessageId::new(), MessageId::new(), MessageId::new());
        let thread = thread(
            ThreadId::new(),
            Anchor::Pursuit(PursuitId::new()),
            None,
            vec![
                // Said second, kept first: the clock stepped back.
                message(two, Some(one), body("agreed"), act(1), Vec::new()),
                message(one, None, body("this reads oddly"), act(2), Vec::new()),
                message(three, None, body("I will take it"), act(3), Vec::new()),
            ],
        )
        .expect("a conversation written by a clock that stepped back");

        let said: Vec<MessageId> = thread.messages().iter().map(|held| held.id()).collect();
        assert_eq!(said, vec![one, two, three]);
    }

    /// Replies answering each other in a circle are still refused:
    /// none of them can be put after its parent, and a conversation
    /// that answers itself is not one anybody had.
    #[test]
    fn stored_replies_that_answer_each_other_are_refused() {
        let (first, second) = (MessageId::new(), MessageId::new());
        let refused = thread(
            ThreadId::new(),
            Anchor::Pursuit(PursuitId::new()),
            None,
            vec![
                message(first, Some(second), body("agreed"), act(1), Vec::new()),
                message(second, Some(first), body("so do I"), act(2), Vec::new()),
            ],
        );
        assert!(
            matches!(refused, Err(ForgeError::NotInThatThread)),
            "{refused:?}"
        );
    }

    /// And a conversation nothing was said in does not come back as an
    /// empty one.
    #[test]
    fn a_stored_thread_with_nothing_in_it_is_refused() {
        let refused = thread(
            ThreadId::new(),
            Anchor::Pursuit(PursuitId::new()),
            None,
            Vec::new(),
        );
        assert!(
            matches!(refused, Err(ForgeError::EmptyThread)),
            "{refused:?}"
        );
    }
}
