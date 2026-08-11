//! SQLite backend — connection lifecycle and schema migration built on
//! `rusqlite-isle`.
//!
//! ## Layout
//!
//! - [`migrations`] — `PRAGMA user_version`-based append-only migrations.
//! - [`open`] / [`open_in_memory`] — entry points that return an
//!   `(AsyncIsle, AsyncIsleDriver)` pair with the pragmas set and the
//!   schema migrated.
//!
//! ## Responsibilities
//!
//! Repository adapters (`SqlitePersonaRepository`, and so on) take an
//! `AsyncIsle` handle from this module and issue queries through
//! `isle.call(|conn| ...)`. Connection pragmas and schema initialisation
//! are the sole concern of this module — adapters never reissue them.
//!
//! ## Where migrations run
//!
//! The authoritative entry point for schema migration is the
//! `asterism-server init` / `migrate` CLI subcommand. To keep the
//! first-run experience friendly, [`open_and_migrate`] also applies any
//! pending migrations before returning — the CLI is the source of
//! truth, the in-process call is a convenience for the app / server
//! processes so they never fail to start on a fresh install.
//!
//! ## Pragma choices
//!
//! - `journal_mode = WAL` — the standard 1-writer / N-readers setup. A
//!   reader pool (`IslePool`) can be added later once contention is
//!   measurable.
//! - `synchronous = NORMAL` — the usual tradeoff paired with WAL.
//! - `foreign_keys = ON` — enables cascading deletes at the DB layer.
//!   SQLite defaults to `OFF` per connection, so this must be reissued
//!   inside every init closure.
//! - `busy_timeout = 5000` — Asterism ships two processes that share the
//!   same file (UI + server), so writer contention can happen; waiting for
//!   a few seconds is nicer than an immediate `SQLITE_BUSY`.
//!
//! One non-pragma setting rides along: transactions default to `BEGIN
//! IMMEDIATE`. `busy_timeout` does not protect a DEFERRED transaction
//! that opens with a read and upgrades to a write — SQLite bypasses
//! the busy handler on the upgrade (waiting cannot help, the read
//! snapshot is already stale) and returns `SQLITE_BUSY` on the spot.
//! The apalis job pool writes to the same file every poll tick, so
//! that window is hit for real (snapshot-freeze e2e flake, 2026-07-28
//! / 2026-08-01). Taking the write lock at `BEGIN` keeps every wait
//! on the busy-handler path.

pub mod map;
pub mod migrations;
pub mod repo;

use rusqlite::Connection;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver, IsleError};
use std::path::Path;

pub use migrations::{LATEST_VERSION, migrate};

/// Init closure shared by every connection entry point (pragmas +
/// migration).
fn init_connection(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    pragma_only(conn)?;
    migrate(conn)
}

/// Opens the Asterism database, applying any pending migrations before
/// returning. The returned `AsyncIsleDriver` owns the lifecycle —
/// callers should `driver.shutdown().await` at process exit to drain
/// queued jobs and join the SQLite thread.
///
/// Migration is convenience-first: the CLI is the authoritative
/// entry point, but embedding processes (the Tauri UI, the standalone
/// server's `serve` subcommand) call this helper so they succeed even
/// on a fresh install. Anything that needs stricter control should call
/// [`open_expecting_latest`] instead.
pub async fn open_and_migrate(
    path: impl AsRef<Path>,
) -> Result<(AsyncIsle, AsyncIsleDriver), IsleError> {
    AsyncIsle::spawn(path, init_connection).await
}

/// Opens an in-memory Asterism database with migrations applied (used
/// by tests and preview tooling). WAL is a no-op on memory databases.
pub async fn open_and_migrate_in_memory() -> Result<(AsyncIsle, AsyncIsleDriver), IsleError> {
    AsyncIsle::open_in_memory(init_connection).await
}

/// Opens the database and asserts that it is already at
/// [`LATEST_VERSION`]. Fails otherwise — used by tooling that wants to
/// verify state without silently changing the schema.
pub async fn open_expecting_latest(
    path: impl AsRef<Path>,
) -> Result<(AsyncIsle, AsyncIsleDriver), IsleError> {
    let (isle, driver) = AsyncIsle::spawn(path, pragma_only).await?;
    let version: i64 = isle
        .call(|conn| conn.pragma_query_value(None, "user_version", |row| row.get(0)))
        .await?;
    if version != LATEST_VERSION {
        driver.shutdown().await.ok();
        return Err(IsleError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    Ok((isle, driver))
}

/// Pragma init without migration — used by [`open_expecting_latest`]
/// so the caller can decide what to do with an unexpected version
/// instead of silently upgrading.
fn pragma_only(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    // Not a pragma: `BEGIN IMMEDIATE` by default, so a transaction can
    // never start as a read and hit the busy-handler-bypassing upgrade
    // path (see the module doc).
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
    Ok(())
}

/// Reads the current schema version from an already-open isle.
pub async fn schema_version(isle: &AsyncIsle) -> Result<i64, IsleError> {
    isle.call(|conn| conn.pragma_query_value(None, "user_version", |row| row.get(0)))
        .await
}

/// What `asset.source_locator` / `material.locator` hold, for a fixture
/// that writes a row with `INSERT` instead of through the repository.
///
/// The columns hold the tagged form, and hand-writing that in a fixture
/// would be a second copy of the encoding — one that keeps passing after
/// the real one moves. This goes the way production goes: the caller's
/// spelling through [`SourceLocator::from_wire`], the value through
/// [`SourceLocator::to_storage`].
///
/// A fixture that wants to seed a row in the *pre-tag* spelling (a
/// migration test seeding below V63) writes the bare string and must not
/// call this.
#[cfg(test)]
pub(crate) fn stored_locator(raw: &str) -> String {
    asterism_core::domain::source_locator::SourceLocator::from_wire(raw)
        .expect("a fixture locator")
        .to_storage()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_applies_migrations_to_latest() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let version: i64 = isle
            .call(|conn| conn.pragma_query_value(None, "user_version", |row| row.get(0)))
            .await
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
        driver.shutdown().await.unwrap();
    }

    /// SQLite bypasses the busy handler when a DEFERRED transaction
    /// tries to upgrade from read to write while another connection
    /// holds the write lock — it returns SQLITE_BUSY immediately and
    /// `busy_timeout` never gets a say (the snapshot-freeze e2e flake,
    /// 2026-07-28 / 2026-08-01). So a transaction opened on this
    /// module's connections must take the write lock up front: a write
    /// from another connection, attempted while the transaction is
    /// open but before it has written anything, must already be locked
    /// out instead of sneaking in between the read and the upgrade.
    #[tokio::test]
    async fn a_transaction_takes_the_write_lock_up_front() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("asterism.db");
        let (isle, driver) = open_and_migrate(&path).await.unwrap();
        let contender_path = path.clone();
        let observed = isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let contender = Connection::open(&contender_path)?;
                contender.busy_timeout(std::time::Duration::from_millis(50))?;
                let outcome = contender.execute_batch("CREATE TABLE probe (x INTEGER)");
                tx.rollback()?;
                Ok(outcome.err().and_then(|e| e.sqlite_error_code()))
            })
            .await
            .unwrap();
        assert_eq!(
            observed,
            Some(rusqlite::ErrorCode::DatabaseBusy),
            "an open transaction must already hold the write lock"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn migrate_is_idempotent() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        // Running the migration a second time must succeed with no changes
        // (this is the restart path).
        isle.call(migrate).await.unwrap();
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn foreign_keys_reject_orphan_asset() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let result = isle
            .call(|conn| {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                        modality, occurred_at, created_at, updated_at)
                     VALUES (?1, ?2, 'fs', 'x.md', 'state', 0, 0, 0)",
                    rusqlite::params![
                        uuid::Uuid::now_v7(),
                        // Persona id that does not exist.
                        uuid::Uuid::now_v7()
                    ],
                )
            })
            .await;
        assert!(
            result.is_err(),
            "the FK constraint must reject an orphan asset"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn persona_uuid_blob_roundtrip_and_cascade() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona_id = uuid::Uuid::now_v7();
        let asset_id = uuid::Uuid::now_v7();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at)
                 VALUES (?1, 'alice', 'Alice', 0, 0)",
                rusqlite::params![persona_id],
            )?;
            tx.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', 'notes/state-01.md', 'state', 0, 0, 0)",
                rusqlite::params![asset_id, persona_id],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();

        // Verifies the UUID BLOB round-trip and the cascade delete on the
        // DB layer.
        let (loaded, remaining): (uuid::Uuid, i64) = isle
            .call(move |conn| {
                let loaded = conn.query_row(
                    "SELECT persona_id FROM asset WHERE id = ?1",
                    rusqlite::params![asset_id],
                    |row| row.get(0),
                )?;
                conn.execute(
                    "DELETE FROM persona WHERE id = ?1",
                    rusqlite::params![persona_id],
                )?;
                let remaining =
                    conn.query_row("SELECT count(*) FROM asset", [], |row| row.get(0))?;
                Ok((loaded, remaining))
            })
            .await
            .unwrap();
        assert_eq!(loaded, persona_id);
        assert_eq!(
            remaining, 0,
            "deleting the persona must cascade to its assets"
        );
        driver.shutdown().await.unwrap();
    }
}
