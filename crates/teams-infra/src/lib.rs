//! # teams-infra — adapters for the teams plane
//!
//! The SQLite layer (#89, second slice of #83): the teams-owned
//! database, its migration series, the state tables over the
//! `teams-core` domain types, and the per-team append-only ledger.
//! Auth v0 (#91, third slice) adds the instance-local password adapter
//! and the DB-backed session store. The local blob adapter (#93,
//! fourth slice) is the CAS backing behind `teams-core`'s blob port —
//! staging → verify → fsync → rename. The backup command is a
//! follow-up slice.
//!
//! ## Layout
//!
//! - [`paths`] — where the teams database and the blob store live: the
//!   profile conventions of `asterism-infra`, mirrored for the teams
//!   plane (own env pair, own home root, own marker), so the two
//!   planes never open each other's files.
//! - [`sqlite`] — connection lifecycle (WAL, through the workspace's
//!   `rusqlite-isle` line), the fresh `PRAGMA user_version` migration
//!   series starting at V1, and the repository.
//! - [`auth`] — the #83 §5 auth v0 adapter: argon2id credentials
//!   behind `teams-core`'s auth port, opaque sessions with expiry and
//!   a cleanup path.
//! - [`blob`] — the local CAS adapter behind `teams-core`'s blob port:
//!   `blobs/sha256/<2ch>/<64hex>` plus a staging dir, the
//!   declared-digest write path, and the startup sweep (#83 §3).
//!
//! ## The one write rule
//!
//! Every public state-changing operation on
//! [`SqliteTeamsRepository`](sqlite::repo::SqliteTeamsRepository) opens
//! one transaction, applies the state change **and** appends the
//! corresponding ledger event, and commits or rolls back the two
//! together (#83 §2 audit-log pattern). No public method writes state
//! without appending, and none appends without a state change — the
//! single documented exception is the locator, whose operations are
//! private-space and by design never land in any team's ledger.
//!
//! ## Dependency rule
//!
//! This crate depends on `teams-core` and never on `asterism-infra` /
//! `-contract` / `-server` (#83 §4): those are the local app's
//! plumbing, and the teams plane owns its own.

#![warn(missing_docs)]

pub mod auth;
pub mod blob;
pub mod paths;
pub mod sqlite;
