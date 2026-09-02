//! End-to-end guard for the purge two-step and the zero-link sweep
//! (#95, the #83 §3 lifecycle): mark hides a link behind the blob
//! route's one `404`, unmark restores it intact, reclaim respects the
//! grace window and triggers the sweep, and the events route shows the
//! whole story.
//!
//! Same wiring as the blob suite: the real router through `oneshot`,
//! an in-memory teams DB, a tempdir-backed blob store. The grace
//! window is per harness — `0` where reclaim should succeed at once,
//! an hour where it should refuse.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite_isle::AsyncIsleDriver;
use sha2::{Digest as _, Sha256};
use teams_core::domain::identity::RegistrationPolicy;
use teams_core::port::blob::BlobStore as _;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::gc::GcGuard;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;

const GOOD: &str = "correct horse battery staple";
const HOUR_MS: i64 = 60 * 60 * 1000;

fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

struct Harness {
    ctx: Arc<TeamsCtx>,
    router: Router,
    driver: AsyncIsleDriver,
    #[allow(dead_code)] // Held so the blob root outlives every request.
    blob_dir: tempfile::TempDir,
}

async fn harness(purge_grace_ms: i64) -> Harness {
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
        projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        device_token_ttl_ms: teams_server::state::DEFAULT_DEVICE_TOKEN_TTL_MS,
        device_token_idle_ms: None,
        auth_limiter: RateLimiter::new(1_000, Duration::from_secs(60)),
        purge_grace_ms,
        gc_guard: Arc::new(GcGuard::new()),
    });
    let router = teams_server::http::router(ctx.clone());
    Harness {
        ctx,
        router,
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

/// A request whose hit body is raw bytes (the blob read) — status
/// only, no JSON assumption.
async fn call_status(router: &Router, request: Request<Body>) -> StatusCode {
    router
        .clone()
        .oneshot(request)
        .await
        .expect("router response")
        .status()
}

fn post_authed(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build authed POST")
}

fn get_authed(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build authed GET")
}

fn put_blob(uri: &str, token: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/octet-stream")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(bytes))
        .expect("build blob PUT")
}

async fn user(h: &Harness, login: &str) -> String {
    provision(h, login, false).await
}

async fn admin_user(h: &Harness, login: &str) -> String {
    provision(h, login, true).await
}

async fn provision(h: &Harness, login: &str, admin: bool) -> String {
    h.ctx
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
    body["token"].as_str().unwrap().to_string()
}

async fn create_team(h: &Harness, token: &str) -> String {
    let (status, body) = call(
        &h.router,
        Request::builder()
            .method("POST")
            .uri("/teams/create")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create team: {body}");
    body["team_id"].as_str().unwrap().to_string()
}

/// Invites `user_id` as a plain member through the real route.
async fn invite(h: &Harness, team_id: &str, owner_token: &str, login: &str) -> String {
    let token = user(h, login).await;
    let (_, me) = call(
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
    let user_id = me["user_id"].as_str().unwrap();
    let (status, body) = call(
        &h.router,
        Request::builder()
            .method("POST")
            .uri(format!("/teams/{team_id}/members/invite"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {owner_token}"))
            .body(Body::from(
                serde_json::json!({ "user_id": user_id, "role": "member" }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "invite {login}: {body}");
    token
}

async fn upload(h: &Harness, team_id: &str, token: &str, bytes: &[u8]) -> String {
    let digest = digest_of(bytes);
    let (status, body) = call(
        &h.router,
        put_blob(
            &format!("/teams/{team_id}/blobs?digest={digest}"),
            token,
            bytes.to_vec(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload: {body}");
    digest
}

async fn event_kinds(h: &Harness, team_id: &str, token: &str) -> Vec<String> {
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/events"), token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "events: {body}");
    body["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect()
}

fn read_uri(team_id: &str, digest: &str) -> String {
    format!("/teams/{team_id}/blobs/{digest}")
}

fn mark_uri(team_id: &str, digest: &str) -> String {
    format!("/teams/{team_id}/blobs/{digest}/purge/mark")
}

fn unmark_uri(team_id: &str, digest: &str) -> String {
    format!("/teams/{team_id}/blobs/{digest}/purge/unmark")
}

fn reclaim_uri(team_id: &str) -> String {
    format!("/teams/{team_id}/blobs/purge/reclaim")
}

fn marked_uri(team_id: &str) -> String {
    format!("/teams/{team_id}/blobs/purge/marked")
}

#[tokio::test]
async fn the_purge_lifecycle_runs_end_to_end_and_the_stream_keeps_the_story() {
    // Grace 0: every mark is reclaimable the instant it lands.
    let h = harness(0).await;
    let alice = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let bytes = b"marked, restored, marked again, reclaimed".to_vec();
    let digest = upload(&h, &team_id, &alice, &bytes).await;

    // A canonical miss body, to compare the marked link's 404 against.
    let (miss_status, miss_body) = call(
        &h.router,
        get_authed(&read_uri(&team_id, &digest_of(b"never uploaded")), &alice),
    )
    .await;
    assert_eq!(miss_status, StatusCode::NOT_FOUND);

    // Mark: the receipt is the ledger event, member-stamped.
    let (status, body) = call(&h.router, post_authed(&mark_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK, "mark: {body}");
    assert_eq!(body["kind"], "teams.blob_link.purge_marked/1");
    assert_eq!(body["actor_kind"], "member");

    // Hidden from the read surface — and indistinguishable from a
    // digest that never existed, status and body alike.
    let (status, body) = call(&h.router, get_authed(&read_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "marked read: {body}");
    assert_eq!(
        body, miss_body,
        "a marked link and a never-linked digest must answer identically"
    );
    // The bytes themselves survive the grace window.
    assert!(h.ctx.blobs.exists(&digest).await.unwrap());

    // Inside the team the mark is sayable state: the marked-list read
    // shows the digest, its mark instant, and when reclaim may take it
    // — everything unmark needs (grace 0, so the two instants agree).
    let (status, body) = call(&h.router, get_authed(&marked_uri(&team_id), &alice)).await;
    assert_eq!(status, StatusCode::OK, "marked list: {body}");
    let marked = body["marked"].as_array().unwrap();
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0]["digest"], digest);
    let marked_at = marked[0]["marked_at_ms"].as_i64().unwrap();
    assert!(marked_at > 0);
    assert_eq!(marked[0]["reclaimable_at_ms"], marked_at);

    // Unmark: restored intact — the same bytes stream back.
    let (status, body) = call(
        &h.router,
        post_authed(&unmark_uri(&team_id, &digest), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unmark: {body}");
    assert_eq!(body["kind"], "teams.blob_link.purge_unmarked/1");
    let response = h
        .router
        .clone()
        .oneshot(get_authed(&read_uri(&team_id, &digest), &alice))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let got = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(got.as_ref(), bytes.as_slice(), "restored intact");
    let (status, body) = call(&h.router, get_authed(&marked_uri(&team_id), &alice)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["marked"],
        serde_json::json!([]),
        "the unmark emptied the marked list"
    );

    // Mark again, reclaim: the link goes, the event lands, the sweep
    // takes the now-unlinked bytes — reported as a count, never a
    // digest list (the sweep is instance-wide).
    let (status, _) = call(&h.router, post_authed(&mark_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_id), &alice)).await;
    assert_eq!(status, StatusCode::OK, "reclaim: {body}");
    assert_eq!(body["removed_digests"], serde_json::json!([digest]));
    assert_eq!(body["swept"], 1);
    assert!(
        body.get("swept_digests").is_none(),
        "the sweep must never be reported as digests: {body}"
    );
    assert_eq!(body["event"]["kind"], "teams.blob_link.reclaimed/1");

    // Bytes gone, link gone, same 404 as ever.
    assert!(!h.ctx.blobs.exists(&digest).await.unwrap(), "bytes swept");
    let (status, _) = call(&h.router, get_authed(&read_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The events route shows the whole mark/unmark/mark/reclaim
    // history — the record survives the bytes (#83 §3).
    let kinds = event_kinds(&h, &team_id, &alice).await;
    assert_eq!(
        kinds,
        vec![
            "teams.team.created/1",
            "teams.blob.copy_completed/1",
            "teams.blob_link.purge_marked/1",
            "teams.blob_link.purge_unmarked/1",
            "teams.blob_link.purge_marked/1",
            "teams.blob_link.reclaimed/1",
        ]
    );

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn reclaim_is_refused_while_the_grace_window_runs_and_unmark_still_works() {
    // An hour of grace: nothing marked in this test can ripen.
    let h = harness(HOUR_MS).await;
    let alice = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let digest = upload(&h, &team_id, &alice, b"still inside the window").await;

    // Reclaim with nothing marked at all: refused.
    let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_id), &alice)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "nothing marked: {body}");

    let (status, _) = call(&h.router, post_authed(&mark_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK);

    // Reclaim before the window elapses: refused, nothing removed,
    // bytes intact.
    let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_id), &alice)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "early reclaim: {body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("grace window has not elapsed"),
        "the refusal names the window: {body}"
    );
    assert!(h.ctx.blobs.exists(&digest).await.unwrap());

    // The mark is still restorable — the window bounds reclaim, not
    // unmark.
    let (status, _) = call(
        &h.router,
        post_authed(&unmark_uri(&team_id, &digest), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let status = call_status(&h.router, get_authed(&read_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn purge_authority_is_owner_or_admin_stamped_as_such() {
    let h = harness(0).await;
    let alice = user(&h, "alice").await;
    let op = admin_user(&h, "op").await;
    let team_id = create_team(&h, &alice).await;
    let bob = invite(&h, &team_id, &alice, "bob").await;
    let carol = user(&h, "carol").await;
    let digest = upload(&h, &team_id, &alice, b"authority under test").await;

    // A plain member may not mark, reclaim, or list the marked set; a
    // non-member is stopped by the gate itself. All 403 — the
    // owner-only convention every membership verb follows, not the
    // read surface's 404 conflation.
    for (token, who) in [(&bob, "member"), (&carol, "non-member")] {
        let (status, body) =
            call(&h.router, post_authed(&mark_uri(&team_id, &digest), token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who} mark: {body}");
        let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_id), token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who} reclaim: {body}");
        let (status, body) = call(&h.router, get_authed(&marked_uri(&team_id), token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who} marked list: {body}");
    }
    // Nothing landed from the refused attempts.
    let kinds = event_kinds(&h, &team_id, &alice).await;
    assert!(
        kinds.iter().all(|k| !k.contains("purge")),
        "refused verbs must write nothing: {kinds:?}"
    );

    // An admin — outside the roster — may run the whole two-step
    // (marked list included: whoever may unmark must see what is
    // marked), and every event reads back admin-stamped, never
    // disguised (#83 §1, the delete row's reclaim sibling).
    let (status, body) = call(&h.router, post_authed(&mark_uri(&team_id, &digest), &op)).await;
    assert_eq!(status, StatusCode::OK, "admin mark: {body}");
    assert_eq!(body["actor_kind"], "admin");
    let (status, body) = call(&h.router, get_authed(&marked_uri(&team_id), &op)).await;
    assert_eq!(status, StatusCode::OK, "admin marked list: {body}");
    assert_eq!(body["marked"][0]["digest"], digest);
    let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_id), &op)).await;
    assert_eq!(status, StatusCode::OK, "admin reclaim: {body}");
    assert_eq!(body["event"]["actor_kind"], "admin");

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_reclaim_never_sweeps_bytes_another_team_still_links() {
    let h = harness(0).await;
    let alice = user(&h, "alice").await;
    let bob = user(&h, "bob").await;
    let team_a = create_team(&h, &alice).await;
    let team_b = create_team(&h, &bob).await;
    let bytes = b"shared across teams".to_vec();
    let digest_a = upload(&h, &team_a, &alice, &bytes).await;
    let digest_b = upload(&h, &team_b, &bob, &bytes).await;
    assert_eq!(digest_a, digest_b);

    // Team A marks and reclaims its link. The bytes survive: team B
    // still links them — the sweep never deletes a linked blob.
    let (status, _) = call(
        &h.router,
        post_authed(&mark_uri(&team_a, &digest_a), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_a), &alice)).await;
    assert_eq!(status, StatusCode::OK, "team A reclaim: {body}");
    assert_eq!(body["removed_digests"], serde_json::json!([digest_a]));
    assert_eq!(body["swept"], 0, "linked elsewhere — nothing to sweep");
    assert!(h.ctx.blobs.exists(&digest_a).await.unwrap());
    let status = call_status(&h.router, get_authed(&read_uri(&team_b, &digest_b), &bob)).await;
    assert_eq!(status, StatusCode::OK, "team B still reads its link");

    // Team B lets go too — now the bytes have no link left, and the
    // sweep takes them.
    let (status, _) = call(&h.router, post_authed(&mark_uri(&team_b, &digest_b), &bob)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&h.router, post_authed(&reclaim_uri(&team_b), &bob)).await;
    assert_eq!(status, StatusCode::OK, "team B reclaim: {body}");
    assert_eq!(body["swept"], 1);
    assert!(!h.ctx.blobs.exists(&digest_b).await.unwrap());

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn re_uploading_a_marked_digest_is_a_distinguishable_conflict_that_changes_nothing() {
    let h = harness(HOUR_MS).await;
    let alice = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let bob = invite(&h, &team_id, &alice, "bob").await;
    let bytes = b"marked and then re-uploaded".to_vec();
    let digest = upload(&h, &team_id, &alice, &bytes).await;
    let (status, _) = call(&h.router, post_authed(&mark_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK);
    let events_before = event_kinds(&h, &team_id, &alice).await.len();

    // A member re-uploads the marked digest: not the plain "already
    // linked" (which would gaslight against the 404 every read gives),
    // but the purge-aware conflict naming the remedy — team-visible
    // state said to a member of the team that holds it (#95).
    let (status, body) = call(
        &h.router,
        put_blob(
            &format!("/teams/{team_id}/blobs?digest={digest}"),
            &bob,
            bytes.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "marked re-upload: {body}");
    assert_eq!(body["kind"], "marked_for_purge");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unmark") && message.contains("reclaim"),
        "the refusal names both remedies: {message}"
    );

    // No state change anywhere: the mark stands, no event landed, the
    // read surface still hides the link.
    assert_eq!(event_kinds(&h, &team_id, &alice).await.len(), events_before);
    let (status, body) = call(&h.router, get_authed(&marked_uri(&team_id), &alice)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["marked"].as_array().unwrap().len(), 1);
    let (status, _) = call(&h.router, get_authed(&read_uri(&team_id, &digest), &bob)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    h.driver.shutdown().await.unwrap();
}
