//! `HttpScanner` — paginated HTTP source scanner.
//!
//! Asks a URL for a page of records, hands each one over, follows the
//! cursor the response carried, and repeats. The caller supplies the
//! request and says how to read the answer; this scanner knows nothing
//! about any particular service.
//!
//! **The inbound twin of [`SqliteScanner`](super::sqlite::SqliteScanner),
//! and deliberately not an API client.** That one takes a `SELECT` and a
//! column map and has no opinion about anybody's schema; this takes a
//! request and a few field names and has no opinion about anybody's API.
//! A named service is configuration rather than code, which is the only
//! version of "thirty or forty adapters" that stays maintainable.
//!
//! # Why this one exists
//!
//! Not for the sources it reaches. The port's classification was written
//! for remote sources, and no *scanner* had ever been one: a directory
//! cannot be rate limited and a SQLite file is never briefly
//! unreachable. [`RateLimited`](crate::SourceError::RateLimited) was
//! constructed nowhere in this workspace but a fixture in the runner's
//! tests, so `retry_after` — the field that whole class exists for —
//! had never been filled in by anything that read a header. An
//! interface is only as good as the adapter shaped least like the ones
//! it was written against, and this is that adapter.
//!
//! Three things it does differently, each of which the port had ruled
//! on and nothing had exercised:
//!
//! - **Its offset cannot be compared.** `FsScanner` compares paths and
//!   `SqliteScanner` compares ids, so both can decide whether a record
//!   falls before a resumption point. A cursor can only be handed back
//!   to the source that issued it. The port's rule that an offset is
//!   opaque and only its writer reads it is what makes that work.
//! - **Its checkpoints are coarse.** `FsScanner` emits one behind every
//!   file, and `SqliteScanner` behind every row it can order. A page is
//!   the smallest thing this can take up after, so fifty records share
//!   one checkpoint.
//! - **Resuming costs nothing.** The others read from the beginning and
//!   skip what is behind the point. This asks the source to start
//!   there, so the records before it are never fetched, parsed or paid
//!   for.
//!
//! # What it does not know how to do
//!
//! One pagination dialect: a cursor at a path in the response body, sent
//! back as a query parameter. `Link: rel="next"` headers and page
//! numbers are the other two common ones and are not here — the shape to
//! add them is an enum in place of [`Cursor`], and adding it before a
//! second dialect is actually needed would be guessing at what it needs
//! to hold.
//!
//! It also leaves
//! [`payload_is_whole_artefact`](super::SourceScanner::payload_is_whole_artefact)
//! at its `false` default, for the same reason `SqliteScanner` does: a
//! record lifted out of a JSON array and addressed `<url>#<id>` has no
//! bytes of its own at that address, so a digest declared from here
//! could never be checked.

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::stream::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{RawItem, ScanEvent, ScanFuture, ScanMode, SourceScanner};
use crate::port::{SourceError, SyncState};

/// How the next page is asked for.
///
/// One dialect, named rather than assumed: the response carries a token
/// at `path`, and the next request sends it as the query parameter
/// `param`.
///
/// Nothing at `path`, or `null` there, is the last page. A string or a
/// number is the token. Anything else present is a failure rather than
/// an end — see [`next_cursor`].
#[derive(Debug, Clone)]
pub struct Cursor {
    /// Path to the token in the response body, walked key by key.
    /// Empty means the token is the whole body, which no real service
    /// does and which is therefore not special-cased.
    pub path: Vec<String>,
    /// Query parameter the token is sent back as.
    pub param: String,
}

impl Cursor {
    /// A cursor read from `path` and sent back as `param`.
    pub fn new(
        path: impl IntoIterator<Item = impl Into<String>>,
        param: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            param: param.into(),
        }
    }
}

/// Where the records are in the response, and what identifies one.
#[derive(Debug, Clone)]
pub struct RecordMap {
    /// Path to the array of records, walked key by key. Empty means the
    /// body is itself the array.
    pub items_path: Vec<String>,
    /// Field on each record holding its id.
    ///
    /// Combined with the URL's address — scheme, host and path, without
    /// the query string — to form [`RawItem::locator`], which is what
    /// the server's identity reads. So it has to be stable for the life
    /// of the record: a field the service calls `position` or `index` is
    /// not one, and neither is anything that changes when the caller
    /// moves a `since=` window.
    pub id_field: String,
    /// Optional field holding the record's occurrence time: RFC 3339
    /// text, or an integer read as unix epoch milliseconds.
    pub timestamp_field: Option<String>,
}

impl RecordMap {
    /// A map with only the required parts set.
    pub fn new(
        items_path: impl IntoIterator<Item = impl Into<String>>,
        id_field: impl Into<String>,
    ) -> Self {
        Self {
            items_path: items_path.into_iter().map(Into::into).collect(),
            id_field: id_field.into(),
            timestamp_field: None,
        }
    }

    /// Sets the optional timestamp field.
    pub fn with_timestamp(mut self, field: impl Into<String>) -> Self {
        self.timestamp_field = Some(field.into());
        self
    }
}

/// Paginated HTTP scanner.
#[derive(Debug, Clone)]
pub struct HttpScanner {
    url: String,
    headers: Vec<(String, String)>,
    records: RecordMap,
    cursor: Cursor,
    source_kind: String,
    max_pages: usize,
    client: reqwest::Client,
}

/// How long one request may take before it is a failure rather than a
/// wait.
///
/// A whole-request ceiling, matching the outbound HTTP exporter's. A
/// source that accepts a connection and then says nothing is
/// indistinguishable from one that is slow, and without a clock the
/// import waits for either forever with nothing reported — which is the
/// same silence a runaway paging loop produces and just as hard to see.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// How many pages a single scan will follow before calling the source
/// broken.
///
/// A ceiling and not a limit: reaching it **fails the run** rather than
/// ending it quietly, because a scan that stopped early and reported
/// success is the silent loss this whole port is built to rule out.
///
/// It exists because the cycle guard below catches only the simplest
/// non-progress — a token handed straight back. A source alternating
/// two cursors, or issuing a fresh one forever, defeats it and pages
/// until somebody notices, which on the evidence of this branch's own
/// fixture is thirty-seven minutes and counting. High enough that a
/// real source reaching it is a bug and not a Tuesday; [`with_max_pages`]
/// is there for whoever has the exception.
///
/// [`with_max_pages`]: HttpScanner::with_max_pages
const DEFAULT_MAX_PAGES: usize = 10_000;

impl HttpScanner {
    /// Builds a scanner over `url`, reading records per `records` and
    /// following pages per `cursor`.
    pub fn new(url: impl Into<String>, records: RecordMap, cursor: Cursor) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            records,
            cursor,
            source_kind: "http".into(),
            max_pages: DEFAULT_MAX_PAGES,
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client build"),
        }
    }

    /// Raises or lowers the page ceiling — see [`DEFAULT_MAX_PAGES`].
    ///
    /// For a source that genuinely has more pages than the default, and
    /// for a caller who would rather be told sooner that one is not
    /// advancing.
    pub fn with_max_pages(mut self, pages: usize) -> Self {
        self.max_pages = pages.max(1);
        self
    }

    /// Adds a header to every request.
    ///
    /// Where a credential goes, and the only place one safely does:
    /// this scanner has no notion of an auth scheme, so whatever the
    /// service wants is spelled by the caller who knows.
    ///
    /// Headers are not recorded anywhere. The URL is — whole, query
    /// string included, in the partition a position is filed under and
    /// in every failure message — so a token in a query parameter ends
    /// up in the database and the logs, and one here does not.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Overrides the slug written to [`RawItem::source_kind`].
    pub fn with_source_kind(mut self, slug: impl Into<String>) -> Self {
        self.source_kind = slug.into();
        self
    }

    /// The address half of a record's locator: this URL without its
    /// query string.
    ///
    /// Kept apart from [`partition_key`](Self::partition_key), which
    /// keeps the query, and the split is the one `SqliteScanner` makes:
    /// a row is addressed `{db}#{id}` while its partition is the
    /// database *and* the `SELECT`. Parameters choose *which* records
    /// come back; they are not part of *where* a record lives.
    ///
    /// Getting this wrong duplicates. The server's identity is
    /// `(persona_id, source_kind, source_locator)` compared as text, so
    /// a locator carrying `?since=A` and one carrying `?since=B` are two
    /// rows for one record — and two runs of the same import with a
    /// moved window is the ordinary way to use such a flag.
    fn address(&self) -> &str {
        match self.url.split_once('?') {
            Some((base, _)) => base,
            None => &self.url,
        }
    }

    /// The resumable unit: this scanner's kind and its URL.
    ///
    /// The URL whole, query string included, because the parameters a
    /// caller put on it choose what the source returns — a `since=` or a
    /// `type=` makes it a different set of records, and a position in
    /// one is not a position in the other. The cursor parameter is not
    /// on it, because that is this scanner's own addition and changes
    /// per request rather than per configuration.
    fn partition_key(&self) -> String {
        format!("kind={}|url={}", self.source_kind, self.url)
    }

    /// The cursor a resumption point says to start from.
    ///
    /// The partition is compared, as
    /// [`SourceScanner::scan`](super::SourceScanner::scan) requires of
    /// every scanner. The *token* is not: unlike a path or a row id it
    /// is handed straight back to the source, which is the only thing
    /// that knows what it means. That is the port's opacity rule doing
    /// its job — and the reason this scanner can resume without reading
    /// what it has already read.
    fn resume_after(&self, state: Option<SyncState>) -> Result<Option<String>, SourceError> {
        let Some(state) = state else {
            return Ok(None);
        };
        if state.partition != self.partition_key() {
            return Err(SourceError::Config(format!(
                "cannot resume: the state is for {:?} and this scanner reads {:?}",
                state.partition,
                self.partition_key()
            )));
        }
        match state.offset.get("after_cursor").and_then(|v| v.as_str()) {
            Some(token) => Ok(Some(token.to_string())),
            None => Err(SourceError::Config(format!(
                "cannot resume: the state for {:?} carries no `after_cursor`",
                state.partition
            ))),
        }
    }
}

impl SourceScanner for HttpScanner {
    /// One: a URL with a cursor behind it is exactly a thing with a
    /// position in it.
    fn partition(&self) -> Option<String> {
        Some(self.partition_key())
    }

    fn scan(&self, mode: ScanMode, resume_from: Option<SyncState>) -> ScanFuture<'_> {
        let this = self.clone();
        Box::pin(async move {
            if mode == ScanMode::Watch {
                // A watch over HTTP is a polling loop, and how often to
                // poll is a question about somebody's rate limit rather
                // than about this scanner. Refused loudly rather than
                // answered with a number picked here.
                return Err(SourceError::Config(
                    "HttpScanner does not support watch mode: polling belongs to a caller \
                     that knows the source's rate limit"
                        .into(),
                ));
            }

            // Refused before the first request, so a caller that asked
            // to resume and cannot hears it instead of quietly reading
            // the source from its beginning.
            let start = this.resume_after(resume_from)?;

            let (tx, rx) = mpsc::channel::<Result<ScanEvent, SourceError>>(64);
            tokio::spawn(async move {
                this.walk(start, tx).await;
            });

            Ok(ReceiverStream::new(rx).boxed())
        })
    }
}

impl HttpScanner {
    /// Page after page until the source stops offering a next cursor,
    /// or something ends the run.
    async fn walk(self, start: Option<String>, tx: mpsc::Sender<Result<ScanEvent, SourceError>>) {
        let partition = self.partition_key();
        let mut cursor = start;
        for page in 1.. {
            if page > self.max_pages {
                let _ = tx
                    .send(Err(SourceError::Source(format!(
                        "{}: stopped after {} pages without reaching the end of the \
                         source — it is either larger than this scanner's ceiling or \
                         not advancing; see HttpScanner::with_max_pages",
                        self.url, self.max_pages
                    ))))
                    .await;
                return;
            }
            let body = match self.fetch(cursor.as_deref()).await {
                Ok(body) => body,
                Err(err) => {
                    // Every failure this can raise ends the run: there
                    // is no next page to ask for when the current one
                    // did not arrive, and the cursor that would have
                    // asked came out of the page that is missing.
                    let _ = tx.send(Err(err)).await;
                    return;
                }
            };

            let items = match walk_path(&body, &self.records.items_path) {
                Some(Value::Array(items)) => items.clone(),
                // The source answered, and this build cannot find the
                // records in what it said. `Source` rather than
                // `Config`, because a path the caller got wrong and a
                // response that changed shape look identical from here
                // and only one of them is worth telling an operator to
                // go and fix — see [`classify`](Self::classify) for
                // where that cut is drawn over statuses.
                _ => {
                    let _ = tx
                        .send(Err(SourceError::Source(format!(
                            "no array of records at {:?}",
                            self.records.items_path.join(".")
                        ))))
                        .await;
                    return;
                }
            };

            for item in items {
                match self.to_raw_item(&item) {
                    Ok(raw) => {
                        if tx.send(Ok(ScanEvent::Item(raw))).await.is_err() {
                            return;
                        }
                    }
                    // One record, not the page: a record with no usable
                    // id is one the locator cannot address, and the
                    // rest of the page is unaffected. `FsScanner` makes
                    // the same call about an unreadable file;
                    // `SqliteScanner` cannot, because the failure
                    // detaches its cursor. The port leaves the answer
                    // to the scanner for exactly that reason — each one
                    // knows what its own source can still do.
                    Err(err) => {
                        if tx.send(Err(err)).await.is_err() {
                            return;
                        }
                    }
                }
            }

            let next = match next_cursor(walk_path(&body, &self.cursor.path)) {
                Ok(next) => next,
                Err(unsendable) => {
                    let _ = tx
                        .send(Err(SourceError::Source(format!(
                            "{}: the next-page token at {:?} is {unsendable}, which \
                             cannot be sent back as a query parameter",
                            self.url,
                            self.cursor.path.join(".")
                        ))))
                        .await;
                    return;
                }
            };
            let Some(next) = next else {
                // The last page. **No checkpoint for it**, because the
                // only token this scanner can hand back is one the
                // source issued, and the source issued none — so the
                // position it can honestly report is the one before
                // this page. A later run re-reads the final page and
                // the locators answer for the duplicates.
                //
                // Representing "exhausted" would mean inventing a token
                // with no meaning to the source, which is the one thing
                // an opaque offset must never contain.
                return;
            };

            // A source that hands back the token it was just given is
            // not advancing, and following it is an importer that
            // never returns and never says why. Ending is the only
            // thing that can be said truthfully, and `Source` is the
            // class: the far end is behaving in a way no configuration
            // here can fix.
            //
            // This is the one thing this scanner does with a cursor
            // besides hand it back, and it is *equality* rather than
            // order — "the same token twice" needs no understanding of
            // what a token means, which is why it does not break the
            // opacity rule that the rest of this file rests on.
            //
            // Found by a test fixture, not by reasoning: a fake source
            // that repeated its last answer turned a mutation of this
            // loop into a thirty-seven-minute hang instead of a failed
            // assertion. A real service that echoes a cursor would have
            // done the same to an import.
            if cursor.as_deref() == Some(next.as_str()) {
                let _ = tx
                    .send(Err(SourceError::Source(format!(
                        "{}: pagination is not advancing — the source returned the \
                         same cursor it was given",
                        self.url
                    ))))
                    .await;
                return;
            }

            // After the page, because a checkpoint says everything
            // already yielded is dealt with — and a page is the
            // smallest unit this source can be taken up after. The
            // token stored is the one that fetches what comes *next*,
            // so resuming is a request rather than a comparison.
            //
            // Emitted even when a record on this page was lost, and
            // that is not this scanner's oversight: a checkpoint says
            // the page was handed over, and whether the records on it
            // *landed* is the runner's to know —
            // `ImportSummary::resume_from` withholds the position from
            // any run that failed a record, so a lost record here means
            // nothing is stored and the next run re-reads the page.
            let checkpoint = ScanEvent::Checkpoint(SyncState::new(
                partition.clone(),
                json!({ "after_cursor": next.clone() }),
            ));
            if tx.send(Ok(checkpoint)).await.is_err() {
                return;
            }
            cursor = Some(next);
        }
    }

    /// One page, or the classified reason there is none.
    async fn fetch(&self, cursor: Option<&str>) -> Result<Value, SourceError> {
        let mut request = self.client.get(&self.url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(cursor) = cursor {
            request = request.query(&[(&self.cursor.param, cursor)]);
        }

        let response = request.send().await.map_err(|err| {
            // Nothing came back. The socket was refused, the name did
            // not resolve, the read timed out — all of which the same
            // call in a moment may survive.
            SourceError::Transient(format!("{}: {err}", self.url))
        })?;

        let status = response.status();
        if status.is_success() {
            return response.json::<Value>().await.map_err(|err| {
                // It answered, and what it said is not JSON. The far
                // end's trouble, not the operator's settings.
                SourceError::Source(format!("{}: response did not parse: {err}", self.url))
            });
        }

        Err(self.classify(status, response.headers()))
    }

    /// What an unsuccessful status costs the run.
    ///
    /// The four cuts the port draws, applied to HTTP for the first time
    /// in this workspace:
    ///
    /// - **429** is [`RateLimited`](SourceError::RateLimited), carrying
    ///   `Retry-After` when the source stated one. That header is the
    ///   whole reason the class is separate from `Transient`: it is the
    ///   difference between a report that says "failed" and one that
    ///   says when the source is expected back.
    /// - **502, 503, 504** are [`Transient`](SourceError::Transient).
    ///   Each means the far side is unavailable *right now* — a bad
    ///   gateway, a service down, a timeout upstream — which is the
    ///   definition of the class.
    /// - **Other 5xx**, 500 among them, are
    ///   [`Source`](SourceError::Source). A 500 is the source saying
    ///   this request broke it, and repeating an unchanged request that
    ///   broke it is not a plan. Kept apart from the three above
    ///   deliberately: "come back in a minute" and "this will not work
    ///   until somebody looks at it" ask different things of whoever
    ///   reads the report.
    /// - **4xx** are [`Config`](SourceError::Config). A 401, a 403, a
    ///   404 on a URL the caller typed — each is settings, and each
    ///   repeats identically until a person changes something.
    fn classify(
        &self,
        status: reqwest::StatusCode,
        headers: &reqwest::header::HeaderMap,
    ) -> SourceError {
        let url = &self.url;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return SourceError::RateLimited {
                retry_after: retry_after(headers),
                message: format!("{url}: HTTP {status}"),
            };
        }
        if matches!(status.as_u16(), 502..=504) {
            return SourceError::Transient(format!("{url}: HTTP {status}"));
        }
        if status.is_server_error() {
            return SourceError::Source(format!("{url}: HTTP {status}"));
        }
        SourceError::Config(format!("{url}: HTTP {status}"))
    }

    /// One record, or the reason this one is lost.
    fn to_raw_item(&self, item: &Value) -> Result<RawItem, SourceError> {
        let id = item
            .get(&self.records.id_field)
            .and_then(value_as_id)
            .ok_or_else(|| {
                SourceError::item(
                    self.url.clone(),
                    format!(
                        "record has no usable {:?}, so nothing can address it",
                        self.records.id_field
                    ),
                )
            })?;
        let payload = serde_json::to_vec(item).map_err(|err| {
            SourceError::item(
                format!("{}#{id}", self.address()),
                format!("record did not serialise: {err}"),
            )
        })?;
        let occurred_at = self
            .records
            .timestamp_field
            .as_deref()
            .and_then(|field| item.get(field))
            .and_then(parse_timestamp);

        Ok(RawItem {
            source_kind: self.source_kind.clone(),
            locator: format!("{}#{id}", self.address()),
            payload,
            occurred_at,
            extra: json!({ "source_url": self.url }),
        })
    }
}

/// The next page's token, `None` at the end of the source, or the name
/// of a shape that cannot be one.
///
/// Absent and `null` are the end — the two ways a service says there is
/// no more. A string is the token; so is a number, because a service
/// that numbers its cursors is sending the same thing in a different
/// JSON type and both go back on the query string identically.
///
/// Anything else is a **failure and not an end**, which is the whole
/// point of this function existing. The first shape of this read the
/// token with `as_str` and treated everything that was not a string as
/// the last page: a source whose cursor was a number imported its first
/// page and reported a clean, complete run. Silence over a source with
/// more to give is the failure mode this port was built to rule out,
/// and it had arrived here by way of a convenience method.
fn next_cursor(value: Option<&Value>) -> Result<Option<String>, &'static str> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(token)) => Ok(Some(token.clone())),
        Some(Value::Number(token)) => Ok(Some(token.to_string())),
        Some(Value::Bool(_)) => Err("a boolean"),
        Some(Value::Array(_)) => Err("an array"),
        Some(Value::Object(_)) => Err("an object"),
    }
}

/// Walks `path` key by key. An empty path is the value itself.
fn walk_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |cursor, key| cursor.get(key))
}

/// An id as text, from the two JSON types an id is.
///
/// A number is rendered rather than refused, because plenty of services
/// number their records and a locator is text either way. Everything
/// else is refused, the empty string included — which is the one a real
/// service plausibly sends, and an address of `<url>#` would collide
/// every such record onto one row.
fn value_as_id(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// RFC 3339 text, or an integer read as unix epoch milliseconds.
fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        Value::Number(n) => n.as_i64().and_then(DateTime::from_timestamp_millis),
        _ => None,
    }
}

/// The wait a `Retry-After` header states, in the one form worth
/// reading.
///
/// HTTP allows delay-seconds or an HTTP-date. Only the first is read
/// here: the second needs a clock both ends agree on, and a wait
/// computed from a skewed clock is worse than no wait stated at all —
/// `None` means "it refused us without saying for how long", which is
/// honest, and a wrong number is not.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::port::Disposition;

    /// One scripted answer from the fake source.
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Reply {
        /// A page of records with the given ids, and the cursor that
        /// fetches whatever comes after it.
        fn page(ids: &[&str], next: Option<&str>) -> Self {
            let items: Vec<String> = ids
                .iter()
                .map(|id| format!(r#"{{"id":"{id}","body":"record {id}"}}"#))
                .collect();
            let next = match next {
                Some(token) => format!(r#""{token}""#),
                None => "null".into(),
            };
            Self::ok(format!(
                r#"{{"data":{{"items":[{}]}},"paging":{{"next":{next}}}}}"#,
                items.join(",")
            ))
        }

        fn ok(body: impl Into<String>) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                body: body.into(),
            }
        }

        fn status(status: u16) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body: "{}".into(),
            }
        }

        fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.into(), value.into()));
            self
        }
    }

    /// What the tests read the fake source's memory through.
    #[derive(Clone, Default)]
    struct Asked(Arc<Mutex<Vec<Option<String>>>>);

    impl Asked {
        /// Every cursor the scanner requested with, in order. `None` is
        /// a request that carried no cursor — the first page.
        fn cursors(&self) -> Vec<Option<String>> {
            self.0.lock().expect("the fake source's log").clone()
        }
    }

    /// A loopback source that answers `replies` in order and records
    /// what it was asked for.
    ///
    /// Hand-rolled for the reason the runner's counting server is: this
    /// crate has no HTTP test dependency, and what these tests need is a
    /// socket that says 429 on demand — which is the one thing a real
    /// service will not do when asked.
    async fn spawn_source(replies: Vec<Reply>) -> (u16, Asked) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        let asked = Asked::default();
        let log = asked.clone();
        let replies = Arc::new(Mutex::new(replies.into_iter().collect::<Vec<_>>()));

        tokio::spawn(async move {
            let mut served = 0usize;
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                // Read to the end of the head; a GET carries no body.
                let mut request = String::new();
                let mut buf = vec![0u8; 8 * 1024];
                loop {
                    let Ok(n) = socket.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if request.contains("\r\n\r\n") {
                        break;
                    }
                }

                let target = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let cursor = target.split_once('?').and_then(|(_, query)| {
                    query
                        .split('&')
                        .find_map(|pair| pair.strip_prefix("cursor=").map(|v| v.to_string()))
                });
                log.0.lock().expect("the fake source's log").push(cursor);

                let reply = {
                    let replies = replies.lock().expect("the script");
                    // Past the end of the script the source says so,
                    // rather than repeating its last answer. A fixture
                    // that repeats turns "the scanner never stops
                    // paging" into a hang, which is how the guard above
                    // came to be written — a test that hangs reports
                    // nothing, for as long as somebody lets it run.
                    let exhausted = Reply::status(500);
                    let reply = replies.get(served).unwrap_or(&exhausted);
                    let headers: String = reply
                        .headers
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}\r\n"))
                        .collect();
                    format!(
                        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\n{headers}\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        reply.status,
                        reply.body.len(),
                        reply.body
                    )
                };
                served += 1;
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        (port, asked)
    }

    fn scanner(port: u16) -> HttpScanner {
        HttpScanner::new(
            format!("http://127.0.0.1:{port}/records"),
            RecordMap::new(["data", "items"], "id"),
            Cursor::new(["paging", "next"], "cursor"),
        )
    }

    async fn drain_events(
        scanner: &HttpScanner,
        resume_from: Option<SyncState>,
    ) -> Vec<Result<ScanEvent, SourceError>> {
        match scanner.scan(ScanMode::Enumerate, resume_from).await {
            Ok(stream) => stream.collect().await,
            Err(err) => vec![Err(err)],
        }
    }

    fn locators(events: &[Result<ScanEvent, SourceError>]) -> Vec<String> {
        events
            .iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Item(item)) => Some(item.locator.clone()),
                _ => None,
            })
            .collect()
    }

    fn checkpoints(events: &[Result<ScanEvent, SourceError>]) -> Vec<SyncState> {
        events
            .iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Checkpoint(state)) => Some(state.clone()),
                _ => None,
            })
            .collect()
    }

    fn failures(events: &[Result<ScanEvent, SourceError>]) -> Vec<SourceError> {
        events
            .iter()
            .filter_map(|r| r.as_ref().err().cloned())
            .collect()
    }

    /// Every page, every record once, and a checkpoint where a page
    /// ends rather than where a record does.
    ///
    /// The coarse checkpoint is the point: two of them for three pages,
    /// because the last page carried no cursor and this scanner will not
    /// invent one. A run that reaches the end therefore reports the
    /// position before the final page, and the next run re-reads it —
    /// duplicates the locator answers for, rather than a token the
    /// source never issued.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_paginated_source_is_walked_to_its_end() {
        let (port, asked) = spawn_source(vec![
            Reply::page(&["1", "2"], Some("c1")),
            Reply::page(&["3", "4"], Some("c2")),
            Reply::page(&["5", "6"], None),
        ])
        .await;
        let scanner = scanner(port);
        let at = |id: &str| format!("http://127.0.0.1:{port}/records#{id}");

        let events = drain_events(&scanner, None).await;
        assert_eq!(
            locators(&events),
            ["1", "2", "3", "4", "5", "6"].map(at).to_vec(),
            "each record once, in the order the pages gave them"
        );
        assert_eq!(
            checkpoints(&events)
                .iter()
                .map(|state| state.offset.clone())
                .collect::<Vec<_>>(),
            vec![
                json!({ "after_cursor": "c1" }),
                json!({ "after_cursor": "c2" })
            ],
            "one per page that offered a next cursor, and none for the last"
        );
        assert_eq!(
            asked.cursors(),
            vec![None, Some("c1".into()), Some("c2".into())],
            "and the scanner followed the source's own tokens"
        );
    }

    /// **A resumed scan asks the source to start there**, so the pages
    /// behind the point are never fetched.
    ///
    /// Asserted on what the source was asked for, not on what the run
    /// counted. A count of zero is also what a scanner that requested
    /// everything and threw it away would report, and the difference is
    /// the whole value of a cursor: the other two scanners genuinely do
    /// read what they skip.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_resumed_scan_asks_the_source_to_start_there() {
        let (port, asked) = spawn_source(vec![Reply::page(&["5", "6"], None)]).await;
        let scanner = scanner(port);

        let point = SyncState::new(
            SourceScanner::partition(&scanner).expect("a URL has a partition"),
            json!({ "after_cursor": "c2" }),
        );
        let events = drain_events(&scanner, Some(point)).await;

        assert_eq!(locators(&events).len(), 2);
        assert_eq!(
            asked.cursors(),
            vec![Some("c2".into())],
            "one request, carrying the token, and no walk from the beginning"
        );
    }

    /// A rate limit carries the wait the source stated, and the wait
    /// reaches whoever reads the failure.
    ///
    /// The header is the entire reason this class is separate from
    /// `Transient`, and until this scanner nothing in the workspace had
    /// ever filled it in from one. Both halves are asserted: the
    /// duration on the value, and the rendering an operator actually
    /// sees.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_rate_limit_carries_the_wait_the_source_stated() {
        let (port, _) =
            spawn_source(vec![Reply::status(429).with_header("Retry-After", "90")]).await;

        let events = drain_events(&scanner(port), None).await;
        let err = failures(&events).into_iter().next().expect("one failure");
        assert_eq!(
            err.disposition(),
            Disposition::FailedRetryable {
                after: Some(Duration::from_secs(90))
            },
            "the wait is what a caller acts on: {err}"
        );
        assert!(
            err.to_string().contains("retry after 90s"),
            "and it reaches a person through the message: {err}"
        );
    }

    /// A `Retry-After` this build will not read leaves the wait unsaid
    /// rather than guessed.
    ///
    /// HTTP allows an HTTP-date there, and reading one needs a clock
    /// both ends agree on. `None` says "it refused us without saying for
    /// how long", which is true; a number computed from a skewed clock
    /// would not be.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wait_this_build_cannot_read_is_left_unsaid() {
        let (port, _) = spawn_source(vec![
            Reply::status(429).with_header("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT"),
        ])
        .await;

        let events = drain_events(&scanner(port), None).await;
        let err = failures(&events).into_iter().next().expect("one failure");
        assert_eq!(
            err.disposition(),
            Disposition::FailedRetryable { after: None },
            "still a rate limit, still retryable, and honest about the wait: {err}"
        );
    }

    /// What each unsuccessful status costs the run.
    ///
    /// The first time this workspace has had to draw these cuts over
    /// HTTP. `502`/`503`/`504` say the far side is unavailable right
    /// now; a plain `500` says this request broke it, which repeating it
    /// unchanged will do again; a `4xx` is the caller's own URL or
    /// credential.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_status_says_what_the_failure_cost() {
        /// The class a status is expected to land in, named so a
        /// failure message says which rule broke rather than which
        /// variant index did.
        #[derive(Debug, PartialEq, Eq)]
        enum Class {
            Config,
            Source,
            Transient,
        }

        fn class_of(err: &SourceError) -> Class {
            match err {
                SourceError::Config(_) => Class::Config,
                SourceError::Source(_) => Class::Source,
                SourceError::Transient(_) => Class::Transient,
                other => panic!("no status is expected to map here: {other}"),
            }
        }

        let cases = [
            (401, Class::Config, "a credential is settings"),
            (404, Class::Config, "a URL the caller typed is settings"),
            (500, Class::Source, "this request broke it"),
            (502, Class::Transient, "the far side is unavailable now"),
            (503, Class::Transient, "the far side is unavailable now"),
            (504, Class::Transient, "the far side is unavailable now"),
        ];
        for (status, expected, why) in cases {
            let (port, _) = spawn_source(vec![Reply::status(status)]).await;
            let events = drain_events(&scanner(port), None).await;
            let err = failures(&events).into_iter().next().expect("one failure");
            assert_eq!(class_of(&err), expected, "HTTP {status}: {why}, got {err}");
            assert_ne!(
                err.disposition(),
                Disposition::RecordLost,
                "a page that never arrived is not one lost record: {err}"
            );
        }
    }

    /// Nothing came back at all, which the same call in a moment may
    /// survive.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_source_that_cannot_be_reached_is_transient() {
        let scanner = HttpScanner::new(
            "http://127.0.0.1:1/records",
            RecordMap::new(["data", "items"], "id"),
            Cursor::new(["paging", "next"], "cursor"),
        );
        let events = drain_events(&scanner, None).await;
        let err = failures(&events).into_iter().next().expect("one failure");
        assert!(matches!(err, SourceError::Transient(_)), "{err}");
    }

    /// A record with no id costs that record, and the page goes on.
    ///
    /// A third source answering the port's one question for its own
    /// reasons: `FsScanner` takes the next file, `SqliteScanner` cannot
    /// because the failure detaches its cursor, and this one can
    /// because the page is already in hand.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_record_with_no_id_costs_that_record_and_the_page_goes_on() {
        let (port, _) = spawn_source(vec![Reply::ok(
            r#"{"data":{"items":[
                 {"id":"1","body":"a"},
                 {"body":"no id at all"},
                 {"id":"3","body":"c"}
               ]},"paging":{"next":null}}"#,
        )])
        .await;

        let events = drain_events(&scanner(port), None).await;
        assert_eq!(
            locators(&events).len(),
            2,
            "the readable records still arrive"
        );
        let err = failures(&events).into_iter().next().expect("one failure");
        assert_eq!(
            err.disposition(),
            Disposition::RecordLost,
            "one record, not the run: {err}"
        );
    }

    /// A cursor the source sends as a number is followed, not mistaken
    /// for the end of the source.
    ///
    /// The first shape of this read the token with `as_str`, so a
    /// service that numbered its cursors imported its first page and
    /// reported a clean, complete run — silence over a source with more
    /// to give, which is the failure this port exists to rule out.
    /// Nothing in the three-page fixture could have shown it: every
    /// token there is a string.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_numeric_cursor_is_a_cursor() {
        let (port, asked) = spawn_source(vec![
            Reply::ok(r#"{"data":{"items":[{"id":"1"}]},"paging":{"next":2}}"#),
            Reply::ok(r#"{"data":{"items":[{"id":"2"}]},"paging":{"next":null}}"#),
        ])
        .await;

        let events = drain_events(&scanner(port), None).await;
        assert!(failures(&events).is_empty(), "{:?}", failures(&events));
        assert_eq!(locators(&events).len(), 2, "both pages, not just the first");
        assert_eq!(
            asked.cursors(),
            vec![None, Some("2".into())],
            "the number went back on the query string as itself"
        );
    }

    /// A token that is there and cannot be sent ends the run loudly.
    ///
    /// The other half of the same rule. An object at the cursor's path
    /// is a source this build does not know how to follow, and saying
    /// so is the only honest answer — reporting the page as the last
    /// one would be the silence above by another route.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_token_this_build_cannot_send_is_a_failure_and_not_an_end() {
        let (port, _) = spawn_source(vec![Reply::ok(
            r#"{"data":{"items":[{"id":"1"}]},"paging":{"next":{"after":"x"}}}"#,
        )])
        .await;

        let events = drain_events(&scanner(port), None).await;
        assert_eq!(locators(&events).len(), 1, "the page still arrived");
        let err = failures(&events).into_iter().next().expect("one failure");
        assert!(
            matches!(err, SourceError::Source(_)),
            "the far end speaks a dialect this build does not: {err}"
        );
        assert!(
            err.to_string().contains("an object"),
            "and it says what it found: {err}"
        );
    }

    /// A source that keeps handing back the token it was given ends
    /// the run instead of being followed forever.
    ///
    /// The scanner asks twice — once without a cursor, once with the
    /// one it was given — and the second answer repeating that token is
    /// where it stops. Asserted on the request count, because "it
    /// stopped" and "it is still going" look identical from the
    /// summary until one of them finishes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_source_that_repeats_its_cursor_ends_the_run() {
        let (port, asked) = spawn_source(vec![
            Reply::page(&["1"], Some("stuck")),
            Reply::page(&["2"], Some("stuck")),
        ])
        .await;

        let events = drain_events(&scanner(port), None).await;
        let err = failures(&events).into_iter().next().expect("one failure");
        assert!(
            matches!(err, SourceError::Source(_)),
            "the far end is misbehaving, and no setting here fixes it: {err}"
        );
        assert_eq!(
            asked.cursors(),
            vec![None, Some("stuck".into())],
            "two requests and no third"
        );
    }

    /// A source that never reaches an end fails the run rather than
    /// being followed until somebody notices.
    ///
    /// The cycle guard catches only a token handed straight back. This
    /// source alternates two, which defeats it and pages forever — the
    /// shape that cost this branch thirty-seven minutes when a fixture
    /// did it by accident. The ceiling is what bounds it, and reaching
    /// one is a **failure**: a scan that stopped early and reported
    /// success would be the silent loss the whole port is built against.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_source_that_never_ends_hits_the_ceiling_and_says_so() {
        let (port, asked) = spawn_source(vec![
            Reply::page(&["1"], Some("a")),
            Reply::page(&["2"], Some("b")),
            Reply::page(&["3"], Some("a")),
            Reply::page(&["4"], Some("b")),
            Reply::page(&["5"], Some("a")),
        ])
        .await;

        let events = drain_events(&scanner(port).with_max_pages(3), None).await;
        assert_eq!(
            locators(&events).len(),
            3,
            "the pages it did read still arrive"
        );
        let err = failures(&events).into_iter().next().expect("one failure");
        assert!(
            matches!(err, SourceError::Source(_)),
            "a source that will not end is the far side's trouble: {err}"
        );
        assert_eq!(
            asked.cursors().len(),
            3,
            "and it stopped asking rather than running on"
        );
    }

    /// A record's address does not move when the caller moves a query
    /// parameter.
    ///
    /// The identity the server compares is
    /// `(persona_id, source_kind, source_locator)` as text, so a locator
    /// carrying `?since=A` and one carrying `?since=B` would be two rows
    /// for one record — and moving that window between runs is the
    /// ordinary way to use such a flag. The partition keeps the query,
    /// because parameters choose *which* records come back; the locator
    /// does not, because they are not part of *where* one lives.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_records_address_does_not_move_with_a_query_parameter() {
        let (port, _) = spawn_source(vec![Reply::page(&["7"], None)]).await;
        let with_query = HttpScanner::new(
            format!("http://127.0.0.1:{port}/records?since=2026-01-01"),
            RecordMap::new(["data", "items"], "id"),
            Cursor::new(["paging", "next"], "cursor"),
        );

        let events = drain_events(&with_query, None).await;
        assert_eq!(
            locators(&events),
            vec![format!("http://127.0.0.1:{port}/records#7")],
            "the window the caller asked through is not part of the address"
        );
        assert!(
            SourceScanner::partition(&with_query)
                .expect("a URL has a partition")
                .contains("since=2026-01-01"),
            "but it is part of what the position is a position in"
        );
    }

    /// The source answered and this build cannot find the records in
    /// what it said.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_response_with_no_records_where_they_were_promised_is_the_source_s() {
        let (port, _) = spawn_source(vec![Reply::ok(r#"{"unexpected":"shape"}"#)]).await;
        let events = drain_events(&scanner(port), None).await;
        let err = failures(&events).into_iter().next().expect("one failure");
        assert!(matches!(err, SourceError::Source(_)), "{err}");
    }

    /// A state from another URL is refused rather than ignored, and
    /// refused before a request is made.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_state_this_scanner_cannot_use_is_refused() {
        let (port, asked) = spawn_source(vec![Reply::page(&["1"], None)]).await;
        let scanner = scanner(port);

        for state in [
            SyncState::new(
                "kind=http|url=http://elsewhere/records",
                json!({ "after_cursor": "c1" }),
            ),
            SyncState::new(
                SourceScanner::partition(&scanner).expect("a URL has a partition"),
                json!({ "page": 7 }),
            ),
        ] {
            let err = scanner
                .scan(ScanMode::Enumerate, Some(state))
                .await
                .err()
                .expect("the scan does not start");
            assert!(matches!(err, SourceError::Config(_)), "{err}");
        }
        assert!(
            asked.cursors().is_empty(),
            "and nothing was requested while finding that out"
        );
    }

    /// Watch is refused rather than answered with a polling interval
    /// picked here.
    #[tokio::test(flavor = "multi_thread")]
    async fn watch_is_refused_because_the_interval_is_not_this_scanner_s() {
        let (port, _) = spawn_source(vec![Reply::page(&["1"], None)]).await;
        let err = scanner(port)
            .scan(ScanMode::Watch, None)
            .await
            .err()
            .expect("no watch");
        assert!(matches!(err, SourceError::Config(_)), "{err}");
    }

    /// A record out of a JSON array has no bytes at the address it is
    /// given, so no digest declared from here could ever be checked.
    #[test]
    fn a_record_is_not_a_whole_artefact() {
        let scanner = HttpScanner::new(
            "http://example.invalid/records",
            RecordMap::new(["data", "items"], "id"),
            Cursor::new(["paging", "next"], "cursor"),
        );
        assert!(!scanner.payload_is_whole_artefact());
    }
}
