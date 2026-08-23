# asterism-server::http

HTTP transport — axum router.

Route conventions: RPC-style endpoints that mirror the Tauri command
surface (for example `POST /asterism/assets/add`). Contract DTOs are
reused verbatim so the same shape flows through the HTTP body, the
Tauri IPC bridge, and the MCP tool schemas (`crate::mcp`, nested on
this router at `/mcp`).

The server is bound to loopback in v1 and does not authenticate
requests.

# What the three transports owe each other

Stated here because it is one rule about three crates, and the two
halves of it are not the same rule. `README.md` says it in one line
under **API**; this is the long version, and the other two point at
it rather than restating it.

**HTTP and Tauri owe each other every verb a person invokes, and
that is an obligation rather than a description of the tree.** A
person using the app should not find a verb missing because it was
added on the other side, so a verb landing here lands in
`asterism-ui`'s `commands` too, in the same change.

The tree does not meet it today, and the shortfall is one
direction: every command has a route, and 34 routed handlers have
no command. Nine of those are the same job under another name
(`declare_asset_provenance` is `asset_declare_provenance`; the four
`threads_about_*` reads are one `list_forge_threads_about`). Nine
are what a person never invokes — the process's own controls
(`health`, `shutdown_process`), byte-serving routes the app reaches
through Tauri's asset protocol instead (`get_asset_file`,
`put_thumb`), and diagnostics a socket client reads. **The
remaining sixteen are the debt**: series-strategy CRUD, `rename_tag`
/ `delete_tag` / `merge_tags`, `rebuild_index`,
`rescan_duplicates`, `organize_by_location`, `remeasure_dims`,
`list_observations`, `list_streams`, `declare_asset_source_type`,
`fetch_visual_model` and `get_setting`. They are unfinished work,
not sanctioned differences, and the count above is the way to see
whether that list is shrinking.

A count taken the other way — commands that have a route — is 128
of 137 before the forge's commands and cannot answer this question,
because the direction that goes short is this one.

Two differences in *shape* are by design. Attribution: these
handlers build a context from command fields with
[`asserted`](crate::attribution::asserted), while a Tauri command
uses `AttributionContext::owner_surface()`, because the desktop's IPC
*is* the owner's surface rather than a caller making a claim. And the
id: a route carries it in the path and overwrites the body's copy,
while a command takes it as its own argument and leaves the field on
the command struct unread.

**MCP is curated and owes nothing.** `crate::mcp` is a use-case
vocabulary an agent reads through `tools/list`, not a projection of
every route; its own module doc has the reasoning. A verb landing
here does *not* land there, and adding one is a decision about what
an agent should be offered.

Nothing checks any of this. It is a rule a person applies while
adding a route, which is why it is written here rather than only in
`README.md`, and why "the forge already does it this way" is not
evidence that a gap is deliberate. The sixteen above are what that
reasoning costs when nobody counts.

## Functions

- `record_webview_diag` — Re-emits one webview-origin diagnostic as a `tracing` event, which
- `router` — Builds the router; the caller binds a listener and calls

