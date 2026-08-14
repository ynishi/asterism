//! What a stored id set does once one of the rows it names has been
//! folded.
//!
//! A Snapshot's membership is content: a fold does not rewrite it
//! (`a_fold_never_rewrites_a_snapshot` — "a content-addressed member set
//! must not be edited by a fold"), so afterwards the freeze still names
//! the headstone, correctly. Every surface that reads such a set was
//! then hydrating the headstone verbatim, which meant a Snapshot grid
//! drawing a row the grid refuses everywhere else, and an export
//! receiving the same artefact twice under two ids that name one row.
//!
//! # Why redirecting rather than dropping, and how these tests say so
//!
//! Dropping the headstone is the other obvious answer, and it produces
//! the same count in the ordinary case — a freeze holding both the
//! keeper and the row folded into it draws one card either way. So the
//! pair `a_freeze_holding_both_sides…` / `a_freeze_holding_only_the_folded_row…`
//! exists: the second names **only** the headstone, where dropping
//! yields zero members and redirecting yields one. Without that case
//! the tests could not choose between the two implementations.
//!
//! `CoreMode::ReadOnly` throughout: `merge_into` folds inside its own
//! transaction, so no worker is needed to reach the state under test,
//! and none running means the constellation sees the edges this fixture
//! wrote rather than a rebuild's.

use std::sync::{Arc, Mutex};

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, MergeAssetsCommand,
    RegisterPersonaCommand,
};
use asterism_dispatch_sdk::{
    Derived, DispatchContext, DispatchState, Exporter, ExporterError, Handle,
};
use asterism_infra::dispatch::{DispatchRunEnv, ExporterRegistry, ReEnqueue, run_dispatch_run};
use asterism_infra::sqlite;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing.
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

/// Three rows under one persona: the pair a merge will collapse, and a
/// bystander that is never folded and never a keeper.
struct Fixture {
    tmp: tempfile::TempDir,
    core: CoreCtx,
    router: Router,
    persona_id: String,
    keeper: String,
    loser: String,
    bystander: String,
}

impl Fixture {
    async fn three_rows() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let corpus = tmp.path().join("corpus");
        std::fs::create_dir_all(&corpus).expect("corpus dir");
        for name in ["keeper.png", "loser.png", "bystander.png"] {
            std::fs::write(corpus.join(name), b"placeholder\n").expect("write");
        }

        let core = init_core_with(
            &tmp.path().join("asterism.db"),
            Arc::new(LogEmitter),
            CoreMode::ReadOnly,
            Some(&tmp.path().join("tantivy")),
        )
        .await
        .expect("init_core");
        let router = asterism_server::http::router(ServerCtx::from_core(&core));

        let persona = core
            .persona_service
            .register(
                RegisterPersonaCommand {
                    name: "E2E".into(),
                    pack_id: Some(format!("e2e-named-ids-{}", uuid::Uuid::now_v7())),
                },
                &unattributed(),
            )
            .await
            .expect("register persona");

        let mut ids = Vec::new();
        for (i, name) in ["keeper.png", "loser.png", "bystander.png"]
            .iter()
            .enumerate()
        {
            ids.push(
                core.asset_service
                    .add(
                        add_command(
                            &persona.id,
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

        Self {
            tmp,
            core,
            router,
            persona_id: persona.id,
            keeper: ids[0].clone(),
            loser: ids[1].clone(),
            bystander: ids[2].clone(),
        }
    }

    /// Freezes exactly these ids, **before** any fold, so the stored
    /// membership is the one the merge will not rewrite.
    async fn freeze(&self, asset_ids: Vec<String>) -> String {
        self.core
            .snapshot_service
            .create(
                CreateSnapshotCommand {
                    persona_id: self.persona_id.clone(),
                    asset_ids,
                },
                &unattributed(),
            )
            .await
            .expect("freeze")
            .id
    }

    /// Folds `loser` into `keeper` through the manual merge verb.
    async fn merge(&self) {
        let run = self
            .core
            .asset_service
            .merge_assets(
                MergeAssetsCommand {
                    keeper_id: self.keeper.clone(),
                    discard_ids: vec![self.loser.clone()],
                    member_ids: vec![self.keeper.clone(), self.loser.clone()],
                    dry_run: false,
                },
                &unattributed(),
            )
            .await
            .expect("the merge runs");
        assert!(
            run.committed && run.folded_ids == vec![self.loser.clone()],
            "every test here starts from a fold that actually happened: {run:?}"
        );
    }
}

/// **A freeze holding both sides of a later merge draws one card.**
///
/// The keeper's, not the headstone's, and not two.
#[tokio::test(flavor = "multi_thread")]
async fn a_freeze_holding_both_sides_of_a_merge_draws_the_keeper_once() {
    let fx = Fixture::three_rows().await;
    let snapshot = fx.freeze(vec![fx.keeper.clone(), fx.loser.clone()]).await;
    fx.merge().await;

    let members = fx
        .core
        .snapshot_service
        .snapshot_members(&snapshot)
        .await
        .expect("members");
    assert_eq!(
        members.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![fx.keeper.as_str()],
        "the two ids name one row now, so the grid draws one card"
    );
}

/// **A freeze that named only the folded row still draws one card — the
/// keeper's.**
///
/// This is the case that chooses between the two implementations.
/// Dropping headstones answers `0` here; redirecting answers `1`. Both
/// answer `1` to the test above, which is why that one cannot stand
/// alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_freeze_holding_only_the_folded_row_draws_the_keeper() {
    let fx = Fixture::three_rows().await;
    let snapshot = fx
        .freeze(vec![fx.loser.clone(), fx.bystander.clone()])
        .await;
    fx.merge().await;

    let members = fx
        .core
        .snapshot_service
        .snapshot_members(&snapshot)
        .await
        .expect("members");
    assert_eq!(
        members.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![fx.keeper.as_str(), fx.bystander.as_str()],
        "the folded member resolves to its keeper and keeps its place; \
         dropping it would answer with the bystander alone"
    );
}

/// A queue that records instead of running — the dispatch runner needs
/// one, and nothing here reads it.
#[derive(Default)]
struct SilentReEnqueue;

#[async_trait::async_trait]
impl ReEnqueue for SilentReEnqueue {
    async fn reenqueue(
        &self,
        _dispatch_id: &asterism_core::domain::value::DispatchId,
    ) -> Result<(), asterism_core::error::DomainError> {
        Ok(())
    }
}

/// An exporter that records the inputs it was handed and produces
/// nothing. The whole claim of the dispatch test is about the slice the
/// runner materialises, so the backend is the one part that stands in.
#[derive(Default)]
struct RecordingExporter {
    seen: Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl Exporter for RecordingExporter {
    fn slug(&self) -> &str {
        "recording"
    }

    fn accepts(&self, _action: &str) -> bool {
        true
    }

    async fn dispatch(&self, ctx: DispatchContext<'_>) -> Result<Handle, ExporterError> {
        self.seen
            .lock()
            .expect("input log")
            .push(ctx.inputs.iter().map(|c| c.id.clone()).collect());
        Ok(Handle::new("recording", serde_json::json!({})))
    }

    async fn poll(
        &self,
        _ctx: DispatchContext<'_>,
        _handle: &Handle,
    ) -> Result<DispatchState, ExporterError> {
        Ok(DispatchState::Done)
    }

    async fn harvest(
        &self,
        _ctx: DispatchContext<'_>,
        _handle: &Handle,
    ) -> Result<Vec<Derived>, ExporterError> {
        Ok(Vec::new())
    }
}

/// **An export of a freeze that folded hands the exporter one input.**
///
/// Two ids naming one row is one artefact, and an exporter that received
/// it twice would write it twice — the concrete cost of hydrating a
/// stored id set verbatim.
#[tokio::test(flavor = "multi_thread")]
async fn an_export_of_a_folded_freeze_receives_one_input() {
    let fx = Fixture::three_rows().await;
    let snapshot = fx.freeze(vec![fx.keeper.clone(), fx.loser.clone()]).await;
    fx.merge().await;

    let exporter = Arc::new(RecordingExporter::default());
    let dispatch = fx
        .core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: snapshot.clone(),
                exporter_slug: "recording".into(),
                action: "write".into(),
                params_json: serde_json::json!({}).to_string(),
                operator_ai: None,
                pursuit_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("create dispatch");

    // The runner the composition root builds, with the registry swapped
    // for one holding the recorder. Everything reaching the input slice
    // is what `core_init` wires.
    let (isle, driver) = sqlite::open_and_migrate(&fx.tmp.path().join("asterism.db"))
        .await
        .expect("second isle");
    let env = DispatchRunEnv {
        registry: ExporterRegistry::single(exporter.clone()),
        service: Arc::new(
            asterism_core::application_support::DispatchRunnerService::new(
                Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
                Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
                Arc::new(sqlite::repo::SqliteAssetRepository::new(isle.clone())),
                Arc::new(sqlite::repo::SqliteEdgeRepository::new(isle.clone())),
                Arc::new(sqlite::repo::SqlitePersonaRepository::new(isle.clone())),
                fx.core.asset_service.clone(),
                Arc::new(SilentQueue),
            ),
        ),
        snapshots: Arc::new(sqlite::repo::SqliteSnapshotRepository::new(isle.clone())),
        dispatches: Arc::new(sqlite::repo::SqliteDispatchRepository::new(isle.clone())),
        assets: Arc::new(sqlite::repo::SqliteAssetRepository::new(isle.clone())),
        reenqueue: Arc::new(SilentReEnqueue),
    };

    let payload = serde_json::json!({ "dispatch_id": dispatch.id });
    let mut ticks = 0;
    loop {
        run_dispatch_run(&env, &payload)
            .await
            .expect("dispatch tick");
        ticks += 1;
        let dto = fx
            .core
            .dispatch_service
            .get(&dispatch.id)
            .await
            .expect("dispatch get");
        if matches!(dto.state.as_str(), "done" | "failed" | "cancelled") {
            assert_eq!(dto.state, "done", "the export ran");
            break;
        }
        assert!(ticks < 8, "dispatch did not reach a terminal state");
    }
    driver.shutdown().await.ok();

    let seen = exporter.seen.lock().expect("input log").clone();
    assert_eq!(
        seen,
        vec![vec![fx.keeper.clone()]],
        "one dispatch, one input: the freeze's two ids name one row"
    );
}

/// A job queue that swallows — `DispatchRunnerService` needs one and
/// this test asserts nothing about what reify enqueues.
struct SilentQueue;

#[async_trait::async_trait]
impl asterism_core::domain::repository::JobQueue for SilentQueue {
    async fn enqueue(
        &self,
        _kind: asterism_core::domain::job::JobKind,
        _payload: serde_json::Value,
    ) -> Result<String, asterism_core::error::DomainError> {
        Ok("task-1".into())
    }
}

/// **The constellation's `same_selection` sibling is the keeper.**
///
/// The freeze names the headstone and always will; what the burst draws
/// beside the subject is the row that id now means. The label assertion
/// rides along because the freeze can only be found by the id it stored
/// — a redirect that forgot to carry that back would draw the card with
/// no freeze named on it.
#[tokio::test(flavor = "multi_thread")]
async fn a_selection_sibling_is_the_keeper_and_still_names_its_freeze() {
    let fx = Fixture::three_rows().await;
    let snapshot = fx
        .freeze(vec![fx.bystander.clone(), fx.loser.clone()])
        .await;
    fx.merge().await;

    let burst = fx
        .core
        .asset_service
        .constellation_of(&fx.bystander, None, 20)
        .await
        .expect("constellation");
    let selection: Vec<_> = burst
        .iter()
        .filter(|item| item.edge.kind == "same_selection")
        .collect();
    assert_eq!(
        selection
            .iter()
            .map(|i| i.card.id.as_str())
            .collect::<Vec<_>>(),
        vec![fx.keeper.as_str()],
        "the sibling is the row the frozen id now names, not the headstone"
    );
    assert_eq!(
        selection[0].edge.label.as_deref(),
        Some(format!("snapshot · {}", &snapshot[..8]).as_str()),
        "and the freeze it came from is still named, though the freeze \
         holds the headstone's id and not this card's"
    );
}

/// **A headstone read by id says it was folded.**
///
/// Reads by id deliberately reach a headstone — that is what makes an
/// old reference resolvable — so the record has to carry the one field
/// that says the row is no longer part of the library. The live row is
/// the disagreement: it answers the same shape with the field absent, so
/// this cannot pass against a DTO that always fills it in.
#[tokio::test(flavor = "multi_thread")]
async fn a_headstone_read_by_id_says_it_was_folded() {
    let fx = Fixture::three_rows().await;
    fx.merge().await;

    let (status, body) = get(&fx.router, &format!("/asterism/assets/{}", fx.loser)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a headstone is still readable by id"
    );
    assert_eq!(
        body["asset"]["folded_into"], fx.keeper,
        "the record names the row this id now means: {body}"
    );
    assert_eq!(
        body["asset"]["fold_policy"], "auto",
        "nobody ruled these rows apart — they were merged: {body}"
    );

    let (status, live) = get(&fx.router, &format!("/asterism/assets/{}", fx.keeper)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        live["asset"]["folded_into"].is_null(),
        "a live row was folded into nothing, and says so by omission: {live}"
    );
    assert_eq!(
        live["asset"]["fold_policy"], "auto",
        "every row carries a policy; absence is not one of its values: {live}"
    );
}

async fn get(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build GET");
    let response = router.clone().oneshot(request).await.expect("router");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, body)
}
