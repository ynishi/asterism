//! Sending a release, end to end: the route, the transfer, the sidecar,
//! and what the far side is holding when the run reports done.
//!
//! The unit tests in `asterism-exporter-transfer` answer what the
//! adapter *decides* against a far side held in memory — put order,
//! remote names, the sidecar's cells, the two refusals, what the attempt
//! record keeps. What only a run can answer is whether the pieces meet:
//! whether the bytes that arrive are the release's stamped copies rather
//! than the library's own, whether the file list the send wrote is the
//! one the adapter read, and whether a run that harvests nothing parks
//! the row in `Done` with no output assets.
//!
//! # The far side is a directory
//!
//! The endpoint is `file://`, which is a destination the adapter carries
//! for exactly this reason — `asterism_exporter_transfer::transport`'s
//! `Scheme::File` is where that is argued. An SSH server stood up inside
//! this process would answer for that server: the same `Transport` the
//! protocols implement is what the bytes go through either way, and what
//! this suite is asking about is everything above it. The SFTP path is
//! exercised by `sftp_against_a_named_endpoint`, which is `#[ignore]`d
//! because it needs a host.

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

// ---- the harness, as the release suite beside this one has it ----

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

async fn ok(router: &Router, request: Request<Body>) -> serde_json::Value {
    let named = format!("{} {}", request.method(), request.uri());
    let (status, body) = call(router, request).await;
    assert_eq!(status, StatusCode::OK, "{named}: {body}");
    body
}

/// The 1×1 PNG that stands in for what a line carries — a real
/// container, because the stamp rewrites one and the send reads what
/// the stamp left.
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

/// What a generated file's container says about itself, so the
/// disclosure has something to build from.
fn comfy() -> String {
    serde_json::json!({ "Software": "ComfyUI", "workflow": "{}" }).to_string()
}

/// A persona and `count` fingerprinted assets whose files exist on
/// disk.
///
/// The fingerprint goes in through the repository for the reason the
/// release suite gives: it lands in the same statement as the digest it
/// belongs to, so the `material_hash` job stays the only writer of it.
async fn seed(
    router: &Router,
    tmp: &std::path::Path,
    count: usize,
) -> (Vec<String>, Vec<std::path::PathBuf>) {
    let persona = ok(
        router,
        post(
            "/asterism/personas/register",
            serde_json::to_value(RegisterPersonaCommand {
                name: "forge sends".into(),
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
        asterism_core::domain::repository::AssetRepository::set_material_fingerprint(
            &assets,
            &AssetId::from_uuid(uuid::Uuid::parse_str(&id).expect("a uuid")),
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
    drop(driver);
    (ids, paths)
}

/// A line carrying one entry per asset, landed by one satisfied close,
/// then released into `out`.
///
/// Answers the release body as the route recorded it, plus the library
/// paths so a caller can check they were left alone.
async fn a_release(
    core: &CoreCtx,
    router: &Router,
    tmp: &std::path::Path,
    count: usize,
    out: &std::path::Path,
) -> (serde_json::Value, Vec<std::path::PathBuf>) {
    let (assets, held) = seed(router, tmp, count).await;
    let line = ok(
        router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "send", "strategy_id": "mainline-first" }),
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

    let released = ok(
        router,
        post(
            &format!("/asterism/forge/lines/{line_id}/points/{point}/releases"),
            serde_json::json!({
                "persona_id": core_persona(router).await,
                "output_dir": out.display().to_string(),
            }),
        ),
    )
    .await;
    run_to_terminal(
        core,
        tmp,
        released["dispatch_id"].as_str().expect("a dispatch id"),
        Arc::new(asterism_exporter_file::FileExporter::new()),
        true,
    )
    .await;

    // Read it back, because what a send reads is the file rows the
    // stamping pass wrote and those do not exist at the moment the
    // release is recorded.
    let read_back = ok(
        router,
        get(&format!(
            "/asterism/forge/releases/{}",
            released["id"].as_str().expect("a release id")
        )),
    )
    .await;
    (read_back, held)
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

/// Drives one dispatch to a terminal state through the real runner.
///
/// `stamping` is what decides whether the release's outbound pass is
/// wired in. A send's run has one too and it does nothing — the
/// dispatch is not a release, so the pass returns at its first
/// question — and leaving it on is how this suite says so rather than
/// assuming it.
async fn run_to_terminal(
    core: &CoreCtx,
    tmp: &std::path::Path,
    dispatch: &str,
    exporter: Arc<dyn asterism_dispatch_sdk::Exporter>,
    stamping: bool,
) {
    let (isle, driver) = sqlite::open_and_migrate(&tmp.join("asterism.db"))
        .await
        .expect("a handle for the runner");
    let env = DispatchRunEnv {
        registry: ExporterRegistry::single(exporter),
        service: core.support.dispatch_runner.clone(),
        snapshots: Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        dispatches: Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        assets: Arc::new(sqlite::repo::SqliteAssetRepository::new(isle)),
        reenqueue: Arc::new(Silent),
        outbound: stamping.then(|| {
            core.support.release_stamping.clone()
                as Arc<dyn asterism_core::application_support::OutboundStamping>
        }),
    };
    let payload = serde_json::json!({ "dispatch_id": dispatch });
    for _ in 0..8 {
        asterism_infra::dispatch::run_dispatch_run(&env, &payload)
            .await
            .expect("a dispatch tick");
    }
    drop(driver);
}

/// A profile for a directory on this machine, with the sidecar columns
/// an agency would declare.
fn profile(to: &std::path::Path) -> String {
    serde_json::json!({
        "endpoint": format!("file://{}", to.display()),
        "sidecar": {
            "filename": "metadata.csv",
            "columns": [
                { "header": "Filename", "template": "{{item.remote_name}}" },
                { "header": "Asset", "template": "{{item.asset_id}}" },
                { "header": "Modality", "template": "{{item.card.modality?}}" },
                { "header": "Generated", "template": "{{params.extras.ai_keyword}}" }
            ]
        },
        "extras": { "ai_keyword": "_ai_generated" }
    })
    .to_string()
}

/// Everything the acceptance criteria ask of one send, in the order a
/// send happens.
#[tokio::test]
async fn a_send_puts_the_stamped_set_and_its_sidecar_on_the_far_side() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");
    let (release, held) = a_release(&core, &router, tmp.path(), 2, &out).await;
    let release_id = release["id"].as_str().expect("a release id").to_string();
    let far = tmp.path().join("agency").join("incoming");

    // 1. A send is recorded, carries an act, and names both the
    //    destination label and the run that carries the bytes.
    let send = ok(
        &router,
        post(
            &format!("/asterism/forge/releases/{release_id}/sends"),
            serde_json::json!({
                "destination": "adobe-stock",
                "profile_json": profile(&far),
            }),
        ),
    )
    .await;
    assert_eq!(send["release_id"], release_id.as_str());
    assert_eq!(send["destination"], "adobe-stock");
    assert_eq!(send["actor_kind"], "user");
    assert!(send["at_ms"].as_i64().expect("an instant") > 0);
    let dispatch_id = send["dispatch_id"].as_str().expect("a dispatch id");

    run_to_terminal(
        &core,
        tmp.path(),
        dispatch_id,
        Arc::new(asterism_exporter_transfer::TransferExporter::new()),
        true,
    )
    .await;

    // 2. Exactly the release's files landed, byte for byte, in the
    //    directory the profile named.
    let stamped: Vec<std::path::PathBuf> = release["files"]
        .as_array()
        .expect("the release wrote files")
        .iter()
        .map(|file| std::path::PathBuf::from(file["path"].as_str().expect("a path")))
        .collect();
    assert_eq!(stamped.len(), 2, "one stamped copy per member");
    for copy in &stamped {
        let there = far.join(copy.file_name().expect("a basename"));
        assert_eq!(
            std::fs::read(&there).unwrap_or_else(|e| panic!("{}: {e}", there.display())),
            std::fs::read(copy).expect("the stamped copy"),
            "{} is the copy the release stamped, byte for byte",
            there.display()
        );
    }
    // And the library's own files were never in it.
    for original in &held {
        assert_eq!(
            std::fs::read(original).expect("read the library's copy"),
            png(),
            "{} was not touched",
            original.display()
        );
        assert!(
            !far.join(original.file_name().expect("a basename")).exists()
                || std::fs::read(far.join(original.file_name().unwrap())).unwrap() != png(),
            "an unstamped original did not travel"
        );
    }

    // The attempt record names each file and what the far side said.
    let dispatch = ok(&router, get(&format!("/asterism/dispatch/{dispatch_id}"))).await;
    let attempt: serde_json::Value = serde_json::from_str(
        dispatch["attempt_json"]
            .as_str()
            .expect("the exporter recorded the call"),
    )
    .expect("json");
    let recorded = attempt["files"].as_array().expect("a row per file");
    assert_eq!(recorded.len(), 2);
    for row in recorded {
        assert_eq!(row["outcome"], "sent");
        assert!(row["bytes"].as_u64().expect("a byte count") > 0);
        assert!(row["answer"].is_null());
    }
    assert_eq!(attempt["sidecar"]["outcome"], "sent");
    assert_eq!(attempt["sidecar"]["rows"], 2);

    // 3. The sidecar has one row per file in send order, under the
    //    headers the profile declared, rendered from the input cards.
    let csv = std::fs::read_to_string(far.join("metadata.csv")).expect("the sidecar");
    let rows: Vec<&str> = csv.trim_end_matches("\r\n").split("\r\n").collect();
    assert_eq!(rows.len(), 3, "a header and a row per file: {csv:?}");
    assert_eq!(rows[0], "Filename,Asset,Modality,Generated");
    for (nth, row) in rows[1..].iter().enumerate() {
        let cells: Vec<&str> = row.split(',').collect();
        assert_eq!(
            cells[0],
            stamped[nth]
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a basename"),
            "send order is the release's own order"
        );
        assert_eq!(cells[1], release["files"][nth]["asset_id"]);
        assert_eq!(cells[2], "image", "rendered from the input's card");
        assert_eq!(cells[3], "_ai_generated");
    }

    // 5. Nothing was made: the run is done with no output assets, and
    //    no new asset row appeared for what left.
    assert_eq!(dispatch["state"], "done", "{dispatch}");
    assert!(
        dispatch["output_asset_ids"]
            .as_array()
            .expect("a list")
            .is_empty(),
        "harvest yields nothing, so reify mints nothing: {dispatch}"
    );
}

/// A re-submission after a rejection is two records, and a release
/// lists them.
#[tokio::test]
async fn two_sends_of_one_release_are_two_records() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");
    let (release, _) = a_release(&core, &router, tmp.path(), 1, &out).await;
    let release_id = release["id"].as_str().expect("a release id").to_string();

    let first = ok(
        &router,
        post(
            &format!("/asterism/forge/releases/{release_id}/sends"),
            serde_json::json!({
                "destination": "adobe-stock",
                "profile_json": profile(&tmp.path().join("first")),
            }),
        ),
    )
    .await;
    let second = ok(
        &router,
        post(
            &format!("/asterism/forge/releases/{release_id}/sends"),
            serde_json::json!({
                "destination": "dreamstime",
                "profile_json": profile(&tmp.path().join("second")),
            }),
        ),
    )
    .await;

    assert_ne!(first["id"], second["id"]);
    assert_ne!(
        first["dispatch_id"], second["dispatch_id"],
        "each send starts a run of its own"
    );

    let listed = ok(
        &router,
        get(&format!("/asterism/forge/releases/{release_id}/sends")),
    )
    .await;
    let listed = listed.as_array().expect("a list");
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[0]["destination"], "dreamstime",
        "most recent first: {listed:?}"
    );
}

/// The default answer to a credential over cleartext is no, and the
/// dispatch row is where a reader finds out.
#[tokio::test]
async fn an_ftp_endpoint_without_the_opt_in_fails_the_run_and_says_so_on_the_row() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");
    let (release, _) = a_release(&core, &router, tmp.path(), 1, &out).await;

    let mut insecure: serde_json::Value =
        serde_json::from_str(&profile(&tmp.path().join("unused"))).expect("json");
    insecure["endpoint"] = serde_json::json!("ftp://files.example.invalid/incoming");
    let send = ok(
        &router,
        post(
            &format!(
                "/asterism/forge/releases/{}/sends",
                release["id"].as_str().expect("a release id")
            ),
            serde_json::json!({
                "destination": "somewhere in the clear",
                "profile_json": insecure.to_string(),
            }),
        ),
    )
    .await;
    let dispatch_id = send["dispatch_id"].as_str().expect("a dispatch id");

    run_to_terminal(
        &core,
        tmp.path(),
        dispatch_id,
        Arc::new(asterism_exporter_transfer::TransferExporter::new()),
        true,
    )
    .await;

    let dispatch = ok(&router, get(&format!("/asterism/dispatch/{dispatch_id}"))).await;
    assert_eq!(dispatch["state"], "failed", "{dispatch}");
    let attempt: serde_json::Value = serde_json::from_str(
        dispatch["attempt_json"]
            .as_str()
            .expect("a refusal is recorded like any other call"),
    )
    .expect("json");
    assert!(
        attempt["refused"]
            .as_str()
            .expect("the reason")
            .contains("allow_insecure"),
        "{attempt}"
    );
    // The send is still a record: a failed send is what happened, and a
    // second send is a second record.
    let listed = ok(
        &router,
        get(&format!(
            "/asterism/forge/releases/{}/sends",
            release["id"].as_str().expect("a release id")
        )),
    )
    .await;
    assert_eq!(listed.as_array().expect("a list").len(), 1);
}

/// A release whose run has not written its files has nothing to send.
///
/// Reachable rather than hypothetical: the release route answers as soon
/// as the dispatch is started, so a caller holding a release id is
/// holding one whose files are on the way.
#[tokio::test]
async fn a_release_that_has_written_nothing_cannot_be_sent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");

    let (assets, _) = seed(&router, tmp.path(), 1).await;
    let line = ok(
        &router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "unsent", "strategy_id": "mainline-first" }),
        ),
    )
    .await;
    let line_id = line["id"].as_str().expect("a line id").to_string();
    let work = ok(
        &router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({ "line_id": line_id, "title": "not yet" }),
        ),
    )
    .await;
    let work_id = work["id"].as_str().expect("a pursuit id").to_string();
    ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{work_id}/push"),
            serde_json::json!({ "ops": [{
                "entry_id": uuid::Uuid::now_v7().to_string(),
                "kind": "add",
                "content_asset_id": assets[0],
                "name": "cut-0",
            }] }),
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
    let history = ok(&router, get(&format!("/asterism/forge/lines/{line_id}"))).await;
    let point = history["changes"]
        .as_array()
        .expect("a chain")
        .last()
        .expect("a change point")["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let unrun = ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{line_id}/points/{point}/releases"),
            serde_json::json!({
                "persona_id": core_persona(&router).await,
                "output_dir": out.display().to_string(),
            }),
        ),
    )
    .await;

    let (status, body) = call(
        &router,
        post(
            &format!(
                "/asterism/forge/releases/{}/sends",
                unrun["id"].as_str().expect("a release id")
            ),
            serde_json::json!({
                "destination": "too soon",
                "profile_json": profile(&tmp.path().join("nothing")),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

/// The reserved key is the send's to write, and a profile that set it
/// would decide which bytes leave.
#[tokio::test]
async fn a_profile_that_writes_the_sends_own_key_is_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");
    let (released, _) = a_release(&core, &router, tmp.path(), 1, &out).await;

    let mut overreaching: serde_json::Value =
        serde_json::from_str(&profile(&tmp.path().join("mine"))).expect("json");
    overreaching["release"] = serde_json::json!({ "files": [] });
    let (status, body) = call(
        &router,
        post(
            &format!(
                "/asterism/forge/releases/{}/sends",
                released["id"].as_str().expect("a release id")
            ),
            serde_json::json!({
                "destination": "mine",
                "profile_json": overreaching.to_string(),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// The SFTP half, against a host somebody named.
///
/// `#[ignore]`d because it needs one: set `ASTERISM_TEST_SFTP_ENDPOINT`
/// to an `sftp://host[:port]/dir`, `ASTERISM_TEST_SFTP_USER`,
/// `ASTERISM_TEST_SFTP_FINGERPRINT` to the host key's SHA-256
/// fingerprint as OpenSSH spells one, and either
/// `ASTERISM_TEST_SFTP_PASSWORD` or `ASTERISM_TEST_SFTP_KEY` (a path).
/// Run it with `--ignored`.
///
/// What it adds over the suite above is the protocol: the same send,
/// through the same `Transport`, with `russh` on the other side of it.
/// A send hangs off a release and a release goes when its line is
/// dropped, so a line one of whose releases has been sent still drops.
///
/// `forge_send.release_id` is `RESTRICT` and SQLite checks that at the
/// statement, so a `discard` that deleted the releases without taking
/// the sends first would be refused by the database rather than leaving
/// a stray row — the line would become undroppable, pinned by a record
/// of where its files went. `Lines::discard` is where what a drop takes
/// is decided, and this is that sentence held to the adapter.
#[tokio::test]
async fn a_line_whose_release_was_sent_can_still_be_dropped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");
    let (release, _) = a_release(&core, &router, tmp.path(), 1, &out).await;
    let release_id = release["id"].as_str().expect("a release id").to_string();
    let line_id = release["line_id"].as_str().expect("a line id").to_string();

    let send = ok(
        &router,
        post(
            &format!("/asterism/forge/releases/{release_id}/sends"),
            serde_json::json!({
                "destination": "adobe-stock",
                "profile_json": profile(&tmp.path().join("agency")),
            }),
        ),
    )
    .await;
    let send_id = send["id"].as_str().expect("a send id").to_string();

    ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{line_id}/archive"),
            serde_json::json!({}),
        ),
    )
    .await;
    let (status, dropped) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{line_id}/discard"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a sent release must not pin its line: {dropped}"
    );

    // The send went with the release, and the release with the line.
    let (status, _) = call(
        &router,
        get(&format!("/asterism/forge/releases/{release_id}/sends")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the release went");
    let (status, _) = call(&router, get(&format!("/asterism/forge/lines/{line_id}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the line went");
    assert!(!send_id.is_empty());
}

#[tokio::test]
#[ignore = "needs an SFTP host named by ASTERISM_TEST_SFTP_ENDPOINT"]
async fn sftp_against_a_named_endpoint() {
    let Ok(endpoint) = std::env::var("ASTERISM_TEST_SFTP_ENDPOINT") else {
        panic!("ASTERISM_TEST_SFTP_ENDPOINT names the host this test sends to");
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let out = tmp.path().join("outbound");
    let (release, _) = a_release(&core, &router, tmp.path(), 1, &out).await;

    let mut over_ssh: serde_json::Value =
        serde_json::from_str(&profile(&tmp.path().join("unused"))).expect("json");
    over_ssh["endpoint"] = serde_json::json!(endpoint);
    over_ssh["host_key"] = serde_json::json!({
        "fingerprint": std::env::var("ASTERISM_TEST_SFTP_FINGERPRINT")
            .expect("ASTERISM_TEST_SFTP_FINGERPRINT"),
    });
    over_ssh["auth"] = serde_json::json!({
        "user": std::env::var("ASTERISM_TEST_SFTP_USER").expect("ASTERISM_TEST_SFTP_USER"),
        "secret_ref": "ASTERISM_TEST_SFTP_PASSWORD",
        "key_ref": std::env::var("ASTERISM_TEST_SFTP_KEY")
            .ok()
            .map(|_| "ASTERISM_TEST_SFTP_KEY"),
    });

    let send = ok(
        &router,
        post(
            &format!(
                "/asterism/forge/releases/{}/sends",
                release["id"].as_str().expect("a release id")
            ),
            serde_json::json!({
                "destination": "the named endpoint",
                "profile_json": over_ssh.to_string(),
            }),
        ),
    )
    .await;
    let dispatch_id = send["dispatch_id"].as_str().expect("a dispatch id");
    run_to_terminal(
        &core,
        tmp.path(),
        dispatch_id,
        Arc::new(asterism_exporter_transfer::TransferExporter::new()),
        true,
    )
    .await;

    let dispatch = ok(&router, get(&format!("/asterism/dispatch/{dispatch_id}"))).await;
    assert_eq!(dispatch["state"], "done", "{dispatch}");
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
