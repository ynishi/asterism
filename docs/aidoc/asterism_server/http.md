# asterism-server::http

HTTP transport — axum router.

Route conventions: RPC-style endpoints that mirror the Tauri command
surface (for example `POST /asterism/assets/add`). Contract DTOs are
reused verbatim so the same shape flows through the HTTP body, the
Tauri IPC bridge, and the MCP tool schemas (`crate::mcp`, nested on
this router at `/mcp`).

The server is bound to loopback in v1 and does not authenticate
requests.

# Why the pursuit routes are plural

`dispatch` is singular here and `snapshots` is plural, so the
pursuit family had to pick one rather than match whichever
neighbour was read last. It is plural (`/asterism/pursuits/...`)
because it has the shape the plural families have and `dispatch`
does not: a collection read that answers with many rows
(`GET /asterism/pursuits?persona_id=…`, the multi-pursuit overview
the design calls a first-class need) alongside the per-id reads.
`personas`, `assets`, `groups`, `snapshots` and `threads` are all
spelled that way; `dispatch` is the outlier, and one outlier is not
a convention to extend.

## Functions

- `record_webview_diag` — Re-emits one webview-origin diagnostic as a `tracing` event, which
- `router` — Builds the router; the caller binds a listener and calls

