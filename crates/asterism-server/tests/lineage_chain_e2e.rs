//! A three-hop chain, read back in one request.
//!
//! This is what the whole feature is for: an artefact goes out to a
//! generator, comes back, goes out again, comes back once more. Four
//! assets, three hops, two of which happened outside the application
//! and are only known because someone declared them.
//!
//! The assertions are about what a reader needs from that: every node
//! with its distance, the links between them, where the chain begins,
//! and the exports it passed through in order.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, PurgeAssetCommand,
    RegisterPersonaCommand, TrashAssetCommand,
};
use asterism_contract::dto::DerivedDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the derivation chain,
/// not about who ingested each hop.
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

/// Sends one asset out through a `file` dispatch and reifies one copy
/// per `name`, returning `(dispatch_id, export_copy_asset_ids)` — the
/// ids a caller holds after an export.
async fn export_through_file(
    core: &CoreCtx,
    persona_id: &str,
    source_asset_id: &str,
    outbox: &std::path::Path,
    names: &[&str],
) -> (String, Vec<String>) {
    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona_id.to_string(),
                asset_ids: vec![source_asset_id.to_string()],
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
                params_json: String::new(),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    let mut derived = Vec::with_capacity(names.len());
    for name in names {
        let target = outbox.join(name);
        std::fs::write(&target, "exported\n").expect("write export");
        derived.push(DerivedDto {
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
        });
    }
    let dispatch_id = asterism_core::domain::value::DispatchId::from_uuid(
        uuid::Uuid::parse_str(&dispatch.id).expect("dispatch id is a uuid"),
    );
    // Stands in for the `DispatchRun` runner: `reify` is a support
    // service (`CoreCtx.support`), not something a transport can reach.
    let job = core
        .support
        .dispatch_runner
        .reify(&dispatch_id, derived)
        .await
        .expect("reify");
    (
        dispatch.id,
        job.output_asset_ids
            .iter()
            .map(|id| id.to_string())
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_three_hop_chain_reads_back_as_one_route() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    std::fs::create_dir_all(&outbox).expect("outbox dir");
    let plate = corpus.join("plate.md");
    let first_return = corpus.join("return-1.md");
    let second_return = corpus.join("return-2.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");
    std::fs::write(&first_return, "# return 1\n").expect("write return 1");
    std::fs::write(&second_return, "# return 2\n").expect("write return 2");

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
                pack_id: Some("e2e-lineage-chain".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Hop 0: the original. It carries a `bundle_id` because that is
    // what the real ingest path does — the image importer sets one to
    // group a PNG with the notes extracted from its tEXt chunks. A
    // bundle is not a dispatch, so this asset must report no dispatch
    // at all; reading `bundle_id` as one invents a hop that never
    // happened (found by dogfooding, 2026-07-29).
    let mut original_command = add_command(
        &persona.id,
        plate.to_str().unwrap(),
        1_785_000_000_000,
        None,
    );
    original_command.bundle_id = Some("bundle-from-the-importer".into());
    let original = core
        .asset_service
        .add(original_command, &unattributed())
        .await
        .expect("add original");

    // Hop 1: out and back.
    let (dispatch_a, exports_a) =
        export_through_file(&core, &persona.id, &original.id, &outbox, &["hop-a.md"]).await;
    let export_a = exports_a[0].clone();
    let returned_a = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                first_return.to_str().unwrap(),
                1_785_000_100_000,
                Some(format!("dispatch:{dispatch_a}")),
            ),
            &unattributed(),
        )
        .await
        .expect("add first return");

    // Hop 2: out again, from what came back.
    let (dispatch_b, exports_b) =
        export_through_file(&core, &persona.id, &returned_a.id, &outbox, &["hop-b.md"]).await;
    let export_b = exports_b[0].clone();
    let returned_b = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                second_return.to_str().unwrap(),
                1_785_000_200_000,
                Some(format!("dispatch:{dispatch_b}")),
            ),
            &unattributed(),
        )
        .await
        .expect("add second return");

    // Read the chain from its far end.
    let view = core
        .asset_service
        .lineage_of(&returned_b.id, None, 8)
        .await
        .expect("lineage");

    assert!(!view.truncated, "a five-node chain fits well inside budget");
    assert_eq!(view.asset_id, returned_b.id);

    let depth_of = |id: &str| {
        view.nodes
            .iter()
            .find(|n| n.card.id == id)
            .unwrap_or_else(|| panic!("node {id} is in the walk"))
            .depth
    };
    // returned_b → export_b → returned_a → export_a → original.
    assert_eq!(depth_of(&returned_b.id), 0);
    assert_eq!(depth_of(&export_b), 1);
    assert_eq!(depth_of(&returned_a.id), 2);
    assert_eq!(depth_of(&export_a), 3);
    assert_eq!(depth_of(&original.id), 4);
    assert_eq!(view.nodes.len(), 5, "five assets, no strays");

    // Nodes come back in the order the chain happened.
    let depths: Vec<i32> = view.nodes.iter().map(|n| n.depth).collect();
    let mut sorted = depths.clone();
    sorted.sort_unstable();
    assert_eq!(depths, sorted);

    assert_eq!(
        view.roots,
        vec![original.id.clone()],
        "the chain begins at the asset nothing else produced"
    );
    // The backbone: which exports this artefact travelled through,
    // nearest hop first. Two exports happened, so two entries — an
    // asset that merely carries a `bundle_id` must not add a third.
    assert_eq!(
        view.dispatch_ids,
        vec![dispatch_b.clone(), dispatch_a.clone()],
        "one entry per export that actually ran"
    );
    assert_eq!(
        view.nodes
            .iter()
            .find(|n| n.card.id == original.id)
            .expect("the original is in the walk")
            .dispatch_id,
        None,
        "the original was imported, not produced by a dispatch"
    );
    assert_eq!(view.edges.len(), 4, "four links between five nodes");

    // The same chain read shallowly must not pass for the whole
    // thing. A depth-limited walk that stayed silent about stopping
    // is the difference between a short story and a wrong one.
    let shallow = core
        .asset_service
        .lineage_of(&returned_b.id, None, 1)
        .await
        .expect("shallow lineage");
    assert_eq!(shallow.nodes.len(), 2, "one hop out from the queried asset");
    assert!(
        shallow.truncated,
        "three more hops exist above and were not reached"
    );
    assert!(
        shallow.roots.is_empty(),
        "nothing reached is a root; the chain demonstrably continues"
    );
}

/// An export that produced three copies is still *one* export. Before
/// the backbone deduplicated, every copy's `_dispatch` stamp pushed
/// the same dispatch id, so a fan-out read back as three hops.
#[tokio::test(flavor = "multi_thread")]
async fn an_n_output_export_names_its_dispatch_once_in_the_backbone() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    std::fs::create_dir_all(&outbox).expect("outbox dir");
    let plate = corpus.join("plate.md");
    let returned_file = corpus.join("return.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");
    std::fs::write(&returned_file, "# return\n").expect("write return");

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
                pack_id: Some("e2e-lineage-fanout".into()),
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
                plate.to_str().unwrap(),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let (dispatch, exports) = export_through_file(
        &core,
        &persona.id,
        &original.id,
        &outbox,
        &["fan-1.md", "fan-2.md", "fan-3.md"],
    )
    .await;
    assert_eq!(exports.len(), 3, "the export fanned out into three copies");

    let returned = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned_file.to_str().unwrap(),
                1_785_000_100_000,
                Some(format!("dispatch:{dispatch}")),
            ),
            &unattributed(),
        )
        .await
        .expect("add returned");

    let view = core
        .asset_service
        .lineage_of(&returned.id, None, 8)
        .await
        .expect("lineage");

    // returned + three copies + the original.
    assert_eq!(view.nodes.len(), 5, "the whole fan is in the walk");
    assert_eq!(
        view.dispatch_ids,
        vec![dispatch],
        "three copies, one export, one backbone entry"
    );
}

/// Two exports of the same original are two routes. Reading the
/// lineage of an artefact that came back through one of them must not
/// surface the other export's copy — that is a sibling, not a hop —
/// and the backbone must name only the dispatch actually travelled.
/// (Found by dogfooding, 2026-08-01: the walk stepped from the
/// original back down to its other child, so the sibling copy showed
/// up at depth 1 and its dispatch contaminated the backbone.)
#[tokio::test(flavor = "multi_thread")]
async fn a_sibling_export_stays_out_of_the_route() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    std::fs::create_dir_all(&outbox).expect("outbox dir");
    let plate = corpus.join("plate.md");
    let returned_file = corpus.join("return.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");
    std::fs::write(&returned_file, "# return\n").expect("write return");

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
                pack_id: Some("e2e-lineage-sibling".into()),
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
                plate.to_str().unwrap(),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let (sibling_dispatch, sibling_exports) =
        export_through_file(&core, &persona.id, &original.id, &outbox, &["sibling.md"]).await;
    let (dispatch, exports) =
        export_through_file(&core, &persona.id, &original.id, &outbox, &["travelled.md"]).await;
    let returned = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned_file.to_str().unwrap(),
                1_785_000_100_000,
                Some(format!("dispatch:{dispatch}")),
            ),
            &unattributed(),
        )
        .await
        .expect("add returned");

    let view = core
        .asset_service
        .lineage_of(&returned.id, None, 8)
        .await
        .expect("lineage");

    assert_eq!(
        view.nodes.len(),
        3,
        "returned → its copy → the original; the sibling copy is not a hop"
    );
    assert!(
        view.nodes.iter().all(|n| n.card.id != sibling_exports[0]),
        "the other export's copy stays out of the walk"
    );
    let depth_of = |id: &str| {
        view.nodes
            .iter()
            .find(|n| n.card.id == id)
            .unwrap_or_else(|| panic!("node {id} is in the walk"))
            .depth
    };
    assert_eq!(depth_of(&returned.id), 0);
    assert_eq!(depth_of(&exports[0]), 1);
    assert_eq!(depth_of(&original.id), 2);
    assert_eq!(
        view.dispatch_ids,
        vec![dispatch],
        "only the export actually travelled is on the backbone"
    );
    assert!(
        !view.dispatch_ids.contains(&sibling_dispatch),
        "an export the artefact never passed through is not a hop"
    );
    assert_eq!(view.roots, vec![original.id.clone()]);
    assert_eq!(view.edges.len(), 2, "two links between three nodes");
}

/// Export copies are working files; sooner or later the user throws
/// them away. The route the artefact took must not go with them — the
/// resolved claim recorded on the artefact itself is the copy of the
/// hop's identity that stays.
#[tokio::test(flavor = "multi_thread")]
async fn the_backbone_outlives_its_export_copies() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    std::fs::create_dir_all(&outbox).expect("outbox dir");
    let plate = corpus.join("plate.md");
    let returned_file = corpus.join("return.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");
    std::fs::write(&returned_file, "# return\n").expect("write return");

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
                pack_id: Some("e2e-lineage-purge".into()),
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
                plate.to_str().unwrap(),
                1_785_000_000_000,
                None,
            ),
            &unattributed(),
        )
        .await
        .expect("add original");

    let (dispatch, exports) =
        export_through_file(&core, &persona.id, &original.id, &outbox, &["copy.md"]).await;
    let returned = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned_file.to_str().unwrap(),
                1_785_000_100_000,
                Some(format!("dispatch:{dispatch}")),
            ),
            &unattributed(),
        )
        .await
        .expect("add returned");

    // Throw the export copy away for good. `edge` rows cascade with
    // the asset, so this also severs the walkable chain.
    core.asset_service
        .trash(
            TrashAssetCommand {
                asset_id: exports[0].clone(),
                comment: None,
            },
            &unattributed(),
        )
        .await
        .expect("trash the copy");
    core.asset_service
        .purge(
            PurgeAssetCommand {
                asset_id: exports[0].clone(),
            },
            &unattributed(),
        )
        .await
        .expect("purge the copy");

    let view = core
        .asset_service
        .lineage_of(&returned.id, None, 8)
        .await
        .expect("lineage");

    assert_eq!(
        view.nodes.len(),
        1,
        "the copy and its edges are gone; only the survivor remains"
    );
    assert_eq!(
        view.dispatch_ids,
        vec![dispatch],
        "the resolved claim on the survivor still names the export it came through"
    );
}
