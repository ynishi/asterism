//! `SqliteScanner` — SQLite source scanner.
//!
//! Opens an arbitrary SQLite database read-only, runs a user-supplied
//! `SELECT` statement, and emits every row as a `RawItem`. Column
//! mapping is spelled out by the caller so the same scanner can drive
//! importers over totally unrelated schemas (chat exports, message DBs,
//! bespoke tools' scratch tables, and so on).
//!
//! **Resumable only on the caller's word.** The query is the caller's,
//! so only the caller knows whether it has an order to take up inside;
//! [`SqliteScanner::ordered_by_id`] is where they say so. Without it
//! this scanner emits no checkpoints and refuses to resume, rather than
//! resuming inside an order nobody promised.
//!
//! Async is faked at the edge: `rusqlite` is blocking, so the scan
//! actually runs on a dedicated `spawn_blocking` task and pushes rows
//! into a bounded mpsc.
//!
//! This scanner leaves
//! [`payload_is_whole_artefact`](super::SourceScanner::payload_is_whole_artefact)
//! at its `false` default, and the reason is worth stating rather than
//! inheriting: a row's `body` column is a value out of a database, and
//! the `<db>#<id>` address it is given has no bytes of its own for
//! anybody to read back. A digest declared from here could never be
//! checked, which is precisely what the server refuses.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use chrono::{DateTime, Utc};
use futures::stream::StreamExt;
use rusqlite::{Connection, OpenFlags, types::Value};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{RawItem, ScanEvent, ScanFuture, ScanMode, SourceScanner};
use crate::port::{SourceError, SyncState};

/// Column-to-`RawItem` mapping supplied by the importer.
#[derive(Debug, Clone)]
pub struct ColumnMap {
    /// Column used as the row identifier — combined with the DB path to
    /// form `RawItem.locator` (which the server-side unique index relies
    /// on for idempotency).
    pub id: String,
    /// Column read as the item payload (encoded as bytes).
    pub body: String,
    /// Optional column with the item's occurrence time. If the value is
    /// text it is parsed as RFC 3339; if it is an integer it is treated
    /// as unix epoch milliseconds.
    pub timestamp: Option<String>,
}

impl ColumnMap {
    /// Builds a column map with only the required columns configured.
    pub fn new(id: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            body: body.into(),
            timestamp: None,
        }
    }

    /// Sets the optional timestamp column.
    pub fn with_timestamp(mut self, column: impl Into<String>) -> Self {
        self.timestamp = Some(column.into());
        self
    }
}

/// SQLite scanner.
///
/// - `db_path`     — path to the SQLite file (opened read-only).
/// - `query`       — SELECT statement that yields the rows to import.
/// - `columns`     — how to map the returned columns onto `RawItem`.
/// - `source_kind` — slug written to `RawItem.source_kind`
///   (defaults to `"sqlite"`).
#[derive(Debug, Clone)]
pub struct SqliteScanner {
    db_path: PathBuf,
    query: String,
    columns: ColumnMap,
    source_kind: String,
    ordered_by_id: bool,
}

impl SqliteScanner {
    /// Builds a scanner around `db_path` / `query` / column mapping.
    pub fn new(db_path: impl Into<PathBuf>, query: impl Into<String>, columns: ColumnMap) -> Self {
        Self {
            db_path: db_path.into(),
            query: query.into(),
            columns,
            source_kind: "sqlite".into(),
            ordered_by_id: false,
        }
    }

    /// Declares that the query returns rows in ascending order of the
    /// id column, which is what makes this scanner resumable.
    ///
    /// The caller has to say it because only the caller can know it.
    /// Resuming means "take up after row N", and that is only the same
    /// set as "the rows not yet seen" if the order is ascending by id
    /// and stable between runs. A `SELECT` without an `ORDER BY`
    /// promises no order at all, so resuming one would skip rows that
    /// happened to sort before N on the second run and were never
    /// yielded on the first — silently, and with nothing to notice it
    /// by afterwards.
    ///
    /// Nothing here can check the claim. What it can do is refuse to
    /// resume without it, which is what it does: a resumption asked for
    /// and not available is a [`Config`](crate::SourceError::Config)
    /// failure, not a quiet start from the beginning.
    ///
    /// Note what this buys and what it does not. The rows before the
    /// resumption point are still read out of SQLite — the query is the
    /// caller's and is not rewritten — and what is saved is the
    /// parsing, the upload and the server-side work for every record
    /// already handled.
    pub fn ordered_by_id(mut self) -> Self {
        self.ordered_by_id = true;
        self
    }

    /// The resumable unit: this database and this query.
    ///
    /// Both, because a different query over the same file selects a
    /// different set of rows, and a row id from one says nothing about
    /// a position in the other.
    fn partition(&self) -> String {
        format!("db={}|query={}", self.db_path.display(), self.query)
    }

    /// The row id a resumption point says was the last one handled, or
    /// an answer for a caller that has to be told why it cannot resume.
    fn resume_after(&self, state: Option<SyncState>) -> Result<Option<IdKey>, SourceError> {
        let Some(state) = state else {
            return Ok(None);
        };
        if !self.ordered_by_id {
            return Err(SourceError::Config(
                "cannot resume: this scanner has not been told its query orders rows by the \
                 id column, and resuming an unordered query skips rows nobody has seen — \
                 see SqliteScanner::ordered_by_id"
                    .into(),
            ));
        }
        if state.partition != self.partition() {
            return Err(SourceError::Config(
                "cannot resume: the state was written for another database or another query".into(),
            ));
        }
        match state.offset.get("after_id").and_then(IdKey::from_json) {
            Some(id) => Ok(Some(id)),
            None => Err(SourceError::Config(
                "cannot resume: the state carries no `after_id` this scanner can order — \
                 it holds an integer or a string, the two things an id column is"
                    .into(),
            )),
        }
    }

    /// Overrides the slug written to `RawItem.source_kind`.
    pub fn with_source_kind(mut self, slug: impl Into<String>) -> Self {
        self.source_kind = slug.into();
        self
    }
}

impl SourceScanner for SqliteScanner {
    fn scan(&self, mode: ScanMode, resume_from: Option<SyncState>) -> ScanFuture<'_> {
        let this = self.clone();
        Box::pin(async move {
            if !this.db_path.exists() {
                return Err(SourceError::Config(format!(
                    "db does not exist: {}",
                    this.db_path.display()
                )));
            }
            if mode == ScanMode::Watch {
                // Meaningful watch would require SQL triggers or a
                // polling loop; neither is worth cementing before we
                // have a real consumer. Refuse loudly instead of
                // pretending.
                return Err(SourceError::Config(
                    "SqliteScanner does not support watch mode yet".into(),
                ));
            }

            // Refused before the query runs, so a caller that asked to
            // resume and cannot hears it instead of receiving the whole
            // table again.
            let after_id = this.resume_after(resume_from)?;
            // `None` when the caller has not vouched for the order: a
            // scan with no resumable position emits no checkpoints,
            // rather than points it would refuse to honour.
            let partition = this.ordered_by_id.then(|| this.partition());

            let (tx, rx) = mpsc::channel::<Result<ScanEvent, SourceError>>(64);
            let db_path = this.db_path.clone();
            let query = this.query.clone();
            let columns = this.columns.clone();
            let source_kind = this.source_kind.clone();

            tokio::task::spawn_blocking(move || {
                run_query(
                    &db_path,
                    &query,
                    &columns,
                    &source_kind,
                    Resume {
                        partition: partition.as_deref(),
                        after_id: after_id.as_ref(),
                    },
                    &tx,
                );
            });

            Ok(ReceiverStream::new(rx).boxed())
        })
    }
}

fn run_query(
    db_path: &Path,
    query: &str,
    columns: &ColumnMap,
    source_kind: &str,
    resume: Resume<'_>,
    tx: &mpsc::Sender<Result<ScanEvent, SourceError>>,
) {
    let conn = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(err) => {
            // `Config`, beside the `exists` check above it: the file
            // is the one the caller named, and `sqlite3_open_v2` does
            // not read it — what it refuses is the path and the
            // permissions, which are that choice. What is *in* the
            // file is not answered here at all; `prepare` below is
            // where a page is first read, and where a file that is not
            // a database says so.
            let _ = tx.blocking_send(Err(SourceError::Config(format!("open failed: {err}"))));
            return;
        }
    };
    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            // More than one answer arrives here, and the default is
            // the interesting decision.
            //
            // `prepare` is where a page is first read for any query
            // that resolves a table — opening reads none — so
            // everything wrong with the *file* surfaces at this line,
            // alongside everything wrong with the *query*. A file that is not a database, one whose pages
            // are malformed, one the filesystem would not read, a
            // database another process has locked: all of them, and
            // whatever SQLite adds next.
            //
            // So the default is `Source`, and only what this scanner
            // can recognise as the caller's is `Config`. The other way
            // round is what was here first, and it made every code
            // nobody had thought about into a sentence telling an
            // operator to go and fix their settings — which is the one
            // thing `Config` says, and the one thing that must not be
            // said on a guess. A corrupt database reported that way is
            // how the shape was noticed.
            //
            // `SQLITE_ERROR` — `ErrorCode::Unknown`, which is what
            // rusqlite calls it — is the caller's: a statement SQLite
            // will not parse, one naming a table that is not there, one
            // naming a column that is not there. Matched on the code
            // rather than on the message, which is prose SQLite is free
            // to reword.
            //
            // Both rusqlite variants, because it arrives as both and
            // the difference is not about us: a syntax error and an
            // unknown column come back as `SqlInputError`, which
            // carries the offset into the statement, while an unknown
            // table comes back as a plain `SqliteFailure`. Measured,
            // not assumed — and the tests below hold it, because the
            // first version of this matched one variant and quietly
            // filed the other two as the source's fault.
            let caller_wrote_the_query = match &err {
                rusqlite::Error::SqliteFailure(e, _) => e.code == rusqlite::ErrorCode::Unknown,
                rusqlite::Error::SqlInputError { error, .. } => {
                    error.code == rusqlite::ErrorCode::Unknown
                }
                _ => false,
            };
            let failure = if caller_wrote_the_query {
                SourceError::Config(format!("prepare failed: {err}"))
            } else {
                SourceError::Source(format!("prepare failed: {err}"))
            };
            let _ = tx.blocking_send(Err(failure));
            return;
        }
    };
    let names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let id_idx = match column_index(&names, &columns.id) {
        Some(i) => i,
        None => {
            let _ = tx.blocking_send(Err(SourceError::Config(format!(
                "id column {:?} missing from result set",
                columns.id
            ))));
            return;
        }
    };
    let body_idx = match column_index(&names, &columns.body) {
        Some(i) => i,
        None => {
            let _ = tx.blocking_send(Err(SourceError::Config(format!(
                "body column {:?} missing from result set",
                columns.body
            ))));
            return;
        }
    };
    let ts_idx = columns
        .timestamp
        .as_deref()
        .and_then(|c| column_index(&names, c));

    let mut rows = match stmt.query([]) {
        Ok(r) => r,
        Err(err) => {
            let _ = tx.blocking_send(Err(SourceError::Source(format!("query failed: {err}"))));
            return;
        }
    };

    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let id_value: Value = row.get(id_idx).unwrap_or(Value::Null);
                let body_value: Value = row.get(body_idx).unwrap_or(Value::Null);
                let ts_value: Option<Value> = ts_idx.map(|i| row.get(i).unwrap_or(Value::Null));

                let id_repr = value_to_string(&id_value);
                let payload = value_to_bytes(&body_value);
                let occurred_at = ts_value.and_then(|v| parse_timestamp(&v));

                let mut extras = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    if i == id_idx || i == body_idx || Some(i) == ts_idx {
                        continue;
                    }
                    let raw: Value = row.get(i).unwrap_or(Value::Null);
                    extras.insert(name.clone(), value_to_json(&raw));
                }
                extras.insert("__row_id".into(), json!(id_repr.clone()));

                let id_key = IdKey::of(&id_value);

                // Skipped in this process rather than in the query:
                // the statement is the caller's and is not rewritten,
                // so SQLite still reads these rows. What resuming saves
                // is everything after this line.
                if let (Some(after), Some(key)) = (resume.after_id, &id_key)
                    && key.is_at_or_before(after)
                {
                    continue;
                }

                let item = RawItem {
                    source_kind: source_kind.to_string(),
                    locator: format!("{}#{}", db_path.display(), id_repr),
                    payload,
                    occurred_at,
                    extra: serde_json::Value::Object(extras),
                };
                if tx.blocking_send(Ok(ScanEvent::Item(item))).is_err() {
                    return;
                }
                // After the row, because a checkpoint says everything
                // already yielded is dealt with.
                //
                // Only when the caller vouched for the order, which is
                // why the partition arrives as an `Option`. A scanner
                // that emitted points it would later refuse to resume
                // from would hand its caller something to store, print
                // and pass back, and answer it with a `Config` failure.
                //
                // And only for a row whose id is a position at all —
                // see [`IdKey::of`].
                if let (Some(partition), Some(key)) = (resume.partition, &id_key) {
                    let checkpoint = ScanEvent::Checkpoint(SyncState::new(
                        partition.to_string(),
                        json!({ "after_id": key.to_json() }),
                    ));
                    if tx.blocking_send(Ok(checkpoint)).is_err() {
                        return;
                    }
                }
            }
            Ok(None) => break,
            Err(err) => {
                // One row was lost, and this scan is over — which the
                // port allows to be two separate facts, because they
                // are.
                //
                // Over, because `rusqlite` resets the statement when a
                // step fails and detaches it — `Rows::advance` calls
                // `reset` there, with the comment "prevents infinite
                // loop on error" — so every later `next` answers
                // `Ok(None)`. Continuing would not read the remaining
                // rows, it would report the query as finished. Ending
                // says the true thing.
                //
                // The locator is the database rather than the row: the
                // id is read out of the row that just failed to read,
                // so at this point there is nothing more precise to
                // name.
                let _ = tx.blocking_send(Err(SourceError::item(
                    db_path.display().to_string(),
                    format!("row read failed: {err}"),
                )));
                break;
            }
        }
    }
}

fn column_index(columns: &[String], name: &str) -> Option<usize> {
    columns.iter().position(|c| c.eq_ignore_ascii_case(name))
}

/// An id's place in the order the caller vouched for.
///
/// The skip is a comparison, and a comparison is only worth as much as
/// the order it reproduces. The first shape of this compared the id's
/// rendered text, which gets the ordinary case exactly wrong: over an
/// `INTEGER PRIMARY KEY`, `"10" <= "9"` holds, so a scan resumed after
/// row 9 dropped every row from 10 to 89 and reported a clean, empty
/// run. That is the silent skipping [`SqliteScanner::ordered_by_id`]
/// exists to rule out, arriving through the code that implements it.
///
/// So an id is carried as what SQLite stored, and compared the way
/// SQLite orders that type: integers by value, text bytewise — the
/// `BINARY` collation, which is what a column that names no other one
/// gets.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IdKey {
    Int(i64),
    Text(String),
}

impl IdKey {
    /// The key for a row's id, or `None` for a value this scanner will
    /// not order.
    ///
    /// A real, a blob or a NULL in an id column is not a position in an
    /// ascending sequence, and inventing one is how rows go missing. A
    /// row without a key is yielded and earns no checkpoint, so a
    /// resumption takes up before it and reads it again — a duplicate
    /// the server answers from the locator, rather than a loss nobody
    /// sees.
    fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Integer(n) => Some(Self::Int(*n)),
            Value::Text(s) => Some(Self::Text(s.clone())),
            _ => None,
        }
    }

    /// The key as it is written into a [`SyncState`] offset, keeping
    /// the type: `9` and `"9"` are different positions and a state that
    /// blurred them would compare against the wrong order on the way
    /// back in.
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Int(n) => json!(n),
            Self::Text(s) => json!(s),
        }
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::Number(n) => n.as_i64().map(Self::Int),
            serde_json::Value::String(s) => Some(Self::Text(s.clone())),
            _ => None,
        }
    }

    /// Whether this row falls at or before `after`, and has therefore
    /// been handled already.
    ///
    /// Only like with like. An id column holding both integers and text
    /// cannot be ascending in any order a resumption could use, and
    /// guessing which side such a row falls on is the same silent loss
    /// by another route — so it is not handled, which yields the row.
    fn is_at_or_before(&self, after: &Self) -> bool {
        match (self, after) {
            (Self::Int(a), Self::Int(b)) => a <= b,
            (Self::Text(a), Self::Text(b)) => a <= b,
            _ => false,
        }
    }
}

/// What a scan was told about taking up where another stopped.
struct Resume<'a> {
    /// The partition to checkpoint under, or `None` for a scan with no
    /// resumable position.
    partition: Option<&'a str>,
    /// The last id a previous run handled, if this is a resumption.
    after_id: Option<&'a IdKey>,
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Integer(n) => n.to_string(),
        Value::Real(r) => r.to_string(),
        Value::Text(s) => s.clone(),
        Value::Blob(b) => format!("blob:{}", b.len()),
    }
}

fn value_to_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => Vec::new(),
        Value::Integer(n) => n.to_string().into_bytes(),
        Value::Real(r) => r.to_string().into_bytes(),
        Value::Text(s) => s.clone().into_bytes(),
        Value::Blob(b) => b.clone(),
    }
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Integer(n) => json!(n),
        Value::Real(r) => json!(r),
        Value::Text(s) => serde_json::Value::String(s.clone()),
        Value::Blob(b) => json!({ "blob_len": b.len() }),
    }
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Integer(ms) => DateTime::<Utc>::from_timestamp_millis(*ms),
        Value::Real(secs) => DateTime::<Utc>::from_timestamp_millis((secs * 1000.0) as i64),
        Value::Text(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}

// Silences an unused-import warning if `Future` is only referenced
// through the type alias on nightly toolchains.
const _PIN_TYPE_HINT: Option<Pin<Box<dyn Future<Output = ()> + Send>>> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Disposition;
    use futures::StreamExt;

    /// Builds a real SQLite file with one row, and hands back its path.
    fn a_database(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let conn = Connection::open(&path).expect("create");
        conn.execute_batch(
            "CREATE TABLE entries(id INTEGER PRIMARY KEY, body TEXT);
             INSERT INTO entries VALUES(1, 'hello');",
        )
        .expect("seed");
        path
    }

    fn columns() -> ColumnMap {
        ColumnMap::new("id", "body")
    }

    /// Runs a scan and returns everything the stream yielded,
    /// checkpoints included.
    async fn drain_events(
        scanner: SqliteScanner,
        resume_from: Option<SyncState>,
    ) -> Vec<Result<ScanEvent, SourceError>> {
        match scanner.scan(ScanMode::Enumerate, resume_from).await {
            Ok(stream) => stream.collect().await,
            Err(err) => vec![Err(err)],
        }
    }

    /// The same, with the bookkeeping dropped — what a parser would
    /// ever be handed.
    async fn drain(scanner: SqliteScanner) -> Vec<Result<RawItem, SourceError>> {
        drain_events(scanner, None)
            .await
            .into_iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Item(item)) => Some(Ok(item)),
                Ok(ScanEvent::Checkpoint(_)) => None,
                Err(err) => Some(Err(err)),
            })
            .collect()
    }

    /// The locators, in the order the query yielded them.
    fn locators(events: &[Result<ScanEvent, SourceError>]) -> Vec<String> {
        events
            .iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Item(item)) => Some(item.locator.clone()),
                _ => None,
            })
            .collect()
    }

    /// The last checkpoint the scan emitted, which is what a runner
    /// would keep.
    fn last_checkpoint(events: &[Result<ScanEvent, SourceError>]) -> Option<SyncState> {
        events
            .iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Checkpoint(state)) => Some(state.clone()),
                _ => None,
            })
            .next_back()
    }

    /// The classification is only worth having if a scanner can fill it
    /// in, and `prepare` is where that is hardest: everything wrong
    /// with the file and everything wrong with the query surface at the
    /// same call. These are the cases, measured against the library
    /// rather than assumed — and the two `SqlInputError` ones are why
    /// the match reads both rusqlite variants.
    #[tokio::test]
    async fn a_prepare_failure_says_whose_fault_it_was() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = a_database(tmp.path(), "good.sqlite");

        // The caller's: SQLite will not parse it, or it names something
        // that is not there.
        for query in [
            "SELEC 1",
            "SELECT id, body FROM nope",
            "SELECT nope, body FROM entries",
        ] {
            let out = drain(SqliteScanner::new(&db, query, columns())).await;
            let err = out
                .into_iter()
                .next()
                .expect("one failure")
                .expect_err("the query is refused");
            assert_eq!(
                err.disposition(),
                Disposition::Failed,
                "the run is over either way: {err}"
            );
            assert!(
                matches!(err, SourceError::Config(_)),
                "and it is the query, which is the caller's: {err}"
            );
        }

        // The source's: a file that is not a database at all.
        let garbage = tmp.path().join("garbage.sqlite");
        std::fs::write(&garbage, b"not a database, just bytes").expect("write");
        let out = drain(SqliteScanner::new(
            &garbage,
            "SELECT id, body FROM entries",
            columns(),
        ))
        .await;
        let err = out
            .into_iter()
            .next()
            .expect("one failure")
            .expect_err("the file is refused");
        assert!(
            matches!(err, SourceError::Source(_)),
            "a file that is not a database is not the operator's settings: {err}"
        );

        // And the case that showed the classification was the wrong way
        // round: a real database whose pages are ruined. It answers
        // `SQLITE_CORRUPT`, not `SQLITE_NOTADB`, so a rule that listed
        // the codes it knew and defaulted to `Config` told the operator
        // to go and fix their configuration.
        let corrupt = tmp.path().join("corrupt.sqlite");
        let mut bytes = std::fs::read(&db).expect("read");
        for byte in bytes.iter_mut().take(600).skip(100) {
            *byte = 0xFF;
        }
        std::fs::write(&corrupt, &bytes).expect("write");
        let out = drain(SqliteScanner::new(
            &corrupt,
            "SELECT id, body FROM entries",
            columns(),
        ))
        .await;
        let err = out
            .into_iter()
            .next()
            .expect("one failure")
            .expect_err("the database is refused");
        assert!(
            matches!(err, SourceError::Source(_)),
            "a corrupt database is the source's own trouble: {err}"
        );
    }

    /// A path the caller named that is not there is theirs, and the
    /// scan never starts — so the failure comes back from `scan` rather
    /// than on the stream.
    #[tokio::test]
    async fn a_database_that_is_not_there_is_the_callers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = SqliteScanner::new(tmp.path().join("nope.sqlite"), "SELECT 1", columns())
            .scan(ScanMode::Enumerate, None)
            .await
            .err()
            .expect("no file, no scan");
        assert!(matches!(err, SourceError::Config(_)), "{err}");
    }

    /// The rows come through, and the locator addresses the row rather
    /// than the database — which is what makes a re-scan recognise it.
    #[tokio::test]
    async fn rows_arrive_addressed_by_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = a_database(tmp.path(), "good.sqlite");
        let out = drain(SqliteScanner::new(
            &db,
            "SELECT id, body FROM entries",
            columns(),
        ))
        .await;
        assert_eq!(out.len(), 1);
        let item = out.into_iter().next().unwrap().expect("a row");
        assert_eq!(item.payload, b"hello");
        assert_eq!(item.locator, format!("{}#1", db.display()));
        assert_eq!(item.source_kind, "sqlite");
    }

    /// The same property as the filesystem's, over the other source: a
    /// resumed query yields the rows after the checkpoint and not the
    /// table again.
    #[tokio::test]
    async fn a_resumed_query_yields_only_the_rows_after_the_checkpoint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("many.sqlite");
        let conn = Connection::open(&path).expect("create");
        conn.execute_batch(
            "CREATE TABLE entries(id INTEGER PRIMARY KEY, body TEXT);
             INSERT INTO entries VALUES(1, 'one'), (2, 'two'), (3, 'three');",
        )
        .expect("seed");
        drop(conn);

        let scanner = || {
            SqliteScanner::new(&path, "SELECT id, body FROM entries ORDER BY id", columns())
                .ordered_by_id()
        };
        let at = |id: u32| format!("{}#{id}", path.display());

        let first = drain_events(scanner(), None).await;
        assert_eq!(locators(&first), vec![at(1), at(2), at(3)]);

        let after_first = match &first[1] {
            Ok(ScanEvent::Checkpoint(state)) => state.clone(),
            other => panic!("a checkpoint follows each row, found {other:?}"),
        };

        let second = drain_events(scanner(), Some(after_first)).await;
        assert_eq!(
            locators(&second),
            vec![at(2), at(3)],
            "the row already handled does not come back"
        );
        assert_eq!(
            last_checkpoint(&second).map(|state| state.offset),
            Some(json!({ "after_id": 3 })),
        );
    }

    /// Ten rows is where a text comparison of the ids goes wrong, and
    /// three cannot show it: `"10" <= "9"` holds, so a scan resumed
    /// after row 9 dropped every row from 10 on and reported a clean,
    /// empty run — the silent skipping `ordered_by_id` exists to rule
    /// out, arriving through the code that implements it.
    #[tokio::test]
    async fn integer_ids_are_ordered_as_numbers_and_not_as_text() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("many.sqlite");
        let conn = Connection::open(&path).expect("create");
        conn.execute_batch("CREATE TABLE entries(id INTEGER PRIMARY KEY, body TEXT);")
            .expect("schema");
        for id in 1..=12 {
            conn.execute(
                "INSERT INTO entries VALUES(?1, ?2)",
                rusqlite::params![id, format!("row {id}")],
            )
            .expect("seed");
        }
        drop(conn);

        let scanner = || {
            SqliteScanner::new(&path, "SELECT id, body FROM entries ORDER BY id", columns())
                .ordered_by_id()
        };
        let at = |id: u32| format!("{}#{id}", path.display());

        let first = drain_events(scanner(), None).await;
        // The checkpoint behind row 9: two events per row, so the row
        // is at index 16 and its checkpoint at 17.
        let after_nine = match &first[17] {
            Ok(ScanEvent::Checkpoint(state)) => state.clone(),
            other => panic!("a checkpoint follows each row, found {other:?}"),
        };
        assert_eq!(
            after_nine.offset,
            json!({ "after_id": 9 }),
            "and the id keeps its type on the way out: 9, not \"9\""
        );

        let second = drain_events(scanner(), Some(after_nine)).await;
        assert_eq!(
            locators(&second),
            vec![at(10), at(11), at(12)],
            "the rows after 9 are 10, 11 and 12 — not none of them"
        );
    }

    /// Resuming a query nobody has promised is ordered would skip rows
    /// that sorted before the checkpoint on this run and were never
    /// yielded on the last — silently. So it is refused, and refused
    /// before the query runs.
    #[tokio::test]
    async fn resuming_is_refused_unless_the_caller_vouched_for_the_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = a_database(tmp.path(), "good.sqlite");
        let query = "SELECT id, body FROM entries";
        let partition = format!("db={}|query={query}", db.display());

        // And without the promise there is nothing to offer in the
        // first place: a scan that would refuse to resume does not hand
        // out points to resume from.
        let events = drain_events(SqliteScanner::new(&db, query, columns()), None).await;
        assert_eq!(locators(&events).len(), 1, "the row still arrives");
        assert_eq!(
            last_checkpoint(&events),
            None,
            "and nothing that looks like a position it would honour"
        );

        let err = SqliteScanner::new(&db, query, columns())
            .scan(
                ScanMode::Enumerate,
                Some(SyncState::new(partition, json!({ "after_id": "1" }))),
            )
            .await
            .err()
            .expect("no promise, no resumption");
        assert!(matches!(err, SourceError::Config(_)), "{err}");

        // And with the promise, a state written for another query is
        // still refused: a row id means nothing outside the query that
        // produced it.
        let elsewhere = SyncState::new(
            "db=/other.sqlite|query=SELECT 1",
            json!({ "after_id": "1" }),
        );
        let err = SqliteScanner::new(&db, query, columns())
            .ordered_by_id()
            .scan(ScanMode::Enumerate, Some(elsewhere))
            .await
            .err()
            .expect("another query's position is not this one's");
        assert!(matches!(err, SourceError::Config(_)), "{err}");
    }
}
