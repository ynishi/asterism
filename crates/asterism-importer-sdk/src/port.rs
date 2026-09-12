//! The inbound port's shared vocabulary: what a failure is, and what a
//! resumption point holds.
//!
//! [`SourceError`] is what the scanner traits are written against.
//! [`SyncState`] is not yet in any signature: the type and its
//! serialised form are settled here first, ahead of the transport that
//! will carry it. Both are deliberately the part of the port that does
//! not depend on how an adapter is run — whether Asterism starts it and
//! reads it, or it runs itself and pushes. A rate limit is a rate limit
//! either way, and a cursor holds the same thing either way.
//!
//! ## Why a classification at all
//!
//! [`SourceError`] replaced a three-variant enum whose variants said
//! where a failure happened rather than what to do about it. Nothing
//! downstream could act on that, so the decision moved into the
//! scanners: `SqliteScanner` returned from its own reader thread after
//! sending a source-level failure, ending the stream, and `FsScanner`
//! let its watch task fall out of its loop. Each author decided, in
//! their own way, that this failure meant stop — and an adapter that
//! decided otherwise would keep a dead scan alive with nothing able to
//! tell. One adapter deciding that is a preference. Thirty or forty
//! deciding it separately is thirty or forty answers to one question,
//! and an upstream change moves every one of them.
//!
//! The five classes here are the ones inbound frameworks converged on:
//! Airbyte's `config_error` / `transient_error` / `system_error`
//! failure types, `RATE_LIMITED` broken out as an action of its own,
//! and Kafka Connect's coarser `RetriableException`. Carrying on past a
//! single bad record is in both of those too — Connect's
//! `errors.tolerance` with a dead-letter queue, Airbyte's `IGNORE`
//! action — but as an operator setting rather than a thing the source
//! says. Here it is a variant, because the scanner is what knows that
//! one file would not open while the directory is fine.

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
    /// **Retried by a caller that has a backoff loop.** The SDK's own
    /// [`run_import`](crate::runner::run_import) does not have one yet
    /// and ends the run instead; what it does not do is step over this
    /// as though one file had failed.
    #[error("source temporarily unavailable: {0}")]
    Transient(String),

    /// The source asked us to slow down.
    ///
    /// **Retried on the same terms as [`Transient`](Self::Transient)**,
    /// and kept apart from it for two reasons. The wait is usually longer and told to us
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

    /// Where the failure happened, when the class names a place.
    ///
    /// Only [`Item`](Self::Item) has one: it is the class that names a
    /// thing inside the source rather than the source itself, which is
    /// what lets a report say what was skipped instead of saying that
    /// something, somewhere, was.
    pub fn locator(&self) -> Option<&str> {
        match self {
            Self::Item { locator, .. } => Some(locator),
            _ => None,
        }
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
/// `offset` is the position reached inside it. Kafka Connect has
/// carried the split from the start, as `sourcePartition` and
/// `sourceOffset`; Airbyte arrived at it, and its protocol still names
/// the single-blob form it left behind `LEGACY`, beside the per-stream
/// one that replaced it. The reason is that a source with several
/// independently-advancing parts cannot be described by one position,
/// and a run that stops halfway through the third of nine folders has
/// to be able to say which folder.
///
/// ## What the core may look at
///
/// `offset` is opaque: it belongs to the adapter that wrote it, and
/// nothing here parses, validates or migrates it. `partition` is opaque
/// too, with one exception — its **identity**. Whatever comes to store
/// these will compare partitions, to know which state a checkpoint
/// replaces and which ones exist; it will never interpret what the
/// string means. Nothing stores them yet, and that reservation is the
/// whole of what the core is allowed to do with the field.
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
        // Against the text, not only the value. `preserve_order` makes
        // `Map` an `IndexMap`, whose `PartialEq` ignores key order — so
        // an implementation that sorted the keys on the way through
        // would satisfy the comparison above and still hand back a
        // different document. The serialised form is what a store would
        // hold and what a later run would compare, so it is what is
        // pinned.
        assert_eq!(
            serde_json::to_string(&back).expect("serialises"),
            wire,
            "byte for byte, key order included"
        );
        assert!(
            wire.contains(r#""b":1,"a""#),
            "and the order is the one the adapter wrote: {wire}"
        );
    }

    /// The whole argument for `partition` being a string, stated as a
    /// test: with `preserve_order` two maps that agree on content but
    /// not on insertion order serialise differently, so a value could
    /// not be compared as text without somebody owning a
    /// canonicalisation of it — and the core is not allowed to read it.
    #[test]
    fn two_maps_that_agree_on_content_do_not_agree_as_text() {
        let one = serde_json::json!({ "account": "me", "album": "2019" });
        let other = serde_json::json!({ "album": "2019", "account": "me" });
        assert_eq!(one, other, "as values they are the same");
        assert_ne!(
            serde_json::to_string(&one).expect("serialises"),
            serde_json::to_string(&other).expect("serialises"),
            "as text they are not, which is why a partition is a string"
        );
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
