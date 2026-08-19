//! End-to-end guard for the team blob routes (#93, the #83 §3
//! mechanics): the declared-digest contract on upload, the one-tx
//! link+event ordering, server-side-only dedupe, and the read
//! surface's indistinguishable `404`.
//!
//! Drives the real router through `oneshot` over an in-memory teams DB
//! and a tempdir-backed blob store — the same wiring the binary
//! assembles, nothing bypassed.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};
use sha2::{Digest as _, Sha256};
use teams_core::domain::identity::RegistrationPolicy;
use teams_core::port::blob::BlobStore as _;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;
use uuid::Uuid;

const GOOD: &str = "correct horse battery staple";

/// The shared digest notation, spelled by the client's own hasher —
/// which is exactly what a promotion does at promote time (#83 §3).
fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

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

/// A GET whose hit body is raw bytes, not JSON — returns the response
/// whole so tests can look at status, headers and body separately.
async fn call_raw(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
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

fn put_blob(uri: &str, token: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/octet-stream")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(bytes))
        .expect("build blob PUT")
}

/// Provisions an account and logs it in through the real route.
async fn user(h: &Harness, login: &str) -> (Uuid, String) {
    provision(h, login, false).await
}

/// Same, with the operator flag — the instance capacity of #83 §1.
async fn operator_user(h: &Harness, login: &str) -> (Uuid, String) {
    provision(h, login, true).await
}

async fn provision(h: &Harness, login: &str, operator: bool) -> (Uuid, String) {
    let user_id = h
        .ctx
        .auth
        .create_account(login, login, GOOD, operator, now_ms())
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

async fn events_of(h: &Harness, team_id: &str, token: &str) -> Vec<serde_json::Value> {
    let (status, body) = call(
        &h.router,
        get_authed(&format!("/teams/{team_id}/events"), token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "events: {body}");
    body.as_array().expect("events array").clone()
}

fn upload_uri(team_id: &str, digest: &str) -> String {
    format!("/teams/{team_id}/blobs?digest={digest}")
}

fn read_uri(team_id: &str, digest: &str) -> String {
    format!("/teams/{team_id}/blobs/{digest}")
}

#[tokio::test]
async fn the_happy_path_lands_blob_link_and_event_and_streams_back() {
    let h = harness().await;
    let (alice_id, alice) = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let bytes = b"the promoted artefact".to_vec();
    let digest = digest_of(&bytes);

    let (status, body) = call(
        &h.router,
        put_blob(&upload_uri(&team_id, &digest), &alice, bytes.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload: {body}");
    assert_eq!(body["digest"], digest);
    assert_eq!(body["event"]["kind"], "teams.blob.copy_completed/1");
    assert_eq!(body["event"]["actor_kind"], "member");
    assert_eq!(body["event"]["actor_user_id"], alice_id.to_string());

    // The event is really in the stream, readable through the events
    // route — not only in the response.
    let events = events_of(&h, &team_id, &alice).await;
    let last = events.last().unwrap();
    assert_eq!(last["kind"], "teams.blob.copy_completed/1");
    assert_eq!(last["event_id"], body["event"]["event_id"]);
    let subjects = last["subjects"].as_array().unwrap();
    assert_eq!(subjects.len(), 1);
    assert_eq!(subjects[0]["ref_type"], "blob");
    assert_eq!(subjects[0]["value"], digest);

    // And the bytes come back, streamed, typed, with their length.
    let (status, headers, got) =
        call_raw(&h.router, get_authed(&read_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, bytes);
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(
        headers["content-length"],
        bytes.len().to_string().as_str(),
        "length from the open handle"
    );

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_missing_declared_digest_is_refused_before_anything_lands() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let events_before = events_of(&h, &team_id, &alice).await.len();

    let (status, body) = call(
        &h.router,
        put_blob(
            &format!("/teams/{team_id}/blobs"),
            &alice,
            b"bytes".to_vec(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no digest: {body}");
    assert_eq!(body["kind"], "Validation");
    assert!(
        body["message"].as_str().unwrap().contains("mandatory"),
        "the refusal must say the digest is mandatory: {body}"
    );

    assert!(h.ctx.blobs.list().await.unwrap().is_empty(), "no blob");
    assert_eq!(events_of(&h, &team_id, &alice).await.len(), events_before);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_mismatch_rejects_the_whole_op_with_no_blob_no_link_no_event() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let declared = digest_of(b"what alice chose");
    let arriving = b"what the path held at upload".to_vec();
    let computed = digest_of(&arriving);
    let events_before = events_of(&h, &team_id, &alice).await.len();

    let (status, body) = call(
        &h.router,
        put_blob(&upload_uri(&team_id, &declared), &alice, arriving),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "mismatch: {body}");
    assert_eq!(body["kind"], "Conflict");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains(&declared) && message.contains(&computed),
        "the body must carry computed vs declared: {message}"
    );

    // Nothing landed on any layer: no blob, no link, no event.
    assert!(h.ctx.blobs.list().await.unwrap().is_empty(), "no blob");
    let team_uuid = Uuid::parse_str(&team_id).unwrap();
    assert!(
        h.ctx.repo.blob_links(team_uuid).await.unwrap().is_empty(),
        "no link"
    );
    assert_eq!(events_of(&h, &team_id, &alice).await.len(), events_before);

    // Neither digest reads back.
    for digest in [&declared, &computed] {
        let (status, _) = call(&h.router, get_authed(&read_uri(&team_id, digest), &alice)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_team_uploads_the_same_digest_in_full_and_gets_its_own_link() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (_bob_id, bob) = user(&h, "bob").await;
    let team_a = create_team(&h, &alice).await;
    let team_b = create_team(&h, &bob).await;
    let bytes = b"shared across teams".to_vec();
    let digest = digest_of(&bytes);

    let (status, first) = call(
        &h.router,
        put_blob(&upload_uri(&team_a, &digest), &alice, bytes.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first upload: {first}");

    // The second team's upload of the full body succeeds identically:
    // same status, same shape, its own event — nothing in the response
    // says the CAS already held the bytes (no skip signal).
    let (status, second) = call(
        &h.router,
        put_blob(&upload_uri(&team_b, &digest), &bob, bytes.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "second upload: {second}");
    assert_eq!(second["digest"], digest);
    assert_eq!(second["event"]["kind"], "teams.blob.copy_completed/1");
    assert_eq!(
        first.as_object().unwrap().keys().collect::<Vec<_>>(),
        second.as_object().unwrap().keys().collect::<Vec<_>>(),
        "both uploads answer with the same shape"
    );

    // One physical copy; two links; each team reads through its own.
    assert_eq!(h.ctx.blobs.list().await.unwrap(), vec![digest.clone()]);
    for (team, token) in [(&team_a, &alice), (&team_b, &bob)] {
        let team_uuid = Uuid::parse_str(team).unwrap();
        assert_eq!(h.ctx.repo.blob_links(team_uuid).await.unwrap().len(), 1);
        let (status, _, got) =
            call_raw(&h.router, get_authed(&read_uri(team, &digest), token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got, bytes);
    }

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_duplicate_link_is_the_repositorys_refusal_after_the_full_body() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let bytes = b"uploaded twice by the same team".to_vec();
    let digest = digest_of(&bytes);

    let (status, _) = call(
        &h.router,
        put_blob(&upload_uri(&team_id, &digest), &alice, bytes.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events_before = events_of(&h, &team_id, &alice).await.len();

    // The re-upload is refused with the #89 repository's own answer
    // (`400 Validation`, "already linked") — after the body has been
    // accepted in full. One link, one event, one copy remain.
    let (status, body) = call(
        &h.router,
        put_blob(&upload_uri(&team_id, &digest), &alice, bytes),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "duplicate: {body}");
    assert_eq!(body["kind"], "Validation");
    assert!(
        body["message"].as_str().unwrap().contains("already linked"),
        "the refusal names the duplicate link: {body}"
    );
    assert_eq!(events_of(&h, &team_id, &alice).await.len(), events_before);
    let team_uuid = Uuid::parse_str(&team_id).unwrap();
    assert_eq!(h.ctx.repo.blob_links(team_uuid).await.unwrap().len(), 1);
    assert_eq!(h.ctx.blobs.list().await.unwrap().len(), 1);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_read_miss_is_the_same_404_and_the_operator_reads_hits() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (_carol_id, carol) = user(&h, "carol").await;
    let (_op_id, op) = operator_user(&h, "op").await;
    let team_a = create_team(&h, &alice).await;
    let team_c = create_team(&h, &carol).await;
    let bytes = b"alice's blob".to_vec();
    let digest = digest_of(&bytes);
    let (status, _) = call(
        &h.router,
        put_blob(&upload_uri(&team_a, &digest), &alice, bytes.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Every way of missing — member, non-member, and operator arms —
    // one indistinguishable answer:
    let misses = [
        // A member asking their own team for a digest nobody uploaded.
        (read_uri(&team_a, &digest_of(b"never uploaded")), &alice),
        // A non-member probing the team that does hold the digest.
        (read_uri(&team_a, &digest), &carol),
        // A member asking their own team for a digest linked only
        // elsewhere — the CAS holds it; their team does not.
        (read_uri(&team_c, &digest), &carol),
        // A team that does not exist at all.
        (read_uri(&Uuid::now_v7().to_string(), &digest), &alice),
        // The operator asking a team for a digest nobody uploaded —
        // the read capacity is general (§1), the link row still rules.
        (read_uri(&team_a, &digest_of(b"never uploaded")), &op),
        // The operator asking a team that does not hold the digest,
        // though another team (and the CAS) does.
        (read_uri(&team_c, &digest), &op),
        // The operator asking a team that does not exist.
        (read_uri(&Uuid::now_v7().to_string(), &digest), &op),
    ];
    let mut answers = Vec::new();
    for (uri, token) in misses {
        let (status, body) = call(&h.router, get_authed(&uri, token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} must be 404: {body}");
        answers.push(body);
    }
    assert!(
        answers.windows(2).all(|pair| pair[0] == pair[1]),
        "every miss must carry the identical body: {answers:?}"
    );

    // The member with the link still reads it — the 404s above are
    // visibility, not absence.
    let (status, _, _) = call_raw(&h.router, get_authed(&read_uri(&team_a, &digest), &alice)).await;
    assert_eq!(status, StatusCode::OK);

    // And the operator, outside every roster, reads the linked digest
    // too: §1's read boundary is general — roster, events, and blob
    // bytes alike.
    let (status, headers, got) =
        call_raw(&h.router, get_authed(&read_uri(&team_a, &digest), &op)).await;
    assert_eq!(status, StatusCode::OK, "operator read of a linked digest");
    assert_eq!(got, bytes);
    assert_eq!(headers["content-type"], "application/octet-stream");

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_induced_failure_after_the_cas_write_leaves_only_an_orphan_blob() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let team_id = create_team(&h, &alice).await;
    let bytes = b"orphaned by the induced failure".to_vec();
    let digest = digest_of(&bytes);
    let events_before = events_of(&h, &team_id, &alice).await.len();

    // Fail the ledger append inside the link transaction — after the
    // handler has already made the bytes durable in the CAS.
    h.isle
        .call(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER induced_failure
                 BEFORE INSERT ON ledger_event
                 WHEN new.kind = 'teams.blob.copy_completed/1'
                 BEGIN SELECT RAISE(ABORT, 'induced failure'); END;",
            )
        })
        .await
        .unwrap();

    let (status, body) = call(
        &h.router,
        put_blob(&upload_uri(&team_id, &digest), &alice, bytes.clone()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "induced failure: {body}"
    );

    // The tx rolled back whole: no link, no event — and the read
    // surface agrees the digest does not exist for the team.
    let team_uuid = Uuid::parse_str(&team_id).unwrap();
    assert!(h.ctx.repo.blob_links(team_uuid).await.unwrap().is_empty());
    assert_eq!(events_of(&h, &team_id, &alice).await.len(), events_before);
    let (status, _) = call(&h.router, get_authed(&read_uri(&team_id, &digest), &alice)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The orphan blob is the documented residue (#83 §3 ordering:
    // harmless, swept later) — bytes in the CAS, referenced by nothing.
    assert_eq!(h.ctx.blobs.list().await.unwrap(), vec![digest.clone()]);

    // With the trigger gone the same upload succeeds — converging on
    // the orphan's bytes, exactly the dedupe path.
    h.isle
        .call(|conn| conn.execute_batch("DROP TRIGGER induced_failure"))
        .await
        .unwrap();
    let (status, _) = call(
        &h.router,
        put_blob(&upload_uri(&team_id, &digest), &alice, bytes),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.ctx.blobs.list().await.unwrap().len(), 1);

    h.driver.shutdown().await.unwrap();
}

#[tokio::test]
async fn upload_authority_is_membership_and_malformed_digests_are_400() {
    let h = harness().await;
    let (_alice_id, alice) = user(&h, "alice").await;
    let (_carol_id, carol) = user(&h, "carol").await;
    let team_id = create_team(&h, &alice).await;
    let bytes = b"bytes".to_vec();

    // A non-member's upload is the gate's 403 — a mutation, like every
    // other team verb, not part of the read surface's 404 conflation.
    let (status, body) = call(
        &h.router,
        put_blob(
            &upload_uri(&team_id, &digest_of(&bytes)),
            &carol,
            bytes.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "non-member upload: {body}");

    // Digests the store does not admit: bare hex, wrong axis.
    for wrong in ["a".repeat(64), format!("cr1-sha256:{}", "a".repeat(64))] {
        let (status, body) = call(
            &h.router,
            put_blob(&upload_uri(&team_id, &wrong), &alice, bytes.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong}: {body}");
        assert_eq!(body["kind"], "Validation");
    }

    // A malformed digest on the read path is a 400 too — grammar,
    // not existence.
    let (status, _) = call(
        &h.router,
        get_authed(&read_uri(&team_id, &"a".repeat(64)), &alice),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    h.driver.shutdown().await.unwrap();
}
