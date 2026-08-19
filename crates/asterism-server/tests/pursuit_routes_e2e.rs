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
use asterism_contract::sidecar::{SIDECAR_IDENTITY_KEY, SIDECAR_SCHEMA, SIDECAR_SUFFIX};
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

    // The kept asset enters the ledger first — a verdict names a
    // candidate, and the candidate set is derived, never supplied.
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
                "verdicts": [{ "asset_id": kept, "verdict": "keep" }],
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
    let frozen = str_at(&closed, "snapshot_id").to_string();

    let (_, after_close) = call(&router, get(&format!("/asterism/pursuits/{pursuit_id}"))).await;
    assert_eq!(
        after_close.get("standing").and_then(|v| v.as_str()),
        Some("closed_satisfied"),
        "standing re-derives from the event just written"
    );

    // Closing again is a second fact, not an overwrite — the ledger
    // still holds the candidate, so the same verdict stands again.
    let (status, _) = call(
        &router,
        post(
            "/asterism/pursuits/close",
            serde_json::json!({
                "pursuit_id": pursuit_id,
                "outcome": "satisfied",
                "verdicts": [{ "asset_id": kept, "verdict": "keep" }],
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
    assert_eq!(
        view.get("rounds").and_then(|r| r.as_array()).map(Vec::len),
        Some(0),
        "nothing was dispatched under this line of work"
    );

    // The freeze the close produced is a real snapshot, readable by id.
    let (status, snapshot) = call(&router, get(&format!("/asterism/snapshots/{frozen}"))).await;
    assert_eq!(status, StatusCode::OK, "frozen set: {snapshot}");

    // An unknown pursuit is a 404 rather than an empty answer.
    let (status, missing) = call(
        &router,
        get("/asterism/pursuits/0198c1c2-dead-7000-8000-00000000dead"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown pursuit: {missing}");
}

/// The close's materialisation, seen from the route: the kept set is
/// canonicalised before freezing, so two closes stating the same
/// members in different order share one snapshot — and a close that
/// kept nothing records no snapshot at all.
#[tokio::test(flavor = "multi_thread")]
async fn close_freezes_canonically_and_records_nothing_kept() {
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
    let (_, second) = call(&router, open("second")).await;
    let (_, empty_handed) = call(&router, open("empty-handed")).await;

    for pursuit in [&first, &second] {
        for asset in [&a, &b] {
            let (status, entered) = call(
                &router,
                post(
                    "/asterism/pursuits/tx",
                    serde_json::json!({
                        "pursuit_id": str_at(pursuit, "id"),
                        "kind": "in",
                        "asset_id": asset,
                        "origin": "imported",
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "tx in: {entered}");
        }
    }

    let close = |pursuit: &str, kept: Vec<&str>| {
        let verdicts: Vec<serde_json::Value> = kept
            .iter()
            .map(|a| serde_json::json!({ "asset_id": a, "verdict": "keep" }))
            .collect();
        post(
            "/asterism/pursuits/close",
            serde_json::json!({
                "pursuit_id": pursuit,
                "outcome": "satisfied",
                "verdicts": verdicts,
            }),
        )
    };
    let (_, one) = call(&router, close(str_at(&first, "id"), vec![&a, &b])).await;
    let (_, other) = call(&router, close(str_at(&second, "id"), vec![&b, &a])).await;
    assert_eq!(
        str_at(&one, "snapshot_id"),
        str_at(&other, "snapshot_id"),
        "the same kept set stated in either order is one frozen conclusion"
    );

    let (status, nothing) = call(
        &router,
        post(
            "/asterism/pursuits/close",
            serde_json::json!({
                "pursuit_id": str_at(&empty_handed, "id"),
                "outcome": "satisfied",
                "verdicts": [],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "empty close: {nothing}");
    assert_eq!(
        nothing.get("snapshot_id"),
        Some(&serde_json::Value::Null),
        "concluding with nothing kept is a state, not an empty snapshot"
    );

    // An abandoned close that carries verdicts is refused before
    // anything is frozen.
    let (status, refused) = call(
        &router,
        post(
            "/asterism/pursuits/close",
            serde_json::json!({
                "pursuit_id": str_at(&first, "id"),
                "outcome": "abandoned",
                "verdicts": [{ "asset_id": a, "verdict": "keep" }],
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an abandoned close decides nothing: {refused}"
    );
}

/// Restamp over HTTP: a round moves, and the two walls hold.
#[tokio::test(flavor = "multi_thread")]
async fn restamp_moves_a_round_and_refuses_what_it_must() {
    let (tmp, core, router, persona) = harness("restamp").await;
    let source = seed_asset(&core, tmp.path(), &persona.id, "source.md").await;

    let (status, snapshot) = call(
        &router,
        post(
            "/asterism/snapshots/create",
            serde_json::json!({ "persona_id": persona.id, "asset_ids": [source] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "freeze: {snapshot}");

    // The round goes out filed under the line of work it names — the
    // filing restamp exists to move. A dispatch naming none carries no
    // stamp, and there is nothing to move.
    let (_, first_guess) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({ "persona_id": persona.id, "title": "the first guess" }),
        ),
    )
    .await;
    let first_guess_id = str_at(&first_guess, "id").to_string();

    let (status, round) = call(
        &router,
        post(
            "/asterism/dispatch/create",
            serde_json::json!({
                "snapshot_id": str_at(&snapshot, "id"),
                "exporter_slug": "file",
                "action": "write",
                "params_json": "",
                "pursuit_id": first_guess_id,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "dispatch: {round}");
    assert_eq!(
        str_at(&round, "pursuit_id"),
        first_guess_id,
        "the supplied stamp is the filing"
    );

    let (_, target) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({ "persona_id": persona.id, "title": "the real line" }),
        ),
    )
    .await;
    let target_id = str_at(&target, "id").to_string();
    assert_ne!(first_guess_id, target_id, "two distinct lines of work");

    let (status, moved) = call(
        &router,
        post(
            "/asterism/pursuits/restamp-dispatch",
            serde_json::json!({
                "dispatch_id": str_at(&round, "id"),
                "to_pursuit_id": target_id,
                "operator_ai": "claude-code",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restamp: {moved}");
    assert_eq!(
        str_at(&moved, "pursuit_id"),
        target_id,
        "the answer is the row as it now stands"
    );

    // The round shows up in the target's view and has left the mint's.
    let (_, target_view) = call(
        &router,
        get(&format!("/asterism/pursuits/{target_id}/view")),
    )
    .await;
    assert_eq!(
        target_view
            .get("rounds")
            .and_then(|r| r.as_array())
            .map(Vec::len),
        Some(1)
    );
    let (_, vacated_view) = call(
        &router,
        get(&format!("/asterism/pursuits/{first_guess_id}/view")),
    )
    .await;
    assert_eq!(
        vacated_view
            .get("rounds")
            .and_then(|r| r.as_array())
            .map(Vec::len),
        Some(0),
        "a round is filed in one place at a time"
    );

    // Wall one: the filing never leaves its persona.
    let stranger = register(&core, "restamp-stranger").await;
    let (_, foreign) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({ "persona_id": stranger.id }),
        ),
    )
    .await;
    let (status, crossing) = call(
        &router,
        post(
            "/asterism/pursuits/restamp-dispatch",
            serde_json::json!({
                "dispatch_id": str_at(&round, "id"),
                "to_pursuit_id": str_at(&foreign, "id"),
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a cross-persona target is refused: {crossing}"
    );

    // Wall two: a target that does not exist is refused rather than
    // written as a dangling filing.
    let (status, nowhere) = call(
        &router,
        post(
            "/asterism/pursuits/restamp-dispatch",
            serde_json::json!({
                "dispatch_id": str_at(&round, "id"),
                "to_pursuit_id": "0198c1c2-dead-7000-8000-00000000dead",
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an unknown target is refused: {nowhere}"
    );
}

/// The repair the id-choosing create exists for: a return arrived
/// naming a pursuit this library had no row for, the claim was recorded
/// unresolved, and creating the pursuit *at that id* is what lets the
/// sweep join them.
///
/// This is the pursuit-only branch of `reresolve_unresolved` — the
/// dispatch half of the same claim resolved at ingest and must stay
/// untouched, so a sweep that only repaired rows whose derivation was
/// broken would never reach this asset.
#[tokio::test(flavor = "multi_thread")]
async fn a_pursuit_created_at_a_claimed_id_repairs_the_return() {
    let (tmp, core, router, persona) = harness("claimed-id").await;
    let source = seed_asset(&core, tmp.path(), &persona.id, "plate.md").await;

    let (_, snapshot) = call(
        &router,
        post(
            "/asterism/snapshots/create",
            serde_json::json!({ "persona_id": persona.id, "asset_ids": [source] }),
        ),
    )
    .await;
    let (_, round) = call(
        &router,
        post(
            "/asterism/dispatch/create",
            serde_json::json!({
                "snapshot_id": str_at(&snapshot, "id"),
                "exporter_slug": "file",
                "action": "write",
                "params_json": "",
            }),
        ),
    )
    .await;

    // The artefact comes back with a sidecar naming a pursuit that has
    // no row here — the shape of a file written on another machine, or
    // restored ahead of the pursuit it belongs to.
    let claimed = "0198c1c2-beef-7000-8000-00000000beef";
    let returned = tmp.path().join("returned.md");
    std::fs::write(&returned, "# returned\n").expect("write returned");
    let sidecar = serde_json::json!({
        "id": source,
        SIDECAR_IDENTITY_KEY: {
            "schema": SIDECAR_SCHEMA,
            "dispatch_id": str_at(&round, "id"),
            "pursuit_id": claimed,
            "exporter_slug": "file",
            "source_asset_id": source,
        }
    });
    std::fs::write(
        format!("{}{}", returned.display(), SIDECAR_SUFFIX),
        serde_json::to_vec_pretty(&sidecar).unwrap(),
    )
    .expect("write sidecar");

    let child = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                returned.to_str().unwrap(),
                Some("sidecar".into()),
            ),
            &unattributed(),
        )
        .await
        .expect("an unresolvable pursuit claim never refuses the file");
    let trace = |dto: &asterism_contract::dto::AssetDto, key: &str| -> Option<bool> {
        serde_json::from_str::<serde_json::Value>(dto.extra_json.as_deref()?)
            .ok()?
            .get("_trace")?
            .get(key)?
            .as_bool()
    };
    assert_eq!(
        trace(&child, "pursuit_resolved"),
        Some(false),
        "nothing answers to the claimed id yet"
    );
    assert_eq!(
        trace(&child, "resolved"),
        Some(false),
        "the round it names is still pending, so the derivation half is \
         unresolved too — and stays that way through the sweep below, \
         which is what makes this the pursuit-only repair"
    );

    // The repair: create the pursuit under the id that was claimed.
    let (status, adopted) = call(
        &router,
        post(
            "/asterism/pursuits/open",
            serde_json::json!({
                "persona_id": persona.id,
                "pursuit_id": claimed,
                "title": "the line the return came from",
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

    let repaired = core
        .asset_service
        .reresolve_unresolved()
        .await
        .expect("sweep");
    assert_eq!(repaired, 1, "one note had an answer it did not have before");

    let after = core
        .asset_service
        .detail(asterism_contract::query::GetAssetDetailQuery {
            asset_id: child.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("read the return back");
    assert_eq!(
        trace(&after.asset, "pursuit_resolved"),
        Some(true),
        "the claim resolves once the pursuit exists under that id"
    );
    assert_eq!(
        trace(&after.asset, "resolved"),
        Some(false),
        "the two halves repair independently: the derivation still has \
         no finished round to point at, and the repair did not pretend \
         otherwise"
    );

    // And the return is now readable as part of that line of work.
    let (_, view) = call(&router, get(&format!("/asterism/pursuits/{claimed}/view"))).await;
    let returns: Vec<&str> = view
        .get("returns")
        .and_then(|r| r.as_array())
        .expect("returns array")
        .iter()
        .map(|v| v.as_str().expect("asset id"))
        .collect();
    assert!(
        returns.contains(&child.id.as_str()),
        "the repaired return files under the pursuit it named: {view}"
    );
}
