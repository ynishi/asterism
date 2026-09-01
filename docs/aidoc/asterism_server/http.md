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

What that leaves on each side is a set of sanctioned differences,
and what makes one sanctioned is part of the rule. A route may
stand without a command when it is the same job under another name,
or when it is one a person never invokes through the app: the
process's own controls, byte-serving routes the app reaches through
Tauri's asset protocol instead, diagnostics a socket client reads,
and a single-key read the desktop already answers with a wider one.
A command may stand without a route on the same first ground, when
it stages a desktop fact a socket client does not have, when it is a
batch second form over a single-item route, or when it talks to a
team rather than to this process.

Which names are on each side, and the reason each one is there,
lives in `tests/transport_parity.rs`, which fails when either list
stops matching the tree. No size of either is stated — not there and
not here — because a count belongs in the file holding the list or
nowhere, and this passage is what happens when it goes anywhere
else. It carried one until the test existed, and that count went
stale three times: twice while #136's debt was being paid, and once
when #169 added a verb to both surfaces and left the total behind.

The team verbs are where the obligation above is deliberately
one-sided, and #153 added them. They are not verbs against this
server — they are a member's client talking to somebody else's,
carrying that server's session — so a route here would be this
process proxying a connection it does not hold and cannot
authenticate. The obligation is about a person not losing a verb
by being on the other surface, and a socket client that wants a team
has the same client library the desktop has. Nothing forbids the
edge — `asterism-teams-client` is an `asterism-*` crate and #83 §4
permits it here exactly as it permits it in `asterism-ui` — so if a
headless deployment ever needs to publish a line, this is a
decision to revisit rather than a wall.

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

`tests/transport_parity.rs` checks the first half of this and not
the second: it fails on a name that gained a route or a command
without the other, or on an exception list that no longer matches
the tree. Whether a difference *deserves* to be on one of those
lists is still a person's judgement, which is why the reasoning
above is written here rather than only in `README.md`, and why "the
forge already does it this way" is not evidence that a gap is
deliberate. The sixteen-verb debt #136 counted is what that
reasoning cost while nobody counted; the check is the answer to the
question #136 left open and pointed at #124.

Both directions of that check run on the branch that owes them:
the test reads a file from each crate and sits in this one, and
the cross-member reader list `changed-packages` selects from pairs
the desktop's command module with this crate, so a branch touching
only that file selects this crate too. The test's own doc says why
it sits on this side.

## Functions

- `record_webview_diag` — Re-emits one webview-origin diagnostic as a `tracing` event, which
- `router` — Builds the router; the caller binds a listener and calls

