//! Source type end to end — the read the detail pane's row sits on,
//! and the verb it drives (#108).
//!
//! The write verb's own behaviour (the `_trace` entry it files, what
//! it refuses, what a retraction removes) is pinned where it lives;
//! what these cases pin is the round trip the panel's verification bar
//! names: an assertion made through the real service shows up on the
//! next read with who and when beside it, a retraction returns the row
//! to what the evidence says on its own, and a term the vocabulary
//! does not define is refused rather than recorded.
//!
//! The fixture ingests real files and runs no jobs, so the container
//! has not been fingerprinted when these cases read — which is itself
//! one of the answers under test: the read reports *not yet read*
//! rather than collapsing it into *declares nothing*, the distinction
//! the storage keeps. The evidence-established state (a container
//! whose keys name a generator or a camera) is the same derivation
//! `record_for` composes and is pinned beside it in
//! `domain::disclosure` and `asterism-infra`'s disclosure_service
//! tests; nothing here re-derives it.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, DeclareSourceTypeCommand, RegisterPersonaCommand,
};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
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

struct Fixture {
    core: asterism_server::core_init::CoreCtx,
    asset: String,
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
                pack_id: Some("e2e-source-type".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let path = corpus.join("subject.md");
    std::fs::write(&path, "# subject\n").expect("write file");
    let asset = core
        .asset_service
        .add(
            add_command(&persona.id, path.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");

    Fixture {
        core,
        asset: asset.id,
        _tmp: tmp,
    }
}

fn declare(asset_id: &str, term: Option<&str>) -> DeclareSourceTypeCommand {
    DeclareSourceTypeCommand {
        asset_id: asset_id.to_string(),
        source_type: term.map(str::to_string),
        operator_ai: None,
    }
}

#[tokio::test]
async fn an_assertion_round_trips_and_a_retraction_returns_to_the_evidence() {
    let f = fixture().await;

    // Before anything is said: no assertion, and the container reads
    // as *not yet read* — no job ran, so the fingerprint is pending,
    // and that is a different absence from "declares nothing".
    let before = f
        .core
        .asset_service
        .source_type_of(&f.asset, None)
        .await
        .expect("read");
    assert!(before.asserted.is_none());
    assert_eq!(before.evidence, None);
    assert!(
        before.evidence_pending,
        "no fingerprint job ran, so the container is not yet read"
    );

    // Assert by short name; the store keeps the URI and the read hands
    // back the short name — one spelling for a person, one for a file.
    f.core
        .asset_service
        .declare_source_type(declare(&f.asset, Some("humanEdits")), &unattributed())
        .await
        .expect("assert");
    let asserted = f
        .core
        .asset_service
        .source_type_of(&f.asset, None)
        .await
        .expect("read");
    let entry = asserted.asserted.expect("the assertion shows on reload");
    assert_eq!(entry.source_type, "humanEdits");
    assert_eq!(entry.operator, None);
    assert!(
        entry.declared_at_ms.is_some(),
        "the verb records the moment, and the read carries it"
    );

    // Retraction is an absent term, and the row returns to what the
    // evidence says on its own.
    f.core
        .asset_service
        .declare_source_type(declare(&f.asset, None), &unattributed())
        .await
        .expect("retract");
    let after = f
        .core
        .asset_service
        .source_type_of(&f.asset, None)
        .await
        .expect("read");
    assert!(after.asserted.is_none());
    assert_eq!(after.evidence, None);
}

#[tokio::test]
async fn a_term_the_vocabulary_does_not_define_is_refused_not_recorded() {
    let f = fixture().await;

    // `negativeFilm` is a real IPTC term this corpus deliberately does
    // not carry — the closed set is five, and the door is the verb.
    let refusal = f
        .core
        .asset_service
        .declare_source_type(declare(&f.asset, Some("negativeFilm")), &unattributed())
        .await;
    assert!(refusal.is_err(), "an unknown term must not be recorded");

    let read = f
        .core
        .asset_service
        .source_type_of(&f.asset, None)
        .await
        .expect("read");
    assert!(read.asserted.is_none(), "a refusal writes nothing");
}
