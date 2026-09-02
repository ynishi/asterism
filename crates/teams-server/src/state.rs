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

use crate::oidc::OidcSignIn;
use crate::rate_limit::RateLimiter;

/// How long a session lives from login (24 hours). Sessions are
/// short-lived by design (#83 §1); a client that outlives this logs in
/// again.
pub const DEFAULT_SESSION_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// How long a device token lives from its mint unless the instance
/// says otherwise: **90 days** (#204, made policy by #163).
///
/// Policy rather than a constant, because a lifetime is a deployment's
/// trade between re-logins and exposure — the same reason a session's
/// is a field here — and because a fixed number is the one answer a
/// security questionnaire refuses: the expected shape is a ceiling an
/// operator sets, which is what `teams-server serve --device-token-days`
/// is. What stays fixed is that each token's window is fixed *at its
/// mint*, never slid forward on use; the adapter's mint says why.
pub const DEFAULT_DEVICE_TOKEN_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// How long a device token may go unpresented before it stops
/// resolving, unless the instance says otherwise: **30 days**. A
/// laptop in a drawer holds a credential nobody is using, and this is
/// what bounds it; NIST SP 800-63B is where an inactivity timeout
/// comes from as an expectation. `--device-token-idle-days 0` turns
/// it off.
pub const DEFAULT_DEVICE_TOKEN_IDLE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The purge grace window's safe default: **7 days**, the
/// delayed-deletion period GitLab ships for the same trash→purge shape
/// (#83 §1 names it as the precedent). Configurable per instance with
/// `teams-server serve --purge-grace-seconds`; tests run a tiny one.
pub const DEFAULT_PURGE_GRACE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Auth rate limit: attempts allowed per key per window (#83 §5 — one
/// limiter, shared by every route that lets somebody present a
/// credential).
pub const AUTH_RATE_LIMIT_MAX: u32 = 10;

/// Auth rate limit window.
pub const AUTH_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// Bundle the HTTP layer shares via axum state.
pub struct TeamsCtx {
    /// State tables + ledger, behind the one-tx write rule (#89).
    pub repo: SqliteTeamsRepository,
    /// Credentials and sessions — auth v0 (#83 §5).
    pub auth: PasswordAuth,
    /// The identity provider this instance signs people in through
    /// (#163), or `None` for an instance that verifies passwords and
    /// nothing else.
    pub oidc: Option<Arc<OidcSignIn>>,
    /// The instance's CAS — bytes only; visibility is the link rows'
    /// question, answered through [`Self::repo`] (#83 §3, #93).
    pub blobs: LocalFileStorageAdapter,
    /// Whether any authenticated user may create teams, or only
    /// admins (#83 §1's closed-registration flag).
    pub registration: RegistrationPolicy,
    /// Session lifetime handed to every login.
    pub session_ttl_ms: i64,
    /// Device-token lifetime handed to every mint
    /// ([`DEFAULT_DEVICE_TOKEN_TTL_MS`] unless the operator says
    /// otherwise).
    pub device_token_ttl_ms: i64,
    /// How long a device token may go unpresented, or `None` for no
    /// bound ([`DEFAULT_DEVICE_TOKEN_IDLE_MS`] unless the operator says
    /// otherwise).
    pub device_token_idle_ms: Option<i64>,
    /// The one limiter every credential-presenting endpoint sits
    /// behind — which of them those are is `http`'s module doc.
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
