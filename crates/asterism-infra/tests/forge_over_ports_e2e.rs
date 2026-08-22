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
use asterism_core::domain::forge::closings::Closings;
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::Actor;
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::op::{Op, OpKind};
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome, Pursuit};
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::table::EntryStates;
use asterism_core::domain::forge::model::value::{Content, EntryId, Name, StrategyId};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::domain::forge::strategies::{Builtin, MainlineFirst};
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::DomainError;
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
    clock: Arc<Wound>,
    /// The ports again, for the tests that ask one directly rather
    /// than through a service — a caller holding a stale value is the
    /// case the port exists for, and a service never produces one.
    lines_port: Arc<dyn Lines>,
    closings: Arc<dyn Closings>,
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
        memory: Option<MemoryForge>,
        isle: Option<AsyncIsle>,
    ) -> Self {
        let clock = Arc::new(Wound::default());
        let rules = Arc::new(Builtin::default());
        let actors = Arc::new(MemoryActors::new());

        Self {
            lines: LineService::new(lines.clone(), rules.clone(), actors.clone(), clock.clone()),
            work: PursuitService::new(
                pursuits,
                lines.clone(),
                closings.clone(),
                rules,
                asterism_core::domain::forge::boundary::StoreClient::new(Arc::new(HoldsEverything)),
                actors,
                clock.clone(),
            ),
            clock,
            lines_port: lines,
            closings,
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
            &world.persona,
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
    let tried = world.content().await;
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
                &world.persona,
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

/// A close aimed at a change point the line has left is refused, and
/// writes nothing.
///
/// The service always aims at the head it just read, so this asks the
/// port directly — which is the layer that has to hold the rule,
/// because a caller holding a stale line is exactly the case it exists
/// for.
#[tokio::test]
async fn a_stale_aim_is_refused_over_memory() {
    a_stale_aim_is_refused(&World::over_memory()).await;
}

#[tokio::test]
async fn a_stale_aim_is_refused_over_sqlite() {
    let (world, driver) = World::over_sqlite().await;
    a_stale_aim_is_refused(&world).await;
    driver.shutdown().await.unwrap();
}

async fn a_stale_aim_is_refused(world: &World) {
    use asterism_core::domain::forge::model::closing::close as end_work;

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
            &world.persona,
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
        asterism_core::domain::forge::model::act::Act::new(
            at(3),
            Actor::User(asterism_core::domain::forge::model::value::ActorId::new()),
        ),
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

    let refused =
        Closings::commit(&*world.closings, &line.id(), &work.id(), genesis, &closing).await;
    assert!(
        matches!(refused, Err(DomainError::Conflict(_))),
        "a close only lands on the head as it is: {refused:?}"
    );

    // And nothing of it was written: the work is still open, and the
    // line still ends where the other close left it.
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
        matches!(refused, Err(DomainError::Conflict(_))),
        "the store cannot argue the model out of its own rule: {refused:?}"
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
/// retry only runs when the line moves *after* the service has read
/// it, which is what `Stale` arranges — it hands the service a line
/// from before the move, once, and then tells the truth.
///
/// Worth pinning because the loop is the only thing standing between
/// "somebody landed while you were deciding" and a refusal the caller
/// would have to understand. Without it the close is correct and
/// useless.
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
    }

    let store = MemoryForge::new();
    let stale = Arc::new(Stale {
        real: store.clone(),
        armed: Mutex::new(None),
    });
    let clock = Arc::new(Wound::default());
    let rules = Arc::new(Builtin::default());
    let actors = Arc::new(MemoryActors::new());
    let persona = PersonaId::new();

    let lines = LineService::new(stale.clone(), rules.clone(), actors.clone(), clock.clone());
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
                &persona,
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

    // Both are on, in the order they landed, and the retry wrote one
    // change point rather than two.
    let line = Lines::get(&store, &line.id()).await.unwrap().unwrap();
    assert_eq!(alive(&line.states()), vec!["mine", "theirs"]);
    let chain = line.history().changes();
    assert_eq!(chain.len(), 2, "one point per close, retry and all");
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
    let genesis = line.head();

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

    // Aimed at the genesis, which the line left two minutes ago.
    Closings::commit(&*world.closings, &line.id(), &work.id(), genesis, &ending)
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
            &world.persona,
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
        current.head(),
        &ending,
    )
    .await
    .expect("the first ending lands");

    let again = Closings::commit(
        &*world.closings,
        &line.id(),
        &work.id(),
        current.head(),
        &ending,
    )
    .await
    .expect_err("work ends once");
    let said = again.to_string();
    assert!(
        said.contains("already ended"),
        "an ending already there is not a thing to read again for: {said}"
    );
    assert!(
        !said.contains("a pass arrived"),
        "and it is not reported as a fork, which is the misreading a \
         substring test makes: {said}"
    );

    driver.shutdown().await.unwrap();
}
