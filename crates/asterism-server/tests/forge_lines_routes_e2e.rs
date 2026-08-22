//! The line's ten verbs, over the real router.
//!
//! `forge_wiring_e2e` proves the services against the store the
//! application builds. This proves the ten routes in front of them:
//! method, path, extractor, the parse from a wire string to a typed id,
//! and the shape that comes back.
//!
//! # What can be wrong here and nowhere else
//!
//! 1. **The id in the path does not reach the service.** Every act
//!    route here takes one, and a handler that read the body's
//!    `line_id` instead would pass a service test and archive the
//!    wrong line.
//! 2. **A refusal reaches the caller as the wrong status.** The forge
//!    answers `Conflict` for a race and `Validation` for malformed
//!    input, and the existing `ApiError` mapping is what turns those
//!    into 409 and 400. Nothing else asserts that the forge's
//!    refusals land there.
//! 3. **The drop's response is dropped.** `discard` answers with the
//!    assets it released and there is no second way to ask — a handler
//!    returning `{"discarded": true}` would look fine and lose the
//!    only answer there is.
//!
//! So these drive [`asterism_server::http::router`] through `oneshot`,
//! which exercises all of that without binding a port.

use std::sync::Arc;

use asterism_contract::command::RegisterPersonaCommand;
use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::model::op::Op;
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome};
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::value::{Content, LineId, Name};
use asterism_core::domain::forge::strategies::MainlineFirst;
use asterism_core::domain::value::AssetId;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn who(subject: &str) -> AttributionContext {
    AttributionContext::asserted(Some(Author::Subject(subject.into())), None).expect("a subject")
}

async fn harness(tmp: &std::path::Path) -> (CoreCtx, Router) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    (core, router)
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
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "body is not JSON ({error}): {}",
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

/// Registers a persona and an asset through the routes that own them,
/// so the content a line names is one the boundary can find.
async fn an_asset(router: &Router) -> (String, String) {
    let (status, persona) = call(
        router,
        post(
            "/asterism/personas/register",
            serde_json::to_value(RegisterPersonaCommand {
                name: "forge routes".into(),
                pack_id: None,
            })
            .expect("serialise"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{persona}");
    let persona_id = persona["id"].as_str().expect("a persona id").to_string();

    let (status, asset) = call(
        router,
        post(
            "/asterism/assets/add",
            serde_json::json!({
                "persona_id": persona_id,
                "source_kind": "fs",
                "locator": "/tmp/forge-routes/one.png",
                "modality": "image",
                "occurred_at_ms": 1_785_000_000_000i64,
                "labels": [],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{asset}");
    (
        persona_id,
        asset["id"].as_str().expect("an asset id").to_string(),
    )
}

/// The whole of a line's lifecycle, through the routes: open it, move
/// its description, land something on it, read it both ways, archive,
/// reopen, archive again and drop it.
#[tokio::test]
async fn a_line_lives_its_whole_life_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (_persona, asset) = an_asset(&router).await;

    // Open. The strategy is a slug, and the rules are readable from
    // the route that lists them rather than known to the caller.
    let (status, rules) = call(&router, get("/asterism/forge/strategies")).await;
    assert_eq!(status, StatusCode::OK, "{rules}");
    let carried = rules.as_array().expect("a list of rules");
    assert!(!carried.is_empty(), "this deployment carries rules");
    let mainline = MainlineFirst.id().to_string();
    assert!(
        carried
            .iter()
            .any(|rule| rule["id"].as_str() == Some(mainline.as_str())),
        "the built-in rule this test points a line at is one of them: {rules}"
    );

    let (status, line) = call(
        &router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "ROOT", "strategy_id": mainline }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{line}");
    let id = line["id"].as_str().expect("a line id").to_string();
    assert_eq!(line["standing"], "open");
    assert_eq!(line["name"], "ROOT");

    // Listed.
    let (status, lines) = call(&router, get("/asterism/forge/lines")).await;
    assert_eq!(status, StatusCode::OK, "{lines}");
    assert_eq!(lines.as_array().expect("a list").len(), 1);

    // Renamed, and the rule changed. Neither is a landing, so the head
    // does not move.
    let head_before = line["head_id"].as_str().expect("a head").to_string();
    let (status, said) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/rename"),
            serde_json::json!({ "name": "the only line" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{said}");
    // A write answers with the line, so nothing has to ask again.
    assert_eq!(said["name"], "the only line", "{said}");
    assert_eq!(said["head_id"], head_before, "a rename is not a landing");
    let (status, said) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/strategy"),
            serde_json::json!({ "strategy_id": mainline }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(said["strategy_id"], mainline, "{said}");

    // Something lands on it, through the services — the pursuit's own
    // routes are a later issue, and this test is about the line's.
    let line_id = LineId::from_uuid(uuid::Uuid::parse_str(&id).expect("a uuid"));
    let work = core
        .pursuit_service
        .open(&line_id, None, Intent::default(), &who("ana"))
        .await
        .expect("work opens");
    let content = Content::of(AssetId::from_uuid(
        uuid::Uuid::parse_str(&asset).expect("a uuid"),
    ));
    core.pursuit_service
        .push(
            &work.id(),
            vec![Op::add(content, Name::new("key visual").expect("a name"))],
            None,
            &who("ana"),
        )
        .await
        .expect("a round");
    core.pursuit_service
        .close(&work.id(), Outcome::Satisfied, None, &who("ana"))
        .await
        .expect("it lands");

    // Read both ways. The fold says what is on the line; the history
    // says how it got there, and the head has moved now.
    let (status, states) = call(&router, get(&format!("/asterism/forge/lines/{id}/states"))).await;
    assert_eq!(status, StatusCode::OK, "{states}");
    let entries = states.as_array().expect("a list of entries");
    assert_eq!(entries.len(), 1, "one entry landed: {states}");
    assert_eq!(entries[0]["alive"], true);
    assert_eq!(entries[0]["name"], "key visual");
    assert_eq!(entries[0]["content_asset_id"], asset);

    let (status, history) = call(&router, get(&format!("/asterism/forge/lines/{id}"))).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["line"]["name"], "the only line");
    assert_ne!(
        history["line"]["head_id"].as_str(),
        Some(head_before.as_str()),
        "the landing moved the head"
    );
    let changes = history["changes"].as_array().expect("the chain");
    assert_eq!(changes.len(), 1, "one landing: {history}");
    assert_eq!(changes[0]["parent_id"], history["genesis_id"]);
    let table = changes[0]["table"].as_array().expect("the table");
    assert_eq!(table.len(), 1);
    assert_eq!(table[0]["existence"], "present");
    assert_eq!(table[0]["content_asset_id"], asset);
    assert_eq!(table[0]["name"], "key visual");

    // Archived, reopened, archived again.
    let (status, said) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/archive"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(
        said["standing"], "archived",
        "the write answers with the line"
    );
    // And the line agrees when read back, which is what says the
    // response is the state rather than a hopeful echo of the verb.
    let (_, line) = call(&router, get(&format!("/asterism/forge/lines/{id}"))).await;
    assert_eq!(line["line"]["standing"], "archived");

    let (status, said) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/reopen"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(said["standing"], "open", "the write answers with the line");
    let (_, line) = call(&router, get(&format!("/asterism/forge/lines/{id}"))).await;
    assert_eq!(line["line"]["standing"], "open");

    call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/archive"),
            serde_json::json!({}),
        ),
    )
    .await;

    // Dropped — and the response is the only place the released assets
    // are ever named.
    let (status, dropped) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/discard"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dropped}");
    assert_eq!(dropped["line_id"], id);
    assert_eq!(
        dropped["released_asset_ids"]
            .as_array()
            .expect("the released set"),
        &vec![serde_json::Value::String(asset.clone())],
        "the asset the line held comes back as an asset id: {dropped}"
    );

    let (status, gone) = call(&router, get(&format!("/asterism/forge/lines/{id}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
}

/// A refusal from the model reaches the caller as the status its kind
/// means, through the mapping every other route already uses.
#[tokio::test]
async fn a_refused_verb_answers_with_the_status_its_refusal_means() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let mainline = MainlineFirst.id().to_string();

    // Malformed input: a path segment that is not an id at all.
    let (status, said) = call(&router, get("/asterism/forge/lines/not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{said}");
    assert_eq!(said["kind"], "Validation");

    // A rule this deployment does not carry.
    let (status, said) = call(
        &router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "ROOT", "strategy_id": "no-such-rule" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{said}");

    // A line nobody opened.
    let absent = uuid::Uuid::now_v7();
    let (status, said) = call(&router, get(&format!("/asterism/forge/lines/{absent}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{said}");

    // Dropping an open line is the model's own refusal, and it is 400
    // rather than 409 on purpose. The division `ForgeError` draws is
    // not "did the caller ask for something the state disallows" but
    // "did the state move under them": `NotOnHead`, `NameTaken` and
    // `Collides` are races and read as conflicts, and everything else
    // is a caller who has to do something different. Nothing about
    // reading again and retrying helps a line that was never
    // archived — the caller has to archive it.
    let (status, line) = call(
        &router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "ROOT", "strategy_id": mainline }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{line}");
    let id = line["id"].as_str().expect("a line id");
    let (status, said) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{id}/discard"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an open line is not droppable, and retrying will never change that: {said}"
    );
    assert_eq!(said["kind"], "Validation");

    // No *single request* can produce a 409 here, which is why there
    // is no 409 assertion in this file. `discard` reaches both of the
    // line's races — a work list that moved, and a line reopened under
    // a decided drop — and both answer `Conflict`, but raising either
    // takes a second caller arriving between this one's read and its
    // write. They are pinned where a test can arrange that: over the
    // ports, in `asterism-infra`'s `forge_over_ports_e2e`.
}

/// The id in the path is the one that moves, even when the body names
/// another.
///
/// The command carries `line_id` so the same shape can serve MCP and
/// the desktop, where there is no path to take it from. Over HTTP the
/// path wins, and a handler that read the body would archive somebody
/// else's line while answering 200.
#[tokio::test]
async fn the_path_names_the_line_and_the_body_cannot_redirect_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let mainline = MainlineFirst.id().to_string();

    let mut ids = Vec::new();
    for name in ["one", "two"] {
        let (status, line) = call(
            &router,
            post(
                "/asterism/forge/lines",
                serde_json::json!({ "name": name, "strategy_id": mainline }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{line}");
        ids.push(line["id"].as_str().expect("a line id").to_string());
    }
    let (target, decoy) = (&ids[0], &ids[1]);

    let (status, said) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{target}/archive"),
            serde_json::json!({ "line_id": decoy }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{said}");

    let (_, moved) = call(&router, get(&format!("/asterism/forge/lines/{target}"))).await;
    assert_eq!(moved["line"]["standing"], "archived");
    let (_, untouched) = call(&router, get(&format!("/asterism/forge/lines/{decoy}"))).await;
    assert_eq!(
        untouched["line"]["standing"], "open",
        "the body named this one and it did not move"
    );
}
