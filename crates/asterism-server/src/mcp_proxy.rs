//! MCP stdio proxy — the lifecycle-aware entry point MCP clients spawn.
//!
//! ## Why a proxy instead of serving the tools directly over stdio
//!
//! The tool surface lives in the running app (Full core: writer lock +
//! job worker) and is served at `/mcp` on the loopback port. A client
//! that connects to that port directly couples its session to the app
//! process's lifetime: the client must be (re)started after the app,
//! and a backend rebuild mid-session strands it. This proxy inverts the
//! dependency:
//!
//! - **Starts with no backend.** `get_info` is answered locally, so the
//!   MCP handshake succeeds even when the app is down.
//! - **Connects on access.** Every forwarded request first ensures the
//!   backend is up (`/asterism/health`); if it is not, the app is
//!   launched and the health endpoint polled before forwarding.
//! - **Forwards schemas, does not own them.** `tools/list`,
//!   `tools/call`, `resources/list`, `resources/read` are relayed to
//!   the backend's `/mcp`, so tool shapes always describe the build
//!   that is actually serving — this binary does not need a rebuild
//!   when the tool set changes.
//! - **Owns the lifecycle vocabulary.** `app_status` (probe, never
//!   launches) and `app_restart` (shutdown → relaunch → health poll)
//!   are implemented here, in the process that survives the backend's
//!   death — the piece a tool living inside the backend can never do
//!   cleanly.
//!
//! A dropped backend connection is retried once through a fresh
//! connection before the error is surfaced, so an `app_restart` (or a
//! manual app quit + relaunch) does not strand the MCP session.

use std::path::PathBuf;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{Peer, RequestContext, RoleClient, RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};

/// How the proxy brings the app up when the backend is unreachable.
pub enum AppLaunch {
    /// `open -a Asterism` — the app resolved by LaunchServices name.
    Default,
    /// Explicit target: a `.app` bundle (via `open`) or a plain binary
    /// (spawned directly, e.g. a headless serve).
    Path(PathBuf),
    /// Never launch; report unreachable instead. Test harnesses use
    /// this so a proxy pointed at a dead port cannot pop the real app.
    Disabled,
}

/// The stdio-facing MCP server that relays to the app's `/mcp`.
pub struct McpProxy {
    base_url: String,
    launch: AppLaunch,
    http: reqwest::Client,
    /// Cached backend connection; `None` until first forwarded request
    /// and after an invalidation. Guarded by one lock so concurrent
    /// first-calls produce one launch + one connection, not a stampede.
    backend: tokio::sync::Mutex<Option<RunningService<RoleClient, ()>>>,
}

/// One probe / relaunch poll step.
const POLL_STEP: Duration = Duration::from_millis(500);
/// Launch → healthy ceiling (matches the restart recipe's 20s).
const LAUNCH_POLL_STEPS: u32 = 40;
/// Shutdown → port-down ceiling.
const SHUTDOWN_POLL_STEPS: u32 = 10;

impl McpProxy {
    /// Builds a proxy targeting `http://127.0.0.1:{port}`.
    pub fn new(port: u16, launch: AppLaunch) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            launch,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("reqwest client"),
            backend: tokio::sync::Mutex::new(None),
        }
    }

    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    /// `GET /asterism/health` — `Some(body)` when a backend answered.
    async fn backend_health(&self) -> Option<serde_json::Value> {
        let response = self
            .http
            .get(format!("{}/asterism/health", self.base_url))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json().await.ok()
    }

    /// Spawns the app without waiting for it (health polling is the
    /// caller's job). `open` returns immediately; a plain binary is
    /// left running detached.
    fn launch_app(&self) -> Result<(), McpError> {
        let mut command = match &self.launch {
            AppLaunch::Path(path) if path.extension().is_some_and(|ext| ext == "app") => {
                let mut c = tokio::process::Command::new("open");
                c.arg(path);
                c
            }
            AppLaunch::Path(path) => tokio::process::Command::new(path),
            AppLaunch::Default => {
                let mut c = tokio::process::Command::new("open");
                c.args(["-a", "Asterism"]);
                c
            }
            AppLaunch::Disabled => {
                return Err(McpError::internal_error(
                    format!(
                        "asterism backend is not reachable at {} and app launch is disabled",
                        self.base_url
                    ),
                    None,
                ));
            }
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().map_err(|e| {
            McpError::internal_error(
                format!(
                    "failed to launch the Asterism app ({e}); start it manually or point \
                     --app / $ASTERISM_APP at the bundle"
                ),
                None,
            )
        })?;
        Ok(())
    }

    /// Health-or-launch: returns once `/asterism/health` answers,
    /// launching the app if the first probe fails.
    async fn wait_backend_up(&self) -> Result<(), McpError> {
        if self.backend_health().await.is_some() {
            return Ok(());
        }
        self.launch_app()?;
        for _ in 0..LAUNCH_POLL_STEPS {
            tokio::time::sleep(POLL_STEP).await;
            if self.backend_health().await.is_some() {
                return Ok(());
            }
        }
        Err(McpError::internal_error(
            format!(
                "asterism backend did not become healthy at {} within {}s of launch",
                self.base_url,
                (POLL_STEP * LAUNCH_POLL_STEPS).as_secs()
            ),
            None,
        ))
    }

    /// Hands out a peer to the backend `/mcp`, ensuring the app is up
    /// and connecting lazily. The lock serializes concurrent
    /// first-access so exactly one launch/connect happens.
    async fn peer(&self) -> Result<Peer<RoleClient>, McpError> {
        let mut guard = self.backend.lock().await;
        if let Some(service) = guard.as_ref() {
            return Ok(service.peer().clone());
        }
        self.wait_backend_up().await?;
        let transport = StreamableHttpClientTransport::from_uri(self.mcp_url());
        let service = ().serve(transport).await.map_err(|e| {
            McpError::internal_error(
                format!("mcp connect to {} failed: {e}", self.mcp_url()),
                None,
            )
        })?;
        let peer = service.peer().clone();
        *guard = Some(service);
        Ok(peer)
    }

    /// Drops the cached backend connection (next access reconnects).
    async fn invalidate(&self) {
        let mut guard = self.backend.lock().await;
        if let Some(service) = guard.take() {
            let _ = service.cancel().await;
        }
    }

    /// Invalidate + reconnect — the single retry a forwarded request
    /// gets after a transport-level failure (typical after the backend
    /// restarted underneath a cached session).
    async fn reconnected_peer(&self) -> Result<Peer<RoleClient>, McpError> {
        self.invalidate().await;
        self.peer().await
    }

    /// The proxy-owned lifecycle tools appended to the backend's list.
    fn local_tools() -> Vec<Tool> {
        let empty_schema = || {
            serde_json::json!({ "type": "object", "properties": {} })
                .as_object()
                .cloned()
                .unwrap_or_default()
        };
        vec![
            Tool::new(
                "app_status",
                "Probe the Asterism app's /asterism/health without launching it. Returns \
                 {up, health?}; health carries git_sha / pid / started_at_ms — compare \
                 git_sha against the repo HEAD to tell whether a rebuild is actually the \
                 build that is serving.",
                empty_schema(),
            ),
            Tool::new(
                "app_restart",
                "Restart the Asterism app to pick up a new build: POST \
                 /asterism/admin/shutdown, wait for the port to go down, relaunch, and \
                 poll health (≤20s). Returns {before, after} health snapshots — a changed \
                 pid proves a new process took over; compare git_sha to verify the build. \
                 If the app was not running it is simply launched.",
                empty_schema(),
            ),
        ]
    }

    async fn tool_app_status(&self) -> CallToolResult {
        let body = match self.backend_health().await {
            Some(health) => serde_json::json!({ "up": true, "health": health }),
            None => serde_json::json!({ "up": false, "base_url": self.base_url }),
        };
        CallToolResult::success(vec![ContentBlock::text(body.to_string())])
    }

    async fn tool_app_restart(&self) -> Result<CallToolResult, McpError> {
        let before = self.backend_health().await;
        // The cached MCP session dies with the process; drop it first so
        // a concurrent forward reconnects instead of reusing a corpse.
        self.invalidate().await;
        if before.is_some() {
            let _ = self
                .http
                .post(format!("{}/asterism/admin/shutdown", self.base_url))
                .send()
                .await;
            for _ in 0..SHUTDOWN_POLL_STEPS {
                tokio::time::sleep(POLL_STEP).await;
                if self.backend_health().await.is_none() {
                    break;
                }
            }
        }
        self.wait_backend_up().await?;
        let after = self.backend_health().await;
        let body = serde_json::json!({
            "restarted": true,
            "before": before,
            "after": after,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            body.to_string(),
        )]))
    }
}

impl ServerHandler for McpProxy {
    fn get_info(&self) -> ServerInfo {
        // Answered locally — this is what lets the stdio handshake
        // succeed while the app is down.
        let mut info = ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        );
        info.server_info.name = "asterism".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(format!(
            "Asterism MCP proxy (stdio). Tools and resources are forwarded to the \
             Asterism app's /mcp at {}; if the app is not running it is launched on \
             first access. Local lifecycle tools: app_status (health probe, never \
             launches) and app_restart (graceful shutdown + relaunch — use after a \
             rebuild so the serving process picks up the new binary). Read \
             asterism://guides/onboarding for the tool flow.",
            self.base_url
        ));
        info
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let peer = self.peer().await?;
        let mut result = match peer.list_tools(request.clone()).await {
            Ok(result) => result,
            Err(_) => {
                let peer = self.reconnected_peer().await?;
                peer.list_tools(request).await.map_err(service_err)?
            }
        };
        // The backend answers in one page today; append the lifecycle
        // tools to the terminal page so a paging client sees them once.
        if result.next_cursor.is_none() {
            result.tools.extend(Self::local_tools());
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        match request.name.as_ref() {
            "app_status" => return Ok(CallToolResponse::Complete(self.tool_app_status().await)),
            "app_restart" => {
                return Ok(CallToolResponse::Complete(self.tool_app_restart().await?));
            }
            _ => {}
        }
        let peer = self.peer().await?;
        let result = match peer.call_tool(request.clone()).await {
            Ok(result) => result,
            Err(_) => {
                let peer = self.reconnected_peer().await?;
                peer.call_tool(request).await.map_err(service_err)?
            }
        };
        Ok(CallToolResponse::Complete(result))
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let peer = self.peer().await?;
        match peer.list_resources(request.clone()).await {
            Ok(result) => Ok(result),
            Err(_) => {
                let peer = self.reconnected_peer().await?;
                peer.list_resources(request).await.map_err(service_err)
            }
        }
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let peer = self.peer().await?;
        let result = match peer.read_resource(request.clone()).await {
            Ok(result) => result,
            Err(_) => {
                let peer = self.reconnected_peer().await?;
                peer.read_resource(request).await.map_err(service_err)?
            }
        };
        Ok(result.into())
    }
}

/// Maps a client-side [`ServiceError`] onto the error we answer the
/// stdio side with: a backend JSON-RPC error passes through verbatim,
/// anything transport-shaped becomes an internal error naming the hop.
fn service_err(err: ServiceError) -> McpError {
    match err {
        ServiceError::McpError(e) => e,
        other => McpError::internal_error(format!("backend mcp call failed: {other}"), None),
    }
}
