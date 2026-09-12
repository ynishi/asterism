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
//! ## Why the classes are what they are
//!
//! [`SourceError`] replaced a three-variant enum that named where a
//! failure happened — source unavailable, item read failed, other.
//! Nothing downstream could act on that, so a run against a rejected
//! credential and a run that skipped one unreadable file produced the
//! same report: a count, and a message only a person could read.
//!
//! The classes here are cuts a caller acts on differently. A
//! configuration nobody has changed will be refused again, so the run
//! stops and the message is for whoever wrote it. A source that was
//! briefly unreachable will not, so the same run repeated is worth
//! something — and a rate limit is that case with the wait stated
//! rather than guessed, which is what lets a report say when the source
//! is expected back instead of only that it failed. A record that could
//! not be read costs that record, and a run missing one file out of ten
//! thousand is not a failed import.
//!
//! What the classification does not do is tell a scanner how to behave.
//! That is [`SourceError`]'s own section below, and it is one rule.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a [`SourceError`] means for the run it happened in.
///
/// Returned by [`SourceError::disposition`] so a caller branches on a
/// decision rather than on which variant it happens to be holding — the
/// mapping from class to meaning belongs to the port, not to each of
/// its callers.
///
/// ## What this is not about
///
/// It says nothing about whether more items follow. That is the
/// scanner's own fact, and the stream is where it is answered: a
/// scanner with more to give yields more, one without ends. The first
/// shape of this enum did claim it — an item failure was documented as
/// "keeps scanning" — and `SqliteScanner` refuted it on the day it was
/// written: a row it cannot read ends its stream, for the reason
/// written beside the `break` that does it. Both scanners were right
/// about their own sources; the port was wrong to hold an opinion.
///
/// So a caller reads the stream to its end whatever it is handed, and
/// uses this to decide what the run was worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// One record was lost. The run can still be a success that is
    /// missing something, and the report names what.
    RecordLost,
    /// The run failed, and the same run repeated may not: a source
    /// briefly unreachable, or one that asked us to wait. `after` is
    /// how long it said to wait, and `None` when it did not say.
    FailedRetryable { after: Option<Duration> },
    /// The run failed, and repeating it changes nothing until somebody
    /// does: a rejected configuration, a source that broke.
    Failed,
}

/// Why a source could not be read.
///
/// Each variant names what the failure cost rather than where it
/// happened, and [`disposition`](Self::disposition) is where that is
/// written down. A caller that matches on the variants directly is free
/// to, but it is then deciding the policy a second time.
///
/// ## What a scanner owes
///
/// One thing, and it is about the stream rather than about any class: a
/// scanner with nothing more to give **ends its stream**. A failure it
/// cannot continue past is followed by no further items; a failure it
/// can is followed by the next one. It does not have to say which kind
/// it was — ending is how it says so, and that is why no class here
/// claims to know.
///
/// What it must not do is emit a failure it cannot continue past and
/// then keep emitting it. That is not a mismatched class, it is a
/// scanner that never ends, and no caller can rescue it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The configuration is wrong: a path that is not there, a
    /// credential the source rejected, a query naming a column that
    /// does not exist.
    ///
    /// **The run failed**, and is the one class where that is not a
    /// disappointment: repeating it cannot fix it and the message is
    /// for the person who wrote the configuration. Separated from
    /// [`Source`](Self::Source) because a run that failed on the
    /// operator's settings and a run that failed on the source's own
    /// trouble ask different things of whoever reads the report.
    #[error("configuration rejected: {0}")]
    Config(String),

    /// The source could not be reached, and the same call may well
    /// work in a moment: a refused connection, a timeout, a 503.
    ///
    /// **The run failed and repeating it may succeed.** Whether
    /// anything does repeat it is the caller's; what the port says is
    /// that this is not a run to file as done.
    #[error("source temporarily unavailable: {0}")]
    Transient(String),

    /// The source asked us to slow down.
    ///
    /// **The run failed and repeating it may succeed**, on the same
    /// terms as [`Transient`](Self::Transient), and kept apart from it
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
    /// **The run failed.**
    #[error("source failed: {0}")]
    Source(String),

    /// One record could not be read.
    ///
    /// **The run can still be a success that is missing something** —
    /// one unreadable file in a directory of ten thousand is a line in
    /// the report, not a failed import — which is what makes this class
    /// worth having separately. Whether the scanner goes on to the next
    /// record is its own business: `FsScanner` does, `SqliteScanner`
    /// cannot and ends instead, and both are reported the same way.
    ///
    /// It carries the locator so the report can name the record, as
    /// precisely as the scanner knew it at the time.
    #[error("item unreadable at {locator}: {message}")]
    Item {
        /// Where the item that failed lives, in the source's own terms.
        locator: String,
        /// Why it could not be read.
        message: String,
    },
}

impl SourceError {
    /// What this failure means for the run it happened in.
    pub fn disposition(&self) -> Disposition {
        match self {
            Self::Item { .. } => Disposition::RecordLost,
            Self::Transient(_) => Disposition::FailedRetryable { after: None },
            Self::RateLimited { retry_after, .. } => Disposition::FailedRetryable {
                after: *retry_after,
            },
            Self::Config(_) | Self::Source(_) => Disposition::Failed,
        }
    }

    /// Whether this cost one record rather than the run.
    ///
    /// A convenience over [`disposition`](Self::disposition) for the
    /// common loop, which only needs to know which of the two it is
    /// holding.
    pub fn is_record_lost(&self) -> bool {
        matches!(self.disposition(), Disposition::RecordLost)
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
    /// pinned to what it says the run was worth.
    #[test]
    fn every_class_states_what_the_run_was_worth() {
        assert_eq!(
            SourceError::item("/a.png", "permission denied").disposition(),
            Disposition::RecordLost
        );
        assert_eq!(
            SourceError::Transient("connection refused".into()).disposition(),
            Disposition::FailedRetryable { after: None }
        );
        assert_eq!(
            SourceError::rate_limited("429").disposition(),
            Disposition::FailedRetryable { after: None }
        );
        assert_eq!(
            SourceError::Config("no such directory".into()).disposition(),
            Disposition::Failed
        );
        assert_eq!(
            SourceError::Source("malformed response".into()).disposition(),
            Disposition::Failed
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
            Disposition::FailedRetryable {
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
        assert!(!config.is_record_lost());
        assert!(item.is_record_lost());
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
