//! Shared server context — what every handler and gate reads.
//!
//! Assembled once (by `main.rs` over the profile database, by the
//! route tests over an in-memory one) and passed as axum state. Both
//! halves — repository and credential store — wrap clones of the same
//! `AsyncIsle`, so state, ledger, credentials and sessions live in one
//! SQLite file behind one writer.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use teams_core::domain::identity::RegistrationPolicy;
use teams_infra::auth::password::PasswordAuth;
use teams_infra::blob::LocalFileStorageAdapter;
use teams_infra::gc::GcGuard;
use teams_infra::sqlite::SqliteTeamsRepository;
use teams_infra::sqlite::projection::SqliteProjectionStore;

use crate::rate_limit::RateLimiter;

/// How long a session lives from login (24 hours). Sessions are
/// short-lived by design (#83 §1); a client that outlives this logs in
/// again.
pub const DEFAULT_SESSION_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// The purge grace window's safe default: **7 days**, the
/// delayed-deletion period GitLab ships for the same trash→purge shape
/// (#83 §1 names it as the precedent). Configurable per instance with
/// `teams-server serve --purge-grace-seconds`; tests run a tiny one.
pub const DEFAULT_PURGE_GRACE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

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
    /// Whether any authenticated user may create teams, or only
    /// admins (#83 §1's closed-registration flag).
    pub registration: RegistrationPolicy,
    /// Session lifetime handed to every login.
    pub session_ttl_ms: i64,
    /// The one limiter every auth endpoint sits behind.
    pub auth_limiter: RateLimiter,
    /// How long a purge mark must sit before reclaim may remove it
    /// (#95). [`DEFAULT_PURGE_GRACE_MS`] unless the operator says
    /// otherwise; tests use a tiny one.
    pub purge_grace_ms: i64,
    /// The guard between the write paths that put bytes in the CAS and
    /// the zero-link sweep: each holds it shared across rename → link
    /// commit, the sweep holds it exclusive (`teams_infra::gc`). Two
    /// paths hold it — the blob upload (#93) and the forge's content
    /// verb (#151) — because both end in that same pair of steps.
    pub gc_guard: Arc<GcGuard>,
    /// Captured descriptions, keyed by entry (#148 decision 12).
    ///
    /// Its own field rather than a method on [`Self::repo`], for the
    /// reason it is its own module in `teams-infra`: the projection
    /// sits outside the forge and outside the state tables the ledger
    /// is the receipt for, and a handler reaching it through the
    /// repository would make it look like one of those.
    pub projections: SqliteProjectionStore,
}

/// Milliseconds since the Unix epoch, now — the single clock every
/// handler stamps `occurred_at` and session expiry from.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock predates the Unix epoch")
        .as_millis() as i64
}
