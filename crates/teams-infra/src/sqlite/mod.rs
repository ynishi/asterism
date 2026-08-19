//! SQLite backend for the teams plane — connection lifecycle and schema
//! migration built on `rusqlite-isle`, mirroring `asterism-infra`'s
//! conventions over the teams-owned database.
//!
//! ## Layout
//!
//! - [`migrations`] — `PRAGMA user_version`-based append-only
//!   migrations, a fresh series starting at V1 (this database shares
//!   nothing with the app database — #83 §4).
//! - [`repo`] — the repository over the state tables and the ledger.
//! - [`open_and_migrate`] / [`open_and_migrate_in_memory`] — entry
//!   points that return an `(AsyncIsle, AsyncIsleDriver)` pair with the
//!   pragmas set and the schema migrated.
//!
//! ## Pragma choices
//!
//! Same set as the app database, for the same reasons:
//!
//! - `journal_mode = WAL` — the 1-writer / N-readers setup #83 §4
//!   names as the workload fit (append-only ledger, short tx).
//! - `synchronous = NORMAL` — the usual tradeoff paired with WAL.
//! - `foreign_keys = ON` — SQLite defaults to `OFF` per connection, so
//!   it is reissued inside every init closure.
//! - `busy_timeout = 5000` — one process by deployment shape, but a
//!   backup command will eventually share the file, and waiting beats
//!   an immediate `SQLITE_BUSY`.
//!
//! One non-pragma setting rides along: transactions default to `BEGIN
//! IMMEDIATE`. Every repository write opens a transaction that will
//! write, and a DEFERRED transaction that upgrades from read to write
//! bypasses the busy handler on the upgrade (the same trap
//! `asterism-infra` documents from its 2026-07/08 flakes) — taking the
//! write lock at `BEGIN` keeps every wait on the busy-handler path.

pub mod map;
pub mod migrations;
pub mod repo;

use rusqlite::Connection;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver, IsleError};
use std::path::Path;

pub use migrations::{LATEST_VERSION, migrate};
pub use repo::SqliteTeamsRepository;

/// Init closure shared by every connection entry point (pragmas +
/// migration).
fn init_connection(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    pragma_only(conn)?;
    migrate(conn)
}

/// Opens the teams database, applying any pending migrations before
/// returning. The returned `AsyncIsleDriver` owns the lifecycle —
/// callers should `driver.shutdown().await` at process exit to drain
/// queued jobs and join the SQLite thread.
///
/// The default path is [`crate::paths::default_db_path`]; taking the
/// path as a parameter keeps this function free of environment reads,
/// the same split the paths module uses internally.
pub async fn open_and_migrate(
    path: impl AsRef<Path>,
) -> Result<(AsyncIsle, AsyncIsleDriver), IsleError> {
    AsyncIsle::spawn(path, init_connection).await
}

/// Opens an in-memory teams database with migrations applied (used by
/// tests). WAL is a no-op on memory databases.
pub async fn open_and_migrate_in_memory() -> Result<(AsyncIsle, AsyncIsleDriver), IsleError> {
    AsyncIsle::open_in_memory(init_connection).await
}

fn pragma_only(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
    Ok(())
}

/// Reads the current schema version from an already-open isle.
pub async fn schema_version(isle: &AsyncIsle) -> Result<i64, IsleError> {
    isle.call(|conn| conn.pragma_query_value(None, "user_version", |row| row.get(0)))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_applies_migrations_to_latest() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        assert_eq!(schema_version(&isle).await.unwrap(), LATEST_VERSION);
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn migrate_is_idempotent() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        // Running the migration a second time must succeed with no
        // changes (this is the restart path).
        isle.call(migrate).await.unwrap();
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_file_backed_open_runs_in_wal_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("teams.db");
        let (isle, driver) = open_and_migrate(&path).await.unwrap();
        let mode: String = isle
            .call(|conn| conn.pragma_query_value(None, "journal_mode", |row| row.get(0)))
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        driver.shutdown().await.unwrap();
    }
}
