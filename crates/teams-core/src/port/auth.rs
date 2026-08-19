//! `port::auth` — credential verification behind a provider swap
//! (#83 §1).
//!
//! Credentials live behind this port so the v0 choice (instance-local
//! argon2id password adapter, #83 §5) stays an adapter detail and a
//! later OIDC adapter is a new implementation, not a domain change.
//! Sessions are deliberately **not** here: a session is a short-lived
//! infra artifact that resolves to a `user_id` and the ledger never
//! sees one — `teams-infra` owns that table and its expiry.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::DomainError;

/// Verifies a presented credential and resolves it to a user.
#[async_trait]
pub trait CredentialVerifier: Send + Sync {
    /// Checks `secret` for the account identified by `login`.
    ///
    /// `Ok(Some(user_id))` on success, `Ok(None)` on a wrong or
    /// unknown credential — one arm for both, so the port cannot leak
    /// which half failed (username enumeration); `Err` only when the
    /// provider itself failed to answer.
    async fn verify(&self, login: &str, secret: &str) -> Result<Option<Uuid>, DomainError>;
}
