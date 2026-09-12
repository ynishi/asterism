//! The inbound port's shared vocabulary: what a failure is, and what a
//! resumption point holds.
//!
//! These two types are what the scanner traits are written against, and
//! they are deliberately the part of the port that does not depend on
//! how an adapter is run — whether Asterism starts it and reads it, or
//! it runs itself and pushes. A rate limit is a rate limit either way,
//! and a cursor holds the same thing either way.
//!
//! ## Why a classification at all
//!
//! [`SourceError`] replaced a three-variant enum whose variants said
//! where the failure happened rather than what to do about it, and the
//! runner treated all three alike: record the message, carry on to the
//! next item. A source that had gone away was handled as one unreadable
//! file, so an import against a moved directory or an expired
//! credential spun through its whole stream reporting errors instead of
//! stopping and saying so. With seven importers that is a bad hour;
//! with thirty it is thirty separate answers to the same question.
//!
//! The five classes here are the ones inbound frameworks converged on —
//! Airbyte's `config_error` / `transient_error` / `system_error` with
//! `RATE_LIMITED` broken out as its own action, and Kafka Connect's
//! coarser `RetriableException` — plus the per-item class this port
//! already had and which none of those needs, because they stream rows
//! rather than read files.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a caller should do about a [`SourceError`].
///
/// Returned by [`SourceError::disposition`] so a caller branches on a
/// decision rather than on which variant it happens to be holding — the
/// mapping from class to action belongs to the port, not to each of its
/// callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Take the next item. Something specific failed and the rest of
    /// the scan is unaffected.
    KeepScanning,
    /// The same request may work later. `after` is how long to wait
    /// when the source said so, and `None` when it did not — a caller
    /// with a backoff policy of its own uses that instead.
    Retry { after: Option<Duration> },
    /// Nothing further in this run will succeed. Stop and report.
    EndRun,
}

/// Why a source could not be read.
///
/// Each variant names an action rather than a location, and
/// [`disposition`](Self::disposition) is where that action is written
/// down. A caller that matches on the variants directly is free to, but
/// it is then deciding the policy a second time.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// The configuration is wrong: a path that is not there, a
    /// credential the source rejected, a query naming a column that
    /// does not exist.
    ///
    /// **Ends the run**, and is the one class where that is not a
    /// disappointment: retrying cannot fix it and the message is for
    /// the person who wrote the configuration. Separated from
    /// [`Source`](Self::Source) because a run that failed on the
    /// operator's settings and a run that failed on the source's own
    /// trouble ask different things of whoever reads the report.
    #[error("configuration rejected: {0}")]
    Config(String),

    /// The source could not be reached, and the same call may well
    /// work in a moment: a refused connection, a timeout, a 503.
    ///
    /// **Retried**, with whatever backoff the caller has.
    #[error("source temporarily unavailable: {0}")]
    Transient(String),

    /// The source asked us to slow down.
    ///
    /// **Retried**, and kept apart from [`Transient`](Self::Transient)
    /// for two reasons. The wait is usually longer and told to us
    /// rather than guessed, so a caller that treats it as an ordinary
    /// transient either waits far too little and is refused again or
    /// far too much and stalls. And it is the one failure where a
    /// caller can say *when* the source is expected back, which is the
    /// difference between a progress line that reads "failed" and one
    /// that reads "waiting until 14:32".
    ///
    /// `retry_after` is what the source stated (`Retry-After`, a
    /// reset timestamp in a header, a documented window). `None` means
    /// it refused us without saying for how long.
    #[error("rate limited{}: {message}", match retry_after {
        Some(d) => format!(", retry after {}s", d.as_secs()),
        None => String::new(),
    })]
    RateLimited {
        /// How long the source said to wait, when it said.
        retry_after: Option<Duration>,
        /// What the source told us.
        message: String,
    },

    /// The source failed in a way this run cannot get past, and it is
    /// not the configuration's fault: a corrupt database, a response
    /// that does not parse, a bug here.
    ///
    /// **Ends the run.**
    #[error("source failed: {0}")]
    Source(String),

    /// One item could not be read. The scan is unaffected.
    ///
    /// **Keeps scanning**, which is what makes this class worth having
    /// separately: one unreadable file in a directory of ten thousand
    /// is a line in the report, not the end of an import. It is the
    /// only class the previous enum got right, and it carries the
    /// locator so the report can name what was skipped.
    #[error("item unreadable at {locator}: {message}")]
    Item {
        /// Where the item that failed lives, in the source's own terms.
        locator: String,
        /// Why it could not be read.
        message: String,
    },
}

impl SourceError {
    /// What a caller should do about this failure.
    pub fn disposition(&self) -> Disposition {
        match self {
            Self::Item { .. } => Disposition::KeepScanning,
            Self::Transient(_) => Disposition::Retry { after: None },
            Self::RateLimited { retry_after, .. } => Disposition::Retry {
                after: *retry_after,
            },
            Self::Config(_) | Self::Source(_) => Disposition::EndRun,
        }
    }

    /// Whether the scan can carry on past this.
    ///
    /// A convenience over [`disposition`](Self::disposition) for the
    /// common loop, which only needs to know whether to take the next
    /// item.
    pub fn is_item_local(&self) -> bool {
        matches!(self.disposition(), Disposition::KeepScanning)
    }

    /// Builds an [`Item`](Self::Item) failure for `locator`.
    pub fn item(locator: impl Into<String>, message: impl std::fmt::Display) -> Self {
        Self::Item {
            locator: locator.into(),
            message: message.to_string(),
        }
    }

    /// Builds a [`RateLimited`](Self::RateLimited) failure with no
    /// stated wait.
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::RateLimited {
            retry_after: None,
            message: message.into(),
        }
    }
}

/// Where a scan left off, so the next one can start there.
///
/// Two levels, not one. `partition` names a unit that can be resumed on
/// its own — a folder, an album, a table, one account's stream — and
/// `offset` is the position reached inside it. Every inbound framework
/// that started with a single opaque blob has since split it: Kafka
/// Connect carries `sourcePartition` and `sourceOffset` as two maps,
/// and Airbyte's protocol still names its single-blob form `LEGACY`
/// beside the per-stream one that replaced it. The reason is that a
/// source with several independently-advancing parts cannot be
/// described by one position, and a run that stops halfway through the
/// third of nine folders has to be able to say which folder.
///
/// ## What the core may look at
///
/// `offset` is opaque: it belongs to the adapter that wrote it, and
/// nothing here parses, validates or migrates it. `partition` is opaque
/// too, with one exception — its **identity**. The core compares
/// partitions to know which state a checkpoint replaces, and stores
/// them to know which ones exist. It never interprets what the string
/// means.
///
/// That is why `partition` is a string and not a JSON value, though
/// Connect's equivalent is a map. This workspace builds `serde_json`
/// with `preserve_order`, so a map's serialised form follows insertion
/// order; two partitions with identical content written in a different
/// order would not compare equal as text, and making them compare
/// equal would mean the core owning a canonicalisation of a value it is
/// not supposed to read. An adapter whose partition is compound encodes
/// it — `"account=me/album=2019"` — and owns that encoding, which is
/// the same bargain as `offset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// The resumable unit this state is for. Opaque, except that the
    /// core compares it.
    pub partition: String,
    /// The position reached within that unit. Entirely the adapter's;
    /// carried and handed back untouched.
    pub offset: Value,
}

impl SyncState {
    /// A state for `partition` at `offset`.
    pub fn new(partition: impl Into<String>, offset: Value) -> Self {
        Self {
            partition: partition.into(),
            offset,
        }
    }

    /// The starting state for a partition nothing has recorded yet.
    ///
    /// `offset` is JSON `null` rather than an absent field, so a
    /// reader tells "we have been here and there is nothing to resume
    /// from" apart from "we have never been here", which is the
    /// absence of the whole record.
    pub fn begin(partition: impl Into<String>) -> Self {
        Self::new(partition, Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classification exists to be acted on, so each class is
    /// pinned to the action it names.
    #[test]
    fn every_class_states_what_a_caller_does() {
        assert_eq!(
            SourceError::item("/a.png", "permission denied").disposition(),
            Disposition::KeepScanning
        );
        assert_eq!(
            SourceError::Transient("connection refused".into()).disposition(),
            Disposition::Retry { after: None }
        );
        assert_eq!(
            SourceError::rate_limited("429").disposition(),
            Disposition::Retry { after: None }
        );
        assert_eq!(
            SourceError::Config("no such directory".into()).disposition(),
            Disposition::EndRun
        );
        assert_eq!(
            SourceError::Source("malformed response".into()).disposition(),
            Disposition::EndRun
        );
    }

    /// A stated wait reaches the caller as a duration rather than as
    /// prose it would have to parse out of the message.
    #[test]
    fn a_stated_wait_survives_as_a_duration() {
        let err = SourceError::RateLimited {
            retry_after: Some(Duration::from_secs(90)),
            message: "429 Too Many Requests".into(),
        };
        assert_eq!(
            err.disposition(),
            Disposition::Retry {
                after: Some(Duration::from_secs(90))
            }
        );
        assert!(
            err.to_string().contains("retry after 90s"),
            "and it is legible in the report too: {err}"
        );
    }

    /// The one distinction the old enum could not make: a configuration
    /// the source refused ends the run, an unreadable file does not,
    /// and neither is told from the other by looking at the message.
    #[test]
    fn a_refused_configuration_is_not_an_unreadable_item() {
        let config = SourceError::Config("token rejected".into());
        let item = SourceError::item("/photos/3.png", "token rejected");
        assert!(!config.is_item_local());
        assert!(item.is_item_local());
        assert_ne!(config.disposition(), item.disposition());
    }

    /// The offset belongs to whoever wrote it. It comes back byte for
    /// byte, including a shape the core has no schema for.
    #[test]
    fn an_offset_the_core_cannot_read_round_trips_unchanged() {
        let state = SyncState::new(
            "account=me/album=2019",
            serde_json::json!({
                "page_token": "CAEQAQ",
                "seen": [3, 1, 2],
                "nested": { "b": 1, "a": { "deep": null } }
            }),
        );
        let wire = serde_json::to_string(&state).expect("serialises");
        let back: SyncState = serde_json::from_str(&wire).expect("deserialises");
        assert_eq!(back, state);
        assert_eq!(back.offset, state.offset, "the payload is not touched");
    }

    /// "Nothing to resume from" and "never been here" are different
    /// answers, and the first one is a record that exists.
    #[test]
    fn a_beginning_is_a_state_rather_than_an_absence() {
        let begin = SyncState::begin("folder=/inbox");
        assert_eq!(begin.offset, Value::Null);
        let wire = serde_json::to_value(&begin).expect("serialises");
        assert!(
            wire.get("offset").is_some(),
            "the field is present and null, not missing: {wire}"
        );
    }
}
