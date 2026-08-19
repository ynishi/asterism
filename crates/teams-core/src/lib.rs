//! # teams-core — domain layer of the Asterism teams plane
//!
//! First slice of #83: the types and invariants of the hosted Team
//! plane, with no IO anywhere in the crate. What a Team *is* — who may
//! act on it, what the ledger records, what the store promises — is
//! decided here; SQLite, the filesystem, HTTP and auth adapters arrive
//! in the follow-up slices (`teams-infra` / `teams-server`).
//!
//! ## Layout
//!
//! - `domain::identity` — [`User`](domain::identity::User) /
//!   [`Membership`](domain::identity::Membership) /
//!   [`InstanceOperator`](domain::identity::InstanceOperator) /
//!   [`ActorStamp`](domain::identity::ActorStamp), the last-owner rule,
//!   and the #83 §1 authority table as decision functions.
//! - `domain::ledger` — the append-only
//!   [`LedgerEvent`](domain::ledger::LedgerEvent) envelope and the v0
//!   kind registry. The payload is opaque to the substrate.
//! - `domain::store` — [`TeamBlobLink`](domain::store::TeamBlobLink) /
//!   [`Locator`](domain::store::Locator) and the declared-digest
//!   verification rule (accept or reject the whole op; no third
//!   outcome).
//! - `port` — the traits `teams-infra` implements: blob storage,
//!   credential verification, and the share port reserved for #63.
//! - `error` — [`DomainError`], the crate's `thiserror` enum.
//!
//! ## Dependency rule
//!
//! This crate depends on `asterism-core` and on no other asterism-*
//! crate (#83 §4). What it takes from there is vocabulary — the
//! `sha256:`-prefixed digest notation and its parser — reused as-is so
//! the teams plane and the local app spell a byte fingerprint one way.

#![warn(missing_docs)]

pub mod domain;
pub mod error;
pub mod port;

pub use error::DomainError;
