//! Auth adapters for the teams plane (#83 §5).
//!
//! v0 is the instance-local password adapter ([`password`]): argon2id
//! hashing behind `teams-core`'s
//! [`CredentialVerifier`](teams_core::port::auth::CredentialVerifier)
//! port, plus the DB-backed opaque session store — the session lives
//! here and not in the domain on purpose (#83 §1: a session is a
//! short-lived infra artifact that resolves to a `user_id`, and the
//! ledger never sees one). OIDC is a later adapter behind the same
//! port; 2FA is deferred by design.

pub mod password;
