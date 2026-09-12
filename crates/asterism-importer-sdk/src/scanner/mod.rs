//! `SourceScanner` trait and shared item type.
//!
//! Enumerates or watches an external source and produces
//! [`ScanEvent`]s: the [`RawItem`]s themselves, and the points a later
//! scan could take up from.
//! Bundled implementations live in the sibling modules
//! ([`fs`] and [`sqlite`]); importer authors typically reuse one
//! instead of writing their own.

pub mod fs;
pub mod sqlite;

use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use serde_json::Value;
use std::pin::Pin;

use crate::port::{SourceError, SyncState};

/// A raw scanned item — a payload plus the metadata needed to attribute
/// it back to its origin.
#[derive(Debug, Clone)]
pub struct RawItem {
    /// Source kind slug.
    ///
    /// **Ownership: the importer, not the scanner.** Scanners provide a
    /// default that identifies the *transport* (`"fs"`, `"sqlite"`,
    /// `"http"`) so ad-hoc tools work out of the box, but a published
    /// importer overrides it with a slug that names *itself* —
    /// `"cc"` for the Claude Code importer, `"persona-journal"` for
    /// the persona-journal one, `"apple-notes"` for a hypothetical
    /// Apple Notes importer. Do this via
    /// [`crate::scanner::fs::FsScanner::with_source_kind`] (or the
    /// equivalent on other scanners) at construction time.
    ///
    /// Why: the server recognises a re-arriving record by
    /// `(persona_id, source_kind, source_locator)`. Two different
    /// importers that both leave the default `"fs"` and happen to touch
    /// the same file would then share a source-kind namespace, and one
    /// would be handed the other's row instead of minting its own. The
    /// slug must be **stable across releases of the same importer** —
    /// renaming it later looks to the server like a brand-new source
    /// and re-imports everything.
    ///
    /// The slug flows verbatim into [`crate::footprint::FootprintSource::kind`]
    /// through the parser; parsers just pass it through
    /// (`item.source_kind.clone()`) and do not synthesise a different
    /// one.
    pub source_kind: String,
    /// Where the item is, inside the source (filesystem path, DB row
    /// id, URL, …).
    ///
    /// An address, and only that. The server looks it up before it
    /// mints — inside the persona, among live rows — so a value that is
    /// stable across scans is what makes a re-scanned item recognisable
    /// as the same item. It is not a uniqueness constraint: a hit is
    /// answered by handing back the row that was already there, and a
    /// caller that means to produce a second row at one address says so
    /// (`on_duplicate = separate`).
    pub locator: String,
    /// Raw bytes of the item; the parser decides how to decode them.
    pub payload: Vec<u8>,
    /// Occurrence time when the scanner can derive one cheaply
    /// (filesystem `mtime`, row timestamp column, HTTP `Date` header).
    pub occurred_at: Option<DateTime<Utc>>,
    /// Scanner-specific metadata (file stat blob, row columns, HTTP
    /// headers, …). Preserved verbatim so parsers can lift whatever
    /// they need.
    pub extra: Value,
}

/// Scan mode passed to [`SourceScanner::scan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    /// Emit every current item and finish.
    Enumerate,
    /// Emit every current item, then continue emitting new / changed
    /// items indefinitely (filesystem notify, SQL trigger, HTTP SSE, …).
    Watch,
}

/// What a scanner puts on its stream: a record, or a point it could be
/// resumed from.
///
/// The two travel together rather than through separate calls, which is
/// where every inbound framework surveyed put it — Airbyte's state
/// messages, Singer's `STATE`, the offset Kafka Connect hangs on each
/// record. The reason is that a resumption point means nothing on its
/// own: it says "everything before this is dealt with", and *before
/// this* is a position in the stream it arrived on.
#[derive(Debug, Clone)]
pub enum ScanEvent {
    /// One record, for the parser.
    Item(RawItem),
    /// Everything already yielded can be considered handled, and a scan
    /// given this state would take up after it.
    ///
    /// A promise about the scanner's side only. Whether the records
    /// before it *landed* is the runner's question, and why a caller
    /// must not store one of these until it has an answer to that — see
    /// [`ImportSummary::resume_from`](crate::ImportSummary::resume_from).
    Checkpoint(SyncState),
}

/// Async stream of scan events, or failures.
///
/// A failure on this stream is not necessarily the end of it, and what
/// it is followed by is this scanner's to decide: another event, or
/// nothing. [`SourceError::disposition`](crate::SourceError::disposition)
/// says what the failure cost the run, which is a different question
/// and deliberately not this one.
pub type ItemStream = BoxStream<'static, Result<ScanEvent, SourceError>>;

/// Future returned by [`SourceScanner::scan`] — resolves to the item
/// stream once the scanner has finished setup.
///
/// A failure here is a failure to *start*, and the classes read the
/// same as anywhere else: a scanner refused with a 503 says
/// [`Transient`](crate::SourceError::Transient), and a caller with a
/// backoff loop may try the scan again.
pub type ScanFuture<'a> =
    Pin<Box<dyn std::future::Future<Output = Result<ItemStream, SourceError>> + Send + 'a>>;

/// Trait every source scanner implements.
///
/// `scan` returns a boxed async stream of [`ScanEvent`]s, or failures.
/// A failure does not by itself end the stream — whether anything
/// follows it is this scanner's answer, and `FsScanner` and
/// `SqliteScanner` give different ones about a record they could not
/// read.
pub trait SourceScanner: Send + Sync {
    /// Starts scanning; the returned future resolves to a stream that
    /// yields events one at a time.
    ///
    /// `resume_from` is a [`SyncState`] this scanner emitted on an
    /// earlier run, or `None` to start at the beginning. What it means
    /// belongs to the scanner that wrote it — the core carries these
    /// without reading them — so a scanner is handed back only its own
    /// and may take the `offset` at face value.
    ///
    /// It may not take the *partition* at face value. A state names a
    /// unit that can be resumed, and a scanner configured differently
    /// since — a new root, another query — is being asked to take up
    /// inside something it is no longer scanning. That is a
    /// [`Config`](crate::SourceError::Config) failure and not a quiet
    /// restart: an importer that silently began again would re-import
    /// the source and look, from the outside, exactly like one that
    /// resumed.
    ///
    /// A scanner that cannot resume at all says so the same way, and
    /// says it in its own documentation as well. Ignoring the argument
    /// is not one of the answers: a caller that asked to resume and was
    /// not told it could not has been told something false about what
    /// it is about to receive.
    fn scan(&self, mode: ScanMode, resume_from: Option<SyncState>) -> ScanFuture<'_>;

    /// Whether [`RawItem::payload`] is the **complete byte content of
    /// what [`RawItem::locator`] addresses** — a whole file, a whole
    /// response body — rather than something lifted out of a container.
    ///
    /// Only a scanner can answer this, because only it knows what it
    /// read. [`fs::FsScanner`] hands over exactly the bytes at the path
    /// it names, so it says yes. [`sqlite::SqliteScanner`] hands over
    /// one column of one row and names it `<db>#<id>`, so it says no:
    /// the payload is a value out of a database, and the address has no
    /// bytes of its own at all.
    ///
    /// What turns on it is [`crate::AssetSpec::declared_content_hash`].
    /// The pipeline digests the payload and declares it **only** when
    /// this is `true` *and* the resulting spec still carries the raw
    /// item's own locator — see
    /// [`run_import`](crate::runner::run_import). Both halves are
    /// needed: a Claude Code session file is a whole file (the first
    /// holds) whose messages are addressed `<file>#<uuid>` (the second
    /// does not), and a digest of the session log attached to a message
    /// inside it would be a claim about bytes nobody will ever hash.
    ///
    /// # The default is `false`, and that is the safe direction
    ///
    /// A scanner that has not thought about it declares nothing, which
    /// costs an ingest one server-side read it was going to do anyway.
    /// The other default would have a scanner asserting digests over
    /// payloads it assembled, and the server has no way to tell that
    /// claim from a true one until the hash job disagrees with it.
    fn payload_is_whole_artefact(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two bundled scanners answer differently, and which way round
    /// they answer is the whole content of the rule.
    ///
    /// Asserted as a pair rather than one at a time: a swap compiles,
    /// reads plausibly, and turns one scanner silent while making the
    /// other assert digests over database columns.
    #[test]
    fn the_bundled_scanners_disagree_about_what_they_hand_over() {
        assert!(
            fs::FsScanner::new("/tmp").payload_is_whole_artefact(),
            "a file read whole is the case a digest can be stated for"
        );
        assert!(
            !sqlite::SqliteScanner::new(
                "/tmp/none.sqlite",
                "SELECT id, body FROM entries",
                sqlite::ColumnMap::new("id", "body"),
            )
            .payload_is_whole_artefact(),
            "a column out of a row is not the bytes at `<db>#<id>`"
        );
    }
}
