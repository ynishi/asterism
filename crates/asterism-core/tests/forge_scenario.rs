//! One line, four people, and everything that happens to it.
//!
//! The other forge tests each pin one rule or one refusal. This one
//! runs a flow with the shape real work has — several pursuits open at
//! once, a line moving under them more than once, a resolution that is
//! itself overtaken — and then asks the log the questions the layer
//! exists to answer.
//!
//! Two things are being checked that no unit test can reach.
//!
//! **That resolving survives being done twice.** Every other test
//! resolves one collision once. A design can be right for one round
//! and wrong for the second, because the second starts from a request
//! the first one rewrote.
//!
//! **That the record answers what it is for.** A history of choices is
//! worth keeping only if, for any one selection, it says what was
//! chosen out of what, by whom, when, and in which piece of work.
//! Those are asked at the bottom, from the logs, through the public
//! API — which is also a check that a reader outside this crate can
//! ask them at all.

use asterism_core::domain::forge::model::act::{Act, Actor};
use asterism_core::domain::forge::model::change::collisions;
use asterism_core::domain::forge::model::closing::{Closing, close};
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::op::{Op, OpKind};
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome, Pursuit, Round};
use asterism_core::domain::forge::model::react::react;
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::table::EntryStates;
use asterism_core::domain::forge::model::value::{
    ActorId, ChangePointId, Content, EntryId, Name, NodeId,
};
use asterism_core::domain::forge::strategies::{
    Builtin, ByHand, DiscardMine, MainlineFirst, Strategies,
};
use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

/// The four people in this story, and the server.
struct Cast {
    ana: ActorId,
    boro: ActorId,
    cyd: ActorId,
    dai: ActorId,
    server: ActorId,
}

impl Cast {
    fn new() -> Self {
        Self {
            ana: ActorId::new(),
            boro: ActorId::new(),
            cyd: ActorId::new(),
            dai: ActorId::new(),
            server: ActorId::new(),
        }
    }
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 21, 9, minute, 0).unwrap()
}

fn by(who: ActorId, minute: u32) -> Act {
    Act::new(at(minute), Actor::User(who))
}

fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

fn content() -> Content {
    Content::from_uuid(Uuid::now_v7())
}

fn pass(work: &mut Pursuit, ops: Vec<Op>, act: Act) {
    work.push(Round::new(work.log().head(), ops, None, act).unwrap())
        .unwrap();
}

/// Opens work, writes one pass, closes it satisfied, and lands it.
/// The short way somebody puts something on a line.
fn lands(line: &mut Line, who: ActorId, ops: Vec<Op>, minute: u32) -> ChangePointId {
    let mut work = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(who, minute),
    );
    pass(&mut work, ops, by(who, minute));
    let closing = close(line, &work, Outcome::Satisfied, None, by(who, minute)).unwrap();
    let moved = closing.point().expect("that work landed").id();
    closing.apply(line, &mut work).unwrap();
    moved
}

/// What is alive on the line, by name.
fn alive(states: &EntryStates) -> Vec<String> {
    let mut names: Vec<_> = states
        .values()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_ref())
        .map(|name| name.as_str().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn a_line_four_people_and_what_the_record_can_answer() {
    let cast = Cast::new();
    let rules = Builtin::default();
    let mut line = Line::open(name(Line::ROOT), MainlineFirst.id(), by(cast.ana, 0));

    // ---- Ana puts the first thing on the line. -------------------
    let cut = EntryId::new();
    let ana_content = content();
    lands(
        &mut line,
        cast.ana,
        vec![Op::add_to(cut, ana_content, name("cut-01"))],
        1,
    );
    assert_eq!(alive(&line.states()), vec!["cut-01"]);

    // ---- Boro starts work on it, and Cyd finishes first. ---------
    let mut boro = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(cast.boro, 2),
    );
    let boro_content = content();
    pass(
        &mut boro,
        vec![Op::replace(cut, boro_content)],
        by(cast.boro, 3),
    );

    let cyd_content = content();
    lands(&mut line, cast.cyd, vec![Op::replace(cut, cyd_content)], 4);

    // Boro is now asking for something the line has moved past.
    let found = collisions(&line, &boro).unwrap();
    assert_eq!(found.len(), 1, "one axis, moved once");
    assert!(close(&line, &boro, Outcome::Satisfied, None, by(cast.boro, 5)).is_err());

    // ---- The line's rule resolves it, the first time. ------------
    let rule = rules.get(line.strategy()).expect("the line's rule");
    let settled = react(&line, &boro, rule, cast.server, by(cast.boro, 6))
        .unwrap()
        .expect("the rule had something to write");
    boro.push(settled).unwrap();

    assert!(collisions(&line, &boro).unwrap().is_empty());

    // ---- Dai moves the same axis before Boro closes. -------------
    let dai_content = content();
    lands(&mut line, cast.dai, vec![Op::replace(cut, dai_content)], 7);

    // The second round. This is the case a single resolution cannot
    // reach: Boro's request was rewritten by the first resolution, and
    // what collides now is what that resolution left behind.
    let again = collisions(&line, &boro).unwrap();
    assert!(
        !again.is_empty(),
        "conceding once does not settle the axis for good"
    );

    let settled = react(&line, &boro, rule, cast.server, by(cast.boro, 8))
        .unwrap()
        .expect("the rule answers the second round too");
    boro.push(settled).unwrap();
    assert!(collisions(&line, &boro).unwrap().is_empty());

    // ---- Boro lands. ---------------------------------------------
    let closing = close(&line, &boro, Outcome::Satisfied, None, by(cast.boro, 9)).unwrap();
    assert!(closing.lands());
    let boro_point = closing.point().unwrap().id();
    closing.apply(&mut line, &mut boro).unwrap();

    // The line kept Dai's value for the contested entry, and Boro's
    // work is beside it rather than over it.
    let states = line.states();
    assert_eq!(states[&cut].content, Some(dai_content));
    assert!(
        states.values().any(|state| state.alive
            && state.content == Some(boro_content)
            && state.name.as_ref().map(Name::as_str) != Some("cut-01")),
        "what Boro asked for is on the line, under a name of its own"
    );

    // ---- Ana tries something and drops it. -----------------------
    let mut ana = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(cast.ana, 10),
    );
    let tried = content();
    pass(&mut ana, vec![Op::replace(cut, tried)], by(cast.ana, 11));
    lands(&mut line, cast.cyd, vec![Op::replace(cut, content())], 12);

    let mut discarding = Line::open(name("scratch"), DiscardMine.id(), by(cast.ana, 10));
    let _ = &mut discarding; // the rule is exercised on its own line below

    // On this line the rule keeps both, so Ana drops hers by hand:
    // the same three operations the rule would have written.
    let fork = Op::add(ana_content, name("cut-01 (dropped)"));
    let forked = fork.entry();
    pass(
        &mut ana,
        vec![
            fork,
            Op::replace(forked, tried),
            Op::remove(forked),
            Op::replace(cut, line.states()[&cut].content.unwrap()),
        ],
        by(cast.ana, 13),
    );

    // Nothing of Ana's is left to put on the line, and closing says so
    // rather than moving the line to say nothing.
    assert!(collisions(&line, &ana).unwrap().is_empty());
    assert!(
        close(&line, &ana, Outcome::Satisfied, None, by(cast.ana, 14)).is_err(),
        "work that dropped everything has nothing to land"
    );
    let giving_up = close(&line, &ana, Outcome::Abandoned, None, by(cast.ana, 15)).unwrap();
    assert!(!giving_up.lands());
    giving_up.apply(&mut line, &mut ana).unwrap();

    // ================================================================
    // What the record can answer.
    // ================================================================

    // (a) The population: everything anybody ever proposed for this
    //     line, whether it lived or not. Read off the work logs.
    let proposed: Vec<EntryId> = [&boro, &ana]
        .iter()
        .flat_map(|work| work.log().rounds())
        .flat_map(|round| round.ops())
        .map(Op::entry)
        .collect();
    assert!(
        proposed.contains(&cut) && proposed.contains(&forked),
        "the entry that was argued over and the one that was dropped are both in the record"
    );

    // (b) What lived: folded from the history, never kept beside it.
    let living = alive(&line.states());
    assert!(living.contains(&"cut-01".to_string()));
    assert!(
        !living.contains(&"cut-01 (dropped)".to_string()),
        "what Ana dropped is not on the line"
    );

    // (c) What was dropped is still readable, with what it held.
    let dropped = line
        .history()
        .changes()
        .iter()
        .flat_map(|point| point.table().rows())
        .find(|(entry, _)| **entry == forked);
    assert!(
        dropped.is_none(),
        "Ana's fork never reached the line — it is in her work log and nowhere else"
    );
    let in_her_log: Vec<_> = ana
        .log()
        .rounds()
        .iter()
        .flat_map(|round| round.ops())
        .filter(|op| op.entry() == forked)
        .collect();
    assert_eq!(
        in_her_log.len(),
        3,
        "forked, changed, removed — the whole of what she tried"
    );
    assert!(matches!(in_her_log[1].kind(), OpKind::Replace { content } if *content == tried));

    // (d) Who: a person's passes and the server's are told apart on
    //     the node, without opening it.
    let boro_rounds = boro.log().rounds();
    let (mine, servers): (Vec<&Round>, Vec<&Round>) =
        boro_rounds.iter().partition(|r| !r.act().by().is_system());
    assert_eq!(mine.len(), 1, "Boro wrote one pass himself");
    assert_eq!(servers.len(), 2, "the rule wrote two, one per round");
    assert_eq!(servers[0].act().by(), Actor::System(cast.server));
    assert_eq!(mine[0].act().by(), Actor::User(cast.boro));

    // (e) When: the clock is pinned, so the record says when rather
    //     than roughly when.
    assert_eq!(mine[0].act().at(), at(3));
    assert_eq!(servers[0].act().at(), at(6));
    assert_eq!(servers[1].act().at(), at(8));

    // (f) Which piece of work: the change point names the pursuit it
    //     came out of, and the node in it that ended the work.
    let landed = line
        .history()
        .changes()
        .iter()
        .find(|point| point.id() == boro_point)
        .expect("Boro's change point");
    assert_eq!(landed.from(), boro.id());
    assert_eq!(landed.by(), boro.log().head());
    assert_eq!(boro.outcome(), Some(Outcome::Satisfied));
    assert_eq!(ana.outcome(), Some(Outcome::Abandoned));
}

/// A line settled by hand reaches the same place, with a person
/// writing what the rule would have written.
#[test]
fn a_line_settled_by_hand_reaches_the_same_place() {
    let cast = Cast::new();
    let mut line = Line::open(name(Line::ROOT), ByHand.id(), by(cast.ana, 0));
    let cut = EntryId::new();
    lands(
        &mut line,
        cast.ana,
        vec![Op::add_to(cut, content(), name("cut-01"))],
        1,
    );

    let mut boro = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(cast.boro, 2),
    );
    let boro_content = content();
    pass(
        &mut boro,
        vec![Op::replace(cut, boro_content)],
        by(cast.boro, 3),
    );
    lands(&mut line, cast.cyd, vec![Op::replace(cut, content())], 4);

    // The rule writes nothing, and the collision is still there.
    assert!(
        react(&line, &boro, &ByHand, cast.server, by(cast.boro, 5))
            .unwrap()
            .is_none()
    );
    assert!(!collisions(&line, &boro).unwrap().is_empty());

    // Boro writes the same three operations himself.
    let theirs = line.states()[&cut].content.unwrap();
    let fork = Op::add(theirs, name("cut-01 (2)"));
    let forked = fork.entry();
    pass(
        &mut boro,
        vec![
            fork,
            Op::replace(forked, boro_content),
            Op::replace(cut, theirs),
        ],
        by(cast.boro, 6),
    );

    assert!(collisions(&line, &boro).unwrap().is_empty());
    let closing = close(&line, &boro, Outcome::Satisfied, None, by(cast.boro, 7)).unwrap();
    closing.apply(&mut line, &mut boro).unwrap();

    assert_eq!(line.states()[&cut].content, Some(theirs));
    assert_eq!(line.states()[&forked].content, Some(boro_content));
    // And every pass on that log is a person's.
    assert!(
        boro.log()
            .rounds()
            .iter()
            .all(|r| !r.act().by().is_system())
    );
}

/// Ends are not the same thing as landings: what a change point names
/// is always a pursuit that was satisfied, and abandoned work leaves
/// the line where it was.
#[test]
fn abandoning_leaves_the_line_alone_and_the_attempt_readable() {
    let cast = Cast::new();
    let mut line = Line::open(name(Line::ROOT), MainlineFirst.id(), by(cast.ana, 0));
    let before = line.head();

    let mut boro = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(cast.boro, 1),
    );
    let attempted = content();
    pass(
        &mut boro,
        vec![Op::add(attempted, name("never landed"))],
        by(cast.boro, 2),
    );

    let closing = close(&line, &boro, Outcome::Abandoned, None, by(cast.boro, 3)).unwrap();
    assert!(!closing.lands());
    closing.apply(&mut line, &mut boro).unwrap();

    assert_eq!(line.head(), before, "the line did not move");
    assert!(line.states().is_empty());
    // The attempt is in the work log, in full.
    let written: Vec<_> = boro
        .log()
        .rounds()
        .iter()
        .flat_map(|round| round.ops())
        .collect();
    assert_eq!(written.len(), 1);
    assert!(matches!(written[0].kind(), OpKind::Add { content, .. } if *content == attempted));
}

/// The node ids in a work log order it, and nothing about a clock
/// does — a pass minted with an earlier timestamp still sits where the
/// chain puts it.
#[test]
fn the_chain_orders_a_work_log_and_the_clock_does_not() {
    let cast = Cast::new();
    let line = Line::open(name(Line::ROOT), MainlineFirst.id(), by(cast.ana, 0));
    let mut boro = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(cast.boro, 5),
    );

    let entry = EntryId::new();
    let first = content();
    let second = content();
    pass(
        &mut boro,
        vec![Op::add_to(entry, first, name("cut-01"))],
        by(cast.boro, 9),
    );
    // Written second, stamped an hour earlier.
    pass(
        &mut boro,
        vec![Op::replace(entry, second)],
        by(cast.boro, 1),
    );

    assert_eq!(
        boro.request()[&entry].content(),
        Some(second),
        "the later pass wins because it is later in the chain"
    );
    let parents: Vec<NodeId> = boro.log().rounds().iter().map(Round::parent).collect();
    assert_eq!(parents[0], boro.log().open().id());
    assert_eq!(parents[1], boro.log().rounds()[0].id());
}

/// Closing is terminal, and the same closing cannot be applied twice.
#[test]
fn a_closing_is_spent_when_it_is_applied() {
    let cast = Cast::new();
    let mut line = Line::open(name(Line::ROOT), MainlineFirst.id(), by(cast.ana, 0));
    let mut boro = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        by(cast.boro, 1),
    );
    pass(
        &mut boro,
        vec![Op::add(content(), name("cut-01"))],
        by(cast.boro, 2),
    );

    let closing: Closing = close(&line, &boro, Outcome::Satisfied, None, by(cast.boro, 3)).unwrap();
    let again = close(&line, &boro, Outcome::Satisfied, None, by(cast.boro, 3)).unwrap();
    closing.apply(&mut line, &mut boro).unwrap();

    // The second decision was made against the line as it was, and the
    // line has moved since — so it cannot be applied on top.
    assert!(again.apply(&mut line, &mut boro).is_err());
}
