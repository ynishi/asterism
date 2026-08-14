//! The pursuit lifecycle (#29), end to end through `CoreCtx`: open
//! (with parenthood and its persona wall), close-satisfied freezing
//! the kept set in canonical ascending order (asserted by dedupe:
//! two closes over the same members in different input orders share
//! one snapshot row), close-abandoned, reopen, derived standing, and
//! the restamp repair verb with its cross-persona refusal.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, ClosePursuitCommand, CreateDispatchCommand, CreateSnapshotCommand,
    OpenPursuitCommand, RegisterPersonaCommand, ReopenPursuitCommand, RestampDispatchCommand,
};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

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

#[tokio::test(flavor = "multi_thread")]
async fn the_lifecycle_round_trips_and_the_close_freeze_is_canonical() {
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
                pack_id: Some("e2e-pursuit-lifecycle".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let stranger = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "Stranger".into(),
                pack_id: Some("e2e-pursuit-stranger".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register second persona");

    let mut assets = Vec::new();
    for (name, at) in [("a.md", 1_785_000_000_000_i64), ("b.md", 1_785_000_000_100)] {
        let path = corpus.join(name);
        std::fs::write(&path, format!("# {name}\n")).expect("write asset file");
        let dto = core
            .asset_service
            .add(
                add_command(&persona.id, path.to_str().unwrap(), at),
                &unattributed(),
            )
            .await
            .expect("add asset");
        assets.push(dto.id);
    }

    // ---- open: intent up front, parenthood, the persona wall -------
    let root = core
        .pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: persona.id.clone(),
                parent_pursuit_id: None,
                title: Some("  hero line  ".into()),
                note: None,
            },
            &unattributed(),
        )
        .await
        .expect("open root pursuit");
    assert_eq!(root.standing, "open", "a fresh pursuit is open");
    assert_eq!(
        root.title.as_deref(),
        Some("hero line"),
        "labels are trimmed to one storable representation"
    );
    let child = core
        .pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: persona.id.clone(),
                parent_pursuit_id: Some(root.id.clone()),
                title: None,
                note: Some("spawned".into()),
            },
            &unattributed(),
        )
        .await
        .expect("open child pursuit");
    assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));
    let crossing = core
        .pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: stranger.id.clone(),
                parent_pursuit_id: Some(root.id.clone()),
                title: None,
                note: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        crossing.is_err(),
        "parenthood never crosses personas: {crossing:?}"
    );

    // ---- close-satisfied: the freeze is canonical ------------------
    // Two pursuits, same kept members, opposite input orders. The
    // close path sorts ascending before freezing, so both conclusions
    // are one snapshot row — that is the dedupe the convention buys.
    let forward = core
        .pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: root.id.clone(),
                outcome: "satisfied".into(),
                kept_asset_ids: vec![assets[0].clone(), assets[1].clone()],
                note: Some("kept both".into()),
            },
            &unattributed(),
        )
        .await
        .expect("close root satisfied");
    let backward = core
        .pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: child.id.clone(),
                outcome: "satisfied".into(),
                kept_asset_ids: vec![assets[1].clone(), assets[0].clone()],
                note: None,
            },
            &unattributed(),
        )
        .await
        .expect("close child satisfied");
    let frozen = forward.snapshot_id.expect("a kept set freezes");
    assert_eq!(
        backward.snapshot_id.as_deref(),
        Some(frozen.as_str()),
        "identical kept sets dedupe to one snapshot regardless of input order"
    );
    assert_eq!(
        core.pursuit_service
            .get(&root.id)
            .await
            .expect("get closed root")
            .standing,
        "closed_satisfied"
    );

    // A dispatch input frozen from the same members in pick order is a
    // different statement — and when the pick order differs from
    // ascending, a different snapshot row.
    let picked = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![assets[1].clone(), assets[0].clone()],
            },
            &unattributed(),
        )
        .await
        .expect("freeze pick-ordered input");
    assert_ne!(
        picked.id, frozen,
        "core keeps caller order as identity; the ascending close is the forge's own convention"
    );

    // ---- reopen, close-abandoned, the empty conclusion -------------
    core.pursuit_service
        .reopen(
            ReopenPursuitCommand {
                pursuit_id: root.id.clone(),
                note: Some("second thoughts".into()),
            },
            &unattributed(),
        )
        .await
        .expect("reopen root");
    assert_eq!(
        core.pursuit_service.get(&root.id).await.unwrap().standing,
        "open",
        "reopen re-derives to open"
    );
    let rejected = core
        .pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: root.id.clone(),
                outcome: "abandoned".into(),
                kept_asset_ids: vec![assets[0].clone()],
                note: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        rejected.is_err(),
        "an abandoned close keeps nothing: {rejected:?}"
    );
    let nothing_kept = core
        .pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: root.id.clone(),
                outcome: "satisfied".into(),
                kept_asset_ids: Vec::new(),
                note: Some("concluded with nothing kept".into()),
            },
            &unattributed(),
        )
        .await
        .expect("an empty conclusion is a defined state");
    assert_eq!(nothing_kept.snapshot_id, None);
    let abandoned = core
        .pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: child.id.clone(),
                outcome: "abandoned".into(),
                kept_asset_ids: Vec::new(),
                note: Some("dropped".into()),
            },
            &unattributed(),
        )
        .await
        .expect("close child abandoned");
    assert_eq!(abandoned.snapshot_id, None);
    assert_eq!(
        core.pursuit_service.get(&child.id).await.unwrap().standing,
        "closed_abandoned"
    );

    // ---- the log, whole ---------------------------------------------
    let history: Vec<String> = core
        .pursuit_service
        .events(&root.id)
        .await
        .expect("root history")
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        history,
        vec!["closed_satisfied", "reopened", "closed_satisfied"],
        "every fact stays; standing is a projection, not an edit"
    );
    let listed = core
        .pursuit_service
        .list(&persona.id, 10)
        .await
        .expect("list persona pursuits");
    // The two explicit opens, most-recent first, each with its own
    // derived standing (root's latest fact is the nothing-kept close).
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, child.id);
    assert_eq!(listed[0].standing, "closed_abandoned");
    assert_eq!(listed[1].standing, "closed_satisfied");

    // ---- restamp: the repair verb through the service --------------
    let round = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: picked.id.clone(),
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
    assert_ne!(minted, root.id, "an unstamped request minted fresh");
    let moved = core
        .pursuit_service
        .restamp_dispatch(
            RestampDispatchCommand {
                dispatch_id: round.id.clone(),
                to_pursuit_id: root.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("re-file the round under the named line of work");
    assert_eq!(
        moved.pursuit_id.as_deref(),
        Some(root.id.as_str()),
        "the stamp moved; the recorded move carries the prior filing"
    );
    let foreign = core
        .pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: stranger.id.clone(),
                parent_pursuit_id: None,
                title: None,
                note: None,
            },
            &unattributed(),
        )
        .await
        .expect("open a stranger's pursuit");
    let crossing_restamp = core
        .pursuit_service
        .restamp_dispatch(
            RestampDispatchCommand {
                dispatch_id: round.id.clone(),
                to_pursuit_id: foreign.id.clone(),
            },
            &unattributed(),
        )
        .await;
    assert!(
        crossing_restamp.is_err(),
        "a filing never leaves its persona: {crossing_restamp:?}"
    );

    // ---- intent pinned: closed is standing, not a wall --------------
    // Filing a new round under a closed pursuit is deliberately legal:
    // a close is a fact about a moment, not a lock, and work after it
    // changes live standing rather than being refused. This assert
    // exists so a future reader treats the absence of a guard as the
    // design, not as a gap to fix.
    let late_round = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: picked.id.clone(),
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: None,
                pursuit_id: Some(child.id.clone()),
            },
            &unattributed(),
        )
        .await
        .expect("a closed pursuit still accepts new rounds");
    assert_eq!(late_round.pursuit_id.as_deref(), Some(child.id.as_str()));

    // ---- the kept set is validated like every other freeze ----------
    let ghost_kept = core
        .pursuit_service
        .close(
            ClosePursuitCommand {
                pursuit_id: child.id.clone(),
                outcome: "satisfied".into(),
                kept_asset_ids: vec!["0198c1c2-beef-7000-8000-00000000beef".into()],
                note: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        ghost_kept.is_err(),
        "keeping an asset the library does not hold is refused: {ghost_kept:?}"
    );
}
