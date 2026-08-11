//! End-to-end: a caller declares what the bytes hash to, and the
//! server checks it without ever believing it.
//!
//! The declaration crosses a gap nothing else survives — `add` returns
//! without opening the file, and the digest is computed later on a job
//! worker — so the only way to know the two halves meet is to run both
//! of them. `Full` mode takes the writer lock and spawns that worker,
//! the same reason `duplicate_detection_e2e` uses it.
//!
//! # The fixture that carries the weight
//!
//! `a_declared_digest_the_bytes_disagree_with_…` is the one that can
//! fail.
//! An implementation that simply stored the caller's string as the
//! fingerprint would pass every agreeing fixture ever written, because
//! there the claim and the recomputed value are the same characters and
//! no assertion can tell which of them it is looking at. Only a claim
//! the file disagrees with separates "we read the bytes" from "we
//! copied the header of the request".
//!
//! Everything is read back through `detail`, never off the DTO `add`
//! returned: that one is projected from the in-memory entity and would
//! report the claim whether or not a column was ever written.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_contract::dto::AssetDetailDto;
use asterism_contract::query::GetAssetDetailQuery;
use asterism_core::application::AssetService;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::content_hash;
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the declared digest,
/// not about who declared it.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
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

async fn detail_of(service: &AssetService, asset_id: &str) -> AssetDetailDto {
    service
        .detail(GetAssetDetailQuery {
            asset_id: asset_id.to_string(),
            viewer_subject: None,
        })
        .await
        .expect("read the asset back")
}

/// The `_trace.declared_hash` note as the wire carries it, or `None`
/// when the row has no such note.
fn declared_hash_note(detail: &AssetDetailDto) -> Option<serde_json::Value> {
    let extra: serde_json::Value =
        serde_json::from_str(detail.asset.extra_json.as_deref()?).expect("extra is valid JSON");
    extra.get("_trace")?.get("declared_hash").cloned()
}

/// Polls `detail` until `ready` holds. Fingerprinting is asynchronous
/// and its verdict lands after it, so waiting on the verdict rather
/// than on the digest is what keeps the assertions from racing the
/// worker.
async fn wait_for(
    service: &AssetService,
    asset_id: &str,
    what: &str,
    ready: impl Fn(&AssetDetailDto) -> bool,
) -> AssetDetailDto {
    for _ in 0..120 {
        let detail = detail_of(service, asset_id).await;
        if ready(&detail) {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{what} did not happen within 30s");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declared_digest_the_bytes_disagree_with_is_recorded_and_costs_the_asset_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let bytes = b"what is actually on the disk\n";
    let file = corpus.join("contested.png");
    std::fs::write(&file, bytes).expect("write corpus file");
    let actual = content_hash::of_bytes(bytes);
    // A real, well-formed digest — of something else. The caller hashed
    // the wrong file, or hashed it before an editor rewrote it.
    let declared = content_hash::of_bytes(b"what the caller thought it was\n");
    assert_ne!(
        declared, actual,
        "the fixture only means something if they differ"
    );

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
                pack_id: Some("e2e-declared-hash-mismatch".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut command = add_command(&persona.id, file.to_str().unwrap(), 1_785_000_000_000);
    command.declared_content_hash = Some(declared.clone());
    let registered = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("a declaration is a claim, and a claim is never a reason to refuse the file");

    let settled = wait_for(
        &core.asset_service,
        &registered.id,
        "the declared digest was checked",
        |detail| {
            declared_hash_note(detail)
                .and_then(|note| note.get("verified").cloned())
                .is_some()
        },
    )
    .await;

    // The load-bearing assertion. What is stored is what the file
    // hashes to — not the string the caller sent, which is sitting a
    // few bytes away in the same row and would satisfy every fixture
    // where the two agree.
    assert_eq!(
        settled.asset.content_hash.as_deref(),
        Some(actual.as_str()),
        "the material must carry the recomputed digest, not the declared one"
    );
    assert_ne!(
        settled.asset.content_hash.as_deref(),
        Some(declared.as_str()),
        "a declaration must never become the fingerprint"
    );

    let note = declared_hash_note(&settled).expect("the verdict is on the row");
    assert_eq!(note["verified"], serde_json::json!(false));
    assert_eq!(
        note["value"],
        serde_json::json!(declared),
        "specified: what the caller said"
    );
    assert_eq!(
        note["got"],
        serde_json::json!(actual),
        "and got: what the bytes say — a reader with only one of them cannot \
         tell which side to go and look at"
    );

    // Nothing else happened to the asset. A disagreement is a finding,
    // not a verdict on the file: the row is still live and still in its
    // persona's grid, which is the listing that drops trashed and
    // folded-away rows.
    let listed = core
        .asset_service
        .list(asterism_contract::query::ListAssetsQuery {
            persona_id: Some(persona.id.clone()),
            ..Default::default()
        })
        .await
        .expect("list");
    assert!(
        listed.items.iter().any(|card| card.id == registered.id),
        "the asset is still in the grid"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declared_digest_the_bytes_agree_with_is_recorded_as_agreement() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let bytes = b"the caller and the disk agree\n";
    let file = corpus.join("agreed.png");
    std::fs::write(&file, bytes).expect("write corpus file");
    let digest = content_hash::of_bytes(bytes);

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
                pack_id: Some("e2e-declared-hash-match".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut command = add_command(&persona.id, file.to_str().unwrap(), 1_785_000_000_000);
    command.declared_content_hash = Some(digest.clone());
    let registered = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("add");

    // Before the worker gets there, the claim is on the row with no
    // verdict — "not checked yet" is a state, and it has to be
    // distinguishable from "checked and disagreed".
    let claim = declared_hash_note(&detail_of(&core.asset_service, &registered.id).await);
    if let Some(claim) = claim {
        assert_eq!(claim["value"], serde_json::json!(digest));
        assert_eq!(claim["axis"], serde_json::json!("artefact"));
    }

    let settled = wait_for(
        &core.asset_service,
        &registered.id,
        "the declared digest was checked",
        |detail| {
            declared_hash_note(detail)
                .and_then(|note| note.get("verified").cloned())
                .is_some()
        },
    )
    .await;

    let note = declared_hash_note(&settled).expect("the verdict is on the row");
    assert_eq!(note["verified"], serde_json::json!(true));
    assert_eq!(note["value"], serde_json::json!(digest));
    assert!(
        note.get("got").is_none(),
        "on agreement the digest is on the material; a second copy here is one \
         more thing to reconcile: {note}"
    );
    assert!(
        note.get("checked_at_ms").is_some(),
        "when it was answered, so a claim from before the check is not read as \
         one that passed: {note}"
    );
    assert_eq!(settled.asset.content_hash.as_deref(), Some(digest.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_undeclared_registration_is_hashed_exactly_as_before() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let bytes = b"nobody said anything about these bytes\n";
    let file = corpus.join("silent.png");
    std::fs::write(&file, bytes).expect("write corpus file");

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
                pack_id: Some("e2e-declared-hash-absent".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let registered = core
        .asset_service
        .add(
            add_command(&persona.id, file.to_str().unwrap(), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add");

    let settled = wait_for(
        &core.asset_service,
        &registered.id,
        "the file was fingerprinted",
        |detail| detail.asset.content_hash.is_some(),
    )
    .await;
    assert_eq!(
        settled.asset.content_hash.as_deref(),
        Some(content_hash::of_bytes(bytes).as_str())
    );
    assert!(
        declared_hash_note(&settled).is_none(),
        "an unrecorded declaration must not read back as an answered one"
    );
}

/// The four shapes a declaration is refused in, and the proof that a
/// refusal wrote nothing.
///
/// The last check is what makes the refusals worth having: the locator
/// is looked up before anything is minted, so if a refused ingest had
/// got as far as saving the row, the honest retry would be answered
/// with the id of a registration the caller was told had failed —
/// silently, and with none of the fields the retry carried. (The
/// sentence here used to say `UNIQUE(source_kind, source_locator)`,
/// which was true when it was written; V61 demoted that constraint to a
/// plain index and the lookup took over its job. The hazard is the
/// same, and it stopped being an error the caller would see.)
#[tokio::test(flavor = "multi_thread")]
async fn a_declared_digest_nothing_can_ever_check_is_refused_before_anything_is_written() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("refused.png");
    std::fs::write(&file, b"bytes\n").expect("write corpus file");
    let path = file.to_str().unwrap().to_string();

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
                pack_id: Some("e2e-declared-hash-refused".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    for (declared, expected) in [
        // A different question's answer.
        ("phash:0f0f0f0f".to_string(), "no known algorithm tag"),
        // No tag at all — guessing the algorithm is how two of them
        // become one spelling.
        ("a".repeat(64), "no known algorithm tag"),
        // Right tag, a shape the hasher cannot produce: keeping it
        // would report a mismatch about a file that is fine.
        ("sha256:not-a-digest".to_string(), "lowercase hex"),
        // A field that says nothing while looking like it says
        // something.
        ("   ".to_string(), "blank"),
    ] {
        let mut command = add_command(&persona.id, &path, 1_785_000_000_000);
        command.declared_content_hash = Some(declared.clone());
        let err = core
            .asset_service
            .add(command, &unattributed())
            .await
            .expect_err("a declaration nothing can check must not be accepted")
            .to_string();
        assert!(
            err.contains(expected),
            "{declared:?} should be refused for {expected}: {err}"
        );
    }

    // The content axis is **not** in the list above any more. It was,
    // while nothing computed that axis — a claim taken then would have
    // read as pending forever. There is a column and a walker now, so
    // the claim is answerable and is taken, under its own axis label.
    let content_claim = format!(
        "{}{}",
        asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
        "a".repeat(64)
    );
    let accepted_path = corpus.join("accepted.png");
    std::fs::write(&accepted_path, b"bytes\n").expect("write corpus file");
    let mut command = add_command(
        &persona.id,
        accepted_path.to_str().unwrap(),
        1_785_000_000_500,
    );
    command.declared_content_hash = Some(content_claim.clone());
    let accepted = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("a content-axis claim is answerable now, so it is taken");

    let note = declared_hash_note(&detail_of(&core.asset_service, &accepted.id).await)
        .expect("the claim is recorded on the row");
    assert_eq!(note["value"], serde_json::json!(content_claim));
    assert_eq!(
        note["axis"],
        serde_json::json!("content"),
        "the axis label is what lets the job pick which recomputed value to check against"
    );
    assert!(
        note.get("verified").is_none(),
        "a claim carries no verdict at registration; the bytes have not been read yet"
    );

    // A locator with no bytes behind it. The hash job records
    // `unhashable:no-bytes` for these and never opens anything, so the
    // claim's verdict would never arrive.
    let mut command = add_command(
        &persona.id,
        "/logs/session.jsonl#0198c1c2-aaaa",
        1_785_000_001_000,
    );
    command.declared_content_hash = Some(content_hash::of_bytes(b"anything"));
    let err = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect_err("a fragment locator has no bytes to check a claim against")
        .to_string();
    assert!(err.contains("no bytes to read"), "{err}");

    // And the refusals reserved nothing: the same locator still ingests.
    let registered = core
        .asset_service
        .add(
            add_command(&persona.id, &path, 1_785_000_002_000),
            &unattributed(),
        )
        .await
        .expect("the refused registrations left nothing behind to collide with");
    assert!(!registered.id.is_empty());
}
