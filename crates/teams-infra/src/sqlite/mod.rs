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
//! - [`open_existing_at_latest`] — the maintenance verbs' entry point
//!   (#95): pragmas only, **no migration**, refused unless the schema
//!   is already current.
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

/// Opens the teams database **without migrating**, refusing unless its
/// schema is exactly [`LATEST_VERSION`] — the entry point for the
/// maintenance verbs (`teams-server gc` / `backup`, #95).
///
/// A maintenance verb must never change the instance it maintains as a
/// side effect of looking at it: a newer binary asked to *back up* a
/// pre-upgrade database would otherwise migrate it first and archive
/// the migrated schema — the wrong artefact, produced by the command
/// whose whole job was to preserve the instance as it stands. Only the
/// per-connection pragmas are applied here (a persistent-state write
/// like the WAL switch happened at the instance's creation and is a
/// no-op on every database this plane produced); on a version mismatch
/// — older *or* newer — the connection is shut down and the error
/// names the fix.
pub async fn open_existing_at_latest(
    path: impl AsRef<Path>,
) -> Result<(AsyncIsle, AsyncIsleDriver), teams_core::DomainError> {
    let path = path.as_ref();
    let (isle, driver) = AsyncIsle::spawn(path, pragma_only).await.map_err(|e| {
        teams_core::DomainError::Infra(anyhow::anyhow!("cannot open teams db: {e}"))
    })?;
    let version = match schema_version(&isle).await {
        Ok(version) => version,
        Err(e) => {
            driver.shutdown().await.ok();
            return Err(teams_core::DomainError::Infra(anyhow::anyhow!(
                "cannot read schema version: {e}"
            )));
        }
    };
    if version != LATEST_VERSION {
        driver.shutdown().await.ok();
        return Err(teams_core::DomainError::Validation(format!(
            "teams database at {} is at schema v{version}, and this build expects \
             v{LATEST_VERSION}; maintenance commands never migrate as a side effect — \
             run `teams-server serve` or `teams-server init` (of the matching build) to \
             migrate first",
            path.display()
        )));
    }
    Ok((isle, driver))
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
    async fn the_maintenance_open_refuses_a_version_mismatch_and_mutates_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("teams.db");
        let (_isle, driver) = open_and_migrate(&path).await.unwrap();
        driver.shutdown().await.unwrap();

        // The current version opens fine.
        let (isle, driver) = open_existing_at_latest(&path).await.unwrap();
        assert_eq!(schema_version(&isle).await.unwrap(), LATEST_VERSION);
        driver.shutdown().await.unwrap();

        // An instance a version behind — as an older build left it.
        let stale = LATEST_VERSION - 1;
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", stale).unwrap();
        let object_count: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        drop(conn);

        // Refused, the error naming both versions and the fix…
        let refused = open_existing_at_latest(&path).await;
        let message = match refused {
            Err(e) => e.to_string(),
            Ok((_, driver)) => {
                driver.shutdown().await.ok();
                panic!("a stale schema must be refused");
            }
        };
        for expected in [
            &format!("v{stale}"),
            &format!("v{LATEST_VERSION}"),
            &"teams-server init".to_string(),
        ] {
            assert!(message.contains(expected.as_str()), "{expected}: {message}");
        }

        // …and nothing moved: version and schema exactly as found.
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, stale, "the refusal must not migrate");
        let after: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, object_count, "the refusal must not touch the schema");
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
