# asterism-server::mcp_proxy

MCP stdio proxy — the lifecycle-aware entry point MCP clients spawn.

## Why a proxy instead of serving the tools directly over stdio

The tool surface lives in the running app (Full core: writer lock +
job worker) and is served at `/mcp` on the loopback port. A client
that connects to that port directly couples its session to the app
process's lifetime: the client must be (re)started after the app,
and a backend rebuild mid-session strands it. This proxy inverts the
dependency:

- **Starts with no backend.** `get_info` is answered locally, so the
  MCP handshake succeeds even when the app is down.
- **Connects on access.** Every forwarded request first ensures the
  backend is up (`/asterism/health`); if it is not, the app is
  launched and the health endpoint polled before forwarding.
- **Forwards schemas, does not own them.** `tools/list`,
  `tools/call`, `resources/list`, `resources/read` are relayed to
  the backend's `/mcp`, so tool shapes always describe the build
  that is actually serving — this binary does not need a rebuild
  when the tool set changes.
- **Owns the lifecycle vocabulary.** `app_status` (probe, never
  launches) and `app_restart` (shutdown → relaunch → health poll)
  are implemented here, in the process that survives the backend's
  death — the piece a tool living inside the backend can never do
  cleanly.

A dropped backend connection is retried once through a fresh
connection before the error is surfaced, so an `app_restart` (or a
manual app quit + relaunch) does not strand the MCP session.

## Types

- `AppLaunch` — How the proxy brings the app up when the backend is unreachable.
- `McpProxy` — The stdio-facing MCP server that relays to the app's `/mcp`.

