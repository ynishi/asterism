//! The preview-rendition loop, end to end: a VP9 WebM (the format the
//! webview cannot display) goes in, the status
//! endpoint reports `pending`, the `preview_gen` job transcodes it
//! through the real ffmpeg, and the status flips to `ready` with an
//! H.264 MP4 on disk. An MP4 asset answers `not_needed` without ever
//! paying for a transcode.
//!
//! Requires ffmpeg (fail-loud, like every fixture test here). Its own
//! test binary because `init_core` opens the profile-global Tantivy
//! index (one core per test binary, as with the sibling e2e files).

use std::process::Command;
use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about preview rendition, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: None,
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

fn synthesise(ffmpeg: &std::path::Path, dest: &std::path::Path) {
    let status = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=160x120:rate=10",
        ])
        .arg(dest)
        .status()
        .expect("synthesise fixture");
    assert!(
        status.success(),
        "ffmpeg could not write {}",
        dest.display()
    );
}

/// The same probe the transcode itself runs
/// (`$ASTERISM_FFMPEG` → the sidecar beside the executable → `PATH` →
/// the usual prefixes). Borrowed rather than reimplemented: this file
/// used to carry its own `PATH`-first copy with no sidecar step, so
/// the fixture could fail to find the binary the job under test would
/// have found.
fn ffmpeg_or_die() -> std::path::PathBuf {
    asterism_infra::jobs::thumb_ffmpeg::ffmpeg_binary()
        .expect("ffmpeg is required for this test: brew install ffmpeg (or set $ASTERISM_FFMPEG)")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vp9_webm_gains_a_playable_rendition_and_an_mp4_never_pays() {
    let ffmpeg = ffmpeg_or_die();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let webm = corpus.join("clip.webm");
    let mp4 = corpus.join("clip.mp4");
    synthesise(&ffmpeg, &webm);
    synthesise(&ffmpeg, &mp4);

    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-video-preview".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let webm_asset = core
        .asset_service
        .add(
            add_command(&persona.id, webm.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add webm");
    let mp4_asset = core
        .asset_service
        .add(
            add_command(&persona.id, mp4.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add mp4");

    // The natively-playable format never pays for a transcode.
    let native = core
        .asset_service
        .video_preview(&mp4_asset.id, None)
        .await
        .expect("mp4 status");
    assert_eq!(native.status, "not_needed");
    assert_eq!(native.path, None);

    // First ask answers pending and enqueues the transcode…
    let first = core
        .asset_service
        .video_preview(&webm_asset.id, None)
        .await
        .expect("webm status");
    assert_eq!(first.status, "pending", "first ask kicks the job off");

    // …and polling rides it to ready, exactly as the pane does.
    let mut ready = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let status = core
            .asset_service
            .video_preview(&webm_asset.id, None)
            .await
            .expect("poll status");
        match status.status.as_str() {
            "ready" => {
                ready = Some(status);
                break;
            }
            "pending" => continue,
            other => panic!("unexpected status {other:?} ({:?})", status.detail),
        }
    }
    let ready = ready.expect("the transcode finishes within the poll budget");
    let path = std::path::PathBuf::from(ready.path.expect("ready carries the path"));
    let bytes = std::fs::read(&path).expect("rendition on disk");
    assert_eq!(&bytes[4..8], b"ftyp", "the rendition is an MP4");
    assert!(
        path.starts_with(tmp.path()),
        "the rendition lives beside the sandboxed database, not in a user profile"
    );

    // The wire shape: same answer over the HTTP route.
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/asterism/assets/{}/video-preview", webm_asset.id))
                .body(axum::body::Body::empty())
                .expect("build request"),
        )
        .await
        .expect("route answers");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ready"));
}

/// Even the *status* is an existence oracle — a restricted asset must
/// answer as an absent one for an outside viewer, while the owner and a
/// shared subject still read it. No ffmpeg needed: the gate answers
/// before the format is even looked at, which the owner's `not_needed`
/// on a non-video file pins from the other side.
#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_assets_preview_status_is_absent_for_an_outside_viewer() {
    use asterism_core::domain::repository::AssetRepository;
    use asterism_core::domain::value::{AssetId, Visibility};

    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let png = corpus.join("still.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nstill").expect("write png");

    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-video-preview-visibility".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let asset = core
        .asset_service
        .add(
            add_command(&persona.id, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");

    // Restrict through the repository over a second handle — visibility
    // is not on the write surface yet (same shape as the sibling
    // visibility e2e files).
    {
        let (isle, driver) =
            asterism_infra::sqlite::open_and_migrate(&tmp.path().join("asterism.db"))
                .await
                .expect("second isle");
        let repo = asterism_infra::sqlite::repo::SqliteAssetRepository::new(isle);
        let id = AssetId::from_uuid(uuid::Uuid::parse_str(&asset.id).expect("uuid"));
        let mut row = repo
            .find(&id)
            .await
            .expect("find asset")
            .expect("asset exists");
        row.visibility = Visibility::Restricted {
            sharing: vec!["alice".into()],
        };
        repo.save(&row).await.expect("save asset");
        driver.shutdown().await.ok();
    }

    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let status_of = |uri: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri(uri)
                        .body(axum::body::Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("route answers");
            response.status()
        }
    };

    let uri = format!("/asterism/assets/{}/video-preview", asset.id);
    assert_eq!(
        status_of(uri.clone()).await,
        axum::http::StatusCode::OK,
        "no viewer_subject is the owner"
    );
    assert_eq!(
        status_of(format!("{uri}?viewer_subject=alice")).await,
        axum::http::StatusCode::OK,
        "a shared subject reads the status"
    );
    assert_eq!(
        status_of(format!("{uri}?viewer_subject=bob")).await,
        axum::http::StatusCode::NOT_FOUND,
        "an outside subject must not learn the asset exists"
    );
}
