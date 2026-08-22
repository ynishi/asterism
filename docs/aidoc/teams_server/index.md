# teams-server 0.0.0

# teams-server — the hosted Team plane's server library

Third slice of #83 (#91): auth v0 and the team/membership HTTP API.
The binary (`main.rs`) owns the CLI; this library owns what the
route tests drive in-process:

- [`http`] — the axum `/teams/*` router: the session → user →
  membership gate, the authority checks over `teams-core`'s
  decision functions, and the domain-refusal → 4xx mapping.
- [`rate_limit`] — the one limiter every auth endpoint sits behind
  (#83 §5: from v0, not retrofitted).
- [`state`] — the shared [`TeamsCtx`](state::TeamsCtx) the handlers
  read: repository, credential store, registration policy.

The blob routes are #93's, the purge routes and the `gc` / `backup`
CLI verbs #95's; the MCP surface is a later slice — the module docs
say which issue owns each.

## Modules

- [`http`](http.md): HTTP transport — the axum `/teams/*` router (#83 §5, the #91
- [`rate_limit`](rate_limit.md): The auth rate limiter — hand-rolled, per-key, sliding-window.
- [`state`](state.md): Shared server context — what every handler and gate reads.

