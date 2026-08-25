//! # teams-infra — adapters for the teams plane
//!
//! The SQLite layer (#89, second slice of #83): the teams-owned
//! database, its migration series, the state tables over the
//! `teams-core` domain types, and the per-team append-only ledger.
//! Auth v0 (#91, third slice) adds the instance-local password adapter
//! and the DB-backed session store. The local blob adapter (#93,
//! fourth slice) is the CAS backing behind `teams-core`'s blob port —
//! staging → verify → fsync → rename. The #95 slice adds the purge
//! two-step's mark state ([`sqlite`] V3 + the repository's
//! mark/unmark/reclaim), the zero-link sweep ([`gc`]) and the backup
//! ([`backup`]).
//!
//! ## Layout
//!
//! - [`paths`] — where the teams database and the blob store live: the
//!   profile conventions of `asterism-infra`, mirrored for the teams
//!   plane (own env pair, own home root, own marker), so the two
//!   planes never open each other's files.
//! - [`sqlite`] — connection lifecycle (WAL, through the workspace's
//!   `rusqlite-isle` line), the fresh `PRAGMA user_version` migration
//!   series starting at V1, the repository, and — since #150 — the
//!   forge the team hosts ([`sqlite::forge`]).
//! - [`forge`] — the row shapes that forge sits on, which are the local
//!   plane's again because the dependency rule below forbids sharing
//!   the module they came from.
//! - [`auth`] — the #83 §5 auth v0 adapter: argon2id credentials
//!   behind `teams-core`'s auth port, opaque sessions with expiry and
//!   a cleanup path.
//! - [`blob`] — the local CAS adapter behind `teams-core`'s blob port:
//!   `blobs/sha256/<2ch>/<64hex>` plus a staging dir, the
//!   declared-digest write path, and the startup sweep (#83 §3).
//! - [`gc`] — the zero-link sweep (#83 §3 registry-GC shape): bytes no
//!   team links are deleted, under the guard that keeps the sweep and
//!   a racing upload from interleaving.
//! - [`backup`] — quiesce → `VACUUM INTO` → DB-first/blobs-after
//!   archive (#83 §4).
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
//! **The rule has a second writer, and it is the same rule** (#148
//! decision 17). Every write-port method on
//! [`TeamForge`](sqlite::forge::TeamForge) does the same thing for the
//! forge's rows, through the same append. Its documented exception is
//! minting a forge handle, which is not something somebody did — it
//! happens on the way to a write, and the write records who.
//!
//! ## Dependency rule
//!
//! This crate depends on `teams-core` and on `asterism-core`, and
//! never on `asterism-infra` / `-contract` / `-server` (#83 §4): those
//! are the local app's plumbing, and the teams plane owns its own. The
//! `asterism-core` edge is #148 decision 20's — the team hosts the
//! forge by implementing the ports `asterism-core` declares, so the
//! model and the traits are named here and nothing below them is.

#![warn(missing_docs)]

pub mod auth;
pub mod backup;
pub mod blob;
pub mod forge;
pub mod gc;
pub mod paths;
pub mod sqlite;
