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

/// Version 1 → 2: auth v0 (#83 §5, the #91 slice) — instance-local
/// credentials and DB-backed opaque sessions.
///
/// - **`user_account` holds credentials, not identity semantics.** The
///   domain's `User` stays credential-free behind `port::auth`; this
///   table is where the v0 password adapter keeps the argon2id PHC
///   string. `is_operator` marks the env/CLI bootstrap identity
///   (#83 §1 InstanceOperator) — a flag on the *account*, deliberately
///   nowhere near `membership`: the operator lives outside the
///   membership table, and owning a team is an explicit membership row
///   like anyone else's.
/// - **`auth_session.token_hash` is the SHA-256 of the opaque token**,
///   never the token itself, so the database never contains a usable
///   bearer credential. Expiry is `expires_at` epoch ms; resolve-time
///   rejection deletes the row and `cleanup_expired` sweeps in bulk —
///   the index on `expires_at` is what the sweep walks.
const V2_AUTH_TABLES: &str = r#"
CREATE TABLE user_account (
    user_id       BLOB PRIMARY KEY,
    login         TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    is_operator   INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_user_account_login ON user_account(login);

CREATE TABLE auth_session (
    token_hash TEXT NOT NULL,
    user_id    BLOB NOT NULL REFERENCES user_account(user_id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (token_hash)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_auth_session_user    ON auth_session(user_id);
CREATE INDEX idx_auth_session_expires ON auth_session(expires_at);
"#;

/// Version 2 → 3: the purge mark (#83 §3 lifecycle, the #95 slice).
///
/// `purge_marked_at` lands on **`team_blob_link`** — a state table, the
/// SoT — and nowhere near the ledger: `NULL` is a live link, a
/// timestamp is a link marked for purge at that instant, hidden from
/// normal reads and restorable (unmark) until reclaim removes the row
/// outright once the grace window elapses. This is deliberately *not*
/// a soft delete on any ledger table (those stay append-only, trigger-
/// guarded, exactly as V1 built them); the mark/unmark/reclaim history
/// lives in the ledger as first-class events, and this column only
/// answers the state question "is this link visible right now".
///
/// The partial index serves the two hot lookups the mark adds: a
/// team's marked set (reclaim, and the marked-links read) and "is
/// anything marked" — both filter on `purge_marked_at IS NOT NULL`,
/// which is expected to be a tiny minority of rows.
const V3_PURGE_MARK: &str = r#"
ALTER TABLE team_blob_link ADD COLUMN purge_marked_at INTEGER;

CREATE INDEX idx_team_blob_link_marked
    ON team_blob_link(team_id, purge_marked_at)
    WHERE purge_marked_at IS NOT NULL;
"#;

/// Version 3 → 4: the model registry (#126, the first serving step) —
/// the instance's carriage of the provider-authored entry.
///
/// - **Instance scope, so no `team_id`** — the one-active-model rule
///   is per instance (#126 decision 1), and this is the first state
///   table keyed to neither a team nor a user. Instance scope puts it
///   outside the ledger's reach (#83 §2: the ledger's streams are
///   per-team);
///   publish/supersede history lives in this table's own rows instead,
///   which is a deliberate deferral of instance-scope audit, not a
///   drift into it.
/// - **`entry` is the provider's bytes verbatim** (#126 decision 2 —
///   the instance is a carrier); `model_id` is lifted by the domain's
///   envelope validation purely so history is readable by model.
/// - **At most one live row**, enforced by a unique index over a
///   constant expression filtered to `superseded_at IS NULL` — the
///   partial-index shape V3 established, made unique. (An index on the
///   column itself would not do it: SQLite treats NULLs as distinct in
///   unique indexes.) Publishing supersedes in the same transaction,
///   so the constraint is belt and braces, never the mechanism.
/// - **Superseded rows are kept**, `superseded_at` stamped — the
///   rollback question #126 leaves open stays answerable; how long to
///   keep them is decided when someone needs to trim, not silently
///   here.
const V4_MODEL_REGISTRY: &str = r#"
CREATE TABLE model_registry_entry (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id      TEXT    NOT NULL,
    entry         TEXT    NOT NULL,
    published_at  INTEGER NOT NULL,
    superseded_at INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_model_registry_one_live
    ON model_registry_entry((1))
    WHERE superseded_at IS NULL;
"#;

/// Version 4 → 5: the registry carries the trained head (#132 phase
/// 3), not a model entry.
///
/// The model-entry schema V4 carried lost its only consumer when the
/// fetch flow retired; the head artifact — kilobytes of JSON — takes
/// the same seat under the same rules (opaque bytes, one live row,
/// superseded history kept). The rename is honesty, not mechanics:
/// `label` is what supersession is keyed by now, and a table named
/// for model entries would invite the next reader to store one.
/// Existing rows are deleted rather than carried: they hold
/// model-entry JSON nothing can consume any more, and the new
/// envelope's read would refuse them anyway — better an empty
/// registry than one that errors on its first GET. The unique
/// expression index is re-created under the new name (an index does
/// not follow a table rename by name, only by attachment).
const V5_HEAD_REGISTRY: &str = r#"
DELETE FROM model_registry_entry;

DROP INDEX idx_model_registry_one_live;

ALTER TABLE model_registry_entry RENAME TO head_registry_entry;

ALTER TABLE head_registry_entry RENAME COLUMN model_id TO label;

CREATE UNIQUE INDEX idx_head_registry_one_live
    ON head_registry_entry((1))
    WHERE superseded_at IS NULL;
"#;

/// Migrations in application order. **Append only** — never rewrite an
/// existing batch.
const MIGRATIONS: &[&str] = &[
    V1_INITIAL_SCHEMA,
    V2_AUTH_TABLES,
    V3_PURGE_MARK,
    V4_MODEL_REGISTRY,
    V5_HEAD_REGISTRY,
];

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
    fn v3_adds_the_purge_mark_to_the_link_table_only() {
        let conn = migrated();

        // The mark column exists on the state table…
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'team_blob_link'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            ddl.contains("purge_marked_at"),
            "team_blob_link must carry the purge mark: {ddl}"
        );

        // …and on nothing else: the mark is state, and in particular
        // it never reaches the ledger (no soft delete there — #95).
        let elsewhere: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE sql LIKE '%purge_marked_at%'
                   AND name NOT LIKE '%team_blob_link%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(elsewhere, 0, "the mark belongs to team_blob_link alone");
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
