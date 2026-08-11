//! Labels stated twice, stored once — end to end through the two verbs
//! that write a label list.
//!
//! `dedup_labels` has its own unit tests next to the function, and the
//! repository has one for a row that already carries a repeat. Neither
//! can say that the write verbs *call* it: a helper nobody reaches is
//! the shape this file exists to catch, and it is the shape that was
//! actually shipped — an importer that puts the message role in the
//! labels alongside the source's own tags produced `assistant` twice on
//! one card, and the grid's keyed `{#each}` answered that with
//! `each_key_duplicate`, taking the whole virtual list down
//! [measured 2026-08-11].
//!
//! Every fixture below states the repeat **non-adjacently** and in an
//! order that is not the sorted one. Over `["a", "a", "b"]` a dedup that
//! only looked at neighbours, and over any list a dedup that sorted
//! first, would pass — so the axis under test has to disagree with both
//! defaults or the assertion is about nothing.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand, UpdateAssetMetaCommand};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str, labels: Vec<String>) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: None,
        occurred_at_ms: 1_785_000_000_000,
        session_id: None,
        external_session_key: None,
        external_key: None,
        bundle_id: None,
        labels,
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

struct Fixture {
    core: asterism_server::core_init::CoreCtx,
    persona: String,
    corpus: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

impl Fixture {
    /// Ingests a fresh file under `name` with the labels as stated.
    async fn add(&self, name: &str, labels: &[&str]) -> asterism_contract::dto::AssetDto {
        let path = self.corpus.join(name);
        std::fs::write(&path, format!("# {name}\n")).expect("write file");
        self.core
            .asset_service
            .add(
                add_command(
                    &self.persona,
                    path.to_str().unwrap(),
                    labels.iter().map(|l| (*l).to_string()).collect(),
                ),
                &unattributed(),
            )
            .await
            .expect("add asset")
    }
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
                pack_id: Some("e2e-label-dedup".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    Fixture {
        core,
        persona: persona.id.clone(),
        corpus,
        _tmp: tmp,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ingest_that_states_a_label_twice_stores_it_once() {
    let fx = fixture().await;
    let asset = fx.add("one.md", &["assistant", "cc", "assistant"]).await;

    // `inbox` is the triage default the ingest appends; everything
    // before it is the caller's list, minus the second `assistant` and
    // in the order it was stated.
    assert_eq!(asset.labels, vec!["assistant", "cc", "inbox"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ingest_that_restates_the_triage_label_does_not_double_it() {
    let fx = fixture().await;
    // The re-run shape: a caller carrying the label set it read back
    // last time, which already contains `inbox`. The `contains` guard
    // in the ingest covers this one; the assertion is here so the two
    // rules are pinned together — dropping either would double a chip.
    let asset = fx.add("two.md", &["inbox", "cc"]).await;

    assert_eq!(asset.labels, vec!["inbox", "cc"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_meta_update_that_states_a_label_twice_stores_it_once() {
    let fx = fixture().await;
    let asset = fx.add("three.md", &[]).await;

    let updated = fx
        .core
        .asset_service
        .update_meta(
            UpdateAssetMetaCommand {
                asset_id: asset.id.clone(),
                // `["b", "a", "b"]`: the repeat is not adjacent, and the
                // surviving pair is not in sorted order — so neither
                // wrong dedup passes here.
                labels: Some(vec!["b".into(), "a".into(), "b".into()]),
                register_note: None,
                cover: None,
                title: None,
                rating: None,
                modality: None,
                bundle_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("update meta");

    // A full replace: the update names the whole list, so `inbox` is
    // gone by the caller's instruction, not by the dedup.
    assert_eq!(updated.labels, vec!["b", "a"]);

    // And it survives the round trip rather than only the return value.
    let reread = fx
        .core
        .asset_service
        .detail(asterism_contract::query::GetAssetDetailQuery {
            asset_id: asset.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("detail");
    assert_eq!(reread.asset.labels, vec!["b", "a"]);
}
