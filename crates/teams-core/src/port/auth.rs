//! `port::auth` — credential verification behind a provider swap
//! (#83 §1).
//!
//! Credentials live behind this port so the v0 choice (instance-local
//! argon2id password adapter, #83 §5) stays an adapter detail. It was
//! cut expecting an OIDC adapter to be a second implementation; when
//! that sign-in arrived (#163) it sat beside the port instead, because
//! what a provider hands back is not a login and a secret —
//! `teams-infra`'s auth module says where the two paths meet. Sessions
//! are deliberately **not** here: a session is a short-lived infra
//! artifact that resolves to a `user_id` and the ledger never sees one
//! — `teams-infra` owns that table and its expiry.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::DomainError;

/// Verifies a presented credential and resolves it to a user.
#[async_trait]
pub trait CredentialVerifier: Send + Sync {
    /// Checks `secret` for the account identified by `login`.
    ///
    /// `Ok(Some(user_id))` on success, `Ok(None)` on everything else a
    /// caller must not be able to tell apart — a wrong credential, an
    /// unknown login, an account that holds no password, an account
    /// the instance has locked — one arm for all of them, so the port
    /// cannot leak which it was (username enumeration, and now which
    /// accounts sign in elsewhere or are locked); `Err` only when the
    /// provider itself failed to answer.
    async fn verify(&self, login: &str, secret: &str) -> Result<Option<Uuid>, DomainError>;
}
