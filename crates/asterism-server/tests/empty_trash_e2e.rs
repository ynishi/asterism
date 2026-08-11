//! Emptying the trash takes the trash, and only the trash.
//!
//! The command carries no filter, so the interesting fixture is a
//! library where the two sides disagree: some assets thrown away, some
//! still live. A sweep that took everything, or one that took nothing,
//! would both pass against a trash-only fixture.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, EmptyTrashCommand, RegisterPersonaCommand, TrashAssetCommand,
};
use asterism_contract::query::ListAssetsQuery;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the trash, not about
/// who ingested the row.
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

/// Ids visible on one side of the trash, in whatever order the grid
/// would list them.
async fn ids_on_side(core: &CoreCtx, trash: Option<&str>) -> Vec<String> {
    let page = core
        .asset_service
        .list(ListAssetsQuery {
            trash: trash.map(str::to_string),
            ..Default::default()
        })
        .await
        .expect("list");
    page.items.into_iter().map(|card| card.id).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn emptying_the_trash_takes_every_trashed_asset_and_no_live_one() {
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
                pack_id: Some("e2e-empty-trash".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Four assets, two of which get thrown away. The survivors are
    // what makes the assertion non-vacuous: an `empty_trash` that
    // ignored the trash stamp would take them too.
    let mut ids = Vec::new();
    for (n, name) in ["keep-a.md", "keep-b.md", "toss-a.md", "toss-b.md"]
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
    let (kept, tossed) = (ids[..2].to_vec(), ids[2..].to_vec());

    for id in &tossed {
        core.asset_service
            .trash(
                TrashAssetCommand {
                    asset_id: id.clone(),
                },
                &unattributed(),
            )
            .await
            .expect("trash");
    }

    // The fixture really does straddle both sides before the sweep.
    let mut before_trashed = ids_on_side(&core, Some("trashed")).await;
    before_trashed.sort();
    let mut expected_trashed = tossed.clone();
    expected_trashed.sort();
    assert_eq!(
        before_trashed, expected_trashed,
        "two assets are in the trash before the sweep"
    );
    assert_eq!(
        ids_on_side(&core, None).await.len(),
        2,
        "two assets are live before the sweep"
    );

    let result = core
        .asset_service
        .empty_trash(EmptyTrashCommand::default(), &unattributed())
        .await
        .expect("empty trash");

    assert_eq!(result.purged, 2, "both trashed assets were purged");
    assert_eq!(result.skipped, 0, "nothing was skipped");

    assert!(
        ids_on_side(&core, Some("trashed")).await.is_empty(),
        "the trash is empty afterwards"
    );

    let mut after_live = ids_on_side(&core, None).await;
    after_live.sort();
    let mut expected_live = kept.clone();
    expected_live.sort();
    assert_eq!(
        after_live, expected_live,
        "the live assets are untouched — the sweep reaches only trashed rows"
    );

    // Purged means gone, not hidden: the detail read for a swept id
    // must fail rather than return a row on the `any` side.
    let any_side = ids_on_side(&core, Some("any")).await;
    for id in &tossed {
        assert!(
            !any_side.contains(id),
            "purged asset {id} is gone from the library, not merely filtered out"
        );
    }
}

/// Emptying an already-empty trash is a no-op that still succeeds —
/// the button stays pressable without the caller having to guess.
#[tokio::test(flavor = "multi_thread")]
async fn emptying_an_empty_trash_purges_nothing_and_leaves_the_library_alone() {
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
                pack_id: Some("e2e-empty-trash-noop".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let path = corpus.join("only.md");
    std::fs::write(&path, "# body\n").expect("write file");
    let live = core
        .asset_service
        .add(
            add_command(&persona.id, path.to_str().unwrap(), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add asset");

    let result = core
        .asset_service
        .empty_trash(EmptyTrashCommand::default(), &unattributed())
        .await
        .expect("empty trash");

    assert_eq!(result.purged, 0, "nothing was in the trash");
    assert_eq!(result.skipped, 0, "and nothing failed");
    assert_eq!(
        ids_on_side(&core, None).await,
        vec![live.id],
        "the live asset is still there"
    );
}
