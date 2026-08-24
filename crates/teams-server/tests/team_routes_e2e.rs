//! End-to-end guard for the team/membership routes (#91): the
//! membership gate's statuses, the #83 §1 authority table, the
//! last-owner refusals, and role changes reading back old + new
//! through the events route.
//!
//! Drives the real router through `oneshot` over an in-memory teams
//! DB. Accounts are provisioned through the credential store (the v0
//! account source — the CLI wraps the same call) and every session is
//! minted through the real login route, so nothing here bypasses what
//! production traffic crosses.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;
use uuid::Uuid;

const GOOD: &str = "correct horse battery staple";

struct Harness {
    ctx: Arc<TeamsCtx>,
    router: Router,
    #[allow(dead_code)] // Held so the isle outlives every request.
    isle: AsyncIsle,
    driver: AsyncIsleDriver,
    #[allow(dead_code)] // Held so the blob root outlives every request.
    blob_dir: tempfile::TempDir,
}

async fn harness(registration: RegistrationPolicy) -> Harness {
    let (isle, driver) = teams_infra::sqlite::open_and_migrate_in_memory()
        .await
        .expect("open in-memory teams db");
    let blob_dir = tempfile::tempdir().expect("blob tempdir");
    let blobs = teams_infra::blob::LocalFileStorageAdapter::open(blob_dir.path().join("blobs"))
        .await
        .expect("open blob store");
    let ctx = Arc::new(TeamsCtx {
        repo: SqliteTeamsRepository::new(isle.clone()),
        auth: PasswordAuth::new(isle.clone()),
        blobs,
        registration,
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

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST")
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

/// Provisions an account and logs it in through the real route,
/// returning `(user_id, token)`.
async fn user(h: &Harness, login: &str, admin: bool) -> (Uuid, String) {
    let user_id = h
        .ctx
        .auth
        .create_account(login, login, GOOD, admin, now_ms())
        .await
        .expect("create account");
    let (status, body) = call(
        &h.router,
        post(
            "/teams/auth/login",
            serde_json::json!({ "login": login, "password": GOOD }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login for {login}: {body}");
    (user_id, body["token"].as_str().unwrap().to_string())
}

/// Creates a team through the route as `token`'s user, returning its
/// id.
async fn create_team(h: &Harness, token: &str) -> String {
    let (status, body) = call(
        &h.router,
        post_authed("/teams/create", token, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create team: {body}");
    body["team_id"].as_str().unwrap().to_string()
}

async fn events_of(h: &Harness, team_id: &str, token: &str) -> Vec<serde_json::Value> {
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/events"), token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "events: {body}");
    body["events"].as_array().expect("events array").clone()
}

#[tokio::test]
async fn the_gate_answers_each_failure_with_its_own_status() {
    let h = harness(RegistrationPolicy::Open).await;
    let (_alice_id, alice) = user(&h, "alice", false).await;
    let (_carol_id, carol) = user(&h, "carol", false).await;
    let team_id = create_team(&h, &alice).await;
    let roster_uri = format!("/teams/{team_id}/roster");

    // No token.
    let no_token = Request::builder()
        .uri(&roster_uri)
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(&h.router, no_token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "Unauthorized");

    // A token that never existed.
    let (status, _) = call(&h.router, get_authed(&roster_uri, "not-a-real-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Authenticated, but not a member.
    let (status, body) = call(&h.router, get_authed(&roster_uri, &carol)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["kind"], "Forbidden");

    // A team that does not exist.
    let ghost = Uuid::now_v7();
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{ghost}/roster"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["kind"], "NotFound");

    // The member reads their roster.
    let (status, body) = call(&h.router, get_authed(&roster_uri, &alice)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"].as_array().unwrap().len(), 1);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_authority_table_is_enforced_end_to_end() {
    let h = harness(RegistrationPolicy::Open).await;
    let (_alice_id, alice) = user(&h, "alice", false).await;
    let (bob_id, bob) = user(&h, "bob", false).await;
    let (carol_id, _carol) = user(&h, "carol", false).await;
    let (_op_id, op) = user(&h, "op", true).await;
    let team_id = create_team(&h, &alice).await;

    // The owner invites — permitted.
    let (status, body) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/members/invite"),
            &alice,
            serde_json::json!({ "user_id": bob_id.to_string(), "role": "member" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "owner invite: {body}");
    assert_eq!(body["kind"], "teams.membership.added/1");
    assert_eq!(body["actor_kind"], "member");

    // A plain member may do nothing in the table: invite, remove,
    // grant, revoke, delete — each is 403 and none leaves an event.
    let before = events_of(&h, &team_id, &alice).await.len();
    let attempts = [
        (
            format!("/teams/{team_id}/members/invite"),
            serde_json::json!({ "user_id": carol_id.to_string(), "role": "member" }),
        ),
        (
            format!("/teams/{team_id}/members/remove"),
            serde_json::json!({ "user_id": bob_id.to_string() }),
        ),
        (
            format!("/teams/{team_id}/owners/grant"),
            serde_json::json!({ "user_id": bob_id.to_string() }),
        ),
        (
            format!("/teams/{team_id}/owners/revoke"),
            serde_json::json!({ "user_id": bob_id.to_string() }),
        ),
        (format!("/teams/{team_id}/delete"), serde_json::json!({})),
    ];
    for (uri, payload) in &attempts {
        let (status, body) = call(&h.router, post_authed(uri, &bob, payload.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "member at {uri}: {body}");
    }
    assert_eq!(events_of(&h, &team_id, &alice).await.len(), before);

    // An admin, outside the roster, gets delete and nothing else
    // (#83 §1: no implicit invite / remove / grant / purge).
    for (uri, payload) in &attempts[..4] {
        let (status, body) = call(&h.router, post_authed(uri, &op, payload.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "admin at {uri}: {body}");
    }
    let (status, body) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/delete"),
            &op,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin delete: {body}");
    // …and the stamp says an admin did it — never disguised.
    assert_eq!(body["actor_kind"], "admin");
    assert_eq!(body["kind"], "teams.team.deleted/1");

    // The stream (which outlives the team row) agrees.
    let team_uuid = Uuid::parse_str(&team_id).unwrap();
    let stream = h.ctx.repo.events(team_uuid).await.unwrap();
    assert!(stream.last().unwrap().actor.is_admin());

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn closed_registration_flips_creation_to_an_admin() {
    let h = harness(RegistrationPolicy::Closed).await;
    let (alice_id, alice) = user(&h, "alice", false).await;
    let (op_id, op) = user(&h, "op", true).await;

    // A regular user may not create.
    let (status, body) = call(
        &h.router,
        post_authed("/teams/create", &alice, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "closed create: {body}");

    // An admin must name the founding owner — never implicitly a
    // member, so an ownerless create is malformed…
    let (status, body) = call(
        &h.router,
        post_authed("/teams/create", &op, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "ownerless create: {body}");

    // …and naming a user who has no account is refused too.
    let (status, _) = call(
        &h.router,
        post_authed(
            "/teams/create",
            &op,
            serde_json::json!({ "owner_user_id": Uuid::now_v7().to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Naming alice works: she owns the team, the admin stays outside
    // it, and the creation is admin-stamped.
    let (status, body) = call(
        &h.router,
        post_authed(
            "/teams/create",
            &op,
            serde_json::json!({ "owner_user_id": alice_id.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin create: {body}");
    let team_id = body["team_id"].as_str().unwrap().to_string();
    assert_eq!(body["event"]["actor_kind"], "admin");

    let (status, roster) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/roster"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let members = roster["members"].as_array().unwrap();
    assert_eq!(members.len(), 1, "the admin must not be in the roster");
    assert_eq!(members[0]["user_id"], alice_id.to_string());
    assert_eq!(members[0]["role"], "owner");

    // The bootstrap admin exists outside membership — no roster
    // anywhere holds op. (One team on the instance; its roster is
    // alice alone, asserted above; op's id differs.)
    assert_ne!(alice_id, op_id);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_user_founds_their_own_team_and_only_their_own() {
    let h = harness(RegistrationPolicy::Open).await;
    let (alice_id, alice) = user(&h, "alice", false).await;
    let (bob_id, _bob) = user(&h, "bob", false).await;

    // Naming yourself is the explicit spelling of the default.
    let (status, body) = call(
        &h.router,
        post_authed(
            "/teams/create",
            &alice,
            serde_json::json!({ "owner_user_id": alice_id.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "self-named create: {body}");
    assert_eq!(body["event"]["actor_kind"], "member");

    // Naming someone else is refused — founding a team you will not
    // own is an admin's move, under closed registration.
    let (status, body) = call(
        &h.router,
        post_authed(
            "/teams/create",
            &alice,
            serde_json::json!({ "owner_user_id": bob_id.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "foreign owner: {body}");

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_last_owner_refusals_leave_state_and_stream_untouched() {
    let h = harness(RegistrationPolicy::Open).await;
    let (alice_id, alice) = user(&h, "alice", false).await;
    let (bob_id, _bob) = user(&h, "bob", false).await;
    let team_id = create_team(&h, &alice).await;
    call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/members/invite"),
            &alice,
            serde_json::json!({ "user_id": bob_id.to_string(), "role": "member" }),
        ),
    )
    .await;
    let events_before = events_of(&h, &team_id, &alice).await.len();

    // Removing the last owner, and the self-demotion spelling of it —
    // both 409, the "state refuses this" status, not a 500 and not a
    // 400.
    for (uri, payload) in [
        (
            format!("/teams/{team_id}/members/remove"),
            serde_json::json!({ "user_id": alice_id.to_string() }),
        ),
        (
            format!("/teams/{team_id}/owners/revoke"),
            serde_json::json!({ "user_id": alice_id.to_string() }),
        ),
    ] {
        let (status, body) = call(&h.router, post_authed(&uri, &alice, payload)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{uri}: {body}");
        assert_eq!(body["kind"], "Conflict");
    }

    // Nothing moved: alice still owns, and the stream grew by nothing.
    let (_, roster) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/roster"), &alice),
    )
    .await;
    let members = roster["members"].as_array().unwrap();
    let alice_row = members
        .iter()
        .find(|m| m["user_id"] == alice_id.to_string())
        .expect("alice still in roster");
    assert_eq!(alice_row["role"], "owner");
    assert_eq!(events_of(&h, &team_id, &alice).await.len(), events_before);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_role_change_reads_back_old_and_new_through_the_events_route() {
    let h = harness(RegistrationPolicy::Open).await;
    let (_alice_id, alice) = user(&h, "alice", false).await;
    let (bob_id, _bob) = user(&h, "bob", false).await;
    let team_id = create_team(&h, &alice).await;
    call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/members/invite"),
            &alice,
            serde_json::json!({ "user_id": bob_id.to_string(), "role": "member" }),
        ),
    )
    .await;

    // Grant: the response is the appended event, old + new visible.
    let (status, granted) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/owners/grant"),
            &alice,
            serde_json::json!({ "user_id": bob_id.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "grant: {granted}");
    assert_eq!(granted["kind"], "teams.membership.role_changed/1");
    let payload: serde_json::Value =
        serde_json::from_str(granted["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(payload["old"], "member");
    assert_eq!(payload["new"], "owner");

    // The same entry comes back through the events route — the write
    // really is in the stream, not only in the response.
    let events = events_of(&h, &team_id, &alice).await;
    let last = events.last().unwrap();
    assert_eq!(last["kind"], "teams.membership.role_changed/1");
    assert_eq!(last["event_id"], granted["event_id"]);
    assert_eq!(last["seq"], granted["seq"]);
    let payload: serde_json::Value =
        serde_json::from_str(last["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(payload["old"], "member");
    assert_eq!(payload["new"], "owner");
    assert_eq!(payload["user_id"], bob_id.to_string());

    // And the reverse direction records the reverse pair.
    let (status, revoked) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/owners/revoke"),
            &alice,
            serde_json::json!({ "user_id": bob_id.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let payload: serde_json::Value =
        serde_json::from_str(revoked["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(payload["old"], "owner");
    assert_eq!(payload["new"], "member");

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_targets_are_validation_errors_not_500s() {
    let h = harness(RegistrationPolicy::Open).await;
    let (_alice_id, alice) = user(&h, "alice", false).await;
    let team_id = create_team(&h, &alice).await;
    let invite_uri = format!("/teams/{team_id}/members/invite");

    // A role word the domain does not admit.
    let (status, body) = call(
        &h.router,
        post_authed(
            &invite_uri,
            &alice,
            serde_json::json!({ "user_id": Uuid::now_v7().to_string(), "role": "admin" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad role: {body}");
    assert_eq!(body["kind"], "Validation");

    // A user id that is not a UUID.
    let (status, _) = call(
        &h.router,
        post_authed(
            &invite_uri,
            &alice,
            serde_json::json!({ "user_id": "not-a-uuid", "role": "member" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A well-formed id with no account behind it.
    let (status, _) = call(
        &h.router,
        post_authed(
            &invite_uri,
            &alice,
            serde_json::json!({ "user_id": Uuid::now_v7().to_string(), "role": "member" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Removing someone who is not a member — the domain's refusal,
    // surfaced as 400.
    let (bob_id, _) = user(&h, "bob", false).await;
    let (status, _) = call(
        &h.router,
        post_authed(
            &format!("/teams/{team_id}/members/remove"),
            &alice,
            serde_json::json!({ "user_id": bob_id.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_events_route_pages_and_hands_back_where_to_resume() {
    let h = harness(RegistrationPolicy::Open).await;
    let (_, alice) = user(&h, "alice", false).await;
    let team_id = create_team(&h, &alice).await;

    // Founding event plus one per invite, so the stream outgrows a
    // page of two.
    for name in ["bob", "carol", "dave", "erin"] {
        let (member_id, _) = user(&h, name, false).await;
        let (status, body) = call(
            &h.router,
            post_authed(
                &format!("/teams/{team_id}/members/invite"),
                &alice,
                serde_json::json!({ "user_id": member_id.to_string(), "role": "member" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "invite {name}: {body}");
    }

    // Walking in pages of two reproduces the whole stream in order,
    // with nothing repeated at a boundary and nothing skipped.
    let whole = events_of(&h, &team_id, &alice).await;
    assert!(whole.len() > 2, "the fixture must outgrow one page");

    let mut walked: Vec<serde_json::Value> = Vec::new();
    let mut after: Option<i64> = None;
    loop {
        let uri = match after {
            Some(seq) => format!("/teams/{team_id}/events?after={seq}&limit=2"),
            None => format!("/teams/{team_id}/events?limit=2"),
        };
        let (status, body) = call(&h.router, get_authed(&uri, &alice)).await;
        assert_eq!(status, StatusCode::OK, "page: {body}");
        let page = body["events"].as_array().unwrap().clone();
        assert!(page.len() <= 2, "a page never exceeds its limit");
        walked.extend(page);
        match body["next_after"].as_i64() {
            Some(seq) => after = Some(seq),
            None => break,
        }
    }
    assert_eq!(walked, whole);

    // The parameterless call is the first page, not the whole stream —
    // the contract change this route is making. With a default of 100
    // and a fixture of five that is the same list, so what pins the
    // change is the shape: an object carrying `events`, never the bare
    // array this used to answer with.
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/events"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.is_object(),
        "the events read answers with a page: {body}"
    );
    assert!(body["events"].is_array());

    // A limit above the ceiling is clamped rather than refused: asking
    // for more than there is, is not an error.
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/events?limit=100000"), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "clamped limit: {body}");
    assert_eq!(body["events"].as_array().unwrap().len(), whole.len());

    h.driver.shutdown().await.unwrap();
}
