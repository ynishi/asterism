//! `derived_from: dispatch:<id>` — naming a whole export as the parent.
//!
//! The token a caller most plausibly still holds after a round trip is
//! not an asset id but "the export I ran": one dispatch, N files
//! handed to some outside service. So the dispatch id resolves to the
//! assets that dispatch produced, and every one of them becomes a
//! parent — the same shape `reify` already writes for an N-member
//! snapshot.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files). The in-flight-export case lives in
//! `correlation_ingest_pending_dispatch_e2e.rs` for the same reason.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, RegisterPersonaCommand,
};
use asterism_contract::dto::DerivedDto;
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the dispatch claim,
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
async fn naming_the_export_links_the_return_to_every_artefact_it_produced() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let plate_a = corpus.join("plate-a.md");
    let plate_b = corpus.join("plate-b.md");
    let returned = corpus.join("returned.md");
    std::fs::write(&plate_a, "# plate a\n").expect("write a");
    std::fs::write(&plate_b, "# plate b\n").expect("write b");
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
                pack_id: Some("e2e-correlation-dispatch".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Two originals, frozen into a snapshot and sent out as one export.
    let mut source_ids = Vec::new();
    for (index, path) in [&plate_a, &plate_b].into_iter().enumerate() {
        let dto = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    1_785_000_000_000 + index as i64 * 1_000,
                    None,
                ),
                &unattributed(),
            )
            .await
            .expect("add source");
        source_ids.push(dto.id);
    }

    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: source_ids.clone(),
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
                pursuit_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    // Stand in for the exporter's harvest: two exported copies, which
    // the runner reifies into assets carrying the dispatch id.
    let exported_dir = tmp.path().join("outbox");
    std::fs::create_dir_all(&exported_dir).expect("outbox");
    let derived: Vec<DerivedDto> = ["plate-a.md", "plate-b.md"]
        .into_iter()
        .map(|name| {
            let target = exported_dir.join(name);
            std::fs::write(&target, "exported\n").expect("write export");
            DerivedDto {
                modality: "work_product".into(),
                locator: target.to_string_lossy().into_owned(),
                occurred_at: chrono::Utc::now(),
                cover_hint: None,
                register_note: None,
                labels: Vec::new(),
                file_size_bytes: None,
                duration_ms: None,
                extra: serde_json::Value::Null,
                batch_hint: None,
            }
        })
        .collect();
    let dispatch_id = asterism_core::domain::value::DispatchId::from_uuid(
        uuid::Uuid::parse_str(&dispatch.id).expect("dispatch id is a uuid"),
    );
    let job = core
        .support
        .dispatch_runner
        .reify(&dispatch_id, derived)
        .await
        .expect("reify");
    assert_eq!(job.output_asset_ids.len(), 2, "two exported copies");

    // The return trip: one file back, declared against the export.
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
        .expect("add returned artefact");

    let edges = core
        .asset_service
        .edges_of(&child.id, Some("derived_from"), 10)
        .await
        .expect("edges of child");
    let parents: std::collections::HashSet<String> =
        edges.iter().map(|e| e.to_asset_id.clone()).collect();
    let expected: std::collections::HashSet<String> = job
        .output_asset_ids
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        parents, expected,
        "every artefact the export produced is a parent"
    );

    // The claim is recorded with the form that resolved it, so a
    // reader can tell a directly-named parent from a whole-export one.
    let extra: serde_json::Value =
        serde_json::from_str(child.extra_json.as_deref().expect("extra bag")).expect("extra JSON");
    let trace = extra.get("_trace").expect("trace note");
    assert_eq!(trace.get("resolved").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(trace.get("form").and_then(|v| v.as_str()), Some("dispatch"));
}
