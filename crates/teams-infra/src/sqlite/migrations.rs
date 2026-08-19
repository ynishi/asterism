//! Teams-database schema migrations — `PRAGMA user_version` scheme.
//!
//! A fresh series starting at V1: this database shares nothing with the
//! app database, not even a version counter (#83 §4). The mechanism is
//! `asterism-infra`'s: `MIGRATIONS[i]` upgrades from version `i` to
//! `i + 1`, [`migrate`] applies every pending batch inside its own
//! transaction and bumps `user_version` on success. **Never rewrite a
//! past batch** — schema changes go at the end.
//!
//! ## Schema decisions
//!
//! - **Ids are 16-byte BLOBs (UUID v7)**, timestamps are `INTEGER`
//!   epoch ms, tables are `STRICT` — the workspace conventions.
//! - **State tables are the SoT, the ledger is the record** (#83 §2
//!   audit-log pattern). `team` / `membership` / `team_blob_link` /
//!   `locator` are authoritative; `ledger_event` is what happened.
//! - **`ledger_event` is append-only in the schema, not only in the
//!   API**: no `updated_at`, no soft-delete column anywhere near it,
//!   and `BEFORE UPDATE` / `BEFORE DELETE` triggers that abort — the
//!   repository exposes no update/delete path, and the schema backs
//!   that up against raw SQL too.
//! - **`ledger_event` carries no foreign key to `team`.** The record
//!   outlives the state on purpose: deleting a team removes its rows
//!   (memberships and links cascade) while the same transaction
//!   appends `teams.team.deleted/1` — a stream that must survive the
//!   row it chronicles cannot reference it.
//! - **Subjects land in `ledger_subject`, keyed `(ref_type,
//!   ref_value)`** with an index in that order, so trace queries walk
//!   the index and never parse payload JSON (#83 §2). The table is the
//!   only store of an event's subjects — rebuilding the envelope joins
//!   it, so there is no second copy to drift.
//! - **`seq` is assigned by storage inside the write transaction**
//!   (`MAX(seq) + 1` per team): monotonic by the primary key, gapless
//!   under the single-writer deployment shape because no two
//!   transactions compute it concurrently.
//! - **`role` is TEXT with no CHECK constraint**: the word list is the
//!   domain's ([`teams_core::domain::identity::Role::parse`]), so a
//!   later tier is a new word and a new match arm, not a schema
//!   migration (#83 §1). The repository passes every stored value back
//!   through the domain parser on read.

use rusqlite::Connection;

/// Version 0 → 1: the whole #89 slice — state tables, the ledger, and
/// the subjects index.
const V1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE team (
    id         BLOB PRIMARY KEY,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE membership (
    team_id BLOB NOT NULL REFERENCES team(id) ON DELETE CASCADE,
    user_id BLOB NOT NULL,
    role    TEXT NOT NULL,
    PRIMARY KEY (team_id, user_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_membership_user ON membership(user_id);

CREATE TABLE team_blob_link (
    team_id    BLOB NOT NULL REFERENCES team(id) ON DELETE CASCADE,
    digest     TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (team_id, digest)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_team_blob_link_digest ON team_blob_link(digest);

CREATE TABLE locator (
    user_id     BLOB NOT NULL,
    uri         TEXT NOT NULL,
    digest_hint TEXT,
    seen_at     INTEGER NOT NULL,
    PRIMARY KEY (user_id, uri)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_locator_digest_hint
    ON locator(digest_hint) WHERE digest_hint IS NOT NULL;

CREATE TABLE ledger_event (
    team_id     BLOB    NOT NULL,
    seq         INTEGER NOT NULL,
    event_id    BLOB    NOT NULL,
    actor       TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    kind        TEXT    NOT NULL,
    payload     TEXT    NOT NULL,
    PRIMARY KEY (team_id, seq)
) STRICT, WITHOUT ROWID;

CREATE UNIQUE INDEX idx_ledger_event_id ON ledger_event(event_id);

CREATE TRIGGER ledger_event_no_update
BEFORE UPDATE ON ledger_event
BEGIN
    SELECT RAISE(ABORT, 'ledger_event is append-only');
END;

CREATE TRIGGER ledger_event_no_delete
BEFORE DELETE ON ledger_event
BEGIN
    SELECT RAISE(ABORT, 'ledger_event is append-only');
END;

CREATE TABLE ledger_subject (
    team_id   BLOB    NOT NULL,
    seq       INTEGER NOT NULL,
    ref_type  TEXT    NOT NULL,
    ref_value TEXT    NOT NULL,
    FOREIGN KEY (team_id, seq) REFERENCES ledger_event(team_id, seq)
) STRICT;

CREATE INDEX idx_ledger_subject_ref ON ledger_subject(ref_type, ref_value);

CREATE TRIGGER ledger_subject_no_update
BEFORE UPDATE ON ledger_subject
BEGIN
    SELECT RAISE(ABORT, 'ledger_subject is append-only');
END;

CREATE TRIGGER ledger_subject_no_delete
BEFORE DELETE ON ledger_subject
BEGIN
    SELECT RAISE(ABORT, 'ledger_subject is append-only');
END;
"#;

/// Migrations in application order. **Append only** — never rewrite an
/// existing batch.
const MIGRATIONS: &[&str] = &[V1_INITIAL_SCHEMA];

/// Latest schema version (`MIGRATIONS.len()`).
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

/// Applies every pending migration up to the latest version.
/// Idempotent: re-running against an already-up-to-date database is a
/// no-op. Each batch runs inside its own transaction; a failure rolls
/// back only that batch and leaves earlier migrations in place.
pub fn migrate(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, batch) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(batch)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrated() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn the_series_starts_fresh_at_v1() {
        let conn = migrated();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);

        // Nothing of the app database's schema exists here — the
        // series shares nothing, starting with the tables.
        let app_tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN ('persona', 'asset', 'instance')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(app_tables, 0);
    }

    #[test]
    fn the_ledger_schema_carries_no_updated_at_and_no_soft_delete() {
        let conn = migrated();
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'ledger_event'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for forbidden in ["updated_at", "deleted", "trashed"] {
            assert!(
                !ddl.contains(forbidden),
                "ledger_event must not carry a {forbidden:?} column: {ddl}"
            );
        }
    }
}
