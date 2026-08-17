//! Two remote-backed exporters, driven end to end against a backend
//! that only exists for the length of the test.
//!
//! The sibling round-trip e2e (`dispatch_rein_roundtrip_e2e.rs`) runs
//! the `file` exporter, which does its whole job inside one synchronous
//! call: there is no handle to persist, nothing to poll, and no
//! response shape to misread. Every exporter that talks to a generator
//! is the opposite — the interesting part is the wait. `POST` a job,
//! get an opaque id back, ask about it until the backend says it is
//! finished, then read what it made out of a second response. Each of
//! those hops is a place where a field name, a locator, or a state
//! transition can quietly drift, and none of them were exercised
//! anywhere in this crate before this binary existed.
//!
//! So the backend is faked, and nothing else is. The fake is an axum
//! `Router` on a loopback port speaking the two protocols verbatim
//! (ComfyUI's `POST /prompt` + `GET /history/{id}`, and a generic
//! submit / status / result trio for the schema-driven exporter); the
//! real `ComfyHttpExporter` and `HttpExporter` reach it through a real
//! `reqwest::Client`, and the real `DispatchRun` state machine drives
//! them. What the test owns is the backend's *script* — how many times
//! it says "not yet", and whether it ends in outputs or an error — and
//! that script is what makes the tick count, the re-enqueue log, and
//! the reified assets facts rather than guesses.
//!
//! **Why `CoreMode::ReadOnly`.** Stronger here than in the sibling
//! file. `ReadOnly` opens the job queue without spawning a worker
//! `Monitor`, so the `DispatchRun` job that `DispatchService::create`
//! enqueues sits there and the test is the only thing advancing the
//! state machine. Under `Full` a worker would pick the same dispatch up
//! and **POST to the fake a second time** — the tick count, the
//! re-enqueue log, and the backend's own request log would all stop
//! meaning anything.
//!
//! **What the fake writes.** On submit, when it has been given an
//! output root, it writes the files its own success fixture claims it
//! produced. Reify never stats a locator (`dispatch_runner_service.rs`
//! builds the `Material` from the string alone), so this is not needed
//! to make the assertions pass — it is here because "ComfyUI put files
//! in its output dir" is the situation being modelled, and a locator
//! pointing at nothing would be a different one.
//!
//! Its own test binary because `init_core` opens a Tantivy index (one
//! core per test binary, as with the sibling e2e files).

use std::path::Path;
use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, RegisterPersonaCommand,
};
use asterism_contract::query::{GetAssetDetailQuery, ListAssetsQuery};
use asterism_core::domain::value::DispatchId;
use asterism_core::error::DomainError;
use asterism_dispatch_sdk::Exporter;
use asterism_exporter_comfy::ComfyHttpExporter;
use asterism_exporter_http::HttpExporter;
use asterism_infra::dispatch::{DispatchRunEnv, ExporterRegistry, ReEnqueue, run_dispatch_run};
use asterism_infra::sqlite;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use serde_json::json;

use fake_backend::{FakeBackend, Outcome};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the exporter
/// round trip, not about who asked for it.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// The prompt id the fake ComfyUI hands back. A literal rather than a
/// generated uuid so the poll message and the request log can be
/// asserted verbatim.
const COMFY_JOB_ID: &str = "comfy-e2e-prompt-1";
/// Same idea for the schema-driven backend's job id.
const HTTP_JOB_ID: &str = "http-e2e-job-1";

/// Poll cadence echoed into Comfy's progress message. An odd value so
/// the assertion proves the params reached the exporter rather than
/// matching the crate's 2000 ms default by accident.
const POLL_INTERVAL_MS: u64 = 1234;

/// A 1×1 RGBA PNG, 67 bytes. Nothing on this route decodes pixels —
/// the assets here are minted from locator strings, and the thumbnail /
/// cover jobs do not run in `ReadOnly` — so a minimal header-valid
/// file is the whole fixture.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR len + type
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, // bitdepth/colour + CRC
    0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, // IDAT len + type
    0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, //
    0x0D, 0x0A, 0x2D, 0xB4, // IDAT CRC
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND
    0xAE, 0x42, 0x60, 0x82,
];

/// [`ReEnqueue`] fake that records instead of queueing — the shape
/// `runtime.rs` names in its own doc ("or a test fake in unit tests").
/// In production this hands the next tick back to apalis; here the
/// test is the poll loop, so the useful thing is the log of what the
/// runner asked for.
#[derive(Default)]
struct RecordingReEnqueue {
    /// Dispatch ids the runner asked to see again, in order. A `std`
    /// mutex because the guard never crosses an `await`.
    seen: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ReEnqueue for RecordingReEnqueue {
    async fn reenqueue(&self, dispatch_id: &DispatchId) -> Result<(), DomainError> {
        self.seen
            .lock()
            .expect("re-enqueue log")
            .push(dispatch_id.to_string());
        Ok(())
    }
}

/// The state machine's visible trace: `(state, state_message)` read
/// back off the dispatch row after every tick. The message half is
/// where an exporter's progress hint surfaces, so this is the one
/// artefact that shows the *waiting* rather than just the outcome.
type TickLog = Vec<(String, Option<String>)>;

/// What one export left behind: the ids the ledger now holds, plus
/// what the state machine did on the way there.
struct ExportRun {
    /// The dispatch that ran.
    dispatch_id: String,
    /// The snapshot frozen for it (`extra._dispatch.selection_id`
    /// keeps this id under its historical wire name).
    snapshot_id: String,
    /// Assets reified from the harvest, in harvest order.
    output_ids: Vec<String>,
    /// State + message after each tick, until terminal.
    ticks: TickLog,
    /// Dispatch ids the runner handed back for another tick.
    reenqueued: Vec<String>,
}

/// The facts a reified asset is judged on, read in one query.
struct AssetFacts {
    /// `dispatch-<exporter slug>`, minted by `SourceKind::for_dispatch`.
    source_kind: String,
    /// Where the exporter said the artefact lives.
    locator: String,
    /// Primary material's format fact — guessed from the locator, and
    /// only from the locator.
    mime: Option<String>,
    /// Semantic classification declared by the exporter.
    modality: Option<String>,
    /// Card cover text (from the harvest map's `cover_hint`).
    cover: Option<String>,
    /// Label chips, exporter slug first.
    labels: Vec<String>,
    /// Grid clustering key — the dispatch id for anything reified.
    bundle_id: Option<String>,
    /// Exporter payload merged with the `_dispatch` trace.
    extra: serde_json::Value,
}

/// Boots a core on a fresh database. No router: this binary never
/// speaks HTTP to Asterism itself — the only server involved is the
/// fake backend on the other side of the exporters.
///
/// The returned `CoreCtx` must outlive the test body; dropping it
/// shuts the service graph down.
async fn boot(tmp: &Path) -> CoreCtx {
    init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core")
}

/// A reqwest client fit for talking to a loopback fixture: no proxy
/// (an `http_proxy` in the environment would otherwise swallow
/// `127.0.0.1` requests) and a short timeout so a fake that never
/// answers fails as a test rather than a five-minute hang. Both
/// exporters document `with_client` as the seam for exactly this.
fn backend_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("test client")
}

/// The `AddAssetCommand` an ordinary filesystem ingest would build.
/// Copied from `lineage_chain_e2e.rs` rather than shared: these e2e
/// binaries are each meant to read as one story, and a `tests/common`
/// module would put the fixture a file away from the assertions that
/// depend on it.
fn add_command(
    persona_id: &str,
    locator: &str,
    occurred_at_ms: i64,
    derived_from: Option<String>,
) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: None,
        occurred_at_ms,
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
        derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

/// Assembles the runner's dependency bundle — the one thing the
/// composition root keeps to itself. `CoreCtx` exposes the services
/// but not the repositories behind them, so the read-side ports come
/// from a second isle opened on the same database file (WAL,
/// `busy_timeout`, `BEGIN IMMEDIATE`; every step here is awaited in
/// sequence, so a read never races a write it depends on).
///
/// The exporter is a parameter rather than a constant: this binary
/// runs two of them, and registering exactly one means a slug typo
/// fails as "exporter not registered" instead of reaching a backend
/// nobody meant to call.
async fn dispatch_env(
    db_path: &Path,
    core: &CoreCtx,
    exporter: Arc<dyn Exporter>,
) -> (DispatchRunEnv, Arc<RecordingReEnqueue>) {
    let (isle, driver) = sqlite::open_and_migrate(db_path)
        .await
        .expect("second isle");
    // Dropping the driver joins the SQLite thread and takes the isle
    // with it, so the handle has to outlive every call made through
    // this environment. `core_init` holds its own driver the same way
    // (graceful shutdown is a future addition). The leak is bounded:
    // one driver per test, five in this binary, all reclaimed when the
    // test process exits.
    std::mem::forget(driver);

    let reenqueue = Arc::new(RecordingReEnqueue::default());
    let env = DispatchRunEnv {
        registry: ExporterRegistry::single(exporter),
        service: core.support.dispatch_runner.clone(),
        snapshots: Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        dispatches: Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        assets: Arc::new(sqlite::repo::SqliteAssetRepository::new(isle)),
        reenqueue: reenqueue.clone(),
    };
    (env, reenqueue)
}

/// Ticks the state machine until the dispatch is terminal, returning
/// what the row said after each tick.
///
/// The eight-tick ceiling is a stuck-loop guard, not an expectation:
/// every script in this binary lands in two or three, and the log
/// comes back in the panic message if one does not.
async fn drive_to_terminal(env: &DispatchRunEnv, core: &CoreCtx, dispatch_id: &str) -> TickLog {
    let payload = json!({ "dispatch_id": dispatch_id });
    let mut ticks: TickLog = Vec::new();
    for _ in 0..8usize {
        run_dispatch_run(env, &payload)
            .await
            .expect("dispatch tick");
        let dto = core
            .dispatch_service
            .get(dispatch_id)
            .await
            .expect("dispatch get");
        ticks.push((dto.state.clone(), dto.state_message.clone()));
        if matches!(dto.state.as_str(), "done" | "failed" | "cancelled") {
            return ticks;
        }
    }
    panic!("dispatch did not reach a terminal state in 8 ticks: {ticks:?}");
}

/// Everything an assertion needs about one reified asset, in one
/// round trip.
async fn detail_of(core: &CoreCtx, asset_id: &str) -> AssetFacts {
    let detail = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: asset_id.to_string(),
            viewer_subject: None,
        })
        .await
        .expect("asset detail");
    let asset = detail.asset;
    AssetFacts {
        source_kind: asset.source_kind,
        locator: asset.locator,
        mime: asset.mime,
        modality: asset.modality,
        cover: asset.cover,
        labels: asset.labels,
        bundle_id: asset.bundle_id,
        extra: match asset.extra_json {
            Some(raw) => serde_json::from_str(&raw).expect("extra_json is JSON"),
            None => serde_json::Value::Null,
        },
    }
}

/// Freezes one asset, creates a dispatch for `exporter`, and ticks the
/// runner until it stops — the outbound half of the feature with a
/// real exporter behind it.
async fn export_via(
    core: &CoreCtx,
    db_path: &Path,
    persona_id: &str,
    input_asset_id: &str,
    exporter: Arc<dyn Exporter>,
    action: &str,
    params: serde_json::Value,
) -> ExportRun {
    let slug = exporter.slug().to_string();
    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona_id.to_string(),
                asset_ids: vec![input_asset_id.to_string()],
            },
            &unattributed(),
        )
        .await
        .expect("freeze snapshot");
    let dispatch = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: snapshot.id.clone(),
                exporter_slug: slug,
                action: action.to_string(),
                params_json: params.to_string(),
                operator_ai: None,
                pursuit_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    // `create` enqueued a `DispatchRun` job that nothing will ever
    // pick up (no worker in `ReadOnly`); this environment is the only
    // thing that moves the dispatch.
    let (env, reenqueue) = dispatch_env(db_path, core, exporter).await;
    let ticks = drive_to_terminal(&env, core, &dispatch.id).await;
    let reenqueued = reenqueue.seen.lock().expect("re-enqueue log").clone();

    let done = core
        .dispatch_service
        .get(&dispatch.id)
        .await
        .expect("dispatch get");
    ExportRun {
        dispatch_id: dispatch.id,
        snapshot_id: snapshot.id,
        output_ids: done.output_asset_ids,
        ticks,
        reenqueued,
    }
}

/// The Comfy params both comfy happy-path tests send. `output_dir` is
/// the only difference between them, and it is the only thing that
/// decides whether the harvest records a filesystem path or a `/view`
/// URL.
///
/// The workflow is the shape `schema/comfy_params.example.json`
/// documents — three nodes, of which the exporter is only allowed to
/// touch one. Everything else riding through untouched is half of
/// what the submit-body assertion checks.
fn comfy_params(port: u16, output_dir: Option<&Path>) -> serde_json::Value {
    let mut params = json!({
        "endpoint": format!("http://127.0.0.1:{port}"),
        "workflow": {
            "3": {
                "class_type": "KSampler",
                "inputs": {
                    "seed": 12345,
                    "steps": 30,
                    "cfg": 6.5,
                    "sampler_name": "dpmpp_2m",
                    "scheduler": "karras",
                    "denoise": 0.6
                }
            },
            "10": {
                "class_type": "LoadImage",
                "inputs": { "image": "<substituted with the input asset locator>" }
            },
            "9": {
                "class_type": "SaveImage",
                "inputs": { "filename_prefix": "asterism" }
            }
        },
        "input_slot": "10",
        "poll_interval_ms": POLL_INTERVAL_MS,
    });
    if let Some(dir) = output_dir {
        params["output_dir"] = json!(dir.display().to_string());
    }
    params
}

/// The progress line the comfy exporter writes while it waits — the
/// prompt id it is waiting on and the caller's own poll cadence.
fn comfy_waiting_message() -> String {
    format!("waiting for comfy prompt {COMFY_JOB_ID}; next poll in {POLL_INTERVAL_MS} ms")
}

/// The `outputs` block of a finished Comfy history entry.
///
/// Two things are deliberate. Node `"12"` produces text and no images
/// — the harvest loop skips any node without an `images` array, and a
/// fixture where every node has one would never exercise that. Node
/// `"9"`'s two images differ in `subfolder` (empty / `batch`), which
/// is the branch that decides whether the locator gains a directory
/// segment.
///
/// This fixture does **not** rest on the walk's order over the map,
/// and the sentence that used to stand here said it did — that
/// `serde_json`'s map is a `BTreeMap` "(no `preserve_order`)", so the
/// walk would reach `"12"` before `"9"` however this literal was
/// written. The workspace declares `preserve_order` (see the workspace
/// `Cargo.toml`), so the map is an `IndexMap` and the walk follows the
/// order written here. The conclusion survived only because `"12"`
/// happens to be written first and sorts first as well, which is two
/// coincidences rather than a reason.
///
/// What the assertions actually depend on: only node `"9"` carries an
/// `images` array, so exactly one node contributes outputs whatever
/// order the walk takes, and the ordering that is asserted is inside
/// that array — a real JSON array, which keeps its order under either
/// map type.
fn comfy_success_outputs() -> serde_json::Value {
    json!({
        "12": { "text": ["a node that produced no images"] },
        "9": {
            "images": [
                { "filename": "asterism_00001_.png", "subfolder": "", "type": "output" },
                { "filename": "asterism_00002_.png", "subfolder": "batch", "type": "output" }
            ]
        }
    })
}

/// The schema-driven params: one backend protocol described entirely
/// in JSON.
///
/// Written from the `HttpDispatchParams` grammar in
/// `asterism-exporter-http/src/lib.rs` rather than copied from
/// `schema/http_params.example.json`: this fixture has to point at the
/// per-test backend port and exercise the specific template roots the
/// assertions name, so it stays independent of the example (whose own
/// grammar conformance is pinned by round-trip tests in the exporter
/// crate).
///
/// `extras` is not a field of `HttpDispatchParams`; it survives
/// because serde ignores unknown keys, and `{{params.extras.prompt}}`
/// reads it straight out of the raw params JSON. That is the
/// documented way a caller carries its own values into a template.
///
/// It deliberately keeps the pre-merge spellings — `dispatch` for the
/// submit block, `locator` for the item's URL — because a stored profile
/// is re-read on every re-dispatch, so the aliases that let one keep
/// working are load-bearing. Written the new way, this test would pass
/// while every profile already on a row broke.
fn http_params(port: u16) -> serde_json::Value {
    json!({
        "endpoint": format!("http://127.0.0.1:{port}"),
        "dispatch": {
            "method": "POST",
            "path": "/generate",
            "body_template": {
                "image": "{{input[0].source_locator}}",
                "prompt": "{{params.extras.prompt}}",
                "client_id": "{{dispatch_id}}"
            },
            "handle_from": "$.job_id"
        },
        "poll": {
            "method": "GET",
            "path": "/status/{{handle}}",
            "done_when": { "path": "$.status", "equals": "done" },
            "failed_when": {
                "path": "$.status",
                "equals": "failed",
                "message_path": "$.error"
            },
            "progress_message_path": "$.status_message"
        },
        "harvest": {
            "method": "GET",
            "path": "/result/{{handle}}",
            "items_path": "$.outputs[*]",
            "map": {
                "modality": "image",
                "locator": "{{item.url}}",
                "cover_hint": "{{item.caption?}}",
                "labels_static": ["batch:{{dispatch_id}}"]
            }
        },
        "extras": { "prompt": "a test plate" }
    })
}

/// What the schema-driven backend serves from `/result`. The second
/// item has no `caption`, which is what makes the optional-placeholder
/// branch (`{{item.caption?}}` → empty → no cover) observable.
fn http_result_items() -> serde_json::Value {
    json!([
        { "url": "https://renders.test/first.png", "caption": "first plate" },
        { "url": "https://renders.test/second.png" }
    ])
}

/// A generator that never existed.
///
/// One `Router`, five routes, two protocols: ComfyUI's prompt queue
/// (`POST /prompt`, `GET /history/{id}`) and the generic submit /
/// status / result trio the schema-driven exporter is pointed at. A
/// test picks one protocol and the other three routes stay silent.
///
/// There is no `/view` route. The `/view` URL Comfy hands out when no
/// `output_dir` is configured is a *locator string* — the ledger
/// records it, and nothing in this process fetches it (thumbnail and
/// Re-In work do not run in `ReadOnly`). A route nobody calls would
/// only suggest otherwise.
mod fake_backend {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Json;
    use axum::Router;
    use axum::extract::{Path as UrlPath, State};
    use axum::routing::{get, post};
    use serde_json::{Value, json};

    /// How the scripted job ends.
    pub enum Outcome {
        /// The job succeeded. `outputs` is read per protocol: for
        /// comfy it is the `outputs` object of the history entry
        /// (keyed by node id); for the schema-driven backend it is the
        /// array `/result` serves under `outputs`. Only one protocol
        /// runs per test, so one field carries both without ambiguity.
        Finished { outputs: Value },
        /// The backend gave up, and said this.
        Failed { message: String },
    }

    /// The scripted backend plus everything it observed.
    pub struct FakeBackend {
        /// The opaque id handed back on submit and echoed in every
        /// subsequent path.
        job_id: &'static str,
        /// How many status reads answer "not yet" before the outcome
        /// is revealed.
        pending_reads: usize,
        /// How the job ends.
        outcome: Outcome,
        /// When set, submit writes the files the success fixture
        /// claims were produced, under this root.
        output_root: Option<PathBuf>,
        /// Every request, in order, as `"<METHOD> <path>"`.
        log: Mutex<Vec<String>>,
        /// Full bodies of every submit.
        submissions: Mutex<Vec<Value>>,
        /// Reads of the status-shaped route — comfy's
        /// `GET /history/{id}` (which serves *both* poll and harvest)
        /// and the schema-driven `GET /status/{id}`. One counter
        /// because one protocol runs per test.
        history_reads: AtomicUsize,
    }

    impl FakeBackend {
        /// Builds a backend from its script.
        pub fn new(
            job_id: &'static str,
            pending_reads: usize,
            outcome: Outcome,
            output_root: Option<PathBuf>,
        ) -> Self {
            Self {
                job_id,
                pending_reads,
                outcome,
                output_root,
                log: Mutex::new(Vec::new()),
                submissions: Mutex::new(Vec::new()),
                history_reads: AtomicUsize::new(0),
            }
        }

        /// Every request the backend saw, in order.
        pub fn log(&self) -> Vec<String> {
            self.log.lock().expect("request log").clone()
        }

        /// The body of every submit the backend accepted.
        pub fn submissions(&self) -> Vec<Value> {
            self.submissions.lock().expect("submissions").clone()
        }

        fn record(&self, line: impl Into<String>) {
            self.log.lock().expect("request log").push(line.into());
        }

        fn record_submission(&self, body: Value) {
            self.submissions.lock().expect("submissions").push(body);
        }

        /// One tick of the latch: `true` while the backend should
        /// still say "not yet".
        ///
        /// Monotonic on purpose. Comfy's poll and harvest are the
        /// *same* `GET /history/{id}` call, so a script that could
        /// ever go back to "still generating" would have the harvest
        /// read a half-finished entry one call after the poll said
        /// Done. Once the threshold is crossed it stays crossed.
        fn still_working(&self) -> bool {
            self.history_reads.fetch_add(1, Ordering::SeqCst) < self.pending_reads
        }

        /// Writes the artefacts the success fixture says the backend
        /// produced.
        ///
        /// Called from submit rather than from the completion read:
        /// nothing observes the directory between the two, so the
        /// later moment would buy realism nobody can see, and the
        /// earlier one keeps the write on the path that has the body
        /// in hand.
        fn write_declared_outputs(&self) {
            let Some(root) = &self.output_root else {
                return;
            };
            let Outcome::Finished { outputs } = &self.outcome else {
                return;
            };
            let Some(nodes) = outputs.as_object() else {
                return;
            };
            for node in nodes.values() {
                let Some(images) = node.get("images").and_then(|v| v.as_array()) else {
                    continue;
                };
                for img in images {
                    let filename = img
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("output.png");
                    let subfolder = img
                        .get("subfolder")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let mut path = root.clone();
                    if !subfolder.is_empty() {
                        path.push(subfolder);
                    }
                    std::fs::create_dir_all(&path).expect("backend output dir");
                    path.push(filename);
                    std::fs::write(&path, super::PNG_1X1).expect("backend output file");
                }
            }
        }
    }

    /// Binds an ephemeral loopback port and serves the fake on it.
    ///
    /// Bind happens before the spawn so the exporter's first request
    /// cannot arrive at a closed port: connections made between `bind`
    /// and `serve` wait in the accept backlog, which is why no sleep
    /// is needed here.
    ///
    /// The caller keeps the returned `Arc` for the whole test — it is
    /// how the log and the submissions are read afterwards, and the
    /// serve task holds its own clone regardless.
    pub async fn spawn(backend: FakeBackend) -> (Arc<FakeBackend>, u16) {
        spawn_with_port(|_| backend).await
    }

    /// [`spawn`] for a script that has to name the port it is served
    /// on — a backend whose result URLs point back at itself, which is
    /// what a profile with a `fetch` block downloads from. The listener
    /// is bound first so the port is known before the script is built.
    pub async fn spawn_with_port(
        build: impl FnOnce(u16) -> FakeBackend,
    ) -> (Arc<FakeBackend>, u16) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake backend");
        let port = listener.local_addr().expect("local_addr").port();
        let state = Arc::new(build(port));
        // axum 0.8 path parameters: `{id}`, not the `:id` of 0.7.
        let app = Router::new()
            .route("/prompt", post(comfy_submit))
            .route("/history/{id}", get(comfy_history))
            .route("/generate", post(http_submit))
            .route("/status/{id}", get(http_status))
            .route("/result/{id}", get(http_result))
            .route("/artefact/{name}", get(http_artefact))
            .with_state(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (state, port)
    }

    /// ComfyUI's `POST /prompt`: the queue accepts the graph and
    /// answers with the id everything else is keyed by.
    async fn comfy_submit(
        State(backend): State<Arc<FakeBackend>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        backend.record("POST /prompt");
        backend.record_submission(body);
        backend.write_declared_outputs();
        Json(json!({
            "prompt_id": backend.job_id,
            "number": 1,
            "node_errors": {},
        }))
    }

    /// ComfyUI's `GET /history/{prompt_id}`, which serves both the
    /// poll and the harvest. An unknown id gets an empty object —
    /// exactly what a real Comfy returns for a prompt it has never
    /// heard of, and what the exporter reads as "not ready yet".
    async fn comfy_history(
        State(backend): State<Arc<FakeBackend>>,
        UrlPath(id): UrlPath<String>,
    ) -> Json<Value> {
        backend.record(format!("GET /history/{id}"));
        if id != backend.job_id {
            return Json(json!({}));
        }
        if backend.still_working() {
            return Json(json!({}));
        }
        let entry = match &backend.outcome {
            Outcome::Finished { outputs } => json!({
                "status": { "status_str": "success", "completed": true, "messages": [] },
                "outputs": outputs,
            }),
            // A real ComfyUI reports execution failures by appending an
            // `execution_error` message to `status.messages[]`; it does
            // not write a `status.error` string. The exporter only ever
            // reads `status.error` (`exporter-comfy/src/lib.rs`), so the
            // fixture matches the code as it stands rather than the
            // backend as it is. If the exporter learns to read
            // `messages[]`, this fixture is the thing to update — the
            // assertion below would otherwise keep passing against a
            // shape the real backend never sends.
            Outcome::Failed { message } => json!({
                "status": { "status_str": "error", "completed": false, "error": message },
            }),
        };
        let mut history = serde_json::Map::new();
        history.insert(backend.job_id.to_string(), entry);
        Json(Value::Object(history))
    }

    /// The schema-driven backend's submit.
    async fn http_submit(
        State(backend): State<Arc<FakeBackend>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        backend.record("POST /generate");
        backend.record_submission(body);
        backend.write_declared_outputs();
        Json(json!({ "job_id": backend.job_id }))
    }

    /// The schema-driven backend's status route — the one the
    /// `done_when` / `failed_when` / `progress_message_path` rules are
    /// written against.
    async fn http_status(
        State(backend): State<Arc<FakeBackend>>,
        UrlPath(id): UrlPath<String>,
    ) -> Json<Value> {
        backend.record(format!("GET /status/{id}"));
        if id != backend.job_id {
            return Json(json!({ "status": "unknown" }));
        }
        if backend.still_working() {
            return Json(json!({
                "status": "running",
                "status_message": "rendering tiles",
            }));
        }
        match &backend.outcome {
            Outcome::Finished { .. } => Json(json!({ "status": "done" })),
            Outcome::Failed { message } => Json(json!({
                "status": "failed",
                "error": message,
            })),
        }
    }

    /// The schema-driven backend's result route. Unconditional: the
    /// exporter only reaches it once poll has already said Done.
    async fn http_result(
        State(backend): State<Arc<FakeBackend>>,
        UrlPath(id): UrlPath<String>,
    ) -> Json<Value> {
        backend.record(format!("GET /result/{id}"));
        let items = match &backend.outcome {
            Outcome::Finished { outputs } => outputs.clone(),
            Outcome::Failed { .. } => json!([]),
        };
        // `seed` is the point of the envelope: a value the backend
        // decided, sitting beside the artefacts rather than inside one.
        Json(json!({ "outputs": items, "seed": 913_224 }))
    }

    /// The bytes themselves, for a profile whose `fetch` block pulls
    /// them into custody instead of pointing an asset at this URL.
    async fn http_artefact(
        State(backend): State<Arc<FakeBackend>>,
        UrlPath(name): UrlPath<String>,
    ) -> Vec<u8> {
        backend.record(format!("GET /artefact/{name}"));
        super::PNG_1X1.to_vec()
    }
}

/// The whole outbound feature against a backend that makes it wait:
/// submit, one "not yet", then a finished job whose two images become
/// two assets with a route back to the original.
///
/// The one poll that comes back empty is the point. It is what turns
/// the tick count and the re-enqueue log into statements about the
/// state machine — a runner that harvested straight after dispatch, or
/// forgot to ask for another tick while Running, would land on
/// different numbers here.
#[tokio::test(flavor = "multi_thread")]
async fn a_comfy_export_waits_for_the_backend_and_harvests_what_it_made() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    for dir in [&corpus, &outbox] {
        std::fs::create_dir_all(dir).expect("fixture dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    // Held for the whole test: the assertions below read the request
    // log and the submitted bodies off this handle.
    let (backend, port) = fake_backend::spawn(FakeBackend::new(
        COMFY_JOB_ID,
        1,
        Outcome::Finished {
            outputs: comfy_success_outputs(),
        },
        Some(outbox.clone()),
    ))
    .await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-comfy-harvest".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 fixture path"),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let export = export_via(
        &core,
        &db_path,
        &persona.id,
        &original.id,
        Arc::new(ComfyHttpExporter::with_client(backend_client())),
        "img2img",
        comfy_params(port, Some(&outbox)),
    )
    .await;

    // (A) What the backend was actually told.
    let submissions = backend.submissions();
    assert_eq!(submissions.len(), 1, "one dispatch, one submit");
    let body = &submissions[0];
    assert_eq!(
        body["client_id"], export.dispatch_id,
        "Comfy's client id is the dispatch, so the backchannel names the job Asterism knows"
    );
    assert_eq!(
        body["prompt"]["10"]["inputs"]["image"],
        plate.display().to_string(),
        "freeze, card hydration and the input_slot rewrite all resolved to the original"
    );
    assert_eq!(
        body["prompt"]["3"]["inputs"]["steps"], 30,
        "everything outside the input slot rides through verbatim"
    );
    assert_eq!(
        backend.log(),
        vec![
            "POST /prompt".to_string(),
            format!("GET /history/{COMFY_JOB_ID}"),
            format!("GET /history/{COMFY_JOB_ID}"),
            format!("GET /history/{COMFY_JOB_ID}"),
        ],
        "submit, the poll that came back empty, the poll that said done, and the harvest"
    );

    // (B) The state machine, with the test as its only driver.
    assert_eq!(
        export.ticks,
        vec![
            ("running".to_string(), Some("dispatched".to_string())),
            ("running".to_string(), Some(comfy_waiting_message())),
            ("done".to_string(), None),
        ],
        "the middle row is the exporter's own progress hint, poll interval and all"
    );
    assert_eq!(
        export.reenqueued,
        vec![export.dispatch_id.clone(), export.dispatch_id.clone()],
        "one tick asked for after dispatch, one after the poll that was still running"
    );
    let dispatch = core
        .dispatch_service
        .get(&export.dispatch_id)
        .await
        .expect("dispatch get");
    assert_eq!(dispatch.state, "done");
    assert!(
        dispatch.completed_at_ms.is_some(),
        "a terminal dispatch records when it landed"
    );

    // (C) What the harvest reified. Two, not three: node "12" produced
    // text and no images, and a node with nothing to collect is skipped
    // rather than turned into an asset with no artefact behind it.
    assert_eq!(export.output_ids.len(), 2);
    let first = detail_of(&core, &export.output_ids[0]).await;
    let second = detail_of(&core, &export.output_ids[1]).await;
    assert_eq!(
        first.locator,
        format!("{}/asterism_00001_.png", outbox.display()),
        "output_dir set, empty subfolder: dir and filename, nothing between them"
    );
    assert_eq!(
        second.locator,
        format!("{}/batch/asterism_00002_.png", outbox.display()),
        "a non-empty subfolder becomes a path segment"
    );
    for facts in [&first, &second] {
        assert!(
            Path::new(&facts.locator).is_file(),
            "the backend wrote what it said it made: {}",
            facts.locator
        );
        assert_eq!(facts.source_kind, "dispatch-comfy");
        assert_eq!(facts.modality.as_deref(), Some("image"));
        assert_eq!(
            facts.mime.as_deref(),
            Some("image/png"),
            "the material's format fact comes from the locator's extension"
        );
        assert_eq!(
            facts.bundle_id.as_deref(),
            Some(export.dispatch_id.as_str()),
            "dispatch siblings cluster on the grid by dispatch id"
        );
        // `inbox` is an ingest-side label; nothing in reify adds it.
        assert_eq!(
            facts.labels,
            vec![
                "exporter:comfy".to_string(),
                "comfy:img2img".to_string(),
                "comfy_node:9".to_string(),
            ],
            "the exporter slug is prepended to the exporter's own chips"
        );
        assert_eq!(facts.extra["comfy"]["prompt_id"], COMFY_JOB_ID);
        assert_eq!(facts.extra["comfy"]["node_id"], "9");
        assert_eq!(facts.extra["dispatch_id"], export.dispatch_id);
        assert_eq!(
            facts.extra["_dispatch"],
            json!({
                "selection_id": export.snapshot_id,
                "dispatch_id": export.dispatch_id,
                "exporter_slug": "comfy",
            }),
            "the dispatch trace is merged under its own key, leaving the exporter's payload alone"
        );
    }
    assert_eq!(first.extra["comfy"]["image_index"], 0);
    assert_eq!(first.extra["comfy"]["filename"], "asterism_00001_.png");
    assert_eq!(first.extra["comfy"]["subfolder"], "");
    assert_eq!(second.extra["comfy"]["image_index"], 1);
    assert_eq!(second.extra["comfy"]["filename"], "asterism_00002_.png");
    assert_eq!(second.extra["comfy"]["subfolder"], "batch");

    // (D) What the ledger made of it.
    let edges = core
        .asset_service
        .edges_of(&export.output_ids[0], Some("derived_from"), 10)
        .await
        .expect("derived_from edges");
    assert_eq!(edges.len(), 1, "one input in the snapshot, one parent");
    assert_eq!(edges[0].to_asset_id, original.id);
    assert_eq!(
        edges[0].label.as_deref(),
        Some("dispatch-comfy"),
        "a hop Asterism ran itself, labelled with the exporter that ran it"
    );

    let view = core
        .asset_service
        .lineage_of(&export.output_ids[0], None, 8)
        .await
        .expect("lineage");
    assert!(!view.truncated, "two nodes fit well inside the budget");
    assert_eq!(view.nodes.len(), 2, "the output and the original");
    // The other image of the same dispatch is a sibling, not an
    // ancestor: it hangs off the same original, one hop the other way.
    // A walk that stepped down from a parent into its other children
    // would put it here (the distance guard that stops that is what
    // 10340c6 fixed).
    let sibling = &export.output_ids[1];
    assert!(
        view.nodes.iter().all(|n| &n.card.id != sibling),
        "a sibling export is not on this artefact's route"
    );
    let node_of = |id: &str| {
        view.nodes
            .iter()
            .find(|n| n.card.id == id)
            .unwrap_or_else(|| panic!("node {id} is in the walk"))
    };
    assert_eq!(node_of(&export.output_ids[0]).depth, 0);
    assert_eq!(node_of(&original.id).depth, 1);
    assert_eq!(view.edges.len(), 1, "one link between two nodes");
    assert_eq!(
        view.roots,
        vec![original.id.clone()],
        "the chain begins at the asset nothing else produced"
    );
    assert_eq!(
        view.dispatch_ids,
        vec![export.dispatch_id.clone()],
        "one export happened, so the backbone names it once"
    );
    assert_eq!(
        node_of(&export.output_ids[0]).dispatch_id.as_deref(),
        Some(export.dispatch_id.as_str()),
        "`dispatch_id` on a node means produced by that dispatch"
    );
    assert_eq!(
        node_of(&original.id).dispatch_id,
        None,
        "the original was imported, not produced by an export"
    );

    let page = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona.id.clone()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list assets");
    assert_eq!(
        page.items.len(),
        3,
        "the original and its two derivations, and nothing invented alongside them"
    );
}

/// The same export with `output_dir` left out — the case where
/// Asterism has no view of Comfy's filesystem and records the URL the
/// backend serves the file from instead.
///
/// One params key is the only difference from the happy path, and it
/// changes what a locator *is*: a path on this machine, or a request
/// to another process. Everything downstream reads that string, so
/// this is the test that says which one it gets.
#[tokio::test(flavor = "multi_thread")]
async fn a_comfy_export_told_no_output_dir_records_the_view_url_instead() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    for dir in [&corpus, &outbox] {
        std::fs::create_dir_all(dir).expect("fixture dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    // The backend still writes its files: a real Comfy always writes
    // into its own output dir, whether or not the caller told Asterism
    // where that is. The files existing is what makes the assertion
    // below ("no asset points at them") mean something.
    let (backend, port) = fake_backend::spawn(FakeBackend::new(
        COMFY_JOB_ID,
        1,
        Outcome::Finished {
            outputs: comfy_success_outputs(),
        },
        Some(outbox.clone()),
    ))
    .await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-comfy-view-url".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 fixture path"),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let export = export_via(
        &core,
        &db_path,
        &persona.id,
        &original.id,
        Arc::new(ComfyHttpExporter::with_client(backend_client())),
        "img2img",
        comfy_params(port, None),
    )
    .await;

    // The wait is unchanged — the locator form is a harvest-time
    // decision and touches nothing about the state machine.
    assert_eq!(
        export.ticks,
        vec![
            ("running".to_string(), Some("dispatched".to_string())),
            ("running".to_string(), Some(comfy_waiting_message())),
            ("done".to_string(), None),
        ]
    );
    assert_eq!(export.reenqueued.len(), 2);
    assert_eq!(export.output_ids.len(), 2);

    let first = detail_of(&core, &export.output_ids[0]).await;
    let second = detail_of(&core, &export.output_ids[1]).await;
    assert_eq!(
        first.locator,
        format!("http://127.0.0.1:{port}/view?filename=asterism_00001_.png"),
        "no output_dir: the locator is the URL the backend serves the file from"
    );
    assert_eq!(
        second.locator,
        format!("http://127.0.0.1:{port}/view?filename=asterism_00002_.png&subfolder=batch"),
        "the subfolder parameter is appended only when there is one"
    );
    for facts in [&first, &second] {
        assert_eq!(facts.source_kind, "dispatch-comfy");
        assert_eq!(facts.modality.as_deref(), Some("image"));
        // Current behaviour, deliberately pinned: the mime guess reads
        // the extension of the locator's *path*, and `/view` has none
        // (the query string is dropped first). So a URL-locator image
        // carries `modality = image` with `mime = None`. Not a defect
        // to fix here — it is what "Asterism has never seen this file"
        // looks like in the material layer, and a test that ignored it
        // would go silently green the day the rule changes.
        assert_eq!(facts.mime, None, "a /view URL has no extension to guess");
        assert_eq!(
            facts.bundle_id.as_deref(),
            Some(export.dispatch_id.as_str())
        );
    }

    // The artefacts are on disk — the backend wrote them — and nothing
    // in the library claims them, because nobody told the exporter
    // where to look.
    let on_disk = outbox.join("asterism_00001_.png");
    assert!(on_disk.is_file(), "the backend wrote its output as always");
    let on_disk = on_disk.display().to_string();
    assert!(
        [&first, &second].iter().all(|f| f.locator != on_disk),
        "no output_dir, no filesystem locator — the path is not guessed at"
    );
    // The fake's own log is the control: this URL form was reached
    // through the same three history reads, not by some other route.
    assert_eq!(
        backend.log(),
        vec![
            "POST /prompt".to_string(),
            format!("GET /history/{COMFY_JOB_ID}"),
            format!("GET /history/{COMFY_JOB_ID}"),
            format!("GET /history/{COMFY_JOB_ID}"),
        ]
    );
}

/// The backend refuses the job, and the library records that instead
/// of inventing an artefact.
///
/// The failure has to arrive through the *poll* — the submit succeeded,
/// so the job is Running when the bad news comes. What that buys is the
/// negative half of the harvest contract: a poll that says Failed must
/// stop the machine there, and the absence of a harvest request in the
/// log is the only direct evidence of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_backend_that_reports_an_error_mints_nothing_and_says_why() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    for dir in [&corpus, &outbox] {
        std::fs::create_dir_all(dir).expect("fixture dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    // `pending_reads: 0` — the first poll already has the answer, so
    // the run is two ticks rather than three. The output root is set
    // and stays untouched: a failed job has nothing to declare, so
    // there is nothing for the fake to write.
    let (backend, port) = fake_backend::spawn(FakeBackend::new(
        COMFY_JOB_ID,
        0,
        Outcome::Failed {
            message: "CUDA out of memory while sampling".into(),
        },
        Some(outbox.clone()),
    ))
    .await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-comfy-failure".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 fixture path"),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let export = export_via(
        &core,
        &db_path,
        &persona.id,
        &original.id,
        Arc::new(ComfyHttpExporter::with_client(backend_client())),
        "img2img",
        comfy_params(port, Some(&outbox)),
    )
    .await;

    let reason = "CUDA out of memory while sampling".to_string();
    assert_eq!(
        export.ticks,
        vec![
            ("running".to_string(), Some("dispatched".to_string())),
            ("failed".to_string(), Some(reason)),
        ],
        "the backend's own words reach the row a reader will see"
    );
    assert_eq!(
        export.reenqueued,
        vec![export.dispatch_id.clone()],
        "the tick after dispatch, and no tick after a terminal state"
    );
    assert_eq!(
        backend.log(),
        vec![
            "POST /prompt".to_string(),
            format!("GET /history/{COMFY_JOB_ID}"),
        ],
        "no second history read: a failed poll never reaches the harvest"
    );

    let dispatch = core
        .dispatch_service
        .get(&export.dispatch_id)
        .await
        .expect("dispatch get");
    assert_eq!(dispatch.state, "failed");
    assert!(
        dispatch.output_asset_ids.is_empty(),
        "nothing was produced, so nothing is claimed"
    );
    assert!(
        dispatch.completed_at_ms.is_some(),
        "failure is a landing, and the row records when it happened"
    );

    let page = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona.id.clone()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list assets");
    assert_eq!(page.items.len(), 1, "only the original");
    let written = std::fs::read_dir(&outbox).expect("read outbox").count();
    // Fixture honesty check: the *fake* declines to write files for a
    // Failed outcome (Asterism itself never writes into a comfy
    // output_dir — the exporter only builds locator strings).
    assert_eq!(
        written, 0,
        "the fake backend wrote nothing for a failed job"
    );

    let edges = core
        .asset_service
        .edges_of(&original.id, Some("derived_from"), 10)
        .await
        .expect("derived_from edges");
    assert!(edges.is_empty(), "no derivation, no link");
    let view = core
        .asset_service
        .lineage_of(&original.id, None, 8)
        .await
        .expect("lineage");
    assert_eq!(view.nodes.len(), 1);
    assert_eq!(view.roots, vec![original.id.clone()]);
    assert!(
        view.dispatch_ids.is_empty(),
        "an export that produced nothing is not part of any artefact's route"
    );
}

/// The same state machine driven by an exporter that knows nothing
/// about the backend it is talking to — the protocol arrives as JSON in
/// the dispatch params.
///
/// Three template roots resolve in the submit body alone
/// (`{{input[0].source_locator}}`, `{{params.extras.prompt}}`,
/// `{{dispatch_id}}`), the handle is plucked out of the response by
/// JSONPath and spliced back into two later URLs, and the progress
/// message the middle tick shows was named by a path in the params
/// rather than written into any Rust. That whole chain is what
/// "one adapter, N backends" has to mean to be worth having.
#[tokio::test(flavor = "multi_thread")]
async fn a_schema_driven_http_export_drives_the_same_state_machine() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("fixture dir");
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    // No output root: this backend serves its results from URLs, so
    // there is no directory for it to write into.
    let (backend, port) = fake_backend::spawn(FakeBackend::new(
        HTTP_JOB_ID,
        1,
        Outcome::Finished {
            outputs: http_result_items(),
        },
        None,
    ))
    .await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-http-harvest".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 fixture path"),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let export = export_via(
        &core,
        &db_path,
        &persona.id,
        &original.id,
        Arc::new(HttpExporter::with_client(
            tmp.path().join("custody"),
            backend_client(),
        )),
        "render",
        http_params(port),
    )
    .await;

    let submissions = backend.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0],
        json!({
            "image": plate.display().to_string(),
            "prompt": "a test plate",
            "client_id": export.dispatch_id,
        }),
        "every leaf of the body template resolved, and nothing else was added"
    );
    assert_eq!(
        backend.log(),
        vec![
            "POST /generate".to_string(),
            format!("GET /status/{HTTP_JOB_ID}"),
            format!("GET /status/{HTTP_JOB_ID}"),
            format!("GET /result/{HTTP_JOB_ID}"),
        ],
        "the handle JSONPath fed both later URLs"
    );

    assert_eq!(
        export.ticks,
        vec![
            ("running".to_string(), Some("dispatched".to_string())),
            ("running".to_string(), Some("rendering tiles".to_string())),
            ("done".to_string(), None),
        ],
        "the middle message came out of `progress_message_path`, not out of any Rust"
    );
    assert_eq!(export.reenqueued.len(), 2);
    assert_eq!(export.output_ids.len(), 2, "two items, two assets");

    let first = detail_of(&core, &export.output_ids[0]).await;
    let second = detail_of(&core, &export.output_ids[1]).await;
    assert_eq!(first.locator, "https://renders.test/first.png");
    assert_eq!(second.locator, "https://renders.test/second.png");
    assert_eq!(
        first.cover.as_deref(),
        Some("first plate"),
        "the item's caption became the card's cover"
    );
    assert_eq!(
        second.cover, None,
        "an optional placeholder over a missing field is empty, and empty is no cover"
    );
    for facts in [&first, &second] {
        assert_eq!(facts.source_kind, "dispatch-http");
        assert_eq!(facts.modality.as_deref(), Some("image"));
        assert_eq!(
            facts.labels,
            vec![
                "exporter:http".to_string(),
                format!("batch:{}", export.dispatch_id),
            ],
            "a static label template is substituted like any other"
        );
        assert_eq!(
            facts.bundle_id.as_deref(),
            Some(export.dispatch_id.as_str())
        );
        assert_eq!(facts.extra["_dispatch"]["exporter_slug"], "http");
    }
    assert_eq!(
        first.extra["http"]["item"],
        json!({ "url": "https://renders.test/first.png", "caption": "first plate" }),
        "the backend's own item is kept verbatim beside the mapped fields"
    );
    assert_eq!(
        second.extra["http"]["item"],
        json!({ "url": "https://renders.test/second.png" })
    );

    let edges = core
        .asset_service
        .edges_of(&export.output_ids[0], Some("derived_from"), 10)
        .await
        .expect("derived_from edges");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to_asset_id, original.id);
    assert_eq!(
        edges[0].label.as_deref(),
        Some("dispatch-http"),
        "the edge label follows the exporter slug, whichever exporter ran"
    );
}

/// The same adapter, the same state machine, with the two blocks a
/// hosted platform needs turned on: the bytes are pulled into custody
/// before the harvest returns, and the call that produced them arrives
/// on the asset.
///
/// This is what used to be a second crate. What it proves here is the
/// wiring rather than the mapping — that the locator names the file we
/// hold instead of a URL that is expected to stop working, that the
/// bytes on disk are the bytes the backend served, and that the record
/// riding on the asset carries what we sent and what came back. The
/// envelope assertion is the one with teeth: `seed` is a sibling of the
/// artefacts array, so an adapter that kept only the item that became
/// this asset would drop it.
#[tokio::test(flavor = "multi_thread")]
async fn a_profile_that_asks_for_custody_lands_the_bytes_and_the_record() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("fixture dir");
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    // This backend's result URLs point back at itself, so the script
    // cannot be written until the port is known.
    let (backend, port) = fake_backend::spawn_with_port(|port| {
        FakeBackend::new(
            HTTP_JOB_ID,
            0,
            Outcome::Finished {
                outputs: json!([{ "url": format!("http://127.0.0.1:{port}/artefact/a.png") }]),
            },
            None,
        )
    })
    .await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-http-custody".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 fixture path"),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let custody_root = tmp.path().join("custody");
    let mut params = http_params(port);
    params["fetch"] = json!({ "authenticated": false });
    params["deadline_seconds"] = json!(86_400);

    let export = export_via(
        &core,
        &db_path,
        &persona.id,
        &original.id,
        Arc::new(HttpExporter::with_client(
            custody_root.clone(),
            backend_client(),
        )),
        "render",
        params,
    )
    .await;

    assert_eq!(export.output_ids.len(), 1, "one item, one asset");
    assert!(
        backend.log().contains(&"GET /artefact/a.png".to_string()),
        "the fetch step has to have happened: {:?}",
        backend.log()
    );

    let facts = detail_of(&core, &export.output_ids[0]).await;
    let locator = std::path::Path::new(&facts.locator);
    assert!(
        locator.starts_with(&custody_root),
        "the locator names the file we hold, not the URL the backend served: {}",
        facts.locator
    );
    assert_eq!(
        std::fs::read(locator).expect("the custody file exists"),
        PNG_1X1,
        "what landed on disk is what the backend served"
    );
    assert_eq!(
        facts.extra["http"]["source_url"],
        json!(format!("http://127.0.0.1:{port}/artefact/a.png")),
        "the URL that is expected to expire is kept beside the file"
    );

    let call = &facts.extra["http"]["call"];
    assert_eq!(call["handle"], json!(HTTP_JOB_ID));
    assert_eq!(
        call["request"]["body"]["prompt"], "a test plate",
        "the prompt as sent rides in with the artefact"
    );
    assert_eq!(call["response"]["job_id"], HTTP_JOB_ID);
    assert_eq!(
        call["result"]["seed"], 913_224,
        "the envelope is kept whole, so what the backend decided survives"
    );
    assert!(
        call["submitted_at_ms"].is_i64(),
        "the submit moment is what a deadline is measured from: {call}"
    );

    // The same record read the other way: off the dispatch row, which
    // is where it lives, rather than off an artefact it was copied to.
    // A reader asking what this call sent and what came back of it gets
    // there through the wire shape — no SQL, and no asset needed, which
    // matters most for a submit that produced none.
    let row = core
        .dispatch_service
        .get(&export.dispatch_id)
        .await
        .expect("dispatch get");
    let handle: serde_json::Value = serde_json::from_str(
        row.handle_json
            .as_deref()
            .expect("a dispatch the backend accepted carries the handle it was issued"),
    )
    .expect("the handle payload reaches the wire as JSON text");
    assert_eq!(handle["handle"], json!(HTTP_JOB_ID));
    assert_eq!(
        handle["exchange"]["request"]["body"]["prompt"], "a test plate",
        "the request as sent is readable without opening the database"
    );
    assert_eq!(
        handle["exchange"]["response"]["job_id"], HTTP_JOB_ID,
        "and the response as received beside it"
    );
}

/// The schema-driven half of the failure story: the reason the row
/// shows was pulled out of the backend's response by a JSONPath the
/// caller wrote (`failed_when.message_path`).
///
/// Without this, a deployment could describe a backend whose failures
/// are perfectly well reported and still show its users
/// "backend reported failure" — the fallback string the exporter uses
/// when the path resolves to nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_schema_driven_export_reports_the_backends_own_failure_message() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("fixture dir");
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    let (backend, port) = fake_backend::spawn(FakeBackend::new(
        HTTP_JOB_ID,
        0,
        Outcome::Failed {
            message: "the renderer gave up".into(),
        },
        None,
    ))
    .await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-http-failure".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 fixture path"),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let export = export_via(
        &core,
        &db_path,
        &persona.id,
        &original.id,
        Arc::new(HttpExporter::with_client(
            tmp.path().join("custody"),
            backend_client(),
        )),
        "render",
        http_params(port),
    )
    .await;

    assert_eq!(
        export.ticks,
        vec![
            ("running".to_string(), Some("dispatched".to_string())),
            ("failed".to_string(), Some("the renderer gave up".into())),
        ],
        "`failed_when` fired and `message_path` said what to show"
    );
    assert_eq!(export.reenqueued, vec![export.dispatch_id.clone()]);
    assert_eq!(
        backend.log(),
        vec![
            "POST /generate".to_string(),
            format!("GET /status/{HTTP_JOB_ID}"),
        ],
        "the result route is never reached"
    );
    assert!(export.output_ids.is_empty(), "a failed job reifies nothing");

    let page = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona.id.clone()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list assets");
    assert_eq!(page.items.len(), 1, "only the original");
}
