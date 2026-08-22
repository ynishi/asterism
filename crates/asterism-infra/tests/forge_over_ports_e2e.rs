//! The forge scenario again, driven through the services and a store.
//!
//! `asterism-core`'s `forge_scenario` runs the same shape against the
//! model directly — `Line`, `Pursuit` and the free functions, all held
//! in memory as values. This runs it through
//! [`LineService`] and [`PursuitService`], over
//! [`MemoryForge`], which keeps rows and rebuilds a domain value on
//! every read.
//!
//! # What only this one can catch
//!
//! Two things, and they are the reason the pair is worth the
//! duplication.
//!
//! **That what is written down is enough to rebuild.** The model
//! scenario never stores anything, so every field it relies on is one
//! it is still holding. Here a line is decomposed to rows the moment it
//! is opened and rebuilt from them on the next call — a field nothing
//! kept is a field that comes back missing, and a rule the rebuild
//! cannot satisfy is a read that fails.
//!
//! **That the services carry the act between the two logs.** The model
//! scenario hands `close` a line and a pursuit it is holding. The
//! services have to fetch both, decide against them, and write the
//! ending and the change point through one port call — and a store
//! that took half of it would show up here and nowhere else.
//!
//! What it does not catch is anything about SQLite. That is the point
//! of running it first: the model and the services are answered for
//! before the adapter that persists any of it exists, so when that
//! adapter is wrong, this test still passes and the SQLite one does
//! not.

use std::sync::{Arc, Mutex};

use asterism_core::application::forge::{LineService, PursuitService};
use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::clock::Clock;
use asterism_core::domain::forge::model::act::Actor;
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::op::{Op, OpKind};
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome, Pursuit};
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::table::EntryStates;
use asterism_core::domain::forge::model::value::{Content, EntryId, Name, StrategyId};
use asterism_core::domain::forge::strategies::{Builtin, MainlineFirst};
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::DomainError;
use asterism_infra::memory::forge::{HoldsEverything, MemoryActors, MemoryForge};
use chrono::{DateTime, TimeZone, Utc};

/// A clock somebody winds. The scenario reads like the model one —
/// minute 3 is minute 3 — and every act it produces is checkable
/// afterwards.
#[derive(Debug, Default)]
struct Wound(Mutex<u32>);

impl Wound {
    fn set(&self, minute: u32) {
        *self.0.lock().unwrap() = minute;
    }
}

impl Clock for Wound {
    fn now(&self) -> DateTime<Utc> {
        at(*self.0.lock().unwrap())
    }
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 22, 9, minute, 0).unwrap()
}

fn who(name: &str) -> AttributionContext {
    AttributionContext::asserted(Some(Author::Subject(name.into())), None).expect("a subject")
}

fn name(text: &str) -> Name {
    Name::new(text).expect("a name")
}

fn content() -> Content {
    Content::of(AssetId::new())
}

fn alive(states: &EntryStates) -> Vec<String> {
    let mut names: Vec<String> = states
        .values()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_ref().map(|n| n.as_str().to_owned()))
        .collect();
    names.sort();
    names
}

/// Everything a caller needs, wired the way `core_init` will wire it.
struct World {
    lines: LineService,
    work: PursuitService,
    clock: Arc<Wound>,
    store: MemoryForge,
    persona: PersonaId,
}

impl World {
    fn new() -> Self {
        let store = MemoryForge::new();
        let clock = Arc::new(Wound::default());
        let rules = Arc::new(Builtin::default());
        let actors = Arc::new(MemoryActors::new());

        Self {
            lines: LineService::new(
                Arc::new(store.clone()),
                rules.clone(),
                actors.clone(),
                clock.clone(),
            ),
            work: PursuitService::new(
                Arc::new(store.clone()),
                Arc::new(store.clone()),
                Arc::new(store.clone()),
                rules,
                asterism_core::domain::forge::boundary::StoreClient::new(Arc::new(HoldsEverything)),
                actors,
                clock.clone(),
            ),
            clock,
            store,
            persona: PersonaId::new(),
        }
    }

    /// Opens work, writes one pass, and closes it satisfied — the
    /// whole of what a person doing one small thing does.
    async fn lands(&self, line: &Line, subject: &str, ops: Vec<Op>, minute: u32) -> Pursuit {
        self.clock.set(minute);
        let by = who(subject);
        let opened = self
            .work
            .open(&line.id(), None, Intent::default(), &by)
            .await
            .expect("work opens against a line that exists");
        self.work
            .push(&opened.id(), &self.persona, ops, None, &by)
            .await
            .expect("a pass with operations in it");
        self.work
            .close(&opened.id(), Outcome::Satisfied, None, &by)
            .await
            .expect("nothing was in the way");
        self.work
            .get(&opened.id())
            .await
            .expect("it is still there")
    }
}

/// The scenario `forge_scenario` runs against the model, run against
/// the services and a store that keeps rows.
#[tokio::test]
async fn a_line_four_people_and_a_store_that_keeps_rows() {
    let world = World::new();

    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");

    // Every read below goes back through the store, so this is already
    // the round trip: what came back is what the rows say.
    let read_back = world.lines.get(&line.id()).await.expect("read it back");
    assert_eq!(read_back, line, "a line survives being written down");

    // ---- Ana puts the first thing on the line. --------------------
    let cut = EntryId::new();
    let ana_content = content();
    world
        .lands(
            &line,
            "ana",
            vec![Op::add_to(cut, ana_content, name("cut-01"))],
            1,
        )
        .await;
    let line = world.lines.get(&line.id()).await.unwrap();
    assert_eq!(alive(&line.states()), vec!["cut-01"]);

    // ---- Boro starts work on it, and Cyd finishes first. ----------
    world.clock.set(2);
    let boro = who("boro");
    let boros_work = world
        .work
        .open(&line.id(), None, Intent::default(), &boro)
        .await
        .unwrap();

    world.clock.set(3);
    let boro_content = content();
    world
        .work
        .push(
            &boros_work.id(),
            &world.persona,
            vec![Op::replace(cut, boro_content)],
            None,
            &boro,
        )
        .await
        .unwrap();

    world
        .lands(&line, "cyd", vec![Op::replace(cut, content())], 4)
        .await;

    // Boro is now asking for something the line has moved past. Both
    // the reading and the refusal come back through the store.
    let found = world.work.collisions(&boros_work.id()).await.unwrap();
    assert_eq!(found.len(), 1, "one axis, moved once");

    world.clock.set(5);
    let refused = world
        .work
        .close(&boros_work.id(), Outcome::Satisfied, None, &boro)
        .await;
    assert!(
        matches!(refused, Err(DomainError::Conflict(_))),
        "a collision refuses the close: {refused:?}"
    );

    // ---- The line's rule resolves it, the first time. -------------
    world.clock.set(6);
    let settled = world
        .work
        .resolve(&boros_work.id(), &boro)
        .await
        .unwrap()
        .expect("the rule had something to write");
    assert!(
        settled.act().by().is_system(),
        "a pass the rule wrote is the server's"
    );
    assert!(
        world
            .work
            .collisions(&boros_work.id())
            .await
            .unwrap()
            .is_empty()
    );

    // ---- Dai moves the same axis before Boro closes. --------------
    world
        .lands(&line, "dai", vec![Op::replace(cut, content())], 7)
        .await;
    let dai_content = world.lines.get(&line.id()).await.unwrap().states()[&cut]
        .content
        .expect("Dai's value is what the line carries");

    // The second round: Boro's request was rewritten by the first
    // resolution, and what collides now is what that left behind.
    assert!(
        !world
            .work
            .collisions(&boros_work.id())
            .await
            .unwrap()
            .is_empty(),
        "conceding once does not settle the axis for good"
    );
    world.clock.set(8);
    world
        .work
        .resolve(&boros_work.id(), &boro)
        .await
        .unwrap()
        .expect("the rule answers the second round too");
    assert!(
        world
            .work
            .collisions(&boros_work.id())
            .await
            .unwrap()
            .is_empty()
    );

    // ---- Boro lands. ---------------------------------------------
    world.clock.set(9);
    world
        .work
        .close(&boros_work.id(), Outcome::Satisfied, None, &boro)
        .await
        .expect("nothing is in the way now");

    let line = world.lines.get(&line.id()).await.unwrap();
    let states = line.states();
    assert_eq!(
        states[&cut].content,
        Some(dai_content),
        "the line kept Dai's value for the contested entry"
    );
    assert!(
        states.values().any(|state| state.alive
            && state.content == Some(boro_content)
            && state.name.as_ref().map(Name::as_str) != Some("cut-01")),
        "what Boro asked for is on the line, under a name of its own"
    );

    // ---- Ana tries something and gives up. ------------------------
    world.clock.set(10);
    let ana = who("ana");
    let anas_work = world
        .work
        .open(&line.id(), None, Intent::default(), &ana)
        .await
        .unwrap();
    world.clock.set(11);
    let tried = content();
    world
        .work
        .push(
            &anas_work.id(),
            &world.persona,
            vec![Op::replace(cut, tried)],
            None,
            &ana,
        )
        .await
        .unwrap();

    // She writes the three operations by hand that drop her own work,
    // leaving nothing for the line to carry.
    world.clock.set(13);
    let fork = Op::add(ana_content, name("cut-01 (dropped)"));
    let forked = fork.entry();
    let standing = world.lines.get(&line.id()).await.unwrap().states()[&cut]
        .content
        .unwrap();
    world
        .work
        .push(
            &anas_work.id(),
            &world.persona,
            vec![
                fork,
                Op::replace(forked, tried),
                Op::remove(forked),
                Op::replace(cut, standing),
            ],
            None,
            &ana,
        )
        .await
        .unwrap();

    world.clock.set(14);
    let nothing_to_land = world
        .work
        .close(&anas_work.id(), Outcome::Satisfied, None, &ana)
        .await;
    assert!(
        nothing_to_land.is_err(),
        "work that dropped everything has nothing to land: {nothing_to_land:?}"
    );
    world.clock.set(15);
    world
        .work
        .close(&anas_work.id(), Outcome::Abandoned, None, &ana)
        .await
        .expect("giving up is always allowed");

    // ================================================================
    // What the record can answer — asked of the store, not of values
    // the test kept.
    // ================================================================

    let line = world.lines.get(&line.id()).await.unwrap();
    let boros_work = world.work.get(&boros_work.id()).await.unwrap();
    let anas_work = world.work.get(&anas_work.id()).await.unwrap();

    // (a) The population: everything anybody proposed, read off the
    //     work logs the store handed back.
    let filed = world.work.of_line(&line.id()).await.unwrap();
    assert_eq!(filed.len(), 5, "four that landed and one that gave up");
    let proposed: Vec<EntryId> = filed
        .iter()
        .flat_map(|work| work.log().rounds())
        .flat_map(|round| round.ops())
        .map(Op::entry)
        .collect();
    assert!(
        proposed.contains(&cut) && proposed.contains(&forked),
        "the entry argued over and the one dropped are both in the record"
    );

    // (b) What lived: folded from the history every time it is asked.
    let living = world.lines.states(&line.id()).await.unwrap();
    let living = alive(&living);
    assert!(living.contains(&"cut-01".to_string()));
    assert!(
        !living.contains(&"cut-01 (dropped)".to_string()),
        "what Ana dropped is not on the line"
    );

    // (c) What was dropped is in her log and nowhere else.
    assert!(
        !line
            .history()
            .changes()
            .iter()
            .flat_map(|point| point.table().rows())
            .any(|(entry, _)| *entry == forked),
        "Ana's fork never reached the line"
    );
    let in_her_log: Vec<_> = anas_work
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

    // (d) Who, and (e) when: both survived the round trip through the
    //     rows, including which passes the rule wrote.
    let (mine, servers): (Vec<_>, Vec<_>) = boros_work
        .log()
        .rounds()
        .iter()
        .partition(|round| !round.act().by().is_system());
    assert_eq!(mine.len(), 1, "Boro wrote one pass himself");
    assert_eq!(servers.len(), 2, "the rule wrote two, one per round");
    assert_eq!(mine[0].act().at(), at(3));
    assert_eq!(servers[0].act().at(), at(6));
    assert_eq!(servers[1].act().at(), at(8));
    assert!(matches!(servers[0].act().by(), Actor::System(_)));
    assert!(matches!(mine[0].act().by(), Actor::User(_)));

    // (f) Which piece of work: the change point names the pursuit and
    //     the node that ended it, and both came out of the store.
    let landed = line
        .history()
        .changes()
        .iter()
        .find(|point| point.from() == boros_work.id())
        .expect("Boro's change point");
    assert_eq!(landed.by(), boros_work.log().head());
    assert_eq!(boros_work.outcome(), Some(Outcome::Satisfied));
    assert_eq!(anas_work.outcome(), Some(Outcome::Abandoned));

    // (g) The two halves of a satisfied close are one act. Abandoning
    //     writes an ending and moves the line not at all.
    assert_eq!(
        line.history().changes().len(),
        4,
        "four satisfied closes, four change points — Ana's abandon added none"
    );
}

/// A close that loses the race is retried against the line as it now
/// is, and lands on the second attempt.
///
/// The model cannot be asked this: `close` takes a line by reference
/// and answers about that one. Whether anybody re-reads and tries again
/// is the service's question, and the store is what makes the two
/// answers differ.
#[tokio::test]
async fn a_close_that_loses_the_race_reads_again_and_lands() {
    let world = World::new();
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    // Two pieces of work, cut from the same head, touching different
    // entries — so neither collides with the other and both may land.
    world.clock.set(1);
    let first = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    let second = world
        .work
        .open(&line.id(), None, Intent::default(), &who("cyd"))
        .await
        .unwrap();

    world.clock.set(2);
    for (work, label) in [(&first, "one"), (&second, "two")] {
        world
            .work
            .push(
                &work.id(),
                &world.persona,
                vec![Op::add(content(), name(label))],
                None,
                &who("boro"),
            )
            .await
            .unwrap();
    }

    world.clock.set(3);
    world
        .work
        .close(&first.id(), Outcome::Satisfied, None, &who("boro"))
        .await
        .unwrap();

    // The second was decided against a head that has moved. It reads
    // again rather than refusing.
    world.clock.set(4);
    world
        .work
        .close(&second.id(), Outcome::Satisfied, None, &who("cyd"))
        .await
        .expect("the line moved, and the close was decided again against where it is now");

    let line = world.lines.get(&line.id()).await.unwrap();
    assert_eq!(alive(&line.states()), vec!["one", "two"]);
    assert_eq!(line.history().changes().len(), 2, "both landed, in order");
}

/// A store that kept something the model would not have written cannot
/// hand it back as though it had.
///
/// Reached by writing the rows directly, which is the only way to
/// produce the state at all — every path through the services refuses
/// it on the way in. That is the point: the read half refuses it too,
/// so a repair job, a bad migration or a hand-edited database is
/// caught at the door rather than trusted.
#[tokio::test]
async fn a_line_the_store_could_not_have_been_given_does_not_come_back() {
    use asterism_core::domain::forge::lines::Lines;
    use asterism_core::domain::forge::model::act::Act;
    use asterism_core::domain::forge::model::value::{
        ActorId, ChangePointId, Existence, NodeId, PursuitId,
    };
    use asterism_infra::memory::forge::rows;

    let world = World::new();
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    // Two live entries under one name, in a single change point. The
    // model refuses this in `History::record`; nothing stops a row
    // from saying it.
    let taken = name("key visual");
    let point_id = ChangePointId::new();
    let forged: Vec<rows::ChangeRowRow> = [EntryId::new(), EntryId::new()]
        .into_iter()
        .map(|entry| rows::ChangeRowRow {
            point: point_id,
            entry,
            existence: Some(Existence::Present),
            content: Some(content()),
            name: Some(taken.clone()),
        })
        .collect();

    world.store.force_rows(
        line.id(),
        rows::ChangePointRow {
            id: point_id,
            line: line.id(),
            parent: line.head(),
            from: PursuitId::new(),
            by: NodeId::new(),
            act: rows::ActRow::of(&Act::new(at(1), Actor::User(ActorId::new()))),
        },
        forged,
    );

    let refused = Lines::get(&world.store, &line.id()).await;
    assert!(
        matches!(refused, Err(DomainError::Conflict(_))),
        "the store cannot argue the model out of its own rule: {refused:?}"
    );
}

/// Every strategy the instance carries is answerable through the
/// service, and a line that names one it does not carry is refused
/// rather than quietly settled by something else.
#[tokio::test]
async fn a_line_cannot_be_opened_under_a_rule_this_instance_does_not_carry() {
    let world = World::new();
    world.clock.set(0);

    let carried: Vec<String> = world
        .lines
        .strategies()
        .await
        .into_iter()
        .map(|(id, _)| id.as_str().to_owned())
        .collect();
    assert!(carried.contains(&"mainline-first".to_string()));

    let refused = world
        .lines
        .open(
            name("nowhere"),
            StrategyId::new("a-rule-nobody-wrote").unwrap(),
            &who("ana"),
        )
        .await;
    assert!(refused.is_err(), "{refused:?}");
}
