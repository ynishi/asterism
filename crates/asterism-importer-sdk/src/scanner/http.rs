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
//! for remote sources — [`Transient`](crate::SourceError::Transient),
//! [`RateLimited`](crate::SourceError::RateLimited), the `retry_after` a
//! rate limit carries — and until this scanner nothing in the workspace
//! produced one outside a test fixture. A directory cannot be rate
//! limited and a SQLite file is never briefly unreachable. An interface
//! is only as good as the adapter shaped least like the ones it was
//! written against, and this is that adapter.
//!
//! Three things it does differently, each of which the port had never
//! been asked about:
//!
//! - **Its offset cannot be compared.** `FsScanner` compares paths and
//!   `SqliteScanner` compares ids, so both can decide whether a record
//!   falls before a resumption point. A cursor can only be handed back
//!   to the source that issued it. The port's rule that an offset is
//!   opaque and only its writer reads it is what makes that work.
//! - **Its checkpoints are coarse.** The others emit one behind every
//!   record. A page is the smallest thing this can take up after, so
//!   fifty records share one checkpoint.
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
/// `param`. A response with nothing at `path`, or `null` there, is the
/// last page.
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
    /// Field on each record holding its id. Combined with the URL to
    /// form [`RawItem::locator`], which is what the server's unique
    /// index reads for idempotency — so it has to be stable for the
    /// life of the record, and a field the service calls `position` or
    /// `index` is not one.
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
    client: reqwest::Client,
}

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
            client: reqwest::Client::new(),
        }
    }

    /// Adds a header to every request.
    ///
    /// Where a credential goes, and the only place one does: this
    /// scanner has no notion of an auth scheme, so whatever the service
    /// wants is spelled by the caller who knows.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Overrides the slug written to [`RawItem::source_kind`].
    pub fn with_source_kind(mut self, slug: impl Into<String>) -> Self {
        self.source_kind = slug.into();
        self
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
    /// Unlike the other two scanners, nothing is compared: the token is
    /// handed back to the source, which is the only thing that knows
    /// what it means. That is the port's opacity rule doing its job —
    /// and the reason this scanner can resume without reading what it
    /// has already read.
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
        loop {
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
                // go and fix — see the module doc on classification.
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
                    // `SqliteScanner` cannot, because its cursor is
                    // detached by the failure. Three scanners, three
                    // answers about their own sources, which is what
                    // the port says this decision is.
                    Err(err) => {
                        if tx.send(Err(err)).await.is_err() {
                            return;
                        }
                    }
                }
            }

            let next = walk_path(&body, &self.cursor.path).and_then(|v| v.as_str());
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
            if cursor.as_deref() == Some(next) {
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
            let checkpoint = ScanEvent::Checkpoint(SyncState::new(
                partition.clone(),
                json!({ "after_cursor": next }),
            ));
            if tx.send(Ok(checkpoint)).await.is_err() {
                return;
            }
            cursor = Some(next.to_string());
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
                format!("{}#{id}", self.url),
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
            locator: format!("{}#{id}", self.url),
            payload,
            occurred_at,
            extra: json!({ "source_url": self.url }),
        })
    }
}

/// Walks `path` key by key. An empty path is the value itself.
fn walk_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |cursor, key| cursor.get(key))
}

/// An id as text, from the two JSON types an id is.
///
/// A number is rendered rather than refused, because plenty of services
/// number their records and a locator is text either way. A bool, a
/// null, an object or an array is not an id and is not guessed at.
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
    /// socket that can be made to say 429 — which no real service can.
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
    /// The third answer to the port's one question, and a third source
    /// giving it for its own reasons: `FsScanner` takes the next file,
    /// `SqliteScanner` cannot because the failure detaches its cursor,
    /// and this one can because the page is already in hand.
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
