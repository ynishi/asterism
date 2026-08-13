//! End-to-end guard for the material-layer routes.
//!
//! Eight routes carry the bands over an Asset's material and the
//! chapters inside a structure band. The guards they answer to — who may
//! write into a band, which role holds chapters — are already tested
//! against the service (`asterism-infra/tests/material_layer_service.rs`)
//! and against the component (`MaterialChapters.test.ts`). What no test
//! at either level can see is the *wiring*, and these routes have more
//! of it than most: two of them take an id from the URL and overwrite the
//! one in the body, three different verbs sit behind `POST` on paths that
//! differ only in their last segment, and one of them is a `GET` and a
//! `POST` on the same path.
//!
//! So this file drives the real [`asterism_server::http::router`] through
//! `oneshot`, the way `newly_exposed_routes_e2e.rs` does, and asserts
//! three things the wiring can get wrong:
//!
//! 1. the read path answers at all, and answers with the shape the
//!    surface parses — a band and its chapters in one payload;
//! 2. the path id wins over the body on both routes that carry one,
//!    which is what keeps two callers disagreeing about the target from
//!    writing into different bands;
//! 3. a refusal from the service arrives as a status code, not as a
//!    panic or a 200 carrying an error body — this is the half that
//!    lives entirely in `ApiError`'s mapping.
//!
//! It does not re-test the guards themselves. One refusal is driven
//! through the router to pin the mapping; which acts are refused is the
//! service's business and is asserted there.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about route wiring, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// Spins up a core over a tempdir and returns it with the router built
/// on top.
///
/// `init_core_with` (rather than `init_core`) hands the Tantivy index a
/// tempdir: the default resolves the *active profile's* directory, so a
/// test run would write into whatever profile the developer is using.
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

/// One request through the router. Returns the status and the parsed
/// body, since a route can answer 200 with the wrong shape.
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

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build GET")
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST")
}

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some("video".into()),
        occurred_at_ms: 1_785_000_000_000,
        session_id: None,
        external_session_key: None,
        external_key: None,
        bundle_id: None,
        labels: Vec::new(),
        register_note: None,
        platform: None,
        file_size_bytes: None,
        duration_ms: Some(600_000),
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

/// A persona and two assets under it, each backed by a real file.
///
/// Two because every assertion below needs somewhere the write must
/// *not* have landed; a single-asset fixture would let a route that
/// ignored its path id pass.
async fn seed(core: &CoreCtx, tmp: &std::path::Path, pack_id: &str) -> (String, String) {
    let corpus = tmp.join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(pack_id.into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for name in ["subject.mkv", "decoy.mkv"] {
        let file = corpus.join(name);
        std::fs::write(&file, format!("placeholder for {name}\n")).expect("write file");
        let asset = core
            .asset_service
            .add(
                add_command(&persona.id, file.to_str().unwrap()),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(asset.id);
    }
    (ids[0].clone(), ids[1].clone())
}

/// The whole read path, and the shape a surface parses.
///
/// The empty listing is asserted before anything is created: an asset
/// with no bands is a normal asset, and a 404 there would make the panel
/// treat "nothing yet" as a failure.
///
/// The band comes back nested with its chapters, which is the thing the
/// asset-level GET exists for — a caller that had to fetch each band's
/// sections separately would pay a round trip per band to draw one
/// panel. So the assertion reads through `chapters` rather than
/// stopping at the layer.
#[tokio::test(flavor = "multi_thread")]
async fn the_bands_over_an_asset_are_readable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (asset, _decoy) = seed(&core, tmp.path(), "e2e-material-layer-read").await;

    let (status, body) = call(
        &router,
        get(&format!("/asterism/assets/{asset}/material-layers")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "empty listing: {body}");
    assert_eq!(body.as_array().expect("array").len(), 0);

    let (status, created) = call(
        &router,
        post(
            &format!("/asterism/assets/{asset}/material-layers"),
            serde_json::json!({
                "asset_id": asset,
                "material_ord": null,
                "role": "structure",
                "ord": 0,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create: {created}");
    // A band opened over this route is the person's own and is never the
    // default — both are the service's choice, and both are on the wire.
    assert_eq!(
        (
            created["origin"].as_str(),
            created["role"].as_str(),
            created["is_default"].as_bool()
        ),
        (Some("user"), Some("structure"), Some(false)),
        "created: {created}"
    );
    let layer = created["id"].as_str().expect("layer id").to_string();

    let (status, posted) = call(
        &router,
        post(
            &format!("/asterism/material-layers/{layer}/chapter-marks"),
            serde_json::json!({
                "layer_id": layer,
                "start_ms": 0,
                "end_ms": 90_000,
                "label": "Cold open",
                "ord": 0,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "post chapter: {posted}");

    let (status, listed) = call(
        &router,
        get(&format!("/asterism/assets/{asset}/material-layers")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "listing: {listed}");
    assert_eq!(listed.as_array().expect("array").len(), 1);
    assert_eq!(listed[0]["layer"]["id"], layer);
    assert_eq!(
        listed[0]["chapters"].as_array().expect("chapters").len(),
        1,
        "the asset-level read carries each band's sections with it: {listed}"
    );
    assert_eq!(listed[0]["chapters"][0]["label"], "Cold open");

    // The per-band GET is the same rows by the other door. It is what a
    // surface re-reads after editing one band, so it has to agree.
    let (status, chapters) = call(
        &router,
        get(&format!("/asterism/material-layers/{layer}/chapter-marks")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "per-band listing: {chapters}");
    assert_eq!(chapters, listed[0]["chapters"]);
}

/// The URL decides the target, on both routes that carry an id in the
/// path.
///
/// Each half names a *different* target in the body on purpose, and each
/// then checks that the named one stayed empty. Asserting only the
/// returned id would pass against a handler that wrote to both.
#[tokio::test(flavor = "multi_thread")]
async fn the_path_id_wins_over_the_one_in_the_body() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (asset, decoy) = seed(&core, tmp.path(), "e2e-material-layer-path-id").await;

    // --- POST /assets/{id}/material-layers ---------------------------
    let (status, created) = call(
        &router,
        post(
            &format!("/asterism/assets/{asset}/material-layers"),
            serde_json::json!({
                "asset_id": decoy,
                "material_ord": null,
                "role": "structure",
                "ord": 0,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create: {created}");
    assert_eq!(
        created["asset_id"], asset,
        "the URL decides the target asset, not the body"
    );

    let (_, decoy_bands) = call(
        &router,
        get(&format!("/asterism/assets/{decoy}/material-layers")),
    )
    .await;
    assert_eq!(
        decoy_bands.as_array().expect("array").len(),
        0,
        "the asset named in the body must not receive the band"
    );

    // --- POST /material-layers/{id}/chapter-marks --------------------
    let target = created["id"].as_str().expect("layer id").to_string();
    let (_, other) = call(
        &router,
        post(
            &format!("/asterism/assets/{decoy}/material-layers"),
            serde_json::json!({
                "asset_id": decoy,
                "material_ord": null,
                "role": "structure",
                "ord": 0,
            }),
        ),
    )
    .await;
    let other = other["id"].as_str().expect("layer id").to_string();

    let (status, posted) = call(
        &router,
        post(
            &format!("/asterism/material-layers/{target}/chapter-marks"),
            serde_json::json!({
                "layer_id": other,
                "start_ms": 0,
                "end_ms": null,
                "label": "in the target band",
                "ord": 0,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "post chapter: {posted}");
    assert_eq!(
        posted["layer_id"], target,
        "the URL decides the target band, not the body"
    );

    let (_, in_other) = call(
        &router,
        get(&format!("/asterism/material-layers/{other}/chapter-marks")),
    )
    .await;
    assert_eq!(
        in_other.as_array().expect("array").len(),
        0,
        "the band named in the body must not receive the section"
    );
}

/// A refusal reaches the caller as a status code.
///
/// The case is a write into an imported band — the band read out of the
/// material, whose contents are replaced by reading it again, so a hand
/// edit of it would survive until the next probe and then vanish. The
/// service answers that with `DomainError::Validation`, and what is
/// under test here is only the last step: that `ApiError` turns it into
/// **400** with a body the surface can read, rather than a 500, a panic,
/// or a 200 carrying an error.
///
/// The band is planted over a second connection to the same database,
/// because there is deliberately no route that creates an imported one —
/// a hand-made "imported" band would be a lie about where its contents
/// came from. The production writer is the `chapter_scan` job, and
/// driving a real container through it here would make a test about
/// status codes depend on ffmpeg. The plant is raw `INSERT` for the same
/// reason: no verb above the adapter will produce this row.
///
/// A 404 is asserted alongside it. The two are the whole mapping these
/// routes can reach, and confusing them is the mistake worth catching —
/// "you may not" and "there is no such band" send a caller to different
/// places.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_into_an_imported_band_is_refused_with_a_status_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db = tmp.path().join("asterism.db");
    let (core, router) = harness(tmp.path()).await;
    let (asset, _decoy) = seed(&core, tmp.path(), "e2e-material-layer-refusal").await;

    let imported = uuid::Uuid::now_v7();
    let asset_uuid = uuid::Uuid::parse_str(&asset).expect("asset id is a uuid");
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(&db)
        .await
        .expect("second isle");
    isle.call(move |conn| {
        conn.execute(
            "INSERT INTO material_layer
                 (id, asset_id, material_ord, origin, role, is_default, ord)
             VALUES (?1, ?2, 0, 'imported', 'structure', 1, 0)",
            rusqlite::params![imported, asset_uuid],
        )?;
        Ok(())
    })
    .await
    .expect("plant an imported band");
    driver.shutdown().await.ok();

    // It is readable — reading the file's own list is the ordinary case,
    // and a refusal here would mean the guard had been put on the wrong
    // verb.
    let (status, chapters) = call(
        &router,
        get(&format!(
            "/asterism/material-layers/{imported}/chapter-marks"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read an imported band: {chapters}");
    assert_eq!(chapters.as_array().expect("array").len(), 0);

    let (status, refused) = call(
        &router,
        post(
            &format!("/asterism/material-layers/{imported}/chapter-marks"),
            serde_json::json!({
                "layer_id": imported.to_string(),
                "start_ms": 0,
                "end_ms": null,
                "label": "by hand",
                "ord": 0,
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a write into an imported band is a refusal, not a server fault: {refused}"
    );
    assert_eq!(refused["kind"], "Validation", "{refused}");

    // Deleting the band is the other refused verb, and it is a different
    // route with its own extractor — a body id rather than a path one.
    let (status, refused) = call(
        &router,
        post(
            "/asterism/material-layers/delete",
            serde_json::json!({ "layer_id": imported.to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "delete band: {refused}");
    assert_eq!(refused["kind"], "Validation", "{refused}");

    // And a band that is not there is a different answer from one the
    // caller may not write to.
    let (status, missing) = call(
        &router,
        post(
            "/asterism/material-layers/delete",
            serde_json::json!({ "layer_id": uuid::Uuid::now_v7().to_string() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown band: {missing}");
    assert_eq!(missing["kind"], "NotFound", "{missing}");
}
