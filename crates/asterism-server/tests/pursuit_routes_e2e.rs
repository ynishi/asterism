//! The pursuit verbs over HTTP (#34).
//!
//! `pursuit_lifecycle_e2e` already drives the same verbs through
//! `CoreCtx` and owns the domain semantics. What is under test here is
//! everything between a request and that service: method, path,
//! extractor, attribution translation, and the JSON that comes back.
//! Those are exactly what a service-level test cannot see, and the
//! defect this issue fixes was of that kind — a whole family persisted
//! with no route to reach it.
//!
//! Driven through the real [`asterism_server::http::router`] with
//! `oneshot`, so no port is bound.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_contract::dto::PersonaDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution the fixtures write with: a caller that states
/// nothing records nothing. The routes under test do their own
/// translation from the command body.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// One core over its own tempdir, the router on top of it, and the
/// persona the scenario acts as. The tempdir rides back so it outlives
/// the test body.
async fn harness(tag: &str) -> (tempfile::TempDir, CoreCtx, Router, PersonaDto) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let persona = register(&core, tag).await;
    (tmp, core, router, persona)
}

async fn register(core: &CoreCtx, tag: &str) -> PersonaDto {
    core.persona_service
        .register(
            RegisterPersonaCommand {
                name: tag.into(),
                pack_id: Some(format!("e2e-pursuit-routes-{tag}")),
            },
            &unattributed(),
        )
        .await
        .expect("register persona")
}

/// Registers one on-disk asset for the persona and returns its id.
async fn seed_asset(core: &CoreCtx, dir: &std::path::Path, persona_id: &str, name: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, format!("# {name}\n")).expect("write asset file");
    core.asset_service
        .add(
            add_command(persona_id, path.to_str().unwrap(), None),
            &unattributed(),
        )
        .await
        .expect("add asset")
        .id
}

fn add_command(persona_id: &str, locator: &str, derived_from: Option<String>) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
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
        derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

async fn call(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "body is not JSON ({e}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, json)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build GET")
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST")
}

fn str_at<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{key} missing or not a string in {value}"))
}

/// Open → close → reopen, entirely over HTTP, with the reads answering
/// from the same rows.
///
/// The standing assertions are the point: it is derived from the event
/// log on every read, so a route that returned a stored status column
/// (there is none) or read a different pursuit would show up here.
#[tokio::test(flavor = "multi_thread")]
async fn the_lifecycle_round_trips_over_http() {
    let (tmp, core, router, persona) = harness("lifecycle").await;
    let kept = seed_asset(&core, tmp.path(), &persona.id, "kept.md").await;

    let (status, opened) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({
                "persona_id": persona.id,
                "title": "  hero line  ",
                "operator_ai": "claude-code",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open: {opened}");
    assert_eq!(
        opened.get("standing").and_then(|v| v.as_str()),
        Some("open")
    );
    assert_eq!(
        opened.get("title").and_then(|v| v.as_str()),
        Some("hero line"),
        "the service's trimming is what the route hands back"
    );
    let pursuit_id = str_at(&opened, "id").to_string();

    // The listing sees it, standing included, without a limit stated.
    let (status, listed) = call(
        &router,
        get(&format!("/asterism/pursuits?persona_id={}", persona.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list: {listed}");
    let rows = listed.as_array().expect("array");
    assert_eq!(rows.len(), 1, "one pursuit, one row: {listed}");
    assert_eq!(str_at(&rows[0], "id"), pursuit_id);

    // The asset enters the ledger first — what a line of work is on
    // is derived from its own gestures, never supplied.
    let (status, entered) = call(
        &router,
        post(
            "/asterism/pursuits/tx",
            serde_json::json!({
                "pursuit_id": pursuit_id,
                "kind": "in",
                "asset_id": kept,
                "origin": "imported",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tx in: {entered}");
    assert_eq!(entered.get("kind").and_then(|v| v.as_str()), Some("in"));

    let (status, closed) = call(
        &router,
        post(
            "/asterism/pursuits/close",
            serde_json::json!({
                "pursuit_id": pursuit_id,
                "outcome": "satisfied",
                "note": "this one",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "close: {closed}");
    assert_eq!(
        closed.get("kind").and_then(|v| v.as_str()),
        Some("closed_satisfied")
    );
    assert_eq!(
        closed.get("snapshot_id"),
        Some(&serde_json::Value::Null),
        "the close freezes nothing: {closed}"
    );

    let (_, after_close) = call(&router, get(&format!("/asterism/pursuits/{pursuit_id}"))).await;
    assert_eq!(
        after_close.get("standing").and_then(|v| v.as_str()),
        Some("closed_satisfied"),
        "standing re-derives from the event just written"
    );

    // Closing again is a second fact, not an overwrite.
    let (status, _) = call(
        &router,
        post(
            "/asterism/pursuits/close",
            serde_json::json!({
                "pursuit_id": pursuit_id,
                "outcome": "satisfied",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, reopened) = call(
        &router,
        post(
            "/asterism/pursuits/reopen",
            serde_json::json!({ "pursuit_id": pursuit_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reopen: {reopened}");

    // And a reopen on an open pursuit: a legal fact that moves nothing.
    let (status, _) = call(
        &router,
        post(
            "/asterism/pursuits/reopen",
            serde_json::json!({ "pursuit_id": pursuit_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, events) = call(
        &router,
        get(&format!("/asterism/pursuits/{pursuit_id}/events")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "events: {events}");
    let kinds: Vec<&str> = events
        .as_array()
        .expect("array")
        .iter()
        .map(|e| str_at(e, "kind"))
        .collect();
    assert_eq!(
        kinds,
        vec![
            "closed_satisfied",
            "closed_satisfied",
            "reopened",
            "reopened"
        ],
        "four acts, oldest first, none of them overwritten"
    );

    let (status, view) = call(
        &router,
        get(&format!("/asterism/pursuits/{pursuit_id}/view")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "view: {view}");
    assert_eq!(
        view.get("pursuit")
            .and_then(|p| p.get("standing"))
            .and_then(|v| v.as_str()),
        Some("open"),
        "the last act was a reopen"
    );
    assert_eq!(
        view.get("events").and_then(|e| e.as_array()).map(Vec::len),
        Some(4)
    );
    assert!(
        view.get("rounds").is_none(),
        "the view stopped carrying rounds when the forge stopped \
         dispatching: {view}"
    );

    // The ledger the closes left alone still answers for what entered.
    assert_eq!(
        view.get("txs").and_then(|t| t.as_array()).map(Vec::len),
        Some(1),
        "the entry survives two closes: {view}"
    );

    // An unknown pursuit is a 404 rather than an empty answer.
    let (status, missing) = call(
        &router,
        get("/asterism/pursuits/0198c1c2-dead-7000-8000-00000000dead"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown pursuit: {missing}");
}

/// The close seen from the route: whichever outcome it states, it
/// writes an event and materialises nothing — no snapshot on the
/// event, and a ledger it leaves exactly where it was. An outcome the
/// domain does not know is refused.
#[tokio::test(flavor = "multi_thread")]
async fn close_records_a_fact_over_http_and_materialises_nothing() {
    let (tmp, core, router, persona) = harness("materialise").await;
    let a = seed_asset(&core, tmp.path(), &persona.id, "a.md").await;
    let b = seed_asset(&core, tmp.path(), &persona.id, "b.md").await;

    let open = |title: &str| {
        post(
            "/asterism/pursuits/open",
            serde_json::json!({ "persona_id": persona.id, "title": title }),
        )
    };
    let (_, first) = call(&router, open("first")).await;
    let (_, empty_handed) = call(&router, open("empty-handed")).await;

    for asset in [&a, &b] {
        let (status, entered) = call(
            &router,
            post(
                "/asterism/pursuits/tx",
                serde_json::json!({
                    "pursuit_id": str_at(&first, "id"),
                    "kind": "in",
                    "asset_id": asset,
                    "origin": "imported",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "tx in: {entered}");
    }

    let close = |pursuit: &str, outcome: &str| {
        post(
            "/asterism/pursuits/close",
            serde_json::json!({ "pursuit_id": pursuit, "outcome": outcome }),
        )
    };

    // A pursuit with two members, and a pursuit with none, conclude
    // the same way: an event carrying no frozen set.
    for (pursuit, outcome) in [
        (&first, "satisfied"),
        (&empty_handed, "satisfied"),
        (&empty_handed, "abandoned"),
    ] {
        let (status, closed) = call(&router, close(str_at(pursuit, "id"), outcome)).await;
        assert_eq!(status, StatusCode::OK, "close {outcome}: {closed}");
        assert_eq!(
            closed.get("snapshot_id"),
            Some(&serde_json::Value::Null),
            "the close freezes nothing: {closed}"
        );
    }

    // The members are still the pursuit's own, read back after it
    // closed — the close never reached for them.
    let (status, view) = call(
        &router,
        get(&format!("/asterism/pursuits/{}/view", str_at(&first, "id"))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "view: {view}");
    assert_eq!(
        view.get("txs").and_then(|t| t.as_array()).map(Vec::len),
        Some(2),
        "both entries outlive the close: {view}"
    );

    // An outcome the domain does not know is refused.
    let (status, refused) = call(&router, close(str_at(&first, "id"), "merged")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unknown outcome is refused: {refused}"
    );
}

/// Opening a pursuit at an id the caller names, and what happens when
/// that id is already taken.
///
/// The route has to carry `pursuit_id` through to the service and turn
/// the service's refusal into a `409` rather than a `500` — neither of
/// which a service-level test can see.
#[tokio::test(flavor = "multi_thread")]
async fn a_pursuit_opens_at_a_chosen_id_and_a_taken_one_is_a_conflict() {
    let (_tmp, _core, router, persona) = harness("claimed-id").await;

    let claimed = "0198c1c2-beef-7000-8000-00000000beef";
    let (status, adopted) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({
                "persona_id": persona.id,
                "pursuit_id": claimed,
                "title": "the line opened at a chosen id",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open at a chosen id: {adopted}");
    assert_eq!(str_at(&adopted, "id"), claimed);

    // Naming an id that is now taken is a conflict, not a merge into
    // the row that is already there.
    let (status, taken) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({ "persona_id": persona.id, "pursuit_id": claimed }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "id already in use: {taken}");
}
