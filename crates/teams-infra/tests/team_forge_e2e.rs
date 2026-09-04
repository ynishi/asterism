//! The forge scenario again, driven through the services and over a
//! team's own database.
//!
//! `asterism-infra`'s `forge_over_ports_e2e` runs this shape over the
//! adapters the local plane has. This runs it over the team's own
//! (#148 decision 20), and what only this one can catch is the two
//! things hosting the forge adds:
//!
//! **That a forge write and its ledger entry are one transaction**
//! (decision 17). Every write here leaves exactly one new row in the
//! team's stream, and a write that is refused leaves neither a forge
//! row nor a ledger row — which is the property that cannot be tested
//! anywhere the two live in different databases.
//!
//! **That the team is a boundary the ports never mention.** Two teams
//! run against one file, and neither can read an id of the other's,
//! write through it, or hang new work or a conversation off it —
//! because no port method takes a team, and the adapter is where the
//! scope lives. Every write path is checked for it, including the two
//! whose ids arrive inside a whole domain value rather than as
//! arguments.

use std::sync::{Arc, Mutex};

use asterism_core::application::forge::{Anchored, LineService, PursuitService, ThreadService};
use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::boundary::{Store, StoreClient};
use asterism_core::domain::forge::clock::Clock;
use asterism_core::domain::forge::lines::Lines;
use asterism_core::domain::forge::model::act::{Act, Actor};
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::op::Op;
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome, Pursuit};
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::table::EntryStates;
use asterism_core::domain::forge::model::thread::{Anchor, Body, Message, Thread};
use asterism_core::domain::forge::model::value::{ActorId, Content, EntryId, LineId, Name};
use asterism_core::domain::forge::pursuits::Pursuits;
use asterism_core::domain::forge::strategies::{Builtin, MainlineFirst};
use asterism_core::domain::forge::threads::Threads;
use asterism_core::domain::value::AssetId;
use asterism_core::error::{ConflictKind, DomainError};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use teams_core::domain::identity::{ActorStamp, LedgerActor, Membership, Role};
use teams_core::domain::ledger::{
    FORGE_LINE_OPENED, FORGE_LINE_RENAMED, FORGE_PURSUIT_CLOSED, FORGE_PURSUIT_OPENED,
    FORGE_ROUND_PUSHED, FORGE_THREAD_OPENED, FORGE_THREAD_SAID, LedgerEvent, SubjectRef,
    TEAM_CREATED,
};
use teams_infra::sqlite::forge::TeamForge;
use teams_infra::sqlite::open_and_migrate_in_memory;
use teams_infra::sqlite::repo::SqliteTeamsRepository;
use uuid::Uuid;

/// A clock somebody winds, so minute 3 is minute 3 and every act it
/// produces is checkable afterwards.
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
    Utc.with_ymd_and_hms(2026, 8, 24, 9, minute, 0).unwrap()
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

fn alive(states: &EntryStates) -> Vec<String> {
    let mut names: Vec<String> = states
        .values()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_ref().map(|n| n.as_str().to_owned()))
        .collect();
    names.sort();
    names
}

/// One team, its forge, and the services wired over it.
struct Team {
    id: Uuid,
    lines: LineService,
    work: PursuitService,
    said: ThreadService,
    clock: Arc<Wound>,
    /// The forge itself, for the checks that ask a port rather than a
    /// service — and for `Store`, which no service exposes.
    forge: TeamForge,
    isle: AsyncIsle,
    repo: SqliteTeamsRepository,
}

impl Team {
    /// A team on an already-open database, with its founding owner and
    /// its stream's first event already in place.
    async fn on(isle: &AsyncIsle, display_name: &str) -> Self {
        let repo = SqliteTeamsRepository::new(isle.clone());
        let id = Uuid::now_v7();
        let owner = Uuid::now_v7();
        let actor = LedgerActor::member(ActorStamp {
            user_id: owner,
            display_name: display_name.into(),
        });
        repo.create_team(
            teams_core::domain::identity::Team::new(id, "a team").expect("a team name"),
            Membership {
                user_id: owner,
                team_id: id,
                role: Role::Owner,
            },
            actor.clone(),
            at(0).timestamp_millis(),
        )
        .await
        .expect("a team");

        let forge = TeamForge::for_request(isle.clone(), id, actor);
        let clock = Arc::new(Wound::default());
        let rules = Arc::new(Builtin::default());
        let lines: Arc<dyn asterism_core::domain::forge::lines::Lines> = Arc::new(forge.clone());
        let pursuits: Arc<dyn asterism_core::domain::forge::pursuits::Pursuits> =
            Arc::new(forge.clone());
        let closings: Arc<dyn asterism_core::domain::forge::closings::Closings> =
            Arc::new(forge.clone());
        let threads: Arc<dyn asterism_core::domain::forge::threads::Threads> =
            Arc::new(forge.clone());
        let actors: Arc<dyn asterism_core::domain::forge::boundary::Actors> =
            Arc::new(forge.clone());

        Self {
            id,
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
                closings,
                rules,
                StoreClient::new(Arc::new(forge.clone())),
                actors.clone(),
                clock.clone(),
            ),
            said: ThreadService::new(threads, pursuits, lines, actors, clock.clone()),
            clock,
            forge,
            isle: isle.clone(),
            repo,
        }
    }

    /// An asset this team has — a `team_asset` row, because
    /// `change_row` and `pursuit_op` both carry a foreign key to one.
    ///
    /// What asks `Store::exists` is `PursuitService`, through its
    /// `StoreClient`, before it builds a round; the adapter asks
    /// nothing and the key is the backstop under it. So content that
    /// is not this team's is refused twice, in two places, and this
    /// helper is what keeps the tests on the near side of both.
    ///
    /// The id comes back rather than only the `Content` wrapping it,
    /// because `Store::exists` is asked about the asset and `Content`
    /// keeps the one it holds to itself.
    async fn asset(&self) -> AssetId {
        let asset = AssetId::new();
        let (id, team_id) = (*asset.as_uuid(), self.id);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO team_asset (id, team_id, created_at) VALUES (?1, ?2, 0)",
                    rusqlite::params![id, team_id],
                )
            })
            .await
            .expect("an asset");
        asset
    }

    /// The same, as the forge's reference to it.
    async fn content(&self) -> Content {
        Content::of(self.asset().await)
    }

    /// The whole of this team's stream.
    async fn stream(&self) -> Vec<LedgerEvent> {
        self.repo
            .events_page(self.id, None, 1000)
            .await
            .expect("a readable stream")
    }

    /// How many entries the stream holds.
    async fn stream_len(&self) -> usize {
        self.stream().await.len()
    }

    /// How many rows of `table` this team has.
    async fn rows(&self, table: &'static str) -> i64 {
        let team_id = self.id;
        self.isle
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE team_id = ?1"),
                    rusqlite::params![team_id],
                    |row| row.get(0),
                )
            })
            .await
            .expect("a count")
    }

    /// Opens work, writes one round, and closes it satisfied — the
    /// whole of what a person doing one small thing does.
    async fn lands(&self, line: &Line, subject: &str, ops: Vec<Op>, minute: u32) {
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
    }
}

/// One database, and however many teams the test wants on it.
async fn hosted() -> (AsyncIsle, AsyncIsleDriver) {
    open_and_migrate_in_memory().await.expect("a database")
}

// ----------------------------------------------------------------------
// Forge behaviour, over the team's own tables.
// ----------------------------------------------------------------------

/// The scenario the local plane's adapters run, run against the
/// team's.
#[tokio::test]
async fn a_line_carries_work_through_to_a_history_that_reads_back() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    let line = team
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");

    // Two people land one entry each, in turn.
    let notes = team.content().await;
    let plan = team.content().await;
    team.lands(
        &line,
        "ana",
        vec![Op::add_to(EntryId::new(), notes, name("notes.md"))],
        3,
    )
    .await;
    team.lands(
        &line,
        "bo",
        vec![Op::add_to(EntryId::new(), plan, name("plan.md"))],
        6,
    )
    .await;

    // The line reads back with both, folded out of the chain rather
    // than out of a column — nothing here keeps a "current" copy.
    let states = team.lines.states(&line.id()).await.expect("a line to fold");
    assert_eq!(alive(&states), vec!["notes.md", "plan.md"]);

    // And the history is a chain the walk can follow: genesis plus one
    // change point per close.
    let read = team.lines.get(&line.id()).await.expect("the line");
    assert_eq!(read.history().changes().len(), 2);

    driver.shutdown().await.unwrap();
}

/// A conversation about work is kept whole and comes back whole.
#[tokio::test]
async fn a_conversation_about_work_is_kept_and_read_back() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    let line = team
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    let work = team
        .work
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .expect("work opens");

    team.clock.set(2);
    let thread = team
        .said
        .open(
            Anchored::Pursuit(work.id()),
            Some(name("about the shape")),
            body("this needs another look"),
            &who("ana"),
        )
        .await
        .expect("a conversation opens");
    team.clock.set(4);
    team.said
        .say(&thread.id(), None, body("agreed"), &who("bo"))
        .await
        .expect("something is said");

    let read = team.said.get(&thread.id()).await.expect("the conversation");
    assert_eq!(read.messages().len(), 2);
    assert_eq!(read.title().map(Name::as_str), Some("about the shape"));

    // And it is findable by what it hangs off.
    let about = team
        .said
        .about(Anchored::Pursuit(work.id()))
        .await
        .expect("threads about the work");
    assert_eq!(about.len(), 1);

    driver.shutdown().await.unwrap();
}

// ----------------------------------------------------------------------
// One write, one ledger row, one transaction.
// ----------------------------------------------------------------------

/// Every forge write appends exactly one entry, and the kinds name the
/// verbs rather than the tables.
#[tokio::test]
async fn every_write_appends_exactly_one_entry_naming_what_was_done() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    // Creating the team is the stream's first entry, so the forge's
    // start from a stream that is already running.
    assert_eq!(team.stream_len().await, 1);

    team.clock.set(0);
    let line = team
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    assert_eq!(team.stream_len().await, 2);

    team.clock.set(1);
    team.lines
        .rename(&line.id(), &name("the-line"), &who("ana"))
        .await
        .expect("a rename");
    assert_eq!(team.stream_len().await, 3);

    team.clock.set(2);
    let work = team
        .work
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .expect("work opens");
    assert_eq!(team.stream_len().await, 4);

    team.clock.set(3);
    let asset = team.content().await;
    team.work
        .push(
            &work.id(),
            vec![Op::add_to(EntryId::new(), asset, name("notes.md"))],
            None,
            &who("ana"),
        )
        .await
        .expect("a round");
    assert_eq!(team.stream_len().await, 5);

    team.clock.set(4);
    team.work
        .close(&work.id(), Outcome::Satisfied, None, &who("ana"))
        .await
        .expect("the close");
    assert_eq!(team.stream_len().await, 6);

    team.clock.set(5);
    let thread = team
        .said
        .open(
            Anchored::Pursuit(work.id()),
            None,
            body("worth remembering"),
            &who("ana"),
        )
        .await
        .expect("a conversation");
    assert_eq!(team.stream_len().await, 7);

    team.clock.set(6);
    team.said
        .say(&thread.id(), None, body("noted"), &who("bo"))
        .await
        .expect("a reply");
    assert_eq!(team.stream_len().await, 8);

    // The stream reads as what happened, in order, and `seq` is
    // gapless across the two namespaces.
    let stream = team.stream().await;
    let kinds: Vec<&str> = stream.iter().map(|event| event.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            TEAM_CREATED,
            FORGE_LINE_OPENED,
            FORGE_LINE_RENAMED,
            FORGE_PURSUIT_OPENED,
            FORGE_ROUND_PUSHED,
            FORGE_PURSUIT_CLOSED,
            FORGE_THREAD_OPENED,
            FORGE_THREAD_SAID,
        ]
    );
    let seqs: Vec<i64> = team
        .stream()
        .await
        .iter()
        .map(|event| event.seq.get())
        .collect();
    assert_eq!(seqs, (1..=8).collect::<Vec<_>>());

    driver.shutdown().await.unwrap();
}

/// A close is one transaction, so it is one entry — the ending and the
/// change point were written together and saying it twice would say two
/// things happened.
#[tokio::test]
async fn a_close_that_moves_the_line_is_still_one_entry() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    let line = team
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");

    let before = team.stream_len().await;
    let asset = team.content().await;
    team.lands(
        &line,
        "ana",
        vec![Op::add_to(EntryId::new(), asset, name("notes.md"))],
        3,
    )
    .await;

    // Open, push, close — three writes, three entries — and the change
    // point rode in on the third.
    assert_eq!(team.stream_len().await - before, 3);
    assert_eq!(team.rows("change_point").await, 1);

    let closed = team
        .stream()
        .await
        .into_iter()
        .find(|event| event.kind.as_str() == FORGE_PURSUIT_CLOSED)
        .expect("the close is in the stream");
    assert_eq!(closed.payload["outcome"], serde_json::json!("satisfied"));
    assert!(
        !closed.payload["change_point"].is_null(),
        "a satisfied close names the point it landed: {}",
        closed.payload
    );
    // Both logs are subjects of it, so a trace from either reaches it.
    assert!(
        closed
            .subjects
            .contains(&SubjectRef::forge_line(*line.id().as_uuid()))
    );

    driver.shutdown().await.unwrap();
}

/// A refused write leaves neither half. This is the property the same
/// transaction exists for.
#[tokio::test]
async fn a_write_that_is_refused_leaves_no_row_and_no_entry() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    team.lines
        .open(name("only-one"), MainlineFirst.id(), &who("ana"))
        .await
        .expect("the first line of that name");

    let lines_before = team.rows("line").await;
    let stream_before = team.stream_len().await;

    // The second line of that name is refused by `UNIQUE (team_id,
    // name)` — the constraint this plane adds, and the write it
    // refuses is a whole transaction.
    let refused = team
        .lines
        .open(name("only-one"), MainlineFirst.id(), &who("bo"))
        .await;
    assert!(
        matches!(
            refused,
            Err(DomainError::Conflict {
                kind: ConflictKind::Clashes,
                ..
            })
        ),
        "a duplicate name is a clash: {refused:?}"
    );

    assert_eq!(team.rows("line").await, lines_before, "no line row landed");
    assert_eq!(
        team.stream_len().await,
        stream_before,
        "and no ledger entry did either"
    );

    // The stream is still gapless: the refused append recomputed
    // nothing, because it never committed.
    let seqs: Vec<i64> = team
        .stream()
        .await
        .iter()
        .map(|event| event.seq.get())
        .collect();
    assert_eq!(seqs, (1..=stream_before as i64).collect::<Vec<_>>());

    driver.shutdown().await.unwrap();
}

/// The trace query the subject index exists for: which entries touched
/// this line, answered without reading a payload.
#[tokio::test]
async fn the_subject_index_answers_which_entries_touched_a_line() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    let line = team
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    let asset = team.content().await;
    team.lands(
        &line,
        "ana",
        vec![Op::add_to(EntryId::new(), asset, name("notes.md"))],
        3,
    )
    .await;

    let touching = team
        .repo
        .events_for_subject(team.id, &SubjectRef::forge_line(*line.id().as_uuid()))
        .await
        .expect("a trace");
    let kinds: Vec<&str> = touching.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            FORGE_LINE_OPENED,
            FORGE_PURSUIT_OPENED,
            FORGE_PURSUIT_CLOSED
        ],
        "opening the line, opening work against it, and the close that moved it"
    );

    // A handle this team minted is a subject too, so the same index
    // crosses from a person to their forge writes.
    let by_ana = team
        .repo
        .events_for_subject(
            team.id,
            &SubjectRef::forge_identity(
                teams_core::domain::ledger::ForgeIdentityRef::subject("ana").unwrap(),
            ),
        )
        .await
        .expect("a trace");
    assert!(
        !by_ana.is_empty(),
        "ana's handle is on the entries her writes appended"
    );

    driver.shutdown().await.unwrap();
}

/// The event records the capacity and the forge node records who —
/// revision 6's two records, two fields.
#[tokio::test]
async fn the_event_carries_the_capacity_and_the_payload_carries_the_handle() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    team.lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");

    let opened = team
        .stream()
        .await
        .into_iter()
        .find(|event| event.kind.as_str() == FORGE_LINE_OPENED)
        .expect("the opening is in the stream");

    // The capacity, stamped with the name as it read at write time.
    assert!(!opened.actor.is_admin());
    assert_eq!(opened.actor.stamp().display_name, "Hoshino");

    // And who, as the forge keeps it: a handle, carrying no capacity
    // at all. The two are separate fields and neither is derived from
    // the other.
    let by = opened.payload["by"].as_str().expect("the handle is named");
    let handle = Uuid::parse_str(by).expect("a handle is a uuid");
    let stands_for: String = isle
        .call(move |conn| {
            conn.query_row(
                "SELECT stands_for FROM forge_actor WHERE id = ?1",
                rusqlite::params![handle],
                |row| row.get(0),
            )
        })
        .await
        .expect("the handle has a row");
    assert_eq!(stands_for, "subject");

    driver.shutdown().await.unwrap();
}

// ----------------------------------------------------------------------
// The team is a boundary, and it is in no signature.
// ----------------------------------------------------------------------

/// Two teams on one database see their own lines and nothing else.
#[tokio::test]
async fn two_teams_on_one_database_cannot_see_each_others_lines() {
    let (isle, driver) = hosted().await;
    let first = Team::on(&isle, "Hoshino").await;
    let second = Team::on(&isle, "Kanda").await;

    first.clock.set(0);
    let theirs = first
        .lines
        .open(name("theirs"), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    second.clock.set(0);
    second
        .lines
        .open(name("ours"), MainlineFirst.id(), &who("bo"))
        .await
        .expect("a line opens");

    // A listing is one team's, and `Lines::list` takes no argument
    // saying so.
    let listed: Vec<String> = first
        .lines
        .list()
        .await
        .expect("a listing")
        .iter()
        .map(|line| line.name().as_str().to_owned())
        .collect();
    assert_eq!(listed, vec!["theirs"]);

    // And an id from the other team reads as nothing rather than as
    // somebody else's line — so a caller holding an id it should not
    // have learns nothing from it.
    assert!(
        Lines::get(&second.forge, &theirs.id())
            .await
            .expect("a read")
            .is_none()
    );
    assert!(second.lines.get(&theirs.id()).await.is_err());

    // Neither can it be written to through the other team's handle.
    second.clock.set(1);
    assert!(
        second
            .lines
            .rename(&theirs.id(), &name("mine-now"), &who("bo"))
            .await
            .is_err()
    );
    assert_eq!(
        first.lines.get(&theirs.id()).await.unwrap().name().as_str(),
        "theirs",
        "the other team's write reached nothing"
    );

    // And their streams are their own, each numbered from one.
    assert_eq!(first.stream_len().await, 2);
    assert_eq!(second.stream_len().await, 2);

    driver.shutdown().await.unwrap();
}

/// The two write paths whose ids arrive inside a whole domain value
/// rather than as arguments, asked directly of the ports.
///
/// A service would refuse these earlier — it reads the line before it
/// opens work, and the anchor before it opens a conversation — so the
/// only way to ask whether the *adapter* holds the boundary is to hand
/// it the value a service would never build. A foreign key is not the
/// answer: it says the row exists, and every row here does.
#[tokio::test]
async fn work_and_conversations_cannot_be_hung_off_another_teams_rows() {
    let (isle, driver) = hosted().await;
    let first = Team::on(&isle, "Hoshino").await;
    let second = Team::on(&isle, "Kanda").await;

    first.clock.set(0);
    let line = first
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    let work = first
        .work
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .expect("work opens");

    let rows_before = second.rows("pursuit").await;
    let threads_before = second.rows("forge_thread").await;
    let stream_before = second.stream_len().await;

    // Work against the other team's line, built by hand and handed
    // straight to the port.
    let poached = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent::default(),
        Act::new(at(1), Actor::User(ActorId::new())),
    );
    let refused = Pursuits::open(&second.forge, &poached).await;
    assert!(
        matches!(refused, Err(DomainError::NotFound { entity: "line", .. })),
        "another team's line is not there to open work against: {refused:?}"
    );

    // A conversation about the other team's work, likewise — and the
    // anchor column this one lands in is a real foreign key, which is
    // exactly why the key is not the check.
    let about_theirs = Thread::open(
        Anchor::Pursuit(work.id()),
        None,
        Message::new(
            None,
            body("mine now"),
            Act::new(at(2), Actor::User(ActorId::new())),
        ),
    );
    let refused = Threads::open(&second.forge, &about_theirs).await;
    assert!(
        matches!(
            refused,
            Err(DomainError::NotFound {
                entity: "pursuit",
                ..
            })
        ),
        "another team's work is not there to remark on: {refused:?}"
    );

    // And one anchored to a bare column, which carries no key at all —
    // the round the other team's work opened at.
    let about_their_round = Thread::open(
        Anchor::Round(work.opening().id()),
        None,
        Message::new(
            None,
            body("nor this"),
            Act::new(at(3), Actor::User(ActorId::new())),
        ),
    );
    let refused = Threads::open(&second.forge, &about_their_round).await;
    assert!(
        matches!(
            refused,
            Err(DomainError::NotFound {
                entity: "round",
                ..
            })
        ),
        "a bare anchor column is checked too: {refused:?}"
    );

    // Nothing landed on either side of the boundary.
    assert_eq!(second.rows("pursuit").await, rows_before);
    assert_eq!(second.rows("forge_thread").await, threads_before);
    assert_eq!(second.stream_len().await, stream_before);
    assert_eq!(
        first.rows("pursuit").await,
        1,
        "and the other team's work is as it was"
    );

    driver.shutdown().await.unwrap();
}

/// The name-uniqueness question the forge leaves to its host, answered
/// per team.
#[tokio::test]
async fn a_name_is_taken_within_a_team_and_free_in_the_next_one() {
    let (isle, driver) = hosted().await;
    let first = Team::on(&isle, "Hoshino").await;
    let second = Team::on(&isle, "Kanda").await;

    first.clock.set(0);
    first
        .lines
        .open(name("shared-name"), MainlineFirst.id(), &who("ana"))
        .await
        .expect("the first line of that name");

    // Taken here…
    first.clock.set(1);
    assert!(
        first
            .lines
            .open(name("shared-name"), MainlineFirst.id(), &who("ana"))
            .await
            .is_err()
    );

    // …and free next door, because the namespace belongs to the team.
    second.clock.set(0);
    second
        .lines
        .open(name("shared-name"), MainlineFirst.id(), &who("bo"))
        .await
        .expect("another team's namespace is its own");

    // A rename into a taken name is refused the same way.
    first.clock.set(2);
    let other = first
        .lines
        .open(name("something-else"), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a second line");
    let stream_before = first.stream_len().await;
    assert!(
        first
            .lines
            .rename(&other.id(), &name("shared-name"), &who("ana"))
            .await
            .is_err()
    );
    assert_eq!(
        first.stream_len().await,
        stream_before,
        "a refused rename records nothing"
    );

    driver.shutdown().await.unwrap();
}

/// `Store::exists` answers about this team's assets and no others.
#[tokio::test]
async fn content_is_real_only_to_the_team_that_has_it() {
    let (isle, driver) = hosted().await;
    let first = Team::on(&isle, "Hoshino").await;
    let second = Team::on(&isle, "Kanda").await;

    let theirs = first.asset().await;
    let nobodys = AssetId::new();

    assert!(first.forge.exists(&theirs).await.expect("an answer"));
    assert!(
        !second.forge.exists(&theirs).await.expect("an answer"),
        "another team's asset is not this team's content"
    );
    assert!(!first.forge.exists(&nobodys).await.expect("an answer"));

    // And the service refuses a round naming content the team does not
    // have, before anything is written.
    first.clock.set(0);
    let line = first
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    let work = first
        .work
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .expect("work opens");
    let stream_before = first.stream_len().await;
    assert!(
        first
            .work
            .push(
                &work.id(),
                vec![Op::add_to(
                    EntryId::new(),
                    Content::of(nobodys),
                    name("ghost.md")
                )],
                None,
                &who("ana"),
            )
            .await
            .is_err()
    );
    assert_eq!(
        first.stream_len().await,
        stream_before,
        "a refused round records nothing"
    );

    driver.shutdown().await.unwrap();
}

/// A handle is minted per team, so one person writing in two teams is
/// two rows and no shared identity.
#[tokio::test]
async fn a_forge_handle_belongs_to_the_team_that_minted_it() {
    let (isle, driver) = hosted().await;
    let first = Team::on(&isle, "Hoshino").await;
    let second = Team::on(&isle, "Kanda").await;

    first.clock.set(0);
    first
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    second.clock.set(0);
    second
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");

    assert_eq!(first.rows("forge_actor").await, 1);
    assert_eq!(second.rows("forge_actor").await, 1);

    let both: i64 = isle
        .call(|conn| {
            conn.query_row(
                "SELECT COUNT(DISTINCT id) FROM forge_actor WHERE stands_for = 'subject'",
                [],
                |row| row.get(0),
            )
        })
        .await
        .expect("a count");
    assert_eq!(both, 2, "one subject token, two teams, two handles");

    driver.shutdown().await.unwrap();
}

/// A line only this team can drop, and dropping it takes the record's
/// place in the stream rather than the record itself.
#[tokio::test]
async fn dropping_a_line_takes_its_rows_and_leaves_the_record() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    let line = team
        .lines
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens");
    let asset = team.content().await;
    team.lands(
        &line,
        "ana",
        vec![Op::add_to(EntryId::new(), asset, name("notes.md"))],
        3,
    )
    .await;

    team.clock.set(9);
    team.lines
        .archive(&line.id(), &who("ana"))
        .await
        .expect("an archive");
    let before = team.stream_len().await;
    team.lines
        .discard(&line.id(), &who("ana"))
        .await
        .expect("a drop");

    assert_eq!(team.rows("line").await, 0);
    assert_eq!(team.rows("pursuit").await, 0);
    assert_eq!(team.rows("change_point").await, 0);
    // The record outlives the rows: one more entry, and every earlier
    // one still there.
    assert_eq!(team.stream_len().await, before + 1);
    let dropped = team.stream().await.pop().expect("the drop");
    assert_eq!(dropped.payload["line"], line.id().as_uuid().to_string());

    // The asset it named is released rather than deleted — dropping
    // the line is what lets go of it, and the row is still there for
    // whoever else has it.
    assert_eq!(team.rows("team_asset").await, 1);

    driver.shutdown().await.unwrap();
}

/// The forge's own refusals still reach the caller as the forge's
/// refusals, not as something the ledger did.
#[tokio::test]
async fn a_line_nobody_has_is_not_found_rather_than_a_ledger_problem() {
    let (isle, driver) = hosted().await;
    let team = Team::on(&isle, "Hoshino").await;

    team.clock.set(0);
    let missing = LineId::new();
    let refused = team
        .lines
        .rename(&missing, &name("whatever"), &who("ana"))
        .await;
    assert!(
        matches!(refused, Err(DomainError::NotFound { entity: "line", .. })),
        "{refused:?}"
    );
    assert_eq!(team.stream_len().await, 1, "and nothing was recorded");

    driver.shutdown().await.unwrap();
}
