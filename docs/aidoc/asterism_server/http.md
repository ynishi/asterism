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
direction: 26 routed handlers have no command of the same name.
Ten of those are the same job under another name
(`declare_asset_provenance` is `asset_declare_provenance`;
`declare_asset_source_type` is `asset_declare_source_type`; the
four `threads_about_*` reads are one `list_forge_threads_about`).
Nine are what a person never invokes — the process's own controls
(`health`, `shutdown_process`), byte-serving routes the app reaches
through Tauri's asset protocol instead (`get_asset_file`,
`put_thumb`), and diagnostics a socket client reads. One is a
shape the desktop already has: `get_setting` stays without a
command (#136's decision) because `list_settings` returns every
registry key fully resolved and `set_setting` / `reset_setting`
return the resolved row, so a single-key IPC read would be a
second way to ask a question the app already has answered. **The
remaining six are the debt**: `rebuild_index`, `rescan_duplicates`,
`organize_by_location`, `remeasure_dims` — the maintenance verbs
#136 deferred until whether they need a screen first is answered —
and `train_tag_head` / `pull_tag_head`, which landed with #132
after the last count was taken. They are unfinished work, not
sanctioned differences, and the count above is the way to see
whether that list is shrinking.

A count taken the other way — commands that have a route of the
same name — is 161 of 172 and cannot answer this question, because
the direction that goes short is the other one. The eleven without
one are the seven command-side names of the twins above (four
`threads_about_*` routes collapse into one command, so ten route
names pair with seven command names); `paste_image_import` and
`rehome_dropped_path`, which stage clipboard and drag-drop
material — desktop facts a socket client does not have;
`get_asset_thumbs`, a batch second command over the single-thumb
route; and `active_profile`, a desktop-chrome read of which local
data profile this process opened, with no route today.

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
evidence that a gap is deliberate. The six above are what that
reasoning costs when nobody counts.

## Functions

- `record_webview_diag` — Re-emits one webview-origin diagnostic as a `tracing` event, which
- `router` — Builds the router; the caller binds a listener and calls

