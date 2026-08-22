//! SQLite schema migrations — `PRAGMA user_version` scheme.
//!
//! ## How it works
//!
//! `MIGRATIONS[i]` is the DDL batch that upgrades from version `i` to
//! `i + 1`. [`migrate`] applies every pending batch inside its own
//! transaction and bumps `user_version` on success. **Never rewrite a
//! past batch** — schema changes go at the end (append-only, mirroring
//! the discipline used elsewhere in the workspace).
//!
//! ## Schema decisions
//!
//! - **Ids are 16-byte BLOBs (UUID v7).** Smaller index footprint than
//!   36-byte TEXT ids at Asterism's 100k+ scale. The `uuid` feature of
//!   `rusqlite` provides the `ToSql` / `FromSql` bridge.
//! - **Timestamps are `INTEGER` (unix epoch ms).** Range filters and sorts
//!   use indexes directly; ISO 8601 TEXT would be readable but slower and
//!   larger.
//! - **`STRICT` tables** disallow implicit type conversion (SQLite 3.37+
//!   is bundled by `rusqlite`).
//! - **Visibility is split into two columns** (`vis_restricted` and
//!   `vis_sharing` as a JSON array). This lets the visibility filter be
//!   written directly with the JSON1 extension.
//! - **`labels` / `keywords` / `extra` are JSON TEXT.** They stay
//!   denormalised until a query needs to join on them.
//! - **No dedicated `job` table.** Job persistence is owned by
//!   `apalis-sql`, which creates its own table (`Jobs`, and so on) inside
//!   the same DB via `SqliteStorage::setup`. Duplicating that in a
//!   domain-side mirror would give us two sources of truth for the same
//!   data.

use std::collections::HashMap;

use rusqlite::{Connection, Transaction, params};
use uuid::Uuid;

/// Version 0 → 1: initial schema (persona / asset / tag / asset_tag /
/// edge / thumb_cache).
const V1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE persona (
    id            BLOB PRIMARY KEY,
    pack_id       TEXT UNIQUE,
    name          TEXT NOT NULL,
    accent_color  TEXT,
    display_order INTEGER NOT NULL DEFAULT 0,
    archived      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE asset (
    id              BLOB PRIMARY KEY,
    persona_id      BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    source_kind     TEXT NOT NULL,
    source_locator  TEXT NOT NULL,
    file_size_bytes INTEGER,
    platform        TEXT,
    modality        TEXT NOT NULL,
    labels          TEXT NOT NULL DEFAULT '[]',
    occurred_at     INTEGER NOT NULL,
    session_id      TEXT,
    cover           TEXT,
    keywords        TEXT NOT NULL DEFAULT '[]',
    register_note   TEXT,
    vis_restricted  INTEGER NOT NULL DEFAULT 0,
    vis_sharing     TEXT NOT NULL DEFAULT '[]',
    duration_ms     INTEGER,
    extra           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_asset_persona_occurred
    ON asset(persona_id, occurred_at DESC);
CREATE INDEX idx_asset_persona_modality_occurred
    ON asset(persona_id, modality, occurred_at DESC);
CREATE INDEX idx_asset_occurred
    ON asset(occurred_at DESC);
CREATE INDEX idx_asset_session
    ON asset(session_id) WHERE session_id IS NOT NULL;

CREATE TABLE tag (
    id   BLOB PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    axis TEXT
) STRICT;

CREATE TABLE asset_tag (
    asset_id BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    tag_id   BLOB NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (asset_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_asset_tag_tag ON asset_tag(tag_id, asset_id);

CREATE TABLE edge (
    id         BLOB PRIMARY KEY,
    from_asset BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    to_asset   BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    label      TEXT,
    weight     REAL,
    UNIQUE (from_asset, to_asset, kind)
) STRICT;

CREATE INDEX idx_edge_from ON edge(from_asset, kind, weight DESC);
CREATE INDEX idx_edge_to   ON edge(to_asset);

CREATE TABLE thumb_cache (
    asset_id   BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    size_px    INTEGER NOT NULL,
    data       BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (asset_id, size_px)
) STRICT, WITHOUT ROWID;
"#;

/// Version 1 → 2: import idempotency — a `(source_kind, source_locator)`
/// unique index so re-importing the same file does not produce
/// duplicate assets.
const V2_ASSET_SOURCE_UNIQUE: &str = r#"
CREATE UNIQUE INDEX idx_asset_source_unique
    ON asset(source_kind, source_locator);
"#;

/// Version 2 → 3: bidirectional constellation hover — mirror the
/// existing `idx_edge_from(from_asset, kind, weight DESC)` on the
/// `to_asset` side so `edges_incident` can serve a
/// `WHERE from_asset = ? OR to_asset = ?` query without a full
/// table scan on the incoming side. The old `idx_edge_to(to_asset)`
/// stays put (it still supports plain to-side lookups and the
/// `ON DELETE CASCADE` FK).
const V3_EDGE_TO_KIND_INDEX: &str = r#"
CREATE INDEX idx_edge_to_kind_weight
    ON edge(to_asset, kind, weight DESC);
"#;

/// Version 3 → 4: user-curated groups. `bucket` is a persona-scoped
/// user-created set of assets (the domain calls it `Group`; the
/// table is named `bucket` because SQL reserves the word `GROUP`).
/// `asset_bucket` is the m:n link; the composite `(bucket_id,
/// asset_id)` index mirrors the tag-side layout so the same
/// `EXISTS (… WHERE bucket_id IN (…))` query shape used for tag
/// filtering can be reused for group filtering.
const V4_GROUP_TABLES: &str = r#"
CREATE TABLE bucket (
    id          BLOB PRIMARY KEY,
    persona_id  BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_bucket_persona ON bucket(persona_id);
CREATE UNIQUE INDEX idx_bucket_persona_name ON bucket(persona_id, name);

CREATE TABLE asset_bucket (
    asset_id  BLOB NOT NULL REFERENCES asset(id)  ON DELETE CASCADE,
    bucket_id BLOB NOT NULL REFERENCES bucket(id) ON DELETE CASCADE,
    added_at  INTEGER NOT NULL,
    PRIMARY KEY (asset_id, bucket_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_asset_bucket_bucket
    ON asset_bucket(bucket_id, asset_id);
"#;

/// Version 4 → 5: hand-arranged order within a group. `position` lets
/// users drag-reorder assets so a bucket becomes an ordered collection
/// (the Are.na "Channel" affordance), not just a set. Backfill assigns
/// each existing row a rank by `added_at` so pre-migration groups keep
/// their insertion order once the UI switches to position-sort.
const V5_ASSET_BUCKET_POSITION: &str = r#"
ALTER TABLE asset_bucket ADD COLUMN position INTEGER NOT NULL DEFAULT 0;

UPDATE asset_bucket AS ab
   SET position = (
       SELECT COUNT(*) FROM asset_bucket AS ab2
        WHERE ab2.bucket_id = ab.bucket_id
          AND (ab2.added_at < ab.added_at
               OR (ab2.added_at = ab.added_at AND ab2.asset_id < ab.asset_id))
   );
"#;

/// Version 5 → 6: sidebar organisation tree. `dir` is a persona-scoped
/// folder tree that contains dirs and groups — a pure navigation axis,
/// deliberately separate from the curation axis (`bucket` +
/// `asset_bucket`). Assets never live in a dir; a dir never filters
/// the grid on the SQL side (the UI expands a dir selection into the
/// group ids beneath it, so the existing `group_ids` OR filter shape
/// stays untouched).
///
/// - `(persona_id, parent_id, name)` is unique. SQLite treats NULLs as
///   distinct inside a unique index, so root-level uniqueness needs its
///   own partial index.
/// - `dir.parent_id` cascades: the repository forbids deleting a
///   non-empty dir, so the cascade is a safety net for any future
///   force path, not a behaviour the app relies on.
/// - `bucket.dir_id` is `ON DELETE SET NULL`: curation data must never
///   be destroyed through the organisation axis — a group falls back
///   to the root instead.
const V6_DIR_TABLES: &str = r#"
CREATE TABLE dir (
    id         BLOB PRIMARY KEY,
    persona_id BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    parent_id  BLOB REFERENCES dir(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_dir_persona_parent ON dir(persona_id, parent_id);
CREATE UNIQUE INDEX idx_dir_child_name
    ON dir(persona_id, parent_id, name) WHERE parent_id IS NOT NULL;
CREATE UNIQUE INDEX idx_dir_root_name
    ON dir(persona_id, name) WHERE parent_id IS NULL;

ALTER TABLE bucket ADD COLUMN dir_id BLOB REFERENCES dir(id) ON DELETE SET NULL;

CREATE INDEX idx_bucket_dir ON bucket(dir_id) WHERE dir_id IS NOT NULL;
"#;

/// Version 6 → 7: group-in-group nesting — the Are.na
/// "channel connected into a channel" affordance. `bucket_link` is an
/// m:n *connection* (one group can appear inside many groups, each
/// with its own position), mirroring the `asset_bucket` membership
/// shape rather than a single-parent tree. The repository rejects
/// links that would close a cycle (recursive-CTE reachability check)
/// and links across personas.
const V7_BUCKET_LINK: &str = r#"
CREATE TABLE bucket_link (
    parent_id BLOB NOT NULL REFERENCES bucket(id) ON DELETE CASCADE,
    child_id  BLOB NOT NULL REFERENCES bucket(id) ON DELETE CASCADE,
    added_at  INTEGER NOT NULL,
    position  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (parent_id, child_id),
    CHECK (parent_id <> child_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_bucket_link_child ON bucket_link(child_id, parent_id);
"#;

/// Version 7 → 8: per-asset content-type flags (`has_code` / `has_table`
/// / `has_mermaid` / `has_link`). The Sessions view aggregates these
/// with `MAX(...)` to badge sessions carrying tables, mermaid diagrams,
/// or code blocks — cheap enough to run on every query without a full
/// INSTR scan.
///
/// Existing rows are backfilled from the cover snippet (the same
/// approximation the earlier query-time INSTR used); `cover_gen` will
/// refine the values from the full body next time it processes each
/// asset. Columns are `INTEGER NOT NULL DEFAULT 0` because SQLite
/// STRICT tables reject `BOOLEAN`.
const V8_ASSET_CONTENT_FLAGS: &str = r#"
ALTER TABLE asset ADD COLUMN has_code    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE asset ADD COLUMN has_table   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE asset ADD COLUMN has_mermaid INTEGER NOT NULL DEFAULT 0;
ALTER TABLE asset ADD COLUMN has_link    INTEGER NOT NULL DEFAULT 0;

UPDATE asset SET
  has_code    = CASE WHEN INSTR(cover, '```')        > 0 THEN 1 ELSE 0 END,
  has_mermaid = CASE WHEN INSTR(cover, '```mermaid') > 0 THEN 1 ELSE 0 END,
  has_link    = CASE WHEN INSTR(cover, '](')         > 0 THEN 1 ELSE 0 END,
  has_table   = CASE WHEN INSTR(cover, '|-') > 0 AND INSTR(cover, '|') > 0 THEN 1 ELSE 0 END
WHERE cover IS NOT NULL;
"#;

// V9 (session_summary + triggers) was drafted and then reverted at
// User request — the shape (materialised table vs alternatives, and
// trigger-vs-application maintenance) was chosen without User
// confirmation. If the reader is on this file wondering why V9 is
// missing: the migrations list is authoritative. Any subsequent
// bump should start at V10 to preserve the append-only rule.
//
// The block below is retained (dead code) as the source of the
// physical DDL applied to any DB that briefly reached user_version
// = 9 during the aborted rollout, so a rollback migration can be
// written against a known reference.
#[allow(dead_code)]
const V9_SESSION_SUMMARY_REVERTED: &str = r#"
CREATE TABLE session_summary (
    session_id       TEXT NOT NULL,
    persona_id       BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    message_count    INTEGER NOT NULL,
    first_occurred_at INTEGER NOT NULL,
    last_occurred_at INTEGER NOT NULL,
    first_asset_id   BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    primary_modality TEXT NOT NULL,
    cover_hint       TEXT,
    has_code         INTEGER NOT NULL DEFAULT 0,
    has_table        INTEGER NOT NULL DEFAULT 0,
    has_mermaid      INTEGER NOT NULL DEFAULT 0,
    has_link         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, persona_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_session_summary_last ON session_summary(last_occurred_at DESC);
CREATE INDEX idx_session_summary_persona_last
    ON session_summary(persona_id, last_occurred_at DESC);

-- Backfill from existing asset rows. Correlated subqueries stay
-- inside the migration (one-off cost); every subsequent read hits
-- the materialised table directly.
INSERT INTO session_summary (
    session_id, persona_id, message_count,
    first_occurred_at, last_occurred_at,
    first_asset_id, primary_modality, cover_hint,
    has_code, has_table, has_mermaid, has_link
)
SELECT
    asset.session_id,
    asset.persona_id,
    COUNT(*),
    MIN(asset.occurred_at),
    MAX(asset.occurred_at),
    ( SELECT a2.id FROM asset a2
      WHERE a2.session_id = asset.session_id
        AND a2.persona_id = asset.persona_id
      ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
    ( SELECT a2.modality FROM asset a2
      WHERE a2.session_id = asset.session_id
        AND a2.persona_id = asset.persona_id
      ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
    ( SELECT a2.cover FROM asset a2
      WHERE a2.session_id = asset.session_id
        AND a2.persona_id = asset.persona_id
      ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
    MAX(asset.has_code),
    MAX(asset.has_table),
    MAX(asset.has_mermaid),
    MAX(asset.has_link)
FROM asset
WHERE asset.session_id IS NOT NULL
GROUP BY asset.session_id, asset.persona_id;

-- Maintenance triggers. Each recomputes the affected row from
-- scratch — cheaper than tracking deltas and correct for every
-- column path. `WHEN NEW.session_id IS NOT NULL` skips the
-- session-less majority when it exists (drafts, orphans).
--
-- Assets can move between sessions (session_id UPDATE) or be
-- deleted; the trigger handles OLD and NEW independently so both
-- summaries are refreshed.

CREATE TRIGGER trg_session_summary_ins
AFTER INSERT ON asset
WHEN NEW.session_id IS NOT NULL
BEGIN
    INSERT INTO session_summary (
        session_id, persona_id, message_count,
        first_occurred_at, last_occurred_at,
        first_asset_id, primary_modality, cover_hint,
        has_code, has_table, has_mermaid, has_link
    )
    SELECT
        a.session_id, a.persona_id,
        COUNT(*),
        MIN(a.occurred_at),
        MAX(a.occurred_at),
        ( SELECT a2.id FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        ( SELECT a2.modality FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        ( SELECT a2.cover FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        MAX(a.has_code), MAX(a.has_table), MAX(a.has_mermaid), MAX(a.has_link)
    FROM asset a
    WHERE a.session_id = NEW.session_id AND a.persona_id = NEW.persona_id
    GROUP BY a.session_id, a.persona_id
    ON CONFLICT(session_id, persona_id) DO UPDATE SET
        message_count = excluded.message_count,
        first_occurred_at = excluded.first_occurred_at,
        last_occurred_at = excluded.last_occurred_at,
        first_asset_id = excluded.first_asset_id,
        primary_modality = excluded.primary_modality,
        cover_hint = excluded.cover_hint,
        has_code = excluded.has_code,
        has_table = excluded.has_table,
        has_mermaid = excluded.has_mermaid,
        has_link = excluded.has_link;
END;

CREATE TRIGGER trg_session_summary_upd
AFTER UPDATE ON asset
BEGIN
    -- Old row's session (only if it was session-attached and moved
    -- or was mutated).
    DELETE FROM session_summary
     WHERE session_id = OLD.session_id
       AND persona_id = OLD.persona_id
       AND OLD.session_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM asset
            WHERE session_id = OLD.session_id
              AND persona_id = OLD.persona_id
       );

    INSERT INTO session_summary (
        session_id, persona_id, message_count,
        first_occurred_at, last_occurred_at,
        first_asset_id, primary_modality, cover_hint,
        has_code, has_table, has_mermaid, has_link
    )
    SELECT
        a.session_id, a.persona_id,
        COUNT(*), MIN(a.occurred_at), MAX(a.occurred_at),
        ( SELECT a2.id FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        ( SELECT a2.modality FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        ( SELECT a2.cover FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        MAX(a.has_code), MAX(a.has_table), MAX(a.has_mermaid), MAX(a.has_link)
    FROM asset a
    WHERE a.session_id IS NOT NULL
      AND ((a.session_id = OLD.session_id AND a.persona_id = OLD.persona_id)
        OR (a.session_id = NEW.session_id AND a.persona_id = NEW.persona_id))
    GROUP BY a.session_id, a.persona_id
    ON CONFLICT(session_id, persona_id) DO UPDATE SET
        message_count = excluded.message_count,
        first_occurred_at = excluded.first_occurred_at,
        last_occurred_at = excluded.last_occurred_at,
        first_asset_id = excluded.first_asset_id,
        primary_modality = excluded.primary_modality,
        cover_hint = excluded.cover_hint,
        has_code = excluded.has_code,
        has_table = excluded.has_table,
        has_mermaid = excluded.has_mermaid,
        has_link = excluded.has_link;
END;

CREATE TRIGGER trg_session_summary_del
AFTER DELETE ON asset
WHEN OLD.session_id IS NOT NULL
BEGIN
    DELETE FROM session_summary
     WHERE session_id = OLD.session_id
       AND persona_id = OLD.persona_id
       AND NOT EXISTS (
           SELECT 1 FROM asset
            WHERE session_id = OLD.session_id
              AND persona_id = OLD.persona_id
       );

    INSERT INTO session_summary (
        session_id, persona_id, message_count,
        first_occurred_at, last_occurred_at,
        first_asset_id, primary_modality, cover_hint,
        has_code, has_table, has_mermaid, has_link
    )
    SELECT
        a.session_id, a.persona_id,
        COUNT(*), MIN(a.occurred_at), MAX(a.occurred_at),
        ( SELECT a2.id FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        ( SELECT a2.modality FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        ( SELECT a2.cover FROM asset a2
          WHERE a2.session_id = a.session_id
            AND a2.persona_id = a.persona_id
          ORDER BY a2.occurred_at ASC, a2.id ASC LIMIT 1 ),
        MAX(a.has_code), MAX(a.has_table), MAX(a.has_mermaid), MAX(a.has_link)
    FROM asset a
    WHERE a.session_id = OLD.session_id AND a.persona_id = OLD.persona_id
    GROUP BY a.session_id, a.persona_id
    ON CONFLICT(session_id, persona_id) DO UPDATE SET
        message_count = excluded.message_count,
        first_occurred_at = excluded.first_occurred_at,
        last_occurred_at = excluded.last_occurred_at,
        first_asset_id = excluded.first_asset_id,
        primary_modality = excluded.primary_modality,
        cover_hint = excluded.cover_hint,
        has_code = excluded.has_code,
        has_table = excluded.has_table,
        has_mermaid = excluded.has_mermaid,
        has_link = excluded.has_link;
END;
"#;

/// Version 8 → 9: per-persona UI chrome (`persona_theme`). A 1:1 side
/// table keyed by `persona_id` so a persona can carry a wallpaper
/// asset reference without polluting the `persona` write path. The
/// wallpaper column is `ON DELETE SET NULL` so deleting the referenced
/// asset falls back to "theme with no wallpaper" instead of removing
/// the theme row (other decoration fields land here later; keeping
/// the row alive is the safer default). The persona-level `ON DELETE
/// CASCADE` still removes the whole row when the persona itself goes
/// away.
///
/// (Migration slot 9 was reserved for a session_summary + triggers
/// batch that was reverted at User request without ever shipping.
/// V10 was previously kept free to preserve the append-only rule —
/// this batch reuses slot 9 because no database in the wild reached
/// user_version=9 in production; the reverted batch had only been
/// applied in a local dev session before rollback.)
const V9_PERSONA_THEME: &str = r#"
CREATE TABLE persona_theme (
    persona_id         BLOB PRIMARY KEY REFERENCES persona(id) ON DELETE CASCADE,
    wallpaper_asset_id BLOB REFERENCES asset(id) ON DELETE SET NULL,
    updated_at         INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_persona_theme_wallpaper
    ON persona_theme(wallpaper_asset_id) WHERE wallpaper_asset_id IS NOT NULL;
"#;

/// Version 9 → 10: per-persona identity metadata (`persona_profile`).
/// Kept as its own 1:1 side table so identity signal (avatar / bio
/// / role) and UI chrome (`persona_theme`) can evolve on separate
/// migration streams. `avatar_asset_id` is `ON DELETE SET NULL`
/// so deleting a referenced image quietly clears the avatar
/// rather than dropping the profile entirely; per-persona cascade
/// still removes the row when the persona itself goes.
const V10_PERSONA_PROFILE: &str = r#"
CREATE TABLE persona_profile (
    persona_id      BLOB PRIMARY KEY REFERENCES persona(id) ON DELETE CASCADE,
    avatar_asset_id BLOB REFERENCES asset(id) ON DELETE SET NULL,
    bio_short       TEXT,
    role_tag        TEXT,
    updated_at      INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_persona_profile_avatar
    ON persona_profile(avatar_asset_id) WHERE avatar_asset_id IS NOT NULL;
"#;

/// Version 10 → 11: full-text search side table (`asset_body`).
///
/// Stores the full resolved body text for every asset that has a
/// readable source (plain text file or JSONL fragment), keyed 1:1 on
/// `asset_id`. Two consumers:
///
/// - **Tantivy on-disk index** (`~/.asterism/tantivy/`) rebuilt from
///   this table by the `IndexRebuild` job when it drifts (crash /
///   version bump / manual rebuild). SQLite is the durable truth;
///   Tantivy is a derived rank-time projection.
/// - **Session Reader view** — falls back to `body_text` when the
///   original file is missing (persona-journal SQLite dumps, moved
///   Claude Code logs, and so on).
///
/// `WITHOUT ROWID` because the primary key is the natural `asset_id`
/// BLOB and every read fetches the full row. `ON DELETE CASCADE`
/// removes the body when the asset is deleted; Tantivy is cleaned up
/// asynchronously by the `IndexRebuild` job.
const V11_ASSET_BODY: &str = r#"
CREATE TABLE asset_body (
    asset_id   BLOB PRIMARY KEY REFERENCES asset(id) ON DELETE CASCADE,
    body_text  TEXT NOT NULL,
    body_bytes INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
"#;

/// Version 11 → 12: outbound dispatch — `selection` (persistent grid
/// multi-select snapshot) + `dispatch_job` (one exporter invocation
/// against a selection) + m:n `selection_asset` (ordered members).
///
/// - `selection_asset.position` preserves the order the user picked in
///   (matches the ordering discipline used by `asset_bucket`).
/// - `selection.promoted_group_id` is `ON DELETE SET NULL`: promoting
///   the selection to a Group and then deleting the Group falls back
///   to "un-promoted", never destroys the selection row (downstream
///   dispatch history keeps resolving).
/// - `dispatch_job.selection_id` cascades: deleting a Selection drops
///   its whole outbound history. The MVP has no delete surface for
///   Selections (see `SelectionService` docs), so this cascade is a
///   safety net for future admin tooling.
/// - `dispatch_job.handle_payload` is a JSON TEXT blob owned by the
///   Exporter — opaque to the DB layer.
/// - `output_asset_ids` is a JSON array of hyphenated UUIDs written
///   atomically with the `done` transition. The `derived_from` edges
///   in `edge` are the durable source of truth for lineage; this
///   column is a cached inverse for one-hop "what did this dispatch
///   produce" queries.
const V12_DISPATCH_TABLES: &str = r#"
CREATE TABLE selection (
    id                 BLOB PRIMARY KEY,
    persona_id         BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    name               TEXT,
    promoted_group_id  BLOB REFERENCES bucket(id) ON DELETE SET NULL,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_selection_persona_updated
    ON selection(persona_id, updated_at DESC);

CREATE TABLE selection_asset (
    selection_id BLOB NOT NULL REFERENCES selection(id) ON DELETE CASCADE,
    asset_id     BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    PRIMARY KEY (selection_id, asset_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_selection_asset_asset ON selection_asset(asset_id);

CREATE TABLE dispatch_job (
    id                BLOB PRIMARY KEY,
    selection_id      BLOB NOT NULL REFERENCES selection(id) ON DELETE CASCADE,
    persona_id        BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    exporter_slug     TEXT NOT NULL,
    action            TEXT NOT NULL,
    params_json       TEXT NOT NULL DEFAULT '{}',
    state_slug        TEXT NOT NULL,
    state_message     TEXT,
    progress_current  INTEGER,
    progress_total    INTEGER,
    handle_kind       TEXT,
    handle_payload    TEXT,
    output_asset_ids  TEXT NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    completed_at      INTEGER
) STRICT;

CREATE INDEX idx_dispatch_persona_created
    ON dispatch_job(persona_id, created_at DESC);
CREATE INDEX idx_dispatch_selection_created
    ON dispatch_job(selection_id, created_at DESC);
CREATE INDEX idx_dispatch_state
    ON dispatch_job(state_slug, created_at DESC);
"#;

/// Version 12 → 13: star rating on assets. `rating` is `NULL` for
/// unrated assets (the default) and 0–5 for rated ones. The UI
/// renders it as the industry-standard 5-star widget on each card
/// and via keyboard `0`–`5` on the hovered / selected card. Index
/// added so a persona-scoped "rating DESC" sort is a single seek.
const V13_ASSET_RATING: &str = r#"
ALTER TABLE asset ADD COLUMN rating INTEGER;
CREATE INDEX idx_asset_persona_rating
    ON asset(persona_id, rating DESC, occurred_at DESC)
    WHERE rating IS NOT NULL;
"#;

/// Version 13 → 14: dominant-colour palette. `palette` is a JSON
/// array of up to 5 lowercase `"#rrggbb"` strings extracted by
/// `color-thief` inside the `thumb_gen` handler; `NULL` for
/// non-image assets and rows whose thumbnail has not been produced
/// yet. Not indexed — the search / filter surface will be added
/// later (typically HSV-bucketed grouping, not exact-hex lookup).
const V14_ASSET_PALETTE: &str = r#"
ALTER TABLE asset ADD COLUMN palette TEXT;
"#;

/// Version 14 → 15: `asset_comment` — per-Asset thread of User /
/// Persona notes. `author_kind` is `'user'` or `'persona'`; when
/// `'persona'` the `author_persona_id` FK carries the identity, and
/// `ON DELETE SET NULL` keeps the comment even when the Persona is
/// deleted (renders as "(deleted persona)" downstream). Asset
/// deletion cascades to its comments. `edited_at` distinguishes
/// pristine posts from touched ones for the "(edited)" chip.
const V15_ASSET_COMMENT: &str = r#"
CREATE TABLE asset_comment (
    id                BLOB PRIMARY KEY,
    asset_id          BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    author_kind       TEXT NOT NULL,
    author_persona_id BLOB REFERENCES persona(id) ON DELETE SET NULL,
    body              TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    edited_at         INTEGER,
    CHECK (author_kind IN ('user', 'persona')),
    CHECK (
        (author_kind = 'user'    AND author_persona_id IS NULL)
     OR (author_kind = 'persona' AND author_persona_id IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_asset_comment_asset
    ON asset_comment(asset_id, created_at);
"#;

/// Version 15 → 16: `saved_query` — a named, persistent
/// `(filter, sort)` snapshot pinned in the sidebar next to Selections
/// and Groups.
///
/// - `filter_json` = serialised `ListAssetsQuery`; `sort_json` =
///   serialised `SortSpec`. Both are opaque text blobs owned by the
///   contract layer, so growing the DTO does not require a schema
///   change.
/// - `(persona_id, name)` UNIQUE — the sidebar shows one row per
///   name; duplicate names surface as `DomainError::Conflict`
///   (`409 Conflict` on the HTTP boundary) via the same
///   pattern-match the Group adapter uses.
/// - `position` orders siblings within a persona for stable sidebar
///   rendering; the covering index is scoped to the persona because
///   every sidebar query filters by it.
/// - `ON DELETE CASCADE` on `persona_id` mirrors Group and
///   Selection: dropping a persona sweeps its pinned queries.
const V16_SAVED_QUERY: &str = r#"
CREATE TABLE saved_query (
    id           BLOB PRIMARY KEY,
    persona_id   BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    filter_json  TEXT NOT NULL,
    sort_json    TEXT NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE(persona_id, name)
) STRICT;

CREATE INDEX idx_saved_query_persona_position
    ON saved_query(persona_id, position, updated_at DESC);
"#;

// Covering indexes for the index-only grid listing (`page_index`).
// The asset table rows are wide (cover / keywords / extra), so the
// old seek indexes still paid one table-page lookup per hit —
// [measured 2026-07-21] 110k rows cost ~2.0 s cold / ~0.4 s warm. Every
// column `IndexRow` selects is included so SQLite can serve the
// whole scan from the index (persona-filtered and all-personas
// variants).
const V17_ASSET_INDEX_COVERING: &str = r#"
CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at);
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at);
"#;

// Local telemetry event log (dogfooding metrics). Append-only rows
// recorded by the UI (`app_open` / `persona_switch` / `search` /
// `burst_open` / `asset_open`, kinds are open slugs) and read back by
// the UI + the HTTP API so an agent can aggregate usage summaries.
// Local-only by design — nothing ever leaves the machine.
//
// - `persona_id` carries no FK on purpose: telemetry is history and
//   must survive a persona delete (unlike Group / Selection rows).
// - `payload` is an opaque JSON string owned by the recording side;
//   the schema stays closed to churn while event kinds evolve.
const V18_EVENT_LOG: &str = r#"
CREATE TABLE event_log (
    id           BLOB PRIMARY KEY,
    kind         TEXT NOT NULL,
    occurred_at  INTEGER NOT NULL,
    persona_id   BLOB,
    duration_ms  INTEGER,
    payload      TEXT
) STRICT;

CREATE INDEX idx_event_log_kind_occurred
    ON event_log(kind, occurred_at DESC);
CREATE INDEX idx_event_log_occurred
    ON event_log(occurred_at DESC);
"#;

/// Version 18 → 19: the Selection-model redesign schema wave
/// (implementation order W2). The first migration that transforms
/// data with application logic (hashing / JSON reshaping), hence the
/// first [`Step::App`] entry:
///
/// - `selection` / `selection_asset` are reborn as `snapshot` /
///   `snapshot_asset`: pure content objects (git-tree analogue). The
///   name / updated_at / promoted_group_id columns are dropped,
///   `content_hash` (see `asterism_core::domain::snapshot_hash`) is
///   backfilled, and rows that hash identically per persona are
///   **deduped** — the oldest row survives and every reference is
///   remapped onto it (`UNIQUE(persona_id, content_hash)` could not be
///   created otherwise).
/// - `dispatch_job` is rebuilt (SQLite cannot ALTER an FK action):
///   `selection_id` becomes `snapshot_id` with `ON DELETE CASCADE`
///   flipped to `RESTRICT` (history must outlive snapshot deletion —
///   only the GC job may remove snapshots), plus the provenance columns
///   `source_group_id` / `source_query_json`.
/// - `bucket` grows `kind` ('manual' | 'query'), `query_json`, and
///   `origin_snapshot_id` (the direction-flipped transcription of the
///   old `selection.promoted_group_id`, latest writer wins).
/// - `saved_query` rows are transcribed into `kind='query'` buckets with
///   the `query_json` v1 normalisation: `search_text` leaves the
///   filter blob, `group_ids` stay raw, name collisions get a
///   `" (query)"` (+ counter) suffix. `saved_query` is then dropped.
///
/// The first evaluation of each transcribed query group would belong
/// inside this migration; that is impossible at this layer (the
/// evaluator needs the async isle + the Tantivy index, neither of which
/// exists while the raw connection is migrating), so the initial
/// evaluation runs as a **startup-blocking refresh before the UI
/// serves** — same fail-loud, same no-empty-window guarantee (W2b).
fn v19_selection_model(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    use asterism_contract::query::ListAssetsQuery;
    use asterism_contract::query_group::QueryGroupQuery;
    use asterism_contract::sort::SortSpec;
    use asterism_core::domain::snapshot_hash::content_hash;

    /// Data-shape failure inside the app migration → surfaced as a
    /// constraint-class SQLite error so the whole batch rolls back.
    fn app_err(msg: String) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some(msg),
        )
    }

    // ---- snapshot / snapshot_asset shells -----------------------------
    tx.execute_batch(
        r#"
CREATE TABLE snapshot (
    id           BLOB PRIMARY KEY,
    persona_id   BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    created_at   INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_snapshot_persona_hash
    ON snapshot(persona_id, content_hash);

CREATE TABLE snapshot_asset (
    snapshot_id BLOB NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    asset_id    BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, asset_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_snapshot_asset_asset ON snapshot_asset(asset_id);
"#,
    )?;

    // ---- legacy selection read + content_hash dedupe ------------------
    struct Sel {
        id: Uuid,
        persona: Uuid,
        promoted: Option<Uuid>,
        updated: i64,
    }
    // Oldest first so the earliest duplicate becomes the canonical row.
    let sels: Vec<(Sel, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT id, persona_id, promoted_group_id, created_at, updated_at \
             FROM selection ORDER BY created_at, id",
        )?;
        stmt.query_map([], |r| {
            Ok((
                Sel {
                    id: r.get(0)?,
                    persona: r.get(1)?,
                    promoted: r.get(2)?,
                    updated: r.get(4)?,
                },
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut members: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT selection_id, asset_id FROM selection_asset \
             ORDER BY selection_id, position",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            members.entry(row.get(0)?).or_default().push(row.get(1)?);
        }
    }
    // (persona, hash) → canonical snapshot id; remap covers *every* old
    // selection id (identity for canonical rows) so the dispatch_job
    // rebuild is a single JOIN.
    let mut canon: HashMap<(Uuid, String), Uuid> = HashMap::new();
    let mut remap: HashMap<Uuid, Uuid> = HashMap::new();
    {
        let mut ins = tx.prepare(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (sel, created) in &sels {
            let ids: Vec<String> = members
                .get(&sel.id)
                .map(|v| v.iter().map(Uuid::to_string).collect())
                .unwrap_or_default();
            let hash = content_hash(ids.iter().map(String::as_str));
            let canon_id = *canon.entry((sel.persona, hash.clone())).or_insert(sel.id);
            if canon_id == sel.id {
                ins.execute(params![sel.id, sel.persona, hash, created])?;
            }
            remap.insert(sel.id, canon_id);
        }
    }
    // Members of the canonical rows carry over verbatim (same ids).
    tx.execute_batch(
        r#"
INSERT INTO snapshot_asset (snapshot_id, asset_id, position)
SELECT selection_id, asset_id, position FROM selection_asset
 WHERE selection_id IN (SELECT id FROM snapshot);
"#,
    )?;

    // ---- dispatch_job rebuild (FK action change needs a new table) ----
    // The copy below uses a LEFT JOIN against the remap: an orphan
    // `selection_id` (impossible under the FK-ON invariant, but this is
    // history we refuse to drop silently) yields a NULL `canon_id`,
    // which the STRICT NOT NULL `snapshot_id` column rejects — a loud
    // abort instead of a silently thinner history.
    tx.execute_batch(
        "CREATE TEMP TABLE v19_remap (old_id BLOB PRIMARY KEY, canon_id BLOB NOT NULL)",
    )?;
    {
        let mut ins = tx.prepare("INSERT INTO v19_remap (old_id, canon_id) VALUES (?1, ?2)")?;
        for (old, canon_id) in &remap {
            ins.execute(params![old, canon_id])?;
        }
    }
    tx.execute_batch(
        r#"
CREATE TABLE dispatch_job_new (
    id                BLOB PRIMARY KEY,
    snapshot_id       BLOB NOT NULL REFERENCES snapshot(id) ON DELETE RESTRICT,
    persona_id        BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    exporter_slug     TEXT NOT NULL,
    action            TEXT NOT NULL,
    params_json       TEXT NOT NULL DEFAULT '{}',
    state_slug        TEXT NOT NULL,
    state_message     TEXT,
    progress_current  INTEGER,
    progress_total    INTEGER,
    handle_kind       TEXT,
    handle_payload    TEXT,
    output_asset_ids  TEXT NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    completed_at      INTEGER,
    source_group_id   BLOB REFERENCES bucket(id) ON DELETE SET NULL,
    source_query_json TEXT
) STRICT;

INSERT INTO dispatch_job_new
       (id, snapshot_id, persona_id, exporter_slug, action, params_json,
        state_slug, state_message, progress_current, progress_total,
        handle_kind, handle_payload, output_asset_ids, created_at,
        updated_at, completed_at, source_group_id, source_query_json)
SELECT d.id, r.canon_id, d.persona_id, d.exporter_slug, d.action,
       d.params_json, d.state_slug, d.state_message, d.progress_current,
       d.progress_total, d.handle_kind, d.handle_payload,
       d.output_asset_ids, d.created_at, d.updated_at, d.completed_at,
       NULL, NULL
  FROM dispatch_job d
  LEFT JOIN v19_remap r ON r.old_id = d.selection_id;

DROP TABLE dispatch_job;
ALTER TABLE dispatch_job_new RENAME TO dispatch_job;

CREATE INDEX idx_dispatch_persona_created
    ON dispatch_job(persona_id, created_at DESC);
CREATE INDEX idx_dispatch_snapshot_created
    ON dispatch_job(snapshot_id, created_at DESC);
CREATE INDEX idx_dispatch_state
    ON dispatch_job(state_slug, created_at DESC);
"#,
    )?;

    // ---- bucket: kind / query_json / origin_snapshot_id ---------------
    tx.execute_batch(
        r#"
ALTER TABLE bucket ADD COLUMN kind TEXT NOT NULL DEFAULT 'manual'
    CHECK (kind IN ('manual', 'query'));
ALTER TABLE bucket ADD COLUMN query_json TEXT;
ALTER TABLE bucket ADD COLUMN origin_snapshot_id BLOB
    REFERENCES snapshot(id) ON DELETE RESTRICT;
"#,
    )?;

    // ---- promoted_group_id direction flip (latest writer wins) --------
    {
        // bucket → (updated_at, selection_id, canonical snapshot id)
        let mut latest: HashMap<Uuid, (i64, Uuid, Uuid)> = HashMap::new();
        for (sel, _) in &sels {
            let Some(bucket) = sel.promoted else { continue };
            let canon_id = remap[&sel.id];
            let entry = latest
                .entry(bucket)
                .or_insert((sel.updated, sel.id, canon_id));
            if (sel.updated, sel.id) > (entry.0, entry.1) {
                *entry = (sel.updated, sel.id, canon_id);
            }
        }
        let mut upd = tx.prepare("UPDATE bucket SET origin_snapshot_id = ?2 WHERE id = ?1")?;
        for (bucket, (_, _, canon_id)) in &latest {
            upd.execute(params![bucket, canon_id])?;
        }
    }

    // ---- saved_query → query group transcription ----------------------
    struct Sq {
        id: Uuid,
        persona: Uuid,
        name: String,
        filter_json: String,
        sort_json: String,
        created: i64,
        updated: i64,
    }
    let sqs: Vec<Sq> = {
        let mut stmt = tx.prepare(
            "SELECT id, persona_id, name, filter_json, sort_json, created_at, updated_at \
             FROM saved_query ORDER BY persona_id, position, id",
        )?;
        stmt.query_map([], |r| {
            Ok(Sq {
                id: r.get(0)?,
                persona: r.get(1)?,
                name: r.get(2)?,
                filter_json: r.get(3)?,
                sort_json: r.get(4)?,
                created: r.get(5)?,
                updated: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    {
        let mut exists =
            tx.prepare("SELECT EXISTS(SELECT 1 FROM bucket WHERE persona_id = ?1 AND name = ?2)")?;
        let mut ins = tx.prepare(
            "INSERT INTO bucket (id, persona_id, name, description, created_at, \
                                 updated_at, dir_id, kind, query_json) \
             VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, 'query', ?6)",
        )?;
        for sq in &sqs {
            // v1 normalisation: `search_text` leaves the filter blob
            // (the legacy piggyback, App.svelte saveCurrentQuery);
            // `group_ids` stay as stored — the legacy rows carry
            // pre-expanded manual-group ids, which are valid raw ids.
            // A malformed filter blob aborts the wave
            // loudly; a malformed sort blob degrades to the default axis
            // (losing a sort is cosmetic, losing a filter is not).
            let mut filter_v: serde_json::Value =
                serde_json::from_str(&sq.filter_json).map_err(|e| {
                    app_err(format!(
                        "v19: saved_query {} filter_json unparsable: {e}",
                        sq.id
                    ))
                })?;
            let search_text = filter_v
                .as_object_mut()
                .and_then(|o| o.remove("search_text"))
                .and_then(|v| v.as_str().map(str::to_owned))
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            let filter: ListAssetsQuery = serde_json::from_value(filter_v).map_err(|e| {
                app_err(format!(
                    "v19: saved_query {} filter_json not a ListAssetsQuery: {e}",
                    sq.id
                ))
            })?;
            let sort: SortSpec = serde_json::from_str(&sq.sort_json).unwrap_or_default();
            let query_json = QueryGroupQuery::new(filter, sort, search_text)
                .to_json()
                .map_err(|e| app_err(format!("v19: query_json serialise failed: {e}")))?;

            // Name collision against UNIQUE(persona_id, name): suffix
            // " (query)", then a counter.
            let mut name = sq.name.clone();
            let mut attempt = 0u32;
            while exists.query_row(params![sq.persona, &name], |r| r.get::<_, bool>(0))? {
                attempt += 1;
                name = if attempt == 1 {
                    format!("{} (query)", sq.name)
                } else {
                    format!("{} (query {attempt})", sq.name)
                };
            }
            ins.execute(params![
                sq.id, sq.persona, name, sq.created, sq.updated, query_json
            ])?;
        }
    }

    // ---- drop the legacy tables ---------------------------------------
    tx.execute_batch(
        r#"
DROP TABLE saved_query;
DROP TABLE selection_asset;
DROP TABLE selection;
DROP TABLE v19_remap;
"#,
    )?;

    // ---- integrity gate ------------------------------------------------
    // foreign_keys is OFF for the whole App step (table rebuilds), so
    // check explicitly before the commit seals the wave.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(app_err(
            "v19: foreign_key_check reported violations after the rebuild".into(),
        ));
    }
    Ok(())
}

/// V20 — Query Group refresh outcome columns (W4-b failure
/// signal). Nullable: a bucket that never refreshed (manual groups,
/// or a query group before its first run) carries NULLs. `status` is
/// `'ok'` / `'failed'`; `error` holds the failure text for the UI
/// tooltip (NULL on success). Written by
/// `QueryGroupRepository::mark_refresh_result` after every evaluate.
const V20_BUCKET_REFRESH_SIGNAL: &str = r#"
ALTER TABLE bucket ADD COLUMN last_refresh_at INTEGER;
ALTER TABLE bucket ADD COLUMN last_refresh_status TEXT
    CHECK (last_refresh_status IN ('ok', 'failed'));
ALTER TABLE bucket ADD COLUMN last_refresh_error TEXT;
"#;

/// One migration step: a plain DDL batch, or an application-level
/// transform for waves that must hash / reshape data (first user: V19).
///
/// `App` steps run with `PRAGMA foreign_keys = OFF` (set by [`migrate`]
/// outside the transaction — inside one it is a no-op) so they can
/// rebuild tables; they are expected to end with their own
/// `PRAGMA foreign_key_check`.
enum Step {
    Sql(&'static str),
    App(fn(&Transaction<'_>) -> Result<(), rusqlite::Error>),
}

/// Version 20 → 21: `thread` / `message` tables plus a projection
/// trigger — the AppGlobal Threads primitive that replaces the old
/// `add_memo → filesystem markdown → Asset` capture path.
///
/// - `thread` carries `(anchor_kind, anchor_id)` — `AppGlobal` sits
///   on `('app_global', NULL)`, per-Snapshot / QueryGroup / Card
///   Threads carry the entity uuid. Ordering feeds two indexes:
///   `(anchor_kind, anchor_id)` for the drawer listing, and
///   `last_message_at DESC` for the "most recently active" sort the
///   catalog surfaces.
/// - `message` is append-only from the application side (there is
///   no edit verb). `idempotency_key` is nullable but forms the
///   dedupe key with `thread_id` — HTTP writers retry safely
///   without touching an existing row.
/// - The `AFTER INSERT` and `AFTER DELETE` triggers keep the
///   projection columns on `thread` in sync with `message` writes;
///   the domain layer relies on `thread.last_message_at` /
///   `message_count` being adapter-maintained so no round-trip is
///   needed after `append_message`.
/// - `ON DELETE CASCADE` from `thread` → `message` mirrors the
///   `asset_comment` / persona-scoped patterns; deleting a thread
///   sweeps its messages.
///
/// Anchor id validation is domain-side (`ThreadService`) — the
/// schema stores whatever uuid string arrives, so a phase-4
/// per-Card anchor will not need a migration.
const V21_THREADS: &str = r#"
CREATE TABLE thread (
    id              BLOB PRIMARY KEY,
    title           TEXT NOT NULL,
    anchor_kind     TEXT NOT NULL,
    anchor_id       BLOB,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    last_message_at INTEGER,
    message_count   INTEGER NOT NULL DEFAULT 0,
    archived        INTEGER NOT NULL DEFAULT 0,
    CHECK (anchor_kind IN ('app_global', 'snapshot', 'query_group', 'card')),
    CHECK (
        (anchor_kind = 'app_global' AND anchor_id IS NULL)
     OR (anchor_kind <> 'app_global' AND anchor_id IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_thread_anchor
    ON thread(anchor_kind, anchor_id, last_message_at DESC);
CREATE INDEX idx_thread_last_message
    ON thread(last_message_at DESC);

CREATE TABLE message (
    id                BLOB PRIMARY KEY,
    thread_id         BLOB NOT NULL REFERENCES thread(id) ON DELETE CASCADE,
    author_kind       TEXT NOT NULL,
    author_name       TEXT,
    -- ON DELETE CASCADE (not SET NULL): the sibling CHECK below
    -- requires `author_persona_id IS NOT NULL` whenever
    -- `author_kind = 'persona'`, so a SET-NULL cascade from
    -- persona would trip the CHECK and silently abort the
    -- persona delete. CASCADE sweeps the persona's authored
    -- messages instead — the message body is the persona's
    -- voice, and preserving it as an orphan tombstone would need
    -- a new author variant we do not want at this stage. Human,
    -- claude_code, and agent-authored messages in the same
    -- Thread are untouched.
    author_persona_id BLOB REFERENCES persona(id) ON DELETE CASCADE,
    role              TEXT NOT NULL,
    body              TEXT NOT NULL,
    refs_json         TEXT NOT NULL DEFAULT '[]',
    idempotency_key   TEXT,
    created_at        INTEGER NOT NULL,
    CHECK (author_kind IN ('human', 'claude_code', 'agent', 'persona')),
    CHECK (role IN ('note', 'action', 'system')),
    CHECK (
        (author_kind = 'agent'   AND author_name IS NOT NULL)
     OR (author_kind <> 'agent'  AND author_name IS NULL)
    ),
    CHECK (
        (author_kind = 'persona'  AND author_persona_id IS NOT NULL)
     OR (author_kind <> 'persona' AND author_persona_id IS NULL)
    ),
    UNIQUE (thread_id, idempotency_key)
) STRICT;

CREATE INDEX idx_message_thread
    ON message(thread_id, created_at);

CREATE TRIGGER trg_message_after_insert
AFTER INSERT ON message
BEGIN
    UPDATE thread
       SET message_count   = message_count + 1,
           last_message_at = NEW.created_at,
           updated_at      = NEW.created_at
     WHERE id = NEW.thread_id;
END;

-- AFTER DELETE uses the SQLite clock rather than OLD.created_at.
-- OLD.created_at is the *deleted message's* post timestamp (often
-- older than the current `thread.updated_at`), so mirroring the
-- insert-side pattern would rewind `updated_at` — breaking the
-- "monotonic Last mutation timestamp" contract that SSE catch-up
-- (P2) and any freshness-sort reader relies on. `unixepoch('subsec')`
-- returns fractional seconds; `* 1000` + CAST gives unix epoch ms
-- matching the caller-supplied `created_at` shape.
CREATE TRIGGER trg_message_after_delete
AFTER DELETE ON message
BEGIN
    UPDATE thread
       SET message_count   = MAX(message_count - 1, 0),
           last_message_at = (
               SELECT MAX(created_at) FROM message WHERE thread_id = OLD.thread_id
           ),
           updated_at      = CAST(unixepoch('subsec') * 1000 AS INTEGER)
     WHERE id = OLD.thread_id;
END;
"#;

/// Version 21 → 22: the Modality master (`modality` table) — the open
/// half of the two-layer modality model. Each row is identity +
/// presentation metadata (`slug` / `label` / `sort_order` / `hidden` /
/// `cover_template`) plus a single `kind` reference into the closed
/// `ContentKind` behaviour set. `asset.modality` carries **no** FK to
/// this table on purpose — the importer escape hatch keeps unregistered
/// slugs valid, and an unregistered slug simply renders as "unregistered"
/// on the UI side.
///
/// The seed is the current UI's 11 slugs with their kind mapping;
/// `cover_template` is set only on the three slugs that carried a
/// special-cased `cover_gen` template (`dialogue` / `work_product` /
/// `tape`), so the master reproduces the pre-master cover behaviour
/// exactly (the invariant checked by the infra tests). `sort_order`
/// follows the design's table order.
const V22_MODALITY: &str = r#"
CREATE TABLE modality (
    slug           TEXT PRIMARY KEY,
    label          TEXT NOT NULL,
    kind           TEXT NOT NULL,
    sort_order     INTEGER NOT NULL,
    hidden         INTEGER NOT NULL DEFAULT 0,
    cover_template TEXT
) STRICT;

INSERT INTO modality (slug, label, kind, sort_order, hidden, cover_template) VALUES
    ('image',        'Image',   'image', 0,  0, NULL),
    ('video',        'Video',   'video', 1,  0, NULL),
    ('audio',        'Audio',   'audio', 2,  0, NULL),
    ('dialogue',     'Dialogue', 'text', 3,  0, 'dialogue'),
    ('work_product', 'Work',     'text', 4,  0, 'work_product'),
    ('tape',         'Tape',     'term', 5,  0, 'tape'),
    ('memory',       'Memory',   'text', 6,  0, NULL),
    ('state',        'State',    'text', 7,  0, NULL),
    ('emo',          'Emo',      'text', 8,  0, NULL),
    ('non_rem',      'Non-REM',  'text', 9,  0, NULL),
    ('tick_log',     'Tick',     'text', 10, 0, NULL);
"#;

/// Version 22 → 23: `session` table — the Dialog-modality 1st-class
/// entity. Session was
/// previously "an `asset.session_id` GROUP BY projection" that
/// aggregated any modality carrying a `session_id` (tape / journal /
/// image importers all wrote it), which flooded the SessionsView with
/// per-file tape rows and per-kind journal buckets that had nothing
/// to do with a Dialog session and gave the user no per-Session
/// metadata surface. V23 stands the table up (empty); V26 back-fills
/// rows from the existing dialogue `session_id` values, V27 makes
/// `asset.session_id` a FK to it and adds the modality CHECK.
///
/// Ids are hyphenated UUID v7 strings (TEXT PRIMARY KEY) rather than
/// BLOBs because the value is re-used verbatim as `asset.session_id`
/// (which is TEXT in every schema since V1). `external_key` is the
/// importer-supplied identifier the find-or-create path resolves
/// against — UNIQUE per persona so re-imports converge idempotently.
/// `started_at_ms` / `ended_at_ms` / `message_count` are derived
/// aggregates over the participating assets (P1a seeds them at
/// migration time; P1b's `SessionRebuild` maintains them).
const V23_SESSION: &str = r#"
CREATE TABLE session (
    id            TEXT PRIMARY KEY,
    persona_id    BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    external_key  TEXT NOT NULL,
    title         TEXT,
    note          TEXT,
    cover_hint    TEXT,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms   INTEGER NOT NULL,
    message_count INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (persona_id, external_key)
) STRICT;

CREATE INDEX idx_session_persona_started
    ON session(persona_id, started_at_ms DESC);
"#;

/// Version 23 → 24: `asset.bundle_id` — the constellation-edge grouping
/// key for **non-dialogue** modalities. Session is now Dialog-only
/// (V27 CHECK), so importers that used to write a tape stem / journal
/// composite id / image PNG condition into `session_id` route
/// through this modality-agnostic slot instead. V25 back-fills it
/// from the pre-migration `session_id` values on non-dialogue assets.
///
/// The partial index mirrors the `idx_asset_session` shape (`WHERE
/// bundle_id IS NOT NULL`) so a bundle-scoped read costs a single
/// seek and the majority of assets (no bundle) do not bloat the
/// index.
const V24_ASSET_BUNDLE_ID: &str = r#"
ALTER TABLE asset ADD COLUMN bundle_id TEXT;

CREATE INDEX idx_asset_bundle
    ON asset(bundle_id) WHERE bundle_id IS NOT NULL;
"#;

/// Version 24 → 25: move the pre-Session-entity `session_id` values
/// carried by **non-dialogue** modalities (tape / journal / image /
/// future slot) into the fresh `bundle_id` column. The pre-migration
/// SessionsView flood came from these rows (dogfood snapshot 2026-07-25:
/// 81 tape + 4 journal-kind = 85 tiles); after this batch they still
/// group as constellation-edge siblings via `bundle_id`, but the
/// SessionsView surface is free to filter to `modality = 'dialogue'`.
///
/// V27's CHECK (`session_id IS NULL OR modality = 'dialogue'`)
/// depends on this batch running first — every non-dialogue row must
/// have `session_id = NULL` before the CHECK is installed.
const V25_MIGRATE_NON_DIALOGUE_SESSION_ID: &str = r#"
UPDATE asset
   SET bundle_id = session_id,
       session_id = NULL
 WHERE session_id IS NOT NULL
   AND modality <> 'dialogue';
"#;

/// Version 25 → 26: replace each pre-migration dialogue `session_id`
/// value (an importer-supplied raw string — Claude Code UUID, JSONL
/// stem, …) with the UUID of a freshly-minted `session` row. The
/// participating asset rows all get their `session_id` UPDATE'd to
/// the new UUID so V27's FK (`session_id REFERENCES session(id)`) is
/// satisfied.
///
/// Aggregation shape: for each `(persona_id, session_id)` group we
/// mint one Session row, seed its aggregates from
/// `MIN(occurred_at)` / `MAX(occurred_at)` / `COUNT(*)`, insert it,
/// then update every participating asset to point at the new UUID.
/// A `WHERE session_id = <old_key> AND persona_id = <p> AND modality
/// = 'dialogue'` predicate scopes the update so a re-run against a
/// partially-migrated DB is idempotent (already-migrated rows carry
/// UUID-shaped `session_id` values that do not collide with the raw
/// external keys).
///
/// Runs as an `App` step because it (a) mints application-side UUIDs
/// and (b) does the row-by-row correlated update inside one
/// transaction, which SQL-only migrations cannot express.
fn v26_dialogue_session_backfill(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    // Group every dialogue-modality asset that still carries a raw
    // session_id (post-V25) by (persona, session_id). One
    // Session row is minted per group.
    struct Group {
        persona_id: Uuid,
        external_key: String,
        min_ms: i64,
        max_ms: i64,
        count: i64,
    }
    let groups: Vec<Group> = {
        let mut stmt = tx.prepare(
            "SELECT persona_id, session_id, MIN(occurred_at), MAX(occurred_at), COUNT(*) \
             FROM asset \
             WHERE session_id IS NOT NULL AND modality = 'dialogue' \
             GROUP BY persona_id, session_id \
             ORDER BY persona_id, session_id",
        )?;
        stmt.query_map([], |row| {
            Ok(Group {
                persona_id: row.get(0)?,
                external_key: row.get(1)?,
                min_ms: row.get(2)?,
                max_ms: row.get(3)?,
                count: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut ins_session = tx.prepare(
        "INSERT INTO session \
            (id, persona_id, external_key, title, note, cover_hint, \
             started_at_ms, ended_at_ms, message_count, created_at_ms, updated_at_ms) \
         VALUES (?1, ?2, ?3, NULL, NULL, NULL, ?4, ?5, ?6, ?7, ?7)",
    )?;
    let mut upd_asset = tx.prepare(
        "UPDATE asset \
            SET session_id = ?1 \
          WHERE persona_id = ?2 AND session_id = ?3 AND modality = 'dialogue'",
    )?;

    for g in &groups {
        let new_id = Uuid::now_v7().to_string();
        ins_session.execute(params![
            new_id,
            g.persona_id,
            g.external_key,
            g.min_ms,
            g.max_ms,
            g.count,
            now_ms,
        ])?;
        upd_asset.execute(params![new_id, g.persona_id, g.external_key])?;
    }

    Ok(())
}

/// Version 26 → 27: install the modality invariant + FK on
/// `asset.session_id`. SQLite cannot ALTER a table to add a CHECK or
/// change an FK action, so the canonical SOP is table-rebuild:
/// `CREATE asset_new` with the desired constraints → `INSERT ...
/// SELECT ... FROM asset` (V25 + V26 already normalised the values,
/// so the SELECT feeds a clean payload) → drop the old table →
/// rename → **re-create every index and trigger the pre-rebuild
/// table carried**.
///
/// Constraint deltas:
///
/// - `session_id TEXT REFERENCES session(id) ON DELETE SET NULL` —
///   deleting a Session (only allowed when empty, per
///   `SessionRepository::delete_if_empty`) clears the reference; the
///   asset itself survives.
/// - `CHECK (session_id IS NULL OR modality = 'dialogue')` — Session
///   is Dialog-only from here on; non-dialogue clustering uses
///   `bundle_id`.
///
/// Indexes recreated verbatim from V1 / V2 / V13 / V17 / V24:
/// `idx_asset_persona_occurred`, `idx_asset_persona_modality_occurred`,
/// `idx_asset_occurred`, `idx_asset_session`, `idx_asset_source_unique`,
/// `idx_asset_persona_rating`, `idx_asset_persona_occurred_cover`,
/// `idx_asset_occurred_cover`, `idx_asset_bundle`. There are no
/// trigger definitions on the `asset` table (V9 trigger draft was
/// reverted before shipping).
///
/// Runs as an `App` step because [`migrate`] toggles `foreign_keys =
/// OFF` around the transaction — required for a table rebuild with FK
/// action changes. The batch ends with `PRAGMA foreign_key_check` so
/// V25 / V26 mistakes surface loudly here rather than at the next
/// caller-side write.
fn v27_asset_session_fk_check(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        r#"
CREATE TABLE asset_new (
    id              BLOB PRIMARY KEY,
    persona_id      BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    source_kind     TEXT NOT NULL,
    source_locator  TEXT NOT NULL,
    file_size_bytes INTEGER,
    platform        TEXT,
    modality        TEXT NOT NULL,
    labels          TEXT NOT NULL DEFAULT '[]',
    occurred_at     INTEGER NOT NULL,
    session_id      TEXT REFERENCES session(id) ON DELETE SET NULL,
    cover           TEXT,
    keywords        TEXT NOT NULL DEFAULT '[]',
    register_note   TEXT,
    vis_restricted  INTEGER NOT NULL DEFAULT 0,
    vis_sharing     TEXT NOT NULL DEFAULT '[]',
    duration_ms     INTEGER,
    extra           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    has_code        INTEGER NOT NULL DEFAULT 0,
    has_table       INTEGER NOT NULL DEFAULT 0,
    has_mermaid     INTEGER NOT NULL DEFAULT 0,
    has_link        INTEGER NOT NULL DEFAULT 0,
    rating          INTEGER,
    palette         TEXT,
    bundle_id       TEXT,
    CHECK (session_id IS NULL OR modality = 'dialogue')
) STRICT;

INSERT INTO asset_new
    (id, persona_id, source_kind, source_locator, file_size_bytes, platform,
     modality, labels, occurred_at, session_id, cover, keywords, register_note,
     vis_restricted, vis_sharing, duration_ms, extra, created_at, updated_at,
     has_code, has_table, has_mermaid, has_link, rating, palette, bundle_id)
SELECT
    id, persona_id, source_kind, source_locator, file_size_bytes, platform,
    modality, labels, occurred_at, session_id, cover, keywords, register_note,
    vis_restricted, vis_sharing, duration_ms, extra, created_at, updated_at,
    has_code, has_table, has_mermaid, has_link, rating, palette, bundle_id
  FROM asset;

DROP TABLE asset;
ALTER TABLE asset_new RENAME TO asset;

-- Recreate every index the pre-rebuild `asset` table carried
-- (V1 / V2 / V13 / V17 / V24). There are no triggers on `asset`
-- (V9's session_summary triggers were reverted before shipping).
CREATE INDEX idx_asset_persona_occurred
    ON asset(persona_id, occurred_at DESC);
CREATE INDEX idx_asset_persona_modality_occurred
    ON asset(persona_id, modality, occurred_at DESC);
CREATE INDEX idx_asset_occurred
    ON asset(occurred_at DESC);
CREATE INDEX idx_asset_session
    ON asset(session_id) WHERE session_id IS NOT NULL;
CREATE UNIQUE INDEX idx_asset_source_unique
    ON asset(source_kind, source_locator);
CREATE INDEX idx_asset_persona_rating
    ON asset(persona_id, rating DESC, occurred_at DESC)
    WHERE rating IS NOT NULL;
CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at);
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at);
CREATE INDEX idx_asset_bundle
    ON asset(bundle_id) WHERE bundle_id IS NOT NULL;
"#,
    )?;

    // Guard: V25 / V26 mistakes must surface here, not at the next
    // caller-side write. `foreign_keys` is OFF for the whole App
    // batch (table rebuild), so an explicit check is required before
    // the commit seals the wave.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v27: foreign_key_check reported violations after the rebuild".into()),
        ));
    }
    Ok(())
}

/// Version 27 → 28: composite-Asset columns. session-model v2 promotes
/// a Session from a standalone `session` entity to a **composite Asset**
/// (an Asset that owns member Assets via `container_id`). This batch
/// adds the three columns the composite model needs, all plain
/// `ALTER ... ADD COLUMN` (no rebuild, mirroring V24's `bundle_id`):
///
/// - `container_id BLOB` — composition membership: the id of the
///   composite Asset this row is a member of (self-reference into
///   `asset`). Exclusive 1:n / provenance axis, distinct from the m:n
///   `bucket` (Group) filing. The self-FK (`REFERENCES asset(id) ON
///   DELETE SET NULL`) is deferred to the contract-phase rebuild that
///   also drops the legacy `session_id` column + its CHECK; kept as a
///   bare column here so this batch stays a cheap ALTER.
/// - `title TEXT` — user-authored name (the composite's Session title),
///   a slot separate from the auto-derived `cover`.
/// - `external_key TEXT` — the composite's idempotent re-import key
///   (carried over from `session.external_key`).
///
/// Also seeds the `session` modality into the master (`kind =
/// 'composition'`) so the composite renders with a proper label; the
/// existing `dialogue` (single-message) modality is untouched.
const V28_ASSET_COMPOSITE_COLUMNS: &str = r#"
ALTER TABLE asset ADD COLUMN container_id BLOB;
ALTER TABLE asset ADD COLUMN title TEXT;
ALTER TABLE asset ADD COLUMN external_key TEXT;

CREATE INDEX idx_asset_container
    ON asset(container_id) WHERE container_id IS NOT NULL;

INSERT INTO modality (slug, label, kind, sort_order, hidden, cover_template) VALUES
    ('session', 'Session', 'composition', 11, 0, NULL);
"#;

/// Version 28 → 29: materialise one composite Asset per `session` row
/// and repoint that session's dialogue members onto it via
/// `container_id`.
///
/// Key move: the pre-v2 `asset.session_id` already holds `session.id`,
/// so the composite Asset is minted with `id = session.id` (the same
/// hyphenated UUID, parsed to the 16-byte BLOB `asset.id` uses). Then
/// `UPDATE asset SET container_id = <that id> WHERE session_id =
/// session.id` repoints every member with **zero id remapping** — the
/// member's future `container_id` equals its current `session_id`.
///
/// The composite carries the run's metadata copied from the session
/// row: `title` ← `session.title`, `cover` ← `session.cover_hint`,
/// `register_note` ← `session.note`, `external_key` ←
/// `session.external_key`, `occurred_at` ← `started_at_ms`. Its
/// `modality = 'session'` and it carries no `session_id` of its own
/// (top-level), so the V27 Dialog-only CHECK (`session_id IS NULL OR
/// modality = 'dialogue'`) is satisfied. `source = ('session',
/// session.id)` gives it a globally-unique `idx_asset_source_unique`
/// key.
///
/// expand-contract: the legacy `session` table, `asset.session_id`
/// column, and the Dialog-only CHECK are **left in place** here; the
/// contract-phase migration removes them (and installs the
/// `container_id` self-FK) once the read/write paths have switched.
/// Runs as an `App` step because it mints/parses UUIDs and does a
/// correlated per-session update, which SQL-only batches cannot
/// express. `foreign_keys` is OFF for the batch; a final
/// `foreign_key_check` guards against an orphaned `persona_id`.
fn v29_materialise_session_composites(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    struct SessionRow {
        id: String,
        persona_id: Vec<u8>,
        external_key: String,
        title: Option<String>,
        note: Option<String>,
        cover_hint: Option<String>,
        started_at_ms: i64,
        created_at_ms: i64,
        updated_at_ms: i64,
    }
    let sessions: Vec<SessionRow> = {
        let mut stmt = tx.prepare(
            "SELECT id, persona_id, external_key, title, note, cover_hint, \
             started_at_ms, created_at_ms, updated_at_ms \
             FROM session",
        )?;
        stmt.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                persona_id: row.get(1)?,
                external_key: row.get(2)?,
                title: row.get(3)?,
                note: row.get(4)?,
                cover_hint: row.get(5)?,
                started_at_ms: row.get(6)?,
                created_at_ms: row.get(7)?,
                updated_at_ms: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };

    let mut ins_asset = tx.prepare(
        "INSERT INTO asset \
            (id, persona_id, source_kind, source_locator, modality, occurred_at, \
             title, cover, register_note, external_key, created_at, updated_at) \
         VALUES (?1, ?2, 'session', ?3, 'session', ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    let mut upd_members = tx.prepare("UPDATE asset SET container_id = ?1 WHERE session_id = ?2")?;

    for s in &sessions {
        // Reuse session.id (hyphenated UUID text) as the composite
        // Asset's BLOB id, so `container_id = old session_id` needs no
        // remap. A malformed id is a hard migration error.
        let composite_id: Vec<u8> = Uuid::parse_str(&s.id)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
            .as_bytes()
            .to_vec();
        ins_asset.execute(params![
            composite_id,
            s.persona_id,
            s.id,
            s.started_at_ms,
            s.title,
            s.cover_hint,
            s.note,
            s.external_key,
            s.created_at_ms,
            s.updated_at_ms,
        ])?;
        upd_members.execute(params![composite_id, s.id])?;
    }

    // Guard: a composite's persona_id must reference a live persona.
    // FK enforcement is OFF for the App batch, so check explicitly
    // before the commit seals the wave.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v29: foreign_key_check reported violations after materialise".into()),
        ));
    }
    Ok(())
}

/// Version 32 → 33: trash / purge verb separation.
///
/// Numbered 33 rather than 30 because `main` claimed 30-32 for the
/// session-model v2 wave while this branch was open. The index in
/// [`MIGRATIONS`] **is** the `user_version`, so two branches naming the
/// same number for different SQL is the one merge hazard here that a
/// clean textual merge would not surface: a database migrated on either
/// branch would skip the other's step and then fail on a duplicate
/// column. Renumbering on integration is mandatory, not cosmetic.
///
/// `trashed_at` (epoch ms, `NULL` = live) splits one destructive verb
/// into two: `trash` stamps the column, `purge` physically deletes —
/// and only a stamped row may be purged. The row deliberately stays in
/// the table while trashed, because all ten `ON DELETE CASCADE`
/// children of `asset` (asset_tag / edge ×2 / thumb_cache /
/// asset_bucket **and its hand-arranged `position`** / session_summary /
/// asset_body / snapshot_asset / asset_comment) then survive untouched.
/// Restore is a single `UPDATE`; no value-copying machinery is needed,
/// and nothing can drift out of sync with a future schema change.
///
/// `bucket` gets the same column for the same reason: deleting a Group
/// discards its name, its Dir filing, and the drag-ordered
/// `asset_bucket.position` sequence — the most expensive thing in this
/// schema to reproduce by hand.
///
/// Both indexes are **partial** (`WHERE trashed_at IS NOT NULL`): they
/// serve the trash listing and the retention sweep while staying tiny,
/// since the live hot path filters `trashed_at IS NULL` and is served by
/// the existing `idx_asset_persona_occurred` family.
///
/// Additive `ALTER TABLE` only — no table rebuild, so this runs as a
/// plain SQL step.
const V33_TRASH_COLUMNS: &str = r#"
ALTER TABLE asset  ADD COLUMN trashed_at INTEGER;
ALTER TABLE bucket ADD COLUMN trashed_at INTEGER;

CREATE INDEX idx_asset_trashed
    ON asset(trashed_at) WHERE trashed_at IS NOT NULL;
CREATE INDEX idx_bucket_trashed
    ON bucket(trashed_at) WHERE trashed_at IS NOT NULL;
"#;

/// Version 33 → 34: the persona side of the trash (deleting a persona
/// puts its assets in the trash with it, rather than destroying them).
///
/// Deleting a persona used to be the one destructive path that skipped
/// the trash entirely: `asset.persona_id` is `ON DELETE CASCADE`, so a
/// single `DELETE FROM persona` physically removed every asset that
/// persona ever held — ratings, comments, group filings and all — with
/// no recovery and no search-index cleanup. Now the persona itself goes
/// to the trash and takes its assets with it, and only the retention
/// sweep (or an explicit purge) makes it final.
///
/// `trashed_at` is deliberately **not** `archived`: archiving is a
/// user-facing "hide this persona from the sidebar" toggle over live
/// data, while `trashed_at` marks data on its way out. Conflating them
/// would make un-archiving indistinguishable from restoring.
///
/// The partial index mirrors `idx_asset_trashed` / `idx_bucket_trashed`.
const V34_PERSONA_TRASH: &str = r#"
ALTER TABLE persona ADD COLUMN trashed_at INTEGER;

CREATE INDEX idx_persona_trashed
    ON persona(trashed_at) WHERE trashed_at IS NOT NULL;
"#;

/// Version 34 → 35: `diag_log` — persisted diagnostics, the sink behind
/// the `tracing` subscriber.
///
/// Deliberately *not* `event_log`. That table is shaped for product
/// metrics — `(kind, persona_id, duration_ms, payload)` records
/// `app_open` / `persona_switch` / `search` and friends, and every
/// column carries interaction meaning. A diagnostic has no persona and
/// no duration; folding it in would blur what `event_log` is and would
/// bury five meaningful measurements under background warnings the
/// moment anything renders that list.
///
/// `fields` is the JSON-serialised structured part of a `tracing`
/// event, kept separate from the human-readable `message` so a future
/// reader can filter on a field without parsing prose.
///
/// Local-only and append-only, same contract as `event_log`: rows never
/// leave the machine, and nothing updates them in place.
const V35_DIAG_LOG: &str = r#"
CREATE TABLE diag_log (
    id           BLOB PRIMARY KEY,
    occurred_at  INTEGER NOT NULL,
    level        TEXT NOT NULL,
    target       TEXT NOT NULL,
    message      TEXT NOT NULL,
    fields       TEXT
) STRICT;

CREATE INDEX idx_diag_log_occurred
    ON diag_log(occurred_at DESC);
CREATE INDEX idx_diag_log_level_occurred
    ON diag_log(level, occurred_at DESC);
"#;

/// Version 29 → 30: partial UNIQUE index enforcing one composite per
/// `(persona_id, external_key)`. session-model v2 moves the Session's
/// idempotent re-import key from the old `session` table's
/// `UNIQUE(persona_id, external_key)` onto the composite Asset's
/// `external_key` column, so `SessionRepository::find_or_create` keeps
/// its "one Session per external key, Conflict on race" contract now
/// that composites live in `asset`. Partial (`WHERE external_key IS NOT
/// NULL`) so it only constrains composites — regular assets carry
/// `external_key = NULL` and are unaffected. The V29-materialised
/// composites already satisfy uniqueness (they inherited it from the
/// `session` table's own unique constraint), so this index builds
/// cleanly over existing data.
const V30_ASSET_EXTERNAL_KEY_UNIQUE: &str = r#"
CREATE UNIQUE INDEX idx_asset_external_key
    ON asset(persona_id, external_key) WHERE external_key IS NOT NULL;
"#;

/// Version 30 → 31: contract phase of session-model v2. The Session
/// read/write path is fully composite-Asset-backed (Cycle 5a), so the
/// legacy scaffolding is now dead and removed:
///
/// - drop the `asset.session_id` column and its Dialog-only CHECK
///   (`session_id IS NULL OR modality = 'dialogue'`, V27) — membership
///   is expressed through `container_id`, which is modality-agnostic;
/// - install the `container_id` **self-FK** (`REFERENCES asset(id) ON
///   DELETE SET NULL`) so deleting a composite clears its members'
///   pointer instead of dangling;
/// - drop the now-unused `session` table.
///
/// SQLite cannot ALTER away a column that participates in a CHECK, nor
/// add an FK, so this is the canonical table-rebuild (mirrors V27):
/// `CREATE asset_new` with the target shape → `INSERT ... SELECT`
/// (every column except `session_id`) → drop old → rename → recreate
/// every index except `idx_asset_session`. Runs as an `App` step
/// because [`migrate`] toggles `foreign_keys = OFF` for the rebuild;
/// the batch ends with `PRAGMA foreign_key_check` so a dangling
/// `container_id` surfaces here rather than at the next write.
fn v31_drop_session_scaffolding(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        r#"
CREATE TABLE asset_new (
    id              BLOB PRIMARY KEY,
    persona_id      BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    source_kind     TEXT NOT NULL,
    source_locator  TEXT NOT NULL,
    file_size_bytes INTEGER,
    platform        TEXT,
    modality        TEXT NOT NULL,
    labels          TEXT NOT NULL DEFAULT '[]',
    occurred_at     INTEGER NOT NULL,
    cover           TEXT,
    keywords        TEXT NOT NULL DEFAULT '[]',
    register_note   TEXT,
    vis_restricted  INTEGER NOT NULL DEFAULT 0,
    vis_sharing     TEXT NOT NULL DEFAULT '[]',
    duration_ms     INTEGER,
    extra           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    has_code        INTEGER NOT NULL DEFAULT 0,
    has_table       INTEGER NOT NULL DEFAULT 0,
    has_mermaid     INTEGER NOT NULL DEFAULT 0,
    has_link        INTEGER NOT NULL DEFAULT 0,
    rating          INTEGER,
    palette         TEXT,
    bundle_id       TEXT,
    container_id    BLOB REFERENCES asset(id) ON DELETE SET NULL,
    title           TEXT,
    external_key    TEXT
) STRICT;

INSERT INTO asset_new
    (id, persona_id, source_kind, source_locator, file_size_bytes, platform,
     modality, labels, occurred_at, cover, keywords, register_note,
     vis_restricted, vis_sharing, duration_ms, extra, created_at, updated_at,
     has_code, has_table, has_mermaid, has_link, rating, palette, bundle_id,
     container_id, title, external_key)
SELECT
    id, persona_id, source_kind, source_locator, file_size_bytes, platform,
    modality, labels, occurred_at, cover, keywords, register_note,
    vis_restricted, vis_sharing, duration_ms, extra, created_at, updated_at,
    has_code, has_table, has_mermaid, has_link, rating, palette, bundle_id,
    container_id, title, external_key
  FROM asset;

DROP TABLE asset;
ALTER TABLE asset_new RENAME TO asset;

DROP TABLE session;

-- Recreate every index the pre-rebuild `asset` table carried, EXCEPT
-- idx_asset_session (the session_id column is gone).
CREATE INDEX idx_asset_persona_occurred
    ON asset(persona_id, occurred_at DESC);
CREATE INDEX idx_asset_persona_modality_occurred
    ON asset(persona_id, modality, occurred_at DESC);
CREATE INDEX idx_asset_occurred
    ON asset(occurred_at DESC);
CREATE UNIQUE INDEX idx_asset_source_unique
    ON asset(source_kind, source_locator);
CREATE INDEX idx_asset_persona_rating
    ON asset(persona_id, rating DESC, occurred_at DESC)
    WHERE rating IS NOT NULL;
CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at);
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at);
CREATE INDEX idx_asset_bundle
    ON asset(bundle_id) WHERE bundle_id IS NOT NULL;
CREATE INDEX idx_asset_container
    ON asset(container_id) WHERE container_id IS NOT NULL;
CREATE UNIQUE INDEX idx_asset_external_key
    ON asset(persona_id, external_key) WHERE external_key IS NOT NULL;
"#,
    )?;

    // Guard: a dangling container_id (member pointing at a
    // non-existent composite) must surface here, not at the next write.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v31: foreign_key_check reported violations after the rebuild".into()),
        ));
    }
    Ok(())
}

/// Version 31 → 32: `app_setting` — user overrides for the closed
/// setting registry in `asterism_core::domain::app_setting`.
///
/// The table stores **only overrides**: a key the user never changed has
/// no row and resolves to the code default. That keeps "reset" a
/// `DELETE`, and it lets a later change to a default reach every profile
/// that had not pinned the key.
///
/// `key` is TEXT rather than a slug BLOB because the values are dotted
/// namespaced identifiers (`ui.clean_mode`), and there is no FK to
/// anything: the registry lives in code, so a row whose key a build no
/// longer recognises is simply skipped on read (downgrade tolerance)
/// instead of blocking the whole listing.
///
/// `value_json` holds the value as canonicalised JSON text. A per-kind
/// column set (`bool_value` / `int_value` / …) was rejected: it would
/// push the kind check into the schema while the registry already owns
/// it in code, giving two sources of truth for the same rule.
///
/// Settings live in the profile database on purpose — preferences then
/// travel with a profile backup and stay isolated between `dev` /
/// `dogfood` / `bench`, which `localStorage` could not do.
const V32_APP_SETTING: &str = r#"
CREATE TABLE app_setting (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
"#;

/// Version 35 → 36: the four observation streams.
///
/// Replaces `event_log` + `diag_log`, whose boundary did not match a
/// real distinction: `diag_log` had become "everything that is not a
/// user action", mixing startup narration, swallowed errors and
/// per-listing performance timings behind a single `level` column.
///
/// Four tables because four properties differ across them — writer,
/// volume, value per row, and retention. That is also what makes
/// per-stream policy expressible at all; one table would push those
/// decisions into runtime branches. A single SQLite connection is
/// shared by every feature here, so keeping the high-volume stream
/// (`perf_log`, two rows per grid listing) in its own table also keeps
/// its writes and its retention deletes off the others' backs. The
/// one-timeline property that a single table would have given is
/// recovered on the read side by `observation` below.
///
/// Shared envelope on every stream: `id` / `occurred_at` / `env` /
/// `event` / `attrs` / `correlation_id`. `env` is new and is the column
/// whose absence let a DEV-only perf probe run in production. `event`
/// is a namespaced name (`job.cover_gen.failed`) that identifies the
/// record's type and therefore the shape of `attrs`; its first segment
/// is the stream, so a misfiled row is visible on sight.
///
/// Columns each stream always filters on are promoted out of `attrs`:
/// a value worth comparing *across* records earns a column, a value
/// meaningful only within one `event` stays in the JSON.
///
/// Tags live in per-stream side tables rather than a JSON array,
/// because multi-axis selection is the point: `tag IN (…)` joined once
/// per required tag gives AND semantics an index can serve.
///
/// Existing rows are carried over, not dropped: `event_log` becomes
/// `action_log` (its `kind` was already an `action.*`-shaped slug, so
/// it is prefixed rather than reinterpreted), and `diag_log` splits by
/// target — the two `perf:` records the previous build emitted go to
/// `perf_log`, everything else to the new `diag_log`.
const V36_OBSERVATION_STREAMS: &str = r#"
CREATE TABLE action_log (
    id             BLOB PRIMARY KEY,
    occurred_at    INTEGER NOT NULL,
    env            TEXT NOT NULL,
    event          TEXT NOT NULL,
    attrs          TEXT,
    correlation_id TEXT,
    persona_id     BLOB,
    duration_ms    INTEGER
) STRICT;
CREATE INDEX idx_action_log_occurred ON action_log(occurred_at DESC);
CREATE INDEX idx_action_log_event_occurred ON action_log(event, occurred_at DESC);

CREATE TABLE job_log (
    id             BLOB PRIMARY KEY,
    occurred_at    INTEGER NOT NULL,
    env            TEXT NOT NULL,
    event          TEXT NOT NULL,
    attrs          TEXT,
    correlation_id TEXT,
    task_id        TEXT NOT NULL,
    job_kind       TEXT NOT NULL,
    outcome        TEXT NOT NULL,
    attempt        INTEGER NOT NULL,
    duration_ms    INTEGER
) STRICT;
CREATE INDEX idx_job_log_occurred ON job_log(occurred_at DESC);
CREATE INDEX idx_job_log_kind_occurred ON job_log(job_kind, occurred_at DESC);
CREATE INDEX idx_job_log_outcome_occurred ON job_log(outcome, occurred_at DESC);

CREATE TABLE diag_log_v2 (
    id             BLOB PRIMARY KEY,
    occurred_at    INTEGER NOT NULL,
    env            TEXT NOT NULL,
    event          TEXT NOT NULL,
    attrs          TEXT,
    correlation_id TEXT,
    level          TEXT NOT NULL,
    target         TEXT NOT NULL,
    message        TEXT NOT NULL
) STRICT;
-- Indexed after the rename below, so the names match the final table
-- rather than the scaffolding one.

CREATE TABLE perf_log (
    id             BLOB PRIMARY KEY,
    occurred_at    INTEGER NOT NULL,
    env            TEXT NOT NULL,
    event          TEXT NOT NULL,
    attrs          TEXT,
    correlation_id TEXT,
    op             TEXT NOT NULL,
    duration_ms    INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_perf_log_occurred ON perf_log(occurred_at DESC);
CREATE INDEX idx_perf_log_op_occurred ON perf_log(op, occurred_at DESC);

CREATE TABLE action_log_tag (
    record_id BLOB NOT NULL REFERENCES action_log(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    PRIMARY KEY (record_id, tag)
) STRICT;
CREATE INDEX idx_action_log_tag_tag ON action_log_tag(tag);

CREATE TABLE job_log_tag (
    record_id BLOB NOT NULL REFERENCES job_log(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    PRIMARY KEY (record_id, tag)
) STRICT;
CREATE INDEX idx_job_log_tag_tag ON job_log_tag(tag);

CREATE TABLE diag_log_tag (
    record_id BLOB NOT NULL REFERENCES diag_log_v2(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    PRIMARY KEY (record_id, tag)
) STRICT;
CREATE INDEX idx_diag_log_tag_tag ON diag_log_tag(tag);

CREATE TABLE perf_log_tag (
    record_id BLOB NOT NULL REFERENCES perf_log(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    PRIMARY KEY (record_id, tag)
) STRICT;
CREATE INDEX idx_perf_log_tag_tag ON perf_log_tag(tag);

INSERT INTO action_log (id, occurred_at, env, event, attrs, correlation_id, persona_id, duration_ms)
SELECT id, occurred_at, 'unknown', 'action.' || kind, payload, NULL, persona_id, duration_ms
  FROM event_log;

INSERT INTO perf_log (id, occurred_at, env, event, attrs, correlation_id, op, duration_ms)
SELECT id, occurred_at, 'unknown', 'perf.list_index', fields, NULL, 'list_index',
       -- The old call sites named their timing after the phase rather
       -- than the column. Zero here would not read as "unknown", it
       -- would read as "instant" and silently drag every average down.
       COALESCE(
           json_extract(fields, '$.db_total_ms'),
           json_extract(fields, '$.domain_map_ms'),
           0)
  FROM diag_log
 WHERE message LIKE 'perf:%';

INSERT INTO diag_log_v2 (id, occurred_at, env, event, attrs, correlation_id, level, target, message)
SELECT id, occurred_at, 'unknown', 'diag.legacy', fields, NULL, level, target, message
  FROM diag_log
 WHERE message NOT LIKE 'perf:%';

DROP TABLE event_log;
DROP TABLE diag_log;
ALTER TABLE diag_log_v2 RENAME TO diag_log;
CREATE INDEX idx_diag_log_occurred ON diag_log(occurred_at DESC);
CREATE INDEX idx_diag_log_level_occurred ON diag_log(level, occurred_at DESC);

CREATE VIEW observation AS
    SELECT 'action' AS stream, id, occurred_at, env, event, attrs, correlation_id FROM action_log
    UNION ALL
    SELECT 'job',    id, occurred_at, env, event, attrs, correlation_id FROM job_log
    UNION ALL
    SELECT 'diag',   id, occurred_at, env, event, attrs, correlation_id FROM diag_log
    UNION ALL
    SELECT 'perf',   id, occurred_at, env, event, attrs, correlation_id FROM perf_log;
"#;

/// Version 36 → 37: the Material layer (asset-model v4, P1).
///
/// First slice of the Material / Asset / Card three-layer split.
/// The `asset` row keeps playing the logical-management-unit role; the
/// new `material` table carries the physical-original layer: locator,
/// byte size, and the mime fact. `format` is deliberately **not** an
/// asset column — asking "is this an image?" is a question for the
/// material, which is what dissolves the old `ContentKind`
/// format/structure conflation.
///
/// Shape decisions:
///
/// - PK is `(asset_id, ord)`: a material is an aggregate-internal
///   entity identified through its owning asset (PhotoKit's
///   `PHAssetResource` shape), so it needs no surrogate id — which is
///   also what lets this batch stay a plain SQL step (no UUID minting,
///   compare `v29_materialise_session_composites`). `ord` leaves the
///   1 asset : N materials room open (RAW+JPEG / Live Photo pattern)
///   even though P1 operates strictly at one material per item.
/// - `asset.role` ('item' | 'collection') lands here, backfilled from
///   `modality = 'session'` **while that slug still exists** — P2
///   deletes the slug, so the structural fact must be captured first.
/// - Collections get no material row: a container has no bytes of its
///   own (its members do). This is the v4 invariant
///   "role=Collection ⟹ no material".
/// - `asset.source_locator` / `file_size_bytes` are **copied, not
///   moved**. The grid hot path reads them through covering indexes
///   (`idx_asset_*_cover`) and `AssetCardDto.source_locator` is a
///   required field consumed by the UI and the exporter SDK; the read
///   side migrates one path at a time in P3+.
/// - mime backfill is best-effort fact capture: text-kind modalities
///   (per the master's `kind`) become `text/plain`, media files map
///   from their extension, everything else stays NULL ("unknown", not
///   "not applicable"). Ingest-time writes refine this going forward.
const V37_MATERIAL_LAYER: &str = r#"
ALTER TABLE asset ADD COLUMN role TEXT NOT NULL DEFAULT 'item'
    CHECK (role IN ('item', 'collection'));

UPDATE asset SET role = 'collection' WHERE modality = 'session';

CREATE TABLE material (
    asset_id        BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    ord             INTEGER NOT NULL,
    locator         TEXT NOT NULL,
    file_size_bytes INTEGER,
    mime            TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (asset_id, ord)
) STRICT, WITHOUT ROWID;

INSERT INTO material (asset_id, ord, locator, file_size_bytes, mime, created_at, updated_at)
SELECT a.id, 0, a.source_locator, a.file_size_bytes,
       CASE
           WHEN m.kind IN ('text', 'term')                THEN 'text/plain'
           WHEN lower(a.source_locator) LIKE '%.png'      THEN 'image/png'
           WHEN lower(a.source_locator) LIKE '%.jpg'      THEN 'image/jpeg'
           WHEN lower(a.source_locator) LIKE '%.jpeg'     THEN 'image/jpeg'
           WHEN lower(a.source_locator) LIKE '%.gif'      THEN 'image/gif'
           WHEN lower(a.source_locator) LIKE '%.webp'     THEN 'image/webp'
           WHEN lower(a.source_locator) LIKE '%.mp4'      THEN 'video/mp4'
           WHEN lower(a.source_locator) LIKE '%.mov'      THEN 'video/quicktime'
           WHEN lower(a.source_locator) LIKE '%.webm'     THEN 'video/webm'
           WHEN lower(a.source_locator) LIKE '%.mp3'      THEN 'audio/mpeg'
           WHEN lower(a.source_locator) LIKE '%.wav'      THEN 'audio/wav'
           WHEN lower(a.source_locator) LIKE '%.m4a'      THEN 'audio/mp4'
           ELSE NULL
       END,
       a.created_at, a.updated_at
  FROM asset a
  LEFT JOIN modality m ON m.slug = a.modality
 WHERE a.role = 'item';
"#;

/// Version 37 → 38: modality becomes optional; the conversation slugs
/// and the format-shaped master rows disappear (asset-model v4, P2).
///
/// `modality` is demoted to what it always should have been: the
/// user's semantic classification (Tape / Memory / Emo / …), NULL when
/// unclassified. The three questions it used to answer in one column
/// are now three axes: data format lives on `material.mime` (V37),
/// container structure on `asset.role` (V37), and this batch removes
/// the rows that encoded those foreign axes as classifications:
///
/// - `dialogue` / `session` — structure masquerading as modality. The
///   role backfill already captured the fact (V37), so their assets
///   drop to `modality = NULL` here and the master rows go.
/// - `image` / `video` / `audio` — format masquerading as modality
///   (0 live assets in dogfood; any dev-profile stragglers drop to
///   NULL, their format is already on the material).
/// - `test_mod` — test residue, and the `sort_order` collision with
///   the late `session` row disappears with both.
///
/// `SQLite` cannot drop `NOT NULL`, so this is the canonical rebuild
/// (mirrors V31): `CREATE asset_new` → `INSERT … SELECT` → drop →
/// rename → recreate every index. Runs as an `App` step because
/// [`migrate`] toggles `foreign_keys = OFF` around it; the batch ends
/// with `PRAGMA foreign_key_check`.
fn v38_modality_optional(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        r#"
CREATE TABLE asset_new (
    id              BLOB PRIMARY KEY,
    persona_id      BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    source_kind     TEXT NOT NULL,
    source_locator  TEXT NOT NULL,
    file_size_bytes INTEGER,
    platform        TEXT,
    modality        TEXT,
    labels          TEXT NOT NULL DEFAULT '[]',
    occurred_at     INTEGER NOT NULL,
    cover           TEXT,
    keywords        TEXT NOT NULL DEFAULT '[]',
    register_note   TEXT,
    vis_restricted  INTEGER NOT NULL DEFAULT 0,
    vis_sharing     TEXT NOT NULL DEFAULT '[]',
    duration_ms     INTEGER,
    extra           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    has_code        INTEGER NOT NULL DEFAULT 0,
    has_table       INTEGER NOT NULL DEFAULT 0,
    has_mermaid     INTEGER NOT NULL DEFAULT 0,
    has_link        INTEGER NOT NULL DEFAULT 0,
    rating          INTEGER,
    palette         TEXT,
    bundle_id       TEXT,
    container_id    BLOB REFERENCES asset(id) ON DELETE SET NULL,
    title           TEXT,
    external_key    TEXT,
    trashed_at      INTEGER,
    role            TEXT NOT NULL DEFAULT 'item'
        CHECK (role IN ('item', 'collection'))
) STRICT;

INSERT INTO asset_new
    (id, persona_id, source_kind, source_locator, file_size_bytes, platform,
     modality, labels, occurred_at, cover, keywords, register_note,
     vis_restricted, vis_sharing, duration_ms, extra, created_at, updated_at,
     has_code, has_table, has_mermaid, has_link, rating, palette, bundle_id,
     container_id, title, external_key, trashed_at, role)
SELECT
    id, persona_id, source_kind, source_locator, file_size_bytes, platform,
    modality, labels, occurred_at, cover, keywords, register_note,
    vis_restricted, vis_sharing, duration_ms, extra, created_at, updated_at,
    has_code, has_table, has_mermaid, has_link, rating, palette, bundle_id,
    container_id, title, external_key, trashed_at, role
  FROM asset;

DROP TABLE asset;
ALTER TABLE asset_new RENAME TO asset;

CREATE INDEX idx_asset_persona_occurred
    ON asset(persona_id, occurred_at DESC);
CREATE INDEX idx_asset_persona_modality_occurred
    ON asset(persona_id, modality, occurred_at DESC);
CREATE INDEX idx_asset_occurred
    ON asset(occurred_at DESC);
CREATE UNIQUE INDEX idx_asset_source_unique
    ON asset(source_kind, source_locator);
CREATE INDEX idx_asset_persona_rating
    ON asset(persona_id, rating DESC, occurred_at DESC)
    WHERE rating IS NOT NULL;
CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at);
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at);
CREATE INDEX idx_asset_bundle
    ON asset(bundle_id) WHERE bundle_id IS NOT NULL;
CREATE INDEX idx_asset_container
    ON asset(container_id) WHERE container_id IS NOT NULL;
CREATE UNIQUE INDEX idx_asset_external_key
    ON asset(persona_id, external_key) WHERE external_key IS NOT NULL;
CREATE INDEX idx_asset_trashed
    ON asset(trashed_at) WHERE trashed_at IS NOT NULL;

UPDATE asset SET modality = NULL
 WHERE modality IN ('dialogue', 'session', 'image', 'video', 'audio', 'test_mod');

DELETE FROM modality
 WHERE slug IN ('dialogue', 'session', 'image', 'video', 'audio', 'test_mod');
"#,
    )?;

    // Guard: the rebuild must not orphan a container_id / persona_id.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v38: foreign_key_check reported violations after the rebuild".into()),
        ));
    }
    Ok(())
}

/// Version 38 → 39: drop the `ui.dialogue.show_messages` override row.
///
/// The setting died with the Dialogue slug (asset-model v4 P3): the
/// grid no longer interleaves members, they live inside their
/// container's reader. The registry entry is gone from
/// `SETTING_REGISTRY`, so a surviving override row would be an
/// unreadable orphan — the registry-driven read path ignores unknown
/// keys, but leaving dead rows contradicts the "closed registry"
/// contract the table exists to keep.
const V39_DROP_SHOW_MESSAGES_SETTING: &str = r#"
DELETE FROM app_setting WHERE key = 'ui.dialogue.show_messages';
"#;

/// Version 39 → 40: `asset_color` — the palette facet's index.
///
/// `asset.palette` has carried five dominant-colour hex values per
/// asset since V14, but a hex is not a filter: no two photographs share
/// one. This table holds the quantised form
/// ([`asterism_core::domain::color`]) so "show me the red ones" is an
/// equality predicate with an index behind it, and so the sidebar can
/// count assets per swatch the way the FORMAT facet counts formats.
///
/// A **projection, not a source of truth**: `asset.palette` stays
/// canonical and the rows here are derived from it (this backfill now,
/// `set_palette` from `thumb_gen` afterwards). Dropping the table's
/// contents costs nothing that a re-derivation cannot restore.
///
/// An `App` step because the quantisation reads hex arithmetic that
/// SQL cannot express, and because a palette that fails to parse must
/// be skipped rather than abort the batch — a corrupt JSON blob is a
/// row `thumb_gen` will rewrite, not a reason to block the schema.
///
/// **Trap for a future asset-table rebuild**: `asset_color` hangs off
/// `asset(id)` with `ON DELETE CASCADE`. The canonical rebuild
/// (`CREATE asset_new` → `INSERT … SELECT` → `DROP` → `RENAME`, see
/// [`v38_modality_optional`]) is safe **only** as a `Step::App`, where
/// [`migrate`] holds `foreign_keys = OFF` and the FK re-binds by name
/// after the rename. Written as a `Step::Sql` the same batch runs with
/// foreign keys ON, and `DROP TABLE asset` silently empties this table.
/// The palettes would survive and the facet would go blank.
fn v40_asset_color(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        r#"
CREATE TABLE asset_color (
    asset_id BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    bucket   TEXT NOT NULL,
    PRIMARY KEY (asset_id, bucket)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_asset_color_bucket ON asset_color(bucket);
"#,
    )?;

    let rows: Vec<(Uuid, String)> = {
        let mut stmt = tx.prepare("SELECT id, palette FROM asset WHERE palette IS NOT NULL")?;
        stmt.query_map([], |row| {
            Ok((row.get::<_, Uuid>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut insert =
        tx.prepare("INSERT OR IGNORE INTO asset_color (asset_id, bucket) VALUES (?1, ?2)")?;
    for (asset_id, palette) in rows {
        let Ok(hexes) = serde_json::from_str::<Vec<String>>(&palette) else {
            continue;
        };
        for bucket in asterism_core::domain::color::buckets_of(hexes.iter().map(String::as_str)) {
            insert.execute(params![asset_id, bucket.as_str()])?;
        }
    }
    Ok(())
}

/// Version 40 → 41: `material.content_hash` — the duplicate axis.
///
/// Asset identity is `UNIQUE(source_kind, source_locator)`, which only
/// ever answered "have I seen this *path*". The same photograph under
/// two paths is two assets, and nothing in the schema could say so.
/// This column carries a fingerprint of the bytes
/// ([`asterism_core::domain::content_hash`]), turning "the same
/// picture" into a `GROUP BY`.
///
/// Deliberately **not** `UNIQUE`: a duplicate is something to show the
/// user, not something to refuse. A unique index here would make the
/// second copy fail to import, which is the behaviour that loses data
/// silently — the copy is already on disk either way.
///
/// The column starts NULL everywhere; the `material_hash` job fills it
/// in (per-asset at ingest, and in a chained backfill pass for
/// everything already imported). NULL means unknown, never unique.
const V41_MATERIAL_CONTENT_HASH: &str = r#"
ALTER TABLE material ADD COLUMN content_hash TEXT;

CREATE INDEX idx_material_content_hash
    ON material(content_hash) WHERE content_hash IS NOT NULL;
"#;

/// Version 41 → 42: the `session` modality — what a container *is
/// about*, as opposed to what it structurally *is*.
///
/// V38 deleted the old `session` row because it encoded structure on
/// the semantic axis (`kind = 'composition'`), and structure belongs to
/// `asset.role`. That removal was right but incomplete: nothing took
/// over the semantic question, so every container ended up unclassified
/// — no MODALITY row, no badge, no name. The only thing marking one as
/// a conversation was `source_kind = 'session'`, which is provenance,
/// not classification [measured 2026-07-29: 4 containers, all with
/// `modality IS NULL` and `title IS NULL`].
///
/// The slug returns with a different job. `role = 'collection'` says
/// "this is a container"; `modality = 'session'` says "what it holds is
/// a run of exchanges" — a coding session, a chat, a conversation with
/// a tool. The two axes are orthogonal: an album of photos would be the
/// same role with a different modality.
///
/// `kind = 'text'` because the members read as text; the container owns
/// no material of its own, so nothing here decides how bytes render
/// (that is `render_policy`'s job, keyed on mime).
const V42_SESSION_MODALITY: &str = r#"
INSERT INTO modality (slug, label, kind, sort_order, hidden, cover_template)
VALUES ('session', 'Session', 'text', 3, 0, 'dialogue');

-- Backfill: every existing container is a session (the only container
-- Asterism mints today comes from a conversation importer).
UPDATE asset SET modality = 'session'
 WHERE role = 'collection' AND modality IS NULL;
"#;

/// Version 42 → 43: the `message` modality — the other half of what V42
/// started.
///
/// V42 gave containers a semantic classification; their members still
/// had none. That was tolerable only while the grid hid members behind
/// a `top_level` filter: the moment every Asset is a Card (as it now
/// is), an unclassified member is a row that no facet can reach. The
/// invariant this pair of migrations buys is simple — **every Asset
/// carries a modality, so every Asset is reachable by category**.
///
/// Ordering: `session` (3) then `message` (4), coarse before fine. The
/// rows below shift down one slot to make room; `sort_order` is display
/// order only, so renumbering is safe.
const V43_MESSAGE_MODALITY: &str = r#"
UPDATE modality SET sort_order = sort_order + 1 WHERE sort_order >= 4;

INSERT INTO modality (slug, label, kind, sort_order, hidden, cover_template)
VALUES ('message', 'Message', 'text', 4, 0, 'dialogue');

-- Backfill: anything filed inside a container is a message of it.
UPDATE asset SET modality = 'message'
 WHERE container_id IS NOT NULL AND modality IS NULL;
"#;

/// Version 43 → 44: `modality.kind` → `modality.terminal`.
///
/// The 2-layer model put behaviour behind a closed `ContentKind` so
/// that data could never claim a capability the code lacked. Asset-model
/// v4 then moved the actual behaviour elsewhere — thumbnails and media
/// rendering to the material's mime, containment to `asset.role` — and
/// what remained was one question mime cannot answer: is this text a
/// terminal transcript?
///
/// By then the column was mostly fiction. The backend never read it
/// (both `render_policy` calls pass `terminal: false` outright), the UI
/// read it in one place, and `kind = 'text'` on eight of nine rows meant
/// nothing beyond "not term". Adding a `photo` classification would have
/// required writing `kind = 'text'` on it — a lie, to satisfy a column
/// that decides nothing.
///
/// So the column becomes what it actually is: one bit. A closed set of
/// two values is a bool, and `terminal` says which. If a second display
/// mode ever earns its place, it comes back as a slug column pointing at
/// a closed enum — the shape `cover_template` already uses.
const V44_MODALITY_TERMINAL_BIT: &str = r#"
CREATE TABLE modality_new (
    slug           TEXT PRIMARY KEY,
    label          TEXT NOT NULL,
    sort_order     INTEGER NOT NULL,
    hidden         INTEGER NOT NULL DEFAULT 0,
    cover_template TEXT,
    terminal       INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO modality_new (slug, label, sort_order, hidden, cover_template, terminal)
SELECT slug, label, sort_order, hidden, cover_template,
       CASE WHEN kind = 'term' THEN 1 ELSE 0 END
  FROM modality;

DROP TABLE modality;
ALTER TABLE modality_new RENAME TO modality;
"#;

/// Version 44 → 45: repair the mime of fragment locators.
///
/// A locator carrying a fragment (`shot.png#workflow` — a PNG tEXt
/// note, `session.jsonl#uuid` — one message) addresses a record inside
/// a container, and what the asset stands for is the extracted text,
/// never the container's format. `guess_mime` used to strip the
/// fragment and read the container extension, so every PNG tEXt note
/// imported before this was filed `image/png`; the thumbnail job then
/// took the locator for a path and failed on all of them
/// [measured 2026-07-31: 2785 failed rows in the dogfood job_log]. The
/// classification is fixed at the source (`asterism_core::domain::
/// material::guess_mime`); the rows already written need this.
///
/// Idempotent by shape — the `UPDATE` is a no-op the second time — so
/// it costs nothing on a database that never carried a bad row.
const V45_FRAGMENT_MATERIAL_MIME: &str = r#"
UPDATE material
   SET mime = 'text/plain'
 WHERE locator LIKE '%#%'
   AND mime IS NOT NULL
   AND mime <> 'text/plain';
"#;

/// Version 45 → 46: index the modification stamp so differential sync
/// stops being a full scan.
///
/// `ListAssetsQuery::updated_from_ms` is the "what changed since I last
/// looked" cursor, and its natural caller is a poll loop: the same query
/// re-issued on a timer, each time asking for a window that holds almost
/// nothing. Without an index every one of those polls reads the whole
/// `asset` table to return the handful of rows that moved.
///
/// `DESC` matches how the column is read on both sides — the
/// `SortTarget::UpdatedAt` axis orders most-recently-changed first, and a
/// cursor walk asks for the tail. The index is unconditional (no partial
/// `WHERE`): the column is `NOT NULL` on every row, so there is no subset
/// to carve out, and the polling query does not necessarily carry a
/// persona (an external agent syncs the library, not a sidebar).
///
/// **`created_at` is deliberately left unindexed.** Ingest time is
/// assigned by the writer at insert, so it is effectively monotonic with
/// rowid: a `created_from_ms` window is a suffix of the table in physical
/// order, and the scan the planner already does behaves like a range
/// probe. `updated_at` has no such property — an edit moves an arbitrarily
/// old row to the head of the modification order, which is exactly why it
/// needs its own structure. If ingest polling ever shows up in a profile,
/// it gets its own migration with that measurement attached.
const V46_ASSET_UPDATED_INDEX: &str = r#"
CREATE INDEX idx_asset_updated
    ON asset(updated_at DESC);
"#;

/// Version 46 → 47: attribution columns — who a row is by, and which
/// agent operated on their behalf.
///
/// The table could say where an artefact came from (`source_*`,
/// `_trace`) and who may look at it (`vis_*`), but nothing recorded who
/// wrote it. In a library driven by several agents under one person
/// that is the question an audit asks first, and neither of the
/// existing axes answers it: origin is not authorship, and visibility
/// is about readers.
///
/// `author_kind` / `author_subject` are the pair form of
/// [`Author`](asterism_core::domain::attribution::Author) — `'owner'`
/// with no subject, or `'subject'` with one. `operator_ai` is an open
/// slug (`claude-code`, `codex`, `asterism-ui`); the set of things that
/// can drive Asterism is not closed, so it is not an enum.
///
/// **No CHECK constraint.** SQLite cannot attach a table-level check to
/// a column added by `ALTER TABLE`, and rebuilding `asset` (the V44
/// dance) to gain one would be a large migration for a rule the domain
/// already enforces: every read goes through
/// `Author::from_columns`, which rejects a pair `Author::encode` cannot
/// produce (owner-with-subject, subject-without-one, unknown kind).
///
/// All three start NULL and stay NULL until the write paths are wired.
/// NULL means **unrecorded** — not "authored by the owner". Defaulting
/// to the owner would erase the difference between an assertion and a
/// fill-in, which is exactly what a hosted migration would have to
/// recover; the column follows `material.content_hash` (V41) in leaving
/// the unanswered question visible.
const V47_ASSET_ATTRIBUTION: &str = r#"
ALTER TABLE asset ADD COLUMN author_kind TEXT;
ALTER TABLE asset ADD COLUMN author_subject TEXT;
ALTER TABLE asset ADD COLUMN operator_ai TEXT;
"#;

/// Version 47 → 48: the agent that started a dispatch.
///
/// A dispatch outlives the request that created it — the exporter is
/// polled by a background job, and the outputs are reified minutes or
/// hours later. The operator the caller asserted has to be *on the row*
/// for the runner to stamp it on what the run produces; passing it
/// along in memory would lose it on every restart, which is the one
/// thing a long-running dispatch does routinely.
///
/// Same open-slug vocabulary as `asset.operator_ai`
/// ([`OperatorRef`](asterism_core::domain::attribution::OperatorRef)),
/// and the same meaning for NULL: unrecorded, not "the person at the
/// keyboard". Rows written before this column exists keep it NULL —
/// their operator is not knowable after the fact, and a backfill would
/// be forging exactly the bookkeeping the column is for.
const V48_DISPATCH_OPERATOR: &str = r#"
ALTER TABLE dispatch_job ADD COLUMN operator_ai TEXT;
"#;

/// Version 48 → 49: the instance identity record — who `author_kind =
/// 'owner'` actually refers to.
///
/// V47 could store "the owner" but nothing in the schema said which
/// owner, so the value was a word rather than a reference. This table
/// gives it a referent: one profile database is one Asterism instance,
/// and an instance has exactly one owner
/// ([`InstanceIdentity`](asterism_core::domain::instance::InstanceIdentity)).
///
/// - `CHECK (id = 0)` with `PRIMARY KEY` is how the singleton is
///   enforced by the schema instead of by convention: a second row is
///   rejected by SQLite, not by a code path somebody has to remember.
/// - `instance_id` is a minted UUID v7 — hence a [`Step::App`], since a
///   `Step::Sql` batch cannot mint one (same reason
///   [`v19_selection_model`] is an app step).
/// - `owner_subject` starts NULL and means **unbound**, not unknown: a
///   local instance has no authenticated subject, and writing a
///   placeholder would put a value where the question is. Authentication
///   binds it once.
///
/// The insert runs exactly once because the step itself runs exactly
/// once (`user_version` gates it), and the singleton constraint keeps
/// that true even if a future path tried again.
fn v49_instance_identity(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        r#"
CREATE TABLE instance (
    id            INTEGER PRIMARY KEY CHECK (id = 0),
    instance_id   BLOB NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL,
    owner_subject TEXT
) STRICT;
"#,
    )?;
    tx.execute(
        "INSERT INTO instance (id, instance_id, created_at) VALUES (0, ?1, ?2)",
        params![Uuid::now_v7(), chrono::Utc::now().timestamp_millis()],
    )?;
    Ok(())
}

/// Version 49 → 50: the channel an attribution arrived through, plus
/// the author columns `dispatch_job` was missing.
///
/// `attributed_via` is what makes an authenticated deployment possible
/// without rewriting history: `'owner-surface'` / `'asserted'` /
/// `'authenticated'`
/// ([`AttributionChannel`](asterism_core::domain::attribution::AttributionChannel))
/// records *how* the answer arrived, so a claim by an HTTP caller stays
/// distinguishable from the owner's own app having said it. Without the
/// column every pre-auth row would be indistinguishable from an
/// authenticated one and would have to be discarded as unresolvable.
///
/// `dispatch_job` gains the author pair for the same reason it already
/// carries `operator_ai` (V48): the run outlives its request, and the
/// reified outputs are stamped from the row.
///
/// NULL semantics, three cases that must not be conflated:
///
/// - **all columns NULL** — an ordinary unrecorded write (a background
///   job, or a caller that asserted nothing).
/// - **author / operator set, `attributed_via` NULL** — a row written
///   under V47 / V48, before the channel was tracked. Read as legacy and
///   left that way; a backfill would be inventing the one fact the
///   column exists to record.
/// - **`attributed_via` set** — the channel is known and the row is
///   resolvable at authentication time.
///
/// No CHECK constraint, for the reason V47 gives: `ALTER TABLE ADD
/// COLUMN` cannot carry one. The rule "a recorded author or operator
/// carries a channel" is enforced on the write side in the repository
/// row builders, the same place `Author::from_columns` guards the read.
const V50_ATTRIBUTION_CHANNEL: &str = r#"
ALTER TABLE asset ADD COLUMN attributed_via TEXT;
ALTER TABLE dispatch_job ADD COLUMN author_kind TEXT;
ALTER TABLE dispatch_job ADD COLUMN author_subject TEXT;
ALTER TABLE dispatch_job ADD COLUMN attributed_via TEXT;
"#;

/// Version 50 → 51: `folded_into` (the tombstone) + `fold_policy`.
///
/// Two rows can hold the same bytes under two paths, and until now the
/// only way to resolve that was to trash one of them. Trash is the
/// wrong instrument: retention *physically deletes* what it holds
/// (`scan_purgeable` → `purge`), and the losing row is exactly what has
/// to survive — every stale reference to it (an old UUID in a sidecar
/// claim, a `<locator>#<keyword>` fragment in a PNG note, a dispatch
/// record) resolves through the row that stayed behind. `folded_into`
/// is that row: NULL = a live asset, non-NULL = a headstone pointing at
/// the keeper.
///
/// `BLOB`, not `TEXT`, despite what the subtask spec wrote: `asset` is
/// a `STRICT` table whose `id` is `BLOB PRIMARY KEY`, so a `TEXT`
/// column could not hold a value that ever compares equal to an id.
/// The precedent is `container_id` (V28), the other self-reference.
/// Like that column it is added bare, without `REFERENCES asset(id)` —
/// `ALTER TABLE ADD COLUMN` can carry a REFERENCES clause, but the
/// cascade action worth having here (`ON DELETE RESTRICT`, so purging a
/// keeper cannot silently orphan its headstones) belongs with the same
/// table rebuild V28 deferred its own self-FK to.
///
/// `fold_policy` is the durable half of the ask/fold/separate
/// decision: `'auto'` = nobody has ruled, `'keep'` =
/// a person looked at this pair and said they are different things, so
/// the conflict must not be raised again. It is `NOT NULL DEFAULT
/// 'auto'` because "unruled" is a real answer rather than an absence,
/// which is the opposite of what V47's attribution columns needed.
///
/// **The column-level CHECK holds** — measured, not assumed
/// (`v51_folds_are_marked_and_the_policy_is_checked`). V47 recorded
/// that SQLite cannot attach a *table-level* check to a column added by
/// `ALTER TABLE`; a *column-level* one is a different clause and is not
/// on the list of things `ADD COLUMN` refuses, so the closed set is
/// enforced by the database here and `FoldPolicy::parse` is the second
/// reader rather than the only one. The check binds new writes only —
/// no existing row is re-validated — which costs nothing for a column
/// every existing row acquires by default.
///
/// The partial index is on the **non-NULL** side, the mirror image of
/// `idx_asset_container` / `idx_material_content_hash`: headstones are
/// the rare rows, and the query that needs the structure is "who does
/// this fold point at" rather than "list the live ones" (that one is a
/// `WHERE folded_into IS NULL` conjunct on a scan the other predicates
/// already drive).
const V51_ASSET_FOLD: &str = r#"
ALTER TABLE asset ADD COLUMN folded_into BLOB;
ALTER TABLE asset ADD COLUMN fold_policy TEXT NOT NULL DEFAULT 'auto'
    CHECK (fold_policy IN ('auto', 'keep'));

CREATE INDEX idx_asset_folded_into
    ON asset(folded_into) WHERE folded_into IS NOT NULL;
"#;

/// Version 51 → 52: `on_duplicate` — the strategy declared at
/// registration for a conflict that has not happened yet.
///
/// `add` returns without reading a byte; the fingerprint that can raise
/// a duplicate is computed later by the `MaterialHash` job. So by the
/// time there is anything to decide, the command that carried the
/// caller's intent is gone. The column is where it waits — the same
/// problem, and the same answer, as `dispatch_job.operator_ai` (V48):
/// a runner that starts after its caller has returned can only read
/// what is on the row.
///
/// **Nullable, and NULL is not `'ask'`.** V47's attribution columns
/// established the reading and this column takes it verbatim: NULL
/// means *unrecorded* — nobody declared a strategy for this
/// registration. That matters because the resolution ladder puts an
/// importer / lane setting and a persona default underneath the
/// request, and only an undeclared row is free to pick those up. Defaulting to `'ask'` at write time would
/// forge a request nobody made, and the day the lower layers exist
/// there would be no way to tell the forged ones from the real ones.
/// Neither of those layers is implemented yet, so today every
/// undeclared row resolves to the single built-in default — which is
/// what makes writing the default *in* look free, and is exactly the
/// point at which V41 / V47 recorded that it is not.
///
/// The CHECK is column-level, the form V51 measured to survive
/// `ALTER TABLE ADD COLUMN` (a table-level one does not). It admits
/// NULL without naming it: SQLite fails a CHECK only when it evaluates
/// to false, and `NULL IN (…)` is NULL, not false. Spelling
/// `on_duplicate IS NULL OR …` would read as though absence needed
/// permission, when what it needs is to not be an answer;
/// `v52_a_declared_strategy_is_checked_and_absence_is_allowed` pins
/// both halves so the subtlety is measured rather than trusted.
///
/// No index. The reader is the hash job resolving one asset it already
/// holds the id of, and the fold verb reading a row it has in hand —
/// neither scans on this column, unlike `folded_into`, whose partial
/// index answers "who does this fold point at".
const V52_ASSET_ON_DUPLICATE: &str = r#"
ALTER TABLE asset ADD COLUMN on_duplicate TEXT
    CHECK (on_duplicate IN ('ask', 'fold', 'separate'));
"#;

/// Version 52 → 53: `duplicate_conflict` — the queue of raised "are
/// these two the same thing?" questions.
///
/// The fact that two rows hold the same bytes is already recorded, as an
/// `identical_to` edge. This table holds the part of the event the edge
/// cannot: whether anybody still has to decide about it. The edge is
/// written on all three strategies — including the two that ask nothing
/// — so "an edge exists" and "a question is open" are different
/// statements, and deriving the second from the first would put every
/// deliberately-separate pair in front of a user forever.
///
/// # The key is the unordered pair
///
/// `UNIQUE (pair_lo, pair_hi, axis)` over the *sorted* id pair, with the
/// direction kept beside it in `newcomer_id` / `incumbent_id`. Detection
/// has a direction (the row whose fingerprint just landed is the
/// newcomer); the question does not. Which end raises it depends on
/// which row was fingerprinted first, which depends on whether the bytes
/// arrived through an import or through the backfill walk — keying on
/// the ordered pair would let the same two rows queue twice, once from
/// each end, and a user would answer the same question a second time.
/// The columns are materialised rather than expressed as a generated
/// column so the uniqueness is a plain index over stored values, and the
/// domain's [`pair_key`][pair-key] is the single place the ordering is
/// decided.
///
/// `axis` is in the key because the two fingerprints answer different
/// questions ("every byte" versus "the bytes that decide the decoded
/// result"), and a pair may legitimately raise both. Only `'file'` has a
/// producer today; `'content'` is in the CHECK from the start because
/// this column exists precisely to tell them apart.
///
/// # Resolution closes the row, it does not delete it
///
/// `resolved_at` + `resolution`, both NULL while the question is open.
/// Deleting on resolve would be smaller and would lose the one thing
/// worth keeping — that a conflict was raised and somebody ruled on it
/// — which is the same judgement that keeps the `identical_to` edge
/// after a `keep` ruling. The partial index is on the open side, since
/// the panel reads open questions and answered ones accumulate.
///
/// **A pair whose rows have gone is not a resolution.** Nothing here
/// records "one side was folded / trashed": the reader joins both assets
/// and drops the pair while either is a headstone or in the trash. The
/// trash is reversible, so a stamped-in verdict would have to be
/// un-stamped by whatever restores the row; re-deriving it costs a join
/// on a query that runs when a person opens a panel.
///
/// Both asset ids and the persona cascade on delete, like every other
/// asset-referencing table since V1 (`edge`, `asset_tag`,
/// `asset_comment`): a purge that left conflict rows behind would leave
/// the panel unable to hydrate either side.
///
/// [pair-key]: asterism_core::domain::duplicate_conflict::DuplicateConflict::pair_key
const V53_DUPLICATE_CONFLICT: &str = r#"
CREATE TABLE duplicate_conflict (
    id           BLOB PRIMARY KEY,
    persona_id   BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    pair_lo      BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    pair_hi      BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    newcomer_id  BLOB NOT NULL,
    incumbent_id BLOB NOT NULL,
    axis         TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    detected_at  INTEGER NOT NULL,
    resolved_at  INTEGER,
    resolution   TEXT,
    UNIQUE (pair_lo, pair_hi, axis),
    CHECK (axis IN ('file', 'content')),
    CHECK (resolution IS NULL OR resolution IN ('folded', 'kept')),
    CHECK ((resolved_at IS NULL) = (resolution IS NULL)),
    CHECK (pair_lo <> pair_hi)
) STRICT;

CREATE INDEX idx_duplicate_conflict_open
    ON duplicate_conflict(persona_id, detected_at DESC)
    WHERE resolved_at IS NULL;
"#;

/// Version 53 → 54: `duplicate_conflict.fold_exclusion` — why a pair a
/// lane asked to fold was queued instead.
///
/// The exclusion rules (a pair connected through `derived_from`, a
/// pair one of whose rows is the output of an export run) stop the
/// **automatic** fold and nothing more. The pair still reaches a
/// person, and the manual merge verb is deliberately not bound by
/// them. Without this column the row that reaches
/// the panel is indistinguishable from any other `ask`, so the first
/// thing a person does with a question the rule declined to answer is
/// answer it by hand — never having been told there was a rule.
///
/// # Why on the row and not derived when the panel opens
///
/// The same reason `content_hash` sits here: the row records what was
/// found, so the queue can be read without redoing the finding. And
/// re-deriving would not give the same answer — the `derived_from`
/// graph is written by ingest and by reify and keeps growing, so a
/// panel opened a week later would compute a verdict about a graph the
/// detection never saw. What belongs on the row is what was true when
/// the fold was declined; what belongs at merge time is the merge
/// verb's own warning, computed then, against the graph as it is
/// then.
///
/// NULL is the ordinary state and means "no automatic fold was
/// declined" — nobody asked for one, or the pass was the reason (a
/// conflict the backfill finds is never folded on its own, which is a
/// fact about when the pair was noticed rather than about the pair).
///
/// The CHECK is column-level, the form V51 measured to survive
/// `ALTER TABLE ADD COLUMN`, and it admits NULL without naming it —
/// `NULL IN (…)` is NULL, not false (V52 records the same subtlety).
///
/// No index. The reader is the panel, which already selects on
/// `resolved_at IS NULL` and reads this off the rows it got.
const V54_DUPLICATE_CONFLICT_FOLD_EXCLUSION: &str = r#"
ALTER TABLE duplicate_conflict ADD COLUMN fold_exclusion TEXT
    CHECK (fold_exclusion IN ('lineage', 'dispatch'));
"#;

/// Version 54 → 55: `material.content_region_hash` — the second
/// duplicate axis, and the marker that keeps the first launch after it
/// from reading the whole library.
///
/// V41 added `content_hash`, which is a digest of **every byte** and so
/// answers "is this the same file". Two exports of one picture that
/// differ only in a `tEXt` chunk — a ComfyUI workflow blob, an
/// exporter's timestamp — are not the same file and are the same
/// picture [measured: 9 such groups in a 4,601-image corpus,
/// `fixture-measurements.md`]. This column holds the digest over only
/// the bytes that decide the decoded result
/// ([`asterism_core::domain::content_region`]), under its own versioned
/// tag so a later redefinition of "the content region" cannot be
/// compared against values computed under this one.
///
/// A separate column rather than a rename of the old one: `content_hash`
/// is the wire name on the asset DTO, the MCP and HTTP payloads, the
/// generated TypeScript bindings and the duplicates panel, so renaming
/// it to `file_hash` would land a sweep of every surface in the diff
/// that adds the axis.
///
/// # Why the migration writes to every row
///
/// The fingerprint walk finds work by "the content column holds no
/// answer". Every row that exists when this runs holds NULL, so without
/// the `UPDATE` every pre-existing row would join that walk — the pass
/// for material that has just arrived would silently become a re-read of
/// the whole corpus, counted in the "still fingerprinting" notice as
/// ordinary ingest work. **The two are different acts**, and keeping
/// them apart is what this `UPDATE` buys.
///
/// So every pre-existing row is answered here, with
/// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED) —
/// "nothing walked these bytes", which is exactly true. Those rows keep
/// their file-axis digest and their file-axis duplicate grouping, and
/// the marker is what makes them a *selectable set*, since NULL would be
/// indistinguishable from a row that arrived a minute ago.
///
/// That set is the deferred half of this migration, and
/// [`v56_walk_deferred_content_regions`] is where it is finished: the
/// values are computed in the next step of the same chain, one read per
/// row, because reading files cannot be done in the statement that adds
/// a column. Between the two steps no launch happens, so the marker is
/// a state the running application never has to be correct about.
///
/// Rows inserted afterwards get NULL from the `INSERT` (which names
/// neither hash column — `set_material_fingerprint` owns both) and are
/// picked up by the walk normally.
///
/// # The index
///
/// Same shape as `idx_material_content_hash` (V41) and for the same
/// consumer: the duplicate report groups by this column and would
/// otherwise scan `material` whole. Partial on the non-NULL side, which
/// after the `UPDATE` above excludes only the rows that arrive later and
/// have not been read yet — a small exclusion today, and the one that
/// grows if a large import lands before the hash job drains.
const V55_MATERIAL_CONTENT_REGION_HASH: &str = r#"
ALTER TABLE material ADD COLUMN content_region_hash TEXT;

UPDATE material SET content_region_hash = 'unsupported:not-walked';

CREATE INDEX idx_material_content_region_hash
    ON material(content_region_hash) WHERE content_region_hash IS NOT NULL;
"#;

/// Version 55 → 56: compute the content-axis values V55 deferred —
/// **the second half of one migration, not a feature.**
///
/// V55 added `content_region_hash` and wrote
/// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
/// into every existing row, because filling the column in means reading
/// every original off disk and that could not run in the statement that
/// added it. The marker is the record of which rows were left, and this
/// step is the pass that finishes them. Until it runs, the content axis
/// answers about the fraction of the library that arrived after V55 —
/// on the dogfood corpus, 22 PNGs that produce no content-axis group at
/// all against 289 materials.
///
/// # Why this is a migration step and not a job
///
/// Bringing stored data to the shape the schema promises is the
/// application's own responsibility, and it is what a migration *is*.
/// The alternative shape considered and rejected was a background pass
/// with its own entry point — something a person starts, with progress,
/// resumption from a cursor, and a queue kind of its own. That is the
/// design vocabulary of *ordinary operation*, applied to something that
/// happens once per database. It would have cost a job kind, a port
/// method, a transport verb, a panel control and a progress surface, all
/// of them permanent, to manage a wait that occurs on exactly one
/// launch. It also puts the library in a state the schema says is
/// impossible for as long as nobody presses the button.
///
/// Nothing here needs to survive being interrupted, either: [`migrate`]
/// runs this and the `user_version` bump in **one transaction**, so a
/// process killed halfway rolls the whole step back and the next launch
/// starts it over. Resumption machinery would be a second answer to a
/// question the framework already answers.
///
/// # What this costs, and why that is not the general case
///
/// The set is bounded by what existed when V55 ran, and only the part of
/// it that has a walker is actually read: everything else is answered
/// from its mime without the file being opened. On the corpus this was
/// written against that is **22 PNGs out of 289 materials** — the rest
/// is text and video, which take a marker and no read [measured: dogfood
/// mime distribution]. Material imported after V55 never appears here at
/// all: its `INSERT` names neither hash column, so it arrives NULL and
/// belongs to the ordinary fingerprint walk.
///
/// **That smallness is an accident of timing, not a property of this
/// step.** It is small because the library was small when the column
/// landed. Bumping the region definition (`cr1-` → `cr2-`) is the case
/// where it is not: every row ever walked becomes a target at once, on
/// libraries that by then may be any size, and a startup that reads the
/// whole corpus inside one transaction is not something a released
/// application may do to somebody who only installed an update. That
/// version bump therefore owes an **explicitly managed upgrade moment**
/// — the user knowing it is about to happen and when — rather than a
/// copy of this step (decided 2026-08-05). The selection
/// ([`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk))
/// carries over; the *timing* is what has to be designed then and must
/// not be inherited from here.
///
/// # What it does to a row
///
/// One read of the file answers both axes
/// ([`hash_artefact`](crate::fingerprint::hash_artefact), the same
/// function the `material_hash` job calls, so a file cannot fingerprint
/// differently depending on which pass reached it), and one `UPDATE`
/// writes both columns — the invariant `set_material_fingerprint`
/// keeps, kept here too rather than excepted. The file column is
/// **recomputed rather than trusted**: the bytes are already in memory,
/// so the honest thing costs nothing. On unchanged bytes it lands on the
/// value that was already there; where it does not, the file has changed
/// since it was hashed and the new value is what is true about the file
/// now. That difference is counted and logged, and nothing else follows
/// from it — the duplicate report is a query over the column, so it
/// reflects the corrected value the moment this commits.
///
/// Rows this cannot answer are left carrying the marker, which stays
/// true of them:
///
/// - **A locator with no bytes of its own** (a record inside a
///   conversation log, a remote URL) gets
///   [`UNHASHABLE`](asterism_core::domain::content_hash::UNHASHABLE) on
///   the content axis — a permanent answer, so the row stops being
///   pending on both axes. Its file column is written back **unchanged**
///   rather than overwritten with the marker: whatever measurement is
///   there was made by a pass that had the bytes, and this step never
///   opened the file, so it has nothing truer to say.
/// - **A file that could not be read** — moved, deleted, an unplugged
///   disk — is skipped and keeps `NOT_WALKED`. "Nothing walked these
///   bytes" is exactly what remains true, the file axis goes on grouping
///   the row as it does today, and an unreadable original is not a
///   reason to fail a schema upgrade.
///
/// # What it deliberately does not do: raise duplicate questions
///
/// Content digests landing on a library can reveal pairs (measured: 9
/// groups in a 4,601-image ComfyUI corpus, pixel-identical, file digests
/// different). Those pairs are **visible the moment this commits** —
/// `list_duplicate_groups` groups on this column, so the report answers
/// from the data with no detection involved, and the panel offers its
/// resolution on them like any other group.
///
/// What detection would add is a row in the *conflict queue*, and for
/// this set it would add nothing else. Every pair here is two rows that
/// have been in the library, which is precisely the case
/// [`DetectionOrigin::Backfill`](asterism_core::application_support::duplicate_detection::DetectionOrigin)
/// already rules must never be folded without a person — so the only
/// outcome available is `ask`, i.e. a second copy of what the report
/// shows, produced N at a time by an upgrade, in the list that is meant
/// to hold what an import just raised.
///
/// Against that: detection is three ports and an async call, and a
/// migration holds a `Transaction`. Re-expressing the rule here — which
/// of two rows is the newcomer, the `fold_policy` ruling, the lineage
/// and dispatch exclusions, the `identical_to` write — would be a second
/// implementation of the decision "are these two rows one thing", with
/// nothing to catch the two drifting apart. **No fold is enqueued and no
/// conflict is raised by this step**, and that is fixed by teeth rather
/// than left to be read out of the absence of code.
///
/// The consequence worth stating: pre-existing content-axis pairs reach
/// the user through the report, not the question queue. A pair found on
/// this axis *after* the upgrade — a newly imported file matching one of
/// these digests — is the ordinary detection path's business, and that
/// path is still file-axis only; widening it is its own change.
/// The marker-era stored spelling of one axis outcome — what the hash
/// columns held **before V92** split status and digest apart.
///
/// The two data migrations below (V56 and V65) run at their own point
/// in the chain, where the columns still carry the inline vocabulary,
/// and they read files through `hash_artefact`, which is shared with
/// the ordinary job and now answers in the split shape. This renders
/// its answer back into the spelling their era stored, so a landed
/// migration keeps writing exactly what it wrote when it shipped — V92
/// then converts these rows along with everything else. Frozen here
/// rather than borrowed from the domain, for the reason V56 carries its
/// own locator test: a landed migration must not change what it did
/// because a helper moved.
fn pre_v92_stored_value(record: &asterism_core::domain::measurement::Measurement) -> String {
    use asterism_core::domain::measurement::MeasurementStatus;

    match record.status {
        MeasurementStatus::Computed => record.digest.clone().unwrap_or_default(),
        MeasurementStatus::Unsupported => format!(
            "unsupported:{}",
            record.reason.as_deref().unwrap_or("unknown")
        ),
        MeasurementStatus::EmptySpan => "unsupported:empty-span".to_string(),
        MeasurementStatus::TooLarge => "unsupported:too-large".to_string(),
        MeasurementStatus::NotWalked => "unsupported:not-walked".to_string(),
        MeasurementStatus::NoBytes => "unhashable:no-bytes".to_string(),
        // Neither existed in the marker era: `pending` was NULL, and
        // `failed` was nothing at all. `hash_artefact` produces
        // neither, so this arm is a reader bug made loud on the row
        // rather than a digest invented for it.
        MeasurementStatus::Pending | MeasurementStatus::Failed => String::new(),
    }
}

fn v56_walk_deferred_content_regions(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    use asterism_core::domain::content_hash::UNHASHABLE;
    use asterism_core::domain::content_region::NOT_WALKED;
    use asterism_core::domain::value::MimeType;

    use crate::fingerprint::{MAX_CONTENT_WALK_BYTES, hash_artefact};

    /// **Frozen copy of `content_hash::is_hashable_locator` as it stood
    /// when V56 landed.** Do not change it, and do not replace it with
    /// whatever the domain currently thinks — that is the whole point.
    ///
    /// This walk reads raw column values rather than entities, which is
    /// correct for a landed migration: it has to keep meaning what it
    /// meant when it ran. It used to `use` the live function by name,
    /// and `Step::App` still runs on any database below V56 — so the
    /// domain's later answer for `file://` (now: strip the scheme, open
    /// the file it names) would have reached this walk retroactively and
    /// changed what an old upgrade does.
    ///
    /// It is also the last string test of this shape anywhere. The live
    /// question is `SourceLocator::local_path()`, and there is no `&str`
    /// overload of it to reach for.
    fn was_hashable_at_v56(locator: &str) -> bool {
        if locator.contains('#') {
            return false;
        }
        // A scheme other than `file:` means it is not on this disk.
        // Windows drive letters (`C:\…`) are excluded from the scheme
        // test by the length check — a scheme is longer than one
        // character.
        if let Some((scheme, _)) = locator.split_once("://") {
            return scheme.eq_ignore_ascii_case("file");
        }
        let mut chars = locator.chars();
        match (chars.next(), chars.next(), chars.next()) {
            (Some('/'), _, _) => true,
            (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic() => true,
            _ => false,
        }
    }

    /// One row V55 deferred: where its bytes are, what format the row
    /// believes they are, and what the file axis already recorded.
    struct Deferred {
        asset_id: Uuid,
        ord: i64,
        locator: String,
        mime: Option<String>,
        stored_file: Option<String>,
    }

    // The whole set up front rather than paged: the transaction is open
    // either way, and a cursor would be machinery for resuming something
    // that either commits or rolls back whole.
    let pending: Vec<Deferred> = {
        let mut stmt = tx.prepare(
            "SELECT asset_id, ord, locator, mime, content_hash \
               FROM material \
              WHERE content_region_hash = ?1 \
              ORDER BY asset_id, ord",
        )?;
        stmt.query_map(params![NOT_WALKED], |row| {
            Ok(Deferred {
                asset_id: row.get(0)?,
                ord: row.get(1)?,
                locator: row.get(2)?,
                mime: row.get(3)?,
                stored_file: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    if pending.is_empty() {
        return Ok(());
    }

    let mut update = tx.prepare(
        "UPDATE material SET content_hash = ?1, content_region_hash = ?2 \
          WHERE asset_id = ?3 AND ord = ?4",
    )?;

    let mut walked = 0usize;
    let mut no_bytes = 0usize;
    let mut unreadable = 0usize;
    let mut file_digest_moved = 0usize;

    for row in &pending {
        let (file, content) = if !was_hashable_at_v56(&row.locator) {
            no_bytes += 1;
            (row.stored_file.clone(), UNHASHABLE.to_string())
        } else {
            // Parsed here for the same reason the repository parses:
            // the walk decides on the format, and a raw column value
            // has not been normalised yet.
            let declared = row.mime.as_deref().map(MimeType::parse);
            match hash_artefact(&row.locator, declared.as_ref(), MAX_CONTENT_WALK_BYTES) {
                Ok(fingerprint) => {
                    walked += 1;
                    let file = pre_v92_stored_value(&fingerprint.file);
                    if row
                        .stored_file
                        .as_deref()
                        .is_some_and(|stored| stored != file)
                    {
                        file_digest_moved += 1;
                        // `action.`, not `diag.`: nothing malfunctioned.
                        // The bytes behind this locator are not the bytes
                        // that were hashed, which is a fact about the
                        // library that only re-reading could surface.
                        tracing::warn!(
                            event = "action.content_walk.file_digest_moved",
                            asset_id = %row.asset_id,
                            ord = %row.ord,
                            locator = %row.locator,
                            stored = %row.stored_file.as_deref().unwrap_or_default(),
                            got = %file,
                            "the original's bytes changed since it was fingerprinted"
                        );
                    }
                    (Some(file), pre_v92_stored_value(&fingerprint.content))
                }
                Err(err) => {
                    unreadable += 1;
                    tracing::warn!(
                        event = "diag.content_walk.unreadable",
                        asset_id = %row.asset_id,
                        locator = %row.locator,
                        error = %err,
                        "left carrying the not-walked marker"
                    );
                    continue;
                }
            }
        };
        update.execute(params![file, content, row.asset_id, row.ord])?;
    }

    tracing::info!(
        event = "diag.content_walk.completed",
        pending = pending.len(),
        walked,
        no_bytes,
        unreadable,
        file_digest_moved,
        "content-axis values computed for materials that predate the column"
    );
    Ok(())
}

// Rebuild of the V17 covering indexes so they cover again.
//
// V17's contract was "every column `IndexRow` selects is included so
// SQLite can serve the whole scan from the index". Three later
// migrations grew the query without growing the index: V33 put
// `trashed_at IS NULL` into every live listing's WHERE, V51 did the
// same with `folded_into IS NULL`, and `IndexRow` itself gained
// `updated_at` + `role`. None of those columns are in the V17 key, so
// the planner stopped choosing the cover at all — [measured 2026-08-05,
// EXPLAIN QUERY PLAN on a V56 database] the grid listing runs on
// `idx_asset_persona_occurred` with one table-page lookup per hit,
// which is exactly the cost V17 existed to remove (~2.0 s cold at 110k
// rows, see the V17 comment).
//
// The indexes are **partial** over the live default (`trashed_at IS
// NULL AND folded_into IS NULL`), so dead + folded rows drop out of
// the index instead of being carried in every page of it. Every
// listing query emits both terms verbatim (`QueryParts::build`:
// unconditionally for the fold axis, by `LiveOnly` default for the
// trash axis), so the implication that makes a partial index usable
// holds. `TrashedOnly` / `Any` listings do not match the predicate
// and keep planning exactly as they do today — on the narrow seek
// indexes, which [same run] is also what they used before this batch.
//
// **Both predicate columns are also in the key**, which looks
// redundant (inside this index they are NULL by construction) but is
// not: SQLite's planner uses the implication only to admit the index,
// it still evaluates the query's own `IS NULL` terms — and a term
// whose column is not in the index is a table-page fetch, which
// silently demotes the index from COVERING back to seek [measured
// 2026-08-05, SQLite 3.43.2: identical statement plans as
// `USING INDEX` without the two columns, `USING COVERING INDEX` with
// them]. Two always-NULL entries per row is what that costs.
//
// A future `ALTER TABLE asset` rebuild that recreates indexes from a
// literal list must copy this shape, not V17's.
const V57_ASSET_INDEX_COVERING_LIVE: &str = r#"
DROP INDEX idx_asset_persona_occurred_cover;
DROP INDEX idx_asset_occurred_cover;

CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at,
             updated_at, role, trashed_at, folded_into)
    WHERE trashed_at IS NULL AND folded_into IS NULL;
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at,
             updated_at, role, trashed_at, folded_into)
    WHERE trashed_at IS NULL AND folded_into IS NULL;
"#;

/// Version 66 → 67: the AlbumMeta secondary index.
///
/// AlbumMeta statements live in `asset.extra` under
/// `_trace.meta.<key>.value`, which is where a *statement* belongs — but
/// filtering on one from there means opening the bag on every row, and
/// the bag is the importer's, so its size has no ceiling. That is the
/// same problem `asset_color` was created for: the quantised palette is
/// derived from `asset.palette`, and the facet is an equality seek on a
/// projection instead of a scan over JSON.
///
/// # Why a trigger rather than the write path
///
/// The projection could be rebuilt in `save`, which owns the `extra`
/// column. It is maintained here instead because two other statements
/// write that column directly — the fold note and the declared-hash
/// verdict — and neither goes through an entity. Neither touches
/// `_trace.meta` today, so a Rust-side rebuild would in fact be correct;
/// what it would not be is *guaranteed*. A trigger makes the projection
/// a property of the column instead of a habit of the callers, and the
/// schema already keeps `session` summaries and message counts this way.
///
/// # What lands in it
///
/// One row per (asset, key), holding the current value — the same single
/// slot the write side keeps, so a correction replaces rather than
/// accumulates. Entries whose `value` is missing are skipped: an entry
/// without one is not a statement, and indexing NULL would produce rows
/// that match nothing and count as something.
///
/// The `CASE` around the extraction is not defensive dressing. A bag
/// that is absent, is not JSON, or holds a `_trace.meta` that is not an
/// object must yield an empty object rather than raise — an error inside
/// an `AFTER UPDATE` trigger aborts the write that fired it, so a row
/// with a malformed bag would become unsaveable.
const V67_ASSET_ALBUM_META: &str = r#"
CREATE TABLE asset_album_meta (
    asset_id BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    key      TEXT NOT NULL,
    value    TEXT NOT NULL,
    PRIMARY KEY (asset_id, key)
) STRICT, WITHOUT ROWID;

-- Both directions the filter is asked in: "which row carries this value
-- under this name" and "which row carries this value at all". The
-- second exists because somebody pasting an identifier usually knows
-- the value and not the name it was filed under.
CREATE INDEX idx_asset_album_meta_key_value ON asset_album_meta(key, value);
CREATE INDEX idx_asset_album_meta_value ON asset_album_meta(value);

CREATE TRIGGER trg_asset_album_meta_ins AFTER INSERT ON asset
BEGIN
    INSERT INTO asset_album_meta (asset_id, key, value)
    SELECT NEW.id, entry.key, json_extract(entry.value, '$.value')
      FROM json_each(
               CASE
                   WHEN NEW.extra IS NULL OR NOT json_valid(NEW.extra) THEN '{}'
                   WHEN json_type(NEW.extra, '$._trace.meta') IS NOT 'object' THEN '{}'
                   ELSE json_extract(NEW.extra, '$._trace.meta')
               END
           ) AS entry
     WHERE json_extract(entry.value, '$.value') IS NOT NULL;
END;

CREATE TRIGGER trg_asset_album_meta_upd AFTER UPDATE OF extra ON asset
BEGIN
    DELETE FROM asset_album_meta WHERE asset_id = NEW.id;
    INSERT INTO asset_album_meta (asset_id, key, value)
    SELECT NEW.id, entry.key, json_extract(entry.value, '$.value')
      FROM json_each(
               CASE
                   WHEN NEW.extra IS NULL OR NOT json_valid(NEW.extra) THEN '{}'
                   WHEN json_type(NEW.extra, '$._trace.meta') IS NOT 'object' THEN '{}'
                   ELSE json_extract(NEW.extra, '$._trace.meta')
               END
           ) AS entry
     WHERE json_extract(entry.value, '$.value') IS NOT NULL;
END;

-- The rows that were already here. The declaration verb landed before
-- this index did, so a library can hold statements the projection has
-- never seen.
INSERT INTO asset_album_meta (asset_id, key, value)
SELECT asset.id, entry.key, json_extract(entry.value, '$.value')
  FROM asset,
       json_each(
           CASE
               WHEN asset.extra IS NULL OR NOT json_valid(asset.extra) THEN '{}'
               WHEN json_type(asset.extra, '$._trace.meta') IS NOT 'object' THEN '{}'
               ELSE json_extract(asset.extra, '$._trace.meta')
           END
       ) AS entry
 WHERE json_extract(entry.value, '$.value') IS NOT NULL;
"#;

// Version 57 → 58: the Query-side text predicate — "assets whose body
// contains this string" as an exact, countable `WHERE` term.
//
// This is the Query half of the Query / Retrieval split: it
// answers with a *set*, so it can be counted, sorted on any axis, and
// used to define a Query Group's membership. The Tantivy side stays
// where it is and keeps answering the other question (a ranked
// shortlist for "find me something like this").
//
// # One meaning: substring, at every length
//
// The predicate is "the body contains this string" — no word
// boundaries, no dictionary. `スト` finds `テスト`, `猫` finds `黒猫`.
// A word-segmented index was considered and rejected for this side:
// segmenting `黒猫` as one token makes a search for `猫` miss it, which
// is a hole the person searching cannot see or work around. Splitting
// the *meaning* by query length (short → words, long → substrings)
// would have pushed an index limitation onto the person typing.
//
// What is split is only the **acceleration**:
//
// - 3 characters or more → `asset_fts`, an FTS5 `trigram` index. The
//   tokenizer indexes every 3-character window, which is what makes
//   arbitrary substring lookup an index seek. It needs no dictionary,
//   so CJK text indexes as-is and this migration is plain SQL.
// - 1–2 characters → no index (a trigram index cannot serve a pattern
//   shorter than one trigram). The predicate falls back to `LIKE` over
//   `asset_body`, which is the *same* answer, just scanned. The other
//   predicates in the same `WHERE` narrow the set first, so the scan is
//   over what the chips already selected rather than the whole table.
//
// # Why contentless, and why a key table
//
// `asset_fts` is `content=''`: the body already lives in `asset_body`
// and storing it twice would double the text on disk for nothing. That
// costs the ability to rebuild the index from itself — the recovery
// path is re-running this INSERT from `asset_body`, which is why the
// backfill below is a statement and not a one-off.
//
// An FTS5 table is addressed by an INTEGER rowid and assets are keyed
// by a BLOB uuid, so `asset_fts_key` carries the mapping. The obvious
// alternative — leaning on `asset.rowid` — was rejected: an implicit
// rowid is not stable across `VACUUM` or a table rebuild, and *this*
// codebase rebuilds tables in `Step::App` migrations. A mapping that
// silently shifts under a maintenance operation would produce wrong
// rows with no error, which is the failure class this whole split
// exists to remove. `seq INTEGER PRIMARY KEY` is a real rowid alias
// and survives both.
//
// Orphan direction is deliberate: `asset_fts_key` cascades with the
// asset, so a row deleted outside the indexer leaves an unreferenced
// `asset_fts` row. The read path joins *through* the key table, so an
// orphan can only waste space — never contribute a row to an answer.
const V58_ASSET_TEXT_INDEX: &str = r#"
CREATE TABLE asset_fts_key (
    seq      INTEGER PRIMARY KEY,
    asset_id BLOB NOT NULL UNIQUE REFERENCES asset(id) ON DELETE CASCADE
);

CREATE VIRTUAL TABLE asset_fts USING fts5(
    body,
    content='',
    contentless_delete=1,
    tokenize='trigram'
);

INSERT INTO asset_fts_key (asset_id) SELECT asset_id FROM asset_body;

INSERT INTO asset_fts (rowid, body)
    SELECT k.seq, b.body_text
      FROM asset_body b
      JOIN asset_fts_key k ON k.asset_id = b.asset_id;
"#;

// Version 58 → 59: the two metric columns join the covering indexes,
// because `IndexRow` now selects them.
//
// Same failure V57 was written to repair, one wave later: the light row
// grew `duration_ms` + `file_size_bytes` so the grid can offer the
// length and size axes (an axis the index cannot express is an axis the
// grid cannot offer), and a selected column that is not in the key
// demotes the plan from COVERING back to a seek with one table-page
// lookup per hit. That cost is measured — ~2.0 s cold at 110 k rows,
// see the V17 comment — so growing the query without growing the index
// would trade the whole reason V17 and V57 exist for two sort axes.
//
// Shape copied from V57 verbatim apart from the two added columns:
// still partial over the live default, still carrying `trashed_at` /
// `folded_into` in the key for the reason V57 spells out (the planner
// evaluates those terms itself, and a term whose column is absent
// refetches the row). The guard that catches the next occurrence is
// `the_live_grid_listing_is_served_by_the_covering_index`, which builds
// its statement from the real `IndexRow::COLUMNS`.
const V59_ASSET_INDEX_COVERING_METRICS: &str = r#"
DROP INDEX idx_asset_persona_occurred_cover;
DROP INDEX idx_asset_occurred_cover;

CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at,
             updated_at, duration_ms, file_size_bytes, role,
             trashed_at, folded_into)
    WHERE trashed_at IS NULL AND folded_into IS NULL;
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at,
             updated_at, duration_ms, file_size_bytes, role,
             trashed_at, folded_into)
    WHERE trashed_at IS NULL AND folded_into IS NULL;
"#;

/// Version 59 → 60: `asset_timeline_mark` — a mark placed *into* a
/// time-bearing asset (an instant, or a half-open interval) rather than
/// onto the asset as a whole the way `asset_comment` is.
///
/// - `end_ms IS NULL` is an instant, not "runs to the end of the
///   media"; `end_ms > start_ms` (never equal) because a half-open
///   interval covering nothing is not a mark. Both are the domain's
///   `TimelineSpan` rules, restated here where raw SQL can reach.
/// - **No CHECK on `body`.** The domain requires it non-empty after a
///   Unicode trim; SQLite's one-argument `trim(X)` strips only U+0020,
///   so `CHECK (length(trim(body)) > 0)` would pass `'\t'`, `'\n'`,
///   U+00A0 and U+3000 while looking like a mirror of the rule. The
///   adapter routes every row back through the constructor instead
///   (`AssetTimelineMark::rehydrate`), and the rule stays in one place.
/// - `author_persona_id` is `ON DELETE CASCADE`, unlike V15's
///   `SET NULL` on `asset_comment`. `SET NULL` does not survive the
///   pairing CHECK: an FK action runs as an ordinary UPDATE, so
///   nulling the id while `author_kind` still reads `'persona'` fails
///   the CHECK and aborts the `DELETE FROM persona` itself. `RESTRICT`
///   is available and rejected for a different reason — the ordered
///   sweep in `repo/persona.rs::purge` exists because SQLite does not
///   order sibling-table cascades, and every RESTRICT child added is
///   another line that has to be remembered there. CASCADE closes in
///   the DDL. The consequence is deliberate: purging a persona removes
///   the marks it left, including those on other personas' assets.
/// - Rows are not constrained against each other. Overlapping and
///   exactly-coincident marks on one asset are allowed — overlap is
///   the premise of the model (IIIF Ranges), and de-duplication is the
///   caller's question, not the table's.
/// - The index carries the filter and the leading sort term, not the
///   whole read. `EXPLAIN QUERY PLAN` on the adapter's listing statement
///   (SQLite 3.43.2, 2026-08-06) is two lines:
///
///   ```text
///   SEARCH asset_timeline_mark USING INDEX idx_asset_timeline_mark_asset_start (asset_id=?)
///   USE TEMP B-TREE FOR RIGHT PART OF ORDER BY
///   ```
///
///   The second line is the `id` tie-break; drop `, id` from the
///   statement and it disappears. So the tie-break is a real sort, not
///   something the scan hands over for free — worth knowing before
///   anyone reads "index-backed" as "already ordered". No interval
///   index (R-tree) — there is no range query yet to serve with one.
const V60_ASSET_TIMELINE_MARK: &str = r#"
CREATE TABLE asset_timeline_mark (
    id                BLOB PRIMARY KEY,
    asset_id          BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    start_ms          INTEGER NOT NULL,
    end_ms            INTEGER,
    body              TEXT NOT NULL,
    author_kind       TEXT NOT NULL,
    author_persona_id BLOB REFERENCES persona(id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    edited_at         INTEGER,
    CHECK (start_ms >= 0),
    CHECK (end_ms IS NULL OR end_ms > start_ms),
    CHECK (author_kind IN ('user', 'persona')),
    CHECK (
        (author_kind = 'user'    AND author_persona_id IS NULL)
     OR (author_kind = 'persona' AND author_persona_id IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_asset_timeline_mark_asset_start
    ON asset_timeline_mark(asset_id, start_ms);
"#;

// Version 60 → 61: the Source value stops being unique and becomes a
// lookup.
//
// `Asset : Source` is `N : 1` — many Assets may carry the same Source
// value — and a UNIQUE index asserts `1 : 1`. That contradiction is the
// whole argument; it is not a usage measurement. V2 installed the index
// for "import idempotency", which is a *lookup* requirement, and this
// step gives it back its lookup and takes away its refusal.
//
// # The index stays, and gets busier
//
// The Source value becomes the ingest path's **first** question, asked
// before an `AssetId` is minted, so these columns are read
// on every arrival rather than only when a write collided. Dropping the
// index with the uniqueness would have turned every ingest into a table
// scan.
//
// # `persona_id` joins the key
//
// The library-wide reading was never a decision anyone made: V2's own
// line says "re-importing the same file", which is one library reading
// its own sources. Every same-thing judgment added since is already
// persona-scoped (`find_by_content_hash` takes a `PersonaId`, and so
// does `duplicate_conflict.persona_id`). Left library-wide, persona B
// importing a path persona A already holds would be handed A's row — a
// row B cannot see — and B's only way in would be to declare
// `Separate`, i.e. to declare a duplicate resolution in order to say
// "this file is mine too".
//
// # What replaces the refusal
//
// Nothing in the schema. A second arrival at one location is answered
// by the lookup (the caller is handed the row that was already there),
// and a caller that means to make a second row says so with
// `on_duplicate = 'separate'`. The `duplicate_conflict` machinery is
// where a match becomes a question, and it was always the right home
// for one — a constraint could only refuse.
//
// The four landed definitions of the old index (`:122`, and the three
// table rebuilds that could not be `ALTER TABLE`) are historical
// artefacts and are left as they are. Precedent for a step that drops
// and rebuilds indexes: `V57_ASSET_INDEX_COVERING_LIVE`.
const V61_ASSET_SOURCE_LOOKUP: &str = r#"
DROP INDEX idx_asset_source_unique;

CREATE INDEX idx_asset_source
    ON asset(persona_id, source_kind, source_locator);
"#;

// Version 61 → 62: `external_key` stops being unique and becomes a
// lookup — the same correction V61 made to the Source pair, on the
// other Prop that was carrying a constraint.
//
// `AssetId` is the identity of an Asset; every other column is a Prop,
// and **no Prop carries a constraint the domain reads as "this row *is*
// that thing"**. `external_key` is where a source parks its own id so
// that Album rows are reachable from outside. That is external linkage.
// It is not an identity, and a UNIQUE on it asserts that it is.
//
// # Why the column cannot be unique, concretely
//
// Two reasons, and either alone settles it:
//
// - **An external record legitimately arrives more than once.** Sign an
//   image and ingest it; update it and ingest it again — the source
//   states the same key both times. The key is not in 1 : 1
//   correspondence with an `AssetId`, and repeats are ordinary rather
//   than exceptional.
// - **Ids from different platforms collide.** Two unrelated sources
//   both numbering their records `12345` is normal, and this index's
//   key is `(persona_id, external_key)` — it carries no source
//   discriminator at all, so platform A's `12345` refuses platform B's.
//   Adding `source_kind` to the key would not rescue it: a namespace
//   question only exists if something is being made unique, and after
//   this step nothing is.
//
// # The index stays
//
// Only the uniqueness goes. `SessionRepository::find_by_external_key`
// reads these columns on the Session find-or-create path, so dropping
// the index outright would turn that lookup into a table scan. The
// partial `WHERE external_key IS NOT NULL` is kept for the same reason
// V30 wrote it: it is an index-size choice, not a constraint — a row
// with no key has nothing to look up.
//
// # What replaces the refusal
//
// One `isle.call` closure. `SessionRepository::create` used to insert
// and read the UNIQUE violation back out of the error string; it now
// does the lookup *and* the insert inside a single closure on the
// writer isle, which serialises writers, so select-then-insert is
// atomic there without any index asserting it. That is the mechanism
// the delete guard in the same adapter already uses ("both writes hit
// the same writer isle serially"). A constraint was never needed to
// make a find-or-create single-valued; a serialisation point was.
//
// The three landed definitions of the old index (V30, and the two
// table rebuilds that could not be `ALTER TABLE`) are historical
// artefacts and are left as they are. Precedent for the shape:
// `V61_ASSET_SOURCE_LOOKUP` directly above.
const V62_ASSET_EXTERNAL_KEY_LOOKUP: &str = r#"
DROP INDEX idx_asset_external_key;

CREATE INDEX idx_asset_external_key
    ON asset(persona_id, external_key) WHERE external_key IS NOT NULL;
"#;

/// Version 62 → 63: every locator is rewritten from the delimited
/// spelling into the tagged form.
///
/// Both columns with this encoding are walked —
/// `asset.source_locator` and its denormalised copy `material.locator` —
/// because they are read back through one type and a half-rewritten
/// pair would make the same artefact answer differently depending on
/// which row was asked.
///
/// After this step the columns hold one object per row:
///
/// ```json
/// {"kind":"file",   "path":"/pics/a.png"}
/// {"kind":"record", "container":"/logs/s.jsonl","record":"0198c1c2-…"}
/// {"kind":"remote", "scheme":"hf","target":"org/model/f.safetensors"}
/// {"kind":"logical","name":"chat/0198c1c2/msg-1"}
/// ```
///
/// # The reader is frozen, and this is the last code that knows the
/// delimiter
///
/// [`read_at_v63`] is a **snapshot** of `SourceLocator`'s reader as it
/// stood while the columns were delimited, and the renderer is a
/// snapshot of the tagged shape as it stood when this landed. Neither
/// calls into the live type, for the reason the V56 snapshot beside it
/// gives in as many words: a landed migration reads columns rather than
/// entities and has to keep meaning what it meant when it ran. Importing
/// the live reader is exactly how a later change to the domain's answer
/// reaches backwards into an old upgrade.
///
/// The live type still reads a `#`, in exactly one place and for a
/// different contract: `SourceLocator::from_wire`, the spelling
/// importers send. What ends in this file is the *column's* delimiter.
///
/// # The walk can merge, and that is legal now
///
/// The rendering is **not injective on distinct old values**:
/// `file:///pics/a.png` and `/pics/a.png` are two legal strings today
/// and one locator afterwards, because the boundary consumes the `file:`
/// scheme on purpose so the two compare equal. Two rows spelled those
/// two ways come out of this walk carrying one Source value. Under
/// `idx_asset_source_unique` that was a constraint violation inside a
/// `Step::App` and would have rolled the whole batch back — an upgrade
/// failing on a database that was never wrong. V61 demoted it, which is
/// why this step is ordered after that one, and the merge is now exactly
/// what `N : 1` permits.
///
/// # `/pics/a#b.png` is settled here, by asking the filesystem
///
/// A legal POSIX filename containing `#` was indistinguishable from a
/// container plus a record, and no rule over a string can tell them
/// apart — which is why the domain carried it as a known limitation. A
/// migration can run the one honest test a parser cannot: **ask the
/// filesystem whether the whole string exists as a file.** Three
/// outcomes, and the third is not a guess:
///
/// - **the whole string is a file** → it was a `file`, and the `#` is an
///   ordinary character in its name. This is the reading the delimiter
///   got wrong, and the row is corrected.
/// - **it is not, and the container half is a file** → the `#` was the
///   delimiter, corroborated. `record`.
/// - **neither exists** — the container has moved, been deleted, or sits
///   on a disk that is not plugged in. The filesystem answers *neither
///   way*, so nothing is inferred: the row **keeps the delimiter
///   reading** it has always had, and the fact that it was not settled
///   is reported (`diag.locator_rewrite.undecided`, one record per row
///   plus a count in the completion line). This one does not pretend to
///   know.
///
/// **Why that is a log and not a column**, which is where this departs
/// from V55. V55 had somewhere to write: its marker occupies the value
/// space of the digest column it was deferring, so the mark *is* the
/// answer. Here there is no such column — the locator column holds the
/// locator — and a new one would store a fact that is both cheaply
/// recomputable and quick to go stale. An undecided row is exactly "a
/// `record` whose container is not a file", so a later pass finds its
/// own work by asking the disk it is standing on, which is the only
/// reading that is true at the moment it runs. A stored flag would
/// still say "undecided" after the volume came back, and the copy that
/// drifts is always the one nobody thought was authoritative.
///
/// The probe is reached only by rows that already read as a record, so
/// an ordinary library pays one `stat` per record row and none at all
/// for its files.
fn v63_rewrite_locators_as_tagged_json(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    use std::borrow::Cow;
    use std::path::Path;

    use serde::Serialize;

    /// **Frozen copy of the tagged storage shape as it stood when V63
    /// landed.** Do not point this at the live `SourceLocator`, and do
    /// not edit it if the live shape changes — a further migration
    /// rewrites the column, and this one keeps saying what it said.
    ///
    /// `serde` writes it; nothing here hand-writes JSON, which is what
    /// keeps the field order and the absence of whitespace a property of
    /// the declaration rather than of a `format!` string.
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "lowercase")]
    enum TaggedAtV63<'a> {
        File {
            path: Cow<'a, str>,
        },
        Record {
            container: Cow<'a, str>,
            record: Cow<'a, str>,
        },
        Remote {
            scheme: Cow<'a, str>,
            target: Cow<'a, str>,
        },
        Logical {
            name: Cow<'a, str>,
        },
    }

    /// Frozen copy of the rooted-path test. Hand-rolled rather than
    /// `Path::is_absolute` for the reason the domain gave when it was
    /// there: `is_absolute` is platform-conditional, so `C:\pics\a.png`
    /// would answer `false` on a unix build and the row would be
    /// reclassified by whichever machine ran the upgrade.
    fn was_rooted_at_v63(raw: &str) -> bool {
        let mut chars = raw.chars();
        match (chars.next(), chars.next(), chars.next()) {
            (Some('/'), _, _) => true,
            (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic() => true,
            _ => false,
        }
    }

    /// Frozen copy of the scheme grammar, including the departure from
    /// RFC 3986 that refuses a single character — without it the `C` of
    /// `C://pics/a.png` reads as a scheme and every Windows-spelled path
    /// becomes a remote.
    fn was_a_scheme_at_v63(raw: &str) -> bool {
        let mut chars = raw.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() {
            return false;
        }
        if chars.clone().next().is_none() {
            return false;
        }
        chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    }

    /// One row's reading, before the filesystem is asked anything.
    enum ReadAtV63 {
        File(String),
        Record { container: String, record: String },
        Remote { scheme: String, target: String },
        Logical(String),
    }

    /// **Frozen copy of the delimited reader.** `None` is the one string
    /// the reader refused and the column should never have held — a
    /// blank one.
    ///
    /// The order is load-bearing and is the order the domain used: the
    /// scheme is tested *before* the `#` split, because a URL may carry
    /// a fragment of its own and splitting `https://h/a#b` into a
    /// container and a record would be reading someone else's syntax as
    /// ours. `file:` is consumed rather than recorded — it is a spelling
    /// of a local path, not a fact about the artefact — and what is left
    /// goes through the remaining rules, which is why `file://pics/a.png`
    /// lands as a logical name.
    fn read_at_v63(raw: &str) -> Option<ReadAtV63> {
        if raw.trim().is_empty() {
            return None;
        }
        let body = match raw.split_once("://") {
            Some((scheme, rest)) if was_a_scheme_at_v63(scheme) => {
                if scheme.eq_ignore_ascii_case("file") {
                    rest
                } else if !rest.is_empty() {
                    return Some(ReadAtV63::Remote {
                        scheme: scheme.to_ascii_lowercase(),
                        target: rest.to_string(),
                    });
                } else {
                    // `<scheme>://` with nothing after it addresses
                    // nothing; it is a name like any other.
                    raw
                }
            }
            _ => raw,
        };
        // The split is on the *last* `#`, which is what kept a container
        // path containing one readable.
        if let Some((container, record)) = body.rsplit_once('#')
            && was_rooted_at_v63(container)
            && !record.is_empty()
        {
            return Some(ReadAtV63::Record {
                container: container.to_string(),
                record: record.to_string(),
            });
        }
        if was_rooted_at_v63(body) {
            return Some(ReadAtV63::File(body.to_string()));
        }
        // `raw`, not `body`: a consumed `file://` that left nothing
        // openable behind is still the string that was stored.
        Some(ReadAtV63::Logical(raw.to_string()))
    }

    /// What the pass did, for the completion line.
    #[derive(Default)]
    struct Tally {
        rewritten: usize,
        /// Rows the probe corrected from `record` to `file`.
        probed_to_file: usize,
        /// Rows the probe confirmed as `record` (the container is there).
        probed_to_record: usize,
        /// Rows where neither exists — the delimiter reading is kept and
        /// the row is marked.
        undecided: usize,
        /// Rows the frozen reader refused. Left exactly as they are.
        unreadable: usize,
    }

    /// Reads one column value and renders the tagged form for it, or
    /// `None` for a value the frozen reader refused.
    fn rewrite(raw: &str, row: &dyn std::fmt::Display, tally: &mut Tally) -> Option<String> {
        let Some(reading) = read_at_v63(raw) else {
            tally.unreadable += 1;
            tracing::warn!(
                event = "diag.locator_rewrite.unreadable",
                row = %row,
                locator = %raw,
                "a blank locator is not a value this column can hold; left as it stands"
            );
            return None;
        };
        let tagged = match reading {
            ReadAtV63::File(path) => TaggedAtV63::File {
                path: Cow::Owned(path),
            },
            ReadAtV63::Remote { scheme, target } => TaggedAtV63::Remote {
                scheme: Cow::Owned(scheme),
                target: Cow::Owned(target),
            },
            ReadAtV63::Logical(name) => TaggedAtV63::Logical {
                name: Cow::Owned(name),
            },
            ReadAtV63::Record { container, record } => {
                // The whole string as the delimiter reading left it —
                // `raw` may still carry a consumed `file://`, and the
                // path to probe is what came after it.
                let whole = format!("{container}#{record}");
                let is_file = |p: &str| std::fs::metadata(Path::new(p)).is_ok_and(|m| m.is_file());
                if is_file(&whole) {
                    // The `#` was part of a filename all along. This is
                    // the reading the delimiter got wrong.
                    tally.probed_to_file += 1;
                    TaggedAtV63::File {
                        path: Cow::Owned(whole),
                    }
                } else {
                    if is_file(&container) {
                        tally.probed_to_record += 1;
                    } else {
                        // Neither exists, so the filesystem answered
                        // neither way. Keep what the row has always
                        // meant and say out loud that it was not
                        // settled.
                        tally.undecided += 1;
                        tracing::warn!(
                            event = "diag.locator_rewrite.undecided",
                            row = %row,
                            locator = %raw,
                            container = %container,
                            "neither the whole locator nor its container is a file here, so \
                             the delimiter reading is kept rather than guessed at"
                        );
                    }
                    TaggedAtV63::Record {
                        container: Cow::Owned(container),
                        record: Cow::Owned(record),
                    }
                }
            }
        };
        let rendered = serde_json::to_string(&tagged).ok()?;
        tally.rewritten += 1;
        Some(rendered)
    }

    let mut tally = Tally::default();

    // The whole set up front rather than paged, for the reason the V56
    // walk gives: the transaction is open either way, and a cursor would
    // be machinery for resuming something that either commits or rolls
    // back whole.
    let assets: Vec<(Uuid, String)> = {
        let mut stmt = tx.prepare("SELECT id, source_locator FROM asset ORDER BY id")?;
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?
    };
    {
        let mut update = tx.prepare("UPDATE asset SET source_locator = ?1 WHERE id = ?2")?;
        for (id, raw) in &assets {
            if let Some(rendered) = rewrite(raw, &format_args!("asset {id}"), &mut tally) {
                update.execute(params![rendered, id])?;
            }
        }
    }

    let materials: Vec<(Uuid, i64, String)> = {
        let mut stmt =
            tx.prepare("SELECT asset_id, ord, locator FROM material ORDER BY asset_id, ord")?;
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?
    };
    {
        let mut update =
            tx.prepare("UPDATE material SET locator = ?1 WHERE asset_id = ?2 AND ord = ?3")?;
        for (asset_id, ord, raw) in &materials {
            if let Some(rendered) =
                rewrite(raw, &format_args!("material {asset_id}/{ord}"), &mut tally)
            {
                update.execute(params![rendered, asset_id, ord])?;
            }
        }
    }

    tracing::info!(
        event = "diag.locator_rewrite.completed",
        rows = assets.len() + materials.len(),
        rewritten = tally.rewritten,
        probed_to_file = tally.probed_to_file,
        probed_to_record = tally.probed_to_record,
        undecided = tally.undecided,
        unreadable = tally.unreadable,
        "locators rewritten from the delimited spelling into the tagged form"
    );
    Ok(())
}

/// Version 63 → 64: the **meta axis** — `material.meta_hash` /
/// `material.meta_kv`, and the `duplicate_conflict.axis` vocabulary
/// widened to admit it.
///
/// V41 added the artefact digest ("is this the same file") and V55 the
/// content-region digest ("is this the same picture"). The region is
/// *defined* as the bytes that survive into the decoded result, so the
/// metadata it drops is the exact complement — and nothing hashed that
/// complement. These columns do
/// ([`asterism_core::domain::material_meta`]), under their own
/// versioned tag so a later redefinition of the canonical form cannot
/// be compared against values computed under this one.
///
/// ```text
///    Artefact  =  Content  +  Meta
/// ```
///
/// `Artefact` agreement implies both of the others; neither of the
/// others implies anything about the rest. A generator emits both
/// shapes routinely — one picture re-exported with a caption written in
/// (`Content`, not `Artefact`), and a batch off one workflow whose
/// frames differ only by seed (`Meta`, not `Content`).
///
/// # Two columns, because a digest is the entrance and not the body
///
/// `meta_hash` is the index a lookup groups on. `meta_kv` is the same
/// bytes that were hashed — a JSON object, keys sorted, no whitespace,
/// values exactly as the container stated them — kept because exact
/// equality is the wrong question for metadata on its own: a batch off
/// one workflow differs by a seed, and a digest over the whole of it
/// separates precisely the rows that belong together. The hash answers
/// "made identically" cheaply; the useful question ("made the same way
/// apart from *this*") is a comparison over the object, and that wants
/// the metadata kept rather than only fingerprinted.
///
/// Home is `material` rather than `asset` because the metadata is a
/// fact about *these bytes*, and `material` is `1 : N` per asset with
/// `ord` held open for exactly this kind of divergence: a RAW and its
/// JPEG carry different embedded metadata and must not be made to share
/// one answer.
///
/// # Why the marker, and what finishes it
///
/// Same shape as V55, and for the same reason. The fingerprint walk
/// finds work by "a versioned column holds no answer", so leaving these
/// NULL on every pre-existing row would silently turn the pass for
/// newly arrived material into a re-read of the whole corpus, counted
/// in the "still fingerprinting" notice as ordinary ingest work. Every
/// existing row is answered here with
/// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED) —
/// "nothing walked these bytes", which is exactly true — and that
/// marker is what makes the deferred set *selectable*.
/// [`v65_walk_deferred_material_meta`] is the step that finishes it,
/// in the same chain, before the application serves anything.
///
/// `meta_kv` takes no marker: it holds an object or nothing, and its
/// emptiness is not a state anybody selects on.
///
/// # The `CHECK` cannot be altered, so the table is rebuilt
///
/// `CHECK (axis IN ('file', 'content'))` is a table constraint written
/// by V53, and SQLite has no `ALTER` for one. The rebuild is the
/// standard shape (`CREATE … _new` → `INSERT … SELECT` → `DROP` →
/// `RENAME`), the same one V44 used on `modality`, and it recreates
/// `idx_duplicate_conflict_open`, which goes with the dropped table.
/// The column list is V53's plus V54's `fold_exclusion`, carried
/// verbatim including its column-level CHECK.
///
/// Nothing references `duplicate_conflict`, so the rebuild is safe with
/// foreign keys enforced and this stays a `Step::Sql`; its own two
/// foreign keys are re-declared by name and re-bind to the same tables.
///
/// # `'file'` becomes `'artefact'`, in this rebuild and not a later step
///
/// The strongest axis hashes every byte — that is the **artefact**, not
/// a property of files — so the domain calls it
/// [`Artefact`](asterism_core::domain::duplicate_conflict::DuplicateAxis::Artefact).
/// The stored `axis` value **is** that axis rather than a column name,
/// so it moves with it (column names are a migration bought for
/// readability and stay; the axis rename is where the
/// confusion actually sat). Afterwards no row anywhere says `'file'` on
/// this axis, and
/// [`parse`](asterism_core::domain::duplicate_conflict::DuplicateAxis::parse)
/// refuses the word — the closed vocabulary has one spelling per axis
/// and a build that met the other one would be reading a database this
/// step never touched.
///
/// The mapping rides the rebuild's own `INSERT … SELECT` rather than
/// arriving as its own migration, because this step already rewrites
/// every row of the table: a separate step would rewrite them twice and
/// leave a version in between where the `CHECK` and the data disagree.
///
/// Two more surfaces carry the same slug and are rewritten here, in the
/// one step that owns the word:
///
/// - **`edge.label`** on the `identical_to` edge detection writes beside
///   the queue row ([`EdgeKind::IdenticalTo`](asterism_core::domain::edge::EdgeKind::IdenticalTo)).
///   The `UPDATE` is scoped by `kind`, because `label = 'file'` on any
///   other kind is not this vocabulary and must not be rewritten — the
///   label column is free text shared by every edge kind.
/// - **`asset.extra` `$._trace.declared_hash.axis`** — the note
///   [`declaration_claim`](asterism_core::domain::content_hash::declaration_claim)
///   writes to record which axis an importer's declared digest was
///   about. Nothing parses it back (the checker re-reads the axis off
///   the digest's own tag), but it is a stored slug from this
///   vocabulary, and a note left saying `'file'` would name an axis this
///   build no longer has. `json_set` touches that one path; the
///   `json_extract` guard means a row without the note, or without an
///   `extra` at all, is not rewritten.
const V64_MATERIAL_META: &str = r#"
ALTER TABLE material ADD COLUMN meta_hash TEXT;
ALTER TABLE material ADD COLUMN meta_kv TEXT;

UPDATE material SET meta_hash = 'unsupported:not-walked';

CREATE INDEX idx_material_meta_hash
    ON material(meta_hash) WHERE meta_hash IS NOT NULL;

CREATE TABLE duplicate_conflict_new (
    id             BLOB PRIMARY KEY,
    persona_id     BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    pair_lo        BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    pair_hi        BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    newcomer_id    BLOB NOT NULL,
    incumbent_id   BLOB NOT NULL,
    axis           TEXT NOT NULL,
    content_hash   TEXT NOT NULL,
    detected_at    INTEGER NOT NULL,
    resolved_at    INTEGER,
    resolution     TEXT,
    fold_exclusion TEXT CHECK (fold_exclusion IN ('lineage', 'dispatch')),
    UNIQUE (pair_lo, pair_hi, axis),
    CHECK (axis IN ('artefact', 'content', 'meta')),
    CHECK (resolution IS NULL OR resolution IN ('folded', 'kept')),
    CHECK ((resolved_at IS NULL) = (resolution IS NULL)),
    CHECK (pair_lo <> pair_hi)
) STRICT;

INSERT INTO duplicate_conflict_new
    (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id, axis,
     content_hash, detected_at, resolved_at, resolution, fold_exclusion)
SELECT id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
       CASE axis WHEN 'file' THEN 'artefact' ELSE axis END,
       content_hash, detected_at, resolved_at, resolution, fold_exclusion
  FROM duplicate_conflict;

DROP TABLE duplicate_conflict;
ALTER TABLE duplicate_conflict_new RENAME TO duplicate_conflict;

CREATE INDEX idx_duplicate_conflict_open
    ON duplicate_conflict(persona_id, detected_at DESC)
    WHERE resolved_at IS NULL;

UPDATE edge
   SET label = 'artefact'
 WHERE kind = 'identical_to' AND label = 'file';

UPDATE asset
   SET extra = json_set(extra, '$._trace.declared_hash.axis', 'artefact')
 WHERE json_extract(extra, '$._trace.declared_hash.axis') = 'file';
"#;

/// Version 64 → 65: compute the meta-axis values V64 deferred — **the
/// second half of one migration, not a feature.**
///
/// V64 added the columns and wrote
/// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
/// into every existing row, because filling them in means reading every
/// original off disk and that could not run in the statement that added
/// them. The marker is the record of which rows were left; this is the
/// pass that finishes them. Until it runs the meta axis answers about
/// the fraction of the library that arrived after V64, which is none of
/// it.
///
/// The reasoning V56 set out applies unchanged and is not repeated
/// here: why this is a migration step rather than a background job with
/// a button, why the whole set is taken up front rather than paged, and
/// why nothing needs to survive being interrupted ([`migrate`] runs the
/// step and the `user_version` bump in one transaction).
///
/// # It reads through the live reader, and that is deliberate
///
/// [`v56_walk_deferred_content_regions`] froze a copy of its locator
/// predicate because a landed migration reads *columns* and has to keep
/// meaning what it meant when it ran. This one does not need to: the
/// locators it reads were rewritten into the tagged form by V63, which
/// is behind it, so it parses them with the live
/// [`SourceLocator`](asterism_core::domain::source_locator::SourceLocator)
/// — the same reader every other consumer uses. What it must not do is
/// invent a second reading of the same column, and going through the
/// type is what prevents that.
///
/// # No fold, no conflict
///
/// Like V56: this writes digests and nothing else. Pre-existing
/// meta-axis pairs reach the user through the duplicate report, not the
/// question queue. A pair found on this axis *after* the upgrade — a
/// newly imported file matching one of these digests — is the ordinary
/// detection path's business, and that path already walks every axis.
///
/// # What a row still carrying the marker afterwards means
///
/// Its original could not be opened: moved, deleted, on a disk that was
/// not connected. The statement stays exactly true of it, and it keeps
/// its artefact-axis digest and grouping, so what is missing is the
/// improvement and never a row.
fn v65_walk_deferred_material_meta(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    use asterism_core::domain::content_hash::UNHASHABLE;
    use asterism_core::domain::content_region::NOT_WALKED;
    use asterism_core::domain::source_locator::SourceLocator;
    use asterism_core::domain::value::MimeType;

    use crate::fingerprint::{MAX_CONTENT_WALK_BYTES, hash_artefact};

    /// One row V64 deferred: where its bytes are, and what format the
    /// row believes they are.
    struct Deferred {
        asset_id: Uuid,
        ord: i64,
        locator: String,
        mime: Option<String>,
    }

    let pending: Vec<Deferred> = {
        let mut stmt = tx.prepare(
            "SELECT asset_id, ord, locator, mime \
               FROM material \
              WHERE meta_hash = ?1 \
              ORDER BY asset_id, ord",
        )?;
        stmt.query_map(params![NOT_WALKED], |row| {
            Ok(Deferred {
                asset_id: row.get(0)?,
                ord: row.get(1)?,
                locator: row.get(2)?,
                mime: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    if pending.is_empty() {
        return Ok(());
    }

    let mut update = tx.prepare(
        "UPDATE material SET meta_hash = ?1, meta_kv = ?2 \
          WHERE asset_id = ?3 AND ord = ?4",
    )?;

    let mut walked = 0usize;
    let mut no_bytes = 0usize;
    let mut unreadable = 0usize;
    let mut unparsable = 0usize;

    for row in &pending {
        // A locator with no file behind it — a record inside a
        // container, a remote address — can never have a digest on any
        // axis, and saying so is what keeps the row out of every future
        // walk. A column this cannot parse is left alone instead: the
        // marker is still true of it, and writing a verdict from a
        // value nothing could read would be inventing one.
        let locator = match SourceLocator::try_from(row.locator.as_str()) {
            Ok(locator) => locator,
            Err(err) => {
                unparsable += 1;
                tracing::warn!(
                    event = "diag.meta_walk.unreadable_locator",
                    asset_id = %row.asset_id,
                    ord = %row.ord,
                    error = %err,
                    "left carrying the not-walked marker"
                );
                continue;
            }
        };
        let Some(path) = locator.local_path() else {
            no_bytes += 1;
            update.execute(params![
                UNHASHABLE,
                Option::<String>::None,
                row.asset_id,
                row.ord
            ])?;
            continue;
        };

        // Parsed here for the same reason the repository parses: the
        // walk decides on the format, and a raw column value has not
        // been normalised yet.
        let declared = row.mime.as_deref().map(MimeType::parse);
        match hash_artefact(
            &path.to_string_lossy(),
            declared.as_ref(),
            MAX_CONTENT_WALK_BYTES,
        ) {
            Ok(fingerprint) => {
                walked += 1;
                // Only this axis is written. The other two were
                // answered by the passes that own them, and a value
                // recomputed here landing on top of one of those would
                // make this step a re-fingerprint of the library
                // wearing a meta-axis name.
                update.execute(params![
                    pre_v92_stored_value(&fingerprint.meta),
                    fingerprint.meta_kv,
                    row.asset_id,
                    row.ord
                ])?;
            }
            Err(err) => {
                unreadable += 1;
                tracing::warn!(
                    event = "diag.meta_walk.unreadable",
                    asset_id = %row.asset_id,
                    ord = %row.ord,
                    error = %err,
                    "left carrying the not-walked marker"
                );
            }
        }
    }

    tracing::info!(
        event = "diag.meta_walk.completed",
        pending = pending.len(),
        walked,
        no_bytes,
        unreadable,
        unparsable,
        "meta-axis values computed for materials that predate the columns"
    );
    Ok(())
}

/// Version 65 → 66: `material_mark` replaces `asset_timeline_mark`.
///
/// Same rows, one axis added in front of them. V60 named the
/// coordinate space in the table name, so a mark on an image plane
/// would have been a second table with a second adapter and a second
/// port. The space becomes a column instead: `anchor_kind` says which
/// one a row is expressed in, and the columns that follow are that
/// space's coordinates (`start_ms` / `end_ms` for `'temporal'`). The
/// domain side is `MaterialAnchor`, one variant per space.
///
/// **The DROP loses nothing.** V60 shipped the table, and nothing in
/// the product ever wrote to it: no service holds the port, no command
/// or route reaches one, and the adapter is only reachable from its own
/// tests. So there is no data to carry across and no migration step to
/// write — which is the reason this is a replacement rather than an
/// `ALTER`, and the reason it can be. A release later and the answer
/// would have been a copy with a `'temporal'` literal in the SELECT.
///
/// `IF EXISTS` on the DROP because a database that has run V60 always
/// has the table, and one restored from a partial chain might not; the
/// index goes with the table without being named.
///
/// Carried over from V60 unchanged, with the reasoning that put them
/// there:
///
/// - **No CHECK on `body`.** The domain requires it non-empty after a
///   Unicode trim; SQLite's one-argument `trim(X)` strips only U+0020,
///   so `CHECK (length(trim(body)) > 0)` would pass `'\t'`, `'\n'`,
///   U+00A0 and U+3000 while looking like a mirror of the rule. The
///   adapter routes every row back through the constructor instead
///   (`MaterialMark::rehydrate`), and the rule stays in one place.
/// - `author_persona_id` is `ON DELETE CASCADE`, unlike V15's
///   `SET NULL` on `asset_comment`. `SET NULL` does not survive the
///   pairing CHECK: an FK action runs as an ordinary UPDATE, so
///   nulling the id while `author_kind` still reads `'persona'` fails
///   the CHECK and aborts the `DELETE FROM persona` itself. `RESTRICT`
///   is available and rejected for a different reason — the ordered
///   sweep in `repo/persona.rs::purge` exists because SQLite does not
///   order sibling-table cascades, and every RESTRICT child added is
///   another line that has to be remembered there. CASCADE closes in
///   the DDL. The consequence is deliberate: purging a persona removes
///   the marks it left, including those on other personas' assets.
/// - `STRICT`, and `CHECK (start_ms >= 0)` — the receiver for a
///   write-side conversion that wrapped. Both are V60's; an
///   append-only chain cannot add either later without rebuilding the
///   table, so they are cheaper to keep than to re-derive.
/// - Rows are not constrained against each other. Overlapping and
///   exactly-coincident marks on one asset are allowed — overlap is
///   the premise of the model (IIIF Ranges), and de-duplication is the
///   caller's question, not the table's.
///
/// New in this version:
///
/// - `anchor_kind` is `NOT NULL` with a closed `CHECK`, so the axis is
///   in the schema and not only in the Rust enum. Adding `'spatial'`
///   means widening that CHECK and adding the plane's columns — SQLite
///   cannot alter a CHECK in place, so that migration rebuilds the
///   table. Pre-release, that costs a batch and nothing else.
/// - `start_ms` becomes nullable, since a non-temporal anchor has no
///   millisecond to put there, and
///   `CHECK (anchor_kind <> 'temporal' OR start_ms IS NOT NULL)` keeps
///   it mandatory for the anchor that does. Per-kind column
///   requirements live in CHECKs like this one rather than in NOT NULL,
///   which cannot be conditional.
/// - The index stays `(asset_id, start_ms)`, unchanged in shape and
///   renamed with the table. `EXPLAIN QUERY PLAN` on the adapter's
///   listing statement (SQLite 3.43.2, 2026-08-06) is two lines:
///
///   ```text
///   SEARCH material_mark USING INDEX idx_material_mark_asset_start (asset_id=?)
///   USE TEMP B-TREE FOR RIGHT PART OF ORDER BY
///   ```
///
///   The second line is the `id` tie-break; drop `, id` from the
///   statement and it disappears. So the tie-break is a real sort, not
///   something the scan hands over for free — worth knowing before
///   anyone reads "index-backed" as "already ordered". No interval
///   index (R-tree) — there is no range query yet to serve with one.
///
/// **Left for the second anchor kind**: a `'spatial'` row has
/// `start_ms IS NULL`, and SQLite sorts NULL first, so it would head
/// every `ORDER BY start_ms` listing. Nothing to do about it while
/// `'temporal'` is the only kind that exists; the listing's order is to
/// be decided again — in the port doc as well as here — when it is not.
const V66_MATERIAL_MARK: &str = r#"
DROP TABLE IF EXISTS asset_timeline_mark;

CREATE TABLE material_mark (
    id                BLOB PRIMARY KEY,
    asset_id          BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    anchor_kind       TEXT NOT NULL CHECK (anchor_kind IN ('temporal')),
    start_ms          INTEGER,
    end_ms            INTEGER,
    body              TEXT NOT NULL,
    author_kind       TEXT NOT NULL CHECK (author_kind IN ('user', 'persona')),
    author_persona_id BLOB REFERENCES persona(id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    edited_at         INTEGER,
    CHECK (
        (author_kind = 'user'    AND author_persona_id IS NULL)
     OR (author_kind = 'persona' AND author_persona_id IS NOT NULL)
    ),
    CHECK (anchor_kind <> 'temporal' OR start_ms IS NOT NULL),
    CHECK (start_ms IS NULL OR start_ms >= 0),
    CHECK (end_ms IS NULL OR end_ms > start_ms)
) STRICT;

CREATE INDEX idx_material_mark_asset_start
    ON material_mark(asset_id, start_ms);
"#;

/// Version 67 → 68: `asset_comment` lets a Persona author survive its
/// Persona. V15 wrote two rules that cannot both hold, and the pairing
/// one wins in the worst possible way.
///
/// # The contradiction
///
/// V15's own doc says `ON DELETE SET NULL` "keeps the comment even when
/// the Persona is deleted (renders as "(deleted persona)" downstream)".
/// The pairing `CHECK` beside it forbids exactly that row. An FK action
/// runs as an ordinary `UPDATE`, so `SET NULL` writes
/// `author_persona_id = NULL` while `author_kind` still reads
/// `'persona'`, the `CHECK` refuses it, and the refusal propagates
/// outward: the `DELETE FROM persona` itself aborts. The intended state
/// was unreachable, and the cost of it being unreachable was not a
/// rejected comment but a Persona that cannot be deleted.
///
/// [measured 2026-08-06, SQLite 3.43.2, `PRAGMA foreign_keys = ON`, issue
/// `cb2d2273`]: with one Persona-authored comment present,
/// `DELETE FROM persona` raises `CHECK constraint failed` and the row
/// count is unchanged.
///
/// # Why it had to be the schema that moved
///
/// Both live deletion paths are reachable, and the automatic one fails
/// quietly: `RetentionService::purge_expired` logs
/// `diag.retention.purge_failed` and counts the Persona as `skipped`, so
/// a Persona that ever wrote one comment is skipped by every sweep from
/// then on and accumulates in the trash forever. The hand path
/// (`POST /asterism/personas/purge`) at least surfaces the error. Left
/// alone, the "known limitation" reading would have made a silent leak
/// the specification.
///
/// # What the new `CHECK` says
///
/// Only the direction that was ever load-bearing: a `'user'` comment has
/// no persona id. The `'persona'` side no longer demands one, which is
/// what admits the headstone row (`author_kind = 'persona'`,
/// `author_persona_id IS NULL`) the FK action wants to write.
///
/// Stated as an implication rather than as the surviving half of the old
/// disjunction, matching V60's `anchor_kind <> 'temporal' OR ...` — a
/// `CHECK` naming one kind and one column reads as the rule it is.
///
/// # Not the same call as `material_mark`
///
/// V60/V66 gave `material_mark.author_persona_id` `ON DELETE CASCADE`,
/// and this step does **not** follow it. That was a choice made under
/// the constraint this step removes: `SET NULL` provably did not work,
/// `RESTRICT` costs a line in `repo/persona.rs::purge`'s ordered sweep,
/// so `CASCADE` closed it in the DDL. A comment is prose somebody wrote
/// and a mark is a coordinate, and the deliberate asymmetry — comments
/// outlive their author, marks do not — is now a decision rather than a
/// workaround (decided 2026-08-07). The per-entity deletion semantics
/// get reconciled as a whole in their own pass, not here.
///
/// # Rebuild, not `ALTER`
///
/// SQLite cannot drop a table `CHECK`. Same shape as V64's
/// `duplicate_conflict` rebuild: build beside, copy, drop, rename,
/// recreate the index. The copy is a straight column-for-column
/// `SELECT` — no row changes meaning here, only the constraint does.
const V68_ASSET_COMMENT_KEEPS_ORPHANS: &str = r#"
CREATE TABLE asset_comment_new (
    id                BLOB PRIMARY KEY,
    asset_id          BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    author_kind       TEXT NOT NULL,
    author_persona_id BLOB REFERENCES persona(id) ON DELETE SET NULL,
    body              TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    edited_at         INTEGER,
    CHECK (author_kind IN ('user', 'persona')),
    CHECK (author_kind <> 'user' OR author_persona_id IS NULL)
) STRICT;

INSERT INTO asset_comment_new
    (id, asset_id, author_kind, author_persona_id, body, created_at, edited_at)
SELECT id, asset_id, author_kind, author_persona_id, body, created_at, edited_at
  FROM asset_comment;

DROP TABLE asset_comment;
ALTER TABLE asset_comment_new RENAME TO asset_comment;

CREATE INDEX idx_asset_comment_asset
    ON asset_comment(asset_id, created_at);
"#;

/// Version 68 → 69: pixel dimensions on `asset`.
///
/// Two nullable `INTEGER`s with no default, the shape `rating` (V13) and
/// `palette` (V14) already use: an existing row has never been measured,
/// and `NULL` is the only value that says so. A `DEFAULT 0` would make
/// every asset in the library claim a zero-pixel resolution, which sorts
/// ahead of every real one on an ascending axis — the reading
/// `duration_ms` refuses for the same reason.
///
/// # What the two columns mean
///
/// The **coded** dimensions of the stored bytes, orientation not applied.
/// That is what the parsers measure (the image side reads EXIF
/// Orientation into `extra` and leaves the dimensions alone), so a photo
/// that displays portrait under Orientation 5-8 is stored here as its
/// landscape pair. Display dimensions are a later question and a
/// different one.
///
/// # No `CHECK` for the pair
///
/// The invariant is `width_px IS NULL` ⇔ `height_px IS NULL`, and it is
/// **not** in the DDL. Expressing it needs a table-level `CHECK` (a
/// column constraint cannot read its neighbour), and `ALTER TABLE ADD
/// COLUMN` cannot carry one — it would take the build-beside-copy-drop
/// -rename rebuild V68 above just did to the whole `asset` table, for two
/// columns. The pair is asserted on the write side instead
/// (`AssetService::add` refuses a half; the importer road cannot state
/// one), on the same terms V47 / V50 state for the attribution triple.
///
/// Not indexed: no filter band and no sort axis reads these yet, and an
/// index for a query nobody makes is a write cost with no reader.
///
/// That held for exactly one wave. V70 below puts the pair into the two
/// live covering indexes, because the band and the axis arrived and the
/// light row started selecting their product — the reader the paragraph
/// above was waiting for.
const V69_ASSET_PIXEL_DIMS: &str = r#"
ALTER TABLE asset ADD COLUMN width_px INTEGER;
ALTER TABLE asset ADD COLUMN height_px INTEGER;
"#;

/// Version 69 → 70: the two live covering indexes carry `width_px` /
/// `height_px`, because `IndexRow` now selects their product.
///
/// The third occurrence of one failure, and the reason it keeps
/// happening is worth stating plainly: the grid sorts **index rows**, so
/// an axis the light row cannot express is an axis the grid cannot
/// offer — but every column added to satisfy that demotes the listing
/// plan from COVERING back to a seek with one table-page lookup per hit
/// unless the index grows with it. V57 established the shape, V59 paid
/// it for `duration_ms` + `file_size_bytes`, and this pays it for the
/// resolution axis. The cost avoided is measured: ~2.0 s cold at 110 k
/// rows (see the V17 comment).
///
/// **The two base columns, not the product.** `IndexRow` selects
/// `(width_px * height_px)`, and SQLite computes that from the index
/// entry when both operands are in it — an expression index would serve
/// this one statement and nothing else, while the pair also serves the
/// band in `QueryParts::build`, which compares the same product.
///
/// Shape copied from V59 verbatim apart from the two added columns:
/// still partial over the live default, still carrying `trashed_at` /
/// `folded_into` in the key for the reason V57 spells out. The guard
/// that catches the next occurrence is
/// `the_live_grid_listing_is_served_by_the_covering_index`, which builds
/// its statement from the real `IndexRow::COLUMNS` — it is what caught
/// this one, before the change left the workspace.
const V70_ASSET_INDEX_COVERING_PIXELS: &str = r#"
DROP INDEX idx_asset_persona_occurred_cover;
DROP INDEX idx_asset_occurred_cover;

CREATE INDEX idx_asset_persona_occurred_cover
    ON asset(persona_id, occurred_at DESC, id, modality, labels, created_at,
             updated_at, duration_ms, file_size_bytes, width_px, height_px,
             role, trashed_at, folded_into)
    WHERE trashed_at IS NULL AND folded_into IS NULL;
CREATE INDEX idx_asset_occurred_cover
    ON asset(occurred_at DESC, id, persona_id, modality, labels, created_at,
             updated_at, duration_ms, file_size_bytes, width_px, height_px,
             role, trashed_at, folded_into)
    WHERE trashed_at IS NULL AND folded_into IS NULL;
"#;

/// Version 70 → 71: `asset.dims_probed_at` — the record that a row's
/// bytes were *looked at*, whatever the answer.
///
/// # Why a second column and not the absence of the first
///
/// The backfill walk selects rows whose dimensions are unknown. Without
/// this column that predicate is `width_px IS NULL`, and a row nothing
/// can measure — a text note, an AVI neither video probe reads, a
/// corrupt JPEG, a locator with no local bytes — never stops matching
/// it. That is the failure `material_hash` states in its own words: *a
/// NULL would put the row back in front of every future backfill pass —
/// a walk that never shrinks*. The startup seed is what makes it bite;
/// the property that pass is documented to have ("a startup on an
/// already-hashed library costs one query") is the one this column buys
/// here.
///
/// `material_hash` solves it with a sentinel *inside* the answer
/// (`unhashable:no-bytes` in a TEXT column). **That is not available
/// here**: `width_px` / `height_px` are INTEGER and every value they can
/// hold is a real measurement — including `0`, which
/// `asterism_contract::query::ListAssetsQuery::pixels_min` deliberately
/// treats as a value rather than as absence. A sentinel would have to be
/// a number that means "not a number".
///
/// So the fact moves to its own column, and the three states become
/// distinguishable rather than two of them sharing a spelling:
///
/// | `dims_probed_at` | `width_px` | reading |
/// |---|---|---|
/// | `NULL` | `NULL` | nobody has looked |
/// | set | `NULL` | looked, and these bytes state no dimensions |
/// | set | set | measured |
///
/// The pair invariant is untouched: this column says nothing about
/// `width_px` ⇔ `height_px`, which stays the write path's to enforce.
///
/// Not indexed. The walk orders by `id` and reads the column as a
/// filter, so it is one sequential pass either way, and the only query
/// that touches it runs once per library.
///
/// Not in the covering indexes V70 widened, deliberately: `IndexRow`
/// does not select it, so adding it would grow every live listing's
/// index entry to serve a job that runs once.
const V71_ASSET_DIMS_PROBED_AT: &str = r#"
ALTER TABLE asset ADD COLUMN dims_probed_at INTEGER;
"#;

/// Version 71 → 72: forget the content-axis answer that says nothing
/// walks `image/jpeg`, because something does now.
///
/// A JPEG probe landed. Every material imported before it holds
/// `unsupported:image/jpeg` in `content_region_hash` — the value
/// [`content_region::unsupported_format`](asterism_core::domain::content_region::unsupported_format)
/// renders for a format no probe claims — and
/// [`is_axis_answer`](asterism_core::domain::content_hash::is_axis_answer)
/// reads any `unsupported:` value as a **final** answer. So
/// [`needs_fingerprint`](asterism_core::domain::content_hash::needs_fingerprint)
/// says those rows are done, the ordinary walk never returns to them, and
/// the column ends up holding two meanings at once: JPEGs imported after
/// the probe carry a `cr1-` digest and group with their re-encodings,
/// JPEGs imported before it carry a statement that stopped being true,
/// and nothing distinguishes a library's two halves except when a file
/// happened to arrive. This `UPDATE` writes NULL over the stale half.
///
/// # Not the version-bump case
///
/// [`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk)
/// reserves an announced, user-timed upgrade moment for the day the
/// region definition is versioned up (`cr1-` → `cr2-`), and this step
/// must not be read as the precedent for it. The two invalidate opposite
/// things. A bump invalidates **positives**: every digest ever written,
/// on every format, on a library of any size — measurements being
/// discarded, and a whole disk re-read to replace them. This invalidates
/// **negatives** on one format: rows saying "nothing here reads JPEG",
/// which was true when written and is not now. No digest is discarded,
/// nothing that was ever read is read again, and the set is bounded by
/// how many JPEGs one library holds rather than by the library.
///
/// # Why NULL rather than a marker
///
/// V55 wrote [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
/// instead of leaving NULL, and the difference is what finishes the set.
/// That marker is the record of rows a **migration-chain read** will
/// come back for ([`v56_walk_deferred_content_regions`]), and that shape
/// is affordable only while the set is what one small library held when a
/// column arrived; opening every JPEG in an arbitrary library inside one
/// transaction, before the app will start, is the act V56's own doc rules
/// out.
///
/// NULL is the value the *ordinary* fingerprint walk selects, and that
/// pass is the one built for reads that must not happen inside a
/// transaction: `core_init` enqueues a batch `MaterialHash` at startup
/// when none is pending, the walk self-chains through a cursor, and it is
/// idempotent. Nothing else is needed here — no job kind, no hook, no
/// second predicate — because a row with NULL on this axis is already
/// exactly what that walk is looking for. Counting these rows in the
/// "still fingerprinting" notice is honest: on this axis they really are
/// unread.
///
/// # What NULL costs a row whose original is gone
///
/// The walk writes an answer for a file it can read and **nothing** for
/// one it cannot: an unreadable original is a `HashOutcome::Skipped`, a
/// `diag.material_hash.unreadable` line, and no column touched — only a
/// locator with no bytes *of its own* is given a marker. So a pre-probe
/// JPEG whose original was moved or deleted outside Album held a final
/// answer before this step and no pass came near it; after it the row is
/// NULL, `core_init` enqueues the backfill at every launch, and the row
/// costs one failed open and one log line on each of them, permanently.
/// The duplicate report's `unhashed_count` counts it and stops
/// converging: for these rows the "still fingerprinting" notice never
/// clears.
///
/// That is precisely the property
/// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
/// buys, and [`v56_walk_deferred_content_regions`] leans on it — it
/// *keeps* the marker on a file it could not read, because "nothing
/// walked these bytes" is what remains true of it. Not reaching for the
/// same marker here is a judgement about which rows these are: an
/// original the library can no longer find is a fact about somebody's
/// disk that they should learn, and settling the row quietly is how it
/// stays unlearnt. What is defective is that the only surface it has is
/// a counter that will not reach zero — that is the general problem, an
/// unreachable original visible in the log and retried for ever, and it
/// is tracked separately, rather than answered by a numbered migration
/// reaching past its own set.
///
/// Measured rather than guessed at: a full content-axis rebuild of the
/// developer library — 289 materials, 129 of them file-backed, 6.1 MiB —
/// left exactly 9 rows unanswered, matching its 9
/// `diag.material_hash.unreadable` lines. That library holds no JPEG, so
/// this step clears nothing on it; the 9 are the size of the unreadable
/// set on a real library, not what this step adds to one.
///
/// # The literal is frozen, and pinned
///
/// The `WHERE` names `unsupported:image/jpeg` and deliberately does
/// **not** ask the probe registry which mimes are claimed. A migration is
/// a statement about what version 72 did, and that has to have the same
/// answer in a year: a registry-driven step would clear a different set
/// on every database it ran on, growing as probes are added, and no
/// reader could reconstruct why a given row is NULL. What a hand-typed
/// literal risks instead is drifting from what the code writes, so
/// `the_jpeg_marker_this_migration_clears_is_the_one_the_domain_renders`
/// pins the string against the domain's own rendering — if the
/// vocabulary moves, that test names this migration. It reads the
/// `WHERE` whole while it is there, so a second predicate added beside
/// this one fails the same test: a widening is otherwise invisible,
/// since no fixture holds a row it would newly reach.
///
/// # What it does not touch
///
/// The **content column only**. `meta_hash` legitimately holds the same
/// marker: the JPEG probe declares `meta: false`, so no reading exists to
/// replace it and a row cleared there would come back holding what it
/// holds now. Clearing content alone is enough anyway —
/// `needs_fingerprint` is an OR across the three columns, and the pass
/// recomputes all three from one read.
///
/// **The first sentence of that paragraph stopped being true**, and it
/// is left standing because a migration's doc is a statement about what
/// its own version did. The probe claimed the meta axis once the series
/// axis made a narrow reading of EXIF expressible, and
/// [`V76_CLEAR_STALE_JPEG_META_MARKER`] is the same `UPDATE` over the
/// other column. Nothing about this step changed: a database that ran
/// version 72 and stopped there is in exactly the state described above.
///
/// And **that one marker**, by equality rather than by prefix. Every
/// other value under the tag is either an answer no read improves on or
/// somebody else's work: `unsupported:too-large` (the size gate is
/// unchanged, so the file would be skipped and re-marked),
/// `unsupported:empty-span` (a walk ran and found no region),
/// `unsupported:not-walked` (the deferred set
/// [`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk)
/// owns — clearing it would hand one row to two passes),
/// `unhashable:no-bytes`, and every `cr1-` digest.
///
/// No index changes: `idx_material_content_region_hash` is partial on the
/// non-NULL side, so the cleared rows simply leave it and rejoin when the
/// walk answers them.
const V72_CLEAR_STALE_JPEG_CONTENT_MARKER: &str = r#"
UPDATE material SET content_region_hash = NULL
    WHERE content_region_hash = 'unsupported:image/jpeg';
"#;

/// Version 72 → 73: the series axis gets its two tables — the rules
/// (`series_strategy`) and what they concluded (`material_series`) —
/// and one rule is seeded.
///
/// The axis is [`series`](asterism_core::domain::series): *made the same
/// way*, derived by applying a [`Strategy`](asterism_core::domain::series::Strategy)
/// to a material's `meta_kv` and never touching `m1-`. Two tables
/// because the rule is a registered value and the key is a derivation
/// from it, and the second is discardable in a way the first is not —
/// re-deriving a whole library reads `meta_kv` and no disk at all (the
/// [`series`](asterism_core::domain::series) module doc argues it),
/// which is the property everything below leans on.
///
/// # `series_strategy`, on the `modality` master's terms
///
/// A migration seeds system rows and a person adds more (V22 +
/// `ModalityService`). The shape is the same and one thing about it is
/// not: `modality`'s slug is its identity, so a row there cannot be
/// renamed, while a Strategy is keyed by a surrogate id precisely so it
/// *can* be — `name` is a label, and
/// [`StrategyId`](asterism_core::domain::value::StrategyId) is what
/// derived rows are filed under.
///
/// **`include` / `exclude` are JSON arrays of arrays, not a side
/// table.** They are one value rather than a set of entities: a path is
/// itself a variable-length sequence, so normalising means either two
/// ordinal levels (`strategy_id, kind, path_ord, segment_ord, segment`)
/// or a JSON array per row anyway, and nothing addresses one path — the
/// whole list is read whenever the rule is, and written whenever it is
/// edited. The shape that a side table would quietly change is the
/// round trip: keyed by `(strategy_id, path)` it deduplicates and loses
/// order, so a rule would come back spelled differently from the way its
/// author registered it. That does not move a key —
/// [`select`](asterism_core::domain::series) files by path in a
/// `BTreeMap`, so repetition and order are already immaterial to the
/// digest — which is exactly why the loss would go unnoticed until
/// somebody read their own rule back and found it edited.
///
/// **`name` carries no uniqueness constraint, deliberately.** Two
/// arguments, and the second is the one that decides it. The soft one:
/// the name is documented as display-only ("a rename must not move a
/// single key"), so a `UNIQUE` would be an invitation to treat it as
/// identity — the mistake `bucket`'s own `UNIQUE (persona_id, name)`
/// makes, which is one of the four structural reasons a series is not a
/// Group ([`series`](asterism_core::domain::series), "A key on the
/// material, and not a Group"): it made "one rule, N groups"
/// unspellable there. The hard one: a later migration seeding a second
/// system rule would abort on any database where a person had already
/// used that name for their own, and a numbered step that fails on
/// somebody's library because of a word they chose is not a step that
/// can ship. Two rules called the same thing are two ids and two
/// populations of keys; nothing collapses.
///
/// **`system` is provenance, not permission.** It records that a
/// migration wrote the row, and it grants the row no protection: a
/// person may edit or delete a seeded rule like any other. Refusing
/// would refuse the thing the design is for — a Strategy is meant to
/// be iterated on while watching the groups move — and the
/// worst a deletion does here is cascade away derived keys, which cost
/// a scan to rebuild and no disk read. (Contrast `ModalityService`'s
/// delete guard, which exists because a deleted slug orphans assets and
/// nothing can recompute *them*.) What a future migration wanting to
/// correct a seeded rule needs is not a lock but a way to tell a
/// pristine row from one somebody took over, and it already has one:
/// `system = 1 AND updated_at = created_at` — every write path stamps
/// `updated_at`, and this seed sets the pair equal.
///
/// **That was a promise about code that did not exist**, and it is worth
/// naming as one: when this step was written nothing could edit a rule
/// at all, so "every write path" quantified over a single `INSERT` and
/// the sentence could not have been false. It is kept now.
/// [`SeriesRepository::update_strategy`](asterism_core::domain::repository::SeriesRepository::update_strategy)
/// is the write path that arrived with the registration surface
/// (`PATCH /asterism/series-strategies/{id}`), it names `updated_at` in
/// its `SET` list unconditionally, and it leaves `system` and
/// `created_at` out of that list entirely — so an edited seed still
/// answers to `system = 1` while failing the equality, which is exactly
/// the pair the paragraph above needs.
/// `an_edit_moves_the_stamp_and_keeps_the_seed_addressable_as_one`
/// asserts both halves against the seeded row, and fails when the stamp
/// is dropped from the statement.
///
/// **The other edge of that, stated because the paragraph above only
/// names one of them**: the test is `updated_at = created_at`, and the
/// write path stamps unconditionally, so a `PATCH` carrying an empty
/// body — every field omitted, nothing about the rule changed — moves
/// the stamp and disqualifies the seed from every later corrective
/// migration. The row is then permanently "somebody's", on the strength
/// of a request that altered nothing. That is the safe direction (a step
/// that no-ops leaves a working rule alone, where the alternative
/// overwrites one somebody wrote), and it is the direction a conditional
/// stamp would give up: making the write compare first would mean an
/// edit that happened to restore the seeded values reads as pristine
/// again. It is named here because a later author reading only the
/// paragraph above would take `updated_at <> created_at` to mean *this
/// rule was changed*, and what it means is *this rule was written to*.
///
/// **So a corrective migration is an `UPDATE`, never an `INSERT`.** The
/// permission granted above has a cost and it lands on the next author,
/// because the frozen id makes the wrong form the natural one to reach
/// for: `INSERT OR REPLACE` / `INSERT OR IGNORE` addressed at
/// `X'019fe8f8…'` reads as "make sure the rule is there and correct",
/// and on a library where somebody deleted it that statement **puts it
/// back**. Deleting a system rule is a decision — it stops a Strategy
/// from being derived at all — and a numbered step that silently undoes
/// it is worse than one that does nothing. The form that behaves:
///
/// ```sql
/// UPDATE series_strategy SET include = '…', updated_at = <literal>
///  WHERE id = X'019fe8f8140070008000000000000001'
///    AND system = 1 AND updated_at = created_at;
/// ```
///
/// which no-ops on a deleted row (nothing matches) and on an edited one
/// (the stamps differ), and touches only the row this migration wrote
/// and nobody has since taken over.
///
/// # `material_series`, and the three answers kept apart
///
/// [`SeriesKey`](asterism_core::domain::series::SeriesKey) has three
/// values and `key IS NULL` can only spell two of them, so `outcome`
/// carries the distinction and
/// `CHECK ((outcome = 'derived') = (key IS NOT NULL))` ties the pair
/// together in both directions — a `derived` row with no key and a
/// `not_applicable` row carrying one are equally refused. Without it the
/// pair is held only by whichever writer happened to be careful, and the
/// distinction being kept is the one between *this rule is not about
/// this material* (working as written) and *this rule is about it and
/// selected nothing* (a rule to fix). Collapsing those is the failure
/// [`MaterialMeta`](asterism_core::domain::material_meta::MaterialMeta)
/// keeps three states to avoid on the axis below.
///
/// No `CHECK` on the key's spelling. `sk1-` is a **versioned** tag whose
/// point is that two generations can sit in one column while a library
/// is re-derived (the [`SERIES_KEY_PREFIX`](asterism_core::domain::series::SERIES_KEY_PREFIX)
/// doc); a schema-level prefix test would make the next generation need
/// a migration before a single key could be written under it. The two
/// hash columns on `material` carry no such CHECK either, for the same
/// reason.
///
/// The index is `(strategy_id, key)`, partial on the non-NULL side —
/// `idx_material_content_hash`'s shape (V41). It serves the one question
/// the axis is for, "which materials did this rule put on this key",
/// with the rule leading because a key is only meaningful under one; the
/// NULL rows are the two outcomes that are not groupings, so an index
/// entry for them would be an entry no lookup can ever want.
/// `the_series_lookup_is_served_by_the_strategy_key_index` measures the
/// plan.
///
/// **Nothing here indexes `strategy_id` on its own**, and the paragraphs
/// above lean on a path that fact makes expensive, so the price is
/// stated rather than implied. The primary key leads with `asset_id`,
/// and the index above is partial on `key IS NOT NULL`, which a bare
/// `strategy_id = ?` does not imply — so **every per-strategy statement
/// is a full scan of `material_series`**. That is SQLite's cascade when
/// a rule is deleted (the permission granted above), and it is whatever
/// the re-derivation sweep turns out to be — its trigger was still open
/// when this step was written. At one derived row per material per rule
/// the table is the library times the rules, so "delete a rule and watch
/// the groups move" costs a scan of that, not a seek.
///
/// The index for it is **not** added here, and the answer came in
/// **[`V74_MATERIAL_SERIES_STRATEGY`], which adds it.** What was waited
/// for was the statement: that open trigger closed onto "delete that
/// rule's rows and let the walk re-derive them"
/// ([`SeriesRepository::scan_underived`](asterism_core::domain::repository::SeriesRepository::scan_underived)),
/// so the per-strategy `DELETE` this paragraph priced is not one path
/// among several — it is the invalidation, and the cascade above is the
/// same statement under another name. See V74 for what it costs and what
/// it does not cover.
///
/// # The seed is one rule, and it is the measured one
///
/// `VDSL recipe` is the only Strategy with a measurement behind it:
/// eleven images out of two runs, where digesting everything and every
/// exclusion anyone proposed leave eleven distinct keys, and selecting
/// `["vdsl","script"]` recovers the two runs at five and six (the
/// measurement the [`series`](asterism_core::domain::series) module doc
/// opens with, frozen as that module's tests).
///
/// The character-card rule (`base64_json` over `["ccv3","data","name"]`)
/// is deliberately **not** seeded, and what it would have meant is why:
/// it groups cards by who is depicted, which is a claim about the
/// subject rather than about how the file was made — a second sentence
/// under the same column, and one nobody has measured on a real card
/// library here. Shipping it as a system row would ship that claim with
/// Album's name on it. Nothing stops a person registering it, and the
/// evidence the JPEG probe's module doc gathers (a vendor retreating
/// from filename-prefix stacking after it grouped unrelated downloads)
/// is the argument for letting the author own an unmeasured rule.
///
/// # Everything in the statement is frozen
///
/// The id, both timestamps and the mime are literals. "What did V73 do"
/// has to have one answer in a year, and every alternative fails that:
/// `unixepoch()` stamps a row with when somebody upgraded, a mime read
/// from a registry changes as probes land (V72's `WHERE` refuses the
/// same temptation for the same reason), and a generated id would leave
/// the seeded rule unaddressable by any later step. The id is a v7
/// carrying 2026-08-10 with its random bits zeroed — chosen by hand, and
/// visibly so — and the timestamps are that same moment in
/// milliseconds, which is when the rule was written rather than when a
/// database met it.
const V73_SERIES_STRATEGY: &str = r#"
CREATE TABLE series_strategy (
    id          BLOB PRIMARY KEY,
    name        TEXT NOT NULL,
    applies_to  TEXT NOT NULL,
    decode      TEXT NOT NULL CHECK (decode IN ('none', 'raw_json', 'base64_json')),
    include     TEXT NOT NULL,
    exclude     TEXT NOT NULL,
    system      INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

-- A later step that corrects this rule (see the UPDATE form above) must
-- also DELETE FROM material_series WHERE strategy_id = X'019fe8f8…0001':
-- every key derived under it was derived from the old include list, and
-- the derivation walk selects pairs that have *no* row, so a stale row is
-- one it can never re-offer. That is the whole of invalidation on this
-- axis (JobKind::SeriesDerive), and V74 is the index that makes it a seek.
INSERT INTO series_strategy
    (id, name, applies_to, decode, include, exclude, system, created_at, updated_at)
VALUES
    (X'019fe8f8140070008000000000000001', 'VDSL recipe', 'image/png', 'raw_json',
     '[["vdsl","script"]]', '[]', 1, 1786320000000, 1786320000000);

CREATE TABLE material_series (
    asset_id     BLOB NOT NULL,
    ord          INTEGER NOT NULL,
    strategy_id  BLOB NOT NULL REFERENCES series_strategy(id) ON DELETE CASCADE,
    key          TEXT,
    outcome      TEXT NOT NULL
                 CHECK (outcome IN ('derived', 'nothing_to_select', 'not_applicable')),
    derived_at   INTEGER NOT NULL,
    PRIMARY KEY (asset_id, ord, strategy_id),
    FOREIGN KEY (asset_id, ord) REFERENCES material(asset_id, ord) ON DELETE CASCADE,
    CHECK ((outcome = 'derived') = (key IS NOT NULL))
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_material_series_strategy_key
    ON material_series(strategy_id, key) WHERE key IS NOT NULL;
"#;

/// Version 73 → 74: `material_series` gets the plain `strategy_id`
/// index V73 priced and deferred.
///
/// V73's own doc states the gap: the primary key leads with `asset_id`
/// and the other index is partial on `key IS NOT NULL`, so a bare
/// `strategy_id = ?` implies neither and **every per-strategy statement
/// scans the table** — the library times the rules. It left the choice to
/// the slice that would have a real statement in front of it rather than
/// a guess.
///
/// There is one statement and it arrives twice. Deleting a rule cascades
/// (`ON DELETE CASCADE` on `material_series.strategy_id`), which SQLite
/// runs as `DELETE FROM material_series WHERE strategy_id = ?`; and
/// invalidating an edited rule is that same delete written out, because
/// the derivation walk's predicate is "a `(material, rule)` pair with no
/// row" — so removing a rule's rows *is* how an edit is expressed
/// ([`JobKind::SeriesDerive`](asterism_core::domain::job::JobKind::SeriesDerive)).
/// Both are the path the [`series`](asterism_core::domain::series)
/// module doc sells the axis on: change a rule and watch the groups
/// move. A scan of the whole table per keystroke-scale edit is what
/// "iterate on a Strategy" would actually have cost.
///
/// **Not partial, unlike its sibling.** `idx_material_series_strategy_key`
/// excludes `key IS NULL` because a lookup by key can never want the two
/// outcomes that are not keys; this one is about the *rule*, and a delete
/// has to take every row the rule wrote — the declining ones most of all,
/// since they are the majority (a rule states one `applies_to` and the
/// walk offers it every material).
///
/// **It does not serve the walk**, which is the other statement on this
/// table and wants the opposite thing: `NOT EXISTS (… asset_id = ? AND
/// ord = ? AND strategy_id = ?)` is a point lookup on the full primary
/// key, and `material_series` is `WITHOUT ROWID`, so the primary key *is*
/// the table and there is nothing left to add. Both plans are measured
/// (`the_series_walk_asks_material_series_by_primary_key`,
/// `the_per_strategy_delete_is_served_by_the_strategy_index`).
///
/// The cost is one more b-tree over a table that is the library times the
/// rules, paid on every derivation write. That is the trade V73 named and
/// did not take: a write-side index for a delete-side scan. It is taken
/// here because the delete is not an administrative rarity on this axis —
/// it is the edit.
const V74_MATERIAL_SERIES_STRATEGY: &str = r#"
CREATE INDEX idx_material_series_strategy ON material_series(strategy_id);
"#;

/// Version 74 → 75: `material.meta_raw` — the container's metadata
/// bytes, so the rule that expands them can be rewritten afterwards.
///
/// The vocabulary and the argument for it are
/// [`material_meta_raw`](asterism_core::domain::material_meta_raw); what
/// is here is what the column costs an existing library, which is the
/// part a migration decides.
///
/// # Why a column and not a key
///
/// `meta_kv` **is** the meta digest's input, so a key added to it moves
/// every `m1-` value ever written — including the eight frozen as
/// literals in this workspace, which are frozen because a moved digest
/// cannot be compared against the rows already sitting in a Dogfood
/// database. The bytes have to sit beside the rendering or they redefine
/// the axis they exist to make revisable. Nothing in this step touches
/// `meta_hash`, `meta_kv`, `content_region_hash` or `content_hash`.
///
/// # What it buys, measured rather than argued
///
/// `meta_kv` is lossy in two ways that were measured, not imagined: a
/// `tEXt` value is Latin-1 by the spec and goes through
/// `from_utf8_lossy`, so an accented byte becomes `\u{fffd}` and does
/// not come back; and `zTXt` / `iTXt` / `tIME` / `eXIf` are read by
/// neither axis, which the PNG probe's doc calls a stated gap. With the
/// bytes in the row both are recoverable **without opening the file**,
/// which is the difference between changing how metadata is expanded and
/// re-reading somebody's whole library to do it.
///
/// # No index
///
/// Nothing selects on this column — not the fingerprint walk (see
/// below), not duplicate grouping, not the series derivation. It is read
/// by primary key, one row at a time, by whatever decides to expand it
/// differently. An index would be write cost for no reader.
///
/// # The existing rows take an answer, and it is not NULL
///
/// Every row in the library at this moment gets
/// [`NOT_CAPTURED`](asterism_core::domain::material_meta_raw::NOT_CAPTURED),
/// the same shape V55 and V64 used and for a related reason, though the
/// consequence is different. Those two wrote
/// [`NOT_WALKED`](asterism_core::domain::content_region::NOT_WALKED)
/// because their column *is* read by the fingerprint walk, so NULL there
/// would have turned the pass for newly arrived material into a re-read
/// of the whole corpus. This column is read by no walk at all, and NULL
/// is a legitimate value in it — it is what a format that keeps no bytes
/// stores from here on — so a row left NULL by this step would be
/// saying something about the format rather than about the step.
///
/// **The `UPDATE` is unscoped, and the marker means one thing: the
/// build that read this row did not keep the bytes.** That sentence is
/// equally true of the PNG, the JPEG, the video and the text record, so
/// they all take it. It is deliberately *not* the finer statement it
/// might be read as: this step does not distinguish "not kept" from
/// "nothing here to keep", and that distinction only holds for rows
/// written after V75.
///
/// The alternative was to scope the write (`WHERE mime = 'image/png'`,
/// the one format that keeps bytes today) and it is worse. It would
/// leave every existing JPEG NULL, and NULL is the value that will mean
/// "this format carries none" — so the moment S6 teaches JPEG to keep
/// its `APP1` payload, those rows would be asserting something false
/// about themselves, in a column nothing re-derives. An unscoped write
/// says less and stays true; a scoped one freezes an answer this step
/// is not entitled to give.
///
/// **What that costs whoever writes the deferred pass**: selecting
/// `WHERE meta_raw = 'unsupported:not-captured'` alone hands back every
/// material in the library, including the ones that can never carry
/// bytes — and reading all of those off disk is the exact act the
/// deferral exists to refuse. That pass has to scope by format itself,
/// against the probe registry of the build it runs in. The marker says
/// which rows were never offered; it does not say which are worth
/// offering.
///
/// **And that pass does not exist.** Getting the bytes means reading
/// every original off disk, which is the scale of the decision recorded
/// at
/// [`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk):
/// a released application may not answer an update by reading somebody's
/// whole disk before it will open; that wants an announced upgrade,
/// timed by the person whose disk it is (decided 2026-08-05). So no verb
/// is written here and none is planned in this slice. If one is wanted
/// later, the shape is `remeasure_dims`'s — a scope and a write policy,
/// driven by an explicit request — and the scope is the part this step
/// leaves to it.
///
/// Nothing breaks while the rows stay as they are: they already hold a
/// correct `meta_kv`, and the bytes are only wanted on the day somebody
/// changes how metadata is expanded.
///
/// # It is deliberately not part of `needs_fingerprint`
///
/// Adding it would make every pre-existing row half-filled, and a
/// half-filled row is work — so the next launch would read every file in
/// the library, which is the act the paragraph above refuses. The
/// argument is written out at
/// [`needs_fingerprint`](asterism_core::domain::content_hash::needs_fingerprint),
/// beside the rule it is an exception to, and
/// `an_existing_row_is_not_work_because_it_has_no_raw` is where it has
/// teeth.
const V75_MATERIAL_META_RAW: &str = r#"
ALTER TABLE material ADD COLUMN meta_raw TEXT;

UPDATE material SET meta_raw = 'unsupported:not-captured';
"#;

/// Version 75 → 76: forget the **meta**-axis answer that says nothing
/// reads `image/jpeg`, because something does now.
///
/// [`V72_CLEAR_STALE_JPEG_CONTENT_MARKER`]'s twin, one axis over and one
/// slice later, and the whole of its argument carries: a marker is a
/// final answer to "has anybody looked", so the ordinary walk passes
/// these rows over for good, and the column ends up holding two meanings
/// at once — JPEGs imported after the probe carrying a `m1-` digest,
/// JPEGs imported before it carrying a statement that stopped being
/// true, told apart by nothing except when a file happened to arrive.
///
/// Not the version-bump case, for the reason V72 sets out at length:
/// this invalidates **negatives** on one format rather than every
/// positive on every format. Nothing that was ever read is read again;
/// what is re-read is a set bounded by how many JPEGs one library holds.
///
/// # What the probe now answers, and why V72 could not do this too
///
/// The JPEG probe declared `meta: false` when V72 was written, and the
/// declaration was the reason: with no reading behind the axis, a
/// cleared row would have come back holding exactly what it held before,
/// so V72 says in its own doc that it touches the content column only.
/// That changed with `Decode::Exif` — the series axis made a narrow
/// reading of EXIF expressible, so the probe claims the axis and the
/// rows have somewhere to arrive.
///
/// # It clears one column and answers three
///
/// `meta_hash` alone is set to NULL, and that is enough:
/// [`needs_fingerprint`](asterism_core::domain::content_hash::needs_fingerprint)
/// is an OR across the columns and the pass recomputes all of them from
/// one read. So a row cleared here also gets `meta_kv` and — the part
/// worth naming — `meta_raw`, replacing the
/// [`NOT_CAPTURED`](asterism_core::domain::material_meta_raw::NOT_CAPTURED)
/// V75 wrote across every row in the table. **That is the deferred pass
/// V75 said it was not writing**, arriving for one format as a side
/// effect of a column this step cleared for another reason, and it is
/// affordable for exactly the reason V75's own would not have been: the
/// set is one format's rows rather than the library.
///
/// # The literal is frozen, and pinned
///
/// `unsupported:image/jpeg` is hand-typed and the registry is not
/// consulted, on V72's terms — a migration is a statement about what
/// version 76 did, and it has to have the same answer in a year.
/// `the_jpeg_marker_this_migration_clears_is_the_one_the_domain_renders`
/// holds both steps' `WHERE` clauses against the domain's own rendering,
/// so a vocabulary that moved names these migrations.
///
/// And **that one marker**, by equality rather than by prefix, with the
/// same family of values left alone: `unsupported:too-large` (the size
/// gate is unchanged), `unsupported:empty-span` (a reading ran and found
/// no fields — which is what most JPEGs will store, since of 250 sampled
/// from a real download directory 246 carried no EXIF at all),
/// `unsupported:not-walked`, `unhashable:no-bytes`, and every `m1-`
/// digest.
const V76_CLEAR_STALE_JPEG_META_MARKER: &str = r#"
UPDATE material SET meta_hash = NULL
    WHERE meta_hash = 'unsupported:image/jpeg';
"#;

/// Version 76 → 77: `series_strategy.decode` admits `exif`.
///
/// [`Decode::Exif`](asterism_core::domain::series::Decode::Exif) shipped
/// with the JPEG meta axis above, and V73's `CHECK` names its decoders
/// verbatim, so without this step the first person to register an EXIF
/// rule meets a constraint failure on their own library. The pairing is
/// not left to a reader noticing: `Decode::ALL` is proved complete
/// against the enum's own source
/// (`the_decoder_list_names_every_variant_this_enum_has`) and compared
/// with the shipped schema's `CHECK`
/// (`the_series_tokens_this_schema_admits_are_the_ones_the_domain_writes`),
/// so adding the variant turns the first test red and adding it to the
/// list turns the second red until this step exists. Both were watched
/// to fail in that order.
///
/// # Rebuild, not `ALTER`
///
/// SQLite cannot widen a `CHECK`. Same shape as V64's and V68's
/// rebuilds — build beside, copy, drop, rename — and the copy is a
/// straight column-for-column `SELECT`, since no row changes meaning
/// here and only the constraint moves.
///
/// # …and it is a **parent** table, which is why it is a `Step::App`
///
/// V68 rebuilt a child table under an ordinary SQL batch. This one has
/// `material_series.strategy_id` pointing at it with `ON DELETE
/// CASCADE`, and with foreign keys enabled `DROP TABLE` performs an
/// implicit `DELETE FROM` that **fires those cascades**: every derived
/// key in the library would go, silently, inside a migration whose
/// subject is a constraint. The rows would come back — the derivation
/// walk selects pairs with no row, and re-deriving reads `meta_kv` and
/// no disk — so the damage is a wasted sweep rather than lost data, and
/// it is still not something a numbered step should do without saying
/// so. [`Step::App`] toggles `foreign_keys` off around
/// its transaction, which is what that mechanism is for, and
/// `v77_keeps_the_derived_rows_it_has_no_business_touching` is the
/// measurement.
///
/// The rename is what re-points the child: with foreign keys off SQLite
/// does not rewrite `REFERENCES` clauses, so `material_series` keeps
/// naming `series_strategy` and the renamed table arrives under exactly
/// that name.
fn v77_series_strategy_admits_the_exif_decoder(
    tx: &Transaction<'_>,
) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        r#"
CREATE TABLE series_strategy_new (
    id          BLOB PRIMARY KEY,
    name        TEXT NOT NULL,
    applies_to  TEXT NOT NULL,
    decode      TEXT NOT NULL
                CHECK (decode IN ('none', 'raw_json', 'base64_json', 'exif')),
    include     TEXT NOT NULL,
    exclude     TEXT NOT NULL,
    system      INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

INSERT INTO series_strategy_new
    (id, name, applies_to, decode, include, exclude, system, created_at, updated_at)
SELECT id, name, applies_to, decode, include, exclude, system, created_at, updated_at
  FROM series_strategy;

DROP TABLE series_strategy;
ALTER TABLE series_strategy_new RENAME TO series_strategy;
"#,
    )
}

/// Version 77 → 78: marks belong to a **layer**, and a material's own
/// chapter list has somewhere to live.
///
/// # The problem
///
/// A material can be read more than once by more than one hand. The
/// container declares chapters; a person disagrees and writes their
/// own; a job derives a third set. Until now all three would have been
/// the same rows on the same table, distinguishable only by whoever
/// wrote last — so "read the file again" either destroyed a person's
/// work or duplicated the file's, and nothing could answer "which of
/// these did I write?".
///
/// `material_layer` is that answer as data: `origin` says who produced
/// the band, `role` says what it holds. `chapter_mark` is the aggregate
/// a `'structure'` band holds, and `material_mark` gains the
/// `layer_id` that puts every existing mark into an `'annotation'` one.
///
/// # `material_layer`
///
/// - **The only foreign key is `asset(id)`.** The band is over one
///   original, named by `(asset_id, material_ord)` — the same pair
///   `material` is keyed by — and a composite `REFERENCES material
///   (asset_id, ord)` is deliberately *not* declared. Two reasons, and
///   the second is the load-bearing one. `material_mark` already
///   references the asset directly for the same shape (V60/V66), and
///   nothing guarantees that an asset carrying marks also carries a
///   `material` row at `ord = 0`: V37 wrote those rows only for
///   `role = 'item'` assets and only from the data it had, so a
///   composite FK would make the backfill below partial, and a partial
///   backfill against a `NOT NULL` column is a migration that fails on
///   somebody's library rather than on this developer's. The
///   consequence is stated rather than hidden: a layer can name a
///   `material_ord` no material row carries, and reading one back
///   yields a band over an original that is not there.
/// - **`is_default` is unique per `(asset_id, material_ord, role)`**,
///   enforced by a partial unique index. This is the rule the domain
///   deliberately does not hold: a value object cannot check a fact
///   about other rows, and a service that read them would be racing
///   between its read and its write. The index makes the second writer
///   lose loudly, which is what
///   `application::material_layer_service::default_annotation_layer`
///   answers by re-reading.
/// - **An `'imported'` band is unique per `(asset_id, material_ord,
///   role)`**, by a second partial unique index. `origin` is in the
///   key, not the predicate, so a person may keep as many bands of
///   their own over one material as they like — what the index forbids
///   is a *second copy of the file's own list*, which is not a band
///   somebody made but the same fact read twice. The way to get one is
///   two scans of one asset overlapping:
///   `material_layer_service::imported_structure_layer` looks for the
///   band and opens one if there is none, so two jobs that both look
///   before either writes both find nothing. Without the index the
///   loser's chapters land in a duplicate band that nothing ever reads
///   again and no verb removes; with it, the second insert fails and
///   that function's `Err` arm re-reads and finds the winner's band —
///   the same recovery `default_annotation_layer` already performs.
///   The alternative was a lock, which would be a bigger promise made
///   in a place that cannot keep it.
/// - **`CHECK (role <> 'annotation' OR is_default = 0 OR origin =
///   'user')`** mirrors `MaterialLayer::validate`. A new mark lands in
///   the default annotation band without naming it, so if that band
///   could be imported or machine-owned, every note would be written
///   into a band the immutability rule forbids writing to — refused by
///   the guard, or accepted and then deleted by the next re-probe.
///   Unlike the `body` rule on `material_mark`, this one *can* be
///   stated in SQL exactly (it compares closed slug sets, not Unicode),
///   so it is stated in both places.
/// - **`origin` and `role` are closed `CHECK (… IN (…))` sets**, so the
///   vocabulary is in the schema and not only in the Rust enums.
///   SQLite cannot alter a `CHECK` in place, so a fourth origin means
///   rebuilding this table. Pre-release that costs a batch, and the
///   same trade V66 recorded for `anchor_kind`.
/// - No name column. A band is described by `(origin, role)` and a
///   surface renders that pair; a stored caption would be a second
///   answer to one question, and the one that drifts.
///
/// # `chapter_mark`
///
/// - **`label` carries no non-empty `CHECK`, and unlike
///   `material_mark.body` it carries no domain rule either.** A mark's
///   body is the whole content of something a person chose to write, so
///   a blank one says nothing; a chapter's label is container metadata,
///   and files legitimately declare untitled sections (an MP4 `chpl`
///   entry with an empty string, a Matroska `ChapterAtom` with no
///   `ChapterDisplay`). Refusing those would make an import either drop
///   a section the file really has or invent a title for it.
/// - **`end_ms` is nullable**: MP4's `chpl` declares start times only,
///   and the end of a section is the start of the next one — a fact
///   about other rows, so not something a single chapter can be
///   required to carry. `start_ms` is `NOT NULL` here, unlike on
///   `material_mark`, because there is one coordinate space a chapter
///   can be in and it is the timeline.
/// - `ON DELETE CASCADE` from the layer: the sections in a band *are*
///   the band, so there is no state in which keeping one without the
///   other is the answer.
///
/// # The `material_mark` rebuild, and why the backfill cannot lose a row
///
/// `layer_id` is `NOT NULL` — a nullable column meaning "the default
/// band" would put one fact in two shapes, which is the drift this
/// whole step exists to remove. So every existing mark needs a band,
/// and the loop below mints one default annotation layer
/// (`origin = 'user'`, `is_default = 1`) per asset that has marks.
/// Assets with no marks get nothing: a library of a hundred thousand
/// images does not carry a hundred thousand empty bands to make one
/// code path shorter.
///
/// The copy joins with `LEFT JOIN` rather than `JOIN`, against a
/// `NOT NULL` column, **on purpose**. An inner join would answer a
/// missed asset by silently dropping its marks; the outer join answers
/// it by writing `NULL` into a column that refuses one, which aborts
/// the migration with the rows still in the old table. A backfill bug
/// that fails is recoverable; one that deletes is not.
///
/// `idx_material_mark_layer_start` has no `SELECT` reader yet — the
/// port lists marks by asset, not by band — and it is not there in
/// anticipation of one. Its reader is the `ON DELETE CASCADE` from
/// `material_layer`: SQLite implements a cascade as a search of the
/// child table for the parent key, so with no index on `layer_id` the
/// leading column, deleting one band scans every mark in the library.
/// A band is deleted by a person clicking once, and the cost of that
/// click would grow with the whole collection rather than with the band
/// being removed. The `start_ms` on the tail costs nothing here and is
/// the order a per-band listing would want on the day one is written.
///
/// # Why a `Step::App`
///
/// The backfill mints UUIDs, which SQL cannot do (compare
/// `v29_materialise_session_composites`), and the step drops a table
/// that has children. `Step::App` toggles `foreign_keys` off around its
/// transaction, so `DROP TABLE material_mark` does not fire the
/// author-side cascade, and the rename re-points nothing that needs
/// re-pointing — with foreign keys off SQLite leaves `REFERENCES`
/// clauses alone and the renamed table arrives under exactly the name
/// they already use.
///
/// # Measured
///
/// `EXPLAIN QUERY PLAN` over the DDL below (SQLite 3.43.2, 2026-08-13
/// — the same build V66's note was taken against), for the two
/// statements the new adapters issue:
///
/// ```text
/// SELECT … FROM material_layer WHERE asset_id = ?
///   ORDER BY material_ord, role, ord, id
/// -> SEARCH material_layer USING INDEX idx_material_layer_asset (asset_id=?)
///    USE TEMP B-TREE FOR RIGHT PART OF ORDER BY
///
/// SELECT … FROM chapter_mark WHERE layer_id = ? ORDER BY ord, start_ms, id
/// -> SEARCH chapter_mark USING INDEX idx_chapter_mark_layer_ord (layer_id=?)
///    USE TEMP B-TREE FOR RIGHT PART OF ORDER BY
/// ```
///
/// Both indexes serve the filter and the leading sort terms; the `, id`
/// tie-break is a real sorter in each case, exactly as V66 recorded for
/// `material_mark`. Worth knowing before anyone reads "index-backed" as
/// "already ordered" — and the tie-break is not decoration: `id` is the
/// PRIMARY KEY, so ordering on it makes the result total instead of
/// leaving equal sort keys to the scan.
///
/// # Carried forward from V66
///
/// V66 left a note for the second anchor kind: a `'spatial'` row would
/// have `start_ms IS NULL`, SQLite sorts NULL first, so such rows would
/// head every `ORDER BY start_ms` listing. This step rebuilds the table
/// and **does not settle it** — `anchor_kind` is still `'temporal'`
/// alone, so there is still no row for the question to apply to, and
/// answering it now would mean choosing a listing order for a
/// coordinate space nobody has written. The note stands, unchanged, and
/// belongs to whichever migration adds the second kind. What this step
/// does change is where the question will be asked: a listing is now
/// per layer as well as per asset, so the answer has a band to be
/// scoped to.
const V78_LAYER_TABLES: &str = r#"
CREATE TABLE material_layer (
    id           BLOB PRIMARY KEY,
    asset_id     BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    material_ord INTEGER NOT NULL DEFAULT 0,
    origin       TEXT NOT NULL CHECK (origin IN ('imported', 'user', 'machine')),
    role         TEXT NOT NULL CHECK (role IN ('structure', 'annotation')),
    is_default   INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    ord          INTEGER NOT NULL DEFAULT 0,
    CHECK (material_ord >= 0),
    CHECK (ord >= 0),
    CHECK (role <> 'annotation' OR is_default = 0 OR origin = 'user')
) STRICT;

CREATE UNIQUE INDEX idx_material_layer_one_default
    ON material_layer(asset_id, material_ord, role)
 WHERE is_default = 1;

CREATE UNIQUE INDEX idx_material_layer_one_imported
    ON material_layer(asset_id, material_ord, role)
 WHERE origin = 'imported';

CREATE INDEX idx_material_layer_asset
    ON material_layer(asset_id, material_ord, role, ord);

CREATE TABLE chapter_mark (
    id       BLOB PRIMARY KEY,
    layer_id BLOB NOT NULL REFERENCES material_layer(id) ON DELETE CASCADE,
    start_ms INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms   INTEGER,
    label    TEXT NOT NULL,
    ord      INTEGER NOT NULL DEFAULT 0 CHECK (ord >= 0),
    CHECK (end_ms IS NULL OR end_ms > start_ms)
) STRICT;

CREATE INDEX idx_chapter_mark_layer_ord
    ON chapter_mark(layer_id, ord, start_ms);
"#;

/// The `material_mark` rebuild half of V78 (see the doc on
/// [`V78_LAYER_TABLES`]). Runs after the backfill loop has minted the
/// bands this copy joins against.
const V78_MARK_REBUILD: &str = r#"
CREATE TABLE material_mark_new (
    id                BLOB PRIMARY KEY,
    asset_id          BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    layer_id          BLOB NOT NULL REFERENCES material_layer(id) ON DELETE CASCADE,
    anchor_kind       TEXT NOT NULL CHECK (anchor_kind IN ('temporal')),
    start_ms          INTEGER,
    end_ms            INTEGER,
    body              TEXT NOT NULL,
    author_kind       TEXT NOT NULL CHECK (author_kind IN ('user', 'persona')),
    author_persona_id BLOB REFERENCES persona(id) ON DELETE CASCADE,
    created_at        INTEGER NOT NULL,
    edited_at         INTEGER,
    CHECK (
        (author_kind = 'user'    AND author_persona_id IS NULL)
     OR (author_kind = 'persona' AND author_persona_id IS NOT NULL)
    ),
    CHECK (anchor_kind <> 'temporal' OR start_ms IS NOT NULL),
    CHECK (start_ms IS NULL OR start_ms >= 0),
    CHECK (end_ms IS NULL OR end_ms > start_ms)
) STRICT;

INSERT INTO material_mark_new
    (id, asset_id, layer_id, anchor_kind, start_ms, end_ms, body,
     author_kind, author_persona_id, created_at, edited_at)
SELECT m.id, m.asset_id, l.id, m.anchor_kind, m.start_ms, m.end_ms, m.body,
       m.author_kind, m.author_persona_id, m.created_at, m.edited_at
  FROM material_mark m
  LEFT JOIN material_layer l
         ON l.asset_id     = m.asset_id
        AND l.material_ord = 0
        AND l.role         = 'annotation'
        AND l.is_default   = 1;

DROP TABLE material_mark;
ALTER TABLE material_mark_new RENAME TO material_mark;

CREATE INDEX idx_material_mark_asset_start
    ON material_mark(asset_id, start_ms);
CREATE INDEX idx_material_mark_layer_start
    ON material_mark(layer_id, start_ms);
"#;

/// Applies V78: the two new tables, one default annotation band per
/// asset that already carries marks, and the `material_mark` rebuild
/// that fastens the existing rows to it.
fn v78_material_layers(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(V78_LAYER_TABLES)?;

    // One band per asset that has marks — collected first rather than
    // inserted from a cursor, because the `INSERT` writes to a table
    // the `SELECT` would otherwise be reading through in the same
    // statement.
    let marked: Vec<Uuid> = {
        let mut stmt = tx.prepare("SELECT DISTINCT asset_id FROM material_mark")?;
        stmt.query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    };
    for asset_id in marked {
        tx.execute(
            "INSERT INTO material_layer
                 (id, asset_id, material_ord, origin, role, is_default, ord)
             VALUES (?1, ?2, 0, 'user', 'annotation', 1, 0)",
            params![Uuid::now_v7(), asset_id],
        )?;
    }

    tx.execute_batch(V78_MARK_REBUILD)?;

    // Guard: unlike the rebuilds above, this step *synthesises* rows
    // that carry a foreign key, so it can manufacture an orphan rather
    // than merely carry one over. The loop takes its asset ids from
    // `material_mark` and writes them into `material_layer.asset_id`,
    // which references `asset(id)` — and with foreign keys off for the
    // whole App step, an id no asset carries is inserted without
    // complaint. A mark left behind by a cascade that did not finish is
    // enough to reach it: the loop would mint that mark a band over an
    // asset that is not there, and commit it.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v78: foreign_key_check reported violations after the rebuild".into()),
        ));
    }
    Ok(())
}

/// Version 78 → 79: the pursuit — the minted unit of work (#29,
/// design on #21).
///
/// Three tables and one column. `pursuit` is thin and immutable (no
/// status column — standing derives from `pursuit_event` on read, the
/// `duplicate_conflict` reading); `pursuit_event` holds the one-way
/// lifecycle facts; `pursuit_restamp` records moves of a stamped event
/// between pursuits — the repair verb for mis-filed correlation.
/// `dispatch_job.pursuit_id` is the stamp itself.
///
/// Schema choices worth writing down:
///
/// - **RESTRICT everywhere**, including `persona_id` — unlike
///   `dispatch_job.persona_id`'s CASCADE. The persona purge is
///   hand-rolled precisely because SQLite fires RESTRICT mid-cascade;
///   these tables extend that sequence (`repo/persona.rs`), and a new
///   delete path that forgets them should fail loudly rather than
///   cascade half a record away.
/// - **`kind` carries its CHECK** (a fresh table can, where V47/V50's
///   ALTER could not); the attribution columns stay nullable TEXT with
///   the same NULL-means-unrecorded reading, guarded on write by the
///   repository row builders.
/// - **`subject_kind` admits `'judgment'` now**: the worth-gate table
///   arrives later, and widening a CHECK on a STRICT table costs a
///   rebuild — the two-value set is the design's closed vocabulary,
///   not speculation.
/// - The `(created_at, id)` index on `pursuit_event` matches the
///   standing derivation's sort key; the `id` tie-break is a real
///   sorter (the V66 / V78 lesson).
const V79_PURSUIT_TABLES: &str = r#"
CREATE TABLE pursuit (
    id             BLOB PRIMARY KEY,
    persona_id     BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    parent_id      BLOB REFERENCES pursuit(id) ON DELETE RESTRICT,
    title          TEXT,
    note           TEXT,
    author_kind    TEXT,
    author_subject TEXT,
    operator_ai    TEXT,
    attributed_via TEXT,
    created_at     INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_pursuit_persona_created
    ON pursuit(persona_id, created_at DESC);
CREATE INDEX idx_pursuit_parent
    ON pursuit(parent_id);

CREATE TABLE pursuit_event (
    id             BLOB PRIMARY KEY,
    pursuit_id     BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    persona_id     BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    kind           TEXT NOT NULL
        CHECK (kind IN ('closed_satisfied', 'closed_abandoned', 'reopened')),
    snapshot_id    BLOB REFERENCES snapshot(id) ON DELETE RESTRICT,
    note           TEXT,
    author_kind    TEXT,
    author_subject TEXT,
    operator_ai    TEXT,
    attributed_via TEXT,
    created_at     INTEGER NOT NULL,
    CHECK (snapshot_id IS NULL OR kind = 'closed_satisfied')
) STRICT;

CREATE INDEX idx_pursuit_event_pursuit_created
    ON pursuit_event(pursuit_id, created_at, id);
CREATE INDEX idx_pursuit_event_persona
    ON pursuit_event(persona_id);

CREATE TABLE pursuit_restamp (
    id              BLOB PRIMARY KEY,
    subject_kind    TEXT NOT NULL
        CHECK (subject_kind IN ('dispatch', 'judgment')),
    subject_id      BLOB NOT NULL,
    from_pursuit_id BLOB REFERENCES pursuit(id) ON DELETE RESTRICT,
    to_pursuit_id   BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    author_kind     TEXT,
    author_subject  TEXT,
    operator_ai     TEXT,
    attributed_via  TEXT,
    created_at      INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_pursuit_restamp_subject
    ON pursuit_restamp(subject_kind, subject_id);
CREATE INDEX idx_pursuit_restamp_to
    ON pursuit_restamp(to_pursuit_id);
CREATE INDEX idx_pursuit_restamp_from
    ON pursuit_restamp(from_pursuit_id);

ALTER TABLE dispatch_job ADD COLUMN pursuit_id BLOB
    REFERENCES pursuit(id) ON DELETE RESTRICT;

CREATE INDEX idx_dispatch_pursuit
    ON dispatch_job(pursuit_id);
"#;

/// Applies V79: the pursuit tables, the dispatch stamp column, and the
/// backfill — one legacy dispatch = one single-round pursuit.
///
/// An App step because the backfill mints UUIDs, which no SQL batch can
/// (the [`v19_selection_model`] / [`v49_instance_identity`] reason).
/// The backfill copies `persona_id` and `created_at` from the dispatch
/// row and leaves everything else absent: **attribution stays NULL**
/// (nobody opened these pursuits — the migration did, and absent
/// bookkeeping stays absent rather than being forged, the V48/V50
/// rule), and no grouping heuristic runs — grouping consecutive
/// dispatches would be exactly the inferred correlation the design
/// forbids. Users continue or restamp afterwards; that is a statement,
/// not a guess.
fn v79_pursuit(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(V79_PURSUIT_TABLES)?;

    // Collected first rather than driven from a cursor: the UPDATE
    // writes to the table the SELECT would be reading through (the V78
    // shape).
    let legacy: Vec<(Uuid, Uuid, i64)> = {
        let mut stmt = tx.prepare("SELECT id, persona_id, created_at FROM dispatch_job")?;
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    {
        let mut mint =
            tx.prepare("INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, ?3)")?;
        let mut stamp = tx.prepare("UPDATE dispatch_job SET pursuit_id = ?1 WHERE id = ?2")?;
        for (dispatch_id, persona_id, created_at) in legacy {
            let pursuit_id = Uuid::now_v7();
            mint.execute(params![pursuit_id, persona_id, created_at])?;
            stamp.execute(params![pursuit_id, dispatch_id])?;
        }
    }

    // Guard: the loop synthesises rows that carry foreign keys with
    // foreign keys off for the whole App step (the V78 lesson) — a
    // dispatch row whose persona is gone would mint a pursuit over a
    // persona that is not there, and commit it.
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v79: foreign_key_check reported violations after the backfill".into()),
        ));
    }
    Ok(())
}

/// Version 79 → 80: the reverse-lookup lane for pursuit membership
/// (#29) — two virtual generated columns over `_trace`, each indexed.
///
/// A pursuit's **returns** are assets whose ingest note resolved to
/// one of its rounds (`_trace.dispatch_id`, dispatch join first) or,
/// failing that, to the pursuit directly (`_trace.pursuit_id`). Both
/// keys live inside `asset.extra` JSON, so the naive read is a full
/// scan with a JSON parse per row — a per-view cost that grows with
/// the library, at a documented 100k+ asset scale. The columns
/// surface the two keys **only when their claim resolved** (the
/// authority rule is baked into the column, not repeated per query),
/// and the partial indexes hold just the claim-carrying rows, so a
/// membership probe is an index seek instead of a scan.
///
/// - VIRTUAL rather than STORED because `ALTER TABLE ADD COLUMN` can
///   only add virtual generated columns; the index materialises the
///   values anyway, which is where reads look.
/// - `json_valid` guards both expressions: the app validates
///   `extra_json` on write, but the read side already assumes a
///   corrupt bag is possible, and on a local-first profile a single
///   bad row must degrade to "no claim surfaced", not to "the
///   migration fails and the profile no longer opens" —
///   `json_extract` over invalid JSON is an error, and inside an
///   ALTER it is the whole batch's error.
/// - The columns are derived state over `_trace` — the note stays the
///   fact, the columns are how it is found. Writers keep writing the
///   note; nothing writes the columns.
/// - `Step::Sql`: DDL only, nothing minted.
const V80_TRACE_LOOKUP: &str = r#"
ALTER TABLE asset ADD COLUMN trace_dispatch_id TEXT
    GENERATED ALWAYS AS (
        CASE WHEN json_valid(extra)
              AND json_extract(extra, '$._trace.resolved') = 1
             THEN json_extract(extra, '$._trace.dispatch_id')
        END
    ) VIRTUAL;

ALTER TABLE asset ADD COLUMN trace_pursuit_id TEXT
    GENERATED ALWAYS AS (
        CASE WHEN json_valid(extra)
              AND json_extract(extra, '$._trace.pursuit_resolved') = 1
             THEN json_extract(extra, '$._trace.pursuit_id')
        END
    ) VIRTUAL;

CREATE INDEX idx_asset_trace_dispatch
    ON asset(trace_dispatch_id)
 WHERE trace_dispatch_id IS NOT NULL;

CREATE INDEX idx_asset_trace_pursuit
    ON asset(trace_pursuit_id)
 WHERE trace_pursuit_id IS NOT NULL;
"#;

/// Version 80 → 81: the words a container wrote into an artefact, and
/// the reading that composed a search body.
///
/// Two columns, one feature (the derived search document):
///
/// - `material.meta_text` — the canonical rendering of the artefact's
///   embedded text (`domain::embedded_text`), written by the same pass
///   that fingerprints the bytes. `NULL` is "nobody has looked", which
///   is what every existing row truthfully is — the recovery backfill
///   (`JobKind::MaterialText`) walks exactly the `IS NULL` set, so no
///   default is written here. `'{}'` ("read, and no words") is an
///   answer only a walk may give.
/// - `asset_body.derived_version` — which
///   [`derived_text::COMPOSITION_VERSION`](asterism_core::domain::derived_text::COMPOSITION_VERSION)
///   composed the cached body. `NULL` marks every pre-derivation body
///   as stale, which is deliberate: those bodies are the file's bytes
///   alone, and the backfill (`scan_stale_body`) exists to re-compose
///   them with the sections that did not exist yet. Rows gain the
///   stamp as they are re-composed; nothing is backfilled here.
const V81_DERIVED_TEXT: &str = r#"
ALTER TABLE material ADD COLUMN meta_text TEXT;

ALTER TABLE asset_body ADD COLUMN derived_version INTEGER;
"#;

/// Version 81 → 82: the pursuit's membership ledger and the record of
/// the cull (#22, model on #63) — plus the restamp vocabulary catching
/// up with the model's name for the act.
///
/// `pursuit_tx` is what makes "out of what" answerable: every asset
/// that enters a pursuit, and every mid-work removal or reversal, is
/// one append-only row. The candidate set a cull records is derived
/// from this ledger and frozen at close — never handed in by the
/// caller, which is what the withdrawn `judgment` spec got wrong.
///
/// Schema choices worth writing down:
///
/// - **`asset_id` carries no FK**, on both `pursuit_tx` and
///   `cull_member` — the `dispatch_job.output_asset_ids` stance:
///   history outlives the asset, and a ledger that blocks asset
///   deletion (or loses rows to a cascade) is not a ledger. The
///   candidate *set* survives independently via the RESTRICT edge to
///   `snapshot`.
/// - **`origin` rides `'in'` and nothing else** (two-way CHECK): where
///   a member came from — generated by a round, imported from
///   outside, or brought in from the existing library — is a fact
///   about the entry gesture, meaningless on the other kinds.
/// - **`'update'` is admitted by the CHECK but no verb writes it
///   yet**: it is the model's vocabulary for the external-edit
///   round-trip (#63), and widening a CHECK on a STRICT table costs a
///   rebuild — the V79 `'judgment'` reasoning, which V82 itself now
///   pays for.
/// - **One cull per close event** (`idx_cull_event` UNIQUE): the cull
///   is the record of *that* close's narrowing. A repeat close is a
///   new event and may carry a new cull; nothing is edited.
/// - **`pursuit_restamp` is rebuilt** to say `'cull'` where it
///   reserved `'judgment'`: the value was pre-paid and never written
///   (the domain parser has refused it since V79), so the rebuild
///   copies rows without translating any.
///
/// An App step for one reason: the backfill mints tx ids. Every
/// dispatch output already recorded in `output_asset_ids` becomes an
/// `'in' / 'generated'` row under the dispatch's pursuit — a
/// mechanical transcription of two recorded facts (the stamp, the
/// output list), not a grouping guess; per pursuit, the first dispatch
/// to produce an asset wins and later repeats are skipped, because
/// membership is per (pursuit, asset) while the same asset may appear
/// in several rounds' outputs. Attribution stays NULL (the V48/V50
/// rule: nobody recorded these entries, the migration did) and
/// `created_at` copies the dispatch row's.
fn v82_cull_record(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(V82_CULL_TABLES)?;

    // Collected first rather than driven from a cursor (the V78 / V79
    // shape): the INSERT below writes while this SELECT would read.
    let outputs: Vec<(Uuid, Uuid, String, i64)> = {
        // Ordered so "the first dispatch to produce the asset wins"
        // is a guarantee of the query, not of the scan order.
        let mut stmt = tx.prepare(
            "SELECT pursuit_id, persona_id, output_asset_ids, created_at \
             FROM dispatch_job \
             WHERE pursuit_id IS NOT NULL AND output_asset_ids != '[]' \
             ORDER BY created_at, id",
        )?;
        stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<_, _>>()?
    };
    {
        let mut seen: std::collections::HashSet<(Uuid, Uuid)> = std::collections::HashSet::new();
        let mut insert = tx.prepare(
            "INSERT INTO pursuit_tx \
             (id, pursuit_id, persona_id, kind, asset_id, origin, created_at) \
             VALUES (?1, ?2, ?3, 'in', ?4, 'generated', ?5)",
        )?;
        for (pursuit_id, persona_id, output_json, created_at) in outputs {
            // A row that fails to parse is a corrupt column, and the
            // migration failing loudly is the right outcome — this
            // column is written atomically with the done transition
            // and has no legal non-array state.
            let asset_ids: Vec<Uuid> = serde_json::from_str(&output_json).map_err(|e| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                    Some(format!("v82: output_asset_ids is not a UUID array: {e}")),
                )
            })?;
            for asset_id in asset_ids {
                if seen.insert((pursuit_id, asset_id)) {
                    insert.execute(params![
                        Uuid::now_v7(),
                        pursuit_id,
                        persona_id,
                        asset_id,
                        created_at
                    ])?;
                }
            }
        }
    }

    // Guard: rows were synthesised with foreign keys off for the whole
    // App step (the V78 lesson).
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v82: foreign_key_check reported violations after the backfill".into()),
        ));
    }
    Ok(())
}

/// DDL half of V82 — see [`v82_cull_record`] for the choices.
const V82_CULL_TABLES: &str = r#"
CREATE TABLE pursuit_tx (
    id             BLOB PRIMARY KEY,
    pursuit_id     BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    persona_id     BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    kind           TEXT NOT NULL
        CHECK (kind IN ('in', 'update', 'remove', 'unremove')),
    asset_id       BLOB NOT NULL,
    origin         TEXT
        CHECK (origin IN ('generated', 'imported', 'existing')),
    note           TEXT,
    author_kind    TEXT,
    author_subject TEXT,
    operator_ai    TEXT,
    attributed_via TEXT,
    created_at     INTEGER NOT NULL,
    CHECK ((kind = 'in') = (origin IS NOT NULL))
) STRICT;

CREATE INDEX idx_pursuit_tx_pursuit_created
    ON pursuit_tx(pursuit_id, created_at, id);
CREATE INDEX idx_pursuit_tx_asset
    ON pursuit_tx(asset_id);
CREATE INDEX idx_pursuit_tx_persona
    ON pursuit_tx(persona_id);

CREATE TABLE cull (
    id                    BLOB PRIMARY KEY,
    pursuit_id            BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    persona_id            BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    pursuit_event_id      BLOB NOT NULL REFERENCES pursuit_event(id) ON DELETE RESTRICT,
    candidate_snapshot_id BLOB NOT NULL REFERENCES snapshot(id) ON DELETE RESTRICT,
    note                  TEXT,
    author_kind           TEXT,
    author_subject        TEXT,
    operator_ai           TEXT,
    attributed_via        TEXT,
    created_at            INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_cull_pursuit ON cull(pursuit_id);
CREATE INDEX idx_cull_persona ON cull(persona_id);
CREATE UNIQUE INDEX idx_cull_event ON cull(pursuit_event_id);

CREATE TABLE cull_member (
    cull_id  BLOB NOT NULL REFERENCES cull(id) ON DELETE RESTRICT,
    asset_id BLOB NOT NULL,
    verdict  TEXT NOT NULL CHECK (verdict IN ('keep', 'reject')),
    note     TEXT,
    PRIMARY KEY (cull_id, asset_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_cull_member_asset ON cull_member(asset_id);

CREATE TABLE pursuit_restamp_v82 (
    id              BLOB PRIMARY KEY,
    subject_kind    TEXT NOT NULL
        CHECK (subject_kind IN ('dispatch', 'cull')),
    subject_id      BLOB NOT NULL,
    from_pursuit_id BLOB REFERENCES pursuit(id) ON DELETE RESTRICT,
    to_pursuit_id   BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    author_kind     TEXT,
    author_subject  TEXT,
    operator_ai     TEXT,
    attributed_via  TEXT,
    created_at      INTEGER NOT NULL
) STRICT;

INSERT INTO pursuit_restamp_v82 SELECT * FROM pursuit_restamp;
DROP TABLE pursuit_restamp;
ALTER TABLE pursuit_restamp_v82 RENAME TO pursuit_restamp;

CREATE INDEX idx_pursuit_restamp_subject
    ON pursuit_restamp(subject_kind, subject_id);
CREATE INDEX idx_pursuit_restamp_to
    ON pursuit_restamp(to_pursuit_id);
CREATE INDEX idx_pursuit_restamp_from
    ON pursuit_restamp(from_pursuit_id);
"#;

/// V84 — the project and its lines (#63 decisions 1–3): the repo of
/// the forge's git analogy, the named line it lands on, the entry
/// identity above raw asset ids, the verb sequence that moves it, and
/// the merge record that makes approval and landing one act.
///
/// Shapes worth naming:
///
/// - **`line` is the branch, and "mainline" is a description**: v1
///   mints exactly one row per project, named `main`
///   (application-enforced), and the UNIQUE on `(project_id, name)`
///   is honest because lines have no death — unlike living entry
///   names, whose uniqueness is an application rule precisely so dead
///   names free up without a migration.
/// - **`line_event.asset_id` carries no FK** — the
///   `cull_member.asset_id` stance: a line's history outlives the
///   asset rows it names.
/// - **Two-way CHECKs pair verb and payload** (the `pursuit_tx`
///   `(kind, origin)` precedent): an `add` without a name or a
///   `delete` with an asset is a corrupt row the schema refuses.
/// - **`line_merge`, not `merge`**: the Group/`bucket` precedent —
///   `MERGE` is a statement in enough dialects (SQL Server, Oracle,
///   PostgreSQL 15+) that quoting forever is the fragile choice. The
///   domain type stays `Merge`.
/// - **No attribution triple on `line_merge`** — who approved is
///   who closed, and the close event carries the triple; a copy here
///   would mint a second author for one act. `project` carries it
///   (opening one is a statement, the fourth wave of the
///   channel-carrier list).
/// - **Every line starts empty** — no backfill from historical kept
///   snapshots, which would invent approval events nobody performed.
const V84_FORGE_PROJECT_LINE: &str = r#"
CREATE TABLE project (
    id             BLOB PRIMARY KEY,
    persona_id     BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    name           TEXT NOT NULL,
    note           TEXT,
    author_kind    TEXT,
    author_subject TEXT,
    operator_ai    TEXT,
    attributed_via TEXT,
    created_at     INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_project_persona ON project(persona_id);

CREATE TABLE line (
    id         BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES project(id) ON DELETE RESTRICT,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_line_project_name ON line(project_id, name);

CREATE TABLE line_entry (
    id         BLOB PRIMARY KEY,
    line_id    BLOB NOT NULL REFERENCES line(id) ON DELETE RESTRICT,
    persona_id BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_line_entry_line ON line_entry(line_id);
CREATE INDEX idx_line_entry_persona ON line_entry(persona_id);

CREATE TABLE line_merge (
    id               BLOB PRIMARY KEY,
    pursuit_event_id BLOB NOT NULL REFERENCES pursuit_event(id) ON DELETE RESTRICT,
    persona_id       BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    created_at       INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_line_merge_event ON line_merge(pursuit_event_id);
CREATE INDEX idx_line_merge_persona ON line_merge(persona_id);

CREATE TABLE line_event (
    id         BLOB PRIMARY KEY,
    entry_id   BLOB NOT NULL REFERENCES line_entry(id) ON DELETE RESTRICT,
    persona_id BLOB NOT NULL REFERENCES persona(id) ON DELETE RESTRICT,
    verb       TEXT NOT NULL
        CHECK (verb IN ('add', 'replace', 'delete', 'rename')),
    asset_id   BLOB,
    name       TEXT,
    merge_id   BLOB NOT NULL REFERENCES line_merge(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    CHECK ((verb IN ('add', 'replace')) = (asset_id IS NOT NULL)),
    CHECK ((verb IN ('add', 'rename')) = (name IS NOT NULL))
) STRICT;

CREATE INDEX idx_line_event_entry_created
    ON line_event(entry_id, created_at, id);
CREATE INDEX idx_line_event_merge ON line_event(merge_id);
CREATE INDEX idx_line_event_asset ON line_event(asset_id);
CREATE INDEX idx_line_event_persona ON line_event(persona_id);
"#;

/// V85 — filing, and the columns a targeted IN needs (#63 decisions
/// 4–5): the pursuit learns which project it files under, and the
/// ledger learns to say *which entry* an `in` is aimed at, *which
/// version it saw there*, whether it reached outside its scope, and
/// which member an `update` revises.
///
/// Shapes worth naming:
///
/// - **`pursuit.project_id` is nullable, and that is not an
///   invitation.** Filing is what mints a pursuit at all: exploration
///   runs below the forge and mints nothing, so a row without a project
///   is residue from the retired always-mint rule rather than a mode.
///   Nullable because those rows exist and are not rewritten — the
///   empty-line stance, which refuses to invent a record after the
///   fact.
/// - **`pursuit_tx` is altered, not rebuilt.** The pairing rules below
///   each relate one column to another, which reads like the
///   table-level CHECK `ALTER TABLE ADD COLUMN` cannot carry (V47) —
///   but a *column-level* CHECK both survives ADD COLUMN (measured at
///   V51) and may name other columns, and it fires on INSERT as well as
///   UPDATE. Measured before choosing, on SQLite 3.46.1: a column added
///   with `CHECK (target IS NULL OR kind = 'in')` refuses both a fresh
///   insert and an update that breaks it. So the rules below are
///   column-level, and this history table is never copied row by row —
///   which is the point. A rebuild would show the final shape in one
///   `CREATE TABLE`, and cost a hand-written copy of an append-only
///   ledger to get it; the V82 `pursuit_restamp` rebuild paid that
///   because it was widening a CHECK on an existing column, which ADD
///   COLUMN genuinely cannot do.
/// - **Targeting is a property of an `in` that names something already
///   on a line**, so `target_entry_id` is admitted only on
///   `(kind, origin) = ('in', 'existing')`. The target CHECK carries
///   only half of that on its own: for `kind = 'in'` with a NULL origin
///   it evaluates to NULL, and SQLite fails a CHECK only on false, so
///   what closes it is V82's `(kind = 'in') = (origin IS NOT NULL)`
///   making that combination unreachable in the first place. The two
///   are load-bearing together.
/// - **The second targeting column is gone.** This step also added one
///   holding the version of the entry a caller saw when aiming, with a
///   CHECK admitting it only alongside a target and an index of its
///   own. No writer ever filled it and no reader ever wanted it, and
///   [`V91_DROP_THE_BASE_EVENT_PIN`] takes all three. The DDL below
///   still adds them, because a database walking this chain from
///   scratch has to arrive at the shape V91 expects to alter.
/// - **`supersedes_asset_id` is one-way on purpose.** It is admitted
///   only on `update`, but an `update` is not yet required to carry
///   one: the verb is still reserved (`tx.rs`), nothing writes it, and
///   the other direction is P3's to close once the verb states what it
///   means. V84 could pair its verbs both ways because all four were
///   specified; this one is not, and a CHECK that guesses is worse than
///   a CHECK that waits. It names an asset and still carries no FK, for
///   the reason `pursuit_tx.asset_id` does not: the ledger is history,
///   and history outlives the asset rows it names.
/// - **`out_of_scope` carries no FK and no default beyond 0** — it is a
///   statement the caller made at IN time about crossing into another
///   project's living set, not a fact that can be re-derived later,
///   because the set moves.
const V85_FILING_AND_TARGETED_IN: &str = r#"
ALTER TABLE pursuit ADD COLUMN project_id BLOB REFERENCES project(id) ON DELETE RESTRICT;

CREATE INDEX idx_pursuit_project ON pursuit(project_id);

ALTER TABLE pursuit_tx ADD COLUMN target_entry_id BLOB
    REFERENCES line_entry(id) ON DELETE RESTRICT
    CHECK (target_entry_id IS NULL OR (kind = 'in' AND origin = 'existing'));

ALTER TABLE pursuit_tx ADD COLUMN base_event_id BLOB
    REFERENCES line_event(id) ON DELETE RESTRICT
    CHECK (base_event_id IS NULL OR target_entry_id IS NOT NULL);

ALTER TABLE pursuit_tx ADD COLUMN out_of_scope INTEGER NOT NULL DEFAULT 0
    CHECK (out_of_scope IN (0, 1) AND (out_of_scope = 0 OR kind = 'in'));

ALTER TABLE pursuit_tx ADD COLUMN supersedes_asset_id BLOB
    CHECK (supersedes_asset_id IS NULL OR kind = 'update');

CREATE INDEX idx_pursuit_tx_target ON pursuit_tx(target_entry_id);
CREATE INDEX idx_pursuit_tx_base_event ON pursuit_tx(base_event_id);
"#;

/// Version 82 → 83: the record of a dispatch's latest attempt — what the
/// exporter sent and what came back, on the calls that produced no
/// handle as well as on the ones that did.
///
/// `handle_payload` answers this today only where the backend accepted
/// the job: the exporter builds its record on the way to a handle, and a
/// refused submit returns an error instead, so the row keeps the one
/// sentence in `state_message` and nothing about the call. These two
/// columns are where that record goes, and they sit beside the handle
/// rather than inside it because the handle means "a job exists over
/// there" — a refused submit has no such job, and writing one in would
/// hand the poll loop a reference to nothing.
///
/// Same ownership as `handle_payload`: `attempt_payload` is a JSON TEXT
/// blob the exporter names the shape of, opaque to the DB layer, and
/// `attempt_kind` says whose shape it is.
///
/// Rows written before this column keep it NULL, which reads as "nothing
/// was recorded". There is no backfill and could not be one: the calls
/// these rows describe are over, and their requests and responses were
/// never written down anywhere to recover them from.
const V83_DISPATCH_ATTEMPT: &str = r#"
ALTER TABLE dispatch_job ADD COLUMN attempt_kind TEXT;
ALTER TABLE dispatch_job ADD COLUMN attempt_payload TEXT;
"#;

/// Version 85 → 86: a comment may be pinned to the selection gesture
/// that occasioned it (#65).
///
/// One nullable `TEXT` slug on `asset_comment` — `'trash'`,
/// `'trash_group'` or `'restore'`, the mutating verbs a solo user
/// states a reason at. Every existing row keeps `NULL`, which reads as
/// "an ordinary thread post": before this column no comment was ever
/// posted *by* a gesture, so `NULL` is the truth about all of them and
/// there is nothing to backfill.
///
/// `ALTER TABLE ADD COLUMN` carries the column-level `CHECK` fine
/// (V85's `pursuit_tx` columns set the precedent), and `NULL` passes a
/// `CHECK` by evaluating to unknown, so the guard costs existing rows
/// nothing. Disposal verbs (`empty_trash`, `purge`) are deliberately
/// not in the set — executing a decision already made is not a moment
/// anybody states a reason at — and the `CHECK` is what keeps a later
/// caller from quietly widening that vocabulary in data.
///
/// Not indexed: the reader is the asset's thread
/// (`idx_asset_comment_asset` already serves it), and no query filters
/// by gesture alone yet.
const V86_ASSET_COMMENT_GESTURE: &str = r#"
ALTER TABLE asset_comment ADD COLUMN gesture TEXT
    CHECK (gesture IN ('trash', 'trash_group', 'restore'));
"#;

/// Version 86 → 87: the pursuit stamp on a dispatch becomes a value.
///
/// `dispatch_job.pursuit_id` has carried `REFERENCES pursuit(id) ON
/// DELETE RESTRICT` since V79, and it was the one foreign key anywhere
/// in the schema pointing out of a raw-layer table into a forge one —
/// every other reference to `pursuit`, `project`, `line` and `cull` is
/// forge-internal. That single constraint is what made the two
/// inseparable at the database level: drop the forge's tables and the
/// raw layer's own schema stops standing up.
///
/// They are different domains with different lifecycles, and from the
/// forge's side the stamp on a dispatch row is a value, not a reference
/// the forge owns. So the column survives and the constraint does not:
/// same name, same type, same rows, same `idx_dispatch_pursuit` for
/// a caller to read a pursuit's rounds through. What the column
/// records — which pursuit this round was filed under — is a fact about
/// the round, and facts about a round are what `dispatch_job` is for.
/// What the constraint added on top was an ownership claim the forge
/// does not have.
///
/// Said in the terms that change here: **deleting a pursuit that
/// dispatches still name now succeeds**, and those dispatch rows are
/// left alone — each keeps its stamp, as a value that resolves to
/// nothing. Nothing rewrites them to NULL, because "filed under a
/// pursuit that has since been deleted" and "never filed" are different
/// histories and only one of them is true of these rows. `ON DELETE
/// RESTRICT` semantics survive nowhere: the delete is not refused, not
/// cascaded, and not nulled. A caller that needs to know whether a
/// stamp still names something live has to look — the schema no longer
/// answers that question on the way out, and no longer refuses a stamp
/// naming no row on the way in either.
///
/// SQLite cannot drop a constraint, so this is the canonical rebuild —
/// `CREATE dispatch_job_v87` → `INSERT … SELECT` → drop → rename →
/// recreate every index — in the V31 / V38 shape, and specifically the
/// V82 `pursuit_restamp` shape for a rebuild whose whole purpose is a
/// constraint. Two things that rebuild did not have to handle:
///
/// - **Every column is named on both sides**, rather than copied with
///   `INSERT … SELECT *`. `dispatch_job` reached twenty-five columns
///   through V19's rebuild plus four rounds of `ALTER TABLE … ADD
///   COLUMN` (V48, V50, V79, V83), and over a list that long a
///   column-order surprise should fail loudly here rather than
///   transpose data into neighbouring columns of the same type.
/// - **All four indexes are recreated**, not the two this change is
///   about: `DROP TABLE` takes every index on the table with it, so
///   `idx_dispatch_snapshot_created` and `idx_dispatch_state` have to
///   be written out again alongside `idx_dispatch_persona_created` and
///   `idx_dispatch_pursuit`.
///
/// An `App` step for the reason [`v40_asset_color`] writes down: the
/// canonical rebuild is safe only where [`migrate`] holds
/// `foreign_keys = OFF`, which it does around `App` steps and not
/// around `Sql` ones. The batch ends with `PRAGMA foreign_key_check`,
/// the V79 / V82 guard — the edges this step keeps (`snapshot_id`
/// RESTRICT, `persona_id` CASCADE, `source_group_id` SET NULL) are
/// re-declared on the new table, and a copy that landed a row past one
/// of them surfaces here rather than at the next write.
fn v87_dispatch_stamp_is_a_value(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(V87_DISPATCH_STAMP_UNBOUND)?;

    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v87: foreign_key_check reported violations after the rebuild".into()),
        ));
    }
    Ok(())
}

/// DDL half of V87 — see [`v87_dispatch_stamp_is_a_value`] for the
/// choices. The column list is the physical one, read off the migration
/// history (V19's rebuild, then V48, V50, V79, V83) rather than off
/// `DispatchRow::COLUMNS`, which is a read order and owes this table
/// nothing.
const V87_DISPATCH_STAMP_UNBOUND: &str = r#"
CREATE TABLE dispatch_job_v87 (
    id                BLOB PRIMARY KEY,
    snapshot_id       BLOB NOT NULL REFERENCES snapshot(id) ON DELETE RESTRICT,
    persona_id        BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    exporter_slug     TEXT NOT NULL,
    action            TEXT NOT NULL,
    params_json       TEXT NOT NULL DEFAULT '{}',
    state_slug        TEXT NOT NULL,
    state_message     TEXT,
    progress_current  INTEGER,
    progress_total    INTEGER,
    handle_kind       TEXT,
    handle_payload    TEXT,
    output_asset_ids  TEXT NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    completed_at      INTEGER,
    source_group_id   BLOB REFERENCES bucket(id) ON DELETE SET NULL,
    source_query_json TEXT,
    operator_ai       TEXT,
    author_kind       TEXT,
    author_subject    TEXT,
    attributed_via    TEXT,
    pursuit_id        BLOB,
    attempt_kind      TEXT,
    attempt_payload   TEXT
) STRICT;

INSERT INTO dispatch_job_v87
       (id, snapshot_id, persona_id, exporter_slug, action, params_json,
        state_slug, state_message, progress_current, progress_total,
        handle_kind, handle_payload, output_asset_ids, created_at,
        updated_at, completed_at, source_group_id, source_query_json,
        operator_ai, author_kind, author_subject, attributed_via,
        pursuit_id, attempt_kind, attempt_payload)
SELECT id, snapshot_id, persona_id, exporter_slug, action, params_json,
       state_slug, state_message, progress_current, progress_total,
       handle_kind, handle_payload, output_asset_ids, created_at,
       updated_at, completed_at, source_group_id, source_query_json,
       operator_ai, author_kind, author_subject, attributed_via,
       pursuit_id, attempt_kind, attempt_payload
  FROM dispatch_job;

DROP TABLE dispatch_job;
ALTER TABLE dispatch_job_v87 RENAME TO dispatch_job;

CREATE INDEX idx_dispatch_persona_created
    ON dispatch_job(persona_id, created_at DESC);
CREATE INDEX idx_dispatch_snapshot_created
    ON dispatch_job(snapshot_id, created_at DESC);
CREATE INDEX idx_dispatch_state
    ON dispatch_job(state_slug, created_at DESC);
CREATE INDEX idx_dispatch_pursuit
    ON dispatch_job(pursuit_id);
"#;

/// Version 87 → 88: drops the two tables V82 created for the close's
/// narrowing, and narrows the restamp vocabulary back to the one
/// subject that exists.
///
/// The concept those tables recorded is gone from the code — no verb
/// writes them, no read reaches them, and no type names them. What is
/// left is storage that only that concept ever filled, and leaving it
/// would not be neutral: both tables hold RESTRICT edges into
/// `pursuit`, `persona`, `pursuit_event` and `snapshot`, so rows
/// written before this change would go on refusing a persona purge
/// through a table nothing can any longer explain. Dropping them is
/// therefore the honest half of the deletion rather than an extra: the
/// alternative is a purge path that keeps naming the concept to
/// clear it.
///
/// **This destroys those rows.** They are the concept's own records —
/// which member of a close was kept or dropped, out of which frozen
/// candidate set — and nothing else refers to them; the candidate
/// snapshots they pointed at stay, unreferenced and content-addressed,
/// as any other unreferenced freeze does. The `pursuit_event` rows the
/// closes wrote are untouched, including the `snapshot_id` of any
/// close that froze a kept set: that column is the event's own, and a
/// row already written keeps what it recorded.
///
/// `pursuit_restamp` is rebuilt for its CHECK. V82 widened it to admit
/// a second subject kind that no verb ever minted — the domain has
/// only ever parsed `'dispatch'` — so the copy translates nothing and
/// can find nothing to reject. Narrowing a CHECK on a STRICT table
/// costs a rebuild the same way widening one does (the V79 / V82
/// reasoning), and the three indexes are recreated because `DROP
/// TABLE` takes every index with it.
///
/// An App step for the reason [`v87_dispatch_stamp_is_a_value`] gives:
/// the canonical rebuild is safe only where [`migrate`] holds
/// `foreign_keys = OFF`, which it does around `App` steps and not
/// around `Sql` ones. The closing `PRAGMA foreign_key_check` is the
/// V79 / V82 / V87 guard — here it answers for the drops as much as
/// the rebuild, since a table going away is exactly what would leave a
/// dangling edge if anything still pointed at one.
fn v88_drop_the_close_record(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(V88_DROP_CLOSE_RECORD)?;

    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v88: foreign_key_check reported violations after the drop".into()),
        ));
    }
    Ok(())
}

/// DDL half of V88 — see [`v88_drop_the_close_record`] for the
/// choices. The member table goes first: it is the side that holds the
/// edge.
const V88_DROP_CLOSE_RECORD: &str = r#"
DROP TABLE cull_member;
DROP TABLE cull;

CREATE TABLE pursuit_restamp_v88 (
    id              BLOB PRIMARY KEY,
    subject_kind    TEXT NOT NULL
        CHECK (subject_kind IN ('dispatch')),
    subject_id      BLOB NOT NULL,
    from_pursuit_id BLOB REFERENCES pursuit(id) ON DELETE RESTRICT,
    to_pursuit_id   BLOB NOT NULL REFERENCES pursuit(id) ON DELETE RESTRICT,
    author_kind     TEXT,
    author_subject  TEXT,
    operator_ai     TEXT,
    attributed_via  TEXT,
    created_at      INTEGER NOT NULL
) STRICT;

INSERT INTO pursuit_restamp_v88 SELECT * FROM pursuit_restamp;
DROP TABLE pursuit_restamp;
ALTER TABLE pursuit_restamp_v88 RENAME TO pursuit_restamp;

CREATE INDEX idx_pursuit_restamp_subject
    ON pursuit_restamp(subject_kind, subject_id);
CREATE INDEX idx_pursuit_restamp_to
    ON pursuit_restamp(to_pursuit_id);
CREATE INDEX idx_pursuit_restamp_from
    ON pursuit_restamp(from_pursuit_id);
"#;

/// V89 — drops `pursuit_restamp`, the table behind the restamp repair
/// verb, now that the verb is gone.
///
/// The forge no longer dispatches, so the only subject a restamp could
/// ever name no longer reaches it: `RestampSubject` had one variant,
/// the CHECK V88 narrowed back admitted one value, and the service verb
/// that minted these rows is deleted. Nothing reads the table, nothing
/// writes it, and the persona purge no longer has to sweep it.
///
/// **This destroys those rows.** They recorded which pursuit a round
/// was re-filed under, which is a statement about a relationship the
/// forge no longer has. The `pursuit` rows the two FK columns
/// referenced are untouched and only referenced less;
/// `dispatch_job.pursuit_id` still exists at this step and is dropped
/// by the next one.
///
/// A plain `Sql` step, unlike the rebuilds around it. Nothing
/// references `pursuit_restamp`; its two foreign keys point outward, so
/// dropping it removes edges rather than stranding any, and the three
/// indexes go with the table. That is the case `DROP TABLE` handles
/// with foreign keys left on, so there is no rebuild to hold
/// `foreign_keys = OFF` for.
const V89_DROP_THE_RESTAMP_RECORD: &str = r#"
DROP TABLE pursuit_restamp;
"#;

/// V90 — takes the pursuit stamp off the dispatch, and the lookup lane
/// that read it off the asset.
///
/// A dispatch is a raw-layer export: a frozen input, an exporter, an
/// action, and what came back. Which line of work somebody was on when
/// they started it is not a fact about the export, and
/// `dispatch_job.pursuit_id` was the schema saying otherwise. V87 took
/// the foreign key off that column three steps ago, which left the
/// stamp as a value nothing owns; this takes the value.
///
/// Two tables, for one reason. `asset.trace_pursuit_id` (V80) exists
/// to make a pursuit's returns an index seek instead of a scan, and
/// the read it was cut for — the returns join — went with the column
/// it joined through: it resolved rounds by `SELECT id FROM
/// dispatch_job WHERE pursuit_id = ?`, which no longer parses. A
/// generated column and a partial index that no query names are not
/// neutral leftovers — they are re-derived on every write to `asset`,
/// which is the library's hottest table.
///
/// **This destroys the filing.** Every dispatch row loses which
/// pursuit it was started under, and nothing anywhere else records it:
/// `pursuit_restamp` (the other copy) went at V88, and the `_trace`
/// note in `asset.extra` keeps its `pursuit_id` text because that bag
/// is what an ingest recorded rather than what this schema asserts —
/// the generated column reading it goes, the JSON does not. Nothing
/// re-derives the stamp afterwards. It is deleted because the design
/// says a dispatch does not carry one, not because it was empty.
///
/// The two halves need different tools:
///
/// - `dispatch_job` gets the canonical rebuild — `CREATE
///   dispatch_job_v90` → `INSERT … SELECT` → drop → rename → recreate
///   every index — in the V87 shape it rebuilt for the constraint, one
///   column shorter. Every column is named on both sides for the
///   reason V87 gives: a twenty-four-column list should fail loudly
///   rather than transpose neighbouring columns of the same type. All
///   three surviving indexes are recreated, because `DROP TABLE` takes
///   every index with it, and `idx_dispatch_pursuit` is not among them
///   — it indexed the column being dropped.
/// - `asset` gets `DROP COLUMN`, not a rebuild. The column is VIRTUAL
///   generated, so no row holds a byte of it, and rebuilding the
///   library's largest table to remove an expression would cost the
///   whole table to change a schema line. `idx_asset_trace_pursuit`
///   is dropped first: SQLite refuses `DROP COLUMN` on an indexed
///   column, and this one is named in the index's `WHERE` clause too.
///
/// An `App` step for the reason [`v40_asset_color`] writes down: the
/// canonical rebuild is safe only where [`migrate`] holds
/// `foreign_keys = OFF`, which it does around `App` steps and not
/// around `Sql` ones. The closing `PRAGMA foreign_key_check` is the
/// V79 / V82 / V87 guard — the edges this step keeps (`snapshot_id`
/// RESTRICT, `persona_id` CASCADE, `source_group_id` SET NULL) are
/// re-declared on the new table, and a copy that landed a row past one
/// of them surfaces here rather than at the next write.
fn v90_drop_the_pursuit_stamp(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(V90_DROP_THE_PURSUIT_STAMP)?;

    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("v90: foreign_key_check reported violations after the rebuild".into()),
        ));
    }
    Ok(())
}

/// DDL half of V90 — see [`v90_drop_the_pursuit_stamp`] for the
/// choices. The column list is V87's physical one minus `pursuit_id`,
/// read off the migration history rather than off
/// `DispatchRow::COLUMNS`, which is a read order and owes this table
/// nothing.
const V90_DROP_THE_PURSUIT_STAMP: &str = r#"
CREATE TABLE dispatch_job_v90 (
    id                BLOB PRIMARY KEY,
    snapshot_id       BLOB NOT NULL REFERENCES snapshot(id) ON DELETE RESTRICT,
    persona_id        BLOB NOT NULL REFERENCES persona(id) ON DELETE CASCADE,
    exporter_slug     TEXT NOT NULL,
    action            TEXT NOT NULL,
    params_json       TEXT NOT NULL DEFAULT '{}',
    state_slug        TEXT NOT NULL,
    state_message     TEXT,
    progress_current  INTEGER,
    progress_total    INTEGER,
    handle_kind       TEXT,
    handle_payload    TEXT,
    output_asset_ids  TEXT NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    completed_at      INTEGER,
    source_group_id   BLOB REFERENCES bucket(id) ON DELETE SET NULL,
    source_query_json TEXT,
    operator_ai       TEXT,
    author_kind       TEXT,
    author_subject    TEXT,
    attributed_via    TEXT,
    attempt_kind      TEXT,
    attempt_payload   TEXT
) STRICT;

INSERT INTO dispatch_job_v90
       (id, snapshot_id, persona_id, exporter_slug, action, params_json,
        state_slug, state_message, progress_current, progress_total,
        handle_kind, handle_payload, output_asset_ids, created_at,
        updated_at, completed_at, source_group_id, source_query_json,
        operator_ai, author_kind, author_subject, attributed_via,
        attempt_kind, attempt_payload)
SELECT id, snapshot_id, persona_id, exporter_slug, action, params_json,
       state_slug, state_message, progress_current, progress_total,
       handle_kind, handle_payload, output_asset_ids, created_at,
       updated_at, completed_at, source_group_id, source_query_json,
       operator_ai, author_kind, author_subject, attributed_via,
       attempt_kind, attempt_payload
  FROM dispatch_job;

DROP TABLE dispatch_job;
ALTER TABLE dispatch_job_v90 RENAME TO dispatch_job;

CREATE INDEX idx_dispatch_persona_created
    ON dispatch_job(persona_id, created_at DESC);
CREATE INDEX idx_dispatch_snapshot_created
    ON dispatch_job(snapshot_id, created_at DESC);
CREATE INDEX idx_dispatch_state
    ON dispatch_job(state_slug, created_at DESC);

DROP INDEX idx_asset_trace_pursuit;
ALTER TABLE asset DROP COLUMN trace_pursuit_id;
"#;

/// V91 — takes the base-event pin off the ledger: the column V85 added
/// to hold which version of an entry an `in` was looking at, the CHECK
/// pairing it to a target, and the index over it.
///
/// A pursuit is cut from a line and its `in` already names the entry it
/// works on. A version claim on top of that entry is a different
/// statement, and nothing was ever built to make it: no command carries
/// one, the one production writer hard-codes the target it derives both
/// columns from to `None`, and no reader has ever asked what the column
/// held. The `Option` was saying "nothing fills this yet" rather than
/// stating a model, so it goes rather than waiting for a merge that may
/// never want it. `target_entry_id` stays, and so does its own CHECK.
///
/// **This destroys nothing.** Every row this codebase has written holds
/// NULL here — the writer has hard-coded no target since the column
/// landed, and `git log -S` finds no earlier revision that did
/// otherwise. What it cannot answer for is a row written by hand
/// against a real profile, and no migration can: the value is dropped
/// either way, which is what dropping a column means.
///
/// A plain `Sql` step, and not the canonical rebuild V90 needed. The
/// index is dropped first because SQLite refuses `DROP COLUMN` on an
/// indexed column — the same condition V90 hit on
/// `idx_asset_trace_pursuit`. The other documented refusal, a column
/// "used in a foreign key constraint", was measured rather than assumed
/// before this shape was chosen: a throwaway table whose dropped column
/// carried `REFERENCES parent(id) ON DELETE RESTRICT` dropped cleanly
/// under `PRAGMA foreign_keys = ON`, so that condition covers a column
/// another table's constraint depends on, not an outbound reference
/// from the one going. The CHECK goes with the column because it is a
/// *column* constraint on it, which is the arrangement V85 chose and
/// measured; nothing else in the table names it. With no rebuild there
/// is no copy to hold `foreign_keys = OFF` for, and no reason to
/// hand-copy an append-only ledger — the argument V85 made for altering
/// `pursuit_tx` rather than rebuilding it holds just as well in this
/// direction.
const V91_DROP_THE_BASE_EVENT_PIN: &str = r#"
DROP INDEX idx_pursuit_tx_base_event;
ALTER TABLE pursuit_tx DROP COLUMN base_event_id;
"#;

/// V92 — moves the three-state out of the digest columns (issue #17):
/// a status column beside each of the three hash columns, a reason
/// column for the statuses that carry a payload, and the digest columns
/// left holding digests and nothing else.
///
/// Until here `content_hash` / `content_region_hash` / `meta_hash`
/// carried their own explanations inline — `unsupported:<mime>`,
/// `unsupported:empty-span`, `unsupported:too-large`,
/// `unsupported:not-walked`, `unhashable:no-bytes` — so every reader
/// had to know the marker grammar before it could tell a measurement
/// from a note about why there is no measurement. The established shape
/// for that distinction (`getxattr(2)`'s errno beside a value) is a
/// nullable payload beside a non-nullable status, and this step adopts
/// it before a further marker or axis raises the cost of moving again.
///
/// # The conversion
///
/// One `UPDATE` computes every status from the value the column holds,
/// spelling the marker literals inline the way every landed migration
/// does (the domain constants may move; what this step did may not):
///
/// - `NULL` → `pending` — nobody has looked, on any axis.
/// - the axis's own current-generation digest → `computed`.
/// - `unhashable:no-bytes` → `no-bytes`; the three fixed `unsupported:`
///   markers → their status of the same name.
/// - any other `unsupported:<mime>` → `unsupported`, with the mime —
///   the part of the marker that was genuinely valuable — kept in the
///   reason column (`substr(…, 13)` strips the 12-character prefix).
/// - anything else → `pending` on the versioned axes (a superseded
///   generation reads as no answer, exactly as `is_axis_answer` read
///   it) and `computed` on the file axis, whose vocabulary is not
///   versioned — holding a value there was holding an answer.
///
/// A second `UPDATE` then clears the marker spellings out of the digest
/// columns. Only the marker family is cleared, not everything
/// non-`computed`: a digest of a superseded generation is still a
/// digest, and the walk overwrites it — nulling it here would destroy a
/// measurement to enforce a shape.
///
/// # What deliberately does not change
///
/// `meta_raw` keeps its own inline markers: it is a payload column, not
/// a digest column, nothing groups on it, and its vocabulary
/// (`material_meta_raw`) was out of the issue's scope. The `status`
/// vocabulary's newcomer, `failed`, is written by no conversion — no
/// pre-V92 row can carry it, because the old design recorded nothing
/// for an unreadable original. That gap is exactly what the new column
/// fixes going forward (`mark_material_unreadable`).
///
/// No index: the queries that filter on these columns are the count
/// behind the progress notice and the backfill's page query, both of
/// which already scanned `material` whole under the marker design.
const V92_MATERIAL_FINGERPRINT_STATUS: &str = r#"
ALTER TABLE material ADD COLUMN content_hash_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE material ADD COLUMN content_hash_reason TEXT;
ALTER TABLE material ADD COLUMN content_region_hash_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE material ADD COLUMN content_region_hash_reason TEXT;
ALTER TABLE material ADD COLUMN meta_hash_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE material ADD COLUMN meta_hash_reason TEXT;

UPDATE material SET
    content_hash_status = CASE
        WHEN content_hash IS NULL THEN 'pending'
        WHEN content_hash = 'unhashable:no-bytes' THEN 'no-bytes'
        ELSE 'computed'
    END,
    content_region_hash_status = CASE
        WHEN content_region_hash IS NULL THEN 'pending'
        WHEN content_region_hash GLOB 'cr1-sha256:*' THEN 'computed'
        WHEN content_region_hash = 'unhashable:no-bytes' THEN 'no-bytes'
        WHEN content_region_hash = 'unsupported:empty-span' THEN 'empty-span'
        WHEN content_region_hash = 'unsupported:too-large' THEN 'too-large'
        WHEN content_region_hash = 'unsupported:not-walked' THEN 'not-walked'
        WHEN content_region_hash GLOB 'unsupported:*' THEN 'unsupported'
        ELSE 'pending'
    END,
    content_region_hash_reason = CASE
        WHEN content_region_hash GLOB 'unsupported:*'
         AND content_region_hash NOT IN
             ('unsupported:empty-span', 'unsupported:too-large', 'unsupported:not-walked')
        THEN substr(content_region_hash, 13)
    END,
    meta_hash_status = CASE
        WHEN meta_hash IS NULL THEN 'pending'
        WHEN meta_hash GLOB 'm1-sha256:*' THEN 'computed'
        WHEN meta_hash = 'unhashable:no-bytes' THEN 'no-bytes'
        WHEN meta_hash = 'unsupported:empty-span' THEN 'empty-span'
        WHEN meta_hash = 'unsupported:too-large' THEN 'too-large'
        WHEN meta_hash = 'unsupported:not-walked' THEN 'not-walked'
        WHEN meta_hash GLOB 'unsupported:*' THEN 'unsupported'
        ELSE 'pending'
    END,
    meta_hash_reason = CASE
        WHEN meta_hash GLOB 'unsupported:*'
         AND meta_hash NOT IN
             ('unsupported:empty-span', 'unsupported:too-large', 'unsupported:not-walked')
        THEN substr(meta_hash, 13)
    END;

UPDATE material SET content_hash = NULL
 WHERE content_hash = 'unhashable:no-bytes';
UPDATE material SET content_region_hash = NULL
 WHERE content_region_hash GLOB 'unsupported:*'
    OR content_region_hash = 'unhashable:no-bytes';
UPDATE material SET meta_hash = NULL
 WHERE meta_hash GLOB 'unsupported:*'
    OR meta_hash = 'unhashable:no-bytes';
"#;

/// V93 — `.json` gets the mime `guess_mime` now answers (issue #16):
/// rows written while a whole `.json` file was `text/plain` move to
/// `application/json`, so the content-axis probe that declares that
/// mime can reach them.
///
/// The same repair V45 made for fragments, in the other direction:
/// the classification is fixed at the source
/// (`asterism_core::domain::material::guess_mime`), and the rows
/// already written need this. The predicate mirrors that function's
/// judgement over the tagged locator (V63's shape): a `file`'s path
/// answers by extension, a `remote`'s target and a `logical`'s name by
/// the same suffix with a query string stripped, and a `record` never
/// answers `.json` at all — the record is the artefact, the container's
/// extension answers for the wrong thing. `LIKE` is ASCII
/// case-insensitive, which is the lowercasing the sniff applies, and
/// `%.json` cannot match `.jsonl`, which deliberately stays behind.
///
/// The mirror has one seam, and the file arm's second condition is it:
/// a file **named** `.json` has no extension to `Path::extension`, so
/// the sniff never called it JSON, while `extension_of_text` — the
/// reading the other two arms mirror — answers `json` for a target or
/// name ending in `/.json`. The predicate follows each arm's own
/// reader, which is what "mirrors the judgement" has to mean when the
/// judgements differ.
///
/// Guarded on `mime = 'text/plain'` — the only value the old arm ever
/// wrote for these rows — so a mime an importer stated explicitly is
/// not overridden by a guess. Idempotent by shape: the second run finds
/// no `text/plain` `.json` rows.
const V93_JSON_MATERIAL_MIME: &str = r#"
UPDATE material
   SET mime = 'application/json'
 WHERE mime = 'text/plain'
   AND (
        (json_extract(locator, '$.kind') = 'file'
         AND json_extract(locator, '$.path') LIKE '%.json'
         AND json_extract(locator, '$.path') NOT LIKE '%/.json')
     OR (json_extract(locator, '$.kind') = 'remote'
         AND CASE
               WHEN instr(json_extract(locator, '$.target'), '?') > 0
               THEN substr(json_extract(locator, '$.target'), 1,
                           instr(json_extract(locator, '$.target'), '?') - 1)
               ELSE json_extract(locator, '$.target')
             END LIKE '%.json')
     OR (json_extract(locator, '$.kind') = 'logical'
         AND CASE
               WHEN instr(json_extract(locator, '$.name'), '?') > 0
               THEN substr(json_extract(locator, '$.name'), 1,
                           instr(json_extract(locator, '$.name'), '?') - 1)
               ELSE json_extract(locator, '$.name')
             END LIKE '%.json')
   );
"#;

/// V94 — hands the rows V93 renamed back to the fingerprint walk: the
/// way back a format owes the rows it was refused on, which V72 and V76
/// paid for JPEG's two axes with an `UPDATE` over the marker. Since V92
/// the refusal is a status beside the column rather than a spelling
/// inside it, so the same step is spelled on the status: `unsupported`
/// returns to `pending` — "nobody has looked", which is what is true of
/// these rows now that a probe exists — and the recorded reason goes
/// with it: `text/plain` where V93 renamed the old guess, and whatever
/// mime an importer stated on a row that already declared
/// `application/json` itself.
///
/// Filtered on the mime V93 wrote rather than on the recorded reason,
/// because the reason names what the row *declared then* and the walk
/// selects on what it *declares now* — the two disagree on exactly the
/// rows this step exists for.
///
/// The content axis only. The JSON probe does not claim the meta axis —
/// a JSON document has no container metadata — so a meta row cleared
/// here would come back holding what it holds now, the reasoning V72
/// gave for leaving JPEG's meta column alone while that axis had no
/// reading. The digest column itself needs nothing: V92 already left it
/// NULL for every non-`computed` row.
const V94_CLEAR_STALE_JSON_CONTENT_MARKER: &str = r#"
UPDATE material
   SET content_region_hash_status = 'pending',
       content_region_hash_reason = NULL
 WHERE mime = 'application/json'
   AND content_region_hash_status = 'unsupported';
"#;

/// The visual-feature projection (#112): one row per encoded material
/// per model configuration, plus the failure records that retire a row
/// from the extraction walk.
///
/// The key carries the full derivation identity — `model_id`,
/// `feature_kind`, with `preprocess_ver` beside them — so two models'
/// vectors coexist without seeing each other and replacing a model
/// deletes exactly its own rows (`DELETE … WHERE model_id = ?`), never
/// a value a person asserted. `dim` is stored rather than derived so a
/// blob whose length disagrees with its declared identity is detectably
/// corrupt.
///
/// Rows exist only once extraction has answered: `computed` carries the
/// vector, `failed` carries a reason and no vector (undecodable bytes,
/// unreadable original) — absence *is* the pending state, which is what
/// lets the walk's `NOT EXISTS` predicate offer a row exactly once.
const V95_VISUAL_FEATURE: &str = r#"
CREATE TABLE visual_feature (
    asset_id       BLOB NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
    ord            INTEGER NOT NULL,
    model_id       TEXT NOT NULL,
    feature_kind   TEXT NOT NULL,
    preprocess_ver INTEGER NOT NULL,
    dim            INTEGER,
    vector         BLOB,
    status         TEXT NOT NULL CHECK (status IN ('computed', 'failed')),
    reason         TEXT,
    extracted_at   INTEGER NOT NULL,
    PRIMARY KEY (asset_id, ord, model_id, feature_kind)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_visual_feature_model ON visual_feature (model_id, feature_kind, status);
"#;

/// Migrations in application order. **Append only** — never rewrite an
/// existing batch.
const MIGRATIONS: &[Step] = &[
    Step::Sql(V1_INITIAL_SCHEMA),
    Step::Sql(V2_ASSET_SOURCE_UNIQUE),
    Step::Sql(V3_EDGE_TO_KIND_INDEX),
    Step::Sql(V4_GROUP_TABLES),
    Step::Sql(V5_ASSET_BUCKET_POSITION),
    Step::Sql(V6_DIR_TABLES),
    Step::Sql(V7_BUCKET_LINK),
    Step::Sql(V8_ASSET_CONTENT_FLAGS),
    Step::Sql(V9_PERSONA_THEME),
    Step::Sql(V10_PERSONA_PROFILE),
    Step::Sql(V11_ASSET_BODY),
    Step::Sql(V12_DISPATCH_TABLES),
    Step::Sql(V13_ASSET_RATING),
    Step::Sql(V14_ASSET_PALETTE),
    Step::Sql(V15_ASSET_COMMENT),
    Step::Sql(V16_SAVED_QUERY),
    Step::Sql(V17_ASSET_INDEX_COVERING),
    Step::Sql(V18_EVENT_LOG),
    Step::App(v19_selection_model),
    Step::Sql(V20_BUCKET_REFRESH_SIGNAL),
    Step::Sql(V21_THREADS),
    Step::Sql(V22_MODALITY),
    Step::Sql(V23_SESSION),
    Step::Sql(V24_ASSET_BUNDLE_ID),
    Step::Sql(V25_MIGRATE_NON_DIALOGUE_SESSION_ID),
    Step::App(v26_dialogue_session_backfill),
    Step::App(v27_asset_session_fk_check),
    Step::Sql(V28_ASSET_COMPOSITE_COLUMNS),
    Step::App(v29_materialise_session_composites),
    Step::Sql(V30_ASSET_EXTERNAL_KEY_UNIQUE),
    Step::App(v31_drop_session_scaffolding),
    Step::Sql(V32_APP_SETTING),
    Step::Sql(V33_TRASH_COLUMNS),
    Step::Sql(V34_PERSONA_TRASH),
    Step::Sql(V35_DIAG_LOG),
    Step::Sql(V36_OBSERVATION_STREAMS),
    Step::Sql(V37_MATERIAL_LAYER),
    Step::App(v38_modality_optional),
    Step::Sql(V39_DROP_SHOW_MESSAGES_SETTING),
    Step::App(v40_asset_color),
    Step::Sql(V41_MATERIAL_CONTENT_HASH),
    Step::Sql(V42_SESSION_MODALITY),
    Step::Sql(V43_MESSAGE_MODALITY),
    Step::Sql(V44_MODALITY_TERMINAL_BIT),
    Step::Sql(V45_FRAGMENT_MATERIAL_MIME),
    Step::Sql(V46_ASSET_UPDATED_INDEX),
    Step::Sql(V47_ASSET_ATTRIBUTION),
    Step::Sql(V48_DISPATCH_OPERATOR),
    Step::App(v49_instance_identity),
    Step::Sql(V50_ATTRIBUTION_CHANNEL),
    Step::Sql(V51_ASSET_FOLD),
    Step::Sql(V52_ASSET_ON_DUPLICATE),
    Step::Sql(V53_DUPLICATE_CONFLICT),
    Step::Sql(V54_DUPLICATE_CONFLICT_FOLD_EXCLUSION),
    Step::Sql(V55_MATERIAL_CONTENT_REGION_HASH),
    Step::App(v56_walk_deferred_content_regions),
    Step::Sql(V57_ASSET_INDEX_COVERING_LIVE),
    Step::Sql(V58_ASSET_TEXT_INDEX),
    Step::Sql(V59_ASSET_INDEX_COVERING_METRICS),
    Step::Sql(V60_ASSET_TIMELINE_MARK),
    Step::Sql(V61_ASSET_SOURCE_LOOKUP),
    Step::Sql(V62_ASSET_EXTERNAL_KEY_LOOKUP),
    Step::App(v63_rewrite_locators_as_tagged_json),
    Step::Sql(V64_MATERIAL_META),
    Step::App(v65_walk_deferred_material_meta),
    Step::Sql(V66_MATERIAL_MARK),
    Step::Sql(V67_ASSET_ALBUM_META),
    Step::Sql(V68_ASSET_COMMENT_KEEPS_ORPHANS),
    Step::Sql(V69_ASSET_PIXEL_DIMS),
    Step::Sql(V70_ASSET_INDEX_COVERING_PIXELS),
    Step::Sql(V71_ASSET_DIMS_PROBED_AT),
    Step::Sql(V72_CLEAR_STALE_JPEG_CONTENT_MARKER),
    Step::Sql(V73_SERIES_STRATEGY),
    Step::Sql(V74_MATERIAL_SERIES_STRATEGY),
    Step::Sql(V75_MATERIAL_META_RAW),
    Step::Sql(V76_CLEAR_STALE_JPEG_META_MARKER),
    Step::App(v77_series_strategy_admits_the_exif_decoder),
    Step::App(v78_material_layers),
    Step::App(v79_pursuit),
    Step::Sql(V80_TRACE_LOOKUP),
    Step::Sql(V81_DERIVED_TEXT),
    Step::App(v82_cull_record),
    Step::Sql(V83_DISPATCH_ATTEMPT),
    Step::Sql(V84_FORGE_PROJECT_LINE),
    Step::Sql(V85_FILING_AND_TARGETED_IN),
    Step::Sql(V86_ASSET_COMMENT_GESTURE),
    Step::App(v87_dispatch_stamp_is_a_value),
    Step::App(v88_drop_the_close_record),
    Step::Sql(V89_DROP_THE_RESTAMP_RECORD),
    Step::App(v90_drop_the_pursuit_stamp),
    Step::Sql(V91_DROP_THE_BASE_EVENT_PIN),
    Step::Sql(V92_MATERIAL_FINGERPRINT_STATUS),
    Step::Sql(V93_JSON_MATERIAL_MIME),
    Step::Sql(V94_CLEAR_STALE_JSON_CONTENT_MARKER),
    Step::Sql(V95_VISUAL_FEATURE),
];

/// Latest schema version (`MIGRATIONS.len()`).
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

/// Applies every pending migration up to the latest version. Idempotent:
/// re-running against an already-up-to-date database is a no-op.
///
/// Each batch runs inside its own transaction; a failure rolls back only
/// that batch and leaves earlier migrations in place.
pub fn migrate(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    migrate_to(conn, MIGRATIONS.len())
}

/// Applies pending migrations up to `target` (exclusive upper bound =
/// resulting `user_version`). Split out of [`migrate`] so upgrade tests
/// can stop at a historical version, seed legacy-shape data, and then
/// resume.
pub(crate) fn migrate_to(conn: &mut Connection, target: usize) -> Result<(), rusqlite::Error> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, step) in MIGRATIONS
        .iter()
        .enumerate()
        .take(target)
        .skip(current.max(0) as usize)
    {
        match step {
            Step::Sql(batch) => {
                let tx = conn.transaction()?;
                tx.execute_batch(batch)?;
                tx.pragma_update(None, "user_version", (index + 1) as i64)?;
                tx.commit()?;
            }
            Step::App(f) => {
                // FK enforcement must be toggled outside the transaction.
                conn.pragma_update(None, "foreign_keys", "OFF")?;
                let applied = (|| {
                    let tx = conn.transaction()?;
                    f(&tx)?;
                    tx.pragma_update(None, "user_version", (index + 1) as i64)?;
                    tx.commit()
                })();
                let restored = conn.pragma_update(None, "foreign_keys", "ON");
                applied.and(restored)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_contract::query_group::QueryGroupQuery;
    use asterism_core::domain::snapshot_hash::content_hash;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn
    }

    fn seed_persona(conn: &Connection) -> Uuid {
        let id = Uuid::now_v7();
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, 'p', 'P', 0, 0)",
            params![id],
        )
        .unwrap();
        id
    }

    fn seed_asset(conn: &Connection, persona: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[]', 0, 0, 0)",
            params![id, persona, format!("a-{id}.md")],
        )
        .unwrap();
        id
    }

    fn seed_selection(
        conn: &Connection,
        persona: Uuid,
        created: i64,
        promoted: Option<Uuid>,
        members: &[Uuid],
    ) -> Uuid {
        let id = Uuid::now_v7();
        conn.execute(
            "INSERT INTO selection (id, persona_id, name, promoted_group_id, \
                                    created_at, updated_at) \
             VALUES (?1, ?2, NULL, ?3, ?4, ?4)",
            params![id, persona, promoted, created],
        )
        .unwrap();
        for (pos, m) in members.iter().enumerate() {
            conn.execute(
                "INSERT INTO selection_asset (selection_id, asset_id, position) \
                 VALUES (?1, ?2, ?3)",
                params![id, m, pos as i64],
            )
            .unwrap();
        }
        id
    }

    fn seed_dispatch(conn: &Connection, selection: Uuid, persona: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        conn.execute(
            "INSERT INTO dispatch_job (id, selection_id, persona_id, exporter_slug, \
                                       action, state_slug, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'file', 'export', 'done', 0, 0)",
            params![id, selection, persona],
        )
        .unwrap();
        id
    }

    #[test]
    fn v19_upgrade_dedupes_remaps_and_transcribes() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 18).unwrap();

        let persona = seed_persona(&conn);
        let a1 = seed_asset(&conn, persona);
        let a2 = seed_asset(&conn, persona);
        // Manual bucket whose name will collide with the saved_query's.
        let g1 = Uuid::now_v7();
        conn.execute(
            "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'Trip', 0, 0)",
            params![g1, persona],
        )
        .unwrap();

        // sel1 / sel2 share the same ordered member list (dedupe pair);
        // sel3 has the reverse order (order is identity → stays);
        // sel3 also promoted bucket g1.
        let sel1 = seed_selection(&conn, persona, 100, None, &[a1, a2]);
        let sel2 = seed_selection(&conn, persona, 200, None, &[a1, a2]);
        let sel3 = seed_selection(&conn, persona, 300, Some(g1), &[a2, a1]);
        let d1 = seed_dispatch(&conn, sel1, persona);
        let d2 = seed_dispatch(&conn, sel2, persona);

        // Legacy saved_query: piggybacked search_text + colliding name.
        let sq = Uuid::now_v7();
        conn.execute(
            "INSERT INTO saved_query (id, persona_id, name, filter_json, sort_json, \
                                      position, created_at, updated_at) \
             VALUES (?1, ?2, 'Trip', ?3, ?4, 0, 0, 0)",
            params![
                sq,
                persona,
                format!(
                    r#"{{"persona_id":"{persona}","group_ids":["{g1}"],"search_text":"sunset"}}"#
                ),
                r#"{"target":"tag","order":"alpha","reverse":true}"#,
            ],
        )
        .unwrap();

        migrate_to(&mut conn, 19).unwrap();

        // Dedupe: sel1 canonical, sel2 folded into it, sel3 distinct.
        let snapshots: Vec<(Uuid, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, content_hash FROM snapshot ORDER BY created_at")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].0, sel1);
        assert_eq!(snapshots[1].0, sel3);
        let expect_hash = content_hash([a1.to_string().as_str(), a2.to_string().as_str()]);
        assert_eq!(snapshots[0].1, expect_hash);

        // Both dispatch jobs now reference the canonical snapshot.
        for d in [d1, d2] {
            let sid: Uuid = conn
                .query_row(
                    "SELECT snapshot_id FROM dispatch_job WHERE id = ?1",
                    params![d],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sid, sel1);
        }

        // promoted_group_id flipped into bucket.origin_snapshot_id.
        let (origin, kind): (Option<Uuid>, String) = conn
            .query_row(
                "SELECT origin_snapshot_id, kind FROM bucket WHERE id = ?1",
                params![g1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(origin, Some(sel3));
        assert_eq!(kind, "manual");

        // saved_query transcribed: suffixed name, v1 query_json with the
        // search_text un-piggybacked and raw group_ids preserved.
        let (qname, qjson): (String, String) = conn
            .query_row(
                "SELECT name, query_json FROM bucket WHERE id = ?1",
                params![sq],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(qname, "Trip (query)");
        let parsed = QueryGroupQuery::parse(&qjson).unwrap();
        assert_eq!(parsed.search_text.as_deref(), Some("sunset"));
        assert_eq!(parsed.filter.group_ids, vec![g1.to_string()]);
        assert_eq!(
            parsed.sort,
            serde_json::from_str(r#"{"target":"tag","order":"alpha","reverse":true}"#).unwrap()
        );

        // Legacy tables are gone.
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE name IN ('selection', 'selection_asset', 'saved_query')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);

        // RESTRICT: a snapshot referenced by dispatch history cannot be
        // deleted (the GC-only deletion contract).
        let err = conn.execute("DELETE FROM snapshot WHERE id = ?1", params![sel1]);
        assert!(
            err.is_err(),
            "RESTRICT must block history-referenced delete"
        );

        // The un-referenced... sel3 IS referenced (bucket origin). Its
        // delete must also be blocked by the bucket-side RESTRICT.
        let err = conn.execute("DELETE FROM snapshot WHERE id = ?1", params![sel3]);
        assert!(err.is_err(), "origin-referenced delete must be blocked too");
    }

    #[test]
    fn v19_empty_selections_dedupe_on_empty_hash() {
        // Zero-member selections are legal content objects; two of them
        // (same persona) hash identically and must fold into one row.
        let mut conn = test_conn();
        migrate_to(&mut conn, 18).unwrap();
        let persona = seed_persona(&conn);
        let sel1 = seed_selection(&conn, persona, 100, None, &[]);
        let _sel2 = seed_selection(&conn, persona, 200, None, &[]);

        migrate_to(&mut conn, 19).unwrap();

        let (count, id, hash): (i64, Uuid, String) = conn
            .query_row("SELECT COUNT(*), id, content_hash FROM snapshot", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(count, 1, "empty twins fold into the oldest row");
        assert_eq!(id, sel1);
        assert_eq!(hash, content_hash([]));
    }

    #[test]
    fn v19_malformed_filter_json_aborts_loudly_and_rolls_back() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 18).unwrap();
        let persona = seed_persona(&conn);
        conn.execute(
            "INSERT INTO saved_query (id, persona_id, name, filter_json, sort_json, \
                                      position, created_at, updated_at) \
             VALUES (?1, ?2, 'Broken', 'not json at all', '{}', 0, 0, 0)",
            params![Uuid::now_v7(), persona],
        )
        .unwrap();

        let err = migrate_to(&mut conn, 19);
        assert!(err.is_err(), "malformed filter_json must abort the wave");

        // The failed batch rolled back: still at V18 with the legacy
        // tables intact and re-runnable.
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 18);
        let legacy: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE name IN ('selection', 'saved_query')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legacy, 2);
    }

    #[test]
    fn v22_seeds_the_eleven_modality_master_rows() {
        // Version-scoped to V22: later batches add more rows to the
        // master (V28 seeds the `session` composition modality), so this
        // test stops at V22 to lock the original seed exactly.
        let mut conn = test_conn();
        migrate_to(&mut conn, 22).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM modality", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 11, "the seed carries the 11 current UI slugs");

        // cover_template is set only on the three special-cased slugs so
        // the master reproduces the pre-master cover_gen behaviour.
        let with_template: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM modality WHERE cover_template IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(with_template, 3);

        // Spot-check the kind mapping the design pins: tape → term is
        // the sole term row; dialogue / work_product are text.
        let tape_kind: String = conn
            .query_row("SELECT kind FROM modality WHERE slug = 'tape'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(tape_kind, "term");
        let (dialogue_kind, dialogue_tpl): (String, Option<String>) = conn
            .query_row(
                "SELECT kind, cover_template FROM modality WHERE slug = 'dialogue'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(dialogue_kind, "text");
        assert_eq!(dialogue_tpl.as_deref(), Some("dialogue"));

        // sort_order is dense 0..=10 in the design's table order.
        let orders: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT sort_order FROM modality ORDER BY sort_order")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(orders, (0..=10).collect::<Vec<_>>());
    }

    #[test]
    fn v28_v29_materialise_a_composite_asset_per_session_and_repoint_members() {
        // Seed the pre-session shape, let V26 mint the Session rows,
        // then V28/V29 promote each Session to a composite Asset and
        // repoint its members onto it via container_id.
        let mut conn = test_conn();
        migrate_to(&mut conn, 22).unwrap();
        let persona = seed_persona(&conn);
        seed_session_asset(&conn, persona, "dialogue", "cc.session.42", 100);
        seed_session_asset(&conn, persona, "dialogue", "cc.session.42", 200);
        seed_session_asset(&conn, persona, "dialogue", "cc.session.99", 300);

        // Version-scoped to V29: this test inspects the post-materialise
        // state that still has session_id + the session table (both
        // dropped by V31's contract rebuild).
        migrate_to(&mut conn, 29).unwrap();

        // V28 seeds the composition modality into the master.
        let session_kind: String = conn
            .query_row(
                "SELECT kind FROM modality WHERE slug = 'session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(session_kind, "composition");

        // One composite Asset (modality='session') per Session row.
        let composite_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset WHERE modality = 'session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(composite_count, 2, "one composite per dialogue Session");

        // The composite's id reuses session.id verbatim (parsed to
        // blob), and every member's container_id points at it.
        let sid_42: String = conn
            .query_row(
                "SELECT id FROM session WHERE external_key = 'cc.session.42'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let composite_blob = Uuid::parse_str(&sid_42).unwrap().as_bytes().to_vec();

        let (comp_modality, comp_key, comp_sid): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT modality, external_key, session_id FROM asset WHERE id = ?1",
                params![composite_blob],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(comp_modality, "session");
        assert_eq!(comp_key.as_deref(), Some("cc.session.42"));
        assert_eq!(
            comp_sid, None,
            "composite is top-level (no session_id of its own)"
        );

        let members_repointed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset WHERE session_id = ?1 AND container_id = ?2",
                params![sid_42, composite_blob],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            members_repointed, 2,
            "both members of cc.session.42 repointed onto the composite"
        );
    }

    #[test]
    fn v31_drops_session_scaffolding_and_installs_container_self_fk() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 22).unwrap();
        let persona = seed_persona(&conn);
        seed_session_asset(&conn, persona, "dialogue", "cc.session.fk", 100);

        migrate(&mut conn).unwrap(); // through V31

        // session table dropped.
        let session_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(session_tables, 0, "V31 drops the session table");

        // session_id column dropped; container_id retained.
        let has_session_col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('asset') WHERE name = 'session_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_session_col, 0, "V31 drops asset.session_id");
        let has_container_col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('asset') WHERE name = 'container_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_container_col, 1);

        // idx_asset_session gone; container index retained.
        let idx_session: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_asset_session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx_session, 0, "idx_asset_session dropped with the column");

        // Self-FK: deleting a composite SET NULLs its members' container_id.
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        // This test runs the full chain, so the composite is located by
        // its end-state marker (`role`, V37+) — the 'session' modality
        // slug it was minted with no longer exists after V38.
        let composite: Vec<u8> = conn
            .query_row(
                "SELECT id FROM asset WHERE role = 'collection' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let members_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset WHERE container_id = ?1",
                params![composite],
                |r| r.get(0),
            )
            .unwrap();
        assert!(members_before >= 1, "the seeded session has a member");
        conn.execute("DELETE FROM asset WHERE id = ?1", params![composite])
            .unwrap();
        let members_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset WHERE container_id = ?1",
                params![composite],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            members_after, 0,
            "ON DELETE SET NULL cleared the member's container_id"
        );
    }

    /// Seeds one asset carrying the given `session_id` on the given
    /// modality. `session_id` may be an arbitrary raw string (matches
    /// the pre-V26 shape). Returns the asset's UUID.
    fn seed_session_asset(
        conn: &Connection,
        persona: Uuid,
        modality: &str,
        session_id: &str,
        occurred_at: i64,
    ) -> Uuid {
        let asset = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, session_id, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', ?3, ?4, '[]', ?5, ?6, 0, 0)",
            params![
                asset,
                persona,
                format!("a-{asset}.md"),
                modality,
                occurred_at,
                session_id,
            ],
        )
        .unwrap();
        asset
    }

    #[test]
    fn v25_moves_non_dialogue_session_id_into_bundle_id() {
        // Seed the pre-migration shape (up to V22, before session /
        // bundle_id exists) then run the wave that adds bundle_id +
        // moves non-dialogue session_id values. After V25, every
        // non-dialogue asset has session_id = NULL and its old value
        // sitting in bundle_id; dialogue assets are untouched.
        let mut conn = test_conn();
        migrate_to(&mut conn, 22).unwrap();
        let persona = seed_persona(&conn);
        let tape = seed_session_asset(&conn, persona, "tape", "tape-2026-07-25_001", 10);
        let journal = seed_session_asset(&conn, persona, "state", "persona-journal/aya/state", 20);
        let dialogue = seed_session_asset(
            &conn,
            persona,
            "dialogue",
            "018f8e57-1234-7abc-9def-0123456789ab",
            30,
        );

        migrate_to(&mut conn, 25).unwrap();

        let (t_sid, t_bid): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT session_id, bundle_id FROM asset WHERE id = ?1",
                params![tape],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(t_sid, None, "non-dialogue session_id must be cleared");
        assert_eq!(
            t_bid.as_deref(),
            Some("tape-2026-07-25_001"),
            "old session_id moved into bundle_id"
        );

        let (j_sid, j_bid): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT session_id, bundle_id FROM asset WHERE id = ?1",
                params![journal],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(j_sid, None);
        assert_eq!(j_bid.as_deref(), Some("persona-journal/aya/state"));

        let (d_sid, d_bid): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT session_id, bundle_id FROM asset WHERE id = ?1",
                params![dialogue],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            d_sid.as_deref(),
            Some("018f8e57-1234-7abc-9def-0123456789ab"),
            "dialogue session_id must survive V25"
        );
        assert_eq!(d_bid, None);
    }

    #[test]
    fn v26_mints_one_session_per_persona_external_key_and_rewrites_asset_session_id() {
        // Two dialogue assets share the same session_id, plus a
        // third asset on a different external key. After V26 two
        // Session rows exist and every asset's session_id is the
        // corresponding Session.id (a UUID).
        let mut conn = test_conn();
        migrate_to(&mut conn, 22).unwrap();
        let persona = seed_persona(&conn);
        let a1 = seed_session_asset(&conn, persona, "dialogue", "cc.session.42", 100);
        let a2 = seed_session_asset(&conn, persona, "dialogue", "cc.session.42", 200);
        let a3 = seed_session_asset(&conn, persona, "dialogue", "cc.session.99", 300);
        // Non-dialogue asset with same session_id keyspace to prove
        // it does not confuse the aggregation.
        let _tape = seed_session_asset(&conn, persona, "tape", "cc.session.42", 400);

        migrate_to(&mut conn, 26).unwrap();

        // Session rows: one per (persona, external_key) grouping the
        // dialogue assets carried.
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            session_count, 2,
            "one Session per unique dialogue external_key"
        );

        // Each Session's aggregates come from the participating asset
        // occurrence times.
        let (sid_42, min_42, max_42, count_42): (String, i64, i64, i64) = conn
            .query_row(
                "SELECT id, started_at_ms, ended_at_ms, message_count \
                 FROM session WHERE external_key = 'cc.session.42'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(min_42, 100);
        assert_eq!(max_42, 200);
        assert_eq!(count_42, 2);

        // Every dialogue asset's session_id now equals the Session.id
        // (Uuid string), no longer the raw external key.
        let a1_sid: Option<String> = conn
            .query_row(
                "SELECT session_id FROM asset WHERE id = ?1",
                params![a1],
                |r| r.get(0),
            )
            .unwrap();
        let a2_sid: Option<String> = conn
            .query_row(
                "SELECT session_id FROM asset WHERE id = ?1",
                params![a2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(a1_sid.as_deref(), Some(sid_42.as_str()));
        assert_eq!(a2_sid.as_deref(), Some(sid_42.as_str()));

        let sid_99: String = conn
            .query_row(
                "SELECT id FROM session WHERE external_key = 'cc.session.99'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(sid_99, sid_42, "distinct external_keys → distinct sessions");
        let a3_sid: Option<String> = conn
            .query_row(
                "SELECT session_id FROM asset WHERE id = ?1",
                params![a3],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(a3_sid.as_deref(), Some(sid_99.as_str()));

        // Post-V26 session_ids are all UUID-shaped strings.
        assert!(
            Uuid::parse_str(&sid_42).is_ok(),
            "session.id should be a hyphenated UUID"
        );
    }

    #[test]
    fn v27_check_rejects_non_dialogue_asset_with_session_id() {
        // After V27 the CHECK guards `session_id IS NULL OR modality
        // = 'dialogue'`. An INSERT that tries to carry a session_id
        // on a non-dialogue asset must be rejected.
        let mut conn = test_conn();
        // Version-scoped to V27: V31 drops the session_id column + this
        // CHECK, so this test stops at V27 to exercise the CHECK it is about.
        migrate_to(&mut conn, 27).unwrap();
        let persona = seed_persona(&conn);
        // First mint a valid Session so the FK is not the reason we
        // reject (we want to prove the CHECK, not the FK).
        let session_id = Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO session (id, persona_id, external_key, started_at_ms, ended_at_ms, \
                                  message_count, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, 'cc.session.check', 0, 0, 0, 0, 0)",
            params![session_id, persona],
        )
        .unwrap();
        let asset = Uuid::now_v7();
        let err = conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, occurred_at, session_id, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', ?3, 'tape', 0, ?4, 0, 0)",
            params![asset, persona, format!("a-{asset}.md"), session_id],
        );
        assert!(
            err.is_err(),
            "V27 CHECK must reject non-dialogue asset with session_id"
        );

        // Dialogue side: same INSERT succeeds.
        let dialogue_asset = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, occurred_at, session_id, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', ?3, 'dialogue', 0, ?4, 0, 0)",
            params![
                dialogue_asset,
                persona,
                format!("a-{dialogue_asset}.md"),
                session_id,
            ],
        )
        .unwrap();
    }

    #[test]
    fn v27_recreates_every_asset_index_and_preserves_data() {
        // The table rebuild must not silently drop indexes (V1 / V2 /
        // V13 / V17 / V24). Verify each expected name is present in
        // sqlite_master after `migrate` completes.
        let mut conn = test_conn();
        // Version-scoped to V27: V31 drops idx_asset_session (the
        // session_id column is gone), so this V27-index test stops at V27.
        migrate_to(&mut conn, 27).unwrap();
        let expected_indexes = [
            "idx_asset_persona_occurred",
            "idx_asset_persona_modality_occurred",
            "idx_asset_occurred",
            "idx_asset_session",
            "idx_asset_source_unique",
            "idx_asset_persona_rating",
            "idx_asset_persona_occurred_cover",
            "idx_asset_occurred_cover",
            "idx_asset_bundle",
        ];
        let names: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                     WHERE type = 'index' AND tbl_name = 'asset' \
                     ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        for expected in &expected_indexes {
            assert!(
                names.iter().any(|n| n == expected),
                "expected asset index {expected} to survive V27 rebuild (got {names:?})"
            );
        }

        // Column shape + data survive: seed a pre-V27 row and check
        // it round-trips through the rebuild.
        let mut conn = test_conn();
        migrate_to(&mut conn, 24).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_session_asset(&conn, persona, "tape", "tape-x", 42);
        conn.execute(
            "UPDATE asset SET bundle_id = 'tape-x', session_id = NULL WHERE id = ?1",
            params![asset],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        let (modality, occurred, bundle): (String, i64, Option<String>) = conn
            .query_row(
                "SELECT modality, occurred_at, bundle_id FROM asset WHERE id = ?1",
                params![asset],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(modality, "tape");
        assert_eq!(occurred, 42);
        assert_eq!(bundle.as_deref(), Some("tape-x"));
    }

    /// V36 carries existing observation rows into their new homes
    /// rather than dropping them, and splits the old `diag_log` along
    /// the seam that was missing: perf timings are their own stream.
    #[test]
    fn v36_moves_existing_rows_into_the_right_streams() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 35).unwrap();

        let persona = seed_persona(&conn);
        conn.execute(
            "INSERT INTO event_log (id, kind, occurred_at, persona_id, duration_ms, payload)
             VALUES (?1, 'persona_switch', 100, ?2, 42, '{\"n\":1}')",
            params![Uuid::new_v4(), persona],
        )
        .unwrap();
        // The two shapes the old call sites actually wrote: each named
        // its timing after its own phase, so neither field is the one
        // the new column is called.
        conn.execute(
            "INSERT INTO diag_log (id, occurred_at, level, target, message, fields)
             VALUES (?1, 200, 'INFO', 'asterism_infra::sqlite::repo::asset',
                     'perf: list_index database phase',
                     '{\"op\":\"list_index\",\"query_ms\":9,\"db_total_ms\":18}')",
            params![Uuid::new_v4()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO diag_log (id, occurred_at, level, target, message, fields)
             VALUES (?1, 250, 'INFO', 'asterism_infra::sqlite::repo::asset',
                     'perf: list_index domain mapping',
                     '{\"op\":\"list_index\",\"domain_map_ms\":4}')",
            params![Uuid::new_v4()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO diag_log (id, occurred_at, level, target, message, fields)
             VALUES (?1, 300, 'WARN', 'asterism_core::application', 'something was skipped', NULL)",
            params![Uuid::new_v4()],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        // The action keeps its persona and duration, and its `kind`
        // becomes a namespaced event.
        let (event, duration): (String, i64) = conn
            .query_row("SELECT event, duration_ms FROM action_log", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(event, "action.persona_switch");
        assert_eq!(duration, 42);

        // The perf record went to its own stream, the warning did not,
        // and the timing the old call site wrote under a phase-specific
        // name landed in the column that now holds it. Zero here would
        // read as "instant" and poison every average taken afterwards.
        let timings: Vec<(String, i64)> = {
            let mut stmt = conn
                .prepare("SELECT op, duration_ms FROM perf_log ORDER BY occurred_at")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            timings,
            vec![("list_index".into(), 18), ("list_index".into(), 4)]
        );

        let diag_levels: Vec<String> = {
            let mut stmt = conn.prepare("SELECT level FROM diag_log").unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(diag_levels, vec!["WARN"]);

        // The old tables are gone, and the union view spans the four.
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('event_log', 'diag_log_v2')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);
        let streams: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT DISTINCT stream FROM observation ORDER BY stream")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(streams, vec!["action", "diag", "perf"]);
    }

    /// Tag rows must not outlive the record they classify — otherwise a
    /// retention delete on a stream would leave orphans behind.
    ///
    /// `diag_log_tag` is the case worth pinning: its table is created
    /// against `diag_log_v2` and only becomes `diag_log` through
    /// `ALTER TABLE … RENAME`, so this asserts that SQLite carried the
    /// foreign key across the rename rather than leaving it pointing at
    /// a name that no longer exists.
    #[test]
    fn v36_tags_cascade_with_their_record_across_the_rename() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();

        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO diag_log (id, occurred_at, env, event, level, target, message)
             VALUES (?1, 100, 'dev', 'diag.probe', 'WARN', 'asterism_core', 'text')",
            params![id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO diag_log_tag (record_id, tag) VALUES (?1, 'startup')",
            params![id],
        )
        .unwrap();

        conn.execute("DELETE FROM diag_log WHERE id = ?1", params![id])
            .unwrap();
        let orphans: i64 = conn
            .query_row("SELECT COUNT(*) FROM diag_log_tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0);

        // And the whole graph is consistent — a rename that had broken
        // a reference would surface here rather than at a later delete.
        let mut stmt = conn.prepare("PRAGMA foreign_key_check").unwrap();
        assert_eq!(stmt.query_map([], |_| Ok(())).unwrap().count(), 0);
    }

    #[test]
    fn fresh_chain_reaches_latest_with_snapshot_schema() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
        let has_snapshot: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'snapshot'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_snapshot, 1);
        // Migrating again is a no-op.
        migrate(&mut conn).unwrap();
    }

    /// V37 captures the structural fact (`role`) and the physical layer
    /// (`material`) while the conversation slugs still exist — P2
    /// deletes the slugs, so this is the last version at which the
    /// backfill can read them.
    #[test]
    fn v37_backfills_role_and_one_material_per_item() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 36).unwrap();

        let persona = seed_persona(&conn);
        // `seed_asset` = modality 'dialogue' (master kind 'text').
        let message = seed_asset(&conn, persona);
        let image = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                file_size_bytes, modality, labels, occurred_at, \
                                created_at, updated_at) \
             VALUES (?1, ?2, 'fs', '/pics/Star.PNG', 2048, 'image', '[]', 0, 0, 0)",
            params![image, persona],
        )
        .unwrap();
        // A slug with no master row (the importer escape hatch): the
        // backfill must not lose the row, only leave mime unknown.
        let orphan_slug = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', 'opaque-locator', 'imported_thing', '[]', 0, 0, 0)",
            params![orphan_slug, persona],
        )
        .unwrap();
        let session = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'session', ?3, 'session', '[]', 0, 0, 0)",
            params![session, persona, session.to_string()],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        // Structural fact captured from the slug before P2 deletes it.
        let role: String = conn
            .query_row(
                "SELECT role FROM asset WHERE id = ?1",
                params![session],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(role, "collection");
        let item_roles: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset WHERE role = 'item'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(item_roles, 3);

        // One material per item, none for the collection (a container
        // has no bytes of its own).
        let materials: i64 = conn
            .query_row("SELECT COUNT(*) FROM material", [], |r| r.get(0))
            .unwrap();
        assert_eq!(materials, 3);
        let collection_materials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM material WHERE asset_id = ?1",
                params![session],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(collection_materials, 0);

        // mime is fact capture: master kind wins for text, the file
        // extension for media (case-insensitive), unknown stays NULL.
        let (loc, size, mime): (String, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT locator, file_size_bytes, mime FROM material WHERE asset_id = ?1",
                params![image],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        // V37 copies the locator across verbatim; what the column holds
        // by the time the chain finishes is the tagged form, because
        // V63 rewrote both columns after this step ran. The two facts
        // are asserted together on purpose — V37's own contract is
        // "copied, not derived", and it still holds through the rewrite.
        assert_eq!(loc, r#"{"kind":"file","path":"/pics/Star.PNG"}"#);
        assert_eq!(size, Some(2048));
        assert_eq!(mime.as_deref(), Some("image/png"));
        let message_mime: Option<String> = conn
            .query_row(
                "SELECT mime FROM material WHERE asset_id = ?1",
                params![message],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(message_mime.as_deref(), Some("text/plain"));
        let orphan_mime: Option<String> = conn
            .query_row(
                "SELECT mime FROM material WHERE asset_id = ?1",
                params![orphan_slug],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphan_mime, None);

        // Copied, not moved — the asset row still carries the source
        // columns the grid covering indexes serve (read-side switchover
        // is P3 scope).
        let kept: String = conn
            .query_row(
                "SELECT source_locator FROM asset WHERE id = ?1",
                params![image],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, r#"{"kind":"file","path":"/pics/Star.PNG"}"#);
        assert_eq!(kept, loc, "the copy still says what the original says");
    }

    /// V38 is the slug removal: conversation / format rows leave the
    /// master, their assets drop to `modality = NULL`, and the
    /// structural fact keeps living on `role` (captured by V37 while
    /// the slug still existed).
    #[test]
    fn v38_drops_conversation_and_format_slugs_to_null() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 36).unwrap();

        let persona = seed_persona(&conn);
        // `seed_asset` = modality 'dialogue'.
        let message = seed_asset(&conn, persona);
        let session = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'session', ?3, 'session', '[]', 0, 0, 0)",
            params![session, persona, session.to_string()],
        )
        .unwrap();
        let tape = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', 'tapes/one.txt', 'tape', '[]', 0, 0, 0)",
            params![tape, persona],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        // Conversation assets are unclassified now; their structure
        // survives on `role`.
        let (m_modality, m_role): (Option<String>, String) = conn
            .query_row(
                "SELECT modality, role FROM asset WHERE id = ?1",
                params![message],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(m_modality, None);
        assert_eq!(m_role, "item");
        let (s_modality, s_role): (Option<String>, String) = conn
            .query_row(
                "SELECT modality, role FROM asset WHERE id = ?1",
                params![session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        // The container's structure is `role`; V38 emptied its modality
        // because the old `session` slug encoded structure. V42 filled
        // the semantic axis back in with a slug that answers a different
        // question — what the container holds.
        assert_eq!(s_modality.as_deref(), Some("session"));
        assert_eq!(s_role, "collection");

        // Semantic classification is untouched.
        let t_modality: Option<String> = conn
            .query_row(
                "SELECT modality FROM asset WHERE id = ?1",
                params![tape],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t_modality.as_deref(), Some("tape"));

        // The master keeps only the semantic rows.
        let gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM modality WHERE slug IN \
                 ('dialogue', 'image', 'video', 'audio', 'test_mod')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0);
        // 7 semantic rows survive V38, plus the conversation pair the
        // later migrations restore on the semantic axis: `session`
        // (V42, what a container holds) and `message` (V43, what a
        // member is).
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM modality", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 9);

        // NOT NULL is gone: an unclassified insert succeeds.
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', 'unclassified.md', NULL, '[]', 0, 0, 0)",
            params![Uuid::now_v7(), persona],
        )
        .unwrap();
    }

    #[test]
    fn v39_drops_the_show_messages_override_row() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 38).unwrap();
        conn.execute(
            "INSERT INTO app_setting (key, value_json, updated_at) \
             VALUES ('ui.dialogue.show_messages', 'true', 0)",
            [],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM app_setting \
                 WHERE key = 'ui.dialogue.show_messages'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0, "the orphaned override row is gone");
    }

    #[test]
    fn v37_materials_cascade_with_their_asset() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, created_at, updated_at) \
             VALUES (?1, 0, 'a.md', 0, 0)",
            params![asset],
        )
        .unwrap();

        conn.execute("DELETE FROM asset WHERE id = ?1", params![asset])
            .unwrap();
        let orphans: i64 = conn
            .query_row("SELECT COUNT(*) FROM material", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0);
        let mut stmt = conn.prepare("PRAGMA foreign_key_check").unwrap();
        assert_eq!(stmt.query_map([], |_| Ok(())).unwrap().count(), 0);
    }

    /// The load-bearing claim of V30: while an asset sits in the trash
    /// **every dependent row survives**, so restore needs no replay. If
    /// this test ever fails, the trash has stopped being reversible and
    /// the whole verb split loses its point.
    #[test]
    fn v30_trashed_asset_keeps_its_children_and_purge_takes_them() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);

        // Two representative children: a comment (pure Asterism-side
        // value) and a group filing carrying a hand-arranged position
        // (the most expensive thing to reproduce by hand).
        let group = Uuid::now_v7();
        conn.execute(
            "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'Keepers', 0, 0)",
            params![group, persona],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO asset_bucket (asset_id, bucket_id, added_at, position) \
             VALUES (?1, ?2, 0, 7)",
            params![asset, group],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO asset_comment \
                 (id, asset_id, author_kind, author_persona_id, body, created_at) \
             VALUES (?1, ?2, 'user', NULL, 'keep this', 0)",
            params![Uuid::now_v7(), asset],
        )
        .unwrap();

        let children = |conn: &Connection| -> (i64, i64, i64) {
            let filings: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM asset_bucket WHERE asset_id = ?1",
                    params![asset],
                    |r| r.get(0),
                )
                .unwrap();
            let comments: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM asset_comment WHERE asset_id = ?1",
                    params![asset],
                    |r| r.get(0),
                )
                .unwrap();
            let position: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(position), -1) FROM asset_bucket WHERE asset_id = ?1",
                    params![asset],
                    |r| r.get(0),
                )
                .unwrap();
            (filings, comments, position)
        };

        assert_eq!(children(&conn), (1, 1, 7), "seed state");

        // Trash: a stamp, nothing more.
        conn.execute(
            "UPDATE asset SET trashed_at = 1000 WHERE id = ?1",
            params![asset],
        )
        .unwrap();
        assert_eq!(
            children(&conn),
            (1, 1, 7),
            "trashing must not disturb a single dependent row"
        );

        // Restore: the stamp clears and nothing had to be rebuilt.
        conn.execute(
            "UPDATE asset SET trashed_at = NULL WHERE id = ?1",
            params![asset],
        )
        .unwrap();
        let stamp: Option<i64> = conn
            .query_row(
                "SELECT trashed_at FROM asset WHERE id = ?1",
                params![asset],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stamp.is_none(), "restore clears the stamp");
        assert_eq!(children(&conn), (1, 1, 7), "restore replays nothing");

        // Purge: now the cascade is allowed to do its work.
        conn.execute("DELETE FROM asset WHERE id = ?1", params![asset])
            .unwrap();
        assert_eq!(
            children(&conn),
            (0, 0, -1),
            "purge takes the children with it"
        );
        // The Group itself outlives its member — filing is m:n, and the
        // bucket is a separate entity with its own trash stamp.
        let groups: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bucket WHERE id = ?1",
                params![group],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(groups, 1);
    }

    /// `purge` carries its guard in the DELETE predicate, so the
    /// statement is inert against a live row no matter what happened
    /// between the caller's decision and the write. Exercised at the SQL
    /// level because that is where the guarantee lives — the two-process
    /// deployment (UI + server share the file) means a preceding SELECT
    /// could not provide it.
    #[test]
    fn v30_purge_predicate_refuses_live_rows_and_takes_trashed_ones() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);
        let live = seed_asset(&conn, persona);
        let trashed = seed_asset(&conn, persona);
        conn.execute(
            "UPDATE asset SET trashed_at = 1000 WHERE id = ?1",
            params![trashed],
        )
        .unwrap();

        let purge = |conn: &Connection, id: Uuid| -> usize {
            conn.execute(
                "DELETE FROM asset WHERE id = ?1 AND trashed_at IS NOT NULL",
                params![id],
            )
            .unwrap()
        };

        assert_eq!(purge(&conn, live), 0, "a live row is never purged");
        assert_eq!(purge(&conn, trashed), 1, "a trashed row is purged");
        assert_eq!(
            purge(&conn, trashed),
            0,
            "purging twice is a no-op, not an error"
        );

        let survivors: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))
            .unwrap();
        assert_eq!(survivors, 1, "only the live asset remains");
    }

    /// Sidebar counts must not advertise assets the grid will not show.
    /// Tag and Group counts are the subtle pair: their join tables
    /// (`asset_tag` / `asset_bucket`) deliberately survive trashing, so
    /// counting the join alone silently over-reports.
    #[test]
    fn v30_sidebar_counts_exclude_trashed_assets() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);
        let kept = seed_asset(&conn, persona);
        let gone = seed_asset(&conn, persona);

        let tag = Uuid::now_v7();
        conn.execute(
            "INSERT INTO tag (id, name, axis) VALUES (?1, 'topic', 'channel')",
            params![tag],
        )
        .unwrap();
        let group = Uuid::now_v7();
        conn.execute(
            "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'G', 0, 0)",
            params![group, persona],
        )
        .unwrap();
        for asset in [kept, gone] {
            conn.execute(
                "INSERT INTO asset_tag (asset_id, tag_id) VALUES (?1, ?2)",
                params![asset, tag],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO asset_bucket (asset_id, bucket_id, added_at, position) \
                 VALUES (?1, ?2, 0, 0)",
                params![asset, group],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE asset SET trashed_at = 1000 WHERE id = ?1",
            params![gone],
        )
        .unwrap();

        // Both join tables still hold two rows — that is the point of
        // the design, and the reason the counts need the asset join.
        let filings: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset_bucket", [], |r| r.get(0))
            .unwrap();
        assert_eq!(filings, 2, "trashing keeps the group filing");

        let tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT asset.id) FROM tag \
                 JOIN asset_tag ON asset_tag.tag_id = tag.id \
                 JOIN asset ON asset.id = asset_tag.asset_id \
                 WHERE asset.trashed_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tag_count, 1, "tag chip counts live assets only");

        let group_count: i64 = conn
            .query_row(
                "SELECT COUNT(asset.id) FROM bucket \
                 LEFT JOIN asset_bucket ON asset_bucket.bucket_id = bucket.id \
                 LEFT JOIN asset ON asset.id = asset_bucket.asset_id \
                                AND asset.trashed_at IS NULL \
                 WHERE bucket.id = ?1 \
                 GROUP BY bucket.id",
                params![group],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(group_count, 1, "group card counts live members only");
    }

    /// The persona trash exists to stop `DELETE FROM persona` from being
    /// an unrecoverable cascade over every asset that persona held. The
    /// load-bearing property is the **shared stamp**: restoring a
    /// persona must bring back exactly the assets that went down with
    /// it, and leave the ones the user had thrown away by hand.
    #[test]
    fn v31_persona_trash_shares_a_stamp_so_restore_is_exact() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);
        let with_persona = seed_asset(&conn, persona);
        let thrown_away = seed_asset(&conn, persona);

        // The user trashes one asset by hand, earlier and separately.
        conn.execute(
            "UPDATE asset SET trashed_at = 500 WHERE id = ?1",
            params![thrown_away],
        )
        .unwrap();

        // Now the persona goes to the trash, stamping its live assets
        // with its own timestamp.
        let stamp = 1_000i64;
        conn.execute(
            "UPDATE asset SET trashed_at = ?1 WHERE persona_id = ?2 AND trashed_at IS NULL",
            params![stamp, persona],
        )
        .unwrap();
        conn.execute(
            "UPDATE persona SET trashed_at = ?1 WHERE id = ?2",
            params![stamp, persona],
        )
        .unwrap();

        let stamp_of = |conn: &Connection, id: Uuid| -> Option<i64> {
            conn.query_row(
                "SELECT trashed_at FROM asset WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(stamp_of(&conn, with_persona), Some(stamp));
        assert_eq!(
            stamp_of(&conn, thrown_away),
            Some(500),
            "the hand-trashed asset keeps its own earlier stamp"
        );

        // Restore matches on the persona's stamp only.
        conn.execute(
            "UPDATE asset SET trashed_at = NULL WHERE persona_id = ?1 AND trashed_at = ?2",
            params![persona, stamp],
        )
        .unwrap();
        conn.execute(
            "UPDATE persona SET trashed_at = NULL WHERE id = ?1",
            params![persona],
        )
        .unwrap();

        assert_eq!(
            stamp_of(&conn, with_persona),
            None,
            "the asset that went down with the persona comes back"
        );
        assert_eq!(
            stamp_of(&conn, thrown_away),
            Some(500),
            "…and the one the user threw away stays thrown away"
        );

        // Nothing was destroyed along the way.
        let alive: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))
            .unwrap();
        assert_eq!(alive, 2);

        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name = 'idx_persona_trashed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
    }

    // The "a trashed persona's sessions leave the listing" case lives in
    // `repo::asset::tests::list_sessions_hides_a_trashed_personas_sessions`,
    // where it can drive the production query. A version here would have
    // to re-state the predicate, and would then keep passing if the real
    // query ever lost it.

    /// Both trash columns exist and default to live, and the partial
    /// indexes that serve the trash view / retention sweep are in place.
    #[test]
    fn v30_adds_trash_columns_defaulting_to_live() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let asset_stamp: Option<i64> = conn
            .query_row(
                "SELECT trashed_at FROM asset WHERE id = ?1",
                params![asset],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            asset_stamp.is_none(),
            "an asset inserted without naming trashed_at is live"
        );

        let group = Uuid::now_v7();
        conn.execute(
            "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'G', 0, 0)",
            params![group, persona],
        )
        .unwrap();
        let group_stamp: Option<i64> = conn
            .query_row(
                "SELECT trashed_at FROM bucket WHERE id = ?1",
                params![group],
                |r| r.get(0),
            )
            .unwrap();
        assert!(group_stamp.is_none(), "a new Group is live");

        for index in ["idx_asset_trashed", "idx_bucket_trashed"] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    params![index],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "{index} must exist");
        }
    }

    /// V40 derives the colour facet from the palettes already on disk.
    /// The upgrade has to survive the two shapes a real database
    /// carries besides well-formed arrays — a corrupt blob and no
    /// palette at all — because a schema step that aborts on one bad
    /// row leaves the whole database unmigrated.
    #[test]
    fn v40_backfills_buckets_from_existing_palettes() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 39).unwrap();

        let persona = seed_persona(&conn);
        let two_colours = seed_asset(&conn, persona);
        let duplicate_hues = seed_asset(&conn, persona);
        let corrupt = seed_asset(&conn, persona);
        let no_palette = seed_asset(&conn, persona);

        let set_palette = |id: Uuid, raw: &str| {
            conn.execute(
                "UPDATE asset SET palette = ?1 WHERE id = ?2",
                params![raw, id],
            )
            .unwrap();
        };
        // `r##` because the payload itself contains `"#`.
        set_palette(two_colours, r##"["#ff0000","#ffffff"]"##);
        // Three reds → one Red row: the facet counts assets.
        set_palette(duplicate_hues, r##"["#ff0000","#e03131","#cc1111"]"##);
        set_palette(corrupt, "not json at all");

        migrate_to(&mut conn, 40).unwrap();

        let buckets_of_asset = |id: Uuid| -> Vec<String> {
            let mut stmt = conn
                .prepare("SELECT bucket FROM asset_color WHERE asset_id = ?1 ORDER BY bucket")
                .unwrap();
            stmt.query_map(params![id], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(buckets_of_asset(two_colours), vec!["red", "white"]);
        assert_eq!(buckets_of_asset(duplicate_hues), vec!["red"]);
        assert!(
            buckets_of_asset(corrupt).is_empty(),
            "an unparseable palette yields no swatch, and does not abort the batch"
        );
        assert!(buckets_of_asset(no_palette).is_empty());

        // The projection follows its asset out of the database.
        conn.execute("DELETE FROM asset WHERE id = ?1", params![two_colours])
            .unwrap();
        assert!(
            buckets_of_asset(two_colours).is_empty(),
            "asset_color rows cascade with the asset"
        );
    }

    /// V45 repairs what `guess_mime` used to answer through a
    /// fragment. The row it exists for is the PNG tEXt note: text
    /// living inside an image, filed `image/png`, which aimed the
    /// thumbnail job at a locator no filesystem can open.
    #[test]
    fn v45_reclassifies_fragment_locators_as_text() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 44).unwrap();

        let persona = seed_persona(&conn);
        let seed_material = |locator: &str, mime: Option<&str>| -> Uuid {
            let id = Uuid::now_v7();
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, '[]', 0, 0, 0)",
                params![id, persona, locator],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at) \
                 VALUES (?1, 0, ?2, ?3, 0, 0)",
                params![id, locator, mime],
            )
            .unwrap();
            id
        };
        let png_note = seed_material("/pics/shot.png#workflow", Some("image/png"));
        let already_text = seed_material("/logs/session.jsonl#msg-1", Some("text/plain"));
        let real_image = seed_material("/pics/shot.png", Some("image/png"));
        let unknown = seed_material("/pics/thing.xyz#part", None);

        migrate(&mut conn).unwrap();

        let mime_of = |id: Uuid| -> Option<String> {
            conn.query_row(
                "SELECT mime FROM material WHERE asset_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(mime_of(png_note).as_deref(), Some("text/plain"));
        assert_eq!(mime_of(already_text).as_deref(), Some("text/plain"));
        assert_eq!(
            mime_of(real_image).as_deref(),
            Some("image/png"),
            "a locator without a fragment is the container itself, and keeps its format"
        );
        assert_eq!(
            mime_of(unknown),
            None,
            "the repair corrects wrong answers, it does not invent missing ones"
        );
    }

    /// V47 opens the attribution columns without answering for anybody:
    /// a row that predates the migration stays NULL on all three, which
    /// reads as *unrecorded* rather than "authored by the owner".
    #[test]
    fn v47_adds_attribution_columns_left_unrecorded() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 46).unwrap();

        let persona = seed_persona(&conn);
        let legacy = seed_asset(&conn, persona);

        migrate(&mut conn).unwrap();

        let attribution_of = |id: Uuid| -> (Option<String>, Option<String>, Option<String>) {
            conn.query_row(
                "SELECT author_kind, author_subject, operator_ai FROM asset WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(attribution_of(legacy), (None, None, None));

        // The pair is writable in both of its valid shapes; the
        // half-shapes are the domain's to refuse (`Author::from_columns`),
        // because `ALTER TABLE ADD COLUMN` cannot carry a table-level
        // CHECK — see the step doc.
        conn.execute(
            "UPDATE asset SET author_kind = 'subject', author_subject = 'alice', \
             operator_ai = 'claude-code' WHERE id = ?1",
            params![legacy],
        )
        .unwrap();
        assert_eq!(
            attribution_of(legacy),
            (
                Some("subject".to_string()),
                Some("alice".to_string()),
                Some("claude-code".to_string())
            )
        );
    }

    /// V51 opens the fold axis: a row that predates it is live
    /// (`folded_into IS NULL`) and unruled (`fold_policy = 'auto'`),
    /// which is what every row was before anybody could fold anything.
    ///
    /// The second half is the measurement the step doc cites: a
    /// **column-level** CHECK does survive `ALTER TABLE ADD COLUMN`,
    /// unlike the table-level one V47 could not have. Run it rather
    /// than reason about it — the answer decides whether the closed set
    /// has one enforcer or two.
    #[test]
    fn v51_folds_are_marked_and_the_policy_is_checked() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 50).unwrap();

        let persona = seed_persona(&conn);
        let legacy = seed_asset(&conn, persona);

        migrate(&mut conn).unwrap();

        fn fold_of(conn: &Connection, id: Uuid) -> (Option<Uuid>, String) {
            conn.query_row(
                "SELECT folded_into, fold_policy FROM asset WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        }
        assert_eq!(fold_of(&conn, legacy), (None, "auto".to_string()));

        // Re-running the whole chain against an up-to-date database is
        // a no-op rather than a duplicate-column error.
        migrate(&mut conn).unwrap();
        assert_eq!(fold_of(&conn, legacy), (None, "auto".to_string()));

        // A headstone is a plain id write, and the id round-trips as
        // the BLOB every other asset reference is — the reason this
        // column is not TEXT.
        let keeper = seed_asset(&conn, persona);
        conn.execute(
            "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
            params![legacy, keeper],
        )
        .unwrap();
        assert_eq!(fold_of(&conn, legacy), (Some(keeper), "auto".to_string()));

        conn.execute(
            "UPDATE asset SET fold_policy = 'keep' WHERE id = ?1",
            params![keeper],
        )
        .unwrap();
        assert_eq!(fold_of(&conn, keeper).1, "keep");

        let rejected = conn.execute(
            "UPDATE asset SET fold_policy = 'maybe' WHERE id = ?1",
            params![keeper],
        );
        let err = rejected.expect_err(
            "the column-level CHECK survived ADD COLUMN, so SQLite refuses an unknown policy",
        );
        assert!(
            err.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {err}"
        );
        assert_eq!(fold_of(&conn, keeper).1, "keep");
    }

    #[test]
    fn v52_a_declared_strategy_is_checked_and_absence_is_allowed() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 51).unwrap();

        let persona = seed_persona(&conn);
        let legacy = seed_asset(&conn, persona);

        migrate(&mut conn).unwrap();

        fn strategy_of(conn: &Connection, id: Uuid) -> Option<String> {
            conn.query_row(
                "SELECT on_duplicate FROM asset WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
        }

        // A row that predates the column declared nothing, and the
        // migration does not decide on its behalf. `'ask'` here would be
        // a request nobody made.
        assert_eq!(
            strategy_of(&conn, legacy),
            None,
            "the added column starts unrecorded, not at the default the detector applies"
        );

        // Re-running the chain against an up-to-date database is a
        // no-op rather than a duplicate-column error.
        migrate(&mut conn).unwrap();
        assert_eq!(strategy_of(&conn, legacy), None);

        for declared in ["ask", "fold", "separate"] {
            conn.execute(
                "UPDATE asset SET on_duplicate = ?2 WHERE id = ?1",
                params![legacy, declared],
            )
            .unwrap_or_else(|e| panic!("{declared:?} is in the closed set: {e}"));
            assert_eq!(strategy_of(&conn, legacy).as_deref(), Some(declared));
        }

        // The CHECK survived ADD COLUMN, so the database is the first
        // reader of the closed set and `OnDuplicate::parse` the second.
        let err = conn
            .execute(
                "UPDATE asset SET on_duplicate = 'maybe' WHERE id = ?1",
                params![legacy],
            )
            .expect_err("an unknown strategy is refused by the database itself");
        assert!(
            err.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {err}"
        );
        assert_eq!(
            strategy_of(&conn, legacy).as_deref(),
            Some("separate"),
            "the refused write left the declared value alone"
        );

        // …and the same CHECK still admits absence: `NULL IN (…)` is
        // NULL, which is not false, so it passes without being named.
        conn.execute(
            "UPDATE asset SET on_duplicate = NULL WHERE id = ?1",
            params![legacy],
        )
        .expect("absence is allowed by a CHECK that never mentions it");
        assert_eq!(strategy_of(&conn, legacy), None);
    }

    /// V53's queue holds one row per pair per axis, whichever end raised
    /// it, and refuses the shapes that would make the queue lie: a
    /// half-set resolution, an axis nobody computes, a row against
    /// itself.
    #[test]
    fn v53_one_pair_queues_once_and_the_answer_is_all_or_nothing() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let older = seed_asset(&conn, persona);
        let newer = seed_asset(&conn, persona);
        // `seed_asset` mints UUID v7 in call order, so the sort is
        // known — but the point is that the *caller* sorts, and the
        // column pair is what the index sees.
        let (lo, hi) = if older <= newer {
            (older, newer)
        } else {
            (newer, older)
        };

        let raise = |newcomer: Uuid, incumbent: Uuid, axis: &str| {
            conn.execute(
                "INSERT INTO duplicate_conflict
                     (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                      axis, content_hash, detected_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'sha256:abc', 1)",
                params![Uuid::now_v7(), persona, lo, hi, newcomer, incumbent, axis],
            )
        };

        raise(newer, older, "artefact").expect("the first detection queues the pair");

        // The same pair detected from the other end — what the backfill
        // walk produces if it reaches the rows in the opposite order. It
        // is the same question, so it does not queue twice.
        let again = raise(older, newer, "artefact")
            .expect_err("the unordered pair is the key, so the mirror event is refused");
        assert!(
            again.to_string().contains("UNIQUE constraint failed"),
            "unexpected rejection: {again}"
        );

        // A different axis is a different question about the same pair.
        raise(newer, older, "content").expect("the axis is part of the key");

        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM duplicate_conflict WHERE resolved_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 2, "two axes, one pair, two open questions");

        // An axis nothing computes is refused rather than stored as a
        // word the panel would have to guess at.
        let bad_axis = raise(newer, older, "vibes").expect_err("the axis set is closed");
        assert!(
            bad_axis.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {bad_axis}"
        );

        // Answering is all-or-nothing: a stamp with no verdict, or a
        // verdict with no stamp, would leave the queue unable to say
        // whether the question is open.
        for (stamp, verdict) in [("2", "NULL"), ("NULL", "'kept'")] {
            let half = conn
                .execute(
                    &format!(
                        "UPDATE duplicate_conflict SET resolved_at = {stamp}, \
                         resolution = {verdict} WHERE axis = 'artefact'"
                    ),
                    [],
                )
                .expect_err("half an answer is not an answer");
            assert!(
                half.to_string().contains("CHECK constraint failed"),
                "unexpected rejection: {half}"
            );
        }
        conn.execute(
            "UPDATE duplicate_conflict SET resolved_at = 2, resolution = 'kept' \
             WHERE axis = 'artefact'",
            [],
        )
        .expect("both halves together are the answer");

        // The answered row stays — the record that a conflict was raised
        // and ruled on is the reason resolution closes rather than
        // deletes.
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM duplicate_conflict", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 2);

        // A row cannot conflict with itself: the lookup returns the
        // asset that was just fingerprinted alongside the others, so
        // this is the mistake a caller makes once.
        let self_pair = conn.execute(
            "INSERT INTO duplicate_conflict
                 (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                  axis, content_hash, detected_at)
             VALUES (?1, ?2, ?3, ?3, ?3, ?3, 'artefact', 'sha256:abc', 1)",
            params![Uuid::now_v7(), persona, older],
        );
        let err = self_pair.expect_err("a pair of one is not a pair");
        assert!(
            err.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {err}"
        );

        // Purging an asset takes its questions with it — a queue row
        // whose sides cannot be hydrated is one the panel would crash
        // on rather than skip.
        conn.execute("DELETE FROM asset WHERE id = ?1", params![older])
            .unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM duplicate_conflict", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "both rows referenced the purged asset");
    }

    /// V54's column records why an automatic fold was declined, admits
    /// absence without naming it, and refuses a rule nothing applies.
    #[test]
    fn v54_a_declined_fold_names_a_rule_and_absence_stays_absent() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let older = seed_asset(&conn, persona);
        let newer = seed_asset(&conn, persona);
        let (lo, hi) = if older <= newer {
            (older, newer)
        } else {
            (newer, older)
        };

        let raise = |axis: &str, reason: Option<&str>| {
            conn.execute(
                "INSERT INTO duplicate_conflict
                     (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                      axis, content_hash, detected_at, fold_exclusion)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'sha256:abc', 1, ?8)",
                params![Uuid::now_v7(), persona, lo, hi, newer, older, axis, reason],
            )
        };

        raise("artefact", Some("lineage"))
            .expect("a declined fold names the rule that declined it");
        raise("content", Some("dispatch")).expect("both rules are storable");

        let reasons: Vec<Option<String>> = conn
            .prepare("SELECT fold_exclusion FROM duplicate_conflict ORDER BY axis")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            reasons,
            vec![Some("lineage".to_string()), Some("dispatch".to_string())],
            "the artefact-axis row sorts first, and each kept its own reason"
        );

        // A rule nothing implements would reach the panel as a warning
        // naming something that never ran.
        let unknown = raise("artefact", Some("derived")).expect_err("the rule set is closed");
        assert!(
            unknown.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {unknown}"
        );

        // Absence is the ordinary state — an `ask` nobody asked to fold
        // — and the CHECK admits it without mentioning it, because
        // `NULL IN (…)` is NULL rather than false.
        conn.execute(
            "UPDATE duplicate_conflict SET fold_exclusion = NULL WHERE axis = 'artefact'",
            [],
        )
        .expect("a question with no declined fold carries no reason");
        let cleared: Option<String> = conn
            .query_row(
                "SELECT fold_exclusion FROM duplicate_conflict WHERE axis = 'artefact'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cleared, None);
    }

    /// V64 widens the axis vocabulary to admit `'meta'` — and the
    /// widening is asserted as a behaviour change **across** the step,
    /// because a `CHECK` cannot be altered and the rebuild that replaces
    /// it is where the other constraints can be lost.
    ///
    /// The fixture disagrees with itself at V63: the same `INSERT` is
    /// refused before the upgrade and accepted after. Without the first
    /// half the second would hold on a build where the CHECK had simply
    /// been dropped.
    #[test]
    fn v64_admits_the_meta_axis_and_keeps_the_rest_of_the_table() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 63).unwrap();

        let persona = seed_persona(&conn);
        let older = seed_asset(&conn, persona);
        let newer = seed_asset(&conn, persona);
        let (lo, hi) = if older <= newer {
            (older, newer)
        } else {
            (newer, older)
        };

        let raise = |conn: &Connection, axis: &str| -> Result<usize, rusqlite::Error> {
            conn.execute(
                "INSERT INTO duplicate_conflict
                     (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                      axis, content_hash, detected_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'sha256:abc', 1)",
                params![Uuid::now_v7(), persona, lo, hi, newer, older, axis],
            )
        };

        raise(&conn, "file").expect("the axis that already existed");
        let refused = raise(&conn, "meta").expect_err("at V63 the vocabulary is two words");
        assert!(
            refused.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {refused}"
        );

        migrate(&mut conn).unwrap();

        // The row written before the upgrade survived the rebuild —
        // under the axis's new spelling, which the rebuild's own
        // `INSERT … SELECT` maps. The rename itself is asserted by
        // `v64_renames_the_file_axis_and_leaves_its_neighbour_alone`;
        // what this line holds is that the row was not lost.
        let carried: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM duplicate_conflict WHERE axis = 'artefact'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(carried, 1, "the rebuild copied the table's contents");

        raise(&conn, "meta").expect("the third axis is storable now");
        // …and the vocabulary is still closed, so the widening did not
        // become an absence.
        let unknown = raise(&conn, "metadata").expect_err("the axis set is closed");
        assert!(
            unknown.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {unknown}"
        );

        // The rebuild carries the rest of the table with it. The
        // uniqueness on `(pair_lo, pair_hi, axis)` is the one that would
        // put a pair in front of a person twice if it were lost.
        let repeated = raise(&conn, "meta").expect_err("one pair, one question per axis");
        assert!(
            repeated.to_string().contains("UNIQUE constraint failed"),
            "unexpected rejection: {repeated}"
        );
        // And the index the panel reads open questions through.
        let open_index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                  WHERE type = 'index' AND name = 'idx_duplicate_conflict_open'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open_index, 1, "the index went with the dropped table");

        // The cascade is a property of the FK clauses, which a rebuild
        // re-declares by hand and can therefore drop.
        conn.execute("DELETE FROM asset WHERE id = ?1", params![older])
            .unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM duplicate_conflict", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "both rows referenced the purged asset");
    }

    /// V64 rewrites the strongest axis's stored slug from `'file'` to
    /// `'artefact'` — **and touches nothing else on the column.**
    ///
    /// The `'content'` row beside it is the whole fixture. Asserting
    /// only that the first row now reads `'artefact'` would pass just as
    /// well against a blanket `UPDATE duplicate_conflict SET axis =
    /// 'artefact'`, which is a rename and a data loss wearing the same
    /// result. Two rows disagreeing on the axis is what separates them.
    ///
    /// The queue row's other columns are checked too: the mapping rides
    /// the rebuild's `INSERT … SELECT`, where a mis-ordered projection
    /// would put the axis in the wrong slot and still produce a table
    /// full of plausible rows.
    #[test]
    fn v64_renames_the_file_axis_and_leaves_its_neighbour_alone() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 63).unwrap();

        let persona = seed_persona(&conn);
        let older = seed_asset(&conn, persona);
        let newer = seed_asset(&conn, persona);
        let (lo, hi) = if older <= newer {
            (older, newer)
        } else {
            (newer, older)
        };

        // Same pair on both axes — two questions, and after the upgrade
        // exactly one of them should have changed its word.
        for (axis, digest) in [("file", "sha256:abc"), ("content", "cr1-sha256:def")] {
            conn.execute(
                "INSERT INTO duplicate_conflict
                     (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                      axis, content_hash, detected_at, fold_exclusion)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 77, 'lineage')",
                params![Uuid::now_v7(), persona, lo, hi, newer, older, axis, digest],
            )
            .expect("both axes are storable at V63");
        }

        migrate(&mut conn).unwrap();

        let rows: Vec<(String, String, i64, Option<String>)> = conn
            .prepare(
                "SELECT axis, content_hash, detected_at, fold_exclusion \
                   FROM duplicate_conflict ORDER BY axis",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(
            rows,
            vec![
                (
                    "artefact".to_string(),
                    "sha256:abc".to_string(),
                    77,
                    Some("lineage".to_string())
                ),
                (
                    "content".to_string(),
                    "cr1-sha256:def".to_string(),
                    77,
                    Some("lineage".to_string())
                ),
            ],
            "the file axis was renamed, the content axis was carried verbatim"
        );
    }

    /// The same rename on `edge.label`, **scoped by edge kind.**
    ///
    /// `label` is free text shared by every kind: a `derived_from` edge
    /// labelled `'file'` is a claim about where a row came from and has
    /// nothing to do with the duplicate vocabulary. The row of another
    /// kind carrying that label is the point of the fixture — an
    /// `UPDATE` written without the `kind` predicate passes every other
    /// assertion here and silently rewrites it.
    #[test]
    fn v64_renames_identical_to_labels_and_no_other_kind() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 63).unwrap();

        let persona = seed_persona(&conn);
        let a = seed_asset(&conn, persona);
        let b = seed_asset(&conn, persona);

        let seed_edge = |conn: &Connection, kind: &str, label: &str| {
            conn.execute(
                "INSERT INTO edge (id, from_asset, to_asset, kind, label) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![Uuid::now_v7(), a, b, kind, label],
            )
            .expect("the pair is edge-able");
        };

        seed_edge(&conn, "identical_to", "file");
        // Same word, different kind: not this vocabulary.
        seed_edge(&conn, "derived_from", "file");
        // Same kind, a different axis: not this rename.
        seed_edge(&conn, "reference", "content");

        migrate(&mut conn).unwrap();

        let labels: Vec<(String, Option<String>)> = conn
            .prepare("SELECT kind, label FROM edge ORDER BY kind")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(
            labels,
            vec![
                ("derived_from".to_string(), Some("file".to_string())),
                ("identical_to".to_string(), Some("artefact".to_string())),
                ("reference".to_string(), Some("content".to_string())),
            ],
            "only the duplicate-axis label moved"
        );
    }

    /// The third stored copy of the slug: the declared-hash note in
    /// `asset.extra`, rewritten **at its one JSON path**.
    ///
    /// Two negatives carry this one. A note on the content axis must
    /// survive (a blanket rewrite of the path would pass without it),
    /// and a `_trace.source` of `'file'` — the importer channel
    /// vocabulary, an unrelated set that happens to share the word —
    /// must survive too, which is what a rewrite reaching for `'file'`
    /// anywhere in the bag would destroy.
    #[test]
    fn v64_renames_the_declared_hash_note_and_nothing_else_in_the_bag() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 63).unwrap();

        let persona = seed_persona(&conn);
        let claimed = seed_asset(&conn, persona);
        let on_content = seed_asset(&conn, persona);
        let imported = seed_asset(&conn, persona);
        let bare = seed_asset(&conn, persona);

        let put = |asset: Uuid, extra: &str| {
            conn.execute(
                "UPDATE asset SET extra = ?2 WHERE id = ?1",
                params![asset, extra],
            )
            .unwrap();
        };
        put(
            claimed,
            r#"{"_trace":{"declared_hash":{"value":"sha256:abc","axis":"file"}}}"#,
        );
        put(
            on_content,
            r#"{"_trace":{"declared_hash":{"value":"cr1-sha256:def","axis":"content"}}}"#,
        );
        // A different `_trace` key whose vocabulary also has the word.
        put(imported, r#"{"_trace":{"source":"file"}}"#);
        // `extra` is nullable, and `json_extract` of a NULL is NULL —
        // which is not equal to anything, so the row is not touched.

        migrate(&mut conn).unwrap();

        let axis_of = |asset: Uuid| -> Option<String> {
            conn.query_row(
                "SELECT json_extract(extra, '$._trace.declared_hash.axis') \
                   FROM asset WHERE id = ?1",
                params![asset],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(axis_of(claimed).as_deref(), Some("artefact"));
        assert_eq!(axis_of(on_content).as_deref(), Some("content"));
        assert_eq!(axis_of(imported), None, "no note to rewrite");

        // The claim itself is untouched — only the label moved.
        let value: Option<String> = conn
            .query_row(
                "SELECT json_extract(extra, '$._trace.declared_hash.value') \
                   FROM asset WHERE id = ?1",
                params![claimed],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value.as_deref(), Some("sha256:abc"));

        let source: Option<String> = conn
            .query_row(
                "SELECT json_extract(extra, '$._trace.source') FROM asset WHERE id = ?1",
                params![imported],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            source.as_deref(),
            Some("file"),
            "the importer channel is a different vocabulary that shares the word"
        );

        let untouched: Option<String> = conn
            .query_row(
                "SELECT extra FROM asset WHERE id = ?1",
                params![bare],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(untouched, None, "a row with no bag is not given one");
    }

    /// After the rename the `CHECK` refuses the old slug.
    ///
    /// Without this the constraint is decoration: the data migration
    /// would have moved every existing row while a writer compiled
    /// against the old vocabulary went on inserting `'file'` beside
    /// them, and the column would hold both spellings of one axis with
    /// nothing to say which was meant.
    #[test]
    fn v64_refuses_the_axis_slug_it_migrated_away_from() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let older = seed_asset(&conn, persona);
        let newer = seed_asset(&conn, persona);
        let (lo, hi) = if older <= newer {
            (older, newer)
        } else {
            (newer, older)
        };

        let raise = |axis: &str| {
            conn.execute(
                "INSERT INTO duplicate_conflict
                     (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                      axis, content_hash, detected_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'sha256:abc', 1)",
                params![Uuid::now_v7(), persona, lo, hi, newer, older, axis],
            )
        };

        let refused = raise("file").expect_err("the old spelling is not in the vocabulary");
        assert!(
            refused.to_string().contains("CHECK constraint failed"),
            "unexpected rejection: {refused}"
        );
        // …and the new one is, so the assertion above is about the word
        // rather than about the insert being malformed.
        raise("artefact").expect("the axis under its own name");
    }

    /// V64 adds the meta axis **and answers it for every row that
    /// already existed**, so installing the build does not turn the
    /// user's whole library into work — the same shape V55 established,
    /// asserted against the same production predicate.
    ///
    /// The teeth are in the first assertion: the marker is what the
    /// deferred set is *selectable by*, and without it every existing
    /// material would match "no answer on the meta axis" and the next
    /// launch would re-read the whole corpus through the pass meant for
    /// new arrivals.
    #[test]
    fn v64_adds_the_meta_axis_without_handing_the_walk_the_whole_library() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 63).unwrap();

        let persona = seed_persona(&conn);
        let region = format!(
            "{}{}",
            asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
            "a".repeat(64)
        );
        // Rows in the states a pre-V64 database really holds: fully
        // answered on the two axes that existed, and one that can never
        // be fingerprinted at all.
        let stored = [
            (Some("sha256:aaaa"), Some(region.as_str())),
            (
                Some(asterism_core::domain::content_hash::UNHASHABLE),
                Some(asterism_core::domain::content_hash::UNHASHABLE),
            ),
        ];
        for (ord, (file, content)) in stored.iter().enumerate() {
            let asset = seed_asset(&conn, persona);
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, content_hash, \
                                       content_region_hash, created_at, updated_at) \
                 VALUES (?1, 0, ?2, 'image/png', ?3, ?4, 0, 0)",
                params![
                    asset,
                    format!("{{\"kind\":\"file\",\"path\":\"/pics/{ord}.png\"}}"),
                    file,
                    content
                ],
            )
            .unwrap();
        }

        let count = |conn: &Connection, predicate: &str| -> i64 {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM material WHERE {predicate}"),
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Nothing is work before the upgrade, on the two axes that
        // existed then — evaluated in the marker-era spelling, because
        // a database parked at V63 has no status columns yet. The third
        // argument is a column *expression*, so a literal that is
        // already an answer holds that axis constant and leaves the
        // other two being measured.
        let answered = format!("'{}'", asterism_core::domain::content_region::NOT_WALKED);
        assert_eq!(
            count(
                &conn,
                &marker_era_unfingerprinted("content_hash", "content_region_hash", &answered)
            ),
            0,
            "the fixture starts fully answered on the axes that existed"
        );
        // …and everything would be, if the column were added and left
        // NULL. This is the number the marker exists to avoid, and
        // asserting it is what stops the comparison below passing
        // vacuously.
        assert_eq!(
            count(
                &conn,
                &marker_era_unfingerprinted("content_hash", "content_region_hash", "NULL")
            ),
            stored.len() as i64,
            "without an answer written, the predicate claims every existing row"
        );

        migrate(&mut conn).unwrap();

        assert_eq!(
            count(&conn, &status_era_unfingerprinted()),
            0,
            "the migration handed the walk rows it did not have before"
        );

        // The answer written is the state the domain names, not a
        // literal that drifted apart from it. The originals do not
        // exist on disk, so V65 leaves them carrying it — which is the
        // true statement about a file it could not open. Read off the
        // status column, which is where V92 moved the marker.
        let marked: Vec<String> = conn
            .prepare("SELECT meta_hash_status FROM material ORDER BY asset_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            marked,
            vec![
                asterism_core::domain::measurement::MeasurementStatus::NotWalked
                    .as_str()
                    .to_string();
                2
            ],
            "every pre-existing row is answered"
        );
        // The object column takes no marker: it holds an object or
        // nothing, and nothing selects on its emptiness.
        let bodies: i64 = count(&conn, "meta_kv IS NOT NULL");
        assert_eq!(bodies, 0, "no walk produced an object for these rows");
    }

    /// V55 adds the content axis **and answers it for every row that
    /// already existed**, so installing the build does not turn the
    /// user's whole library into work.
    ///
    /// The second half is the one with teeth. A migration that only adds
    /// the column leaves every pre-existing material matching "no answer
    /// on the content axis", and the next launch re-reads the entire
    /// corpus off disk — gigabytes of I/O nobody asked for, on a machine
    /// somebody is using. The assertion is the production predicate
    /// (`unfingerprinted_condition`, what the backfill's page query and
    /// the progress count both run) evaluated across the migration:
    /// the number must not move, and the rows that were work before have
    /// to be exactly the rows that are work after.
    #[test]
    fn v55_adds_the_content_axis_without_handing_the_walk_the_whole_library() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 54).unwrap();

        let persona = seed_persona(&conn);
        // Three rows in the states a pre-V55 database really holds: a
        // fingerprinted material, one that can never be fingerprinted,
        // and one still waiting.
        let stored = [
            Some("sha256:aaaa"),
            Some(asterism_core::domain::content_hash::UNHASHABLE),
            None,
        ];
        let mut ids = Vec::new();
        for (ord, hash) in stored.iter().enumerate() {
            let asset = seed_asset(&conn, persona);
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, content_hash, \
                                       created_at, updated_at) \
                 VALUES (?1, 0, ?2, 'image/png', ?3, 0, 0)",
                params![asset, format!("/pics/{ord}.png"), hash],
            )
            .unwrap();
            ids.push(asset);
        }

        let rows = |conn: &Connection, predicate: &str| -> Vec<Uuid> {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT asset_id FROM material WHERE {predicate} ORDER BY asset_id"
                ))
                .unwrap();
            stmt.query_map([], |r| r.get::<_, Uuid>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        // What the walk asked before this migration: one column, one
        // question.
        let before = rows(&conn, "content_hash IS NULL");
        assert_eq!(
            before.len(),
            1,
            "only the material with no digest is work before the upgrade"
        );
        // What the walk would ask if the column were added and left
        // NULL — the whole table. This is the number the migration
        // exists to avoid, and asserting it here is what stops the
        // comparison below from passing vacuously.
        assert_eq!(
            rows(
                &conn,
                &marker_era_unfingerprinted("content_hash", "NULL", "NULL")
            )
            .len(),
            stored.len(),
            "without an answer written, the new predicate claims every existing row"
        );

        migrate(&mut conn).unwrap();

        let after = rows(&conn, &status_era_unfingerprinted());
        assert_eq!(
            after, before,
            "the migration handed the walk rows it did not have before"
        );

        // The answer written is the state the domain names, not a
        // literal that drifted apart from it — read off the status
        // column, which is where V92 moved the marker.
        let marked: Vec<String> = conn
            .prepare("SELECT content_region_hash_status FROM material ORDER BY asset_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            marked,
            vec![
                asterism_core::domain::measurement::MeasurementStatus::NotWalked
                    .as_str()
                    .to_string();
                stored.len()
            ],
            "every pre-existing row is answered, with the state the domain defines"
        );

        // A row that arrives *after* the migration starts NULL and is
        // work — the marker is a statement about rows that predate the
        // column, not a default on the column.
        let fresh = seed_asset(&conn, persona);
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at) \
             VALUES (?1, 0, '/pics/new.png', 'image/png', 0, 0)",
            params![fresh],
        )
        .unwrap();
        assert!(
            rows(&conn, &status_era_unfingerprinted()).contains(&fresh),
            "a material inserted after the upgrade has to reach the walk"
        );

        // Re-running the chain against an up-to-date database is a
        // no-op rather than a duplicate-column error — and in
        // particular it does not re-mark rows that have since been
        // fingerprinted for real.
        let region = format!(
            "{}{}",
            asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX,
            "a".repeat(64)
        );
        conn.execute(
            "UPDATE material SET content_hash = 'sha256:bbbb', content_region_hash = ?2 \
              WHERE asset_id = ?1",
            params![fresh, region],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        let kept: Option<String> = conn
            .query_row(
                "SELECT content_region_hash FROM material WHERE asset_id = ?1",
                params![fresh],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept.as_deref(), Some(region.as_str()));

        // The index the duplicate report will group by exists, and is
        // partial on the same side as its file-axis sibling.
        let index: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' \
                  AND name = 'idx_material_content_region_hash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(index.contains("content_region_hash IS NOT NULL"), "{index}");
        // The marker survived the full chain above because V56 ran next
        // and could not open any of these locators — no such file exists
        // under `/pics/`. That is the "left carrying the marker" branch,
        // and it is why the assertion on `marked` still describes the
        // whole table after `migrate`.
    }

    /// The marker-era spelling of the fingerprint walk's condition,
    /// frozen for the **pre-upgrade halves** of the tests in this file:
    /// production reads the status columns now (V92), and a database
    /// parked mid-chain does not have them yet. Same rule as
    /// [`pre_v92_stored_value`]'s — a landed era keeps its own spelling.
    fn marker_era_unfingerprinted(file: &str, content: &str, meta: &str) -> String {
        let answered = |column: &str, prefix: &str| {
            format!(
                "({column} GLOB '{prefix}*' \
                  OR {column} GLOB 'unsupported:*' \
                  OR {column} = 'unhashable:no-bytes')"
            )
        };
        format!(
            "{file} IS NULL \
             OR {content} IS NULL \
             OR NOT {} \
             OR {meta} IS NULL \
             OR NOT {}",
            answered(content, "cr1-sha256:"),
            answered(meta, "m1-sha256:")
        )
    }

    /// [`marker_era_unfingerprinted`]'s Rust half — what
    /// `needs_fingerprint` answered while the columns carried the
    /// inline vocabulary.
    fn marker_era_owes(file: Option<&str>, content: Option<&str>, meta: Option<&str>) -> bool {
        let answered = |value: Option<&str>, prefix: &str| {
            value.is_some_and(|v| {
                v.starts_with(prefix) || v.starts_with("unsupported:") || v == "unhashable:no-bytes"
            })
        };
        file.is_none() || !answered(content, "cr1-sha256:") || !answered(meta, "m1-sha256:")
    }

    /// The production walk condition over the `material` table's own
    /// columns — usable only on a database migrated **through V92**.
    fn status_era_unfingerprinted() -> String {
        use crate::sqlite::repo::asset::{AxisColumns, unfingerprinted_condition};

        unfingerprinted_condition(
            &AxisColumns {
                status: "content_hash_status",
                digest: "content_hash",
            },
            &AxisColumns {
                status: "content_region_hash_status",
                digest: "content_region_hash",
            },
            &AxisColumns {
                status: "meta_hash_status",
                digest: "meta_hash",
            },
        )
    }

    /// The production skip test over one migrated row — reads the
    /// status columns V92 added beside the digests.
    fn status_era_owes(conn: &Connection, asset: Uuid) -> bool {
        use asterism_core::domain::content_hash::needs_fingerprint;
        use asterism_core::domain::measurement::MeasurementStatus;

        type Axis = (String, Option<String>);
        let (file, content, meta): (Axis, Axis, Axis) = conn
            .query_row(
                "SELECT content_hash_status, content_hash, \
                        content_region_hash_status, content_region_hash, \
                        meta_hash_status, meta_hash \
                 FROM material WHERE asset_id = ?1",
                params![asset],
                |row| {
                    Ok((
                        (row.get(0)?, row.get(1)?),
                        (row.get(2)?, row.get(3)?),
                        (row.get(4)?, row.get(5)?),
                    ))
                },
            )
            .unwrap();
        needs_fingerprint(
            (
                MeasurementStatus::parse(&file.0).unwrap(),
                file.1.as_deref(),
            ),
            (
                MeasurementStatus::parse(&content.0).unwrap(),
                content.1.as_deref(),
            ),
            (
                MeasurementStatus::parse(&meta.0).unwrap(),
                meta.1.as_deref(),
            ),
        )
    }

    /// Builds a PNG the walker accepts. `PngBuilder::new()` supplies the
    /// signature and a 1×1 grayscale IHDR; `raw_chunk` writes IDAT and
    /// the optional `tEXt` with zero CRCs (the walker reads past them
    /// without checking, and says why). The two `png` fixtures below
    /// share this IHDR so their content-region digests differ only in
    /// the presence of the `tEXt` chunk — the whole point of the axis.
    fn png(pixels: &[u8], text: Option<&[u8]>) -> Vec<u8> {
        let mut b =
            pngmeta::test_util::PngBuilder::new().raw_chunk(*b"IDAT", pixels.len() as u32, pixels);
        if let Some(text) = text {
            b = b.raw_chunk(*b"tEXt", text.len() as u32, text);
        }
        b.build()
    }

    /// Inserts one material carrying the shape a pre-V55 row has: a
    /// locator, a format guess, and whatever the file axis already
    /// recorded. V55 is what puts the marker on it.
    fn seed_material(
        conn: &Connection,
        persona: Uuid,
        locator: &str,
        mime: Option<&str>,
        file_hash: Option<&str>,
    ) -> Uuid {
        let asset = seed_asset(conn, persona);
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, mime, content_hash, \
                                   created_at, updated_at) \
             VALUES (?1, 0, ?2, ?3, ?4, 0, 0)",
            params![asset, locator, mime, file_hash],
        )
        .unwrap();
        asset
    }

    fn content_of(conn: &Connection, asset: Uuid) -> Option<String> {
        conn.query_row(
            "SELECT content_region_hash FROM material WHERE asset_id = ?1",
            params![asset],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn file_of(conn: &Connection, asset: Uuid) -> Option<String> {
        conn.query_row(
            "SELECT content_hash FROM material WHERE asset_id = ?1",
            params![asset],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// One axis's stored triple after the full chain — the shape V92
    /// leaves the columns in.
    fn axis_state_of(
        conn: &Connection,
        asset: Uuid,
        column: &str,
    ) -> (String, Option<String>, Option<String>) {
        conn.query_row(
            &format!(
                "SELECT {column}_status, {column}, {column}_reason \
                 FROM material WHERE asset_id = ?1"
            ),
            params![asset],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    /// V56 is the half of the content-column migration that reads the
    /// files, and this is its whole contract: every row V55 deferred
    /// ends up carrying a digest or a marker that says why it does not.
    ///
    /// The fixture puts one row in each outcome the pass can reach, and
    /// the pair of PNGs is the one that has to agree: two files that
    /// differ only in a `tEXt` chunk are one picture, which is the
    /// entire reason the axis exists. A pass that wrote markers
    /// everywhere would satisfy "the marker is gone" and fail here.
    #[test]
    fn v56_computes_the_content_values_v55_deferred() {
        use asterism_core::domain::content_hash::{CONTENT_DIGEST_PREFIX, UNHASHABLE};

        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| -> String {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).expect("fixture written");
            path.to_string_lossy().into_owned()
        };

        let pixels = b"a compressed stream, near enough";
        let bare = write("bare.png", &png(pixels, None));
        let noted = write("noted.png", &png(pixels, Some(b"workflow\0{...}")));
        let clip = write("clip.mp4", b"\0\0\0\x18ftypmp42not really a movie");

        // Seeded at 54 — *before* the column exists — so the rows are
        // genuinely pre-existing and V55's own `UPDATE` is what puts the
        // marker on them. Seeding after V55 would produce NULLs, which
        // is a different state and the one the ordinary walk owns.
        let mut conn = test_conn();
        migrate_to(&mut conn, 54).unwrap();
        let persona = seed_persona(&conn);

        // The file digests a pre-existing library already holds, taken
        // from the same hasher the pass will use — so a change in this
        // column is a change in the bytes rather than a change of
        // spelling.
        let digest = |path: &str| {
            asterism_core::domain::content_hash::of_bytes(&std::fs::read(path).unwrap())
        };
        let bare_file = digest(&bare);
        let noted_file = digest(&noted);
        let clip_file = digest(&clip);

        let a = seed_material(&conn, persona, &bare, Some("image/png"), Some(&bare_file));
        let b = seed_material(&conn, persona, &noted, Some("image/png"), Some(&noted_file));
        let video = seed_material(&conn, persona, &clip, Some("video/mp4"), Some(&clip_file));
        let record = seed_material(
            &conn,
            persona,
            "/logs/session.jsonl#0198c1c2-aaaa",
            Some("application/json"),
            Some(UNHASHABLE),
        );
        let gone = seed_material(
            &conn,
            persona,
            &dir.path().join("deleted.png").to_string_lossy(),
            Some("image/png"),
            Some("sha256:aaaa"),
        );

        // V55 puts the marker on all five; V56 is the next step of the
        // same chain, so `migrate` runs both. The ordinary walk's rule
        // is asked on each side of that pair — before the column exists
        // it is the one-column question the walk used to ask.
        let count = |conn: &Connection, condition: &str| -> i64 {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM material WHERE {condition}"),
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        let before = count(&conn, "content_hash IS NULL");
        migrate(&mut conn).unwrap();
        let unfingerprinted =
            |conn: &Connection| -> i64 { count(conn, &status_era_unfingerprinted()) };

        // The two PNGs: a real digest each, and the **same** one.
        let a_region = content_of(&conn, a).expect("a walked row carries a value");
        let b_region = content_of(&conn, b).expect("a walked row carries a value");
        assert!(a_region.starts_with(CONTENT_DIGEST_PREFIX), "{a_region}");
        assert_eq!(
            a_region, b_region,
            "two files differing only in a tEXt chunk are one picture"
        );
        assert_ne!(
            bare_file, noted_file,
            "…and they really are different files, or the line above is vacuous"
        );

        // A format with no walker says which format it declined, and a
        // locator with no bytes of its own gets the permanent answer.
        // V56 wrote those as markers and V92, further down the same
        // chain, moved them into the status and reason columns — so the
        // end state read here is the split form.
        assert_eq!(
            axis_state_of(&conn, video, "content_region_hash"),
            (
                "unsupported".to_string(),
                None,
                Some("video/mp4".to_string())
            )
        );
        assert_eq!(
            axis_state_of(&conn, record, "content_region_hash"),
            ("no-bytes".to_string(), None, None)
        );

        // A file that could not be read keeps the deferred state:
        // "nothing walked these bytes" is still exactly true of it.
        assert_eq!(
            axis_state_of(&conn, gone, "content_region_hash"),
            ("not-walked".to_string(), None, None)
        );

        // The file column does not move. Same bytes, same digest — and
        // the row this pass never opened keeps what it had rather than
        // being overwritten with a marker.
        assert_eq!(file_of(&conn, a).as_deref(), Some(bare_file.as_str()));
        assert_eq!(file_of(&conn, b).as_deref(), Some(noted_file.as_str()));
        assert_eq!(file_of(&conn, video).as_deref(), Some(clip_file.as_str()));
        assert_eq!(
            axis_state_of(&conn, record, "content_hash"),
            ("no-bytes".to_string(), None, None)
        );
        assert_eq!(file_of(&conn, gone).as_deref(), Some("sha256:aaaa"));

        // The ordinary fingerprint walk is not handed anything by this.
        // Both numbers are zero — every row was answered on both axes
        // before the pass and still is — so the pass neither created
        // work nor consumed any.
        assert_eq!(before, 0, "the fixture starts fully answered");
        assert_eq!(unfingerprinted(&conn), 0, "and stays that way");

        // Re-running the chain changes nothing: `user_version` is
        // current, so the pass does not re-read the four files it
        // already answered, nor retry the one it could not.
        let snapshot: Vec<(Option<String>, Option<String>)> = conn
            .prepare("SELECT content_hash, content_region_hash FROM material ORDER BY asset_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        migrate(&mut conn).unwrap();
        let again: Vec<(Option<String>, Option<String>)> = conn
            .prepare("SELECT content_hash, content_region_hash FROM material ORDER BY asset_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(snapshot, again);
    }

    /// Teeth: the migration writes fingerprints and **decides nothing**.
    ///
    /// The fixture is the case that would fold if anything here ran
    /// ingest-flavoured detection: two live rows of one persona whose
    /// bytes decode to the same picture, with the younger declaring
    /// `on_duplicate = 'fold'`. After the migration the pair is real —
    /// the two rows share a content digest, which is what makes the
    /// assertions below non-vacuous — and yet no row is a headstone, no
    /// question is queued, and no `identical_to` edge was minted.
    ///
    /// Folding two rows that have both been in the library is a
    /// confirmed act, and an upgrade is not a confirmation. The pairs this migration reveals reach the user
    /// through the duplicate report, which groups on the column and
    /// therefore needs nothing written here to find them.
    #[test]
    fn v56_reveals_a_pair_without_folding_or_queueing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| -> String {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).expect("fixture written");
            path.to_string_lossy().into_owned()
        };
        let pixels = b"one picture, written twice";
        let first = write("first.png", &png(pixels, Some(b"prompt\0a")));
        let second = write("second.png", &png(pixels, Some(b"prompt\0b")));

        // 54, not 55: the rows have to predate the column for V55 to
        // mark them, which is what makes them V56's work.
        let mut conn = test_conn();
        migrate_to(&mut conn, 54).unwrap();
        let persona = seed_persona(&conn);
        let digest = |path: &str| {
            asterism_core::domain::content_hash::of_bytes(&std::fs::read(path).unwrap())
        };
        let older = seed_material(
            &conn,
            persona,
            &first,
            Some("image/png"),
            Some(&digest(&first)),
        );
        let newer = seed_material(
            &conn,
            persona,
            &second,
            Some("image/png"),
            Some(&digest(&second)),
        );
        // The declaration that would make an automatic fold available to
        // anything that asked the strategy.
        conn.execute(
            "UPDATE asset SET on_duplicate = 'fold' WHERE id = ?1",
            params![newer],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        // The pair exists on the content axis…
        let shared = content_of(&conn, older);
        assert_eq!(
            shared,
            content_of(&conn, newer),
            "the fixture has to be a real pair or the assertions below prove nothing"
        );
        assert!(
            shared.as_deref().is_some_and(
                |v| v.starts_with(asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX)
            ),
            "…and a real digest rather than a shared marker: {shared:?}"
        );

        // …and nothing acted on it.
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT COUNT(*) FROM asset WHERE folded_into IS NOT NULL"),
            0,
            "an upgrade folded two of the user's rows together"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM duplicate_conflict"),
            0,
            "the migration queued questions nobody asked for"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM edge WHERE kind = 'identical_to'"),
            0,
            "the migration minted detection's records without running detection"
        );
    }

    /// V67 backfills the statements already in the bags, and then keeps
    /// the projection level with the column from the column's own side.
    ///
    /// The fixture carries the three shapes a real database has besides
    /// a well-formed bag — no `extra` at all, a bag that is not JSON, and
    /// a `_trace` whose `meta` is the wrong type — because the rebuild
    /// runs inside an `AFTER UPDATE` trigger, and an error there does not
    /// skip a row: it aborts the write that fired it. A row whose bag is
    /// malformed has to stay saveable.
    #[test]
    fn v67_backfills_the_statements_already_recorded_and_tracks_the_column_after() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 66).unwrap();

        let persona = seed_persona(&conn);
        let stated = seed_asset(&conn, persona);
        let bare = seed_asset(&conn, persona);
        let corrupt = seed_asset(&conn, persona);
        let wrong_type = seed_asset(&conn, persona);

        fn set_extra(conn: &Connection, id: Uuid, raw: &str) {
            conn.execute(
                "UPDATE asset SET extra = ?1 WHERE id = ?2",
                params![raw, id],
            )
            .unwrap();
        }
        set_extra(
            &conn,
            stated,
            r#"{"camera":"X100","_trace":{"declared_hash":{"value":"sha256:aa"},
                "meta":{"workflow-id":{"value":"wf-1","source":"pushed"},
                        "plate":{"value":"offwhite","source":"manual"},
                        "half-written":{"source":"manual"}}}}"#,
        );
        set_extra(&conn, corrupt, "not json at all");
        set_extra(
            &conn,
            wrong_type,
            r#"{"_trace":{"meta":"a string, not an object"}}"#,
        );

        migrate_to(&mut conn, 67).unwrap();

        let statements_of = |conn: &Connection, id: Uuid| -> Vec<(String, String)> {
            conn.prepare("SELECT key, value FROM asset_album_meta WHERE asset_id = ?1 ORDER BY key")
                .unwrap()
                .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            statements_of(&conn, stated),
            vec![
                ("plate".to_string(), "offwhite".to_string()),
                ("workflow-id".to_string(), "wf-1".to_string()),
            ],
            "an entry with no `value` is not a statement and is not indexed"
        );
        assert!(statements_of(&conn, bare).is_empty());
        assert!(
            statements_of(&conn, corrupt).is_empty(),
            "an unreadable bag yields no row, and does not abort the batch"
        );
        assert!(statements_of(&conn, wrong_type).is_empty());

        // From here the trigger is what keeps the two level. A write
        // that does not go through any entity — which is what the fold
        // note and the declared-hash verdict are — still lands.
        set_extra(
            &conn,
            stated,
            r#"{"_trace":{"meta":{"workflow-id":{"value":"wf-2"}}}}"#,
        );
        assert_eq!(
            statements_of(&conn, stated),
            vec![("workflow-id".to_string(), "wf-2".to_string())],
            "the correction replaced the value and the dropped key went with it"
        );

        // A malformed bag arriving after the fact must not make the row
        // unsaveable.
        set_extra(&conn, stated, "still not json");
        assert!(statements_of(&conn, stated).is_empty());

        // A fresh insert is covered by its own trigger, not only by the
        // backfill that ran once.
        let born_stating = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at, extra) \
             VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[]', 0, 0, 0, ?4)",
            params![
                born_stating,
                persona,
                format!("b-{born_stating}.md"),
                r#"{"_trace":{"meta":{"edition":{"value":"c-12"}}}}"#,
            ],
        )
        .unwrap();
        assert_eq!(
            statements_of(&conn, born_stating),
            vec![("edition".to_string(), "c-12".to_string())]
        );

        // The projection follows its asset out of the database.
        conn.execute("DELETE FROM asset WHERE id = ?1", params![born_stating])
            .unwrap();
        assert!(
            statements_of(&conn, born_stating).is_empty(),
            "asset_album_meta rows cascade with the asset"
        );
    }

    /// V49 mints the identity `author_kind = 'owner'` refers to, exactly
    /// once, and the schema refuses a second one.
    #[test]
    fn v49_mints_one_instance_identity_and_refuses_a_second_row() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let (id, created_at, owner): (Uuid, i64, Option<String>) = conn
            .query_row(
                "SELECT instance_id, created_at, owner_subject FROM instance",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(id.get_version_num(), 7, "the identity is a UUID v7");
        assert!(created_at > 0);
        assert_eq!(
            owner, None,
            "a local instance has no authenticated subject — unbound, not a placeholder"
        );

        // The singleton is the schema's to enforce, not a convention a
        // future write path has to remember.
        let second = conn.execute(
            "INSERT INTO instance (id, instance_id, created_at) VALUES (1, ?1, 0)",
            params![Uuid::now_v7()],
        );
        assert!(
            second.is_err(),
            "CHECK (id = 0) must reject a second instance row"
        );
    }

    /// Re-running the migration must not mint a second identity: the id
    /// is what `Author::Owner` points at, and a reissue would split the
    /// instance in two.
    #[test]
    fn v49_does_not_remint_on_a_second_migrate() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let minted: Uuid = conn
            .query_row("SELECT instance_id FROM instance", [], |r| r.get(0))
            .unwrap();

        migrate(&mut conn).unwrap();

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM instance", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let after: Uuid = conn
            .query_row("SELECT instance_id FROM instance", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, minted, "the identity is minted once, then kept");
    }

    /// V50 opens the channel column on both tables. A row that predates
    /// it keeps an author with no channel — the legacy shape the read
    /// side accepts and a backfill would have to forge.
    #[test]
    fn v50_adds_the_channel_column_leaving_earlier_rows_without_one() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 49).unwrap();

        let persona = seed_persona(&conn);
        let legacy = seed_asset(&conn, persona);
        conn.execute(
            "UPDATE asset SET author_kind = 'owner', operator_ai = 'asterism-ui' WHERE id = ?1",
            params![legacy],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let via: Option<String> = conn
            .query_row(
                "SELECT attributed_via FROM asset WHERE id = ?1",
                params![legacy],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            via, None,
            "a V47-era row records an author with no channel, and stays that way"
        );

        // The dispatch side gains the author pair alongside the channel.
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'h', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        let job = Uuid::now_v7();
        conn.execute(
            "INSERT INTO dispatch_job (id, snapshot_id, persona_id, exporter_slug, action, \
                                       params_json, state_slug, output_asset_ids, \
                                       created_at, updated_at, \
                                       author_kind, author_subject, attributed_via) \
             VALUES (?1, ?2, ?3, 'comfy', 'run', '{}', 'pending', '[]', 0, 0, \
                     'subject', 'alice', 'asserted')",
            params![job, snapshot, persona],
        )
        .unwrap();
        let stored: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT author_kind, author_subject, attributed_via FROM dispatch_job \
                 WHERE id = ?1",
                params![job],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            stored,
            (
                Some("subject".to_string()),
                Some("alice".to_string()),
                Some("asserted".to_string())
            )
        );
    }

    /// The closed list of tables that record a channel, and wiring more
    /// services to *receive* an `AttributionContext` does not extend it
    /// (an operation ledger is a separate decision, not a column that
    /// appears table by table). Two waves so far: `asset` /
    /// `dispatch_job` (V47-V50), and the pursuit family (V79) — forge
    /// events are actor-carrying by design (#29: who opened the
    /// pursuit, who recorded the close, who ordered the restamp), with
    /// the same NULL-means-unrecorded reading and the same write-side
    /// channel guard.
    ///
    /// Read off the schema rather than a hand-kept list: a new
    /// `attributed_via` would otherwise arrive silently, and every one of
    /// them is a place the attribution rule has to answer for — which channel
    /// wrote it, what a NULL means there, and how auth resolves it
    /// later. Third wave: the ledger (V82) — a membership gesture is a
    /// statement somebody makes (#22: "who decided"), so it carries
    /// the triple. Fourth wave: the project (V84) —
    /// opening one is a statement; `line_merge` deliberately does
    /// not carry the triple (who approved is who closed, and the
    /// close event already says so), and the line tables are
    /// derivation surfaces, not statements.
    #[test]
    fn only_settled_tables_carry_a_channel_column() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT m.name FROM sqlite_master m, pragma_table_info(m.name) p \
                 WHERE m.type = 'table' AND p.name = 'attributed_via' \
                 ORDER BY m.name",
            )
            .unwrap();
        let carriers: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap();

        assert_eq!(
            carriers,
            vec![
                "asset".to_string(),
                "dispatch_job".to_string(),
                "project".to_string(),
                "pursuit".to_string(),
                "pursuit_event".to_string(),
                "pursuit_tx".to_string(),
            ],
            "a table gained an attribution channel; that is a design decision, not a migration \
             detail — settle where attribution state lives before changing this list"
        );
    }

    /// V84's rules are pairing rules, and they live in the schema: a
    /// verb's payload columns travel with the verb (two two-way
    /// CHECKs), a project holds one line per name, and a close event
    /// carries at most one merge. Asserted at insert level because
    /// `LineVerb::from_columns` only guards the Rust side — a typo
    /// inside the SQL CHECK text would otherwise go unseen until the
    /// write path arrives (the `v78_holds_the_two_rules` precedent:
    /// the schema is where the property actually lives).
    #[test]
    fn v84_pairs_verb_and_payload_and_keeps_lines_and_merges_unique() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);

        let project = Uuid::now_v7();
        conn.execute(
            "INSERT INTO project (id, persona_id, name, created_at) VALUES (?1, ?2, 'album', 0)",
            params![project, persona],
        )
        .unwrap();

        let line = Uuid::now_v7();
        let line_named_main = |id: Uuid| {
            conn.execute(
                "INSERT INTO line (id, project_id, name, created_at) \
                 VALUES (?1, ?2, 'main', 0)",
                params![id, project],
            )
        };
        line_named_main(line).unwrap();
        assert!(
            line_named_main(Uuid::now_v7()).is_err(),
            "one project holds one line per name"
        );

        let entry = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line_entry (id, line_id, persona_id, created_at) \
             VALUES (?1, ?2, ?3, 0)",
            params![entry, line, persona],
        )
        .unwrap();

        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let close = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit_event (id, pursuit_id, persona_id, kind, created_at) \
             VALUES (?1, ?2, ?3, 'closed_satisfied', 0)",
            params![close, pursuit, persona],
        )
        .unwrap();

        let merge = Uuid::now_v7();
        let merge_of_close = |id: Uuid| {
            conn.execute(
                "INSERT INTO line_merge (id, pursuit_event_id, persona_id, created_at) \
                 VALUES (?1, ?2, ?3, 0)",
                params![id, close, persona],
            )
        };
        merge_of_close(merge).unwrap();
        assert!(
            merge_of_close(Uuid::now_v7()).is_err(),
            "one close event carries at most one merge"
        );

        let event = |verb: &str, with_asset: bool, with_name: bool| {
            conn.execute(
                "INSERT INTO line_event \
                     (id, entry_id, persona_id, verb, asset_id, name, merge_id, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                params![
                    Uuid::now_v7(),
                    entry,
                    persona,
                    verb,
                    with_asset.then(Uuid::now_v7),
                    with_name.then_some("key visual"),
                    merge,
                ],
            )
        };

        event("add", true, true).expect("an add carries both payloads");
        event("replace", true, false).expect("a replace carries the asset alone");
        event("rename", false, true).expect("a rename carries the name alone");
        event("delete", false, false).expect("a delete carries nothing");

        assert!(event("add", true, false).is_err(), "a nameless add");
        assert!(event("add", false, true).is_err(), "an assetless add");
        assert!(
            event("replace", false, false).is_err(),
            "an assetless replace"
        );
        assert!(
            event("replace", true, true).is_err(),
            "a replace naming things"
        );
        assert!(event("rename", false, false).is_err(), "a nameless rename");
        assert!(
            event("delete", true, false).is_err(),
            "a delete carrying an asset"
        );
        assert!(
            event("fold", false, false).is_err(),
            "the verb set is closed"
        );
    }

    /// V85's rules are pairing rules carried by **column-level** CHECKs
    /// on ALTER-added columns, which is the arrangement the step doc
    /// says was measured rather than assumed. This is where the
    /// measurement is pinned: each rule is asserted against a real
    /// insert, so a CHECK that survived the ALTER without firing — the
    /// failure mode that would make the whole choice wrong — cannot
    /// pass unnoticed.
    ///
    /// The other half is the ledger a pre-V85 database already holds.
    /// `pursuit_tx` rows are history and a lost one is a gesture nobody
    /// can re-perform, so a row is seeded *before* the step runs and
    /// read back after with every attribution column named — the four
    /// that a careless widening would drop.
    ///
    /// It runs to *latest* rather than stopping at 85, so what it
    /// asserts is the shape a caller meets today: V91 dropped the pin
    /// column, its CHECK and its index, and the rules asserted below
    /// are the three that outlived it. That makes this the test that
    /// answers for V91 leaving the surviving column-level CHECKs
    /// standing — `DROP COLUMN` rewrites the table's schema text, and a
    /// CHECK lost in that rewrite would fire nowhere and say nothing.
    #[test]
    fn v85_leaves_the_ledger_alone_and_pairs_the_new_columns() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 84).unwrap();
        let persona = seed_persona(&conn);

        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let legacy_tx = Uuid::now_v7();
        let legacy_asset = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit_tx \
                 (id, pursuit_id, persona_id, kind, asset_id, origin, note, \
                  author_kind, author_subject, operator_ai, attributed_via, created_at) \
             VALUES (?1, ?2, ?3, 'in', ?4, 'generated', 'first light', \
                     'subject', 'alice', 'claude-code', 'mcp', 7)",
            params![legacy_tx, pursuit, persona, legacy_asset],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        type LegacyRow = (
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );
        let carried: LegacyRow = conn
            .query_row(
                "SELECT asset_id, kind, origin, note, author_kind, author_subject, \
                        operator_ai, attributed_via, created_at \
                 FROM pursuit_tx WHERE id = ?1",
                params![legacy_tx],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            carried,
            (
                legacy_asset,
                "in".to_string(),
                Some("generated".to_string()),
                Some("first light".to_string()),
                Some("subject".to_string()),
                Some("alice".to_string()),
                Some("claude-code".to_string()),
                Some("mcp".to_string()),
                7
            ),
            "the gesture, its origin, and the whole attribution triple are untouched"
        );
        assert_eq!(
            conn.query_row(
                "SELECT out_of_scope, target_entry_id IS NULL, \
                        supersedes_asset_id IS NULL \
                 FROM pursuit_tx WHERE id = ?1",
                params![legacy_tx],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?
                ))
            )
            .unwrap(),
            (0, 1, 1),
            "a gesture that predates the columns aimed at nothing and reached outside nothing"
        );
        assert_eq!(
            conn.query_row(
                "SELECT project_id IS NULL FROM pursuit WHERE id = ?1",
                params![pursuit],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "and a pursuit that predates filing is left unfiled rather than given a project"
        );

        // A project, its line, and one entry to aim at.
        let project = Uuid::now_v7();
        conn.execute(
            "INSERT INTO project (id, persona_id, name, created_at) VALUES (?1, ?2, 'album', 0)",
            params![project, persona],
        )
        .unwrap();
        let line = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line (id, project_id, name, created_at) VALUES (?1, ?2, 'main', 0)",
            params![line, project],
        )
        .unwrap();
        let entry = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line_entry (id, line_id, persona_id, created_at) VALUES (?1, ?2, ?3, 0)",
            params![entry, line, persona],
        )
        .unwrap();

        conn.execute(
            "UPDATE pursuit SET project_id = ?1 WHERE id = ?2",
            params![project, pursuit],
        )
        .expect("a pursuit files under a project");

        let tx = |kind: &str,
                  origin: Option<&str>,
                  target: Option<Uuid>,
                  out_of_scope: i64,
                  supersedes: Option<Uuid>| {
            conn.execute(
                "INSERT INTO pursuit_tx \
                     (id, pursuit_id, persona_id, kind, asset_id, origin, \
                      target_entry_id, out_of_scope, \
                      supersedes_asset_id, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
                params![
                    Uuid::now_v7(),
                    pursuit,
                    persona,
                    kind,
                    Uuid::now_v7(),
                    origin,
                    target,
                    out_of_scope,
                    supersedes,
                ],
            )
        };

        tx("in", Some("existing"), Some(entry), 0, None)
            .expect("an existing IN may name the entry it aims at");
        tx("in", Some("generated"), None, 0, None).expect("an untargeted IN stays legal");
        tx("in", Some("existing"), Some(entry), 1, None)
            .expect("an IN may declare it reached outside its scope");
        tx("update", None, None, 0, Some(legacy_asset))
            .expect("an update may name the member it revises");
        tx("update", None, None, 0, None)
            .expect("an update without one is still admitted — P3 closes that direction");

        assert!(
            tx("in", Some("generated"), Some(entry), 0, None).is_err(),
            "only an existing-origin IN targets an entry"
        );
        assert!(
            tx("remove", None, Some(entry), 0, None).is_err(),
            "a remove targets nothing"
        );
        assert!(
            tx("remove", None, None, 1, None).is_err(),
            "only an IN can reach outside a scope"
        );
        assert!(
            tx("in", Some("generated"), None, 0, Some(legacy_asset)).is_err(),
            "only an update supersedes"
        );

        // The target column references rather than merely records, and
        // a reference nobody checks is a column of loose uuids.
        // Asserted with an id that resolves to nothing, which is the
        // one thing the CHECKs above cannot catch.
        assert!(
            tx("in", Some("existing"), Some(Uuid::now_v7()), 0, None).is_err(),
            "a target that names no entry"
        );
    }

    /// Two waves that were both written as "the next few steps" are one
    /// chain now, and this is the assertion that they interleave rather
    /// than overwrite: attribution (V47-V50) runs first, the identity
    /// axis (V51-V56) after it, and a database that walks the whole
    /// chain carries the columns of both.
    ///
    /// Run from an empty database and from a mid-chain one, because
    /// they exercise different things. Empty walks every step in order.
    /// V45 is the version a real profile is sitting at, so the second
    /// pass is the upgrade an existing library will actually perform —
    /// with rows already in `asset`, which is the table both waves
    /// `ALTER`.
    #[test]
    fn both_waves_reach_one_asset_table_from_empty_and_from_a_live_v45() {
        for start in [0usize, 45] {
            let mut conn = test_conn();
            // A row that predates both waves, so every `ALTER` below
            // has existing data to carry rather than an empty table.
            if start > 0 {
                migrate_to(&mut conn, start).unwrap();
                let persona = seed_persona(&conn);
                seed_asset(&conn, persona);
            }

            migrate(&mut conn).unwrap();

            let columns = |conn: &Connection, table: &str| -> Vec<String> {
                let mut stmt = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                    .unwrap();
                stmt.query_map([], |r| r.get(0))
                    .unwrap()
                    .collect::<Result<Vec<String>, _>>()
                    .unwrap()
            };
            let asset = columns(&conn, "asset");
            for column in [
                // The attribution wave.
                "author_kind",
                "author_subject",
                "operator_ai",
                "attributed_via",
                // The identity wave.
                "folded_into",
                "fold_policy",
                "on_duplicate",
            ] {
                assert!(
                    asset.iter().any(|name| name == column),
                    "starting at {start}, `asset` has no `{column}` after the full chain: {asset:?}"
                );
            }
            assert!(
                columns(&conn, "material")
                    .iter()
                    .any(|name| name == "content_region_hash"),
                "starting at {start}, the identity wave's last column is missing"
            );
            // The two steps that are not `ALTER`s: one table from each
            // wave, either of which a mis-ordered chain could drop.
            let table_count = |name: &str| -> i64 {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![name],
                    |r| r.get(0),
                )
                .unwrap()
            };
            assert_eq!(table_count("instance"), 1, "starting at {start}");
            assert_eq!(table_count("duplicate_conflict"), 1, "starting at {start}");

            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version, LATEST_VERSION, "starting at {start}");
        }
    }

    /// V66 leaves `material_mark` and the index its only listing reads
    /// through in place, and takes V60's `asset_timeline_mark` away
    /// with it.
    ///
    /// Three groups of CHECK, asserted through raw SQL because raw SQL
    /// is the route they exist for:
    ///
    /// - the anchor: `anchor_kind` is closed, and `'temporal'` without
    ///   a `start_ms` is refused — the conditional NOT NULL that a
    ///   NOT NULL cannot express;
    /// - the span: `end_ms` equal to or below `start_ms`, and a
    ///   negative `start_ms`;
    /// - the author: the pairing of `author_kind` with
    ///   `author_persona_id`, plus the FK being `CASCADE` where V15's
    ///   `asset_comment` is `SET NULL`. That last one is the part worth
    ///   pinning: `SET NULL` runs as an UPDATE that breaks the pairing
    ///   CHECK and takes the whole `DELETE FROM persona` down with it.
    #[test]
    fn v66_replaces_asset_timeline_mark_with_material_mark() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        fn objects(conn: &Connection, kind: &str, name: &str) -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
                params![kind, name],
                |r| r.get(0),
            )
            .unwrap()
        }
        assert_eq!(objects(&conn, "table", "material_mark"), 1);
        assert_eq!(
            objects(&conn, "index", "idx_material_mark_asset_start"),
            1,
            "the listing index is what makes a per-asset timeline read cheap"
        );
        assert_eq!(
            objects(&conn, "table", "asset_timeline_mark"),
            0,
            "V60's table is replaced, not left beside its successor"
        );
        assert_eq!(
            objects(&conn, "index", "idx_asset_timeline_mark_asset_start"),
            0,
            "and its index goes with it"
        );

        let author = seed_persona(&conn);
        // A second persona owning the asset, so the author FK is the
        // only path from the author to the mark.
        let owner = Uuid::now_v7();
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, 'owner', 'O', 0, 0)",
            params![owner],
        )
        .unwrap();
        let asset = seed_asset(&conn, owner);
        // Since V78 a mark belongs to a band, so the fixture opens the
        // one the service would have opened on the first post. The
        // CHECKs under test are still V66's; this is the row they now
        // hang off.
        let layer = Uuid::now_v7();
        conn.execute(
            "INSERT INTO material_layer \
                 (id, asset_id, material_ord, origin, role, is_default, ord) \
             VALUES (?1, ?2, 0, 'user', 'annotation', 1, 0)",
            params![layer, asset],
        )
        .unwrap();

        let insert = |id: Uuid,
                      anchor: &str,
                      start: Option<i64>,
                      end: Option<i64>,
                      kind: &str,
                      pid: Option<Uuid>| {
            conn.execute(
                "INSERT INTO material_mark \
                     (id, asset_id, layer_id, anchor_kind, start_ms, end_ms, body, author_kind, \
                      author_persona_id, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'here', ?7, ?8, 0)",
                params![id, asset, layer, anchor, start, end, kind, pid],
            )
        };

        let instant = Uuid::now_v7();
        insert(instant, "temporal", Some(1_000), None, "user", None).unwrap();
        insert(
            Uuid::now_v7(),
            "temporal",
            Some(1_000),
            Some(1_001),
            "user",
            None,
        )
        .unwrap();

        assert!(
            insert(
                Uuid::now_v7(),
                "temporal",
                Some(1_000),
                Some(1_000),
                "user",
                None
            )
            .is_err(),
            "end_ms == start_ms covers nothing"
        );
        assert!(
            insert(
                Uuid::now_v7(),
                "temporal",
                Some(1_000),
                Some(999),
                "user",
                None
            )
            .is_err(),
            "inverted interval"
        );
        assert!(
            insert(Uuid::now_v7(), "temporal", Some(-1), None, "user", None).is_err(),
            "negative start — the receiver for a write-side conversion that wrapped"
        );
        assert!(
            insert(Uuid::now_v7(), "temporal", None, None, "user", None).is_err(),
            "a temporal anchor with nowhere to point"
        );
        assert!(
            insert(Uuid::now_v7(), "spatial", Some(0), None, "user", None).is_err(),
            "an anchor kind the schema does not carry columns for yet"
        );
        assert!(
            insert(Uuid::now_v7(), "temporal", Some(0), None, "persona", None).is_err(),
            "persona author without an id"
        );
        assert!(
            insert(
                Uuid::now_v7(),
                "temporal",
                Some(0),
                None,
                "user",
                Some(author)
            )
            .is_err(),
            "user author carrying an id"
        );
        assert!(
            insert(Uuid::now_v7(), "temporal", Some(0), None, "ghost", None).is_err(),
            "unknown author kind"
        );

        // The author's own mark, on someone else's asset.
        let by_author = Uuid::now_v7();
        insert(
            by_author,
            "temporal",
            Some(2_000),
            None,
            "persona",
            Some(author),
        )
        .unwrap();

        conn.execute("DELETE FROM persona WHERE id = ?1", params![author])
            .expect("SET NULL here would abort on the pairing CHECK; CASCADE does not");

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM material_mark WHERE id = ?1",
                params![by_author],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "the author's mark goes with the author");
        let others: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM material_mark WHERE id = ?1",
                params![instant],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(others, 1, "and nothing else does");
    }

    /// V68 in one delete, with the version before it standing beside the
    /// version after so the difference is the assertion rather than a
    /// claim about it.
    ///
    /// At `user_version = 67` the `DELETE FROM persona` **fails**: the
    /// `SET NULL` runs as an `UPDATE`, the pairing `CHECK` refuses the
    /// half-cleared row, and the refusal takes the delete with it. The
    /// row count afterwards is what shows the Persona is still there —
    /// which is the bug (`cb2d2273`): a Persona that wrote one comment
    /// could never be purged, and the retention sweep skipped it
    /// silently on every pass.
    ///
    /// The same delete after the step succeeds and the comment stays,
    /// now naming a Persona nobody can look up.
    ///
    /// The comment is seeded **before** the migration, so the rebuild's
    /// `INSERT … SELECT` has a Persona-authored row to carry across.
    #[test]
    fn v68_lets_a_persona_be_purged_and_keeps_what_it_wrote() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 67).unwrap();

        let author = seed_persona(&conn);
        // A second Persona owns the asset, so the author FK is the only
        // path from the author to the comment — otherwise the asset
        // cascade would take the comment and prove nothing. Spelled out
        // rather than `seed_persona` twice: `pack_id` is UNIQUE and the
        // helper hardcodes one.
        let owner = Uuid::now_v7();
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, 'owner', 'O', 0, 0)",
            params![owner],
        )
        .unwrap();
        let asset = seed_asset(&conn, owner);
        let comment = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset_comment \
                 (id, asset_id, author_kind, author_persona_id, body, created_at) \
             VALUES (?1, ?2, 'persona', ?3, 'worth keeping', 0)",
            params![comment, asset, author],
        )
        .unwrap();

        let purge =
            |conn: &Connection| conn.execute("DELETE FROM persona WHERE id = ?1", params![author]);
        let personas = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM persona WHERE id = ?1",
                params![author],
                |r| r.get(0),
            )
            .unwrap()
        };

        let refused = purge(&conn).expect_err("V67 cannot delete the author of a comment");
        assert!(
            refused.to_string().contains("CHECK constraint failed"),
            "the delete has to die on the pairing CHECK specifically, not on \
             some other constraint that would make this test pass for the \
             wrong reason: {refused}"
        );
        assert_eq!(personas(&conn), 1, "and the Persona is still there");

        migrate(&mut conn).unwrap();

        purge(&conn).expect("V68 relaxed the CHECK the FK action was breaking");
        assert_eq!(personas(&conn), 0);

        let (kind, persona_id, body): (String, Option<Uuid>, String) = conn
            .query_row(
                "SELECT author_kind, author_persona_id, body \
                   FROM asset_comment WHERE id = ?1",
                params![comment],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("the comment survives its author");
        assert_eq!(
            body, "worth keeping",
            "the prose is the thing that survives"
        );
        assert_eq!(
            kind, "persona",
            "a Persona wrote it, and that stays true after the Persona is gone"
        );
        assert_eq!(
            persona_id, None,
            "the identity does not survive — the row that answered it is the \
             row the User deleted"
        );
    }

    /// The half of V15's pairing rule that V68 keeps. Dropping the
    /// `CHECK` outright would have admitted a `'user'` comment carrying
    /// a persona id, which is the state no FK action ever writes and no
    /// reader knows how to render.
    #[test]
    fn v68_still_refuses_a_user_comment_carrying_a_persona_id() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let refused = conn
            .execute(
                "INSERT INTO asset_comment \
                     (id, asset_id, author_kind, author_persona_id, body, created_at) \
                 VALUES (?1, ?2, 'user', ?3, 'mine', 0)",
                params![Uuid::now_v7(), asset, persona],
            )
            .expect_err("'user' + an id is still nonsense");
        assert!(
            refused.to_string().contains("CHECK constraint failed"),
            "{refused}"
        );

        let unknown = conn
            .execute(
                "INSERT INTO asset_comment \
                     (id, asset_id, author_kind, author_persona_id, body, created_at) \
                 VALUES (?1, ?2, 'ghost', NULL, 'boo', 0)",
                params![Uuid::now_v7(), asset],
            )
            .expect_err("and the vocabulary is still closed");
        assert!(
            unknown.to_string().contains("CHECK constraint failed"),
            "{unknown}"
        );
    }

    /// V69 adds the two dimension columns, and the rows that were already
    /// there stay unmeasured.
    ///
    /// The fixture is seeded **at V68** and read after the upgrade, so
    /// the assertion is about what the step does to a library that
    /// existed before it. Seeding after the migration would assert the
    /// column default and say nothing about the upgrade.
    ///
    /// `NULL` rather than `0` is the whole point of the column shape: a
    /// zero is a measurement, sorts like one, and there is no way back
    /// from it to "nobody looked".
    #[test]
    fn v69_adds_nullable_pixel_dims_and_leaves_existing_rows_unmeasured() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 68).unwrap();
        let persona = seed_persona(&conn);
        let existing = seed_asset(&conn, persona);

        let before: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('asset')")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            !before.iter().any(|c| c == "width_px" || c == "height_px"),
            "V68 has no dimension columns, so the step below is what adds them: {before:?}"
        );

        migrate(&mut conn).unwrap();

        // Declared type and nullability, off the table itself.
        let mut stmt = conn
            .prepare("SELECT name, type, \"notnull\", dflt_value FROM pragma_table_info('asset')")
            .unwrap();
        let described: Vec<(String, String, i64, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for column in ["width_px", "height_px"] {
            let (_, kind, notnull, default) = described
                .iter()
                .find(|(name, ..)| name == column)
                .unwrap_or_else(|| panic!("V69 did not add {column}: {described:?}"));
            assert_eq!(kind, "INTEGER", "{column} is an integer column");
            assert_eq!(*notnull, 0, "{column} is nullable");
            assert_eq!(
                default.as_deref(),
                None,
                "{column} has no default — a 0 would make every legacy row \
                 claim a zero-pixel resolution"
            );
        }

        // The row that predates the step reads as unmeasured, and `NULL`
        // is asserted as `NULL` rather than through a typed getter that
        // would turn it into a zero on the way out.
        let (width, height): (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT width_px, height_px FROM asset WHERE id = ?1",
                params![existing],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (width, height),
            (None, None),
            "a V68-era asset must not come out of the upgrade claiming a size"
        );
        let nulls: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset \
                 WHERE width_px IS NULL AND height_px IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(nulls, 1, "and the whole table is in that state");
    }

    /// The marker a pre-probe import left on the content axis of a JPEG,
    /// spelled out here rather than borrowed from the domain.
    ///
    /// A fixture that built its values by calling
    /// `content_region::unsupported_format` would agree with the domain
    /// by construction, which is the one thing a fixture must not do: the
    /// rendering and the migration's literal could move together and
    /// every assertion below would keep passing over a set that is no
    /// longer the one the column holds. The two are tied together in
    /// exactly one place instead —
    /// [`the_jpeg_marker_this_migration_clears_is_the_one_the_domain_renders`].
    const STALE_JPEG_MARKER: &str = "unsupported:image/jpeg";

    /// One JPEG material at the V71 shape: a file-axis digest, whatever
    /// the caller says about the content axis, and the meta marker a
    /// pre-probe import left — which is the same string, since neither
    /// axis had a reading then.
    fn seed_jpeg_material(conn: &Connection, persona: Uuid, ord: usize, content: &str) -> Uuid {
        seed_jpeg_material_with_meta(conn, persona, ord, content, STALE_JPEG_MARKER)
    }

    /// The same, for a case that needs the two walking columns to hold
    /// different things — a row the size gate refused holds its own
    /// marker on both, and no migration here is about it.
    fn seed_jpeg_material_with_meta(
        conn: &Connection,
        persona: Uuid,
        ord: usize,
        content: &str,
        meta: &str,
    ) -> Uuid {
        let asset = seed_asset(conn, persona);
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, mime, content_hash, \
                                   content_region_hash, meta_hash, created_at, updated_at) \
             VALUES (?1, 0, ?2, 'image/jpeg', ?3, ?4, ?5, 0, 0)",
            params![
                asset,
                format!("{{\"kind\":\"file\",\"path\":\"/pics/{ord}.jpg\"}}"),
                format!("sha256:{}", "b".repeat(64)),
                content,
                meta,
            ],
        )
        .unwrap();
        asset
    }

    /// One material, and what its three hash columns hold — the file
    /// axis, the content axis, the meta axis, in that order.
    type HashRow = (Uuid, Option<String>, Option<String>, Option<String>);

    /// Every material's three hash columns, in a stable order.
    fn hash_columns(conn: &Connection) -> Vec<HashRow> {
        conn.prepare(
            "SELECT asset_id, content_hash, content_region_hash, meta_hash \
             FROM material ORDER BY asset_id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    /// V72 clears the content-axis marker that says nothing reads JPEG,
    /// and leaves every other value in the column exactly as it found it.
    ///
    /// The fixture holds one row per marker kind plus a digest, because
    /// the failure worth catching is a `WHERE` that reaches wider than
    /// the one value: a `GLOB 'unsupported:*'` would clear the size gate's
    /// answer, the empty-span answer and the deferred-walk set at the same
    /// time, and every one of those rows would be re-read to be written
    /// back exactly as it was — except `unsupported:not-walked`, which
    /// would land in two passes at once.
    ///
    /// Asserted row by row rather than as a count, so a statement that
    /// cleared the right *number* of rows and the wrong ones fails here.
    #[test]
    fn v72_clears_the_stale_jpeg_content_marker_and_leaves_every_other_answer() {
        use asterism_core::domain::content_hash::{CONTENT_DIGEST_PREFIX, UNHASHABLE};
        use asterism_core::domain::content_region::{EMPTY_SPAN, NOT_WALKED, TOO_LARGE};

        let mut conn = test_conn();
        migrate_to(&mut conn, 71).unwrap();
        let persona = seed_persona(&conn);

        let digest = format!("{CONTENT_DIGEST_PREFIX}{}", "a".repeat(64));
        let stored = [
            STALE_JPEG_MARKER,
            TOO_LARGE,
            EMPTY_SPAN,
            NOT_WALKED,
            // The rest of the `unsupported:<mime>` family, and the label
            // for a file nobody named: same shape as the stale value, and
            // no probe arrived for either.
            "unsupported:video/mp4",
            "unsupported:unknown",
            UNHASHABLE,
            digest.as_str(),
        ];
        let seeded: Vec<(Uuid, &str)> = stored
            .iter()
            .enumerate()
            .map(|(ord, value)| (seed_jpeg_material(&conn, persona, ord, value), *value))
            .collect();

        // The fixture starts with every row answered, so that a cleared
        // column below is this migration's doing and not a seed.
        assert!(
            hash_columns(&conn)
                .iter()
                .all(|(_, _, content, _)| content.is_some()),
            "no row starts NULL on the content axis"
        );

        migrate(&mut conn).unwrap();

        // The chain run above includes V92, which moved every surviving
        // marker into the status and reason columns — so the end state
        // asserted per row is the split form of exactly the value V72
        // left, or the `pending` a cleared row converts to.
        for (asset, before) in &seeded {
            let expected: (&str, Option<&str>, Option<&str>) = match *before {
                STALE_JPEG_MARKER => ("pending", None, None),
                TOO_LARGE => ("too-large", None, None),
                EMPTY_SPAN => ("empty-span", None, None),
                NOT_WALKED => ("not-walked", None, None),
                "unsupported:video/mp4" => ("unsupported", None, Some("video/mp4")),
                "unsupported:unknown" => ("unsupported", None, Some("unknown")),
                UNHASHABLE => ("no-bytes", None, None),
                kept => ("computed", Some(kept), None),
            };
            let (status, value, reason) = axis_state_of(&conn, *asset, "content_region_hash");
            assert_eq!(
                (status.as_str(), value.as_deref(), reason.as_deref()),
                expected,
                "{before} is not this step's business and must survive it \
                 (as its post-V92 form)"
            );
            // The meta axis holds the same string on every one of these
            // rows, and V72 does not touch it — the JPEG probe declared
            // `meta: false` when this step was written, so clearing it
            // would have asked for a reading nobody had. **V76 clears
            // it**, one slice later and for the same reason on its own
            // axis, and the chain run above includes that step: what is
            // asserted here is therefore the end state of both, and the
            // selectivity of V76's own `WHERE` is
            // [`v76_clears_the_stale_jpeg_meta_marker_and_leaves_every_other_answer`].
            assert_eq!(
                axis_state_of(&conn, *asset, "meta_hash"),
                ("pending".to_string(), None, None),
                "V76 cleared the meta column"
            );
        }
    }

    /// The point of the step: a row it cleared is picked up by the
    /// ordinary fingerprint walk, and was not before.
    ///
    /// Asserted against both evaluations of the rule — the domain
    /// predicate and the SQL the backfill's page query and the progress
    /// count are built from — rather than trusted from
    /// [`needs_fingerprint`](asterism_core::domain::content_hash::needs_fingerprint)'s
    /// docstring, because "the marker is a final answer" is exactly the
    /// sentence that made these rows invisible in the first place.
    ///
    /// The row that must **not** move is in the fixture for the same
    /// reason the pre-migration assertion is: without them the test would
    /// pass over a migration that cleared the whole column.
    #[test]
    fn v72_hands_the_cleared_jpeg_row_to_the_ordinary_fingerprint_walk() {
        use asterism_core::domain::content_region::TOO_LARGE;

        let mut conn = test_conn();
        migrate_to(&mut conn, 71).unwrap();
        let persona = seed_persona(&conn);
        let stale = seed_jpeg_material(&conn, persona, 0, STALE_JPEG_MARKER);
        // Both columns, because that is what the size gate writes: a
        // file it declined to read carries `too-large` on each axis. It
        // also keeps this row out of V76's `WHERE`, so it stays the
        // control it was written to be after the chain grew a second
        // clearing step.
        let gated = seed_jpeg_material_with_meta(&conn, persona, 1, TOO_LARGE, TOO_LARGE);

        // The pre-upgrade half runs in the marker era's own spellings —
        // a database parked at V71 has no status columns — and the
        // post-upgrade half in production's, which is the era the walk
        // actually runs in after the chain.
        let marker_owes = |conn: &Connection, asset: Uuid| -> bool {
            let (file, content, meta): (Option<String>, Option<String>, Option<String>) = conn
                .query_row(
                    "SELECT content_hash, content_region_hash, meta_hash \
                     FROM material WHERE asset_id = ?1",
                    params![asset],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            marker_era_owes(file.as_deref(), content.as_deref(), meta.as_deref())
        };
        let selected = |conn: &Connection, condition: &str| -> Vec<Uuid> {
            conn.prepare(&format!(
                "SELECT asset_id FROM material WHERE {condition} ORDER BY asset_id"
            ))
            .unwrap()
            .query_map([], |row| row.get::<_, Uuid>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
        };

        assert!(
            !marker_owes(&conn, stale),
            "before the step the marker reads as a final answer — that is the bug"
        );
        assert!(!marker_owes(&conn, gated));
        assert!(
            selected(
                &conn,
                &marker_era_unfingerprinted("content_hash", "content_region_hash", "meta_hash")
            )
            .is_empty(),
            "and the walk's own query agrees, so nothing is work yet"
        );

        migrate(&mut conn).unwrap();

        assert!(
            status_era_owes(&conn, stale),
            "a JPEG imported before the probe now owes a pass"
        );
        assert!(
            !status_era_owes(&conn, gated),
            "and the file the size gate refused still does not"
        );
        assert_eq!(
            selected(&conn, &status_era_unfingerprinted()),
            vec![stale],
            "the walk selects exactly the row the step cleared"
        );
    }

    /// Teeth on the frozen literal: the string V72 matches on is the one
    /// the domain renders for `image/jpeg`.
    ///
    /// The migration names it verbatim and must go on naming it verbatim
    /// — asking the probe registry which mimes are claimed would make one
    /// numbered step clear a different set on every database, growing as
    /// probes land, with no way to reconstruct why a row is NULL. The
    /// hazard a hand-typed literal carries instead is drift, and this is
    /// where it is caught: read the literal out of the statement itself
    /// and compare it with what the code writes.
    ///
    /// The parse is the part that can go quietly wrong, so it is an
    /// `expect` rather than a `contains` — a `WHERE` rewritten as a
    /// `GLOB`, or moved onto another column, fails here rather than
    /// leaving the comparison to run against nothing.
    ///
    /// And it reads the predicate **whole**, from the `WHERE` to the
    /// semicolon, rather than lifting the first literal out of it. A
    /// second disjunct is otherwise invisible: this test would go on
    /// comparing the literal it still finds first, and no other test in
    /// the file holds a row a plausible widening would reach. Appending
    /// `OR content_region_hash GLOB 'unsupported:image/*'` leaves the
    /// row-by-row fixture green — its other values are `video/mp4`,
    /// `unknown`, the two walk answers, the no-bytes answer and a digest,
    /// not one of them an image — while on somebody's library it clears
    /// every GIF, WebP and HEIC marker as well, and each of those rows is
    /// then read off disk whole so that the walk can write back the value
    /// it already held.
    /// **Both steps are read, and each is asserted to name its own
    /// column and no other.** They are the same statement over two
    /// axes, so a widening copied from one to the other — the plausible
    /// edit, since the second was written by looking at the first — is
    /// exactly what the column assertions below refuse.
    #[test]
    fn the_jpeg_marker_this_migration_clears_is_the_one_the_domain_renders() {
        use asterism_core::domain::content_region::unsupported_format;
        use asterism_core::domain::value::{ImageFormat, MimeType};

        // The marker-era spelling the frozen statements name, rebuilt
        // from the domain's own rendering of the format so the
        // drift-catch survives the V92 representation change.
        let rendered = format!(
            "unsupported:{}",
            unsupported_format(Some(&MimeType::Image(ImageFormat::Jpeg)))
                .record()
                .reason
                .expect("an unsupported outcome names its format")
        );
        let cleared = |version: &str, statement: &str, column: &str| {
            let predicate = statement
                .split_once("WHERE")
                .unwrap_or_else(|| {
                    panic!("{version} selects the rows it clears rather than emptying the column")
                })
                .1
                .trim()
                .trim_end_matches(';')
                .trim_end();
            let (matched, beside_it) = predicate
                .strip_prefix(&format!("{column} = '"))
                .unwrap_or_else(|| {
                    panic!("{version} matches `{column}` by equality against a literal")
                })
                .split_once('\'')
                .expect("and the literal is closed");
            assert!(
                beside_it.is_empty(),
                "{version}'s `WHERE` names one comparison and `{beside_it}` follows it — \
                 whatever that widens to is cleared on every library, and no fixture in \
                 this file holds a row to show it"
            );
            assert_eq!(
                matched, rendered,
                "{version} clears `{matched}` while an unclaimed image/jpeg is stored as \
                 `{rendered}` — one of the two moved, and a migration cannot follow"
            );
            // The fixtures above are written against the same string, and
            // they spell it themselves rather than calling the domain.
            assert_eq!(matched, STALE_JPEG_MARKER);
        };

        cleared(
            "V72",
            V72_CLEAR_STALE_JPEG_CONTENT_MARKER,
            "content_region_hash",
        );
        cleared("V76", V76_CLEAR_STALE_JPEG_META_MARKER, "meta_hash");

        // One column each, and the one each says. `content_region_hash`
        // does not contain `content_hash` as a substring, so every line
        // below is a real assertion about which columns a statement
        // names.
        assert!(
            !V72_CLEAR_STALE_JPEG_CONTENT_MARKER.contains("meta_hash"),
            "the meta axis is V76's to clear, four versions later"
        );
        assert!(
            !V76_CLEAR_STALE_JPEG_META_MARKER.contains("content_region_hash"),
            "the content axis was cleared by V72 and holds a real answer now"
        );
        for statement in [
            V72_CLEAR_STALE_JPEG_CONTENT_MARKER,
            V76_CLEAR_STALE_JPEG_META_MARKER,
        ] {
            assert!(
                !statement.contains("content_hash"),
                "the file axis holds a digest that is still true of the bytes"
            );
        }
    }

    /// **V76 clears the meta-axis marker that says nothing reads JPEG,
    /// and leaves every other value in that column exactly as it found
    /// it** — [`v72_clears_the_stale_jpeg_content_marker_and_leaves_every_other_answer`]
    /// one axis over.
    ///
    /// The fixture holds one row per marker kind plus a digest, for the
    /// reason the content-axis version gives: a `WHERE` that reached
    /// wider than the one value would clear the size gate's answer, the
    /// empty-span answer and the deferred-walk set at the same time, and
    /// every one of those rows would then be read off disk to be written
    /// back exactly as it was.
    ///
    /// **`unsupported:empty-span` is the row to watch here**, and it is
    /// not the same case it was on the content axis. Every JPEG the
    /// probe reads from now on that carries no EXIF — 246 of 250 sampled
    /// from a real download directory — stores exactly that, so a
    /// widening to `GLOB 'unsupported:*'` would re-read most of a photo
    /// library on every upgrade to write back the value it already had.
    #[test]
    fn v76_clears_the_stale_jpeg_meta_marker_and_leaves_every_other_answer() {
        use asterism_core::domain::content_hash::{META_DIGEST_PREFIX, UNHASHABLE};
        use asterism_core::domain::content_region::{EMPTY_SPAN, NOT_WALKED, TOO_LARGE};

        let mut conn = test_conn();
        migrate_to(&mut conn, 75).unwrap();
        let persona = seed_persona(&conn);

        let digest = format!("{META_DIGEST_PREFIX}{}", "a".repeat(64));
        let stored = [
            STALE_JPEG_MARKER,
            TOO_LARGE,
            EMPTY_SPAN,
            NOT_WALKED,
            "unsupported:video/mp4",
            "unsupported:unknown",
            UNHASHABLE,
            digest.as_str(),
        ];
        let seeded: Vec<(Uuid, &str)> = stored
            .iter()
            .enumerate()
            .map(|(ord, value)| {
                // The content column holds a digest on every row, so
                // nothing below can be this step reaching the wrong axis.
                let content = format!("cr1-sha256:{}", "e".repeat(64));
                (
                    seed_jpeg_material_with_meta(&conn, persona, ord, &content, value),
                    *value,
                )
            })
            .collect();

        assert!(
            hash_columns(&conn)
                .iter()
                .all(|(_, _, _, meta)| meta.is_some()),
            "no row starts NULL on the meta axis"
        );

        migrate(&mut conn).unwrap();

        // As in V72's twin: V92 runs later in the same chain, so the end
        // state per row is the split form of the value V76 left.
        for (asset, before) in &seeded {
            let expected: (&str, Option<&str>, Option<&str>) = match *before {
                STALE_JPEG_MARKER => ("pending", None, None),
                TOO_LARGE => ("too-large", None, None),
                EMPTY_SPAN => ("empty-span", None, None),
                NOT_WALKED => ("not-walked", None, None),
                "unsupported:video/mp4" => ("unsupported", None, Some("video/mp4")),
                "unsupported:unknown" => ("unsupported", None, Some("unknown")),
                UNHASHABLE => ("no-bytes", None, None),
                kept => ("computed", Some(kept), None),
            };
            let (status, value, reason) = axis_state_of(&conn, *asset, "meta_hash");
            assert_eq!(
                (status.as_str(), value.as_deref(), reason.as_deref()),
                expected,
                "{before} is not this step's business and must survive it \
                 (as its post-V92 form)"
            );
            let (_, content, _) = axis_state_of(&conn, *asset, "content_region_hash");
            assert!(
                content.is_some_and(|value| value.starts_with("cr1-")),
                "the meta column only"
            );
        }
    }

    /// The point of the step: a row it cleared is picked up by the
    /// ordinary fingerprint walk, and was not before.
    ///
    /// [`v72_hands_the_cleared_jpeg_row_to_the_ordinary_fingerprint_walk`]'s
    /// twin, and the interesting half is the control: the row that must
    /// not move here holds a **content** digest and the stale meta
    /// marker, which is the state a JPEG imported between V72 and this
    /// step is in. Its content column is a real answer, so only the meta
    /// clearing can make it work — a step that cleared nothing would
    /// leave it settled, and one that cleared by prefix would take the
    /// size-gated row with it.
    #[test]
    fn v76_hands_the_cleared_jpeg_row_to_the_ordinary_fingerprint_walk() {
        use asterism_core::domain::content_region::TOO_LARGE;

        let mut conn = test_conn();
        migrate_to(&mut conn, 75).unwrap();
        let persona = seed_persona(&conn);
        let walked = format!("cr1-sha256:{}", "e".repeat(64));
        let stale = seed_jpeg_material_with_meta(&conn, persona, 0, &walked, STALE_JPEG_MARKER);
        let gated = seed_jpeg_material_with_meta(&conn, persona, 1, TOO_LARGE, TOO_LARGE);

        // Era-appropriate spellings on each side of the chain, as in
        // V72's twin above.
        let marker_owes = |conn: &Connection, asset: Uuid| -> bool {
            let (file, content, meta): (Option<String>, Option<String>, Option<String>) = conn
                .query_row(
                    "SELECT content_hash, content_region_hash, meta_hash \
                     FROM material WHERE asset_id = ?1",
                    params![asset],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            marker_era_owes(file.as_deref(), content.as_deref(), meta.as_deref())
        };
        let selected = |conn: &Connection, condition: &str| -> Vec<Uuid> {
            conn.prepare(&format!(
                "SELECT asset_id FROM material WHERE {condition} ORDER BY asset_id"
            ))
            .unwrap()
            .query_map([], |row| row.get::<_, Uuid>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
        };

        assert!(
            !marker_owes(&conn, stale),
            "before the step the marker reads as a final answer — that is the bug"
        );
        assert!(!marker_owes(&conn, gated));
        assert!(
            selected(
                &conn,
                &marker_era_unfingerprinted("content_hash", "content_region_hash", "meta_hash")
            )
            .is_empty(),
            "nothing is work yet"
        );

        migrate(&mut conn).unwrap();

        assert!(
            status_era_owes(&conn, stale),
            "a JPEG whose metadata nobody read now owes a pass"
        );
        assert!(
            !status_era_owes(&conn, gated),
            "and the file the size gate refused still does not"
        );
        assert_eq!(
            selected(&conn, &status_era_unfingerprinted()),
            vec![stale],
            "the walk selects exactly the row the step cleared"
        );
    }

    /// **The widened `CHECK` admits `exif` and still refuses a token no
    /// decoder spells** — and the rows the rebuild carried are all
    /// there.
    ///
    /// Three things a table rebuild can lose, asserted one at a time:
    /// the rows it copies, the constraint it exists to widen, and the
    /// constraint it must not drop while widening it. The seeded VDSL
    /// rule is the row, and it is checked field by field rather than
    /// counted, because a `SELECT` that named the columns in the wrong
    /// order copies a table without losing a row.
    #[test]
    fn v77_widens_the_decoder_vocabulary_without_losing_the_rules_it_carried() {
        use asterism_core::domain::series::Decode;

        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let seeded: (String, String, String, String, String, i64, i64, i64) = conn
            .query_row(
                "SELECT name, applies_to, decode, include, exclude, system, created_at, updated_at \
                   FROM series_strategy WHERE id = ?1",
                params![Uuid::parse_str(VDSL_STRATEGY_ID).unwrap()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .expect("the rule V73 seeded survived the rebuild");
        assert_eq!(
            seeded,
            (
                "VDSL recipe".to_string(),
                "image/png".to_string(),
                "raw_json".to_string(),
                r#"[["vdsl","script"]]"#.to_string(),
                "[]".to_string(),
                1,
                1_786_320_000_000,
                1_786_320_000_000,
            ),
            "every column, in the shape V73 wrote it"
        );
        // …including the pair a later corrective migration addresses the
        // seed by, which a rebuild that re-stamped `updated_at` would
        // have broken silently.
        assert_eq!(seeded.6, seeded.7, "the seed still reads as untouched");

        let register = |decode: &str| -> Result<usize, rusqlite::Error> {
            conn.execute(
                "INSERT INTO series_strategy \
                     (id, name, applies_to, decode, include, exclude, system, created_at, updated_at) \
                 VALUES (?1, 'under test', 'image/jpeg', ?2, '[]', '[]', 0, 1, 1)",
                params![Uuid::now_v7(), decode],
            )
        };

        for decode in Decode::ALL {
            register(decode.as_str())
                .unwrap_or_else(|err| panic!("the schema refuses `{}`: {err}", decode.as_str()));
        }
        let refused =
            register("prose_pairs").expect_err("a decoder nothing ships must not be storable");
        assert!(
            refused.to_string().contains("CHECK"),
            "refused by the constraint rather than by something else: {refused}"
        );
    }

    /// **The rebuild keeps the derived rows it has no business
    /// touching.**
    ///
    /// `material_series.strategy_id` points at the rebuilt table with
    /// `ON DELETE CASCADE`, and with foreign keys enabled `DROP TABLE`
    /// runs an implicit `DELETE FROM` that fires them — so a step
    /// written as an ordinary SQL batch would empty a library's derived
    /// keys while claiming to widen a constraint. [`Step::App`] turns
    /// foreign keys off around its transaction, and this is that
    /// difference made visible.
    ///
    /// The second half is that the reference still holds afterwards: the
    /// rename has to leave `material_series` pointing at the new table,
    /// or the cascade this test just protected stops working for the
    /// deletion it is actually for.
    #[test]
    fn v77_keeps_the_derived_rows_it_has_no_business_touching() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 76).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let persona = seed_persona(&conn);
        let strategy = Uuid::parse_str(VDSL_STRATEGY_ID).unwrap();
        let asset = seed_asset(&conn, persona);
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, mime, created_at, updated_at) \
             VALUES (?1, 0, '{\"kind\":\"file\",\"path\":\"/pics/a.png\"}', 'image/png', 0, 0)",
            params![asset],
        )
        .unwrap();
        let key = format!("sk1-sha256:{}", "a".repeat(64));
        conn.execute(
            "INSERT INTO material_series (asset_id, ord, strategy_id, key, outcome, derived_at) \
             VALUES (?1, 0, ?2, ?3, 'derived', 0)",
            params![asset, strategy, key],
        )
        .unwrap();

        let derived = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM material_series", [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(derived(&conn), 1);

        migrate(&mut conn).unwrap();
        assert_eq!(
            derived(&conn),
            1,
            "the rebuild dropped the parent table and the cascade took the key with it"
        );

        // And the reference survives the rename, which is the same
        // cascade doing the job it is for.
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute(
            "DELETE FROM series_strategy WHERE id = ?1",
            params![strategy],
        )
        .unwrap();
        assert_eq!(
            derived(&conn),
            0,
            "deleting a rule still takes its derived keys with it"
        );
    }

    /// Running the chain again changes nothing — including the statement
    /// itself, not merely the `user_version` guard that skips it.
    ///
    /// The second half is what carries the claim. A second [`migrate`] on
    /// an up-to-date database executes no batch at all, so a test that
    /// stopped there would hold for any statement whatsoever; re-running
    /// the batch by hand is what shows the `UPDATE` has nothing left to
    /// match once the marker is gone.
    #[test]
    fn v72_changes_nothing_the_second_time_it_runs() {
        use asterism_core::domain::content_hash::CONTENT_DIGEST_PREFIX;
        use asterism_core::domain::content_region::{NOT_WALKED, TOO_LARGE};

        let mut conn = test_conn();
        migrate_to(&mut conn, 71).unwrap();
        let persona = seed_persona(&conn);
        let digest = format!("{CONTENT_DIGEST_PREFIX}{}", "c".repeat(64));
        for (ord, value) in [STALE_JPEG_MARKER, TOO_LARGE, NOT_WALKED, digest.as_str()]
            .iter()
            .enumerate()
        {
            seed_jpeg_material(&conn, persona, ord, value);
        }

        migrate(&mut conn).unwrap();
        let settled = hash_columns(&conn);
        assert!(
            settled.iter().any(|(_, _, content, _)| content.is_none()),
            "the first run cleared something, so the comparisons below are not vacuous"
        );

        migrate(&mut conn).unwrap();
        assert_eq!(hash_columns(&conn), settled, "the chain is a no-op now");

        conn.execute_batch(V72_CLEAR_STALE_JPEG_CONTENT_MARKER)
            .unwrap();
        assert_eq!(
            hash_columns(&conn),
            settled,
            "and so is the statement: a cleared row no longer matches it"
        );

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    /// V78 gives every already-marked asset a band and fastens its
    /// marks to it — and gives an unmarked asset nothing.
    ///
    /// Seeded at V77 in the shape that version's `material_mark`
    /// actually has (no `layer_id` column), so the assertions are about
    /// the step rather than about a fixture written against the new
    /// schema. Two assets on purpose: one with marks, one without. With
    /// only the first, a backfill that opened a band for *every* asset
    /// would pass — and on a real library that is a hundred thousand
    /// empty rows.
    ///
    /// The two marks on one asset are the other half: they have to end
    /// up in the *same* band, not one each, which is what makes it a
    /// default rather than a per-mark wrapper.
    #[test]
    fn v78_gives_every_marked_asset_one_band_and_keeps_its_marks() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 77).unwrap();
        let persona = seed_persona(&conn);
        let marked = seed_asset(&conn, persona);
        let untouched = seed_asset(&conn, persona);

        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        for (id, start) in [(first, 1_000_i64), (second, 5_000)] {
            conn.execute(
                "INSERT INTO material_mark (id, asset_id, anchor_kind, start_ms, end_ms, \
                                            body, author_kind, author_persona_id, created_at) \
                 VALUES (?1, ?2, 'temporal', ?3, NULL, 'here', 'user', NULL, 0)",
                params![id, marked, start],
            )
            .unwrap();
        }

        migrate_to(&mut conn, 78).unwrap();

        let bands: Vec<(Uuid, Uuid, i64, String, String, i64, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, asset_id, material_ord, origin, role, is_default, ord \
                       FROM material_layer",
                )
                .unwrap();
            stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
        };
        assert_eq!(bands.len(), 1, "one band, and only for the marked asset");
        let (band_id, band_asset, material_ord, origin, role, is_default, ord) = bands[0].clone();
        assert_eq!(band_asset, marked);
        assert_eq!(material_ord, 0);
        assert_eq!(
            (origin.as_str(), role.as_str()),
            ("user", "annotation"),
            "a band a person will write notes into has to be theirs to write into"
        );
        assert_eq!((is_default, ord), (1, 0));

        let placed: Vec<(Uuid, Uuid, i64)> = {
            let mut stmt = conn
                .prepare("SELECT id, layer_id, start_ms FROM material_mark ORDER BY start_ms")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            placed,
            vec![(first, band_id, 1_000), (second, band_id, 5_000)],
            "both marks survive the rebuild, in the one band"
        );

        // Nothing was opened for the asset that had no marks.
        let for_untouched: i64 = conn
            .query_row(
                "SELECT count(*) FROM material_layer WHERE asset_id = ?1",
                params![untouched],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(for_untouched, 0);

        // The column the whole step exists for refuses a mark with no
        // band — the property `NOT NULL` is carrying, asserted rather
        // than assumed from the DDL text.
        assert!(
            conn.execute(
                "INSERT INTO material_mark (id, asset_id, layer_id, anchor_kind, start_ms, \
                                            body, author_kind, created_at) \
                 VALUES (?1, ?2, NULL, 'temporal', 0, 'orphan', 'user', 0)",
                params![Uuid::now_v7(), marked],
            )
            .is_err(),
            "a mark that belongs to no band is the state layers exist to remove"
        );
    }

    /// V78 refuses a second default band on one `(asset, material,
    /// role)`, and refuses a default annotation band that is not the
    /// user's.
    ///
    /// Both rules are cross-checked in Rust as well — the first in the
    /// service, the second in `MaterialLayer::validate` — so what is
    /// asserted here is that the *schema* holds them: a row arriving by
    /// a route that skips the domain (a hand-written `INSERT`, a future
    /// migration) is refused by the database itself.
    ///
    /// The legal rows are inserted first in each pair. Without them the
    /// test would pass against a schema that refused every insert.
    #[test]
    fn v78_holds_the_two_rules_the_domain_cannot_hold_alone() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);

        let band = |origin: &str, role: &str, is_default: i64, material_ord: i64| {
            conn.execute(
                "INSERT INTO material_layer \
                     (id, asset_id, material_ord, origin, role, is_default, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![
                    Uuid::now_v7(),
                    asset,
                    material_ord,
                    origin,
                    role,
                    is_default
                ],
            )
        };

        band("imported", "structure", 1, 0).expect("the file's own list may be the default");
        band("user", "annotation", 1, 0).expect("so may the user's own notes — a different role");
        band("user", "structure", 0, 0).expect("a second structure band that is not the default");
        band("imported", "structure", 1, 1)
            .expect("and a default over a *second* material is a different triple");

        assert!(
            band("user", "structure", 1, 0).is_err(),
            "two default structure bands over one material is the state the index forbids"
        );
        assert!(
            band("imported", "annotation", 1, 1).is_err(),
            "a note lands in the default annotation band, so it cannot be one nobody may write to"
        );
        band("imported", "annotation", 0, 1)
            .expect("the same band is fine as long as it is not the default");
    }

    /// V78 admits one `'imported'` band per `(asset, material, role)`
    /// and refuses the second — while leaving the user's own bands
    /// uncounted.
    ///
    /// This is the rule that makes two concurrent scans of one asset
    /// safe. `imported_structure_layer` looks for the band before it
    /// opens one, so two jobs interleaved between that read and that
    /// write both decide to open it; the index is what turns the
    /// loser's write into an error it recovers from by re-reading,
    /// instead of a duplicate band that nothing reads and no verb
    /// removes. Asserted here rather than in the service, because a
    /// service test cannot produce the interleaving — the schema is
    /// where the property actually lives.
    ///
    /// The user rows go in *after* the refusal on purpose: they are
    /// what shows the index is keyed on `origin` rather than simply
    /// forbidding a second structure band.
    #[test]
    fn v78_admits_one_imported_band_per_material_and_role() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);

        let band = |origin: &str, role: &str, is_default: i64, material_ord: i64| {
            conn.execute(
                "INSERT INTO material_layer \
                     (id, asset_id, material_ord, origin, role, is_default, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![
                    Uuid::now_v7(),
                    asset,
                    material_ord,
                    origin,
                    role,
                    is_default
                ],
            )
        };

        band("imported", "structure", 1, 0).expect("the file's own chapter list");
        assert!(
            band("imported", "structure", 0, 0).is_err(),
            "a second copy of the file's own list is the duplicate two racing scans would leave"
        );

        // Same origin, but a different triple in each case.
        band("imported", "annotation", 0, 0).expect("a different role is a different band");
        band("imported", "structure", 0, 1).expect("and so is a second material");

        // The person may keep as many passes of their own as they like:
        // those are bands somebody made, not one fact read twice.
        band("user", "structure", 0, 0).expect("the user's own chapters");
        band("user", "structure", 0, 0).expect("and a second pass over the same material");
    }

    /// Teeth: every step is named for the version it produces.
    ///
    /// `MIGRATIONS[i]` upgrades a database from `i` to `i + 1`, and the
    /// name says which — `V1_INITIAL_SCHEMA` is `MIGRATIONS[0]`. Nothing
    /// in the type system holds that, and the case where it breaks is
    /// exactly the one this file just went through: a step inserted in
    /// the middle renumbers every step after it, and a name left behind
    /// is a lie in the first place a reader looks.
    ///
    /// Read off this file's own source — names rather than meanings, and
    /// no build dependency (the same trade
    /// `asterism-core/tests/attribution_guards.rs` makes).
    #[test]
    fn every_step_is_named_for_the_version_it_produces() {
        let list = include_str!("migrations.rs")
            .split_once("const MIGRATIONS: &[Step] = &[")
            .expect("the list is declared in this file")
            .1
            .split_once("\n];")
            .expect("and it is closed")
            .0;

        let mut named = 0usize;
        let mut mismatched: Vec<String> = Vec::new();
        for line in list.lines() {
            let line = line.trim();
            let Some(rest) = line
                .strip_prefix("Step::Sql(")
                .or_else(|| line.strip_prefix("Step::App("))
            else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let produces = named + 1;
            if !name.starts_with(&format!("V{produces}_"))
                && !name.starts_with(&format!("v{produces}_"))
            {
                mismatched.push(format!(
                    "MIGRATIONS[{named}] takes a database to version {produces} but is named `{name}`"
                ));
            }
            named += 1;
        }

        // The parse is the thing that can go silently wrong here: a
        // reader that matched nothing would report no mismatches.
        assert_eq!(
            named,
            MIGRATIONS.len(),
            "the source parse found {named} steps against {} in the list",
            MIGRATIONS.len()
        );
        assert!(
            mismatched.is_empty(),
            "a step's name and its index disagree. The index is the \
             `user_version` it writes, so the name is what has to move: {mismatched:#?}"
        );
    }

    /// V79 backfills one single-round pursuit per existing dispatch —
    /// and nothing more. Copied `persona_id` / `created_at`, NULL
    /// attribution (nobody opened these pursuits; the migration did),
    /// no grouping: two dispatches that could plausibly be one line of
    /// work still get two pursuits, because grouping-by-heuristic is
    /// the inferred correlation the design forbids (#29).
    #[test]
    fn v79_backfills_one_single_round_pursuit_per_dispatch() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 78).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'cafe', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snapshot_asset (snapshot_id, asset_id, position) \
             VALUES (?1, ?2, 0)",
            params![snapshot, asset],
        )
        .unwrap();
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        for (id, created) in [(first, 1_000_i64), (second, 5_000)] {
            conn.execute(
                "INSERT INTO dispatch_job (id, snapshot_id, persona_id, exporter_slug, \
                                           action, state_slug, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'file', 'export', 'done', ?4, ?4)",
                params![id, snapshot, persona, created],
            )
            .unwrap();
        }

        // Stops at 79 rather than running to latest: V90 drops
        // `dispatch_job.pursuit_id`, and the backfill asserted below is
        // V79's own answer, not the schema's current one.
        migrate_to(&mut conn, 79).unwrap();

        let stamped: Vec<(Uuid, Option<Uuid>, i64)> = {
            let mut stmt = conn
                .prepare("SELECT id, pursuit_id, created_at FROM dispatch_job ORDER BY created_at")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(stamped.len(), 2);
        let mut minted = std::collections::HashSet::new();
        for (dispatch, pursuit, dispatch_created) in &stamped {
            let pursuit = pursuit.expect("every legacy dispatch is stamped");
            assert!(
                minted.insert(pursuit),
                "no grouping: dispatch {dispatch} must not share a pursuit"
            );
            let (p_persona, p_created, title, author_kind, operator_ai, via): (
                Uuid,
                i64,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = conn
                .query_row(
                    "SELECT persona_id, created_at, title, author_kind, operator_ai, \
                            attributed_via \
                       FROM pursuit WHERE id = ?1",
                    params![pursuit],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(p_persona, persona);
            assert_eq!(
                p_created, *dispatch_created,
                "the pursuit inherits the dispatch's clock, not the migration's"
            );
            assert_eq!(
                (title, author_kind, operator_ai, via),
                (None, None, None, None),
                "absent bookkeeping stays absent"
            );
        }
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pursuit WHERE id NOT IN \
                 (SELECT pursuit_id FROM dispatch_job WHERE pursuit_id IS NOT NULL)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            orphans, 0,
            "the backfill mints nothing beyond the dispatches"
        );
    }

    /// V82 transcribes recorded outputs into the ledger — one
    /// `'in'/'generated'` row per (pursuit, asset), first dispatch
    /// wins when the same asset appears in two rounds' outputs, NULL
    /// attribution, the dispatch's clock. The restamp CHECK is checked
    /// at the end of the chain rather than at V82, which is the only
    /// thing this test can honestly say about it.
    #[test]
    fn v82_transcribes_outputs_into_the_ledger_once_per_membership() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 81).unwrap();
        let persona = seed_persona(&conn);
        let a = seed_asset(&conn, persona);
        let b = seed_asset(&conn, persona);
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'cafe', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snapshot_asset (snapshot_id, asset_id, position) \
             VALUES (?1, ?2, 0)",
            params![snapshot, a],
        )
        .unwrap();
        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, 0)",
            params![pursuit, persona],
        )
        .unwrap();
        // Two rounds of one pursuit; `a` appears in both outputs, `b`
        // in the second only. An idle dispatch with no outputs rides
        // along to prove it contributes nothing.
        for (id, outputs, created) in [
            (Uuid::now_v7(), format!("[\"{a}\"]"), 1_000_i64),
            (Uuid::now_v7(), format!("[\"{a}\", \"{b}\"]"), 2_000),
            (Uuid::now_v7(), "[]".to_string(), 3_000),
        ] {
            conn.execute(
                "INSERT INTO dispatch_job (id, snapshot_id, persona_id, exporter_slug, \
                                           action, state_slug, output_asset_ids, pursuit_id, \
                                           created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'file', 'export', 'done', ?4, ?5, ?6, ?6)",
                params![id, snapshot, persona, outputs, pursuit, created],
            )
            .unwrap();
        }

        migrate(&mut conn).unwrap();

        struct LedgerRow {
            asset_id: Uuid,
            kind: String,
            origin: Option<String>,
            author_kind: Option<String>,
            created_at: i64,
        }
        let rows: Vec<LedgerRow> = {
            let mut stmt = conn
                .prepare(
                    "SELECT asset_id, kind, origin, author_kind, created_at \
                       FROM pursuit_tx WHERE pursuit_id = ?1 ORDER BY created_at, id",
                )
                .unwrap();
            stmt.query_map(params![pursuit], |r| {
                Ok(LedgerRow {
                    asset_id: r.get(0)?,
                    kind: r.get(1)?,
                    origin: r.get(2)?,
                    author_kind: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
        };
        assert_eq!(rows.len(), 2, "one membership per asset, not per output");
        assert_eq!(
            (rows[0].asset_id, rows[0].created_at),
            (a, 1_000),
            "the first dispatch to produce the asset names the entry's clock"
        );
        assert_eq!((rows[1].asset_id, rows[1].created_at), (b, 2_000));
        for row in &rows {
            assert_eq!(row.kind, "in");
            assert_eq!(row.origin.as_deref(), Some("generated"));
            assert_eq!(
                row.author_kind, None,
                "nobody recorded these entries; the migration did"
            );
        }
    }

    /// V87 rebuilds `dispatch_job` to drop one constraint and nothing
    /// else: every column keeps its value, every index comes back, and
    /// the pursuit a row names can now be deleted out from under it.
    ///
    /// Seeded at 86 with every one of the twenty-five columns non-NULL
    /// — the point of the rebuild is that a hand-written column list
    /// could transpose two neighbours of the same type without SQLite
    /// noticing, and a fixture that leaves the optional columns NULL
    /// would not catch it. The values are deliberately distinguishable
    /// from each other for the same reason.
    #[test]
    fn v87_unbinds_the_dispatch_stamp_and_keeps_the_row() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 86).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'facade', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snapshot_asset (snapshot_id, asset_id, position) \
             VALUES (?1, ?2, 0)",
            params![snapshot, asset],
        )
        .unwrap();
        let bucket = Uuid::now_v7();
        conn.execute(
            "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'g', 0, 0)",
            params![bucket, persona],
        )
        .unwrap();
        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, title, created_at) \
             VALUES (?1, ?2, 'the round', 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let dispatch = Uuid::now_v7();
        conn.execute(
            "INSERT INTO dispatch_job \
                 (id, snapshot_id, persona_id, exporter_slug, action, params_json, \
                  state_slug, state_message, progress_current, progress_total, \
                  handle_kind, handle_payload, output_asset_ids, created_at, \
                  updated_at, completed_at, source_group_id, source_query_json, \
                  operator_ai, author_kind, author_subject, attributed_via, \
                  pursuit_id, attempt_kind, attempt_payload) \
             VALUES (?1, ?2, ?3, 'file', 'copy', '{\"dir\":\"/out\"}', \
                     'done', 'wrote 1', 7, 9, \
                     'remote', '{\"job\":\"h-1\"}', ?4, 1000, \
                     2000, 3000, ?5, '{\"q\":\"tagged\"}', \
                     'agent-slug', 'owner', 'alice', 'mcp', \
                     ?6, 'http', '{\"status\":200}')",
            params![
                dispatch,
                snapshot,
                persona,
                format!("[\"{asset}\"]"),
                bucket,
                pursuit
            ],
        )
        .unwrap();

        // Stops at 87 rather than running to latest: V90 drops the
        // column this rebuild kept, and what is asserted below is V88's
        // own answer — the constraint gone and the value still there.
        migrate_to(&mut conn, 87).unwrap();

        // Every column, by value. `output_asset_ids` is compared
        // against the string that was written rather than re-derived,
        // so a rebuild that dropped it would fail here too.
        // Three groups because a Rust tuple stops being comparable and
        // printable past twelve elements, and the table has
        // twenty-five columns.
        type Head = (
            Uuid,
            Uuid,
            Uuid,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
        );
        type Middle = (
            Option<i64>,
            Option<String>,
            Option<String>,
            String,
            i64,
            i64,
            Option<i64>,
            Option<Uuid>,
        );
        type Tail = (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<Uuid>,
            Option<String>,
            Option<String>,
        );
        let (head, middle, tail): (Head, Middle, Tail) = conn
            .query_row(
                "SELECT id, snapshot_id, persona_id, exporter_slug, action, params_json, \
                        state_slug, state_message, progress_current, progress_total, \
                        handle_kind, handle_payload, output_asset_ids, created_at, \
                        updated_at, completed_at, source_group_id, source_query_json, \
                        operator_ai, author_kind, author_subject, attributed_via, \
                        pursuit_id, attempt_kind, attempt_payload \
                   FROM dispatch_job WHERE id = ?1",
                params![dispatch],
                |r| {
                    Ok((
                        (
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                        ),
                        (
                            r.get(9)?,
                            r.get(10)?,
                            r.get(11)?,
                            r.get(12)?,
                            r.get(13)?,
                            r.get(14)?,
                            r.get(15)?,
                            r.get(16)?,
                        ),
                        (
                            r.get(17)?,
                            r.get(18)?,
                            r.get(19)?,
                            r.get(20)?,
                            r.get(21)?,
                            r.get(22)?,
                            r.get(23)?,
                            r.get(24)?,
                        ),
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            head,
            (
                dispatch,
                snapshot,
                persona,
                "file".to_string(),
                "copy".to_string(),
                r#"{"dir":"/out"}"#.to_string(),
                "done".to_string(),
                Some("wrote 1".to_string()),
                Some(7),
            ),
            "the rebuild copied the row, it did not reshape it"
        );
        assert_eq!(
            middle,
            (
                Some(9),
                Some("remote".to_string()),
                Some(r#"{"job":"h-1"}"#.to_string()),
                format!("[\"{asset}\"]"),
                1000,
                2000,
                Some(3000),
                Some(bucket),
            ),
            "including the two integers and the two JSON blobs that sit \
             next to each other"
        );
        assert_eq!(
            tail,
            (
                Some(r#"{"q":"tagged"}"#.to_string()),
                Some("agent-slug".to_string()),
                Some("owner".to_string()),
                Some("alice".to_string()),
                Some("mcp".to_string()),
                Some(pursuit),
                Some("http".to_string()),
                Some(r#"{"status":200}"#.to_string()),
            ),
            "the columns the ALTERs appended survive in their own order"
        );

        // The constraint, gone: the stamp names no foreign key now.
        let stamp_edges: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA foreign_key_list('dispatch_job')")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(2))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            !stamp_edges.iter().any(|t| t == "pursuit"),
            "no edge from the dispatch table into the forge: {stamp_edges:?}"
        );
        assert!(
            stamp_edges.iter().any(|t| t == "snapshot")
                && stamp_edges.iter().any(|t| t == "persona")
                && stamp_edges.iter().any(|t| t == "bucket"),
            "and the edges that were never in question stay: {stamp_edges:?}"
        );

        // Deleting the pursuit succeeds, and the stamp stays behind as
        // a value that resolves to nothing.
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();
        conn.execute("DELETE FROM pursuit WHERE id = ?1", params![pursuit])
            .unwrap();
        let survivor: Option<Uuid> = conn
            .query_row(
                "SELECT pursuit_id FROM dispatch_job WHERE id = ?1",
                params![dispatch],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            survivor,
            Some(pursuit),
            "the round was filed under that pursuit, and still was"
        );
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pursuit WHERE id = ?1",
                params![pursuit],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 0, "and the pursuit really is gone");

        // Every index the table carried, by name — the two this change
        // is about and the two `DROP TABLE` would have taken silently.
        let indexes: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                      WHERE type = 'index' AND tbl_name = 'dispatch_job' \
                      ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        for wanted in [
            "idx_dispatch_persona_created",
            "idx_dispatch_pursuit",
            "idx_dispatch_snapshot_created",
            "idx_dispatch_state",
        ] {
            assert!(
                indexes.iter().any(|n| n == wanted),
                "{wanted} did not come back: {indexes:?}"
            );
        }
    }

    /// V88 drops the two tables the close's narrowing was recorded in
    /// and narrows the restamp CHECK back to the one subject that
    /// exists.
    ///
    /// Seeded at 87 with a populated cull and member — the drop has to
    /// be exercised against rows, not against empty tables, because
    /// what made leaving them costly was the RESTRICT edges those rows
    /// hold into `pursuit`, `persona`, `pursuit_event` and `snapshot`.
    /// The pursuit is deleted afterwards with foreign keys on: at 86
    /// those rows would have refused it.
    #[test]
    fn v88_drops_the_close_record_and_frees_what_it_pinned() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 87).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'narrowed', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let event = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit_event (id, pursuit_id, persona_id, kind, created_at) \
             VALUES (?1, ?2, ?3, 'closed_satisfied', 0)",
            params![event, pursuit, persona],
        )
        .unwrap();
        let cull = Uuid::now_v7();
        conn.execute(
            "INSERT INTO cull (id, pursuit_id, persona_id, pursuit_event_id, \
                               candidate_snapshot_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![cull, pursuit, persona, event, snapshot],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cull_member (cull_id, asset_id, verdict) VALUES (?1, ?2, 'keep')",
            params![cull, asset],
        )
        .unwrap();

        // Stops at 88 rather than running to latest: V89 drops
        // `pursuit_restamp` outright, and the CHECK and indexes
        // asserted below are V89's own answer, not the schema's
        // current one.
        migrate_to(&mut conn, 88).unwrap();

        // Both tables are gone, indexes with them.
        let left: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                      WHERE name LIKE 'cull%' OR tbl_name LIKE 'cull%' \
                      ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(left.is_empty(), "nothing of the record is left: {left:?}");

        // The close event it hung on is untouched — the drop takes the
        // record, not the history it was written beside.
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pursuit_event WHERE id = ?1",
                params![event],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1, "the close event stays");

        // The restamp CHECK admits the one subject and refuses what
        // V82 had reserved.
        let dispatch_row = conn.execute(
            "INSERT INTO pursuit_restamp \
                 (id, subject_kind, subject_id, to_pursuit_id, created_at) \
             VALUES (?1, 'dispatch', ?2, ?3, 0)",
            params![Uuid::now_v7(), Uuid::now_v7(), pursuit],
        );
        assert!(
            dispatch_row.is_ok(),
            "'dispatch' is admitted: {dispatch_row:?}"
        );
        let narrowed = conn.execute(
            "INSERT INTO pursuit_restamp \
                 (id, subject_kind, subject_id, to_pursuit_id, created_at) \
             VALUES (?1, 'cull', ?2, ?3, 0)",
            params![Uuid::now_v7(), Uuid::now_v7(), pursuit],
        );
        assert!(
            narrowed.is_err(),
            "the vocabulary went with the concept: {narrowed:?}"
        );
        for wanted in [
            "idx_pursuit_restamp_subject",
            "idx_pursuit_restamp_to",
            "idx_pursuit_restamp_from",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                      WHERE type = 'index' AND name = ?1",
                    params![wanted],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "{wanted} did not come back");
        }

        // What the rows pinned is free: at 86 the cull's RESTRICT edges
        // would have refused this delete.
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();
        conn.execute("DELETE FROM pursuit_restamp", []).unwrap();
        conn.execute("DELETE FROM pursuit_event WHERE id = ?1", params![event])
            .unwrap();
        conn.execute("DELETE FROM pursuit WHERE id = ?1", params![pursuit])
            .unwrap();
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pursuit WHERE id = ?1",
                params![pursuit],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 0, "the pursuit really is gone");
    }

    /// V89 drops `pursuit_restamp` and takes its three indexes with it,
    /// leaving what it pointed at alone.
    ///
    /// Seeded at 88 with a row, for the reason the V88 test seeds one:
    /// a drop exercised against an empty table would not show that the
    /// two RESTRICT edges into `pursuit` go with it. The stamp on
    /// `dispatch_job` is checked afterwards because it is the thing
    /// most easily mistaken for part of this change — the restamp
    /// record goes, the filing it recorded moves to stays.
    #[test]
    fn v89_drops_the_restamp_record_and_leaves_the_filing() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 88).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'unstamped', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snapshot_asset (snapshot_id, asset_id, position) \
             VALUES (?1, ?2, 0)",
            params![snapshot, asset],
        )
        .unwrap();
        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let dispatch = Uuid::now_v7();
        conn.execute(
            "INSERT INTO dispatch_job \
                 (id, snapshot_id, persona_id, exporter_slug, action, state_slug, \
                  pursuit_id, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'file', 'copy', 'pending', ?4, 0, 0)",
            params![dispatch, snapshot, persona, pursuit],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pursuit_restamp \
                 (id, subject_kind, subject_id, to_pursuit_id, created_at) \
             VALUES (?1, 'dispatch', ?2, ?3, 0)",
            params![Uuid::now_v7(), dispatch, pursuit],
        )
        .unwrap();

        // Stops at 89 rather than running to latest: V90 drops
        // `dispatch_job.pursuit_id`, and the filing this step leaves
        // alone is what is asserted below.
        migrate_to(&mut conn, 89).unwrap();

        // The table and its three indexes are gone together.
        let left: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                      WHERE name LIKE '%pursuit_restamp%' \
                         OR tbl_name LIKE '%pursuit_restamp%' \
                      ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(left.is_empty(), "nothing of the record is left: {left:?}");

        // The filing the dropped rows recorded moves to is
        // `dispatch_job`'s own column, and it still holds its value.
        let stamp: Option<Uuid> = conn
            .query_row(
                "SELECT pursuit_id FROM dispatch_job WHERE id = ?1",
                params![dispatch],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stamp, Some(pursuit), "the stamp is untouched");

        // And the pursuit the two dropped FK columns pinned is free.
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();
        conn.execute("DELETE FROM dispatch_job WHERE id = ?1", params![dispatch])
            .unwrap();
        conn.execute("DELETE FROM pursuit WHERE id = ?1", params![pursuit])
            .unwrap();
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pursuit WHERE id = ?1",
                params![pursuit],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 0, "the pursuit really is gone");
    }

    /// V90 takes the stamp off the dispatch and the lookup column off
    /// the asset, and a rebuild is the one kind of migration where
    /// "it ran" and "it kept the data" are different claims.
    ///
    /// Seeded at 89 with a stamped row whose every other column is
    /// distinguishable, because the risk a hand-written `INSERT …
    /// SELECT` over twenty-four columns carries is not failing — it is
    /// landing `author_kind` in `author_subject` and saying nothing.
    /// The three surviving indexes are asserted by name for the same
    /// reason V87's are: `DROP TABLE` takes every index with it, and a
    /// recreate that was forgotten costs a seek per read with nothing
    /// to notice it.
    #[test]
    fn v90_drops_the_stamp_and_carries_every_other_column_across() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 89).unwrap();
        let persona = seed_persona(&conn);
        let asset = seed_asset(&conn, persona);
        let snapshot = Uuid::now_v7();
        conn.execute(
            "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
             VALUES (?1, ?2, 'deface', 0)",
            params![snapshot, persona],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snapshot_asset (snapshot_id, asset_id, position) \
             VALUES (?1, ?2, 0)",
            params![snapshot, asset],
        )
        .unwrap();
        let bucket = Uuid::now_v7();
        conn.execute(
            "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'g', 0, 0)",
            params![bucket, persona],
        )
        .unwrap();
        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, title, created_at) \
             VALUES (?1, ?2, 'the line', 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let dispatch = Uuid::now_v7();
        conn.execute(
            "INSERT INTO dispatch_job \
                 (id, snapshot_id, persona_id, exporter_slug, action, params_json, \
                  state_slug, state_message, progress_current, progress_total, \
                  handle_kind, handle_payload, output_asset_ids, created_at, \
                  updated_at, completed_at, source_group_id, source_query_json, \
                  operator_ai, author_kind, author_subject, attributed_via, \
                  pursuit_id, attempt_kind, attempt_payload) \
             VALUES (?1, ?2, ?3, 'file', 'copy', '{\"dir\":\"/out\"}', \
                     'done', 'wrote 1', 7, 9, \
                     'remote', '{\"job\":\"h-1\"}', ?4, 1000, \
                     2000, 3000, ?5, '{\"q\":\"tagged\"}', \
                     'agent-slug', 'owner', 'alice', 'mcp', \
                     ?6, 'http', '{\"status\":200}')",
            params![
                dispatch,
                snapshot,
                persona,
                format!("[\"{asset}\"]"),
                bucket,
                pursuit
            ],
        )
        .unwrap();
        // An asset whose `_trace` note resolved both halves, so the
        // generated column being dropped has a value to lose and the
        // one being kept has a value to hold on to.
        let returned = seed_asset(&conn, persona);
        conn.execute(
            "UPDATE asset SET extra = ?1 WHERE id = ?2",
            params![
                r#"{"_trace":{"resolved":true,"dispatch_id":"d-1","pursuit_resolved":true,"pursuit_id":"p-1"}}"#,
                returned
            ],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        // The stamp is gone from the table, and only the stamp.
        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info('dispatch_job')").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            !columns.iter().any(|c| c == "pursuit_id"),
            "the stamp is off the table: {columns:?}"
        );
        assert_eq!(columns.len(), 24, "one column left, not two: {columns:?}");

        // Every other column landed where it started — the failure a
        // long hand-written column list makes possible is a silent
        // transposition, not an error. Read back as rendered values
        // rather than a typed tuple: twenty-four of those is past what
        // `Debug` and `PartialEq` are implemented for, and what is
        // being asserted is which value sits in which column.
        let row: Vec<String> = conn
            .query_row(
                "SELECT id, snapshot_id, persona_id, exporter_slug, action, params_json, \
                        state_slug, state_message, progress_current, progress_total, \
                        handle_kind, handle_payload, output_asset_ids, created_at, \
                        updated_at, completed_at, source_group_id, source_query_json, \
                        operator_ai, author_kind, author_subject, attributed_via, \
                        attempt_kind, attempt_payload \
                   FROM dispatch_job WHERE id = ?1",
                params![dispatch],
                |r| {
                    (0..24)
                        .map(|i| Ok(format!("{:?}", r.get::<_, rusqlite::types::Value>(i)?)))
                        .collect::<Result<Vec<_>, _>>()
                },
            )
            .unwrap();
        let expected: Vec<String> = [
            format!("Blob({:?})", dispatch.as_bytes().to_vec()),
            format!("Blob({:?})", snapshot.as_bytes().to_vec()),
            format!("Blob({:?})", persona.as_bytes().to_vec()),
            "Text(\"file\")".to_string(),
            "Text(\"copy\")".to_string(),
            "Text(\"{\\\"dir\\\":\\\"/out\\\"}\")".to_string(),
            "Text(\"done\")".to_string(),
            "Text(\"wrote 1\")".to_string(),
            "Integer(7)".to_string(),
            "Integer(9)".to_string(),
            "Text(\"remote\")".to_string(),
            "Text(\"{\\\"job\\\":\\\"h-1\\\"}\")".to_string(),
            format!("Text({:?})", format!("[\"{asset}\"]")),
            "Integer(1000)".to_string(),
            "Integer(2000)".to_string(),
            "Integer(3000)".to_string(),
            format!("Blob({:?})", bucket.as_bytes().to_vec()),
            "Text(\"{\\\"q\\\":\\\"tagged\\\"}\")".to_string(),
            "Text(\"agent-slug\")".to_string(),
            "Text(\"owner\")".to_string(),
            "Text(\"alice\")".to_string(),
            "Text(\"mcp\")".to_string(),
            "Text(\"http\")".to_string(),
            "Text(\"{\\\"status\\\":200}\")".to_string(),
        ]
        .to_vec();
        assert_eq!(
            row, expected,
            "the rebuild copied the row minus one column, it did not reshape it"
        );

        // The three surviving indexes came back, and the stamp's did
        // not — `DROP TABLE` took all four.
        let indexes: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                      WHERE type = 'index' AND tbl_name = 'dispatch_job' \
                        AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            indexes,
            vec![
                "idx_dispatch_persona_created",
                "idx_dispatch_snapshot_created",
                "idx_dispatch_state",
            ],
            "every index the table still needs was written out again"
        );

        // The asset side: the lookup column and its partial index are
        // gone, its sibling stays, and the `_trace` bag is untouched —
        // what an ingest recorded is not what this schema asserts.
        let asset_columns: Vec<String> = {
            // `table_xinfo`, not `table_info`: a VIRTUAL generated
            // column is hidden from the latter, so the assertion below
            // would pass against a column that is still there.
            let mut stmt = conn.prepare("PRAGMA table_xinfo('asset')").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            !asset_columns.iter().any(|c| c == "trace_pursuit_id"),
            "the pursuit lookup column is gone: {asset_columns:?}"
        );
        assert!(
            asset_columns.iter().any(|c| c == "trace_dispatch_id"),
            "its sibling stays: {asset_columns:?}"
        );
        let left: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                      WHERE type = 'index' AND name = 'idx_asset_trace_pursuit'",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(left.is_empty(), "the partial index went with it: {left:?}");
        let bag: String = conn
            .query_row(
                "SELECT extra FROM asset WHERE id = ?1",
                params![returned],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            bag.contains(r#""pursuit_id":"p-1""#),
            "the note keeps what the ingest recorded: {bag}"
        );
    }

    /// V91 drops the base-event pin, its index and its CHECK, and
    /// leaves the rest of the ledger row where it was.
    ///
    /// Seeded at 90 with a row that *does* hold a pin, which no shipped
    /// writer has ever produced. That is the point: the shipped writer
    /// is not the only thing that can have written this table, and a
    /// `DROP COLUMN` that only ever met NULLs would prove nothing about
    /// the one case where the FK edge is real. The pinned event is
    /// deleted afterwards to show the RESTRICT edge went with the
    /// column rather than merely stopping being read.
    ///
    /// The three columns around the dropped one are read back by name
    /// and compared as a tuple, because the failure this step can cause
    /// is not an error — `TxRow::from_row` reads `pursuit_tx` by
    /// ordinal, and a column removed from the middle of the SELECT list
    /// shifts every read after it onto a neighbour of the same type.
    #[test]
    fn v91_drops_the_pin_and_leaves_the_rest_of_the_gesture() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 90).unwrap();
        let persona = seed_persona(&conn);

        let pursuit = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit (id, persona_id, created_at) VALUES (?1, ?2, 0)",
            params![pursuit, persona],
        )
        .unwrap();
        let project = Uuid::now_v7();
        conn.execute(
            "INSERT INTO project (id, persona_id, name, created_at) VALUES (?1, ?2, 'album', 0)",
            params![project, persona],
        )
        .unwrap();
        let line = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line (id, project_id, name, created_at) VALUES (?1, ?2, 'main', 0)",
            params![line, project],
        )
        .unwrap();
        let entry = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line_entry (id, line_id, persona_id, created_at) VALUES (?1, ?2, ?3, 0)",
            params![entry, line, persona],
        )
        .unwrap();

        // A real event to pin: the column carries an FK, so a loose
        // uuid would be refused before the pin was ever written.
        let close = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit_event (id, pursuit_id, persona_id, kind, created_at) \
             VALUES (?1, ?2, ?3, 'closed_satisfied', 0)",
            params![close, pursuit, persona],
        )
        .unwrap();
        let merge = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line_merge (id, pursuit_event_id, persona_id, created_at) \
             VALUES (?1, ?2, ?3, 0)",
            params![merge, close, persona],
        )
        .unwrap();
        let pinned = Uuid::now_v7();
        conn.execute(
            "INSERT INTO line_event \
                 (id, entry_id, persona_id, verb, asset_id, name, merge_id, created_at) \
             VALUES (?1, ?2, ?3, 'add', ?4, 'key visual', ?5, 0)",
            params![pinned, entry, persona, Uuid::now_v7(), merge],
        )
        .unwrap();

        let aimed = Uuid::now_v7();
        let asset = Uuid::now_v7();
        conn.execute(
            "INSERT INTO pursuit_tx \
                 (id, pursuit_id, persona_id, kind, asset_id, origin, \
                  target_entry_id, base_event_id, out_of_scope, note, \
                  author_kind, author_subject, operator_ai, attributed_via, created_at) \
             VALUES (?1, ?2, ?3, 'in', ?4, 'existing', ?5, ?6, 1, 'aimed', \
                     'subject', 'alice', 'claude-code', 'mcp', 11)",
            params![aimed, pursuit, persona, asset, entry, pinned],
        )
        .unwrap();

        migrate_to(&mut conn, 91).unwrap();

        // The column is gone, and so is the index over it.
        let columns: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM pragma_table_info('pursuit_tx')")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            !columns.iter().any(|c| c == "base_event_id"),
            "the pin column is gone: {columns:?}"
        );
        assert!(
            columns.iter().any(|c| c == "target_entry_id"),
            "the entry it was aimed beside stays: {columns:?}"
        );
        let left: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                      WHERE type = 'index' AND name = 'idx_pursuit_tx_base_event'",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(left.is_empty(), "the index went with it: {left:?}");

        // The gesture that carried the pin is still a gesture, and
        // every column beside the dropped one still holds its own
        // value rather than a neighbour's.
        type AimedRow = (
            Uuid,
            String,
            Option<String>,
            Option<Uuid>,
            i64,
            Option<Uuid>,
            Option<String>,
            Option<String>,
            i64,
        );
        let row: AimedRow = conn
            .query_row(
                "SELECT asset_id, kind, origin, target_entry_id, out_of_scope, \
                        supersedes_asset_id, note, author_subject, created_at \
                 FROM pursuit_tx WHERE id = ?1",
                params![aimed],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                asset,
                "in".to_string(),
                Some("existing".to_string()),
                Some(entry),
                1,
                None,
                Some("aimed".to_string()),
                Some("alice".to_string()),
                11
            ),
            "the aim, the out-of-scope claim and the attribution are untouched"
        );

        // The RESTRICT edge went with the column: the event the row
        // pinned is free, and the entry it still names is not.
        conn.execute("DELETE FROM line_event WHERE id = ?1", params![pinned])
            .expect("nothing pins the event any more");
        assert!(
            conn.execute("DELETE FROM line_entry WHERE id = ?1", params![entry])
                .is_err(),
            "the aim it still carries holds the entry"
        );

        // And the pairing rules the step left standing survived the
        // schema rewrite `DROP COLUMN` performs.
        assert!(
            conn.execute(
                "INSERT INTO pursuit_tx \
                     (id, pursuit_id, persona_id, kind, asset_id, origin, \
                      target_entry_id, created_at) \
                 VALUES (?1, ?2, ?3, 'in', ?4, 'generated', ?5, 0)",
                params![Uuid::now_v7(), pursuit, persona, Uuid::now_v7(), entry],
            )
            .is_err(),
            "only an existing-origin IN targets an entry, still"
        );
    }

    /// A profile carrying a corrupt `extra` bag still upgrades: the
    /// bad row degrades to "no claim surfaced", it does not turn into
    /// a migration failure that keeps the profile from opening.
    #[test]
    fn v80_upgrades_over_a_corrupt_extra_bag() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 79).unwrap();
        let persona = seed_persona(&conn);
        let corrupt = seed_asset(&conn, persona);
        conn.execute(
            "UPDATE asset SET extra = 'not json' WHERE id = ?1",
            params![corrupt],
        )
        .unwrap();

        // Stops at 80 rather than running to latest: V90 drops
        // `trace_pursuit_id`, and both generated columns are what this
        // test reads back.
        migrate_to(&mut conn, 80).unwrap();

        let surfaced: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT trace_dispatch_id, trace_pursuit_id FROM asset WHERE id = ?1",
                params![corrupt],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(surfaced, (None, None), "a corrupt bag surfaces nothing");
    }

    /// V80's columns surface a `_trace` claim iff it resolved — the
    /// authority rule lives in the column definition — and the probe
    /// the membership read issues is an index seek, not a scan.
    #[test]
    fn v80_surfaces_resolved_trace_claims_and_probes_by_index() {
        let mut conn = test_conn();
        // Stops at 80 rather than running to latest: V90 drops
        // `trace_pursuit_id` and its index, and both lookup columns are
        // what this test probes.
        migrate_to(&mut conn, 80).unwrap();
        let persona = seed_persona(&conn);

        let resolved = seed_asset(&conn, persona);
        let unresolved = seed_asset(&conn, persona);
        let bare = seed_asset(&conn, persona);
        conn.execute(
            "UPDATE asset SET extra = ?1 WHERE id = ?2",
            params![
                r#"{"_trace":{"resolved":true,"dispatch_id":"d-1","pursuit_resolved":true,"pursuit_id":"p-1"}}"#,
                resolved
            ],
        )
        .unwrap();
        conn.execute(
            "UPDATE asset SET extra = ?1 WHERE id = ?2",
            params![
                r#"{"_trace":{"resolved":false,"dispatch_id":"d-1","pursuit_resolved":false,"pursuit_id":"p-1"}}"#,
                unresolved
            ],
        )
        .unwrap();

        let hits: Vec<Uuid> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM asset WHERE trace_dispatch_id = 'd-1' \
                       AND trace_dispatch_id IS NOT NULL",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            hits,
            vec![resolved],
            "only the resolved claim surfaces; unresolved and bare rows stay NULL"
        );
        let pursuit_hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset WHERE trace_pursuit_id = 'p-1' \
                   AND trace_pursuit_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pursuit_hits, 1);
        let _ = bare;

        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id FROM asset \
                  WHERE trace_dispatch_id IN ('d-1', 'd-2') \
                    AND trace_dispatch_id IS NOT NULL",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(
            plan.contains("idx_asset_trace_dispatch"),
            "the membership probe must be an index seek, not a scan: {plan}"
        );
    }

    /// V61 takes the refusal off `(source_kind, source_locator)` and
    /// leaves the lookup — under a key that now starts with the persona.
    ///
    /// Asserted as a behaviour change across the step rather than as a
    /// list of index names: the same two `INSERT`s are attempted at V60
    /// and again after the upgrade, so the fixture disagrees with itself
    /// exactly where the migration acts. A names-only test would pass on
    /// a `CREATE UNIQUE INDEX idx_asset_source`.
    #[test]
    fn v61_lets_two_rows_hold_one_source_value_and_keeps_the_lookup() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 60).unwrap();
        let persona = seed_persona(&conn);

        let held = "/pics/held.png";
        let insert = |conn: &Connection, owner: Uuid| -> Result<usize, rusqlite::Error> {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[]', 0, 0, 0)",
                params![Uuid::now_v7(), owner, held],
            )
        };
        insert(&conn, persona).unwrap();
        assert!(
            insert(&conn, persona).is_err(),
            "at V60 the pair is unique, so the second row is refused — without this the \
             assertion after the upgrade would hold whatever V61 did"
        );

        migrate(&mut conn).unwrap();

        insert(&conn, persona).expect("`N : 1` — many Assets may carry one Source value");
        // …and a second persona holding the same path is an ordinary
        // first import over there, not a duplicate over here.
        let other = Uuid::now_v7();
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, 'q', 'Q', 0, 0)",
            params![other],
        )
        .unwrap();
        insert(&conn, other).expect("the persona is part of the key");

        let index_named = |name: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![name],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            index_named("idx_asset_source_unique"),
            0,
            "the UNIQUE is gone"
        );
        assert_eq!(
            index_named("idx_asset_source"),
            1,
            "and the columns keep an index — the ingest path's first lookup reads them on \
             every arrival now, not only when a write collided"
        );
        // The lookup the ingest path makes is served by it, key order and
        // all. `EXPLAIN QUERY PLAN` is how that is checkable without
        // asserting the SQL text of the index back to itself.
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id FROM asset \
                   WHERE persona_id = ?1 AND source_kind = ?2 AND source_locator = ?3",
                params![persona, "fs", held],
                |r| r.get(3),
            )
            .unwrap();
        assert!(
            plan.contains("idx_asset_source"),
            "the three-column lookup planned without its index: {plan}"
        );
    }

    /// V62 takes the refusal off `external_key` and leaves the lookup.
    ///
    /// Asserted as a behaviour change across the step, for the reason
    /// the V61 test states: the index keeps its **name**, so a test that
    /// checked `sqlite_master` for `idx_asset_external_key` would pass
    /// unchanged against a `CREATE UNIQUE INDEX` of the same name and
    /// would therefore assert nothing about this migration at all. The
    /// same two `INSERT`s are attempted at V61 and again after the
    /// upgrade, so the fixture disagrees with itself exactly where the
    /// migration acts.
    #[test]
    fn v62_lets_two_rows_hold_one_external_key_and_keeps_the_lookup() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 61).unwrap();
        let persona = seed_persona(&conn);

        // One external record, stated twice: signed once, updated and
        // re-ingested. The source says the same key both times.
        let stated = "issue-12345";
        let insert = |conn: &Connection, kind: &str| -> Result<usize, rusqlite::Error> {
            let id = Uuid::now_v7();
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, external_key, \
                                    modality, labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'dialogue', '[]', 0, 0, 0)",
                params![id, persona, kind, format!("/pics/{id}.png"), stated],
            )
        };
        insert(&conn, "fs").unwrap();
        assert!(
            insert(&conn, "fs").is_err(),
            "at V61 the key is unique, so the re-ingest is refused — without this the \
             assertion after the upgrade would hold whatever V62 did"
        );
        assert!(
            insert(&conn, "gitea").is_err(),
            "and the old key carries no source discriminator, so at V61 another \
             platform's `issue-12345` is refused by the first platform's"
        );

        migrate(&mut conn).unwrap();

        insert(&conn, "fs").expect("an external record legitimately arrives more than once");
        insert(&conn, "gitea")
            .expect("and two platforms numbering a record alike is not one record");

        let index_named = |name: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![name],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            index_named("idx_asset_external_key"),
            1,
            "the index stays — the Session find-or-create reads it on every arrival"
        );
        // Still partial, so a row with no key stays out of it: an
        // index-size choice, which is all the `WHERE` ever was.
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params!["idx_asset_external_key"],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !sql.to_uppercase().contains("UNIQUE"),
            "the index is back, and it is still a constraint: {sql}"
        );
        assert!(
            sql.contains("external_key IS NOT NULL"),
            "the partial predicate is an index-size choice and should have survived: {sql}"
        );
        // And the lookup it exists for is planned through it.
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id FROM asset \
                   WHERE persona_id = ?1 AND external_key = ?2",
                params![persona, stated],
                |r| r.get(3),
            )
            .unwrap();
        assert!(
            plan.contains("idx_asset_external_key"),
            "the external-key lookup planned without its index: {plan}"
        );
    }

    /// Seeds one asset and its material at the **pre-tag** spelling, the
    /// way a database below V63 holds them.
    ///
    /// Deliberately not `stored_locator`: these rows are what the walk
    /// has to read, so writing them through the live boundary would seed
    /// the answer.
    fn seed_delimited_locator(conn: &Connection, persona: Uuid, locator: &str) -> Uuid {
        let id = Uuid::now_v7();
        conn.execute(
            "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                modality, labels, occurred_at, created_at, updated_at) \
             VALUES (?1, ?2, 'fs', ?3, 'dialogue', '[]', 0, 0, 0)",
            params![id, persona, locator],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, created_at, updated_at) \
             VALUES (?1, 0, ?2, 0, 0)",
            params![id, locator],
        )
        .unwrap();
        id
    }

    /// Both locator columns of one asset, read raw.
    fn locators_of(conn: &Connection, asset: Uuid) -> (String, String) {
        let a = conn
            .query_row(
                "SELECT source_locator FROM asset WHERE id = ?1",
                params![asset],
                |r| r.get(0),
            )
            .unwrap();
        let m = conn
            .query_row(
                "SELECT locator FROM material WHERE asset_id = ?1 AND ord = 0",
                params![asset],
                |r| r.get(0),
            )
            .unwrap();
        (a, m)
    }

    /// **The walk rewrites and does not lose.**
    ///
    /// One persona holding the four spellings that read differently:
    /// `/pics/a.png` and `file:///pics/a.png` must **merge onto one
    /// value** (the `file:` scheme is consumed, deliberately, so the two
    /// compare equal), `pics/a.png` must land `logical` (a rootless path
    /// is openable by nobody), and `/pics/a#b.png` is decided by the
    /// file probe — here neither the whole string nor its container
    /// exists, so the delimiter reading stands and the row is marked.
    ///
    /// The merge is the half that could not have run before V61: under
    /// `idx_asset_source_unique` two rows arriving at one value is a
    /// constraint violation inside a `Step::App`, and the batch would
    /// have rolled back. The fixture asserts the two spellings really
    /// are two distinct strings beforehand, or "they agree afterwards"
    /// would hold of a walk that did nothing.
    #[test]
    fn v63_rewrites_every_locator_and_merges_the_two_spellings_of_one_path() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 62).unwrap();
        let persona = seed_persona(&conn);

        let bare = seed_delimited_locator(&conn, persona, "/pics/a.png");
        let schemed = seed_delimited_locator(&conn, persona, "file:///pics/a.png");
        let rootless = seed_delimited_locator(&conn, persona, "pics/a.png");
        let hashed = seed_delimited_locator(&conn, persona, "/pics/a#b.png");

        let before = locators_of(&conn, bare);
        assert_ne!(
            before.0,
            locators_of(&conn, schemed).0,
            "the two spellings are two strings before the walk — without this the merge \
             asserted below would hold of a walk that changed nothing"
        );

        migrate(&mut conn).unwrap();

        // Nothing was lost: four rows in, four rows out, on both tables.
        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count("asset"), 4);
        assert_eq!(count("material"), 4);

        let bare = locators_of(&conn, bare);
        let schemed = locators_of(&conn, schemed);
        let rootless = locators_of(&conn, rootless);
        let hashed = locators_of(&conn, hashed);

        assert_eq!(bare.0, r#"{"kind":"file","path":"/pics/a.png"}"#);
        // The merge, as **byte identity in the column** rather than as
        // two values that happen to mean the same thing: the ingest
        // lookup is an equality test on this text.
        assert_eq!(
            schemed.0, bare.0,
            "the consumed `file:` scheme leaves one Source value, and one string for it"
        );
        assert_eq!(
            rootless.0, r#"{"kind":"logical","name":"pics/a.png"}"#,
            "a rootless path is a caller-minted name — no process that reads it can open it"
        );
        assert_eq!(
            hashed.0, r#"{"kind":"record","container":"/pics/a","record":"b.png"}"#,
            "neither `/pics/a#b.png` nor `/pics/a` exists here, so the delimiter reading is \
             kept rather than guessed at"
        );

        // The denormalised copy agrees with its asset, row for row. A
        // walk that rewrote one table would leave the same artefact
        // answering differently depending on which row was asked.
        for (asset, material) in [bare, schemed, rootless, hashed] {
            assert_eq!(asset, material);
            // …and every value the walk wrote reads back through the
            // boundary that will read it in production.
            asterism_core::domain::source_locator::SourceLocator::try_from(asset.as_str())
                .expect("a rewritten column value is a locator");
        }
    }

    /// **The probe, both ways.** The one honest test a domain parser
    /// cannot run: is the whole string a file on this disk?
    ///
    /// Three rows, one per outcome, and the first two are the pair that
    /// makes this non-vacuous — they are the *same shape of string* and
    /// differ only in what the filesystem says. A walk that always
    /// answered `file` would fail the second; one that never probed
    /// would fail the first.
    ///
    /// The expectations are rendered by the **live** boundary rather
    /// than spelled out, which makes this the guard on the other half of
    /// the freeze: V63 carries its own copy of the tagged shape on
    /// purpose, so if the live rendering ever moves, this fails loudly
    /// instead of the migration silently writing a form nothing reads.
    /// The literal encoding is pinned in
    /// `v63_rewrites_every_locator_and_merges_the_two_spellings_of_one_path`
    /// and in the domain's own tests.
    #[test]
    fn v63_asks_the_filesystem_which_reading_a_hash_in_a_path_was() {
        use asterism_core::domain::source_locator::{
            ContainerRecord, LocalPath, RecordAddress, SourceLocator,
        };

        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str| -> String {
            let path = dir.path().join(name);
            std::fs::write(&path, b"bytes").expect("fixture written");
            path.to_string_lossy().into_owned()
        };

        // (a) a file whose own name carries a `#`, and it is really there.
        let real_file = write("a#b.png");
        // (b) a container that is really there, addressed per record.
        let container = write("s.jsonl");
        let record = format!("{container}#0198c1c2-aaaa");
        // (c) neither half exists — the disk answers neither way.
        let gone = dir.path().join("gone#b.png").to_string_lossy().into_owned();

        let mut conn = test_conn();
        migrate_to(&mut conn, 62).unwrap();
        let persona = seed_persona(&conn);
        let a = seed_delimited_locator(&conn, persona, &real_file);
        let b = seed_delimited_locator(&conn, persona, &record);
        let c = seed_delimited_locator(&conn, persona, &gone);

        migrate(&mut conn).unwrap();

        let tagged = |asset: Uuid| locators_of(&conn, asset).0;
        let as_file = |path: &str| {
            SourceLocator::from(LocalPath::try_from(path).expect("absolute")).to_storage()
        };
        let as_record = |container: &str, record: &str| {
            SourceLocator::from(ContainerRecord::new(
                LocalPath::try_from(container).expect("absolute"),
                RecordAddress::try_from(record).expect("non-empty"),
            ))
            .to_storage()
        };

        assert_eq!(
            tagged(a),
            as_file(&real_file),
            "the whole string is a file on this disk, so the `#` was part of its name — the \
             reading the delimiter got wrong, corrected"
        );
        assert_eq!(
            tagged(b),
            as_record(&container, "0198c1c2-aaaa"),
            "the whole string is not a file and the container is, so the `#` was the \
             delimiter — corroborated rather than assumed"
        );
        assert_eq!(
            tagged(c),
            as_record(&dir.path().join("gone").to_string_lossy(), "b.png"),
            "a container that is gone answers neither way, so the row keeps the reading it \
             has always had"
        );
    }

    /// The id V73 seeds its one rule under, spelled the way a reader of
    /// the statement sees it.
    ///
    /// Written out rather than read back from the row, so the assertions
    /// below are about the literal in the migration and not about
    /// whatever happens to be in the table.
    const VDSL_STRATEGY_ID: &str = "019fe8f8-1400-7000-8000-000000000001";

    /// One PNG asset with one material — what a derived row hangs off,
    /// `material_series` being keyed on `(asset_id, ord)`.
    fn seed_png_material(conn: &Connection, persona: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        seed_material(
            conn,
            persona,
            &format!("{{\"kind\":\"file\",\"path\":\"/pics/{id}.png\"}}"),
            Some("image/png"),
            None,
        )
    }

    /// One `series_strategy` row, whole: the id, the four fields that
    /// make up the rule (`name` / `applies_to` / `decode` / `include` /
    /// `exclude`), and the provenance triple (`system` / `created_at` /
    /// `updated_at`).
    type StrategyColumns = (Uuid, String, String, String, String, String, i64, i64, i64);

    /// The seeded rule is the measured one, at the frozen id, and marked
    /// as untouched.
    ///
    /// Every column is asserted because each one is a decision the doc
    /// comment argues for: the mime it claims, the decoder it picks, the
    /// path it selects (the only selection anybody measured), an empty
    /// exclude list rather than a NULL, and the pair
    /// `system = 1 AND updated_at = created_at` — which is the test a
    /// later migration would use to tell a pristine seed from a rule
    /// somebody took over, and is worth nothing if this row does not
    /// start out satisfying it.
    ///
    /// The id is compared against the constant above rather than merely
    /// checked for existence: an id minted at migration time would leave
    /// the seeded rule unaddressable by any later step, and it would
    /// look identical to this one in every other assertion.
    #[test]
    fn v73_seeds_the_measured_vdsl_rule_under_a_frozen_id() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let rows: Vec<StrategyColumns> = conn
            .prepare(
                "SELECT id, name, applies_to, decode, include, exclude, \
                        system, created_at, updated_at FROM series_strategy",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(
            rows.len(),
            1,
            "one rule is seeded — the character-card rule is deliberately not one of them"
        );
        let (id, name, applies_to, decode, include, exclude, system, created, updated) =
            rows.into_iter().next().unwrap();
        assert_eq!(id, Uuid::parse_str(VDSL_STRATEGY_ID).unwrap());
        assert_eq!(name, "VDSL recipe");
        assert_eq!(applies_to, "image/png");
        assert_eq!(decode, "raw_json");
        assert_eq!(
            include, r#"[["vdsl","script"]]"#,
            "selecting the recipe is the reading measured at two groups — \
             `domain::series`'s tests hold the eleven images it came from"
        );
        assert_eq!(exclude, "[]", "an empty list, not a NULL");
        assert_eq!(system, 1);
        assert_eq!(
            (created, updated),
            (1_786_320_000_000, 1_786_320_000_000),
            "the stamps are the frozen moment the rule was written, and equal — \
             which is what makes `updated_at <> created_at` mean somebody edited it"
        );
    }

    /// The `CHECK` holds the outcome and the key together, **both
    /// ways**.
    ///
    /// The pair is the whole reason `outcome` is a column: `key IS NULL`
    /// spells two of the three answers alike, and a row that says
    /// `derived` without one — or names a silence while carrying a key —
    /// is a row no reader can interpret. Each of the four shapes is
    /// attempted against the real table, because a constraint is the one
    /// kind of rule that cannot be checked by reading it.
    ///
    /// The two accepted shapes are asserted first: without them a
    /// `CHECK (0)` would satisfy every rejection below.
    ///
    /// Checked by mutation on 2026-08-10, in both directions, because an
    /// equality between two predicates is exactly the kind of constraint
    /// that is easy to write as an implication and leave half-enforced:
    ///
    /// - constraint removed → *"`derived` with no key is an answer that
    ///   says it has one and does not: 1"* (the row was written);
    /// - narrowed to `CHECK (outcome <> 'derived' OR key IS NOT NULL)`,
    ///   which is the plausible half → *"an answer naming a silence must
    ///   not be filed carrying a key: 1"*.
    ///
    /// Restored, it passes.
    #[test]
    fn v73_refuses_a_row_whose_outcome_and_key_disagree() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        let persona = seed_persona(&conn);
        let strategy = Uuid::parse_str(VDSL_STRATEGY_ID).unwrap();
        let key = format!("sk1-sha256:{}", "a".repeat(64));

        let file = |outcome: &str, key: Option<&str>| -> Result<usize, rusqlite::Error> {
            let asset = seed_png_material(&conn, persona);
            conn.execute(
                "INSERT INTO material_series \
                     (asset_id, ord, strategy_id, key, outcome, derived_at) \
                 VALUES (?1, 0, ?2, ?3, ?4, 0)",
                params![asset, strategy, key, outcome],
            )
        };

        file("derived", Some(&key)).expect("a key filed as derived is the ordinary row");
        file("nothing_to_select", None).expect("the rule ran and selected nothing");
        file("not_applicable", None).expect("the rule is not about this material");

        let no_key = file("derived", None)
            .expect_err("`derived` with no key is an answer that says it has one and does not");
        assert!(
            no_key.to_string().contains("CHECK"),
            "refused by the constraint rather than by something else: {no_key}"
        );
        for silent in ["nothing_to_select", "not_applicable"] {
            let carried = file(silent, Some(&key))
                .expect_err("an answer naming a silence must not be filed carrying a key");
            assert!(
                carried.to_string().contains("CHECK"),
                "`{silent}` with a key was refused by something other than the \
                 constraint: {carried}"
            );
        }
    }

    /// Teeth on the two frozen vocabularies: the tokens this schema
    /// admits are the ones the domain writes.
    ///
    /// The `CHECK`s name their values verbatim, and they have to — a
    /// constraint that asked the code what it was about would not be a
    /// constraint. The hazard that leaves is drift, and it is silent in
    /// the direction that matters: rename a variant's token and every
    /// insert fails at runtime on somebody's library, with nothing in
    /// this file to catch it, because no fixture here spells the new
    /// word.
    ///
    /// Read out of **the schema a migrated database ends up with**
    /// rather than retyped, and rather than out of the step that first
    /// wrote it. Those are three different things and the difference
    /// showed up the moment a `CHECK` was widened: V73's text is a
    /// frozen statement about version 73 and must not move, while the
    /// constraint a row is actually inserted against is V77's. A test
    /// reading the older literal would have compared the domain with a
    /// rule no database still enforces — passing while `exif` was
    /// unspellable, or failing while it was fine, depending on which
    /// side moved first. `sqlite_master` is the only copy that is
    /// neither frozen nor retyped.
    ///
    /// **The other side is not retyped either**, and that is the half
    /// this test used to be missing. A literal array of variants written
    /// here catches a rename and sleeps through an addition: nothing
    /// makes it exhaustive, so `Decode::Exif` — already named as coming
    /// by the enum's own doc — would compile, pass this test, and reach
    /// a user's library as a `CHECK` violation the first time somebody
    /// registered an EXIF rule. It reads
    /// [`Decode::ALL`](asterism_core::domain::series::Decode::ALL) and
    /// [`SeriesKey::OUTCOMES`](asterism_core::domain::series::SeriesKey::OUTCOMES)
    /// instead, whose completeness is proved against the enum's own
    /// source by `the_decoder_list_names_every_variant_this_enum_has`.
    #[test]
    fn the_series_tokens_this_schema_admits_are_the_ones_the_domain_writes() {
        use asterism_core::domain::series::{Decode, SeriesKey};

        let mut conn = test_conn();
        migrate(&mut conn).unwrap();

        let admitted = |table: &str, column: &str| -> Vec<String> {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| panic!("`{table}` is in the migrated schema"));
            let opened = format!("CHECK ({column} IN (");
            let inside = sql
                .split_once(&opened)
                .unwrap_or_else(|| panic!("`{table}` admits `{column}` by a literal list"))
                .1
                .split_once("))")
                .expect("and the list is closed")
                .0;
            inside
                .split(',')
                .map(|token| token.trim().trim_matches('\'').to_string())
                .collect()
        };

        assert_eq!(
            admitted("series_strategy", "decode"),
            Decode::ALL
                .iter()
                .map(|d| d.as_str().to_string())
                .collect::<Vec<_>>(),
            "a decoder the schema admits that the domain cannot spell is a rule \
             nothing can carry out; one it refuses is a rule nothing can register — \
             and a decoder shipped without widening the `CHECK` is the second of those"
        );
        assert_eq!(
            admitted("material_series", "outcome"),
            SeriesKey::OUTCOMES
                .iter()
                .map(|token| (*token).to_string())
                .collect::<Vec<_>>(),
            "the answers the domain distinguishes are the ones the column holds"
        );
    }

    /// Deleting a rule takes its derived rows with it, and deleting a
    /// material takes its answers.
    ///
    /// This is the cost of the decision recorded in the V73 doc — a
    /// system row is provenance, not permission, so a person may delete
    /// the seeded rule — and it is affordable for one reason: what
    /// cascades away is derivable from `meta_kv` without reading a byte
    /// off disk. The assertion is that it really is a cascade and not a
    /// foreign-key refusal, which would leave a person unable to remove
    /// a rule they had ever used.
    #[test]
    fn v73_derived_rows_go_with_the_rule_and_with_the_material() {
        let mut conn = test_conn();
        migrate(&mut conn).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let persona = seed_persona(&conn);
        let strategy = Uuid::parse_str(VDSL_STRATEGY_ID).unwrap();

        let file = |asset: Uuid| {
            conn.execute(
                "INSERT INTO material_series \
                     (asset_id, ord, strategy_id, key, outcome, derived_at) \
                 VALUES (?1, 0, ?2, ?3, 'derived', 0)",
                params![asset, strategy, format!("sk1-sha256:{}", "a".repeat(64))],
            )
            .unwrap();
        };
        let derived_rows = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM material_series", [], |r| r.get(0))
                .unwrap()
        };

        let dropped = seed_png_material(&conn, persona);
        let kept = seed_png_material(&conn, persona);
        file(dropped);
        file(kept);
        assert_eq!(derived_rows(&conn), 2);

        conn.execute("DELETE FROM material WHERE asset_id = ?1", params![dropped])
            .expect("a material with derived answers is still deletable");
        assert_eq!(derived_rows(&conn), 1, "its answer went with it");

        conn.execute(
            "DELETE FROM series_strategy WHERE id = ?1",
            params![strategy],
        )
        .expect("a rule that has derived something is still deletable");
        assert_eq!(
            derived_rows(&conn),
            0,
            "and every key filed under it goes — re-derivable from `meta_kv`, no disk read"
        );
    }

    /// **V75 answers every existing row — every format alike — and
    /// moves nothing else.**
    ///
    /// The column is a pure addition, so the assertion is in two halves.
    /// The first is that a row already in the library comes out carrying
    /// `unsupported:not-captured` rather than NULL: NULL is a legitimate
    /// value in this column from here on (it is what a format that keeps
    /// no bytes stores), so a row left NULL by this step would be saying
    /// something about the format instead of about the step.
    ///
    /// The video row is in the fixture because the write is **unscoped**
    /// and the doc says so: the marker means "the build that read this
    /// row did not keep the bytes", which is equally true of a format
    /// that will never keep any. A scoped write is what the doc argues
    /// against — it would freeze a wrong answer for the JPEGs S6 teaches
    /// to keep bytes — and this is the assertion that would fail if
    /// somebody added the `WHERE`.
    ///
    /// The second half is the one the slice lives or dies by: the three
    /// fingerprint columns and `meta_kv` are byte-for-byte what they
    /// were. A `meta_kv` that moved would move `m1-`, and `m1-` values
    /// are frozen as literals against rows already sitting in a Dogfood
    /// database.
    #[test]
    fn v75_answers_every_existing_row_and_leaves_the_fingerprint_columns_alone() {
        use asterism_core::domain::material_meta_raw::NOT_CAPTURED;

        let mut conn = test_conn();
        migrate_to(&mut conn, 74).unwrap();
        let persona = seed_persona(&conn);

        // A row in the state a fingerprinted PNG is really in: three
        // answers and the object the meta digest was taken over.
        let canonical = r#"{"prompt":"a cat"}"#;
        let meta = asterism_core::domain::material_meta::digest_of(canonical);
        let asset = seed_png_material(&conn, persona);
        conn.execute(
            "UPDATE material SET content_hash = ?2, content_region_hash = ?3, \
                                 meta_hash = ?4, meta_kv = ?5 \
              WHERE asset_id = ?1",
            params![
                asset,
                format!("sha256:{}", "a".repeat(64)),
                format!("cr1-sha256:{}", "b".repeat(64)),
                meta,
                canonical,
            ],
        )
        .unwrap();
        let before: Vec<Option<String>> = fingerprint_columns(&conn, asset);

        // A format nothing will ever keep bytes for, beside it.
        let clip = seed_material(
            &conn,
            persona,
            "{\"kind\":\"file\",\"path\":\"/clips/a.mp4\"}",
            Some("video/mp4"),
            Some(&format!("sha256:{}", "c".repeat(64))),
        );

        migrate(&mut conn).unwrap();

        let raw = |asset: Uuid| -> Option<String> {
            conn.query_row(
                "SELECT meta_raw FROM material WHERE asset_id = ?1",
                params![asset],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            raw(asset).as_deref(),
            Some(NOT_CAPTURED),
            "a row the build that read it kept no bytes for says so"
        );
        assert_eq!(
            raw(clip).as_deref(),
            Some(NOT_CAPTURED),
            "and so does a format that will never keep any: the write is unscoped, \
             and the marker is a statement about the build rather than the format"
        );
        assert_eq!(
            fingerprint_columns(&conn, asset),
            before,
            "this step is an addition: a moved meta_kv is a moved m1-, and those are \
             frozen against rows already written"
        );
    }

    /// **A row V75 answered is not work for the fingerprint walk.**
    ///
    /// The whole reason the column is not part of
    /// [`needs_fingerprint`](asterism_core::domain::content_hash::needs_fingerprint):
    /// it holds no answer on the *raw* the day it arrives, and a rule
    /// that asked would call every row in the library half-filled — so
    /// the next launch reads every file on somebody's disk, which is the
    /// act the deferral exists to refuse.
    ///
    /// Asserted against both evaluations of the rule, like V72's, since
    /// one of them is SQL and could be widened on its own. The rows that
    /// bracket it are the point: NULL on this column is not work either,
    /// and a row that genuinely owes a pass still does.
    #[test]
    fn an_existing_row_is_not_work_because_it_has_no_raw() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 74).unwrap();
        let persona = seed_persona(&conn);

        let answered = seed_png_material(&conn, persona);
        let unfinished = seed_png_material(&conn, persona);
        conn.execute(
            "UPDATE material SET content_hash = ?2, content_region_hash = ?3, meta_hash = ?4 \
              WHERE asset_id = ?1",
            params![
                answered,
                format!("sha256:{}", "a".repeat(64)),
                format!("cr1-sha256:{}", "b".repeat(64)),
                format!("m1-sha256:{}", "c".repeat(64)),
            ],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let selected: Vec<Uuid> = conn
            .prepare(&format!(
                "SELECT asset_id FROM material WHERE {} ORDER BY asset_id",
                status_era_unfingerprinted()
            ))
            .unwrap()
            .query_map([], |row| row.get::<_, Uuid>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            !status_era_owes(&conn, answered),
            "the row carries `unsupported:not-captured` on a column nothing asks about"
        );
        assert_eq!(
            selected,
            vec![unfinished],
            "and the walk's own query agrees — only the row that never had a \
             fingerprint is work"
        );
        assert!(
            status_era_owes(&conn, unfinished),
            "which is what keeps the line above honest"
        );
    }

    /// The four columns one fingerprint pass writes, minus the one V75
    /// adds — what an addition has to leave alone.
    fn fingerprint_columns(conn: &Connection, asset: Uuid) -> Vec<Option<String>> {
        conn.query_row(
            "SELECT content_hash, content_region_hash, meta_hash, meta_kv \
             FROM material WHERE asset_id = ?1",
            params![asset],
            |row| Ok(vec![row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?]),
        )
        .unwrap()
    }

    /// V92 converts every marker-era value into its status/reason
    /// triple and leaves the digest columns holding digests and nothing
    /// else — asserted over the full vocabulary each axis could hold on
    /// the eve of the step, one row per shape.
    ///
    /// Two properties carry the weight. The mime inside an
    /// `unsupported:<mime>` marker survives, in the reason column —
    /// that is the part of the old design the issue called genuinely
    /// valuable — and a value the conversion does not recognise falls
    /// to `pending` on the versioned axes rather than being invented an
    /// answer, exactly as `is_axis_answer` read it before.
    #[test]
    fn v92_moves_every_marker_into_the_status_and_reason_columns() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 91).unwrap();
        let persona = seed_persona(&conn);

        // (content-axis value before, expected (status, value, reason)
        // after). The meta axis is exercised through the same seeding
        // helper with its own digest tag below.
        type Converted<'a> = (&'a str, Option<&'a str>, Option<&'a str>);
        let region = format!("cr1-sha256:{}", "a".repeat(64));
        let stale = format!("cr0-sha256:{}", "b".repeat(64));
        let cases: Vec<(Option<&str>, Converted<'_>)> = vec![
            (None, ("pending", None, None)),
            (Some(&region), ("computed", Some(&region), None)),
            // A superseded generation read as no answer before, and
            // still does — but the digest is a measurement and stays.
            (Some(&stale), ("pending", Some(&stale), None)),
            (Some("unsupported:empty-span"), ("empty-span", None, None)),
            (Some("unsupported:too-large"), ("too-large", None, None)),
            (Some("unsupported:not-walked"), ("not-walked", None, None)),
            (
                Some("unsupported:video/mp4"),
                ("unsupported", None, Some("video/mp4")),
            ),
            (
                Some("unsupported:unknown"),
                ("unsupported", None, Some("unknown")),
            ),
            (Some("unhashable:no-bytes"), ("no-bytes", None, None)),
        ];
        let seeded: Vec<(Uuid, Converted<'_>)> = cases
            .iter()
            .enumerate()
            .map(|(ord, (before, after))| {
                let asset = seed_asset(&conn, persona);
                conn.execute(
                    "INSERT INTO material (asset_id, ord, locator, mime, content_hash, \
                                           content_region_hash, meta_hash, \
                                           created_at, updated_at) \
                     VALUES (?1, 0, ?2, 'image/png', 'sha256:aaaa', ?3, ?4, 0, 0)",
                    params![
                        asset,
                        format!("{{\"kind\":\"file\",\"path\":\"/pics/v92-{ord}.png\"}}"),
                        before,
                        // The meta axis takes the same value, except the
                        // digest shapes, which take its own tag.
                        match *before {
                            Some(v) if v == region => Some("m1-sha256:aaaa"),
                            Some(v) if v == stale => Some("m0-sha256:bbbb"),
                            other => other,
                        },
                    ],
                )
                .unwrap();
                (asset, *after)
            })
            .collect();
        // The file axis's own two markers, on one more row: a digest
        // stays computed, and the no-bytes marker converts.
        let no_bytes = seed_asset(&conn, persona);
        conn.execute(
            "INSERT INTO material (asset_id, ord, locator, mime, content_hash, \
                                   created_at, updated_at) \
             VALUES (?1, 0, '{\"kind\":\"file\",\"path\":\"/pics/v92-nb.png\"}', \
                     'image/png', 'unhashable:no-bytes', 0, 0)",
            params![no_bytes],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        for (asset, (status, value, reason)) in &seeded {
            assert_eq!(
                axis_state_of(&conn, *asset, "content_region_hash"),
                (
                    status.to_string(),
                    value.map(str::to_string),
                    reason.map(str::to_string)
                ),
                "the content axis converts by value"
            );
            // Every seeded row's file column held a digest, and stays
            // computed with it.
            assert_eq!(
                axis_state_of(&conn, *asset, "content_hash"),
                ("computed".to_string(), Some("sha256:aaaa".into()), None)
            );
        }
        assert_eq!(
            axis_state_of(&conn, no_bytes, "content_hash"),
            ("no-bytes".to_string(), None, None),
            "the file axis's marker converts like the others"
        );

        // The digest columns hold digests and nothing else — the
        // invariant every reader now leans on.
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM material \
                 WHERE content_hash GLOB 'unsupported:*' \
                    OR content_hash = 'unhashable:no-bytes' \
                    OR content_region_hash GLOB 'unsupported:*' \
                    OR content_region_hash = 'unhashable:no-bytes' \
                    OR meta_hash GLOB 'unsupported:*' \
                    OR meta_hash = 'unhashable:no-bytes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            leftovers, 0,
            "no marker spelling survives in a digest column"
        );
    }

    /// V93 renames the whole-file `.json` rows to the mime `guess_mime`
    /// answers now, and V94 hands exactly those rows back to the
    /// fingerprint walk — asserted over the locator shapes the sniff
    /// distinguishes, one row per judgement.
    ///
    /// Two properties carry the weight. The pair of migrations moves a
    /// row from "answered: unsupported" to "pending" **only** where the
    /// declared mime changed — `.jsonl`, a record inside a container,
    /// and a mime an importer stated all keep both their mime and their
    /// answer — and the meta axis moves on no row at all, because the
    /// JSON probe does not claim it and a cleared row would only be
    /// re-refused.
    #[test]
    fn v93_renames_whole_json_rows_and_v94_hands_them_back_to_the_walk() {
        let mut conn = test_conn();
        migrate_to(&mut conn, 92).unwrap();
        let persona = seed_persona(&conn);

        // One row per locator judgement, seeded in the shape a `.json`
        // row held on the eve of the step: declared `text/plain`,
        // refused on both walking axes with that mime as the reason,
        // file axis answered.
        let seed = |locator: &str, mime: &str| -> Uuid {
            let asset = seed_asset(&conn, persona);
            conn.execute(
                "INSERT INTO material (asset_id, ord, locator, mime, \
                                       content_hash, content_hash_status, \
                                       content_region_hash_status, \
                                       content_region_hash_reason, \
                                       meta_hash_status, meta_hash_reason, \
                                       created_at, updated_at) \
                 VALUES (?1, 0, ?2, ?3, 'sha256:aaaa', 'computed', \
                         'unsupported', ?3, 'unsupported', ?3, 0, 0)",
                params![asset, locator, mime],
            )
            .unwrap();
            asset
        };

        let whole = seed(r#"{"kind":"file","path":"/docs/a.json"}"#, "text/plain");
        let upper = seed(r#"{"kind":"file","path":"/docs/B.JSON"}"#, "text/plain");
        let lines = seed(r#"{"kind":"file","path":"/docs/c.jsonl"}"#, "text/plain");
        let record = seed(
            r#"{"kind":"record","container":"/docs/d.json","record":"one"}"#,
            "text/plain",
        );
        let queried = seed(
            r#"{"kind":"remote","scheme":"https","target":"https://x/y.json?sig=1"}"#,
            "text/plain",
        );
        let logical = seed(
            r#"{"kind":"logical","name":"harvest/f.json"}"#,
            "text/plain",
        );
        let stated = seed(
            r#"{"kind":"file","path":"/docs/e.json"}"#,
            "application/octet-stream",
        );
        // A file *named* `.json` has no extension to `Path::extension`,
        // so the sniff never called it JSON — the seam the file arm's
        // second condition exists for.
        let hidden = seed(r#"{"kind":"file","path":"/docs/.json"}"#, "text/plain");

        migrate(&mut conn).unwrap();

        let mime_of = |asset: Uuid| -> String {
            conn.query_row(
                "SELECT mime FROM material WHERE asset_id = ?1",
                params![asset],
                |r| r.get(0),
            )
            .unwrap()
        };

        // The renamed rows: `guess_mime`'s judgement, including the
        // lowercasing and the query strip, and the walk owes them again.
        for asset in [whole, upper, queried, logical] {
            assert_eq!(mime_of(asset), "application/json");
            assert_eq!(
                axis_state_of(&conn, asset, "content_region_hash"),
                ("pending".to_string(), None, None),
                "V94 returns the refused content axis to \"nobody has looked\""
            );
            assert_eq!(
                axis_state_of(&conn, asset, "meta_hash"),
                (
                    "unsupported".to_string(),
                    None,
                    Some("text/plain".to_string())
                ),
                "the meta axis is not claimed and does not move"
            );
            assert!(
                status_era_owes(&conn, asset),
                "a pending content axis re-enters the fingerprint walk"
            );
        }

        // The rows the sniff never called `.json`: mime and answer both
        // stay, and the walk still considers them answered.
        for (asset, mime) in [
            (lines, "text/plain"),
            (record, "text/plain"),
            (hidden, "text/plain"),
            (stated, "application/octet-stream"),
        ] {
            assert_eq!(mime_of(asset), mime);
            assert_eq!(
                axis_state_of(&conn, asset, "content_region_hash"),
                ("unsupported".to_string(), None, Some(mime.to_string())),
            );
            assert!(!status_era_owes(&conn, asset), "{mime} stays answered");
        }
    }
}
