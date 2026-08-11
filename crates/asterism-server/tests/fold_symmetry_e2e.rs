//! The two entry points to a fold have to leave the same world behind.
//!
//! `resolve_duplicate_conflict` answers a **detected pair** and hands
//! the fold to a job. `merge_assets` carries out a person's ruling over
//! a set and folds inside its own transaction. The second landed two
//! days after the first grew its after-effects, and inherited none of
//! them: a hand-merged row kept its Tantivy document and its persona's
//! Query Groups were never told.
//!
//! The damage that leaves is invisible from every filtered surface,
//! which is why it survived. `search` intersects its shortlist with
//! `filter_ids`, and that predicate excludes folded rows — so the stale
//! document is never *drawn*. It is counted: it occupies one of the
//! `RETRIEVAL_K_CEILING` places the shortlist has, and it inflates the
//! `candidates_considered` a caller is told the answer was drawn from.
//! Assertions therefore go under the filter, straight at the index.
//!
//! # The two modes, and why each test needs the one it uses
//!
//! `both_fold_entry_points_leave_the_same_work_behind` runs
//! `CoreMode::ReadOnly`: no worker, so an enqueue stays on the queue to
//! be counted instead of racing something that drains it. The conflict
//! the automatic path answers is raised through the repository rather
//! than by fingerprinting a file, because the fingerprint is a *job* and
//! there is nothing here to run it.
//!
//! `a_manual_merge_takes_the_headstone_out_of_search` runs
//! `CoreMode::Full` for the opposite reason: the claim is that the
//! enqueued job runs and cleans up, so the worker is the thing under
//! test. Its assets are real text files — an image produces a document
//! with no body to retrieve on, and this test has to see a document
//! disappear.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{
    AddAssetCommand, ConflictResolution, MergeAssetsCommand, RegisterPersonaCommand,
    ResolveDuplicateConflictCommand,
};
use asterism_core::domain::duplicate_conflict::{DuplicateAxis, DuplicateConflict};
use asterism_core::domain::repository::{
    AssetRepository, AssetRetriever, RetrievalIntent, RetrievalQuery,
};
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_infra::jobs::jobs_snapshot;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about what a fold leaves
/// behind, not about who ran it.
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

/// Registers a persona and returns its id in both forms the calls below
/// want.
async fn persona(core: &CoreCtx, tag: &str) -> (String, PersonaId) {
    let dto = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(format!("e2e-{tag}-{}", uuid::Uuid::now_v7())),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let parsed = PersonaId::from_uuid(dto.id.parse().expect("persona uuid"));
    (dto.id, parsed)
}

/// Per-kind pending counts, keyed by the job's slug.
///
/// Read rather than a recording double so what is counted is what the
/// production queue actually persisted. `ReadOnly` mode never drains, so
/// a count taken after a call is a count of everything that call pushed.
async fn pending_by_kind(core: &CoreCtx) -> BTreeMap<String, u64> {
    jobs_snapshot(&core.jobs_pool)
        .await
        .expect("job snapshot")
        .by_kind
        .into_iter()
        .filter(|(_, slice)| slice.pending > 0)
        .map(|(kind, slice)| (kind, slice.pending))
        .collect()
}

/// What `after` holds that `before` did not — the work one call left on
/// the queue.
fn delta(before: &BTreeMap<String, u64>, after: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for (kind, count) in after {
        let grew = count - before.get(kind).copied().unwrap_or(0);
        if grew > 0 {
            out.insert(kind.clone(), grew);
        }
    }
    out
}

/// **The two entry points enqueue the same work.**
///
/// Two pairs under one persona, folded two different ways, and the
/// queue delta around each call is compared. A fix to one path only
/// fails here, whichever path it was: the two deltas are asserted
/// against each other *and* against the literal `asset_fold: 1`, so
/// "both do nothing" cannot pass either.
#[tokio::test(flavor = "multi_thread")]
async fn both_fold_entry_points_leave_the_same_work_behind() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    for name in [
        "auto-keeper.png",
        "auto-loser.png",
        "hand-keeper.png",
        "hand-loser.png",
    ] {
        std::fs::write(corpus.join(name), b"placeholder\n").expect("write");
    }

    let db_path = tmp.path().join("asterism.db");
    let core = init_core_with(
        &db_path,
        Arc::new(LogEmitter),
        // No worker: an enqueue stays on the queue where it can be
        // counted, and the fold this test is about is the one the two
        // service calls ask for rather than one a worker performed.
        CoreMode::ReadOnly,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let (persona_id, persona_uuid) = persona(&core, "fold-symmetry").await;

    let mut ids = Vec::new();
    for (i, name) in [
        "auto-keeper.png",
        "auto-loser.png",
        "hand-keeper.png",
        "hand-loser.png",
    ]
    .iter()
    .enumerate()
    {
        ids.push(
            core.asset_service
                .add(
                    add_command(
                        &persona_id,
                        corpus.join(name).to_str().unwrap(),
                        1_786_000_000_000 + i as i64 * 1_000,
                    ),
                    &unattributed(),
                )
                .await
                .expect("add")
                .id,
        );
    }
    let (auto_keeper, auto_loser, hand_keeper, hand_loser) = (
        ids[0].clone(),
        ids[1].clone(),
        ids[2].clone(),
        ids[3].clone(),
    );

    // The question the automatic path answers. Raised through the
    // repository because raising it the production way means running the
    // fingerprint job, and this mode has no worker to run it — the
    // surface under test starts at the confirm, not at detection.
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(&db_path)
        .await
        .expect("second isle");
    let repo = asterism_infra::sqlite::repo::SqliteAssetRepository::new(isle.clone());
    let conflict = DuplicateConflict::raise(
        persona_uuid,
        AssetId::from_uuid(auto_loser.parse().expect("uuid")),
        AssetId::from_uuid(auto_keeper.parse().expect("uuid")),
        DuplicateAxis::Artefact,
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        None,
        chrono::Utc::now(),
    )
    .expect("raise a question about two distinct rows");
    assert!(
        repo.record_duplicate_conflict(&conflict)
            .await
            .expect("record the question"),
        "the question is new"
    );
    driver.shutdown().await.ok();

    let before_auto = pending_by_kind(&core).await;
    core.asset_service
        .resolve_duplicate_conflict(
            ResolveDuplicateConflictCommand {
                conflict_id: conflict.id.to_string(),
                resolution: ConflictResolution::Folded,
                keeper_id: Some(auto_keeper.clone()),
            },
            &unattributed(),
        )
        .await
        .expect("the automatic path accepts the fold");
    let after_auto = pending_by_kind(&core).await;
    let automatic = delta(&before_auto, &after_auto);

    let before_hand = pending_by_kind(&core).await;
    let run = core
        .asset_service
        .merge_assets(
            MergeAssetsCommand {
                keeper_id: hand_keeper.clone(),
                discard_ids: vec![hand_loser.clone()],
                member_ids: vec![hand_keeper.clone(), hand_loser.clone()],
                dry_run: false,
            },
            &unattributed(),
        )
        .await
        .expect("the manual path runs");
    assert!(
        run.committed && run.folded_ids == vec![hand_loser.clone()],
        "the fixture must actually fold on the manual path: {run:?}"
    );
    let after_hand = pending_by_kind(&core).await;
    let manual = delta(&before_hand, &after_hand);

    assert_eq!(
        automatic,
        BTreeMap::from([("asset_fold".to_string(), 1)]),
        "one fold job for one folded row, from the path that has always \
         had one"
    );
    assert_eq!(
        manual, automatic,
        "the two entry points to a fold owe the same after-effects, and \
         say so through the same job"
    );

    // A preview asks for none of it: nothing was written, so there is
    // nothing outside a transaction to tidy up.
    let before_preview = pending_by_kind(&core).await;
    core.asset_service
        .merge_assets(
            MergeAssetsCommand {
                keeper_id: hand_keeper.clone(),
                discard_ids: vec![auto_keeper.clone()],
                member_ids: vec![hand_keeper.clone(), auto_keeper.clone()],
                dry_run: true,
            },
            &unattributed(),
        )
        .await
        .expect("the preview runs");
    assert_eq!(
        delta(&before_preview, &pending_by_kind(&core).await),
        BTreeMap::new(),
        "a preview folded nothing, so it owes nothing"
    );
}

/// **A hand-merged row leaves the retrieval index**, through the job the
/// merge enqueued and the worker ran.
///
/// The probe is a second, read-only handle on the same index directory:
/// it sees exactly what was committed, which is what the search path
/// would see — and it sees it *under* the SQL filter that would hide the
/// stale document from every surface a person can reach.
///
/// The keeper is the disagreement. Both rows carry the search word, so
/// an assertion that simply found the index empty would pass against a
/// fold that wiped it, and against a fixture that never indexed
/// anything.
#[tokio::test(flavor = "multi_thread")]
async fn a_manual_merge_takes_the_headstone_out_of_search() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    // Real text, so the indexer has a body to write. The word is
    // nonsense on purpose: nothing else in a fresh library can match it.
    for name in ["keeper.txt", "loser.txt"] {
        std::fs::write(corpus.join(name), b"heliotrope marginalia\n").expect("write");
    }

    let tantivy = tmp.path().join("tantivy");
    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        // The worker is the thing under test: the merge enqueues, and
        // the claim is that what it enqueued does the cleaning.
        CoreMode::Full,
        Some(&tantivy),
    )
    .await
    .expect("init_core");
    let (persona_id, _) = persona(&core, "fold-search").await;

    let mut ids = Vec::new();
    for (i, name) in ["keeper.txt", "loser.txt"].iter().enumerate() {
        ids.push(
            core.asset_service
                .add(
                    add_command(
                        &persona_id,
                        corpus.join(name).to_str().unwrap(),
                        1_786_000_000_000 + i as i64 * 1_000,
                    ),
                    &unattributed(),
                )
                .await
                .expect("add")
                .id,
        );
    }
    let (keeper, loser) = (ids[0].clone(), ids[1].clone());
    let keeper_id = AssetId::from_uuid(keeper.parse().expect("uuid"));
    let loser_id = AssetId::from_uuid(loser.parse().expect("uuid"));

    let probe = |dir: std::path::PathBuf| async move {
        asterism_infra::search::TantivyIndex::open_read_only(dir)
            .expect("read-only handle")
            .retrieve(&RetrievalQuery {
                intent: RetrievalIntent::Text("heliotrope".to_string()),
                scope: None,
                k: 10,
            })
            .await
            .expect("retrieve")
            .candidates
            .into_iter()
            .map(|c| c.asset_id)
            .collect::<Vec<_>>()
    };

    // Wait for the index to hold both rows. Without this the assertion
    // afterwards would hold over documents that never existed.
    let mut indexed = Vec::new();
    for _ in 0..120 {
        indexed = probe(tantivy.clone()).await;
        if indexed.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        indexed.len(),
        2,
        "both rows must be retrievable before the merge: {indexed:?}"
    );

    core.asset_service
        .merge_assets(
            MergeAssetsCommand {
                keeper_id: keeper.clone(),
                discard_ids: vec![loser.clone()],
                member_ids: vec![keeper.clone(), loser.clone()],
                dry_run: false,
            },
            &unattributed(),
        )
        .await
        .expect("the merge runs");

    let mut left = Vec::new();
    for _ in 0..120 {
        left = probe(tantivy.clone()).await;
        if !left.contains(&loser_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !left.contains(&loser_id),
        "the headstone's document is still there 30s after the merge: {left:?}"
    );
    assert_eq!(
        left,
        vec![keeper_id],
        "and the keeper is still retrievable — a fold retires one row, \
         not the index: {left:?}"
    );
}
