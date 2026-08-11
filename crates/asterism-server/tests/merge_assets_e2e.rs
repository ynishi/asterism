//! End-to-end: the manual merge verb is reachable over HTTP, and the
//! contracts the wire keeps that the service e2e (`manual_merge_e2e`)
//! does not cover are asserted **on the transport itself**.
//!
//! The service e2e asserts the verb's semantics against the service
//! trait. This binary asserts what the HTTP boundary adds on top: that
//! `refusals` ride back on a `200 OK` body (an all-or-nothing merge that
//! could not touch a row is a decision the caller has to re-make, not a
//! call error), that a `MergePlan::declare` refusal shows up as `400`,
//! that `warnings` populate on the preview and disappear on the commit,
//! and that the two branches return the same shape a caller can read on.
//! Dropping any of these off the wire would let the panel infer state
//! from a status code the endpoint does not actually promise.
//!
//! `Full` mode throughout, for the same reason `manual_merge_e2e` uses
//! it: the pipeline being exercised is the one the worker runs.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, MergeAssetsCommand, RegisterPersonaCommand, TrashAssetCommand,
};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution these fixtures write with — the same one the service
/// e2e uses. The merge itself carries no per-caller record, so what
/// matters here is that the HTTP surface routes the write with **some**
/// attribution attached, and the shape of the answer never depends on
/// which caller it was.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(
    persona_id: &str,
    source_kind: &str,
    locator: &str,
    occurred_at_ms: i64,
    derived_from: Option<String>,
) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: source_kind.into(),
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
        derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

/// Three rows under one persona, plus a router over the same services
/// the HTTP surface publishes.
struct Fixture {
    tmp: tempfile::TempDir,
    core: CoreCtx,
    router: Router,
    keeper: String,
    /// The first row folded — the axis the warning teeth vary along.
    discard_a: String,
    /// The second row folded — a plain `fs` row for every test.
    discard_b: String,
}

impl Fixture {
    /// Ingests three rows: the keeper, then `discard_a` under
    /// `discard_source_kind` and optionally under a `derived_from`
    /// claim, then a plain `discard_b`.
    async fn three_assets(discard_source_kind: &str, discard_a_derived_from_keeper: bool) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let corpus = tmp.path().join("corpus");
        std::fs::create_dir_all(&corpus).expect("corpus dir");
        for name in ["keeper.png", "a.png", "b.png"] {
            std::fs::write(corpus.join(name), b"placeholder\n").expect("write");
        }

        let core = init_core_with(
            &tmp.path().join("asterism.db"),
            Arc::new(LogEmitter),
            CoreMode::Full,
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
                    pack_id: Some(format!("e2e-http-merge-{}", uuid::Uuid::now_v7())),
                },
                &unattributed(),
            )
            .await
            .expect("register persona");

        let keeper = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    "fs",
                    corpus.join("keeper.png").to_str().unwrap(),
                    1_786_000_000_000,
                    None,
                ),
                &unattributed(),
            )
            .await
            .expect("add keeper")
            .id;

        let derived_from = if discard_a_derived_from_keeper {
            Some(format!("asset:{keeper}"))
        } else {
            None
        };
        let discard_a = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    discard_source_kind,
                    corpus.join("a.png").to_str().unwrap(),
                    1_786_000_001_000,
                    derived_from,
                ),
                &unattributed(),
            )
            .await
            .expect("add discard_a")
            .id;

        let discard_b = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    "fs",
                    corpus.join("b.png").to_str().unwrap(),
                    1_786_000_002_000,
                    None,
                ),
                &unattributed(),
            )
            .await
            .expect("add discard_b")
            .id;

        Self {
            tmp,
            core,
            router,
            keeper,
            discard_a,
            discard_b,
        }
    }

    /// The command the http surface is exercised with — folds
    /// `discard_a` and `discard_b` into `keeper`.
    fn merge(&self, dry_run: bool) -> MergeAssetsCommand {
        MergeAssetsCommand {
            keeper_id: self.keeper.clone(),
            discard_ids: vec![self.discard_a.clone(), self.discard_b.clone()],
            member_ids: vec![
                self.keeper.clone(),
                self.discard_a.clone(),
                self.discard_b.clone(),
            ],
            dry_run,
        }
    }

    /// Reads `folded_into` off a row over a second connection — the
    /// fold's own effect, which no read DTO carries. Same shape as
    /// `manual_merge_e2e::folded_into`; the two binaries share the
    /// evidence but not the assertion (one is about the service, the
    /// other about what the wire agrees to say about the same event).
    async fn folded_into(&self, asset_id: &str) -> Option<String> {
        let (isle, driver) =
            asterism_infra::sqlite::open_and_migrate(&self.tmp.path().join("asterism.db"))
                .await
                .expect("second isle");
        let id: uuid::Uuid = asset_id.parse().expect("asset id");
        let keeper = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT folded_into FROM asset WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get::<_, Option<uuid::Uuid>>(0),
                )
            })
            .await
            .expect("read folded_into");
        driver.shutdown().await.ok();
        keeper.map(|k| k.to_string())
    }
}

async fn post(
    router: &Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST");
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

/// A plain three-row merge over the HTTP surface. The wire agrees on the
/// happy-path shape: `200 OK`, `committed: true`, both discards on
/// `folded_ids`, no warnings, no refusals.
///
/// The keeper's own `folded_into` is read off a second SQLite connection
/// afterwards — the DTO's `committed` flag is the very thing we would
/// be second-guessing, so an independent read is what makes this teeth
/// actual proof rather than the endpoint's own claim.
#[tokio::test(flavor = "multi_thread")]
async fn a_three_row_commit_returns_the_run_shape_and_the_fold_lands() {
    let fx = Fixture::three_assets("fs", false).await;

    let (status, body) = post(
        &fx.router,
        "/asterism/duplicates/merge",
        serde_json::to_value(fx.merge(false)).expect("serialise the merge"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "the commit answered: {body}");
    assert_eq!(body["committed"], true, "a run returns committed: {body}");
    assert_eq!(body["keeper_id"], fx.keeper);
    assert_eq!(
        body["folded_ids"]
            .as_array()
            .expect("folded_ids array")
            .len(),
        2,
        "both discards were folded on the wire's answer: {body}"
    );
    assert!(
        body["warnings"]
            .as_array()
            .expect("warnings array")
            .is_empty(),
        "no rule declined this pair and the commit branch never returns \
         warnings anyway: {body}"
    );
    assert!(
        body["refusals"]
            .as_array()
            .expect("refusals array")
            .is_empty(),
        "every row was foldable: {body}"
    );

    // The fold really landed. Reading `folded_into` off both discards
    // rather than trusting the DTO — otherwise this teeth is asserting
    // the endpoint's story against itself.
    assert_eq!(
        fx.folded_into(&fx.discard_a).await.as_deref(),
        Some(fx.keeper.as_str())
    );
    assert_eq!(
        fx.folded_into(&fx.discard_b).await.as_deref(),
        Some(fx.keeper.as_str())
    );
    assert_eq!(
        fx.folded_into(&fx.keeper).await,
        None,
        "the keeper stayed live"
    );
}

/// The `dry_run: true` branch returns the same shape as the commit,
/// distinguishable only by `committed: false`. That is the port
/// contract — a run following a preview reads the answer back on the
/// same fields — and it has to hold on the wire.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_returns_the_preview_shape_and_writes_nothing() {
    let fx = Fixture::three_assets("fs", false).await;

    let (status, body) = post(
        &fx.router,
        "/asterism/duplicates/merge",
        serde_json::to_value(fx.merge(true)).expect("serialise the preview"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "a preview is not an error: {body}");
    assert_eq!(
        body["committed"], false,
        "a preview reports a prediction, not a run"
    );
    assert!(
        body["warnings"]
            .as_array()
            .expect("warnings array")
            .is_empty(),
        "no rule declined this pair: {body}"
    );
    assert!(
        body["refusals"]
            .as_array()
            .expect("refusals array")
            .is_empty(),
        "every row was foldable when the plan was checked: {body}"
    );
    assert_eq!(
        body["folded_ids"]
            .as_array()
            .expect("folded_ids array")
            .len(),
        2,
        "the preview names what it would fold: {body}"
    );

    // Nothing was written — read `folded_into` off both discards
    // rather than trusting `committed: false`.
    assert_eq!(fx.folded_into(&fx.discard_a).await, None);
    assert_eq!(fx.folded_into(&fx.discard_b).await, None);
}

/// A pair the lineage rule would have declined an automatic fold of
/// warns on the preview. `warnings` is populated with the exact pair
/// and slug, on `200 OK`; the rule is not binding on a person's ruling.
#[tokio::test(flavor = "multi_thread")]
async fn a_lineage_pair_warns_on_the_preview_over_http() {
    // `discard_a` is derived from `keeper` — same shape the service
    // e2e's fixture uses. The rule catches this pair, the response has
    // to say so.
    let fx = Fixture::three_assets("fs", true).await;

    let (status, body) = post(
        &fx.router,
        "/asterism/duplicates/merge",
        serde_json::to_value(fx.merge(true)).expect("serialise the preview"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "a warning is not an error: {body}");
    assert_eq!(body["committed"], false);
    let warnings = body["warnings"]
        .as_array()
        .expect("warnings array on the DTO");
    assert_eq!(warnings.len(), 1, "one pair, one warning: {body}");
    let warning = &warnings[0];
    assert_eq!(warning["keeper_id"], fx.keeper);
    assert_eq!(warning["headstone_id"], fx.discard_a);
    assert_eq!(
        warning["kind"], "lineage",
        "the slug the wire uses is the enum's — not a rendered sentence: {body}"
    );
    // The preview still names the rows it would fold; the rule warned,
    // it did not refuse the merge.
    assert_eq!(
        body["folded_ids"]
            .as_array()
            .expect("folded_ids array")
            .len(),
        2,
        "the preview folds anyway — the rule is not binding: {body}"
    );
}

/// A member set that does not equal `{keeper} ∪ discards` is refused by
/// [`MergePlan::declare`] before any write. On the wire this is a
/// `400` with `kind: Validation` — the same `ApiError` shape every
/// other validation refusal takes, so a caller does not need to
/// special-case this verb.
#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_declaration_is_a_four_hundred() {
    let fx = Fixture::three_assets("fs", false).await;

    // Discard both rows but declare only two members — the caller
    // ticked two rows on a screen that had three. `MergePlan::declare`
    // catches this above the transaction; the HTTP boundary reports it
    // as a validation refusal.
    let bad = MergeAssetsCommand {
        keeper_id: fx.keeper.clone(),
        discard_ids: vec![fx.discard_a.clone(), fx.discard_b.clone()],
        member_ids: vec![fx.keeper.clone(), fx.discard_a.clone()],
        dry_run: true,
    };

    let (status, body) = post(
        &fx.router,
        "/asterism/duplicates/merge",
        serde_json::to_value(&bad).expect("serialise"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a malformed declaration is not a 200: {body}"
    );
    assert_eq!(
        body["kind"], "Validation",
        "the ApiError shape names Validation for MergePlan::declare's refusals: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("does not account for every declared member"),
        "the message carries the refusal's own words: {body}"
    );

    // Nothing was written on either discard — the check ran above the
    // transaction, and the HTTP boundary did not swallow the error.
    assert_eq!(fx.folded_into(&fx.discard_a).await, None);
    assert_eq!(fx.folded_into(&fx.discard_b).await, None);
}

/// **Refusals ride on a `200 OK` body, not on the status code.**
///
/// This is the wire-level contract the whole surface hinges on: an
/// all-or-nothing merge that could not touch a row is a decision the
/// caller has to re-make, and returning `409` (or `422`) would let
/// panels treat it as a call error and never surface the reason.
/// The response is the same DTO shape as the
/// happy path, `refusals` non-empty, `committed: false`.
///
/// Trashing the keeper is what forces every discard to refuse with
/// `KeeperTrashed`. It is not the only way a refusal happens, but it
/// is deterministic and reads out the same shape every refusal does.
#[tokio::test(flavor = "multi_thread")]
async fn refusals_ride_on_a_two_hundred_ok_body() {
    let fx = Fixture::three_assets("fs", false).await;

    // Trash the keeper: every discard now refuses with `KeeperTrashed`.
    fx.core
        .asset_service
        .trash(
            TrashAssetCommand {
                asset_id: fx.keeper.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("trash keeper");

    let (status, body) = post(
        &fx.router,
        "/asterism/duplicates/merge",
        serde_json::to_value(fx.merge(false)).expect("serialise the commit"),
    )
    .await;

    // The load-bearing claim. If this ever flips to a 4xx, the whole
    // wire contract has to be re-explained to the panel.
    assert_eq!(
        status,
        StatusCode::OK,
        "refusals are a response field, not a status: {body}"
    );
    assert_eq!(
        body["committed"], false,
        "a refused merge is never committed: {body}"
    );
    let refusals = body["refusals"]
        .as_array()
        .expect("refusals array on the DTO");
    assert_eq!(
        refusals.len(),
        2,
        "both discards refused for the same reason (keeper trashed): {body}"
    );
    for refusal in refusals {
        assert_eq!(
            refusal["reason"], "the keeper is in the trash",
            "the slug names why: {body}"
        );
    }
    // All-or-nothing: `folded_ids` is empty because one refusal
    // abandons the whole merge.
    assert!(
        body["folded_ids"]
            .as_array()
            .expect("folded_ids array")
            .is_empty(),
        "the all-or-nothing rule leaves nothing on the folded list: {body}"
    );

    // And no discard moved on either row — the response's committed
    // flag agreed, and the storage layer agrees with it.
    assert_eq!(fx.folded_into(&fx.discard_a).await, None);
    assert_eq!(fx.folded_into(&fx.discard_b).await, None);
}
