//! What a provenance claim *asserts*, end to end.
//!
//! `correlation_ingest_e2e.rs` covers the claim surviving the pipeline.
//! This one covers the sentence it makes. Until now every claim meant
//! `derived_from` — one artefact came out of another, an assertion
//! nothing in the corpus can contradict afterwards — because that was
//! the only edge the verb could write. A person who worked from two
//! references has said something weaker and true, and the only ways to
//! record it were to overstate it or to lose it.
//!
//! So the assertions here are about *which* edge appears and, just as
//! much, which one does not: a `reference` claim that also wrote a
//! `derived_from` edge would be the overstatement this exists to stop,
//! and the graph would not say so anywhere.
//!
//! `ReadOnly` rather than `Full`: the question is what one synchronous
//! verb writes, and the job worker would only add edges from a rebuild
//! that has nothing to do with it. Survival across a rebuild is the
//! neighbouring file's subject, and `EdgeKind::Reference` is already on
//! the non-synth side of `is_synth()` with the kind it shares that
//! answer with.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, DeclareProvenanceCommand, RegisterPersonaCommand,
};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

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

/// A persona and three assets in one temp profile: one that will do the
/// claiming and two it can name.
struct Fixture {
    core: asterism_server::core_init::CoreCtx,
    child: String,
    parent_a: String,
    parent_b: String,
    _tmp: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-provenance-relation".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for (index, name) in ["child", "parent-a", "parent-b"].iter().enumerate() {
        let path = corpus.join(format!("{name}.md"));
        std::fs::write(&path, format!("# {name}\n")).expect("write file");
        let asset = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    1_785_000_000_000 + index as i64 * 60_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(asset.id);
    }

    Fixture {
        core,
        child: ids[0].clone(),
        parent_a: ids[1].clone(),
        parent_b: ids[2].clone(),
        _tmp: tmp,
    }
}

fn declare(asset_id: &str, parent_id: &str, relation: Option<&str>) -> DeclareProvenanceCommand {
    DeclareProvenanceCommand {
        asset_id: asset_id.to_string(),
        derived_from: format!("asset:{parent_id}"),
        relation: relation.map(str::to_string),
        operator_ai: None,
    }
}

/// Edge kinds on `asset_id`, sorted, so an assertion can name the whole
/// set rather than one member of it.
async fn edge_kinds(core: &asterism_server::core_init::CoreCtx, asset_id: &str) -> Vec<String> {
    let mut kinds: Vec<String> = core
        .asset_service
        .edges_of(asset_id, None, 20)
        .await
        .expect("edges of asset")
        .into_iter()
        .map(|e| e.kind)
        .collect();
    kinds.sort();
    kinds
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reference_claim_writes_a_reference_edge_and_only_that() {
    let fx = fixture().await;
    fx.core
        .asset_service
        .declare_provenance(
            declare(&fx.child, &fx.parent_a, Some("reference")),
            &unattributed(),
        )
        .await
        .expect("declare a reference");

    // The whole set, not just "a reference edge exists": a claim that
    // wrote both would be the overstatement this field exists to stop,
    // and asserting only the presence of the weaker edge would pass.
    assert_eq!(edge_kinds(&fx.core, &fx.child).await, vec!["reference"]);

    let edges = fx
        .core
        .asset_service
        .edges_of(&fx.child, Some("reference"), 10)
        .await
        .expect("reference edges");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to_asset_id, fx.parent_a);
    // Same label as its stronger sibling: both are a claim Asterism
    // accepted from whoever declared it, as opposed to a hop it watched.
    assert_eq!(edges[0].label.as_deref(), Some("correlated-ingest"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claim_with_no_relation_still_means_derived_from() {
    let fx = fixture().await;
    fx.core
        .asset_service
        .declare_provenance(declare(&fx.child, &fx.parent_a, None), &unattributed())
        .await
        .expect("declare without a relation");

    // Every caller that predates the field sends exactly this, and the
    // reading of what they already wrote must not move.
    assert_eq!(edge_kinds(&fx.core, &fx.child).await, vec!["derived_from"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_two_relations_coexist_on_one_asset() {
    let fx = fixture().await;
    // "I made this from A, with B in front of me" — the case the
    // vocabulary exists for. Both claims are true at once and neither
    // replaces the other: provenance is append-only.
    fx.core
        .asset_service
        .declare_provenance(
            declare(&fx.child, &fx.parent_a, Some("derived_from")),
            &unattributed(),
        )
        .await
        .expect("declare the origin");
    fx.core
        .asset_service
        .declare_provenance(
            declare(&fx.child, &fx.parent_b, Some("reference")),
            &unattributed(),
        )
        .await
        .expect("declare the reference");

    assert_eq!(
        edge_kinds(&fx.core, &fx.child).await,
        vec!["derived_from", "reference"]
    );
    let derived = fx
        .core
        .asset_service
        .edges_of(&fx.child, Some("derived_from"), 10)
        .await
        .expect("derived edges");
    let referenced = fx
        .core
        .asset_service
        .edges_of(&fx.child, Some("reference"), 10)
        .await
        .expect("reference edges");
    // Each names its own parent: a second claim that re-pointed the
    // first would lose which of the two the artefact came out of.
    assert_eq!(derived[0].to_asset_id, fx.parent_a);
    assert_eq!(referenced[0].to_asset_id, fx.parent_b);
}

/// The `_trace` note's `relation`, if it has one.
fn noted_relation(asset: &asterism_contract::dto::AssetDto) -> Option<String> {
    let extra: serde_json::Value =
        serde_json::from_str(asset.extra_json.as_deref()?).expect("extra is json");
    Some(extra.get("_trace")?.get("relation")?.as_str()?.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_note_records_the_relation_so_a_later_repair_can_carry_it() {
    let fx = fixture().await;
    // A claim naming a parent that is not there: recorded, unresolved,
    // and left for `retry_unresolved_provenance` to pick up later. That
    // sweep re-writes the edge from the note alone, so a note that did
    // not say `reference` would silently repair into `derived_from` —
    // the promotion the whole field exists to prevent, arriving by a
    // side door.
    let missing = "0198c1c2-0000-7000-8000-000000000000";
    let asset = fx
        .core
        .asset_service
        .declare_provenance(
            DeclareProvenanceCommand {
                asset_id: fx.child.clone(),
                derived_from: format!("asset:{missing}"),
                relation: Some("reference".into()),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("an unresolvable claim is recorded, not refused");

    assert_eq!(noted_relation(&asset).as_deref(), Some("reference"));
    // Nothing resolved, so nothing was drawn.
    assert!(edge_kinds(&fx.core, &fx.child).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resolved_claim_notes_its_relation_too() {
    let fx = fixture().await;
    let asset = fx
        .core
        .asset_service
        .declare_provenance(
            declare(&fx.child, &fx.parent_a, Some("reference")),
            &unattributed(),
        )
        .await
        .expect("declare a reference");
    // Both branches of `ResolvedOrigin` write the relation. A note that
    // carried it only when the claim failed would describe the graph
    // for exactly the rows whose graph is empty.
    assert_eq!(noted_relation(&asset).as_deref(), Some("reference"));
}

#[tokio::test(flavor = "multi_thread")]
async fn declaring_provenance_keeps_the_other_notes_in_the_bag() {
    // `_trace` holds more than one statement — a declared hash, a fold
    // record, a list of what a merge absorbed. The provenance writer
    // replaces the object it lives in, so this asks whether the others
    // are still there afterwards.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let path = corpus.join("declared.md");
    std::fs::write(&path, "# declared\n").expect("write file");

    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-trace-bag".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut cmd = add_command(&persona.id, path.to_str().unwrap(), 1_785_000_000_000);
    cmd.declared_content_hash = Some(format!("sha256:{}", "a".repeat(64)));
    let asset = core
        .asset_service
        .add(cmd, &unattributed())
        .await
        .expect("add with a declared hash");

    let before: serde_json::Value =
        serde_json::from_str(asset.extra_json.as_deref().expect("extra")).expect("json");
    assert!(
        before["_trace"]["declared_hash"].is_object(),
        "the declared hash is recorded at ingest: {before}"
    );

    let after_asset = core
        .asset_service
        .declare_provenance(declare(&asset.id, &asset.id, None), &unattributed())
        .await
        .expect("declare provenance");
    let after: serde_json::Value =
        serde_json::from_str(after_asset.extra_json.as_deref().expect("extra")).expect("json");

    assert!(
        after["_trace"]["declared_hash"].is_object(),
        "declaring provenance must not take the declared hash with it: {after}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_claim_does_not_leave_the_first_ones_fields_behind() {
    let fx = fixture().await;
    // First: a claim that cannot resolve. The note gets `reason` and no
    // `form`.
    let missing = "0198c1c2-0000-7000-8000-000000000000";
    let first = fx
        .core
        .asset_service
        .declare_provenance(
            DeclareProvenanceCommand {
                asset_id: fx.child.clone(),
                derived_from: format!("asset:{missing}"),
                relation: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("declare an unresolvable claim");
    let before: serde_json::Value =
        serde_json::from_str(first.extra_json.as_deref().expect("extra")).expect("json");
    assert!(before["_trace"]["reason"].is_string(), "{before}");

    // Then a claim that does resolve. Keeping only "insert what the new
    // note has" would leave `reason` standing, so the row would carry a
    // resolved claim and an explanation of why it failed.
    let second = fx
        .core
        .asset_service
        .declare_provenance(declare(&fx.child, &fx.parent_a, None), &unattributed())
        .await
        .expect("declare a resolvable claim");
    let after: serde_json::Value =
        serde_json::from_str(second.extra_json.as_deref().expect("extra")).expect("json");
    assert_eq!(after["_trace"]["resolved"], serde_json::json!(true));
    assert!(
        after["_trace"].get("reason").is_none(),
        "the failed claim's explanation must not survive its replacement: {after}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_relation_writes_nothing_at_all() {
    let fx = fixture().await;
    let err = fx
        .core
        .asset_service
        .declare_provenance(
            declare(&fx.child, &fx.parent_a, Some("derived-from")),
            &unattributed(),
        )
        .await
        .expect_err("a typo must not be filed under the stronger claim");
    assert!(
        err.to_string().contains("unknown provenance relation"),
        "the refusal says what was wrong: {err}"
    );
    // And it is refused *before* anything is written. Defaulting would
    // promote the claim; refusing but writing the edge first would do
    // the same thing more quietly.
    assert!(edge_kinds(&fx.core, &fx.child).await.is_empty());
}
