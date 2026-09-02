# teams-server 0.0.0

# teams-server — the hosted Team plane's server library

Third slice of #83 (#91): auth v0 and the team/membership HTTP API.
The binary (`main.rs`) owns the CLI; this library owns what the
route tests drive in-process:

- [`http`] — the axum `/teams/*` router: the session → user →
  membership gate, the authority checks over `teams-core`'s
  decision functions, and the domain-refusal → 4xx mapping.
- `forge` — the team's hosted forge over HTTP (#151): the local
  surface mirrored under `/teams/{team_id}/forge/*`, and the verbs
  hosting adds. Its routes are merged inside [`http`]'s gate. Not
  linked, and not public: the module is `pub(crate)` because it
  exports no type, only routes that reach a caller through
  [`http::router`].
- [`oidc`] — sign-in through the instance's identity provider
  (#163): the attempts an app starts, the pages a browser walks,
  and the collect that ends in an ordinary session. Its routes are
  registered from [`http`].
- [`rate_limit`] — the one limiter every auth endpoint sits behind
  (#83 §5: from v0, not retrofitted).
- [`state`] — the shared [`TeamsCtx`](state::TeamsCtx) the handlers
  read: every store and setting a request needs, assembled once.
  The struct's own fields are the list.

The blob routes are #93's, the purge routes and the `gc` / `backup`
CLI verbs #95's; the MCP surface is a later slice — the module docs
say which issue owns each.

## Modules

- [`http`](http.md): HTTP transport — the axum `/teams/*` router (#83 §5, the #91
- [`oidc`](oidc.md): Sign-in through the instance's identity provider (#163): the
- [`rate_limit`](rate_limit.md): The auth rate limiter — hand-rolled, per-key, sliding-window.
- [`state`](state.md): Shared server context — what every handler and gate reads.

