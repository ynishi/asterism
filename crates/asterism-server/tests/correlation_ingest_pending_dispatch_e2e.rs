//! A dispatch still in flight has no outputs yet.
//!
//! That is "the answer can still change", not "there is no parent".
//! Recording the claim keeps it repairable; resolving it to an empty
//! parent set would look like success and leave nothing to come back
//! to.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, RegisterPersonaCommand,
};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the provenance claim,
/// not about who ingested the row.
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
async fn an_export_that_has_produced_nothing_yet_is_recorded_not_guessed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let source = corpus.join("plate.md");
    let returned = corpus.join("returned.md");
    std::fs::write(&source, "# plate\n").expect("write source");
    std::fs::write(&returned, "# returned\n").expect("write returned");

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
                pack_id: Some("e2e-correlation-pending".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let source_asset = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                source.to_str().unwrap(),
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
                asset_ids: vec![source_asset.id.clone()],
            },
            &unattributed(),
        )
        .await
        .expect("freeze snapshot");

    // Created but never reified — the state a caller is in while the
    // outside service is still working.
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

    let child = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned.to_str().unwrap(),
                1_785_000_100_000,
                Some(format!("dispatch:{}", dispatch.id)),
            ),
            &unattributed(),
        )
        .await
        .expect("the artefact lands regardless");

    assert!(
        core.asset_service
            .edges_of(&child.id, Some("derived_from"), 10)
            .await
            .expect("edges of child")
            .is_empty(),
        "no outputs yet, so nothing to point at"
    );

    let extra: serde_json::Value =
        serde_json::from_str(child.extra_json.as_deref().expect("extra bag")).expect("extra JSON");
    let trace = extra.get("_trace").expect("trace note");
    assert_eq!(trace.get("resolved").and_then(|v| v.as_bool()), Some(false));
    let reason = trace
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("a reason");
    assert!(
        reason.contains("produced no assets yet"),
        "reason should say the export is still in flight: {reason}"
    );
}

/// The other half of "the answer can still change": once the export
/// lands, the recorded claim is retried without anyone asking. The
/// dispatch runner sweeps pending claims after `reify`, so the link
/// appears the moment it becomes true.
#[tokio::test(flavor = "multi_thread")]
async fn a_pending_claim_resolves_itself_once_the_export_lands() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    std::fs::create_dir_all(&outbox).expect("outbox dir");
    let source = corpus.join("plate.md");
    let returned = corpus.join("returned.md");
    std::fs::write(&source, "# plate\n").expect("write source");
    std::fs::write(&returned, "# returned\n").expect("write returned");

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
                pack_id: Some("e2e-correlation-pending-repair".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let source_asset = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                source.to_str().unwrap(),
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
                asset_ids: vec![source_asset.id.clone()],
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

    // The artefact comes back *before* the export has produced
    // anything — recorded, unresolved, no edges.
    let child = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned.to_str().unwrap(),
                1_785_000_100_000,
                Some(format!("dispatch:{}", dispatch.id)),
            ),
            &unattributed(),
        )
        .await
        .expect("the artefact lands regardless");
    assert!(
        core.asset_service
            .edges_of(&child.id, Some("derived_from"), 10)
            .await
            .expect("edges of child")
            .is_empty(),
        "nothing to point at yet"
    );

    // Now the export lands. `reify` writes the outputs and then sweeps
    // pending claims — no separate trigger, no manual repair call.
    let copy = outbox.join("copy.md");
    std::fs::write(&copy, "exported\n").expect("write export copy");
    let dispatch_id = asterism_core::domain::value::DispatchId::from_uuid(
        uuid::Uuid::parse_str(&dispatch.id).expect("dispatch id is a uuid"),
    );
    let job = core
        .support
        .dispatch_runner
        .reify(
            &dispatch_id,
            vec![asterism_contract::dto::DerivedDto {
                modality: "work_product".into(),
                locator: copy.to_string_lossy().into_owned(),
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

    let edges = core
        .asset_service
        .edges_of(&child.id, Some("derived_from"), 10)
        .await
        .expect("edges of child after reify");
    assert_eq!(
        edges.len(),
        1,
        "the claim resolved to the export copy the dispatch produced"
    );
    assert_eq!(edges[0].to_asset_id, export_copy_id);

    let detail = core
        .asset_service
        .detail(asterism_contract::query::GetAssetDetailQuery {
            asset_id: child.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("detail of child");
    let extra: serde_json::Value =
        serde_json::from_str(detail.asset.extra_json.as_deref().expect("extra bag"))
            .expect("extra JSON");
    let trace = extra.get("_trace").expect("trace note");
    assert_eq!(trace.get("resolved").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        trace.get("claim").and_then(|v| v.as_str()),
        Some(format!("dispatch:{}", dispatch.id).as_str()),
        "the verbatim claim survives resolution"
    );
    assert_eq!(
        trace.get("dispatch_id").and_then(|v| v.as_str()),
        Some(dispatch.id.as_str()),
        "the hop's identity is on the survivor, not only on the copy"
    );
    // The arrival channel does not change when the claim resolves:
    // the ingest-payload claim stays `pushed` through the sweep's
    // rewrite of the note.
    assert_eq!(
        trace.get("source").and_then(|v| v.as_str()),
        Some("pushed"),
        "re-resolution carries the recorded channel forward"
    );
}
