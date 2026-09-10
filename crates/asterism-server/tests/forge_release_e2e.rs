//! Releasing a change point, end to end: the route, the freeze, the
//! run, and what the files carry when they land.
//!
//! The unit tests above this answer what a release *decides* — which
//! entries are members, what the assertion states, what a stamp's two
//! halves mean. What only a run can answer is whether the pieces meet:
//! whether the snapshot the route mints holds what the change point
//! carried, whether the dispatch it starts is a copy, whether the
//! copies carry a packet by the time the run reports done, and whether
//! the library's own file is left alone while that happens.
//!
//! The writer is unsigned, which is the state every install starts in
//! and the state this repository ships. So the manifest half is
//! `skipped` on every file, and asserting that is the point rather than
//! a concession: a build with no certificate is doing what it was
//! configured to do, and the release has to say so per file rather than
//! leave a reader to infer it. The signed half of the same question —
//! a manifest a reference reader reports intact under an untrusted
//! signer, carrying the release act and the pursuit's shape — is
//! answered in `asterism-infra`'s own tests, beside the certificate
//! fixture that can produce one.

use std::sync::Arc;

use asterism_contract::command::RegisterPersonaCommand;
use asterism_core::domain::measurement::{Measurement, MeasurementStatus};
use asterism_core::domain::repository::MaterialFingerprint;
use asterism_core::domain::value::{AssetId, DispatchId};
use asterism_infra::dispatch::{DispatchRunEnv, ExporterRegistry, ReEnqueue};
use asterism_infra::sqlite;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

// ---- the harness, as the other forge route suites have it -------

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
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "body is not JSON ({error}): {}",
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

/// Asserts 200 and hands back the body, naming the request that failed.
///
/// The URI is in the message because this suite drives a dozen routes
/// through one helper, and "404" without it says only that one of them
/// is wrong.
async fn ok(router: &Router, request: Request<Body>) -> serde_json::Value {
    let named = format!("{} {}", request.method(), request.uri());
    let (status, body) = call(router, request).await;
    assert_eq!(status, StatusCode::OK, "{named}: {body}");
    body
}

/// The 1×1 PNG that stands in for what a line carries.
///
/// Real bytes rather than a placeholder, because a copy is what this
/// suite is about: the exporter reads the file, the stamp rewrites the
/// copy, and both need a container something can be written into.
fn png() -> Vec<u8> {
    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(kind);
        hasher.update(payload);
        out.extend_from_slice(&hasher.finalize().to_be_bytes());
        out
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
    png.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x02]));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}

/// What a generated file's container says about itself.
///
/// Without it the disclosure establishes nothing and the packet is
/// skipped for having nothing to say — which is a real outcome and not
/// the one this suite is asking about.
fn comfy() -> String {
    serde_json::json!({ "Software": "ComfyUI", "workflow": "{}" }).to_string()
}

/// A persona, and `count` assets whose files exist on disk and whose
/// container metadata has been fingerprinted.
///
/// The fingerprint goes in through the repository rather than through a
/// route, on the same terms `disclosure_service`'s fixtures take: it
/// lands in the same statement as the digest it belongs to, so the
/// `material_hash` job is the only writer of it and this takes that
/// job's road rather than reaching around it. A second handle over the
/// same file is what a worker would hold.
async fn seed(
    router: &Router,
    tmp: &std::path::Path,
    count: usize,
) -> (String, Vec<String>, Vec<std::path::PathBuf>) {
    let persona = ok(
        router,
        post(
            "/asterism/personas/register",
            serde_json::to_value(RegisterPersonaCommand {
                name: "forge releases".into(),
                pack_id: None,
            })
            .expect("serialise"),
        ),
    )
    .await;
    let persona_id = persona["id"].as_str().expect("a persona id").to_string();

    let held = tmp.join("library");
    std::fs::create_dir_all(&held).expect("a library directory");
    let (isle, driver) = sqlite::open_and_migrate(&tmp.join("asterism.db"))
        .await
        .expect("a second handle on the same database");
    let assets = sqlite::repo::SqliteAssetRepository::new(isle);

    let mut ids = Vec::new();
    let mut paths = Vec::new();
    for nth in 0..count {
        let path = held.join(format!("{nth}.png"));
        std::fs::write(&path, png()).expect("write the library's own copy");
        let added = ok(
            router,
            post(
                "/asterism/assets/add",
                serde_json::json!({
                    "persona_id": persona_id,
                    "source_kind": "fs",
                    "locator": path.display().to_string(),
                    "modality": "image",
                    "occurred_at_ms": 1_785_000_000_000i64 + nth as i64,
                    "labels": [],
                }),
            ),
        )
        .await;
        let id = added["id"].as_str().expect("an asset id").to_string();
        let parsed = AssetId::from_uuid(uuid::Uuid::parse_str(&id).expect("a uuid"));
        asterism_core::domain::repository::AssetRepository::set_material_fingerprint(
            &assets,
            &parsed,
            0,
            &MaterialFingerprint {
                file: Measurement::bare(MeasurementStatus::NoBytes),
                content: Measurement::bare(MeasurementStatus::NoBytes),
                meta: Measurement::computed("m1-sha256:0".into()),
                meta_kv: Some(comfy()),
                meta_raw: None,
                meta_text: None,
            },
        )
        .await
        .expect("the fingerprint the hash job would have written");
        ids.push(id);
        paths.push(path);
    }
    // The handle is dropped and the driver with it; the core holds its
    // own over the same file.
    drop(driver);
    (persona_id, ids, paths)
}

/// A line carrying one entry per asset, landed by one satisfied close.
///
/// Answers `(line id, change point id, asset ids, library paths)`.
async fn a_landed_line(
    router: &Router,
    tmp: &std::path::Path,
    count: usize,
) -> (String, String, Vec<String>, Vec<std::path::PathBuf>) {
    let (persona, assets, paths) = seed(router, tmp, count).await;
    let line = ok(
        router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "release", "strategy_id": "mainline-first" }),
        ),
    )
    .await;
    let line_id = line["id"].as_str().expect("a line id").to_string();

    let work = ok(
        router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({ "line_id": line_id, "title": "the key visual" }),
        ),
    )
    .await;
    let work_id = work["id"].as_str().expect("a pursuit id").to_string();

    let ops: Vec<serde_json::Value> = assets
        .iter()
        .enumerate()
        .map(|(nth, asset)| {
            serde_json::json!({
                "entry_id": uuid::Uuid::now_v7().to_string(),
                "kind": "add",
                "content_asset_id": asset,
                "name": format!("cut-{nth}"),
            })
        })
        .collect();
    ok(
        router,
        post(
            &format!("/asterism/forge/pursuits/{work_id}/push"),
            serde_json::json!({ "ops": ops }),
        ),
    )
    .await;
    ok(
        router,
        post(
            &format!("/asterism/forge/pursuits/{work_id}/close"),
            serde_json::json!({ "outcome": "satisfied" }),
        ),
    )
    .await;

    let history = ok(router, get(&format!("/asterism/forge/lines/{line_id}"))).await;
    let point = history["changes"]
        .as_array()
        .expect("a chain")
        .last()
        .expect("one change point")["id"]
        .as_str()
        .expect("a change point id")
        .to_string();
    let _ = persona;
    (line_id, point, assets, paths)
}

/// A re-enqueue that records nothing: this suite ticks the state
/// machine by hand.
struct Silent;

#[async_trait::async_trait]
impl ReEnqueue for Silent {
    async fn reenqueue(&self, _dispatch_id: &DispatchId) -> Result<(), asterism_core::DomainError> {
        Ok(())
    }
}

/// Drives one dispatch to a terminal state through the real runner,
/// with the release wired in as the thing that stamps what leaves.
///
/// A second handle over the same database, the way every dispatch suite
/// here builds one — what is under test is the runner, and the runner
/// takes its ports as arguments.
async fn run_to_done(core: &CoreCtx, tmp: &std::path::Path, dispatch: &str) {
    let (isle, driver) = sqlite::open_and_migrate(&tmp.join("asterism.db"))
        .await
        .expect("a handle for the runner");
    let env = DispatchRunEnv {
        registry: ExporterRegistry::single(Arc::new(asterism_exporter_file::FileExporter::new())),
        service: core.support.dispatch_runner.clone(),
        snapshots: Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        dispatches: Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        assets: Arc::new(sqlite::repo::SqliteAssetRepository::new(isle)),
        reenqueue: Arc::new(Silent),
        // The wiring under test: the runner hands the copies over, and
        // the release decides what to do with them.
        outbound: Some(core.release_service.clone()),
    };
    let payload = serde_json::json!({ "dispatch_id": dispatch });
    for _ in 0..8 {
        asterism_infra::dispatch::run_dispatch_run(&env, &payload)
            .await
            .expect("a dispatch tick");
    }
    drop(driver);
}

/// Everything the acceptance criteria ask of one release, in the order
/// a release happens.
#[tokio::test]
async fn a_release_freezes_what_the_change_point_carried_and_stamps_what_leaves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (line, point, assets, held) = a_landed_line(&router, tmp.path(), 2).await;
    let out = tmp.path().join("outbound");

    let before = ok(&router, get(&format!("/asterism/forge/lines/{line}"))).await;

    let release = ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{line}/points/{point}/releases"),
            serde_json::json!({
                "persona_id": core_persona(&router).await,
                "output_dir": out.display().to_string(),
            }),
        ),
    )
    .await;

    // 1. It carries an act, names what it released, and left the line
    //    where it was.
    assert_eq!(release["change_point_id"], point);
    assert_eq!(release["line_id"], line);
    assert_eq!(release["actor_kind"], "user");
    assert!(release["at_ms"].as_i64().expect("an instant") > 0);
    let after = ok(&router, get(&format!("/asterism/forge/lines/{line}"))).await;
    assert_eq!(after, before, "a release is not a change point");

    // 2. The freeze holds exactly what the change point's live entries
    //    named, and the run over it is a copy.
    let snapshot = ok(
        &router,
        get(&format!(
            "/asterism/snapshots/{}",
            release["snapshot_id"].as_str().expect("a snapshot id")
        )),
    )
    .await;
    let frozen: Vec<String> = snapshot["asset_ids"]
        .as_array()
        .expect("members")
        .iter()
        .map(|id| id.as_str().expect("an id").to_string())
        .collect();
    assert_eq!(frozen.len(), assets.len());
    for asset in &assets {
        assert!(
            frozen.contains(asset),
            "the freeze holds {asset}: {frozen:?}"
        );
    }
    let dispatch_id = release["dispatch_id"].as_str().expect("a dispatch id");
    let dispatch = ok(&router, get(&format!("/asterism/dispatch/{dispatch_id}"))).await;
    assert_eq!(dispatch["exporter_slug"], "file");
    assert_eq!(dispatch["action"], "write");
    assert_eq!(dispatch["snapshot_id"], release["snapshot_id"]);
    let params: serde_json::Value =
        serde_json::from_str(dispatch["params_json"].as_str().expect("params")).expect("json");
    assert_eq!(params["mode"], "copy");
    assert_eq!(params["output_dir"], out.display().to_string());

    // The run has not happened, so nothing has been written yet — and
    // the release says that by holding no stamps rather than by
    // claiming any outcome.
    assert!(release["files"].as_array().expect("files").is_empty());

    run_to_done(&core, tmp.path(), dispatch_id).await;

    // 3. Every copy carries the packet, and the library's own file is
    //    exactly as it was.
    let written: Vec<std::path::PathBuf> = std::fs::read_dir(&out)
        .expect("the output directory")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(written.len(), assets.len(), "one copy per member");
    for copy in &written {
        let bytes = std::fs::read(copy).expect("read the copy");
        assert!(
            String::from_utf8_lossy(&bytes).contains("trainedAlgorithmicMedia"),
            "{} carries the disclosure packet",
            copy.display()
        );
    }
    for original in &held {
        assert_eq!(
            std::fs::read(original).expect("read the library's copy"),
            png(),
            "{} was not touched",
            original.display()
        );
    }

    // 4. With no certificate configured, the manifest half is reported
    //    skipped per file, and nothing else about it differs.
    let read_back = ok(
        &router,
        get(&format!(
            "/asterism/forge/releases/{}",
            release["id"].as_str().expect("a release id")
        )),
    )
    .await;
    let files = read_back["files"].as_array().expect("files");
    assert_eq!(files.len(), assets.len(), "a stamp per file");
    for file in files {
        assert_eq!(file["xmp"]["state"], "written");
        assert_eq!(file["manifest"]["state"], "skipped");
        assert_eq!(file["manifest"]["detail"], "no_signing_identity");
        assert!(
            file["path"]
                .as_str()
                .expect("a path")
                .starts_with(out.to_str().expect("a utf-8 temporary directory")),
            "the stamp names the copy, not the original: {file}"
        );
    }
}

/// A set going out twice is two records, and neither is the other.
#[tokio::test]
async fn two_releases_of_one_change_point_are_two_records() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let (line, point, _, _) = a_landed_line(&router, tmp.path(), 1).await;
    let persona = core_persona(&router).await;

    let first = ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{line}/points/{point}/releases"),
            serde_json::json!({
                "persona_id": persona,
                "output_dir": tmp.path().join("first").display().to_string(),
            }),
        ),
    )
    .await;
    let second = ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{line}/points/{point}/releases"),
            serde_json::json!({
                "persona_id": persona,
                "output_dir": tmp.path().join("second").display().to_string(),
            }),
        ),
    )
    .await;

    assert_ne!(first["id"], second["id"]);
    assert_ne!(
        first["dispatch_id"], second["dispatch_id"],
        "each release starts a run of its own"
    );
    assert_eq!(
        first["snapshot_id"], second["snapshot_id"],
        "the same members in the same order are one freeze, deduped on content"
    );

    let listed = ok(
        &router,
        get(&format!(
            "/asterism/forge/lines/{line}/points/{point}/releases"
        )),
    )
    .await;
    assert_eq!(listed.as_array().expect("a list").len(), 2);
}

/// A change point that took the last entry off has nothing to write
/// out, and the answer is a refusal rather than an empty export.
///
/// Two refusals live at this route and they are not the same answer. A
/// node this line's history does not have is a 404 — the line does not
/// have it, whoever else might. A node it does have that folds to
/// nothing live is a 409: the request is well formed and the state is
/// what is in the way, and putting something back on the line makes the
/// same request work.
#[tokio::test]
async fn releasing_a_change_point_that_carries_nothing_is_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let (line, landed, _, _) = a_landed_line(&router, tmp.path(), 1).await;
    let persona = core_persona(&router).await;

    // Take the only entry off, which lands a change point whose fold
    // has nothing alive in it.
    let states = ok(
        &router,
        get(&format!("/asterism/forge/lines/{line}/states")),
    )
    .await;
    let entry = states
        .as_array()
        .expect("a state per entry")
        .first()
        .expect("one entry")["entry_id"]
        .as_str()
        .expect("an entry id")
        .to_string();
    let work = ok(
        &router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({ "line_id": line, "title": "take it off" }),
        ),
    )
    .await;
    let work_id = work["id"].as_str().expect("a pursuit id").to_string();
    ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{work_id}/push"),
            serde_json::json!({ "ops": [{ "entry_id": entry, "kind": "remove" }] }),
        ),
    )
    .await;
    ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{work_id}/close"),
            serde_json::json!({ "outcome": "satisfied" }),
        ),
    )
    .await;
    let history = ok(&router, get(&format!("/asterism/forge/lines/{line}"))).await;
    let emptied = history["changes"]
        .as_array()
        .expect("a chain")
        .last()
        .expect("the removal")["id"]
        .as_str()
        .expect("a change point id")
        .to_string();
    assert_ne!(emptied, landed, "the removal is its own node");

    let (status, body) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{line}/points/{emptied}/releases"),
            serde_json::json!({
                "persona_id": persona,
                "output_dir": tmp.path().join("nothing").display().to_string(),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // The earlier change point still releases: what is folded is the
    // state *then*, and taking the entry off afterwards did not move
    // it.
    ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{line}/points/{landed}/releases"),
            serde_json::json!({
                "persona_id": persona,
                "output_dir": tmp.path().join("then").display().to_string(),
            }),
        ),
    )
    .await;

    // And a node this line's history does not have is the other answer.
    let (status, body) = call(
        &router,
        post(
            &format!(
                "/asterism/forge/lines/{line}/points/{}/releases",
                uuid::Uuid::now_v7()
            ),
            serde_json::json!({
                "persona_id": persona,
                "output_dir": tmp.path().join("elsewhere").display().to_string(),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// The one persona this suite registers.
async fn core_persona(router: &Router) -> String {
    let personas = ok(router, get("/asterism/personas")).await;
    personas
        .as_array()
        .expect("a list")
        .first()
        .expect("the registered persona")["id"]
        .as_str()
        .expect("a persona id")
        .to_string()
}
