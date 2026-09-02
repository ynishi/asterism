# teams-server::rate_limit

The auth rate limiter — hand-rolled, per-key, sliding-window.

## Why hand-rolled

#83 §5 requires one limiter over ALL auth endpoints from v0. The
workspace has no per-key rate-limiting dependency: `tower`'s own
limit layers are global concurrency/rate caps with no notion of a
client key, and the crates that do keyed limiting (`tower_governor`
and friends) would arrive for exactly one middleware.
A sliding-window log over a `Mutex<HashMap>` is ~40 lines, has no
background task, and its failure mode (a mutex) is simpler than a
dependency's upgrade treadmill — so the decision, recorded here, is
to hand-roll until a second consumer wants something richer.

## Key choice

Per client IP when the connection carries one
(`into_make_service_with_connect_info` in `main.rs`); a fixed
`"local"` key otherwise (in-process tests drive the router without
a socket). Keying by IP rather than by login means a spray across
many logins from one address is still one bucket, and a distributed
attacker burns addresses, not accounts.

## Types

- `RateLimiter` — A per-key sliding-window counter: at most `max` accepted hits per

