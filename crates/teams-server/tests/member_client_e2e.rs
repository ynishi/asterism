//! End-to-end guard for the member's half of #148: a promotion from a
//! client's machine landing on a team-hosted line, what the ledger
//! says about it, what a second member reads back, and what the
//! relation does when the team end vanishes.
//!
//! ## Why this suite binds a socket
//!
//! Every other suite here drives the router through `oneshot`, which
//! is right when the router is what is under test. Here the **client**
//! is under test, and the client speaks HTTP — it builds URLs, sets
//! `Authorization`, streams a body, and reads the house error shape
//! back off the wire. Handing it a `Router` would test everything
//! except the part that is new. So the server runs on an ephemeral
//! port and the client talks to it the way it would talk to a
//! deployment.
//!
//! ## Why it lives in this crate
//!
//! Because it cannot live in the other one: #83 §4 forbids
//! `asterism-* -> teams-*` in any form, so `asterism-teams-client`
//! cannot take `teams-server` even as a dev-dependency. The direction
//! that is permitted is argued where the dependency is declared, in
//! this crate's manifest.
//!
//! The relation's **adapter** is tested where it lives, over a real
//! database, in `asterism-infra/tests/team_asset_link.rs`. What stands
//! in for it here is a fake behind the same port.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asterism_core::domain::asset::Asset;
use asterism_core::domain::asset_comment::CommentAuthor;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::material::Material;
use asterism_core::domain::material_layer::{LayerOrigin, LayerRole, MaterialLayer};
use asterism_core::domain::material_mark::{MaterialAnchor, MaterialMark, TimelineSpan};
use asterism_core::domain::repository::AssetLinkRepository;
use asterism_core::domain::source_locator::LocalPath;
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind, SourceRef};
use asterism_core::error::DomainError;
use asterism_teams_client::link::{Missing, reap, verify};
use asterism_teams_client::mapper::{LocalSubject, PromotedMark, read_projection_body};
use asterism_teams_client::promotion::{Promotion, promote};
use asterism_teams_client::{TeamsClient, TeamsClientError};
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
// A server on a port, and two members on it.
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
        oidc: None,
        projections: SqliteProjectionStore::new(isle.clone()),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        device_token_ttl_ms: teams_server::state::DEFAULT_DEVICE_TOKEN_TTL_MS,
        device_token_idle_ms: None,
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
///
/// Straight at the repository rather than over the invite route. The
/// client speaks that route since #210, so this is a fixture's
/// shortcut rather than the only way in: the suites that call this are
/// about what a member does once a roster holds them, not about how it
/// came to.
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
// A local Asset to promote.
// ----------------------------------------------------------------------

/// An Asset with one material on disk, plus the marks a person wrote
/// on it and one an importer wrote (which must not travel).
struct Local {
    asset: Asset,
    user_marks: Vec<PromotedMark>,
    #[allow(dead_code)] // Held so the material file outlives the promotion.
    dir: tempfile::TempDir,
}

fn local_asset(title: &str, bytes: &[u8], said: &str) -> Local {
    let dir = tempfile::tempdir().expect("a material dir");
    let path = dir.path().join("material.bin");
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
    let theirs = MaterialLayer::new(
        asset.id,
        0,
        LayerOrigin::Imported,
        LayerRole::Annotation,
        false,
        1,
    )
    .expect("an imported layer");
    let marks = vec![mark(&mine, said), mark(&theirs, "what the container said")];
    let user_marks = PromotedMark::gather(&[mine, theirs], &marks);

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

// ----------------------------------------------------------------------
// The relation, behind its port.
// ----------------------------------------------------------------------

/// An in-memory `AssetLinkRepository`.
///
/// The real one is `asterism-infra`'s and is tested there. This stands
/// in so the promotion's *ordering* — content, round, link — is under
/// test here without dragging the crate §4 names by name into this
/// graph.
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
        // Nothing is ever deleted from this fake, so the local end
        // never dangles here. That half is `asterism-infra`'s test.
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

// ----------------------------------------------------------------------
// The tests.
// ----------------------------------------------------------------------

/// The whole act, once, with a second member reading it back.
#[tokio::test]
async fn a_promotion_lands_and_a_second_member_reads_it() {
    let h = harness().await;
    let (alice, alice_client) = member(&h, "alice").await;
    let (bob, bob_client) = member(&h, "bob").await;

    let team_id = alice_client
        .create_team(None)
        .await
        .expect("found a team")
        .team_id;
    let team = TeamScopedId::parse(&team_id, "team id").unwrap();
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

    let local = local_asset(
        "What Alice called it",
        b"the bytes of the thing",
        "chapter 2 is wrong",
    );
    let links = Rows::default();
    let outcome = promote(
        &alice_client,
        &links,
        Promotion {
            team_id: team,
            line_id: line,
            pursuit_id: pursuit,
            subject: LocalSubject {
                asset: &local.asset,
                user_marks: &local.user_marks,
            },
            named: "the-thing",
        },
        now_ms(),
    )
    .await
    .expect("promote");

    assert!(!outcome.already_promoted);
    assert_eq!(
        outcome.bytes_already_held,
        Some(false),
        "the question was put, and nothing had been sent to this team yet"
    );
    let team_asset = outcome.team_asset_id.expect("a team asset was minted");
    assert_eq!(links.count(), 1, "the promotion left one row at home");

    // The round named the entry, and the entry is on the work.
    let pushed = outcome.pursuit.expect("the round came back");
    let named: Vec<&str> = pushed
        .rounds
        .iter()
        .flat_map(|round| round.ops.iter())
        .map(|op| op.entry_id.as_str())
        .collect();
    assert!(named.contains(&outcome.key.entry_id.to_string().as_str()));

    // The team holds the conversion, and says what it was converted
    // from.
    let resolved = bob_client
        .resolve_content(team, &[team_asset])
        .await
        .expect("resolve");
    assert_eq!(resolved.held.len(), 1, "{resolved:?}");
    assert_eq!(
        resolved.held[0].digest.as_deref(),
        Some(outcome.digest.as_str())
    );
    assert!(resolved.unknown.is_empty());

    // The second member reads the projection. The body crossed as an
    // opaque string and is opened by the one thing allowed to open one
    // — the mapper, branching on the version the body carries.
    let seen = bob_client
        .entry_projection(team, line, outcome.key.entry_id)
        .await
        .expect("read the projection")
        .expect("there is one");
    assert_eq!(seen.promoted_by, alice.to_string());
    assert_eq!(seen.version, 1, "the envelope's declaration");

    let view = read_projection_body(&seen.body).expect("the mapper reads its own body");
    assert_eq!(view.version, 1, "and the body's own");
    assert_eq!(view.title.as_deref(), Some("What Alice called it"));
    assert_eq!(view.marks.len(), 1);
    assert_eq!(view.marks[0].said, "chapter 2 is wrong");

    // Decision 4: what an importer wrote stayed home.
    assert!(
        !seen.body.contains("what the container said"),
        "an Imported mark travelled: {}",
        seen.body
    );

    // The ledger says who brought it and in what capacity.
    let page = bob_client.events(team, None, None).await.expect("events");
    let entered = page
        .events
        .iter()
        .find(|event| event.kind == "forge.content.entered/1")
        .expect("the content event");
    assert_eq!(entered.actor_user_id, alice.to_string());
    assert_eq!(entered.actor_kind, "member");
    assert!(
        page.events
            .iter()
            .any(|event| event.kind == "forge.round.pushed/1"),
        "the round that named it is in the stream too"
    );
}

/// A projection belongs to its team, and a member of another team
/// cannot read it by knowing its ids.
///
/// The gate proves membership of the team in the *path*; a line id is
/// unique across teams, so `(line, entry)` alone would find the row
/// whoever asked. This is the test for the other half — the read is
/// scoped to the team the gate established, so the same ids asked
/// under a different team's prefix answer as absent.
#[tokio::test]
async fn a_projection_does_not_cross_to_another_team() {
    let h = harness().await;
    let (_alice, alice_client) = member(&h, "alice").await;
    let (_mallory, mallory_client) = member(&h, "mallory").await;

    // Alice's team, with a promotion on it.
    let alices = TeamScopedId::parse(
        &alice_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();
    let line = TeamScopedId::parse(
        &alice_client
            .open_line(alices, "alice's line", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &alice_client
            .open_pursuit(alices, line, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();
    let local = local_asset(
        "Alice's private working title",
        b"alice's bytes",
        "alice's note",
    );
    let links = Rows::default();
    let outcome = promote(
        &alice_client,
        &links,
        Promotion {
            team_id: alices,
            line_id: line,
            pursuit_id: pursuit,
            subject: LocalSubject {
                asset: &local.asset,
                user_marks: &local.user_marks,
            },
            named: "the-thing",
        },
        now_ms(),
    )
    .await
    .unwrap();

    // Alice reads her own.
    assert!(
        alice_client
            .entry_projection(alices, line, outcome.key.entry_id)
            .await
            .unwrap()
            .is_some()
    );

    // Mallory has a team of her own, and is not on Alice's roster. She
    // knows Alice's line and entry ids — which is the premise, not a
    // stretch: they ride on the wire and appear in a ledger.
    let mallorys = TeamScopedId::parse(
        &mallory_client.create_team(None).await.unwrap().team_id,
        "team id",
    )
    .unwrap();

    // Asking under Alice's prefix is refused by the gate.
    let refused = mallory_client
        .entry_projection(alices, line, outcome.key.entry_id)
        .await
        .expect_err("mallory is not on that roster");
    match refused {
        TeamsClientError::Refused { status, .. } => assert_eq!(status, 403),
        other => panic!("expected a refusal, got {other:?}"),
    }

    // And asking under her own prefix — where the gate lets her in —
    // answers as absent, because the read is scoped to the team the
    // gate established rather than to the ids in the path.
    assert!(
        mallory_client
            .entry_projection(mallorys, line, outcome.key.entry_id)
            .await
            .expect("her own team is hers to ask about")
            .is_none(),
        "another team's projection came back"
    );
}

/// Decision 7: two members promoting identical content get one
/// `TeamAsset` each over one stored copy.
#[tokio::test]
async fn identical_content_promoted_twice_mints_two_team_assets() {
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
            .open_line(team, "one line", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();

    let bytes = b"exactly the same bytes";
    let mut minted = Vec::new();
    let mut digests = Vec::new();
    for (who, client, title) in [
        (alice, &alice_client, "Alice's name for it"),
        (bob, &bob_client, "Bob's name for it"),
    ] {
        let pursuit = TeamScopedId::parse(
            &client
                .open_pursuit(team, line, None, None)
                .await
                .unwrap()
                .id,
            "pursuit id",
        )
        .unwrap();
        let local = local_asset(title, bytes, "mine");
        let links = Rows::default();
        let outcome = promote(
            client,
            &links,
            Promotion {
                team_id: team,
                line_id: line,
                pursuit_id: pursuit,
                subject: LocalSubject {
                    asset: &local.asset,
                    user_marks: &local.user_marks,
                },
                named: &format!("entry-of-{who}"),
            },
            now_ms(),
        )
        .await
        .expect("promote");
        minted.push(outcome.team_asset_id.expect("a mint"));
        digests.push(outcome.digest);
    }

    assert_eq!(digests[0], digests[1], "the bytes are the same");
    assert_ne!(
        minted[0], minted[1],
        "one TeamAsset per promotion, so who brought what survives the second contributor"
    );

    let resolved = alice_client
        .resolve_content(team, &minted)
        .await
        .expect("resolve both");
    assert_eq!(resolved.held.len(), 2);
    for held in &resolved.held {
        assert_eq!(held.digest.as_deref(), Some(digests[0].as_str()));
    }

    // The have-check saw the second copy as already there, even though
    // the mint still needed the verb.
    let held = alice_client
        .have_content(team, vec![digests[0].clone()])
        .await
        .unwrap();
    assert_eq!(held.held, vec![digests[0].clone()]);
}

/// A repeat of the same promotion is answered from the machine that
/// already did it, and sends nothing.
#[tokio::test]
async fn promoting_the_same_asset_onto_the_same_line_twice_is_a_repeat() {
    let h = harness().await;
    let (_alice, client) = member(&h, "alice").await;
    let team =
        TeamScopedId::parse(&client.create_team(None).await.unwrap().team_id, "team id").unwrap();
    let line = TeamScopedId::parse(
        &client
            .open_line(team, "one line", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &client
            .open_pursuit(team, line, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();

    let local = local_asset("once", b"one set of bytes", "said once");
    let links = Rows::default();
    let subject = LocalSubject {
        asset: &local.asset,
        user_marks: &local.user_marks,
    };
    let promotion = Promotion {
        team_id: team,
        line_id: line,
        pursuit_id: pursuit,
        subject,
        named: "the-thing",
    };

    let first = promote(&client, &links, promotion, now_ms()).await.unwrap();
    assert!(!first.already_promoted);

    let again = promote(&client, &links, promotion, now_ms()).await.unwrap();
    assert!(again.already_promoted, "the relation answered it");
    assert!(again.team_asset_id.is_none(), "nothing was minted");
    assert_eq!(
        again.bytes_already_held, None,
        "nothing was going to be sent, so the have-check was never put"
    );
    assert_eq!(
        again.key, first.key,
        "and it names the promotion that did happen"
    );
    assert_eq!(links.count(), 1);
}

/// Decision 9: when the team end vanishes, verify says so and reap
/// removes the row.
#[tokio::test]
async fn a_discarded_line_leaves_rows_that_verify_finds_and_reap_removes() {
    let h = harness().await;
    let (_alice, client) = member(&h, "alice").await;
    let team =
        TeamScopedId::parse(&client.create_team(None).await.unwrap().team_id, "team id").unwrap();
    let line = TeamScopedId::parse(
        &client.open_line(team, "doomed", MAINLINE).await.unwrap().id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &client
            .open_pursuit(team, line, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();

    let local = local_asset(
        "in the doomed line",
        b"bytes that outlive their line",
        "mine",
    );
    let links = Rows::default();
    let outcome = promote(
        &client,
        &links,
        Promotion {
            team_id: team,
            line_id: line,
            pursuit_id: pursuit,
            subject: LocalSubject {
                asset: &local.asset,
                user_marks: &local.user_marks,
            },
            named: "the-thing",
        },
        now_ms(),
    )
    .await
    .unwrap();

    // Before anything vanishes, both ends are there.
    let clean = verify(&client, &links, team).await.expect("verify");
    assert!(clean.is_clean(), "{clean:?}");

    // Discarding takes the line and its log. Two things have to
    // happen first, and both are the forge's own order rather than
    // this test's: work has to end, and a line is dropped from the
    // archive rather than from active use.
    client
        .close_pursuit(team, pursuit, "abandoned", None)
        .await
        .expect("end the work");
    client.archive_line(team, line).await.expect("archive");
    client.discard_line(team, line).await.expect("discard");

    let found = verify(&client, &links, team).await.expect("verify again");
    assert_eq!(found.dangling.len(), 1, "{found:?}");
    assert_eq!(found.dangling[0].why, Missing::TeamEntry);
    assert_eq!(found.dangling[0].link.key, outcome.key);

    assert_eq!(reap(&links, &found.keys()).await.unwrap(), 1);
    assert_eq!(links.count(), 0, "the row is gone");

    let after = verify(&client, &links, team)
        .await
        .expect("verify a third time");
    assert!(after.is_clean());
}

/// A projection may only describe an entry the round it rides on
/// names.
#[tokio::test]
async fn a_projection_for_an_entry_the_round_does_not_touch_is_refused() {
    use asterism_contract::forge::ForgeOpDto;
    use asterism_teams_wire::projection::{EntryProjectionEnvelope, PROJECTION_VERSION};

    let h = harness().await;
    let (_alice, client) = member(&h, "alice").await;
    let team =
        TeamScopedId::parse(&client.create_team(None).await.unwrap().team_id, "team id").unwrap();
    let line = TeamScopedId::parse(
        &client
            .open_line(team, "one line", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &client
            .open_pursuit(team, line, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();

    let entry = TeamScopedId::new();
    let elsewhere = TeamScopedId::new();
    let bytes = b"some bytes";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.bin");
    std::fs::write(&path, bytes).unwrap();
    let digest = {
        use asterism_contract::digest::ContentHasher;
        let mut hasher = ContentHasher::new();
        hasher.update(bytes);
        hasher.finish()
    };
    let entered = client
        .enter_content(team, pursuit, &digest, &path)
        .await
        .expect("enter content");

    let refused = client
        .push_round(
            team,
            pursuit,
            vec![ForgeOpDto {
                entry_id: entry.to_string(),
                kind: "add".to_string(),
                content_asset_id: Some(entered.asset_id.clone()),
                name: Some("the-thing".to_string()),
            }],
            None,
            vec![EntryProjectionEnvelope {
                entry_id: elsewhere.to_string(),
                version: PROJECTION_VERSION,
                body: r#"{"v":1,"title":"about somebody else's entry"}"#.to_string(),
            }],
        )
        .await
        .expect_err("a projection aimed elsewhere is refused");

    match refused {
        TeamsClientError::Refused {
            status, message, ..
        } => {
            assert_eq!(status, 400);
            assert!(message.contains("operates on"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // And nothing was captured for either entry.
    assert!(
        client
            .entry_projection(team, line, elsewhere)
            .await
            .unwrap()
            .is_none()
    );
}

/// A push with no projections is the mirror's own request, and lands
/// with nothing captured.
#[tokio::test]
async fn a_push_without_projections_captures_nothing() {
    use asterism_contract::forge::ForgeOpDto;

    let h = harness().await;
    let (_alice, client) = member(&h, "alice").await;
    let team =
        TeamScopedId::parse(&client.create_team(None).await.unwrap().team_id, "team id").unwrap();
    let line = TeamScopedId::parse(
        &client
            .open_line(team, "one line", MAINLINE)
            .await
            .unwrap()
            .id,
        "line id",
    )
    .unwrap();
    let pursuit = TeamScopedId::parse(
        &client
            .open_pursuit(team, line, None, None)
            .await
            .unwrap()
            .id,
        "pursuit id",
    )
    .unwrap();

    let bytes = b"plain bytes";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.bin");
    std::fs::write(&path, bytes).unwrap();
    let digest = {
        use asterism_contract::digest::ContentHasher;
        let mut hasher = ContentHasher::new();
        hasher.update(bytes);
        hasher.finish()
    };
    let entered = client
        .enter_content(team, pursuit, &digest, &path)
        .await
        .unwrap();

    let entry = TeamScopedId::new();
    client
        .push_round(
            team,
            pursuit,
            vec![ForgeOpDto {
                entry_id: entry.to_string(),
                kind: "add".to_string(),
                content_asset_id: Some(entered.asset_id),
                name: Some("unnamed-by-anybody".to_string()),
            }],
            None,
            Vec::new(),
        )
        .await
        .expect("a bare push");

    assert!(
        client
            .entry_projection(team, line, entry)
            .await
            .unwrap()
            .is_none(),
        "an entry nobody described has no projection"
    );
}

// ----------------------------------------------------------------------
// The roster writes, over the client (#210).
// ----------------------------------------------------------------------

/// An owner fills a roster and empties it again through the client,
/// and the ledger says what each act was.
///
/// One test rather than five, because the five are one sequence: there
/// is nobody to grant a role to until somebody was invited, and nobody
/// to remove until they hold one. Split apart, each would spend its
/// body re-inviting, which tests the fixture rather than the verb.
#[tokio::test]
async fn the_roster_writes_go_over_the_client() {
    let h = harness().await;
    let (_alice_id, alice) = member(&h, "alice").await;
    let (bob_id, _bob) = member(&h, "bob").await;
    let team =
        TeamScopedId::parse(&alice.create_team(None).await.unwrap().team_id, "team id").unwrap();

    let added = alice
        .invite_member(team, &bob_id.to_string(), "member")
        .await
        .expect("invite bob");
    assert_eq!(added.kind, "teams.membership.added/1");
    assert_eq!(
        added.actor_kind, "member",
        "an owner acts in their membership, not as the instance"
    );
    assert_eq!(
        alice.roster(team).await.unwrap().members.len(),
        2,
        "alice founded it and bob was let in"
    );

    let granted = alice
        .grant_owner(team, &bob_id.to_string())
        .await
        .expect("grant bob the owner role");
    assert_eq!(granted.kind, "teams.membership.role_changed/1");

    let revoked = alice
        .revoke_owner(team, &bob_id.to_string())
        .await
        .expect("revoke it again, alice being an owner still");
    assert_eq!(revoked.kind, "teams.membership.role_changed/1");

    let removed = alice
        .remove_member(team, &bob_id.to_string())
        .await
        .expect("remove bob");
    assert_eq!(removed.kind, "teams.membership.removed/1");
    assert_eq!(
        alice.roster(team).await.unwrap().members.len(),
        1,
        "the roster is alice's again"
    );

    let deleted = alice.delete_team(team).await.expect("delete the team");
    assert_eq!(deleted.kind, "teams.team.deleted/1");
}

/// A member takes themself out through the client, and the last owner
/// cannot.
#[tokio::test]
async fn a_member_leaves_through_the_client() {
    let h = harness().await;
    let (_alice_id, alice) = member(&h, "alice").await;
    let (bob_id, bob) = member(&h, "bob").await;
    let team =
        TeamScopedId::parse(&alice.create_team(None).await.unwrap().team_id, "team id").unwrap();
    alice
        .invite_member(team, &bob_id.to_string(), "member")
        .await
        .expect("invite bob");

    let left = bob.leave_team(team).await.expect("bob leaves");
    assert_eq!(left.kind, "teams.membership.removed/1");
    assert_eq!(
        left.actor_user_id,
        bob_id.to_string(),
        "stamped to the one leaving, which is what tells this from a removal"
    );
    assert_eq!(
        alice.roster(team).await.unwrap().members.len(),
        1,
        "the roster is alice's again"
    );

    match alice.leave_team(team).await {
        Err(TeamsClientError::Refused { status, .. }) => assert_eq!(status, 409),
        other => panic!("the last owner should not be able to leave: {other:?}"),
    }
}

/// The last owner cannot go, and what the client hands back says the
/// team's state refused it rather than the request being wrong.
///
/// A 409 rather than a 400 is the point of the assertion: the request
/// named a real member and spelled every field, and the team is what
/// says no. The client keeps the status; what happens to it on the way
/// to a screen is #211's.
#[tokio::test]
async fn the_last_owner_refusal_reaches_the_client_as_a_conflict() {
    let h = harness().await;
    let (alice_id, alice) = member(&h, "alice").await;
    let team =
        TeamScopedId::parse(&alice.create_team(None).await.unwrap().team_id, "team id").unwrap();

    match alice.remove_member(team, &alice_id.to_string()).await {
        Err(TeamsClientError::Refused { status, kind, .. }) => {
            assert_eq!(status, 409);
            assert_eq!(kind, "Conflict");
        }
        other => panic!("removing the last owner should be refused: {other:?}"),
    }

    match alice.revoke_owner(team, &alice_id.to_string()).await {
        Err(TeamsClientError::Refused { status, .. }) => assert_eq!(status, 409),
        other => panic!("demoting the last owner should be refused: {other:?}"),
    }

    assert_eq!(
        alice.roster(team).await.unwrap().members.len(),
        1,
        "neither refusal moved the roster"
    );
}
