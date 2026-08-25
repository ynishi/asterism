//! End-to-end guard for `teams-server backup`'s substance (#95, the
//! #83 §4 shape): the archive an instance writes restores to a
//! *working* instance — unpack, point a fresh server at the pair, and
//! an existing link serves its bytes end-to-end. The DB-first order
//! rule is asserted where it is readable: in the archive's own entry
//! order.
//!
//! The instance here is file-backed (a backup of `:memory:` would
//! prove nothing about the live-file/WAL hazard the snapshot exists to
//! avoid); everything it holds is put there through the real router.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sha2::{Digest as _, Sha256};
use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::backup::{ARCHIVE_DB_ENTRY, create_backup};
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::gc::GcGuard;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_server::rate_limit::RateLimiter;
use teams_server::state::{TeamsCtx, now_ms};
use tower::ServiceExt;

const GOOD: &str = "correct horse battery staple";

fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Assembles the binary's wiring over an explicit db path and blob
/// root — the same shape `serve` builds, which is exactly what restore
/// documentation tells the operator to point at the unpacked archive.
async fn instance(
    db_path: &Path,
    blob_root: &Path,
) -> (
    Arc<TeamsCtx>,
    Router,
    rusqlite_isle::AsyncIsle,
    rusqlite_isle::AsyncIsleDriver,
) {
    let (isle, driver) = teams_infra::sqlite::open_and_migrate(db_path)
        .await
        .expect("open teams db");
    let blobs = LocalFileStorageAdapter::open(blob_root)
        .await
        .expect("open blob store");
    let ctx = Arc::new(TeamsCtx {
        repo: SqliteTeamsRepository::new(isle.clone()),
        auth: PasswordAuth::new(isle.clone()),
        projections: teams_infra::sqlite::projection::SqliteProjectionStore::new(isle.clone()),
        blobs,
        registration: RegistrationPolicy::Open,
        session_ttl_ms: 60_000,
        auth_limiter: RateLimiter::new(1_000, Duration::from_secs(60)),
        purge_grace_ms: 0,
        gc_guard: Arc::new(GcGuard::new()),
    });
    let router = teams_server::http::router(ctx.clone());
    (ctx, router, isle, driver)
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

async fn login(ctx: &Arc<TeamsCtx>, router: &Router, login: &str, create: bool) -> String {
    if create {
        ctx.auth
            .create_account(login, login, GOOD, false, now_ms())
            .await
            .expect("create account");
    }
    let (status, body) = call(
        router,
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
    assert_eq!(status, StatusCode::OK, "login {login}: {body}");
    body["token"].as_str().unwrap().to_string()
}

fn entry_names(archive: &Path) -> Vec<String> {
    let file = std::fs::File::open(archive).unwrap();
    let mut tar = tar::Archive::new(file);
    tar.entries()
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .path()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[tokio::test]
async fn a_backup_restores_to_a_working_instance_and_the_archive_is_db_first() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("live");
    std::fs::create_dir_all(&live).unwrap();

    // --- The live instance: one user, one team, two blobs, all
    // through the real routes.
    let (ctx, router, isle, driver) = instance(&live.join("teams.db"), &live.join("blobs")).await;
    let alice = login(&ctx, &router, "alice", true).await;
    let (status, body) = call(
        &router,
        Request::builder()
            .method("POST")
            .uri("/teams/create")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {alice}"))
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create team: {body}");
    let team_id = body["team_id"].as_str().unwrap().to_string();
    let blobs: [&[u8]; 2] = [b"the first artefact", b"the second artefact"];
    for bytes in blobs {
        let digest = digest_of(bytes);
        let (status, body) = call(
            &router,
            Request::builder()
                .method("PUT")
                .uri(format!("/teams/{team_id}/blobs?digest={digest}"))
                .header("content-type", "application/octet-stream")
                .header("authorization", format!("Bearer {alice}"))
                .body(Body::from(bytes.to_vec()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "upload: {body}");
    }

    // --- The backup, over the live (still open, WAL-mode) instance.
    let archive = dir.path().join("backup.tar");
    let report = create_backup(&isle, &live.join("blobs"), &archive)
        .await
        .unwrap();
    assert_eq!(report.blob_files, 2);

    // DB first — the ordering rule, asserted in the artefact itself
    // (tar preserves entry order): the snapshot precedes every blob.
    let names = entry_names(&archive);
    assert_eq!(
        names[0], ARCHIVE_DB_ENTRY,
        "the DB snapshot leads: {names:?}"
    );
    // …and every blob the snapshot's links reference is in there.
    for bytes in blobs {
        let hex = digest_of(bytes);
        let hex = hex.strip_prefix("sha256:").unwrap();
        let expected = format!("blobs/sha256/{}/{hex}", &hex[..2]);
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }

    // The live instance's day is done.
    driver.shutdown().await.unwrap();

    // --- Restore, exactly as the command's help documents: unpack,
    // point a fresh server at db/teams.db and blobs/.
    let restored = dir.path().join("restored");
    tar::Archive::new(std::fs::File::open(&archive).unwrap())
        .unpack(&restored)
        .unwrap();
    let (r_ctx, r_router, _r_isle, r_driver) =
        instance(&restored.join(ARCHIVE_DB_ENTRY), &restored.join("blobs")).await;

    // The account came back with the state — a fresh login against the
    // restored instance, no re-provisioning.
    let alice = login(&r_ctx, &r_router, "alice", false).await;

    // And an existing link serves its bytes end-to-end.
    for bytes in blobs {
        let digest = digest_of(bytes);
        let response = r_router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/teams/{team_id}/blobs/{digest}"))
                    .header("authorization", format!("Bearer {alice}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "restored read {digest}");
        let got = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(got.as_ref(), bytes, "restored bytes {digest}");
    }

    r_driver.shutdown().await.unwrap();
}
