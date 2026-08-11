//! End-to-end proof for correlation ingest (P1).
//!
//! The chain this covers is the one that leaves the application: a file
//! is exported, goes through some outside generator, and comes back as
//! new bytes that carry nothing of their parent. The caller that ran
//! the chain declares the parent on the way back in, and the link has
//! to survive the asynchronous job pipeline that fires right afterwards
//! — `auto_tag` chain-enqueues `edge_rebuild`, and before the write
//! path was scoped to synth kinds that rebuild deleted the declaration
//! as collateral.
//!
//! So the assertion is deliberately two-phase: the edge exists on
//! return *and* is still there once a rebuild has demonstrably run.
//!
//! The unresolvable-claim case lives in
//! `correlation_ingest_unresolved_e2e.rs` — `init_core` opens the
//! profile-global Tantivy index, so one core per test binary.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
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
async fn a_declared_parent_becomes_an_edge_that_outlives_the_rebuild() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    // Shared wording so `auto_tag` produces overlapping keywords and
    // the rebuild has a synth edge to write — that edge is the signal
    // that the rebuild actually ran.
    let source = corpus.join("source.md");
    let returned = corpus.join("returned.md");
    std::fs::write(&source, "# starfield study\nthe original plate\n").expect("write source");
    std::fs::write(&returned, "# starfield study\nthe returned plate\n").expect("write returned");

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
                pack_id: Some("e2e-correlation".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let parent = core
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
        .expect("add parent");

    let child = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned.to_str().unwrap(),
                1_785_000_060_000,
                Some(format!("asset:{}", parent.id)),
            ),
            &unattributed(),
        )
        .await
        .expect("add child");

    // Phase 1: the link is there the moment ingest returns.
    let derived = core
        .asset_service
        .edges_of(&child.id, Some("derived_from"), 10)
        .await
        .expect("edges of child");
    assert_eq!(derived.len(), 1, "one declared parent, one edge");
    assert_eq!(derived[0].to_asset_id, parent.id);
    assert_eq!(derived[0].label.as_deref(), Some("correlated-ingest"));

    // Phase 2: wait for a rebuild to leave its mark (any synth edge on
    // the child), then re-check the declaration. Polling rather than
    // sleeping so a slow machine does not flake.
    let mut rebuilt = false;
    for _ in 0..120 {
        let all = core
            .asset_service
            .edges_of(&child.id, None, 20)
            .await
            .expect("edges of child");
        if all.iter().any(|e| e.kind != "derived_from") {
            rebuilt = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        rebuilt,
        "edge_rebuild never wrote a synth edge, so this run proves nothing about survival"
    );

    let derived_after = core
        .asset_service
        .edges_of(&child.id, Some("derived_from"), 10)
        .await
        .expect("edges of child after rebuild");
    assert_eq!(
        derived_after.len(),
        1,
        "the rebuild recomputes synth edges; the declared parent is not its to delete"
    );
    assert_eq!(derived_after[0].to_asset_id, parent.id);
}
