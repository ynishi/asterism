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
//!   [`InstanceAdmin`](domain::identity::InstanceAdmin) /
//!   [`ActorStamp`](domain::identity::ActorStamp), the last-owner rule,
//!   and the #83 §1 authority table as decision functions.
//! - `domain::ledger` — the append-only
//!   [`LedgerEvent`](domain::ledger::LedgerEvent) envelope and the v0
//!   kind registry. The payload is opaque to the substrate.
//! - `domain::store` — [`TeamBlobLink`](domain::store::TeamBlobLink) /
//!   [`Locator`](domain::store::Locator) and the declared-digest
//!   verification rule (accept or reject the whole op; no third
//!   outcome).
//! - `port` — the traits `teams-infra` implements: blob storage and
//!   credential verification.
//! - `error` — [`DomainError`], the crate's `thiserror` enum.
//!
//! ## Dependency rule
//!
//! What this crate takes from the local app is vocabulary — the
//! `sha256:`-prefixed digest notation and its parser from
//! `asterism-core`, reused as-is so the teams plane and the local app
//! spell a byte fingerprint one way.
//!
//! Which `asterism-*` edges may be declared at all is stated once,
//! beside the dependency itself in `Cargo.toml` (#83 §4): the
//! never-list, what is deliberately not on it, and which direction the
//! licence boundary guards. What the rule comes to *here* is that the
//! types below are spelled in the teams plane's own words — no
//! invariant in this crate is stated in a shape the desktop app owns.

#![warn(missing_docs)]

pub mod domain;
pub mod error;
pub mod port;

pub use error::DomainError;
