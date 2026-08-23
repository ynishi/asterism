//! The pursuit's verbs, over the real router.
//!
//! `forge_lines_routes_e2e` does this for the line. The three things
//! that can be wrong here and nowhere else are the same three, with
//! one of them sharper.
//!
//! 1. **The id in the path does not reach the service.** Seven of
//!    these routes take a pursuit id, and the three that also carry a
//!    command carry a `pursuit_id` field in it — that field is there
//!    so one command shape can serve MCP and the desktop, where there
//!    is no path to take an id from. Over HTTP the path wins, and a
//!    handler reading the body instead would pass every service test
//!    while writing to whatever the caller named.
//!    [`the_body_cannot_redirect_a_write`] sends the two disagreeing.
//! 2. **A refusal reaches the caller as the wrong status.** A round
//!    naming content nothing holds is a `Validation` and must arrive
//!    as 400; work that has already ended is a `Conflict` and must
//!    arrive as 409.
//! 3. **`resolve`'s two ordinary answers are not both ordinary.** A
//!    rule that declines writes nothing, and that is an outcome rather
//!    than a failure. A handler treating it as 204, or as an error,
//!    would be wrong in a way no service test can see — the service
//!    returns `Option<Round>` and both arms are `Ok`. Both are asked
//!    for here, and both must be 200 with a body that says which
//!    happened.

use std::sync::Arc;

use asterism_contract::command::RegisterPersonaCommand;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

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

/// Asserts 200 and hands back the body, naming what failed if not.
async fn ok(router: &Router, request: Request<Body>) -> serde_json::Value {
    let (status, body) = call(router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Registers a persona and adds `count` assets through the routes that
/// own them, so the content a round names is content the boundary can
/// find.
async fn assets(router: &Router, count: usize) -> Vec<String> {
    let persona = ok(
        router,
        post(
            "/asterism/personas/register",
            serde_json::to_value(RegisterPersonaCommand {
                name: "forge pursuits".into(),
                pack_id: None,
            })
            .expect("serialise"),
        ),
    )
    .await;
    let persona_id = persona["id"].as_str().expect("a persona id").to_string();

    let mut ids = Vec::new();
    for nth in 0..count {
        let asset = ok(
            router,
            post(
                "/asterism/assets/add",
                serde_json::json!({
                    "persona_id": persona_id,
                    "source_kind": "fs",
                    "locator": format!("/tmp/forge-pursuits/{nth}.png"),
                    "modality": "image",
                    "occurred_at_ms": 1_785_000_000_000i64 + nth as i64,
                    "labels": [],
                }),
            ),
        )
        .await;
        ids.push(asset["id"].as_str().expect("an asset id").to_string());
    }
    ids
}

async fn a_line(router: &Router, name: &str) -> String {
    let line = ok(
        router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": name, "strategy_id": "mainline-first" }),
        ),
    )
    .await;
    line["id"].as_str().expect("a line id").to_string()
}

async fn open_work(router: &Router, line: &str, title: &str) -> String {
    let work = ok(
        router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({ "line_id": line, "title": title }),
        ),
    )
    .await;
    assert_eq!(work["line_id"], line);
    assert_eq!(work["title"], title);
    assert!(
        work["rounds"].as_array().expect("rounds").is_empty(),
        "work opens with nothing written"
    );
    assert!(work["close"].is_null(), "work opens open");
    work["id"].as_str().expect("a pursuit id").to_string()
}

fn add(entry: &str, content: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "entry_id": entry,
        "kind": "add",
        "content_asset_id": content,
        "name": name,
    })
}

fn replace(entry: &str, content: &str) -> serde_json::Value {
    serde_json::json!({ "entry_id": entry, "kind": "replace", "content_asset_id": content })
}

async fn push(router: &Router, work: &str, ops: Vec<serde_json::Value>) -> serde_json::Value {
    ok(
        router,
        post(
            &format!("/asterism/forge/pursuits/{work}/push"),
            serde_json::json!({ "ops": ops }),
        ),
    )
    .await
}

async fn close(router: &Router, work: &str, outcome: &str) -> (StatusCode, serde_json::Value) {
    call(
        router,
        post(
            &format!("/asterism/forge/pursuits/{work}/close"),
            serde_json::json!({ "outcome": outcome }),
        ),
    )
    .await
}

/// Work opened, written to, collided, resolved by the line's rule, and
/// ended — reading `collisions` and `behind` where a screen would.
///
/// The collision is made the way one happens: two pieces of work cut
/// from the same head, both moving one entry's content, and the second
/// one landing first.
#[tokio::test]
async fn two_pieces_of_work_collide_and_the_rule_settles_it_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let content = assets(&router, 3).await;
    let line = a_line(&router, "over-http").await;
    let entry = Uuid::now_v7().to_string();

    // Something has to be on the line before two people can disagree
    // about it.
    let first = open_work(&router, &line, "put it there").await;
    push(&router, &first, vec![add(&entry, &content[0], "cut-01")]).await;
    let (status, landed) = close(&router, &first, "satisfied").await;
    assert_eq!(status, StatusCode::OK, "{landed}");
    assert_eq!(landed["close"]["outcome"], "satisfied");

    // Two pieces of work, cut from the same head.
    let mine = open_work(&router, &line, "mine").await;
    let theirs = open_work(&router, &line, "theirs").await;
    push(&router, &mine, vec![replace(&entry, &content[1])]).await;
    push(&router, &theirs, vec![replace(&entry, &content[2])]).await;

    // Nothing collides yet: the line has not moved under either.
    let quiet = ok(
        &router,
        get(&format!("/asterism/forge/pursuits/{mine}/collisions")),
    )
    .await;
    assert!(quiet.as_array().expect("collisions").is_empty());

    // Theirs lands. Now mine is behind, and asking for an axis the
    // line moved.
    let (status, body) = close(&router, &theirs, "satisfied").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let behind = ok(
        &router,
        get(&format!("/asterism/forge/pursuits/{mine}/behind")),
    )
    .await;
    assert_eq!(
        behind.as_array().expect("behind").len(),
        1,
        "one landing this work has not seen: {behind}"
    );

    let collisions = ok(
        &router,
        get(&format!("/asterism/forge/pursuits/{mine}/collisions")),
    )
    .await;
    let collisions = collisions.as_array().expect("collisions");
    assert_eq!(collisions.len(), 1, "{collisions:#?}");
    assert_eq!(collisions[0]["entry_id"], entry);
    assert_eq!(collisions[0]["axis"], "content");

    // Closing now is refused, and as a conflict rather than a
    // validation: the line moved, which is a race the caller can act
    // on.
    let (status, refused) = close(&router, &mine, "satisfied").await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["kind"], "Conflict");

    // The line's rule answers it. `mainline-first` keeps what is on
    // the line and carries this work's version onto a new entry.
    let settled = ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{mine}/resolve"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(
        !settled["round"].is_null(),
        "the rule had something to write: {settled}"
    );
    assert_eq!(
        settled["round"]["actor_kind"], "system",
        "a rule's round is the server's, not the caller's"
    );
    assert!(
        settled["collisions"]
            .as_array()
            .expect("collisions")
            .is_empty(),
        "nothing is left to settle: {settled}"
    );

    // And now it lands.
    let (status, landed) = close(&router, &mine, "satisfied").await;
    assert_eq!(status, StatusCode::OK, "{landed}");
    assert_eq!(landed["close"]["outcome"], "satisfied");

    // Read back through the two routes that answer about a line's
    // work rather than about one piece of it.
    let all = ok(
        &router,
        get(&format!("/asterism/forge/lines/{line}/pursuits")),
    )
    .await;
    assert_eq!(
        all.as_array().expect("pursuits").len(),
        3,
        "every piece of work against the line, ended or not: {all}"
    );

    let states = ok(
        &router,
        get(&format!("/asterism/forge/lines/{line}/states")),
    )
    .await;
    let alive: Vec<&str> = states
        .as_array()
        .expect("states")
        .iter()
        .filter(|state| state["alive"].as_bool().unwrap_or(false))
        .filter_map(|state| state["name"].as_str())
        .collect();
    assert_eq!(
        alive.len(),
        2,
        "the line's version kept its name and this work's is beside it: {states}"
    );
}

/// A rule that writes nothing answers 200, and the body says so.
///
/// Nothing collides here, so there is nothing for the rule to settle.
/// That is the same shape as a rule declining to settle a collision it
/// can see, and it is the arm a handler is most likely to get wrong.
#[tokio::test]
async fn resolving_nothing_is_an_answer_rather_than_an_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let content = assets(&router, 1).await;
    let line = a_line(&router, "nothing-to-settle").await;
    let entry = Uuid::now_v7().to_string();

    let work = open_work(&router, &line, "quiet").await;
    push(&router, &work, vec![add(&entry, &content[0], "cut-01")]).await;

    let settled = ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{work}/resolve"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(
        settled["round"].is_null(),
        "nothing was written, and that is an outcome: {settled}"
    );
    assert!(
        settled["collisions"]
            .as_array()
            .expect("collisions")
            .is_empty()
    );

    // The work is untouched: still one round, still open.
    let after = ok(&router, get(&format!("/asterism/forge/pursuits/{work}"))).await;
    assert_eq!(after["rounds"].as_array().expect("rounds").len(), 1);
    assert!(after["close"].is_null());
}

/// The path names the target, and a body that says otherwise is
/// ignored.
///
/// The `pursuit_id` field exists so one command can serve a transport
/// with no path to read. Over HTTP it must not be able to redirect the
/// write, which is the failure a service test cannot see.
#[tokio::test]
async fn the_body_cannot_redirect_a_write() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let content = assets(&router, 1).await;
    let line = a_line(&router, "path-wins").await;
    let entry = Uuid::now_v7().to_string();

    let target = open_work(&router, &line, "the one in the path").await;
    let decoy = open_work(&router, &line, "the one in the body").await;

    let written = ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{target}/push"),
            serde_json::json!({
                "pursuit_id": decoy,
                "ops": [add(&entry, &content[0], "cut-01")],
            }),
        ),
    )
    .await;
    assert_eq!(written["id"], target);
    assert_eq!(written["rounds"].as_array().expect("rounds").len(), 1);

    let untouched = ok(&router, get(&format!("/asterism/forge/pursuits/{decoy}"))).await;
    assert!(
        untouched["rounds"].as_array().expect("rounds").is_empty(),
        "the body named this one and it must not have been written to: {untouched}"
    );
}

/// The refusals, each as the status its kind maps to.
#[tokio::test]
async fn what_the_routes_refuse_and_with_which_status() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let content = assets(&router, 1).await;
    let line = a_line(&router, "refusals").await;
    let entry = Uuid::now_v7().to_string();

    // Work nobody opened.
    let missing = Uuid::now_v7();
    let (status, body) = call(&router, get(&format!("/asterism/forge/pursuits/{missing}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["kind"], "NotFound");

    // An id that is not one.
    let (status, body) = call(&router, get("/asterism/forge/pursuits/not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["kind"], "Validation");

    let work = open_work(&router, &line, "refusing").await;

    // A round naming content nothing holds. This is the check
    // `resolve` was missing and `push` was not.
    let (status, body) = call(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{work}/push"),
            serde_json::json!({ "ops": [add(&entry, &Uuid::now_v7().to_string(), "cut-01")] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["kind"], "Validation");

    // An outcome that is neither word.
    let (status, body) = call(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{work}/close"),
            serde_json::json!({ "outcome": "mostly" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["kind"], "Validation");

    // Work that has already ended. A `Conflict`, and the one a client
    // must not blind-retry — which the status does not say and the
    // route's doc comment does.
    push(&router, &work, vec![add(&entry, &content[0], "cut-01")]).await;
    let (status, body) = close(&router, &work, "satisfied").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = close(&router, &work, "satisfied").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["kind"], "Conflict");
}

/// Two conflicts from one route, told apart by the caller without
/// reading a sentence.
///
/// This is the whole reason `reason` exists. Both are 409 and both are
/// `"kind": "Conflict"`, and a client seeing only that either retries
/// both — looping forever on the second — or retries neither, giving up
/// on a race it would win by asking again. What separates them is a
/// token, and the token has to survive from where the refusal is
/// raised all the way out to the body.
#[tokio::test]
async fn two_conflicts_from_close_carry_different_advice() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let content = assets(&router, 3).await;
    let line = a_line(&router, "two-conflicts").await;
    let entry = Uuid::now_v7().to_string();

    let first = open_work(&router, &line, "put it there").await;
    push(&router, &first, vec![add(&entry, &content[0], "cut-01")]).await;
    let (status, _) = close(&router, &first, "satisfied").await;
    assert_eq!(status, StatusCode::OK);

    // Asking again for work that is over. Nothing will ever change
    // that, and the token says so.
    let (status, over) = close(&router, &first, "satisfied").await;
    assert_eq!(status, StatusCode::CONFLICT, "{over}");
    assert_eq!(over["kind"], "Conflict");
    assert_eq!(
        over["reason"], "settled",
        "work that has ended is over, not contended: {over}"
    );

    // The line moving under open work. The same close works once the
    // work is resolved, which is a different instruction entirely.
    let mine = open_work(&router, &line, "mine").await;
    let theirs = open_work(&router, &line, "theirs").await;
    push(&router, &mine, vec![replace(&entry, &content[1])]).await;
    push(&router, &theirs, vec![replace(&entry, &content[2])]).await;
    let (status, _) = close(&router, &theirs, "satisfied").await;
    assert_eq!(status, StatusCode::OK);

    let (status, blocked) = close(&router, &mine, "satisfied").await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    assert_eq!(blocked["kind"], "Conflict");
    assert_eq!(
        blocked["reason"], "blocked",
        "resolve first, then this same close works: {blocked}"
    );

    // Same status, same kind, opposite advice — which is the thing
    // that could not be expressed before.
    assert_ne!(over["reason"], blocked["reason"]);

    // `"blocked"` arrives here two ways, which is why the token is
    // where the reading stops and the message is where it goes on.
    // The second way is the line being archived: nothing about this
    // close collides, and reopening the line lets the identical
    // request through.
    let after = open_work(&router, &line, "after hours").await;
    push(&router, &after, vec![add(&entry, &content[0], "cut-02")]).await;
    let (status, _) = call(
        &router,
        post(
            &format!("/asterism/forge/lines/{line}/archive"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, shut) = close(&router, &after, "satisfied").await;
    assert_eq!(status, StatusCode::CONFLICT, "{shut}");
    assert_eq!(
        shut["reason"], "blocked",
        "an archived line is state refusing, and reopening it clears: {shut}"
    );
    assert_ne!(
        shut["message"], blocked["message"],
        "one token, two things to do — the message is the part that says which"
    );

    // And a refusal that is not a conflict carries no token at all:
    // a 400 asks one thing of a caller, so a field to branch on would
    // be a field with one value.
    let (status, bad) = call(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{mine}/close"),
            serde_json::json!({ "outcome": "mostly" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    assert!(bad.get("reason").is_none(), "{bad}");
}

/// Work opened from work, read back through the route that answers it.
#[tokio::test]
async fn work_opened_from_work_is_reachable_from_its_parent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let line = a_line(&router, "children").await;

    let parent = open_work(&router, &line, "the parent").await;
    let child = ok(
        &router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({
                "line_id": line,
                "parent_id": parent,
                "title": "the child",
            }),
        ),
    )
    .await;
    assert_eq!(child["parent_id"], parent);

    let children = ok(
        &router,
        get(&format!("/asterism/forge/pursuits/{parent}/children")),
    )
    .await;
    let children = children.as_array().expect("children");
    assert_eq!(children.len(), 1, "{children:#?}");
    assert_eq!(children[0]["id"], child["id"]);

    let none = ok(
        &router,
        get(&format!(
            "/asterism/forge/pursuits/{}/children",
            child["id"].as_str().expect("an id")
        )),
    )
    .await;
    assert!(none.as_array().expect("children").is_empty());
}
