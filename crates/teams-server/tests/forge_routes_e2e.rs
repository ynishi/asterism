//! End-to-end guard for the team's forge over HTTP (#151, the #148
//! decision 19 slice): the mirrored verbs, the boundary revision 5
//! draws around them, the three verbs hosting adds, and what the
//! ledger says about all of it.
//!
//! Drives the real router through `oneshot` over an in-memory teams DB
//! and a tempdir-backed blob store — the same wiring the binary
//! assembles, nothing bypassed. In particular the content verb goes in
//! as bytes over the wire and comes out of the CAS the same way an
//! upload does, because the two share that path deliberately (#83 §3).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use sha2::{Digest as _, Sha256};
use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;
use uuid::Uuid;

const GOOD: &str = "correct horse battery staple";

/// The rule every line here is pointed at, by the slug the strategies
/// route lists rather than by a type this crate imports — the mirror
/// is what is under test, and a client reads the slug off the wire.
const MAINLINE: &str = "mainline-first";

struct Harness {
    ctx: Arc<TeamsCtx>,
    router: Router,
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
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        auth_limiter: RateLimiter::new(1_000, Duration::from_secs(60)),
        purge_grace_ms: 0,
        gc_guard: Arc::new(teams_infra::gc::GcGuard::new()),
    });
    let router = teams_server::http::router(ctx.clone());
    Harness {
        ctx,
        router,
        isle,
        driver,
        blob_dir,
    }
}

// ----------------------------------------------------------------------
// Wire helpers.
// ----------------------------------------------------------------------

async fn call(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "body is not JSON ({e}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, json)
}

fn post_authed(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .expect("build authed POST")
}

fn get_authed(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build authed GET")
}

fn put_bytes(uri: &str, token: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/octet-stream")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(bytes))
        .expect("build authed PUT")
}

/// The shared digest notation, spelled by the client's own hasher.
fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

// ----------------------------------------------------------------------
// Fixtures.
// ----------------------------------------------------------------------

async fn user(h: &Harness, login: &str) -> (Uuid, String) {
    provision(h, login, false).await
}

async fn admin_user(h: &Harness, login: &str) -> (Uuid, String) {
    provision(h, login, true).await
}

async fn provision(h: &Harness, login: &str, admin: bool) -> (Uuid, String) {
    let user_id = h
        .ctx
        .auth
        .create_account(login, login, GOOD, admin, now_ms())
        .await
        .expect("create account");
    let (status, body) = call(
        &h.router,
        Request::builder()
            .method("POST")
            .uri("/teams/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "login": login, "password": GOOD }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login for {login}: {body}");
    (user_id, body["token"].as_str().unwrap().to_string())
}

async fn create_team(h: &Harness, token: &str) -> String {
    let (status, body) = call(
        &h.router,
        post_authed("/teams/create", token, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create team: {body}");
    body["team_id"].as_str().unwrap().to_string()
}

async fn invite(h: &Harness, team: &str, owner: &str, user_id: Uuid, role: &str) {
    let (status, body) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/members/invite"),
            owner,
            serde_json::json!({ "user_id": user_id.to_string(), "role": role }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "invite: {body}");
}

async fn open_line(h: &Harness, team: &str, token: &str, name: &str) -> String {
    let (status, body) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines"),
            token,
            serde_json::json!({ "name": name, "strategy_id": MAINLINE }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open line: {body}");
    body["id"].as_str().expect("a line id").to_string()
}

async fn open_pursuit(h: &Harness, team: &str, token: &str, line: &str) -> String {
    let (status, body) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/pursuits"),
            token,
            serde_json::json!({ "line_id": line }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open pursuit: {body}");
    body["id"].as_str().expect("a pursuit id").to_string()
}

/// Puts bytes to the content verb against `pursuit`, hashing them the
/// way a client would.
///
/// Returns the response whole rather than the asset it minted, because
/// half the calls here are the ones that refuse: the closed pursuit,
/// the other team's work. A helper that unwrapped the happy path would
/// be unusable for exactly the cases the verb is most worth testing.
async fn enter_content(
    h: &Harness,
    team: &str,
    token: &str,
    pursuit: &str,
    bytes: &[u8],
) -> (StatusCode, serde_json::Value) {
    let digest = digest_of(bytes);
    call(
        &h.router,
        put_bytes(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/content?digest={digest}"),
            token,
            bytes.to_vec(),
        ),
    )
    .await
}

async fn events_of(h: &Harness, team: &str, token: &str) -> Vec<serde_json::Value> {
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{team}/events"), token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "events: {body}");
    body["events"].as_array().expect("events array").clone()
}

fn kinds(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .map(|event| event["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

// ----------------------------------------------------------------------
// (a) The whole of it, over the wire.
// ----------------------------------------------------------------------

/// Login, a team, a line, work against it, content in, a round naming
/// that content, a close, and the line's history saying so — the
/// mirror carrying one person's small piece of work end to end.
#[tokio::test]
async fn a_member_works_a_line_from_login_to_landing() {
    let h = harness().await;
    let (alice_id, alice) = user(&h, "alice").await;
    let team = create_team(&h, &alice).await;

    // The rule is a slug read off the wire, not a constant a client
    // is assumed to know.
    let (status, rules) = call(
        &h.router,
        get_authed(&format!("/teams/{team}/forge/strategies"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rules}");
    assert!(
        rules
            .as_array()
            .expect("a list of rules")
            .iter()
            .any(|rule| rule["id"] == MAINLINE),
        "the hosted forge carries the same built-in rules as the local one: {rules}"
    );

    let line = open_line(&h, &team, &alice, "ROOT").await;
    let pursuit = open_pursuit(&h, &team, &alice, &line).await;

    // Content, against open work — the one entry point (#148
    // decision 5).
    let bytes = b"the promoted artefact".to_vec();
    let (status, entered) = enter_content(&h, &team, &alice, &pursuit, &bytes).await;
    assert_eq!(status, StatusCode::OK, "enter content: {entered}");
    assert_eq!(entered["digest"], digest_of(&bytes));
    assert_eq!(entered["pursuit_id"], pursuit);
    assert_eq!(entered["event"]["kind"], "forge.content.entered/1");
    assert_eq!(entered["event"]["actor_kind"], "member");
    assert_eq!(entered["event"]["actor_user_id"], alice_id.to_string());
    let asset = entered["asset_id"]
        .as_str()
        .expect("a team asset")
        .to_string();

    // A round names it. The content was there first, which is the
    // ordering decision 5 exists to keep.
    let entry = Uuid::now_v7().to_string();
    let (status, pushed) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/push"),
            &alice,
            serde_json::json!({
                "ops": [{
                    "entry_id": entry,
                    "kind": "add",
                    "content_asset_id": asset,
                    "name": "cut-01",
                }],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "push: {pushed}");
    assert_eq!(
        pushed["rounds"].as_array().expect("rounds").len(),
        1,
        "a push answers with the work whole: {pushed}"
    );

    // Closed satisfied, so the line moves.
    let (status, closed) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/close"),
            &alice,
            serde_json::json!({ "outcome": "satisfied" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "close: {closed}");
    assert_eq!(closed["close"]["outcome"], "satisfied", "{closed}");

    // The history says what landed, and the fold says what is on it.
    let (status, history) = call(
        &h.router,
        get_authed(&format!("/teams/{team}/forge/lines/{line}"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "history: {history}");
    assert_eq!(
        history["changes"].as_array().expect("changes").len(),
        1,
        "one close, one change point: {history}"
    );
    let (status, states) = call(
        &h.router,
        get_authed(&format!("/teams/{team}/forge/lines/{line}/states"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "states: {states}");
    let alive: Vec<&serde_json::Value> = states
        .as_array()
        .expect("states")
        .iter()
        .filter(|state| state["alive"] == true)
        .collect();
    assert_eq!(alive.len(), 1, "{states}");
    assert_eq!(alive[0]["name"], "cut-01");
    assert_eq!(
        alive[0]["content_asset_id"], asset,
        "what is on the line is the team's asset, not a local one"
    );
}

// ----------------------------------------------------------------------
// (b) The boundary (#148 revision 5).
// ----------------------------------------------------------------------

/// A plain member opens lines and works them; somebody outside the
/// team does not see them at all.
#[tokio::test]
async fn membership_is_the_whole_answer_for_working_a_line() {
    let h = harness().await;
    let (_owner_id, owner) = user(&h, "alice").await;
    let (bob_id, bob) = user(&h, "bob").await;
    let (_carol_id, carol) = user(&h, "carol").await;
    let team = create_team(&h, &owner).await;
    invite(&h, &team, &owner, bob_id, "member").await;

    // A plain member opens a line, opens work and pushes — no
    // seniority anywhere in it (revision 5).
    let line = open_line(&h, &team, &bob, "bob's line").await;
    let pursuit = open_pursuit(&h, &team, &bob, &line).await;
    let (status, said) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/threads"),
            &bob,
            serde_json::json!({
                "anchor_kind": "pursuit",
                "pursuit_id": pursuit,
                "said": "starting on this",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a member says things: {said}");

    // Carol is in no team. The gate answers before any handler runs.
    let (status, refused) = call(
        &h.router,
        get_authed(&format!("/teams/{team}/forge/lines"), &carol),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines"),
            &carol,
            serde_json::json!({ "name": "not yours", "strategy_id": MAINLINE }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

/// Discarding a line is the one verb that wants an owner — it is the
/// one that takes the log with it.
#[tokio::test]
async fn only_an_owner_discards_a_line() {
    let h = harness().await;
    let (_owner_id, owner) = user(&h, "alice").await;
    let (bob_id, bob) = user(&h, "bob").await;
    let team = create_team(&h, &owner).await;
    invite(&h, &team, &owner, bob_id, "member").await;

    let line = open_line(&h, &team, &bob, "bob's line").await;

    // The archive is a member's, and a discard reaches the line
    // through it.
    let (status, archived) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines/{line}/archive"),
            &bob,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "archive: {archived}");
    assert_eq!(archived["standing"], "archived");

    // The discard is not.
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines/{line}/discard"),
            &bob,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|said| said.contains("discarding a line")),
        "the refusal names the verb: {refused}"
    );

    // The owner's goes through.
    let (status, dropped) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines/{line}/discard"),
            &owner,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "discard: {dropped}");
    assert_eq!(dropped["line_id"], line);
}

/// An admin standing outside the roster has no verb on somebody's
/// forge — not the work, and not the discard either.
#[tokio::test]
async fn an_admin_outside_the_roster_works_nobodys_forge() {
    let h = harness().await;
    let (_owner_id, owner) = user(&h, "alice").await;
    let (_admin_id, root) = admin_user(&h, "root").await;
    let team = create_team(&h, &owner).await;
    let line = open_line(&h, &team, &owner, "ROOT").await;

    // Reading is the general §1 boundary and stays available.
    let (status, lines) = call(
        &h.router,
        get_authed(&format!("/teams/{team}/forge/lines"), &root),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "an admin may read: {lines}");

    // Writing is not.
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines"),
            &root,
            serde_json::json!({ "name": "root's line", "strategy_id": MAINLINE }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines/{line}/discard"),
            &root,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

/// A command that states an author is refused rather than quietly
/// overwritten (#148 revision 6).
#[tokio::test]
async fn a_caller_cannot_state_who_it_is_on_a_teams_forge() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team = create_team(&h, &alice).await;

    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/lines"),
            &alice,
            serde_json::json!({
                "name": "ROOT",
                "strategy_id": MAINLINE,
                "author_kind": "subject",
                "author_subject": "somebody-else",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|said| said.contains("authenticated member")),
        "{refused}"
    );
}

// ----------------------------------------------------------------------
// (c) The content verb's refusals.
// ----------------------------------------------------------------------

/// Work that has ended takes no more content, and work belonging to
/// another team is not there to take any.
#[tokio::test]
async fn content_enters_against_open_work_of_this_team_and_nothing_else() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (_bob_id, bob) = user(&h, "bob").await;
    let mine = create_team(&h, &alice).await;
    let theirs = create_team(&h, &bob).await;

    let line = open_line(&h, &mine, &alice, "ROOT").await;
    let ended = open_pursuit(&h, &mine, &alice, &line).await;
    let (status, closed) = call(
        &h.router,
        post_authed(
            &format!("/teams/{mine}/forge/pursuits/{ended}/close"),
            &alice,
            serde_json::json!({ "outcome": "abandoned" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "close: {closed}");

    let (status, refused) = enter_content(&h, &mine, &alice, &ended, b"too late").await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["reason"], "settled", "{refused}");

    // Bob's own work, asked about through Alice's team: her prefix
    // gates her membership, and the id reads as absent because it is
    // not her team's.
    let their_line = open_line(&h, &theirs, &bob, "ROOT").await;
    let their_work = open_pursuit(&h, &theirs, &bob, &their_line).await;
    let (status, refused) = enter_content(&h, &mine, &alice, &their_work, b"not yours").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
}

/// A round may not name content the team does not hold — checked by
/// the service against this team's store, before any row is written.
#[tokio::test]
async fn a_round_cannot_name_content_this_team_does_not_hold() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (_bob_id, bob) = user(&h, "bob").await;
    let mine = create_team(&h, &alice).await;
    let theirs = create_team(&h, &bob).await;

    // Bob brings content into his own team.
    let their_line = open_line(&h, &theirs, &bob, "ROOT").await;
    let their_work = open_pursuit(&h, &theirs, &bob, &their_line).await;
    let (status, entered) = enter_content(&h, &theirs, &bob, &their_work, b"bob's bytes").await;
    assert_eq!(status, StatusCode::OK, "{entered}");
    let their_asset = entered["asset_id"].as_str().expect("an asset").to_string();

    // Alice names it on her own line. The id is well-formed and the
    // team does not have it, which is the whole of the answer — the
    // service's own refusal, in the same words the local surface uses
    // for content that is not there, because on this plane "not there"
    // means "not this team's".
    let line = open_line(&h, &mine, &alice, "ROOT").await;
    let work = open_pursuit(&h, &mine, &alice, &line).await;
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{mine}/forge/pursuits/{work}/push"),
            &alice,
            serde_json::json!({
                "ops": [{
                    "entry_id": Uuid::now_v7().to_string(),
                    "kind": "add",
                    "content_asset_id": their_asset,
                    "name": "cut-01",
                }],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|said| said.contains("content that does not exist")),
        "{refused}"
    );

    // And nothing was written: the check runs before the round is
    // built, so the work is still where it was.
    let (status, work_now) = call(
        &h.router,
        get_authed(&format!("/teams/{mine}/forge/pursuits/{work}"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{work_now}");
    assert_eq!(work_now["rounds"], serde_json::json!([]), "{work_now}");
}

/// Two members bringing identical bytes get an asset each over one
/// stored copy (#148 decision 7).
#[tokio::test]
async fn the_same_bytes_twice_are_two_assets_and_one_copy() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (bob_id, bob) = user(&h, "bob").await;
    let team = create_team(&h, &alice).await;
    invite(&h, &team, &alice, bob_id, "member").await;

    let line = open_line(&h, &team, &alice, "ROOT").await;
    let hers = open_pursuit(&h, &team, &alice, &line).await;
    let his = open_pursuit(&h, &team, &bob, &line).await;
    let bytes = b"identical".to_vec();

    let (status, first) = enter_content(&h, &team, &alice, &hers, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (status, second) = enter_content(&h, &team, &bob, &his, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{second}");

    assert_ne!(
        first["asset_id"], second["asset_id"],
        "an asset per promotion, so who brought what survives the second contributor"
    );
    assert_eq!(first["digest"], second["digest"], "one stored copy");
    // And each promotion is its own entry in the stream.
    let entered = kinds(&events_of(&h, &team, &alice).await)
        .iter()
        .filter(|kind| kind.as_str() == "forge.content.entered/1")
        .count();
    assert_eq!(entered, 2);
}

// ----------------------------------------------------------------------
// (d) The two reads hosting adds.
// ----------------------------------------------------------------------

/// The bulk resolve answers about this team's assets and says nothing
/// about anybody else's; the have-check answers about this team's
/// digests.
#[tokio::test]
async fn resolve_and_have_answer_inside_the_team_and_nowhere_else() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (_bob_id, bob) = user(&h, "bob").await;
    let mine = create_team(&h, &alice).await;
    let theirs = create_team(&h, &bob).await;

    let line = open_line(&h, &mine, &alice, "ROOT").await;
    let work = open_pursuit(&h, &mine, &alice, &line).await;
    let bytes = b"mine".to_vec();
    let (status, entered) = enter_content(&h, &mine, &alice, &work, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{entered}");
    let asset = entered["asset_id"].as_str().expect("an asset").to_string();

    let their_line = open_line(&h, &theirs, &bob, "ROOT").await;
    let their_work = open_pursuit(&h, &theirs, &bob, &their_line).await;
    let their_bytes = b"theirs".to_vec();
    let (status, theirs_entered) =
        enter_content(&h, &theirs, &bob, &their_work, &their_bytes).await;
    assert_eq!(status, StatusCode::OK, "{theirs_entered}");
    let their_asset = theirs_entered["asset_id"]
        .as_str()
        .expect("an asset")
        .to_string();

    // Resolve: mine comes back whole, theirs and a made-up one come
    // back as unknown — the same answer, which is the point.
    let stranger = Uuid::now_v7().to_string();
    let (status, resolved) = call(
        &h.router,
        post_authed(
            &format!("/teams/{mine}/forge/content/resolve"),
            &alice,
            serde_json::json!({ "asset_ids": [&asset, &their_asset, &stranger] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resolve: {resolved}");
    let held = resolved["held"].as_array().expect("held");
    assert_eq!(held.len(), 1, "{resolved}");
    assert_eq!(held[0]["asset_id"], asset);
    assert_eq!(held[0]["digest"], digest_of(&bytes));
    assert_eq!(held[0]["entered_for_pursuit_id"], work);
    let unknown = resolved["unknown"].as_array().expect("unknown");
    assert_eq!(unknown.len(), 2, "{resolved}");
    assert!(unknown.contains(&serde_json::json!(their_asset)));
    assert!(unknown.contains(&serde_json::json!(stranger)));

    // Have: the digest this team holds, and not the one it does not.
    let absent = digest_of(b"never sent");
    let (status, have) = call(
        &h.router,
        post_authed(
            &format!("/teams/{mine}/forge/content/have"),
            &alice,
            serde_json::json!({
                "digests": [digest_of(&bytes), digest_of(&their_bytes), absent],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "have: {have}");
    assert_eq!(
        have["held"],
        serde_json::json!([digest_of(&bytes)]),
        "the answer is bounded by this team's links, never by the store: {have}"
    );

    // A digest that is not one is the request's grammar error, not a
    // quiet "not held".
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{mine}/forge/content/have"),
            &alice,
            serde_json::json!({ "digests": ["not-a-digest"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}

/// Neither bulk verb lets a caller decide how long the server stops
/// answering anybody.
///
/// Both walk their input one statement at a time on a single
/// connection, so the list is bounded — and refused rather than
/// truncated, because a truncated answer to either is a wrong one: the
/// resolve would call ids unknown that it never looked at, and the
/// have-check would ask for bytes the team has.
#[tokio::test]
async fn a_bulk_read_is_bounded_and_says_so_rather_than_truncating() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team = create_team(&h, &alice).await;

    let too_many: Vec<String> = (0..501).map(|_| Uuid::now_v7().to_string()).collect();
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/content/resolve"),
            &alice,
            serde_json::json!({ "asset_ids": too_many }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|said| said.contains("the most is 500")),
        "the refusal says what the ceiling is, so a caller can split: {refused}"
    );

    let too_many: Vec<String> = (0..501)
        .map(|n| digest_of(format!("{n}").as_bytes()))
        .collect();
    let (status, refused) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/content/have"),
            &alice,
            serde_json::json!({ "digests": too_many }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // And the ceiling itself is answerable, not the first refusal.
    let at_the_line: Vec<String> = (0..500).map(|_| Uuid::now_v7().to_string()).collect();
    let (status, resolved) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/content/resolve"),
            &alice,
            serde_json::json!({ "asset_ids": at_the_line }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["held"], serde_json::json!([]), "{resolved}");
}

// ----------------------------------------------------------------------
// (e) What the ledger says.
// ----------------------------------------------------------------------

/// One line's worth of work lands in the team's stream in order, and
/// the subject filter finds it from either end.
///
/// Four of the forge's kinds and the content entry, which is what one
/// person doing one small thing writes — not the registry's coverage.
/// What this pins is that the writes reach the stream at all, that
/// they arrive in the order they happened, and that the two indexes a
/// promotion touches (the work, the digest) both lead back to it.
#[tokio::test]
async fn the_stream_carries_the_forge_and_the_subject_filter_finds_it() {
    let h = harness().await;
    let (alice_id, alice) = user(&h, "alice").await;
    let team = create_team(&h, &alice).await;

    let line = open_line(&h, &team, &alice, "ROOT").await;
    let pursuit = open_pursuit(&h, &team, &alice, &line).await;
    let bytes = b"the artefact".to_vec();
    let (status, entered) = enter_content(&h, &team, &alice, &pursuit, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{entered}");
    let asset = entered["asset_id"].as_str().expect("an asset").to_string();
    let entry = Uuid::now_v7().to_string();
    let (status, pushed) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/push"),
            &alice,
            serde_json::json!({
                "ops": [{
                    "entry_id": entry,
                    "kind": "add",
                    "content_asset_id": asset,
                    "name": "cut-01",
                }],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pushed}");
    let (status, closed) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/close"),
            &alice,
            serde_json::json!({ "outcome": "satisfied" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{closed}");

    // The whole stream, through the page the substrate already had.
    let events = events_of(&h, &team, &alice).await;
    let said = kinds(&events);
    assert_eq!(
        said,
        vec![
            "teams.team.created/1",
            "forge.line.opened/1",
            "forge.pursuit.opened/1",
            "forge.content.entered/1",
            "forge.round.pushed/1",
            "forge.pursuit.closed/1",
        ],
        "one event per act, in the order they happened: {events:#?}"
    );
    for event in &events {
        assert_eq!(event["actor_user_id"], alice_id.to_string());
    }

    // The subject filter, from the work's end: everything that
    // happened to this pursuit, including the content that entered
    // against it.
    let (status, page) = call(
        &h.router,
        get_authed(
            &format!("/teams/{team}/events/subject?type=forge_pursuit&value={pursuit}"),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "subject page: {page}");
    let about_work = kinds(page["events"].as_array().expect("events"));
    assert_eq!(
        about_work,
        vec![
            "forge.pursuit.opened/1",
            "forge.content.entered/1",
            "forge.round.pushed/1",
            "forge.pursuit.closed/1",
        ],
        "{page}"
    );

    // And from the store's end: the digest the bytes hashed to.
    let (status, page) = call(
        &h.router,
        get_authed(
            &format!(
                "/teams/{team}/events/subject?type=digest&value={}",
                digest_of(&bytes)
            ),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        kinds(page["events"].as_array().expect("events")),
        vec!["forge.content.entered/1"],
        "{page}"
    );

    // The line, which the close moved.
    let (status, page) = call(
        &h.router,
        get_authed(
            &format!("/teams/{team}/events/subject?type=forge_line&value={line}"),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        kinds(page["events"].as_array().expect("events"))
            .contains(&"forge.line.opened/1".to_string()),
        "{page}"
    );

    // A subject kind the ledger has no name for is the request's
    // grammar error; a well-formed subject nothing references is an
    // empty page rather than a 404.
    let (status, refused) = call(
        &h.router,
        get_authed(
            &format!("/teams/{team}/events/subject?type=whatever&value={line}"),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // The value is the subject vocabulary's business rather than this
    // route's, and what that means differs by kind. A forge handle
    // outside #102's set is refused, because that type has a grammar…
    let (status, refused) = call(
        &h.router,
        get_authed(
            &format!("/teams/{team}/events/subject?type=forge_identity&value=who-knows"),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // …and a digest that is not one is carried through and matches
    // nothing. Deliberately not the have-check's rule for the same
    // notation: there a nonsense digest reading as "not held" would
    // send bytes wrongly, and here it asks about a subject nothing
    // wrote, whose honest answer is no events.
    let (status, empty) = call(
        &h.router,
        get_authed(
            &format!("/teams/{team}/events/subject?type=digest&value=not-a-digest"),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["events"], serde_json::json!([]), "{empty}");
    let (status, empty) = call(
        &h.router,
        get_authed(
            &format!(
                "/teams/{team}/events/subject?type=forge_line&value={}",
                Uuid::now_v7()
            ),
            &alice,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["events"], serde_json::json!([]), "{empty}");
    assert_eq!(empty["next_after"], serde_json::Value::Null, "{empty}");
}

/// The subject read pages on the same contract as the whole-stream
/// one: a full page carries a cursor, a short one ends the walk.
#[tokio::test]
async fn the_subject_read_pages_the_way_the_stream_read_does() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team = create_team(&h, &alice).await;
    let line = open_line(&h, &team, &alice, "ROOT").await;

    // Four events about this line: the open, and three renames.
    for name in ["one", "two", "three"] {
        let (status, said) = call(
            &h.router,
            post_authed(
                &format!("/teams/{team}/forge/lines/{line}/rename"),
                &alice,
                serde_json::json!({ "name": name }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{said}");
    }

    let uri = format!("/teams/{team}/events/subject?type=forge_line&value={line}");
    let (status, first) = call(&h.router, get_authed(&format!("{uri}&limit=2"), &alice)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["events"].as_array().expect("events").len(), 2);
    let cursor = first["next_after"].as_i64().expect("a full page cursors");

    let (status, second) = call(
        &h.router,
        get_authed(&format!("{uri}&limit=2&after={cursor}"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["events"].as_array().expect("events").len(), 2);

    let cursor = second["next_after"].as_i64().expect("a full page cursors");
    let (status, third) = call(
        &h.router,
        get_authed(&format!("{uri}&limit=2&after={cursor}"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{third}");
    assert_eq!(third["events"], serde_json::json!([]), "{third}");
    assert_eq!(
        third["next_after"],
        serde_json::Value::Null,
        "a short page ends the walk: {third}"
    );

    // And the events it paged are this line's, not the team's — the
    // team's stream also holds the create.
    let whole = events_of(&h, &team, &alice).await;
    assert_eq!(whole.len(), 5, "{whole:#?}");
}
