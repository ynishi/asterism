# asterism-server 0.0.0

# asterism-server — library surface.

Exposes the shared backend initialisation ([`core_init`]) and the HTTP
transport ([`http::router`] over [`state::ServerCtx`]) so both the
standalone `asterism-server` binary and the Tauri UI can build the same
service graph. The binary (`main.rs`) is a thin CLI shell over this
library.

## Modules

- [`attribution`](attribution.md): Turning what a remote caller said into the attribution a write
- [`core_init`](core_init.md): Shared backend initialisation for both the Tauri UI and the standalone
- [`http`](http.md): HTTP transport — axum router.
- [`mcp`](mcp.md): MCP transport — the third adapter over the same application services.
- [`mcp_proxy`](mcp_proxy.md): MCP stdio proxy — the lifecycle-aware entry point MCP clients spawn.
- [`state`](state.md): Backend context for the standalone server. Thin wrapper over the

