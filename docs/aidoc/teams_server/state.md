# teams-server::state

Shared server context — what every handler and gate reads.

Assembled once (by `main.rs` over the profile database, by the
route tests over an in-memory one) and passed as axum state. Both
halves — repository and credential store — wrap clones of the same
`AsyncIsle`, so state, ledger, credentials and sessions live in one
SQLite file behind one writer.

## Functions

- `now_ms` — Milliseconds since the Unix epoch, now — the single clock every

## Types

- `TeamsCtx` — Bundle the HTTP layer shares via axum state.

## Constants

- `AUTH_RATE_LIMIT_MAX` — Auth rate limit: attempts allowed per key per window (#83 §5 — one
- `AUTH_RATE_LIMIT_WINDOW` — Auth rate limit window.
- `DEFAULT_DEVICE_TOKEN_IDLE_MS` — How long a device token may go unpresented before it stops
- `DEFAULT_DEVICE_TOKEN_TTL_MS` — How long a device token lives from its mint unless the instance
- `DEFAULT_PURGE_GRACE_MS` — The purge grace window's safe default: **7 days**, the
- `DEFAULT_SESSION_TTL_MS` — How long a session lives from login (24 hours). Sessions are

