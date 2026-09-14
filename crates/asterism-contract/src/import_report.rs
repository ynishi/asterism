//! What an importer run reports back to whatever started it.
//!
//! A machine-readable summary, written to a path the caller names, and
//! the reason it exists rather than being scraped off stderr: the
//! progress lines are for a person and are free to change, while this
//! is a contract between two binaries. A supervisor parsing "done —
//! ok=3 err=0" would break the day somebody improved the wording, and
//! would break silently, reporting zero.
//!
//! It carries the **class** of whatever ended the run and not only its
//! message. The class is what a caller acts on — three slices of this
//! port went into drawing it — and it is the part a scheduler needs in
//! order to know whether running again is worth anything.

use serde::{Deserialize, Serialize};

/// One run's outcome, as the importer saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    /// Records the server accepted.
    pub imported: u64,
    /// Records that did not land.
    pub failed: u64,
    /// What ended the run, if anything did.
    pub ended_by: Option<ReportedFailure>,
}

/// The failure that ended a run, flattened for the wire.
///
/// Not `SourceError` itself: that type is the importer SDK's and the
/// server does not depend on it, which is the point of this crate
/// sitting between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedFailure {
    /// `config`, `transient`, `rate_limited`, `source`, or `item`.
    pub class: String,
    /// What it said, for a person.
    pub message: String,
    /// How long the source asked us to wait, when it said — the field
    /// `rate_limited` exists to carry.
    pub retry_after_secs: Option<u64>,
}
