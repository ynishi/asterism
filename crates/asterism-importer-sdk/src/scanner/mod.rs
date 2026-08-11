//! `SourceScanner` trait and shared item type.
//!
//! Enumerates or watches an external source and produces [`RawItem`]s.
//! Bundled implementations live in the sibling modules
//! ([`fs`], and future `sqlite` / `http`); importer authors typically
//! reuse one instead of writing their own.

pub mod fs;
pub mod sqlite;

use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use serde_json::Value;
use std::pin::Pin;

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

/// Errors returned by scanners.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// Source path / URL / query was invalid or unreachable.
    #[error("source unavailable: {0}")]
    SourceUnavailable(String),
    /// Item-level I/O failure that should be surfaced but not necessarily
    /// abort the whole scan.
    #[error("item read failed: {0}")]
    ItemReadFailed(String),
    /// Wraps any other transport / library failure.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Async stream of scanned items (or per-item errors).
pub type ItemStream = BoxStream<'static, Result<RawItem, ScanError>>;

/// Future returned by [`SourceScanner::scan`] — resolves to the item
/// stream once the scanner has finished setup.
pub type ScanFuture<'a> =
    Pin<Box<dyn std::future::Future<Output = Result<ItemStream, ScanError>> + Send + 'a>>;

/// Trait every source scanner implements.
///
/// `scan` returns a boxed async stream of `RawItem`s (or per-item
/// errors, so a single bad row does not tear the whole scan down).
pub trait SourceScanner: Send + Sync {
    /// Starts scanning; the returned future resolves to a stream that
    /// yields items one at a time.
    fn scan(&self, mode: ScanMode) -> ScanFuture<'_>;

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
