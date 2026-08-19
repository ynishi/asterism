//! The pursuit lifecycle (#29), end to end through `CoreCtx`: open
//! with parenthood and its persona wall, the close that records a
//! fact and leaves the ledger alone, either outcome and the latest
//! one winning, and the append-only event log with derived standing.
//!
//! One test per scenario, each over its own core (an `init_core` per
//! test, as the sibling e2e files do) — a failure names its scenario
//! instead of hiding everything downstream of the first broken
//! assert, and no scenario's acts can shift another's expectations.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, ClosePursuitCommand, OpenPursuitCommand, RecordPursuitTxCommand,
    RegisterPersonaCommand, ReopenPursuitCommand,
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
                project_id: None,
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

async fn close(
    core: &CoreCtx,
    pursuit_id: &str,
    outcome: &str,
) -> Result<asterism_contract::dto::PursuitEventDto, asterism_core::DomainError> {
    core.pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: pursuit_id.to_string(),
                outcome: outcome.into(),
                note: None,
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
                project_id: None,
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
async fn close_records_the_fact_and_leaves_the_ledger_where_it_is() {
    let (tmp, core, persona) = boot("close").await;
    let a = seed_asset(&core, &tmp, &persona.id, "a.md").await;
    let b = seed_asset(&core, &tmp, &persona.id, "b.md").await;

    let pursuit = open(&core, &persona.id, None, None).await;
    tx_in(&core, &pursuit.id, &a).await;
    tx_in(&core, &pursuit.id, &b).await;

    let closed = close(&core, &pursuit.id, "satisfied")
        .await
        .expect("close satisfied");
    assert_eq!(
        closed.snapshot_id, None,
        "the close freezes nothing: {closed:?}"
    );
    assert_eq!(
        core.pursuit_service
            .get(&pursuit.id)
            .await
            .unwrap()
            .standing,
        "closed_satisfied"
    );

    // What the line of work was on is still readable afterwards,
    // because the close never touched it.
    let view = core
        .pursuit_service
        .view(&pursuit.id)
        .await
        .expect("view the closed pursuit");
    assert_eq!(
        view.txs.len(),
        2,
        "both entries survive the close: {:?}",
        view.txs
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn either_outcome_records_a_fact_and_the_latest_one_wins() {
    let (_tmp, core, persona) = boot("abandon").await;
    let pursuit = open(&core, &persona.id, None, None).await;

    let satisfied = close(&core, &pursuit.id, "satisfied")
        .await
        .expect("a close over an empty ledger is a defined state");
    assert_eq!(satisfied.snapshot_id, None);

    let abandoned = close(&core, &pursuit.id, "abandoned")
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

    close(&core, &pursuit.id, "satisfied")
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
    close(&core, &pursuit.id, "abandoned")
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

/// A closed pursuit is a standing, not a lock — the ledger still
/// accepts a gesture afterwards, and the view still reads it back.
///
/// Nothing consults standing on the write path, so closing changes what
/// the pursuit reads as and refuses nothing. Asserted so a future reader
/// treats the missing guard as the design, not as a gap to fix.
#[tokio::test(flavor = "multi_thread")]
async fn a_closed_pursuit_is_a_standing_and_not_a_lock() {
    let (tmp, core, persona) = boot("closed-standing").await;
    let asset = seed_asset(&core, &tmp, &persona.id, "a.md").await;

    let target = open(&core, &persona.id, None, Some("the real line")).await;
    close(&core, &target.id, "satisfied")
        .await
        .expect("close the target");
    tx_in(&core, &target.id, &asset).await;

    // The view composes what the record holds: the close fact, and the
    // gesture the close did not refuse.
    let opened = core
        .pursuit_service
        .view(&target.id)
        .await
        .expect("view the pursuit");
    assert_eq!(opened.pursuit.standing, "closed_satisfied");
    assert_eq!(opened.events.len(), 1);
    assert_eq!(opened.txs.len(), 1, "a close refuses no later gesture");
}
