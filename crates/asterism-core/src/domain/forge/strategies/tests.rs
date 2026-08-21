//! What each rule does, taken all the way to the line.
//!
//! Every case here runs the real path — a line, two pieces of work,
//! the rule, and a close — because what a rule returns is only half of
//! what it means. The other half is what the line carries afterwards,
//! and a rule that returns plausible operations while leaving the line
//! somewhere nobody asked for is the failure these exist to catch.

use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::change::collisions;
use crate::domain::forge::model::closing::close;
use crate::domain::forge::model::error::ForgeError;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::pursuit::{Intent, Outcome, Pursuit, Round};
use crate::domain::forge::model::react::react;
use crate::domain::forge::model::strategy::{Divergence, Strategy, StrategyError};
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{ActorId, Content, EntryId, Name, StrategyId};
use crate::domain::forge::strategies::{
    BothDiverge, Builtin, ByHand, DiscardMine, MainlineFirst, MineFirst, Strategies,
};
use chrono::{TimeZone, Utc};
use uuid::Uuid;

fn act(minute: u32) -> Act {
    Act::new(
        Utc.with_ymd_and_hms(2026, 8, 21, 12, minute, 0).unwrap(),
        Actor::User(ActorId::new()),
    )
}

fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

fn content() -> Content {
    Content::from_uuid(Uuid::now_v7())
}

fn line_by(rule: &dyn Strategy) -> Line {
    Line::open(name(Line::ROOT), rule.id(), act(0))
}

fn work_on(line: &Line) -> Pursuit {
    Pursuit::open(line.id(), None, line.head(), Intent::default(), act(1))
}

fn passing(mut work: Pursuit, ops: Vec<Op>, minute: u32) -> Pursuit {
    work.push(Round::new(work.log().head(), ops, None, act(minute)).unwrap())
        .unwrap();
    work
}

fn landed(line: &mut Line, ops: Vec<Op>, minute: u32) {
    let mut work = passing(work_on(line), ops, minute);
    let closing = close(line, &work, Outcome::Satisfied, None, act(minute)).unwrap();
    closing.apply(line, &mut work).unwrap();
}

/// One entry on the line, and work that disagrees with it about the
/// content. The shape every rule is asked about.
struct Standoff {
    line: Line,
    work: Pursuit,
    entry: EntryId,
    mine: Content,
    theirs: Content,
    started: Content,
}

fn standoff(rule: &dyn Strategy) -> Standoff {
    let mut line = line_by(rule);
    let entry = EntryId::new();
    let started = content();

    // The entry exists before the work begins, so there is an earlier
    // value for a rule to fork from.
    landed(
        &mut line,
        vec![Op::add_to(entry, started, name("cut-01"))],
        1,
    );

    let mine = content();
    let work = passing(work_on(&line), vec![Op::replace(entry, mine)], 2);

    let theirs = content();
    landed(&mut line, vec![Op::replace(entry, theirs)], 3);

    Standoff {
        line,
        work,
        entry,
        mine,
        theirs,
        started,
    }
}

/// Runs the rule and closes, and answers with what the line carries.
fn settled(rule: &dyn Strategy, mut at: Standoff) -> (Line, Pursuit, EntryStates) {
    let round = react(&at.line, &at.work, rule, ActorId::new(), act(4))
        .unwrap()
        .expect("the rule answered");
    at.work.push(round).unwrap();

    let closing = close(&at.line, &at.work, Outcome::Satisfied, None, act(5)).unwrap();
    closing.apply(&mut at.line, &mut at.work).unwrap();

    let states = at.line.states();
    (at.line, at.work, states)
}

/// Every live entry's name, sorted.
fn names(states: &EntryStates) -> Vec<String> {
    let mut names: Vec<_> = states
        .values()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_ref())
        .map(|name| name.as_str().to_string())
        .collect();
    names.sort();
    names
}

/// The registry answers for every rule it carries, and for none it
/// does not.
#[test]
fn every_rule_is_findable_by_the_name_it_gives() {
    let rules = Builtin::default();

    for rule in rules.all() {
        assert_eq!(rules.get(&rule.id()).expect("its own name").id(), rule.id());
    }
    assert!(
        rules
            .get(&StrategyId::new("from-elsewhere").unwrap())
            .is_none()
    );
}

/// Two rules answering to one name would make which one settles a line
/// depend on the order they happen to be listed in.
#[test]
fn no_two_rules_share_a_name() {
    let rules = Builtin::default();
    let mut names: Vec<_> = rules.all().iter().map(|rule| rule.id()).collect();
    let before = names.len();

    names.sort();
    names.dedup();

    assert_eq!(names.len(), before);
}

/// Somebody chooses between these, so every one of them has to say
/// what it is.
#[test]
fn every_rule_says_what_it_is_for() {
    for rule in Builtin::default().all() {
        let about = rule.about();
        assert!(!about.name.trim().is_empty(), "{}", rule.id());
        assert!(!about.summary.trim().is_empty(), "{}", rule.id());
    }
}

#[test]
fn mainline_first_leaves_the_line_where_it_was_and_carries_ours_beside_it() {
    let at = standoff(&MainlineFirst);
    let (entry, mine, theirs) = (at.entry, at.mine, at.theirs);

    let (_, work, states) = settled(&MainlineFirst, at);

    // The entry the line already had is untouched.
    assert_eq!(states[&entry].content, Some(theirs));
    // And this work's version is on the line under a name of its own.
    assert_eq!(names(&states), vec!["cut-01", "cut-01 (2)"]);
    let carried = states
        .iter()
        .find(|(id, _)| **id != entry)
        .expect("a second entry");
    assert_eq!(carried.1.content, Some(mine));
    assert!(carried.1.alive);
    // The work log says what happened, in ordinary operations.
    assert!(
        work.log()
            .rounds()
            .iter()
            .any(|round| round.act().by().is_system())
    );
}

/// This work's version keeps the name, and the contested entry goes.
/// It cannot keep the entry as well — see the rule's own docs.
#[test]
fn mine_first_hands_this_works_version_the_name() {
    let at = standoff(&MineFirst);
    let (entry, mine, theirs) = (at.entry, at.mine, at.theirs);

    let (_, _, states) = settled(&MineFirst, at);

    assert!(!states[&entry].alive, "the contested entry is taken off");
    assert_eq!(names(&states), vec!["cut-01", "cut-01 (2)"]);

    let under = |called: &str| {
        states
            .values()
            .find(|state| state.alive && state.name.as_ref().map(Name::as_str) == Some(called))
            .and_then(|state| state.content)
    };
    assert_eq!(under("cut-01"), Some(mine));
    assert_eq!(under("cut-01 (2)"), Some(theirs));
}

#[test]
fn both_diverge_takes_the_old_entry_off_and_puts_both_on_new_ones() {
    let at = standoff(&BothDiverge);
    let (entry, mine, theirs) = (at.entry, at.mine, at.theirs);

    let (_, _, states) = settled(&BothDiverge, at);

    // The entry they disagreed about is off the line, and what it held
    // is still readable.
    assert!(!states[&entry].alive);
    assert_eq!(states[&entry].content, Some(theirs));
    // Both versions are on the line, neither of them the original.
    let alive: Vec<_> = states
        .iter()
        .filter(|(_, state)| state.alive)
        .map(|(_, state)| state.content)
        .collect();
    assert_eq!(alive.len(), 2);
    assert!(alive.contains(&Some(mine)));
    assert!(alive.contains(&Some(theirs)));
    assert_eq!(names(&states), vec!["cut-01", "cut-01 (2)"]);
}

/// Dropping this work's version leaves the work with nothing to put on
/// the line — which is exactly right, and is what closing says.
#[test]
fn discard_mine_leaves_the_line_alone_and_the_work_with_nothing_to_say() {
    let mut at = standoff(&DiscardMine);
    let (entry, mine, theirs, started) = (at.entry, at.mine, at.theirs, at.started);

    let round = react(&at.line, &at.work, &DiscardMine, ActorId::new(), act(4))
        .unwrap()
        .expect("the rule answered");
    at.work.push(round).unwrap();

    // Nothing collides any more, and there is nothing left to land.
    assert!(collisions(&at.line, &at.work).unwrap().is_empty());
    assert_eq!(
        close(&at.line, &at.work, Outcome::Satisfied, None, act(5)),
        Err(ForgeError::NothingToRecord)
    );
    assert_eq!(at.line.states()[&entry].content, Some(theirs));

    // What was tried is in the work log — forked from the value the
    // entry had when the work began, changed, and taken off.
    let written: Vec<_> = at
        .work
        .log()
        .rounds()
        .iter()
        .filter(|round| round.act().by().is_system())
        .flat_map(|round| round.ops().iter().cloned())
        .collect();
    let forked = written[0].entry();
    assert_ne!(forked, entry);
    assert!(matches!(
        written[0].kind(),
        crate::domain::forge::model::op::OpKind::Add { content, .. } if *content == started
    ));
    assert!(matches!(
        written[1].kind(),
        crate::domain::forge::model::op::OpKind::Replace { content } if *content == mine
    ));
    assert!(matches!(
        written[2].kind(),
        crate::domain::forge::model::op::OpKind::Remove
    ));

    // And it can be closed as what it was: an attempt that stopped.
    let closing = close(&at.line, &at.work, Outcome::Abandoned, None, act(6)).unwrap();
    assert!(!closing.lands());
}

/// The rule that leaves it to a person writes nothing, and the
/// collision is still there afterwards — which is the state somebody
/// acts on.
#[test]
fn by_hand_leaves_the_collision_standing() {
    let at = standoff(&ByHand);

    let answered = react(&at.line, &at.work, &ByHand, ActorId::new(), act(4)).unwrap();

    assert!(answered.is_none());
    assert!(!collisions(&at.line, &at.work).unwrap().is_empty());
    assert!(matches!(
        close(&at.line, &at.work, Outcome::Satisfied, None, act(5)),
        Err(ForgeError::Collides(_))
    ));
}

/// Nothing lands without somebody having written it. The line where
/// resolution is manual cannot be closed past by doing nothing at all.
#[test]
fn by_hand_cannot_land_without_anybody_writing() {
    let at = standoff(&ByHand);
    let before = at.line.states()[&at.entry].content;

    // Reacting as often as anybody likes changes nothing.
    for minute in 4..8 {
        assert!(
            react(&at.line, &at.work, &ByHand, ActorId::new(), act(minute))
                .unwrap()
                .is_none()
        );
    }

    assert_eq!(at.line.states()[&at.entry].content, before);
    assert!(matches!(
        close(&at.line, &at.work, Outcome::Satisfied, None, act(9)),
        Err(ForgeError::Collides(_))
    ));
}

/// The failure the whole shape exists to prevent: work carrying a
/// value the line has since moved past, landing anyway and putting the
/// old value back.
#[test]
fn stale_work_cannot_land_over_a_line_that_moved() {
    let at = standoff(&MainlineFirst);

    assert!(matches!(
        close(&at.line, &at.work, Outcome::Satisfied, None, act(4)),
        Err(ForgeError::Collides(_))
    ));
    // And nothing the work can write on its own clears that without
    // saying something about the axis in question.
    let work = passing(at.work, vec![Op::add(content(), name("unrelated"))], 5);
    assert!(matches!(
        close(&at.line, &work, Outcome::Satisfied, None, act(6)),
        Err(ForgeError::Collides(_))
    ));
}

/// A rule is not trusted because it returned something.
#[test]
fn a_rule_that_does_not_settle_what_it_was_asked_is_refused() {
    struct Pretends;

    impl Strategy for Pretends {
        fn id(&self) -> StrategyId {
            MainlineFirst.id()
        }

        fn about(&self) -> crate::domain::forge::model::strategy::About {
            crate::domain::forge::model::strategy::About {
                name: "Pretends".into(),
                summary: "Writes something unrelated and calls it settled.".into(),
            }
        }

        fn resolve(&self, _at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError> {
            Ok(vec![Op::add(
                Content::from_uuid(Uuid::now_v7()),
                Name::new("beside the point").unwrap(),
            )])
        }
    }

    let at = standoff(&MainlineFirst);

    let refused = react(&at.line, &at.work, &Pretends, ActorId::new(), act(4));

    assert_eq!(refused.unwrap_err(), ForgeError::Unsettled);
}

/// A rule that refuses writes no pass at all, and the work is left
/// exactly as it was.
#[test]
fn a_rule_that_refuses_writes_nothing() {
    struct Stuck;

    impl Strategy for Stuck {
        fn id(&self) -> StrategyId {
            MainlineFirst.id()
        }

        fn about(&self) -> crate::domain::forge::model::strategy::About {
            crate::domain::forge::model::strategy::About {
                name: "Stuck".into(),
                summary: "Never decides.".into(),
            }
        }

        fn resolve(&self, _at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError> {
            Err(StrategyError::Undecidable("nothing to go on".into()))
        }
    }

    let at = standoff(&MainlineFirst);
    let before = at.work.log().rounds().len();

    let refused = react(&at.line, &at.work, &Stuck, ActorId::new(), act(4));

    assert!(matches!(refused, Err(ForgeError::Strategy(_))));
    assert_eq!(at.work.log().rounds().len(), before);
}

#[test]
fn a_rule_this_line_does_not_settle_by_is_refused() {
    let at = standoff(&MainlineFirst);

    let refused = react(&at.line, &at.work, &ByHand, ActorId::new(), act(4));

    assert_eq!(refused.unwrap_err(), ForgeError::WrongStrategy);
}

#[test]
fn there_is_nothing_to_react_to_when_nothing_collides() {
    let mut line = line_by(&MainlineFirst);
    landed(&mut line, vec![Op::add(content(), name("theirs"))], 1);
    let work = passing(work_on(&line), vec![Op::add(content(), name("mine"))], 2);

    let answered = react(&line, &work, &MainlineFirst, ActorId::new(), act(3)).unwrap();

    assert!(answered.is_none());
}

/// Every rule the forge ships leaves the work able to close, except
/// the one whose whole purpose is not to.
#[test]
fn every_automatic_rule_leaves_the_work_able_to_close() {
    for rule in Builtin::default().all() {
        // The rule that leaves it to a person writes nothing, and the
        // one that drops this work's version leaves nothing to land —
        // both are covered on their own terms above.
        if rule.id() == ByHand.id() || rule.id() == DiscardMine.id() {
            continue;
        }
        let at = standoff(rule);

        let round = react(&at.line, &at.work, rule, ActorId::new(), act(4))
            .unwrap()
            .unwrap_or_else(|| panic!("{} answered nothing", rule.id()));
        let mut work = at.work;
        work.push(round).unwrap();

        assert!(
            collisions(&at.line, &work).unwrap().is_empty(),
            "{} left a collision standing",
            rule.id()
        );
        assert!(
            close(&at.line, &work, Outcome::Satisfied, None, act(5))
                .is_ok_and(|closing| closing.lands()),
            "{} left the work unable to close",
            rule.id()
        );
    }
}

/// Resolving twice on one piece of work must not hand out one name
/// twice. The first resolution's entry is in the request and not on
/// the line, so the line cannot object to its name — and the refusal,
/// if it came, would come at the far end of the work.
#[test]
fn resolving_twice_does_not_hand_out_one_name_twice() {
    let mut at = standoff(&MainlineFirst);
    let entry = at.entry;

    let first = react(&at.line, &at.work, &MainlineFirst, ActorId::new(), act(4))
        .unwrap()
        .expect("the rule answered");
    at.work.push(first).unwrap();

    // The line moves the same axis again while the work is still open.
    landed(&mut at.line, vec![Op::replace(entry, content())], 5);
    assert!(!collisions(&at.line, &at.work).unwrap().is_empty());

    let second = react(&at.line, &at.work, &MainlineFirst, ActorId::new(), act(6))
        .unwrap()
        .expect("the rule answered the second round");
    at.work.push(second).unwrap();

    // Both resolutions land together, and the line applies the whole
    // request at once — which is where two entries under one name
    // would be refused.
    let closing = close(&at.line, &at.work, Outcome::Satisfied, None, act(7)).unwrap();
    closing.apply(&mut at.line, &mut at.work).unwrap();

    let states = at.line.states();
    let mut minted = names(&states);
    let before = minted.len();
    minted.dedup();
    assert_eq!(minted.len(), before, "every live name is used once");
}

/// Attribution is not decoration here: what a rule wrote has to be
/// tellable from what a person wrote.
#[test]
fn what_a_rule_writes_is_written_as_the_server() {
    let at = standoff(&MainlineFirst);
    let server = ActorId::new();

    let round = react(&at.line, &at.work, &MainlineFirst, server, act(4))
        .unwrap()
        .expect("the rule answered");

    assert_eq!(round.act().by(), Actor::System(server));
    assert!(!at.work.log().rounds()[0].act().by().is_system());
}
