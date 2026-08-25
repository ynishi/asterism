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
//!   that up against raw SQL too. What this costs when somebody asks
//!   to be erased, and the three records that have to answer together,
//!   is worked out in [`teams_core::domain::ledger`] rather than
//!   restated here.
//! - **The ledger keeps everything, and v0 says so on purpose.** No
//!   pruning, no archival tier, no window — a team's stream holds
//!   every event it ever appended. That was true before it was
//!   decided, by omission; stating it makes it a position with a cost
//!   somebody can weigh. The cost is not the disk the rows take, which
//!   is small: it is that `teams-server backup` snapshots the database
//!   with `VACUUM INTO`, which writes the whole file every time, so
//!   the ledger's growth is paid again on every backup rather than
//!   once at write. An instance that backs up nightly pays for its
//!   entire history nightly. Trimming is a decision for whoever meets
//!   that bill, and it needs this schema's append-only triggers
//!   answered for first — there is no `DELETE` path here to reach for.
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
///   string. The flag this batch names `is_operator` — [`V6`] renames
///   it to `is_admin` — marks the env/CLI bootstrap identity (#83 §1,
///   [`InstanceAdmin`](teams_core::domain::identity::InstanceAdmin)):
///   a flag on the *account*, deliberately nowhere near `membership`,
///   because an admin lives outside the membership table and owning a
///   team is an explicit membership row like anyone else's.
///
/// [`V6`]: V6_ADMIN_RENAME
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

/// Version 5 → 6: the instance capacity is called admin (#148
/// revisions 7 and 8).
///
/// A column rename and nothing else — the flag means exactly what it
/// meant, and every account keeps the value it had. What the rename is
/// for is that "operator" was carrying several meanings at once: the
/// capacity a person holds over an instance, the agent that carried a
/// write out (which the attribution triple already spells that way),
/// and the human who runs the deployment — which is the sense this
/// crate's own prose keeps for it. The capacity is the one that moved,
/// being the one with a single writer and no wire format pinning it.
///
/// **The ledger is not renamed with it.** Rows written before this
/// batch carry `"operator"` inside their `actor` JSON, and
/// `ledger_event` is append-only in the schema — the `BEFORE UPDATE`
/// trigger V1 installed aborts any statement that would restate them,
/// and a batch determined to rewrite them would have to drop that
/// trigger first, which is a decision rather than an oversight. So the
/// accommodation is on the read side and permanent: the domain's
/// `LedgerActor::Admin` carries `#[serde(alias = "operator")]` for
/// good, writes emit `admin`, and no batch after this one may assume
/// the old tag is gone.
const V6_ADMIN_RENAME: &str = r#"
ALTER TABLE user_account RENAME COLUMN is_operator TO is_admin;
"#;

/// Version 6 → 7: the team hosts a forge (#148 decision 20).
///
/// The local plane's forge tables, replicated here under the same
/// names — `line`, `change_point`, `change_row`, `pursuit`,
/// `pursuit_node`, `pursuit_op`, `forge_actor` and the three thread
/// tables — plus `team_asset`, which is this plane's own. Separate
/// database files, so the shared names clash with nothing and the
/// adapter differs from `asterism-infra`'s by as little as it can.
///
/// **Staying literally parallel is the mitigation, not an accident.**
/// More than one set of adapters tracks this model from here on, and
/// what makes a drift between them visible is that the schemas can be
/// diffed: a column that moved on one plane and not the other shows up
/// as a difference between this batch and the local plane's forge
/// batches — `V96`, `V97`, `V98`, `V102` and `V103` — rather than as
/// behaviour somebody notices later. So the shapes are copied down to
/// the `CHECK` constraints, and every deliberate difference is one of
/// the six below.
///
/// **1. `team_id` on every table, and it carries no key.** The scope
/// the local plane does not have. The column is redundant against the
/// ids alone — a `line_id` is a v7 uuid and identifies its line across
/// every team — and it is here anyway, because scoping a read to a
/// team is then a predicate on the table being read rather than a join
/// back to `line` through whichever chain of parents that table
/// happens to hang off.
///
/// A reference to `team(id)` is what it looks like it wants, and it
/// would decide a question this batch has no business deciding.
/// `CASCADE` would make deleting a team delete its forge — except that
/// it could not, because every key *inside* the forge is `RESTRICT`
/// and a cascade into `line` is refused by the change points on it. So
/// the key would either fail a team deletion that works today, or —
/// written the other way — quietly destroy a line's whole history as a
/// side effect of a membership gesture. What actually releases a
/// line's contents is `Lines::discard`, which is a forge verb with
/// rules of its own, and wiring a team deletion to it is a decision
/// with an owner and a place, neither of which is a foreign key
/// declaration. **Deleting a team leaves its forge rows behind, and
/// this paragraph is where that is written down.** #151 and #152 were
/// expected to settle it, on the reasoning that deciding what a team's
/// departure does to its work needs the surface that asks the
/// question; both landed and neither did, so the decision is still
/// unowned. `asset_projection` (V9) carries a `team_id` for the same
/// reason and is in the same position: a sweep is expressible, and
/// nothing calls one.
///
/// **2. `UNIQUE (team_id, name)` on `line`.** The forge's `Name`
/// deliberately leaves name uniqueness to whoever owns the namespace
/// (#148 decision 1), and on this plane that is the team. The local
/// plane answers the same question with silence, which is the right
/// answer for a namespace with one person in it.
///
/// **3. `change_row.content` and `pursuit_op.content` point at
/// `team_asset`.** The one downward foreign key the forge holds, aimed
/// at this plane's own surrogate. `RESTRICT` for the local plane's
/// reason: a line says what is on it *now*, so a line naming bytes
/// somebody deleted is a line lying about the present.
///
/// **4. `team_asset` is identity and nothing else.** `id`, its team,
/// and when it was minted (#148 decisions 3 and 7). What composes it
/// against the CAS is conversion, which arrives with the content verb;
/// what is here is the identity `Store::exists` answers about, which
/// is all the forge ever asks of the layer below it. (The verb landed
/// in #151 and [`V8_TEAM_ASSET_CONTENT`] added the two columns that
/// carry the conversion — this paragraph describes the table as this
/// batch built it, which is what a migration doc is for.)
///
/// **5. `forge_actor` is keyed within a team** — `UNIQUE (team_id,
/// stands_for, COALESCE(subject, ''))`, over the coalesce for the
/// local plane's reason (SQLite counts NULLs as distinct, so a plain
/// index would admit a second owner). `display_name` is the write-time
/// snapshot the local plane gained in `V103`, present from the start
/// here.
///
/// **6. The three composite-key tables are `WITHOUT ROWID`.** This
/// plane's house style, and the tables it applies to are exactly the
/// ones whose primary key is not a single BLOB. It changes no
/// semantics — which is why it is safe to differ on.
///
/// `change_point` is written in the shape the local plane arrived at
/// in `V102` rather than the one `V96` built and `V102` rebuilt: a new
/// database has no rows to carry across, so the keys go in at
/// `CREATE`. `parent_id` stays bare there and on `pursuit.base_id` and
/// `pursuit_node.parent_id`, for the reason `V102` gives — a parent is
/// either the genesis or a change point, and the genesis is columns on
/// `line` rather than a row.
const V7_FORGE_TABLES: &str = r#"
CREATE TABLE team_asset (
    id         BLOB PRIMARY KEY,
    team_id    BLOB NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_team_asset_team ON team_asset(team_id);

CREATE TABLE line (
    id           BLOB PRIMARY KEY,
    team_id      BLOB NOT NULL,
    name         TEXT NOT NULL,
    strategy     TEXT NOT NULL,
    standing     TEXT NOT NULL
        CHECK (standing IN ('open', 'archived')),
    genesis_id   BLOB NOT NULL,
    genesis_at   INTEGER NOT NULL,
    genesis_by   BLOB NOT NULL,
    genesis_kind TEXT NOT NULL
        CHECK (genesis_kind IN ('user', 'system')),
    created_at   INTEGER NOT NULL,
    created_by   BLOB NOT NULL,
    created_kind TEXT NOT NULL
        CHECK (created_kind IN ('user', 'system')),
    updated_at   INTEGER NOT NULL,
    updated_by   BLOB NOT NULL,
    updated_kind TEXT NOT NULL
        CHECK (updated_kind IN ('user', 'system'))
) STRICT;

CREATE UNIQUE INDEX idx_line_team_name ON line(team_id, name);
CREATE INDEX idx_line_team ON line(team_id);

CREATE TABLE pursuit (
    id           BLOB PRIMARY KEY,
    team_id      BLOB NOT NULL,
    line_id      BLOB NOT NULL REFERENCES line(id) ON DELETE RESTRICT,
    parent_id    BLOB REFERENCES pursuit(id) ON DELETE RESTRICT,
    open_node    BLOB NOT NULL,
    base_id      BLOB NOT NULL,
    title        TEXT,
    note         TEXT,
    open_at      INTEGER NOT NULL,
    open_by      BLOB NOT NULL,
    open_kind    TEXT NOT NULL
        CHECK (open_kind IN ('user', 'system')),
    created_at   INTEGER NOT NULL,
    created_by   BLOB NOT NULL,
    created_kind TEXT NOT NULL
        CHECK (created_kind IN ('user', 'system')),
    updated_at   INTEGER NOT NULL,
    updated_by   BLOB NOT NULL,
    updated_kind TEXT NOT NULL
        CHECK (updated_kind IN ('user', 'system'))
) STRICT;

CREATE INDEX idx_pursuit_line ON pursuit(line_id);
CREATE INDEX idx_pursuit_parent ON pursuit(parent_id);
CREATE INDEX idx_pursuit_team ON pursuit(team_id);
CREATE UNIQUE INDEX idx_pursuit_line_id ON pursuit(line_id, id);

CREATE TABLE pursuit_node (
    id         BLOB PRIMARY KEY,
    team_id    BLOB NOT NULL,
    pursuit_id BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    parent_id  BLOB NOT NULL,
    kind       TEXT NOT NULL
        CHECK (kind IN ('round', 'close')),
    outcome    TEXT
        CHECK (outcome IN ('satisfied', 'abandoned')),
    note       TEXT,
    at         INTEGER NOT NULL,
    actor_id   BLOB NOT NULL,
    actor_kind TEXT NOT NULL
        CHECK (actor_kind IN ('user', 'system')),
    CHECK ((kind = 'close') = (outcome IS NOT NULL))
) STRICT;

CREATE UNIQUE INDEX idx_pursuit_node_on_parent
    ON pursuit_node(pursuit_id, parent_id);
CREATE UNIQUE INDEX idx_pursuit_node_one_close
    ON pursuit_node(pursuit_id) WHERE kind = 'close';
CREATE INDEX idx_pursuit_node_pursuit ON pursuit_node(pursuit_id);

CREATE TABLE pursuit_op (
    node_id  BLOB NOT NULL REFERENCES pursuit_node(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL,
    team_id  BLOB NOT NULL,
    entry_id BLOB NOT NULL,
    verb     TEXT NOT NULL
        CHECK (verb IN ('add', 'replace', 'rename', 'remove')),
    content  BLOB REFERENCES team_asset(id) ON DELETE RESTRICT,
    name     TEXT,
    PRIMARY KEY (node_id, position),
    CHECK ((verb IN ('add', 'replace')) = (content IS NOT NULL)),
    CHECK ((verb IN ('add', 'rename')) = (name IS NOT NULL))
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_pursuit_op_entry ON pursuit_op(entry_id);
CREATE INDEX idx_pursuit_op_content ON pursuit_op(content);

CREATE TABLE change_point (
    id         BLOB PRIMARY KEY,
    team_id    BLOB NOT NULL,
    line_id    BLOB NOT NULL REFERENCES line(id) ON DELETE RESTRICT,
    parent_id  BLOB NOT NULL,
    from_work  BLOB NOT NULL,
    by_node    BLOB NOT NULL REFERENCES pursuit_node(id) ON DELETE RESTRICT,
    at         INTEGER NOT NULL,
    actor_id   BLOB NOT NULL,
    actor_kind TEXT NOT NULL
        CHECK (actor_kind IN ('user', 'system')),
    FOREIGN KEY (line_id, from_work)
        REFERENCES pursuit(line_id, id) ON DELETE RESTRICT
) STRICT;

CREATE UNIQUE INDEX idx_change_point_on_parent
    ON change_point(line_id, parent_id);
CREATE INDEX idx_change_point_line ON change_point(line_id);
CREATE INDEX idx_change_point_from ON change_point(from_work);
CREATE INDEX idx_change_point_by ON change_point(by_node);

CREATE TABLE change_row (
    point_id  BLOB NOT NULL REFERENCES change_point(id) ON DELETE RESTRICT,
    entry_id  BLOB NOT NULL,
    team_id   BLOB NOT NULL,
    existence TEXT
        CHECK (existence IN ('present', 'absent')),
    content   BLOB REFERENCES team_asset(id) ON DELETE RESTRICT,
    name      TEXT,
    PRIMARY KEY (point_id, entry_id),
    CHECK (existence IS NOT NULL OR content IS NOT NULL OR name IS NOT NULL),
    CHECK (existence IS NOT 'absent' OR (content IS NULL AND name IS NULL))
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_change_row_entry ON change_row(entry_id);
CREATE INDEX idx_change_row_content ON change_row(content);

CREATE TABLE forge_actor (
    id           BLOB PRIMARY KEY,
    team_id      BLOB NOT NULL,
    stands_for   TEXT NOT NULL
        CHECK (stands_for IN ('owner', 'subject', 'unrecorded', 'server')),
    subject      TEXT,
    display_name TEXT,
    created_at   INTEGER NOT NULL,
    CHECK ((stands_for = 'subject') = (subject IS NOT NULL))
) STRICT;

CREATE UNIQUE INDEX idx_forge_actor_stands_for
    ON forge_actor(team_id, stands_for, COALESCE(subject, ''));

CREATE TABLE forge_thread (
    id                  BLOB PRIMARY KEY,
    team_id             BLOB NOT NULL,
    anchor_kind         TEXT NOT NULL
        CHECK (anchor_kind IN ('pursuit', 'round', 'entry', 'change_point')),
    anchor_pursuit      BLOB REFERENCES pursuit(id) ON DELETE RESTRICT,
    anchor_node         BLOB,
    anchor_entry        BLOB,
    anchor_change_point BLOB REFERENCES change_point(id) ON DELETE RESTRICT,
    title               TEXT,
    created_at          INTEGER NOT NULL,
    created_by          BLOB NOT NULL,
    created_kind        TEXT NOT NULL
        CHECK (created_kind IN ('user', 'system')),
    updated_at          INTEGER NOT NULL,
    updated_by          BLOB NOT NULL,
    updated_kind        TEXT NOT NULL
        CHECK (updated_kind IN ('user', 'system')),
    CHECK ((anchor_kind = 'pursuit') = (anchor_pursuit IS NOT NULL)),
    CHECK ((anchor_kind IN ('round', 'entry')) = (anchor_node IS NOT NULL)),
    CHECK ((anchor_kind = 'entry') = (anchor_entry IS NOT NULL)),
    CHECK ((anchor_kind = 'change_point') = (anchor_change_point IS NOT NULL))
) STRICT;

CREATE INDEX idx_forge_thread_anchor_pursuit ON forge_thread(anchor_pursuit);
CREATE INDEX idx_forge_thread_anchor_node ON forge_thread(anchor_node);
CREATE INDEX idx_forge_thread_anchor_change_point ON forge_thread(anchor_change_point);
CREATE INDEX idx_forge_thread_team ON forge_thread(team_id);

CREATE TABLE forge_thread_message (
    id        BLOB PRIMARY KEY,
    team_id   BLOB NOT NULL,
    thread_id BLOB NOT NULL REFERENCES forge_thread(id) ON DELETE RESTRICT,
    parent_id BLOB REFERENCES forge_thread_message(id) ON DELETE RESTRICT,
    body      TEXT NOT NULL,
    said_at   INTEGER NOT NULL,
    said_by   BLOB NOT NULL,
    said_kind TEXT NOT NULL
        CHECK (said_kind IN ('user', 'system'))
) STRICT;

CREATE INDEX idx_forge_thread_message_thread
    ON forge_thread_message(thread_id, said_at);

CREATE TABLE forge_thread_revision (
    message_id BLOB NOT NULL REFERENCES forge_thread_message(id) ON DELETE RESTRICT,
    position   INTEGER NOT NULL,
    team_id    BLOB NOT NULL,
    body       TEXT NOT NULL,
    said_at    INTEGER NOT NULL,
    said_by    BLOB NOT NULL,
    said_kind  TEXT NOT NULL
        CHECK (said_kind IN ('user', 'system')),
    PRIMARY KEY (message_id, position)
) STRICT, WITHOUT ROWID;
"#;

/// Version 7 → 8: what a `team_asset` was converted from (#148
/// decision 5, the content verb).
///
/// `V7` left this open in as many words — "what composes it against
/// the CAS is conversion, which arrives with the content verb" — and
/// the verb is #151's. Two columns, both nullable, and the v0
/// conversion fills both.
///
/// **`digest` is the CAS entry the asset was converted from.** One
/// blob, which is the whole of the v0 conversion; decision 3 says a
/// conversion may be several materials or a Collection, and the shape
/// that admits those without rewriting this table is a second table
/// keyed by asset — which is why the column is nullable rather than
/// `NOT NULL`. An asset composed some other way leaves it empty and
/// says so by that, instead of carrying a digest that stands for one
/// part of itself. Not `UNIQUE`: decision 7 mints an asset per
/// promotion over one stored copy, so two rows sharing a digest is the
/// arrangement, not a fault.
///
/// **`entered_for` is the work the content arrived against.** It
/// records decision 5's attachment rather than enforcing it: the
/// column is nullable and unconstrained, so what the schema keeps is
/// the evidence for rows that have one, and the invariant itself is
/// the content verb's — it is checked at the door, in the same
/// transaction that writes this row. Reading the column as the
/// enforcement would be reading a record as a rule.
/// It carries no foreign key, on `team_id`'s reasoning one batch
/// up and one of its own: `Lines::discard` deletes the work against a
/// line, and a `RESTRICT` key here would refuse the one verb that
/// releases this content, while a `CASCADE` would delete the record of
/// what was brought in as a side effect of dropping a line.
///
/// **Neither column is indexed, because nothing reads by them.** The
/// two reads over this table both arrive with an id: `Store::exists`
/// asks about one, the bulk resolve asks about a list, and the primary
/// key serves both. The have-check looks like the exception and is
/// not — it answers out of `team_blob_link`, whose own primary key is
/// `(team_id, digest)`. An index here would be write cost on a table
/// that gains a row per promotion, bought for no reader; the batch is
/// append-only, so the moment to leave it out is this one.
const V8_TEAM_ASSET_CONTENT: &str = r#"
ALTER TABLE team_asset ADD COLUMN digest TEXT;
ALTER TABLE team_asset ADD COLUMN entered_for BLOB;
"#;

/// V9 — what a promoter said about an entry (#148 decisions 12 and
/// 14).
///
/// **Outside the forge, and the table is where that is visible.** The
/// forge has three axes and #102 forbids a column that answers what
/// the history already answers, so a description does not go on a
/// change point. It sits here instead, keyed `(line_id, entry_id)`.
/// What being outside buys is stated where the type is defined
/// (`teams_core::domain::projection`).
///
/// **`team_id` is what scopes it, and it is not part of the key.**
/// Exactly the arrangement `V7` gives every forge table: a line id is
/// unique across teams, so the key is already exact and adding the
/// scope column to it would widen a key for nothing. What the column
/// is for is the read — a caller arrives holding a team's session and
/// a `(line, entry)` pair, and without this column the store can only
/// answer whether *somebody's* entry has a description, not whether
/// this caller's team has one. That is a cross-team read, and it is
/// the reason this column is `NOT NULL` and every statement over this
/// table filters on it.
///
/// It also gives a team's departure something to find. Deleting a team
/// leaves its forge rows behind today and would leave these too; the
/// difference is that a sweep is now *expressible*, which it was not
/// while the table had no idea whose rows it held. Who owns that
/// sweep is the same open question `V7` records for the forge rows,
/// and this batch does not answer it.
///
/// **`body` is opaque, and every column that is not here is the
/// decision.** Decision 14 keeps the body free of columns, validation
/// and indexes on this plane: no `title`, no `description`, no `tags`,
/// nothing lifted out of it — the check the decision states is that a
/// column naming something inside the body breaks it. So `body` is
/// `TEXT` holding whatever the member's mapper wrote, stored and
/// handed back unread.
///
/// `version` is not such a column — it is a fact about the envelope
/// rather than a field of the description, and
/// `teams_core::domain::projection` is where that distinction is
/// argued. A migration is where a column gets added, which is why the
/// rule is restated here and not merely pointed at.
///
/// **One row per entry, replaced rather than versioned.** Decision 12
/// says a projection is captured at the time and only a forge op
/// replaces one — so the history of what was said is the ledger's
/// (each replacing push has its own event), and this table holds the
/// present. `promoted_by` and `pushed_at` are the stamp on the row the
/// present came from, on the write-time-capture discipline the ledger
/// already keeps.
///
/// **No foreign key to `line` or to any entry.** There is no entry
/// table to point at — an entry is a name that appears in change rows
/// rather than a row of its own — and a key on `line_id` would refuse
/// `Lines::discard`, the one verb that takes a line with its log. A
/// discarded line leaves its projections behind, which is the same
/// arrangement `team_asset` keeps for its own reasons one batch up:
/// releasing is not deleting, and reclaiming storage is the purge's
/// job rather than a side effect of dropping a line.
const V9_ASSET_PROJECTION: &str = r#"
CREATE TABLE asset_projection (
    line_id     BLOB    NOT NULL,
    entry_id    BLOB    NOT NULL,
    team_id     BLOB    NOT NULL,
    version     INTEGER NOT NULL,
    body        TEXT    NOT NULL,
    promoted_by BLOB    NOT NULL,
    pushed_at   INTEGER NOT NULL,
    PRIMARY KEY (line_id, entry_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_asset_projection_team ON asset_projection(team_id);
"#;

/// Migrations in application order. **Append only** — never rewrite an
/// existing batch.
const MIGRATIONS: &[&str] = &[
    V1_INITIAL_SCHEMA,
    V2_AUTH_TABLES,
    V3_PURGE_MARK,
    V4_MODEL_REGISTRY,
    V5_HEAD_REGISTRY,
    V6_ADMIN_RENAME,
    V7_FORGE_TABLES,
    V8_TEAM_ASSET_CONTENT,
    V9_ASSET_PROJECTION,
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
    use uuid::Uuid;

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
    fn v6_renames_the_account_flag_and_carries_its_values_across() {
        // The rename is a rename: a row written under the old name
        // reads back under the new one with the value it had.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (index, batch) in MIGRATIONS.iter().enumerate().take(5) {
            conn.execute_batch(batch).unwrap();
            conn.pragma_update(None, "user_version", (index + 1) as i64)
                .unwrap();
        }
        conn.execute(
            "INSERT INTO user_account
             (user_id, login, display_name, password_hash, is_operator, created_at)
             VALUES (X'00', 'op', 'Operator', 'hash', 1, 0)",
            [],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let admin: bool = conn
            .query_row("SELECT is_admin FROM user_account", [], |row| row.get(0))
            .unwrap();
        assert!(admin, "the flag keeps its value across the rename");

        // And the old name is gone rather than duplicated.
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'user_account'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!ddl.contains("is_operator"), "{ddl}");
    }

    #[test]
    fn v7_scopes_every_forge_table_to_a_team() {
        let conn = migrated();
        for table in [
            "line",
            "change_point",
            "change_row",
            "pursuit",
            "pursuit_node",
            "pursuit_op",
            "forge_actor",
            "forge_thread",
            "forge_thread_message",
            "forge_thread_revision",
            "team_asset",
        ] {
            let scoped: i64 = conn
                .query_row(
                    "SELECT count(*) FROM pragma_table_info(?1)
                      WHERE name = 'team_id' AND \"notnull\" = 1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap_or_else(|e| panic!("{table} must exist: {e}"));
            assert_eq!(scoped, 1, "{table} must be scoped to a team");
        }
    }

    #[test]
    fn v8_reaches_a_database_that_already_has_assets_on_it() {
        // The upgrade path, which the fresh-database tests cannot ask
        // about: `V8` adds columns to a table `V7` may already have
        // rows in, and an `ALTER TABLE ADD COLUMN` that demanded a
        // value would refuse them. Both columns are nullable for that
        // reason among others, and this is where "among others" is
        // checked.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (index, batch) in MIGRATIONS.iter().enumerate().take(7) {
            conn.execute_batch(batch).unwrap();
            conn.pragma_update(None, "user_version", (index + 1) as i64)
                .unwrap();
        }
        let (asset, team) = (Uuid::now_v7(), Uuid::now_v7());
        conn.execute(
            "INSERT INTO team_asset (id, team_id, created_at) VALUES (?1, ?2, 7)",
            rusqlite::params![asset, team],
        )
        .expect("an asset minted before the content verb existed");

        migrate(&mut conn).unwrap();

        let (digest, entered_for, created_at): (Option<String>, Option<Uuid>, i64) = conn
            .query_row(
                "SELECT digest, entered_for, created_at FROM team_asset WHERE id = ?1",
                rusqlite::params![asset],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the row survives the upgrade");
        assert_eq!(digest, None, "nothing invents a conversion for it");
        assert_eq!(entered_for, None);
        assert_eq!(created_at, 7, "and what it did carry is untouched");
    }

    #[test]
    fn v8_lets_one_digest_stand_behind_two_assets() {
        // Decision 7: two members promoting identical content produce
        // two `TeamAsset`s over one stored copy, which is the only
        // arrangement where "who brought what" survives the second
        // contributor. A unique index on the digest would forbid it.
        let conn = migrated();
        let team = Uuid::now_v7();
        let digest = format!("sha256:{}", "a".repeat(64));
        let mint = |asset: Uuid| {
            conn.execute(
                "INSERT INTO team_asset (id, team_id, created_at, digest, entered_for)
                 VALUES (?1, ?2, 0, ?3, ?4)",
                rusqlite::params![asset, team, digest, Uuid::now_v7()],
            )
        };
        mint(Uuid::now_v7()).expect("the first promotion");
        mint(Uuid::now_v7()).expect("the second promotion of the same bytes");

        // And a conversion that is not one blob leaves both empty
        // rather than carrying a digest that stands for a part of
        // itself — which is what nullable is for here.
        conn.execute(
            "INSERT INTO team_asset (id, team_id, created_at) VALUES (?1, ?2, 0)",
            rusqlite::params![Uuid::now_v7(), team],
        )
        .expect("an asset composed some other way");
    }

    #[test]
    fn v7_answers_the_name_question_the_forge_leaves_to_its_host() {
        let conn = migrated();
        let team = Uuid::now_v7();
        let other = Uuid::now_v7();
        let open = |team_id: Uuid, name: &str| {
            conn.execute(
                "INSERT INTO line
                 (id, team_id, name, strategy, standing, genesis_id, genesis_at, genesis_by,
                  genesis_kind, created_at, created_by, created_kind,
                  updated_at, updated_by, updated_kind)
                 VALUES (?1, ?2, ?3, 'mainline-first', 'open', ?4, 0, ?4, 'user',
                         0, ?4, 'user', 0, ?4, 'user')",
                rusqlite::params![Uuid::now_v7(), team_id, name, Uuid::now_v7()],
            )
        };

        open(team, "notes").expect("the first line of a name");
        // The same name in the same team is the collision the host
        // answers…
        assert!(open(team, "notes").is_err());
        // …and the same name in another team is not a collision at
        // all: the namespace belongs to the team, not to the instance.
        open(other, "notes").expect("another team's namespace is its own");
    }

    #[test]
    fn v7_keeps_the_forge_rules_the_local_plane_states() {
        let conn = migrated();
        let ddl: String = conn
            .query_row(
                "SELECT group_concat(sql, ';') FROM sqlite_master
                  WHERE name IN ('idx_change_point_on_parent',
                                 'idx_pursuit_node_on_parent',
                                 'idx_pursuit_node_one_close')",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // Neither log forks, and work ends once — the three indexes
        // that are the concurrency control rather than a check beside
        // it. They are keyed on the node ids alone here as they are on
        // the local plane: a line id is unique across teams, so adding
        // the scope column would widen a key that is already exact.
        for expected in [
            "change_point(line_id, parent_id)",
            "pursuit_node(pursuit_id, parent_id)",
            "pursuit_node(pursuit_id) WHERE kind = 'close'",
        ] {
            assert!(ddl.contains(expected), "{expected} missing from: {ddl}");
        }
    }

    #[test]
    fn the_projection_lifts_no_field_out_of_its_body() {
        // A column added here for a field a client happened to be
        // putting in its bodies is how #148 decision 14 would be
        // broken, and this is the test that says so before it ships.
        let conn = migrated();
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'asset_projection'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for lifted in [
            "title",
            "description",
            "summary",
            "tags",
            "labels",
            "keywords",
            "marks",
        ] {
            assert!(
                !ddl.contains(lifted),
                "asset_projection must not name {lifted:?}, which lives inside the body: {ddl}"
            );
        }

        // And nothing indexes the body either — an index over a
        // projection's contents is a reader that has opinions about
        // what is in one. The team scope is indexed, which is a fact
        // about whose row it is rather than about what the row says.
        let indexed: Vec<String> = conn
            .prepare(
                "SELECT sql FROM sqlite_master
                  WHERE type = 'index' AND tbl_name = 'asset_projection'
                    AND sql IS NOT NULL",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(indexed.len(), 1, "{indexed:?}");
        assert!(indexed[0].contains("team_id"), "{indexed:?}");
        assert!(!indexed[0].contains("body"), "{indexed:?}");
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
