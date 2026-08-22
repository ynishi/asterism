# asterism-server::http

HTTP transport — axum router.

Route conventions: RPC-style endpoints that mirror the Tauri command
surface (for example `POST /asterism/assets/add`). Contract DTOs are
reused verbatim so the same shape flows through the HTTP body, the
Tauri IPC bridge, and the MCP tool schemas (`crate::mcp`, nested on
this router at `/mcp`).

The server is bound to loopback in v1 and does not authenticate
requests.

## Functions

- `record_webview_diag` — Re-emits one webview-origin diagnostic as a `tracing` event, which
- `router` — Builds the router; the caller binds a listener and calls

