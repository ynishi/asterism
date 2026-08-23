//! End-to-end guard for the model registry routes (#126, the first
//! serving step): operator-only publish, any-member read, verbatim
//! bytes, and supersession.
//!
//! Same wiring as the blob route guard — the real router over an
//! in-memory teams DB, nothing bypassed.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use teams_core::domain::identity::RegistrationPolicy;
use teams_core::domain::model_registry::ENTRY_SCHEMA_V1;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;

const GOOD: &str = "correct horse battery staple";
const REGISTRY: &str = "/teams/models/registry";

struct Harness {
    ctx: Arc<TeamsCtx>,
    router: Router,
    #[allow(dead_code)] // Held so the isle outlives every request.
    isle: AsyncIsle,
    #[allow(dead_code)]
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

/// Provisions an account and logs it in through the real route.
async fn provision(h: &Harness, login: &str, operator: bool) -> String {
    h.ctx
        .auth
        .create_account(login, login, GOOD, operator, now_ms())
        .await
        .expect("create account");
    let response = h
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/teams/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "login": login, "password": GOOD }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("login response");
    assert_eq!(response.status(), StatusCode::OK, "login for {login}");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    body["token"].as_str().unwrap().to_string()
}

fn put_entry(token: &str, body: impl Into<String>) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(REGISTRY)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.into()))
        .expect("build registry PUT")
}

fn get_entry(token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri(REGISTRY);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).expect("build registry GET")
}

/// An entry with provider-chosen formatting — the whitespace and key
/// order are part of what "verbatim" must preserve.
fn entry_bytes(model_id: &str, marker: &str) -> String {
    format!(
        "{{\n  \"schema\": \"{ENTRY_SCHEMA_V1}\",\n  \"model_id\": \"{model_id}\",\n  \"marker\": \"{marker}\"\n}}\n"
    )
}

async fn status_and_bytes(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

#[tokio::test]
async fn the_operator_publishes_and_a_member_reads_the_same_bytes() {
    let h = harness().await;
    let operator = provision(&h, "op", true).await;
    let member = provision(&h, "alice", false).await;
    let authored = entry_bytes("siglip2-base-patch16-256", "round-one");

    let (status, _, body) =
        status_and_bytes(&h.router, put_entry(&operator, authored.clone())).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "publish: {}",
        String::from_utf8_lossy(&body)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(receipt["model_id"], "siglip2-base-patch16-256");
    assert!(receipt["published_at_ms"].as_i64().unwrap() > 0);

    // The member reads back the provider's bytes, verbatim — not a
    // re-serialization (#126 decision 2: the entry is the trust
    // anchor, the instance is transport).
    let (status, headers, bytes) = status_and_bytes(&h.router, get_entry(Some(&member))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(String::from_utf8(bytes).unwrap(), authored);
}

#[tokio::test]
async fn publishing_again_supersedes_what_members_read() {
    let h = harness().await;
    let operator = provision(&h, "op", true).await;

    let (status, _, _) = status_and_bytes(
        &h.router,
        put_entry(&operator, entry_bytes("model-a", "old")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let replacement = entry_bytes("model-b", "new");
    let (status, _, _) =
        status_and_bytes(&h.router, put_entry(&operator, replacement.clone())).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, bytes) = status_and_bytes(&h.router, get_entry(Some(&operator))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(bytes).unwrap(), replacement);
}

#[tokio::test]
async fn the_write_is_the_operators_and_the_read_is_authenticated() {
    let h = harness().await;
    let member = provision(&h, "alice", false).await;

    // Before anything is published, the read is a plain 404.
    let (status, _, _) = status_and_bytes(&h.router, get_entry(Some(&member))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A non-operator publish is refused — the model is an instance
    // concern, and the instance capacity is the write authority.
    let (status, _, body) = status_and_bytes(
        &h.router,
        put_entry(&member, entry_bytes("model-a", "nope")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );

    // No token at all is the gate's usual 401.
    let (status, _, _) = status_and_bytes(&h.router, get_entry(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_carrier_validates_the_envelope_and_nothing_deeper() {
    let h = harness().await;
    let operator = provision(&h, "op", true).await;

    // Wrong schema tag, missing model_id, non-JSON: all 400, nothing
    // stored.
    for bad in [
        "{ \"schema\": \"not-a-schema\", \"model_id\": \"m\" }".to_string(),
        format!("{{ \"schema\": \"{ENTRY_SCHEMA_V1}\" }}"),
        "not json at all".to_string(),
    ] {
        let (status, _, body) = status_and_bytes(&h.router, put_entry(&operator, bad)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{}",
            String::from_utf8_lossy(&body)
        );
    }
    let (status, _, _) = status_and_bytes(&h.router, get_entry(Some(&operator))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nothing must have landed");

    // A body the member app would still have to verify (no files, no
    // digests) is carriable: the envelope is the instance's whole
    // reading (#126 decision 2).
    let thin = format!("{{ \"schema\": \"{ENTRY_SCHEMA_V1}\", \"model_id\": \"thin\" }}");
    let (status, _, _) = status_and_bytes(&h.router, put_entry(&operator, thin)).await;
    assert_eq!(status, StatusCode::OK);
}
