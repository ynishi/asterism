//! # teams-infra — adapters for the teams plane (shell)
//!
//! Near-empty by design in this slice: #83 §4 puts the SQLite adapter
//! (membership / links / locators + ledger append, one tx), the local
//! blob adapter (staging → verify → fsync → rename), the password auth
//! adapter and the backup command here, and those are the follow-up
//! slices. What exists now is the crate boundary itself, so the
//! dependency edges — this crate depends on `teams-core` and never on
//! `asterism-infra` — are fixed before any adapter code exists to
//! blur them.

#![warn(missing_docs)]

// Deliberately no items yet — see the crate-level doc.
