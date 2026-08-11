//! End-to-end guard for the routes that closed the transport gap.
//!
//! Ten Tauri commands reached a service no HTTP route did, so a caller
//! over HTTP could not read the comment thread, ask for the grid's own
//! index, or promote a Snapshot — the UI was the only way in. The routes
//! that fixed that are thin, which is exactly why they need a test at
//! this level rather than the service level: what can be wrong is the
//! *wiring*.
//!
//! Three wiring mistakes a service test cannot see:
//!
//! 1. wrong extractor — `hydrate_cards` takes an unbounded id list, so
//!    it is a POST body; declared as `Query` it would reject every call
//! 2. wrong path — a comment posted through `/assets/{id}/comments` must
//!    land on the asset in the URL even when the body names another
//! 3. missing route — the whole defect being fixed here
//!
//! So these drive the real [`asterism_server::http::router`] through
//! `oneshot`, which exercises method, path, extractor and response
//! serialisation without binding a port.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, AddAssetToGroupCommand, CreateGroupCommand, RegisterPersonaCommand,
};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about route wiring, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// Spins up a core over a tempdir and returns it with the router built
/// on top.
///
/// `init_core_with` (rather than `init_core`) hands the Tantivy index a
/// tempdir: the default resolves the *active profile's* directory, so a
/// test run would write into whatever profile the developer is using.
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

/// One request through the router. Returns the status and the parsed
/// body, since a route can answer 200 with the wrong shape.
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

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some("image".into()),
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
    }
}

/// The comment thread over HTTP: post, read back, edit, delete.
///
/// Asserts the path override too — a body naming a different asset must
/// not decide where the comment lands, or two callers disagreeing about
/// the target would silently write to different threads.
#[tokio::test(flavor = "multi_thread")]
async fn the_comment_thread_is_reachable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("commented.md");
    std::fs::write(&file, "body\n").expect("write file");
    let other = corpus.join("other.md");
    std::fs::write(&other, "other\n").expect("write other");

    let (core, router) = harness(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-routes-comments".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let asset = core
        .asset_service
        .add(
            add_command(&persona.id, file.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");
    let decoy = core
        .asset_service
        .add(
            add_command(&persona.id, other.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add decoy");

    // Empty thread reads as an empty list, not a 404: an asset with no
    // comments is a normal asset.
    let (status, body) = call(
        &router,
        get(&format!("/asterism/assets/{}/comments", asset.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "empty thread: {body}");
    assert_eq!(body.as_array().expect("array").len(), 0);

    // The body names the decoy on purpose — the URL must win.
    let (status, posted) = call(
        &router,
        post(
            &format!("/asterism/assets/{}/comments", asset.id),
            serde_json::json!({
                "asset_id": decoy.id,
                "author_kind": "user",
                "author_persona_id": null,
                "body": "posted over http",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "post: {posted}");
    assert_eq!(
        posted["asset_id"], asset.id,
        "the URL decides the target asset, not the body"
    );
    let comment_id = posted["id"].as_str().expect("comment id").to_string();

    let (_, listed) = call(
        &router,
        get(&format!("/asterism/assets/{}/comments", asset.id)),
    )
    .await;
    assert_eq!(listed.as_array().expect("array").len(), 1);
    assert_eq!(listed[0]["body"], "posted over http");
    assert!(
        listed[0]["edited_at_ms"].is_null(),
        "a pristine post carries no edit stamp"
    );

    // The decoy's thread stayed empty, which is the other half of the
    // path-override assertion.
    let (_, decoy_thread) = call(
        &router,
        get(&format!("/asterism/assets/{}/comments", decoy.id)),
    )
    .await;
    assert_eq!(
        decoy_thread.as_array().expect("array").len(),
        0,
        "the asset named in the body must not receive the comment"
    );

    let (status, edited) = call(
        &router,
        post(
            "/asterism/comments/edit",
            serde_json::json!({
                "asset_id": asset.id,
                "comment_id": comment_id,
                "body": "edited over http",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "edit: {edited}");
    assert_eq!(edited["body"], "edited over http");
    assert!(
        edited["edited_at_ms"].is_i64(),
        "an edit stamps edited_at_ms"
    );

    let (status, deleted) = call(
        &router,
        post(
            "/asterism/comments/delete",
            serde_json::json!({ "comment_id": comment_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete: {deleted}");
    let (_, after) = call(
        &router,
        get(&format!("/asterism/assets/{}/comments", asset.id)),
    )
    .await;
    assert_eq!(after.as_array().expect("array").len(), 0);

    // Idempotent: deleting again is not an error. A caller retrying a
    // timed-out delete should not have to distinguish the two cases.
    let (status, _) = call(
        &router,
        post(
            "/asterism/comments/delete",
            serde_json::json!({ "comment_id": comment_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete is idempotent");
}

/// `GET /assets/index` + `POST /assets/hydrate` — the read pair the grid
/// itself performs, and the reason `hydrate` is a POST.
#[tokio::test(flavor = "multi_thread")]
async fn the_grid_read_pair_is_reachable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let a = corpus.join("a.md");
    let b = corpus.join("b.md");
    std::fs::write(&a, "a\n").expect("write a");
    std::fs::write(&b, "b\n").expect("write b");

    let (core, router) = harness(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-routes-grid".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let first = core
        .asset_service
        .add(
            add_command(&persona.id, a.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add a");
    let second = core
        .asset_service
        .add(
            add_command(&persona.id, b.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add b");

    let (status, index) = call(&router, get("/asterism/assets/index?offset=0&limit=50")).await;
    assert_eq!(status, StatusCode::OK, "index: {index}");
    let ids: Vec<&str> = index["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert!(ids.contains(&first.id.as_str()) && ids.contains(&second.id.as_str()));
    assert_eq!(index["total"], 2);

    // Index rows are the light projection: no cover text, no locator.
    // If those appear the endpoint is answering with cards, which is
    // what `list_assets` already does.
    //
    // `file_size_bytes` was on this list and is not any more. The line
    // between the two projections is not "small fields here, big ones
    // there" — it is what the client *sorts* on versus what it *paints*:
    // the grid orders these rows itself, so a sort key withheld from
    // them is an axis the picker cannot offer, while a render field
    // withheld costs one hydration round-trip and nothing else. Size and
    // length are keys (`SortTarget::FileSize` / `Duration`); cover and
    // locator are paint.
    let entry = &index["items"][0];
    for heavy in ["source_locator", "cover", "snippet", "score"] {
        assert!(
            entry.get(heavy).is_none(),
            "index rows must stay light; found {heavy} in {entry}"
        );
    }

    let (status, cards) = call(
        &router,
        post(
            "/asterism/assets/hydrate",
            serde_json::json!({ "ids": [first.id, second.id], "viewer_subject": null }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "hydrate: {cards}");
    assert_eq!(cards.as_array().expect("array").len(), 2);
    assert!(
        cards[0].get("source_locator").is_some(),
        "hydrated cards carry what index rows omit: {}",
        cards[0]
    );

    // An unknown id drops out rather than failing the batch — a viewport
    // is a guess about what is still there.
    let (status, partial) = call(
        &router,
        post(
            "/asterism/assets/hydrate",
            serde_json::json!({
                "ids": [first.id, "019f0000-0000-7000-8000-000000000000"],
                "viewer_subject": null,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "partial hydrate: {partial}");
    assert_eq!(
        partial.as_array().expect("array").len(),
        1,
        "a missing id is skipped, not an error"
    );
}

/// `GET /assets/{id}/groups` — the membership read behind the UI's
/// "already added" state.
#[tokio::test(flavor = "multi_thread")]
async fn group_membership_of_an_asset_is_reachable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("filed.md");
    std::fs::write(&file, "filed\n").expect("write file");

    let (core, router) = harness(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-routes-groups".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let asset = core
        .asset_service
        .add(
            add_command(&persona.id, file.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");

    let (status, none_yet) = call(
        &router,
        get(&format!("/asterism/assets/{}/groups", asset.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unfiled: {none_yet}");
    assert_eq!(none_yet.as_array().expect("array").len(), 0);

    let group = core
        .asset_service
        .create_group(
            CreateGroupCommand {
                persona_id: persona.id.clone(),
                name: "Filed".into(),
                description: None,
            },
            &unattributed(),
        )
        .await
        .expect("create group");
    core.asset_service
        .add_asset_to_group(
            AddAssetToGroupCommand {
                asset_id: asset.id.clone(),
                group_id: group.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("add to group");

    let (_, filed) = call(
        &router,
        get(&format!("/asterism/assets/{}/groups", asset.id)),
    )
    .await;
    let names: Vec<&str> = filed
        .as_array()
        .expect("array")
        .iter()
        .map(|g| g["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["Filed"]);
}

/// The Snapshot pair: the reverse lookup from an asset, and promoting a
/// frozen pick into a Group.
///
/// `promote-to-group` had only the fused `promote-volatile` sibling on
/// the wire, so an agent that already held a Snapshot had no way to
/// promote it without minting a second one.
#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_pair_is_reachable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let first = corpus.join("first.md");
    let second = corpus.join("second.md");
    std::fs::write(&first, "first\n").expect("write first");
    std::fs::write(&second, "second\n").expect("write second");

    let (core, router) = harness(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-routes-snapshots".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    // Distinct occurrence times: the assertions below play the time axis
    // against the frozen order, which needs the two to be separable.
    let mut older = add_command(&persona.id, first.to_str().unwrap());
    older.occurred_at_ms = 1_785_000_000_000;
    let mut newer = add_command(&persona.id, second.to_str().unwrap());
    newer.occurred_at_ms = 1_785_000_001_000;
    let a = core
        .asset_service
        .add(older, &unattributed())
        .await
        .expect("add first");
    let b = core
        .asset_service
        .add(newer, &unattributed())
        .await
        .expect("add second");

    // Before any freeze the reverse lookup is empty rather than absent.
    let (status, none_yet) = call(
        &router,
        get(&format!("/asterism/assets/{}/snapshots", a.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "no snapshots yet: {none_yet}");
    assert_eq!(none_yet.as_array().expect("array").len(), 0);

    // Freeze `b` then `a` — the reverse order of registration, so the
    // promoted Group's membership order cannot match by coincidence.
    let snapshot = core
        .snapshot_service
        .create(
            asterism_contract::command::CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![b.id.clone(), a.id.clone()],
            },
            &unattributed(),
        )
        .await
        .expect("create snapshot");

    let (_, containing) = call(
        &router,
        get(&format!("/asterism/assets/{}/snapshots", a.id)),
    )
    .await;
    let ids: Vec<&str> = containing
        .as_array()
        .expect("array")
        .iter()
        .map(|s| s["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec![snapshot.id.as_str()]);

    let (status, promoted) = call(
        &router,
        post(
            "/asterism/snapshots/promote-to-group",
            serde_json::json!({
                "snapshot_id": snapshot.id,
                "name": "From Snapshot",
                "description": null,
                "dir_id": null,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "promote: {promoted}");
    let group_id = promoted["group_id"].as_str().expect("group id");

    // The Group carries the frozen order. Asserted against `occurred_at`
    // rather than the hand-arrangement axis on purpose: for a filter
    // naming one Group, arrival order *is* the arrangement, so
    // `group` + `ordered` would agree even with the sort skipped. The
    // two assets were registered oldest-first and frozen newest-first,
    // so this pair of requests can only both hold if the axis ran.
    //
    // Whether `/assets/index` honours an axis at all is pinned by
    // `sorted_list_e2e::index_and_list_agree_on_every_axis`.
    let arrangement = |sort: &str| {
        format!("/asterism/assets/index?group_ids={group_id}&offset=0&limit=50&sort={sort}")
    };
    let ids_of = |v: &serde_json::Value| -> Vec<String> {
        v["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|i| i["id"].as_str().expect("id").to_string())
            .collect()
    };

    let (_, by_hand) = call(
        &router,
        get(&arrangement(
            "%7B%22target%22%3A%22group%22%2C%22order%22%3A%22ordered%22%2C%22reverse%22%3Afalse%7D",
        )),
    )
    .await;
    assert_eq!(
        ids_of(&by_hand),
        vec![b.id.clone(), a.id.clone()],
        "the promoted Group holds the Snapshot's frozen order"
    );

    let (_, by_time) = call(
        &router,
        get(&arrangement(
            "%7B%22target%22%3A%22occurred_at%22%2C%22order%22%3A%22updated%22%2C%22reverse%22%3Atrue%7D",
        )),
    )
    .await;
    assert_eq!(
        ids_of(&by_time),
        vec![a.id.clone(), b.id.clone()],
        "oldest-first must cross the frozen order, or the axis is not running"
    );
}

/// `POST /personas/reorder` — the sidebar hand arrangement that
/// `Sort: persona` + `Order: As arranged` reads back.
#[tokio::test(flavor = "multi_thread")]
async fn the_persona_hand_arrangement_is_writable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;

    let mut ids = Vec::new();
    for name in ["Alpha", "Beta", "Gamma"] {
        let persona = core
            .persona_service
            .register(
                RegisterPersonaCommand {
                    name: name.into(),
                    pack_id: Some(format!("e2e-routes-reorder-{name}")),
                },
                &unattributed(),
            )
            .await
            .expect("register persona");
        ids.push(persona.id);
    }

    // Reverse the registration order: an arrangement that matched it
    // would pass even with the write skipped.
    let reversed: Vec<String> = ids.iter().rev().cloned().collect();
    let (status, body) = call(
        &router,
        post(
            "/asterism/personas/reorder",
            serde_json::json!({ "ordered_ids": reversed }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reorder: {body}");

    let listed = core.persona_service.list().await.expect("list personas");
    let got: Vec<&str> = listed.iter().map(|p| p.id.as_str()).collect();
    let want: Vec<&str> = reversed.iter().map(String::as_str).collect();
    assert_eq!(
        got, want,
        "the list order follows the arrangement that was written"
    );
}

/// The repair verb over HTTP: declare an origin for an asset that is
/// already in the library.
///
/// Asserts the path override too, same rule as the comment thread — a
/// body naming a different asset must not decide whose provenance gets
/// written.
#[tokio::test(flavor = "multi_thread")]
async fn provenance_is_declarable_over_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "Routes".into(),
                pack_id: Some("e2e-routes-provenance".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let parent = core
        .asset_service
        .add(
            add_command(&persona.id, tmp.path().join("parent.png").to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add parent");
    let child = core
        .asset_service
        .add(
            add_command(&persona.id, tmp.path().join("child.png").to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add child");

    // The body lies about the target; the URL must win.
    let (status, body) = call(
        &router,
        post(
            &format!("/asterism/assets/{}/provenance", child.id),
            serde_json::json!({
                "asset_id": parent.id,
                "derived_from": format!("asset:{}", parent.id),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "declare: {body}");
    assert_eq!(
        body.get("id").and_then(|v| v.as_str()),
        Some(child.id.as_str()),
        "the URL's asset is the one that got the claim"
    );
    let extra: serde_json::Value = serde_json::from_str(
        body.get("extra_json")
            .and_then(|v| v.as_str())
            .expect("extra bag on the response"),
    )
    .expect("extra JSON");
    let trace = extra.get("_trace").expect("trace note");
    assert_eq!(trace.get("resolved").and_then(|v| v.as_bool()), Some(true));
    // Channel bookkeeping: this endpoint is the after-the-fact
    // declaration channel, so the note records `source: "manual"` —
    // never caller-asserted, derived from the route itself.
    assert_eq!(
        trace.get("source").and_then(|v| v.as_str()),
        Some("manual"),
        "a claim through the repair verb is recorded as manual"
    );

    // The edge is readable back through the lineage route.
    let (status, view) = call(
        &router,
        get(&format!("/asterism/assets/{}/lineage", child.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "lineage: {view}");
    let parents: Vec<&str> = view
        .get("nodes")
        .and_then(|n| n.as_array())
        .expect("nodes")
        .iter()
        .filter(|n| n.get("depth").and_then(|d| d.as_i64()) == Some(1))
        .filter_map(|n| n.pointer("/card/id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        parents,
        vec![parent.id.as_str()],
        "the declared parent sits one hop above"
    );
}

/// AlbumMeta over HTTP: file a statement, correct it, take it back.
///
/// The route sits next to `update-meta`, which means something else
/// entirely (the asset's own columns), so the path is asserted as much
/// as the effect. The decoy carries the same override check the comment
/// route needs: two callers disagreeing about the target must not write
/// to different rows and both report success.
#[tokio::test(flavor = "multi_thread")]
async fn the_album_meta_route_files_a_statement_on_the_asset_in_the_url() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("stated.md");
    std::fs::write(&file, "body\n").expect("write file");
    let other = corpus.join("decoy.md");
    std::fs::write(&other, "other\n").expect("write other");

    let (core, router) = harness(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-routes-album-meta".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let asset = core
        .asset_service
        .add(
            add_command(&persona.id, file.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add asset");
    let decoy = core
        .asset_service
        .add(
            add_command(&persona.id, other.to_str().unwrap()),
            &unattributed(),
        )
        .await
        .expect("add decoy");

    // The decoy gets a statement of its own first. Without one its bag
    // is empty either way, and "the decoy did not receive it" would pass
    // against a handler that ignored the URL entirely — the axis has to
    // disagree with the default before the assertion means anything.
    let (status, seeded) = call(
        &router,
        post(
            &format!("/asterism/assets/{}/album-meta", decoy.id),
            serde_json::json!({ "asset_id": decoy.id, "key": "plate", "value": "decoy-own" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "seed the decoy: {seeded}");

    // The body names the decoy on purpose — the URL must win.
    let (status, stated) = call(
        &router,
        post(
            &format!("/asterism/assets/{}/album-meta", asset.id),
            serde_json::json!({
                "asset_id": decoy.id,
                "key": "workflow-id",
                "value": "wf-http-1",
                "operator_ai": "claude-code",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "declare: {stated}");
    assert_eq!(
        stated["id"], asset.id,
        "the URL decides the target asset, not the body"
    );
    let bag: serde_json::Value =
        serde_json::from_str(stated["extra_json"].as_str().expect("extra_json")).expect("json");
    assert_eq!(
        bag["_trace"]["meta"]["workflow-id"]["value"],
        serde_json::json!("wf-http-1")
    );
    // `manual`, since this route is the after-the-fact one. The ingest
    // route's `album_meta` stamps `pushed` — the same names, told apart
    // by how they arrived.
    assert_eq!(
        bag["_trace"]["meta"]["workflow-id"]["source"],
        serde_json::json!("manual")
    );
    assert_eq!(
        bag["_trace"]["meta"]["workflow-id"]["operator"],
        serde_json::json!("claude-code")
    );

    // Omitting `value` is the removal spelling — there is no second
    // route for it, so this is where that decision is exercised.
    let (status, removed) = call(
        &router,
        post(
            &format!("/asterism/assets/{}/album-meta", asset.id),
            serde_json::json!({ "asset_id": asset.id, "key": "workflow-id" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "remove: {removed}");
    let bag: serde_json::Value =
        serde_json::from_str(removed["extra_json"].as_str().expect("extra_json")).expect("json");
    assert!(bag["_trace"].get("meta").is_none(), "{bag}");

    // The other half of the path-override assertion.
    let (status, decoy_detail) =
        call(&router, get(&format!("/asterism/assets/{}", decoy.id))).await;
    assert_eq!(status, StatusCode::OK, "decoy detail: {decoy_detail}");
    let decoy_bag: serde_json::Value = serde_json::from_str(
        decoy_detail
            .pointer("/asset/extra_json")
            .and_then(|v| v.as_str())
            .expect("the decoy carries its own statement"),
    )
    .expect("json");
    let keys: Vec<&String> = decoy_bag["_trace"]["meta"]
        .as_object()
        .expect("the decoy's meta object")
        .keys()
        .collect();
    assert_eq!(
        keys,
        vec!["plate"],
        "the asset named in the body must not receive the statement: {decoy_bag}"
    );
}
