//! A provenance claim that cannot be honoured must not cost the user
//! the artefact.
//!
//! The token travels outside the application — through a shell, a chat
//! window, someone's clipboard — so it arrives stale or mistyped
//! sooner or later. When it does, the file is still a file: it lands,
//! and the claim is recorded with the reason it failed so the link can
//! be repaired later instead of silently never existing.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index, and two cores in one process contend for the
//! writer lock (the sibling e2e files follow the same one-core-per-file
//! shape).

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution this fixture writes with: a caller that states
/// nothing, which records nothing.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unresolvable_claim_still_lands_the_artefact_and_says_why() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let returned = corpus.join("returned.md");
    std::fs::write(&returned, "# orphan\n").expect("write returned");

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
                pack_id: Some("e2e-correlation-orphan".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // A parent id that was never in this library — the ordinary shape
    // of a mistyped or stale token.
    let missing = "0198c1c2-0000-7000-8000-000000000000";
    let child = core
        .asset_service
        .add(
            AddAssetCommand {
                persona_id: persona.id.clone(),
                source_kind: "fs".into(),
                locator: returned.to_str().unwrap().to_string(),
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
                derived_from: Some(format!("asset:{missing}")),
                author_kind: None,
                author_subject: None,
                operator_ai: None,
                on_duplicate: None,
                declared_content_hash: None,
                album_meta: Default::default(),
            },
            &unattributed(),
        )
        .await
        .expect("a broken provenance claim must not cost the user the file");

    assert!(
        core.asset_service
            .edges_of(&child.id, Some("derived_from"), 10)
            .await
            .expect("edges of child")
            .is_empty(),
        "nothing to point at, so no edge"
    );

    let extra: serde_json::Value = serde_json::from_str(
        child
            .extra_json
            .as_deref()
            .expect("the failed claim is written to the row's extra bag"),
    )
    .expect("extra is JSON");
    let trace = extra
        .get("_trace")
        .expect("the claim is recorded even though it did not resolve");
    assert_eq!(trace.get("resolved").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        trace.get("derived_from").and_then(|v| v.as_str()),
        Some(format!("asset:{missing}").as_str())
    );
    let reason = trace
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("a reason a human can act on");
    assert!(
        reason.contains("not in this library"),
        "reason should name the problem: {reason}"
    );
    // Channel bookkeeping survives the failure: an `asset:` claim
    // carried on the ingest payload is `pushed`, resolved or not.
    assert_eq!(
        trace.get("source").and_then(|v| v.as_str()),
        Some("pushed"),
        "an ingest-payload claim is recorded as pushed even when unresolved"
    );
}
