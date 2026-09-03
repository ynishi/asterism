//! End-to-end guard for an admin's reach over somebody else's sign-in
//! (#213): the account verbs the instance admin had no route for.
//!
//! What #213 states as its verification: an admin lists another
//! account's devices and revokes them all, and that account's next
//! device login is a `401` whose reason names the instance; an admin
//! locks an account and every way in refuses it while its ledger
//! stamps still resolve — the password, device and session arms here,
//! the provider arm in `oidc_sign_in_e2e`, which has the provider to
//! drive it with; a member's own listing and revoke behave as
//! they did; a non-admin reaching any of these routes is `403`; and
//! the record of each act is readable from the instance's own record,
//! which is where this issue's first shape puts account-level acts.
//!
//! Driven through the real router by `oneshot`, for the reason
//! `device_token_e2e` gives.

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
    #[allow(dead_code)] // Held so the store outlives every request.
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

fn request(
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build request"),
        None => builder.body(Body::empty()).expect("build request"),
    }
}

/// An account (admin or not), its id, and a session from the password
/// arm.
async fn account(h: &Harness, login: &str, admin: bool) -> (String, String) {
    let user_id = h
        .ctx
        .auth
        .create_account(login, login, GOOD, admin, now_ms())
        .await
        .expect("create account");
    let (status, body) = call(
        &h.router,
        request(
            "POST",
            "/teams/auth/login",
            None,
            Some(serde_json::json!({ "login": login, "password": GOOD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (
        user_id.to_string(),
        body["token"].as_str().expect("session token").to_string(),
    )
}

async fn mint(h: &Harness, session: &str, label: &str) -> String {
    let (status, body) = call(
        &h.router,
        request(
            "POST",
            "/teams/auth/device",
            Some(session),
            Some(serde_json::json!({ "label": label })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mint refused: {body}");
    body["token"].as_str().expect("device token").to_string()
}

async fn device_login(h: &Harness, token: &str) -> (StatusCode, serde_json::Value) {
    call(
        &h.router,
        request(
            "POST",
            "/teams/auth/device/login",
            None,
            Some(serde_json::json!({ "token": token })),
        ),
    )
    .await
}

#[tokio::test]
async fn an_admin_signs_another_account_out_everywhere_and_the_device_is_told_who_did_it() {
    let h = harness().await;
    let (_, admin) = account(&h, "operator", true).await;
    let (hoshino, session) = account(&h, "hoshino", false).await;
    let laptop = mint(&h, &session, "Hoshino's laptop").await;
    let phone = mint(&h, &session, "Hoshino's phone").await;

    // The admin sees what the owner sees, and no more: two rows, no
    // values.
    let (status, body) = call(
        &h.router,
        request(
            "GET",
            &format!("/teams/admin/accounts/{hoshino}/devices"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tokens = body["tokens"].as_array().expect("tokens");
    assert_eq!(tokens.len(), 2);
    for row in tokens {
        assert!(
            row.get("token").is_none(),
            "a listing carries no value: {row}"
        );
        assert!(row.get("token_hash").is_none(), "nor a digest: {row}");
    }

    // Sign the account out everywhere.
    let (status, body) = call(
        &h.router,
        request(
            "DELETE",
            &format!("/teams/admin/accounts/{hoshino}/devices"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // Each device's next login says the instance did it — not the
    // `revoked` an owner's own act earns — and says it once: the
    // tombstone is read and gone.
    for token in [&laptop, &phone] {
        let (status, body) = device_login(&h, token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["reason"], "revoked_by_instance", "{body}");
        let (status, body) = device_login(&h, token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["reason"], "revoked", "{body}");
    }

    // The owner's listing is empty now, read through the password
    // session that minted the tokens — a session outlives the revoke
    // of what it minted (#204's rule, kept).
    let (status, body) = call(
        &h.router,
        request("GET", "/teams/auth/device", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tokens"].as_array().expect("tokens").len(), 0);

    // The act is on the account's record, stamped with the admin.
    let (status, body) = call(
        &h.router,
        request(
            "GET",
            &format!("/teams/admin/accounts/{hoshino}/events"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["locked_at_ms"], serde_json::Value::Null);
    let events = body["events"].as_array().expect("events");
    assert_eq!(events.len(), 1, "{body}");
    assert_eq!(events[0]["kind"], "devices_revoked");
    assert_eq!(events[0]["actor_name"], "operator");
    assert_eq!(events[0]["subject_user_id"], hoshino);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_locked_account_is_refused_every_way_in_while_its_stamps_keep_resolving() {
    let h = harness().await;
    let (operator, admin) = account(&h, "operator", true).await;
    let (hoshino, session) = account(&h, "hoshino", false).await;
    let signed_out = mint(&h, &session, "Hoshino's old phone").await;

    // Something the account did, so the ledger has a stamp to keep.
    let (status, body) = call(
        &h.router,
        request(
            "POST",
            "/teams/create",
            Some(&session),
            Some(serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let team_id = body["team_id"].as_str().expect("team id").to_string();

    // #213's own order: sign the person out everywhere, then lock. A
    // token minted between the two is what the lock answers for; the
    // one taken back before it is answered as taken back, lock or no
    // lock, so the lock never masks the end a token met.
    let (status, _) = call(
        &h.router,
        request(
            "DELETE",
            &format!("/teams/admin/accounts/{hoshino}/devices"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let device = mint(&h, &session, "Hoshino's laptop").await;

    let lock = format!("/teams/admin/accounts/{hoshino}/lock");
    let (status, body) = call(&h.router, request("POST", &lock, Some(&admin), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = device_login(&h, &signed_out).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], "revoked_by_instance", "{body}");

    // The password arm: the same 401 as an unknown login.
    let (status, _) = call(
        &h.router,
        request(
            "POST",
            "/teams/auth/login",
            None,
            Some(serde_json::json!({ "login": "hoshino", "password": GOOD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The device arm: a 401 that says why.
    let (status, body) = device_login(&h, &device).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], "locked", "{body}");
    // The session it already held: the gate's 401.
    let (status, _) = call(&h.router, request("GET", "/teams", Some(&session), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The ledger still names the founder by name, from the admin's
    // side of the gate.
    let (status, body) = call(
        &h.router,
        request(
            "GET",
            &format!("/teams/{team_id}/events"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stamped = body.to_string();
    assert!(
        stamped.contains("hoshino"),
        "the founder's stamp went: {stamped}"
    );

    // The record says who locked it and that it is locked.
    let (status, body) = call(
        &h.router,
        request(
            "GET",
            &format!("/teams/admin/accounts/{hoshino}/events"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["locked_at_ms"].is_i64(), "{body}");
    assert_eq!(body["events"][0]["kind"], "devices_revoked");
    assert_eq!(body["events"][1]["kind"], "locked");
    assert_eq!(body["events"][1]["actor_user_id"], operator);

    // Locking again records nothing; unlocking gives everything back.
    let (status, _) = call(&h.router, request("POST", &lock, Some(&admin), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&h.router, request("DELETE", &lock, Some(&admin), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = device_login(&h, &device).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the token was kept through the lock: {body}"
    );
    let (status, _) = call(&h.router, request("GET", "/teams", Some(&session), None)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the session was kept through the lock"
    );
    let (status, body) = call(
        &h.router,
        request(
            "GET",
            &format!("/teams/admin/accounts/{hoshino}/events"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["locked_at_ms"], serde_json::Value::Null);
    let kinds: Vec<&str> = body["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["devices_revoked", "locked", "unlocked"]);

    // An admin cannot lock themself.
    let (status, _) = call(
        &h.router,
        request(
            "POST",
            &format!("/teams/admin/accounts/{operator}/lock"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn everybody_signs_in_again_and_the_act_is_on_every_account_s_page() {
    let h = harness().await;
    let (_, admin) = account(&h, "operator", true).await;
    let (hoshino, session_a) = account(&h, "hoshino", false).await;
    let (_, session_b) = account(&h, "kanade", false).await;
    let a = mint(&h, &session_a, "A").await;
    let b = mint(&h, &session_b, "B").await;
    let mine = mint(&h, &admin, "the admin's own").await;

    let (status, _) = call(
        &h.router,
        request("DELETE", "/teams/admin/devices", Some(&admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for token in [&a, &b, &mine] {
        let (status, body) = device_login(&h, token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["reason"], "revoked_by_instance", "{body}");
    }

    // On the instance's record once, with no subject; on each
    // account's page, because it reached each.
    let (status, body) = call(
        &h.router,
        request("GET", "/teams/admin/events", Some(&admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events = body["events"].as_array().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["subject_user_id"], serde_json::Value::Null);
    let (status, body) = call(
        &h.router,
        request(
            "GET",
            &format!("/teams/admin/accounts/{hoshino}/events"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["events"].as_array().expect("events").len(), 1);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_member_reaching_any_admin_route_is_refused_and_their_own_verbs_stand() {
    let h = harness().await;
    let (hoshino, session) = account(&h, "hoshino", false).await;
    let (_, other) = account(&h, "kanade", false).await;
    let token = mint(&h, &session, "Hoshino's laptop").await;

    for (method, uri) in [
        ("GET", format!("/teams/admin/accounts/{hoshino}/devices")),
        ("DELETE", format!("/teams/admin/accounts/{hoshino}/devices")),
        ("POST", format!("/teams/admin/accounts/{hoshino}/lock")),
        ("DELETE", format!("/teams/admin/accounts/{hoshino}/lock")),
        ("GET", format!("/teams/admin/accounts/{hoshino}/events")),
        ("DELETE", "/teams/admin/devices".to_string()),
        ("GET", "/teams/admin/events".to_string()),
    ] {
        let (status, body) = call(&h.router, request(method, &uri, Some(&other), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
        // And nobody at all is the gate's 401, before any handler.
        let (status, _) = call(&h.router, request(method, &uri, None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }

    // Nothing happened to the account: its token still logs in, its
    // own listing and revoke behave as #204 left them.
    let (status, _) = device_login(&h, &token).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &h.router,
        request("GET", "/teams/auth/device", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = body["tokens"][0]["id"]
        .as_str()
        .expect("handle")
        .to_string();
    let (status, _) = call(
        &h.router,
        request(
            "DELETE",
            &format!("/teams/auth/device/{id}"),
            Some(&session),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = device_login(&h, &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], "revoked", "an owner's own revoke: {body}");

    // An account that does not exist is a 404 to an admin, and the
    // same 403 to a member: the handler's first check is the admin's,
    // so a member learns nothing about which accounts exist.
    let nobody = format!("/teams/admin/accounts/{}/devices", uuid::Uuid::now_v7());
    let (status, _) = call(&h.router, request("GET", &nobody, Some(&other), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, admin) = account(&h, "operator", true).await;
    let (status, _) = call(&h.router, request("GET", &nobody, Some(&admin), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    h.driver.shutdown().await.unwrap();
}
