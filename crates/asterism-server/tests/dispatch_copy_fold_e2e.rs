//! The reason the fingerprint enqueue and the exclusion rules shipped
//! together: an export in copy mode produces an asset whose bytes are
//! its own input's, and nothing else stops that pair from being folded.
//!
//! Each half is harmless alone. The exclusions with no enqueue behind
//! them are rules waiting for a fingerprint that only arrives on the
//! next restart's backfill walk. The
//! enqueue with no exclusions is the accident: `on_duplicate = fold`,
//! a digest that lands seconds after the export, and the copy is
//! folded into the original — taking with it the `derived_from` edge
//! that said an export happened at all.
//!
//! `dispatch_rein_roundtrip_e2e` already pins the precondition ("copy
//! mode writes the input bytes verbatim"). This binary is what that
//! fact means once the two rows are fingerprinted.
//!
//! # What is real here, and what stands in
//!
//! Real: the `file` exporter, the `DispatchRun` state machine, `reify`,
//! the SQLite repositories, and `detect_duplicate` — the same function
//! the `material_hash` handler calls.
//!
//! Standing in: the job worker. `CoreMode::ReadOnly` opens the queue
//! without a `Monitor`, which is what makes the enqueue observable (a
//! recorded push rather than a race with something draining it) and
//! leaves the test to do what the handler would have done next — write
//! the digest, then ask what it means. Both steps are spelled out below
//! rather than hidden in a helper, because a reader has to be able to
//! see that the second one is the production call.
//!
//! Its own test binary because `init_core` opens a Tantivy index, as
//! with the sibling e2e files.

use std::sync::{Arc, Mutex};

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, RegisterPersonaCommand,
};
use asterism_core::application_support::DispatchRunnerService;
use asterism_core::application_support::duplicate_detection::{
    Detection, DetectionOrigin, DetectionPorts, detect_duplicate,
};
use asterism_core::domain::content_hash::of_bytes;
use asterism_core::domain::duplicate_conflict::FoldExclusion;
use asterism_core::domain::edge::EdgeKind;
use asterism_core::domain::job::JobKind;
use asterism_core::domain::repository::{AssetRepository, EdgeRepository, JobQueue};
use asterism_core::domain::value::{AssetId, DispatchId, OnDuplicate, PersonaId, SourceKind};
use asterism_core::error::DomainError;
use asterism_exporter_file::FileExporter;
use asterism_infra::dispatch::{DispatchRunEnv, ExporterRegistry, ReEnqueue, run_dispatch_run};
use asterism_infra::sqlite;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// A 1×1 RGBA PNG, 67 bytes — the same minimal fixture the round-trip
/// binary uses. Nothing decodes it here; what matters is that the
/// exporter copies it byte for byte.
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

/// A queue that records instead of running. Both things this test
/// measures are pushes: the fingerprint `reify` asks for, and the fold
/// the detector must not ask for.
#[derive(Default)]
struct RecordingQueue {
    pushed: Mutex<Vec<(JobKind, serde_json::Value)>>,
}

#[async_trait::async_trait]
impl JobQueue for RecordingQueue {
    async fn enqueue(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
    ) -> Result<String, DomainError> {
        self.pushed.lock().expect("queue log").push((kind, payload));
        Ok("task-1".into())
    }

    async fn enqueue_with_priority(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
        _priority: i32,
    ) -> Result<String, DomainError> {
        self.enqueue(kind, payload).await
    }

    async fn has_pending_batch(&self, _kind: JobKind) -> Result<bool, DomainError> {
        Ok(false)
    }
}

impl RecordingQueue {
    fn of_kind(&self, wanted: JobKind) -> Vec<serde_json::Value> {
        self.pushed
            .lock()
            .expect("queue log")
            .iter()
            .filter(|(kind, _)| *kind == wanted)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

/// [`ReEnqueue`] fake — the test is the poll loop, so the next tick is
/// recorded rather than queued.
#[derive(Default)]
struct RecordingReEnqueue;

#[async_trait::async_trait]
impl ReEnqueue for RecordingReEnqueue {
    async fn reenqueue(&self, _dispatch_id: &DispatchId) -> Result<(), DomainError> {
        Ok(())
    }
}

/// The repositories this test drives directly, on their own isle over
/// the same database file (WAL, `busy_timeout`; every call below is
/// awaited in sequence, so no read races a write it depends on).
struct Ports {
    assets: Arc<sqlite::repo::SqliteAssetRepository>,
    edges: Arc<sqlite::repo::SqliteEdgeRepository>,
}

async fn boot(tmp: &std::path::Path) -> CoreCtx {
    init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        // No worker: the enqueue stays observable and nothing drains
        // the dispatch out from under the test.
        CoreMode::ReadOnly,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core")
}

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. This test is about what an export
/// does to the fold rules, not about who ran it.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str, occurred_at_ms: i64) -> AddAssetCommand {
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
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exported_copy_is_not_folded_into_the_input_it_copied() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    for dir in [&corpus, &outbox] {
        std::fs::create_dir_all(dir).expect("scan dir");
    }
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let db_path = tmp.path().join("asterism.db");
    let core = boot(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-copy-fold".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let persona_id = PersonaId::from_uuid(persona.id.parse().expect("persona uuid"));

    let original = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().expect("utf-8 path"),
                1_785_000_000_000,
            ),
            &unattributed(),
        )
        .await
        .expect("add the original");
    let original_id = AssetId::from_uuid(original.id.parse().expect("asset uuid"));

    // ---- the export -------------------------------------------------

    let (isle, driver) = sqlite::open_and_migrate(&db_path)
        .await
        .expect("second isle");
    // Dropping the driver joins the SQLite thread and takes the isle
    // with it, so it has to outlive every call below. One per test,
    // reclaimed when the process exits — the same arrangement
    // `dispatch_rein_roundtrip_e2e` documents.
    std::mem::forget(driver);
    let ports = Ports {
        assets: Arc::new(sqlite::repo::SqliteAssetRepository::new(isle.clone())),
        edges: Arc::new(sqlite::repo::SqliteEdgeRepository::new(isle.clone())),
    };

    let queue = Arc::new(RecordingQueue::default());
    // The runner the composition root builds, with its queue swapped
    // for one that records. Everything else is what `core_init` wires.
    let runner = Arc::new(DispatchRunnerService::new(
        Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        ports.assets.clone(),
        ports.edges.clone(),
        Arc::new(sqlite::repo::SqlitePersonaRepository::new(isle.clone())),
        core.asset_service.clone(),
        queue.clone(),
    ));
    let env = DispatchRunEnv {
        registry: ExporterRegistry::single(Arc::new(FileExporter::new())),
        service: runner,
        snapshots: Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        dispatches: Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        assets: ports.assets.clone(),
        reenqueue: Arc::new(RecordingReEnqueue),
    };

    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![original.id.clone()],
            },
            &unattributed(),
        )
        .await
        .expect("freeze snapshot");
    let dispatch = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: snapshot.id,
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: serde_json::json!({
                    "output_dir": outbox.display().to_string(),
                    "mode": "copy",
                    "emit_metadata": false,
                })
                .to_string(),
                operator_ai: None,
                pursuit_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    let payload = serde_json::json!({ "dispatch_id": dispatch.id });
    let mut ticks = 0;
    loop {
        run_dispatch_run(&env, &payload)
            .await
            .expect("dispatch tick");
        ticks += 1;
        let dto = core
            .dispatch_service
            .get(&dispatch.id)
            .await
            .expect("dispatch get");
        if matches!(dto.state.as_str(), "done" | "failed" | "cancelled") {
            assert_eq!(dto.state, "done", "the export ran");
            break;
        }
        assert!(ticks < 8, "dispatch did not reach a terminal state");
    }
    let done = core
        .dispatch_service
        .get(&dispatch.id)
        .await
        .expect("dispatch get");
    let copy_id_str = done
        .output_asset_ids
        .first()
        .cloned()
        .expect("the export reified one copy");
    let copy_id = AssetId::from_uuid(copy_id_str.parse().expect("asset uuid"));

    // ---- half one: the export asks for its output to be hashed ------
    //
    // Without this the copy has no fingerprint until a restart's
    // backfill walk reaches it, which is the gap the exclusions on
    // their own would leave open.
    assert_eq!(
        queue.of_kind(JobKind::MaterialHash),
        vec![serde_json::json!({ "asset_id": copy_id_str })],
        "reify enqueued exactly one fingerprint, for the row it minted"
    );
    // And it asked after writing the lineage, not before: a worker that
    // picked the job up the instant it was pushed has to be able to see
    // where the copy came from, or the rule below has nothing to read.
    let lineage = ports
        .edges
        .edges_incident(&copy_id, Some(EdgeKind::DerivedFrom), 10)
        .await
        .expect("provenance edges");
    assert_eq!(lineage.len(), 1, "the copy descends from its input");
    assert_eq!(lineage[0].edge.to, original_id);

    // ---- what the worker would do next ------------------------------

    let exported = outbox.join("plate.png");
    let original_bytes = std::fs::read(&plate).expect("read the input");
    let exported_bytes = std::fs::read(&exported).expect("read the copy");
    assert_eq!(
        exported_bytes, original_bytes,
        "copy mode writes the input bytes verbatim — the premise of this whole file"
    );
    let digest = of_bytes(&exported_bytes);

    // The lane says fold.
    //
    // In SQL, and both halves of that are deliberate. The layer that
    // would declare this — an importer / lane setting, the middle rung
    // of the resolution ladder — is unimplemented, and a
    // dispatch's outputs have no other way to carry a strategy. Nor
    // would `save` write it: `on_duplicate` is a kept column, so a
    // whole-row save leaves the registration's own declaration alone
    // (that is what stops a metadata round trip from inventing one).
    // Same shape as the `fold_policy` fixture in the detection tests:
    // the column is real, the surface that writes it is a later wave.
    let copy_uuid = *copy_id.as_uuid();
    isle.call(move |conn| {
        let touched = conn.execute(
            "UPDATE asset SET on_duplicate = 'fold' WHERE id = ?1",
            rusqlite::params![copy_uuid],
        )?;
        assert_eq!(touched, 1, "the lane setting landed on a row that exists");
        Ok(())
    })
    .await
    .expect("declare the lane strategy");

    // The fingerprints the `material_hash` job would have written —
    // every axis, as one write, the way the job writes them. The two
    // walking axes take a marker: this fixture is about the artefact
    // axis, and a marker is not a duplicate key, so neither of the
    // others can answer for it.
    let fingerprint = asterism_core::domain::repository::MaterialFingerprint {
        file: digest.clone(),
        content: asterism_core::domain::content_region::EMPTY_SPAN.to_string(),
        meta: asterism_core::domain::content_region::EMPTY_SPAN.to_string(),
        meta_kv: None,
        meta_text: None,
        meta_raw: None,
    };
    for id in [&original_id, &copy_id] {
        ports
            .assets
            .set_material_fingerprint(id, 0, &fingerprint)
            .await
            .expect("write the digest");
    }
    let holders = ports
        .assets
        .find_by_content_hash(
            &persona_id,
            asterism_core::domain::duplicate_conflict::DuplicateAxis::Artefact,
            &digest,
        )
        .await
        .expect("hash lookup");
    assert_eq!(
        holders.len(),
        2,
        "the input and its copy really do collide — without this the rest is vacuous"
    );
    assert_eq!(holders[0].id, original_id, "the input is the older row");
    assert_eq!(
        holders[1].source.kind.as_str(),
        SourceKind::for_dispatch("file").expect("slug").as_str(),
        "the younger row is the export's product"
    );
    assert_eq!(
        holders[1].on_duplicate,
        Some(OnDuplicate::Fold),
        "and its lane asked for a fold without confirmation"
    );

    // ---- half two: the fold is declined, with a reason --------------

    let outcome = detect_duplicate(
        DetectionPorts {
            assets: ports.assets.as_ref(),
            edges: ports.edges.as_ref(),
            queue: queue.as_ref(),
        },
        &copy_id,
        0,
        &fingerprint,
        DetectionOrigin::Ingest,
        chrono::Utc::now(),
    )
    .await
    .expect("detection ran");

    assert_eq!(
        outcome,
        Detection::Queued(original_id),
        "a lane that said fold, a pair that must not be folded by a machine"
    );
    assert!(
        queue.of_kind(JobKind::AssetFold).is_empty(),
        "nothing was enqueued to fold the export into its own input"
    );

    let open = ports
        .assets
        .list_open_duplicate_conflicts(Some(&persona_id), 10)
        .await
        .expect("the conflict queue");
    assert_eq!(open.len(), 1, "one pair, one question");
    assert_eq!(
        (open[0].newcomer, open[0].incumbent),
        (copy_id, original_id)
    );
    assert_eq!(
        open[0].fold_exclusion,
        Some(FoldExclusion::Dispatch),
        "and the panel is told why it is being asked rather than told"
    );

    // What a fold would have cost: the copy is still a row of its own,
    // and the edge saying an export produced it is still there.
    let copy_row = ports
        .assets
        .find(&copy_id)
        .await
        .expect("read the copy")
        .expect("the copy is still a row");
    assert!(
        copy_row.folded_into.is_none(),
        "the export's output is not a headstone"
    );
    let lineage = ports
        .edges
        .edges_incident(&copy_id, Some(EdgeKind::DerivedFrom), 10)
        .await
        .expect("provenance edges");
    assert_eq!(
        lineage.len(),
        1,
        "the record that this artefact came out of an export survives"
    );
    assert_eq!(lineage[0].edge.to, original_id);
}
