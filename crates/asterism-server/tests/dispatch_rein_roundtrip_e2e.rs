//! A round trip taken literally: out through the exporter, back
//! through the importer.
//!
//! Every other e2e in this crate declares provenance by hand — it calls
//! `asset_service.add` with `derived_from: Some("dispatch:…")` and
//! checks what the ledger makes of it. That skips the half of the
//! feature that actually runs in production: the `file` exporter writes
//! a `<name>.meta.json` sidecar beside the artefact it copies out, and
//! the image importer declares `derived_from: sidecar` because it found
//! one sitting next to the file coming in. Nobody types the claim; the
//! filesystem carries it. This binary is where that hand-off is
//! exercised, so the link is checked at the seam where it can break —
//! one `.is_file()` probe in `asterism-importer-image` and one sidecar
//! read in `AssetService`.
//!
//! Both legs are real: the corpus is scanned by `FsScanner`, parsed by
//! `ImageParser`, and POSTed to the actual router over loopback; the
//! export is driven through the real `DispatchRun` state machine with
//! the real `FileExporter` behind it.
//!
//! **Why `CoreMode::ReadOnly`.** Its rustdoc is written for the
//! standalone server sharing a database with a running UI, which reads
//! like a mismatch here. What this test needs from it is the other
//! half of the same property: `ReadOnly` opens the job queue without
//! spawning a worker `Monitor`. `DispatchService::create` still
//! enqueues the `DispatchRun` job — it just sits there, because nothing
//! drains it. That leaves the test as the only thing advancing the
//! state machine, so the tick count and the re-enqueue log are facts
//! about the runner rather than a race with a background worker. Under
//! `Full` a worker would reach the same exporter concurrently and both
//! numbers would stop meaning anything.
//!
//! Its own test binary because `init_core` opens a Tantivy index (one
//! core per test binary, as with the sibling e2e files).

use std::sync::Arc;

use asterism_contract::command::{
    CreateDispatchCommand, CreateSnapshotCommand, RegisterPersonaCommand,
};
use asterism_contract::query::{GetAssetDetailQuery, ListAssetsQuery};
use asterism_contract::sidecar::{SIDECAR_IDENTITY_KEY, SIDECAR_SCHEMA};
use asterism_core::domain::value::DispatchId;
use asterism_core::error::DomainError;
use asterism_exporter_file::FileExporter;
use asterism_importer_image::ImageParser;
use asterism_importer_sdk::{FsScanner, ImportOptions, ImportSummary, ScanMode, run_import};
use asterism_infra::dispatch::{DispatchRunEnv, ExporterRegistry, ReEnqueue, run_dispatch_run};
use asterism_infra::sqlite;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution this fixture writes with: a caller that states
/// nothing, which records nothing.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}
use asterism_server::state::ServerCtx;

/// A 1×1 RGBA PNG, 67 bytes. Nothing on this route decodes pixels —
/// the dimensions the parser reads come from the IHDR header, the
/// `tEXt` scan finds no chunks, and the thumbnail / cover jobs do not
/// run in `ReadOnly` — so a minimal header-valid file is the whole
/// fixture.
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

/// What the artefact picked up on its way through the outside world.
/// PNG readers stop at `IEND`, so appending leaves a file that is
/// still a readable PNG and demonstrably not the one that went out.
const PROCESSING_MARK: &[u8] = b"asterism-e2e-processed\n";

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

/// What one export left behind: the ids the ledger now holds, plus
/// what the state machine did on the way there.
struct ExportRun {
    /// The dispatch that ran.
    dispatch_id: String,
    /// The single asset it reified — the copy sitting in `outbox`.
    copy_id: String,
    /// Ticks needed to reach a terminal state.
    ticks: usize,
    /// Dispatch ids the runner handed back for another tick.
    reenqueued: Vec<String>,
}

/// Boots a core plus the real router on an ephemeral loopback port.
/// The `CoreCtx` must outlive the test body — dropping it shuts the
/// service graph down underneath the serve task.
///
/// Bind happens before the spawn so the importer's first request
/// cannot arrive at a closed port: connections made between `bind`
/// and `serve` wait in the accept backlog.
async fn boot(tmp: &std::path::Path) -> (CoreCtx, u16) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (core, port)
}

/// Assembles the runner's dependency bundle — the one thing the
/// composition root keeps to itself. `CoreCtx` exposes the services
/// but not the repositories behind them, so the read-side ports come
/// from a second isle opened on the same database file (WAL,
/// `busy_timeout`, `BEGIN IMMEDIATE`; every step here is awaited in
/// sequence, so a read never races a write it depends on).
async fn dispatch_env(
    db_path: &std::path::Path,
    core: &CoreCtx,
) -> (DispatchRunEnv, Arc<RecordingReEnqueue>) {
    let (isle, driver) = sqlite::open_and_migrate(db_path)
        .await
        .expect("second isle");
    // Dropping the driver joins the SQLite thread and takes the isle
    // with it, so the handle has to outlive every call made through
    // this environment. `core_init` holds its own driver the same way
    // (graceful shutdown is a future addition). The leak is bounded:
    // one driver per test, three in this binary, all reclaimed when
    // the test process exits.
    std::mem::forget(driver);

    let reenqueue = Arc::new(RecordingReEnqueue::default());
    let env = DispatchRunEnv {
        // Only `file` is registered, so a typo in the slug fails as a
        // missing exporter rather than reaching a network backend.
        registry: ExporterRegistry::single(Arc::new(FileExporter::new())),
        service: core.support.dispatch_runner.clone(),
        snapshots: Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        dispatches: Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        assets: Arc::new(sqlite::repo::SqliteAssetRepository::new(isle)),
        reenqueue: reenqueue.clone(),
    };
    (env, reenqueue)
}

/// Ticks the state machine until the dispatch is terminal, returning
/// how many ticks that took. The file exporter does its filesystem
/// work synchronously inside `dispatch`, so two is the expected
/// number: one to send, one to poll-and-harvest.
async fn drive_to_terminal(env: &DispatchRunEnv, core: &CoreCtx, dispatch_id: &str) -> usize {
    let payload = serde_json::json!({ "dispatch_id": dispatch_id });
    for tick in 1..=8usize {
        run_dispatch_run(env, &payload)
            .await
            .expect("dispatch tick");
        let dto = core
            .dispatch_service
            .get(dispatch_id)
            .await
            .expect("dispatch get");
        if matches!(dto.state.as_str(), "done" | "failed" | "cancelled") {
            return tick;
        }
    }
    panic!("dispatch did not reach a terminal state in 8 ticks");
}

/// Runs the real importer over one directory: scan → parse → POST to
/// the router on `port`.
///
/// `ScanMode::Enumerate` is fixed rather than taken as an argument —
/// `Watch` keeps the stream open forever, which in a test is a hang
/// with no message.
async fn import_png_dir(root: &std::path::Path, persona_id: &str, port: u16) -> ImportSummary {
    let mut options = ImportOptions::new(persona_id);
    options.server = format!("http://127.0.0.1:{port}");
    run_import(
        &FsScanner::new(root).with_extensions(["png"]),
        &ImageParser::new(Some("E2E".into())),
        ScanMode::Enumerate,
        options,
    )
    .await
    .expect("import run")
}

/// Finds the asset holding `locator`. `ImportSummary` counts rows, it
/// does not name them, and the list query has no locator filter — so
/// the path the scanner recorded (`entry.path().display()`, no
/// canonicalisation) is the join key.
async fn asset_id_by_locator(
    core: &CoreCtx,
    persona_id: &str,
    locator: &std::path::Path,
) -> String {
    let wanted = locator.display().to_string();
    let page = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona_id.to_string()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list assets");
    page.items
        .iter()
        .find(|card| card.source_locator == wanted)
        .unwrap_or_else(|| panic!("no asset holds locator {wanted}"))
        .id
        .clone()
}

/// The asset's `extra` bag, parsed. `Null` when the asset carries
/// none — so a caller can ask for a key either way.
async fn extra_of(core: &CoreCtx, asset_id: &str) -> serde_json::Value {
    let detail = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: asset_id.to_string(),
            viewer_subject: None,
        })
        .await
        .expect("asset detail");
    match detail.asset.extra_json {
        Some(raw) => serde_json::from_str(&raw).expect("extra_json is JSON"),
        None => serde_json::Value::Null,
    }
}

/// Sends one asset out through a `file` dispatch and drives it home:
/// freeze → create → tick the runner → read back what it reified.
///
/// `copy` mode with `emit_metadata` is the shape this whole binary is
/// about — it puts real bytes and a real sidecar in `outbox` for the
/// return leg to pick up.
async fn export_original(
    core: &CoreCtx,
    db_path: &std::path::Path,
    persona_id: &str,
    original_id: &str,
    outbox: &std::path::Path,
) -> ExportRun {
    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona_id.to_string(),
                asset_ids: vec![original_id.to_string()],
            },
            &unattributed(),
        )
        .await
        .expect("freeze snapshot");
    let params_json = serde_json::json!({
        // `resolve_output_dir` refuses anything relative.
        "output_dir": outbox.display().to_string(),
        "mode": "copy",
        "emit_metadata": true,
    })
    .to_string();
    let dispatch = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: snapshot.id,
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    // `create` enqueued a `DispatchRun` job that nothing will ever
    // pick up (no worker in `ReadOnly`); this environment is the only
    // thing that moves the dispatch.
    let (env, reenqueue) = dispatch_env(db_path, core).await;
    let ticks = drive_to_terminal(&env, core, &dispatch.id).await;
    let reenqueued = reenqueue.seen.lock().expect("re-enqueue log").clone();

    let done = core
        .dispatch_service
        .get(&dispatch.id)
        .await
        .expect("dispatch get");
    let copy_id = done
        .output_asset_ids
        .first()
        .cloned()
        .expect("the export reified one copy");
    ExportRun {
        dispatch_id: dispatch.id,
        copy_id,
        ticks,
        reenqueued,
    }
}

/// The whole loop, with nothing standing in for anything: the file
/// exporter writes the sidecar, the file is processed outside the
/// library, and the image importer finds the sidecar and declares
/// where the artefact came from.
#[tokio::test(flavor = "multi_thread")]
async fn an_exported_artefact_comes_back_through_its_own_sidecar() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Three siblings, never nested: an `outbox` under `corpus` would
    // be walked by the corpus scan and the export copy would import
    // itself.
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    let inbox = tmp.path().join("inbox");
    for dir in [&corpus, &outbox, &inbox] {
        std::fs::create_dir_all(dir).expect("scan dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let (core, port) = boot(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-rein-roundtrip".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Leg 1: the original arrives the way every photo does.
    let first = import_png_dir(&corpus, &persona.id, port).await;
    assert_eq!(
        first,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let original_id = asset_id_by_locator(&core, &persona.id, &plate).await;

    // The original went through the same importer as the return leg;
    // the only difference between them is whether a sidecar was
    // sitting next to the file. So the original must carry the
    // importer's own `extra` shape and no provenance claim at all —
    // an origin declared here would land on every photo in a library.
    //
    // Asserted on `_trace.source` rather than on `_trace` itself. The
    // bag is shared: `source` is the provenance channel, and
    // `declared_hash` is the importer's digest claim, which every
    // whole-file scan now carries. Reading the bag's absence as "no
    // provenance" stopped being true the moment a second kind of claim
    // moved in, and the fact this test is for is the provenance one.
    let original_extra = extra_of(&core, &original_id).await;
    assert!(
        original_extra
            .get("_trace")
            .and_then(|trace| trace.get("source"))
            .is_none(),
        "a file with no sidecar beside it claims no origin: {original_extra}"
    );
    // …and the bag is present, so the assertion above is about the
    // absent key rather than about an absent bag.
    assert!(
        original_extra["_trace"]["declared_hash"]["value"]
            .as_str()
            .is_some_and(|declared| declared.starts_with("sha256:")),
        "the importer read the whole file, so it declares its digest: {original_extra}"
    );
    assert_eq!(original_extra["filename"], "plate.png");
    assert!(
        original_extra.get("png_text_keys").is_none(),
        "the importer no longer summarises chunk keywords: a PNG's text \
         is that row's own metadata, read off its bytes by the hash \
         job. It used to assert this key was `[]`: {original_extra}"
    );

    // Out through the exporter.
    let export = export_original(&core, &db_path, &persona.id, &original_id, &outbox).await;

    let exported = outbox.join("plate.png");
    let sidecar_path = outbox.join("plate.png.meta.json");
    assert!(
        exported.is_file(),
        "the default filename template keeps the input's own name"
    );
    assert_eq!(
        std::fs::read(&exported).expect("read export copy"),
        PNG_1X1,
        "copy mode writes the input bytes verbatim"
    );
    assert!(sidecar_path.is_file(), "emit_metadata wrote the sidecar");

    let sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar_path).expect("read sidecar"))
            .expect("sidecar is JSON");
    // The identity block is the part that survives the trip: it names
    // the export, which is what the return leg resolves against.
    let identity = &sidecar[SIDECAR_IDENTITY_KEY];
    assert_eq!(identity["schema"], SIDECAR_SCHEMA);
    assert_eq!(identity["dispatch_id"], export.dispatch_id.as_str());
    assert_eq!(identity["exporter_slug"], "file");
    assert_eq!(identity["source_asset_id"], original_id.as_str());
    // The rest of the body is the input's card, unfiltered.
    assert_eq!(sidecar["id"], original_id.as_str());
    assert_eq!(sidecar["source_locator"], plate.display().to_string());

    // The state machine, with the test as its only driver.
    assert_eq!(
        export.ticks, 2,
        "one tick to send, one to poll and harvest — the file exporter has nothing to wait for"
    );
    assert_eq!(
        export.reenqueued,
        vec![export.dispatch_id.clone()],
        "the runner asked for exactly one more tick"
    );
    let dispatch = core
        .dispatch_service
        .get(&export.dispatch_id)
        .await
        .expect("dispatch get");
    assert_eq!(dispatch.state, "done");
    assert_eq!(dispatch.output_asset_ids.len(), 1, "one input, one copy");
    assert!(
        dispatch.completed_at_ms.is_some(),
        "a terminal dispatch records when it landed"
    );
    assert_eq!(
        asset_id_by_locator(&core, &persona.id, &exported).await,
        export.copy_id,
        "the reified copy is the asset holding the written path"
    );

    // Outside the library: something reads the export, does its work,
    // and writes the result under a new name. The sidecar travels
    // with it — that is the convention the whole loop rests on.
    let returned_file = inbox.join("plate-out.png");
    let mut processed = std::fs::read(&exported).expect("read export copy");
    processed.extend_from_slice(PROCESSING_MARK);
    std::fs::write(&returned_file, &processed).expect("write processed file");
    std::fs::copy(&sidecar_path, inbox.join("plate-out.png.meta.json"))
        .expect("carry the sidecar along");

    // Leg 2: the same importer, the same scan mode, a different
    // directory. Nothing about the call says "this is a return".
    let second = import_png_dir(&inbox, &persona.id, port).await;
    assert_eq!(
        second,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let returned_id = asset_id_by_locator(&core, &persona.id, &returned_file).await;
    assert_ne!(
        std::fs::read(&returned_file).expect("read returned file"),
        std::fs::read(&exported).expect("read export copy"),
        "the artefact that came back is not the one that went out"
    );

    // What the sidecar bought: a resolved claim naming the export.
    let returned_extra = extra_of(&core, &returned_id).await;
    let trace = &returned_extra["_trace"];
    assert_eq!(trace["resolved"], true);
    assert_eq!(
        trace["form"], "sidecar-dispatch",
        "resolved through the identity block, not the card id fallback"
    );
    assert_eq!(
        trace["claim"], "sidecar",
        "the parser declares that it saw one; naming what it named is the server's job"
    );
    assert_eq!(trace["dispatch_id"], export.dispatch_id.as_str());
    assert_eq!(trace["derived_from"], serde_json::json!([export.copy_id]));

    let edges = core
        .asset_service
        .edges_of(&returned_id, Some("derived_from"), 10)
        .await
        .expect("derived_from edges");
    assert_eq!(edges.len(), 1, "one parent: the copy it came out of");
    assert_eq!(edges[0].to_asset_id, export.copy_id);
    assert_eq!(
        edges[0].label.as_deref(),
        Some("correlated-ingest"),
        "a claim accepted from the outside hop, not a dispatch Asterism observed"
    );

    // Read the whole route from the artefact that came back.
    let view = core
        .asset_service
        .lineage_of(&returned_id, None, 8)
        .await
        .expect("lineage");
    assert!(!view.truncated, "three nodes fit well inside the budget");
    assert_eq!(
        view.nodes.len(),
        3,
        "the artefact, the copy it came out of, the original"
    );
    let depth_of = |id: &str| {
        view.nodes
            .iter()
            .find(|n| n.card.id == id)
            .unwrap_or_else(|| panic!("node {id} is in the walk"))
            .depth
    };
    assert_eq!(depth_of(&returned_id), 0);
    assert_eq!(depth_of(&export.copy_id), 1);
    assert_eq!(depth_of(&original_id), 2);
    // Nodes come back in the order the chain happened.
    let depths: Vec<i32> = view.nodes.iter().map(|n| n.depth).collect();
    let mut sorted = depths.clone();
    sorted.sort_unstable();
    assert_eq!(depths, sorted);
    assert_eq!(
        view.roots,
        vec![original_id.clone()],
        "the chain begins at the asset nothing else produced"
    );
    assert_eq!(
        view.dispatch_ids,
        vec![export.dispatch_id.clone()],
        "one export happened, so the backbone names it once"
    );
    assert_eq!(view.edges.len(), 2, "two links between three nodes");

    // `dispatch_id` on a node means "produced by this dispatch", which
    // is only true of the copy. The original was imported; so was the
    // artefact that came back — it *travelled through* an export,
    // which the backbone says and the node must not.
    let dispatch_of = |id: &str| {
        view.nodes
            .iter()
            .find(|n| n.card.id == id)
            .unwrap_or_else(|| panic!("node {id} is in the walk"))
            .dispatch_id
            .clone()
    };
    assert_eq!(dispatch_of(&original_id), None);
    assert_eq!(
        dispatch_of(&export.copy_id),
        Some(export.dispatch_id.clone())
    );
    assert_eq!(dispatch_of(&returned_id), None);

    // The same route read shallowly must not pass for the whole thing.
    let shallow = core
        .asset_service
        .lineage_of(&returned_id, None, 1)
        .await
        .expect("shallow lineage");
    assert_eq!(shallow.nodes.len(), 2, "one hop out from the queried asset");
    assert!(
        shallow.truncated,
        "the original is above and was not reached"
    );
    assert!(
        shallow.roots.is_empty(),
        "nothing reached is a root; the chain demonstrably continues"
    );
}

/// The control: the same file, processed the same way, imported from
/// the same directory — with the sidecar left behind. Adjacency is not
/// provenance, and a library that guessed otherwise would invent a hop
/// for every file that happened to land near an export.
#[tokio::test(flavor = "multi_thread")]
async fn a_return_that_left_its_sidecar_behind_is_just_a_new_artefact() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    let inbox = tmp.path().join("inbox");
    for dir in [&corpus, &outbox, &inbox] {
        std::fs::create_dir_all(dir).expect("scan dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let (core, port) = boot(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-rein-no-sidecar".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let first = import_png_dir(&corpus, &persona.id, port).await;
    assert_eq!(
        first,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let original_id = asset_id_by_locator(&core, &persona.id, &plate).await;

    let export = export_original(&core, &db_path, &persona.id, &original_id, &outbox).await;
    // The export itself is unremarkable — the difference this test is
    // about happens on the way back, not on the way out.
    let dispatch = core
        .dispatch_service
        .get(&export.dispatch_id)
        .await
        .expect("dispatch get");
    assert_eq!(dispatch.state, "done");
    assert_eq!(dispatch.output_asset_ids.len(), 1);

    let exported = outbox.join("plate.png");
    let returned_file = inbox.join("plate-out.png");
    let mut processed = std::fs::read(&exported).expect("read export copy");
    processed.extend_from_slice(PROCESSING_MARK);
    std::fs::write(&returned_file, &processed).expect("write processed file");
    // No `fs::copy` of the sidecar: this is the only line that differs
    // from the happy path.

    let second = import_png_dir(&inbox, &persona.id, port).await;
    assert_eq!(
        second,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let returned_id = asset_id_by_locator(&core, &persona.id, &returned_file).await;

    let returned_extra = extra_of(&core, &returned_id).await;
    // Positive control: the importer's bag did arrive. Without this,
    // a regression that stops `extra_json` from being stored at all
    // would leave every negative assertion below vacuously green.
    assert_eq!(returned_extra["filename"], "plate-out.png");
    // No *provenance* was claimed, so none is recorded — not even a
    // failed claim. Narrowed to `_trace.source` for the reason given at
    // the original's assertion: the digest claim shares the bag and is
    // written for every whole-file scan, so the bag's presence no
    // longer says anything about provenance either way.
    assert!(
        returned_extra
            .get("_trace")
            .and_then(|trace| trace.get("source"))
            .is_none(),
        "the sidecar was left behind, so no origin is recorded — not \
         even a failed claim: {returned_extra}"
    );
    let edges = core
        .asset_service
        .edges_of(&returned_id, Some("derived_from"), 10)
        .await
        .expect("derived_from edges");
    assert!(edges.is_empty(), "no declaration, no link");

    let view = core
        .asset_service
        .lineage_of(&returned_id, None, 8)
        .await
        .expect("lineage");
    assert_eq!(view.nodes.len(), 1, "an artefact with no route is its own");
    assert!(view.dispatch_ids.is_empty());
    assert_eq!(view.roots, vec![returned_id.clone()]);
    assert!(
        !view.truncated,
        "the walk ended because there was nothing above, not because it ran out of budget"
    );
}

/// A sidecar naming an export this library never ran. The file is
/// still imported — refusing it would lose the artefact over a
/// bookkeeping detail — but the claim is recorded as unresolved, with
/// the reason, rather than quietly dropped. A link that is simply
/// missing looks identical to one that was never asked for, and the
/// difference is exactly what someone debugging a chain needs.
#[tokio::test(flavor = "multi_thread")]
async fn a_sidecar_naming_an_export_this_library_never_ran_still_lands_the_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    let inbox = tmp.path().join("inbox");
    for dir in [&corpus, &outbox, &inbox] {
        std::fs::create_dir_all(dir).expect("scan dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let (core, port) = boot(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-rein-unresolved".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let first = import_png_dir(&corpus, &persona.id, port).await;
    assert_eq!(
        first,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let original_id = asset_id_by_locator(&core, &persona.id, &plate).await;

    let export = export_original(&core, &db_path, &persona.id, &original_id, &outbox).await;
    let exported = outbox.join("plate.png");
    let sidecar_path = outbox.join("plate.png.meta.json");

    let returned_file = inbox.join("plate-out.png");
    let mut processed = std::fs::read(&exported).expect("read export copy");
    processed.extend_from_slice(PROCESSING_MARK);
    std::fs::write(&returned_file, &processed).expect("write processed file");

    // The sidecar travels, but naming a dispatch nobody here ran —
    // a file passed between two libraries, or one rewritten by a tool
    // that did not understand what it was copying.
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar_path).expect("read sidecar"))
            .expect("sidecar is JSON");
    let stranger = uuid::Uuid::now_v7().to_string();
    // Fixture guard: if the rewrite ever produced the real id, every
    // "unresolved" assertion below would be testing the happy path.
    assert_ne!(stranger, export.dispatch_id);
    sidecar[SIDECAR_IDENTITY_KEY]["dispatch_id"] = serde_json::json!(stranger);
    std::fs::write(
        inbox.join("plate-out.png.meta.json"),
        serde_json::to_vec_pretty(&sidecar).expect("re-serialise sidecar"),
    )
    .expect("write rewritten sidecar");

    let second = import_png_dir(&inbox, &persona.id, port).await;
    assert_eq!(
        second,
        ImportSummary {
            imported: 1,
            failed: 0
        },
        "an unresolvable claim is not a failed import"
    );
    let returned_id = asset_id_by_locator(&core, &persona.id, &returned_file).await;

    let returned_extra = extra_of(&core, &returned_id).await;
    let trace = &returned_extra["_trace"];
    assert_eq!(trace["resolved"], false);
    assert_eq!(
        trace["derived_from"], "sidecar",
        "an unresolved note keeps the claim verbatim; there are no parents to list"
    );
    assert!(
        trace["reason"]
            .as_str()
            .expect("an unresolved note says why")
            .contains("is not in this library"),
        "the reason names what was looked for and not found, got {:?}",
        trace["reason"]
    );
    assert!(
        trace.get("form").is_none(),
        "form describes how a claim resolved; this one did not"
    );

    let edges = core
        .asset_service
        .edges_of(&returned_id, Some("derived_from"), 10)
        .await
        .expect("derived_from edges");
    assert!(
        edges.is_empty(),
        "a claim that did not resolve writes no link"
    );

    let view = core
        .asset_service
        .lineage_of(&returned_id, None, 8)
        .await
        .expect("lineage");
    assert_eq!(view.nodes.len(), 1);
    // The export the file *actually* came out of (`export.dispatch_id`)
    // is still in the library; the rewritten sidecar is what severed
    // the link, and nothing recovered it behind the artefact's back.
    assert!(
        view.dispatch_ids.is_empty(),
        "an unconfirmed hop stays off the backbone — a ledger is better empty than wrong"
    );
}
