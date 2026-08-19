//! # teams-infra — adapters for the teams plane
//!
//! This slice (#89, second slice of #83) is the SQLite layer: the
//! teams-owned database, its migration series, the state tables over
//! the `teams-core` domain types, and the per-team append-only ledger.
//! The local blob adapter (staging → verify → fsync → rename), the
//! password auth adapter and the backup command are the follow-up
//! slices.
//!
//! ## Layout
//!
//! - [`paths`] — where the teams database lives: the profile
//!   conventions of `asterism-infra`, mirrored for the teams plane
//!   (own env pair, own home root, own marker), so the two planes
//!   never open each other's files.
//! - [`sqlite`] — connection lifecycle (WAL, through the workspace's
//!   `rusqlite-isle` line), the fresh `PRAGMA user_version` migration
//!   series starting at V1, and the repository.
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

pub mod paths;
pub mod sqlite;
