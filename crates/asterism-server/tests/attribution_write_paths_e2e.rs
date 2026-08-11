//! Attribution on the write paths: what arrived is what gets recorded,
//! and nothing else does.
//!
//! Two rules run through every case here:
//!
//! 1. an attribution lands verbatim, **together with the channel it
//!    arrived through**, and
//! 2. its absence lands as **nothing** — no owner default, no key on the
//!    note, no operator invented from the fact that some code path was
//!    running.
//!
//! The second rule is the one that needs a test. A default would look
//! right in every screenshot and would be indistinguishable from a real
//! assertion the moment a second subject exists — which is exactly the
//! state a hosted migration would have to un-guess.
//!
//! The channel is what makes the first rule checkable at all. An
//! attribution with no channel is the shape rows written before the
//! column carry, so "who" alone cannot tell an owner the desktop app
//! recorded from an owner some HTTP client claimed to be.
//!
//! # What each surface is pinned through
//!
//! - **HTTP / MCP** — the real router (`oneshot`), so the pin covers the
//!   adapter's own translation of the command's assertion fields into a
//!   context, not just the service call underneath it.
//! - **The owner's surface** — the **service**, called with
//!   `AttributionContext::owner_surface()`. The Tauri command functions
//!   take `State<'_, AppState>` and there is no harness that can build
//!   one, so a test that called the service and claimed to be pinning
//!   Tauri would be naming something it did not exercise. What the Tauri
//!   arm actually passes is held by the type (the argument is required)
//!   and by ST2c's structural guard.
//!
//! Its own test binary because `init_core` opens a Tantivy index and
//! the sibling e2e files follow the same one-core-per-file shape.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{
    AddAssetCommand, CreateDispatchCommand, CreateSnapshotCommand, DeclareProvenanceCommand,
    RegisterPersonaCommand,
};
use asterism_contract::dto::DerivedDto;
use asterism_core::domain::attribution::{AttributionContext, OperatorRef};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

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

/// A caller that states nothing. Used where the fixture is about some
/// other axis of the same test.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None).expect("stating nothing is always valid")
}

async fn core_and_router(tmp: &std::path::Path, pack: &str) -> (CoreCtx, Router, String) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(pack.into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    (core, router, persona.id)
}

/// One JSON POST through the real router, read back as JSON.
async fn post_json(
    router: &Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("host", "127.0.0.1")
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
    // A rejection from the extractor layer answers plain text, so the
    // body is reported verbatim rather than turned into a panic about
    // JSON — the assertion that follows wants to name what came back.
    let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
    });
    (status, json)
}

/// `add_command` as a JSON object, so a case can set the assertion
/// fields on top of a body the extractor actually accepts.
fn add_body(persona_id: &str, locator: &str, occurred_at_ms: i64) -> serde_json::Value {
    serde_json::to_value(add_command(persona_id, locator, occurred_at_ms))
        .expect("AddAssetCommand serialises")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owners_own_surface_records_the_owner_and_how_it_got_there() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let owned = corpus.join("owned.md");
    let contradictory = corpus.join("contradictory.md");
    for path in [&owned, &contradictory] {
        std::fs::write(path, "# body\n").expect("write corpus file");
    }

    let (core, _router, persona) = core_and_router(tmp.path(), "e2e-attribution-owner").await;

    // The owner's surface answers both halves: *who* (the owner) and
    // *how the answer arrived* (the surface itself). No command field
    // took part — this is a fact about the entry point.
    let recorded = core
        .asset_service
        .add(
            add_command(&persona, owned.to_str().unwrap(), 1_785_000_000_000),
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("the owner's own surface may write");
    assert_eq!(recorded.author_kind.as_deref(), Some("owner"));
    assert_eq!(
        recorded.author_subject, None,
        "the owner is a reference to the instance, not a token of its own"
    );
    assert_eq!(
        recorded.attributed_via.as_deref(),
        Some("owner-surface"),
        "without the channel this row is indistinguishable from a claimed one"
    );
    assert_eq!(recorded.operator_ai, None);

    // A command that *also* states an attribution is a contradiction on
    // this surface, and it is refused rather than silently dropped: the
    // caller asked for something the write did not do.
    let mut stated = add_command(&persona, contradictory.to_str().unwrap(), 1_785_000_001_000);
    stated.author_kind = Some("subject".into());
    stated.author_subject = Some("alice".into());
    let err = core
        .asset_service
        .add(stated, &AttributionContext::owner_surface())
        .await
        .expect_err("the owner's surface cannot also be told who is writing");
    let message = err.to_string();
    assert!(
        message.contains("author_kind") && message.contains("author_subject"),
        "the refusal should name the fields that were set: {message}"
    );

    // And the refusal is total — nothing landed under that locator.
    let retried = core
        .asset_service
        .add(
            add_command(&persona, contradictory.to_str().unwrap(), 1_785_000_001_000),
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("the rejected ingest left no row behind to collide with");
    assert_eq!(retried.author_kind.as_deref(), Some("owner"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_ingest_records_the_claim_and_labels_it_as_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let asserted = corpus.join("asserted.md");
    let silent = corpus.join("silent.md");
    let claimed_owner = corpus.join("claimed-owner.md");
    let half_written = corpus.join("half-written.md");
    for path in [&asserted, &silent, &claimed_owner, &half_written] {
        std::fs::write(path, "# body\n").expect("write corpus file");
    }

    let (_core, router, persona) = core_and_router(tmp.path(), "e2e-attribution-http").await;

    // (1) A stated pair lands verbatim, labelled as a claim.
    let mut stated = add_body(&persona, asserted.to_str().unwrap(), 1_785_000_000_000);
    stated["author_kind"] = serde_json::json!("subject");
    stated["author_subject"] = serde_json::json!("alice");
    stated["operator_ai"] = serde_json::json!("claude-code");
    let (status, body) = post_json(&router, "/asterism/assets/add", stated).await;
    assert_eq!(status, StatusCode::OK, "add failed: {body}");
    assert_eq!(body["author_kind"], "subject");
    assert_eq!(body["author_subject"], "alice");
    assert_eq!(body["operator_ai"], "claude-code");
    assert_eq!(
        body["attributed_via"], "asserted",
        "an HTTP caller is believed and labelled as such"
    );

    // (2) Stating nothing records nothing — including the channel. A row
    //     that attributes nobody must not say it was asserted.
    let (status, body) = post_json(
        &router,
        "/asterism/assets/add",
        add_body(&persona, silent.to_str().unwrap(), 1_785_000_001_000),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "add failed: {body}");
    assert!(
        body.get("author_kind").is_none() || body["author_kind"].is_null(),
        "an absent author must not default to the owner: {body}"
    );
    assert!(body.get("author_subject").is_none() || body["author_subject"].is_null());
    assert!(body.get("operator_ai").is_none() || body["operator_ai"].is_null());
    assert!(
        body.get("attributed_via").is_none() || body["attributed_via"].is_null(),
        "no attribution means no channel either: {body}"
    );

    // (3) A remote caller cannot call itself the owner. If it could, its
    //     rows would be indistinguishable from the desktop app's.
    let mut claim = add_body(&persona, claimed_owner.to_str().unwrap(), 1_785_000_002_000);
    claim["author_kind"] = serde_json::json!("owner");
    let (status, body) = post_json(&router, "/asterism/assets/add", claim).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an owner claim over HTTP must be refused, not recorded: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("owner"),
        "the refusal should say what it refused: {body}"
    );

    // (4) A pair that cannot hold together is refused rather than
    //     rounded down to something plausible.
    let mut half = add_body(&persona, half_written.to_str().unwrap(), 1_785_000_003_000);
    half["author_kind"] = serde_json::json!("subject");
    let (status, body) = post_json(&router, "/asterism/assets/add", half).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "\"subject\" with no subject names nobody: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_mcp_ingest_is_an_assertion_like_any_other_remote_caller() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let plate = corpus.join("mcp.md");
    std::fs::write(&plate, "# body\n").expect("write corpus file");

    let (_core, router, persona) = core_and_router(tmp.path(), "e2e-attribution-mcp").await;

    // The MCP surface shares the service with HTTP but has its own
    // translation site, so the channel it records is its own claim.
    let session = mcp_handshake(&router).await;
    let mut arguments = add_body(&persona, plate.to_str().unwrap(), 1_785_000_000_000);
    arguments["author_kind"] = serde_json::json!("subject");
    arguments["author_subject"] = serde_json::json!("alice");
    arguments["operator_ai"] = serde_json::json!("claude-code");
    let reply = mcp_tool_call(&router, &session, "asset_add", arguments).await;
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("asset_add returned no text payload: {reply}"));
    let asset: serde_json::Value = serde_json::from_str(text).expect("the payload is the AssetDto");
    assert_eq!(asset["author_kind"], "subject");
    assert_eq!(asset["author_subject"], "alice");
    assert_eq!(asset["operator_ai"], "claude-code");
    assert_eq!(
        asset["attributed_via"], "asserted",
        "an MCP client states its own attribution, exactly like an HTTP one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_repair_verb_records_who_repaired_and_leaves_authorship_alone() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let by_agent = corpus.join("by-agent.md");
    let by_nobody = corpus.join("by-nobody.md");
    for path in [&by_agent, &by_nobody] {
        std::fs::write(path, "# body\n").expect("write corpus file");
    }

    let (core, _router, persona) = core_and_router(tmp.path(), "e2e-attribution-repair").await;

    // The asset is authored by someone; the repair is performed by an
    // agent. The two must not collide.
    let authored = core
        .asset_service
        .add(
            add_command(&persona, by_agent.to_str().unwrap(), 1_785_000_000_000),
            &AttributionContext::asserted(
                Some(asterism_core::domain::attribution::Author::Subject(
                    "alice".into(),
                )),
                None,
            )
            .expect("a named subject is what the asserted channel is for"),
        )
        .await
        .expect("add authored asset");

    // A claim that does not resolve still writes a note, which is what
    // makes this readable without a second asset to point at.
    let missing = "0198c1c2-0000-7000-8000-000000000000";
    let repaired = core
        .asset_service
        .declare_provenance(
            DeclareProvenanceCommand {
                asset_id: authored.id.clone(),
                derived_from: format!("asset:{missing}"),
                relation: None,
                operator_ai: Some("codex".into()),
            },
            // The verb takes a context like every mutation and does not
            // use it: `operator_ai` here is part of the *claim*, a
            // different subject from the row's own attribution.
            &unattributed(),
        )
        .await
        .expect("declare provenance");

    let extra: serde_json::Value =
        serde_json::from_str(repaired.extra_json.as_deref().expect("a trace note"))
            .expect("extra is JSON");
    let trace = extra.get("_trace").expect("the claim is recorded");
    assert_eq!(
        trace.get("operator").and_then(|v| v.as_str()),
        Some("codex"),
        "the note names the agent that made the declaration"
    );
    assert_eq!(
        trace.get("source").and_then(|v| v.as_str()),
        Some("manual"),
        "channel and operator are different axes and both survive"
    );
    assert_eq!(
        repaired.author_subject.as_deref(),
        Some("alice"),
        "repairing a link is not authoring the asset"
    );
    assert_eq!(
        repaired.attributed_via.as_deref(),
        Some("asserted"),
        "and the row keeps the channel its own ingest arrived through"
    );
    assert_eq!(
        repaired.operator_ai, None,
        "the repairing agent is a fact about the repair, not about the row"
    );

    // No operator asserted → the key is absent, not null. A null would
    // read as a value someone wrote down.
    let silent_asset = core
        .asset_service
        .add(
            add_command(&persona, by_nobody.to_str().unwrap(), 1_785_000_001_000),
            &unattributed(),
        )
        .await
        .expect("add second asset");
    let silent_repair = core
        .asset_service
        .declare_provenance(
            DeclareProvenanceCommand {
                asset_id: silent_asset.id.clone(),
                derived_from: format!("asset:{missing}"),
                relation: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("declare provenance without an operator");
    let extra: serde_json::Value =
        serde_json::from_str(silent_repair.extra_json.as_deref().expect("a trace note"))
            .expect("extra is JSON");
    let trace = extra.get("_trace").expect("the claim is recorded");
    assert!(
        trace.get("operator").is_none(),
        "an unasserted operator leaves no key at all: {trace}"
    );
    assert_eq!(
        trace.get("source").and_then(|v| v.as_str()),
        Some("manual"),
        "the channel is still derived structurally"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dispatch_carries_its_whole_attribution_through_to_what_it_reifies() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    let outbox = tmp.path().join("outbox");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    std::fs::create_dir_all(&outbox).expect("outbox dir");
    let plate = corpus.join("plate.md");
    std::fs::write(&plate, "# plate\n").expect("write plate");

    let (core, _router, persona) = core_and_router(tmp.path(), "e2e-attribution-dispatch").await;

    let source = core
        .asset_service
        .add(
            add_command(&persona, plate.to_str().unwrap(), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add source");
    let snapshot = core
        .snapshot_service
        .create(
            CreateSnapshotCommand {
                persona_id: persona.clone(),
                asset_ids: vec![source.id.clone()],
            },
            &unattributed(),
        )
        .await
        .expect("freeze snapshot");

    // Two runs over the same freeze, arriving through different
    // channels. What each output carries has to be *that run's* answer:
    // the reify happens through the runner, long after the caller that
    // supplied it has gone, so reading the job back from the repository
    // before reifying is the point — an attribution held only in memory
    // would not survive it.
    let asserted_run = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: snapshot.id.clone(),
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: Some("asterism-ui".into()),
            },
            &AttributionContext::asserted(
                None,
                Some(OperatorRef::new("asterism-ui").expect("a non-empty slug")),
            )
            .expect("an operator with no author is still an attribution"),
        )
        .await
        .expect("create asserted dispatch");
    let owned_run = core
        .dispatch_service
        .create(
            CreateDispatchCommand {
                snapshot_id: snapshot.id.clone(),
                exporter_slug: "file".into(),
                action: "write".into(),
                params_json: String::new(),
                operator_ai: None,
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("create owner-surface dispatch");

    let asserted_output = reify_one_output(&core, &asserted_run.id, &outbox, "asserted.md").await;
    assert_eq!(
        asserted_output.asset.operator_ai.as_deref(),
        Some("asterism-ui"),
        "the agent that started the run is the agent this output came through"
    );
    assert_eq!(
        asserted_output.asset.author_kind, None,
        "who a generated output is by is not a question the exporter can answer"
    );
    assert_eq!(
        asserted_output.asset.attributed_via.as_deref(),
        Some("asserted"),
        "the channel keeps its dispatch-time meaning: how the request to \
         start the run arrived, not the background job that finished it"
    );

    let extra: serde_json::Value = serde_json::from_str(
        asserted_output
            .asset
            .extra_json
            .as_deref()
            .expect("the dispatch trace"),
    )
    .expect("extra is JSON");
    let trace = extra.get("_dispatch").expect("the dispatch trace");
    assert_eq!(
        trace.get("operator").and_then(|v| v.as_str()),
        Some("asterism-ui"),
        "the run's own note names its operator, so a purged output does not erase it"
    );

    let owned_output = reify_one_output(&core, &owned_run.id, &outbox, "owned.md").await;
    assert_eq!(
        owned_output.asset.author_kind.as_deref(),
        Some("owner"),
        "a run started from the owner's own surface produces the owner's assets"
    );
    assert_eq!(owned_output.asset.author_subject, None);
    assert_eq!(
        owned_output.asset.attributed_via.as_deref(),
        Some("owner-surface"),
        "and the channel survives an outlet no constructor could have produced"
    );
    assert_eq!(
        owned_output.asset.operator_ai, None,
        "the owner's surface says who, not through what"
    );
}

/// Reifies a one-file export for `dispatch_id` and returns the detail of
/// the single asset it produced.
async fn reify_one_output(
    core: &CoreCtx,
    dispatch_id: &str,
    outbox: &std::path::Path,
    name: &str,
) -> asterism_contract::dto::AssetDetailDto {
    let exported = outbox.join(name);
    std::fs::write(&exported, "exported\n").expect("write export");
    let id = asterism_core::domain::value::DispatchId::from_uuid(
        uuid::Uuid::parse_str(dispatch_id).expect("dispatch id is a uuid"),
    );
    let job = core
        .support
        .dispatch_runner
        .reify(
            &id,
            vec![DerivedDto {
                modality: "work_product".into(),
                locator: exported.to_string_lossy().into_owned(),
                occurred_at: chrono::Utc::now(),
                cover_hint: None,
                register_note: None,
                labels: Vec::new(),
                file_size_bytes: None,
                duration_ms: None,
                extra: serde_json::Value::Null,
                batch_hint: None,
            }],
        )
        .await
        .expect("reify");
    let output_id = job
        .output_asset_ids
        .first()
        .expect("one exported copy")
        .to_string();
    core.asset_service
        .detail(asterism_contract::query::GetAssetDetailQuery {
            asset_id: output_id,
            viewer_subject: None,
        })
        .await
        .expect("read the reified asset")
}

/// `initialize` → `notifications/initialized`, returning the session id.
/// Same legacy session flow the MCP transport e2e drives.
async fn mcp_handshake(router: &Router) -> String {
    let (status, session, reply) = mcp_post(
        router,
        None,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "attribution-e2e", "version": "0"},
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "initialize failed: {reply}");
    let session = session.expect("initialize returns a session id");
    let (status, _, _) = mcp_post(
        router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert!(status.is_success(), "initialized notification failed");
    session
}

async fn mcp_tool_call(
    router: &Router,
    session: &str,
    tool: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let (status, _, reply) = mcp_post(
        router,
        Some(session),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tool} failed: {reply}");
    assert!(
        reply["result"]["isError"] != serde_json::Value::Bool(true),
        "{tool} answered an error: {reply}"
    );
    reply
}

/// One JSON-RPC message POSTed to `/mcp`; responses arrive as SSE frames
/// whose `data:` lines carry the JSON-RPC messages.
async fn mcp_post(
    router: &Router,
    session: Option<&str>,
    message: serde_json::Value,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        // `oneshot` builds a raw request with no Host header, but the
        // service's DNS-rebinding guard requires one and allows only
        // loopback names.
        .header("host", "127.0.0.1")
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json");
    if let Some(session) = session {
        builder = builder.header("mcp-session-id", session);
    }
    let request = builder
        .body(Body::from(message.to_string()))
        .expect("build MCP POST");
    let (status, session, bytes) = tokio::time::timeout(Duration::from_secs(20), async {
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
    let text = String::from_utf8_lossy(&bytes);
    let last_data = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).expect("SSE data frame is JSON"))
        .or_else(|| {
            (!bytes.is_empty() && text.trim_start().starts_with('{'))
                .then(|| serde_json::from_slice(&bytes).expect("JSON body"))
        })
        .unwrap_or(serde_json::Value::Null);
    (status, session, last_data)
}
