//! End-to-end guard for auth v0 (#83 §5, the #91 slice): login /
//! logout, session expiry, and the one limiter every
//! credential-presenting endpoint sits behind.
//!
//! The suite drives the real router through `oneshot` — same
//! discipline as `asterism-server`'s route suites: a handler reached
//! by the wrong extractor or a gate that a route slipped out from
//! under is a defect no unit test of the adapter can see. The database
//! is in-memory; the credential store and repository share its isle,
//! exactly as the binary wires them.

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

const GOOD: &str = "correct horse battery staple";

struct Harness {
    ctx: Arc<TeamsCtx>,
    router: Router,
    isle: AsyncIsle,
    driver: AsyncIsleDriver,
    #[allow(dead_code)] // Held so the blob root outlives every request.
    blob_dir: tempfile::TempDir,
}

/// A context over a fresh in-memory teams DB. The limiter is generous
/// by default so only the test that is *about* the limiter trips it.
async fn harness(limiter: RateLimiter) -> Harness {
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
        projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle.clone()),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        auth_limiter: limiter,
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

fn generous() -> RateLimiter {
    RateLimiter::new(1_000, Duration::from_secs(60))
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

async fn session_rows(isle: &AsyncIsle) -> i64 {
    isle.call(|conn| conn.query_row("SELECT count(*) FROM auth_session", [], |r| r.get(0)))
        .await
        .expect("count sessions")
}

#[tokio::test]
async fn login_issues_a_session_and_a_wrong_password_does_not() {
    let h = harness(generous()).await;
    let user_id = h
        .ctx
        .auth
        .create_account("hoshino", "Hoshino", GOOD, false, now_ms())
        .await
        .unwrap();

    // Success: a token that the gate accepts.
    let (status, body) = call(
        &h.router,
        post(
            "/teams/auth/login",
            serde_json::json!({ "login": "hoshino", "password": GOOD }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token").to_string();
    assert!(!token.is_empty());
    assert_eq!(body["user_id"], user_id.to_string());
    assert_eq!(body["display_name"], "Hoshino");
    assert_eq!(body["admin"], false);
    let (status, _) = call(
        &h.router,
        post_authed("/teams/create", &token, serde_json::json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the issued session must pass the gate"
    );

    // A wrong password and an unknown login are the same 401 — the
    // response must not say which half failed.
    for (login, password) in [("hoshino", "wrong-password"), ("nobody", GOOD)] {
        let (status, body) = call(
            &h.router,
            post(
                "/teams/auth/login",
                serde_json::json!({ "login": login, "password": password }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["kind"], "Unauthorized");
    }

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_expired_session_is_rejected_and_cleaned_up() {
    let h = harness(generous()).await;
    let user_id = h
        .ctx
        .auth
        .create_account("hoshino", "Hoshino", GOOD, false, now_ms())
        .await
        .unwrap();

    // A session whose expiry already passed — minted directly so the
    // clock does not have to be faked.
    let token = h
        .ctx
        .auth
        .create_session(user_id, now_ms() - 120_000, 60_000)
        .await
        .unwrap();
    assert_eq!(session_rows(&h.isle).await, 1);

    let (status, body) = call(
        &h.router,
        post_authed("/teams/create", &token, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "expired must read as 401");
    assert_eq!(body["kind"], "Unauthorized");
    // The rejection deleted the row — expiry cleanup on touch.
    assert_eq!(session_rows(&h.isle).await, 0);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn logout_destroys_the_session() {
    let h = harness(generous()).await;
    h.ctx
        .auth
        .create_account("hoshino", "Hoshino", GOOD, false, now_ms())
        .await
        .unwrap();
    let (_, body) = call(
        &h.router,
        post(
            "/teams/auth/login",
            serde_json::json!({ "login": "hoshino", "password": GOOD }),
        ),
    )
    .await;
    let token = body["token"].as_str().unwrap().to_string();

    let (status, _) = call(
        &h.router,
        post_authed("/teams/auth/logout", &token, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The token resolves to nothing now.
    let (status, _) = call(
        &h.router,
        post_authed("/teams/create", &token, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Logging out without any token is the missing-credential 401.
    let (status, _) = call(&h.router, post("/teams/auth/logout", serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn one_limiter_covers_every_credential_presenting_endpoint() {
    // Three attempts per window; in-process requests share one key.
    let h = harness(RateLimiter::new(3, Duration::from_secs(60))).await;
    h.ctx
        .auth
        .create_account("hoshino", "Hoshino", GOOD, false, now_ms())
        .await
        .unwrap();

    for _ in 0..3 {
        let (status, _) = call(
            &h.router,
            post(
                "/teams/auth/login",
                serde_json::json!({ "login": "hoshino", "password": "wrong-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // The fourth attempt is refused before credentials are looked at —
    // a correct password gets the same 429.
    let (status, body) = call(
        &h.router,
        post(
            "/teams/auth/login",
            serde_json::json!({ "login": "hoshino", "password": GOOD }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["kind"], "RateLimited");

    // One bucket, not one per route (#83 §5). Logout presents a token
    // of its own and resolves it itself, so it sits in the limited
    // block beside the login arm and finds the same budget already
    // spent. The device verbs present a session the gate has resolved
    // and sit outside it — `http`'s module doc is where that line is
    // drawn. Nothing asserts which side they landed on; the router is
    // where it is visible, in one block.
    let (status, _) = call(
        &h.router,
        post_authed("/teams/auth/logout", "irrelevant", serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    h.driver.shutdown().await.unwrap();
}
