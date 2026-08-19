//! Shared server context — what every handler and gate reads.
//!
//! Assembled once (by `main.rs` over the profile database, by the
//! route tests over an in-memory one) and passed as axum state. Both
//! halves — repository and credential store — wrap clones of the same
//! `AsyncIsle`, so state, ledger, credentials and sessions live in one
//! SQLite file behind one writer.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::sqlite::SqliteTeamsRepository;

use crate::rate_limit::RateLimiter;

/// How long a session lives from login (24 hours). Sessions are
/// short-lived by design (#83 §1); a client that outlives this logs in
/// again.
pub const DEFAULT_SESSION_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// Auth rate limit: attempts allowed per key per window (#83 §5 — one
/// limiter over ALL auth endpoints from v0).
pub const AUTH_RATE_LIMIT_MAX: u32 = 10;

/// Auth rate limit window.
pub const AUTH_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// Bundle the HTTP layer shares via axum state.
pub struct TeamsCtx {
    /// State tables + ledger, behind the one-tx write rule (#89).
    pub repo: SqliteTeamsRepository,
    /// Credentials and sessions — auth v0 (#83 §5).
    pub auth: PasswordAuth,
    /// The instance's CAS — bytes only; visibility is the link rows'
    /// question, answered through [`Self::repo`] (#83 §3, #93).
    pub blobs: LocalFileStorageAdapter,
    /// Whether any authenticated user may create teams, or only the
    /// operator (#83 §1's closed-registration flag).
    pub registration: RegistrationPolicy,
    /// Session lifetime handed to every login.
    pub session_ttl_ms: i64,
    /// The one limiter every auth endpoint sits behind.
    pub auth_limiter: RateLimiter,
}

/// Milliseconds since the Unix epoch, now — the single clock every
/// handler stamps `occurred_at` and session expiry from.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock predates the Unix epoch")
        .as_millis() as i64
}
