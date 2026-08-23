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

use asterism_core::application::forge::{Anchored, LineService, PursuitService, ThreadService};
use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::clock::Clock;
use asterism_core::domain::forge::closings::{Closings, Deciding};
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::{Act, Actor};
use asterism_core::domain::forge::model::closing::{Closing, close as end_work};
use asterism_core::domain::forge::model::line::{Line, Standing};
use asterism_core::domain::forge::model::op::{Op, OpKind};
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome, Pursuit};
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::table::EntryStates;
use asterism_core::domain::forge::model::thread::{Body, Message};
use asterism_core::domain::forge::model::value::{
    ActorId, Content, EntryId, Name, NodeId, PursuitId, StrategyId,
};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::domain::forge::strategies::{Builtin, MainlineFirst};
use asterism_core::domain::forge::threads::Threads;
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::{ConflictKind, DomainError};
use asterism_infra::memory::forge::{HoldsEverything, MemoryActors, MemoryForge};
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::SqliteForge;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};

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

fn body(said: &str) -> Body {
    Body::new(said).expect("something was said")
}

/// The decision a caller would make again, for the tests that ask the
/// port directly.
///
/// The model, and a time somebody chose — the same two things the
/// service assembles, without the service.
struct Decides {
    outcome: Outcome,
    at: DateTime<Utc>,
}

impl Deciding for Decides {
    fn close(&self, line: &Line, pursuit: &Pursuit) -> Result<Closing, DomainError> {
        Ok(end_work(
            line,
            pursuit,
            self.outcome,
            None,
            Act::new(self.at, Actor::User(ActorId::new())),
        )?)
    }
}

fn decides(outcome: Outcome, minute: u32) -> Arc<dyn Deciding> {
    Arc::new(Decides {
        outcome,
        at: at(minute),
    })
}

/// Somebody who will not decide again, so a refused write stays
/// refused.
///
/// What it is for is asking what the store keeps when the second
/// answer never comes — which is nothing, and that is the property
/// worth pinning separately from the re-decision itself.
struct Refuses;

impl Deciding for Refuses {
    fn close(&self, _line: &Line, _pursuit: &Pursuit) -> Result<Closing, DomainError> {
        Err(DomainError::settled("this caller decides once"))
    }
}

/// Work cut from the line, asking for one entry of its own — so two of
/// these collide with nothing and both may land.
async fn cut(world: &World, line: &Line, label: &str) -> Pursuit {
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name(label))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();
    work
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

/// Everything a caller needs, wired the way `core_init` will wire it,
/// over whichever store the test was handed.
struct World {
    lines: LineService,
    work: PursuitService,
    /// What was said about any of it.
    said: ThreadService,
    clock: Arc<Wound>,
    /// The ports again, for the tests that ask one directly rather
    /// than through a service — a caller holding a stale value is the
    /// case the port exists for, and a service never produces one.
    lines_port: Arc<dyn Lines>,
    pursuits_port: Arc<dyn Pursuits>,
    closings: Arc<dyn Closings>,
    threads_port: Arc<dyn Threads>,
    persona: PersonaId,
    /// Set only over the in-memory store, for the one test that has to
    /// write rows nothing would ever write.
    memory: Option<MemoryForge>,
    /// Set only over SQLite, where `content` is a foreign key and a
    /// reference has to name a row that exists.
    isle: Option<AsyncIsle>,
}

impl World {
    fn over_memory() -> Self {
        let store = MemoryForge::new();
        Self::wire(
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            Some(store),
            None,
        )
    }

    /// The same wiring over a migrated database.
    ///
    /// The driver goes with the world: dropping it shuts the isle down,
    /// and a test that let it go would find every later call answering
    /// on a closed connection.
    async fn over_sqlite() -> (Self, AsyncIsleDriver) {
        let (isle, driver) = open_and_migrate_in_memory().await.expect("a database");
        let store = SqliteForge::new(isle.clone());
        let world = Self::wire(
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            None,
            Some(isle.clone()),
        );
        world.seed_persona().await;
        (world, driver)
    }

    fn wire(
        lines: Arc<dyn Lines>,
        pursuits: Arc<dyn Pursuits>,
        closings: Arc<dyn Closings>,
        threads: Arc<dyn Threads>,
        memory: Option<MemoryForge>,
        isle: Option<AsyncIsle>,
    ) -> Self {
        let clock = Arc::new(Wound::default());
        let rules = Arc::new(Builtin::default());
        let actors = Arc::new(MemoryActors::new());
        // The same handles for both services, so a write from one and
        // a remark on it from the other resolve to one actor.
        let actors_again = actors.clone();

        Self {
            lines: LineService::new(
                lines.clone(),
                pursuits.clone(),
                rules.clone(),
                actors.clone(),
                clock.clone(),
            ),
            work: PursuitService::new(
                pursuits.clone(),
                lines.clone(),
                closings.clone(),
                rules,
                asterism_core::domain::forge::boundary::StoreClient::new(Arc::new(HoldsEverything)),
                actors,
                clock.clone(),
            ),
            said: ThreadService::new(
                threads.clone(),
                pursuits.clone(),
                lines.clone(),
                actors_again,
                clock.clone(),
            ),
            clock,
            lines_port: lines,
            pursuits_port: pursuits,
            closings,
            threads_port: threads,
            persona: PersonaId::new(),
            memory,
            isle,
        }
    }

    /// A persona for the assets to hang off, over SQLite only.
    async fn seed_persona(&self) {
        let Some(isle) = &self.isle else { return };
        let id = *self.persona.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived, \
                                      created_at, updated_at) \
                 VALUES (?1, 'p', NULL, 0, 0, 0, 0)",
                rusqlite::params![id],
            )
        })
        .await
        .expect("a persona");
    }

    /// Content that exists.
    ///
    /// Over SQLite this writes an asset first, because `change_row`
    /// and `pursuit_op` carry a foreign key to one — which is the forge
    /// holding what it names, and the reason a reference here cannot
    /// be a uuid nobody minted. Over the in-memory store there is
    /// nothing below to hold, so it is the id and no row.
    async fn content(&self) -> Content {
        let asset = AssetId::new();
        if let Some(isle) = &self.isle {
            let (id, persona) = (*asset.as_uuid(), *self.persona.as_uuid());
            isle.call(move |conn| {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                        modality, labels, occurred_at, created_at, updated_at) \
                     VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[]', 0, 0, 0)",
                    rusqlite::params![id, persona, format!("a-{id}.md")],
                )
            })
            .await
            .expect("an asset");
        }
        Content::of(asset)
    }

    /// Opens work, writes one round, and closes it satisfied — the
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
            .push(&opened.id(), ops, None, &by)
            .await
            .expect("a round with operations in it");
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
async fn a_line_four_people_over_memory() {
    a_line_four_people(&World::over_memory()).await;
}

#[tokio::test]
async fn a_line_four_people_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_line_four_people(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_line_four_people(world: &World) {
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
    let ana_content = world.content().await;
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
    let boro_content = world.content().await;
    world
        .work
        .push(
            &boros_work.id(),
            vec![Op::replace(cut, boro_content)],
            None,
            &boro,
        )
        .await
        .unwrap();

    world
        .lands(
            &line,
            "cyd",
            vec![Op::replace(cut, world.content().await)],
            4,
        )
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
        matches!(
            refused,
            Err(DomainError::Conflict {
                kind: ConflictKind::Blocked,
                ..
            })
        ),
        "a collision refuses the close, and resolving it is the way \
         through rather than asking again: {refused:?}"
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
        "a round the rule wrote is the server's"
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
        .lands(
            &line,
            "dai",
            vec![Op::replace(cut, world.content().await)],
            7,
        )
        .await;
    let dai_content = world.lines.get(&line.id()).await.unwrap().states()[&cut]
        .content
        .expect("Dai's value is what the line carries");

    // The second resolution: Boro's request was rewritten by the first
    // one, and what collides now is what that left behind.
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
        .expect("the rule answers the second resolution too");
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
    let tried = world.content().await;
    world
        .work
        .push(&anas_work.id(), vec![Op::replace(cut, tried)], None, &ana)
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
    //     pursuits the store handed back.
    let filed = world.work.of_line(&line.id()).await.unwrap();
    assert_eq!(filed.len(), 5, "four that landed and one that gave up");
    let proposed: Vec<EntryId> = filed
        .iter()
        .flat_map(|work| work.rounds())
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
    //     rows, including which rounds the rule wrote.
    let (mine, servers): (Vec<_>, Vec<_>) = boros_work
        .rounds()
        .iter()
        .partition(|round| !round.act().by().is_system());
    assert_eq!(mine.len(), 1, "Boro wrote one round himself");
    assert_eq!(
        servers.len(),
        2,
        "the rule wrote two, one for each resolution"
    );
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
    assert_eq!(landed.by(), boros_work.head());
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

/// Two pieces of work cut from one head land one after the other, and
/// the second lands on what the first left.
///
/// `Head → P1, P2 → close P1 → Head' → close P2 → Head''`, which is
/// the ordinary shape of two people working at once. Nothing here
/// races: the second close reads a line that has already moved and
/// decides against it, so it lands on its first attempt. What that
/// proves is that a close aims at the head as it is when the close is
/// decided, rather than at the head the work was cut from.
#[tokio::test]
async fn two_pursuits_from_one_head_over_memory() {
    two_pursuits_from_one_head(&World::over_memory()).await;
}

#[tokio::test]
async fn two_pursuits_from_one_head_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    two_pursuits_from_one_head(&world).await;
    driver.shutdown().await.unwrap();
}

async fn two_pursuits_from_one_head(world: &World) {
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
                vec![Op::add(world.content().await, name(label))],
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

    // The second is decided against a line that has already moved, and
    // aims at where it is now.
    world.clock.set(4);
    world
        .work
        .close(&second.id(), Outcome::Satisfied, None, &who("cyd"))
        .await
        .expect("the second close aims at the head the first one left");

    let line = world.lines.get(&line.id()).await.unwrap();
    assert_eq!(alive(&line.states()), vec!["one", "two"]);

    // The chain, spelled out: genesis → the first close's point → the
    // second's. Each names the one before it, and the second names the
    // first rather than the genesis both were cut from.
    let chain = line.history().changes();
    assert_eq!(chain.len(), 2, "two closes, two change points");
    assert_eq!(chain[0].parent(), line.history().genesis().id());
    assert_eq!(chain[1].parent(), chain[0].id());
    assert_eq!(chain[0].from(), first.id());
    assert_eq!(chain[1].from(), second.id());
    assert_eq!(line.head(), chain[1].id());
}

/// A close aimed at a change point the line has left is not written
/// where it was aimed. The store asks for another and keeps that one.
///
/// The service always aims at the head it just read, so this asks the
/// port directly — which is the layer that has to hold the rule,
/// because a caller holding a stale line is exactly the case it exists
/// for.
#[tokio::test]
async fn a_stale_aim_is_decided_again_over_memory() {
    a_stale_aim_is_decided_again(&World::over_memory()).await;
}

#[tokio::test]
async fn a_stale_aim_is_decided_again_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_stale_aim_is_decided_again(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_stale_aim_is_decided_again(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let genesis = line.head();

    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("mine"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();

    // The closing is decided here, against the line as it is now —
    // which means its change point takes the genesis as its parent.
    let before = world.lines.get(&line.id()).await.unwrap();
    let work = world.work.get(&work.id()).await.unwrap();
    world.clock.set(3);
    let closing = end_work(
        &before,
        &work,
        Outcome::Satisfied,
        None,
        Act::new(at(3), Actor::User(ActorId::new())),
    )
    .expect("nothing collides yet");
    assert_eq!(closing.point().unwrap().parent(), genesis);

    // And *then* somebody else lands on the genesis. The closing above
    // is now aimed at a node the line has left, and it does not know
    // it — which is the whole case: the caller decided against a line
    // that has moved since, and only the write can tell it so.
    world
        .lands(
            &line,
            "cyd",
            vec![Op::add(world.content().await, name("theirs"))],
            2,
        )
        .await;

    Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        &closing,
        decides(Outcome::Satisfied, 4),
    )
    .await
    .expect("what the parent refused, deciding again answered");

    // The work ended once, and on the line the two closes are in the
    // order they landed rather than side by side on the genesis.
    let after = world.work.get(&work.id()).await.unwrap();
    assert_eq!(after.outcome(), Some(Outcome::Satisfied));
    let held = world.lines.get(&line.id()).await.unwrap();
    let chain = held.history().changes();
    assert_eq!(chain.len(), 2, "one point per close, re-decision and all");
    assert_eq!(chain[1].parent(), chain[0].id());
    assert_eq!(chain[1].from(), work.id());
    assert_eq!(
        chain[1].act().at(),
        at(4),
        "the ending that landed is the one decided second"
    );
    assert_eq!(alive(&held.states()), vec!["mine", "theirs"]);
}

/// A conversation about a round is written, corrected, and read back
/// with both what it says now and what it said first.
///
/// The round trip the store owes: ids the caller chose, the order the
/// messages were said in, a reply pointing at its parent, and every
/// correction in the order it was made. A store that kept only the
/// current body would pass a test that read `body()` and lose the
/// record this whole primitive exists to keep.
#[tokio::test]
async fn a_conversation_is_kept_whole_and_read_back_whole_over_memory() {
    a_conversation_is_kept_whole_and_read_back_whole(&World::over_memory()).await;
}

#[tokio::test]
async fn a_conversation_is_kept_whole_and_read_back_whole_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_conversation_is_kept_whole_and_read_back_whole(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_conversation_is_kept_whole_and_read_back_whole(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = cut(world, &line, "mine").await;
    let round = world.work.get(&work.id()).await.unwrap().rounds()[0].id();

    world.clock.set(2);
    let thread = world
        .said
        .open(
            Anchored::Round(work.id(), round),
            Some(name("about this round")),
            body("this reads oddly"),
            &who("cyd"),
        )
        .await
        .expect("a round somebody wrote is a thing to remark on");

    world.clock.set(3);
    let first = thread.messages()[0].id();
    let reply = world
        .said
        .say(&thread.id(), Some(first), body("agreed"), &who("boro"))
        .await
        .unwrap();

    world.clock.set(4);
    world
        .said
        .amend(
            &thread.id(),
            &first,
            body("this reads oddly to me"),
            &who("cyd"),
        )
        .await
        .unwrap();

    let read = world.said.get(&thread.id()).await.expect("it is kept");
    assert_eq!(read.id(), thread.id());
    assert_eq!(read.title().map(Name::as_str), Some("about this round"));
    assert_eq!(read.messages().len(), 2);

    // Order is the clock here, which is this record and no other.
    assert_eq!(read.messages()[0].id(), first);
    assert_eq!(read.messages()[1].id(), reply.id());
    assert_eq!(read.messages()[1].parent(), Some(first));

    // The correction is what it says now; what it said first is still
    // there, which is the whole reason amending appends.
    assert_eq!(read.messages()[0].body().as_str(), "this reads oddly to me");
    assert_eq!(read.messages()[0].said().as_str(), "this reads oddly");
    assert_eq!(read.messages()[0].revisions().len(), 1);
    assert_eq!(read.messages()[0].revisions()[0].act().at(), at(4));

    // And it is findable by what it hangs off, which is how anybody
    // looking at the round would reach it.
    let anchored = world
        .said
        .about(Anchored::Round(work.id(), round))
        .await
        .unwrap();
    assert_eq!(anchored.len(), 1);
    assert_eq!(anchored[0].id(), thread.id());

    // The pursuit as a whole is a different anchor, and nothing about
    // the round answers to it.
    assert!(
        world
            .said
            .about(Anchored::Pursuit(work.id()))
            .await
            .unwrap()
            .is_empty(),
        "a remark on one round is not a remark on the work"
    );
}

/// A thread hangs off something that was written, and the service is
/// where that is answered.
///
/// `Anchor` is built from the thing rather than from its id so that a
/// thread about something nobody wrote is not a value anybody can
/// make. A caller has ids, so the reading has to happen somewhere —
/// here, before anything is kept.
#[tokio::test]
async fn a_conversation_about_something_nobody_wrote_is_refused_over_memory() {
    a_conversation_about_something_nobody_wrote_is_refused(&World::over_memory()).await;
}

#[tokio::test]
async fn a_conversation_about_something_nobody_wrote_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_conversation_about_something_nobody_wrote_is_refused(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_conversation_about_something_nobody_wrote_is_refused(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = cut(world, &line, "mine").await;
    let round = world.work.get(&work.id()).await.unwrap().rounds()[0].id();

    // Work nobody opened.
    let refused = world
        .said
        .open(
            Anchored::Pursuit(PursuitId::new()),
            None,
            body("about what?"),
            &who("cyd"),
        )
        .await;
    assert!(refused.is_err(), "{refused:?}");

    // A round this work does not have.
    let refused = world
        .said
        .open(
            Anchored::Round(work.id(), NodeId::new()),
            None,
            body("about what?"),
            &who("cyd"),
        )
        .await;
    assert!(refused.is_err(), "{refused:?}");

    // And an entry the round did not touch — the model's own refusal,
    // reached through the service because the service is what has the
    // round to ask it of.
    let refused = world
        .said
        .open(
            Anchored::Entry(work.id(), round, EntryId::new()),
            None,
            body("about what?"),
            &who("cyd"),
        )
        .await;
    assert!(
        refused.is_err(),
        "a remark about what a round did to something has to be about \
         something it did: {refused:?}"
    );

    assert!(
        world
            .said
            .about(Anchored::Round(work.id(), round))
            .await
            .unwrap()
            .is_empty(),
        "and none of the three was kept"
    );
}

/// A reply reaching out of its own conversation is refused by the
/// store, not only by the model.
///
/// The service asks the model against the thread as it read it. That
/// is the same read every caller does and the same window every caller
/// has: the port asks again, against the thread it is writing to.
#[tokio::test]
async fn a_reply_to_another_conversation_is_refused_over_memory() {
    a_reply_to_another_conversation_is_refused(&World::over_memory()).await;
}

#[tokio::test]
async fn a_reply_to_another_conversation_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_reply_to_another_conversation_is_refused(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_reply_to_another_conversation_is_refused(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = cut(world, &line, "mine").await;

    world.clock.set(2);
    let mine = world
        .said
        .open(
            Anchored::Pursuit(work.id()),
            None,
            body("what is this for?"),
            &who("cyd"),
        )
        .await
        .unwrap();
    let theirs = world
        .said
        .open(
            Anchored::Pursuit(work.id()),
            None,
            body("separate question"),
            &who("boro"),
        )
        .await
        .unwrap();

    // Asked of the port directly, because the service refuses it a
    // step earlier and this is the layer that has to hold the rule.
    let stray = Message::new(
        Some(theirs.messages()[0].id()),
        body("answering over there"),
        Act::new(at(3), Actor::User(ActorId::new())),
    );
    let refused = Threads::say(&*world.threads_port, &mine.id(), &stray).await;
    // A `Validation`, and the same one `Thread::say` gives — which is
    // what the port's doc claims and, until this was fixed, was not
    // true: this answered `Conflict` while the model answered
    // `Validation`, so one situation had two statuses depending on
    // which door it came through. Nothing here is contended. The
    // caller addressed one conversation and named a message of
    // another, and no row could change to make that hold.
    assert!(
        matches!(refused, Err(DomainError::Validation(_))),
        "a reply belongs to one conversation, and asking again with \
         the same pair never changes that: {refused:?}"
    );
    assert_eq!(
        world.said.get(&mine.id()).await.unwrap().messages().len(),
        1,
        "and it was not kept"
    );

    // Two conversations about one thing stay two, because merging them
    // would be deciding they were about the same thing.
    let both = world
        .said
        .about(Anchored::Pursuit(work.id()))
        .await
        .unwrap();
    assert_eq!(both.len(), 2);
}

/// Archiving stops a line moving, reopening lets it move again, and
/// what says so is a close rather than a field.
///
/// A standing that only ever showed up in a read would be a standing
/// nothing depended on. What is asked here is the behaviour it exists
/// for: satisfied work cannot land on an archived line, giving up
/// still can, and the work that was refused lands after the line comes
/// back — the same pursuit, not a fresh one, because being refused
/// left it open.
#[tokio::test]
async fn an_archived_line_refuses_a_landing_until_it_is_reopened_over_memory() {
    an_archived_line_refuses_a_landing_until_it_is_reopened(&World::over_memory()).await;
}

#[tokio::test]
async fn an_archived_line_refuses_a_landing_until_it_is_reopened_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    an_archived_line_refuses_a_landing_until_it_is_reopened(&world).await;
    driver.shutdown().await.unwrap();
}

async fn an_archived_line_refuses_a_landing_until_it_is_reopened(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    world.clock.set(1);
    let mine = cut(world, &line, "mine").await;
    let giving_up = cut(world, &line, "theirs").await;

    world.clock.set(2);
    world.lines.archive(&line.id(), &who("ana")).await.unwrap();
    assert_eq!(
        world.lines.get(&line.id()).await.unwrap().standing(),
        Standing::Archived
    );

    let refused = world
        .work
        .close(&mine.id(), Outcome::Satisfied, None, &who("boro"))
        .await;
    assert!(
        refused.is_err(),
        "nothing lands on a line somebody is finished with: {refused:?}"
    );

    // Giving up is not landing, so it goes through — and the line does
    // not move for it.
    world
        .work
        .close(&giving_up.id(), Outcome::Abandoned, None, &who("boro"))
        .await
        .expect("work against an archived line can still be abandoned");
    assert!(
        world
            .lines
            .get(&line.id())
            .await
            .unwrap()
            .history()
            .changes()
            .is_empty()
    );

    world.clock.set(3);
    world.lines.reopen(&line.id(), &who("ana")).await.unwrap();
    world
        .work
        .close(&mine.id(), Outcome::Satisfied, None, &who("boro"))
        .await
        .expect("the refusal left the work open, so it lands now");

    let held = world.lines.get(&line.id()).await.unwrap();
    assert_eq!(held.standing(), Standing::Open);
    assert_eq!(alive(&held.states()), vec!["mine"]);
}

/// A drop takes the line, its history and every pursuit against it,
/// including work filed under other work.
///
/// The nesting is the part worth arranging rather than assuming. Every
/// foreign key inside the forge is `RESTRICT` and `pursuit.parent_id`
/// points at `pursuit`, so a chain of work is the shape no single
/// ordering of deletes answers — over SQLite this passes because the
/// keys are deferred to the commit, and over the in-memory store
/// because there are no keys to answer to. Both have to end with
/// nothing left.
#[tokio::test]
async fn a_drop_takes_the_line_and_the_work_filed_under_it_over_memory() {
    a_drop_takes_the_line_and_the_work_filed_under_it(&World::over_memory()).await;
}

#[tokio::test]
async fn a_drop_takes_the_line_and_the_work_filed_under_it_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_drop_takes_the_line_and_the_work_filed_under_it(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_drop_takes_the_line_and_the_work_filed_under_it(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    // An epic, work filed under it, and both landed — so the line has
    // a history, the work has ended, and the parent chain is real.
    world.clock.set(1);
    let epic = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    let under = world
        .work
        .open(&line.id(), Some(epic.id()), Intent::default(), &who("boro"))
        .await
        .unwrap();
    for (work, label) in [(&epic, "epic"), (&under, "under")] {
        world
            .work
            .push(
                &work.id(),
                vec![Op::add(world.content().await, name(label))],
                None,
                &who("boro"),
            )
            .await
            .unwrap();
        world.clock.set(2);
        world
            .work
            .close(&work.id(), Outcome::Satisfied, None, &who("boro"))
            .await
            .unwrap();
    }

    world.clock.set(3);
    world.lines.archive(&line.id(), &who("ana")).await.unwrap();
    let released = world
        .lines
        .discard(&line.id(), &who("ana"))
        .await
        .expect("archived, ended, and nothing outside points in");

    assert_eq!(
        released.len(),
        2,
        "both entries' content, from the line and the work together"
    );
    assert!(
        world.lines.get(&line.id()).await.is_err(),
        "the line is gone"
    );
    for work in [&epic, &under] {
        assert!(
            world.work.get(&work.id()).await.is_err(),
            "and so is the work against it"
        );
    }
    assert!(
        world.lines.list().await.unwrap().is_empty(),
        "nothing is left listing"
    );
}

/// A drop refuses when something outside the line points into it, and
/// the refusal arrives from the commit rather than from a statement.
///
/// This is the half the deferred foreign keys do *not* excuse, and the
/// reason deferring them is safe. Work on one line can be filed under
/// work on another — nothing forbids it — so dropping the second line
/// would leave the first pointing at a parent that is gone. Deferring
/// moves that check to `COMMIT`, where it still fires and still takes
/// the whole drop with it.
///
/// Over SQLite only: the check is a foreign key, and the in-memory
/// store has none to fire.
#[tokio::test]
async fn a_drop_something_outside_points_into_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;

    world.clock.set(0);
    let going = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let staying = world
        .lines
        .open(name("other"), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    // The parent is on the line being dropped; the child is on the one
    // that stays.
    world.clock.set(1);
    let parent = world
        .work
        .open(&going.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    let child = world
        .work
        .open(
            &staying.id(),
            Some(parent.id()),
            Intent::default(),
            &who("boro"),
        )
        .await
        .unwrap();
    for work in [&parent, &child] {
        world
            .work
            .close(&work.id(), Outcome::Abandoned, None, &who("boro"))
            .await
            .unwrap();
    }

    world.clock.set(2);
    world.lines.archive(&going.id(), &who("ana")).await.unwrap();
    let refused = world
        .lines
        .discard(&going.id(), &who("ana"))
        .await
        .expect_err("the other line's work is filed under this line's");
    assert!(
        matches!(refused, DomainError::Validation(_)),
        "not a race — reading again finds the same reference: {refused:?}"
    );

    // And the whole drop came back: the line, its work, and the line
    // that pointed at it are all still readable.
    world.lines.get(&going.id()).await.expect("the line stayed");
    world.work.get(&parent.id()).await.expect("and its work");
    world
        .work
        .get(&child.id())
        .await
        .expect("and what pointed at it");

    driver.shutdown().await.unwrap();
}

/// Dropping a line takes what was said about its work with it.
///
/// A conversation is anchored to a pursuit, a round, an entry as a round
/// had it, or a change point — every one of which goes when the line
/// does. Leaving the thread behind would keep a remark about something
/// that is not there, which is the state `restore` refuses to read
/// back and the schema refuses to hold: the pursuit and change-point
/// anchors are foreign keys, so a drop that ignored them would be
/// refused rather than wrong.
#[tokio::test]
async fn dropping_a_line_takes_what_was_said_about_it_over_memory() {
    dropping_a_line_takes_what_was_said_about_it(&World::over_memory()).await;
}

#[tokio::test]
async fn dropping_a_line_takes_what_was_said_about_it_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    dropping_a_line_takes_what_was_said_about_it(&world).await;
    driver.shutdown().await.unwrap();
}

async fn dropping_a_line_takes_what_was_said_about_it(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = cut(world, &line, "mine").await;
    let round = world.work.get(&work.id()).await.unwrap().rounds()[0].id();

    // One of each anchor that a line can take with it: the work, a
    // round of it, and what landed.
    world.clock.set(2);
    let about_work = world
        .said
        .open(
            Anchored::Pursuit(work.id()),
            None,
            body("what is this for?"),
            &who("cyd"),
        )
        .await
        .unwrap();
    let about_round = world
        .said
        .open(
            Anchored::Round(work.id(), round),
            None,
            body("this reads oddly"),
            &who("cyd"),
        )
        .await
        .unwrap();

    world.clock.set(3);
    world
        .work
        .close(&work.id(), Outcome::Satisfied, None, &who("boro"))
        .await
        .unwrap();
    let landed = world.lines.get(&line.id()).await.unwrap().head();
    let about_landing = world
        .said
        .open(
            Anchored::Change(line.id(), landed),
            None,
            body("good, this is what we wanted"),
            &who("ana"),
        )
        .await
        .unwrap();

    world.clock.set(4);
    world.lines.archive(&line.id(), &who("ana")).await.unwrap();
    world
        .lines
        .discard(&line.id(), &who("ana"))
        .await
        .expect("what was said about the work goes with the work");

    for thread in [&about_work, &about_round, &about_landing] {
        assert!(
            world.said.get(&thread.id()).await.is_err(),
            "the conversation went with the line it was about"
        );
    }
}

/// A drop decided against work that has grown since is refused, and
/// takes nothing.
///
/// This is the race the port's `covering` argument exists for: what a
/// caller was told the drop releases came from a list of pursuits, and
/// a pursuit opened after that list was read is content the answer
/// left out. Refusing is the only honest move — writing would free
/// bytes nobody was told about, silently.
#[tokio::test]
async fn a_drop_that_does_not_cover_the_work_is_refused_over_memory() {
    a_drop_that_does_not_cover_the_work_is_refused(&World::over_memory()).await;
}

#[tokio::test]
async fn a_drop_that_does_not_cover_the_work_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_drop_that_does_not_cover_the_work_is_refused(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_drop_that_does_not_cover_the_work_is_refused(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .close(&work.id(), Outcome::Abandoned, None, &who("boro"))
        .await
        .unwrap();
    world.clock.set(2);
    world.lines.archive(&line.id(), &who("ana")).await.unwrap();

    // Asked directly, naming no work at all — which is what a caller
    // holding a list read before that pursuit existed would send.
    let refused = Lines::discard(&*world.lines_port, &line.id(), &[])
        .await
        .expect_err("the work against this line is not the work this drop covers");
    assert!(
        matches!(
            refused,
            DomainError::Conflict {
                kind: ConflictKind::Raced,
                ..
            }
        ),
        "a list that has moved is a race, not malformed input: {refused:?}"
    );

    // And nothing went: the line still reads, and so does its work.
    world.lines.get(&line.id()).await.expect("the line stayed");
    world
        .work
        .get(&work.id())
        .await
        .expect("and so did the work");
}

/// A drop naming work that is not against the line is refused as the
/// caller's mistake, not as a race.
///
/// The two ways `covering` can differ from what is there are not one
/// refusal. Work the drop did not name is work opened since the list
/// was read — that is the race. A name that is not against this line
/// cannot have arrived that way: nothing removes a pursuit but a drop
/// of its line, and this line is the one still here. Reporting it as a
/// conflict would tell a caller to read again and retry, and reading
/// again returns the same answer forever.
#[tokio::test]
async fn a_drop_naming_another_lines_work_is_refused_over_memory() {
    a_drop_naming_another_lines_work_is_refused(&World::over_memory()).await;
}

#[tokio::test]
async fn a_drop_naming_another_lines_work_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_drop_naming_another_lines_work_is_refused(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_drop_naming_another_lines_work_is_refused(world: &World) {
    world.clock.set(0);
    let going = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let staying = world
        .lines
        .open(name("the other one"), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    world.clock.set(1);
    let mine = world
        .work
        .open(&going.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .close(&mine.id(), Outcome::Abandoned, None, &who("boro"))
        .await
        .unwrap();
    let theirs = world
        .work
        .open(&staying.id(), None, Intent::default(), &who("cyd"))
        .await
        .unwrap();

    world.clock.set(2);
    world.lines.archive(&going.id(), &who("ana")).await.unwrap();

    let refused = Lines::discard(&*world.lines_port, &going.id(), &[mine.id(), theirs.id()])
        .await
        .expect_err("the other line's work is not this drop's to release");
    assert!(
        matches!(refused, DomainError::Validation(_)),
        "naming somebody else's work is a mistake, not a race: {refused:?}"
    );

    // And nothing went, on either line.
    world.lines.get(&going.id()).await.expect("the line stayed");
    world
        .work
        .get(&theirs.id())
        .await
        .expect("and so did the work it named");
}

/// A drop of a line somebody took back out of the archive is refused,
/// and takes nothing.
///
/// The other half of the same race. A drop is decided against an
/// archived line — that is where "finished with" is said — and the
/// standing it was decided against is a field that can move between
/// the decision and the write, exactly as the work list can. Holding
/// the condition against the caller's copy of the line would drop a
/// line somebody is using again, which is the one loss here that
/// nobody asked for.
#[tokio::test]
async fn a_drop_of_a_line_reopened_under_it_is_refused_over_memory() {
    a_drop_of_a_line_reopened_under_it_is_refused(&World::over_memory()).await;
}

#[tokio::test]
async fn a_drop_of_a_line_reopened_under_it_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_drop_of_a_line_reopened_under_it_is_refused(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_drop_of_a_line_reopened_under_it_is_refused(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .close(&work.id(), Outcome::Abandoned, None, &who("boro"))
        .await
        .unwrap();
    world.clock.set(2);
    world.lines.archive(&line.id(), &who("ana")).await.unwrap();

    // Decided against the archived line, and then somebody wants it
    // back before the write runs.
    world.clock.set(3);
    world.lines.reopen(&line.id(), &who("cyd")).await.unwrap();

    let refused = Lines::discard(&*world.lines_port, &line.id(), &[work.id()])
        .await
        .expect_err("a drop is decided against an archived line, and this one is open");
    assert!(
        matches!(
            refused,
            DomainError::Conflict {
                kind: ConflictKind::Raced,
                ..
            }
        ),
        "a standing that moved is a race, not malformed input: {refused:?}"
    );

    // And nothing went: the line still reads, still open, and so does
    // its work.
    let stayed = world.lines.get(&line.id()).await.expect("the line stayed");
    assert_eq!(stayed.standing(), Standing::Open);
    world
        .work
        .get(&work.id())
        .await
        .expect("and so did the work");
}

/// A close whose *work* moved under it is decided again too, and the
/// round that arrived is in what lands.
///
/// The other race [`Deciding`] names, and the one a caller never sees
/// coming: a close is decided against a pursuit, somebody writes a
/// round to that pursuit, and the ending now sits on a node something
/// else has. Both stores answer it the same way — SQLite from
/// `UNIQUE (pursuit_id, parent_id)`, the in-memory store from the same
/// rule asked of its rows.
///
/// What the last assertion pins is that this is a decision and not a
/// retry: the round that arrived is folded into what the second answer
/// puts on the line, which replaying the first answer could not do.
#[tokio::test]
async fn a_close_whose_work_moved_is_decided_again_over_memory() {
    a_close_whose_work_moved_is_decided_again(&World::over_memory()).await;
}

#[tokio::test]
async fn a_close_whose_work_moved_is_decided_again_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_close_whose_work_moved_is_decided_again(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_close_whose_work_moved_is_decided_again(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("mine"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();

    // Decided against the work as it stands, so the ending sits on the
    // round above.
    let held = world.work.get(&work.id()).await.unwrap();
    let closing = end_work(
        &world.lines.get(&line.id()).await.unwrap(),
        &held,
        Outcome::Satisfied,
        None,
        Act::new(at(2), Actor::User(ActorId::new())),
    )
    .expect("nothing collides");
    assert_eq!(closing.close().parent(), held.head());

    // And *then* another round arrives on the same work.
    world.clock.set(3);
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("later"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();

    Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        &closing,
        decides(Outcome::Satisfied, 4),
    )
    .await
    .expect("the ending was decided again, on the node the work now ends at");

    let after = world.work.get(&work.id()).await.unwrap();
    assert_eq!(after.outcome(), Some(Outcome::Satisfied));
    assert_eq!(
        after.close().unwrap().act().at(),
        at(4),
        "the ending that landed is the one decided second"
    );

    let held = world.lines.get(&line.id()).await.unwrap();
    assert_eq!(held.history().changes().len(), 1, "one close, one point");
    assert_eq!(
        alive(&held.states()),
        vec!["later", "mine"],
        "the round that arrived is on the line, which a replayed decision \
         would have left off"
    );
}

/// And when nobody decides again, the refusal stands and nothing is
/// kept.
///
/// The half of the rule the re-decision hides: the parent is still
/// what refuses, and a caller that has no second answer is told so
/// with both logs untouched.
#[tokio::test]
async fn a_stale_aim_nobody_decides_again_writes_nothing_over_memory() {
    a_stale_aim_nobody_decides_again_writes_nothing(&World::over_memory()).await;
}

#[tokio::test]
async fn a_stale_aim_nobody_decides_again_writes_nothing_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_stale_aim_nobody_decides_again_writes_nothing(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_stale_aim_nobody_decides_again_writes_nothing(world: &World) {
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let genesis = line.head();

    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("mine"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();

    let before = world.lines.get(&line.id()).await.unwrap();
    let work = world.work.get(&work.id()).await.unwrap();
    let closing = end_work(
        &before,
        &work,
        Outcome::Satisfied,
        None,
        Act::new(at(3), Actor::User(ActorId::new())),
    )
    .expect("nothing collides yet");
    assert_eq!(closing.point().unwrap().parent(), genesis);

    world
        .lands(
            &line,
            "cyd",
            vec![Op::add(world.content().await, name("theirs"))],
            2,
        )
        .await;

    let refused = Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        &closing,
        Arc::new(Refuses),
    )
    .await;
    assert!(
        matches!(refused, Err(DomainError::Conflict { .. })),
        "a close only lands on a parent nothing has taken: {refused:?}"
    );

    // And nothing of it was written — including the ending, which goes
    // in before the change point that refused.
    let after = world.work.get(&work.id()).await.unwrap();
    assert_eq!(after.outcome(), None, "the refused close left no ending");
    assert_eq!(
        world
            .lines
            .get(&line.id())
            .await
            .unwrap()
            .history()
            .changes()
            .len(),
        1,
        "and put nothing on the line"
    );
}

/// A store that kept something the model would not have written cannot
/// hand it back as though it had.
///
/// Reached by writing the rows directly, which is the only way to
/// produce the state at all — every path through the services refuses
/// it on the way in. That is the point: the read half refuses it too,
/// so a repair job, a bad migration or a hand-edited database is
/// caught at the door rather than trusted.
///
/// Asked of the in-memory store, because it is the one with a way to
/// write rows nothing checked (`force_rows`). What refuses them is
/// `rows::read_line`, which both stores share, so the SQLite side is
/// answered by the same code — and separately, by a hand-edited
/// database in
/// [`a_stored_outcome_this_model_has_no_name_for_is_refused`].
#[tokio::test]
async fn a_line_the_store_could_not_have_been_given_does_not_come_back() {
    use asterism_core::domain::forge::lines::Lines;
    use asterism_core::domain::forge::model::act::Act;
    use asterism_core::domain::forge::model::value::{
        ActorId, ChangePointId, Existence, NodeId, PursuitId,
    };
    use asterism_infra::forge::rows;

    let world = World::over_memory();
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
    let filling = world.content().await;
    let forged: Vec<rows::ChangeRowRow> = [EntryId::new(), EntryId::new()]
        .into_iter()
        .map(|entry| rows::ChangeRowRow {
            point: point_id,
            entry,
            existence: Some(Existence::Present),
            content: Some(filling),
            name: Some(taken.clone()),
        })
        .collect();

    world
        .memory
        .as_ref()
        .expect("the forced-rows test runs over memory")
        .force_rows(
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

    let refused = Lines::get(&*world.lines_port, &line.id()).await;
    assert!(
        matches!(refused, Err(DomainError::Infra(_))),
        "the store cannot argue the model out of its own rule: {refused:?}"
    );
    // And it is not offered to the caller as anything it did. This
    // test's own name says the store could not have been given the
    // row; a `Conflict` would have said the caller collided with
    // something, and the kinds would then have told it whether to ask
    // again — about a row that will read back the same way forever.
    let Err(refused) = refused else {
        unreachable!("asserted above")
    };
    assert!(
        refused.to_string().contains("could not have been written"),
        "the refusal says whose fault it is: {refused}"
    );
}

/// Every strategy the instance carries is answerable through the
/// service, and a line that names one it does not carry is refused
/// rather than quietly settled by something else.
#[tokio::test]
async fn a_line_cannot_be_opened_under_a_rule_this_instance_does_not_carry() {
    let world = World::over_memory();
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

/// A close whose line moves between the read and the write is decided
/// again, against the line as it now is.
///
/// The test above never reaches this: its second close reads a line
/// that has already moved, so it aims correctly the first time. The
/// re-decision only runs when the line moves *after* the service has
/// read it, which is what `Stale` arranges — it hands the service a
/// line from before the move, once, and then tells the truth.
///
/// Worth pinning through the service because the whole point is that
/// the service does nothing: it decides once, hands that decision over
/// with a way to make it again, and the store answers the race without
/// ever coming back. What it proves is that the way to decide again
/// arrives intact — a service that passed one nothing could call would
/// pass every test that asks the port directly.
///
/// Asked of the in-memory store, because `Stale` has to sit between
/// the service and whatever is beneath it, and the simpler store makes
/// the seam obvious. Note which line the re-decision reads: the store
/// reads its own rows rather than going back through `Stale`, so the
/// second answer is against the line that really moved. Both stores
/// refuse the first attempt the same way — the parent is taken —
/// which is the fact [`a_stale_aim_is_decided_again_over_sqlite`] pins
/// on the other side.
#[tokio::test]
async fn a_close_whose_line_moves_under_it_is_decided_again() {
    use asterism_core::domain::forge::lines::Lines;
    use asterism_core::domain::forge::model::act::Act;
    use asterism_core::domain::forge::model::value::LineId;

    /// A `Lines` that answers with a line from before the move, the
    /// first time it is asked after being armed.
    #[derive(Debug)]
    struct Stale {
        real: MemoryForge,
        armed: Mutex<Option<Line>>,
    }

    #[async_trait::async_trait]
    impl Lines for Stale {
        async fn open(&self, line: &Line) -> Result<(), DomainError> {
            Lines::open(&self.real, line).await
        }

        async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError> {
            if let Some(before) = self.armed.lock().unwrap().take() {
                return Ok(Some(before));
            }
            Lines::get(&self.real, id).await
        }

        async fn list(&self) -> Result<Vec<Line>, DomainError> {
            Lines::list(&self.real).await
        }

        async fn rename(&self, id: &LineId, name: &Name, act: &Act) -> Result<(), DomainError> {
            Lines::rename(&self.real, id, name, act).await
        }

        async fn set_strategy(
            &self,
            id: &LineId,
            strategy: &StrategyId,
            act: &Act,
        ) -> Result<(), DomainError> {
            Lines::set_strategy(&self.real, id, strategy, act).await
        }

        async fn set_standing(
            &self,
            id: &LineId,
            standing: Standing,
            act: &Act,
        ) -> Result<(), DomainError> {
            Lines::set_standing(&self.real, id, standing, act).await
        }

        async fn discard(&self, id: &LineId, covering: &[PursuitId]) -> Result<(), DomainError> {
            Lines::discard(&self.real, id, covering).await
        }
    }

    let store = MemoryForge::new();
    let stale = Arc::new(Stale {
        real: store.clone(),
        armed: Mutex::new(None),
    });
    let clock = Arc::new(Wound::default());
    let rules = Arc::new(Builtin::default());
    let actors = Arc::new(MemoryActors::new());

    let lines = LineService::new(
        stale.clone(),
        Arc::new(store.clone()),
        rules.clone(),
        actors.clone(),
        clock.clone(),
    );
    let work_service = PursuitService::new(
        Arc::new(store.clone()),
        stale.clone(),
        Arc::new(store.clone()),
        rules,
        asterism_core::domain::forge::boundary::StoreClient::new(Arc::new(HoldsEverything)),
        actors,
        clock.clone(),
    );

    clock.set(0);
    let line = lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let before_the_move = lines.get(&line.id()).await.unwrap();

    // Two pieces of work from the genesis, on entries of their own.
    clock.set(1);
    let mine = work_service
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    let theirs = work_service
        .open(&line.id(), None, Intent::default(), &who("cyd"))
        .await
        .unwrap();
    for (work, label) in [(&mine, "mine"), (&theirs, "theirs")] {
        work_service
            .push(
                &work.id(),
                vec![Op::add(Content::of(AssetId::new()), name(label))],
                None,
                &who("boro"),
            )
            .await
            .unwrap();
    }

    // Theirs lands. The line is now one change point along.
    clock.set(2);
    work_service
        .close(&theirs.id(), Outcome::Satisfied, None, &who("cyd"))
        .await
        .unwrap();

    // Now arm the stale answer: the next read of the line hands back
    // the genesis-only version, so the close is decided against a line
    // that has moved since — and its first commit is refused.
    *stale.armed.lock().unwrap() = Some(before_the_move);

    clock.set(3);
    work_service
        .close(&mine.id(), Outcome::Satisfied, None, &who("boro"))
        .await
        .expect("the first attempt was refused; the second decided against the line as it is");

    // Both are on, in the order they landed, and deciding again wrote
    // one change point rather than two.
    let line = Lines::get(&store, &line.id()).await.unwrap().unwrap();
    assert_eq!(alive(&line.states()), vec!["mine", "theirs"]);
    let chain = line.history().changes();
    assert_eq!(chain.len(), 2, "one point per close, re-decision and all");
    assert_eq!(chain[0].from(), theirs.id());
    assert_eq!(chain[1].from(), mine.id());
    assert_eq!(chain[1].parent(), chain[0].id());
}

/// Giving up is not refused because somebody else landed first.
///
/// An abandoned close puts nothing on the line, so where the line is
/// has nothing to do with it — the model says as much, refusing an
/// abandoned close for nothing but the wrong line or a second ending.
/// A store that checked the head anyway would refuse work for giving
/// up in the wrong millisecond, and the caller would have no move that
/// helps: there is nothing to re-decide.
#[tokio::test]
async fn an_abandoned_close_survives_a_moved_line_over_memory() {
    an_abandoned_close_survives_a_moved_line(&World::over_memory()).await;
}

#[tokio::test]
async fn an_abandoned_close_survives_a_moved_line_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    an_abandoned_close_survives_a_moved_line(&world).await;
    driver.shutdown().await.unwrap();
}

async fn an_abandoned_close_survives_a_moved_line(world: &World) {
    use asterism_core::domain::forge::model::act::Act;
    use asterism_core::domain::forge::model::closing::close as end_work;
    use asterism_core::domain::forge::model::value::ActorId;

    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    world.clock.set(1);
    let giving_up = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();

    // Somebody else lands, so the genesis is no longer the head.
    world
        .lands(
            &line,
            "cyd",
            vec![Op::add(world.content().await, name("theirs"))],
            2,
        )
        .await;

    let work = world.work.get(&giving_up.id()).await.unwrap();
    let stale = world.lines.get(&line.id()).await.unwrap();
    let ending = end_work(
        &stale,
        &work,
        Outcome::Abandoned,
        None,
        Act::new(at(3), Actor::User(ActorId::new())),
    )
    .expect("giving up is always allowed");
    assert!(!ending.lands());

    // Decided against a line that left the genesis two minutes ago,
    // and handed a decider that would refuse — which is never asked,
    // because an ending that lands nothing has no parent to lose.
    Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        &ending,
        Arc::new(Refuses),
    )
    .await
    .expect("nothing about this ending depends on where the line is");

    let after = world.work.get(&work.id()).await.unwrap();
    assert_eq!(after.outcome(), Some(Outcome::Abandoned));
    assert_eq!(
        world
            .lines
            .get(&line.id())
            .await
            .unwrap()
            .history()
            .changes()
            .len(),
        1,
        "and it still put nothing on the line"
    );
}

/// The forge holds what it names, and the schema is what says so.
///
/// Only asked over SQLite, because it is the only store with a layer
/// below to hold anything: the in-memory one keeps an id and there is
/// nothing underneath it to delete. That asymmetry is the honest one —
/// the guard is a foreign key, and a store with no foreign keys cannot
/// be asked whether it has this one.
#[tokio::test]
async fn an_asset_on_a_line_cannot_be_deleted_and_neither_can_its_persona() {
    let (world, driver) = World::over_sqlite().await;
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    let held = world.content().await;
    world
        .lands(&line, "ana", vec![Op::add(held, name("one"))], 1)
        .await;

    let isle = world.isle.clone().expect("over sqlite");
    let asset = *held.as_uuid();
    let refused = isle
        .call(move |conn| conn.execute("DELETE FROM asset WHERE id = ?1", rusqlite::params![asset]))
        .await;
    assert!(
        refused.is_err(),
        "a line naming bytes somebody deleted is a line lying about the present"
    );

    let persona = *world.persona.as_uuid();
    let owner = isle
        .call(move |conn| {
            conn.execute(
                "DELETE FROM persona WHERE id = ?1",
                rusqlite::params![persona],
            )
        })
        .await;
    assert!(
        owner.is_err(),
        "and the cascade that would take the asset stops at the same edge"
    );

    // The entry going off the line does not release it: the change
    // point that put it there still names it, and bringing the entry
    // back is a verb that needs the content to be there.
    let entry = *line_entries(&world, &line.id())
        .await
        .first()
        .expect("one entry");
    world.lands(&line, "ana", vec![Op::remove(entry)], 2).await;
    let still = isle
        .call(move |conn| conn.execute("DELETE FROM asset WHERE id = ?1", rusqlite::params![asset]))
        .await;
    assert!(
        still.is_err(),
        "taking an entry off releases nothing; only dropping the line does"
    );

    driver.shutdown().await.unwrap();
}

/// Every entry the line has heard of, in a stable order.
async fn line_entries(
    world: &World,
    line: &asterism_core::domain::forge::model::value::LineId,
) -> Vec<EntryId> {
    world
        .lines
        .states(line)
        .await
        .unwrap()
        .keys()
        .copied()
        .collect()
}

/// A second ending and a fork are told apart, and the message says
/// which.
///
/// Both come out of one table, and one of them is a prefix of the
/// other in what SQLite reports — so a substring test reads the fork
/// as the ending. They mean opposite things to a caller: a fork is
/// answered by reading again, and an ending already there is not
/// answered by anything.
///
/// Over SQLite only. The distinction is the schema's, and the
/// in-memory store has no constraint to make it.
#[tokio::test]
async fn a_second_ending_and_a_fork_do_not_read_as_each_other() {
    use asterism_core::domain::forge::closings::Closings;
    use asterism_core::domain::forge::model::act::Act;
    use asterism_core::domain::forge::model::closing::close as end_work;
    use asterism_core::domain::forge::model::value::ActorId;

    let (world, driver) = World::over_sqlite().await;
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("one"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();

    // Decide an ending, land it, then try to land the same one again.
    let held = world.work.get(&work.id()).await.unwrap();
    let current = world.lines.get(&line.id()).await.unwrap();
    let ending = end_work(
        &current,
        &held,
        Outcome::Abandoned,
        None,
        Act::new(at(2), Actor::User(ActorId::new())),
    )
    .unwrap();
    Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        &ending,
        decides(Outcome::Abandoned, 2),
    )
    .await
    .expect("the first ending lands");

    // Handed somebody who refuses to decide again, so the two answers
    // are told apart by which one comes back. A fork is decided again
    // and `Refuses` would say so in its own words; an ending is final
    // and is never asked. Under a substring test the ending's column
    // list matches the fork's, this call asks `Refuses`, and the
    // assertion below fails on its message rather than passing on a
    // phrase nothing produces.
    let again = Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        &ending,
        Arc::new(Refuses),
    )
    .await
    .expect_err("work ends once");
    let said = again.to_string();
    assert!(
        said.contains("already ended"),
        "an ending already there is not a thing to decide again for: {said}"
    );

    driver.shutdown().await.unwrap();
}

/// A close whose change point names a node the line never had is
/// refused, rather than written and never readable again.
///
/// The unique indexes do not catch this: they refuse a parent used
/// twice, not one that was never there. And it is reachable — the
/// port takes a line id and a closing separately, so a closing decided
/// against one line and committed against another names a node the
/// second line has never heard of. The row would go in, and the line
/// would stop being readable from then on, because `restore::chain`
/// walks from the genesis and refuses a history it cannot cover.
///
/// Over SQLite only: the in-memory store is asked the same question in
/// its own test below.
#[tokio::test]
async fn a_close_naming_a_node_the_line_never_had_is_refused_over_sqlite() {
    use asterism_core::domain::forge::closings::Closings;
    use asterism_core::domain::forge::lines::Lines;
    use asterism_core::domain::forge::model::act::Act;
    use asterism_core::domain::forge::model::closing::close as end_work;
    use asterism_core::domain::forge::model::value::ActorId;

    let (world, driver) = World::over_sqlite().await;
    world.clock.set(0);

    // Two lines. Work is opened against the first.
    let mine = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let theirs = world
        .lines
        .open(name("other"), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();

    world.clock.set(1);
    let work = world
        .work
        .open(&mine.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("one"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();

    // Decided against `mine`, so the change point's parent is `mine`'s
    // genesis — a node `theirs` has never had.
    let held = world.work.get(&work.id()).await.unwrap();
    let closing = end_work(
        &world.lines.get(&mine.id()).await.unwrap(),
        &held,
        Outcome::Satisfied,
        None,
        Act::new(at(2), Actor::User(ActorId::new())),
    )
    .unwrap();

    let refused = Closings::commit(
        &*world.closings,
        &theirs.id(),
        &work.id(),
        &closing,
        decides(Outcome::Satisfied, 3),
    )
    .await
    .expect_err("that node is not on this line");
    assert!(
        matches!(refused, DomainError::Validation(_)),
        "not a race — reading again finds the same thing: {refused:?}"
    );

    // And the line it was aimed at is still readable, which is the
    // whole point: nothing was written that the read half cannot turn
    // back into a value.
    Lines::get(&*world.lines_port, &theirs.id())
        .await
        .expect("the line still reads")
        .expect("and is still there");
    assert_eq!(
        world.work.get(&work.id()).await.unwrap().outcome(),
        None,
        "and the refused close left no ending"
    );

    driver.shutdown().await.unwrap();
}

/// A stored node kind the model has no name for is refused rather than
/// guessed at.
///
/// The wildcard this replaces read an unknown `outcome` as
/// `satisfied` — work that gave up coming back as work that landed.
/// A CHECK keeps such a row out of an ordinary write, which is why the
/// coercion looked harmless; the read half is what answers for a
/// database somebody repaired by hand.
#[tokio::test]
async fn a_stored_outcome_this_model_has_no_name_for_is_refused() {
    use asterism_core::domain::forge::pursuits::Pursuits;

    let (world, driver) = World::over_sqlite().await;
    world.clock.set(0);
    let line = world
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    world.clock.set(1);
    let work = world
        .work
        .open(&line.id(), None, Intent::default(), &who("boro"))
        .await
        .unwrap();
    world
        .work
        .push(
            &work.id(),
            vec![Op::add(world.content().await, name("one"))],
            None,
            &who("boro"),
        )
        .await
        .unwrap();
    world
        .work
        .close(&work.id(), Outcome::Abandoned, None, &who("boro"))
        .await
        .unwrap();

    // Reach past the CHECK the way a repair job would, then read.
    let isle = world.isle.clone().expect("over sqlite");
    let id = *work.id().as_uuid();
    isle.call(move |conn| {
        conn.pragma_update(None, "ignore_check_constraints", "ON")?;
        let moved = conn.execute(
            "UPDATE pursuit_node SET outcome = 'finished' \
              WHERE pursuit_id = ?1 AND kind = 'close'",
            rusqlite::params![id],
        )?;
        conn.pragma_update(None, "ignore_check_constraints", "OFF")?;
        assert_eq!(moved, 1, "one ending to edit");
        Ok(())
    })
    .await
    .unwrap();

    let refused = Pursuits::get(&*world.pursuits_port, &work.id()).await;
    assert!(
        refused.is_err(),
        "a value nobody could have written does not come back as one somebody did: \
         {refused:?}"
    );

    driver.shutdown().await.unwrap();
}
