//! End-to-end guard for the visibility gate on
//! `GET /asterism/assets/{id}/thumbs/{size_px}`.
//!
//! A thumbnail is a rendition of the artefact — it leaks exactly what
//! the original would — so the route applies the same filtering
//! contract as `/file` and the detail read: an asset restricted away
//! from `viewer_subject` answers as an absent one.
//!
//! Two shapes of leak are pinned, because they fail independently:
//!
//! - the **bytes**: a cached thumbnail served to an outside viewer
//! - the **existence**: this route's cache-miss branch is a `202`, so a
//!   gate placed after the cache probe would refuse the bytes yet
//!   confirm the asset is real through the 404/202 split. The gate must
//!   answer first, which also makes an unknown id a 404 rather than a
//!   queued 202.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_core::domain::repository::AssetRepository;
use asterism_core::domain::value::{AssetId, Visibility};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about thumbnail visibility,
/// not about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Same harness shape as `original_file_e2e`: core over a tempdir, the
/// real router on top.
async fn harness(tmp: &std::path::Path) -> (CoreCtx, Router) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    (core, router)
}

/// One GET through the router, read as status + raw bytes.
async fn get_bytes(router: &Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build GET");
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
        .to_bytes()
        .to_vec();
    (status, bytes)
}

/// The error envelope every non-2xx answer on this router carries.
fn as_error(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap_or_else(|e| {
        panic!(
            "error body is not JSON ({e}): {}",
            String::from_utf8_lossy(bytes)
        )
    })
}

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some("image".into()),
        occurred_at_ms: 1_785_000_000_000,
        session_id: None,
        external_session_key: None,
        external_key: None,
        bundle_id: None,
        labels: Vec::new(),
        register_note: None,
        platform: None,
        file_size_bytes: None,
        duration_ms: None,
        width_px: None,
        height_px: None,
        extra_json: None,
        cover_hint: None,
        auto_organize_base_dir: None,
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

async fn register(core: &CoreCtx, pack_id: &str) -> String {
    core.persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(pack_id.to_string()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona")
        .id
}

/// Rewrites one asset's visibility straight through the repository —
/// visibility is not on the write surface yet, so the test uses the
/// same port the service reads, over a second handle on the same
/// database file.
async fn restrict_to(db_path: &std::path::Path, asset_id: &str, sharing: &[&str]) {
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(db_path)
        .await
        .expect("second isle");
    let repo = asterism_infra::sqlite::repo::SqliteAssetRepository::new(isle);
    let id = AssetId::from_uuid(uuid::Uuid::parse_str(asset_id).expect("asset id is a uuid"));
    let mut asset = repo
        .find(&id)
        .await
        .expect("find asset")
        .expect("asset exists");
    asset.visibility = Visibility::Restricted {
        sharing: sharing.iter().map(|s| (*s).to_string()).collect(),
    };
    repo.save(&asset).await.expect("save asset");
    driver.shutdown().await.ok();
}

/// The bytes leak: a cached thumbnail of a restricted asset.
///
/// The three calls differ only in `viewer_subject`, so a handler that
/// dropped the parameter — or skipped the gate — cannot pass by
/// accident: owner and the shared subject read the bytes, an outside
/// subject gets the answer an absent asset gives.
#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_thumbnail_is_absent_for_an_outside_viewer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let png = corpus.join("private.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nprivate").expect("write png");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-thumb-visibility").await;
    let asset = core
        .asset_service
        .add(
            add_command(&persona, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");
    let thumb_bytes: Vec<u8> = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4];
    core.thumb_service
        .put(&asset.id, 128, thumb_bytes.clone())
        .await
        .expect("cache thumb");
    restrict_to(&tmp.path().join("asterism.db"), &asset.id, &["alice"]).await;

    let uri = format!("/asterism/assets/{}/thumbs/128", asset.id);

    let (status, body) = get_bytes(&router, &uri).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "no viewer_subject is the owner, who can always read"
    );
    assert_eq!(body, thumb_bytes, "the cached bytes come back whole");

    let (status, _) = get_bytes(&router, &format!("{uri}?viewer_subject=alice")).await;
    assert_eq!(status, StatusCode::OK, "a shared subject reads the bytes");

    let (status, body) = get_bytes(&router, &format!("{uri}?viewer_subject=bob")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an outside subject must not be able to tell the asset exists"
    );
    assert_eq!(as_error(&body)["kind"], "NotFound");
}

/// The existence leak: no cached thumbnail, so the ungated route would
/// answer 202. The outside viewer must still see the absent-asset
/// answer — a 202 here confirms the asset is real (and enqueues work on
/// a stranger's behalf), which is exactly the oracle the gate's
/// placement before the cache probe exists to close. The owner's 202 on
/// the same asset pins that the miss branch itself still works.
#[tokio::test(flavor = "multi_thread")]
async fn a_cache_miss_does_not_become_an_existence_oracle() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let png = corpus.join("uncached.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nuncached").expect("write png");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-thumb-oracle").await;
    let asset = core
        .asset_service
        .add(
            add_command(&persona, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");
    restrict_to(&tmp.path().join("asterism.db"), &asset.id, &["alice"]).await;

    let uri = format!("/asterism/assets/{}/thumbs/128", asset.id);

    let (status, body) = get_bytes(&router, &format!("{uri}?viewer_subject=bob")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a restricted asset with no cached thumb must not answer 202"
    );
    assert_eq!(as_error(&body)["kind"], "NotFound");

    let (status, _) = get_bytes(&router, &uri).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the owner still gets the queued-generation answer"
    );
}

/// An id nothing is filed under is a 404 — not a queued 202, which the
/// ungated route used to answer. Separate from the restricted case so
/// the "indistinguishable from absent" claim has both sides pinned.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_asset_is_a_404_not_a_queued_202() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;

    let (status, body) = get_bytes(
        &router,
        "/asterism/assets/0198c1c2-0000-7000-8000-000000000000/thumbs/128",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(as_error(&body)["kind"], "NotFound");
}
