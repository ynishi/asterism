//! A sidecar with no identity block still connects — one hop shorter.
//!
//! Sidecars written before the identity block existed, and ones a
//! person wrote by hand, carry only the card. The card's `id` names
//! the original, so the link lands there instead of on the export copy
//! that sat between them. That is a real difference in what the chain
//! says, so `_trace.form` records which route resolved it rather than
//! flattening both into "sidecar".
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_contract::sidecar::SIDECAR_SUFFIX;
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
async fn a_sidecar_without_an_identity_block_falls_back_and_a_missing_one_is_reported() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let plate = corpus.join("plate.md");
    let returned = corpus.join("returned.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");
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
                pack_id: Some("e2e-correlation-sidecar-fallback".into()),
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

    // Card only — the shape a pre-identity-block export produced.
    std::fs::write(
        format!("{}{}", returned.display(), SIDECAR_SUFFIX),
        serde_json::to_vec_pretty(&serde_json::json!({
            "id": source.id,
            "source_locator": plate.to_string_lossy(),
        }))
        .unwrap(),
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
    assert_eq!(edges[0].to_asset_id, source.id);

    let extra: serde_json::Value =
        serde_json::from_str(child.extra_json.as_deref().expect("extra bag")).expect("extra JSON");
    assert_eq!(
        extra
            .get("_trace")
            .and_then(|t| t.get("form"))
            .and_then(|v| v.as_str()),
        Some("sidecar-asset"),
        "the shorter route is recorded as such, not disguised as the full one"
    );

    // Second half, same core (one `init_core` per test binary — it
    // opens the profile-global Tantivy index): the declaration was
    // made but the file it points at is not there. Nothing to
    // resolve, and nothing to invent.
    let orphan = corpus.join("orphan.md");
    std::fs::write(&orphan, "# orphan\n").expect("write orphan");

    let child = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                orphan.to_str().unwrap(),
                1_785_000_200_000,
                Some("sidecar".into()),
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
            .is_empty()
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
        reason.contains("no sidecar at"),
        "reason should name the file it looked for: {reason}"
    );
}
