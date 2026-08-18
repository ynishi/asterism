//! The cull record (#22, model on #63), end to end through `CoreCtx`:
//! the membership ledger's gestures and their refusals, the close
//! resolving verdicts against the ledger (removal's default reject,
//! salvage, the untouched staying silent, the existing-member rule),
//! the candidate set frozen from the ledger, and the per-asset
//! verdict history read.
//!
//! One test per scenario over its own core, as the sibling e2e files
//! do.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, ClosePursuitCommand, CullVerdictEntry, OpenPursuitCommand, PurgeAssetCommand,
    RecordPursuitTxCommand, RegisterPersonaCommand, TrashAssetCommand,
};
use asterism_contract::dto::PersonaDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

async fn boot(tag: &str) -> (tempfile::TempDir, CoreCtx, PersonaDto) {
    let tmp = tempfile::tempdir().expect("tempdir");
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
                name: tag.into(),
                pack_id: Some(format!("e2e-cull-{tag}")),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    (tmp, core, persona)
}

async fn seed_asset(
    core: &CoreCtx,
    tmp: &tempfile::TempDir,
    persona_id: &str,
    name: &str,
) -> String {
    let path = tmp.path().join(name);
    std::fs::write(&path, format!("# {name}\n")).expect("write asset file");
    core.asset_service
        .add(
            AddAssetCommand {
                persona_id: persona_id.to_string(),
                source_kind: "fs".into(),
                locator: path.to_str().unwrap().to_string(),
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
            },
            &unattributed(),
        )
        .await
        .expect("add asset")
        .id
}

async fn open(core: &CoreCtx, persona_id: &str) -> String {
    core.pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: persona_id.to_string(),
                pursuit_id: None,
                project_id: None,
                parent_pursuit_id: None,
                title: None,
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("open pursuit")
        .id
}

async fn tx(
    core: &CoreCtx,
    pursuit_id: &str,
    kind: &str,
    asset_id: &str,
    origin: Option<&str>,
) -> Result<asterism_contract::dto::PursuitTxDto, asterism_core::DomainError> {
    core.pursuit_service
        .record_tx(
            RecordPursuitTxCommand {
                pursuit_id: pursuit_id.to_string(),
                kind: kind.into(),
                asset_id: asset_id.to_string(),
                origin: origin.map(str::to_string),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
}

fn verdict(asset_id: &str, verdict: &str) -> CullVerdictEntry {
    CullVerdictEntry {
        asset_id: asset_id.to_string(),
        verdict: verdict.into(),
        note: None,
    }
}

async fn close_satisfied(
    core: &CoreCtx,
    pursuit_id: &str,
    verdicts: Vec<CullVerdictEntry>,
) -> Result<asterism_contract::dto::PursuitEventDto, asterism_core::DomainError> {
    core.pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: pursuit_id.to_string(),
                outcome: "satisfied".into(),
                verdicts,
                note: None,
                cull_note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
}

/// The ledger refuses gestures that misunderstand it, and derives
/// membership from the latest one that did not.
#[tokio::test(flavor = "multi_thread")]
async fn the_ledger_refuses_what_misreads_it() {
    let (tmp, core, persona) = boot("ledger").await;
    let a = seed_asset(&core, &tmp, &persona.id, "a.md").await;
    let pursuit = open(&core, &persona.id).await;

    let no_origin = tx(&core, &pursuit, "in", &a, None).await;
    assert!(no_origin.is_err(), "an in names its origin: {no_origin:?}");

    let removed_first = tx(&core, &pursuit, "remove", &a, None).await;
    assert!(
        removed_first.is_err(),
        "removing a non-member is refused: {removed_first:?}"
    );

    tx(&core, &pursuit, "in", &a, Some("imported"))
        .await
        .expect("enter");
    let twice = tx(&core, &pursuit, "in", &a, Some("imported")).await;
    assert!(
        twice.is_err(),
        "a present member does not re-enter: {twice:?}"
    );

    let unremoved = tx(&core, &pursuit, "unremove", &a, None).await;
    assert!(
        unremoved.is_err(),
        "unremoving an unremoved member is refused: {unremoved:?}"
    );

    tx(&core, &pursuit, "remove", &a, None)
        .await
        .expect("remove");
    let re_in = tx(&core, &pursuit, "in", &a, Some("imported")).await;
    assert!(
        re_in.is_err(),
        "a removed member re-enters by unremove, not a second in: {re_in:?}"
    );
    tx(&core, &pursuit, "unremove", &a, None)
        .await
        .expect("unremove restores");

    let reserved = tx(&core, &pursuit, "update", &a, None).await;
    assert!(
        reserved.is_err(),
        "update is reserved for the round-trip slice: {reserved:?}"
    );
}

/// The close resolves the verdicts: keep freezes, an unspoken removal
/// culls as reject, salvage overrides it, and the untouched member
/// gets no row at all.
#[tokio::test(flavor = "multi_thread")]
async fn the_close_resolves_defaults_salvage_and_silence() {
    let (tmp, core, persona) = boot("resolve").await;
    let kept = seed_asset(&core, &tmp, &persona.id, "kept.md").await;
    let doomed = seed_asset(&core, &tmp, &persona.id, "doomed.md").await;
    let saved = seed_asset(&core, &tmp, &persona.id, "saved.md").await;
    let untouched = seed_asset(&core, &tmp, &persona.id, "untouched.md").await;
    let pursuit = open(&core, &persona.id).await;
    for asset in [&kept, &doomed, &saved, &untouched] {
        tx(&core, &pursuit, "in", asset, Some("imported"))
            .await
            .expect("enter");
    }
    tx(&core, &pursuit, "remove", &doomed, None)
        .await
        .expect("remove doomed");
    tx(&core, &pursuit, "remove", &saved, None)
        .await
        .expect("remove saved");

    let event = close_satisfied(
        &core,
        &pursuit,
        vec![verdict(&kept, "keep"), verdict(&saved, "keep")],
    )
    .await
    .expect("close");

    let view = core.pursuit_service.view(&pursuit).await.expect("view");
    assert_eq!(view.culls.len(), 1, "one close, one cull");
    let cull = &view.culls[0];
    assert_eq!(
        cull.pursuit_event_id, event.id,
        "the cull belongs to the close event"
    );
    let mut verdicts: Vec<(&str, &str)> = cull
        .members
        .iter()
        .map(|m| (m.asset_id.as_str(), m.verdict.as_str()))
        .collect();
    verdicts.sort();
    let mut expected = vec![
        (kept.as_str(), "keep"),
        (saved.as_str(), "keep"),
        (doomed.as_str(), "reject"),
    ];
    expected.sort();
    assert_eq!(
        verdicts, expected,
        "keep as stated, salvage as stated, unspoken removal as reject — \
         and the untouched member absent"
    );

    // The kept set is exactly the keep verdicts; the candidate set is
    // everything the ledger admitted, removed members included.
    let kept_snapshot = event.snapshot_id.expect("keeps freeze");
    let frozen_kept = core
        .snapshot_service
        .get_snapshot(&kept_snapshot)
        .await
        .expect("kept snapshot");
    let mut kept_ids = frozen_kept.asset_ids.clone();
    kept_ids.sort();
    let mut expected_kept = vec![kept.clone(), saved.clone()];
    expected_kept.sort();
    assert_eq!(kept_ids, expected_kept);
    let frozen_candidates = core
        .snapshot_service
        .get_snapshot(&cull.candidate_snapshot_id)
        .await
        .expect("candidate snapshot");
    assert_eq!(
        frozen_candidates.asset_ids.len(),
        4,
        "the candidate set holds all four entries: {frozen_candidates:?}"
    );

    // The per-asset history answers the acceptance question; the
    // untouched member has none.
    let history = core
        .pursuit_service
        .asset_culls(&doomed, 10)
        .await
        .expect("asset culls");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].verdict, "reject");
    assert_eq!(history[0].pursuit_id, pursuit);
    assert_eq!(history[0].candidate_snapshot_id, cull.candidate_snapshot_id);
    let silent = core
        .pursuit_service
        .asset_culls(&untouched, 10)
        .await
        .expect("asset culls of the untouched");
    assert!(silent.is_empty(), "the act said nothing about it");
}

/// History outlives the asset: a ledger member purged mid-pursuit can
/// still be rejected — the verdict row carries no foreign key — but a
/// keep of one is refused, and the freezes hold only the survivors.
#[tokio::test(flavor = "multi_thread")]
async fn a_purged_member_can_be_rejected_but_never_kept() {
    let (tmp, core, persona) = boot("purged").await;
    let kept = seed_asset(&core, &tmp, &persona.id, "kept.md").await;
    let gone = seed_asset(&core, &tmp, &persona.id, "gone.md").await;
    let pursuit = open(&core, &persona.id).await;
    for asset in [&kept, &gone] {
        tx(&core, &pursuit, "in", asset, Some("imported"))
            .await
            .expect("enter");
    }
    core.asset_service
        .trash(
            TrashAssetCommand {
                asset_id: gone.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("trash");
    core.asset_service
        .purge(
            PurgeAssetCommand {
                asset_id: gone.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("purge");

    let keep_of_dead = close_satisfied(&core, &pursuit, vec![verdict(&gone, "keep")]).await;
    assert!(
        keep_of_dead.is_err(),
        "a purged member cannot be kept: {keep_of_dead:?}"
    );

    let event = close_satisfied(
        &core,
        &pursuit,
        vec![verdict(&kept, "keep"), verdict(&gone, "reject")],
    )
    .await
    .expect("rejecting dead history still closes");
    let view = core.pursuit_service.view(&pursuit).await.expect("view");
    let cull = &view.culls[0];
    assert!(
        cull.members
            .iter()
            .any(|m| m.asset_id == gone && m.verdict == "reject"),
        "the verdict row outlives the asset: {cull:?}"
    );
    let candidates = core
        .snapshot_service
        .get_snapshot(&cull.candidate_snapshot_id)
        .await
        .expect("candidate snapshot");
    assert_eq!(
        candidates.asset_ids,
        vec![kept.clone()],
        "the freeze holds the surviving members only"
    );
    assert!(event.snapshot_id.is_some(), "the keep froze");
}

/// The existing-member rule: keeping what the library already holds
/// is the untouched default, not a statement — except as salvage.
#[tokio::test(flavor = "multi_thread")]
async fn an_existing_member_takes_reject_only_except_salvage() {
    let (tmp, core, persona) = boot("existing").await;
    let held = seed_asset(&core, &tmp, &persona.id, "held.md").await;
    let pursuit = open(&core, &persona.id).await;
    tx(&core, &pursuit, "in", &held, Some("existing"))
        .await
        .expect("bring in");

    let kept = close_satisfied(&core, &pursuit, vec![verdict(&held, "keep")]).await;
    assert!(
        kept.is_err(),
        "keep of a present existing member is refused: {kept:?}"
    );

    tx(&core, &pursuit, "remove", &held, None)
        .await
        .expect("remove");
    let salvaged = close_satisfied(&core, &pursuit, vec![verdict(&held, "keep")])
        .await
        .expect("salvage is the exception");
    assert!(
        salvaged.snapshot_id.is_some(),
        "a salvaged keep freezes like any keep"
    );
}
