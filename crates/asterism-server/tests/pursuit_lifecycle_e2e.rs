//! The pursuit lifecycle (#29), end to end through `CoreCtx`: open
//! with parenthood and its persona wall, close-satisfied freezing the
//! kept set canonically, close-abandoned and the empty conclusion,
//! the append-only event log with derived standing, and the restamp
//! repair verb.
//!
//! One test per scenario, each over its own core (an `init_core` per
//! test, as the sibling e2e files do) — a failure names its scenario
//! instead of hiding everything downstream of the first broken
//! assert, and no scenario's acts can shift another's expectations.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, ClosePursuitCommand, CreateDispatchCommand, CreateSnapshotCommand,
    CullVerdictEntry, OpenPursuitCommand, RecordPursuitTxCommand, RegisterPersonaCommand,
    ReopenPursuitCommand, RestampDispatchCommand,
};
use asterism_contract::dto::PersonaDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// One core over its own tempdir, plus the persona the scenario acts
/// as. The tempdir rides back so it outlives the test body.
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
    let persona = register(&core, tag).await;
    (tmp, core, persona)
}

async fn register(core: &CoreCtx, tag: &str) -> PersonaDto {
    core.persona_service
        .register(
            RegisterPersonaCommand {
                name: tag.into(),
                pack_id: Some(format!("e2e-pursuit-{tag}")),
            },
            &unattributed(),
        )
        .await
        .expect("register persona")
}

/// Registers one on-disk asset for the persona and returns its id.
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

async fn open(
    core: &CoreCtx,
    persona_id: &str,
    parent: Option<&str>,
    title: Option<&str>,
) -> asterism_contract::dto::PursuitDto {
    core.pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: persona_id.to_string(),
                pursuit_id: None,
                parent_pursuit_id: parent.map(str::to_string),
                title: title.map(str::to_string),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("open pursuit")
}

/// Brings an asset into a pursuit's ledger (`in` / `imported` — the
/// seeded fixtures came from outside).
async fn tx_in(core: &CoreCtx, pursuit_id: &str, asset_id: &str) {
    core.pursuit_service
        .record_tx(
            RecordPursuitTxCommand {
                pursuit_id: pursuit_id.to_string(),
                kind: "in".into(),
                asset_id: asset_id.to_string(),
                origin: Some("imported".into()),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("record in");
}

/// A `keep` verdict entry.
fn keep(asset_id: &str) -> CullVerdictEntry {
    CullVerdictEntry {
        asset_id: asset_id.to_string(),
        verdict: "keep".into(),
        note: None,
    }
}

async fn close(
    core: &CoreCtx,
    pursuit_id: &str,
    outcome: &str,
    verdicts: Vec<CullVerdictEntry>,
) -> Result<asterism_contract::dto::PursuitEventDto, asterism_core::DomainError> {
    core.pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: pursuit_id.to_string(),
                outcome: outcome.into(),
                verdicts,
                note: None,
                cull_note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn open_names_intent_and_walls_parenthood() {
    let (_tmp, core, persona) = boot("open").await;

    let root = open(&core, &persona.id, None, Some("  hero line  ")).await;
    assert_eq!(root.standing, "open", "a fresh pursuit is open");
    assert_eq!(
        root.title.as_deref(),
        Some("hero line"),
        "labels are trimmed to one storable representation"
    );

    let child = open(&core, &persona.id, Some(&root.id), None).await;
    assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));

    let stranger = register(&core, "open-stranger").await;
    let crossing = core
        .pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: stranger.id.clone(),
                pursuit_id: None,
                parent_pursuit_id: Some(root.id.clone()),
                title: None,
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        crossing.is_err(),
        "parenthood never crosses personas: {crossing:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn close_satisfied_freezes_the_kept_set_canonically() {
    let (tmp, core, persona) = boot("freeze").await;
    let a = seed_asset(&core, &tmp, &persona.id, "a.md").await;
    let b = seed_asset(&core, &tmp, &persona.id, "b.md").await;

    // Two pursuits, same kept members, opposite verdict orders: the
    // close path derives the kept set from the `keep` verdicts and
    // freezes ascending, so both conclusions are one snapshot row —
    // the dedupe the convention buys.
    let first = open(&core, &persona.id, None, None).await;
    let second = open(&core, &persona.id, None, None).await;
    for pursuit in [&first, &second] {
        tx_in(&core, &pursuit.id, &a).await;
        tx_in(&core, &pursuit.id, &b).await;
    }
    let forward = close(&core, &first.id, "satisfied", vec![keep(&a), keep(&b)])
        .await
        .expect("close forward");
    let backward = close(&core, &second.id, "satisfied", vec![keep(&b), keep(&a)])
        .await
        .expect("close backward");
    let frozen = forward.snapshot_id.expect("a kept set freezes");
    assert_eq!(
        backward.snapshot_id.as_deref(),
        Some(frozen.as_str()),
        "identical kept sets dedupe to one snapshot regardless of input order"
    );
    assert_eq!(
        core.pursuit_service.get(&first.id).await.unwrap().standing,
        "closed_satisfied"
    );

    // A dispatch input frozen from the same members in pick order is a
    // different statement — and when the pick differs from ascending,
    // a different snapshot row.
    let picked = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![b, a],
            },
            &unattributed(),
        )
        .await
        .expect("freeze pick-ordered input");
    assert_ne!(
        picked.id, frozen,
        "core keeps caller order as identity; the ascending close is the forge's own convention"
    );

    // A verdict names a candidate: an asset the ledger never admitted
    // is refused, whether or not the library holds it.
    let ghost = close(
        &core,
        &first.id,
        "satisfied",
        vec![keep("0198c1c2-beef-7000-8000-00000000beef")],
    )
    .await;
    assert!(
        ghost.is_err(),
        "judging what never entered is refused: {ghost:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn abandonment_keeps_nothing_and_the_empty_conclusion_is_defined() {
    let (_tmp, core, persona) = boot("abandon").await;
    let pursuit = open(&core, &persona.id, None, None).await;

    let kept_on_abandon = close(
        &core,
        &pursuit.id,
        "abandoned",
        vec![keep("0198c1c2-beef-7000-8000-00000000beef")],
    )
    .await;
    assert!(
        kept_on_abandon.is_err(),
        "an abandoned close decides nothing: {kept_on_abandon:?}"
    );

    let nothing_kept = close(&core, &pursuit.id, "satisfied", Vec::new())
        .await
        .expect("an empty conclusion is a defined state");
    assert_eq!(nothing_kept.snapshot_id, None);

    let abandoned = close(&core, &pursuit.id, "abandoned", Vec::new())
        .await
        .expect("a repeat close is a new fact, not an error");
    assert_eq!(abandoned.snapshot_id, None);
    assert_eq!(
        core.pursuit_service
            .get(&pursuit.id)
            .await
            .unwrap()
            .standing,
        "closed_abandoned",
        "the latest fact wins"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_log_is_append_only_and_standing_is_a_projection() {
    let (_tmp, core, persona) = boot("log").await;
    let pursuit = open(&core, &persona.id, None, None).await;
    let other = open(&core, &persona.id, None, None).await;

    close(&core, &pursuit.id, "satisfied", Vec::new())
        .await
        .expect("first close");
    core.pursuit_service
        .reopen(
            ReopenPursuitCommand {
                pursuit_id: pursuit.id.clone(),
                note: Some("second thoughts".into()),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("reopen");
    assert_eq!(
        core.pursuit_service
            .get(&pursuit.id)
            .await
            .unwrap()
            .standing,
        "open",
        "reopen re-derives to open"
    );
    close(&core, &pursuit.id, "abandoned", Vec::new())
        .await
        .expect("second close");

    let history: Vec<String> = core
        .pursuit_service
        .events(&pursuit.id)
        .await
        .expect("history")
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        history,
        vec!["closed_satisfied", "reopened", "closed_abandoned"],
        "every fact stays; standing is a projection, not an edit"
    );

    let listed = core
        .pursuit_service
        .list(&persona.id, 10)
        .await
        .expect("list");
    assert_eq!(listed.len(), 2, "both explicit opens, most-recent first");
    assert_eq!(listed[0].id, other.id);
    assert_eq!(listed[0].standing, "open");
    assert_eq!(listed[1].id, pursuit.id);
    assert_eq!(listed[1].standing, "closed_abandoned");
}

#[tokio::test(flavor = "multi_thread")]
async fn restamp_refiles_a_round_and_the_walls_hold() {
    let (tmp, core, persona) = boot("restamp").await;
    let asset = seed_asset(&core, &tmp, &persona.id, "a.md").await;
    let input = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![asset],
            },
            &unattributed(),
        )
        .await
        .expect("freeze input");

    let round = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: input.id.clone(),
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: None,
                pursuit_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch (mints its own pursuit)");
    let minted = round.pursuit_id.clone().expect("always-mint stamped it");

    let target = open(&core, &persona.id, None, Some("the real line")).await;
    assert_ne!(minted, target.id, "an unstamped request minted fresh");
    let moved = core
        .pursuit_service
        .restamp_dispatch(
            RestampDispatchCommand {
                dispatch_id: round.id.clone(),
                to_pursuit_id: target.id.clone(),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("re-file the round under the named line of work");
    assert_eq!(moved.pursuit_id.as_deref(), Some(target.id.as_str()));

    let stranger = register(&core, "restamp-stranger").await;
    let foreign = open(&core, &stranger.id, None, None).await;
    let crossing = core
        .pursuit_service
        .restamp_dispatch(
            RestampDispatchCommand {
                dispatch_id: round.id.clone(),
                to_pursuit_id: foreign.id.clone(),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        crossing.is_err(),
        "a filing never leaves its persona: {crossing:?}"
    );

    // Closed is standing, not a lock: a new round files under a
    // closed pursuit and changes live standing rather than being
    // refused. Asserted so a future reader treats the missing guard
    // as the design, not as a gap to fix.
    close(&core, &target.id, "satisfied", Vec::new())
        .await
        .expect("close the target");
    let late = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: input.id,
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: None,
                pursuit_id: Some(target.id.clone()),
            },
            &unattributed(),
        )
        .await
        .expect("a closed pursuit still accepts new rounds");
    assert_eq!(late.pursuit_id.as_deref(), Some(target.id.as_str()));

    // The view composes what the record correlates: both rounds (the
    // restamped one and the late one), the close fact, and — with no
    // ingest in this scenario — an empty returns population, present
    // as a set rather than absent as a field.
    let opened = core
        .pursuit_service
        .view(&target.id)
        .await
        .expect("view the pursuit");
    assert_eq!(opened.pursuit.standing, "closed_satisfied");
    let round_ids: Vec<&str> = opened.rounds.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(round_ids, vec![round.id.as_str(), late.id.as_str()]);
    assert!(opened.returns.is_empty());
    assert_eq!(opened.events.len(), 1);
}
