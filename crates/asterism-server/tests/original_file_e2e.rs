//! End-to-end guard for `GET /asterism/assets/{id}/file` — the route
//! that made the library readable from off this machine.
//!
//! Until it existed the only binary an HTTP caller could obtain was a
//! thumbnail. The original came back as `locator`, a path on *this*
//! disk: a remote agent held a name with nothing behind it.
//!
//! What can be wrong here is not the byte copy — it is the four
//! not-the-bytes answers, each about a different subject, and the header
//! that tells the caller what it just received:
//!
//! - a restricted asset must be indistinguishable from an absent one
//! - a locator that is not a local file is a fact about the *asset*
//!   (409), not about the request (404)
//! - a locator whose file has gone is neither of those
//! - `Content-Type` comes from the material, and unknown must degrade to
//!   `application/octet-stream` rather than to a guess
//!
//! So these drive the real [`asterism_server::http::router`] through
//! `oneshot`, and read the response as bytes rather than JSON.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_core::domain::repository::AssetRepository;
use asterism_core::domain::value::{AssetId, Visibility};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Spins up a core over a tempdir and returns it with the router built
/// on top. Same shape as `newly_exposed_routes_e2e`: `init_core_with`
/// keeps the Tantivy index inside the tempdir instead of the developer's
/// active profile.
/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the file route, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

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

/// One GET through the router, read as raw bytes.
///
/// Returns the header pair the caller needs to interpret the payload
/// alongside it: a 200 carrying the right bytes under the wrong
/// `Content-Type` is still a defect for anything that renders them.
async fn get_bytes(
    router: &Router,
    uri: &str,
) -> (StatusCode, Option<String>, Option<String>, Vec<u8>) {
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
    let header = |name: axum::http::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = header(axum::http::header::CONTENT_TYPE);
    let content_length = header(axum::http::header::CONTENT_LENGTH);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec();
    (status, content_type, content_length, bytes)
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

/// Rewrites one asset's visibility straight through the repository.
///
/// There is no command for it — visibility is not on the write surface
/// yet — so the test reaches for the same port the service reads, over a
/// second handle on the same database file.
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

/// The happy path, plus the header contract that makes the bytes usable.
///
/// The fixture is deliberately not text: a byte-for-byte assertion over
/// ASCII would pass through any lossy re-encoding on the way out. The
/// second asset carries an extension the mime map does not name, which
/// is the only way to see the `application/octet-stream` fallback —
/// `guess_mime` answers `None` there, and `None` must not become a
/// guess.
#[tokio::test(flavor = "multi_thread")]
async fn the_original_comes_back_whole_and_typed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    // A PNG signature followed by bytes no UTF-8 decoder accepts.
    let mut png_bytes: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png_bytes.extend((0u16..600).map(|n| (n % 256) as u8));
    let png = corpus.join("star.png");
    std::fs::write(&png, &png_bytes).expect("write png");

    let opaque_bytes: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x01, 0x02];
    let opaque = corpus.join("blob.xyz");
    std::fs::write(&opaque, &opaque_bytes).expect("write opaque");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-original-file").await;
    let asset = core
        .asset_service
        .add(
            add_command(&persona, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");
    let untyped = core
        .asset_service
        .add(
            add_command(&persona, opaque.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add untyped asset");

    let (status, content_type, content_length, body) =
        get_bytes(&router, &format!("/asterism/assets/{}/file", asset.id)).await;
    assert_eq!(status, StatusCode::OK, "body: {:?}", body.len());
    assert_eq!(
        content_type.as_deref(),
        Some("image/png"),
        "the material's mime types the response"
    );
    assert_eq!(
        content_length.as_deref(),
        Some(png_bytes.len().to_string().as_str()),
        "a caller sizing a buffer reads this"
    );
    assert_eq!(body, png_bytes, "the original arrives byte for byte");

    let (status, content_type, _, body) =
        get_bytes(&router, &format!("/asterism/assets/{}/file", untyped.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        content_type.as_deref(),
        Some("application/octet-stream"),
        "an unknown format is served as opaque bytes, never as a guess"
    );
    assert_eq!(body, opaque_bytes);
}

/// Visibility is the same rule the detail read applies, on the same
/// axis: owner and a shared subject get the bytes, an outside subject
/// gets the answer an absent asset gives.
///
/// The three calls differ only in `viewer_subject`, so a handler that
/// dropped the parameter — or a service that skipped the check — cannot
/// pass by accident.
#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_original_is_absent_for_an_outside_viewer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let png = corpus.join("private.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nprivate").expect("write png");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-original-file-visibility").await;
    let asset = core
        .asset_service
        .add(
            add_command(&persona, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");
    restrict_to(&tmp.path().join("asterism.db"), &asset.id, &["alice"]).await;

    let uri = format!("/asterism/assets/{}/file", asset.id);

    let (status, _, _, _) = get_bytes(&router, &uri).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "no viewer_subject is the owner, who can always read"
    );

    let (status, _, _, _) = get_bytes(&router, &format!("{uri}?viewer_subject=alice")).await;
    assert_eq!(status, StatusCode::OK, "a shared subject reads the bytes");

    let (status, _, _, body) = get_bytes(&router, &format!("{uri}?viewer_subject=bob")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an outside subject must not be able to tell the asset exists"
    );
    assert_eq!(as_error(&body)["kind"], "NotFound");
}

/// An id nothing is filed under. Separate from the restricted case on
/// purpose: the two must be indistinguishable to the caller, which is
/// only worth asserting if both are pinned.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_asset_is_a_404() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;

    let (status, _, _, body) = get_bytes(
        &router,
        "/asterism/assets/0198c1c2-0000-7000-8000-000000000000/file",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(as_error(&body)["kind"], "NotFound");
}

/// The row is here, the locator is a path, and nothing is at that path.
///
/// Answered `404` because the status vocabulary is the shared
/// `DomainError` mapping; the message is what carries "the file, not the
/// asset" — so it is asserted rather than left to the status alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_vanished_original_names_the_file_in_its_404() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let png = corpus.join("gone.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\ngone").expect("write png");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-original-file-vanished").await;
    let asset = core
        .asset_service
        .add(
            add_command(&persona, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");

    // The ledger keeps the row; the disk loses the file. This is the
    // ordinary shape of an external corpus the user reorganises.
    std::fs::remove_file(&png).expect("remove png");

    let (status, _, _, body) =
        get_bytes(&router, &format!("/asterism/assets/{}/file", asset.id)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let error = as_error(&body);
    assert_eq!(error["kind"], "NotFound");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("asset original file"),
        "the message must say the file is missing, not the asset: {message}"
    );
    assert!(
        message.contains("gone.png"),
        "naming the path is what makes this repairable: {message}"
    );
}

/// A locator that never was a file on this disk. The asset exists and is
/// visible — what cannot be served is a property of the asset, so this
/// is a conflict and not a not-found.
#[tokio::test(flavor = "multi_thread")]
async fn a_remote_locator_is_a_conflict_not_a_404() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-original-file-remote").await;

    let remote = core
        .asset_service
        .add(
            add_command(&persona, "https://example.test/a.png"),
            &unattributed(),
        )
        .await
        .expect("add remote asset");
    // The other half of the same class: a record addressed inside a
    // container file. It has no bytes of its own either.
    let inside_container = core
        .asset_service
        .add(
            add_command(
                &persona,
                "/logs/session.jsonl#0198c1c2-aaaa-7000-8000-000000000000",
            ),
            &unattributed(),
        )
        .await
        .expect("add fragment asset");

    for (label, id) in [("remote", &remote.id), ("fragment", &inside_container.id)] {
        let (status, _, _, body) = get_bytes(&router, &format!("/asterism/assets/{id}/file")).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{label} locator: the asset is here, its original is not a file"
        );
        let error = as_error(&body);
        assert_eq!(error["kind"], "Conflict");
        assert!(
            error["message"]
                .as_str()
                .unwrap_or_default()
                .contains("not a local file"),
            "{label} locator: {error}"
        );
    }
}

/// The `file://` spellings. The scheme is consumed at the storage
/// boundary rather than carried, so `file:///pics/a.png` **is** the
/// locator `/pics/a.png` — one value, whatever case the scheme was
/// spelled in, and the served path is a path rather than a spelling.
///
/// The case-insensitivity is asserted through the identity it creates:
/// registering `FILE://<same path>` after `file://<same path>` is
/// answered with the row that is already there, because the two are one
/// locator. (Before the type they were two legal strings and the
/// consumer stripped the scheme on its own each time it needed a path.
/// Until V61 the second registration was *refused* by the UNIQUE; now
/// the lookup answers it, which is the difference between a constraint
/// and a lookup.)
///
/// A rootless `file://pics/a.png` must be refused, not resolved against
/// the server process CWD: after the scheme is consumed the path has no
/// root, so it is a name, and a name has no bytes.
#[tokio::test(flavor = "multi_thread")]
async fn file_scheme_spellings_serve_or_refuse_never_resolve_relative() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let bytes = b"\x89PNG\r\n\x1a\nschemed".to_vec();
    let png = corpus.join("schemed.png");
    std::fs::write(&png, &bytes).expect("write png");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-original-file-scheme").await;

    let lower = core
        .asset_service
        .add(
            add_command(&persona, &format!("file://{}", png.to_str().unwrap())),
            &unattributed(),
        )
        .await
        .expect("add file:// asset");
    // The same path, scheme spelled in upper case: the same locator, so
    // the lookup hands back the row that already holds it rather than
    // minting a second one.
    let upper = core
        .asset_service
        .add(
            add_command(&persona, &format!("FILE://{}", png.to_str().unwrap())),
            &unattributed(),
        )
        .await
        .expect("a second arrival is not an error");
    assert_eq!(
        upper.id, lower.id,
        "FILE:// and file:// name one locator, so this is that record arriving again"
    );
    // …and the bare spelling is that same locator too, which is the
    // whole reason the scheme is consumed.
    let bare = core
        .asset_service
        .add(
            add_command(&persona, png.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("nor is a third");
    assert_eq!(
        bare.id, lower.id,
        "the bare path is the same locator as the schemed one"
    );

    let rootless = core
        .asset_service
        .add(add_command(&persona, "file://pics/a.png"), &unattributed())
        .await
        .expect("add rootless file:// asset");

    let (status, _, _, body) =
        get_bytes(&router, &format!("/asterism/assets/{}/file", lower.id)).await;
    assert_eq!(status, StatusCode::OK, "the schemed row serves its file");
    assert_eq!(body, bytes, "and serves the file's own bytes");
    // The reference the caller gets back names the path, not the
    // spelling — `original_file` no longer re-derives one from the
    // other.
    let served = core
        .asset_service
        .original_file(&lower.id, None)
        .await
        .expect("a file:// locator resolves");
    assert_eq!(served.path, png);
    assert_eq!(served.locator, png.to_string_lossy());

    let (status, _, _, body) =
        get_bytes(&router, &format!("/asterism/assets/{}/file", rootless.id)).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a rootless file:// must not resolve against the process CWD"
    );
    assert_eq!(as_error(&body)["kind"], "Conflict");
}

/// A locator naming a directory. `open(2)` succeeds on one, so without
/// the regular-file check this would be a `200` whose body dies on the
/// first read (`EISDIR`) — a worse answer than any status.
#[tokio::test(flavor = "multi_thread")]
async fn a_directory_locator_is_a_conflict_not_a_dying_stream() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let (core, router) = harness(tmp.path()).await;
    let persona = register(&core, "e2e-original-file-dir").await;
    let asset = core
        .asset_service
        .add(
            add_command(&persona, corpus.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add dir-locator asset");

    let (status, _, _, body) =
        get_bytes(&router, &format!("/asterism/assets/{}/file", asset.id)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let error = as_error(&body);
    assert_eq!(error["kind"], "Conflict");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not a regular file"),
        "{error}"
    );
}
