//! The conversation's verbs, over the real router.
//!
//! The line's and the pursuit's route tests name three things that can
//! be wrong at this layer and nowhere else. Two of them are the same
//! here — the id in the path reaching the service, and a refusal
//! arriving as the wrong status — and the third is this surface's own.
//!
//! **An anchor is resolved rather than accepted, and four routes are
//! why.** `Anchored` has four variants of three different arities. One
//! route taking a discriminator would need a different set of required
//! parameters per value, and no router refuses a wrong combination: a
//! caller naming a round while passing an entry id gets an answer about
//! something else, or a 500. Four paths, each carrying exactly the ids
//! its anchor needs, make the wrong combination unwritable — which is a
//! claim about what a caller *cannot* do, so
//! [`each_about_route_answers_only_for_its_own_anchor`] shows the four
//! answering separately rather than asserting the absence.
//!
//! And the anchor is checked against what the forge holds, not taken on
//! trust, so the two ways of getting it wrong answer differently: work
//! nobody opened is a `404`, an entry the round never touched is a
//! `400` about the anchor.

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

async fn ok(router: &Router, request: Request<Body>) -> serde_json::Value {
    let (status, body) = call(router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// A persona, an asset, a line, and work against it with one round —
/// the smallest world with all four anchors in it.
struct World {
    line: String,
    point: String,
    pursuit: String,
    round: String,
    entry: String,
}

async fn a_world(router: &Router) -> World {
    let persona = ok(
        router,
        post(
            "/asterism/personas/register",
            serde_json::to_value(RegisterPersonaCommand {
                name: "forge threads".into(),
                pack_id: None,
            })
            .expect("serialise"),
        ),
    )
    .await;
    let persona_id = persona["id"].as_str().expect("a persona id").to_string();

    let asset = ok(
        router,
        post(
            "/asterism/assets/add",
            serde_json::json!({
                "persona_id": persona_id,
                "source_kind": "fs",
                "locator": "/tmp/forge-threads/one.png",
                "modality": "image",
                "occurred_at_ms": 1_785_000_000_000i64,
                "labels": [],
            }),
        ),
    )
    .await;
    let content = asset["id"].as_str().expect("an asset id").to_string();

    let line = ok(
        router,
        post(
            "/asterism/forge/lines",
            serde_json::json!({ "name": "threaded", "strategy_id": "mainline-first" }),
        ),
    )
    .await;
    let line = line["id"].as_str().expect("a line id").to_string();

    let entry = Uuid::now_v7().to_string();
    let work = ok(
        router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({ "line_id": line, "title": "the work" }),
        ),
    )
    .await;
    let pursuit = work["id"].as_str().expect("a pursuit id").to_string();

    let after = ok(
        router,
        post(
            &format!("/asterism/forge/pursuits/{pursuit}/push"),
            serde_json::json!({
                "ops": [{
                    "entry_id": entry,
                    "kind": "add",
                    "content_asset_id": content,
                    "name": "cut-01",
                }],
            }),
        ),
    )
    .await;
    let round = after["rounds"][0]["id"]
        .as_str()
        .expect("a round id")
        .to_string();

    // Land it, so there is a change point to hang the fourth anchor
    // off. A second piece of work carries the line forward without
    // ending the one the other three anchors are about.
    let landing = ok(
        router,
        post(
            "/asterism/forge/pursuits",
            serde_json::json!({ "line_id": line, "title": "the landing" }),
        ),
    )
    .await;
    let landing = landing["id"].as_str().expect("a pursuit id").to_string();
    ok(
        router,
        post(
            &format!("/asterism/forge/pursuits/{landing}/push"),
            serde_json::json!({
                "ops": [{
                    "entry_id": Uuid::now_v7().to_string(),
                    "kind": "add",
                    "content_asset_id": content,
                    "name": "cut-02",
                }],
            }),
        ),
    )
    .await;
    ok(
        router,
        post(
            &format!("/asterism/forge/pursuits/{landing}/close"),
            serde_json::json!({ "outcome": "satisfied" }),
        ),
    )
    .await;

    let history = ok(router, get(&format!("/asterism/forge/lines/{line}"))).await;
    let point = history["changes"][0]["id"]
        .as_str()
        .expect("a change point id")
        .to_string();

    World {
        line,
        point,
        pursuit,
        round,
        entry,
    }
}

async fn open_about(router: &Router, anchor: serde_json::Value) -> String {
    let thread = ok(router, post("/asterism/forge/threads", anchor)).await;
    thread["id"].as_str().expect("a thread id").to_string()
}

/// A conversation from opening to reading it back, through every route
/// it is reachable from — and then the line it hangs off is dropped and
/// it is gone.
#[tokio::test]
async fn a_conversation_lives_its_whole_life_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let world = a_world(&router).await;

    let thread = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "round",
            "pursuit_id": world.pursuit,
            "node_id": world.round,
            "title": "about this round",
            "said": "why this name?",
        }),
    )
    .await;

    // Said, then corrected. The correction is what `amend` answers
    // with — not the message as it now reads.
    let said = ok(
        &router,
        post(
            &format!("/asterism/forge/threads/{thread}/say"),
            serde_json::json!({ "said": "because the old one was taken" }),
        ),
    )
    .await;
    let said_id = said["id"].as_str().expect("a message id").to_string();
    assert_eq!(said["said"], "because the old one was taken");
    assert_eq!(said["first_said"], "because the old one was taken");

    let correction = ok(
        &router,
        post(
            &format!("/asterism/forge/threads/{thread}/amend"),
            serde_json::json!({
                "message_id": said_id,
                "said": "because the old one was taken by cut-02",
            }),
        ),
    )
    .await;
    assert_eq!(
        correction["said"],
        "because the old one was taken by cut-02"
    );
    assert!(
        correction.get("id").is_none(),
        "a correction is not a message: {correction}"
    );

    // Read whole, and what was said first is still there.
    let whole = ok(&router, get(&format!("/asterism/forge/threads/{thread}"))).await;
    let messages = whole["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 2, "{whole}");
    let corrected = &messages[1];
    assert_eq!(corrected["said"], "because the old one was taken by cut-02");
    assert_eq!(
        corrected["first_said"], "because the old one was taken",
        "a correction does not overwrite what was said: {corrected}"
    );
    assert_eq!(
        corrected["revisions"].as_array().expect("revisions").len(),
        1
    );
    assert_eq!(whole["anchor"]["kind"], "round");
    assert_eq!(whole["anchor"]["node_id"], world.round);

    // Renamed, and renaming says nothing in the conversation.
    let renamed = ok(
        &router,
        post(
            &format!("/asterism/forge/threads/{thread}/rename"),
            serde_json::json!({ "title": "the naming question" }),
        ),
    )
    .await;
    assert_eq!(renamed["title"], "the naming question");
    assert_eq!(
        renamed["messages"].as_array().expect("messages").len(),
        2,
        "a title is a label, not something said: {renamed}"
    );

    // And taken off again.
    let bare = ok(
        &router,
        post(
            &format!("/asterism/forge/threads/{thread}/rename"),
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(bare["title"].is_null(), "{bare}");

    // Reachable from the round it is about.
    let about = ok(
        &router,
        get(&format!(
            "/asterism/forge/pursuits/{}/rounds/{}/threads",
            world.pursuit, world.round
        )),
    )
    .await;
    let about = about.as_array().expect("threads");
    assert_eq!(about.len(), 1, "{about:#?}");
    assert_eq!(about[0]["id"], thread);

    // The line goes, and the conversation about its work goes with it.
    // Nothing deletes a thread on its own — this is the only way one
    // ends.
    //
    // Three steps, and the order is the model's: a drop is decided
    // against an archived line, an archived line refuses while work is
    // open against it, and work ends before either. Each of those
    // refuses out of order rather than being waived, so the test walks
    // them the way a caller has to.
    ok(
        &router,
        post(
            &format!("/asterism/forge/pursuits/{}/close", world.pursuit),
            serde_json::json!({ "outcome": "abandoned" }),
        ),
    )
    .await;
    ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{}/archive", world.line),
            serde_json::json!({}),
        ),
    )
    .await;
    ok(
        &router,
        post(
            &format!("/asterism/forge/lines/{}/discard", world.line),
            serde_json::json!({}),
        ),
    )
    .await;

    let (status, gone) = call(&router, get(&format!("/asterism/forge/threads/{thread}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
}

/// Four anchors, four routes, and each answers only for its own.
///
/// The wrong combination is not asserted against, because it cannot be
/// written: there is no place in
/// `/asterism/forge/pursuits/{id}/rounds/{node}/threads` to put an
/// entry id. What is shown instead is that four conversations about
/// four different things stay apart.
#[tokio::test]
async fn each_about_route_answers_only_for_its_own_anchor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let world = a_world(&router).await;

    let about_work = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "pursuit",
            "pursuit_id": world.pursuit,
            "said": "about the work",
        }),
    )
    .await;
    let about_round = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "round",
            "pursuit_id": world.pursuit,
            "node_id": world.round,
            "said": "about the round",
        }),
    )
    .await;
    let about_entry = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "entry",
            "pursuit_id": world.pursuit,
            "node_id": world.round,
            "entry_id": world.entry,
            "said": "about the entry",
        }),
    )
    .await;
    let about_change = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "change",
            "line_id": world.line,
            "change_point_id": world.point,
            "said": "about what landed",
        }),
    )
    .await;

    let only = |body: &serde_json::Value, expected: &str| {
        let found = body.as_array().expect("threads");
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0]["id"], expected);
    };

    only(
        &ok(
            &router,
            get(&format!(
                "/asterism/forge/pursuits/{}/threads",
                world.pursuit
            )),
        )
        .await,
        &about_work,
    );
    only(
        &ok(
            &router,
            get(&format!(
                "/asterism/forge/pursuits/{}/rounds/{}/threads",
                world.pursuit, world.round
            )),
        )
        .await,
        &about_round,
    );
    only(
        &ok(
            &router,
            get(&format!(
                "/asterism/forge/pursuits/{}/rounds/{}/entries/{}/threads",
                world.pursuit, world.round, world.entry
            )),
        )
        .await,
        &about_entry,
    );
    only(
        &ok(
            &router,
            get(&format!(
                "/asterism/forge/lines/{}/points/{}/threads",
                world.line, world.point
            )),
        )
        .await,
        &about_change,
    );
}

/// More than one conversation can hang off one thing.
///
/// Two people starting separate conversations about a round is not a
/// mistake to merge, which is why every `about` route answers a list.
#[tokio::test]
async fn two_conversations_about_one_round_both_come_back() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let world = a_world(&router).await;

    let anchor = serde_json::json!({
        "anchor_kind": "round",
        "pursuit_id": world.pursuit,
        "node_id": world.round,
    });
    let mut first = anchor.clone();
    first["said"] = "is this right?".into();
    let mut second = anchor.clone();
    second["said"] = "unrelated question".into();

    let one = open_about(&router, first).await;
    let two = open_about(&router, second).await;
    assert_ne!(one, two);

    let found = ok(
        &router,
        get(&format!(
            "/asterism/forge/pursuits/{}/rounds/{}/threads",
            world.pursuit, world.round
        )),
    )
    .await;
    assert_eq!(found.as_array().expect("threads").len(), 2, "{found}");
}

/// What the routes refuse, and with which status.
///
/// The two anchor failures are the pair #122 asks for by name: work
/// nobody opened is not found, and an entry the round never touched is
/// a refusal about the anchor rather than a missing thing.
#[tokio::test]
async fn what_the_routes_refuse_and_with_which_status() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let world = a_world(&router).await;

    // Work nobody opened.
    let (status, body) = call(
        &router,
        post(
            "/asterism/forge/threads",
            serde_json::json!({
                "anchor_kind": "pursuit",
                "pursuit_id": Uuid::now_v7().to_string(),
                "said": "about nothing",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["kind"], "NotFound");

    // An entry that round never touched.
    let (status, body) = call(
        &router,
        post(
            "/asterism/forge/threads",
            serde_json::json!({
                "anchor_kind": "entry",
                "pursuit_id": world.pursuit,
                "node_id": world.round,
                "entry_id": Uuid::now_v7().to_string(),
                "said": "about an entry it never had",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["kind"], "Validation");

    // An anchor kind that does not exist.
    let (status, body) = call(
        &router,
        post(
            "/asterism/forge/threads",
            serde_json::json!({
                "anchor_kind": "vibes",
                "pursuit_id": world.pursuit,
                "said": "about the vibes",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // A conversation opened saying nothing.
    let (status, body) = call(
        &router,
        post(
            "/asterism/forge/threads",
            serde_json::json!({
                "anchor_kind": "pursuit",
                "pursuit_id": world.pursuit,
                "said": "",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // A reply naming a message of another conversation. **400, not
    // 409** — the caller addressed one thread and named something that
    // is not in it, and no change of state makes that hold. This
    // answered 409 through the port and 400 through the service until
    // that was fixed on this branch; both doors agree now, and this is
    // the door a caller reaches.
    let mine = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "pursuit",
            "pursuit_id": world.pursuit,
            "said": "mine",
        }),
    )
    .await;
    let theirs = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "round",
            "pursuit_id": world.pursuit,
            "node_id": world.round,
            "said": "theirs",
        }),
    )
    .await;
    let over_there = ok(&router, get(&format!("/asterism/forge/threads/{theirs}"))).await;
    let stray = over_there["messages"][0]["id"]
        .as_str()
        .expect("a message id")
        .to_string();

    let (status, body) = call(
        &router,
        post(
            &format!("/asterism/forge/threads/{mine}/say"),
            serde_json::json!({ "replying_to": stray, "said": "answering over there" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["kind"], "Validation");
    assert!(
        body.get("reason").is_none(),
        "not a conflict, so no retry advice: {body}"
    );
}

/// The path names the target, and a body that says otherwise is
/// ignored.
#[tokio::test]
async fn the_body_cannot_redirect_a_write() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;
    let world = a_world(&router).await;

    let target = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "pursuit",
            "pursuit_id": world.pursuit,
            "said": "the one in the path",
        }),
    )
    .await;
    let decoy = open_about(
        &router,
        serde_json::json!({
            "anchor_kind": "round",
            "pursuit_id": world.pursuit,
            "node_id": world.round,
            "said": "the one in the body",
        }),
    )
    .await;

    ok(
        &router,
        post(
            &format!("/asterism/forge/threads/{target}/say"),
            serde_json::json!({ "thread_id": decoy, "said": "which one?" }),
        ),
    )
    .await;

    let written = ok(&router, get(&format!("/asterism/forge/threads/{target}"))).await;
    assert_eq!(written["messages"].as_array().expect("messages").len(), 2);

    let untouched = ok(&router, get(&format!("/asterism/forge/threads/{decoy}"))).await;
    assert_eq!(
        untouched["messages"].as_array().expect("messages").len(),
        1,
        "the body named this one and it must not have been written to: {untouched}"
    );
}
