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

use super::{RawItem, ScanError, ScanFuture, ScanMode, SourceScanner};

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
                return Err(ScanError::SourceUnavailable(format!(
                    "db does not exist: {}",
                    this.db_path.display()
                )));
            }
            if mode == ScanMode::Watch {
                // Meaningful watch would require SQL triggers or a
                // polling loop; neither is worth cementing before we
                // have a real consumer. Refuse loudly instead of
                // pretending.
                return Err(ScanError::SourceUnavailable(
                    "SqliteScanner does not support watch mode yet".into(),
                ));
            }

            let (tx, rx) = mpsc::channel::<Result<RawItem, ScanError>>(64);
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
    tx: &mpsc::Sender<Result<RawItem, ScanError>>,
) {
    let conn = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(err) => {
            let _ = tx.blocking_send(Err(ScanError::SourceUnavailable(format!(
                "open failed: {err}"
            ))));
            return;
        }
    };
    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            let _ = tx.blocking_send(Err(ScanError::SourceUnavailable(format!(
                "prepare failed: {err}"
            ))));
            return;
        }
    };
    let names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let id_idx = match column_index(&names, &columns.id) {
        Some(i) => i,
        None => {
            let _ = tx.blocking_send(Err(ScanError::SourceUnavailable(format!(
                "id column {:?} missing from result set",
                columns.id
            ))));
            return;
        }
    };
    let body_idx = match column_index(&names, &columns.body) {
        Some(i) => i,
        None => {
            let _ = tx.blocking_send(Err(ScanError::SourceUnavailable(format!(
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
            let _ = tx.blocking_send(Err(ScanError::SourceUnavailable(format!(
                "query failed: {err}"
            ))));
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
                let _ = tx.blocking_send(Err(ScanError::ItemReadFailed(format!(
                    "row read failed: {err}"
                ))));
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
