# asterism-server::mcp

MCP transport — the third adapter over the same application services.

Tools are a curated use-case vocabulary, not a mirror of the 100+
HTTP routes: an agent walking `tools/list` should see the ledger's
actual entry points (search / list / get / add / lineage / comments /
catalog / dispatch) and nothing that only exists to serve the grid's
rendering loop. Anything not covered here is reachable over the HTTP
API on the same port — `get_info` says so in `instructions`.

Being curated is the half of the rule that applies here, and
[`crate::http`]'s module doc is where all of it is written: HTTP and
Tauri are mirrors and owe each other every verb, while this surface
owes none and gains one only by somebody deciding an agent should be
offered it.

Input schemas come from the same `asterism-contract` types that back
HTTP bodies and Tauri IPC (contract feature `json-schema`), so the
three transports cannot drift on shape. Thin parameter structs exist
only where HTTP used path/query extractors instead of a body type
(mirroring `http.rs`'s `LineageParams` and friends).

This handler is served over **streamable-http**, nested at `/mcp` on
the loopback axum router ([`streamable_service`]) — it exists
wherever the HTTP API does (Tauri-embedded serve and the standalone
binary alike). MCP clients that spawn a child process use
`asterism-server mcp` instead, which is a lifecycle-aware stdio
*proxy* onto this same endpoint (see `crate::mcp_proxy`), not a
second instance of this handler.

Domain failures are reported as tool results (`is_error: true`) with
the same `{kind, message}` shape the HTTP `ApiError` writes, rather
than JSON-RPC protocol errors: a NotFound is an answer the calling
agent should read, not a broken call.

## Functions

- `streamable_service` — Builds the streamable-http tower service the axum router nests at

## Types

- `AssetCommentsParams` — `asset_comments` input.
- `AssetLineageParams` — `asset_lineage` input — mirrors the HTTP `LineageParams` query pair
- `AsterismMcp` — The MCP server handler — a thin tool facade over [`ServerCtx`].
- `CatalogOverviewParams` — `catalog_overview` input.
- `DispatchGetParams` — `dispatch_get` input.
- `DuplicateConflictsParams` — `duplicate_conflicts` input — the same persona / limit pair the
- `MaterialLayersParams` — `material_layers` input.
- `MaterialMarksParams` — `material_marks` input.

