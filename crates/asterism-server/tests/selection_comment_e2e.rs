//! A selection gesture may carry a sentence (#65).
//!
//! The verbs under test are `trash`, `restore` and `trash_group`: each
//! may carry an optional one-line remark, and the remark lands as an
//! `AssetComment` pinned to the gesture — actor, time and verb on the
//! row — so the asset's thread shows when in its life each sentence
//! was said. The interesting fixtures are the negative spaces: a
//! gesture with no remark writes no comment (the mechanism is a
//! footnote, not a log), a whitespace-only remark is a blank submit
//! rather than an error, and a group fan-out reaches the members and
//! nobody else.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, AddAssetToGroupCommand, CreateGroupCommand, RegisterPersonaCommand,
    RestoreAssetCommand, TrashAssetCommand, TrashGroupCommand,
};
use asterism_contract::dto::AssetCommentDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. The comment author is decided by
/// the gesture write itself (`User`), not by this value.
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

async fn thread_of(core: &CoreCtx, asset_id: &str) -> Vec<AssetCommentDto> {
    core.asset_comment_service
        .list(asset_id)
        .await
        .expect("list comments")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gesture_remark_lands_as_a_pinned_comment_and_silence_lands_as_nothing() {
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
                pack_id: Some("e2e-selection-comment".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for (n, name) in [
        "spoken.md",
        "silent.md",
        "blank.md",
        "member-a.md",
        "member-b.md",
        "outsider.md",
    ]
    .iter()
    .enumerate()
    {
        let path = corpus.join(name);
        std::fs::write(&path, "# body\n").expect("write file");
        let dto = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    1_785_000_000_000 + n as i64 * 1_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(dto.id);
    }
    let (spoken, silent, blank) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    let (member_a, member_b, outsider) = (ids[3].clone(), ids[4].clone(), ids[5].clone());

    // A trash with a sentence: the thread gains one comment, pinned to
    // the verb, spoken by the user.
    core.asset_service
        .trash(
            TrashAssetCommand {
                asset_id: spoken.clone(),
                comment: Some("wrong hands again".into()),
            },
            &unattributed(),
        )
        .await
        .expect("trash with remark");
    let thread = thread_of(&core, &spoken).await;
    assert_eq!(thread.len(), 1, "one gesture, one footnote");
    assert_eq!(thread[0].body, "wrong hands again");
    assert_eq!(thread[0].gesture.as_deref(), Some("trash"));
    assert_eq!(thread[0].author_kind, "user");

    // A salvage with a sentence: same asset, second footnote, and the
    // thread now reads as the asset's life — thrown, then pulled back.
    core.asset_service
        .restore(
            RestoreAssetCommand {
                asset_id: spoken.clone(),
                comment: Some("keep for the pose, not the face".into()),
            },
            &unattributed(),
        )
        .await
        .expect("restore with remark");
    let thread = thread_of(&core, &spoken).await;
    assert_eq!(thread.len(), 2);
    assert_eq!(thread[1].gesture.as_deref(), Some("restore"));
    assert_eq!(thread[1].body, "keep for the pose, not the face");

    // A mute gesture stays mute — the mechanism is opt-in prose, not a
    // gesture log.
    core.asset_service
        .trash(
            TrashAssetCommand {
                asset_id: silent.clone(),
                comment: None,
            },
            &unattributed(),
        )
        .await
        .expect("trash without remark");
    assert!(
        thread_of(&core, &silent).await.is_empty(),
        "no remark, no comment"
    );

    // Whitespace-only is a blank submit, silently discarded — the same
    // stance the comment UI takes — not a validation error that would
    // refuse the trash itself.
    core.asset_service
        .trash(
            TrashAssetCommand {
                asset_id: blank.clone(),
                comment: Some("   ".into()),
            },
            &unattributed(),
        )
        .await
        .expect("trash with blank remark");
    assert!(
        thread_of(&core, &blank).await.is_empty(),
        "a blank remark lands as nothing"
    );

    // A group trash fans the sentence out to the members — the batch
    // the sentence was said over — and to nobody else.
    let group = core
        .asset_service
        .create_group(
            CreateGroupCommand {
                persona_id: persona.id.clone(),
                name: "round-3".into(),
                description: None,
            },
            &unattributed(),
        )
        .await
        .expect("create group");
    for member in [&member_a, &member_b] {
        core.asset_service
            .add_asset_to_group(
                AddAssetToGroupCommand {
                    asset_id: member.clone(),
                    group_id: group.id.clone(),
                },
                &unattributed(),
            )
            .await
            .expect("file member");
    }
    core.asset_service
        .trash_group(
            TrashGroupCommand {
                group_id: group.id.clone(),
                comment: Some("this round's angle was wrong".into()),
            },
            &unattributed(),
        )
        .await
        .expect("trash group with remark");
    for member in [&member_a, &member_b] {
        let thread = thread_of(&core, member).await;
        assert_eq!(thread.len(), 1, "each member carries the batch remark");
        assert_eq!(thread[0].gesture.as_deref(), Some("trash_group"));
        assert_eq!(thread[0].body, "this round's angle was wrong");
    }
    assert!(
        thread_of(&core, &outsider).await.is_empty(),
        "the fan-out reaches the filing's members, not the library"
    );
}
