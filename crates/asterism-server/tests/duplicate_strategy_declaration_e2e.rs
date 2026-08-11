//! The duplicate strategy a caller declares at registration is still
//! there afterwards — and silence still reads as silence.
//!
//! `add` returns without reading a byte of the artefact, and the
//! fingerprint that can raise a duplicate is computed later by a
//! background job. So the declaration has to survive the gap on the
//! row; nothing else outlives the call. Nothing consumes it yet
//! (detection, the queue and the fold verb are later subtasks), which
//! is precisely why it needs a test now: an unread column is where a
//! write path rots unnoticed.
//!
//! Every assertion here re-reads through `detail`, not the DTO `add`
//! hands back. That DTO is projected from the in-memory entity, so it
//! would report the declaration whether or not the column was ever
//! written.
//!
//! Its own test binary because `init_core` opens a Tantivy index and
//! the sibling e2e files follow the same one-core-per-file shape.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, OnDuplicate, RegisterPersonaCommand};
use asterism_contract::query::GetAssetDetailQuery;
use asterism_core::domain::attribution::AttributionContext;
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the declared
/// strategy, not about who declared it.
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

#[tokio::test(flavor = "multi_thread")]
async fn a_declared_strategy_is_persisted_and_an_undeclared_one_stays_unrecorded() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

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
                pack_id: Some("e2e-duplicate-strategy".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Each of the three, declared and read back off the row.
    for (index, (declared, slug)) in [
        (OnDuplicate::Ask, "ask"),
        (OnDuplicate::Fold, "fold"),
        (OnDuplicate::Separate, "separate"),
    ]
    .into_iter()
    .enumerate()
    {
        let file = corpus.join(format!("{slug}.md"));
        std::fs::write(&file, "# body\n").expect("write corpus file");
        let mut command = add_command(
            &persona.id,
            file.to_str().unwrap(),
            1_785_000_000_000 + index as i64 * 1_000,
        );
        command.on_duplicate = Some(declared);
        let registered = core
            .asset_service
            .add(command, &unattributed())
            .await
            .expect("a declared strategy is not a reason to refuse the ingest");

        let stored = core
            .asset_service
            .detail(GetAssetDetailQuery {
                asset_id: registered.id.clone(),
                viewer_subject: None,
            })
            .await
            .expect("read the asset back");
        assert_eq!(
            stored.asset.on_duplicate.as_deref(),
            Some(slug),
            "{slug} was declared at registration and is not on the row"
        );
    }

    // No declaration → nothing recorded. Not `ask`: `ask` is what an
    // undeclared registration resolves to while it is the only default
    // there is, and the two are different facts. Only the unrecorded one
    // can pick up an importer / persona default when those exist.
    let silent = corpus.join("silent.md");
    std::fs::write(&silent, "# body\n").expect("write corpus file");
    let registered = core
        .asset_service
        .add(
            add_command(&persona.id, silent.to_str().unwrap(), 1_785_000_009_000),
            &unattributed(),
        )
        .await
        .expect("add silent");
    let stored = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: registered.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("read the asset back");
    assert_eq!(
        stored.asset.on_duplicate, None,
        "an undeclared registration must not read back as a request to ask"
    );
}

/// An unknown token is refused, and the refusal happens above every
/// side effect an ingest has.
///
/// Not by validating early inside `add` — by the field being a closed
/// enum, so the command carrying it never deserialises and the service
/// is never reached. That is a stronger guarantee than ordering the
/// validation correctly, and the test says what it actually measures:
/// the payload that would have minted a Session container before
/// failing is refused whole.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_strategy_is_refused_before_the_ingest_can_write_anything() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("contested.md");
    std::fs::write(&file, "# body\n").expect("write corpus file");

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
                pack_id: Some("e2e-duplicate-strategy-reject".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // The payload carries an `external_session_key` on purpose: that is
    // the field whose resolution *writes* (find-or-create mints a
    // container), so this is the shape that could have left a Session
    // behind for an ingest that never landed.
    let payload = serde_json::json!({
        "persona_id": persona.id,
        "source_kind": "fs",
        "locator": file.to_str().unwrap(),
        "occurred_at_ms": 1_785_000_000_000i64,
        "external_session_key": "a-session-that-must-not-be-minted",
        "labels": [],
        "register_note": null,
        "platform": null,
        "file_size_bytes": null,
        "duration_ms": null,
        "extra_json": null,
        "cover_hint": null,
        "on_duplicate": "maybe",
    });
    let err = serde_json::from_value::<AddAssetCommand>(payload.clone())
        .expect_err("\"maybe\" is not one of the three answers");
    let message = err.to_string();
    assert!(
        message.contains("maybe"),
        "the error should quote the value that does not hold: {message}"
    );
    assert!(
        message.contains("ask") && message.contains("fold") && message.contains("separate"),
        "and say what the answers are: {message}"
    );
    // Measured, so the claim above is about the message that exists:
    // `unknown variant `maybe`, expected one of `ask`, `fold`,
    // `separate``. It does not name the field — `serde_json` drops the
    // path — which is why the assertions are about the value and the
    // set rather than about `on_duplicate` appearing in the text.

    // The same payload with a real token parses — so the refusal above
    // was about the value, not about the shape of the request.
    let mut accepted = payload;
    accepted["on_duplicate"] = serde_json::json!("fold");
    let command = serde_json::from_value::<AddAssetCommand>(accepted)
        .expect("`fold` is one of the three answers");
    assert_eq!(command.on_duplicate, Some(OnDuplicate::Fold));

    // And the locator is still free: the refused registration left
    // nothing behind to collide with, because it never ran.
    let registered = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("the refused payload reserved nothing");
    let stored = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: registered.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("read the asset back");
    assert_eq!(stored.asset.on_duplicate.as_deref(), Some("fold"));
}
