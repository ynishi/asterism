//! `SqliteScanner` — SQLite source scanner.
//!
//! Opens an arbitrary SQLite database read-only, runs a user-supplied
//! `SELECT` statement, and emits every row as a `RawItem`. Column
//! mapping is spelled out by the caller so the same scanner can drive
//! importers over totally unrelated schemas (chat exports, message DBs,
//! bespoke tools' scratch tables, and so on).
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

use super::{RawItem, ScanFuture, ScanMode, SourceScanner};
use crate::port::SourceError;

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
}

impl SqliteScanner {
    /// Builds a scanner around `db_path` / `query` / column mapping.
    pub fn new(db_path: impl Into<PathBuf>, query: impl Into<String>, columns: ColumnMap) -> Self {
        Self {
            db_path: db_path.into(),
            query: query.into(),
            columns,
            source_kind: "sqlite".into(),
        }
    }

    /// Overrides the slug written to `RawItem.source_kind`.
    pub fn with_source_kind(mut self, slug: impl Into<String>) -> Self {
        self.source_kind = slug.into();
        self
    }
}

impl SourceScanner for SqliteScanner {
    fn scan(&self, mode: ScanMode) -> ScanFuture<'_> {
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

            let (tx, rx) = mpsc::channel::<Result<RawItem, SourceError>>(64);
            let db_path = this.db_path.clone();
            let query = this.query.clone();
            let columns = this.columns.clone();
            let source_kind = this.source_kind.clone();

            tokio::task::spawn_blocking(move || {
                run_query(&db_path, &query, &columns, &source_kind, &tx);
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
    tx: &mpsc::Sender<Result<RawItem, SourceError>>,
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

                let item = RawItem {
                    source_kind: source_kind.to_string(),
                    locator: format!("{}#{}", db_path.display(), id_repr),
                    payload,
                    occurred_at,
                    extra: serde_json::Value::Object(extras),
                };
                if tx.blocking_send(Ok(item)).is_err() {
                    return;
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

    /// Runs a scan and returns everything the stream yielded.
    async fn drain(scanner: SqliteScanner) -> Vec<Result<RawItem, SourceError>> {
        match scanner.scan(ScanMode::Enumerate).await {
            Ok(stream) => stream.collect().await,
            Err(err) => vec![Err(err)],
        }
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
            .scan(ScanMode::Enumerate)
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
}
