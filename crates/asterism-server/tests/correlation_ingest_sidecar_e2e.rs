//! `derived_from: sidecar` — letting the file next to the artefact
//! answer where it came from.
//!
//! This is the path that needs no token handling at all: the exporter
//! wrote `<name>.meta.json` beside the payload, the whole directory
//! went out and came back, and the ingest side reads the sidecar.
//!
//! Preference order matters and is asserted here: the identity block's
//! `dispatch_id` names the *export* the file travelled through, so it
//! wins over the card's `id`, which names the original one hop further
//! up. Both cases resolve — `_trace.form` records which one did.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, RegisterPersonaCommand,
};
use asterism_contract::dto::DerivedDto;
use asterism_contract::sidecar::{SIDECAR_IDENTITY_KEY, SIDECAR_SCHEMA, SIDECAR_SUFFIX};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the sidecar claim, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

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

#[tokio::test(flavor = "multi_thread")]
async fn a_sidecar_links_the_return_through_the_export_it_names() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let plate = corpus.join("plate.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");

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
                pack_id: Some("e2e-correlation-sidecar".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let source = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                plate.to_str().unwrap(),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add source");

    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![source.id.clone()],
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
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    // The exported copy, reified the way the runner does it.
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&outbox).expect("outbox");
    let exported = outbox.join("plate.md");
    std::fs::write(&exported, "exported\n").expect("write export");
    let dispatch_id = asterism_core::domain::value::DispatchId::from_uuid(
        uuid::Uuid::parse_str(&dispatch.id).expect("dispatch id is a uuid"),
    );
    let job = core
        .support
        .dispatch_runner
        .reify(
            &dispatch_id,
            vec![DerivedDto {
                modality: "work_product".into(),
                locator: exported.to_string_lossy().into_owned(),
                occurred_at: chrono::Utc::now(),
                cover_hint: None,
                register_note: None,
                labels: Vec::new(),
                file_size_bytes: None,
                duration_ms: None,
                extra: serde_json::Value::Null,
                batch_hint: None,
            }],
        )
        .await
        .expect("reify");
    let export_copy_id = job.output_asset_ids[0].to_string();

    // The return leg: a new file plus the sidecar that travelled with
    // it, naming both the export and (as fallback) the original.
    let inbox = tmp.path().join("inbox");
    std::fs::create_dir_all(&inbox).expect("inbox");
    let returned = inbox.join("returned.md");
    std::fs::write(&returned, "# returned\n").expect("write returned");
    let sidecar = serde_json::json!({
        "id": source.id,
        "source_locator": plate.to_string_lossy(),
        SIDECAR_IDENTITY_KEY: {
            "schema": SIDECAR_SCHEMA,
            "dispatch_id": dispatch.id,
            "exporter_slug": "file",
            "source_asset_id": source.id,
        }
    });
    std::fs::write(
        format!("{}{}", returned.display(), SIDECAR_SUFFIX),
        serde_json::to_vec_pretty(&sidecar).unwrap(),
    )
    .expect("write sidecar");

    let child = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned.to_str().unwrap(),
                1_785_000_100_000,
                Some("sidecar".into()),
            ),
            &unattributed(),
        )
        .await
        .expect("add returned artefact");

    let edges = core
        .asset_service
        .edges_of(&child.id, Some("derived_from"), 10)
        .await
        .expect("edges of child");
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].to_asset_id, export_copy_id,
        "the export it travelled through, not the original one hop further up"
    );

    let extra: serde_json::Value =
        serde_json::from_str(child.extra_json.as_deref().expect("extra bag")).expect("extra JSON");
    assert_eq!(
        extra
            .get("_trace")
            .and_then(|t| t.get("form"))
            .and_then(|v| v.as_str()),
        Some("sidecar-dispatch")
    );
    // Channel bookkeeping: an ingest-time `sidecar` claim is the
    // importer reporting what it found next to the file — `embedded`,
    // as opposed to `pushed` (caller payload) and `manual` (repair
    // verb after the fact).
    assert_eq!(
        extra
            .get("_trace")
            .and_then(|t| t.get("source"))
            .and_then(|v| v.as_str()),
        Some("embedded"),
        "a detected sidecar claim is recorded as embedded"
    );
}
