//! End-to-end guard for the stdio MCP proxy (`asterism-server mcp`).
//!
//! Exercises the real hop chain: an rmcp client speaks stdio-shaped MCP
//! to [`McpProxy`] over an in-memory duplex, and the proxy forwards to
//! a real backend — the actual [`asterism_server::http::router`] bound
//! to an ephemeral loopback port, `/mcp` service and all. What this
//! proves is the lifecycle contract:
//!
//! - the proxy's handshake succeeds and lifecycle tools answer without
//!   the proxy owning any tool schema of its own;
//! - forwarded calls land on the same core the backend serves over
//!   HTTP (the persona registered through the core shows up through
//!   the proxied `catalog_overview`);
//! - with the backend down and launching disabled, a forwarded call
//!   fails with the named guidance error instead of hanging — the
//!   "starts with no backend, connects on access" half of the design.
//!
//! App launching itself (`open …`) is deliberately out of scope:
//! [`AppLaunch::Disabled`] exists exactly so this test cannot pop the
//! real desktop app on a developer machine.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::RegisterPersonaCommand;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::mcp_proxy::{AppLaunch, McpProxy};
use asterism_server::state::ServerCtx;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ReadResourceRequestParams};

/// Boots the real router on an ephemeral loopback port. The `CoreCtx`
/// must outlive the test body — dropping it shuts the service graph
/// down underneath the serve task.
async fn spawn_backend(tmp: &std::path::Path) -> (CoreCtx, u16) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (core, port)
}

/// Serves `proxy` over an in-memory duplex and returns a connected rmcp
/// client — the same wire shape a spawning MCP client would drive.
async fn connect_client(
    proxy: McpProxy,
) -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    let (server_read, server_write) = tokio::io::split(server_io);
    tokio::spawn(async move {
        // Hold the running service until the wire closes — dropping it
        // right after the handshake would cancel the session.
        if let Ok(service) = rmcp::serve_server(proxy, (server_read, server_write)).await {
            let _ = service.waiting().await;
        }
    });
    let (client_read, client_write) = tokio::io::split(client_io);
    ().serve((client_read, client_write))
        .await
        .expect("client handshake against the proxy")
}

/// Extracts the first text content block of a tool result as JSON.
fn tool_json(result: &CallToolResult) -> serde_json::Value {
    let value = serde_json::to_value(result).expect("serialize tool result");
    let text = value["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result carried no text content: {value}"));
    serde_json::from_str(text).expect("tool text content should be JSON")
}

#[tokio::test]
async fn proxy_forwards_to_the_live_backend_and_adds_lifecycle_tools() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, port) = spawn_backend(tmp.path()).await;

    // Seed through the core directly — if the proxied call can see this
    // persona, the forward landed on the same ledger, not a look-alike.
    let ctx = ServerCtx::from_core(&core);
    ctx.persona_service
        .register(
            RegisterPersonaCommand {
                name: "proxy-e2e".into(),
                pack_id: None,
            },
            // Seeding through the core, stating nothing: this test is
            // about where the proxied call lands, not about who wrote.
            &asterism_core::domain::attribution::AttributionContext::asserted(None, None)
                .expect("stating no author and no operator is always valid"),
        )
        .await
        .expect("register persona");

    let client = connect_client(McpProxy::new(port, AppLaunch::Disabled)).await;

    // Handshake answered locally by the proxy.
    let info = client.peer_info().expect("server info after handshake");
    let server_info = info
        .server_info
        .as_ref()
        .expect("proxy should identify itself");
    assert_eq!(server_info.name, "asterism");

    // tools/list = backend's curated set + the proxy's lifecycle pair.
    let tools = client.list_tools(None).await.expect("list_tools");
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "asset_search",
        "catalog_overview",
        "app_status",
        "app_restart",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }

    // Local lifecycle tool: health probe reports the backend identity.
    let status = client
        .call_tool(CallToolRequestParams::new("app_status"))
        .await
        .expect("app_status");
    let status = tool_json(&status);
    assert_eq!(
        status["up"], true,
        "backend should be reported up: {status}"
    );
    assert!(
        status["health"]["git_sha"].is_string() && status["health"]["pid"].is_number(),
        "health should carry build identity: {status}"
    );

    // Forwarded tool: the persona seeded through the core is visible.
    let overview = client
        .call_tool(CallToolRequestParams::new("catalog_overview"))
        .await
        .expect("catalog_overview");
    let overview = tool_json(&overview);
    let personas = overview["personas"]
        .as_array()
        .unwrap_or_else(|| panic!("personas array expected: {overview}"));
    assert!(
        personas.iter().any(|p| p["name"] == "proxy-e2e"),
        "proxied call should see the core-registered persona: {overview}"
    );

    // Forwarded resource: the onboarding guide comes from the backend.
    let guide = client
        .read_resource(ReadResourceRequestParams::new(
            "asterism://guides/onboarding",
        ))
        .await
        .expect("read onboarding guide");
    let guide = serde_json::to_value(&guide).expect("serialize resource");
    let body = guide["contents"][0]["text"].as_str().unwrap_or_default();
    assert!(
        body.contains("catalog_overview"),
        "guide body should be the backend's onboarding text"
    );
}

#[tokio::test]
async fn proxy_reports_unreachable_backend_instead_of_hanging() {
    // Reserve a port, then free it — nothing listens there afterwards.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);

    let client = connect_client(McpProxy::new(port, AppLaunch::Disabled)).await;

    let error = tokio::time::timeout(
        Duration::from_secs(10),
        client.call_tool(CallToolRequestParams::new("asset_search")),
    )
    .await
    .expect("must answer, not hang")
    .expect_err("forwarded call must fail with the backend down");
    let message = error.to_string();
    assert!(
        message.contains("launch is disabled"),
        "error should carry the launch guidance, got: {message}"
    );

    // The probe-only lifecycle tool still answers, reporting down.
    let status = client
        .call_tool(CallToolRequestParams::new("app_status"))
        .await
        .expect("app_status works with the backend down");
    let status = tool_json(&status);
    assert_eq!(status["up"], false, "backend should be down: {status}");
}
