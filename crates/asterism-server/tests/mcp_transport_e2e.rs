//! End-to-end guard for the MCP transport nested at `/mcp`.
//!
//! Drives the real [`asterism_server::http::router`] through `oneshot`,
//! the same way the HTTP route e2e does — so what is exercised is the
//! wiring: the tower service actually nested on the router, the JSON-RPC
//! session handshake, tool listing, schema generation from the contract
//! types, and tool calls landing on the same services the HTTP handlers
//! share.
//!
//! The client side speaks the legacy session flow (`initialize` →
//! `notifications/initialized` → requests carrying `Mcp-Session-Id`),
//! which is what today's MCP clients negotiate; responses arrive as SSE
//! frames whose `data:` lines carry the JSON-RPC messages.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Same tempdir harness as `newly_exposed_routes_e2e` — the Tantivy
/// index override keeps the test out of the developer's profile.
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

/// One JSON-RPC message POSTed to `/mcp`. Returns the status, the
/// `Mcp-Session-Id` response header (present on `initialize`), and the
/// last `data:` SSE frame parsed as JSON (`Null` for bodyless answers
/// such as the 202 a notification gets).
///
/// The whole exchange sits under a timeout: a session stream that never
/// terminates would otherwise hang `collect` forever, and a hang is a
/// worse failure report than a named timeout.
async fn mcp_call(
    router: &Router,
    session: Option<&str>,
    message: serde_json::Value,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        // `oneshot` builds a raw request with no Host header, but the
        // service's DNS-rebinding guard requires one and allows only
        // loopback names. Real clients always send it.
        .header("host", "127.0.0.1")
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json");
    if let Some(session) = session {
        builder = builder.header("mcp-session-id", session);
    }
    let request = builder
        .body(Body::from(message.to_string()))
        .expect("build MCP POST");
    let response = tokio::time::timeout(Duration::from_secs(20), async {
        let response = router.clone().oneshot(request).await.expect("router");
        let status = response.status();
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        (status, session, bytes)
    })
    .await
    .expect("MCP exchange timed out — the response stream never terminated");
    let (status, session, bytes) = response;
    let text = String::from_utf8_lossy(&bytes);
    let last_data = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).expect("SSE data frame is JSON"))
        .or_else(|| {
            // json_response mode answers plain JSON when it can.
            (!bytes.is_empty() && text.trim_start().starts_with('{'))
                .then(|| serde_json::from_slice(&bytes).expect("JSON body"))
        })
        .unwrap_or(serde_json::Value::Null);
    (status, session, last_data)
}

/// Runs the `initialize` → `notifications/initialized` handshake and
/// returns the session id plus the `InitializeResult`.
async fn handshake(router: &Router) -> (String, serde_json::Value) {
    let (status, session, reply) = mcp_call(
        router,
        None,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "e2e", "version": "0"},
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "initialize failed: {reply}");
    let session = session.expect("initialize answers with a session id");
    let (status, _, _) = mcp_call(
        router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "initialized notification");
    (session, reply["result"].clone())
}

/// Calls one tool and returns the parsed `CallToolResult`.
async fn tool_call(
    router: &Router,
    session: &str,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let (status, _, reply) = mcp_call(
        router,
        Some(session),
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tools/call {name}: {reply}");
    assert!(
        reply["error"].is_null(),
        "tools/call {name} answered a protocol error: {reply}"
    );
    reply["result"].clone()
}

/// Parses the JSON payload a tool packed into its text content block.
fn tool_json(result: &serde_json::Value) -> serde_json::Value {
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result has no text content: {result}"));
    serde_json::from_str(text).expect("tool content is JSON")
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

/// The handshake works and `tools/list` publishes the curated set with
/// contract-derived input schemas.
///
/// The schema assertion pins one field that only exists on the contract
/// type (`group_ids` on `ListAssetsQuery`): if the tool ever drifts to
/// a hand-written parameter struct, this is the line that notices.
#[tokio::test(flavor = "multi_thread")]
async fn the_mcp_endpoint_lists_the_curated_tools() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;

    let (session, init) = handshake(&router).await;
    assert_eq!(init["serverInfo"]["name"], "asterism");
    assert!(
        init["instructions"]
            .as_str()
            .expect("instructions present")
            .contains("catalog_overview"),
        "instructions should route an agent to the discovery tool"
    );

    let (status, _, reply) = mcp_call(
        &router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tools = reply["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list has no tools array: {reply}"));
    let mut names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "asset_add",
            "asset_comment_add",
            "asset_comments",
            "asset_culls",
            "asset_declare_meta",
            "asset_get",
            "asset_lineage",
            "asset_list",
            "asset_search",
            "assets_merge",
            "catalog_overview",
            "dispatch_get",
            "duplicate_conflict_resolve",
            "duplicate_conflicts",
            "material_layers",
            "material_mark_add",
            "material_marks",
            "project_get",
            "project_list",
            "project_open",
            "pursuit_close",
            "pursuit_open",
            "pursuit_reopen",
            "pursuit_restamp_dispatch",
            "pursuit_tx",
            "pursuit_view",
        ],
        "the curated tool vocabulary changed"
    );

    let asset_list = tools
        .iter()
        .find(|tool| tool["name"] == "asset_list")
        .expect("asset_list is published");
    let properties = &asset_list["inputSchema"]["properties"];
    assert!(
        !properties["group_ids"].is_null() && !properties["sort"].is_null(),
        "asset_list input schema should come from ListAssetsQuery: {properties}"
    );

    // Same point on the write side, and the reason `asset_add` needs no
    // hand-maintained schema: the tool takes `AddAssetCommand` itself,
    // so a field added to the contract reaches an agent's tool list
    // without anything here being edited. `on_duplicate` is the newest
    // one, and it carries a closed set — which is worth pinning because
    // a caller reading the schema is how the three answers become
    // discoverable at all.
    let asset_add = tools
        .iter()
        .find(|tool| tool["name"] == "asset_add")
        .expect("asset_add is published");
    let add_schema = asset_add["inputSchema"].to_string();
    assert!(
        add_schema.contains("on_duplicate"),
        "asset_add input schema should follow AddAssetCommand: {add_schema}"
    );
    for answer in ["ask", "fold", "separate"] {
        assert!(
            add_schema.contains(answer),
            "the closed set should reach the schema, {answer} did not: {add_schema}"
        );
    }

    // The onboarding guide resource is listed and readable — the pair
    // enable_resources() + list_resources + read_resource all need to
    // agree, and a mismatch between the URI a client discovers here and
    // the one the read handler recognises is exactly the kind of drift
    // a single-file constant should prevent.
    let (status, _, reply) = mcp_call(
        &router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list", "params": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let resources = reply["result"]["resources"]
        .as_array()
        .unwrap_or_else(|| panic!("resources/list has no resources array: {reply}"));
    let uris: Vec<&str> = resources
        .iter()
        .map(|r| r["uri"].as_str().expect("resource uri"))
        .collect();
    assert!(
        uris.contains(&"asterism://guides/onboarding"),
        "the onboarding guide should be published: {uris:?}"
    );

    let (status, _, reply) = mcp_call(
        &router,
        Some(&session),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "resources/read",
            "params": {"uri": "asterism://guides/onboarding"},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let contents = &reply["result"]["contents"][0];
    assert_eq!(
        contents["mimeType"], "text/markdown",
        "onboarding guide is markdown"
    );
    let body = contents["text"].as_str().expect("guide text");
    assert!(
        body.contains("catalog_overview") && body.contains("asset_lineage"),
        "the guide body should mention the recommended tool flow"
    );
}

/// Tool calls land on the same services as HTTP: a write through
/// `asset_add` is visible to `asset_list` and `asset_comments`, and a
/// domain failure comes back as a tool-level error carrying the same
/// `{kind, message}` shape as the HTTP boundary — not a protocol error.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_tools_read_and_write_the_same_ledger_as_http() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("via-mcp.md");
    std::fs::write(&file, "ingested over MCP\n").expect("write file");

    let (core, router) = harness(tmp.path()).await;
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "MCP E2E".into(),
                pack_id: Some("e2e-mcp".into()),
            },
            // The fixture is not the surface under test here: the MCP
            // tools reach the services through their own adapter, which
            // states nothing on a persona write.
            &asterism_core::domain::attribution::AttributionContext::asserted(None, None)
                .expect("stating no author and no operator is always valid"),
        )
        .await
        .expect("register persona");

    let (session, _) = handshake(&router).await;

    // Discovery answers with the persona registered above.
    let overview = tool_json(
        &tool_call(
            &router,
            &session,
            10,
            "catalog_overview",
            serde_json::json!({}),
        )
        .await,
    );
    let persona_ids: Vec<&str> = overview["personas"]
        .as_array()
        .expect("personas array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(
        persona_ids.contains(&persona.id.as_str()),
        "catalog_overview should list the registered persona"
    );

    // Write through the tool, read it back through the tool. The ingest
    // carries an AlbumMeta statement, which is the case this pair of
    // surfaces exists for: an agent is the caller that knows a
    // generator's own reference at import time.
    let mut add = add_command(&persona.id, file.to_str().unwrap());
    add.album_meta = [("workflow-id".to_string(), "wf-mcp-1".to_string())]
        .into_iter()
        .collect();
    let command = serde_json::to_value(add).expect("serialise AddAssetCommand");
    let added = tool_json(&tool_call(&router, &session, 11, "asset_add", command).await);
    let asset_id = added["id"].as_str().expect("added asset id").to_owned();

    let listed = tool_json(
        &tool_call(
            &router,
            &session,
            12,
            "asset_list",
            serde_json::json!({"offset": 0, "limit": 10}),
        )
        .await,
    );
    let listed_ids: Vec<&str> = listed["items"]
        .as_array()
        .or_else(|| listed["assets"].as_array())
        .unwrap_or_else(|| panic!("asset_list page carries no item array: {listed}"))
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(listed_ids, vec![asset_id.as_str()]);

    // The statement that rode in on the command, and then the tool that
    // corrects it. `pushed` → `manual` is the whole point of recording a
    // channel: an agent finding out later that the value it handed over
    // was wrong is a different kind of evidence from the handover.
    let bag: serde_json::Value =
        serde_json::from_str(added["extra_json"].as_str().expect("extra_json")).expect("json");
    assert_eq!(
        bag["_trace"]["meta"]["workflow-id"]["value"],
        serde_json::json!("wf-mcp-1")
    );
    assert_eq!(
        bag["_trace"]["meta"]["workflow-id"]["source"],
        serde_json::json!("pushed")
    );
    let corrected = tool_json(
        &tool_call(
            &router,
            &session,
            16,
            "asset_declare_meta",
            serde_json::json!({
                "asset_id": asset_id,
                "key": "workflow-id",
                "value": "wf-mcp-2",
                "operator_ai": "claude-code",
            }),
        )
        .await,
    );
    let bag: serde_json::Value =
        serde_json::from_str(corrected["extra_json"].as_str().expect("extra_json")).expect("json");
    let entry = &bag["_trace"]["meta"]["workflow-id"];
    assert_eq!(entry["value"], serde_json::json!("wf-mcp-2"));
    assert_eq!(entry["source"], serde_json::json!("manual"));
    assert_eq!(entry["operator"], serde_json::json!("claude-code"));

    // The comment round-trip, authored as the persona the agent acts
    // for — `author_kind` is the comment model's closed `user` /
    // `persona` set, so an agent's own voice is a persona's.
    let comment = tool_json(
        &tool_call(
            &router,
            &session,
            13,
            "asset_comment_add",
            serde_json::json!({
                "asset_id": asset_id,
                "author_kind": "persona",
                "author_persona_id": persona.id,
                "body": "posted over MCP",
            }),
        )
        .await,
    );
    assert_eq!(comment["body"], "posted over MCP");
    let thread = tool_json(
        &tool_call(
            &router,
            &session,
            14,
            "asset_comments",
            serde_json::json!({"asset_id": asset_id}),
        )
        .await,
    );
    assert_eq!(thread.as_array().map(Vec::len), Some(1));

    // A domain failure is a readable answer, not a broken call. The id
    // is shaped like an asset id (a well-formed UUID) so the failure is
    // the lookup, not the parse.
    let missing = tool_call(
        &router,
        &session,
        15,
        "asset_get",
        serde_json::json!({
            "asset_id": "00000000-0000-7000-8000-000000000000",
            "viewer_subject": null,
        }),
    )
    .await;
    assert_eq!(
        missing["isError"], true,
        "a NotFound should surface as a tool error: {missing}"
    );
    let error = tool_json(&missing);
    assert_eq!(
        error["kind"], "NotFound",
        "tool errors carry the HTTP boundary's kind: {error}"
    );
}

/// The agent-facing half of the pursuit surface: name the line of work,
/// file a round under it, and read back what is in it.
///
/// The dispatch itself is started over HTTP because there is no tool for
/// it — which is the point of the read: an agent that named the pursuit
/// can see the round somebody else filed under it, because both surfaces
/// write the same rows.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_opens_a_pursuit_and_reads_what_is_filed_under_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let file = corpus.join("plate.md");
    std::fs::write(&file, "# plate\n").expect("write file");

    let (core, router) = harness(tmp.path()).await;
    let unattributed = asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid");
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "MCP Pursuit".into(),
                pack_id: Some("e2e-mcp-pursuit".into()),
            },
            &unattributed,
        )
        .await
        .expect("register persona");
    let source = core
        .asset_service
        .add(
            add_command(&persona.id, file.to_str().unwrap()),
            &unattributed,
        )
        .await
        .expect("add source asset");

    let (session, _) = handshake(&router).await;

    let opened = tool_json(
        &tool_call(
            &router,
            &session,
            20,
            "pursuit_open",
            serde_json::json!({
                "persona_id": persona.id,
                "title": "variants for the cover",
                "operator_ai": "claude-code",
            }),
        )
        .await,
    );
    assert_eq!(opened["standing"], "open");
    let pursuit_id = opened["id"].as_str().expect("pursuit id").to_owned();

    // A round filed under the pursuit the agent just named, through the
    // surface that has the verb.
    let snapshot = core
        .snapshot_service
        .create(
            asterism_contract::command::CreateSnapshotCommand {
                persona_id: persona.id.clone(),
                asset_ids: vec![source.id.clone()],
            },
            &unattributed,
        )
        .await
        .expect("freeze");
    let round = core
        .dispatch_service
        .create(
            asterism_contract::command::CreateDispatchCommand {
                snapshot_id: snapshot.id.clone(),
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: Some("claude-code".into()),
                pursuit_id: Some(pursuit_id.clone()),
            },
            &unattributed,
        )
        .await
        .expect("dispatch under the named pursuit");
    assert_eq!(round.pursuit_id.as_deref(), Some(pursuit_id.as_str()));

    let view = tool_json(
        &tool_call(
            &router,
            &session,
            21,
            "pursuit_view",
            serde_json::json!({ "pursuit_id": pursuit_id }),
        )
        .await,
    );
    assert_eq!(view["pursuit"]["standing"], "open");
    let rounds = view["rounds"].as_array().expect("rounds array");
    assert_eq!(rounds.len(), 1, "the round reads back: {view}");
    assert_eq!(rounds[0]["id"], serde_json::json!(round.id));

    // The source enters the ledger over the same transport — a
    // verdict names a candidate, and the candidate set is derived,
    // never supplied.
    let entered = tool_json(
        &tool_call(
            &router,
            &session,
            24,
            "pursuit_tx",
            serde_json::json!({
                "pursuit_id": pursuit_id,
                "kind": "in",
                "asset_id": source.id,
                "origin": "existing",
                "operator_ai": "claude-code",
            }),
        )
        .await,
    );
    assert_eq!(entered["kind"], "in", "the gesture reads back: {entered}");

    // Concluding it is a recorded act with a frozen kept set, and the
    // standing the next read gives back. The source entered as
    // `existing`, so the statement it takes is `reject` — and the
    // close still freezes nothing, which is the point being asserted:
    // kept is the keep verdicts, not the survivors.
    let closed = tool_json(
        &tool_call(
            &router,
            &session,
            22,
            "pursuit_close",
            serde_json::json!({
                "pursuit_id": pursuit_id,
                "outcome": "satisfied",
                "verdicts": [{ "asset_id": source.id, "verdict": "reject" }],
                "operator_ai": "claude-code",
            }),
        )
        .await,
    );
    assert_eq!(closed["kind"], "closed_satisfied");
    assert!(
        closed["snapshot_id"].is_null(),
        "no keep verdicts, nothing frozen: {closed}"
    );
    let history = tool_json(
        &tool_call(
            &router,
            &session,
            25,
            "asset_culls",
            serde_json::json!({ "asset_id": source.id }),
        )
        .await,
    );
    let verdicts = history.as_array().expect("verdict rows");
    assert_eq!(verdicts.len(), 1, "one act judged this asset: {history}");
    assert_eq!(verdicts[0]["verdict"], "reject");
    assert_eq!(verdicts[0]["pursuit_id"], serde_json::json!(pursuit_id));
    let after = tool_json(
        &tool_call(
            &router,
            &session,
            23,
            "pursuit_view",
            serde_json::json!({ "pursuit_id": pursuit_id }),
        )
        .await,
    );
    assert_eq!(after["pursuit"]["standing"], "closed_satisfied");
}
