//! End-to-end guard for device tokens (#204): the mint behind the
//! gate, the login arm in front of it, the owner-scoped list and
//! revoke, and the instance where nobody ever asks for one.
//!
//! The suite drives the real router through `oneshot`, for
//! `auth_session_e2e`'s reason: what is being checked here is largely
//! *which router a route landed in* — a mint that slipped out from
//! behind the gate, or a login arm that slipped behind it, is a defect
//! no unit test of the adapter can see. The database is in-memory and
//! the credential store shares its isle, exactly as the binary wires
//! them.
//!
//! The claim the last test makes is the one #204 states as its
//! verification: an instance where no client ever asks for a device
//! token behaves as it did before this existed. That is why it asserts
//! about the table being empty rather than about the routes.

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
use teams_server::state::{
    DEFAULT_DEVICE_TOKEN_IDLE_MS, DEFAULT_DEVICE_TOKEN_TTL_MS, TeamsCtx, now_ms,
};
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

async fn harness() -> Harness {
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
        oidc: None,
        projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle.clone()),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        device_token_ttl_ms: DEFAULT_DEVICE_TOKEN_TTL_MS,
        device_token_idle_ms: Some(DEFAULT_DEVICE_TOKEN_IDLE_MS),
        // Generous: nothing here is about the limiter, and the arm
        // that shares its bucket is covered by `auth_session_e2e`.
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
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build authed GET")
}

fn delete_authed(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build authed DELETE")
}

async fn device_rows(isle: &AsyncIsle) -> i64 {
    isle.call(|conn| conn.query_row("SELECT count(*) FROM device_token", [], |r| r.get(0)))
        .await
        .expect("count device tokens")
}

/// An account, and a session token from the password arm.
async fn account_with_session(h: &Harness, login: &str, display: &str) -> String {
    h.ctx
        .auth
        .create_account(login, display, GOOD, false, now_ms())
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
    assert_eq!(status, StatusCode::OK);
    body["token"].as_str().expect("session token").to_string()
}

async fn mint(h: &Harness, session: &str, label: &str) -> (String, String) {
    let (status, body) = call(
        &h.router,
        post_authed(
            "/teams/auth/device",
            session,
            serde_json::json!({ "label": label }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mint refused: {body}");
    (
        body["token"].as_str().expect("device token").to_string(),
        body["id"].as_str().expect("handle").to_string(),
    )
}

#[tokio::test]
async fn a_minted_token_logs_in_and_the_session_it_makes_is_an_ordinary_one() {
    let h = harness().await;
    let session = account_with_session(&h, "hoshino", "Hoshino").await;
    let (device_token, id) = mint(&h, &session, "Hoshino's MacBook").await;
    assert!(!device_token.is_empty());
    assert!(!id.is_empty());

    // The device arm answers with a session in the same shape the
    // password arm answers with…
    let (status, body) = call(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": device_token }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], "Hoshino");
    assert_eq!(body["admin"], false);
    let from_device = body["token"].as_str().expect("session token").to_string();
    assert_ne!(
        from_device, device_token,
        "the session is minted, not the device token handed back"
    );

    // …and it is one the gate accepts, which is the whole claim: past
    // this point nothing can tell how the caller logged in.
    let (status, _) = call(
        &h.router,
        post_authed("/teams/create", &from_device, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Presenting it again mints a second session — a device token is
    // not consumed by use.
    let (status, _) = call(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": device_token }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn minting_needs_a_session_and_a_label() {
    let h = harness().await;
    let session = account_with_session(&h, "hoshino", "Hoshino").await;

    // No session at all, and a session that resolves to nothing, are
    // the gate's one 401.
    for token in ["", "not-a-session"] {
        let (status, _) = call(
            &h.router,
            post_authed(
                "/teams/auth/device",
                token,
                serde_json::json!({ "label": "MacBook" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // A blank label is the adapter's refusal, surfaced as the house
    // 400 rather than a token nobody can tell apart from another.
    for label in ["", "   "] {
        let (status, body) = call(
            &h.router,
            post_authed(
                "/teams/auth/device",
                &session,
                serde_json::json!({ "label": label }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["kind"], "Validation");
    }
    assert_eq!(device_rows(&h.isle).await, 0, "nothing landed");

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_revoked_token_is_unauthorized() {
    let h = harness().await;
    let session = account_with_session(&h, "hoshino", "Hoshino").await;
    let (device_token, id) = mint(&h, &session, "MacBook").await;

    let (status, _) = call(
        &h.router,
        delete_authed(&format!("/teams/auth/device/{id}"), &session),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(device_rows(&h.isle).await, 0);

    let (status, body) = call(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": device_token }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "Unauthorized");

    // Revoking again, and revoking a handle that never existed, are
    // the same success — the route is no existence oracle for ids.
    for handle in [id.as_str(), &uuid::Uuid::now_v7().to_string()] {
        let (status, _) = call(
            &h.router,
            delete_authed(&format!("/teams/auth/device/{handle}"), &session),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    // A handle that is not a uuid is a refusal about the request's
    // grammar, which confirms nothing about any row.
    let (status, body) = call(
        &h.router,
        delete_authed("/teams/auth/device/not-a-uuid", &session),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["kind"], "Validation");

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_expired_token_is_unauthorized_and_its_row_is_gone() {
    let h = harness().await;
    let session = account_with_session(&h, "hoshino", "Hoshino").await;
    let user_id = h
        .ctx
        .auth
        .resolve_session(&session, now_ms())
        .await
        .expect("resolve the session")
        .expect("the session the password arm just issued")
        .user_id;

    // Minted far enough in the past that its fixed window has closed —
    // minted directly, so the clock does not have to be faked and the
    // route's own sweep is not what the assertion is about.
    let stale = h
        .ctx
        .auth
        .mint_device_token(
            user_id,
            "old laptop",
            now_ms() - h.ctx.device_token_ttl_ms - 1_000,
            h.ctx.device_token_ttl_ms,
        )
        .await
        .expect("mint a stale token");
    assert_eq!(device_rows(&h.isle).await, 1);

    let (status, body) = call(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": stale.token }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "expired must read as 401");
    assert_eq!(body["kind"], "Unauthorized");
    // The resolve meets the row before the sweep takes it, so the
    // reason is the token's own end and not "revoked" (#163).
    assert_eq!(body["reason"], "expired", "{body}");
    // Gone: the resolve deleted it on touch, and the sweep after it
    // would have taken it anyway. That the row cannot outlive its
    // expiry is the fact this surface owes.
    assert_eq!(device_rows(&h.isle).await, 0);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_listing_carries_the_label_and_never_the_token() {
    let h = harness().await;
    let session = account_with_session(&h, "hoshino", "Hoshino").await;
    let (device_token, id) = mint(&h, &session, "Hoshino's MacBook").await;

    let (status, body) = call(&h.router, get_authed("/teams/auth/device", &session)).await;
    assert_eq!(status, StatusCode::OK);
    let tokens = body["tokens"].as_array().expect("tokens").clone();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["id"], id);
    assert_eq!(tokens[0]["label"], "Hoshino's MacBook");
    assert!(
        tokens[0]["last_used_at_ms"].is_null(),
        "nothing has presented it yet"
    );

    // Neither the value nor its digest appears anywhere in the body —
    // asserted over the whole serialisation rather than field by
    // field, so a field added later cannot smuggle one in.
    let rendered = body.to_string();
    assert!(
        !rendered.contains(&device_token),
        "the token is in {rendered}"
    );
    let digest = {
        use sha2::{Digest, Sha256};
        Sha256::digest(device_token.as_bytes())
            .iter()
            .fold(String::new(), |mut out, byte| {
                use std::fmt::Write as _;
                let _ = write!(out, "{byte:02x}");
                out
            })
    };
    assert!(!rendered.contains(&digest), "the hash is in {rendered}");

    // Using it stamps the listing, which is what a person reads before
    // deciding whether a device is still theirs.
    let (status, _) = call(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": device_token }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = call(&h.router, get_authed("/teams/auth/device", &session)).await;
    assert!(
        body["tokens"][0]["last_used_at_ms"].is_i64(),
        "a use must show: {body}"
    );

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn another_account_can_neither_see_nor_revoke_my_token() {
    let h = harness().await;
    let mine = account_with_session(&h, "hoshino", "Hoshino").await;
    let theirs = account_with_session(&h, "someone", "Someone Else").await;
    let (device_token, id) = mint(&h, &mine, "Hoshino's MacBook").await;

    // Their listing is theirs, and it is empty.
    let (status, body) = call(&h.router, get_authed("/teams/auth/device", &theirs)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["tokens"].as_array().expect("tokens").is_empty(),
        "another account's tokens must not be listed: {body}"
    );

    // Their revoke answers the same 204 a handle that named nothing
    // gets — and takes nothing, which is the half that matters.
    let (status, _) = call(
        &h.router,
        delete_authed(&format!("/teams/auth/device/{id}"), &theirs),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(device_rows(&h.isle).await, 1);
    let (status, _) = call(
        &h.router,
        post(
            "/teams/auth/device/login",
            serde_json::json!({ "token": device_token }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the owner's token still resolves");

    // And the owner can still take it.
    let (status, _) = call(
        &h.router,
        delete_authed(&format!("/teams/auth/device/{id}"), &mine),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(device_rows(&h.isle).await, 0);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_instance_where_nobody_mints_holds_no_device_token() {
    // #204's verification, stated as a test: the password flow is
    // untouched by any of this, and an instance whose clients never
    // ask for a device token has nothing on disk that could be one.
    let h = harness().await;
    let session = account_with_session(&h, "hoshino", "Hoshino").await;

    let (status, _) = call(
        &h.router,
        post_authed("/teams/create", &session, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &h.router,
        post_authed("/teams/auth/logout", &session, serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(device_rows(&h.isle).await, 0);

    h.driver.shutdown().await.unwrap();
}
