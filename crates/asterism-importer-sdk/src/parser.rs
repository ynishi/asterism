//! `SourceParser` — turn a scanned [`RawItem`] into one or more
//! [`Footprint`]s.
//!
//! Parsers are the only piece an importer author must write; everything
//! else in the pipeline (scanning, mapping to `AddAssetCommand`, HTTP,
//! progress) is provided by the SDK.
//!
//! # Contract
//!
//! Return one `Footprint` per **thing the user collected** (one chat
//! message, one image, one doc, one note). Do **not** return one giant
//! `Footprint` per raw item when the item contains many collectibles —
//! for example, one `.jsonl` file typically becomes many
//! `Footprint::ChatMessage`s, not one summary. See
//! `crate::footprint::Footprint` for how each variant maps to the
//! server-side asset.
//!
//! # `occurred_at` fallback ladder
//!
//! Every footprint needs a `DateTime<Utc>`. Parsers resolve it in this
//! order, top wins:
//!
//! 1. **A timestamp inside the payload** — a `timestamp` field on the
//!    JSON record, a `created_at` column on the SQLite row, an EXIF
//!    `DateTimeOriginal`. This is per-*record* and is by far the most
//!    accurate.
//! 2. **[`crate::scanner::RawItem::occurred_at`]** — what the scanner
//!    derived from the container (file `mtime`, HTTP `Date` header,
//!    row timestamp column when the scanner already lifted it). This
//!    is per-*container*, so it is a good fallback for single-record
//!    items (one image = one file, one doc = one file) and a
//!    **coarse** fallback for multi-record containers (all messages
//!    in one `.jsonl` share the same `mtime`, which loses ordering).
//! 3. **`Utc::now()`** — last resort when neither of the above is
//!    available. Signals to the caller that ordering downstream will
//!    be unreliable.
//!
//! Never invert the order. Using the file `mtime` for a chat message
//! inside a session log makes every message look like it arrived at
//! the moment the file was last flushed, which erases the
//! within-session ordering the domain relies on for edge / grid
//! placement.
//!
//! # Partial success on multi-footprint items
//!
//! One `RawItem` may yield many footprints (JSONL: one file → many
//! messages; SQLite: one table → many rows). If some records inside
//! the item are malformed, prefer to **skip them and return the good
//! ones** rather than returning `ParseError::Malformed` for the whole
//! batch — a single bad line should not drop the rest of a session.
//! Reserve [`ParseError::Malformed`] for cases where the whole
//! `RawItem` is unusable (wrong file type, unreadable header).
//!
//! Skipping is not silent. Count what was dropped and say so once per
//! container at the end of `parse`; [`RecordAddresses`] owns both the
//! count and the wording. An importer that drops records without a
//! word leaves the operator reading a run that looks complete.
//!
//! # A record's address is the source's to give
//!
//! A record inside a container is addressed
//! `<container>#<the id the source declared>`. When the source
//! declares no id there is no address, and the record does not become
//! an asset. Do **not** substitute the record's position — a line
//! number, an array index, an ordinal.
//!
//! A position describes the container's contents at one moment, not
//! the record. Insert one line ahead and every address behind the
//! insert lands on its neighbour, where the server's
//! `(source_kind, source_locator)` lookup finds the neighbour's row
//! and discards the arriving payload. Nothing errors; the import
//! reports success and the record is gone. The address is also
//! unreadable in the other direction — the readers in
//! `asterism-infra` match a fragment against the record's own id, and
//! no record has the id `L3`, so the body never resolves.
//!
//! [`RecordAddresses`] is the shared implementation of this rule.

use crate::footprint::Footprint;
use crate::scanner::RawItem;

/// Errors returned by parsers.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The scanned item is not in the shape the parser expects (bad
    /// header, wrong file extension, malformed content).
    #[error("parse failed for {locator}: {message}")]
    Malformed {
        /// The `locator` of the offending `RawItem`.
        locator: String,
        /// A short human-readable reason.
        message: String,
    },
    /// Wraps any other library / codec failure.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Trait every source parser implements.
///
/// One `RawItem` may produce zero, one, or many `Footprint`s.
/// Returning an empty vector is a legitimate "skip" signal (for
/// example when the scanner picked up an unrelated file).
pub trait SourceParser: Send + Sync {
    /// Parses one raw item.
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError>;
}

/// One container's worth of the addressing rule above: hands back the
/// id the source declared, and keeps count of the records it had to
/// turn down.
///
/// A parser builds one of these per `RawItem`, asks
/// [`declared`](Self::declared) for each record's id, skips the
/// records that come back `None`, and calls [`report`](Self::report)
/// once at the end of `parse`.
///
/// ```
/// # use asterism_importer_sdk::RecordAddresses;
/// let mut addresses = RecordAddresses::in_container("/logs/s.jsonl");
/// assert_eq!(addresses.declared(Some("u1")), Some("u1"));
/// assert_eq!(addresses.declared(None), None); // caller skips this record
/// addresses.report(); // one line on stderr, naming the container
/// ```
///
/// The report goes to stderr directly rather than through `tracing`:
/// no importer crate installs a subscriber, so a `tracing::warn!` here
/// would compile, pass review, and print nothing. The neighbouring
/// per-container line in [`crate::runner`] is written the same way.
pub struct RecordAddresses {
    container: String,
    total: usize,
    dropped: usize,
}

impl RecordAddresses {
    /// Starts a tally for one container — pass the `RawItem`'s
    /// locator, which is what the report names.
    pub fn in_container(container: impl Into<String>) -> Self {
        Self {
            container: container.into(),
            total: 0,
            dropped: 0,
        }
    }

    /// Accounts for one record and returns the id the source declared
    /// for it.
    ///
    /// `None` — including an id that is present but blank — means the
    /// record has no address and the caller must skip it. The caller
    /// must not invent one; see the module rustdoc for what happens
    /// downstream when it does.
    pub fn declared<'a>(&mut self, id: Option<&'a str>) -> Option<&'a str> {
        self.total += 1;
        let id = id.filter(|s| !s.trim().is_empty());
        if id.is_none() {
            self.dropped += 1;
        }
        id
    }

    /// The single line this container has to say, or `None` when every
    /// record had an address of its own.
    ///
    /// This is per scan, not per lifetime: scanning the same container
    /// again reports the same records again. Collapsing repeats would
    /// mean holding state across runs, which is a bigger promise than
    /// a parser should make.
    fn diagnostic(&self) -> Option<String> {
        (self.dropped > 0).then(|| {
            format!(
                "records dropped: {} of {} carried no id of their own ({})",
                self.dropped, self.total, self.container
            )
        })
    }

    /// Writes [`Self::diagnostic`] to stderr. Call once, at the end of
    /// `parse`; silent when nothing was dropped.
    pub fn report(&self) {
        if let Some(line) = self.diagnostic() {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_id_comes_back_unchanged() {
        let mut addresses = RecordAddresses::in_container("/logs/s.jsonl");
        assert_eq!(addresses.declared(Some("u1")), Some("u1"));
        assert_eq!(addresses.dropped, 0);
        assert_eq!(addresses.total, 1);
    }

    #[test]
    fn a_record_with_no_id_is_turned_down_and_counted() {
        let mut addresses = RecordAddresses::in_container("/logs/s.jsonl");
        assert_eq!(addresses.declared(None), None);
        assert_eq!(addresses.dropped, 1, "the caller has to skip it");
        assert_eq!(addresses.total, 1, "and it still counts as a record");
    }

    /// A present-but-blank id would address the container itself
    /// (`/logs/s.jsonl#`), which is a worse failure than dropping the
    /// record: every blank-id record in the file collides on one row.
    #[test]
    fn a_blank_id_is_not_an_id() {
        let mut addresses = RecordAddresses::in_container("/logs/s.jsonl");
        assert_eq!(addresses.declared(Some("")), None);
        assert_eq!(addresses.declared(Some("   ")), None);
        assert_eq!(addresses.dropped, 2);
    }

    #[test]
    fn a_container_that_dropped_nothing_says_nothing() {
        let mut addresses = RecordAddresses::in_container("/logs/s.jsonl");
        addresses.declared(Some("u1"));
        addresses.declared(Some("u2"));
        assert_eq!(
            addresses.diagnostic(),
            None,
            "a clean container is not worth a line"
        );
    }

    #[test]
    fn the_report_names_the_container_and_both_counts() {
        let mut addresses = RecordAddresses::in_container("/logs/s.jsonl");
        addresses.declared(Some("u1"));
        addresses.declared(None);
        addresses.declared(None);

        // Pinned whole rather than probed with `contains('2')`: a
        // digit search passes on any container path that happens to
        // hold that digit, so it would stop testing the counts the
        // moment the fixture path changed. Both counts matter — 2 of 3
        // is a bad container, 2 of 4000 is a normal one, and only the
        // pair tells them apart.
        assert_eq!(
            addresses.diagnostic().as_deref(),
            Some("records dropped: 2 of 3 carried no id of their own (/logs/s.jsonl)"),
        );
    }
}
