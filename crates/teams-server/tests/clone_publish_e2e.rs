//! End-to-end guard for #153: taking a copy of what a team holds, and
//! seeding a team's line from a private one.
//!
//! ## Why this suite binds a socket, and why it is here
//!
//! Both reasons are `member_client_e2e.rs`'s and its module doc argues
//! them: the **client** is what is under test and the client speaks
//! HTTP, so the server runs on an ephemeral port; and #83 §4 forbids
//! `asterism-* -> teams-*` in any form, so the suite cannot live beside
//! the code it drives.
//!
//! ## Why the private line is built out of the model rather than a
//! store
//!
//! A publication reads a `Line` — its current state for the cheap
//! seeding, its whole chain for the re-enactment. The adapters that
//! keep one are `asterism-infra`'s, which is the first name on §4's
//! never-list and stays off this graph. It does not have to be here:
//! the forge's model mints its own history, so `chain` below opens a
//! line, works against it and lands each change point through the same
//! `closing::close` a service would, with no store under any of it.
//! What that costs is that the private line here has no rows anywhere,
//! which is exactly what a publication does not care about.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asterism_core::domain::asset::Asset;
use asterism_core::domain::asset_comment::CommentAuthor;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::forge::model::act::{Act, Actor};
use asterism_core::domain::forge::model::closing;
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::op::Op;
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome, Pursuit, Round};
use asterism_core::domain::forge::model::value::{ActorId, Content, Name, StrategyId};
use asterism_core::domain::material::Material;
use asterism_core::domain::material_layer::{LayerOrigin, LayerRole, MaterialLayer};
use asterism_core::domain::material_mark::{MaterialAnchor, MaterialMark, TimelineSpan};
use asterism_core::domain::repository::AssetLinkRepository;
use asterism_core::domain::source_locator::LocalPath;
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind, SourceRef};
use asterism_core::error::DomainError;
use asterism_teams_client::TeamsClient;
use asterism_teams_client::clone::{Arrival, CloneRequest, Imports, clone_entry};
use asterism_teams_client::mapper::{LocalSubject, PromotedMark};
use asterism_teams_client::promotion::{Promotion, promote};
use asterism_teams_client::publish::{HeldSubject, Holdings, Publication, Seeding, publish};
use chrono::Utc;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use teams_core::domain::identity::{ActorStamp, LedgerActor, Membership, RegistrationPolicy, Role};
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_infra::sqlite::projection::SqliteProjectionStore;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use uuid::Uuid;

const GOOD: &str = "correct horse battery staple";
const MAINLINE: &str = "mainline-first";

// ----------------------------------------------------------------------
// A server on a port, and members on it. As `member_client_e2e.rs`.
// ----------------------------------------------------------------------

struct Harness {
    ctx: Arc<TeamsCtx>,
    addr: SocketAddr,
    #[allow(dead_code)] // Held so the isle outlives every request.
    isle: AsyncIsle,
    #[allow(dead_code)] // Held so the driver outlives every request.
    driver: AsyncIsleDriver,
    #[allow(dead_code)] // Held so the blob root outlives every request.
    blob_dir: tempfile::TempDir,
}

async fn harness() -> Harness {
    let (isle, driver) = teams_infra::sqlite::open_and_migrate_in_memory()
        .await
        .expect("open in-memory teams db");
    let blob_dir = tempfile::tempdir().expect("blob tempdir");
    let blobs = LocalFileStorageAdapter::open(blob_dir.path().join("blobs"))
        .await
        .expect("open blob store");
    let ctx = Arc::new(TeamsCtx {
        repo: SqliteTeamsRepository::new(isle.clone()),
        auth: PasswordAuth::new(isle.clone()),
        projections: SqliteProjectionStore::new(isle.clone()),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        auth_limiter: RateLimiter::new(1_000, Duration::from_secs(60)),
        purge_grace_ms: 0,
        gc_guard: Arc::new(teams_infra::gc::GcGuard::new()),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("the port it got");
    let router = teams_server::http::router(ctx.clone());
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    Harness {
        ctx,
        addr,
        isle,
        driver,
        blob_dir,
    }
}

/// An account, and a client already logged in as it.
async fn member(h: &Harness, login: &str) -> (Uuid, TeamsClient) {
    let user_id = h
        .ctx
        .auth
        .create_account(login, login, GOOD, false, now_ms())
        .await
        .expect("create account");
    let mut client = TeamsClient::new(format!("http://{}", h.addr));
    client.login(login, GOOD).await.expect("log in");
    (user_id, client)
}

/// Puts a second account on the roster.
async fn join(h: &Harness, team: Uuid, owner: Uuid, joining: Uuid) {
    let owner = h
        .ctx
        .auth
        .account(owner)
        .await
        .expect("read the owner")
        .expect("an owner");
    h.ctx
        .repo
        .add_member(
            Membership {
                user_id: joining,
                team_id: team,
                role: Role::Member,
            },
            LedgerActor::member(ActorStamp {
                user_id: owner.user_id,
                display_name: owner.display_name,
            }),
            now_ms(),
        )
        .await
        .expect("add the second member");
}

// ----------------------------------------------------------------------
// Local assets, and a private line made of them.
// ----------------------------------------------------------------------

/// An Asset with one material on disk, plus the marks a person wrote on
/// it and one an importer wrote (which must not travel).
struct Local {
    asset: Asset,
    user_marks: Vec<PromotedMark>,
    #[allow(dead_code)] // Held so the material file outlives the send.
    dir: tempfile::TempDir,
}

fn local_asset(title: &str, bytes: &[u8], said: &str) -> Local {
    let dir = tempfile::tempdir().expect("a material dir");
    let path = dir.path().join("material.png");
    let mut file = std::fs::File::create(&path).expect("write the material");
    file.write_all(bytes).expect("write the bytes");
    drop(file);

    let locator: asterism_core::domain::source_locator::SourceLocator =
        LocalPath::try_from(path).expect("a rooted path").into();
    let mut asset = Asset::new(
        PersonaId::new(),
        SourceRef::of_locator(
            SourceKind::new(SourceKind::FS).expect("the fs source kind"),
            locator.clone(),
        ),
        None,
        Utc::now(),
        &AttributionContext::asserted(None, None).expect("stating nobody"),
    );
    asset.title = Some(title.to_string());
    asset
        .attach_material(Material::primary(
            locator,
            Some(bytes.len() as u64),
            Utc::now(),
        ))
        .expect("an item may carry a material");

    let mine = MaterialLayer::new(
        asset.id,
        0,
        LayerOrigin::User,
        LayerRole::Annotation,
        false,
        0,
    )
    .expect("a user layer");
    let marks = vec![mark(&mine, said)];
    let user_marks = PromotedMark::gather(&[mine], &marks);

    Local {
        asset,
        user_marks,
        dir,
    }
}

fn mark(layer: &MaterialLayer, said: &str) -> MaterialMark {
    MaterialMark::new(
        layer.asset_id,
        layer.id,
        MaterialAnchor::Temporal(TimelineSpan::new(1_000, Some(2_000)).expect("a span")),
        CommentAuthor::User,
        said,
        Utc::now(),
    )
    .expect("a mark")
}

/// Whoever wrote the private line. A local actor id, which is the thing
/// a re-enactment must be seen not to carry across.
fn privately() -> (ActorId, Act) {
    let who = ActorId::new();
    (who, Act::new(Utc::now(), Actor::User(who)))
}

/// Lands one change point on a line, out of the ops given.
///
/// The long way round on purpose: open work against the line's head,
/// push a round, and let `closing::close` mint the change point. Going
/// through the model's own door is what makes this a history the
/// publication can walk rather than a shape assembled to look like one.
fn land(line: &mut Line, ops: Vec<Op>, act: Act) {
    let mut work = Pursuit::open(line.id(), None, line.head(), Intent::default(), act);
    work.push(Round::new(work.head(), ops, None, act).expect("a round says something"))
        .expect("open work takes a round");
    closing::close(line, &work, Outcome::Satisfied, None, act)
        .expect("the work is reconcilable")
        .apply(line, &mut work)
        .expect("landing it");
}

/// Work opened against the line and given up on. It lands nothing, and
/// a publication must not carry it anywhere.
fn abandon(line: &Line, ops: Vec<Op>, act: Act) -> Pursuit {
    let mut work = Pursuit::open(
        line.id(),
        None,
        line.head(),
        Intent {
            title: Some(Name::new("the idea we dropped").expect("a name")),
            note: Some("this is the private deliberation".to_string()),
        },
        act,
    );
    work.push(Round::new(work.head(), ops, None, act).expect("a round"))
        .expect("open work takes a round");
    closing::close(line, &work, Outcome::Abandoned, None, act)
        .expect("giving up is always reconcilable")
        .apply(&mut line.clone(), &mut work)
        .expect("ending it");
    work
}

// ----------------------------------------------------------------------
// The three ports a caller fills.
// ----------------------------------------------------------------------

/// An in-memory `AssetLinkRepository`. The real one is
/// `asterism-infra`'s and is tested there.
#[derive(Default)]
struct Rows(Mutex<BTreeMap<(Uuid, Uuid, Uuid), AssetLink>>);

impl Rows {
    fn keyed(key: &AssetLinkKey) -> (Uuid, Uuid, Uuid) {
        (
            *key.team_id.as_uuid(),
            *key.line_id.as_uuid(),
            *key.entry_id.as_uuid(),
        )
    }

    fn count(&self) -> usize {
        self.0.lock().expect("the rows").len()
    }
}

#[async_trait::async_trait]
impl AssetLinkRepository for Rows {
    async fn record(&self, link: &AssetLink) -> Result<(), DomainError> {
        self.0
            .lock()
            .expect("the rows")
            .entry(Self::keyed(&link.key))
            .or_insert_with(|| link.clone());
        Ok(())
    }

    async fn list_for_team(&self, team_id: TeamScopedId) -> Result<Vec<AssetLink>, DomainError> {
        Ok(self
            .0
            .lock()
            .expect("the rows")
            .values()
            .filter(|link| link.key.team_id == team_id)
            .cloned()
            .collect())
    }

    async fn for_asset(
        &self,
        team_id: TeamScopedId,
        local_asset_id: &AssetId,
    ) -> Result<Vec<AssetLink>, DomainError> {
        Ok(self
            .0
            .lock()
            .expect("the rows")
            .values()
            .filter(|link| link.key.team_id == team_id && &link.local_asset_id == local_asset_id)
            .cloned()
            .collect())
    }

    async fn dangling_locally(
        &self,
        _team_id: TeamScopedId,
    ) -> Result<Vec<AssetLink>, DomainError> {
        Ok(Vec::new())
    }

    async fn reap(&self, keys: &[AssetLinkKey]) -> Result<u64, DomainError> {
        let mut rows = self.0.lock().expect("the rows");
        let mut removed = 0;
        for key in keys {
            if rows.remove(&Self::keyed(key)).is_some() {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// What a line's content is, on this machine.
#[derive(Default)]
struct Vault(Mutex<BTreeMap<AssetId, (Asset, Vec<PromotedMark>)>>);

impl Vault {
    fn hold(&self, local: &Local) {
        self.0.lock().expect("the vault").insert(
            local.asset.id,
            (local.asset.clone(), local.user_marks.clone()),
        );
    }
}

#[async_trait::async_trait]
impl Holdings for Vault {
    async fn subject(&self, content: AssetId) -> Result<HeldSubject, DomainError> {
        let held = self.0.lock().expect("the vault").get(&content).cloned();
        held.map(|(asset, user_marks)| HeldSubject { asset, user_marks })
            .ok_or_else(|| DomainError::not_found("asset", content))
    }
}

/// The local library a clone lands in.
///
/// A faithful miniature of what `AssetService::add` does with the pair:
/// look it up, hand back what is there, and mint only on a miss. The
/// real one is the local plane's and is tested over a real database in
/// `asterism-infra/tests/clone_source.rs`; what stands in for it here
/// is this, so the clone's *ordering* — ask, fetch, record — is what
/// this suite is looking at.
/// What a clone was recorded as: the id it landed under, how many bytes
/// came with it, and what the promoter had called it.
type Recorded = (AssetId, u64, Option<String>);

/// Keyed the way the real lookup keys it — `(source_kind, locator)`.
type Pair = (String, String);

#[derive(Default)]
struct Library(Mutex<BTreeMap<Pair, Recorded>>);

impl Library {
    fn count(&self) -> usize {
        self.0.lock().expect("the library").len()
    }

    fn only(&self) -> (Pair, Recorded) {
        let rows = self.0.lock().expect("the library");
        assert_eq!(rows.len(), 1, "expected one row, found {rows:?}");
        rows.iter()
            .map(|(pair, row)| (pair.clone(), row.clone()))
            .next()
            .expect("the row")
    }
}

#[async_trait::async_trait]
impl Imports for Library {
    async fn held(&self, source_kind: &str, locator: &str) -> Result<Option<AssetId>, DomainError> {
        Ok(self
            .0
            .lock()
            .expect("the library")
            .get(&(source_kind.to_string(), locator.to_string()))
            .map(|(id, _, _)| *id))
    }

    async fn record(&self, arrival: Arrival<'_>) -> Result<AssetId, DomainError> {
        let mut rows = self.0.lock().expect("the library");
        let row = rows
            .entry((arrival.source_kind.to_string(), arrival.locator.to_string()))
            .or_insert_with(|| {
                (
                    AssetId::new(),
                    arrival.bytes,
                    arrival.cover_hint.map(ToString::to_string),
                )
            });
        Ok(row.0)
    }
}

// ----------------------------------------------------------------------
// Clone (#148 decision 10).
// ----------------------------------------------------------------------

/// The whole copy, and the three things that make it an import.
#[tokio::test]
async fn a_clone_mints_its_own_id_writes_no_relation_row_and_says_where_it_came_from() {
    let h = harness().await;
    let (alice, alice_client) = member(&h, "alice").await;
    let (bob, bob_client) = member(&h, "bob").await;

    let team = TeamScopedId::parse(
        &alice_client
            .create_team(None)
            .await
            .expect("found a team")
            .team_id,
        "team id",
    )
    .unwrap();
    join(&h, *team.as_uuid(), alice, bob).await;

    let line = TeamScopedId::parse(
        &alice_client
            .open_line(team, "the shared line", MAINLINE)
            .await
            .expect("open a line")
            .id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &alice_client
            .open_pursuit(team, line, Some("bringing something in"), None)
            .await
            .expect("open work")
            .id,
        "pursuit id",
    )
    .unwrap();

    // Alice puts something on the line, and lands it — an entry is on a
    // line only once the work that named it is satisfied.
    let mine = local_asset(
        "What Alice called it",
        b"the bytes of the thing",
        "chapter 2 is wrong",
    );
    let hers = Rows::default();
    let promoted = promote(
        &alice_client,
        &hers,
        Promotion {
            team_id: team,
            line_id: line,
            pursuit_id: pursuit,
            subject: LocalSubject {
                asset: &mine.asset,
                user_marks: &mine.user_marks,
            },
            named: "the-thing.png",
        },
        now_ms(),
    )
    .await
    .expect("promote");
    alice_client
        .close_pursuit(team, pursuit, "satisfied", None)
        .await
        .expect("land it");

    // Bob takes a copy.
    let root = tempfile::tempdir().expect("a clone root");
    let library = Library::default();
    let his = Rows::default();
    let persona = PersonaId::new();
    let cloned = clone_entry(
        &bob_client,
        &library,
        CloneRequest {
            team_id: team,
            line_id: line,
            entry_id: promoted.key.entry_id,
            persona_id: &persona,
            root: root.path(),
        },
        Utc::now(),
    )
    .await
    .expect("clone");

    // New ids: the copy is not Alice's asset, and it is not the team's.
    assert!(!cloned.already_held);
    assert_ne!(
        cloned.asset_id, mine.asset.id,
        "the copy minted its own id rather than carrying the one it was copied from"
    );
    assert_eq!(cloned.team_asset_id, promoted.team_asset_id.unwrap());

    // No relation row. A row means "I put this there", and Bob did not.
    assert_eq!(
        his.count(),
        0,
        "a clone wrote a relation row, and only a promotion may"
    );

    // It says where it came from, the way every other import does.
    let ((kind, locator), (recorded, bytes, cover)) = library.only();
    assert_eq!(kind, SourceKind::TEAM_LINE);
    assert_eq!(recorded, cloned.asset_id);
    assert_eq!(bytes, b"the bytes of the thing".len() as u64);
    for id in [
        team.to_string(),
        line.to_string(),
        promoted.key.entry_id.to_string(),
        promoted.team_asset_id.unwrap().to_string(),
    ] {
        assert!(locator.contains(&id), "{locator} does not name {id}");
    }
    assert!(
        locator.ends_with(".png"),
        "the extension the line's name carried is what classifies the copy: {locator}"
    );

    // The bytes really arrived, and what the promoter said came with
    // them.
    assert_eq!(cloned.bytes, Some(b"the bytes of the thing".len() as u64));
    assert_eq!(
        std::fs::read(&cloned.locator).expect("the copy is on disk"),
        b"the bytes of the thing"
    );
    let view = cloned.projection.expect("the promoter said something");
    assert_eq!(view.title.as_deref(), Some("What Alice called it"));
    assert_eq!(cover.as_deref(), Some("What Alice called it"));
}

/// The existing duplicate machinery is asked, and answers before a byte
/// moves.
#[tokio::test]
async fn cloning_the_same_entry_twice_is_answered_from_what_is_already_here() {
    let h = harness().await;
    let (alice, alice_client) = member(&h, "alice").await;
    let (bob, bob_client) = member(&h, "bob").await;

    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();
    join(&h, *team.as_uuid(), alice, bob).await;
    let line = TeamScopedId::parse(
        &alice_client
            .open_line(team, "the shared line", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &alice_client
            .open_pursuit(team, line, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();

    let mine = local_asset("once", b"one copy of this", "said");
    let hers = Rows::default();
    let promoted = promote(
        &alice_client,
        &hers,
        Promotion {
            team_id: team,
            line_id: line,
            pursuit_id: pursuit,
            subject: LocalSubject {
                asset: &mine.asset,
                user_marks: &mine.user_marks,
            },
            named: "once.png",
        },
        now_ms(),
    )
    .await
    .unwrap();
    alice_client
        .close_pursuit(team, pursuit, "satisfied", None)
        .await
        .unwrap();

    let root = tempfile::tempdir().expect("a clone root");
    let library = Library::default();
    let persona = PersonaId::new();
    let request = CloneRequest {
        team_id: team,
        line_id: line,
        entry_id: promoted.key.entry_id,
        persona_id: &persona,
        root: root.path(),
    };

    let first = clone_entry(&bob_client, &library, request, Utc::now())
        .await
        .expect("the first copy");
    let again = clone_entry(&bob_client, &library, request, Utc::now())
        .await
        .expect("the second ask");

    assert!(!first.already_held);
    assert!(again.already_held, "the second clone was not recognised");
    assert_eq!(
        again.asset_id, first.asset_id,
        "the second ask minted a second copy of one thing"
    );
    assert_eq!(again.bytes, None, "the second ask fetched bytes it had");
    assert_eq!(library.count(), 1, "one thing, one row");
    assert_eq!(
        first.locator, again.locator,
        "the locator is what the duplicate machinery compares, so it has to be the same \
         string both times"
    );
}

// ----------------------------------------------------------------------
// Publish (#148 decision 11).
// ----------------------------------------------------------------------

/// A private line with two change points and one abandoned pursuit.
///
/// Change point one adds `a`. Change point two replaces `a`'s content
/// and adds `b`. So the line holds three contents and presents two
/// entries — which is what makes the two seedings cost visibly
/// different amounts.
struct Private {
    line: Line,
    vault: Vault,
    who: ActorId,
    #[allow(dead_code)] // Held so every material file outlives the send.
    assets: Vec<Local>,
}

fn private_line() -> Private {
    let (who, act) = privately();
    let mut line = Line::open(
        Name::new("my own line").expect("a name"),
        StrategyId::new(MAINLINE).expect("a rule"),
        act,
    );

    let first = local_asset("the first take", b"first bytes", "not happy with this");
    let second = local_asset("the second take", b"second bytes", "better");
    let other = local_asset("something else", b"other bytes", "unrelated");
    let dropped = local_asset("never landed", b"dropped bytes", "we thought better of it");

    let vault = Vault::default();
    for local in [&first, &second, &other, &dropped] {
        vault.hold(local);
    }

    let entry = asterism_core::domain::forge::model::value::EntryId::new();
    land(
        &mut line,
        vec![Op::add_to(
            entry,
            Content::of(first.asset.id),
            Name::new("a.png").expect("a name"),
        )],
        act,
    );
    land(
        &mut line,
        vec![
            Op::replace(entry, Content::of(second.asset.id)),
            Op::add(
                Content::of(other.asset.id),
                Name::new("b.png").expect("a name"),
            ),
        ],
        act,
    );

    // Deliberation that went nowhere. It is on this machine and must
    // stay here (#66 decision 2).
    let _ = abandon(
        &line,
        vec![Op::add(
            Content::of(dropped.asset.id),
            Name::new("c.png").expect("a name"),
        )],
        act,
    );

    Private {
        line,
        vault,
        who,
        assets: vec![first, second, other, dropped],
    }
}

/// The default seeding: what the line holds now, and nothing about how
/// it got there.
#[tokio::test]
async fn a_line_published_as_it_stands_is_a_genesis_and_one_change_point() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();

    let private = private_line();
    let links = Rows::default();
    let published = publish(
        &alice_client,
        &links,
        &private.vault,
        Publication {
            team_id: team,
            line: &private.line,
            named: "what we are working from",
            strategy_id: MAINLINE,
            seeding: Seeding::CurrentState,
        },
        now_ms(),
    )
    .await
    .expect("publish");

    assert!(!published.reenacted);
    assert_eq!(published.change_points, 1);
    assert_eq!(
        published.contents_sent, 2,
        "the current state is two entries, so two contents — not the three the line holds"
    );

    let history = alice_client
        .line_history(team, published.line_id)
        .await
        .expect("read it back");
    assert_eq!(
        history.changes.len(),
        1,
        "the team's line got one change point whatever the private line's history was"
    );

    let states = alice_client
        .line_states(team, published.line_id)
        .await
        .expect("what is on it");
    let mut names: Vec<&str> = states
        .iter()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_deref())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["a.png", "b.png"]);

    assert_eq!(
        links.count(),
        2,
        "the publication put both entries there, and a row says so"
    );
}

/// The option at init: the chain replayed, and every word of what that
/// means.
#[tokio::test]
async fn a_re_enacted_line_replays_the_chain_restamps_the_acts_and_leaves_the_work_at_home() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();

    let private = private_line();
    let links = Rows::default();
    let published = publish(
        &alice_client,
        &links,
        &private.vault,
        Publication {
            team_id: team,
            line: &private.line,
            named: "the whole story",
            strategy_id: MAINLINE,
            seeding: Seeding::Reenactment,
        },
        now_ms(),
    )
    .await
    .expect("publish");

    assert!(published.reenacted, "the word is what the caller reports");
    assert_eq!(published.change_points, 2, "one per change point, replayed");
    assert_eq!(
        published.contents_sent, 3,
        "a re-enactment sends every content the line ever named, including the one that \
         was replaced — this is the cost the doc states"
    );

    let history = alice_client
        .line_history(team, published.line_id)
        .await
        .expect("read it back");
    assert_eq!(history.changes.len(), 2);

    // Restamped. Every landing on the team's line is the publisher's,
    // and the actor who wrote the private line appears nowhere: the
    // team plane has no handle for them, and inventing one would be the
    // team's record claiming what it does not know.
    let stamps: Vec<&str> = history
        .changes
        .iter()
        .map(|point| point.actor_id.as_str())
        .collect();
    assert_eq!(stamps[0], stamps[1], "two landings, one publisher");
    assert!(
        !stamps.contains(&private.who.to_string().as_str()),
        "the private line's actor crossed: {stamps:?}"
    );
    for point in &history.changes {
        assert_eq!(point.actor_kind, "user");
    }

    // The chain says what the private one said: the entry added first
    // is the entry replaced second, rather than a second entry beside
    // it.
    let added: Vec<&str> = history.changes[0]
        .table
        .iter()
        .map(|row| row.entry_id.as_str())
        .collect();
    assert_eq!(added.len(), 1);
    assert!(
        history.changes[1]
            .table
            .iter()
            .any(|row| row.entry_id == added[0]),
        "the replacement landed on a different entry than the add"
    );

    let states = alice_client
        .line_states(team, published.line_id)
        .await
        .expect("what is on it");
    let alive: Vec<&str> = states
        .iter()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_deref())
        .collect();
    assert_eq!(alive.len(), 2, "{alive:?}");

    // The work logs stayed home. Two pursuits, one per change point,
    // and nothing carrying what the abandoned one was called.
    let work = alice_client
        .pursuits_of_line(team, published.line_id)
        .await
        .expect("the work on it");
    assert_eq!(
        work.len(),
        2,
        "a pursuit crossed that was not one of the two re-enactments"
    );
    for one in &work {
        assert_eq!(
            one.close.as_ref().map(|c| c.outcome.as_str()),
            Some("satisfied")
        );
        assert_ne!(one.title.as_deref(), Some("the idea we dropped"));
        assert_ne!(
            one.note.as_deref(),
            Some("this is the private deliberation")
        );
    }
    let said = format!("{work:?}");
    assert!(
        !said.contains("the idea we dropped") && !said.contains("private deliberation"),
        "the abandoned work travelled"
    );

    // And the rounds say the word, so a reader of the team's line is
    // told what they are looking at.
    assert!(
        work.iter()
            .flat_map(|one| one.rounds.iter())
            .any(|round| round.note.as_deref() == Some("re-enacted from a private line")),
        "no round says it was re-enacted"
    );

    assert_eq!(
        links.count(),
        2,
        "the correspondence is recorded once per entry the line ends up holding, not once \
         per time it was written"
    );
    let rows = links.list_for_team(team).await.unwrap();
    assert!(
        rows.iter()
            .any(|row| row.local_asset_id == private.assets[1].asset.id),
        "the row for the replaced entry names what it holds now, not what it held first"
    );
    assert!(
        !rows
            .iter()
            .any(|row| row.local_asset_id == private.assets[0].asset.id),
        "a row still names the content that was replaced"
    );
}

/// The clone's own path, taken against a line that was published rather
/// than promoted onto — the two halves of #153 meeting.
#[tokio::test]
async fn what_a_publication_seeded_is_what_a_clone_takes_back() {
    let h = harness().await;
    let (alice, alice_client) = member(&h, "alice").await;
    let (bob, bob_client) = member(&h, "bob").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();
    join(&h, *team.as_uuid(), alice, bob).await;

    let private = private_line();
    let links = Rows::default();
    let published = publish(
        &alice_client,
        &links,
        &private.vault,
        Publication {
            team_id: team,
            line: &private.line,
            named: "shared",
            strategy_id: MAINLINE,
            seeding: Seeding::CurrentState,
        },
        now_ms(),
    )
    .await
    .expect("publish");

    let states = bob_client
        .line_states(team, published.line_id)
        .await
        .expect("bob reads it through, rather than mirroring it");
    let entry = states
        .iter()
        .find(|state| state.alive && state.name.as_deref() == Some("a.png"))
        .expect("the entry alice seeded");

    let root = tempfile::tempdir().expect("a clone root");
    let library = Library::default();
    let persona = PersonaId::new();
    let cloned = clone_entry(
        &bob_client,
        &library,
        CloneRequest {
            team_id: team,
            line_id: published.line_id,
            entry_id: TeamScopedId::parse(&entry.entry_id, "entry id").unwrap(),
            persona_id: &persona,
            root: root.path(),
        },
        Utc::now(),
    )
    .await
    .expect("clone what was published");

    assert_eq!(
        std::fs::read(&cloned.locator).expect("on disk"),
        b"second bytes",
        "the copy is what the entry holds now, which is the replacement"
    );
    assert_eq!(
        cloned.projection.and_then(|view| view.title),
        Some("the second take".to_string())
    );
}

/// The one shape a clone refuses, and the reason it is not an error
/// worth guessing past.
#[tokio::test]
async fn an_entry_the_line_took_off_is_not_something_to_copy() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();

    let private = private_line();
    let links = Rows::default();
    let published = publish(
        &alice_client,
        &links,
        &private.vault,
        Publication {
            team_id: team,
            line: &private.line,
            named: "shared",
            strategy_id: MAINLINE,
            seeding: Seeding::CurrentState,
        },
        now_ms(),
    )
    .await
    .expect("publish");

    // Take one off.
    let states = alice_client
        .line_states(team, published.line_id)
        .await
        .unwrap();
    let entry = states
        .iter()
        .find(|state| state.alive && state.name.as_deref() == Some("b.png"))
        .expect("it is on the line");
    let pursuit = TeamScopedId::parse(
        &alice_client
            .open_pursuit(team, published.line_id, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();
    alice_client
        .push_round(
            team,
            pursuit,
            vec![asterism_contract::forge::ForgeOpDto {
                entry_id: entry.entry_id.clone(),
                kind: "remove".to_string(),
                content_asset_id: None,
                name: None,
            }],
            None,
            Vec::new(),
        )
        .await
        .expect("ask for it to come off");
    alice_client
        .close_pursuit(team, pursuit, "satisfied", None)
        .await
        .expect("land it");

    let root = tempfile::tempdir().expect("a clone root");
    let library = Library::default();
    let persona = PersonaId::new();
    let refused = clone_entry(
        &alice_client,
        &library,
        CloneRequest {
            team_id: team,
            line_id: published.line_id,
            entry_id: TeamScopedId::parse(&entry.entry_id, "entry id").unwrap(),
            persona_id: &persona,
            root: root.path(),
        },
        Utc::now(),
    )
    .await;

    let message = refused
        .expect_err("cloning what is not on the line")
        .to_string();
    assert!(
        message.contains("taken off"),
        "the refusal does not say what is wrong: {message}"
    );
    assert_eq!(library.count(), 0, "a refused clone recorded something");
}

/// What a publication refuses, and that it refuses it before the team
/// has a line.
///
/// The order is the claim. A refusal met half-way through leaves a line
/// on somebody else's team holding some of what was meant for it, and
/// unlike a local line that is not the publisher's to tidy away — so
/// everything answerable from the private line alone is answered first.
#[tokio::test]
async fn a_line_that_cannot_be_seeded_is_refused_before_the_team_has_one() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();

    // A line holding nothing. Its only entry was taken back off, so a
    // change point seeded from it would say nothing, which the model
    // refuses — after the line existed, if nothing asked first.
    let (_who, act) = privately();
    let mut empty = Line::open(
        Name::new("emptied").expect("a name"),
        StrategyId::new(MAINLINE).expect("a rule"),
        act,
    );
    let only = local_asset("gone", b"gone bytes", "said");
    let vault = Vault::default();
    vault.hold(&only);
    let entry = asterism_core::domain::forge::model::value::EntryId::new();
    land(
        &mut empty,
        vec![Op::add_to(
            entry,
            Content::of(only.asset.id),
            Name::new("a.png").expect("a name"),
        )],
        act,
    );
    land(&mut empty, vec![Op::remove(entry)], act);

    let links = Rows::default();
    let refused = publish(
        &alice_client,
        &links,
        &vault,
        Publication {
            team_id: team,
            line: &empty,
            named: "nothing to give",
            strategy_id: MAINLINE,
            seeding: Seeding::CurrentState,
        },
        now_ms(),
    )
    .await
    .expect_err("a line holding nothing seeds nothing");
    assert!(refused.to_string().contains("holds nothing"), "{refused}");

    // A second live entry on the same content. A promotion's repeat
    // check would answer it from the first, and the team would receive
    // one entry where the private line has two — so it is refused
    // rather than silently narrowed.
    let mut shared = Line::open(
        Name::new("twice").expect("a name"),
        StrategyId::new(MAINLINE).expect("a rule"),
        act,
    );
    let once = local_asset("once", b"once bytes", "said");
    let twice = Vault::default();
    twice.hold(&once);
    land(
        &mut shared,
        vec![
            Op::add(
                Content::of(once.asset.id),
                Name::new("a.png").expect("a name"),
            ),
            Op::add(
                Content::of(once.asset.id),
                Name::new("also-a.png").expect("a name"),
            ),
        ],
        act,
    );

    let refused = publish(
        &alice_client,
        &links,
        &twice,
        Publication {
            team_id: team,
            line: &shared,
            named: "twice over",
            strategy_id: MAINLINE,
            seeding: Seeding::CurrentState,
        },
        now_ms(),
    )
    .await
    .expect_err("two live entries on one content");
    assert!(refused.to_string().contains("same content"), "{refused}");

    // Neither refusal left a line behind.
    let lines = alice_client.lines(team).await.expect("the team's lines");
    assert!(
        lines.is_empty(),
        "a refused publication opened a line anyway: {lines:?}"
    );

    // And the re-enactment does take the shape the current state
    // refused, because its chain names each entry in its own right.
    let published = publish(
        &alice_client,
        &links,
        &twice,
        Publication {
            team_id: team,
            line: &shared,
            named: "twice over, replayed",
            strategy_id: MAINLINE,
            seeding: Seeding::Reenactment,
        },
        now_ms(),
    )
    .await
    .expect("re-enacting a line whose entries share content");
    let states = alice_client
        .line_states(team, published.line_id)
        .await
        .expect("what is on it");
    assert_eq!(
        states.iter().filter(|s| s.alive).count(),
        2,
        "the re-enactment lost one of the two entries: {states:?}"
    );
}

/// The two histories the first re-enactment refused.
///
/// A row states only the axes its change point moved, so two ordinary
/// shapes carry fewer axes than a naive reading expects. A **revival**
/// — `Op::add_to` putting an entry the line took off back on — folds to
/// a row stating existence alone, because the content and name it
/// restates already match the head and `normalise` drops them. And a
/// pursuit that both **replaces and renames** one entry folds to a row
/// stating neither existence, because `op::fold` emits one row per
/// entry however many ops touched it.
///
/// Matching a row's axes against a fixed set of four shapes refuses
/// both, and tells the publisher their line is malformed. It is not.
#[tokio::test]
async fn a_revival_and_a_replace_with_a_rename_re_enact() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();

    let (_who, act) = privately();
    let mut line = Line::open(
        Name::new("the awkward line").expect("a name"),
        StrategyId::new(MAINLINE).expect("a rule"),
        act,
    );
    let first = local_asset("first", b"first bytes", "said");
    let second = local_asset("second", b"second bytes", "said");
    let vault = Vault::default();
    vault.hold(&first);
    vault.hold(&second);

    let entry = asterism_core::domain::forge::model::value::EntryId::new();
    // 1 — add it.
    land(
        &mut line,
        vec![Op::add_to(
            entry,
            Content::of(first.asset.id),
            Name::new("a.png").expect("a name"),
        )],
        act,
    );
    // 2 — replace and rename in one pursuit, which folds to one row
    //     stating content and name and no existence.
    land(
        &mut line,
        vec![
            Op::replace(entry, Content::of(second.asset.id)),
            Op::rename(entry, Name::new("renamed.png").expect("a name")),
        ],
        act,
    );
    // 3 — take it off.
    land(&mut line, vec![Op::remove(entry)], act);
    // 4 — put it back, restating what it already held. `normalise`
    //     drops both axes, leaving existence alone.
    land(
        &mut line,
        vec![Op::add_to(
            entry,
            Content::of(second.asset.id),
            Name::new("renamed.png").expect("a name"),
        )],
        act,
    );

    // Before publishing anything: check this line really does carry the
    // two shapes, so that a change to `normalise` cannot quietly leave
    // this test exercising the easy path while still passing.
    let shapes: Vec<(bool, bool, bool)> = line
        .history()
        .changes()
        .iter()
        .map(|point| {
            let row = point.table().rows().values().next().expect("one row");
            (
                row.existence().is_some(),
                row.content().is_some(),
                row.name().is_some(),
            )
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            (true, true, true),   // 1 — the add
            (false, true, true),  // 2 — replace and rename, no existence
            (true, false, false), // 3 — the removal
            (true, false, false), // 4 — the revival, existence alone
        ],
        "this line no longer carries the shapes the test is about"
    );

    let links = Rows::default();
    let published = publish(
        &alice_client,
        &links,
        &vault,
        Publication {
            team_id: team,
            line: &line,
            named: "the awkward line",
            strategy_id: MAINLINE,
            seeding: Seeding::Reenactment,
        },
        now_ms(),
    )
    .await
    .expect("a revival and a combined replace/rename are ordinary history");

    assert_eq!(published.change_points, 4);

    // The entry came back as itself, not as a second entry beside the
    // first, and it came back holding what it held.
    let states = alice_client
        .line_states(team, published.line_id)
        .await
        .expect("what is on it");
    let alive: Vec<_> = states.iter().filter(|s| s.alive).collect();
    assert_eq!(
        alive.len(),
        1,
        "the revival added a second entry rather than reviving the first: {states:?}"
    );
    assert_eq!(alive[0].name.as_deref(), Some("renamed.png"));

    let history = alice_client
        .line_history(team, published.line_id)
        .await
        .expect("read it back");
    let touched: Vec<&str> = history
        .changes
        .iter()
        .flat_map(|point| point.table.iter())
        .map(|row| row.entry_id.as_str())
        .collect();
    assert!(
        touched.windows(2).all(|pair| pair[0] == pair[1]),
        "one local entry became more than one team entry: {touched:?}"
    );
}

/// A clone of an entry that is not there at all.
#[tokio::test]
async fn cloning_an_entry_the_line_never_had_is_refused() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let team = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();
    let line = TeamScopedId::parse(
        &alice_client
            .open_line(team, "empty", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();

    let root = tempfile::tempdir().expect("a clone root");
    let library = Library::default();
    let persona = PersonaId::new();
    let refused = clone_entry(
        &alice_client,
        &library,
        CloneRequest {
            team_id: team,
            line_id: line,
            entry_id: TeamScopedId::new(),
            persona_id: &persona,
            root: root.path(),
        },
        Utc::now(),
    )
    .await;

    assert!(
        refused
            .expect_err("there is no such entry")
            .to_string()
            .contains("no entry"),
    );
    assert_eq!(library.count(), 0);
}
