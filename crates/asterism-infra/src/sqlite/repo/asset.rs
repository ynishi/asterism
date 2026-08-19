//! SQLite adapter for the `AssetRepository` port.
//!
//! Hot-path methods (`list` / `list_index`) build [`AssetCard`] /
//! [`AssetIndex`](asterism_core::domain::asset::AssetIndex) projections
//! directly from a row scan without materialising the full entity.
//! Visibility filtering is always applied inside SQL; to make it
//! impossible to forget, the `WHERE` clause is built by a single
//! [`QueryParts`] helper — `filter_ids` (the SQL half of the search
//! path) goes through the same builder.

use asterism_contract::query::TagMatch;
use asterism_core::domain::asset::{
    Asset, AssetCard, AssetQuery, ContentFlags, TrashFilter, UNCLASSIFIED_MODALITY,
};
use asterism_core::domain::attribution::{Author, OperatorRef, PersistedAttribution};
use asterism_core::domain::color::{ColorBucket, buckets_of};
use asterism_core::domain::duplicate_conflict::DuplicateAxis;
use asterism_core::domain::duplicate_conflict::{
    ConflictResolution, DuplicateConflict, FoldExclusion,
};
use asterism_core::domain::material::Material;
use asterism_core::domain::merge_plan::MergePlan;
use asterism_core::domain::repository::{
    AssetRepository, ChapterScanCandidate, DimsCandidate, DimsProbe, DimsScope, DimsWritePolicy,
    DuplicateGroup, FingerprintedMaterial, FoldOutcome, FoldRefusal, FoldReport,
    MaterialFingerprint, MergeOutcome, SourceLookupScope, UnhashedMaterial,
};
use asterism_core::domain::session::{Session, SessionMetadata};
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::domain::value::{
    AssetId, AssetRole, BundleId, CoverText, DuplicateConflictId, ExternalSessionKey, FoldPolicy,
    GroupId, Keyword, Label, MimeType, Modality, OnDuplicate, Page, PersonaId, RegisterNote,
    SessionId, SourceKind, SourceRef, Viewer, Visibility, dedup_labels,
};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use rusqlite::types::Value;
use rusqlite_isle::AsyncIsle;
use std::collections::HashMap;
use uuid::Uuid;

use crate::sqlite::map::{
    datetime_to_ms, infra_err, json_to_strings, ms_to_datetime, opt_u32, opt_u64, strings_to_json,
};

/// Hard upper bound on page size (guards against absurd limits).
///
/// V1 was 1_000 while the Album target was ~thousands. Raised to
/// 200_000 during Round 2 dogfood so stress runs in the tens of
/// thousands up to ~100k rows can
/// drive the grid end-to-end without server-side truncation
/// masquerading as a UI-side problem.
const MAX_LIMIT: u64 = 200_000;

/// Ids per `asset.id IN (…)` batch in [`filter_ids`].
///
/// SQLite's `SQLITE_MAX_VARIABLE_NUMBER` is 32_766 on modern builds, and
/// the filter predicate itself already spends a handful of slots (tags /
/// groups / viewer), so the id list is chunked well below the ceiling
/// rather than trusting the caller to keep the candidate set small.
const MAX_ID_FILTER_CHUNK: usize = 500;

/// The `WHERE` fragment that keeps duplicate matching to the values the
/// domain says may stand for "the same picture".
///
/// A prefix test rather than `IS NOT NULL`: the column also holds
/// the `unhashable:` marker for materials that can never have a digest
/// (container records, remote locators), and every one of those shares
/// the same string — grouping without this would report the whole
/// conversation corpus as one duplicate set. The empty-input digest is
/// excluded for the sibling reason: every 0-byte file (usually
/// failed-download debris) carries the same real digest, and a group
/// built from them reads as "the same picture N times" when nothing was
/// ever downloaded.
///
/// Which values have to be named one by one is asked of
/// [`is_duplicate_key`](asterism_core::domain::content_hash::is_duplicate_key)
/// rather than restated here: the prefix test is the half SQL can
/// express, so what is left over is exactly the reserved values that
/// pass it and the domain still refuses. The domain owns the rule; this
/// is only its SQLite dialect, and
/// `the_sql_duplicate_filter_matches_the_domain_predicate` pins the two
/// evaluations to the same verdicts.
///
/// `GLOB` rather than `LIKE` for the prefix: SQLite's `LIKE` folds ASCII
/// case, so `SHA256:…` would pass here and fail
/// [`is_duplicate_key`](asterism_core::domain::content_hash::is_duplicate_key)
/// — the one input on which the two evaluations could disagree. No such
/// value can be stored today (`ContentHasher::finish` writes lower case),
/// which is what makes closing the gap free rather than a change of
/// verdict on any real row.
fn duplicate_key_condition(axis: DuplicateAxis, column: &str) -> String {
    use asterism_core::domain::content_hash;

    // `GLOB` reads `*`, `?` and `[` as syntax. The prefixes are module
    // constants with none of them, and `the_digest_prefixes_are_glob_safe`
    // fails if a later algorithm tag grows one — escaping a constant
    // that cannot vary would be dead code, refusing to guess is not.
    let prefix = content_hash::digest_prefix(axis);
    let mut sql = format!("{column} GLOB '{prefix}*'");
    for value in content_hash::reserved_values(axis)
        .iter()
        .filter(|value| value.starts_with(prefix) && !content_hash::is_duplicate_key(axis, value))
    {
        // Module constants today, quoted the way any literal has to be:
        // a reserved value that ever grows an apostrophe must not end
        // the string early.
        sql.push_str(&format!(" AND {column} <> '{}'", value.replace('\'', "''")));
    }
    sql
}

/// The `material` column holding `axis`'s value, when the material
/// layer holds one at all.
///
/// One function rather than a literal at each site because the duplicate
/// report reads the column **twice** — once to find which values are
/// shared, once to fetch the cards holding each — and the two reading
/// different columns would return groups keyed by one fingerprint whose
/// members were selected by another. That mismatch produces a plausible
/// answer (a group, with members, of the right shape) rather than an
/// error, which is the kind of wrong that survives a review.
///
/// Here rather than on [`DuplicateAxis`] on purpose: which column a value
/// lands in is this adapter's arrangement, and a `storage()` on the
/// domain type would be the domain knowing where rows live. The cost is
/// that the mapping is stated in the adapter — and the compiler still
/// asks for every axis, which is what the enum is for.
fn axis_column(axis: DuplicateAxis, table: &str) -> String {
    let column = match axis {
        DuplicateAxis::Artefact => "content_hash",
        DuplicateAxis::Content => "content_region_hash",
        DuplicateAxis::Meta => "meta_hash",
    };
    format!("{table}.{column}")
}

/// The `WHERE` fragment naming the materials that still owe a
/// fingerprint pass — the SQL dialect of
/// [`needs_fingerprint`](asterism_core::domain::content_hash::needs_fingerprint).
///
/// Built here, from one function, because the rule is evaluated by two
/// statements that have to agree exactly: the backfill's page query and
/// the count behind the "still fingerprinting" notice. They used to
/// share a literal (`content_hash IS NULL`) short enough to retype
/// correctly; with a second column and a versioned vocabulary it is no
/// longer that, and the failure of retyping it is invisible — the count
/// and the walk simply describe different sets, and the notice either
/// never clears or clears while work remains.
///
/// The domain owns the rule and the third evaluation of it is the
/// per-asset job's skip test, in Rust;
/// `the_sql_fingerprint_filter_matches_the_domain_predicate` runs one
/// vector of column shapes through this and through the domain and
/// requires the verdicts to match.
///
/// **The NULL cases are spelled out rather than left to the GLOBs.** A
/// `GLOB` against NULL is NULL, and `NOT NULL` is NULL, which is not
/// true — so a row with no value at all in a versioned column would
/// fail the test that is supposed to be selecting it.
pub(crate) fn unfingerprinted_condition(
    file_column: &str,
    content_column: &str,
    meta_column: &str,
) -> String {
    use asterism_core::domain::content_hash::{
        CONTENT_DIGEST_PREFIX, META_DIGEST_PREFIX, UNHASHABLE,
    };
    use asterism_core::domain::content_region::UNSUPPORTED_PREFIX;

    // One fragment per versioned column, built the same way: the
    // domain's `is_axis_answer` is a prefix test on that axis's tag plus
    // the two markers, and the markers are shared across axes because
    // they say something about the artefact rather than about the
    // measurement.
    let answered = |column: &str, prefix: &str| {
        format!(
            "({column} GLOB '{prefix}*' \
              OR {column} GLOB '{UNSUPPORTED_PREFIX}*' \
              OR {column} = '{UNHASHABLE}')"
        )
    };
    format!(
        "{file_column} IS NULL \
         OR {content_column} IS NULL \
         OR NOT {} \
         OR {meta_column} IS NULL \
         OR NOT {}",
        answered(content_column, CONTENT_DIGEST_PREFIX),
        answered(meta_column, META_DIGEST_PREFIX)
    )
}

/// The `WHERE` fragment naming the materials the content axis has no
/// reading of — the SQL dialect of
/// [`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk).
///
/// A different question from [`unfingerprinted_condition`] over the same
/// column, and kept in a separate function for the reason the domain
/// gives for keeping the two predicates apart: a value that is an
/// *answer* to one is *work* for the other, so a single fragment with a
/// flag would put the whole pre-existing library into the ordinary
/// walk's page query the first time somebody passed the flag wrong.
///
/// Equality against the one marker, quoted rather than bound, because
/// this is composed into statements that carry their own parameters —
/// `the_digest_prefixes_are_glob_safe` is what keeps the interpolation
/// honest.
///
/// **The migration that clears the marker deliberately does not call
/// this.** Migration steps are append-only and frozen once shipped; a
/// step reaching into a helper that a later wave may widen would make an
/// old migration change what it did. It spells the same equality itself,
/// against the same domain constant.
pub(crate) fn unwalked_condition(content_column: &str) -> String {
    use asterism_core::domain::content_region::NOT_WALKED;

    format!("{content_column} = '{}'", NOT_WALKED.replace('\'', "''"))
}

/// Table-qualifies a bare column list (`"id, persona_id, …"` →
/// `"asset.id, asset.persona_id, …"`).
///
/// Derived rather than kept as a second constant: the two lists would
/// have to be edited together forever, and the column order is the
/// positional contract `AssetRow::from_row` reads by — a list that
/// drifted would not fail to compile, it would return the wrong
/// column under the right name.
///
/// Only for a list of **bare column names**, which is what
/// `AssetRow::COLUMNS` is. `CardRow::COLUMNS` is not: it carries
/// expressions (`(register_note IS NOT NULL) AS has_note`, a scalar
/// subquery), and prefixing one of those yields `asset.(register_note
/// …)` — SQL that compiles here and fails at `prepare`.
fn qualify(columns: &str, table: &str) -> String {
    columns
        .split(',')
        .map(|column| format!("{table}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The statement behind
/// [`find_by_content_hash`](AssetRepository::find_by_content_hash),
/// as a function so the query-plan test measures the statement that
/// actually runs rather than a copy of it that can drift away from it.
///
/// `?1` = persona uuid, `?2` = the digest. The column the digest is
/// compared against follows [`axis_column`] rather than being spelled
/// here, so the two axes cannot end up reading each other's values.
///
/// `CROSS JOIN` is not a different join — SQLite computes the same rows
/// — it is the one way to tell the planner which table is the outer
/// loop. It is here because the obvious spelling was measured and was
/// wrong: with `asset … WHERE id IN (SELECT … FROM material …)` SQLite
/// chose
///
/// ```text
/// SEARCH asset USING INDEX idx_asset_persona_occurred (persona_id=?)
/// LIST SUBQUERY 1
/// SEARCH m USING COVERING INDEX idx_material_content_hash (content_hash=?)
/// ```
///
/// — every asset of the persona walked, each tested for membership.
/// That is a per-persona scan on a lookup that runs once per
/// fingerprint, and it gets slower exactly as the library grows.
/// Driving from the digest instead costs the handful of rows that hold
/// it, and a primary-key hit per row. The join order matters far more
/// here than any index would: the material side already has one
/// (`idx_material_content_hash`, V41), which is why this subtask adds
/// no V50. The content axis has the same shape of index on its own
/// column (`idx_material_content_region_hash`, V55), so switching the
/// axis switches which covering index drives the join and nothing else.
fn content_hash_lookup_sql(axis: DuplicateAxis) -> String {
    format!(
        "SELECT {} FROM material m \
          CROSS JOIN asset ON asset.id = m.asset_id \
          WHERE {} = ?2 AND m.ord = 0 \
            AND asset.persona_id = ?1 \
            AND asset.folded_into IS NULL \
          ORDER BY asset.occurred_at ASC, asset.id ASC",
        qualify(AssetRow::COLUMNS, "asset"),
        axis_column(axis, "m")
    )
}

/// The `edge` rows a fold has to remove rather than move, as the
/// `_trace` note records them.
///
/// An edge is somebody's assertion — an exporter naming what it made,
/// a person declaring a backlink, a fingerprint agreeing with another
/// (`EdgeKind::is_synth` calls all three "not recomputable"). Two of
/// them cannot survive a fold: the pair's own edge, which would become
/// a keeper→keeper self-loop, and one whose `(from, to, kind)` the
/// keeper already holds, because the table admits a single row per
/// triple. Deleting either without a record would make the claim
/// disappear with no trace that it was ever made, which the fold
/// refuses to do.
///
/// The note goes on the **headstone**: the dropped edge was the
/// headstone's claim, and the headstone is the row that stops having
/// it. The keeper's own `_trace` records a different thing — the values
/// of the headstone it did *not* take ([`extra_with_absorbed_note`]).
fn dropped_edge_note(
    from: Uuid,
    to: Uuid,
    kind: String,
    label: Option<String>,
    weight: Option<f64>,
    why: &str,
) -> serde_json::Value {
    serde_json::json!({
        "from": from.to_string(),
        "to": to.to_string(),
        "kind": kind,
        "label": label,
        "weight": weight,
        "why": why,
    })
}

/// Writes `note` under `_trace.fold` in an `extra` bag, preserving
/// whatever is already in it. `None` means "do not write" — the bag
/// held something this cannot merge into without destroying it, and a
/// fold is not a reason to lose an importer's data.
///
/// The non-object case follows the ingest side's rule
/// (`asset_service::merge_extra_key`): a bag that is not an object is
/// carried under `_extra` rather than overwritten. A `_trace` that is
/// not an object is refused instead, because the alternative is
/// dropping keys the resolution paths read (`resolved`, `dispatch_id`).
fn extra_with_fold_note(extra: Option<&str>, note: serde_json::Value) -> Option<String> {
    use asterism_core::domain::provenance::TRACE_KEY;

    let mut bag = match extra {
        None => serde_json::Value::Object(serde_json::Map::new()),
        Some(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Null) => serde_json::Value::Object(serde_json::Map::new()),
            Ok(value) if value.is_object() => value,
            Ok(other) => serde_json::json!({ "_extra": other }),
            Err(_) => return None,
        },
    };
    let map = bag.as_object_mut()?;
    let trace = map
        .entry(TRACE_KEY.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let trace = trace.as_object_mut()?;
    trace.insert("fold".to_string(), note);
    Some(bag.to_string())
}

/// Appends `entry` to the `_trace.absorbed` list of an `extra` bag,
/// preserving whatever is already in it. `None` means the same thing it
/// means for [`extra_with_fold_note`]: the bag holds something this
/// cannot merge into without destroying it.
///
/// A **list**, unlike the headstone's `_trace.fold`: a row is folded
/// once, but a keeper can absorb one duplicate after another, and an
/// object at this key would leave only the last of them. An existing
/// value that is not a list is refused rather than replaced, for the
/// reason the sibling refuses a non-object `_trace`.
fn extra_with_absorbed_note(extra: Option<&str>, entry: serde_json::Value) -> Option<String> {
    use asterism_core::domain::provenance::TRACE_KEY;

    let mut bag = match extra {
        None => serde_json::Value::Object(serde_json::Map::new()),
        Some(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Null) => serde_json::Value::Object(serde_json::Map::new()),
            Ok(value) if value.is_object() => value,
            Ok(other) => serde_json::json!({ "_extra": other }),
            Err(_) => return None,
        },
    };
    let map = bag.as_object_mut()?;
    let trace = map
        .entry(TRACE_KEY.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let trace = trace.as_object_mut()?;
    let absorbed = trace
        .entry("absorbed".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    absorbed.as_array_mut()?.push(entry);
    Some(bag.to_string())
}

/// Writes `note` at `_trace.<field>` of an `extra` bag, keeping
/// whatever else the bag and the note object hold. `None` means the
/// same thing it means for the two helpers above: the column carries
/// something this cannot merge into without destroying it.
///
/// **One key is replaced and its neighbours are left alone**, which is
/// the property every caller depends on: `_trace` is a shared bag whose
/// writers do not know about each other — `fold` and `absorbed` above,
/// the declared-hash verdict, the disclosure note, the claim fields.
///
/// Each field is an **object**, not a list like `_trace.absorbed`,
/// because both current keys answer a question that has one current
/// answer. A declaration is made once per registration and the hash job
/// answers it once; a disclosure is whatever the last stamp achieved. A
/// re-run answers the same question about the state that is there now,
/// and a list would grow an entry per sweep and leave a reader working
/// out which line is current. A future key whose history matters wants
/// `absorbed`'s shape instead, and should say so where it is defined.
fn extra_with_trace_field(
    extra: Option<&str>,
    field: &str,
    note: serde_json::Value,
) -> Option<String> {
    use asterism_core::domain::provenance::TRACE_KEY;

    let mut bag = match extra {
        None => serde_json::Value::Object(serde_json::Map::new()),
        Some(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Null) => serde_json::Value::Object(serde_json::Map::new()),
            Ok(value) if value.is_object() => value,
            Ok(other) => serde_json::json!({ "_extra": other }),
            Err(_) => return None,
        },
    };
    let map = bag.as_object_mut()?;
    let trace = map
        .entry(TRACE_KEY.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let trace = trace.as_object_mut()?;
    trace.insert(field.to_string(), note);
    Some(bag.to_string())
}

/// How a fold combines one column of the two rows.
///
/// Each variant is a rule that needs no verdict about which of two
/// people was right — which is the whole reason only six columns have
/// one. Everything else keeps the keeper's value and records what it
/// did not take; see the port doc for the full division.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeRule {
    /// JSON array of strings: the keeper's entries in their own order,
    /// then whatever the headstone had and it did not.
    UnionJson,
    /// Boolean stored as `0` / `1`, combined with `OR`. Used where one
    /// side being set is the answer for the pair — `vis_restricted`
    /// must not be relaxed by folding an open row into a closed one.
    Or,
    /// Largest value, counting only those `> 0`. `rating` uses `0` and
    /// `NULL` for "nobody rated this", and a maximum that took `0` for
    /// a score would be the same number for "unrated" and "rated
    /// zero".
    MaxPositive,
    /// Free text: the non-empty halves joined by a blank line, in
    /// keeper-then-headstone order. Identical text is left alone rather
    /// than doubled — a re-run of the same fold, or two rows that
    /// carried the same note, must not grow the paragraph each time.
    JoinText,
}

/// The columns a fold combines, and the rule each one follows.
const MERGED_COLUMNS: &[(&str, MergeRule)] = &[
    ("labels", MergeRule::UnionJson),
    ("keywords", MergeRule::UnionJson),
    ("vis_sharing", MergeRule::UnionJson),
    ("vis_restricted", MergeRule::Or),
    ("rating", MergeRule::MaxPositive),
    ("register_note", MergeRule::JoinText),
    // The four content-type flags, on the rule the domain already
    // states for them: `ContentFlags::merge` is an OR, which is what
    // the Sessions view has always done when it aggregates a
    // container's members (`MAX(asset.has_code)`, V8). One body
    // carrying a table does not stop carrying it because the row it
    // was folded into had none, and the flags are what a facet chip
    // reaches rows by — dropping the headstone's would make a card
    // disappear from a filter it belongs in.
    ("has_code", MergeRule::Or),
    ("has_table", MergeRule::Or),
    ("has_mermaid", MergeRule::Or),
    ("has_link", MergeRule::Or),
];

/// The columns where the keeper's value stands and the headstone's is
/// written to `_trace.absorbed` when the two disagree.
///
/// Single-valued, every one of them: combining would mean choosing
/// between two claims, and a fold has no basis for that choice. The
/// keeper winning is a decision, not an observation, so what it beat is
/// recorded rather than assumed equal — same bytes do not make two
/// people's titles the same title.
const KEPT_COLUMNS: &[&str] = &[
    "source_kind",
    "source_locator",
    // A Prop the keeper holds and the headstone records, on the same
    // terms as the two above it. `external_key` is what a source said
    // this row is called on the outside; handing the headstone's to the
    // keeper would take that name from the row that answers to it.
    //
    // Nothing about the column refuses a transplant — V62 took the
    // UNIQUE off it, because an external record legitimately arrives
    // twice and two platforms number their records alike. The rule
    // here is the model's, not an index's: a Prop is never promoted to
    // an identity, so the keeper keeps its own name and the headstone's
    // is recorded rather than moved.
    "external_key",
    "file_size_bytes",
    "platform",
    "modality",
    "occurred_at",
    "bundle_id",
    "cover",
    "duration_ms",
    "palette",
    "container_id",
    "title",
    "role",
    "author_kind",
    "author_subject",
    "operator_ai",
    "attributed_via",
    "on_duplicate",
    "fold_policy",
    // The coded pixel dimensions (V69), on the same rule as
    // `file_size_bytes` and `duration_ms` above: a single-valued
    // measurement the keeper keeps, with the other row's recorded.
    //
    // Judged **per column**, which is what this list can express and is
    // worth knowing: two rows that agree on width and differ on height
    // leave only `height_px` in `_trace.absorbed`, so the note does not
    // say that the number was half of one resolution. Changing that means
    // changing the note's shape, not this list.
    "width_px",
    "height_px",
];

/// The columns a fold neither combines nor compares. See the port doc
/// for why each one is here.
///
/// Nothing reads it at run time — leaving a column alone needs no code.
/// It exists so the three lists together are a statement about the
/// whole table, which `the_three_column_groups_cover_the_table` holds
/// to `PRAGMA table_info(asset)`: a column added later belongs to none
/// of them and has to be given a rule rather than inheriting one by
/// accident. Held to a `SELECT` list instead, a column reached the
/// guard only by having been typed into that list.
#[cfg(test)]
const UNTOUCHED_COLUMNS: &[&str] = &[
    "id",
    "persona_id",
    "created_at",
    "updated_at",
    "trashed_at",
    "extra",
    "folded_into",
    // Bookkeeping about a read of *this row's* bytes, so it says
    // nothing about the loser's and nothing to combine. Deliberately
    // not beside `width_px` / `height_px` in the merged list even
    // though it travels with them: absorbing it would mark the keeper
    // as probed on the strength of somebody else's probe, and a keeper
    // that still has no dimensions would then never be looked at again.
    "dims_probed_at",
];

/// The `SELECT` list the merge reads for both rows: the combined
/// columns first, then the kept ones, so the positional indices below
/// follow the two constants.
fn merge_read_columns() -> String {
    MERGED_COLUMNS
        .iter()
        .map(|(column, _)| *column)
        .chain(KEPT_COLUMNS.iter().copied())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A column value as the `_trace` note records it.
///
/// `BLOB` is the id representation (`container_id` is the only one that
/// reaches here), rendered as a UUID so the note reads like every other
/// id in `_trace` rather than as a byte array.
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Integer(n) => serde_json::json!(n),
        Value::Real(f) => serde_json::json!(f),
        Value::Text(text) => serde_json::json!(text),
        Value::Blob(bytes) => match <[u8; 16]>::try_from(bytes.as_slice()) {
            Ok(raw) => serde_json::json!(Uuid::from_bytes(raw).to_string()),
            Err(_) => serde_json::json!(format!("{} bytes", bytes.len())),
        },
    }
}

/// Reads a JSON array column as a string list. `None` = the column did
/// not hold one, which the caller turns into "leave this column alone"
/// rather than into an empty list — a corrupt bag must not be silently
/// replaced by nothing.
fn json_string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Text(text) => serde_json::from_str::<Vec<String>>(text).ok(),
        _ => None,
    }
}

/// `NULL` and `""` are the same answer here: nobody wrote a note.
fn text_or_empty(value: &Value) -> &str {
    match value {
        Value::Text(text) => text.trim(),
        _ => "",
    }
}

/// What the merge decided about the keeper's columns.
struct KeeperMerge {
    /// Columns to write, with their new values. Empty when the two rows
    /// agreed about everything a rule could combine.
    writes: Vec<(&'static str, Value)>,
    /// What the headstone held where the keeper's own value stood, by
    /// column name. Empty when the two rows agreed.
    discarded: serde_json::Map<String, serde_json::Value>,
    /// Columns whose stored representation could not be read as the
    /// rule needs it (a corrupt JSON array). Named so the fold can say
    /// so out loud instead of writing a value it guessed.
    unreadable: Vec<&'static str>,
}

/// Applies the rules of [`MERGED_COLUMNS`] and [`KEPT_COLUMNS`] to one
/// pair of rows, both read positionally through [`merge_read_columns`].
///
/// Pure, and separated from the transaction on purpose: every rule is a
/// judgement about what a duplicate resolution owes the person who put
/// the values there, and a judgement wants a test that does not need a
/// database to state it.
fn merge_keeper_columns(keeper: &[Value], headstone: &[Value]) -> KeeperMerge {
    let mut merge = KeeperMerge {
        writes: Vec::new(),
        discarded: serde_json::Map::new(),
        unreadable: Vec::new(),
    };
    for (index, (column, rule)) in MERGED_COLUMNS.iter().enumerate() {
        let (mine, theirs) = (&keeper[index], &headstone[index]);
        match rule {
            MergeRule::UnionJson => {
                let (Some(mine), Some(theirs)) = (json_string_list(mine), json_string_list(theirs))
                else {
                    merge.unreadable.push(column);
                    continue;
                };
                let mut union = mine.clone();
                for entry in theirs {
                    if !union.contains(&entry) {
                        union.push(entry);
                    }
                }
                if union != mine {
                    merge
                        .writes
                        .push((column, Value::Text(strings_to_json(&union))));
                }
            }
            MergeRule::Or => {
                let flag = |value: &Value| matches!(value, Value::Integer(n) if *n != 0);
                if flag(theirs) && !flag(mine) {
                    merge.writes.push((column, Value::Integer(1)));
                }
            }
            MergeRule::MaxPositive => {
                let score = |value: &Value| match value {
                    Value::Integer(n) if *n > 0 => Some(*n),
                    _ => None,
                };
                match (score(mine), score(theirs)) {
                    (mine, Some(theirs)) if mine.is_none_or(|mine| theirs > mine) => {
                        merge.writes.push((column, Value::Integer(theirs)));
                    }
                    _ => {}
                }
            }
            MergeRule::JoinText => {
                let (mine, theirs) = (text_or_empty(mine), text_or_empty(theirs));
                if theirs.is_empty() || mine == theirs {
                    continue;
                }
                let joined = if mine.is_empty() {
                    theirs.to_string()
                } else {
                    format!("{mine}\n\n{theirs}")
                };
                merge.writes.push((column, Value::Text(joined)));
            }
        }
    }
    for (offset, column) in KEPT_COLUMNS.iter().enumerate() {
        let index = MERGED_COLUMNS.len() + offset;
        let (mine, theirs) = (&keeper[index], &headstone[index]);
        if mine != theirs {
            merge
                .discarded
                .insert((*column).to_string(), value_to_json(theirs));
        }
    }
    merge
}

/// Reads the edges a fold is about to delete and appends them to
/// `out` as notes. Shares its `WHERE` clause with the `DELETE` that
/// follows it, passed in rather than repeated, so the two cannot
/// select different rows.
fn collect_dropped_edges(
    tx: &rusqlite::Transaction<'_>,
    predicate: &str,
    params: impl rusqlite::Params,
    why: &str,
    out: &mut Vec<serde_json::Value>,
) -> Result<(), rusqlite::Error> {
    let mut stmt = tx.prepare(&format!(
        "SELECT from_asset, to_asset, kind, label, weight FROM edge WHERE {predicate}"
    ))?;
    let rows = stmt.query_map(params, |row| {
        Ok(dropped_edge_note(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            why,
        ))
    })?;
    for note in rows {
        out.push(note?);
    }
    Ok(())
}

/// Tells apart the reasons a fold's marking statement matched nothing.
///
/// Runs **inside the fold's own transaction**, so what it reports is
/// the state that statement was evaluated against rather than a later
/// one — the same care [`purge`](AssetRepository::purge) takes with its
/// follow-up probe, which is there for the same purpose.
///
/// The last arm is unreachable by construction (both rows foldable, yet
/// the statement matched none) and fails loudly instead of guessing a
/// verdict: a wrong reason here would be reported to the user as a
/// resolved question.
fn classify_fold_refusal(
    tx: &rusqlite::Transaction<'_>,
    head: Uuid,
    keep: Uuid,
) -> Result<FoldRefusal, rusqlite::Error> {
    use rusqlite::OptionalExtension;

    if head == keep {
        return Ok(FoldRefusal::SameAsset);
    }
    let headstone: Option<bool> = tx
        .query_row(
            "SELECT folded_into IS NOT NULL FROM asset WHERE id = ?1",
            params![head],
            |row| row.get(0),
        )
        .optional()?;
    match headstone {
        None => return Ok(FoldRefusal::Missing),
        Some(true) => return Ok(FoldRefusal::AlreadyFolded),
        Some(false) => {}
    }
    let keeper: Option<(bool, bool)> = tx
        .query_row(
            "SELECT folded_into IS NOT NULL, trashed_at IS NOT NULL FROM asset WHERE id = ?1",
            params![keep],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match keeper {
        None => Ok(FoldRefusal::KeeperMissing),
        Some((true, _)) => Ok(FoldRefusal::KeeperFolded),
        Some((false, true)) => Ok(FoldRefusal::KeeperTrashed),
        Some((false, false)) => Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("fold: nothing was marked although both rows are foldable".into()),
        )),
    }
}

/// Folds one row into another inside a transaction **the caller owns**
/// — the whole of what a fold does, and the only copy of it.
///
/// Two verbs stand a headstone: [`fold_into`](AssetRepository::fold_into)
/// resolves one detected pair, and
/// [`merge_into`](AssetRepository::merge_into) carries out a person's
/// ruling over a set. They differ in what decides the pair and in how
/// many pairs there are, not in what folding *is*, so a second body here
/// would be two answers to one question: the day a column joins
/// [`MERGED_COLUMNS`] or a table joins the re-point, one of the two
/// would get it. The extraction is also why the manual verb needs no
/// tests of its own for the column rules — they are the same statements
/// the fold tests already hold.
///
/// The transaction is a parameter for the same reason. A fold is atomic
/// on its own; a merge is atomic over the whole set (see the port doc),
/// and that is only expressible if the caller decides when to commit.
/// **This function never commits and never rolls back** — a `Skipped`
/// return means it wrote nothing, which is what lets a caller keep going
/// or give up as it chooses.
///
/// The statement order is not arbitrary — the edge half only works this
/// way round:
///
/// 1. **Mark the headstone.** The predicate carries the whole re-read
///    (see the port doc), so this is both the guard and the write. Zero
///    rows here means the call ends having written nothing.
/// 2. **Remove the pair's own edges**, having first read what they
///    claimed. They have to go before the re-point rather than after:
///    `UPDATE` would turn `(headstone, keeper)` into `(keeper, keeper)`,
///    a self-loop the `edge` table has no constraint against — nothing
///    downstream would catch it.
/// 3. **Re-point both sides.** `UPDATE OR IGNORE`, because the keeper
///    may already hold an edge with the same `(from, to, kind)`;
///    without `OR IGNORE` that collision aborts the whole fold, and the
///    pair a user asked to resolve would be un-resolvable for as long as
///    the two shared a neighbour.
/// 4. **Sweep what stayed behind.** After the two updates, an edge still
///    naming the headstone is one `OR IGNORE` refused. Same treatment as
///    step 2: read the claim, write it to `_trace`, delete the row.
/// 5. **Move the filing** — Groups, container children, tags. Each is
///    `INSERT OR IGNORE … SELECT` then `DELETE`, the shape
///    `TagRepository::merge` already uses for "the target keeps what it
///    had, and gains the rest".
/// 6. **Move the conversation** — comments and card-anchored threads,
///    both plain re-points (neither table constrains what the keeper may
///    already have).
/// 7. **Merge the columns.** Both rows are read here, and the keeper is
///    written by **one statement naming only the columns a rule
///    decided** — never a whole-row save, which would carry back every
///    column as it looked when the entity was loaded. Which column
///    follows which rule is the port doc's table; this is the three
///    constants above it.
///
/// `updated_at` is bumped on every row this moves — the headstone, any
/// re-filed child, and the keeper — because
/// `ListAssetsQuery::updated_from_ms` is how a differential sync learns
/// anything changed (V46). A re-parented card that kept its old stamp
/// would be invisible to every poller, and so would a keeper that
/// quietly gained six tags and a comment thread.
///
/// `now` is passed in rather than read here so that every fold of one
/// merge carries the same stamp: a set folded together is one event, and
/// a reader sorting the keeper's `_trace.absorbed` by `at_ms` would
/// otherwise see the members of a single ruling spread over the
/// milliseconds the loop happened to take.
fn fold_one(
    tx: &rusqlite::Transaction<'_>,
    head: Uuid,
    keep: Uuid,
    now: i64,
) -> Result<FoldOutcome, rusqlite::Error> {
    // (1) Guard + mark, one statement. A concurrent worker folding the
    // other way round can only make this match zero rows — see the port
    // doc for why the pair is not a `SELECT` followed by an `UPDATE`.
    let marked = tx.execute(
        "UPDATE asset SET folded_into = ?2, updated_at = ?3 \
          WHERE id = ?1 \
            AND id <> ?2 \
            AND folded_into IS NULL \
            AND EXISTS (SELECT 1 FROM asset k \
                         WHERE k.id = ?2 \
                           AND k.folded_into IS NULL \
                           AND k.trashed_at IS NULL)",
        params![head, keep, now],
    )?;
    if marked == 0 {
        // Read inside the same transaction, so what it reports is the
        // state the statement above saw.
        let refusal = classify_fold_refusal(tx, head, keep)?;
        return Ok(FoldOutcome::Skipped(refusal));
    }

    let mut dropped: Vec<serde_json::Value> = Vec::new();

    // (2) The pair's own edges, in either direction, plus a self-loop on
    // the headstone if one ever got written.
    const PAIR_EDGES: &str = "(from_asset = ?1 AND to_asset IN (?1, ?2)) \
                           OR (from_asset = ?2 AND to_asset = ?1)";
    collect_dropped_edges(
        tx,
        PAIR_EDGES,
        params![head, keep],
        "self_loop",
        &mut dropped,
    )?;
    tx.execute(
        &format!("DELETE FROM edge WHERE {PAIR_EDGES}"),
        params![head, keep],
    )?;

    // (3) Re-point. Counted separately only because the two statements
    // are; an edge cannot be moved twice, since step 2 took every row
    // that named the headstone on both sides.
    let repointed = tx.execute(
        "UPDATE OR IGNORE edge SET from_asset = ?2 WHERE from_asset = ?1",
        params![head, keep],
    )? + tx.execute(
        "UPDATE OR IGNORE edge SET to_asset = ?2 WHERE to_asset = ?1",
        params![head, keep],
    )?;

    // (4) Whatever `OR IGNORE` refused.
    const LEFTOVER_EDGES: &str = "from_asset = ?1 OR to_asset = ?1";
    let noted_so_far = dropped.len();
    collect_dropped_edges(
        tx,
        LEFTOVER_EDGES,
        params![head],
        "duplicate_of_keeper",
        &mut dropped,
    )?;
    let deleted = tx.execute(
        &format!("DELETE FROM edge WHERE {LEFTOVER_EDGES}"),
        params![head],
    )?;
    // The `SELECT` above and this `DELETE` share their predicate through
    // one constant, so a row deleted without being read would mean the
    // two had drifted — and the note would be short by exactly the
    // claims nobody would then know were gone.
    debug_assert_eq!(
        deleted,
        dropped.len() - noted_so_far,
        "every dropped edge is recorded before it goes"
    );

    // The claims those rows carried, on the row they belonged to.
    // Written once, after both sweeps, so the note is the whole list
    // rather than the last one.
    if !dropped.is_empty() {
        let extra: Option<String> = tx.query_row(
            "SELECT extra FROM asset WHERE id = ?1",
            params![head],
            |row| row.get(0),
        )?;
        let note = serde_json::json!({
            "keeper": keep.to_string(),
            "at_ms": now,
            "dropped_edges": dropped,
        });
        match extra_with_fold_note(extra.as_deref(), note) {
            Some(merged) => {
                tx.execute(
                    "UPDATE asset SET extra = ?2 WHERE id = ?1",
                    params![head, merged],
                )?;
            }
            // The bag holds something a merge would destroy. The fold
            // still stands — refusing it over an unreadable column would
            // leave the duplicate unresolvable — but the claim is lost,
            // so it is said out loud rather than dropped in silence.
            None => tracing::warn!(
                event = "diag.fold.trace_note_skipped",
                asset_id = %head,
                keeper_id = %keep,
                dropped_edges = dropped.len(),
                "extra column could not carry the fold note"
            ),
        }
    }

    // (5) Filing. The keeper keeps its own position in a Group it was
    // already in, and its own tag rows: the `OR IGNORE` half of each
    // pair is what says so.
    //
    // Where it stands, the position the headstone had in that Group is
    // about to be deleted with the row that held it, so it is read
    // **before** the insert — after it, every Group looks shared and
    // there would be nothing left to tell apart. This falls under
    // the same rule as `container_id`: the keeper's arrangement wins,
    // and what it displaced is written down.
    let positions_not_taken: Vec<serde_json::Value> = {
        let mut stmt = tx.prepare(
            "SELECT h.bucket_id, h.position FROM asset_bucket h \
              WHERE h.asset_id = ?1 \
                AND EXISTS (SELECT 1 FROM asset_bucket k \
                             WHERE k.asset_id = ?2 \
                               AND k.bucket_id = h.bucket_id)",
        )?;
        let rows = stmt.query_map(params![head, keep], |row| {
            let bucket: Uuid = row.get(0)?;
            let position: i64 = row.get(1)?;
            Ok(serde_json::json!({
                "bucket": bucket.to_string(),
                "position": position,
            }))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let buckets = tx.execute(
        "INSERT OR IGNORE INTO asset_bucket (asset_id, bucket_id, added_at, position) \
         SELECT ?2, bucket_id, added_at, position \
           FROM asset_bucket WHERE asset_id = ?1",
        params![head, keep],
    )?;
    tx.execute(
        "DELETE FROM asset_bucket WHERE asset_id = ?1",
        params![head],
    )?;

    // `id <> ?2` keeps the keeper's own `container_id` untouched in the
    // one arrangement where it would otherwise move: a keeper filed
    // *inside* the row being folded. Re-pointing it would make the
    // keeper its own container; leaving it alone leaves a card filed in
    // a headstone, which the id-named read path still resolves
    // and which no column of the keeper had to change to keep true.
    let children = tx.execute(
        "UPDATE asset SET container_id = ?2, updated_at = ?3 \
          WHERE container_id = ?1 AND id <> ?2",
        params![head, keep, now],
    )?;

    let tags = tx.execute(
        "INSERT OR IGNORE INTO asset_tag (asset_id, tag_id) \
         SELECT ?2, tag_id FROM asset_tag WHERE asset_id = ?1",
        params![head, keep],
    )?;
    tx.execute("DELETE FROM asset_tag WHERE asset_id = ?1", params![head])?;

    // (6) The conversation. Comments keep their own `created_at`, so the
    // keeper's thread reads as one chronology with both sets interleaved
    // instead of two appended blocks. Nothing is de-duplicated: two
    // identical bodies are two things somebody said.
    let comments = tx.execute(
        "UPDATE asset_comment SET asset_id = ?2 WHERE asset_id = ?1",
        params![head, keep],
    )?;
    // `thread` has no foreign key and no uniqueness on its anchor, so
    // this is a plain re-point: a keeper that already had a thread of
    // its own ends up with both.
    let threads = tx.execute(
        "UPDATE thread SET anchor_id = ?2 \
          WHERE anchor_kind = 'card' AND anchor_id = ?1",
        params![head, keep],
    )?;

    // (7) The columns. Both rows are read **here**, inside the
    // transaction, and the write below names only the columns a rule
    // decided — a whole-row save built from an entity read earlier is
    // exactly the lost update this path refuses.
    let read_columns = merge_read_columns();
    let row_of = |id: Uuid| -> Result<Vec<Value>, rusqlite::Error> {
        tx.query_row(
            &format!("SELECT {read_columns} FROM asset WHERE id = ?1"),
            params![id],
            |row| {
                (0..row.as_ref().column_count())
                    .map(|i| row.get::<_, Value>(i))
                    .collect()
            },
        )
    };
    let merged = merge_keeper_columns(&row_of(keep)?, &row_of(head)?);
    if !merged.unreadable.is_empty() {
        // Said out loud rather than guessed at: the stored value is not
        // the shape the rule combines, so the keeper's own column stands
        // untouched and the fold still resolves the duplicate.
        tracing::warn!(
            event = "diag.fold.column_unreadable",
            asset_id = %head,
            keeper_id = %keep,
            columns = ?merged.unreadable,
            "a column could not be read as its merge rule needs it"
        );
    }

    let mut sets: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    for (column, value) in &merged.writes {
        values.push(value.clone());
        sets.push(format!("{column} = ?{}", values.len()));
    }
    let discarded_count = merged.discarded.len() + positions_not_taken.len();
    if discarded_count > 0 {
        let extra: Option<String> = tx.query_row(
            "SELECT extra FROM asset WHERE id = ?1",
            params![keep],
            |row| row.get(0),
        )?;
        let entry = serde_json::json!({
            "from": head.to_string(),
            "at_ms": now,
            "discarded": merged.discarded,
            "positions_not_taken": positions_not_taken,
        });
        match extra_with_absorbed_note(extra.as_deref(), entry) {
            Some(bag) => {
                values.push(Value::Text(bag));
                sets.push(format!("extra = ?{}", values.len()));
            }
            // Same judgement as the headstone's note above: an
            // unreadable bag does not get overwritten, and the loss is
            // stated rather than silent.
            None => tracing::warn!(
                event = "diag.fold.absorbed_note_skipped",
                asset_id = %head,
                keeper_id = %keep,
                discarded = discarded_count,
                "keeper's extra column could not carry the absorbed note"
            ),
        }
    }
    // Always stamped, even by a fold that combined nothing: the keeper
    // gained tags, Groups, comments and edges, and `updated_from_ms`
    // (V46) is how a differential sync hears about any of that.
    values.push(Value::Integer(now));
    sets.push(format!("updated_at = ?{}", values.len()));
    values.push(Value::Blob(keep.as_bytes().to_vec()));
    let target = values.len();
    tx.execute(
        &format!("UPDATE asset SET {} WHERE id = ?{target}", sets.join(", ")),
        rusqlite::params_from_iter(values),
    )?;

    Ok(FoldOutcome::Folded(FoldReport {
        edges_repointed: repointed as u64,
        edges_dropped: dropped.len() as u64,
        buckets_moved: buckets as u64,
        children_repointed: children as u64,
        tags_moved: tags as u64,
        comments_moved: comments as u64,
        threads_reanchored: threads as u64,
        columns_merged: merged.writes.len() as u64,
        values_discarded: discarded_count as u64,
    }))
}

/// How many `folded_into` links a lookup will walk before giving up.
///
/// Chains happen because a keeper may itself be folded later — nothing
/// rewrites the headstones that already point at it, so `A → B` plus
/// `B → C` leaves `A` two hops from the live row. A person resolving
/// duplicates one pair at a time builds those two or three deep; sixteen
/// is far past anything a hand-run merge produces and still bounded, so
/// a chain that got there is a defect rather than a long day's work.
///
/// The bound exists because the walk reads rows the database does not
/// constrain: `folded_into` carries no `REFERENCES asset(id)` (V51 says
/// why), so nothing but this counter stops a cycle written by a bug from
/// spinning the ingest path forever.
const FOLD_RESOLUTION_HOPS: usize = 16;

/// How many rows carrying one Source value a lookup will try before it
/// answers that nothing live holds the locator.
///
/// Several rows carrying one value is ordinary since V61 —
/// `OnDuplicate::Separate` asks for it by name — so a headstone whose
/// chain dead-ends is not the end of the question, only the end of that
/// candidate. Sixty-four is far past what a person filing the same path
/// repeatedly produces, and the bound is here for the same reason the
/// hop bound is: the loop reads rows nothing constrains, and a lookup on
/// the ingest path must not become unbounded work because a locator
/// collected a pathological number of rows.
const FOLD_CANDIDATE_SCAN: usize = 64;

/// Where a walk down `folded_into` ended, for
/// [`SourceLookupScope::Live`].
///
/// A reason and not an `Option<AssetRow>` because the caller does two
/// different things with the dead ends — it reports them, each in its
/// own words, and then tries the next row holding the locator — and
/// because a walk that answered `None` four ways would let the cycle
/// guard be deleted with every test still passing (the hop ceiling would
/// stop the same loop and produce the same `None`).
///
/// The walk judges; it does not report. Warning here would tie the
/// wording to the walk and fire it again for every candidate the caller
/// tries.
// `Resolved` is a whole row and the other five are a word, which clippy
// reads as a case for boxing. Not here: this value is produced once per
// candidate, immediately matched, and the row moved out of it — so the
// box would buy a smaller move on the dead-end paths at the price of a
// heap allocation on the answer path, which is the one that matters.
#[allow(clippy::large_enum_variant)]
enum FoldResolution {
    /// The live row the chain redirects to. The answer a `Live` lookup
    /// hands back in place of the headstone.
    Resolved(AssetRow),
    /// The chain ends in the trash. Ordinary, and the only silent one:
    /// `Live` already passes over a trashed row found directly, and a
    /// trashed row reached through a fold is the same record not being
    /// here.
    Trashed,
    /// A `folded_into` naming no row at all. Only a purge of a keeper
    /// can produce it — the FK that would have refused it is the one
    /// V51 deferred to a table rebuild.
    Dangling(Uuid),
    /// The chain came back to a row it had already passed. No verb can
    /// write one (`fold_one` refuses a keeper that is itself folded), so
    /// reaching one means something wrote the column behind the verb's
    /// back.
    Cycle(Uuid),
    /// The chain outran [`FOLD_RESOLUTION_HOPS`]. The verbs *can* build
    /// a chain this long one link at a time; nobody resolving duplicates
    /// by hand does, so a chain that got there wants looking at rather
    /// than following.
    TooLong,
    /// The chain points at a row in **another persona**. The fold verbs
    /// do not check the persona — `MergePlan::declare` weighs id sets
    /// and `fold_one` has no persona term — so a hand-run merge can
    /// write this, and following it would hand one library another
    /// library's row.
    OtherPersona(Uuid),
}

/// Written out rather than derived so that a failing assertion names the
/// resolved row by id: `AssetRow` is thirty columns wide and does not
/// implement `Debug`, and a test that printed all of it would bury the
/// one thing being asserted about.
impl std::fmt::Debug for FoldResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resolved(row) => write!(f, "Resolved({})", row.id),
            Self::Trashed => f.write_str("Trashed"),
            Self::Dangling(id) => write!(f, "Dangling({id})"),
            Self::Cycle(id) => write!(f, "Cycle({id})"),
            Self::TooLong => f.write_str("TooLong"),
            Self::OtherPersona(id) => write!(f, "OtherPersona({id})"),
        }
    }
}

impl FoldResolution {
    /// Says what went wrong, in the caller's voice, for the dead ends
    /// worth saying anything about.
    ///
    /// A warn and not an error: the caller's question stays answerable
    /// ("no live row of this persona holds this locator"), and failing
    /// the ingest of an unrelated file over a defect in somebody else's
    /// chain would turn a diagnosable state into a stuck import. What
    /// must not happen is a broken chain looking like an unregistered
    /// path with nothing said.
    ///
    /// [`Resolved`](Self::Resolved) and [`Trashed`](Self::Trashed) say
    /// nothing — the first is the ordinary answer, the second the
    /// ordinary absence.
    fn report(&self, from: &AssetRow) {
        match self {
            Self::Resolved(_) | Self::Trashed => {}
            Self::Dangling(missing) => tracing::warn!(
                event = "diag.asset.fold_chain_dangling",
                from = %from.id,
                missing = %missing,
                locator = %from.source_locator,
                "a fold points at a row that is not there (a purged keeper); \
                 this candidate holds no live locator"
            ),
            Self::Cycle(at) => tracing::warn!(
                event = "diag.asset.fold_chain_cycle",
                from = %from.id,
                at = %at,
                locator = %from.source_locator,
                "a fold chain points back at a row it already passed; \
                 this candidate holds no live locator"
            ),
            Self::TooLong => tracing::warn!(
                event = "diag.asset.fold_chain_too_long",
                from = %from.id,
                hops = FOLD_RESOLUTION_HOPS,
                locator = %from.source_locator,
                "a fold chain is longer than the walk will follow; \
                 this candidate holds no live locator"
            ),
            Self::OtherPersona(keeper) => tracing::warn!(
                event = "diag.asset.fold_chain_other_persona",
                from = %from.id,
                persona_id = %from.persona_id,
                keeper = %keeper,
                locator = %from.source_locator,
                "a fold points out of the persona that holds the locator; \
                 the walk stops rather than answer one library with another's row"
            ),
        }
    }
}

/// Follows `folded_into` from a headstone towards the live row the fold
/// redirected it to, for [`SourceLookupScope::Live`].
///
/// A fold leaves the locator on the headstone rather than copying it to
/// the keeper, so a re-import of that path lands on a row that is
/// in no listing. Handing it back is how a caller ends up holding an id
/// nothing can show it. The ruling said those two rows are one thing;
/// after it, the path names the keeper, and this is where that is said.
///
/// The walk stays inside `headstone.persona_id`, and it is the **query**
/// that keeps it there rather than a check on the row afterwards: a
/// cross-persona fold is writable by hand, and reading the row first
/// would mean one library's lookup had another library's locator, title
/// and labels in hand — the leak this is meant to prevent — before
/// deciding not to use them. The miss is then disambiguated with an
/// existence probe that selects no columns, which is what separates
/// [`OtherPersona`](FoldResolution::OtherPersona) from
/// [`Dangling`](FoldResolution::Dangling) without loading anything.
///
/// Each hop reads the whole row rather than probing `folded_into` first
/// and fetching once at the end: the common chain is one link, where
/// that would be two statements instead of one. The cost lands only on
/// the headstone path either way — a live row never gets here.
fn resolve_fold_chain(
    conn: &rusqlite::Connection,
    headstone: &AssetRow,
    first: Uuid,
) -> Result<FoldResolution, rusqlite::Error> {
    use rusqlite::OptionalExtension;

    // A `Vec` and not a `HashSet`: the walk is capped at sixteen, where
    // the linear scan is the cheaper of the two and needs no allocation
    // strategy of its own.
    let mut seen: Vec<Uuid> = vec![headstone.id];
    let mut next = first;
    for _ in 0..FOLD_RESOLUTION_HOPS {
        if seen.contains(&next) {
            return Ok(FoldResolution::Cycle(next));
        }
        seen.push(next);
        let row = conn
            .query_row(
                &format!(
                    "SELECT {} FROM asset WHERE id = ?1 AND persona_id = ?2",
                    AssetRow::COLUMNS
                ),
                params![next, headstone.persona_id],
                AssetRow::from_row,
            )
            .optional()?;
        let Some(row) = row else {
            // Two different states share that miss, and they read very
            // differently in a log. `SELECT 1` tells them apart while
            // still bringing back no column of a row this persona is
            // not entitled to see.
            let stands_elsewhere = conn
                .query_row("SELECT 1 FROM asset WHERE id = ?1", params![next], |_| {
                    Ok(())
                })
                .optional()?
                .is_some();
            return Ok(if stands_elsewhere {
                FoldResolution::OtherPersona(next)
            } else {
                FoldResolution::Dangling(next)
            });
        };
        if row.trashed_at.is_some() {
            return Ok(FoldResolution::Trashed);
        }
        match row.folded_into {
            None => return Ok(FoldResolution::Resolved(row)),
            Some(further) => next = further,
        }
    }
    Ok(FoldResolution::TooLong)
}

/// The keeper a row was folded into, or `None` when it is not a
/// headstone. Read inside the caller's transaction, for the reason
/// [`classify_fold_refusal`] states.
///
/// [`FoldRefusal::AlreadyFolded`] does not say *into whom*, and
/// `merge_into` needs that: a row already folded into the keeper the
/// plan names is the plan already being true, while one folded
/// somewhere else is a different ruling by somebody else.
fn folded_into_of(
    tx: &rusqlite::Transaction<'_>,
    id: Uuid,
) -> Result<Option<Uuid>, rusqlite::Error> {
    use rusqlite::OptionalExtension;

    Ok(tx
        .query_row(
            "SELECT folded_into FROM asset WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<Uuid>>(0),
        )
        .optional()?
        .flatten())
}

/// Adds one fold's counts into the running total a merge reports.
///
/// Every field is a row count, so the sum is the same kind of number the
/// parts are — with one thing to know, which the port doc says out loud:
/// `columns_merged` and `values_discarded` count **writes**, not
/// distinct columns, so three rows that each contributed to `labels`
/// add three.
fn accumulate(total: &mut FoldReport, one: &FoldReport) {
    total.edges_repointed += one.edges_repointed;
    total.edges_dropped += one.edges_dropped;
    total.buckets_moved += one.buckets_moved;
    total.children_repointed += one.children_repointed;
    total.tags_moved += one.tags_moved;
    total.comments_moved += one.comments_moved;
    total.threads_reanchored += one.threads_reanchored;
    total.columns_merged += one.columns_merged;
    total.values_discarded += one.values_discarded;
}

/// Primitive row built inside the isle closure. Used for detail
/// reads and writes; holds every column.
struct AssetRow {
    id: Uuid,
    persona_id: Uuid,
    source_kind: String,
    source_locator: String,
    file_size_bytes: Option<i64>,
    platform: Option<String>,
    modality: Option<String>,
    labels: String,
    occurred_at: i64,
    bundle_id: Option<String>,
    cover: Option<String>,
    keywords: String,
    register_note: Option<String>,
    vis_restricted: bool,
    vis_sharing: String,
    duration_ms: Option<i64>,
    rating: Option<i64>,
    palette: Option<String>,
    extra: Option<String>,
    created_at: i64,
    updated_at: i64,
    // Composition membership (BLOB self-reference) + composite title.
    // Appended after `updated_at` so the positional `row.get` indices of
    // the pre-v2 columns stay put.
    container_id: Option<Uuid>,
    title: Option<String>,
    trashed_at: Option<i64>,
    // Structural role (asset-model v4). Appended last for the same
    // positional-index reason as the composition columns above.
    role: String,
    // Attribution (V47): who the row is by, and which agent operated on
    // their behalf. Appended last, again so the positional `row.get`
    // indices above stay put. All NULL until the write paths are wired.
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
    // Channel the pair above arrived through (V50). Appended last for
    // the same positional reason. NULL on an unrecorded row, and on the
    // V47-era rows that record an author but no channel.
    attributed_via: Option<String>,
    // Fold axis (V51): the headstone's keeper, and whether this row may
    // be folded at all. Appended last for the same positional-index
    // reason as everything above it.
    folded_into: Option<Uuid>,
    fold_policy: String,
    // The strategy declared at registration (V52). Nullable — NULL is
    // "nobody declared", not `'ask'`. Appended last, same reason again.
    on_duplicate: Option<String>,
    // What the source calls this row (V30's column, V62's plain index).
    // Read back so the entity states what is stored rather than a
    // standing `None`; it was invisible to every reader while a UNIQUE
    // was the only thing that looked at it. Appended last, same
    // positional-index reason as everything above.
    external_key: Option<String>,
    // Coded pixel dimensions (V69). Appended last, same positional-index
    // reason as everything above. `i64` because that is what the column
    // holds; the narrowing to `u32` happens in `into_domain`, where an
    // out-of-range row can be reported instead of cast.
    width_px: Option<i64>,
    height_px: Option<i64>,
}

impl AssetRow {
    const COLUMNS: &'static str = "id, persona_id, source_kind, source_locator, \
         file_size_bytes, platform, modality, labels, occurred_at, bundle_id, cover, \
         keywords, register_note, vis_restricted, vis_sharing, duration_ms, rating, \
         palette, extra, created_at, updated_at, container_id, title, trashed_at, role, \
         author_kind, author_subject, operator_ai, attributed_via, folded_into, \
         fold_policy, on_duplicate, external_key, width_px, height_px";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            source_kind: row.get(2)?,
            source_locator: row.get(3)?,
            file_size_bytes: row.get(4)?,
            platform: row.get(5)?,
            modality: row.get(6)?,
            labels: row.get(7)?,
            occurred_at: row.get(8)?,
            bundle_id: row.get(9)?,
            cover: row.get(10)?,
            keywords: row.get(11)?,
            register_note: row.get(12)?,
            vis_restricted: row.get::<_, i64>(13)? != 0,
            vis_sharing: row.get(14)?,
            duration_ms: row.get(15)?,
            rating: row.get(16)?,
            palette: row.get(17)?,
            extra: row.get(18)?,
            created_at: row.get(19)?,
            updated_at: row.get(20)?,
            container_id: row.get(21)?,
            title: row.get(22)?,
            trashed_at: row.get(23)?,
            role: row.get(24)?,
            author_kind: row.get(25)?,
            author_subject: row.get(26)?,
            operator_ai: row.get(27)?,
            attributed_via: row.get(28)?,
            folded_into: row.get(29)?,
            fold_policy: row.get(30)?,
            on_duplicate: row.get(31)?,
            external_key: row.get(32)?,
            width_px: row.get(33)?,
            height_px: row.get(34)?,
        })
    }

    fn into_domain(self) -> Result<Asset, DomainError> {
        // Read through the same guard the write paths use. The column
        // is a JSON array with no uniqueness of its own, so a row
        // written before the write-side dedup existed — or by hand SQL
        // / bulk import — can still hold a repeat. Dropping it on the
        // way out keeps the shape the reader expects without touching
        // the stored row.
        let labels = dedup_labels(
            json_to_strings(&self.labels)?
                .into_iter()
                .map(Label::new)
                .collect::<Result<Vec<_>, _>>()?,
        );
        let keywords = json_to_strings(&self.keywords)?
            .into_iter()
            .map(Keyword::new)
            .collect::<Result<Vec<_>, _>>()?;
        let visibility = if self.vis_restricted {
            Visibility::Restricted {
                sharing: json_to_strings(&self.vis_sharing)?,
            }
        } else {
            Visibility::Open
        };
        let extra = match self.extra {
            Some(json) => serde_json::from_str(&json)
                .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt extra column: {e}")))?,
            None => serde_json::Value::Null,
        };
        // The attribution triple is read back through the one type that
        // can hold an arbitrary channel, so a corrupt pair or an unknown
        // slug surfaces here instead of degrading into a
        // plausible-looking answer — a `(kind, subject)` combination the
        // domain cannot produce would otherwise round down to "the
        // owner". This is why V47 / V50 need no CHECK constraint
        // (`ALTER TABLE ADD COLUMN` cannot carry one anyway).
        let attribution = PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        // Seeded through the hydration constructor rather than a struct
        // literal: the attribution fields are private, so the only way
        // in is the one that also states *where the value came from*.
        // Everything else the row carries is assigned below.
        let mut asset = Asset::from_persisted(
            AssetId::from_uuid(self.id),
            PersonaId::from_uuid(self.persona_id),
            SourceRef {
                kind: SourceKind::new(self.source_kind)?,
                // B1: the read boundary for `asset.source_locator`. The
                // column's spelling stops here — everything above this
                // line holds the value.
                locator: SourceLocator::try_from(self.source_locator.as_str())?,
                file_size_bytes: opt_u64(self.file_size_bytes, "file_size_bytes")?,
                platform: self.platform,
            },
            self.modality.map(Modality::new).transpose()?,
            ms_to_datetime(self.occurred_at)?,
            ms_to_datetime(self.created_at)?,
            ms_to_datetime(self.updated_at)?,
            attribution,
        );
        asset.labels = labels;
        asset.bundle_id = self.bundle_id.map(BundleId::new).transpose()?;
        asset.container_id = self.container_id.map(AssetId::from_uuid);
        asset.title = self.title;
        asset.cover = self.cover.map(CoverText::new).transpose()?;
        asset.keywords = keywords;
        asset.register_note = self.register_note.map(RegisterNote::new).transpose()?;
        asset.visibility = visibility;
        asset.duration_ms = opt_u64(self.duration_ms, "duration_ms")?;
        // Narrowed with both ends checked, unlike `rating` on the next
        // line: a rating is bounded by a domain rule every writer holds
        // to, so a cast there cannot invent a value the column did not
        // hold. A pixel dimension is bounded only by the column, so
        // `4294967296` would cast to `0` — a stated measurement, sorting
        // ahead of every real one — and `-1` to `4294967295`. Both are
        // refused instead (`opt_u32`).
        //
        // Read half-by-half without comparing the two: the pair rule is a
        // write-side assertion, and a row some other writer left with one
        // number must stay readable rather than becoming an unopenable
        // asset.
        asset.width_px = opt_u32(self.width_px, "width_px")?;
        asset.height_px = opt_u32(self.height_px, "height_px")?;
        asset.rating = self.rating.map(|r| r as u8);
        asset.palette = self
            .palette
            .as_deref()
            .map(serde_json::from_str::<Vec<String>>)
            .transpose()
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt palette JSON: {e}")))?;
        asset.extra = extra;
        asset.trashed_at = self.trashed_at.map(ms_to_datetime).transpose()?;
        asset.role = AssetRole::parse(&self.role)?;
        asset.folded_into = self.folded_into.map(AssetId::from_uuid);
        // Rejected rather than degraded to `Auto`, like `role`: a slug
        // outside the closed set can only come from a hand-edited row,
        // and silently reading it as "unruled" would un-do somebody's
        // `keep`.
        asset.fold_policy = FoldPolicy::parse(&self.fold_policy)?;
        // Absence survives as absence: `None` is "nobody declared", and
        // mapping it to `Ask` here would invent the request the column
        // exists to keep distinguishable. A present slug outside the
        // closed set is refused rather than degraded, like `role` and
        // `fold_policy` above.
        asset.on_duplicate = self
            .on_duplicate
            .as_deref()
            .map(OnDuplicate::parse)
            .transpose()?;
        // Read back verbatim and parsed against nothing: it is what a
        // source called this row, and the library does not get to have
        // an opinion about a name it did not choose.
        asset.external_key = self.external_key;
        // `materials` stays empty here — hydrated separately by the read
        // paths that need a truthful entity (`find` / `find_by_source`);
        // see `MaterialRow`.
        Ok(asset)
    }
}

/// Row of the `material` table — the physical-original layer
/// (asset-model v4). Loaded alongside the single-entity read paths and
/// attached to [`Asset::materials`].
struct MaterialRow {
    ord: i64,
    locator: String,
    file_size_bytes: Option<i64>,
    mime: Option<String>,
    content_hash: Option<String>,
    created_at: i64,
    updated_at: i64,
    content_region_hash: Option<String>,
    meta_hash: Option<String>,
    meta_kv: Option<String>,
    meta_text: Option<String>,
}

impl MaterialRow {
    /// The column order is the positional contract [`Self::from_row`]
    /// reads by, so a new column goes on the **end** — inserting one in
    /// the middle would not fail to compile, it would silently return
    /// each following column under the previous one's name.
    const COLUMNS: &'static str = "ord, locator, file_size_bytes, mime, content_hash, \
                                   created_at, updated_at, content_region_hash, \
                                   meta_hash, meta_kv, meta_text";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            ord: row.get(0)?,
            locator: row.get(1)?,
            file_size_bytes: row.get(2)?,
            mime: row.get(3)?,
            content_hash: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            content_region_hash: row.get(7)?,
            meta_hash: row.get(8)?,
            meta_kv: row.get(9)?,
            meta_text: row.get(10)?,
        })
    }

    fn load_for(conn: &rusqlite::Connection, asset: Uuid) -> Result<Vec<Self>, rusqlite::Error> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM material WHERE asset_id = ?1 ORDER BY ord",
            Self::COLUMNS
        ))?;
        let rows = stmt
            .query_map(params![asset], Self::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn into_domain(self) -> Result<Material, DomainError> {
        Ok(Material {
            ord: self.ord as u32,
            // B2: the read boundary for `material.locator`, the second
            // column with this encoding. Parsed here rather than left as
            // text so the backfill walk and the per-asset pass hand
            // `hash_material` the same reading of one artefact.
            locator: SourceLocator::try_from(self.locator.as_str())?,
            file_size_bytes: opt_u64(self.file_size_bytes, "material.file_size_bytes")?,
            // The parse boundary for the format axis, next to the one
            // `AssetRole` already crosses below. Infallible on purpose:
            // the column has no CHECK and V37 backfilled it from
            // extensions in SQL, so a value this codebase does not name
            // is a row to carry, not a mapping to fail.
            mime: self.mime.as_deref().map(MimeType::parse),
            content_hash: self.content_hash,
            content_region_hash: self.content_region_hash,
            meta_hash: self.meta_hash,
            meta_kv: self.meta_kv,
            meta_text: self.meta_text,
            created_at: ms_to_datetime(self.created_at)?,
            updated_at: ms_to_datetime(self.updated_at)?,
        })
    }
}

/// One asset the body-cache backfill is considering.
///
/// Carries the format alongside the locator because the decision the
/// caller makes with it is "should these bytes be read as text", and a
/// scan that returned only a locator left that question unanswerable —
/// which is how the backfill came to read pictures. A struct rather
/// than a fourth tuple element so the field that answers it has a name.
#[derive(Debug, Clone)]
pub struct BodyCandidate {
    /// The asset whose body is missing from the cache.
    pub asset_id: AssetId,
    /// Its owner — the search index is partitioned by persona.
    pub persona_id: PersonaId,
    /// Where the source text would be read from. Typed, because it feeds
    /// `TextLocator` and from there the reader that has to know whether
    /// it is opening a file or finding a record inside a container.
    pub locator: SourceLocator,
    /// The primary material's format, or `None` when the row has
    /// none. Decides whether the locator is read at all.
    pub mime: Option<MimeType>,
}

/// Row of the `duplicate_conflict` table — one raised "are these two
/// the same thing?" question.
///
/// The sorted pair (`pair_lo` / `pair_hi`) is not read back: it is the
/// storage form of the key, derived from the two ids the entity already
/// carries. Reading it would give a second, parallel copy of the pair
/// that could disagree with the direction beside it.
struct ConflictRow {
    id: Uuid,
    persona_id: Uuid,
    newcomer_id: Uuid,
    incumbent_id: Uuid,
    axis: String,
    content_hash: String,
    fold_exclusion: Option<String>,
    detected_at: i64,
    resolved_at: Option<i64>,
    resolution: Option<String>,
}

impl ConflictRow {
    const COLUMNS: &'static str = "id, persona_id, newcomer_id, incumbent_id, axis, \
                                   content_hash, fold_exclusion, detected_at, resolved_at, \
                                   resolution";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            newcomer_id: row.get(2)?,
            incumbent_id: row.get(3)?,
            axis: row.get(4)?,
            content_hash: row.get(5)?,
            fold_exclusion: row.get(6)?,
            detected_at: row.get(7)?,
            resolved_at: row.get(8)?,
            resolution: row.get(9)?,
        })
    }

    fn into_domain(self) -> Result<DuplicateConflict, DomainError> {
        Ok(DuplicateConflict {
            id: DuplicateConflictId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            newcomer: AssetId::from_uuid(self.newcomer_id),
            incumbent: AssetId::from_uuid(self.incumbent_id),
            axis: DuplicateAxis::parse(&self.axis)?,
            content_hash: self.content_hash,
            fold_exclusion: self
                .fold_exclusion
                .as_deref()
                .map(FoldExclusion::parse)
                .transpose()?,
            detected_at: ms_to_datetime(self.detected_at)?,
            resolved_at: self.resolved_at.map(ms_to_datetime).transpose()?,
            resolution: self
                .resolution
                .as_deref()
                .map(ConflictResolution::parse)
                .transpose()?,
        })
    }
}

/// Lightweight row for the grid — built inside the isle closure and
/// converted straight into [`AssetCard`].
struct CardRow {
    id: Uuid,
    persona_id: Uuid,
    modality: Option<String>,
    occurred_at: i64,
    cover: Option<String>,
    labels: String,
    file_size_bytes: Option<i64>,
    duration_ms: Option<i64>,
    // The `Pixels` axis's key. Appended to the end of the select list
    // rather than placed beside the two metric columns it belongs with,
    // because the reads below are index-based: appending leaves every
    // existing index alone.
    pixel_count: Option<i64>,
    source_locator: String,
    created_at: i64,
    updated_at: i64,
    rating: Option<i64>,
    palette: Option<String>,
    has_note: i64,
    has_thread: i64,
    mime: Option<String>,
    role: String,
    title: Option<String>,
    member_count: i64,
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
}

impl CardRow {
    /// `has_note` derives from `register_note IS NOT NULL` (cheap;
    /// same row). `has_thread` uses an EXISTS subquery over
    /// `asset_comment` — the `idx_asset_comment_asset(asset_id, ...)`
    /// index makes it a single covering-index seek per row. `mime`
    /// (asset-model v4 format fact) is a correlated point lookup on the
    /// material `(asset_id, ord)` WITHOUT ROWID PK — same cost class as
    /// the `has_thread` seek.
    ///
    /// A function rather than a `const` because of the one column that
    /// is not a column: `member_count` splices [`MEMBER_POPULATION`],
    /// and Rust has no way to concatenate a `const &str` into another
    /// one. Spelling the predicate out here instead would put the
    /// member rule in two places, which is the arrangement that let
    /// this card claim "5 items" over a container holding four.
    fn columns() -> String {
        format!(
            "id, persona_id, modality, occurred_at, cover, labels, file_size_bytes, \
             duration_ms, source_locator, created_at, updated_at, rating, palette, \
             (register_note IS NOT NULL) AS has_note, \
             EXISTS(SELECT 1 FROM asset_comment WHERE asset_id = asset.id) AS has_thread, \
             (SELECT mime FROM material \
               WHERE material.asset_id = asset.id AND material.ord = 0) AS mime, \
             role, title, \
             (SELECT COUNT(*) FROM asset m \
               WHERE m.container_id = asset.id AND {MEMBER_POPULATION}) AS member_count, \
             author_kind, author_subject, operator_ai, \
             (width_px * height_px) AS pixel_count"
        )
    }

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            modality: row.get(2)?,
            occurred_at: row.get(3)?,
            cover: row.get(4)?,
            labels: row.get(5)?,
            file_size_bytes: row.get(6)?,
            duration_ms: row.get(7)?,
            source_locator: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            rating: row.get(11)?,
            palette: row.get(12)?,
            has_note: row.get(13)?,
            has_thread: row.get(14)?,
            mime: row.get(15)?,
            role: row.get(16)?,
            title: row.get(17)?,
            member_count: row.get(18)?,
            author_kind: row.get(19)?,
            author_subject: row.get(20)?,
            operator_ai: row.get(21)?,
            pixel_count: row.get(22)?,
        })
    }

    fn into_card(self) -> Result<AssetCard, DomainError> {
        Ok(AssetCard {
            id: AssetId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            modality: self.modality.map(Modality::new).transpose()?,
            occurred_at: ms_to_datetime(self.occurred_at)?,
            cover: self.cover.map(CoverText::new).transpose()?,
            // Same read-side guard as `AssetRow::into_domain`, on the
            // projection the grid actually renders its chips from.
            labels: dedup_labels(
                json_to_strings(&self.labels)?
                    .into_iter()
                    .map(Label::new)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            file_size_bytes: opt_u64(self.file_size_bytes, "file_size_bytes")?,
            duration_ms: opt_u64(self.duration_ms, "duration_ms")?,
            // See `IndexRow::into_index` for why the guard is worth more
            // on a product than on a stored column.
            pixel_count: opt_u64(self.pixel_count, "pixel_count")?,
            mime: self.mime,
            // Same read boundary as the entity path above, on the cheap
            // projection: what the card carries is the value, and the
            // mapper renders the display form for the wire.
            source_locator: SourceLocator::try_from(self.source_locator.as_str())?,
            // Enriched by the caller (see `page()` / `list_by_ids`) via a
            // single bulk `SELECT` over `asset_bucket` so the m:n join
            // cost is O(1) round-trips regardless of page size. The
            // primary-group slot comes from the same pass.
            group_ids: Vec::new(),
            primary_group_position: None,
            created_at: ms_to_datetime(self.created_at)?,
            updated_at: ms_to_datetime(self.updated_at)?,
            rating: self.rating.map(|r| r as u8),
            palette: self
                .palette
                .as_deref()
                .map(serde_json::from_str::<Vec<String>>)
                .transpose()
                .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt palette JSON: {e}")))?,
            has_note: self.has_note != 0,
            has_thread: self.has_thread != 0,
            role: AssetRole::parse(&self.role)?,
            title: self.title,
            member_count: self.member_count.max(0) as u64,
            // Same reader the entity path uses (`AssetRow::into_asset`),
            // so a corrupt pair fails identically on both projections
            // instead of silently becoming the owner on the cheaper one.
            author: Author::from_columns(
                self.author_kind.as_deref(),
                self.author_subject.as_deref(),
            )?,
            operator_ai: self.operator_ai.map(OperatorRef::new).transpose()?,
        })
    }
}

/// Bulk-fetches the `Group` ids each asset is filed into.
///
/// SQLite caps a statement at `SQLITE_MAX_VARIABLE_NUMBER` bound
/// parameters — 999 on stock builds, 32_766 on newer ones. A single
/// `IN (?, ?, …)` therefore blows up around a thousand assets, which
/// hits us on the 89k / 200k grid pages. Chunk the input list, run
/// one prepared statement per batch, and merge the rows into a
/// single map. Each query still hits `idx_asset_bucket_asset`, so
/// the cost stays proportional to the number of matching m:n rows.
/// Returns an empty map for an empty input (no query issued).
///
/// Each entry is `(bucket_id, position)`. The position rides along
/// because the card projection needs the asset's slot inside its primary
/// group and this join is already reading the row that holds it — a
/// second pass would re-scan `asset_bucket` for a column the first pass
/// had in hand.
fn fetch_group_ids_map(
    conn: &rusqlite::Connection,
    asset_uuids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<(Uuid, i64)>>, rusqlite::Error> {
    // Stay well under the 999-var floor so the same statement is safe
    // even if the connection is running inside a larger transaction.
    const CHUNK: usize = 500;
    let mut map: HashMap<Uuid, Vec<(Uuid, i64)>> = HashMap::with_capacity(asset_uuids.len());
    for batch in asset_uuids.chunks(CHUNK) {
        let placeholders = vec!["?"; batch.len()].join(", ");
        // The join onto `bucket` is not decoration: `asset_bucket` rows
        // survive a Group being trashed (that is what makes restore
        // free), so reading the link table alone would hand the grid
        // group ids that `GroupRepository::list` no longer returns. The
        // group-axis sort resolves ids against that listing, so a
        // trashed id lands the card in the "unfiled" bucket even though
        // it is still filed in a live Group.
        // Ordered by `bucket_id` so "primary group" (`group_ids[0]`) is a
        // stable answer. Without it the row order is whatever the scan
        // produces, and a card filed in two Groups could drift between
        // buckets across reloads — taking its `primary_group_position`,
        // and therefore its place in the manual order, with it.
        let sql = format!(
            "SELECT asset_bucket.asset_id, asset_bucket.bucket_id, asset_bucket.position \
             FROM asset_bucket \
             JOIN bucket ON bucket.id = asset_bucket.bucket_id \
             WHERE asset_bucket.asset_id IN ({placeholders}) \
               AND bucket.trashed_at IS NULL \
             ORDER BY asset_bucket.asset_id, asset_bucket.bucket_id"
        );
        let params: Vec<Value> = batch
            .iter()
            .map(|u| Value::Blob(u.as_bytes().to_vec()))
            .collect();
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
        while let Some(row) = rows.next()? {
            let asset_id: Uuid = row.get(0)?;
            let bucket_id: Uuid = row.get(1)?;
            let position: i64 = row.get(2)?;
            map.entry(asset_id).or_default().push((bucket_id, position));
        }
    }
    Ok(map)
}

/// Sibling of [`CardRow`] for the index-only path — cover text and
/// source locator are deliberately not selected so the SQL scan and
/// the wire payload stay small enough for 6-figure result sets. Every
/// field on this row is required by some client-side sort / filter
/// axis; the removed columns are hydrated later by
/// [`SqliteAssetRepository::cards_by_ids`].
///
/// That second sentence is the membership rule, and it is why
/// `file_size_bytes` moved from the dropped list to the selected one
/// (with `duration_ms` alongside it) when the metric axes landed: both
/// are sort keys the client evaluates over these rows, so dropping them
/// did not make the row cheap, it made two axes unofferable. They are
/// `INTEGER` columns on `asset` — the same per-row cost as the
/// timestamps — not the unbounded text the other two are.
struct IndexRow {
    id: Uuid,
    persona_id: Uuid,
    modality: Option<String>,
    occurred_at: i64,
    labels: String,
    created_at: i64,
    updated_at: i64,
    duration_ms: Option<i64>,
    file_size_bytes: Option<i64>,
    // The `Pixels` axis's key, multiplied out by SQLite. NULL when either
    // side is unmeasured, which is both of them or neither.
    pixel_count: Option<i64>,
    role: String,
}

impl IndexRow {
    const COLUMNS: &'static str = "id, persona_id, modality, occurred_at, labels, created_at, \
         updated_at, duration_ms, file_size_bytes, \
         (width_px * height_px) AS pixel_count, role";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            modality: row.get(2)?,
            occurred_at: row.get(3)?,
            labels: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            duration_ms: row.get(7)?,
            file_size_bytes: row.get(8)?,
            pixel_count: row.get(9)?,
            role: row.get(10)?,
        })
    }

    fn into_index(self) -> Result<asterism_core::domain::asset::AssetIndex, DomainError> {
        Ok(asterism_core::domain::asset::AssetIndex {
            id: AssetId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            modality: self.modality.map(Modality::new).transpose()?,
            occurred_at: ms_to_datetime(self.occurred_at)?,
            // Same read-side guard as the two projections above.
            labels: dedup_labels(
                json_to_strings(&self.labels)?
                    .into_iter()
                    .map(Label::new)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            // Same enrichment pattern as `CardRow::into_card` — the
            // caller runs a single bulk join through
            // `fetch_group_ids_map` and back-fills.
            group_ids: Vec::new(),
            primary_group_position: None,
            created_at: ms_to_datetime(self.created_at)?,
            updated_at: ms_to_datetime(self.updated_at)?,
            // Read through the same guard the card path uses, so a
            // negative stored value is refused identically on both
            // projections instead of wrapping into a huge length on the
            // cheaper one.
            duration_ms: opt_u64(self.duration_ms, "duration_ms")?,
            file_size_bytes: opt_u64(self.file_size_bytes, "file_size_bytes")?,
            // Through the same guard, which is doing slightly more work
            // here than beside it: the operands are non-negative by their
            // own column guard (`opt_u32`), so a negative product means
            // the multiplication left the integer range rather than that
            // a bad value was stored. Refusing it keeps a wrapped count
            // from being presented as a real one.
            pixel_count: opt_u64(self.pixel_count, "pixel_count")?,
            role: AssetRole::parse(&self.role)?,
        })
    }
}

/// The card's group entries with the *filtered* group first.
///
/// `group_ids[0]` is the primary group: the name the group axis buckets
/// under and the owner of the `position` that axis arranges on. Ordering
/// by `bucket_id` alone makes that answer stable but blind — a card filed
/// in two Groups reports whichever id sorts lower, so browsing one Group
/// could label the band with the *other* Group's name and arrange the
/// page by the *other* Group's slots, silently contradicting the page
/// order `page` / `page_index` already selected (`asset_bucket.position`
/// of the filtered bucket). When the filter names exactly one Group,
/// that Group is the one the user is looking at, so it takes the primary
/// slot. Multi-Group and unfiltered reads keep the `bucket_id` answer:
/// per-bucket position has no meaning across a union.
pub(crate) fn primary_group_first(
    entries: &[(Uuid, i64)],
    sole_group: Option<Uuid>,
) -> Vec<(Uuid, i64)> {
    let Some(sole) = sole_group else {
        return entries.to_vec();
    };
    match entries.iter().position(|(gid, _)| *gid == sole) {
        None | Some(0) => entries.to_vec(),
        Some(i) => {
            let mut out = Vec::with_capacity(entries.len());
            out.push(entries[i]);
            out.extend(
                entries
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, e)| *e),
            );
            out
        }
    }
}

/// Index-side sibling of [`attach_group_ids`] — same bulk join, but
/// mutates [`AssetIndex`] entries instead of `AssetCard`.
fn attach_group_ids_index(
    items: &mut [asterism_core::domain::asset::AssetIndex],
    map: &HashMap<Uuid, Vec<(Uuid, i64)>>,
    sole_group: Option<Uuid>,
) {
    for item in items.iter_mut() {
        if let Some(entries) = map.get(item.id.as_uuid()) {
            let entries = primary_group_first(entries, sole_group);
            item.group_ids = entries
                .iter()
                .map(|(gid, _)| GroupId::from_uuid(*gid))
                .collect();
            item.primary_group_position = entries.first().map(|(_, pos)| *pos);
        }
    }
}

/// Attaches `group_ids` and `primary_group_position` to every card using
/// a group-map from [`fetch_group_ids_map`]. Cards with no matching row
/// keep the default empty vector and a `None` position. `sole_group` is
/// the filter's single Group when it names one — see
/// [`primary_group_first`].
fn attach_group_ids(
    cards: &mut [AssetCard],
    map: &HashMap<Uuid, Vec<(Uuid, i64)>>,
    sole_group: Option<Uuid>,
) {
    for card in cards.iter_mut() {
        if let Some(entries) = map.get(card.id.as_uuid()) {
            let entries = primary_group_first(entries, sole_group);
            card.group_ids = entries
                .iter()
                .map(|(gid, _)| GroupId::from_uuid(*gid))
                .collect();
            // `group_ids[0]` is the primary group, so its slot is the one
            // the group axis orders on.
            card.primary_group_position = entries.first().map(|(_, pos)| *pos);
        }
    }
}

/// The population every sidebar facet counts over: every Asset that
/// still exists in its own right.
///
/// One Asset is one Card, so the only exclusion is the one row that is
/// no longer an asset at all: a headstone (`folded_into IS NOT NULL`)
/// is a redirect left behind by a fold, and its content is counted
/// under the keeper. The constant survives as the single place that
/// statement is written down — the facets used to disagree with each
/// other *and* with the grid because each carried its own idea of what
/// counted (282 / 237 / 264 against 268 actual rows on the dogfood
/// profile), and a shared definition is what stopped that. Every facet
/// therefore acquires the fold rule by construction, the same way they
/// acquire everything else here.
///
/// It also used to exclude members (`container_id IS NULL`), back when
/// the grid hid them behind a `top_level` flag. Deciding visibility in
/// the query is what made a row exist in the data and nowhere in the
/// UI; if a future Card is not 1:1 with an Asset, that belongs in
/// whatever maps Assets to Cards, not in a filter every caller has to
/// remember to set. A headstone is the opposite case and belongs here:
/// it is not a row the UI declines to draw, it is a row that stopped
/// being a distinct thing.
///
/// Qualified (`asset.`) because this is spliced into statements that
/// have a second table in scope (`counts_by_format` joins `material`,
/// `counts_by_color` joins `asset_color`) — same rule as
/// [`QueryParts::build`].
const GRID_POPULATION: &str = "asset.folded_into IS NULL";

/// The population a container's own aggregates count over: the rows
/// that are still members of it.
///
/// Two axes, because a row stops being a member for two different
/// reasons and neither of them clears `container_id`. Trashing keeps
/// the filing on purpose (V30 leaves every child row alone so restore
/// is a stamp clear rather than a value-copy replay), and a fold keeps
/// it too — [`fold_one`] re-points the *headstone's* children at the
/// keeper and never touches the headstone's own `container_id`. So a
/// `COUNT` over `container_id` alone advertises members that neither
/// the grid nor the container's own drill-down will show, which is the
/// same trap `SqliteGroupRepository::list` (`repo::group`) documents for
/// `asset_bucket`.
///
/// A separate constant from [`GRID_POPULATION`] purely because the
/// alias differs, and deliberately not merged with it: the outer
/// statements splice `asset.`-qualified text and these splice
/// `m.`-qualified text, so one constant would be wrong at half its
/// sites. Why the fold term is orthogonal to [`TrashFilter`] rather
/// than riding on it is argued at [`QueryParts::build`] and not
/// repeated here.
///
/// The alias lives **inside** the constant on purpose. A query that
/// wants to call the member table something else cannot splice this
/// text, and finds that out where the rule is written down rather than
/// three months later in a count that is off by the number of folds a
/// person has performed. The other half of that forcing function is
/// the source scan `every_member_query_names_the_member_population`.
///
/// **Not for the queries that ask the opposite question.**
/// [`SessionRepository::delete_if_empty`](asterism_core::domain::repository::SessionRepository::delete_if_empty)
/// and
/// [`ModalityRepository::asset_count`](asterism_core::domain::repository::ModalityRepository::asset_count)
/// ask "is anything still pointing here?", where a trashed or folded
/// row is precisely what has to count — deleting the thing it points at
/// would orphan a row that can still come back (trash) or that nothing
/// may ever delete (headstone). Those two are filterless by argument,
/// stated where they stand, not by omission.
pub(crate) const MEMBER_POPULATION: &str = "m.folded_into IS NULL AND m.trashed_at IS NULL";

/// Same predicate as a conjunct, for queries that already have a
/// `WHERE`.
fn trash_and(trash: TrashFilter) -> &'static str {
    match trash {
        TrashFilter::LiveOnly => "AND trashed_at IS NULL",
        TrashFilter::TrashedOnly => "AND trashed_at IS NOT NULL",
        TrashFilter::Any => "",
    }
}

/// Builds the shared `WHERE` clause and parameter list for the asset
/// queries (visibility filter included).
///
/// Exposed `pub(crate)` so the Query Group evaluator
/// ([`crate::sqlite::repo::query_group`]) reuses the *exact* same filter
/// surface when it materialises a query's members — the read path and
/// the evaluate path have to agree on WHERE semantics, so the builder
/// must not be duplicated. The search path joins the same builder through
/// [`restrict_to_ids`](Self::restrict_to_ids), so all three paths
/// (list / evaluate / search) share one predicate set.
pub(crate) struct QueryParts {
    pub(crate) where_sql: String,
    pub(crate) params: Vec<Value>,
}

impl QueryParts {
    pub(crate) fn build(query: &AssetQuery) -> Self {
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        // The fold axis first, and with no way to switch it off: a
        // headstone is not a thing to list, on either side of the
        // trash. Every consumer of this builder (list / search /
        // list_index / Query-Group evaluate) inherits it by
        // construction, which is the same reason the trash side is
        // built here rather than left to callers.
        //
        // **Orthogonal to `TrashFilter`, including `Any`.** `Any` is
        // the "show me the whole table" diagnostic, and the tempting
        // reading is that it should therefore include headstones —
        // rejected, for a reason that outlives the diagnostic: `Any` is
        // a value on the *trash* axis, and letting it also mean
        // "and folds" would make one enum answer two questions. A
        // caller that switches from `LiveOnly` to `Any` is asking about
        // the trash, and would not expect duplicate rows for every fold
        // ever performed to appear in the listing it is diffing.
        // Reaching headstones stays an explicit, separate read (`find`
        // by id, `find_by_source` by locator, and the fold panel's own
        // query when P2 adds it) rather than a side effect of a flag
        // about something else.
        conditions.push("asset.folded_into IS NULL".into());
        // `LiveOnly` is the query default, which is why a caller cannot
        // leak trashed rows into a listing by forgetting a flag.
        match query.trash {
            TrashFilter::LiveOnly => conditions.push("asset.trashed_at IS NULL".into()),
            TrashFilter::TrashedOnly => conditions.push("asset.trashed_at IS NOT NULL".into()),
            TrashFilter::Any => {}
        }
        if let Some(persona_id) = &query.persona_id {
            conditions.push("persona_id = ?".into());
            params.push(Value::Blob(persona_id.as_uuid().as_bytes().to_vec()));
        }
        if let Some(modality) = &query.modality {
            conditions.push("modality = ?".into());
            params.push(Value::Text(modality.as_str().to_string()));
        }
        if let Some(from) = &query.occurred_from {
            conditions.push("occurred_at >= ?".into());
            params.push(Value::Integer(datetime_to_ms(from)));
        }
        if let Some(until) = &query.occurred_until {
            conditions.push("occurred_at < ?".into());
            params.push(Value::Integer(datetime_to_ms(until)));
        }
        // Ingest and last-modification windows — the differential-sync
        // axes, and the reason both ends are `<=` where the occurrence
        // window's upper end is `<`: the caller replays a cursor the
        // server handed it, so the row sitting exactly on that instant
        // has to stay in the answer.
        //
        // Columns are qualified (`asset.`) because this WHERE clause is
        // spliced into statements that have a second table in scope:
        // `page` / `page_index` join `asset_bucket` on a single-Group
        // filter, and the group-id passes (`page_index`'s `group_sql`,
        // `query_group::fetch_sortable_assets`) select *from*
        // `asset_bucket` joined onto `asset`. `asset_bucket` carries no
        // `created_at` / `updated_at` today, so an unqualified name
        // happens to resolve — the qualification is what keeps that from
        // being a column the link table must never grow. (The search path
        // is not the reason: `filter_ids` runs `SELECT id FROM asset`
        // with no join at all.)
        if let Some(from) = &query.created_from {
            conditions.push("asset.created_at >= ?".into());
            params.push(Value::Integer(datetime_to_ms(from)));
        }
        if let Some(until) = &query.created_until {
            conditions.push("asset.created_at <= ?".into());
            params.push(Value::Integer(datetime_to_ms(until)));
        }
        if let Some(from) = &query.updated_from {
            conditions.push("asset.updated_at >= ?".into());
            params.push(Value::Integer(datetime_to_ms(from)));
        }
        if let Some(until) = &query.updated_until {
            conditions.push("asset.updated_at <= ?".into());
            params.push(Value::Integer(datetime_to_ms(until)));
        }
        // Multi-tag, either combinator. Empty vector adds no clause
        // (filter disabled) under both. The composite
        // `idx_asset_tag_tag(tag_id, asset_id)` serves the seek either
        // way — an `IN` list on one side, an equality on each of the
        // `All` conjuncts on the other.
        if !query.tag_ids.is_empty() {
            match query.tag_match {
                // Any-of: a single `EXISTS (…)` with `tag_id IN (?, ?,
                // …)`, so an asset needs to carry at least one of the
                // requested tags to pass.
                TagMatch::Any => {
                    let placeholders = vec!["?"; query.tag_ids.len()].join(", ");
                    conditions.push(format!(
                        "EXISTS (SELECT 1 FROM asset_tag \
                         WHERE asset_tag.asset_id = asset.id \
                           AND asset_tag.tag_id IN ({placeholders}))"
                    ));
                    for tag_id in &query.tag_ids {
                        params.push(Value::Blob(tag_id.as_uuid().as_bytes().to_vec()));
                    }
                }
                // All-of: one `EXISTS` per tag, ANDed. Written as N
                // separate conjuncts rather than one subquery with
                // `COUNT(DISTINCT tag_id) = N`, because each conjunct is
                // an index seek the planner can order and short-circuit
                // — and because a `COUNT` over the join would depend on
                // the link table having no duplicate `(asset, tag)` row
                // to be correct, which is a second thing to be right
                // about.
                TagMatch::All => {
                    for tag_id in &query.tag_ids {
                        conditions.push(
                            "EXISTS (SELECT 1 FROM asset_tag \
                             WHERE asset_tag.asset_id = asset.id \
                               AND asset_tag.tag_id = ?)"
                                .into(),
                        );
                        params.push(Value::Blob(tag_id.as_uuid().as_bytes().to_vec()));
                    }
                }
            }
        }
        // Multi-group OR (any-of): identical shape to the tag branch's
        // any-of, but against `asset_bucket`. `idx_asset_bucket_bucket
        // (bucket_id, asset_id)` (V4) serves the `IN` seek. There is no
        // all-of here: `tag_match` is named for the tag axis and only
        // combines that one.
        if !query.group_ids.is_empty() {
            let placeholders = vec!["?"; query.group_ids.len()].join(", ");
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM asset_bucket \
                 WHERE asset_bucket.asset_id = asset.id AND asset_bucket.bucket_id IN ({placeholders}))"
            ));
            for group_id in &query.group_ids {
                params.push(Value::Blob(group_id.as_uuid().as_bytes().to_vec()));
            }
        }
        // Composition drill: list the members of a composite Asset. The
        // partial `idx_asset_container` serves the seek. Modality-agnostic
        // (mixes messages + any images that entered the conversation).
        if let Some(container_id) = &query.container_id {
            conditions.push("asset.container_id = ?".into());
            params.push(Value::Blob(container_id.as_uuid().as_bytes().to_vec()));
        }
        // The selectable Unclassified bucket. Distinct from "no
        // modality filter" — this asks for exactly the rows the
        // MODALITY facet has no named row for.
        if query.modality_unset {
            conditions.push("asset.modality IS NULL".into());
        }
        // Format facet — a mime top-level type matched as a prefix
        // against the primary material's format fact. The `(asset_id,
        // ord)` PK serves the seek.
        if let Some(format) = &query.format {
            conditions.push(
                "EXISTS (SELECT 1 FROM material \
                 WHERE material.asset_id = asset.id AND material.ord = 0 \
                   AND material.mime LIKE ? || '/%')"
                    .into(),
            );
            params.push(Value::Text(format.clone()));
        }
        // Colour facet — the quantised palette lives in `asset_color`
        // (one row per asset per swatch), so the predicate is an
        // equality seek on the `(asset_id, bucket)` PK rather than a
        // scan over palette JSON.
        if let Some(color) = &query.color {
            conditions.push(
                "EXISTS (SELECT 1 FROM asset_color \
                 WHERE asset_color.asset_id = asset.id \
                   AND asset_color.bucket = ?)"
                    .into(),
            );
            params.push(Value::Text(color.as_str().to_string()));
        }
        // Star-rating band. `rating` is `NULL` for every asset nobody has
        // judged, and `NULL >= 3` is unknown rather than false, so the
        // bound alone already drops the unrated rows. The
        // `IS NOT NULL` conjunct is emitted anyway, for two reasons: it
        // states the contract in the clause a reader sees instead of
        // leaving it to be derived from three-valued logic, and it is
        // what makes the partial `idx_asset_persona_rating`
        // (`WHERE rating IS NOT NULL`) provably applicable rather than
        // dependent on how far the query planner's prover reaches.
        if query.rating_min.is_some() || query.rating_max.is_some() {
            conditions.push("asset.rating IS NOT NULL".into());
        }
        if let Some(min) = query.rating_min {
            conditions.push("asset.rating >= ?".into());
            params.push(Value::Integer(min as i64));
        }
        if let Some(max) = query.rating_max {
            conditions.push("asset.rating <= ?".into());
            params.push(Value::Integer(max as i64));
        }
        // Playback length and stored size — the same band one column
        // over, twice, so both go through the shared builder below
        // rather than being written out again here. What that buys is
        // narrow but exact: the two axes differ only in which column and
        // which pair of bounds they read, and a hand-copied second block
        // is precisely where a size question ends up answered in
        // milliseconds.
        //
        // Neither column carries an index, so unlike the rating band
        // above, the `IS NOT NULL` conjunct these emit is not making a
        // partial index reachable — it is there for what it states.
        push_metric_band(
            &mut conditions,
            &mut params,
            "asset.duration_ms",
            query.duration_min_ms,
            query.duration_max_ms,
        );
        push_metric_band(
            &mut conditions,
            &mut params,
            "asset.file_size_bytes",
            query.size_min_bytes,
            query.size_max_bytes,
        );
        // Resolution, as a total pixel count. The "column" handed to the
        // shared builder is an expression, which is exactly what makes
        // this the third instance of the same band rather than a fourth
        // shape: `NULL` propagates through `*`, so `IS NOT NULL` over the
        // product excludes a row missing *either* side — and since the
        // pair is written together or not at all
        // (`AssetService::refuse_half_written_dims`), that is precisely
        // the set of rows nobody measured.
        //
        // Parenthesised for the reader rather than for the parser:
        // SQLite binds `*` tighter than both `>=` and `IS NOT`, so the
        // fragments come out correct either way. They are kept because
        // the generated `WHERE` is read during debugging, where an
        // unbracketed product sitting between two `AND`s invites exactly
        // the misreading the brackets remove.
        //
        // Both columns are `u32` on the write side, so the product fits
        // an `i64` for any pair a real capture can carry. SQLite promotes
        // an integer overflow to REAL rather than wrapping, so even a
        // hand-written absurdity stays monotonic here instead of turning
        // a huge picture into a negative one.
        push_metric_band(
            &mut conditions,
            &mut params,
            "(asset.width_px * asset.height_px)",
            query.pixels_min,
            query.pixels_max,
        );
        if let Some(label) = &query.label {
            // `labels` is stored as a JSON array; match on exact
            // element equality via `json_each`.
            conditions.push(
                "EXISTS (SELECT 1 FROM json_each(asset.labels) WHERE json_each.value = ?)".into(),
            );
            params.push(Value::Text(label.as_str().to_string()));
        }
        // AlbumMeta facet — the statements live in `asset.extra`, and
        // this reads the projection `asset_album_meta` instead, for the
        // reason the colour branch above reads `asset_color`: the bag is
        // the importer's and its size has no ceiling, so opening one per
        // row is a scan that grows with what importers happen to record.
        // The projection is kept level by triggers on the column
        // (V67), so this cannot be reading something a write path forgot
        // to maintain.
        //
        // One `EXISTS` for the pair rather than two: naming both asks
        // for a row where *this key holds this value*, which two
        // independent clauses would loosen into "carries this key
        // somewhere and this value somewhere".
        if query.album_meta_key.is_some() || query.album_meta_value.is_some() {
            let mut inner = vec!["asset_album_meta.asset_id = asset.id".to_string()];
            if query.album_meta_key.is_some() {
                inner.push("asset_album_meta.key = ?".into());
            }
            if query.album_meta_value.is_some() {
                inner.push("asset_album_meta.value = ?".into());
            }
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM asset_album_meta WHERE {})",
                inner.join(" AND ")
            ));
            // Bound in clause order, which is key before value — the
            // order the composite `idx_asset_album_meta_key_value` is
            // built in, and the order the conditions were pushed above.
            if let Some(key) = &query.album_meta_key {
                params.push(Value::Text(key.clone()));
            }
            if let Some(value) = &query.album_meta_value {
                params.push(Value::Text(value.clone()));
            }
        }
        if let Some(text) = &query.text_match {
            let (sql, param) = text_match_term(text);
            conditions.push(sql);
            params.push(param);
        }
        // Restricted assets must not surface for subjects outside the
        // sharing list.
        if let Viewer::Subject(subject) = &query.viewer {
            conditions.push(
                "(vis_restricted = 0 OR EXISTS \
                 (SELECT 1 FROM json_each(asset.vis_sharing) WHERE json_each.value = ?))"
                    .into(),
            );
            params.push(Value::Text(subject.clone()));
        }

        let where_sql = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        Self { where_sql, params }
    }
}

/// Appends one inclusive numeric band over a nullable column, together
/// with the conjunct that states which rows the band is even about.
///
/// **The `IS NOT NULL` conjunct is emitted once, from either end.** A
/// `NULL` column already fails `>= ?` and `<= ?` on its own — three-
/// valued logic makes the comparison unknown, not true — so the
/// conjunct changes no answer. It is written because the exclusion is
/// the contract rather than a side effect of how SQL treats unknowns:
/// "under two minutes" is a question about material that plays, and a
/// still image has no length to place in the band. Emitting it per
/// bound instead would say the same thing twice for a two-ended band.
///
/// `column` arrives qualified (`asset.…`) from every caller for the
/// reason the timestamp windows are qualified: this predicate is
/// spliced into statements that have a second table in scope. It
/// matters more here than there — `material` carries a
/// `file_size_bytes` column of its own, so an unqualified size band
/// would become ambiguous the day one of those joins reaches a query
/// built through here, and until that day it would silently be reading
/// whichever table the resolver preferred.
///
/// It may also be an **expression** rather than a bare column — the
/// resolution band passes `(asset.width_px * asset.height_px)`. Two
/// things about that are worth stating, because neither is obvious from
/// the format strings below:
///
/// - The fragments append an operator directly (`{column} >= ?`), so an
///   expression has to be one that survives that. Arithmetic does, on
///   SQLite's precedence — `*` binds tighter than `>=` and than
///   `IS NOT` — and the resolution band brackets itself anyway so the
///   generated SQL reads unambiguously between its neighbouring `AND`s.
///   An expression containing a bare `AND` / `OR` would *not* survive,
///   and would silently widen the whole `WHERE`.
/// - The `IS NOT NULL` conjunct keeps its meaning under an expression
///   rather than gaining a new one: `NULL` propagates through arithmetic,
///   so the conjunct excludes a row missing any operand, which is what a
///   band over a derived value has to mean.
///
/// Bounds are `u64` because a negative *request* is malformed and is
/// refused at the wire, while the column is a SQLite `INTEGER`. A bound
/// past `i64::MAX` therefore names a value the column cannot hold, and
/// both ends saturate there rather than wrapping: `as i64` would turn
/// such a floor into a negative one and widen the band to everything,
/// which is the one direction a too-large bound must not go.
fn push_metric_band(
    conditions: &mut Vec<String>,
    params: &mut Vec<Value>,
    column: &str,
    min: Option<u64>,
    max: Option<u64>,
) {
    if min.is_none() && max.is_none() {
        return;
    }
    conditions.push(format!("{column} IS NOT NULL"));
    if let Some(min) = min {
        conditions.push(format!("{column} >= ?"));
        params.push(Value::Integer(i64::try_from(min).unwrap_or(i64::MAX)));
    }
    if let Some(max) = max {
        conditions.push(format!("{column} <= ?"));
        params.push(Value::Integer(i64::try_from(max).unwrap_or(i64::MAX)));
    }
}

/// Shortest pattern an FTS5 `trigram` index can serve. A pattern below
/// this does not contain a whole trigram, so there is no index entry
/// that could hold it.
const TRIGRAM_MIN_CHARS: usize = 3;

/// Builds the `text_match` term: one meaning (the body contains this
/// string), two ways of reaching it.
///
/// - **3 characters or more** → seek `asset_fts`, the FTS5 `trigram`
///   index (V58). The join runs through `asset_fts_key`, never through
///   an implicit rowid, so a `VACUUM` or a table rebuild cannot quietly
///   re-point it (see the V58 comment).
/// - **1–2 characters** → `LIKE` over `asset_body`. Identical answer,
///   no index: a trigram index cannot serve a pattern shorter than a
///   trigram, and narrowing the *meaning* by query length instead
///   (short → whole words) would drop `猫` inside `黒猫` with nothing
///   on screen to say so.
///
/// The scan the short branch costs is bounded by the rest of the
/// `WHERE`: it sits beside the tag / modality / date terms, so a lit
/// chip narrows the row set before the body is touched. An unfiltered
/// 1-character query over a large library is the slow case, and it is
/// the one the caller can see coming.
///
/// Character count, not `len()`: `猫` is three bytes and one character,
/// and the trigram index counts characters.
fn text_match_term(text: &str) -> (String, Value) {
    if text.chars().count() >= TRIGRAM_MIN_CHARS {
        (
            "asset.id IN (SELECT k.asset_id FROM asset_fts f \
             JOIN asset_fts_key k ON k.seq = f.rowid \
             WHERE f.body MATCH ?)"
                .into(),
            // Wrapped in FTS5 double quotes so the pattern is taken as
            // a string literal. Without it a term containing `-`, `*`,
            // `(`, `:` or `OR` would parse as query syntax and either
            // error or answer a different question — a search for
            // `AND-1` is a search for that text, not a boolean.
            // Inner `"` doubles, per FTS5 string literal rules.
            Value::Text(format!("\"{}\"", text.replace('"', "\"\""))),
        )
    } else {
        (
            "EXISTS (SELECT 1 FROM asset_body b \
             WHERE b.asset_id = asset.id AND b.body_text LIKE ? ESCAPE '\\')"
                .into(),
            // `%` / `_` / `\` in the pattern are the user's literal
            // characters, not wildcards — a search for `50%` is a
            // search for that text.
            Value::Text(format!(
                "%{}%",
                text.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )),
        )
    }
}

impl QueryParts {
    /// Appends `asset.id IN (…)` to the predicate built by
    /// [`build`](Self::build).
    ///
    /// Used by the retrieval path to narrow a candidate shortlist by
    /// the SQL filter surface without duplicating the filter
    /// predicates. Callers must chunk `ids` below SQLite's bound-variable
    /// ceiling — [`MAX_ID_FILTER_CHUNK`] is sized for that.
    fn restrict_to_ids(&mut self, ids: &[AssetId]) {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let clause = format!("asset.id IN ({placeholders})");
        if self.where_sql.is_empty() {
            self.where_sql = format!("WHERE {clause}");
        } else {
            self.where_sql.push_str(&format!(" AND {clause}"));
        }
        for id in ids {
            self.params
                .push(Value::Blob(id.as_uuid().as_bytes().to_vec()));
        }
    }
}

/// SQLite adapter for `AssetRepository` (uses a writer isle).
///
/// `list_sessions` reads the `session` table directly and joins the
/// `asset` aggregate for the derived `message_count` /
/// `started_at_ms` / `ended_at_ms` columns; the removed rkyv
/// snapshot module (previously served this read path) was retired
/// with the Session 1st-class migration because the snapshot's
/// record shape no longer matches the entity.
#[derive(Clone)]
pub struct SqliteAssetRepository {
    isle: AsyncIsle,
}

impl SqliteAssetRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    async fn page(&self, query: &AssetQuery) -> Result<Page<AssetCard>, DomainError> {
        let limit = query.limit.clamp(1, MAX_LIMIT);
        let offset = query.offset;
        let parts = QueryParts::build(query);
        // Single-group filter is the "browse one collection" case: the
        // user's hand-arranged order is the point, so join asset_bucket
        // and sort by `position`. The WHERE branch above already
        // scoped the result set to that bucket via EXISTS, so the join
        // is safe (each surviving asset has exactly one matching row).
        // Union filters (multi-group) fall back to occurred_at because
        // per-bucket position has no meaning across buckets.
        // The one Group the page is scoped to, when the filter names
        // exactly one. Drives both the arrival order below and which
        // Group counts as primary on every returned card
        // (`primary_group_first`).
        let sole_group: Option<Uuid> = if query.group_ids.len() == 1 {
            Some(*query.group_ids[0].as_uuid())
        } else {
            None
        };
        let single_group_bytes: Option<Vec<u8>> = sole_group.map(|uuid| uuid.as_bytes().to_vec());
        let (order_sql, join_sql) = if single_group_bytes.is_some() {
            (
                "ORDER BY asset_bucket.position ASC, asset.id ASC".to_string(),
                "JOIN asset_bucket \
                       ON asset_bucket.asset_id = asset.id \
                      AND asset_bucket.bucket_id = ?"
                    .to_string(),
            )
        } else {
            ("ORDER BY occurred_at DESC".to_string(), String::new())
        };
        let select_sql = format!(
            "SELECT {} FROM asset {} {} {} LIMIT ? OFFSET ?",
            CardRow::columns(),
            join_sql,
            parts.where_sql,
            order_sql,
        );
        let count_sql = format!("SELECT count(*) FROM asset {}", parts.where_sql);
        let params = parts.params;

        let (rows, group_map, total) = self
            .isle
            .call(move |conn| {
                // Join parameter goes first so it lines up with the
                // leading `?` in `JOIN asset_bucket ... = ?`; the
                // WHERE-clause params follow, then LIMIT/OFFSET.
                let mut select_params: Vec<Value> = Vec::with_capacity(params.len() + 3);
                if let Some(bytes) = &single_group_bytes {
                    select_params.push(Value::Blob(bytes.clone()));
                }
                select_params.extend(params.iter().cloned());
                select_params.push(Value::Integer(limit as i64));
                select_params.push(Value::Integer(offset as i64));
                let mut stmt = conn.prepare(&select_sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(select_params), CardRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                let asset_uuids: Vec<Uuid> = rows.iter().map(|r: &CardRow| r.id).collect();
                let group_map = fetch_group_ids_map(conn, &asset_uuids)?;
                let total: i64 = conn.query_row(
                    &count_sql,
                    rusqlite::params_from_iter(params.clone()),
                    |row| row.get(0),
                )?;
                Ok((rows, group_map, total))
            })
            .await
            .map_err(infra_err)?;

        let mut items = rows
            .into_iter()
            .map(CardRow::into_card)
            .collect::<Result<Vec<_>, _>>()?;
        attach_group_ids(&mut items, &group_map, sole_group);
        Ok(Page {
            items,
            offset,
            limit,
            total: Some(total.max(0) as u64),
        })
    }

    /// Index-only counterpart of [`page`]. Uses the same filter /
    /// order / group-join logic but drops cover and source_locator from
    /// the SELECT so a 6-figure result set is cheap over the IPC
    /// boundary — the two metric columns stay in, because they are sort
    /// keys rather than render payload ([`IndexRow`]). The search read
    /// path does not come
    /// through here — it intersects Tantivy hits with
    /// [`filter_ids`](AssetRepository::filter_ids) and hydrates full
    /// cards (small top-K result sets, snippet + score piggyback).
    async fn page_index(
        &self,
        query: &AssetQuery,
    ) -> Result<Page<asterism_core::domain::asset::AssetIndex>, DomainError> {
        let limit = query.limit.clamp(1, MAX_LIMIT);
        let offset = query.offset;
        // Same filter surface as `page` (list mode), differing only in
        // the SELECT column set.
        let parts = QueryParts::build(query);

        // Same contract as `page`: the sole filtered Group owns both the
        // arrival order and the primary-group answer.
        let sole_group: Option<Uuid> = if query.group_ids.len() == 1 {
            Some(*query.group_ids[0].as_uuid())
        } else {
            None
        };
        let single_group_bytes: Option<Vec<u8>> = sole_group.map(|uuid| uuid.as_bytes().to_vec());
        let (order_sql, join_sql) = if single_group_bytes.is_some() {
            (
                "ORDER BY asset_bucket.position ASC, asset.id ASC".to_string(),
                "JOIN asset_bucket \
                       ON asset_bucket.asset_id = asset.id \
                      AND asset_bucket.bucket_id = ?"
                    .to_string(),
            )
        } else {
            ("ORDER BY occurred_at DESC".to_string(), String::new())
        };
        let select_sql = format!(
            "SELECT {} FROM asset {} {} {} LIMIT ? OFFSET ?",
            IndexRow::COLUMNS,
            join_sql,
            parts.where_sql,
            order_sql,
        );
        let count_sql = format!("SELECT count(*) FROM asset {}", parts.where_sql);
        // Group-id map in ONE pass: join `asset_bucket` against the
        // same predicate instead of re-probing per returned id
        // (the old id-chunked `fetch_group_ids_map` cost ~200 IN()
        // queries / ~0.3-0.55 s at 110k rows [measured 2026-07-21]).
        // Over-fetches when LIMIT trims the page — harmless, the
        // attach step only consumes matching ids. The predicate
        // columns are unambiguous: `asset_bucket` shares no column
        // name the WHERE clause references.
        // `position` rides along, and the `bucket_id` ordering pins which
        // group counts as primary — same contract as
        // `fetch_group_ids_map`, which this pass replaced on the index
        // path.
        let group_sql = format!(
            "SELECT asset_bucket.asset_id, asset_bucket.bucket_id, asset_bucket.position \
             FROM asset_bucket JOIN asset ON asset.id = asset_bucket.asset_id {} \
             ORDER BY asset_bucket.asset_id, asset_bucket.bucket_id",
            parts.where_sql,
        );
        let params = parts.params;

        let (rows, group_map, total) = self
            .isle
            .call(move |conn| {
                // Breakdown for the persona-switch stall investigation.
                // No longer dev-only: the records reach stderr under
                // `RUST_LOG` and `diag_log` in every build, so the same numbers
                // are available from a bundled app after the fact.
                let t0 = std::time::Instant::now();
                let mut select_params: Vec<Value> = Vec::with_capacity(params.len() + 3);
                if let Some(bytes) = &single_group_bytes {
                    select_params.push(Value::Blob(bytes.clone()));
                }
                select_params.extend(params.iter().cloned());
                select_params.push(Value::Integer(limit as i64));
                select_params.push(Value::Integer(offset as i64));
                let mut stmt = conn.prepare(&select_sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params_from_iter(select_params),
                        IndexRow::from_row,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let t_query = t0.elapsed();
                let mut group_map: HashMap<Uuid, Vec<(Uuid, i64)>> =
                    HashMap::with_capacity(rows.len());
                {
                    let mut stmt = conn.prepare(&group_sql)?;
                    let mut gm_rows =
                        stmt.query(rusqlite::params_from_iter(params.iter().cloned()))?;
                    while let Some(row) = gm_rows.next()? {
                        let asset_id: Uuid = row.get(0)?;
                        let bucket_id: Uuid = row.get(1)?;
                        let position: i64 = row.get(2)?;
                        group_map
                            .entry(asset_id)
                            .or_default()
                            .push((bucket_id, position));
                    }
                }
                let t_group = t0.elapsed() - t_query;
                let total: i64 = conn.query_row(
                    &count_sql,
                    rusqlite::params_from_iter(params.clone()),
                    |row| row.get(0),
                )?;
                // Perf breakdown of one `list_index` call. `event` names
                // the stream, which is what routes this to `perf_log`
                // instead of the diagnostics table; the timings stay
                // structured so a later "why was this slow" question
                // filters on numbers instead of parsing prose.
                tracing::info!(
                    event = "perf.list_index",
                    op = "list_index",
                    duration_ms = t0.elapsed().as_millis() as u64,
                    phase = "database",
                    rows = rows.len(),
                    query_ms = t_query.as_millis() as u64,
                    group_map_ms = t_group.as_millis() as u64,
                    count_ms = (t0.elapsed() - t_query - t_group).as_millis() as u64,
                    "list_index database phase"
                );
                Ok((rows, group_map, total))
            })
            .await
            .map_err(infra_err)?;

        let t_map0 = std::time::Instant::now();
        let mut items = rows
            .into_iter()
            .map(IndexRow::into_index)
            .collect::<Result<Vec<_>, _>>()?;
        attach_group_ids_index(&mut items, &group_map, sole_group);
        tracing::info!(
            event = "perf.list_index",
            op = "list_index",
            duration_ms = t_map0.elapsed().as_millis() as u64,
            phase = "domain_mapping",
            "list_index domain mapping"
        );
        Ok(Page {
            items,
            offset,
            limit,
            total: Some(total.max(0) as u64),
        })
    }
}

#[async_trait]
impl AssetRepository for SqliteAssetRepository {
    /// By-id fetch — returns trashed rows too, on purpose. This is the
    /// read path the trash view, `restore`, and the `purge` guard all go
    /// through; filtering here would make a trashed asset unrecoverable
    /// through its own id. Listing paths do the excluding
    /// ([`QueryParts`]).
    /// Deliberately unfiltered on both the trash and the fold axis: a
    /// read that *names* a row returns that row. For headstones this is
    /// not a leniency but the mechanism — a stale `asset:<uuid>` claim
    /// or a dispatch record naming a folded id has to reach the row
    /// that carries `folded_into` in order to be redirected to the
    /// keeper at all. Filtering here would
    /// turn every pre-fold reference into a 404.
    async fn find(&self, id: &AssetId) -> Result<Option<Asset>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                let row = conn
                    .query_row(
                        &format!("SELECT {} FROM asset WHERE id = ?1", AssetRow::COLUMNS),
                        params![uuid],
                        AssetRow::from_row,
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                match row {
                    Some(row) => Ok(Some((row, MaterialRow::load_for(conn, uuid)?))),
                    None => Ok(None),
                }
            })
            .await
            .map_err(infra_err)?;
        row.map(|(row, materials)| {
            let mut asset = row.into_domain()?;
            asset.materials = materials
                .into_iter()
                .map(MaterialRow::into_domain)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(asset)
        })
        .transpose()
    }

    /// `json_extract` returns SQLite `0` for JSON `false`, so the
    /// predicate is an integer comparison. Rows without a `_trace`
    /// note yield NULL and drop out. No index backs this — the sweep
    /// runs once per reify and pending claims are rare, so a scan is
    /// cheaper than maintaining an expression index for it.
    ///
    /// Headstones are not swept. A claim on a folded row is a claim on
    /// a redirect, so resolving it names nothing a person can open, and
    /// nothing clears the flag either — the row would come back on
    /// every pass and take a slot from a claim that can still be
    /// answered. The trash is not excluded, on the opposite reasoning:
    /// a trashed row can be restored and would want its provenance
    /// already resolved when it is.
    async fn unresolved_provenance_ids(&self, limit: u32) -> Result<Vec<AssetId>, DomainError> {
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM asset \
                     WHERE json_extract(extra, '$._trace.resolved') = 0 \
                       AND folded_into IS NULL \
                     ORDER BY created_at ASC LIMIT ?1",
                )?;
                let ids = stmt
                    .query_map(params![limit as i64], |row| row.get::<_, uuid::Uuid>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ids)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn save(&self, asset: &Asset) -> Result<(), DomainError> {
        let id = *asset.id.as_uuid();
        let persona_id = *asset.persona_id.as_uuid();
        let source_kind = asset.source.kind.as_str().to_string();
        // B5, the write half. `to_storage()` is a **canonical**
        // rendering, not the string that was read: `file:///pics/a.png`
        // renders as `/pics/a.png` (the scheme is consumed on purpose,
        // so the two spellings are one locator) and `HF://…` renders
        // lowercased. Two consequences, both deliberate:
        //
        // - an ordinary read-modify-write of a row spelled either of
        //   those ways rewrites its `source_locator`, because the upsert
        //   below sets `source_locator = excluded.source_locator`;
        // - two rows spelled the two ways of one path therefore land on
        //   one value, and both of them stand.
        //
        // That second one is why the step demoting the Source pair to a
        // plain index (V61) lands before the migration that rewrites
        // every stored locator: the merge is legal under
        // `N : 1` and was a constraint violation under the UNIQUE, which
        // would have failed the upgrade on a database that was never
        // wrong. Preserving the original spelling instead is not an
        // option — the two spellings comparing equal is the property
        // being bought.
        let locator = asset.source.locator.to_storage();
        let file_size = asset.source.file_size_bytes.map(|v| v as i64);
        let platform = asset.source.platform.clone();
        let modality = asset.modality.as_ref().map(|m| m.as_str().to_string());
        let labels = strings_to_json(&asset.labels.iter().map(|l| l.as_str()).collect::<Vec<_>>());
        let occurred = datetime_to_ms(&asset.occurred_at);
        let bundle = asset.bundle_id.as_ref().map(|b| b.as_str().to_string());
        let cover = asset.cover.as_ref().map(|c| c.as_str().to_string());
        let keywords = strings_to_json(
            &asset
                .keywords
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>(),
        );
        let register = asset.register_note.as_ref().map(|r| r.as_str().to_string());
        let (restricted, sharing) = match &asset.visibility {
            Visibility::Open => (0i64, "[]".to_string()),
            Visibility::Restricted { sharing } => (1i64, strings_to_json(sharing)),
        };
        let duration = asset.duration_ms.map(|v| v as i64);
        // Widening, so no check to make in this direction: every `u32`
        // is an `i64`. The read side is where the column can hold
        // something the type cannot (`opt_u32`), which is also why a
        // fixture for that case has to be written with SQL rather than
        // through this path.
        let width_px = asset.width_px.map(i64::from);
        let height_px = asset.height_px.map(i64::from);
        let rating = asset.rating.map(|v| v as i64);
        let palette = asset
            .palette
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_else(|_| "[]".to_string()));
        let extra = match &asset.extra {
            serde_json::Value::Null => None,
            other => Some(other.to_string()),
        };
        let created = datetime_to_ms(&asset.created_at);
        let updated = datetime_to_ms(&asset.updated_at);
        // Composition membership persists as a BLOB self-reference;
        // `title` is the composite's user-authored name.
        let container = asset
            .container_id
            .as_ref()
            .map(|c| c.as_uuid().as_bytes().to_vec());
        let title = asset.title.clone();
        // Full-row upsert semantics: the entity owns every column,
        // `trashed_at` included, so a round-tripped trashed asset keeps
        // its stamp. Day-to-day trash / restore go through the dedicated
        // verbs instead — they are single-column writes and never need
        // to hydrate the entity.
        let trashed = asset.trashed_at.as_ref().map(datetime_to_ms);
        let role = asset.role.as_str().to_string();
        // Attribution (V47 + V50) travels with the entity like every
        // other column it owns: the pair is written from
        // `Author::encode`, so the two halves can never disagree, and
        // the channel travels with it.
        let (author_kind, author_subject) = match asset.author() {
            Some(author) => {
                let (kind, subject) = author.encode();
                (Some(kind.to_string()), subject.map(str::to_string))
            }
            None => (None, None),
        };
        let operator_ai = asset.operator_ai().map(|o| o.as_str().to_string());
        let attributed_via = asset.attributed_via().map(|c| c.slug().to_string());
        // The write-side rule the channel column adds: a row that
        // records somebody records how that answer arrived. Checked here
        // — the last point where what is about to be written is visible
        // as the column values themselves — because SQLite cannot carry
        // a CHECK on a column added by `ALTER TABLE`.
        super::attribution_guard::assert_channel_recorded(
            "asset",
            author_kind.as_deref(),
            operator_ai.as_deref(),
            attributed_via.as_deref(),
        )?;
        // The declared duplicate strategy (V52). Unlike its two
        // neighbours below it *is* written — by the INSERT only; see
        // the statement.
        let on_duplicate = asset.on_duplicate.map(|d| d.as_str().to_string());
        // Written by the INSERT and left out of the UPDATE, on the same
        // rule as `on_duplicate` above and for a related reason:
        // registration is where a source states what it calls this row,
        // and a whole-row `save` is not a re-statement. It also keeps
        // the Session composite's key — written by
        // `SessionRepository::create`, which is not this path — safe
        // from a metadata round-trip that hydrated it as `None`.
        let external_key = asset.external_key.clone();
        // `folded_into` / `fold_policy` are read off the row by
        // `AssetRow` and written by nothing here — see the column lists
        // in the statement below.
        //
        // Materials are upserted, never deleted, by `save`: an entity
        // whose materials were not hydrated (batch read paths) must not
        // wipe the physical layer on a metadata round-trip. Material
        // removal has no path in the current wave (one immutable
        // original per item).
        let materials: Vec<(i64, String, Option<i64>, Option<String>, i64, i64)> = asset
            .materials
            .iter()
            .map(|m| {
                (
                    m.ord as i64,
                    // The write half for the second column, same
                    // canonical rendering and same caveat as the asset
                    // locator above.
                    m.locator.to_storage(),
                    m.file_size_bytes.map(|v| v as i64),
                    // Stored as its token, the way `role` is.
                    m.mime.as_ref().map(|mime| mime.as_str().to_string()),
                    datetime_to_ms(&m.created_at),
                    datetime_to_ms(&m.updated_at),
                )
            })
            .collect();

        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    // `folded_into` and `fold_policy` appear in neither
                    // the insert list nor the update list, by the same
                    // rule as `palette` and `material.content_hash`
                    // below: the fold verb owns them through
                    // column-specific setters ("keeper
                    // updates go through column setters — a whole-row
                    // save is a lost update"). A new row takes the
                    // column defaults (NULL, `'auto'`), which is
                    // exactly what a freshly registered asset is; and a
                    // metadata round-trip through
                    // find → mutate → save cannot resurrect a
                    // headstone by carrying a stale `None` back over
                    // it.
                    //
                    // `on_duplicate` is in the insert list and out of
                    // the update list, which is neither of the two
                    // rules above but follows from what the value is.
                    // It has to be inserted: registration is the only
                    // moment it is known, and a column the write path
                    // never fills would make the declaration
                    // unreachable — the whole point of the subtask that
                    // added it. It must not be updated: the row records
                    // what the caller declared *then*, and no verb
                    // re-declares it (a resolution's durable outcome is
                    // `fold_policy`, the column next door). Leaving it
                    // out of the update list means no read-modify-write
                    // path can quietly restate a past intention as a
                    // present one.
                    "INSERT INTO asset
                         (id, persona_id, source_kind, source_locator, file_size_bytes,
                          platform, modality, labels, occurred_at, bundle_id,
                          cover, keywords, register_note, vis_restricted, vis_sharing,
                          duration_ms, rating, palette, extra, created_at, updated_at,
                          container_id, title, trashed_at, role,
                          author_kind, author_subject, operator_ai, attributed_via,
                          on_duplicate, external_key, width_px, height_px)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                             ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25,
                             ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33)
                     ON CONFLICT(id) DO UPDATE SET
                         persona_id = excluded.persona_id,
                         source_kind = excluded.source_kind,
                         source_locator = excluded.source_locator,
                         file_size_bytes = excluded.file_size_bytes,
                         platform = excluded.platform,
                         modality = excluded.modality,
                         labels = excluded.labels,
                         occurred_at = excluded.occurred_at,
                         bundle_id = excluded.bundle_id,
                         cover = excluded.cover,
                         keywords = excluded.keywords,
                         register_note = excluded.register_note,
                         vis_restricted = excluded.vis_restricted,
                         vis_sharing = excluded.vis_sharing,
                         duration_ms = excluded.duration_ms,
                         rating = excluded.rating,
                         -- `palette` is deliberately absent from the
                         -- update list: `set_palette` owns that column
                         -- and writes `asset_color` alongside it in one
                         -- transaction. A read-modify-write `save`
                         -- (find → mutate → save) that carried the
                         -- palette back would clobber an extraction
                         -- that landed in between, leaving swatches
                         -- whose palette no longer exists. The INSERT
                         -- still seeds the column, so a freshly
                         -- extracted entity persists normally.
                         extra = excluded.extra,
                         updated_at = excluded.updated_at,
                         container_id = excluded.container_id,
                         title = excluded.title,
                         trashed_at = excluded.trashed_at,
                         role = excluded.role,
                         author_kind = excluded.author_kind,
                         author_subject = excluded.author_subject,
                         operator_ai = excluded.operator_ai,
                         attributed_via = excluded.attributed_via,
                         -- Present in the update list, so **the arriving
                         -- entity wins** — including when it carries
                         -- nothing. Same rule as `duration_ms` and
                         -- `file_size_bytes`, which is the point: all
                         -- three come out of the same probe pass, and
                         -- giving these two a `COALESCE` would make one
                         -- probe's results follow two different
                         -- overwrite rules.
                         --
                         -- The cost is real and taken knowingly: a
                         -- re-ingest whose probe failed replaces a
                         -- measurement with `NULL`, and a backfill that
                         -- measured by decoding loses to the next
                         -- re-ingest that measured by header.
                         width_px = excluded.width_px,
                         height_px = excluded.height_px",
                    params![
                        id,
                        persona_id,
                        source_kind,
                        locator,
                        file_size,
                        platform,
                        modality,
                        labels,
                        occurred,
                        bundle,
                        cover,
                        keywords,
                        register,
                        restricted,
                        sharing,
                        duration,
                        rating,
                        palette,
                        extra,
                        created,
                        updated,
                        container,
                        title,
                        trashed,
                        role,
                        author_kind,
                        author_subject,
                        operator_ai,
                        attributed_via,
                        on_duplicate,
                        external_key,
                        width_px,
                        height_px
                    ],
                )?;
                for (ord, locator, size, mime, created, updated) in &materials {
                    tx.execute(
                        // Neither hash column appears in either list, by
                        // the same rule as `asset.palette` above:
                        // `set_material_fingerprint` owns both. Insert
                        // leaves them NULL (unknown, which is what a
                        // material with unread bytes is), a metadata
                        // round-trip cannot erase a fingerprint computed
                        // in between, and NULL is what puts a freshly
                        // arrived row in front of the walk.
                        "INSERT INTO material
                             (asset_id, ord, locator, file_size_bytes, mime,
                              created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(asset_id, ord) DO UPDATE SET
                             locator = excluded.locator,
                             file_size_bytes = excluded.file_size_bytes,
                             mime = excluded.mime,
                             updated_at = excluded.updated_at",
                        params![id, ord, locator, size, mime, created, updated],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn trash(&self, id: &AssetId, at: DateTime<Utc>) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let stamp = datetime_to_ms(&at);
        self.isle
            .call(move |conn| {
                // `trashed_at IS NULL` keeps the write idempotent while
                // preserving the *original* stamp: re-trashing must not
                // restart the retention clock and win a purge reprieve.
                conn.execute(
                    "UPDATE asset SET trashed_at = ?1 WHERE id = ?2 AND trashed_at IS NULL",
                    params![stamp, uuid],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn restore(&self, id: &AssetId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset SET trashed_at = NULL WHERE id = ?1",
                    params![uuid],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn purge(&self, id: &AssetId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        // The guard lives **in the DELETE's own predicate**, not in a
        // preceding SELECT. A check-then-delete pair would be safe only
        // within a single process, and Asterism ships two that share the
        // database file (see the `busy_timeout` rationale in
        // `crate::sqlite`): a `restore` landing between the check and
        // the delete would let this irreversibly destroy a live asset
        // and its ten cascade children. With the predicate inlined, a
        // concurrent restore can only make the statement match zero
        // rows.
        //
        // The follow-up probe runs only when nothing was deleted, and
        // exists purely to tell "absent" (idempotent no-op) apart from
        // "still live" (Conflict) for the error message.
        //
        // 0 = purged (or already absent), 1 = still live.
        let verdict: u8 = self
            .isle
            .call(move |conn| {
                let deleted = conn.execute(
                    "DELETE FROM asset WHERE id = ?1 AND trashed_at IS NOT NULL",
                    params![uuid],
                )?;
                if deleted > 0 {
                    return Ok(0);
                }
                let live: Option<bool> = conn
                    .query_row(
                        "SELECT trashed_at IS NULL FROM asset WHERE id = ?1",
                        params![uuid],
                        |row| row.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                match live {
                    // Absent: idempotent no-op, the caller's intent
                    // ("this must be gone") already holds.
                    None => Ok(0),
                    Some(true) => Ok(1),
                    // Trashed but the DELETE matched nothing: a
                    // concurrent purge won the race. Same end state.
                    Some(false) => Ok(0),
                }
            })
            .await
            .map_err(infra_err)?;
        if verdict == 1 {
            return Err(DomainError::Conflict(format!(
                "asset {id} is still live; trash it before purging"
            )));
        }
        Ok(())
    }

    /// Unfiltered on the fold axis in the query, like
    /// [`find`](Self::find), and for a reason specific to this lookup:
    /// the fold leaves the locator with the headstone rather than
    /// copying it onto the keeper (the row that holds a Source value is
    /// the one that was imported from there). So a re-arrival at that
    /// path has to *reach* the headstone; excluding folded rows here
    /// would report the path as unseen and let a second copy of an
    /// already-resolved duplicate walk back in.
    ///
    /// Reaching it is not the same as answering with it, and that is
    /// where the two scopes part:
    ///
    /// * `Any` hands back the row the query found. The question is "who
    ///   is standing at this address", and a headstone is standing
    ///   there.
    /// * `Live` walks on through [`resolve_fold_chain`] and answers with
    ///   the keeper. A headstone is in no listing, so returning it hands
    ///   the caller an id nothing can show — and the ingest path passes
    ///   that id straight to whoever re-imported the path.
    ///
    /// The walk is off the common path by construction: it starts only
    /// once `folded_into` came back non-NULL, so an ordinary re-scan of
    /// a live row costs the one statement it always did.
    ///
    /// A walk that dead-ends does **not** end the lookup. It ends that
    /// candidate, and the next row holding the locator is tried, oldest
    /// first, up to [`FOLD_CANDIDATE_SCAN`] of them; only when every one
    /// of them dead-ends is the locator held by nothing live. Ending the
    /// whole lookup instead is a way to mint forever: a fold writes no
    /// `trashed_at`, so the headstone stays live and stays the earliest
    /// row, which means the row just minted is never the one this query
    /// finds — the next sweep asks the same question, gets the same
    /// dead end, and mints again.
    ///
    /// The trash axis is the caller's (`scope`), and the `ORDER BY` is
    /// not cosmetic: since V61 several rows may carry one Source value,
    /// so without it the answer would be whichever row the planner
    /// reached first — and "the row that was already there" is the only
    /// reading under which a re-arrival is idempotent. The candidate
    /// order is that same order, which is why the second statement
    /// repeats it rather than sorting some other way.
    async fn find_by_source(
        &self,
        persona_id: &PersonaId,
        source_kind: &SourceKind,
        locator: &SourceLocator,
        scope: SourceLookupScope,
    ) -> Result<Option<Asset>, DomainError> {
        let persona = *persona_id.as_uuid();
        let kind = source_kind.as_str().to_string();
        // The column holds the storage rendering, so that is what the
        // equality test compares against — and because the rendering is
        // canonical, two callers spelling one locator differently ask
        // the same question here.
        let locator = locator.to_storage();
        let trash_filter = match scope {
            SourceLookupScope::Live => " AND trashed_at IS NULL",
            SourceLookupScope::Any => "",
        };
        let row = self
            .isle
            .call(move |conn| {
                let candidates = format!(
                    "SELECT {} FROM asset \
                     WHERE persona_id = ?1 AND source_kind = ?2 \
                       AND source_locator = ?3{trash_filter} \
                     ORDER BY created_at, id",
                    AssetRow::COLUMNS
                );
                let answer = |conn: &rusqlite::Connection, row: AssetRow| {
                    let materials = MaterialRow::load_for(conn, row.id)?;
                    Ok(Some((row, materials)))
                };

                let first = conn
                    .query_row(
                        &format!("{candidates} LIMIT 1"),
                        params![persona, kind, locator],
                        AssetRow::from_row,
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                let Some(first) = first else {
                    return Ok(None);
                };
                // The fold axis, resolved after the fact rather than in
                // the statement above: only a headstone pays for it, and
                // only the scope that asks about the library rather than
                // about storage. Everything else — every `Any` lookup,
                // and every `Live` lookup that landed on a row nobody
                // folded — is answered by the one statement above and
                // leaves here.
                let keeper = match (scope, first.folded_into) {
                    (SourceLookupScope::Live, Some(keeper)) => keeper,
                    _ => return answer(conn, first),
                };
                match resolve_fold_chain(conn, &first, keeper)? {
                    FoldResolution::Resolved(row) => return answer(conn, row),
                    dead_end => dead_end.report(&first),
                }

                // The oldest row holding this locator leads nowhere, so
                // the question moves to the ones behind it. `OFFSET 1`
                // rather than re-reading the first: it has just been
                // walked, and walking it twice would double the work of
                // exactly the case that is already the slow one.
                let mut stmt = conn.prepare(&format!(
                    "{candidates} LIMIT {} OFFSET 1",
                    FOLD_CANDIDATE_SCAN - 1
                ))?;
                let mut rows = stmt.query(params![persona, kind, locator])?;
                let mut examined = 1usize;
                while let Some(row) = rows.next()? {
                    let candidate = AssetRow::from_row(row)?;
                    examined += 1;
                    let Some(keeper) = candidate.folded_into else {
                        // Live and nobody's headstone: the answer.
                        return answer(conn, candidate);
                    };
                    match resolve_fold_chain(conn, &candidate, keeper)? {
                        FoldResolution::Resolved(row) => return answer(conn, row),
                        dead_end => dead_end.report(&candidate),
                    }
                }
                if examined == FOLD_CANDIDATE_SCAN {
                    // The answer below may be wrong — a row past the
                    // ceiling could have resolved — and that is worth
                    // saying, because a locator with this many rows on
                    // it and not one of them live is a state somebody
                    // needs to look at rather than a routine miss.
                    tracing::warn!(
                        event = "diag.asset.source_candidates_past_ceiling",
                        persona_id = %persona,
                        source_kind = %kind,
                        locator = %locator,
                        examined = FOLD_CANDIDATE_SCAN,
                        "every row this lookup would try for one locator dead-ended; \
                         any further ones were not tried"
                    );
                }
                Ok(None)
            })
            .await
            .map_err(infra_err)?;
        row.map(|(row, materials)| {
            let mut asset = row.into_domain()?;
            asset.materials = materials
                .into_iter()
                .map(MaterialRow::into_domain)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(asset)
        })
        .transpose()
    }

    async fn ids_by_persona(&self, persona_id: &PersonaId) -> Result<Vec<AssetId>, DomainError> {
        let uuid = *persona_id.as_uuid();
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // No LIMIT and no projection: the caller needs the whole
                // set (a truncated one would leave orphaned search
                // documents behind the cascade) and nothing but the ids.
                let mut stmt = conn.prepare("SELECT id FROM asset WHERE persona_id = ?1")?;
                stmt.query_map(params![uuid], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn trash_by_persona(
        &self,
        persona_id: &PersonaId,
        at: DateTime<Utc>,
    ) -> Result<Vec<AssetId>, DomainError> {
        let uuid = *persona_id.as_uuid();
        let stamp = datetime_to_ms(&at);
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // `RETURNING` keeps the stamp and the id list atomic:
                // reading the ids in a separate SELECT would race an
                // individual trash / restore and hand the caller a set
                // that never existed. The caller drops exactly these
                // documents from the search index.
                let mut stmt = conn.prepare(
                    "UPDATE asset SET trashed_at = ?1 \
                     WHERE persona_id = ?2 AND trashed_at IS NULL \
                     RETURNING id",
                )?;
                stmt.query_map(params![stamp, uuid], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn restore_by_persona(
        &self,
        persona_id: &PersonaId,
        stamp: DateTime<Utc>,
    ) -> Result<Vec<AssetId>, DomainError> {
        let uuid = *persona_id.as_uuid();
        let stamp_ms = datetime_to_ms(&stamp);
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // Matching on the exact stamp is the whole point: assets
                // the user had trashed by hand carry a different one and
                // must stay in the trash when the persona comes back.
                let mut stmt = conn.prepare(
                    "UPDATE asset SET trashed_at = NULL \
                     WHERE persona_id = ?1 AND trashed_at = ?2 \
                     RETURNING id",
                )?;
                stmt.query_map(params![uuid, stamp_ms], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn scan_purgeable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<AssetId>, DomainError> {
        let cutoff_ms = datetime_to_ms(&cutoff);
        let limit_i = limit.clamp(1, 5_000) as i64;
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // Served by the partial `idx_asset_trashed`. Oldest
                // first so a capped sweep always drains the longest-held
                // rows before the ones that just landed.
                //
                // Headstones are excluded, and this is the exclusion
                // that decides whether the fold axis works at all.
                // Retention is the one scheduled job that destroys
                // rows, so a folded row that also carries a trash stamp
                // — from `trash_by_persona`, or from a hand trash
                // before the fold — would be physically deleted here,
                // and every stale reference that resolved through it
                // would start returning nothing. That is precisely why
                // a fold is not expressed as a trash stamp.
                let mut stmt = conn.prepare(
                    "SELECT id FROM asset \
                     WHERE trashed_at IS NOT NULL AND trashed_at < ?1 \
                       AND folded_into IS NULL \
                     ORDER BY trashed_at ASC LIMIT ?2",
                )?;
                stmt.query_map(params![cutoff_ms, limit_i], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn list_trashed_ids(&self, limit: u32) -> Result<Vec<AssetId>, DomainError> {
        let limit_i = limit.clamp(1, 100_000) as i64;
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                // Same partial `idx_asset_trashed` as the retention
                // scan, minus the age predicate: "empty the trash"
                // takes what is there now. Oldest first so a capped
                // call still drains deterministically.
                //
                // Headstones are excluded for the same reason as in
                // `scan_purgeable`: this list is handed straight to
                // `purge`, and a redirect that gets deleted stops
                // redirecting. "Empty the trash" is also the one verb a
                // person invokes expecting *everything they can see* to
                // go — and a headstone is not something they can see.
                let mut stmt = conn.prepare(
                    "SELECT id FROM asset \
                     WHERE trashed_at IS NOT NULL \
                       AND folded_into IS NULL \
                     ORDER BY trashed_at ASC LIMIT ?1",
                )?;
                stmt.query_map(params![limit_i], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn list(&self, query: &AssetQuery) -> Result<Page<AssetCard>, DomainError> {
        self.page(query).await
    }

    async fn list_index(
        &self,
        query: &AssetQuery,
    ) -> Result<Page<asterism_core::domain::asset::AssetIndex>, DomainError> {
        self.page_index(query).await
    }

    async fn filter_ids(
        &self,
        ids: &[AssetId],
        query: &AssetQuery,
    ) -> Result<Vec<AssetId>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut kept: Vec<AssetId> = Vec::with_capacity(ids.len());
        // One statement per chunk, not one per candidate id. The parts
        // are rebuilt each round because the parameter vector is moved
        // into the isle closure.
        for chunk in ids.chunks(MAX_ID_FILTER_CHUNK) {
            let mut parts = QueryParts::build(query);
            parts.restrict_to_ids(chunk);
            let sql = format!("SELECT id FROM asset {}", parts.where_sql);
            let params = parts.params;
            let rows: Vec<Uuid> = self
                .isle
                .call(move |conn| {
                    let mut stmt = conn.prepare(&sql)?;
                    let rows = stmt
                        .query_map(rusqlite::params_from_iter(params), |row| {
                            row.get::<_, Uuid>(0)
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await
                .map_err(infra_err)?;
            kept.extend(rows.into_iter().map(AssetId::from_uuid));
        }
        Ok(kept)
    }

    /// `ORDER BY RANDOM() LIMIT k` over the shared predicate.
    ///
    /// The same `QueryParts` every other read path uses, so a chip that
    /// narrows the grid narrows the pool identically — including the
    /// visibility clause and the trash side, which is honoured here
    /// rather than refused: this is SQL over the `asset` table, so the
    /// trashed half is as reachable as the live one (the search path
    /// refuses it only because its index holds live rows alone).
    ///
    /// SQLite sorts the whole filtered set to answer this — there is no
    /// index that can produce a random order, and skipping the sort would
    /// mean biasing the picks towards one end of the table. The cost is
    /// the price of the verb; it is user-initiated, one draw at a time.
    async fn sample(&self, query: &AssetQuery, k: u32) -> Result<Vec<AssetId>, DomainError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let parts = QueryParts::build(query);
        let sql = format!(
            "SELECT id FROM asset {} ORDER BY RANDOM() LIMIT ?",
            parts.where_sql,
        );
        let mut params = parts.params;
        params.push(Value::Integer(k as i64));
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        row.get::<_, Uuid>(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    async fn set_palette(
        &self,
        asset_id: &AssetId,
        palette: Option<Vec<String>>,
    ) -> Result<(), DomainError> {
        let uuid = *asset_id.as_uuid();
        let json = palette
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_else(|_| "[]".into()));
        // The facet index is derived here, not by the caller: the
        // quantisation is a property of the palette, so the two writes
        // belong to the same transaction. `None` clears both.
        let buckets: Vec<&'static str> = palette
            .as_ref()
            .map(|p| {
                buckets_of(p.iter().map(String::as_str))
                    .into_iter()
                    .map(ColorBucket::as_str)
                    .collect()
            })
            .unwrap_or_default();
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "UPDATE asset SET palette = ?1 WHERE id = ?2",
                    params![json, uuid],
                )?;
                // Replace rather than merge: a re-extraction that drops
                // a colour must drop its swatch too.
                tx.execute("DELETE FROM asset_color WHERE asset_id = ?1", params![uuid])?;
                {
                    let mut insert = tx.prepare(
                        "INSERT OR IGNORE INTO asset_color (asset_id, bucket) VALUES (?1, ?2)",
                    )?;
                    for bucket in &buckets {
                        insert.execute(params![uuid, bucket])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn counts_by_persona(
        &self,
        trash: TrashFilter,
    ) -> Result<Vec<(PersonaId, u64)>, DomainError> {
        let rows: Vec<(Uuid, i64)> = self
            .isle
            .call(move |conn| {
                // The sidebar must not advertise assets the grid will
                // not show, which means following the grid to whichever
                // side of the trash it is on.
                let mut stmt = conn.prepare(&format!(
                    "SELECT persona_id, COUNT(*) AS c FROM asset \
                     WHERE {} {} \
                     GROUP BY persona_id \
                     ORDER BY c DESC, persona_id ASC",
                    GRID_POPULATION,
                    trash_and(trash)
                ))?;
                stmt.query_map(params![], |r| {
                    Ok((r.get::<_, Uuid>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, c)| (PersonaId::from_uuid(id), c as u64))
            .collect())
    }

    async fn counts_by_modality(
        &self,
        persona_id: Option<&PersonaId>,
        trash: TrashFilter,
    ) -> Result<Vec<(String, u64)>, DomainError> {
        let pid = persona_id.map(|p| *p.as_uuid());
        let rows: Vec<(Option<String>, i64)> = self
            .isle
            .call(move |conn| match pid {
                None => {
                    // Follows the grid's trash side (see
                    // `counts_by_persona`).
                    let mut stmt = conn.prepare(&format!(
                        "SELECT modality, COUNT(*) AS c FROM asset \
                         WHERE {} {} \
                         GROUP BY modality \
                         ORDER BY c DESC, modality ASC",
                        GRID_POPULATION,
                        trash_and(trash)
                    ))?;
                    stmt.query_map(params![], |r| {
                        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<_, _>>()
                }
                Some(uuid) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT modality, COUNT(*) AS c FROM asset \
                         WHERE persona_id = ?1 AND {} {} \
                         GROUP BY modality \
                         ORDER BY c DESC, modality ASC",
                        GRID_POPULATION,
                        trash_and(trash)
                    ))?;
                    stmt.query_map(params![uuid], |r| {
                        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<_, _>>()
                }
            })
            .await
            .map_err(infra_err)?;
        // Unclassified rows (modality NULL, asset-model v4) report under
        // the reserved `UNCLASSIFIED_MODALITY` key rather than being
        // dropped. Dropping them was the plan when the structural axes
        // were expected to catch them, but nothing did: a row with no
        // modality, no material and no container is in the grid and in
        // no facet at all, so the only way to reach it was to scroll
        // the unfiltered list.
        Ok(rows
            .into_iter()
            .map(|(m, c)| {
                (
                    m.unwrap_or_else(|| UNCLASSIFIED_MODALITY.to_string()),
                    c as u64,
                )
            })
            .collect())
    }

    async fn counts_by_format(
        &self,
        persona_id: Option<&PersonaId>,
        trash: TrashFilter,
    ) -> Result<Vec<(String, u64)>, DomainError> {
        let pid = persona_id.map(|p| *p.as_uuid());
        // Top-level rows only — the facet describes what the grid can
        // show, and members live inside their container's reader. The
        // format is the mime's top-level type; unknown mime (NULL or
        // no '/') carries no format and is skipped in SQL.
        let rows: Vec<(String, i64)> = self
            .isle
            .call(move |conn| match pid {
                None => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT substr(m.mime, 1, instr(m.mime, '/') - 1) AS fmt, \
                                COUNT(*) AS c \
                           FROM asset \
                           JOIN material m ON m.asset_id = asset.id AND m.ord = 0 \
                          WHERE m.mime IS NOT NULL AND instr(m.mime, '/') > 1 \
                            AND {} {} \
                          GROUP BY fmt \
                          ORDER BY c DESC, fmt ASC",
                        GRID_POPULATION,
                        trash_and(trash)
                    ))?;
                    stmt.query_map(params![], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<_, _>>()
                }
                Some(uuid) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT substr(m.mime, 1, instr(m.mime, '/') - 1) AS fmt, \
                                COUNT(*) AS c \
                           FROM asset \
                           JOIN material m ON m.asset_id = asset.id AND m.ord = 0 \
                          WHERE m.mime IS NOT NULL AND instr(m.mime, '/') > 1 \
                            AND {} \
                            AND asset.persona_id = ?1 {} \
                          GROUP BY fmt \
                          ORDER BY c DESC, fmt ASC",
                        GRID_POPULATION,
                        trash_and(trash)
                    ))?;
                    stmt.query_map(params![uuid], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<_, _>>()
                }
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(|(f, c)| (f, c as u64)).collect())
    }

    async fn counts_by_color(
        &self,
        persona_id: Option<&PersonaId>,
        trash: TrashFilter,
    ) -> Result<Vec<(ColorBucket, u64)>, DomainError> {
        let pid = persona_id.map(|p| *p.as_uuid());
        // Top-level rows only, matching the FORMAT facet: the sidebar
        // counts describe the grid, and the grid is top-level.
        // `asset_color` already holds one row per asset per bucket, so
        // COUNT(*) counts assets without a DISTINCT.
        let rows: Vec<(String, i64)> = self
            .isle
            .call(move |conn| match pid {
                None => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT c.bucket AS bucket, COUNT(*) AS n \
                           FROM asset_color c \
                           JOIN asset ON asset.id = c.asset_id \
                          WHERE {} {} \
                          GROUP BY bucket",
                        GRID_POPULATION,
                        trash_and(trash)
                    ))?;
                    stmt.query_map(params![], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<_, _>>()
                }
                Some(uuid) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT c.bucket AS bucket, COUNT(*) AS n \
                           FROM asset_color c \
                           JOIN asset ON asset.id = c.asset_id \
                          WHERE {} \
                            AND asset.persona_id = ?1 {} \
                          GROUP BY bucket",
                        GRID_POPULATION,
                        trash_and(trash)
                    ))?;
                    stmt.query_map(params![uuid], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .collect::<Result<_, _>>()
                }
            })
            .await
            .map_err(infra_err)?;
        // Swatch order, not count order (see the port doc). A slug the
        // closed set does not know is dropped rather than surfaced:
        // it could only come from a hand-edited row, and an unnamed
        // swatch is not something the sidebar can draw.
        let counts: HashMap<String, i64> = rows.into_iter().collect();
        Ok(ColorBucket::ALL
            .into_iter()
            .filter_map(|bucket| {
                counts
                    .get(bucket.as_str())
                    .map(|n| (bucket, (*n).max(0) as u64))
            })
            .collect())
    }

    /// One `UPDATE`, every column the pass produces — the window the
    /// port doc refuses to leave open. Two statements would let a
    /// reader see the row with one axis answered and another not, and
    /// nothing retries the second half.
    ///
    /// `meta_kv` travels in the same statement as the digest it belongs
    /// to for a sharper version of the same reason: they are one
    /// measurement, and a row holding a digest whose object had not
    /// landed yet would show a reader a body that says something other
    /// than what the index was built from. `meta_raw` is in it for the
    /// sharpest version yet — it is what `meta_kv` was rendered *from*,
    /// so a row where the two disagreed would offer a later reading a
    /// container's bytes beside somebody else's rendering of them.
    ///
    /// # A pass that measured nothing writes NULL, and that is a write
    ///
    /// `meta_raw` binds whatever the fingerprint holds, including
    /// `None`. Every caller of this method has just read a file, so
    /// `None` there means "this artefact keeps no bytes" and clearing
    /// the column is the correct answer. The one struct that carries a
    /// `None` it did not measure comes out of
    /// [`scan_fingerprinted_materials`](Self::scan_fingerprinted_materials),
    /// which is a read-only walk — its doc says why it does not select
    /// the column, and nothing hands what it produces back to here.
    async fn set_material_fingerprint(
        &self,
        asset_id: &AssetId,
        ord: u32,
        fingerprint: &MaterialFingerprint,
    ) -> Result<(), DomainError> {
        let uuid = *asset_id.as_uuid();
        let ord = i64::from(ord);
        let file = fingerprint.file.clone();
        let content = fingerprint.content.clone();
        let meta = fingerprint.meta.clone();
        let meta_kv = fingerprint.meta_kv.clone();
        let meta_raw = fingerprint.meta_raw.clone();
        let meta_text = fingerprint.meta_text.clone();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE material SET content_hash = ?1, content_region_hash = ?2, \
                                         meta_hash = ?3, meta_kv = ?4, meta_raw = ?5, \
                                         meta_text = ?6 \
                      WHERE asset_id = ?7 AND ord = ?8",
                    params![file, content, meta, meta_kv, meta_raw, meta_text, uuid, ord],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn note_trace_field(
        &self,
        asset_id: &AssetId,
        field: &str,
        note: serde_json::Value,
    ) -> Result<bool, DomainError> {
        let uuid = *asset_id.as_uuid();
        let field = field.to_string();
        self.isle
            .call(move |conn| {
                use rusqlite::OptionalExtension;

                // Read and write inside one transaction: the merge is a
                // read-modify-write of a JSON column, and two of them
                // racing (a second material of the same asset, a fold
                // note landing at the same moment) would keep only the
                // bag the later one started from.
                //
                // `updated_at` is deliberately not stamped, the same as
                // `set_material_fingerprint` beside it: the fingerprint and
                // what it confirmed are the job's bookkeeping about the
                // row, not a change to what the row holds, and stamping
                // would push every backfilled asset to the front of
                // every recently-changed read.
                let tx = conn.transaction()?;
                let extra: Option<Option<String>> = tx
                    .query_row(
                        "SELECT extra FROM asset WHERE id = ?1",
                        params![uuid],
                        |row| row.get(0),
                    )
                    .optional()?;
                // No row: the asset was purged between the hash landing
                // and this note. Nothing to say it on.
                let Some(extra) = extra else {
                    return Ok(false);
                };
                let Some(merged) = extra_with_trace_field(extra.as_deref(), &field, note) else {
                    return Ok(false);
                };
                tx.execute(
                    "UPDATE asset SET extra = ?2 WHERE id = ?1",
                    params![uuid, merged],
                )?;
                tx.commit()?;
                Ok(true)
            })
            .await
            .map_err(infra_err)
    }

    async fn scan_unhashed_materials(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<UnhashedMaterial>, DomainError> {
        let cursor = after.map(|(id, ord)| (*id.as_uuid(), i64::from(ord)));
        let limit = i64::from(limit);
        let rows: Vec<(Uuid, i64, String, Option<String>)> = self
            .isle
            .call(move |conn| {
                // The trash side is included on purpose: a trashed asset
                // can be restored, and re-reading its bytes later costs
                // the same as reading them now.
                //
                // The cursor compares the composite `(asset_id, ord)`
                // key, matching the ORDER BY. Comparing `asset_id`
                // alone would skip the remaining `ord > 0` materials of
                // an asset a page boundary cut through.
                //
                // `m.mime` travels with the row because the content axis
                // needs the format and this walk has no entity to read
                // it from — see the port's note on why it is not
                // re-derived from the locator here.
                let sql = format!(
                    "SELECT m.asset_id, m.ord, m.locator, m.mime \
                       FROM material m \
                      WHERE ({}) {{CURSOR}} \
                      ORDER BY m.asset_id, m.ord \
                      LIMIT ?1",
                    unfingerprinted_condition(
                        "m.content_hash",
                        "m.content_region_hash",
                        "m.meta_hash"
                    )
                );
                let sql = sql.as_str();
                // Bind exactly what each branch's SQL names — a padded
                // list only survives here because of the `?2` limit,
                // and that is not a property worth relying on.
                match cursor {
                    None => {
                        let mut stmt = conn.prepare(&sql.replace("{CURSOR}", ""))?;
                        stmt.query_map(params![limit], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?
                        .collect::<Result<_, _>>()
                    }
                    Some((uuid, ord)) => {
                        let mut stmt = conn.prepare(&sql.replace(
                            "{CURSOR}",
                            "AND (m.asset_id > ?2 OR (m.asset_id = ?2 AND m.ord > ?3))",
                        ))?;
                        stmt.query_map(params![limit, uuid, ord], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?
                        .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(
                |(asset_id, ord, locator, mime): (_, i64, String, Option<String>)| {
                    Ok(UnhashedMaterial {
                        asset_id: AssetId::from_uuid(asset_id),
                        ord: ord.max(0) as u32,
                        // B4: this walk's own projection has no entity to
                        // read the locator off, so it crosses the same
                        // boundary the entity path crosses — otherwise
                        // the backfill and the per-asset pass would hand
                        // `hash_material` two readings of one artefact.
                        locator: SourceLocator::try_from(locator.as_str())?,
                        // Same parse as the entity's boundary, so the two
                        // fingerprint passes cannot disagree about format.
                        mime: mime.as_deref().map(MimeType::parse),
                    })
                },
            )
            .collect()
    }

    async fn scan_chapter_scan_candidates(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<ChapterScanCandidate>, DomainError> {
        let cursor = after.map(|(id, ord)| (*id.as_uuid(), i64::from(ord)));
        let limit = i64::from(limit);
        let rows: Vec<(Uuid, i64, String, Option<String>)> = self
            .isle
            .call(move |conn| {
                // The mime prefixes are the SQL spelling of
                // `MimeType::carries_chapters`, and the only place the
                // two are apart: this predicate has to run inside the
                // query or the walk reads the whole table. The handler
                // re-asks through the real predicate on every row it is
                // handed, so a drift here narrows the page rather than
                // letting a PNG reach an ffmpeg.
                //
                // The trash side is included for the reason the
                // fingerprint walk includes it: a trashed asset can be
                // restored, and reading its chapters later costs the
                // same as reading them now.
                //
                // `NOT EXISTS` over the imported structure band is the
                // stamp — see `JobKind::ChapterScan`. It is a correlated
                // subquery rather than a `LEFT JOIN … IS NULL` so the
                // planner can stop at the first matching band row
                // (`idx_material_layer_asset` covers
                // `(asset_id, material_ord, role)`).
                let sql = "SELECT m.asset_id, m.ord, m.locator, m.mime \
                             FROM material m \
                            WHERE (m.mime LIKE 'video/%' OR m.mime LIKE 'audio/%') \
                              AND NOT EXISTS ( \
                                    SELECT 1 FROM material_layer l \
                                     WHERE l.asset_id     = m.asset_id \
                                       AND l.material_ord = m.ord \
                                       AND l.role         = 'structure' \
                                       AND l.origin       = 'imported') \
                              {CURSOR} \
                            ORDER BY m.asset_id, m.ord \
                            LIMIT ?1";
                match cursor {
                    None => {
                        let mut stmt = conn.prepare(&sql.replace("{CURSOR}", ""))?;
                        stmt.query_map(params![limit], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?
                        .collect::<Result<_, _>>()
                    }
                    Some((uuid, ord)) => {
                        let mut stmt = conn.prepare(&sql.replace(
                            "{CURSOR}",
                            "AND (m.asset_id > ?2 OR (m.asset_id = ?2 AND m.ord > ?3))",
                        ))?;
                        stmt.query_map(params![limit, uuid, ord], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?
                        .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(
                |(asset_id, ord, locator, mime): (_, i64, String, Option<String>)| {
                    Ok(ChapterScanCandidate {
                        asset_id: AssetId::from_uuid(asset_id),
                        ord: ord.max(0) as u32,
                        // Crosses the same boundary the entity path
                        // crosses, for the reason the fingerprint walk
                        // states: the two routes into one reader must
                        // not hand it two readings of one artefact.
                        locator: SourceLocator::try_from(locator.as_str())?,
                        mime: mime.as_deref().map(MimeType::parse),
                    })
                },
            )
            .collect()
    }

    async fn scan_unrecovered_text(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<UnhashedMaterial>, DomainError> {
        let cursor = after.map(|(id, ord)| (*id.as_uuid(), i64::from(ord)));
        let limit = i64::from(limit);
        let rows: Vec<(Uuid, i64, String, Option<String>)> = self
            .isle
            .call(move |conn| {
                // `mime IS NOT NULL` rides along because a row whose
                // format nothing guessed cannot pass the caller's format
                // test either, and leaving it in the page would spend a
                // locator parse per row to reach the same answer.
                //
                // Trashed rows stay in, on the same terms the hash walk
                // states: a trashed asset can be restored, and reading
                // its bytes later costs exactly what reading them now
                // costs.
                let sql = "SELECT m.asset_id, m.ord, m.locator, m.mime \
                             FROM material m \
                            WHERE m.meta_text IS NULL AND m.mime IS NOT NULL {CURSOR} \
                            ORDER BY m.asset_id, m.ord \
                            LIMIT ?1";
                match cursor {
                    None => {
                        let mut stmt = conn.prepare(&sql.replace("{CURSOR}", ""))?;
                        stmt.query_map(params![limit], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?
                        .collect::<Result<_, _>>()
                    }
                    Some((uuid, ord)) => {
                        let mut stmt = conn.prepare(&sql.replace(
                            "{CURSOR}",
                            "AND (m.asset_id > ?2 OR (m.asset_id = ?2 AND m.ord > ?3))",
                        ))?;
                        stmt.query_map(params![limit, uuid, ord], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?
                        .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(
                |(asset_id, ord, locator, mime): (_, i64, String, Option<String>)| {
                    Ok(UnhashedMaterial {
                        asset_id: AssetId::from_uuid(asset_id),
                        ord: ord.max(0) as u32,
                        locator: SourceLocator::try_from(locator.as_str())?,
                        mime: mime.as_deref().map(MimeType::parse),
                    })
                },
            )
            .collect()
    }

    async fn set_material_embedded_text(
        &self,
        asset_id: &AssetId,
        ord: u32,
        meta_text: Option<&str>,
    ) -> Result<(), DomainError> {
        let uuid = *asset_id.as_uuid();
        let ord = i64::from(ord);
        let meta_text = meta_text.map(str::to_string);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE material SET meta_text = ?1 WHERE asset_id = ?2 AND ord = ?3",
                    params![meta_text, uuid, ord],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn scan_dims_candidates(
        &self,
        scope: DimsScope,
        after: Option<&AssetId>,
        limit: u32,
    ) -> Result<Vec<DimsCandidate>, DomainError> {
        let cursor = after.map(|id| *id.as_uuid());
        let limit = i64::from(limit);
        let rows: Vec<(Uuid, String)> = self
            .isle
            .call(move |conn| {
                // The scope *is* the predicate — the three readings are
                // stated once, here, rather than as a boolean the caller
                // has to translate.
                //
                // The trash side is in every scope, for the reason the
                // fingerprint walk includes it: a trashed asset can be
                // restored, and reading its bytes later costs the same
                // as reading them now.
                //
                // The fold side is in every scope too, and for the
                // reason that is not symmetric with it: a headstone is
                // never restored and never deleted, so its dimensions
                // would be measured once and then be a permanent
                // resident of `Unlooked`'s tail — work whose result no
                // card displays. Outside every scope rather than inside
                // one, because the scope is a question about *what has
                // been measured*, and this is a row that stopped being
                // an asset.
                //
                // Ordering by `id` alone is enough — unlike the material
                // walk there is no second key, because the columns being
                // filled are the asset's own.
                let predicate = match scope {
                    DimsScope::Unlooked => "dims_probed_at IS NULL",
                    DimsScope::Unmeasured => "width_px IS NULL",
                    // `1` rather than an empty string: the fragment is
                    // spliced in front of the cursor's `AND`, so `All`
                    // still needs something for that `AND` to attach to.
                    DimsScope::All => "1",
                };
                let sql = format!(
                    "SELECT id, source_locator \
                       FROM asset \
                      WHERE {predicate} AND folded_into IS NULL {{CURSOR}} \
                      ORDER BY id \
                      LIMIT ?1"
                );
                match cursor {
                    None => {
                        let mut stmt = conn.prepare(&sql.replace("{CURSOR}", ""))?;
                        stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?
                            .collect::<Result<_, _>>()
                    }
                    Some(uuid) => {
                        let mut stmt = conn.prepare(&sql.replace("{CURSOR}", "AND id > ?2"))?;
                        stmt.query_map(params![limit, uuid], |r| Ok((r.get(0)?, r.get(1)?)))?
                            .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(asset_id, locator)| {
                Ok(DimsCandidate {
                    asset_id: AssetId::from_uuid(asset_id),
                    // Across the same boundary the entity path crosses,
                    // so this pass and the importer are looking at one
                    // reading of one locator.
                    locator: SourceLocator::try_from(locator.as_str())?,
                })
            })
            .collect()
    }

    async fn record_dims_probe(
        &self,
        asset_id: &AssetId,
        outcome: DimsProbe,
        policy: DimsWritePolicy,
        probed_at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        // A read that failed leaves **no trace**, so the row stays in
        // every scope it was in and a later pass can try again. Returning
        // before the statement is the whole of that rule; see
        // `DimsProbe::Unreadable`.
        let (width, height) = match outcome {
            DimsProbe::Unreadable => return Ok(()),
            DimsProbe::NothingToMeasure => (None, None),
            DimsProbe::Measured(w, h) => (Some(i64::from(w)), Some(i64::from(h))),
        };
        let id = *asset_id.as_uuid();
        let stamp = probed_at.timestamp_millis();
        self.isle
            .call(move |conn| {
                // One statement, so the pair and the stamp cannot land
                // apart — a row with dimensions and no stamp stays in
                // the startup walk and gets measured again, and a row
                // with a stamp and no dimensions after a successful
                // probe has thrown the answer away.
                //
                // The policy decides one thing: whether an existing
                // value survives. `FillOnly` guards against a concurrent
                // ingest, whose measurement came off the artefact at
                // import time and is the better evidence. `Overwrite` is
                // for a caller who knows something the stored value does
                // not — the file was replaced, or the way we measure
                // changed — and `COALESCE` there would read every
                // artefact in the library and change nothing.
                //
                // `NothingToMeasure` binds `NULL` for both, which under
                // `FillOnly` leaves the columns alone and under
                // `Overwrite` clears them. Clearing is right: the caller
                // asked for a re-measure and this is what came back.
                //
                // No `updated_at` bump under either policy. The stamp is
                // bookkeeping about a read, not a change to the asset,
                // and moving `updated_at` would make a differential-sync
                // consumer re-fetch the whole library because the server
                // audited it (`ListAssetsQuery::updated_from_ms` lists
                // what does move it).
                let assignment = match policy {
                    DimsWritePolicy::FillOnly => {
                        "width_px = COALESCE(width_px, ?2), \
                         height_px = COALESCE(height_px, ?3)"
                    }
                    DimsWritePolicy::Overwrite => "width_px = ?2, height_px = ?3",
                };
                conn.execute(
                    &format!("UPDATE asset SET {assignment}, dims_probed_at = ?4 WHERE id = ?1"),
                    params![id, width, height, stamp],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    async fn scan_fingerprinted_materials(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<FingerprintedMaterial>, DomainError> {
        let cursor = after.map(|(id, ord)| (*id.as_uuid(), i64::from(ord)));
        let limit = i64::from(limit);
        let rows: Vec<(Uuid, i64, String, String, String, Option<String>)> = self
            .isle
            .call(move |conn| {
                // `NOT (…)` over the *same* builder the hashing walk
                // selects with, rather than a second spelling of "has a
                // fingerprint". The two walks partition the table, and
                // they can only be relied on to partition it if the
                // predicate has one definition.
                //
                // Same composite `(asset_id, ord)` cursor and ORDER BY
                // for the reason that walk states: an `asset_id`-only
                // cursor skips the remaining `ord > 0` materials of an
                // asset a page boundary cut through.
                let sql = format!(
                    "SELECT m.asset_id, m.ord, m.content_hash, m.content_region_hash, \
                            m.meta_hash, m.meta_kv \
                       FROM material m \
                      WHERE NOT ({}) {{CURSOR}} \
                      ORDER BY m.asset_id, m.ord \
                      LIMIT ?1",
                    unfingerprinted_condition(
                        "m.content_hash",
                        "m.content_region_hash",
                        "m.meta_hash"
                    )
                );
                let sql = sql.as_str();
                let read = |r: &rusqlite::Row<'_>| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                };
                match cursor {
                    None => {
                        let mut stmt = conn.prepare(&sql.replace("{CURSOR}", ""))?;
                        stmt.query_map(params![limit], read)?
                            .collect::<Result<_, _>>()
                    }
                    Some((uuid, ord)) => {
                        let mut stmt = conn.prepare(&sql.replace(
                            "{CURSOR}",
                            "AND (m.asset_id > ?2 OR (m.asset_id = ?2 AND m.ord > ?3))",
                        ))?;
                        stmt.query_map(params![limit, uuid, ord], read)?
                            .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(
                |(asset_id, ord, file, content, meta, meta_kv)| FingerprintedMaterial {
                    asset_id: AssetId::from_uuid(asset_id),
                    ord: ord.max(0) as u32,
                    fingerprint: MaterialFingerprint {
                        file,
                        content,
                        meta,
                        meta_kv,
                        // **Not selected.** This walk pages the whole
                        // library to re-derive conflicts from digests,
                        // and `meta_raw` can reach a megabyte a row —
                        // read on every page for a comparison that
                        // never looks at it. `None` here is "this walk
                        // did not ask", which is safe because nothing
                        // writes the result back: `detect_duplicate`
                        // takes it and produces conflicts.
                        meta_raw: None,
                        // Same reason, smaller column: the duplicate
                        // walk never compares recovered text.
                        meta_text: None,
                    },
                },
            )
            .collect())
    }

    /// The same predicate as the scan above, from the same builder —
    /// the two answering different sets is what makes the progress
    /// notice lie in one direction or the other.
    async fn unhashed_material_count(&self) -> Result<u64, DomainError> {
        let count: i64 = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM material WHERE {}",
                        unfingerprinted_condition(
                            "content_hash",
                            "content_region_hash",
                            "meta_hash"
                        )
                    ),
                    params![],
                    |r| r.get(0),
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(count.max(0) as u64)
    }

    /// Equality against one marker, not a prefix test over the family,
    /// and the same fragment the migration selects by
    /// ([`unwalked_condition`]) rather than a second spelling of it.
    ///
    /// The other `unsupported:` values are answers that will not change
    /// — a format with no walker, a file past the size gate, a walk that
    /// found no region — and counting them here would report a problem
    /// nothing can fix. `NOT_WALKED` is the only one that means "these
    /// bytes were never read", which after the migration that reads them
    /// leaves exactly the originals it could not open.
    async fn unwalked_material_count(&self) -> Result<u64, DomainError> {
        let count: i64 = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM material WHERE {}",
                        unwalked_condition("content_region_hash")
                    ),
                    params![],
                    |r| r.get(0),
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(count.max(0) as u64)
    }

    /// `INSERT … ON CONFLICT DO NOTHING`, and the row count is the
    /// answer.
    ///
    /// The alternative — read the pair, insert if absent — is the
    /// check-then-write pair `purge` and `fold_into` both avoid: two
    /// workers fingerprinting the two halves of a pair at the same
    /// moment would both read "nothing queued" and both insert, and the
    /// second would fail on the UNIQUE index as an error rather than as
    /// "already asked". Here the conflict clause absorbs it and
    /// `execute` returns 0 rows, which is exactly the `false` the port
    /// promises.
    ///
    /// The sorted pair is taken from
    /// [`DuplicateConflict::pair_key`], not sorted again here: the
    /// domain owns which of two ids is "first", and a second ordering
    /// rule in the adapter is a rule that can be half-changed.
    async fn record_duplicate_conflict(
        &self,
        conflict: &DuplicateConflict,
    ) -> Result<bool, DomainError> {
        let (lo, hi) = conflict.pair_key();
        let id = *conflict.id.as_uuid();
        let persona = *conflict.persona_id.as_uuid();
        let lo = *lo.as_uuid();
        let hi = *hi.as_uuid();
        let newcomer = *conflict.newcomer.as_uuid();
        let incumbent = *conflict.incumbent.as_uuid();
        let axis = conflict.axis.as_str().to_string();
        let hash = conflict.content_hash.clone();
        let exclusion = conflict.fold_exclusion.map(|e| e.as_str().to_string());
        let detected = datetime_to_ms(&conflict.detected_at);
        let resolved = conflict.resolved_at.as_ref().map(datetime_to_ms);
        let resolution = conflict.resolution.map(|r| r.as_str().to_string());
        let inserted = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO duplicate_conflict
                         (id, persona_id, pair_lo, pair_hi, newcomer_id, incumbent_id,
                          axis, content_hash, fold_exclusion, detected_at, resolved_at,
                          resolution)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT (pair_lo, pair_hi, axis) DO NOTHING",
                    params![
                        id, persona, lo, hi, newcomer, incumbent, axis, hash, exclusion, detected,
                        resolved, resolution
                    ],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(inserted > 0)
    }

    /// One statement: the queue joined to both of its sides.
    ///
    /// The join is what makes "still worth asking" a question about the
    /// current state of the two rows rather than about a stamp somebody
    /// remembered to write. A fold or a trash on either side happens
    /// through verbs that know nothing about this table — and must not
    /// have to.
    async fn list_open_duplicate_conflicts(
        &self,
        persona_id: Option<&PersonaId>,
        limit: u32,
    ) -> Result<Vec<DuplicateConflict>, DomainError> {
        let pid = persona_id.map(|p| *p.as_uuid());
        let limit = i64::from(limit);
        let rows: Vec<ConflictRow> = self
            .isle
            .call(move |conn| {
                let sql = |persona_clause: &str| {
                    format!(
                        "SELECT {} \
                           FROM duplicate_conflict c \
                           JOIN asset lo ON lo.id = c.pair_lo \
                           JOIN asset hi ON hi.id = c.pair_hi \
                          WHERE c.resolved_at IS NULL \
                            AND lo.folded_into IS NULL AND hi.folded_into IS NULL \
                            AND lo.trashed_at IS NULL AND hi.trashed_at IS NULL \
                            {persona_clause} \
                          ORDER BY c.detected_at DESC, c.id DESC \
                          LIMIT ?1",
                        qualify(ConflictRow::COLUMNS, "c")
                    )
                };
                match pid {
                    None => {
                        let mut stmt = conn.prepare(&sql(""))?;
                        stmt.query_map(params![limit], ConflictRow::from_row)?
                            .collect::<Result<_, _>>()
                    }
                    Some(uuid) => {
                        let mut stmt = conn.prepare(&sql("AND c.persona_id = ?2"))?;
                        stmt.query_map(params![limit, uuid], ConflictRow::from_row)?
                            .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(ConflictRow::into_domain).collect()
    }

    /// A plain primary-key read — no join, no liveness filter. The port
    /// doc says why: the states this drops would each need a different
    /// answer from the verb that calls it.
    async fn find_duplicate_conflict(
        &self,
        id: &DuplicateConflictId,
    ) -> Result<Option<DuplicateConflict>, DomainError> {
        let key = *id.as_uuid();
        let row: Option<ConflictRow> = self
            .isle
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let sql = format!(
                    "SELECT {} FROM duplicate_conflict WHERE id = ?1",
                    ConflictRow::COLUMNS
                );
                conn.prepare(&sql)?
                    .query_row(params![key], ConflictRow::from_row)
                    .optional()
            })
            .await
            .map_err(infra_err)?;
        row.map(ConflictRow::into_domain).transpose()
    }

    /// One conditional UPDATE. `resolved_at IS NULL` in the `WHERE` is
    /// the compare-and-set that makes the caller's earlier read safe to
    /// act on: the loser of two panels answering at once changes no
    /// rows and is told so, rather than overwriting the first answer.
    async fn close_duplicate_conflict(
        &self,
        id: &DuplicateConflictId,
        resolution: ConflictResolution,
        resolved_at: DateTime<Utc>,
    ) -> Result<bool, DomainError> {
        let key = *id.as_uuid();
        let answer = resolution.as_str().to_string();
        let at = datetime_to_ms(&resolved_at);
        let updated = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE duplicate_conflict
                        SET resolved_at = ?2, resolution = ?3
                      WHERE id = ?1 AND resolved_at IS NULL",
                    params![key, at, answer],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(updated > 0)
    }

    /// One statement plus a materials read per hit.
    ///
    /// The hash lives on `material` and the answer is `asset` rows, so
    /// something has to bridge the two. The join that does it needs
    /// qualified column names: `AssetRow::COLUMNS` is a bare list and
    /// several of its entries (`file_size_bytes`, `created_at`,
    /// `updated_at`) exist on `material` too — the ambiguity that made
    /// [`list_duplicate_groups`](Self::list_duplicate_groups) take two
    /// steps instead. [`qualify`] removes it without a second column
    /// list, so one statement suffices here; the join order it needs
    /// is spelled out in [`content_hash_lookup_sql`] and measured by
    /// `the_hash_lookup_is_served_by_the_content_hash_index`, which is
    /// also where the "no new index" decision is recorded.
    ///
    /// The reserved-value guard is Rust-side only; the SQL deliberately
    /// does **not** repeat [`duplicate_key_condition`]. It could not
    /// change an answer: the predicate is equality against a value that
    /// already passed
    /// [`is_duplicate_key`](asterism_core::domain::content_hash::is_duplicate_key),
    /// and a row equal to it passes the same rule. Restating it would
    /// be a third copy of a rule that already exists twice, for rows it
    /// cannot exclude.
    async fn find_by_content_hash(
        &self,
        persona_id: &PersonaId,
        axis: DuplicateAxis,
        digest: &str,
    ) -> Result<Vec<Asset>, DomainError> {
        // Refused at the door rather than answered — an empty result
        // would read as "nobody else holds these bytes" and an honest
        // one would group every fragment in the corpus. See the port
        // doc; the rule is the domain's, not a second list here.
        //
        // Against the caller's axis, which also catches the crossed
        // pair: a `cr1-sha256:` value asked of the artefact axis is not
        // a duplicate key there, so it is refused rather than compared
        // against a column it does not belong to.
        if !asterism_core::domain::content_hash::is_duplicate_key(axis, digest) {
            return Err(DomainError::Validation(format!(
                "the {} axis lookup needs a real digest, got {digest:?}",
                axis.as_str()
            )));
        }
        let pid = *persona_id.as_uuid();
        let hash = digest.to_string();
        let rows: Vec<(AssetRow, Vec<MaterialRow>)> = self
            .isle
            .call(move |conn| {
                let rows: Vec<AssetRow> = {
                    let mut stmt = conn.prepare(&content_hash_lookup_sql(axis))?;
                    stmt.query_map(params![pid, hash], AssetRow::from_row)?
                        .collect::<Result<_, _>>()?
                };
                rows.into_iter()
                    .map(|row| {
                        let materials = MaterialRow::load_for(conn, row.id)?;
                        Ok((row, materials))
                    })
                    .collect::<Result<Vec<_>, rusqlite::Error>>()
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(row, materials)| {
                let mut asset = row.into_domain()?;
                asset.materials = materials
                    .into_iter()
                    .map(MaterialRow::into_domain)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(asset)
            })
            .collect()
    }

    /// One transaction around one [`fold_one`], committed only when the
    /// fold went through.
    ///
    /// A refusal wrote nothing — the marking statement's own predicate
    /// is the guard, so a zero match ends the call before anything else
    /// runs — and dropping the transaction rather than committing it is
    /// what says so at the storage layer too, rather than only in the
    /// return value.
    async fn fold_into(
        &self,
        headstone: &AssetId,
        keeper: &AssetId,
    ) -> Result<FoldOutcome, DomainError> {
        let (head, keep) = (*headstone.as_uuid(), *keeper.as_uuid());
        let now = datetime_to_ms(&Utc::now());
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let outcome = fold_one(&tx, head, keep, now)?;
                if matches!(outcome, FoldOutcome::Folded(_)) {
                    tx.commit()?;
                }
                Ok(outcome)
            })
            .await
            .map_err(infra_err)
    }

    /// The plan's discarded rows, folded in the order the plan lists
    /// them, inside **one** transaction — see the port doc for why the
    /// order is the caller's and why the transaction is one.
    ///
    /// The loop is the whole of the difference from
    /// [`fold_into`](Self::fold_into): every row goes through the same
    /// [`fold_one`], so a keeper that absorbed three rows is three folds
    /// applied in sequence, each one reading the keeper as the previous
    /// one left it.
    ///
    /// `dry_run` and a refusal end the same way — the transaction is
    /// dropped instead of committed — because they *are* the same thing
    /// at the storage layer: a run whose effects are not kept. The
    /// counts differ in nothing, since they come from the statements
    /// that would have been kept rather than from a second estimate.
    async fn merge_into(
        &self,
        plan: &MergePlan,
        dry_run: bool,
    ) -> Result<MergeOutcome, DomainError> {
        let keep = *plan.keeper().as_uuid();
        let discard: Vec<Uuid> = plan.discard().iter().map(|id| *id.as_uuid()).collect();
        // One stamp for the whole set: the folds are one ruling, not a
        // sequence of independent ones (see [`fold_one`]).
        let now = datetime_to_ms(&Utc::now());
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut outcome = MergeOutcome::empty();
                for head in discard {
                    let head_id = AssetId::from_uuid(head);
                    match fold_one(&tx, head, keep, now)? {
                        FoldOutcome::Folded(report) => {
                            outcome.folded.push(head_id);
                            accumulate(&mut outcome.totals, &report);
                        }
                        // `AlreadyFolded` is the one refusal that can be
                        // agreement rather than disagreement: the row
                        // may already be exactly where the plan says it
                        // belongs. Which keeper it went to has to be
                        // read, because the refusal does not carry it —
                        // folded somewhere *else* is somebody's other
                        // ruling and stops everything.
                        FoldOutcome::Skipped(FoldRefusal::AlreadyFolded)
                            if folded_into_of(&tx, head)? == Some(keep) =>
                        {
                            outcome.already_folded.push(head_id);
                        }
                        FoldOutcome::Skipped(refusal) => {
                            outcome.refusals.push((head_id, refusal));
                        }
                    }
                }
                outcome.committed = !dry_run && outcome.refusals.is_empty();
                if outcome.committed {
                    tx.commit()?;
                }
                Ok(outcome)
            })
            .await
            .map_err(infra_err)
    }

    async fn list_duplicate_groups(
        &self,
        persona_id: Option<&PersonaId>,
        axis: DuplicateAxis,
        limit: u32,
    ) -> Result<Vec<DuplicateGroup>, DomainError> {
        let pid = persona_id.map(|p| *p.as_uuid());
        let limit = i64::from(limit);
        // Two steps rather than one join: first find which hashes are
        // shared, then fetch the cards for those hashes. A single query
        // would have to carry every card column through the GROUP BY,
        // and the card projection already knows how to build itself.
        // Which hashes are allowed to group at all is not decided here:
        // [`duplicate_key_condition`] builds that from the domain rule.
        //
        // Both steps exclude headstones, and both have to: a fold is
        // the *answer* to a duplicate, so leaving the folded row in the
        // first step would keep re-reporting a resolved pair (a group
        // of two, one of which no longer exists), and leaving it in the
        // second would list it as a member of some other group that
        // survived. The first is the visible bug; the second is the one
        // that would still be there after fixing only the first.
        let hashes: Vec<String> = self
            .isle
            .call(move |conn| {
                // Which column carries the axis's fingerprint, and
                // which of its values may stand for sameness, are both
                // asked rather than spelled: the markers the content
                // column carries (`unsupported:not-walked` on a row
                // whose original could not be read, `unsupported:<mime>`
                // on every format with no walker) are strings many rows
                // share,
                // so a `GROUP BY` that admitted them would report most
                // of the library as one duplicate set.
                let column = axis_column(axis, "m");
                let hash_filter = duplicate_key_condition(axis, &column);
                let sql = |persona_clause: &str| {
                    format!(
                        "SELECT {column} \
                           FROM material m \
                           JOIN asset ON asset.id = m.asset_id \
                          WHERE m.ord = 0 \
                            AND {hash_filter} \
                            AND asset.trashed_at IS NULL \
                            AND asset.folded_into IS NULL \
                            {persona_clause} \
                          GROUP BY {column} \
                         HAVING COUNT(*) > 1 \
                          ORDER BY MAX(asset.occurred_at) DESC \
                          LIMIT ?1",
                    )
                };
                // Each branch binds exactly the parameters its SQL
                // names: a params list longer than the statement's
                // placeholder count is an `InvalidParameterCount`
                // error, not a silently ignored extra.
                match pid {
                    None => {
                        let mut stmt = conn.prepare(&sql(""))?;
                        stmt.query_map(params![limit], |r| r.get::<_, String>(0))?
                            .collect::<Result<_, _>>()
                    }
                    Some(uuid) => {
                        let mut stmt = conn.prepare(&sql("AND asset.persona_id = ?2"))?;
                        stmt.query_map(params![limit, uuid], |r| r.get::<_, String>(0))?
                            .collect::<Result<_, _>>()
                    }
                }
            })
            .await
            .map_err(infra_err)?;
        if hashes.is_empty() {
            return Ok(Vec::new());
        }

        let pid_for_members = pid;
        let groups: Vec<(String, Vec<CardRow>)> = self
            .isle
            .call(move |conn| {
                let mut out = Vec::with_capacity(hashes.len());
                // EXISTS rather than a JOIN: the card projection selects
                // unqualified column names, several of which
                // (`file_size_bytes`, `created_at`, …) also exist on
                // `material`, so joining the two makes them ambiguous.
                //
                // The membership test reads the **same column** the
                // first step grouped on, from the same builder. Reading
                // the other one here would answer with a group whose key
                // came from one axis and whose members came from the
                // other — plausible enough in shape to pass a glance.
                // The two steps must read the same column, and the way
                // to guarantee that is to ask the same question twice
                // rather than to carry the first answer along.
                let column = axis_column(axis, "m");
                let sql = |persona_clause: &str| {
                    format!(
                        "SELECT {} \
                           FROM asset \
                          WHERE EXISTS (SELECT 1 FROM material m \
                                         WHERE m.asset_id = asset.id AND m.ord = 0 \
                                           AND {column} = ?1) \
                            AND asset.trashed_at IS NULL \
                            AND asset.folded_into IS NULL \
                            {persona_clause} \
                          ORDER BY asset.occurred_at ASC, asset.id ASC",
                        CardRow::columns()
                    )
                };
                // Same binding rule as the first step: bind what the
                // SQL names, no padding.
                match pid_for_members {
                    None => {
                        let mut stmt = conn.prepare(&sql(""))?;
                        for hash in hashes {
                            let rows: Vec<CardRow> = stmt
                                .query_map(params![hash], CardRow::from_row)?
                                .collect::<Result<_, _>>()?;
                            out.push((hash, rows));
                        }
                    }
                    Some(uuid) => {
                        let mut stmt = conn.prepare(&sql("AND asset.persona_id = ?2"))?;
                        for hash in hashes {
                            let rows: Vec<CardRow> = stmt
                                .query_map(params![hash, uuid], CardRow::from_row)?
                                .collect::<Result<_, _>>()?;
                            out.push((hash, rows));
                        }
                    }
                }
                Ok(out)
            })
            .await
            .map_err(infra_err)?;

        groups
            .into_iter()
            // A group can shrink below two between the two queries (a
            // concurrent trash / delete). Reporting a "duplicate" of
            // one is worse than reporting nothing.
            .filter(|(_, rows)| rows.len() > 1)
            .map(|(content_hash, rows)| {
                let members = rows
                    .into_iter()
                    .map(CardRow::into_card)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(DuplicateGroup {
                    // Echoed from the argument that chose the column,
                    // not restated from a literal. This is the only
                    // place that knows which column ran, so a group
                    // cannot claim an axis the statement did not read.
                    axis,
                    content_hash,
                    members,
                })
            })
            .collect()
    }

    async fn list_sessions(&self, query: &AssetQuery) -> Result<Page<Session>, DomainError> {
        // session-model v2: a Session is a composite Asset
        // (`modality = 'session'`) whose members link via
        // `container_id`. The aggregates
        // (message_count / started / ended) are derived at query time
        // from the members — no stored count to drift. Only
        // `persona_id` / `offset` / `limit` from `AssetQuery` are
        // honoured (the composite listing is metadata-driven; the
        // asset-level filters do not translate).
        let limit = query.limit.clamp(1, MAX_LIMIT);
        let offset = query.offset;
        let persona_uuid = query.persona_id.as_ref().map(|p| *p.as_uuid());

        struct SessionAggregateRow {
            id: Uuid,
            persona_id: Uuid,
            external_key: String,
            title: Option<String>,
            note: Option<String>,
            cover_hint: Option<String>,
            started_at_ms: i64,
            ended_at_ms: i64,
            message_count: i64,
            created_at_ms: i64,
            updated_at_ms: i64,
        }

        let (rows, total) = self
            .isle
            .call(move |conn| {
                // Correlated subqueries over container_id derive the
                // member aggregates; COALESCE folds the empty-composite
                // case back to the composite's own occurred_at (seeded
                // run start). Metadata: title↔title, register_note↔note,
                // cover↔cover_hint.
                //
                // The member aggregates count the members that still
                // are ones — [`MEMBER_POPULATION`] — so a composite
                // whose every message is in the trash or folded away
                // reads as `message_count = 0`. The `delete_if_empty`
                // guard in `repo::session` deliberately asks the
                // opposite question ("is anything at all still pointing
                // here?"), which is why that refusal explains itself
                // rather than just saying "not empty".
                //
                // The time window is on the same population as the
                // count, and has to be: a session whose first message
                // was folded into a message of another session would
                // otherwise be dated by a row it no longer holds.
                let base_select = format!(
                    "SELECT \
                        a.id, a.persona_id, a.external_key, a.title, a.register_note, a.cover, \
                        COALESCE((SELECT MIN(m.occurred_at) FROM asset m \
                                  WHERE m.container_id = a.id AND {MEMBER_POPULATION}), \
                                 a.occurred_at) AS started_at_ms, \
                        COALESCE((SELECT MAX(m.occurred_at) FROM asset m \
                                  WHERE m.container_id = a.id AND {MEMBER_POPULATION}), \
                                 a.occurred_at) AS ended_at_ms, \
                        (SELECT COUNT(*) FROM asset m \
                         WHERE m.container_id = a.id AND {MEMBER_POPULATION}) AS message_count, \
                        a.created_at, a.updated_at \
                     FROM asset a"
                );
                // What makes the composite row itself listable, as
                // opposed to what makes a member countable above.
                //
                // Three terms, and the persona one is the term a Group
                // cannot express for itself: a composite carries its own
                // trash stamp, but trashing a *persona* stamps its
                // assets and not its Sessions, so a trashed persona's
                // Sessions would keep their titles here and merely show
                // `message_count = 0` — which reads as "an empty
                // session" rather than "a persona you put in the trash".
                //
                // The fold term is on `a.` rather than through
                // [`MEMBER_POPULATION`] because the subject is different:
                // this is the composite, not a member of one. It is here
                // because `MergePlan::declare` has no `role` check, so a
                // row with `role = 'collection'` can be folded like any
                // other — and a folded Session that stayed in this
                // listing would be a card the grid refuses to show,
                // counted in a total the grid disagrees with. Whether
                // folding a collection should be possible at all is a
                // different question, asked where `declare` is.
                let listable_composite = "a.trashed_at IS NULL \
                     AND a.folded_into IS NULL \
                     AND a.persona_id IN (SELECT id FROM persona WHERE trashed_at IS NULL)";
                let (select_sql, count_sql, params_vec): (String, String, Vec<Value>) =
                    match persona_uuid {
                        Some(pid) => (
                            format!(
                                "{base_select} \
                                 WHERE a.role = 'collection' AND a.persona_id = ?1 \
                                   AND {listable_composite} \
                                 ORDER BY started_at_ms DESC, a.id \
                                 LIMIT ?2 OFFSET ?3"
                            ),
                            format!(
                                "SELECT COUNT(*) FROM asset a \
                                 WHERE a.role = 'collection' AND a.persona_id = ?1 \
                                   AND {listable_composite}"
                            ),
                            vec![Value::Blob(pid.as_bytes().to_vec())],
                        ),
                        None => (
                            format!(
                                "{base_select} \
                                 WHERE a.role = 'collection' AND {listable_composite} \
                                 ORDER BY started_at_ms DESC, a.id \
                                 LIMIT ?1 OFFSET ?2"
                            ),
                            format!(
                                "SELECT COUNT(*) FROM asset a \
                                 WHERE a.role = 'collection' AND {listable_composite}"
                            ),
                            Vec::new(),
                        ),
                    };
                let mut select_params = params_vec.clone();
                select_params.push(Value::Integer(limit as i64));
                select_params.push(Value::Integer(offset as i64));
                let mut stmt = conn.prepare(&select_sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(select_params), |row| {
                        Ok(SessionAggregateRow {
                            id: row.get(0)?,
                            persona_id: row.get(1)?,
                            external_key: row.get(2)?,
                            title: row.get(3)?,
                            note: row.get(4)?,
                            cover_hint: row.get(5)?,
                            started_at_ms: row.get(6)?,
                            ended_at_ms: row.get(7)?,
                            message_count: row.get(8)?,
                            created_at_ms: row.get(9)?,
                            updated_at_ms: row.get(10)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let total: i64 =
                    conn.query_row(&count_sql, rusqlite::params_from_iter(params_vec), |row| {
                        row.get(0)
                    })?;
                Ok((rows, total))
            })
            .await
            .map_err(infra_err)?;

        let items = rows
            .into_iter()
            .map(|r| {
                Ok(Session {
                    id: SessionId::new(r.id.to_string())?,
                    persona_id: PersonaId::from_uuid(r.persona_id),
                    external_key: ExternalSessionKey::new(r.external_key)?,
                    metadata: SessionMetadata {
                        title: r.title,
                        note: r.note,
                        cover_hint: r.cover_hint,
                    },
                    started_at_ms: r.started_at_ms,
                    ended_at_ms: r.ended_at_ms,
                    message_count: u64::try_from(r.message_count.max(0)).unwrap_or(0),
                    created_at_ms: r.created_at_ms,
                    updated_at_ms: r.updated_at_ms,
                })
            })
            .collect::<Result<Vec<_>, DomainError>>()?;
        Ok(Page {
            items,
            offset,
            limit,
            total: Some(total.max(0) as u64),
        })
    }

    /// Hydrates explicitly requested ids — no trash filter, matching
    /// [`find`](Self::find). The caller already holds the id set from an
    /// index read that applied its own trash side, so re-filtering here
    /// would only break the trash view's own hydration.
    async fn cards_by_ids(
        &self,
        ids: &[AssetId],
        viewer: &Viewer,
    ) -> Result<Vec<AssetCard>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut params: Vec<Value> = ids
            .iter()
            .map(|id| Value::Blob(id.as_uuid().as_bytes().to_vec()))
            .collect();
        let placeholders = vec!["?"; ids.len()].join(", ");
        let mut sql = format!(
            "SELECT {} FROM asset WHERE id IN ({placeholders})",
            CardRow::columns()
        );
        if let Viewer::Subject(subject) = viewer {
            sql.push_str(
                " AND (vis_restricted = 0 OR EXISTS \
                 (SELECT 1 FROM json_each(asset.vis_sharing) WHERE json_each.value = ?))",
            );
            params.push(Value::Text(subject.clone()));
        }
        let (rows, group_map) = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), CardRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                let asset_uuids: Vec<Uuid> = rows.iter().map(|r: &CardRow| r.id).collect();
                let group_map = fetch_group_ids_map(conn, &asset_uuids)?;
                Ok((rows, group_map))
            })
            .await
            .map_err(infra_err)?;
        let mut cards: Vec<AssetCard> = rows
            .into_iter()
            .map(CardRow::into_card)
            .collect::<Result<Vec<_>, _>>()?;
        // Hydration by id carries no Group filter, so the `bucket_id`
        // answer stands.
        attach_group_ids(&mut cards, &group_map, None);
        Ok(cards)
    }

    /// One statement, and a walk only for the rows that turned out to
    /// be headstones.
    ///
    /// The statement carries `folded_into IS NOT NULL`, so the ordinary
    /// case — a named id set with nothing folded in it — costs one query
    /// that returns no rows and no walk at all. The rows it does return
    /// are read whole because [`resolve_fold_chain`] takes an
    /// [`AssetRow`]: one walk implementation serves this and
    /// `find_by_source`, and the price of that is paid only by a set
    /// that actually contains a headstone.
    ///
    /// A dead end is reported through [`FoldResolution::report`] and
    /// left out of the map — the port doc's rule, and the same log line
    /// the locator lookup writes for the same state.
    async fn resolve_folds(
        &self,
        ids: &[AssetId],
    ) -> Result<HashMap<AssetId, AssetId>, DomainError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let params: Vec<Value> = ids
            .iter()
            .map(|id| Value::Blob(id.as_uuid().as_bytes().to_vec()))
            .collect();
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT {} FROM asset WHERE id IN ({placeholders}) AND folded_into IS NOT NULL",
            AssetRow::COLUMNS
        );
        self.isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let headstones = stmt
                    .query_map(rusqlite::params_from_iter(params), AssetRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut resolved = HashMap::with_capacity(headstones.len());
                for row in headstones {
                    let Some(keeper) = row.folded_into else {
                        continue;
                    };
                    match resolve_fold_chain(conn, &row, keeper)? {
                        FoldResolution::Resolved(target) => {
                            resolved
                                .insert(AssetId::from_uuid(row.id), AssetId::from_uuid(target.id));
                        }
                        dead_end => dead_end.report(&row),
                    }
                }
                Ok(resolved)
            })
            .await
            .map_err(infra_err)
    }

    async fn index_by_ids(
        &self,
        ids: &[AssetId],
        viewer: &Viewer,
    ) -> Result<Vec<asterism_core::domain::asset::AssetIndex>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut params: Vec<Value> = ids
            .iter()
            .map(|id| Value::Blob(id.as_uuid().as_bytes().to_vec()))
            .collect();
        let placeholders = vec!["?"; ids.len()].join(", ");
        let mut sql = format!(
            "SELECT {} FROM asset WHERE id IN ({placeholders})",
            IndexRow::COLUMNS
        );
        if let Viewer::Subject(subject) = viewer {
            sql.push_str(
                " AND (vis_restricted = 0 OR EXISTS \
                 (SELECT 1 FROM json_each(asset.vis_sharing) WHERE json_each.value = ?))",
            );
            params.push(Value::Text(subject.clone()));
        }
        let (rows, group_map) = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), IndexRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                let asset_uuids: Vec<Uuid> = rows.iter().map(|r: &IndexRow| r.id).collect();
                let group_map = fetch_group_ids_map(conn, &asset_uuids)?;
                Ok((rows, group_map))
            })
            .await
            .map_err(infra_err)?;
        let mut items: Vec<asterism_core::domain::asset::AssetIndex> = rows
            .into_iter()
            .map(IndexRow::into_index)
            .collect::<Result<Vec<_>, _>>()?;
        // No Group filter on a by-id fetch, so `bucket_id` order decides
        // which group is primary — same rule as the card sibling.
        attach_group_ids_index(&mut items, &group_map, None);
        Ok(items)
    }
}

impl SqliteAssetRepository {
    /// Cover text of the container's earliest live member.
    ///
    /// A container owns no material, so `cover_gen` has nothing of its
    /// own to read — its content *is* its members. The earliest one is
    /// what reads as the container's opening line, the same rule a
    /// mail thread uses to take its subject from the first message.
    /// `None` when no member has a cover yet (the member's own
    /// `cover_gen` may still be queued), which leaves the container
    /// uncovered rather than guessing.
    ///
    /// Served by the partial `idx_asset_container` index; the ORDER BY
    /// sorts only that container's rows.
    ///
    /// The table is aliased `m` for one reason: that is the alias
    /// `MEMBER_POPULATION` is written in, and the rule about which
    /// rows are still members is the rule this query needs. The earlier
    /// spelling selected from an unaliased `asset` and asked only for
    /// `trashed_at IS NULL` — the same sentence, one axis short, which
    /// is how a container came to be titled by a message that had been
    /// folded into another one and was no longer inside it.
    pub async fn first_member_cover(
        &self,
        container_id: &AssetId,
    ) -> Result<Option<String>, DomainError> {
        let uuid = *container_id.as_uuid();
        self.isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT m.cover FROM asset m \
                      WHERE m.container_id = ?1 AND {MEMBER_POPULATION} \
                        AND m.cover IS NOT NULL \
                      ORDER BY m.occurred_at ASC LIMIT 1"
                ))?;
                let mut rows = stmt.query(params![uuid])?;
                match rows.next()? {
                    Some(row) => Ok(Some(row.get::<_, String>(0)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(infra_err)
    }

    /// Live containers that still have no cover.
    ///
    /// A container's cover comes from its earliest member, so it can
    /// only be produced after members exist. Ingest re-enqueues the
    /// container's `cover_gen` on every member, which covers everything
    /// imported from here on — this is the backfill for containers that
    /// were already whole before that wiring existed.
    ///
    /// "Live" is two terms, not one. A folded container is not a card
    /// anywhere, so covering it produces nothing anybody can see — and
    /// because a headstone is permanent and its `cover` stays NULL
    /// forever, it would sit at the head of this queue on every pass
    /// and take a slot from a container that would have shown the
    /// result. Not `MEMBER_POPULATION`: the subject here is the
    /// container, not a member of one.
    pub async fn containers_without_cover(&self, limit: u32) -> Result<Vec<AssetId>, DomainError> {
        let rows: Vec<Uuid> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM asset \
                      WHERE role = 'collection' AND cover IS NULL \
                        AND trashed_at IS NULL AND folded_into IS NULL \
                      ORDER BY occurred_at DESC LIMIT ?1",
                )?;
                stmt.query_map(params![limit], |r| r.get::<_, Uuid>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows.into_iter().map(AssetId::from_uuid).collect())
    }

    /// Partial `UPDATE` for the cover column only (used by pipeline
    /// handlers; not part of the domain port).
    ///
    /// Rationale: a full-row upsert would race between concurrent
    /// handlers — a lost update where `auto_tag` overwrites the cover
    /// with NULL was observed in practice. Each handler must therefore
    /// write only the columns it owns.
    pub async fn set_cover(
        &self,
        id: &AssetId,
        cover: &asterism_core::domain::value::CoverText,
    ) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let cover = cover.as_str().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset SET cover = ?2, updated_at = ?3 WHERE id = ?1",
                    params![uuid, cover, now],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// Partial `UPDATE` for the four content-flag columns only (used
    /// by the `cover_gen` handler after it reads the full body). See
    /// `set_cover` for the rationale on partial updates.
    pub async fn set_content_flags(
        &self,
        id: &AssetId,
        flags: ContentFlags,
    ) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let now = chrono::Utc::now().timestamp_millis();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset SET \
                        has_code    = ?2, \
                        has_table   = ?3, \
                        has_mermaid = ?4, \
                        has_link    = ?5, \
                        updated_at  = ?6 \
                     WHERE id = ?1",
                    params![
                        uuid,
                        flags.has_code as i64,
                        flags.has_table as i64,
                        flags.has_mermaid as i64,
                        flags.has_link as i64,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// Partial `UPDATE` for the keywords column only (used by the
    /// auto-tag handler); see `set_cover` for the rationale.
    pub async fn set_keywords(
        &self,
        id: &AssetId,
        keywords: &[asterism_core::domain::value::Keyword],
    ) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let json = strings_to_json(&keywords.iter().map(|k| k.as_str()).collect::<Vec<_>>());
        let now = chrono::Utc::now().timestamp_millis();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE asset SET keywords = ?2, updated_at = ?3 WHERE id = ?1",
                    params![uuid, json, now],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// Candidate fetch for the `edge_rebuild` job (internal infra API,
    /// not part of the domain port).
    ///
    /// Returns up to `limit` assets that occurred within ±48h of the
    /// target **or** share the target's session id. Keyword-overlap
    /// classification is done separately by the domain `plan_edges`.
    /// Backfill scan for the `IndexRebuild` job. Returns
    /// [`BodyCandidate`] rows in id order for assets whose cached body
    /// is **missing or composed by an older reading**, starting after
    /// `cursor` (exclusive). `limit` is clamped by the caller.
    ///
    /// # Two states, not one
    ///
    /// The anti-join alone ("no `asset_body` row") was the whole
    /// predicate until derived text arrived, and it silently became half
    /// of one. A picture has no body and is found by it; a *text* asset
    /// indexed by an older build has a body — the file's bytes — and was
    /// therefore invisible, so its title, keywords, cover and comment
    /// thread never reached its own document. The second disjunct is
    /// that half: `derived_version` below
    /// [`COMPOSITION_VERSION`](asterism_core::domain::derived_text::COMPOSITION_VERSION),
    /// or `NULL` for a row written before the column existed, is a body
    /// composed from less than the asset now says.
    ///
    /// Re-runs still terminate, and for a sharper reason than before:
    /// every row this returns is re-composed and stamped with the
    /// current version, so it leaves the set. Raising the constant puts
    /// the whole library back into it exactly once, which is the point
    /// of having it.
    ///
    /// An asset that derives to nothing keeps no body (the handler drops
    /// it), so it stays in this set and is re-visited by a later full
    /// walk. That is the pre-existing shape of `skipped_no_body` rather
    /// than a new cost: the walk pages forward on `id`, so it is bounded
    /// within a run, and the alternative — a stored "composed to
    /// nothing" row — would be a body cache holding the empty string,
    /// which the port refuses for its own reasons.
    ///
    /// Trashed rows and headstones are both skipped: this scan decides
    /// what enters the full-text index, and a search hit that resolves
    /// to a folded row would hand the user a card the grid does not
    /// have. The single-doc path enforces the same two rules in Rust
    /// (`jobs::handlers::index_rebuild`) because a row can be trashed
    /// or folded after its job was enqueued.
    pub async fn scan_stale_body(
        &self,
        cursor: Option<&AssetId>,
        limit: u32,
    ) -> Result<Vec<BodyCandidate>, DomainError> {
        let cursor_bytes = cursor.map(|id| id.as_uuid().as_bytes().to_vec());
        let limit_i = limit.clamp(1, 5_000) as i64;
        let composed_by = asterism_core::domain::derived_text::COMPOSITION_VERSION;
        let rows = self
            .isle
            .call(move |conn| {
                let (sql, params_vec): (&str, Vec<rusqlite::types::Value>) = match cursor_bytes {
                    Some(c) => (
                        "SELECT asset.id, asset.persona_id, asset.source_locator, \
                                (SELECT mime FROM material \
                                  WHERE material.asset_id = asset.id AND material.ord = 0) \
                         FROM asset \
                         LEFT JOIN asset_body ON asset_body.asset_id = asset.id \
                         WHERE (asset_body.asset_id IS NULL \
                                OR asset_body.derived_version IS NULL \
                                OR asset_body.derived_version < ?) \
                           AND asset.trashed_at IS NULL \
                           AND asset.folded_into IS NULL \
                           AND asset.id > ? \
                         ORDER BY asset.id ASC LIMIT ?",
                        vec![composed_by.into(), c.into(), limit_i.into()],
                    ),
                    None => (
                        "SELECT asset.id, asset.persona_id, asset.source_locator, \
                                (SELECT mime FROM material \
                                  WHERE material.asset_id = asset.id AND material.ord = 0) \
                         FROM asset \
                         LEFT JOIN asset_body ON asset_body.asset_id = asset.id \
                         WHERE (asset_body.asset_id IS NULL \
                                OR asset_body.derived_version IS NULL \
                                OR asset_body.derived_version < ?) \
                           AND asset.trashed_at IS NULL \
                           AND asset.folded_into IS NULL \
                         ORDER BY asset.id ASC LIMIT ?",
                        vec![composed_by.into(), limit_i.into()],
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec
                    .iter()
                    .map(|v| v as &dyn rusqlite::ToSql)
                    .collect();
                let rows = stmt.query_map(params_ref.as_slice(), |row| {
                    let asset_uuid: Uuid = row.get(0)?;
                    let persona_uuid: Uuid = row.get(1)?;
                    let locator: String = row.get(2)?;
                    let mime: Option<String> = row.get(3)?;
                    Ok((asset_uuid, persona_uuid, locator, mime))
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok::<Vec<(Uuid, Uuid, String, Option<String>)>, rusqlite::Error>(out)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(a, p, locator, mime)| {
                Ok(BodyCandidate {
                    asset_id: AssetId::from_uuid(a),
                    persona_id: PersonaId::from_uuid(p),
                    // Same read boundary as the entity path: this scan
                    // feeds `TextLocator`, and the reader behind it
                    // needs the variant, not the spelling.
                    locator: SourceLocator::try_from(locator.as_str())?,
                    // Parsed at the boundary like every other mime this
                    // adapter hands out.
                    mime: mime.as_deref().map(MimeType::parse),
                })
            })
            .collect()
    }

    /// Fetches assets in the ±48 h time window around `target` (or
    /// sharing its bundle id), up to `limit`. Used by `edge_rebuild`
    /// as its candidate set for `plan_edges`.
    ///
    /// The grouping-key axis switched from `session_id` to
    /// `bundle_id` when the Session model became a Dialog-only
    /// 1st-class entity: `session_id` now points at a specific
    /// conversation and would no longer surface the tape / journal /
    /// PNG-note siblings the
    /// edge fabric needs. `bundle_id` is modality-agnostic and
    /// carries the pre-existing grouping semantics verbatim.
    ///
    /// A headstone is excluded on the same footing as a trashed row:
    /// what comes back here becomes a card in the constellation around
    /// the target, and a fold already said this row is the keeper's
    /// content rather than a neighbour of it. Left in, it would draw a
    /// second card for a picture already on screen — the exact
    /// duplicate somebody resolved.
    pub async fn candidates_near(
        &self,
        target: &Asset,
        limit: u32,
    ) -> Result<Vec<Asset>, DomainError> {
        let target_id = *target.id.as_uuid();
        let center_ms = crate::sqlite::map::datetime_to_ms(&target.occurred_at);
        let window_ms: i64 = 48 * 3_600_000;
        let bundle_id = target
            .bundle_id
            .as_ref()
            .map(|b| b.as_str().to_string())
            .unwrap_or_default();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM asset
                     WHERE id != ?1
                       AND trashed_at IS NULL
                       AND folded_into IS NULL
                       AND (ABS(occurred_at - ?2) < ?3
                            OR (?4 != '' AND bundle_id = ?4))
                     ORDER BY ABS(occurred_at - ?2)
                     LIMIT ?5",
                    AssetRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(
                        params![target_id, center_ms, window_ms, bundle_id, limit as i64],
                        AssetRow::from_row,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        // Materials stay unhydrated on this batch path — edge planning
        // reads time / bundle only, and `save` never deletes materials,
        // so an unhydrated entity is round-trip safe.
        rows.into_iter().map(AssetRow::into_domain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::asset::{Asset, AssetQuery};
    use asterism_core::domain::attribution::{AttributionChannel, AttributionContext};
    use asterism_core::domain::content_hash;
    use asterism_core::domain::content_hash::{EMPTY, UNHASHABLE};
    use asterism_core::domain::repository::AssetRepository;
    use asterism_core::domain::value::{Modality, PersonaId, SourceKind, SourceRef, TagId};
    use rusqlite::params;

    /// The unrecorded context, spelled the way a crate outside
    /// `asterism-core` has to spell it: `unrecorded()` is crate-private
    /// there on purpose, and an empty assertion is defined to be the
    /// same value (attribution rule 3). Most fixtures here are about
    /// some other axis and record nobody.
    /// Parses the spelling a caller sends into the locator, the way the
    /// ingest boundary does. What the *column* holds is the tagged form,
    /// which `to_storage` writes and `try_from` reads; a fixture that
    /// wants to see that text reads the column raw.
    fn loc(raw: impl AsRef<str>) -> SourceLocator {
        SourceLocator::from_wire(raw.as_ref()).expect("locator")
    }

    fn nobody() -> AttributionContext {
        AttributionContext::asserted(None, None).unwrap()
    }

    /// A fingerprint whose file axis is `file` and whose two walking
    /// axes are answered by markers — the shape a material that is not a
    /// walkable picture ends up with.
    ///
    /// For the tests below that are about the file axis: they still have
    /// to write every column, because the verb writes every column, and
    /// a test that left one NULL would leave its row in the fingerprint
    /// walk and quietly change what the *other* assertions in the same
    /// test are measuring.
    fn file_axis(file: &str) -> MaterialFingerprint {
        MaterialFingerprint {
            file: file.to_string(),
            content: asterism_core::domain::content_region::unsupported_format(None).stored_value(),
            meta: asterism_core::domain::material_meta::unsupported_format(None).stored_value(),
            meta_kv: None,
            meta_raw: None,
            meta_text: None,
        }
    }

    /// The file and content columns spelled out — for the tests that are
    /// about the difference between those two. The meta column takes the
    /// same marker `file_axis` gives it, since nothing here is asking
    /// about it.
    fn both_axes(file: &str, content: &str) -> MaterialFingerprint {
        MaterialFingerprint {
            file: file.to_string(),
            content: content.to_string(),
            ..file_axis(file)
        }
    }

    /// The meta column spelled out, the other two answered by markers —
    /// the mirror of `both_axes` for the third axis.
    fn meta_axis(file: &str, meta: &str, meta_kv: Option<&str>) -> MaterialFingerprint {
        MaterialFingerprint {
            meta: meta.to_string(),
            meta_kv: meta_kv.map(str::to_string),
            ..file_axis(file)
        }
    }

    /// Seeds one persona. `pack_id` is `UNIQUE`, so it is derived from the
    /// id — tests that need two personas can call this repeatedly.
    async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
        let pid = Uuid::now_v7();
        let pack = format!("pack-{pid}");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, 'P', 0, 0)",
                params![pid, pack],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        PersonaId::from_uuid(pid)
    }

    /// **Two rows may carry one `external_key`** — the write the old
    /// UNIQUE refused, through the production `save` path.
    ///
    /// Both rows sit in one persona and state one key, because that is
    /// the case the index acted on: an external record signed once,
    /// updated, and ingested again states the same key both times. The
    /// key also survives `save` → `find`, which is what makes the first
    /// half an assertion about a stored value rather than about a write
    /// that happened to return `Ok`.
    #[tokio::test]
    async fn two_rows_in_one_persona_may_carry_one_external_key() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let stated = "issue-12345";
        let mut first = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/pics/signed.png").unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        first.external_key = Some(stated.into());
        repo.save(&first).await.unwrap();

        // The same record, updated upstream and ingested again. A
        // different artefact, the same source-stated name for it.
        let mut again = Asset::new(
            persona,
            SourceRef::new(
                SourceKind::new(SourceKind::FS).unwrap(),
                "/pics/updated.png",
            )
            .unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        again.external_key = Some(stated.into());
        repo.save(&again)
            .await
            .expect("an external record legitimately arrives more than once");

        assert_eq!(
            repo.find(&first.id)
                .await
                .unwrap()
                .unwrap()
                .external_key
                .as_deref(),
            Some(stated),
            "the key has to round-trip, or the write above proved nothing about the column"
        );
        assert_eq!(
            repo.find(&again.id)
                .await
                .unwrap()
                .unwrap()
                .external_key
                .as_deref(),
            Some(stated)
        );

        driver.shutdown().await.unwrap();
    }

    /// **Two platforms may number a record alike.**
    ///
    /// This is the half the old index could not have got right even in
    /// principle: its key was `(persona_id, external_key)` and carried
    /// no source discriminator, so one platform's `12345` refused
    /// another's. The fixture disagrees with the default on exactly that
    /// axis — same key, different `source_kind` — and the rows are also
    /// read back to show that both survived rather than one overwriting
    /// the other.
    #[tokio::test]
    async fn two_source_kinds_may_number_a_record_alike() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let collided = "12345";
        let mut from_a = Asset::new(
            persona,
            SourceRef::new(SourceKind::new("gitea").unwrap(), "/tickets/a-12345.json").unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        from_a.external_key = Some(collided.into());
        repo.save(&from_a).await.unwrap();

        let mut from_b = Asset::new(
            persona,
            SourceRef::new(SourceKind::new("linear").unwrap(), "/tickets/b-12345.json").unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        from_b.external_key = Some(collided.into());
        repo.save(&from_b)
            .await
            .expect("two unrelated sources numbering their records `12345` is ordinary");

        assert_ne!(from_a.id, from_b.id, "two records, two identities");
        let a = repo.find(&from_a.id).await.unwrap().unwrap();
        let b = repo.find(&from_b.id).await.unwrap().unwrap();
        assert_eq!(a.external_key.as_deref(), Some(collided));
        assert_eq!(b.external_key.as_deref(), Some(collided));
        assert_ne!(
            a.source.kind.as_str(),
            b.source.kind.as_str(),
            "the axis under test is the one the old key had no column for"
        );

        driver.shutdown().await.unwrap();
    }

    /// Guards the Cycle-4 persistence wiring: `container_id` / `title`
    /// must survive `save` → `find` (positional COLUMNS/from_row mapping,
    /// upsert arity, BLOB symmetry), and the `AssetQuery.container_id`
    /// drill must return exactly the composite's members.
    #[tokio::test]
    async fn container_id_and_title_round_trip_and_drill() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        // A composite Asset (title set, top-level) …
        let mut composite = Asset::new(
            persona,
            SourceRef::new(SourceKind::new("session").unwrap(), "sess-loc").unwrap(),
            // Containers are unclassified (asset-model v4).
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        composite.title = Some("My chat".into());
        repo.save(&composite).await.unwrap();

        // … and a member pointing at it via container_id.
        let mut member = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "msg-1.md").unwrap(),
            // Conversation members are unclassified too — membership
            // is what `container_id` says, not a slug.
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        member.container_id = Some(composite.id);
        repo.save(&member).await.unwrap();

        let loaded_composite = repo.find(&composite.id).await.unwrap().unwrap();
        assert_eq!(loaded_composite.title.as_deref(), Some("My chat"));
        assert_eq!(
            loaded_composite.container_id, None,
            "composite is top-level"
        );

        let loaded_member = repo.find(&member.id).await.unwrap().unwrap();
        assert_eq!(
            loaded_member.container_id,
            Some(composite.id),
            "member's container_id round-trips as the composite's BLOB id"
        );
        assert_eq!(loaded_member.title, None);

        // Composition drill returns the member, not the composite.
        let page = repo
            .list(&AssetQuery {
                container_id: Some(composite.id),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1, "drill returns the single member");
        assert_eq!(page.items[0].id, member.id);

        driver.shutdown().await.unwrap();
    }

    /// Attribution survives `save` → `find` on both halves of the
    /// author pair, and an asset nobody asserted one for reads back
    /// unrecorded — the distinction the columns exist to keep.
    ///
    /// Guards the same wiring `container_id` does (positional
    /// COLUMNS / `from_row` indices, upsert arity) for the three
    /// columns V47 appended.
    #[tokio::test]
    async fn author_and_operator_round_trip_and_absence_stays_absent() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let new_asset = |locator: &str, attribution: &AttributionContext| {
            Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                Utc::now(),
                attribution,
            )
        };

        // A named subject, operated through an agent, over a channel
        // that says the pair was stated rather than established.
        let asserted = new_asset(
            "attributed.md",
            &AttributionContext::asserted(
                Some(Author::Subject("alice".into())),
                Some(OperatorRef::new("claude-code").unwrap()),
            )
            .unwrap(),
        );
        repo.save(&asserted).await.unwrap();

        // The owner half of the pair: kind without a subject, and the
        // only channel that can carry it.
        let owned = new_asset("owned.md", &AttributionContext::owner_surface());
        repo.save(&owned).await.unwrap();

        // Nobody asserted anything — what a caller that stated no
        // author, and every row written before V47, reads back as.
        let silent = new_asset("silent.md", &nobody());
        repo.save(&silent).await.unwrap();

        let loaded = repo.find(&asserted.id).await.unwrap().unwrap();
        assert_eq!(loaded.author(), Some(&Author::Subject("alice".into())));
        assert_eq!(
            loaded.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(
            loaded.attributed_via(),
            Some(AttributionChannel::Asserted),
            "how the attribution arrived is part of what was stored"
        );

        let loaded_owner = repo.find(&owned.id).await.unwrap().unwrap();
        assert_eq!(loaded_owner.author(), Some(&Author::Owner));
        assert_eq!(
            loaded_owner.operator_ai(),
            None,
            "an author says nothing about which agent typed it"
        );
        assert_eq!(
            loaded_owner.attributed_via(),
            Some(AttributionChannel::OwnerSurface)
        );

        let loaded_silent = repo.find(&silent.id).await.unwrap().unwrap();
        assert_eq!(
            (
                loaded_silent.author().cloned(),
                loaded_silent.operator_ai().cloned(),
                loaded_silent.attributed_via()
            ),
            (None, None, None),
            "unrecorded round-trips as unrecorded, not as the owner"
        );

        // Re-saving a hydrated entity keeps the assertion (full-row
        // upsert: the entity owns these columns).
        repo.save(&loaded).await.unwrap();
        let again = repo.find(&asserted.id).await.unwrap().unwrap();
        assert_eq!(again.author(), Some(&Author::Subject("alice".into())));
        assert_eq!(
            again.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(again.attributed_via(), Some(AttributionChannel::Asserted));

        driver.shutdown().await.unwrap();
    }

    /// A V47-era row — an author with no channel — reads back as what it
    /// is, and cannot be written anew.
    ///
    /// Inserted with raw SQL because that shape is exactly what the
    /// entity path now refuses to produce: there is no
    /// `AttributionContext` that yields an author without a channel, so
    /// the legacy row has to be placed the way the old build placed it.
    #[tokio::test]
    async fn a_v47_era_row_reads_back_and_cannot_be_minted_again() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let legacy = Uuid::now_v7();
        let persona_uuid = *persona.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, labels, \
                                    occurred_at, keywords, vis_restricted, vis_sharing, \
                                    created_at, updated_at, role, author_kind, operator_ai) \
                 VALUES (?1, ?2, 'fs', ?3, '[]', 0, '[]', 0, '[]', 0, 0, 'item', \
                         'owner', 'claude-code')",
                params![
                    legacy,
                    persona_uuid,
                    crate::sqlite::stored_locator("legacy.md")
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let loaded = repo
            .find(&AssetId::from_uuid(legacy))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.author(), Some(&Author::Owner));
        assert_eq!(
            loaded.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(
            loaded.attributed_via(),
            None,
            "the read accepts the legacy shape instead of guessing the channel it never had"
        );

        // Saving it back is the shortest path to minting a fresh row of
        // that shape, and the guard is what stops it — a new row in the
        // legacy bucket would be indistinguishable from one that
        // predates the column, which is the set an authenticated
        // deployment cannot resolve.
        let err = repo
            .save(&loaded)
            .await
            .expect_err("an author with no channel cannot be written anew");
        assert!(
            err.to_string().contains("attribution without a channel"),
            "the refusal should name the rule it enforces: {err}"
        );

        driver.shutdown().await.unwrap();
    }

    /// The card projection carries the same attribution the entity
    /// does, on both of the paths that build a `CardRow` — the listing
    /// (`list`) and the viewport hydration (`cards_by_ids`).
    ///
    /// `CardRow` keeps its own `COLUMNS` string and its own positional
    /// `from_row`, so the entity round-trip above says nothing about
    /// this one: a column appended to the entity list and forgotten
    /// here leaves the grid reading `None` for every row while the
    /// detail view shows the author. The absent case is asserted too,
    /// because an off-by-one in the positional mapping is just as
    /// capable of inventing an author as of dropping one.
    #[tokio::test]
    async fn cards_carry_attribution_and_keep_absence_absent() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let attributed = Asset::new(
            persona,
            SourceRef::new(
                SourceKind::new(SourceKind::FS).unwrap(),
                "card-attributed.md",
            )
            .unwrap(),
            None,
            Utc::now(),
            &AttributionContext::asserted(
                Some(Author::Subject("alice".into())),
                Some(OperatorRef::new("claude-code").unwrap()),
            )
            .unwrap(),
        );
        repo.save(&attributed).await.unwrap();

        let silent = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "card-silent.md").unwrap(),
            None,
            Utc::now(),
            &nobody(),
        );
        repo.save(&silent).await.unwrap();

        let assert_pair = |cards: &[AssetCard], via: &str| {
            let hit = cards
                .iter()
                .find(|c| c.id == attributed.id)
                .unwrap_or_else(|| panic!("{via}: attributed card missing"));
            assert_eq!(
                hit.author,
                Some(Author::Subject("alice".into())),
                "{via}: the card must name the same subject the entity does"
            );
            assert_eq!(
                hit.operator_ai.as_ref().map(OperatorRef::as_str),
                Some("claude-code"),
                "{via}: operator rides the card, not just the entity"
            );

            let quiet = cards
                .iter()
                .find(|c| c.id == silent.id)
                .unwrap_or_else(|| panic!("{via}: silent card missing"));
            assert_eq!(
                (quiet.author.clone(), quiet.operator_ai.clone()),
                (None, None),
                "{via}: unrecorded stays unrecorded on the card too"
            );
        };

        let page = repo
            .list(&AssetQuery {
                persona_id: Some(persona),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_pair(&page.items, "list");

        let hydrated = repo
            .cards_by_ids(&[attributed.id, silent.id], &Viewer::Owner)
            .await
            .unwrap();
        assert_pair(&hydrated, "cards_by_ids");

        driver.shutdown().await.unwrap();
    }

    /// One Asset is one Card, and every facet counts that same set.
    ///
    /// Two habits produced disagreement before this held. The listing
    /// hid members behind a `top_level` flag, so a row could exist in
    /// the data and nowhere in the UI. And each facet carried its own
    /// idea of the population (PERSONA every row, MODALITY the
    /// classified ones, FORMAT the ones with a known mime), so no two
    /// "all" numbers matched — 282 / 237 / 264 against a grid of 268.
    #[tokio::test]
    async fn every_asset_is_a_card_and_the_facets_agree() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        // A container. Two axes, both set: `role` is the structure,
        // `modality` is what it holds — the same pair `SessionRepository`
        // writes when it mints one.
        let mut container = Asset::new(
            persona,
            SourceRef::new(SourceKind::new("session").unwrap(), "sess-role").unwrap(),
            Some(Modality::new("session").unwrap()),
            Utc::now(),
            &nobody(),
        );
        container.role = AssetRole::Collection;
        repo.save(&container).await.unwrap();

        // … one member inside it. It carries a modality of its own:
        // being filed inside something is containment, not a reason to
        // go unclassified.
        let mut member = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "msg-role.md").unwrap(),
            Some(Modality::new("message").unwrap()),
            Utc::now(),
            &nobody(),
        );
        member.container_id = Some(container.id);
        repo.save(&member).await.unwrap();

        // … and one plain top-level item.
        let item = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/pics/role.png").unwrap(),
            Some(Modality::new("tape").unwrap()),
            Utc::now(),
            &nobody(),
        );
        repo.save(&item).await.unwrap();

        // All three are cards. Nothing is held back for being filed
        // inside something.
        let grid = repo.list(&AssetQuery::default()).await.unwrap();
        assert_eq!(grid.items.len(), 3, "container, member and item");
        let container_card = grid
            .items
            .iter()
            .find(|c| c.id == container.id)
            .expect("the container is a card like any other");
        assert_eq!(container_card.role, AssetRole::Collection);
        assert_eq!(
            container_card.member_count, 1,
            "a container's headline number is what is inside it"
        );
        let item_card = grid.items.iter().find(|c| c.id == item.id).unwrap();
        assert_eq!(item_card.member_count, 0, "nothing is inside an item");

        // Every card lands in some PERSONA row.
        let by_persona = repo.counts_by_persona(TrashFilter::LiveOnly).await.unwrap();
        assert_eq!(by_persona, vec![(persona, 3)]);

        // …and in some MODALITY row. Categories cover the whole set
        // because every Asset carries one, which is the property that
        // makes "browse by category" total rather than best-effort.
        let by_modality = repo
            .counts_by_modality(Some(&persona), TrashFilter::LiveOnly)
            .await
            .unwrap();
        assert_eq!(
            by_modality.iter().map(|(_, c)| c).sum::<u64>(),
            3,
            "the facet accounts for every card"
        );
        assert!(
            by_modality
                .iter()
                .all(|(slug, _)| slug != UNCLASSIFIED_MODALITY),
            "nothing falls into the unclassified bucket here"
        );

        // Drilling into a container still returns exactly its members —
        // containment is a filter callers ask for, not a default the
        // listing applies behind their back.
        let inside = repo
            .list(&AssetQuery {
                container_id: Some(container.id),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            inside.items.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![member.id],
        );

        driver.shutdown().await.unwrap();
    }

    /// Guards the container cover path: a container owns no material,
    /// so `cover_gen` reads its earliest member instead. Ordering is
    /// the point — the container should read as its opening line, not
    /// whichever member happened to be ingested first.
    #[tokio::test]
    async fn first_member_cover_takes_the_earliest_covered_member() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut container = Asset::new(
            persona,
            SourceRef::new(SourceKind::new("session").unwrap(), "sess-cover").unwrap(),
            None,
            Utc::now(),
            &nobody(),
        );
        container.role = AssetRole::Collection;
        repo.save(&container).await.unwrap();

        assert_eq!(
            repo.first_member_cover(&container.id).await.unwrap(),
            None,
            "an empty container stays uncovered rather than guessing"
        );

        // Ingested newest-first on purpose: the answer must come from
        // `occurred_at`, not from insertion order.
        let base = Utc::now();
        for (offset_secs, locator, cover) in [
            (120_i64, "msg-late.md", "the later reply"),
            (0, "msg-early.md", "the opening line"),
        ] {
            let mut member = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                base + chrono::Duration::seconds(offset_secs),
                &nobody(),
            );
            member.container_id = Some(container.id);
            repo.save(&member).await.unwrap();
            repo.set_cover(&member.id, &CoverText::new(cover.to_string()).unwrap())
                .await
                .unwrap();
        }

        assert_eq!(
            repo.first_member_cover(&container.id).await.unwrap(),
            Some("the opening line".to_string()),
        );

        driver.shutdown().await.unwrap();
    }

    /// Guards the v4 material wiring: `role` and the primary material
    /// must survive `save` → `find` (hydration), and a metadata
    /// round-trip of an entity whose materials were **not** hydrated
    /// (batch read paths) must not wipe the material rows — `save`
    /// upserts materials, it never deletes them.
    #[tokio::test]
    async fn role_and_materials_round_trip_without_wiping() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/pics/a.png").unwrap(),
            Some(Modality::new("image").unwrap()),
            Utc::now(),
            &nobody(),
        );
        asset
            .attach_material(Material::primary(
                asset.source.locator.clone(),
                Some(42),
                asset.created_at,
            ))
            .unwrap();
        repo.save(&asset).await.unwrap();

        let found = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(found.role, AssetRole::Item);
        assert_eq!(found.materials.len(), 1);
        assert_eq!(found.materials[0].locator, loc("/pics/a.png"));
        // Round-trips as the parsed form: written as its token, read
        // back through `MimeType::parse`.
        assert_eq!(found.materials[0].mime, Some(MimeType::parse("image/png")));
        assert_eq!(found.materials[0].file_size_bytes, Some(42));

        // A round-trip through an unhydrated entity (materials empty)
        // must leave the physical layer untouched.
        let mut unhydrated = found.clone();
        unhydrated.materials.clear();
        unhydrated.title = Some("named later".into());
        repo.save(&unhydrated).await.unwrap();

        let again = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(again.title.as_deref(), Some("named later"));
        assert_eq!(again.materials.len(), 1, "materials survive the round-trip");

        driver.shutdown().await.unwrap();
    }

    /// **What `save` writes is a canonical rendering, not the string
    /// that was read.**
    ///
    /// `file:///pics/…` parses to the path it names — the scheme is
    /// consumed on purpose, so the two spellings are one locator — and
    /// `to_storage()` renders that path into the tagged form. So a row
    /// registered with the scheme lands in the column as
    /// `{"kind":"file","path":"/pics/schemed.png"}`, and an ordinary
    /// read-modify-write of a row that already held the scheme rewrites
    /// its `source_locator` to that.
    ///
    /// Asserted here rather than discovered from a failed upsert. The
    /// consequence is that two rows spelled the two ways of one path
    /// merge onto one value — which is exactly what `N : 1` permits
    /// since V61 demoted the Source pair from a UNIQUE to a lookup, and
    /// the reason the storage-form rewrite is ordered *after* that
    /// demotion. The second half of this test used to assert the
    /// refusal; it asserts the merge, because that refusal is the thing
    /// that went.
    #[tokio::test]
    async fn a_file_scheme_spelling_is_stored_as_the_path_it_names() {
        use crate::sqlite::open_and_migrate_in_memory;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let kind = SourceKind::new(SourceKind::FS).unwrap();

        let mut asset = Asset::new(
            persona,
            SourceRef::new(kind.clone(), "file:///pics/schemed.png").unwrap(),
            None,
            Utc::now(),
            &nobody(),
        );
        asset
            .attach_material(Material::primary(
                asset.source.locator.clone(),
                Some(1),
                asset.created_at,
            ))
            .unwrap();
        repo.save(&asset).await.unwrap();

        // The column, read raw — no parse in between to launder it.
        let id = *asset.id.as_uuid();
        let (stored_asset, stored_material): (String, String) = isle
            .call(move |conn| {
                let a = conn.query_row(
                    "SELECT source_locator FROM asset WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )?;
                let m = conn.query_row(
                    "SELECT locator FROM material WHERE asset_id = ?1 AND ord = 0",
                    params![id],
                    |r| r.get(0),
                )?;
                Ok((a, m))
            })
            .await
            .unwrap();
        assert_eq!(
            stored_asset, r#"{"kind":"file","path":"/pics/schemed.png"}"#,
            "the scheme is consumed at the boundary, so the column holds the path it named — \
             tagged, with the `file` scheme nowhere in it"
        );
        assert_eq!(
            stored_material, stored_asset,
            "and the second column with this encoding agrees, byte for byte"
        );

        // Which means the bare spelling finds the row registered with
        // the scheme — the equality the consumption exists for.
        let found = repo
            .find_by_source(
                &persona,
                &kind,
                &loc("/pics/schemed.png"),
                SourceLookupScope::Live,
            )
            .await
            .unwrap()
            .expect("one locator, either spelling");
        assert_eq!(found.id, asset.id);

        // And a second row spelled the other way is registered rather
        // than refused: the two spellings are one Source value, and one
        // Source value may be carried by many Assets.
        let twin = Asset::new(
            persona,
            SourceRef::new(kind.clone(), "/pics/schemed.png").unwrap(),
            None,
            Utc::now(),
            &nobody(),
        );
        repo.save(&twin)
            .await
            .expect("`N : 1` — the storage layer does not adjudicate sameness");

        // Found by equality on the column, which is what the ingest
        // lookup does — so this also says the two spellings reached it
        // as one string rather than as two that happen to mean the same.
        let both: Vec<Uuid> = isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM asset WHERE source_locator = ?1 ORDER BY created_at, id",
                )?;
                stmt.query_map(params![stored_asset], |r| r.get(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        assert_eq!(
            both.len(),
            2,
            "both rows stand at the one address the two spellings name"
        );
        // …and the lookup answers with the row that was already there,
        // which is what makes a re-arrival idempotent rather than
        // whichever row the planner reached first.
        let held = repo
            .find_by_source(
                &persona,
                &kind,
                &loc("file:///pics/schemed.png"),
                SourceLookupScope::Live,
            )
            .await
            .unwrap()
            .expect("the value is held");
        assert_eq!(held.id, asset.id, "the earliest holder is the answer");

        driver.shutdown().await.unwrap();
    }

    /// The trash is the caller's question, not the lookup's rule.
    ///
    /// This test used to say the opposite — that a trashed row holds a
    /// locator "just as firmly as a live one", so the ingest path could
    /// tell "already imported" from "in the trash, restore it". Both
    /// halves were true of a UNIQUE, and both went with it. The gate on
    /// minting is "is this record **here**", and a record in the trash
    /// is not here; so `Live` passes over it and a re-import mints,
    /// which is what the person importing asked for. `Any` still
    /// answers, for the readers whose question really is "who is
    /// holding this address" — a diagnostic, not a decision.
    #[tokio::test]
    async fn the_trash_is_invisible_to_the_ingest_lookup_and_visible_to_any() {
        use crate::sqlite::open_and_migrate_in_memory;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let kind = SourceKind::new(SourceKind::FS).unwrap();
        let asset = Asset::new(
            persona,
            SourceRef::new(kind.clone(), "notes/held.md").unwrap(),
            Some(Modality::new(Modality::STATE).unwrap()),
            Utc::now(),
            &nobody(),
        );
        repo.save(&asset).await.unwrap();

        // While it is live both scopes agree — which is what makes the
        // disagreement below a fact about the trash rather than about
        // the two scopes being different queries.
        for scope in [SourceLookupScope::Live, SourceLookupScope::Any] {
            let found = repo
                .find_by_source(&persona, &kind, &loc("notes/held.md"), scope)
                .await
                .unwrap();
            assert_eq!(found.map(|a| a.id), Some(asset.id), "{scope:?}");
            assert!(
                repo.find_by_source(&persona, &kind, &loc("notes/absent.md"), scope)
                    .await
                    .unwrap()
                    .is_none(),
                "{scope:?}"
            );
        }

        repo.trash(&asset.id, Utc::now()).await.unwrap();
        assert!(
            repo.find_by_source(
                &persona,
                &kind,
                &loc("notes/held.md"),
                SourceLookupScope::Live
            )
            .await
            .unwrap()
            .is_none(),
            "a record in the trash is not here, so the ingest lookup does not see it"
        );
        let after = repo
            .find_by_source(
                &persona,
                &kind,
                &loc("notes/held.md"),
                SourceLookupScope::Any,
            )
            .await
            .unwrap()
            .expect("the row is still the one standing at that address");
        assert!(
            after.trashed_at.is_some(),
            "…and says where it is, for the reader that asked about storage"
        );

        // Another persona's row is another persona's business: the same
        // address under a different owner is not held here.
        let other = seed_persona(&isle).await;
        assert!(
            repo.find_by_source(&other, &kind, &loc("notes/held.md"), SourceLookupScope::Any)
                .await
                .unwrap()
                .is_none(),
            "the persona is part of the question"
        );

        driver.shutdown().await.unwrap();
    }

    /// Regression: the search read path used to honour `persona_id` and
    /// drop every other filter, so a lit modality / tag chip silently
    /// stopped constraining a text search while the *same* filter still
    /// constrained the Query Group evaluator. `filter_ids` is the SQL
    /// half that closes that gap — it must apply the full shared
    /// predicate set to the candidate ids handed over by Tantivy.
    #[tokio::test]
    async fn filter_ids_applies_the_full_filter_surface() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let other_persona = seed_persona(&isle).await;

        let mk = |owner: PersonaId, modality: &str, locator: &str| {
            Asset::new(
                owner,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                Some(Modality::new(modality).unwrap()),
                chrono::Utc::now(),
                &nobody(),
            )
        };
        // `image` is an open slug (no associated constant) — the modality
        // value space is deliberately not an enum.
        let image = mk(persona, "image", "a.png");
        let dialogue = mk(persona, Modality::STATE, "b.md");
        let foreign = mk(other_persona, "image", "c.png");
        for asset in [&image, &dialogue, &foreign] {
            repo.save(asset).await.unwrap();
        }

        // Tag only the image, so the tag filter and the modality filter
        // select different-sized subsets of the same candidate list.
        let tag_id = Uuid::now_v7();
        let tagged = *image.id.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO tag (id, name, axis) VALUES (?1, 'sky', NULL)",
                params![tag_id],
            )?;
            conn.execute(
                "INSERT INTO asset_tag (asset_id, tag_id) VALUES (?1, ?2)",
                params![tagged, tag_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        // The candidate set stands in for a Tantivy hit list: every
        // asset matched the text, so only the SQL filter can narrow it.
        let candidates = vec![image.id, dialogue.id, foreign.id];
        let sorted = |mut ids: Vec<AssetId>| {
            ids.sort_by_key(|id| *id.as_uuid());
            ids
        };

        // No filter → pass-through (must not silently drop candidates).
        let kept = repo
            .filter_ids(&candidates, &AssetQuery::default())
            .await
            .unwrap();
        assert_eq!(sorted(kept), sorted(candidates.clone()), "unfiltered");

        // Modality — the chip that used to be ignored entirely.
        let kept = repo
            .filter_ids(
                &candidates,
                &AssetQuery {
                    modality: Some(Modality::new("image").unwrap()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(sorted(kept), sorted(vec![image.id, foreign.id]), "modality");

        // Tag (multi-select OR against `asset_tag`).
        let kept = repo
            .filter_ids(
                &candidates,
                &AssetQuery {
                    tag_ids: vec![TagId::from_uuid(tag_id)],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(kept, vec![image.id], "tag");

        // Filters compose (AND across axes), and persona still applies.
        let kept = repo
            .filter_ids(
                &candidates,
                &AssetQuery {
                    persona_id: Some(persona),
                    modality: Some(Modality::new("image").unwrap()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(kept, vec![image.id], "persona AND modality");

        // A candidate that fails the filter yields an empty result
        // rather than falling back to the unfiltered set.
        let kept = repo
            .filter_ids(
                &[dialogue.id],
                &AssetQuery {
                    modality: Some(Modality::new("image").unwrap()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(kept.is_empty(), "no candidate survives");

        // Empty candidate list short-circuits without touching SQL.
        let kept = repo.filter_ids(&[], &AssetQuery::default()).await.unwrap();
        assert!(kept.is_empty(), "empty candidates");

        driver.shutdown().await.unwrap();
    }

    /// Trashing a persona takes its live assets with it and returns
    /// exactly those ids (the caller drops their search documents).
    /// Restoring puts back only that set — an asset the user had already
    /// thrown away stays thrown away, which is the whole reason the
    /// operation keys on a shared stamp instead of "everything under
    /// this persona".
    #[tokio::test]
    async fn persona_trash_and_restore_move_exactly_their_own_assets() {
        use crate::sqlite::open_and_migrate_in_memory;

        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut with_persona = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "a.md").unwrap(),
            Some(Modality::new(Modality::STATE).unwrap()),
            Utc::now(),
            &nobody(),
        );
        let mut thrown_away = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "b.md").unwrap(),
            Some(Modality::new(Modality::STATE).unwrap()),
            Utc::now(),
            &nobody(),
        );
        // Deliberately different stamps: the hand-trashed asset went to
        // the trash earlier, on its own.
        thrown_away.trashed_at = DateTime::from_timestamp_millis(500);
        with_persona.trashed_at = None;
        repo.save(&with_persona).await.unwrap();
        repo.save(&thrown_away).await.unwrap();

        let stamp = DateTime::from_timestamp_millis(1_000).unwrap();
        let taken = repo.trash_by_persona(&persona, stamp).await.unwrap();
        assert_eq!(
            taken,
            vec![with_persona.id],
            "only the live asset is taken, and its id is reported back"
        );
        assert_eq!(
            repo.find(&thrown_away.id)
                .await
                .unwrap()
                .unwrap()
                .trashed_at,
            DateTime::from_timestamp_millis(500),
            "the hand-trashed asset keeps its own stamp"
        );

        let back = repo.restore_by_persona(&persona, stamp).await.unwrap();
        assert_eq!(back, vec![with_persona.id]);
        assert!(
            repo.find(&with_persona.id)
                .await
                .unwrap()
                .unwrap()
                .trashed_at
                .is_none(),
            "restore clears the stamp it set"
        );
        assert!(
            repo.find(&thrown_away.id)
                .await
                .unwrap()
                .unwrap()
                .trashed_at
                .is_some(),
            "restoring a persona must not resurrect what the user threw away"
        );

        driver.shutdown().await.unwrap();
    }

    /// The single filter surface is what makes "no listing leaks a
    /// trashed asset" true by construction rather than by review, so it
    /// gets a direct test: the clause must be present for the default
    /// query without anyone asking for it.
    #[test]
    fn query_parts_excludes_trashed_rows_by_default() {
        let parts = QueryParts::build(&AssetQuery::default());
        assert!(
            parts.where_sql.contains("asset.trashed_at IS NULL"),
            "default query must exclude trashed rows, got: {}",
            parts.where_sql
        );
    }

    /// The star band narrows to the requested range and drops the
    /// unrated rows, from either end of the band.
    ///
    /// The assertions compare *sets* (both sides sorted before the
    /// comparison), which is where a predicate that silently matched
    /// everything shows up: all four ids against the expected two.
    #[tokio::test]
    async fn rating_band_filters_and_excludes_the_unrated() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut ids = std::collections::HashMap::new();
        for (locator, rating, occurred_ms) in [
            ("/pics/one.png", Some(1u8), 1_785_000_004_000_i64),
            ("/pics/three.png", Some(3), 1_785_000_003_000),
            ("/pics/five.png", Some(5), 1_785_000_002_000),
            ("/pics/unrated.png", None, 1_785_000_001_000),
        ] {
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::DateTime::from_timestamp_millis(occurred_ms).unwrap(),
                &nobody(),
            );
            asset.rating = rating;
            repo.save(&asset).await.unwrap();
            ids.insert(locator, asset.id);
        }

        let matched = |min: Option<u8>, max: Option<u8>| {
            let repo = repo.clone();
            async move {
                let page = repo
                    .list(&AssetQuery {
                        persona_id: Some(persona),
                        rating_min: min,
                        rating_max: max,
                        ..AssetQuery::default()
                    })
                    .await
                    .unwrap();
                let mut got: Vec<String> = page
                    .items
                    .iter()
                    .map(|c| c.id.to_string())
                    .collect::<Vec<_>>();
                got.sort();
                (got, page.total)
            }
        };
        let expect = |locators: &[&str]| {
            let mut want: Vec<String> = locators.iter().map(|l| ids[l].to_string()).collect();
            want.sort();
            want
        };

        // Both bounds inclusive: 3 is in a 3..=5 band, 1 is not.
        assert_eq!(
            matched(Some(3), Some(5)).await,
            (expect(&["/pics/three.png", "/pics/five.png"]), Some(2)),
            "band is inclusive at both ends"
        );
        // Upper bound alone still drops the unrated row — "at most three
        // stars" asks about rated assets, and `NULL <= 3` is unknown.
        assert_eq!(
            matched(None, Some(3)).await,
            (expect(&["/pics/one.png", "/pics/three.png"]), Some(2)),
            "an upper bound alone must not sweep in the unrated"
        );
        // Lower bound alone, same exclusion from the other side.
        assert_eq!(
            matched(Some(1), None).await,
            (
                expect(&["/pics/one.png", "/pics/three.png", "/pics/five.png"]),
                Some(3)
            ),
            "rating_min=1 is every rated asset, and only those"
        );
        // A one-value band.
        assert_eq!(
            matched(Some(5), Some(5)).await,
            (expect(&["/pics/five.png"]), Some(1)),
            "min == max selects exactly that many stars"
        );
        // No band named is the only state the unrated asset survives.
        assert_eq!(
            matched(None, None).await.0,
            expect(&[
                "/pics/one.png",
                "/pics/three.png",
                "/pics/five.png",
                "/pics/unrated.png",
            ]),
            "without a band the unrated asset is still part of the corpus"
        );
    }

    /// The length and size bands narrow on their own column, both ends
    /// inclusive, compose with each other, and drop the rows that have
    /// nothing to place in the band — from either end.
    ///
    /// Two properties of the fixture are what stop these assertions from
    /// passing on a predicate that is wired to the wrong column, and
    /// both are deliberate:
    ///
    /// - **length and size run in opposite orders** — the longest clip
    ///   is the smallest file — so a band aimed one column over selects
    ///   a different set rather than the same one by luck.
    /// - **the two absent values sit on different rows, in different
    ///   columns**. The still image has no length but a recorded size;
    ///   the container the importer could not probe is the reverse. A
    ///   fixture whose one unmeasured row was `NULL` in both columns
    ///   would agree with an exclusion aimed at either of them, and a
    ///   fixture where every row carried both would prove nothing about
    ///   exclusion at all.
    #[tokio::test]
    async fn metric_bands_filter_on_their_own_column_and_exclude_the_absent() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut ids = std::collections::HashMap::new();
        for (locator, duration_ms, file_size_bytes) in [
            ("/clips/short.mp4", Some(1_000_u64), Some(9_000_000_u64)),
            ("/clips/long.mp4", Some(120_000), Some(500_000)),
            // A still: nothing plays, but the bytes are on record.
            ("/pics/still.png", None, Some(2_000_000)),
            // The mirror image — a container whose length the importer
            // read but whose size never reached the row.
            ("/clips/unprobed.avi", Some(30_000), None),
        ] {
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::DateTime::from_timestamp_millis(1_785_000_000_000).unwrap(),
                &nobody(),
            );
            asset.duration_ms = duration_ms;
            asset.source.file_size_bytes = file_size_bytes;
            repo.save(&asset).await.unwrap();
            ids.insert(locator, asset.id);
        }

        let matched = |duration: (Option<u64>, Option<u64>), size: (Option<u64>, Option<u64>)| {
            let repo = repo.clone();
            async move {
                let page = repo
                    .list(&AssetQuery {
                        persona_id: Some(persona),
                        duration_min_ms: duration.0,
                        duration_max_ms: duration.1,
                        size_min_bytes: size.0,
                        size_max_bytes: size.1,
                        ..AssetQuery::default()
                    })
                    .await
                    .unwrap();
                let mut got: Vec<String> = page.items.iter().map(|c| c.id.to_string()).collect();
                got.sort();
                got
            }
        };
        let expect = |locators: &[&str]| {
            let mut want: Vec<String> = locators.iter().map(|l| ids[l].to_string()).collect();
            want.sort();
            want
        };

        // A floor: 30 s and up. The one-second clip fails the bound, the
        // still fails for having no length at all.
        assert_eq!(
            matched((Some(30_000), None), (None, None)).await,
            expect(&["/clips/long.mp4", "/clips/unprobed.avi"]),
            "a floor keeps everything at or above it, and only what plays"
        );
        // A ceiling **alone** still drops the still image, even though
        // it is the row a `NULL <= 30000` reading would be most tempted
        // to admit.
        assert_eq!(
            matched((None, Some(30_000)), (None, None)).await,
            expect(&["/clips/short.mp4", "/clips/unprobed.avi"]),
            "an upper bound alone must not sweep in what has no length"
        );
        // Both ends inclusive on the same value.
        assert_eq!(
            matched((Some(30_000), Some(30_000)), (None, None)).await,
            expect(&["/clips/unprobed.avi"]),
            "min == max selects exactly that length"
        );
        // The size axis excludes the *other* row — the one the length
        // axis kept — which is what makes each band its own column's.
        assert_eq!(
            matched((None, None), (None, Some(2_000_000))).await,
            expect(&["/clips/long.mp4", "/pics/still.png"]),
            "a size ceiling drops the row whose bytes were never recorded"
        );
        assert_eq!(
            matched((None, None), (Some(2_000_000), None)).await,
            expect(&["/clips/short.mp4", "/pics/still.png"]),
            "a size floor is inclusive and stays on its own column"
        );
        // Both bands at once: ≥30 s leaves {long, unprobed}, ≤2 MB
        // leaves {long, still}, and the filter is their conjunction.
        assert_eq!(
            matched((Some(30_000), None), (None, Some(2_000_000))).await,
            expect(&["/clips/long.mp4"]),
            "the two bands compose rather than replacing each other"
        );
        // A floor past what a SQLite INTEGER can hold saturates instead
        // of wrapping. `u64::MAX as i64` is -1, which would turn "longer
        // than anything can be" into a band containing every measured
        // row — the one direction a too-large bound must not go.
        assert_eq!(
            matched((Some(u64::MAX), None), (None, None)).await,
            expect(&[]),
            "an unsatisfiable floor selects nothing, not everything"
        );
        // No band named is the only state the unmeasured rows survive.
        assert_eq!(
            matched((None, None), (None, None)).await,
            expect(&[
                "/clips/short.mp4",
                "/clips/long.mp4",
                "/pics/still.png",
                "/clips/unprobed.avi",
            ]),
            "without a band, a still and an unprobed container are still assets"
        );
    }

    /// The exclusion conjunct belongs to the band, not to each bound: a
    /// two-ended band states it once. Emitting it per bound would say
    /// the same thing twice, which is the readable tell that the two
    /// halves were appended independently.
    #[test]
    fn metric_band_states_its_exclusion_once_and_only_when_asked() {
        let both_ends = QueryParts::build(&AssetQuery {
            duration_min_ms: Some(1_000),
            duration_max_ms: Some(2_000),
            size_min_bytes: Some(1),
            size_max_bytes: Some(2),
            ..AssetQuery::default()
        });
        for column in ["asset.duration_ms", "asset.file_size_bytes"] {
            assert_eq!(
                both_ends
                    .where_sql
                    .matches(&format!("{column} IS NOT NULL"))
                    .count(),
                1,
                "{column} band must state its population once, got: {}",
                both_ends.where_sql
            );
        }
        // And no band asked for adds no clause at all — the state that
        // keeps stills and unprobed containers in every listing whose
        // caller never heard of these axes.
        let none = QueryParts::build(&AssetQuery::default());
        assert!(
            !none.where_sql.contains("duration_ms") && !none.where_sql.contains("file_size_bytes"),
            "an unasked band must not narrow the default listing, got: {}",
            none.where_sql
        );
    }

    /// The ingest and modification windows narrow to their own column,
    /// both ends inclusive, and compose with each other.
    ///
    /// The fixture is built so that **no two axes agree**: `created_at`
    /// ascends across the four rows, `updated_at` descends, and
    /// `occurred_at` runs in a third order that matches neither. Without
    /// that, every one of these assertions could pass against a predicate
    /// that was cross-wired to the wrong column, or dropped entirely —
    /// freshly-saved rows normally carry three stamps that move together,
    /// which is exactly the fixture shape that proves nothing.
    #[tokio::test]
    async fn ingest_and_modification_windows_filter_on_their_own_column() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut ids = std::collections::HashMap::new();
        for (locator, occurred_ms, created_ms, updated_ms) in [
            ("/pics/a.png", 5_000_i64, 1_000_i64, 4_000_i64),
            ("/pics/b.png", 8_000, 2_000, 3_000),
            ("/pics/c.png", 2_000, 3_000, 2_000),
            ("/pics/d.png", 9_000, 4_000, 1_000),
        ] {
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::DateTime::from_timestamp_millis(occurred_ms).unwrap(),
                &nobody(),
            );
            // `save` persists whatever the entity carries for these two
            // columns, so the fixture can place them exactly rather than
            // sleeping and hoping the clock cooperates.
            asset.created_at = chrono::DateTime::from_timestamp_millis(created_ms).unwrap();
            asset.updated_at = chrono::DateTime::from_timestamp_millis(updated_ms).unwrap();
            repo.save(&asset).await.unwrap();
            ids.insert(locator, asset.id);
        }

        let at = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).unwrap();
        let expect = |locators: &[&str]| {
            let mut want: Vec<String> = locators.iter().map(|l| ids[l].to_string()).collect();
            want.sort();
            want
        };
        let listed = |query: AssetQuery| {
            let repo = repo.clone();
            async move {
                let page = repo.list(&query).await.unwrap();
                let mut got: Vec<String> = page.items.iter().map(|c| c.id.to_string()).collect();
                got.sort();
                got
            }
        };
        let base = || AssetQuery {
            persona_id: Some(persona),
            ..AssetQuery::default()
        };

        // One bound at a time, and the four answers form two
        // complementary pairs — a `from` wired as an `until` (or an
        // ingest bound reading the modification column) swaps one of
        // these for its opposite.
        assert_eq!(
            listed(AssetQuery {
                created_from: Some(at(3_000)),
                ..base()
            })
            .await,
            expect(&["/pics/c.png", "/pics/d.png"]),
            "created_from is inclusive: the row stamped exactly 3000 stays"
        );
        assert_eq!(
            listed(AssetQuery {
                created_until: Some(at(2_000)),
                ..base()
            })
            .await,
            expect(&["/pics/a.png", "/pics/b.png"]),
            "created_until is inclusive: the row stamped exactly 2000 stays"
        );
        assert_eq!(
            listed(AssetQuery {
                updated_from: Some(at(3_000)),
                ..base()
            })
            .await,
            expect(&["/pics/a.png", "/pics/b.png"]),
            "updated_from reads updated_at, where the order is reversed"
        );
        assert_eq!(
            listed(AssetQuery {
                updated_until: Some(at(2_000)),
                ..base()
            })
            .await,
            expect(&["/pics/c.png", "/pics/d.png"]),
            "updated_until reads updated_at, where the order is reversed"
        );

        // A closed window on each axis. The two windows carry the same
        // numbers and must still answer differently.
        assert_eq!(
            listed(AssetQuery {
                created_from: Some(at(2_000)),
                created_until: Some(at(3_000)),
                ..base()
            })
            .await,
            expect(&["/pics/b.png", "/pics/c.png"]),
            "closed ingest window, both ends inclusive"
        );
        assert_eq!(
            listed(AssetQuery {
                updated_from: Some(at(2_000)),
                updated_until: Some(at(3_000)),
                ..base()
            })
            .await,
            expect(&["/pics/b.png", "/pics/c.png"]),
            "closed modification window, both ends inclusive"
        );

        // The two axes compose as `AND`. Each half alone answers three
        // rows and two rows; only one row is in both, so an
        // implementation that dropped either conjunct returns a
        // different set.
        assert_eq!(
            listed(AssetQuery {
                created_from: Some(at(2_000)),
                updated_from: Some(at(3_000)),
                ..base()
            })
            .await,
            expect(&["/pics/b.png"]),
            "ingest and modification windows intersect"
        );

        // The windows also compose with the pre-existing occurrence
        // window, which runs in a third order: `occurred_at >= 8000` is
        // {b, d} and `created_at <= 2000` is {a, b} — two rows each, one
        // row in common, so dropping either predicate changes the answer.
        assert_eq!(
            listed(AssetQuery {
                occurred_from: Some(at(8_000)),
                created_until: Some(at(2_000)),
                ..base()
            })
            .await,
            expect(&["/pics/b.png"]),
            "the new windows narrow alongside the occurrence window, not instead of it"
        );

        // An inverted window is answered with an empty page rather than
        // an error — the documented divergence from the rating band.
        assert!(
            listed(AssetQuery {
                created_from: Some(at(4_000)),
                created_until: Some(at(1_000)),
                ..base()
            })
            .await
            .is_empty(),
            "an inverted window matches nothing and is not rejected"
        );

        // Search reaches the same predicate through `filter_ids` rather
        // than `list`. Handing it every id and asking for a window has to
        // narrow the same way, or a chip works in the grid and silently
        // does nothing in search results.
        let all: Vec<_> = ["/pics/a.png", "/pics/b.png", "/pics/c.png", "/pics/d.png"]
            .iter()
            .map(|l| ids[l])
            .collect();
        let mut survived: Vec<String> = repo
            .filter_ids(
                &all,
                &AssetQuery {
                    updated_from: Some(at(3_000)),
                    ..base()
                },
            )
            .await
            .unwrap()
            .iter()
            .map(|id| id.to_string())
            .collect();
        survived.sort();
        assert_eq!(
            survived,
            expect(&["/pics/a.png", "/pics/b.png"]),
            "the search path shares the builder, so the window applies there too"
        );
    }

    /// The colour facet is a projection of `asset.palette`, so the two
    /// must never disagree: writing a palette derives its swatches,
    /// re-writing it replaces them (a colour that left the palette
    /// leaves the facet), and clearing it removes them entirely. The
    /// filter and the counts both read that projection.
    #[tokio::test]
    async fn palette_writes_drive_the_colour_facet() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let new_asset = |locator: &str| {
            Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            )
        };
        let reddish = new_asset("/pics/red.png");
        let bluish = new_asset("/pics/blue.png");
        let unextracted = new_asset("/pics/pending.png");
        for asset in [&reddish, &bluish, &unextracted] {
            repo.save(asset).await.unwrap();
        }

        // Two red entries collapse to one Red row — the facet counts
        // assets, not palette entries.
        repo.set_palette(
            &reddish.id,
            Some(vec!["#ff0000".into(), "#cc1111".into(), "#ffffff".into()]),
        )
        .await
        .unwrap();
        repo.set_palette(&bluish.id, Some(vec!["#1c7ed6".into()]))
            .await
            .unwrap();

        let counts = repo
            .counts_by_color(Some(&persona), TrashFilter::LiveOnly)
            .await
            .unwrap();
        assert_eq!(
            counts,
            vec![
                (ColorBucket::Red, 1),
                (ColorBucket::Blue, 1),
                (ColorBucket::White, 1),
            ],
            "swatch order, one row per asset per bucket"
        );

        let red_page = repo
            .list(&AssetQuery {
                persona_id: Some(persona),
                color: Some(ColorBucket::Red),
                ..AssetQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(
            red_page.items.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![reddish.id],
            "the unextracted asset must not match a colour it never claimed"
        );

        // Re-extraction that no longer sees red must drop the swatch.
        repo.set_palette(&reddish.id, Some(vec!["#37b24d".into()]))
            .await
            .unwrap();
        let after = repo
            .counts_by_color(Some(&persona), TrashFilter::LiveOnly)
            .await
            .unwrap();
        assert_eq!(
            after,
            vec![(ColorBucket::Green, 1), (ColorBucket::Blue, 1)],
            "stale swatches are replaced, not merged"
        );

        // Clearing the palette clears the projection with it.
        repo.set_palette(&bluish.id, None).await.unwrap();
        let cleared = repo
            .counts_by_color(Some(&persona), TrashFilter::LiveOnly)
            .await
            .unwrap();
        assert_eq!(cleared, vec![(ColorBucket::Green, 1)]);
    }

    /// The rule for "may this value group as a duplicate" is evaluated
    /// in two places — [`content_hash::is_duplicate_key`] in Rust, and
    /// the SQL [`duplicate_key_condition`] hands to SQLite. Sharing the
    /// constants does not stop the two from drifting: "starts with the
    /// prefix and is not reserved" is still written out twice. One
    /// vector of stored-value shapes therefore goes through both, and
    /// every verdict has to match.
    ///
    /// **On both axes.** The axis is an argument now, and an argument
    /// that only one caller ever passes is an argument nothing tests:
    /// the vector below carries the markers and digests of both columns,
    /// and each axis is asked about all of them — so an axis that
    /// admitted the other's digests, or excluded nothing at all, fails
    /// here rather than in a duplicate report that groups two pictures
    /// which differ.
    #[test]
    fn the_sql_duplicate_filter_matches_the_domain_predicate() {
        use asterism_core::domain::content_hash;
        use asterism_core::domain::content_region;

        let content_digest = format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64));
        let meta_digest = format!("{}{}", content_hash::META_DIGEST_PREFIX, "a".repeat(64));
        let samples = [
            content_hash::of_bytes(b"star"),
            content_hash::of_bytes(b"a different picture"),
            // A real digest that must not group anyway, on either axis.
            content_hash::EMPTY.to_string(),
            content_hash::CONTENT_REGION_EMPTY.to_string(),
            content_hash::META_EMPTY.to_string(),
            content_hash::UNHASHABLE.to_string(),
            // A future algorithm, answering a different question.
            "phash:0f0f0f0f".to_string(),
            // Each prefix and nothing after it.
            content_hash::DIGEST_PREFIX.to_string(),
            content_hash::CONTENT_DIGEST_PREFIX.to_string(),
            content_hash::META_DIGEST_PREFIX.to_string(),
            // The algorithm tag without its separator.
            "sha256".to_string(),
            // A marker shape that is not the one the domain reserves.
            "unhashable:something-else".to_string(),
            // The short stand-in the other tests in this file store.
            "sha256:aaaa".to_string(),
            // The content column's own vocabulary: a digest, and every
            // marker that can sit beside it.
            content_digest.clone(),
            content_region::EMPTY_SPAN.to_string(),
            content_region::TOO_LARGE.to_string(),
            content_region::NOT_WALKED.to_string(),
            "unsupported:video/mp4".to_string(),
            // A region digest from an earlier definition of the region.
            format!("cr0-sha256:{}", "b".repeat(64)),
            // The meta column's own vocabulary. Its markers are the same
            // ones listed above — they say something about the artefact
            // rather than about which walk ran — so only the digest and
            // a superseded generation are new here.
            meta_digest.clone(),
            format!("m0-sha256:{}", "b".repeat(64)),
            // The prefixes in the wrong case — the shape a case-folding
            // prefix test would admit and the domain would refuse.
            content_hash::of_bytes(b"star").to_uppercase(),
            content_digest.to_uppercase(),
            meta_digest.to_uppercase(),
            String::new(),
        ];

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE probe (value TEXT NOT NULL)", params![])
            .unwrap();
        for value in &samples {
            conn.execute("INSERT INTO probe (value) VALUES (?1)", params![value])
                .unwrap();
        }

        for axis in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
            let condition = duplicate_key_condition(axis, "value");
            let sql = format!("SELECT value FROM probe WHERE {condition}");
            let passed_in_sql: std::collections::HashSet<String> = conn
                .prepare(&sql)
                .unwrap()
                .query_map(params![], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

            for value in &samples {
                assert_eq!(
                    content_hash::is_duplicate_key(axis, value),
                    passed_in_sql.contains(value),
                    "Rust and SQL disagree about {value:?} on the {} axis (condition: {condition})",
                    axis.as_str()
                );
            }
            // Agreement is only worth asserting over a vector that the
            // rule splits: all-pass or all-fail would match vacuously,
            // and it has to split on *each* axis — a content axis that
            // admitted nothing would otherwise agree with SQL perfectly.
            assert!(
                samples
                    .iter()
                    .any(|v| content_hash::is_duplicate_key(axis, v)),
                "no sample groups on the {} axis",
                axis.as_str()
            );
            assert!(
                samples
                    .iter()
                    .any(|v| !content_hash::is_duplicate_key(axis, v)),
                "every sample groups on the {} axis",
                axis.as_str()
            );
        }
    }

    /// The fingerprint walk's selection is the same rule in two
    /// languages — [`content_hash::needs_fingerprint`] for the
    /// per-asset job's skip test, [`unfingerprinted_condition`] for the
    /// page query and the count. One vector of `(file, content, meta)`
    /// column shapes goes through both, including the `NULL`s, which are
    /// where a three-valued logic mistake hides: a `GLOB` against `NULL`
    /// is `NULL`, and `NOT NULL` is `NULL` rather than true, so a naive
    /// spelling drops exactly the rows the walk exists to find.
    ///
    /// The two versioned columns are varied **independently**. A vector
    /// that moved them together would pass against a condition that read
    /// only one of them, which is the shape a third column arrives as.
    #[test]
    fn the_sql_fingerprint_filter_matches_the_domain_predicate() {
        use asterism_core::domain::content_hash;
        use asterism_core::domain::content_region;

        let digest = content_hash::of_bytes(b"star");
        let region = format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64));
        let meta = format!("{}{}", content_hash::META_DIGEST_PREFIX, "a".repeat(64));
        let answered_meta = Some(meta.clone());
        let samples: Vec<(Option<String>, Option<String>, Option<String>)> = vec![
            // Nothing looked yet.
            (None, None, None),
            // Half-written, every way round — including the two that
            // differ only in which versioned column is missing.
            (Some(digest.clone()), None, None),
            (None, Some(region.clone()), answered_meta.clone()),
            (Some(digest.clone()), Some(region.clone()), None),
            (Some(digest.clone()), None, answered_meta.clone()),
            // Answered.
            (
                Some(digest.clone()),
                Some(region.clone()),
                answered_meta.clone(),
            ),
            (
                Some(digest.clone()),
                Some(content_region::EMPTY_SPAN.to_string()),
                Some(content_region::EMPTY_SPAN.to_string()),
            ),
            (
                Some(digest.clone()),
                Some(content_region::TOO_LARGE.to_string()),
                Some(content_region::TOO_LARGE.to_string()),
            ),
            (
                Some(digest.clone()),
                Some(content_region::NOT_WALKED.to_string()),
                Some(content_region::NOT_WALKED.to_string()),
            ),
            (
                Some(digest.clone()),
                Some("unsupported:video/mp4".to_string()),
                Some("unsupported:video/mp4".to_string()),
            ),
            (
                Some(content_hash::UNHASHABLE.to_string()),
                Some(content_hash::UNHASHABLE.to_string()),
                Some(content_hash::UNHASHABLE.to_string()),
            ),
            // Not answers: an earlier generation of either definition,
            // and one axis's digest sitting in another's column.
            (
                Some(digest.clone()),
                Some(format!("cr0-sha256:{}", "b".repeat(64))),
                answered_meta.clone(),
            ),
            (
                Some(digest.clone()),
                Some(region.clone()),
                Some(format!("m0-sha256:{}", "b".repeat(64))),
            ),
            (
                Some(digest.clone()),
                Some(digest.clone()),
                answered_meta.clone(),
            ),
            (
                Some(digest.clone()),
                Some(region.clone()),
                Some(region.clone()),
            ),
            (
                Some(digest.clone()),
                Some(String::new()),
                answered_meta.clone(),
            ),
            (
                Some(digest.clone()),
                Some(region.clone()),
                Some(String::new()),
            ),
        ];

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE probe (n INTEGER PRIMARY KEY, f TEXT, c TEXT, m TEXT)",
            params![],
        )
        .unwrap();
        for (n, (file, content, meta)) in samples.iter().enumerate() {
            conn.execute(
                "INSERT INTO probe (n, f, c, m) VALUES (?1, ?2, ?3, ?4)",
                params![n as i64, file, content, meta],
            )
            .unwrap();
        }

        let condition = unfingerprinted_condition("f", "c", "m");
        let selected: std::collections::HashSet<i64> = conn
            .prepare(&format!("SELECT n FROM probe WHERE {condition}"))
            .unwrap()
            .query_map(params![], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        for (n, (file, content, meta)) in samples.iter().enumerate() {
            assert_eq!(
                content_hash::needs_fingerprint(
                    file.as_deref(),
                    content.as_deref(),
                    meta.as_deref()
                ),
                selected.contains(&(n as i64)),
                "Rust and SQL disagree about {file:?} / {content:?} / {meta:?} \
                 (condition: {condition})"
            );
        }
        assert!(!selected.is_empty(), "no sample is work");
        assert!(selected.len() < samples.len(), "every sample is work");
    }

    /// [`duplicate_key_condition`] and [`unfingerprinted_condition`]
    /// interpolate tags and marker prefixes into `GLOB` patterns without
    /// escaping them. That is safe for the four spellings below and
    /// stops being safe the moment one contains `*`, `?` or `[` — a
    /// pattern that would then match by accident rather than by prefix.
    #[test]
    fn the_digest_prefixes_are_glob_safe() {
        use asterism_core::domain::content_hash::{
            CONTENT_DIGEST_PREFIX, DIGEST_PREFIX, META_DIGEST_PREFIX,
        };
        use asterism_core::domain::content_region::UNSUPPORTED_PREFIX;

        for prefix in [
            DIGEST_PREFIX,
            CONTENT_DIGEST_PREFIX,
            META_DIGEST_PREFIX,
            UNSUPPORTED_PREFIX,
        ] {
            assert!(
                !prefix.contains(['*', '?', '[']),
                "{prefix:?} carries GLOB syntax; escape it before interpolating"
            );
        }
        // The values interpolated as equalities rather than patterns
        // still have to survive being quoted.
        assert!(!asterism_core::domain::content_hash::UNHASHABLE.contains('\''));
        assert!(!asterism_core::domain::content_region::NOT_WALKED.contains('\''));
    }

    /// The migration's selection is the same rule in two languages —
    /// [`content_hash::needs_content_walk`] in Rust,
    /// [`unwalked_condition`] in SQL — and the vector below is shared
    /// with the fingerprint rule's own differential test on purpose.
    ///
    /// Running one set of column shapes through **both** conditions is
    /// what pins the thing the design turns on: the two select disjoint
    /// sets over the same column. A change that made the ordinary walk
    /// admit `NOT_WALKED` — the shape that hands a whole pre-existing
    /// library to the pass meant for new arrivals — shows up here as an
    /// overlap rather than as a silent doubling of work.
    #[test]
    fn the_sql_unwalked_filter_matches_the_domain_predicate() {
        use asterism_core::domain::content_hash;
        use asterism_core::domain::content_region;

        let digest = content_hash::of_bytes(b"star");
        let region = format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64));
        let samples: Vec<Option<String>> = vec![
            None,
            Some(content_region::NOT_WALKED.to_string()),
            Some(region.clone()),
            Some(content_region::EMPTY_SPAN.to_string()),
            Some(content_region::TOO_LARGE.to_string()),
            Some("unsupported:video/mp4".to_string()),
            Some(content_hash::UNHASHABLE.to_string()),
            Some(format!("cr0-sha256:{}", "b".repeat(64))),
            Some(digest.clone()),
            Some(String::new()),
        ];

        // The meta column holds an answer on every row, for the reason
        // the file column holds a digest on every row: this test is
        // about the content column, and a second column that was work
        // would make the ordinary walk select everything and the
        // disjointness below vacuous from the other side.
        let meta_answer = format!("{}{}", content_hash::META_DIGEST_PREFIX, "c".repeat(64));
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE probe (n INTEGER PRIMARY KEY, f TEXT, c TEXT, m TEXT)",
            params![],
        )
        .unwrap();
        for (n, content) in samples.iter().enumerate() {
            conn.execute(
                "INSERT INTO probe (n, f, c, m) VALUES (?1, ?2, ?3, ?4)",
                params![n as i64, digest, content, meta_answer],
            )
            .unwrap();
        }

        let select = |condition: &str| -> std::collections::HashSet<i64> {
            conn.prepare(&format!("SELECT n FROM probe WHERE {condition}"))
                .unwrap()
                .query_map(params![], |r| r.get::<_, i64>(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };

        let condition = unwalked_condition("c");
        let migration = select(&condition);
        for (n, content) in samples.iter().enumerate() {
            assert_eq!(
                content_hash::needs_content_walk(content.as_deref()),
                migration.contains(&(n as i64)),
                "Rust and SQL disagree about {content:?} (condition: {condition})"
            );
        }
        assert_eq!(migration.len(), 1, "exactly one sample is the marker");

        // The two passes over one column must not both claim a row. The
        // file side of every sample holds a digest, so whatever the
        // ordinary walk selects here it selects on the content column
        // alone — which is the comparison worth making.
        let ordinary = select(&unfingerprinted_condition("f", "c", "m"));
        assert!(
            ordinary.is_disjoint(&migration),
            "a row claimed by both passes would be read twice: \
             ordinary {ordinary:?} / migration {migration:?}"
        );
        assert!(
            !ordinary.is_empty(),
            "a vacuous disjointness: the ordinary walk selected nothing at all"
        );
    }

    /// The duplicate report groups on the content fingerprint, and has
    /// to be right about three things a naive `GROUP BY` gets wrong: a
    /// hash held by one asset is not a finding, an unhashed material
    /// is unknown rather than unique, and a trashed asset is already
    /// on its way out.
    #[tokio::test]
    async fn duplicate_groups_report_only_live_multi_member_sets() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let make = |locator: &str| {
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            );
            asset.materials = vec![asterism_core::domain::material::Material::primary(
                loc(locator),
                Some(1),
                chrono::Utc::now(),
            )];
            asset
        };
        let twin_a = make("/pics/a.png");
        let twin_b = make("/pics/copy-of-a.png");
        let alone = make("/pics/b.png");
        let trashed_twin = make("/pics/also-a.png");
        let unhashed = make("/pics/pending.png");
        for asset in [&twin_a, &twin_b, &alone, &trashed_twin, &unhashed] {
            repo.save(asset).await.unwrap();
        }

        let shared = "sha256:aaaa";
        for asset in [&twin_a, &twin_b, &trashed_twin] {
            repo.set_material_fingerprint(&asset.id, 0, &file_axis(shared))
                .await
                .unwrap();
        }
        repo.set_material_fingerprint(&alone.id, 0, &file_axis("sha256:bbbb"))
            .await
            .unwrap();
        repo.trash(&trashed_twin.id, chrono::Utc::now())
            .await
            .unwrap();

        // Materials that can never be hashed all share one marker
        // string. Grouping on it would report the whole conversation
        // corpus as a single duplicate set.
        let unhashable_a = make("/logs/session.jsonl#aaaa");
        let unhashable_b = make("/logs/session.jsonl#bbbb");
        for asset in [&unhashable_a, &unhashable_b] {
            repo.save(asset).await.unwrap();
            repo.set_material_fingerprint(
                &asset.id,
                0,
                &MaterialFingerprint {
                    file: UNHASHABLE.to_string(),
                    content: UNHASHABLE.to_string(),
                    meta: UNHASHABLE.to_string(),
                    meta_kv: None,
                    meta_raw: None,
                    meta_text: None,
                },
            )
            .await
            .unwrap();
        }

        // Empty files all share the real digest of zero bytes. They
        // are failure debris, not copies of one picture — a group of
        // them invites a bulk "keep one" over unrelated files.
        let empty_a = make("/downloads/failed-1.png");
        let empty_b = make("/downloads/failed-2.png");
        for asset in [&empty_a, &empty_b] {
            repo.save(asset).await.unwrap();
            repo.set_material_fingerprint(&asset.id, 0, &file_axis(EMPTY))
                .await
                .unwrap();
        }

        // Both scopes are exercised: the persona-less call is the
        // default path (no persona selected) and binds a different
        // parameter set.
        for scope in [Some(&persona), None] {
            let groups = repo
                .list_duplicate_groups(scope, DuplicateAxis::Artefact, 50)
                .await
                .unwrap();
            assert_eq!(
                groups.len(),
                1,
                "one shared digest, one group (scope persona: {})",
                scope.is_some()
            );
            assert_eq!(groups[0].content_hash, shared);
        }

        let groups = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 50)
            .await
            .unwrap();
        assert_eq!(groups.len(), 1, "one shared hash, one group");
        assert_eq!(groups[0].content_hash, shared);
        assert_eq!(
            groups[0]
                .members
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([twin_a.id, twin_b.id]),
            "the trashed copy is not part of the work list"
        );

        // The unhashed asset is invisible to the report but visible in
        // the count — that is the pair that keeps an empty report from
        // reading as a clean bill of health.
        assert_eq!(
            repo.unhashed_material_count().await.unwrap(),
            1,
            "one material still waiting for its bytes to be read"
        );

        // Every row above was written by `file_axis`, so every content
        // column in this fixture holds the same `unsupported:unknown`
        // marker. Nine rows sharing one string is precisely the shape
        // that a `GROUP BY` without the exclusion turns into a single
        // enormous "duplicate" set.
        assert!(
            repo.list_duplicate_groups(Some(&persona), DuplicateAxis::Content, 50)
                .await
                .unwrap()
                .is_empty(),
            "a marker shared by every row is not an agreement about anything"
        );
    }

    /// The axis is an argument now, and the two axes are different
    /// questions about the same rows.
    ///
    /// The fixture is the one measured on the real corpus: two exports
    /// of one picture whose pixel bytes are identical and whose files
    /// differ, because one carries a metadata chunk the other does not.
    /// The file axis is right to call them two files; the content axis
    /// is right to call them one picture; a report that answered the
    /// same way on both would be reading one column twice.
    ///
    /// The markers are here for the failure that costs more than a
    /// missed match. `unsupported:not-walked` sits on every material the
    /// column's migration could not read, and `unsupported:<mime>` on
    /// every format with no walker, so admitting either as a group key
    /// would report a whole swathe of unrelated rows as one duplicate
    /// set — and the panel's resolution for a group is "keep this, trash
    /// the rest".
    #[tokio::test]
    async fn the_duplicate_report_answers_the_axis_it_was_asked_for() {
        use asterism_core::domain::content_region;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let make = |locator: &str| {
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            );
            asset.materials = vec![asterism_core::domain::material::Material::primary(
                loc(locator),
                Some(1),
                chrono::Utc::now(),
            )];
            asset
        };

        let picture = "cr1-sha256:pppp";
        // Same picture, different files: only the metadata chunk moved.
        let bare = make("/comfy/run-1.png");
        let noted = make("/comfy/run-1-with-workflow.png");
        // Same bytes twice: both axes agree, because a file that is
        // byte-identical has a byte-identical region.
        let twin_a = make("/comfy/run-2.png");
        let twin_b = make("/archive/run-2.png");
        for (asset, file, content) in [
            (&bare, "sha256:bare", picture),
            (&noted, "sha256:noted", picture),
            (&twin_a, "sha256:twin", "cr1-sha256:tttt"),
            (&twin_b, "sha256:twin", "cr1-sha256:tttt"),
        ] {
            repo.save(asset).await.unwrap();
            repo.set_material_fingerprint(&asset.id, 0, &both_axes(file, content))
                .await
                .unwrap();
        }

        // Rows whose content column holds a marker. Each pair shares its
        // marker string — so if the exclusion broke, each pair would
        // become a content group. Their file digests are distinct, which
        // keeps them out of the file axis for a reason of their own and
        // makes "in neither report" the literal claim.
        let mut marked = Vec::new();
        for (index, marker) in [
            content_region::NOT_WALKED,
            content_region::NOT_WALKED,
            content_region::TOO_LARGE,
            content_region::TOO_LARGE,
            content_region::EMPTY_SPAN,
            content_region::EMPTY_SPAN,
            "unsupported:video/mp4",
            "unsupported:video/mp4",
        ]
        .into_iter()
        .enumerate()
        {
            let asset = make(&format!("/mixed/{index}.bin"));
            repo.save(&asset).await.unwrap();
            repo.set_material_fingerprint(
                &asset.id,
                0,
                &both_axes(&format!("sha256:marked-{index}"), marker),
            )
            .await
            .unwrap();
            marked.push(asset.id);
        }

        let by_axis = |axis| {
            let repo = &repo;
            async move {
                repo.list_duplicate_groups(Some(&persona), axis, 50)
                    .await
                    .unwrap()
            }
        };

        let file_groups = by_axis(DuplicateAxis::Artefact).await;
        assert_eq!(
            file_groups
                .iter()
                .map(|g| g.content_hash.as_str())
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["sha256:twin"]),
            "the metadata pair is two files, and the file axis says so"
        );
        assert_eq!(
            file_groups[0]
                .members
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([twin_a.id, twin_b.id])
        );

        let content_groups = by_axis(DuplicateAxis::Content).await;
        assert_eq!(
            content_groups
                .iter()
                .map(|g| g.content_hash.as_str())
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([picture, "cr1-sha256:tttt"]),
            "one picture in two files, plus the pair that agrees on both axes"
        );
        let same_picture = content_groups
            .iter()
            .find(|g| g.content_hash == picture)
            .expect("the metadata pair groups on the content axis");
        assert_eq!(
            same_picture
                .members
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([bare.id, noted.id]),
            "…and its members are the two exports, fetched by the same column"
        );

        // Every group says which agreement it reports, and says the one
        // that was asked for.
        for (axis, groups) in [
            (DuplicateAxis::Artefact, &file_groups),
            (DuplicateAxis::Content, &content_groups),
        ] {
            assert!(!groups.is_empty(), "a vacuous axis proves nothing");
            for group in groups.iter() {
                assert_eq!(group.axis, axis, "{}", group.content_hash);
                assert!(
                    content_hash::is_duplicate_key(axis, &group.content_hash),
                    "{} is not a key the domain admits on {}",
                    group.content_hash,
                    axis.as_str()
                );
            }
        }

        // Not one marked row is a member of anything, on either axis.
        let reported: std::collections::HashSet<_> = file_groups
            .iter()
            .chain(content_groups.iter())
            .flat_map(|g| g.members.iter().map(|c| c.id))
            .collect();
        for id in &marked {
            assert!(
                !reported.contains(id),
                "a material carrying a marker was reported as a duplicate"
            );
        }

        // The count behind the notice: exactly the rows nobody walked,
        // not the whole marker family. The other six carry answers that
        // no later pass can improve on, and counting them would put work
        // in front of somebody that nothing can do.
        assert_eq!(
            repo.unwalked_material_count().await.unwrap(),
            2,
            "two materials the content axis has never looked at"
        );
        assert_eq!(
            repo.unhashed_material_count().await.unwrap(),
            0,
            "and none of them is unfingerprinted — a marker is an answer"
        );
    }

    /// Teeth: the meta axis reads `material.meta_hash`, and no other
    /// column.
    ///
    /// `axis_column` is a `match` over three names, and getting it wrong
    /// does not fail — it returns a plausible group keyed by one
    /// fingerprint whose members were selected by another. The fixture
    /// disagrees on every axis at once so that a report keyed off the
    /// wrong column cannot come out looking right: the two rows share a
    /// meta digest and differ on both of the others, which is the shape
    /// a batch off one workflow has.
    #[tokio::test]
    async fn the_meta_axis_reads_the_meta_column() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let make = |locator: &str| {
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            );
            asset.materials = vec![asterism_core::domain::material::Material::primary(
                loc(locator),
                Some(1),
                chrono::Utc::now(),
            )];
            asset
        };

        let canonical = r#"{"workflow":"{\"nodes\":[]}"}"#;
        let making = asterism_core::domain::material_meta::digest_of(canonical);
        let seed_1 = make("/comfy/seed-1.png");
        let seed_2 = make("/comfy/seed-2.png");
        for asset in [&seed_1, &seed_2] {
            repo.save(asset).await.unwrap();
            // `meta_axis` leaves the other two columns holding the
            // `unsupported:` marker, so neither of the other axes can
            // report this pair even by accident.
            repo.set_material_fingerprint(
                &asset.id,
                0,
                &meta_axis(&format!("sha256:{}", asset.id), &making, Some(canonical)),
            )
            .await
            .unwrap();
        }

        let groups = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Meta, 50)
            .await
            .unwrap();
        assert_eq!(groups.len(), 1, "one workflow, one group");
        assert_eq!(groups[0].axis, DuplicateAxis::Meta);
        assert_eq!(groups[0].content_hash, making);
        assert_eq!(
            groups[0]
                .members
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([seed_1.id, seed_2.id])
        );

        // The other two axes have nothing to say about this pair, which
        // is what makes the group above a reading of the meta column
        // rather than of one of theirs.
        for axis in [DuplicateAxis::Artefact, DuplicateAxis::Content] {
            assert!(
                repo.list_duplicate_groups(Some(&persona), axis, 50)
                    .await
                    .unwrap()
                    .is_empty(),
                "{} reported a pair that only agrees on its metadata",
                axis.as_str()
            );
        }

        // And the object travels with the digest, so a field comparison
        // has something to walk.
        let hydrated = repo.find(&seed_1.id).await.unwrap().unwrap();
        assert_eq!(hydrated.materials[0].meta_kv.as_deref(), Some(canonical));
        assert_eq!(
            hydrated
                .material_meta()
                .and_then(|f| f.get("workflow").cloned()),
            Some(r#"{"nodes":[]}"#.to_string()),
            "the value is the container's text, read back unparsed"
        );
    }

    /// The backfill walks every unhashed material and resumes from a
    /// cursor rather than rescanning — a library imported before the
    /// column existed is exactly the case this exists for.
    #[tokio::test]
    async fn unhashed_scan_pages_forward_and_shrinks_as_hashes_land() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut ids = Vec::new();
        for n in 0..3 {
            let locator = format!("/pics/{n}.png");
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), &locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            );
            asset.materials = vec![asterism_core::domain::material::Material::primary(
                loc(&locator),
                Some(1),
                chrono::Utc::now(),
            )];
            repo.save(&asset).await.unwrap();
            ids.push(asset.id);
        }
        ids.sort_by_key(|id| *id.as_uuid());

        let first = repo.scan_unhashed_materials(None, 2).await.unwrap();
        assert_eq!(
            first.iter().map(|m| m.asset_id).collect::<Vec<_>>(),
            ids[..2].to_vec(),
            "page one, in id order"
        );
        assert_eq!(
            first[0].locator,
            loc(format!("/pics/{}.png", {
                // The locator travels with the row so the hasher does not
                // need a second lookup.
                let expected = ids[0];
                (0..3)
                    .find(|n| {
                        first.iter().any(|m| {
                            m.asset_id == expected
                                && m.locator.to_display().ends_with(&format!("{n}.png"))
                        })
                    })
                    .unwrap()
            }))
        );

        let second = repo
            .scan_unhashed_materials(Some((&ids[1], 0)), 2)
            .await
            .unwrap();
        assert_eq!(
            second.iter().map(|m| m.asset_id).collect::<Vec<_>>(),
            ids[2..].to_vec(),
            "the cursor resumes past what was already seen"
        );

        repo.set_material_fingerprint(&ids[0], 0, &file_axis("sha256:cccc"))
            .await
            .unwrap();
        assert_eq!(repo.unhashed_material_count().await.unwrap(), 2);
        let after = repo.scan_unhashed_materials(None, 10).await.unwrap();
        assert_eq!(
            after.iter().map(|m| m.asset_id).collect::<Vec<_>>(),
            ids[1..].to_vec(),
            "a hashed material leaves the work list"
        );
    }

    /// Teeth for the trap this whole subtask is shaped around: a
    /// material whose content axis is answered by a **marker** leaves
    /// the walk.
    ///
    /// The failure it guards is not hypothetical — it is the one the
    /// file axis already hit and fixed (`UNHASHABLE`, V41). A predicate
    /// that asks for a *digest* rather than an *answer* keeps handing
    /// back every artefact that can never have one: the walk never
    /// shrinks, the same files are re-read on every launch forever, and
    /// the "still fingerprinting" notice never clears.
    ///
    /// Every marker is exercised, including the one only the migration
    /// writes, because they are one class and reading them as a class
    /// is what makes a marker added later work without an edit here.
    #[tokio::test]
    async fn a_marker_on_the_content_axis_takes_the_row_out_of_the_walk() {
        use asterism_core::domain::content_region;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = content_hash::of_bytes(b"the whole file");
        let region = format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64));
        let answers = [
            region.as_str(),
            content_region::EMPTY_SPAN,
            content_region::TOO_LARGE,
            content_region::NOT_WALKED,
            "unsupported:video/mp4",
            UNHASHABLE,
        ];

        let mut ids = Vec::new();
        for (n, _) in answers.iter().enumerate() {
            let locator = format!("/pics/answered-{n}.png");
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), &locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            );
            asset.materials = vec![asterism_core::domain::material::Material::primary(
                loc(&locator),
                Some(1),
                chrono::Utc::now(),
            )];
            repo.save(&asset).await.unwrap();
            ids.push(asset.id);
        }

        // Every row is work before anything is written — otherwise the
        // assertion below would hold over rows that were never in the
        // walk to begin with.
        assert_eq!(
            repo.unhashed_material_count().await.unwrap(),
            answers.len() as u64
        );

        for (id, answer) in ids.iter().zip(answers) {
            repo.set_material_fingerprint(
                id,
                0,
                &MaterialFingerprint {
                    file: digest.clone(),
                    content: answer.to_string(),
                    // This test is about the content column's
                    // vocabulary, so the third column is held at one
                    // value that is an answer on any axis. Writing
                    // `answer` here would put a `cr1-` digest in the
                    // meta column, where it is not an answer, and the
                    // count below would be measuring that instead.
                    meta: content_region::EMPTY_SPAN.to_string(),
                    meta_kv: None,
                    meta_raw: None,
                    meta_text: None,
                },
            )
            .await
            .unwrap();
        }

        assert_eq!(
            repo.unhashed_material_count().await.unwrap(),
            0,
            "a marker is an answer; a walk that disagrees never shrinks"
        );
        assert!(
            repo.scan_unhashed_materials(None, 50)
                .await
                .unwrap()
                .is_empty()
        );

        // …and a value that is *not* an answer comes straight back —
        // so the test above is measuring the predicate rather than a
        // walk that stopped returning anything at all.
        let stale = format!("cr0-sha256:{}", "b".repeat(64));
        isle.call({
            let uuid = *ids[0].as_uuid();
            let stale = stale.clone();
            move |conn| {
                conn.execute(
                    "UPDATE material SET content_region_hash = ?2 WHERE asset_id = ?1",
                    params![uuid, stale],
                )?;
                Ok(())
            }
        })
        .await
        .unwrap();
        assert_eq!(repo.unhashed_material_count().await.unwrap(), 1);
    }

    /// Teeth: the count and the scan describe the same set of rows.
    ///
    /// Two statements, one rule. The way this breaks is that somebody
    /// fixes one of them — and neither direction is visible from the
    /// outside: a count that is larger leaves a progress notice that
    /// never clears, a count that is smaller reports "done" while the
    /// walk is still chaining pages over work it will not admit to.
    ///
    /// The fixture puts a row in each state the two columns can be in,
    /// including the half-written shape a pre-column database has.
    #[tokio::test]
    async fn the_count_and_the_scan_answer_the_same_rows() {
        use asterism_core::domain::content_region;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = content_hash::of_bytes(b"the whole file");
        let region = format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64));
        let meta = format!("{}{}", content_hash::META_DIGEST_PREFIX, "a".repeat(64));
        /// One fixture row: label, the three columns as stored, and
        /// whether the walk should call it work.
        type ColumnState<'a> = (
            &'a str,
            Option<&'a str>,
            Option<&'a str>,
            Option<&'a str>,
            bool,
        );
        let states: [ColumnState<'_>; 8] = [
            ("nothing written", None, None, None, true),
            (
                "file only — the pre-column shape",
                Some(&digest),
                None,
                None,
                true,
            ),
            ("content only", None, Some(&region), None, true),
            (
                "all answered",
                Some(&digest),
                Some(&region),
                Some(&meta),
                false,
            ),
            (
                "answered by a marker",
                Some(&digest),
                Some(content_region::NOT_WALKED),
                Some(content_region::NOT_WALKED),
                false,
            ),
            (
                "an earlier region definition",
                Some(&digest),
                Some("cr0-sha256:beef"),
                Some(&meta),
                true,
            ),
            // The two shapes a build that predates the meta columns
            // leaves behind, and the one that predates their current
            // generation. Without these the fixture would agree with a
            // condition that never reads the third column.
            (
                "file and content only — the pre-meta-column shape",
                Some(&digest),
                Some(&region),
                None,
                true,
            ),
            (
                "an earlier meta definition",
                Some(&digest),
                Some(&region),
                Some("m0-sha256:beef"),
                true,
            ),
        ];

        let mut expected = Vec::new();
        for (label, file, content, meta) in states.iter().map(|(l, f, c, m, _)| (l, f, c, m)) {
            let locator = format!("/pics/{}.png", label.replace(' ', "-"));
            let mut asset = Asset::new(
                persona,
                SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), &locator).unwrap(),
                None,
                chrono::Utc::now(),
                &nobody(),
            );
            asset.materials = vec![asterism_core::domain::material::Material::primary(
                loc(&locator),
                Some(1),
                chrono::Utc::now(),
            )];
            repo.save(&asset).await.unwrap();
            // Written with SQL rather than through the verb, because
            // the verb refuses to produce half of these shapes — which
            // is the point of it, and also why the predicate still has
            // to be right about them.
            let uuid = *asset.id.as_uuid();
            let (file, content, meta) = (
                file.map(str::to_string),
                content.map(str::to_string),
                meta.map(str::to_string),
            );
            isle.call(move |conn| {
                conn.execute(
                    "UPDATE material SET content_hash = ?2, content_region_hash = ?3, \
                                         meta_hash = ?4 \
                      WHERE asset_id = ?1",
                    params![uuid, file, content, meta],
                )?;
                Ok(())
            })
            .await
            .unwrap();
            expected.push(asset.id);
        }

        let work: Vec<AssetId> = expected
            .iter()
            .zip(states.iter())
            .filter(|(_, (_, _, _, _, is_work))| *is_work)
            .map(|(id, _)| *id)
            .collect();

        let scanned: std::collections::HashSet<AssetId> = repo
            .scan_unhashed_materials(None, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.asset_id)
            .collect();
        assert_eq!(
            scanned,
            work.iter()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            "the scan picked a different set than the fixture calls work"
        );
        assert_eq!(
            repo.unhashed_material_count().await.unwrap(),
            work.len() as u64,
            "the count and the scan have to be the same statement"
        );
        // Not vacuous in either direction.
        assert!(!work.is_empty() && work.len() < expected.len());
    }

    /// Teeth: one call fills every column, so no reader ever sees a row
    /// with one axis answered and another not — including the meta
    /// object, which must not arrive after the digest it is the body of.
    ///
    /// Written as a check on the row after a single call — the window
    /// several `UPDATE`s would open is not observable from the outside,
    /// so what is pinned instead is the state the verb leaves behind,
    /// plus the fact that a half-filled row is work (which is what makes
    /// the window recoverable rather than permanent if it ever reopens).
    #[tokio::test]
    async fn one_write_answers_every_axis() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let locator = "/pics/one-write.png";
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        asset.materials = vec![asterism_core::domain::material::Material::primary(
            loc(locator),
            Some(1),
            chrono::Utc::now(),
        )];
        repo.save(&asset).await.unwrap();

        let before = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(before.materials[0].content_hash, None);
        assert_eq!(before.materials[0].content_region_hash, None);
        assert_eq!(before.materials[0].meta_hash, None);
        assert_eq!(before.materials[0].meta_kv, None);

        let canonical = r#"{"prompt":"a cat","workflow":"{}"}"#;
        let raw = "undefined:AAAAC3RFWHRwcm9tcHQ=";
        let fingerprint = MaterialFingerprint {
            file: content_hash::of_bytes(b"the whole file"),
            content: format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64)),
            meta: asterism_core::domain::material_meta::digest_of(canonical),
            meta_kv: Some(canonical.to_string()),
            meta_raw: Some(raw.to_string()),
            meta_text: None,
        };
        repo.set_material_fingerprint(&asset.id, 0, &fingerprint)
            .await
            .unwrap();

        let after = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(
            after.materials[0].content_hash.as_deref(),
            Some(fingerprint.file.as_str())
        );
        assert_eq!(
            after.materials[0].content_region_hash.as_deref(),
            Some(fingerprint.content.as_str()),
            "the second column has to arrive with the first, and to be read back"
        );
        assert_eq!(
            after.materials[0].meta_hash.as_deref(),
            Some(fingerprint.meta.as_str()),
            "and so does the third"
        );
        assert_eq!(
            after.materials[0].meta_kv.as_deref(),
            Some(canonical),
            "the object the digest was taken over travels in the same statement"
        );
        // The convenience form a consumer holding an `Asset` reads —
        // derived from the column above, never stored a second time.
        assert_eq!(
            after
                .material_meta()
                .and_then(|fields| fields.get("prompt").cloned()),
            Some("a cat".to_string())
        );
        // The fourth column the verb writes. Read off the row rather
        // than off the entity, because nothing hydrates it into one:
        // `MaterialRow` is loaded by `find` and by the ingest lookup,
        // and a payload that can reach a megabyte a material has no
        // consumer on either path yet (see V75).
        let stored: Option<String> = isle
            .call({
                let uuid = *asset.id.as_uuid();
                move |conn| {
                    conn.query_row(
                        "SELECT meta_raw FROM material WHERE asset_id = ?1 AND ord = 0",
                        params![uuid],
                        |row| row.get(0),
                    )
                }
            })
            .await
            .unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(raw),
            "the bytes the rendering was made from travel in the same statement as it"
        );

        assert!(
            !content_hash::needs_fingerprint(
                after.materials[0].content_hash.as_deref(),
                after.materials[0].content_region_hash.as_deref(),
                after.materials[0].meta_hash.as_deref(),
            ),
            "the row the verb left behind is not work"
        );
    }

    /// Teeth for the column the recovered text is stored in: what the
    /// walk found has to survive the round trip, and its three states
    /// have to stay distinguishable through it.
    ///
    /// The write path computes `meta_text` and the read path hands it to
    /// `derive_text`; between them sits one column, and a column that is
    /// written but never selected (or selected under a neighbour's name)
    /// makes the whole recovery a value that is computed and dropped.
    /// That is the state this file was in before the column existed, and
    /// it is invisible to every test that only checks the digests.
    #[tokio::test]
    async fn recovered_text_survives_the_round_trip_in_all_three_states() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let locator = "/pics/recovered.png";
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        asset.materials = vec![asterism_core::domain::material::Material::primary(
            loc(locator),
            Some(1),
            chrono::Utc::now(),
        )];
        repo.save(&asset).await.unwrap();

        // State one: nobody has looked.
        let before = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(before.materials[0].meta_text, None);

        // State three: the words. Written beside a meta digest that does
        // *not* contain them — the two columns are two readings of the
        // same chunks, and this is the pair that shows they are stored
        // separately rather than one being derived from the other.
        let recovered = r#"{"comment":"Café window","parameters":"a lighthouse"}"#;
        let digest_body = r#"{"Software":"a generator"}"#;
        repo.set_material_fingerprint(
            &asset.id,
            0,
            &MaterialFingerprint {
                file: content_hash::of_bytes(b"the whole file"),
                content: format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64)),
                meta: asterism_core::domain::material_meta::digest_of(digest_body),
                meta_kv: Some(digest_body.to_string()),
                meta_raw: None,
                meta_text: Some(recovered.to_string()),
            },
        )
        .await
        .unwrap();

        let after = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(
            after.materials[0].meta_text.as_deref(),
            Some(recovered),
            "the recovered text is read back under its own name"
        );
        assert_eq!(
            after.materials[0].meta_kv.as_deref(),
            Some(digest_body),
            "and the digest's body is still its own column"
        );

        // State two: read, and these bytes carry no words. Distinct from
        // state one, which is what keeps a text-free picture out of every
        // later pass.
        repo.set_material_fingerprint(
            &asset.id,
            0,
            &MaterialFingerprint {
                file: content_hash::of_bytes(b"the whole file"),
                content: format!("{}{}", content_hash::CONTENT_DIGEST_PREFIX, "a".repeat(64)),
                meta: asterism_core::domain::material_meta::digest_of(digest_body),
                meta_kv: Some(digest_body.to_string()),
                meta_raw: None,
                meta_text: Some("{}".to_string()),
            },
        )
        .await
        .unwrap();
        let emptied = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(
            emptied.materials[0].meta_text.as_deref(),
            Some("{}"),
            "'looked and found nothing' is an answer, not an absence"
        );
    }

    /// Teeth for the half of the backfill predicate that was missing: a
    /// row with a body composed by an **older reading** is work.
    ///
    /// The fixture is the exact shape the defect had. Both assets are
    /// text and both already have a cached body, so the anti-join alone
    /// returns neither — which is what made a transcript's own title,
    /// keywords and comment thread invisible to the walk while every
    /// picture was found. What separates them here is only
    /// `derived_version`, so a scan that went back to the old predicate
    /// fails on the row it is supposed to find rather than on a count.
    #[tokio::test]
    async fn a_body_composed_by_an_older_reading_is_still_work() {
        use asterism_core::domain::derived_text::COMPOSITION_VERSION;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let stale = item(persona, "/notes/composed-by-an-older-build.md");
        let current = item(persona, "/notes/composed-by-this-build.md");
        for asset in [&stale, &current] {
            repo.save(asset).await.unwrap();
        }

        let stale_id = *stale.id.as_uuid();
        let current_id = *current.id.as_uuid();
        isle.call(move |conn| {
            // The row a build without derived text left: a body, and no
            // statement about what composed it.
            conn.execute(
                "INSERT INTO asset_body (asset_id, body_text, body_bytes, indexed_at) \
                 VALUES (?1, 'the file bytes, and nothing about the row', 39, 0)",
                rusqlite::params![stale_id],
            )?;
            conn.execute(
                "INSERT INTO asset_body \
                 (asset_id, body_text, body_bytes, indexed_at, derived_version) \
                 VALUES (?1, 'composed from everything the row says', 37, 0, ?2)",
                rusqlite::params![current_id, COMPOSITION_VERSION],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let page = repo.scan_stale_body(None, 10).await.unwrap();
        assert_eq!(
            page.iter().map(|row| row.asset_id).collect::<Vec<_>>(),
            vec![stale.id],
            "the unstamped body is work and the current one is not"
        );

        // And the walk terminates: re-composing the row takes it out of
        // the set, which is what makes the chained backfill stop.
        use asterism_core::domain::repository::AssetBodyRepository;
        let body = crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone());
        body.upsert(&stale.id, "composed from everything the row says")
            .await
            .unwrap();
        assert!(
            repo.scan_stale_body(None, 10).await.unwrap().is_empty(),
            "a re-composed row leaves the set"
        );
    }

    /// Teeth: the per-asset pass's skip test reads **both** columns.
    ///
    /// The row it has to keep working on is the one a build without the
    /// content column left behind: a file digest, and nothing on the
    /// other axis. A skip test still asking `content_hash.is_some()`
    /// would walk past it forever, and the walk would keep handing it
    /// back — the two would disagree permanently over the same row.
    #[tokio::test]
    async fn a_row_answered_on_one_axis_only_is_still_work_for_the_per_asset_pass() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let locator = "/pics/pre-column.png";
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        asset.materials = vec![asterism_core::domain::material::Material::primary(
            loc(locator),
            Some(1),
            chrono::Utc::now(),
        )];
        repo.save(&asset).await.unwrap();

        // The shape a database written before the content column has.
        let uuid = *asset.id.as_uuid();
        let digest = content_hash::of_bytes(b"read by an older build");
        let stored = digest.clone();
        isle.call(move |conn| {
            conn.execute(
                "UPDATE material SET content_hash = ?2, content_region_hash = NULL \
                  WHERE asset_id = ?1",
                params![uuid, stored],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let hydrated = repo.find(&asset.id).await.unwrap().unwrap();
        let material = &hydrated.materials[0];
        assert_eq!(material.content_hash.as_deref(), Some(digest.as_str()));
        assert_eq!(material.content_region_hash, None);
        assert_eq!(material.meta_hash, None);
        // This is the per-asset job's test, verbatim.
        assert!(
            content_hash::needs_fingerprint(
                material.content_hash.as_deref(),
                material.content_region_hash.as_deref(),
                material.meta_hash.as_deref(),
            ),
            "a row answered on one axis only is still work"
        );
        // …and the walk agrees with it, which is the half that makes
        // the disagreement impossible rather than merely unlikely.
        assert_eq!(repo.unhashed_material_count().await.unwrap(), 1);
    }

    /// A page boundary that cuts through a multi-material asset must
    /// not lose the remaining `ord > 0` rows: the cursor compares the
    /// composite `(asset_id, ord)` key, not the asset id alone. An
    /// id-only cursor resumed "strictly after the last asset", which
    /// silently skipped the second half of a RAW+JPEG-style pair.
    #[tokio::test]
    async fn unhashed_scan_cursor_does_not_skip_ords_cut_by_a_page_boundary() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/pics/pair.raw").unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        let mut secondary = asterism_core::domain::material::Material::primary(
            loc("/pics/pair.jpg"),
            Some(1),
            chrono::Utc::now(),
        );
        secondary.ord = 1;
        asset.materials = vec![
            asterism_core::domain::material::Material::primary(
                loc("/pics/pair.raw"),
                Some(1),
                chrono::Utc::now(),
            ),
            secondary,
        ];
        repo.save(&asset).await.unwrap();

        // Page size 1 cuts between ord 0 and ord 1 of the same asset.
        let first = repo.scan_unhashed_materials(None, 1).await.unwrap();
        assert_eq!(
            (first[0].asset_id, first[0].ord),
            (asset.id, 0),
            "page one ends mid-asset"
        );

        let second = repo
            .scan_unhashed_materials(Some((&first[0].asset_id, first[0].ord)), 1)
            .await
            .unwrap();
        assert_eq!(
            second
                .iter()
                .map(|m| (m.asset_id, m.ord))
                .collect::<Vec<_>>(),
            vec![(asset.id, 1)],
            "the composite cursor picks up the ord the boundary cut through"
        );
    }

    /// `save` must not carry a stale palette back over an extraction.
    /// The realistic sequence is a read-modify-write on metadata
    /// (`find` → edit → `save`) racing the `thumb_gen` job: if the
    /// upsert wrote `palette` it would erase the extraction, leaving
    /// the derived swatches describing a palette that no longer exists.
    #[tokio::test]
    async fn save_does_not_clobber_a_palette_written_underneath_it() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/pics/a.png").unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        repo.save(&asset).await.unwrap();

        // Hydrated before the extraction lands — palette is still None.
        let mut stale = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(stale.palette, None);

        repo.set_palette(&asset.id, Some(vec!["#ff0000".into()]))
            .await
            .unwrap();

        // The metadata edit saves the entity it read a moment ago.
        stale.title = Some("renamed".into());
        repo.save(&stale).await.unwrap();

        let after = repo.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(after.title.as_deref(), Some("renamed"), "the edit landed");
        assert_eq!(
            after.palette,
            Some(vec!["#ff0000".to_string()]),
            "the extraction survived the save"
        );
        assert_eq!(
            repo.counts_by_color(Some(&persona), TrashFilter::LiveOnly)
                .await
                .unwrap(),
            vec![(ColorBucket::Red, 1)],
            "so the swatch still describes a palette that exists"
        );
    }

    #[test]
    fn query_parts_honours_the_trash_side() {
        let trashed = QueryParts::build(&AssetQuery {
            trash: TrashFilter::TrashedOnly,
            ..AssetQuery::default()
        });
        assert!(
            trashed.where_sql.contains("asset.trashed_at IS NOT NULL"),
            "trash view must select stamped rows, got: {}",
            trashed.where_sql
        );

        let any = QueryParts::build(&AssetQuery {
            trash: TrashFilter::Any,
            ..AssetQuery::default()
        });
        assert!(
            !any.where_sql.contains("trashed_at"),
            "`Any` must not constrain the trash side, got: {}",
            any.where_sql
        );
    }

    /// The trash clause must compose with the other predicates rather
    /// than replace them — a persona-scoped trash view still has to be
    /// persona-scoped.
    #[test]
    fn trash_clause_composes_with_other_filters() {
        let parts = QueryParts::build(&AssetQuery {
            persona_id: Some(PersonaId::new()),
            trash: TrashFilter::TrashedOnly,
            ..AssetQuery::default()
        });
        assert!(parts.where_sql.contains("asset.trashed_at IS NOT NULL"));
        assert!(parts.where_sql.contains("persona_id = ?"));
        assert!(parts.where_sql.contains(" AND "));
        assert_eq!(parts.params.len(), 1, "only the persona param is bound");
    }

    // ---- the fold axis (V49) ------------------------------------
    //
    // The rule under test is one sentence: paths that enumerate drop a
    // headstone, paths that name one keep it. Each disappearing path
    // gets its own test rather than one combined assertion, so a
    // regression names the path that lost the filter.
    //
    // Every fixture below folds a row that was **listed a moment
    // earlier in the same test**. That ordering is the point: an
    // assertion that only ever sees the post-fold set would pass just
    // as well over rows the query excluded for some unrelated reason,
    // and would keep passing if the fold filter were deleted.

    /// Stands a headstone the way the fold verb will once it exists.
    ///
    /// There is no setter yet (a later P2 subtask owns it) and `save`
    /// is deliberately unable to write the column, so the fixture goes
    /// straight at the row. The row count is asserted here rather than
    /// at the call sites: a fixture that silently marked nothing would
    /// make every test below pass for the wrong reason.
    async fn fold_into(isle: &AsyncIsle, headstone: &AssetId, keeper: &AssetId) {
        let (headstone, keeper) = (*headstone.as_uuid(), *keeper.as_uuid());
        isle.call(move |conn| {
            let marked = conn.execute(
                "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
                params![headstone, keeper],
            )?;
            assert_eq!(marked, 1, "the fixture must actually stand a headstone");
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Seeds one plain item with a material, so the format facet and
    /// the duplicate report have something to work with.
    ///
    /// `occurred_at` is a **fixed** instant, not `Utc::now()`: two items
    /// built a microsecond apart may or may not land in the same
    /// millisecond, and the fold report counts the columns the two rows
    /// disagree about — a clock-dependent count would make that number
    /// flap. Tests that care about order set the field themselves
    /// (`hashed_item`, `at`).
    fn item(persona: PersonaId, locator: &str) -> Asset {
        item_by(persona, locator, &nobody())
    }

    /// `item`, with the attribution stated — for the tests that are
    /// about what a fold does to somebody's assertion. The attribution
    /// is a constructor argument rather than a field to assign, so a
    /// fixture that needs one has to say which entry point it stands in
    /// for (here: a caller stating its own).
    fn item_by(persona: PersonaId, locator: &str, attribution: &AttributionContext) -> Asset {
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            Some(Modality::new("tape").unwrap()),
            at(1_700_000_000_000),
            attribution,
        );
        let mut material =
            asterism_core::domain::material::Material::primary(loc(locator), Some(1), Utc::now());
        material.mime = Some(MimeType::parse("image/png"));
        asset.materials = vec![material];
        asset
    }

    /// The grid, the index projection and the search filter all read
    /// through one `WHERE` builder, so the fold rule is stated once —
    /// and this test is what says the builder actually carries it.
    #[tokio::test]
    async fn a_folded_row_leaves_the_grid_the_index_and_the_search_filter() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy-of-keeper.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        // Both are ordinary cards while nothing is folded — without
        // this the assertions below would hold over a set that never
        // contained the row.
        let before = repo.list(&AssetQuery::default()).await.unwrap();
        assert_eq!(before.items.len(), 2);
        assert_eq!(before.total, Some(2));

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let grid = repo.list(&AssetQuery::default()).await.unwrap();
        assert_eq!(
            grid.items.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![keeper.id],
            "the headstone is not a card"
        );
        assert_eq!(
            grid.total,
            Some(1),
            "the page total counts what the page shows"
        );

        let index = repo.list_index(&AssetQuery::default()).await.unwrap();
        assert_eq!(
            index.items.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![keeper.id],
            "the index projection reads the same population"
        );

        // The search path intersects Tantivy candidates with the SQL
        // filter through this call. A stale document naming the folded
        // row has to be dropped here even if the index still holds it.
        let kept = repo
            .filter_ids(&[keeper.id, headstone.id], &AssetQuery::default())
            .await
            .unwrap();
        assert_eq!(kept, vec![keeper.id], "a search hit cannot resurrect it");
    }

    /// A headstone is not trash, so neither trash-side view shows it —
    /// including `Any`, the "whole table" diagnostic. The two axes are
    /// independent, and this is the test that says so: the folded row
    /// here also carries a trash stamp, which is the state that would
    /// leak if the fold filter rode on the trash filter.
    #[tokio::test]
    async fn the_trash_views_do_not_show_a_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        let plain_trash = item(persona, "/pics/discarded.png");
        for asset in [&keeper, &headstone, &plain_trash] {
            repo.save(asset).await.unwrap();
        }
        repo.trash(&headstone.id, Utc::now()).await.unwrap();
        repo.trash(&plain_trash.id, Utc::now()).await.unwrap();

        let trashed_before = repo
            .list(&AssetQuery {
                trash: TrashFilter::TrashedOnly,
                ..AssetQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(
            trashed_before.items.len(),
            2,
            "both stamped rows are in the trash view before the fold"
        );

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let trashed = repo
            .list(&AssetQuery {
                trash: TrashFilter::TrashedOnly,
                ..AssetQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(
            trashed.items.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![plain_trash.id],
            "a folded row is not something to restore"
        );

        let any = repo
            .list(&AssetQuery {
                trash: TrashFilter::Any,
                ..AssetQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(
            any.items
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([keeper.id, plain_trash.id]),
            "`Any` widens the trash axis, not the fold axis"
        );
    }

    /// Every sidebar facet counts the population `GRID_POPULATION`
    /// names, so folding a row has to move all four numbers at once.
    /// Facets drifting apart from each other and from the grid is the
    /// documented failure this constant exists to prevent.
    #[tokio::test]
    async fn a_folded_row_leaves_every_sidebar_facet() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        for asset in [&keeper, &headstone] {
            repo.save(asset).await.unwrap();
            repo.set_palette(&asset.id, Some(vec!["#ff0000".into()]))
                .await
                .unwrap();
        }

        let facets = |repo: SqliteAssetRepository| async move {
            (
                repo.counts_by_persona(TrashFilter::LiveOnly).await.unwrap(),
                repo.counts_by_modality(Some(&persona), TrashFilter::LiveOnly)
                    .await
                    .unwrap(),
                repo.counts_by_format(Some(&persona), TrashFilter::LiveOnly)
                    .await
                    .unwrap(),
                repo.counts_by_color(Some(&persona), TrashFilter::LiveOnly)
                    .await
                    .unwrap(),
            )
        };

        let before = facets(repo.clone()).await;
        assert_eq!(before.0, vec![(persona, 2)]);
        assert_eq!(before.1, vec![("tape".to_string(), 2)]);
        assert_eq!(before.2, vec![("image".to_string(), 2)]);
        assert_eq!(before.3, vec![(ColorBucket::Red, 2)]);

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let after = facets(repo.clone()).await;
        assert_eq!(after.0, vec![(persona, 1)], "PERSONA");
        assert_eq!(after.1, vec![("tape".to_string(), 1)], "MODALITY");
        assert_eq!(after.2, vec![("image".to_string(), 1)], "FORMAT");
        assert_eq!(after.3, vec![(ColorBucket::Red, 1)], "COLOUR");
        assert_eq!(
            repo.list(&AssetQuery::default()).await.unwrap().items.len(),
            1,
            "and the grid they describe agrees"
        );
    }

    /// A fold is the answer to a duplicate, so the pair stops being
    /// reported — and stops *occupying the report*.
    ///
    /// The second half is what gives the report's first step
    /// (`GROUP BY … HAVING COUNT(*) > 1`, which picks the hashes)
    /// teeth of its own. Without a limit that step is unobservable
    /// through this port: the member query drops the folded row anyway
    /// and the `rows.len() > 1` guard then discards the one-member
    /// group, so the answer looks the same either way. Under a limit
    /// it stops looking the same — a resolved pair would take a slot
    /// and push a real, unresolved duplicate out of the answer
    /// entirely, which reads to the user as "no duplicates" while two
    /// copies sit in the library. The resolved pair is the *newer* one
    /// here precisely because the hash step orders by
    /// `MAX(occurred_at) DESC`.
    #[tokio::test]
    async fn the_duplicate_report_forgets_a_resolved_pair_and_frees_its_slot() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let older = Utc::now() - chrono::Duration::days(2);
        let newer = Utc::now();
        let at = |locator: &str, when: chrono::DateTime<Utc>| {
            let mut asset = item(persona, locator);
            asset.occurred_at = when;
            asset
        };

        // The pair that will be resolved — the newest thing in the
        // library, so it sorts to the front of the report.
        let keeper = at("/pics/a.png", newer);
        let headstone = at("/pics/copy-of-a.png", newer);
        // …and a genuine, untouched duplicate behind it.
        let (other_a, other_b) = (at("/pics/b.png", older), at("/pics/copy-of-b.png", older));
        for (asset, hash) in [
            (&keeper, "sha256:aaaa"),
            (&headstone, "sha256:aaaa"),
            (&other_a, "sha256:bbbb"),
            (&other_b, "sha256:bbbb"),
        ] {
            repo.save(asset).await.unwrap();
            repo.set_material_fingerprint(&asset.id, 0, &file_axis(hash))
                .await
                .unwrap();
        }

        let before = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 50)
            .await
            .unwrap();
        assert_eq!(before.len(), 2, "two findings before the fold");
        assert_eq!(
            before[0].content_hash, "sha256:aaaa",
            "the newer pair is reported first"
        );

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let all = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 50)
            .await
            .unwrap();
        assert_eq!(
            all.iter()
                .map(|g| g.content_hash.as_str())
                .collect::<Vec<_>>(),
            vec!["sha256:bbbb"],
            "a resolved duplicate is not still a duplicate"
        );

        let capped = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 1)
            .await
            .unwrap();
        assert_eq!(
            capped
                .iter()
                .map(|g| g.content_hash.as_str())
                .collect::<Vec<_>>(),
            vec!["sha256:bbbb"],
            "and it does not spend the one slot the caller asked for, \
             leaving a real duplicate unreported"
        );
    }

    /// The report's second step fetches the members of a surviving
    /// group, and has its own copy of the rule. Three copies of one
    /// file with one folded is the fixture that separates the two: the
    /// group is still a finding, so only the member query can drop the
    /// headstone.
    #[tokio::test]
    async fn the_duplicate_report_keeps_the_group_and_drops_the_folded_member() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/a.png");
        let other = item(persona, "/pics/a-elsewhere.png");
        let headstone = item(persona, "/pics/copy-of-a.png");
        for asset in [&keeper, &other, &headstone] {
            repo.save(asset).await.unwrap();
            repo.set_material_fingerprint(&asset.id, 0, &file_axis("sha256:aaaa"))
                .await
                .unwrap();
        }

        let before = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 50)
            .await
            .unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].members.len(), 3, "three copies before the fold");

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let groups = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 50)
            .await
            .unwrap();
        assert_eq!(groups.len(), 1, "two live copies are still a finding");
        assert_eq!(
            groups[0]
                .members
                .iter()
                .map(|c| c.id)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([keeper.id, other.id]),
            "but the resolved one is no longer on the work list"
        );
    }

    /// The full-text backfill decides what enters the search index. A
    /// headstone that got in would answer a query with a card the grid
    /// cannot show.
    #[tokio::test]
    async fn the_search_index_backfill_skips_a_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/notes/keeper.md");
        let headstone = item(persona, "/notes/copy.md");
        for asset in [&keeper, &headstone] {
            repo.save(asset).await.unwrap();
        }

        let before = repo.scan_stale_body(None, 10).await.unwrap();
        assert_eq!(before.len(), 2, "both are waiting to be indexed");

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let page = repo.scan_stale_body(None, 10).await.unwrap();
        assert_eq!(
            page.iter().map(|row| row.asset_id).collect::<Vec<_>>(),
            vec![keeper.id],
            "the headstone is not indexable material"
        );
    }

    /// The retention sweep is the only scheduled job that destroys
    /// rows, and a headstone is the one row that must survive
    /// indefinitely — every stale reference resolves through it. A
    /// folded row carrying a trash stamp (persona trash, or a hand
    /// trash that preceded the fold) is exactly the state that would
    /// otherwise be purged.
    #[tokio::test]
    async fn the_retention_scan_never_picks_up_a_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        let expired = item(persona, "/pics/discarded.png");
        for asset in [&keeper, &headstone, &expired] {
            repo.save(asset).await.unwrap();
        }
        let long_ago = Utc::now() - chrono::Duration::days(90);
        repo.trash(&headstone.id, long_ago).await.unwrap();
        repo.trash(&expired.id, long_ago).await.unwrap();

        let cutoff = Utc::now();
        let before = repo.scan_purgeable(cutoff, 10).await.unwrap();
        assert_eq!(
            before.len(),
            2,
            "both stamps are past the cutoff before the fold"
        );

        fold_into(&isle, &headstone.id, &keeper.id).await;

        assert_eq!(
            repo.scan_purgeable(cutoff, 10).await.unwrap(),
            vec![expired.id],
            "purging a headstone would break every reference that redirects through it"
        );
    }

    /// "Empty the trash" hands its list straight to `purge`, so it
    /// carries the same rule as the retention scan — and the same
    /// consequence for getting it wrong.
    #[tokio::test]
    async fn emptying_the_trash_never_picks_up_a_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        let discarded = item(persona, "/pics/discarded.png");
        for asset in [&keeper, &headstone, &discarded] {
            repo.save(asset).await.unwrap();
        }
        repo.trash(&headstone.id, Utc::now()).await.unwrap();
        repo.trash(&discarded.id, Utc::now()).await.unwrap();

        assert_eq!(
            repo.list_trashed_ids(100).await.unwrap().len(),
            2,
            "both stamped rows are in the bin before the fold"
        );

        fold_into(&isle, &headstone.id, &keeper.id).await;

        assert_eq!(
            repo.list_trashed_ids(100).await.unwrap(),
            vec![discarded.id],
            "emptying the bin cannot delete a redirect"
        );
    }

    // The container half of the same rule. Everything below asks one
    // question — "who is inside this container right now?" — through a
    // different port, and the fixtures all disagree with the default on
    // exactly the fold axis: every container here holds a member that is
    // folded but still filed, which is the state `MEMBER_POPULATION`
    // exists for and the one a `trashed_at`-only predicate reports as
    // present.

    /// A composite the Sessions view will list.
    ///
    /// `external_key` is set because the projection reads that column
    /// into a non-optional `ExternalSessionKey`; a composite without one
    /// does not fail the filter, it fails the row mapping, which would
    /// make every assertion below an error about the fixture. Its own
    /// `occurred_at` is the fallback the derived window uses when
    /// nothing is inside it — deliberately earlier than every member, so
    /// a window that fell back would be visible rather than plausible.
    fn composite(persona: PersonaId, key: &str, occurred: chrono::DateTime<Utc>) -> Asset {
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new("session").unwrap(), key).unwrap(),
            Some(Modality::new("session").unwrap()),
            occurred,
            &nobody(),
        );
        asset.role = AssetRole::Collection;
        asset.external_key = Some(key.into());
        asset
    }

    /// A member filed inside `container`, carrying its own instant —
    /// the axis the container's time window is derived from.
    fn member(
        persona: PersonaId,
        container: &Asset,
        locator: &str,
        occurred: chrono::DateTime<Utc>,
    ) -> Asset {
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            Some(Modality::new("message").unwrap()),
            occurred,
            &nobody(),
        );
        asset.container_id = Some(container.id);
        asset
    }

    /// **The card's headline number.** A container that says "3 items"
    /// and opens on one is the shape this predicate is for.
    ///
    /// Three members, three states: one live, one trashed, one folded.
    /// The trashed one is what keeps the assertion from passing over a
    /// query that lost the trash half instead, and the count is read
    /// beside the drill-down that renders it — those two are what the
    /// user compares, so they are what the test compares.
    #[tokio::test]
    async fn a_folded_member_leaves_the_containers_headline_count() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let container = composite(persona, "sess.count", at(1_700_000_000_000));
        let kept = member(persona, &container, "/msgs/kept.md", at(1_700_000_060_000));
        let binned = member(
            persona,
            &container,
            "/msgs/binned.md",
            at(1_700_000_090_000),
        );
        let folded = member(
            persona,
            &container,
            "/msgs/folded.md",
            at(1_700_000_120_000),
        );
        for asset in [&container, &kept, &binned, &folded] {
            repo.save(asset).await.unwrap();
        }
        repo.trash(&binned.id, Utc::now()).await.unwrap();

        let headline = |repo: SqliteAssetRepository, id: AssetId| async move {
            repo.list(&AssetQuery::default())
                .await
                .unwrap()
                .items
                .iter()
                .find(|card| card.id == id)
                .expect("a container is a card like any other")
                .member_count
        };

        assert_eq!(
            headline(repo.clone(), container.id).await,
            2,
            "the folded row is still inside and still counted before the fold"
        );

        fold_into(&isle, &folded.id, &kept.id).await;

        assert_eq!(
            headline(repo.clone(), container.id).await,
            1,
            "a folded member is content of the keeper, not an item of this container"
        );
        let inside = repo
            .list(&AssetQuery {
                container_id: Some(container.id),
                ..AssetQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(
            inside.items.iter().map(|card| card.id).collect::<Vec<_>>(),
            vec![kept.id],
            "…which is what opening it shows, and the number has to be that number"
        );
    }

    /// **The Sessions tile.** Its count and its date range read the
    /// same population, so a fold moves both.
    ///
    /// The folded message is the *earliest* one on purpose: that is the
    /// row `MIN(occurred_at)` picks, so a window still derived from it
    /// would start the session at a message it no longer holds. The
    /// composite's own `occurred_at` is earlier still, which separates
    /// "the window moved to the next live member" from "the window fell
    /// back to the empty-composite case".
    #[tokio::test]
    async fn a_folded_message_leaves_the_sessions_tile_count_and_its_window() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let container = composite(persona, "sess.window", at(1_700_000_000_000));
        let opening = member(
            persona,
            &container,
            "/msgs/opening.md",
            at(1_700_000_060_000),
        );
        let reply = member(persona, &container, "/msgs/reply.md", at(1_700_000_120_000));
        for asset in [&container, &opening, &reply] {
            repo.save(asset).await.unwrap();
        }

        let tile = |repo: SqliteAssetRepository| async move {
            let page = repo.list_sessions(&AssetQuery::default()).await.unwrap();
            assert_eq!(page.items.len(), 1, "one composite, one tile");
            page.items.into_iter().next().unwrap()
        };

        let before = tile(repo.clone()).await;
        assert_eq!(before.message_count, 2);
        assert_eq!(
            before.started_at_ms, 1_700_000_060_000,
            "the window starts at the first message, not at the composite's seed"
        );

        fold_into(&isle, &opening.id, &reply.id).await;

        let after = tile(repo.clone()).await;
        assert_eq!(after.message_count, 1, "one message left inside");
        assert_eq!(
            after.started_at_ms, 1_700_000_120_000,
            "and the session begins at the message it still holds"
        );
        assert_eq!(after.ended_at_ms, 1_700_000_120_000);
    }

    /// **One session, one `message_count`, whichever route asks.**
    ///
    /// `GET /sessions/{id}` and `rename` answer through
    /// [`SessionRepository::find_by_id`], the listing answers through
    /// [`list_sessions`](AssetRepository::list_sessions). Until both
    /// counted [`MEMBER_POPULATION`] they disagreed by construction —
    /// the listing filtered its members and the by-id read did not, so
    /// a rename handed back a larger number than the list had just
    /// shown. The fixture holds one member of each kind so both halves
    /// of the disagreement are present at once.
    #[tokio::test]
    async fn a_session_reports_one_message_count_by_id_and_in_the_listing() {
        use asterism_core::domain::repository::SessionRepository as _;
        use asterism_core::domain::value::SessionId;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let sessions = crate::sqlite::repo::session::SqliteSessionRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let container = composite(persona, "sess.agree", at(1_700_000_000_000));
        let kept = member(persona, &container, "/msgs/kept.md", at(1_700_000_060_000));
        let binned = member(
            persona,
            &container,
            "/msgs/binned.md",
            at(1_700_000_090_000),
        );
        let folded = member(
            persona,
            &container,
            "/msgs/folded.md",
            at(1_700_000_120_000),
        );
        for asset in [&container, &kept, &binned, &folded] {
            repo.save(asset).await.unwrap();
        }
        repo.trash(&binned.id, Utc::now()).await.unwrap();
        fold_into(&isle, &folded.id, &kept.id).await;

        let id = SessionId::new(container.id.as_uuid().to_string()).unwrap();
        let by_id = sessions
            .find_by_id(&id)
            .await
            .unwrap()
            .expect("the composite is a session");
        let listed = repo
            .list_sessions(&AssetQuery::default())
            .await
            .unwrap()
            .items
            .into_iter()
            .next()
            .expect("and it is in the listing");

        assert_eq!(
            by_id.message_count, listed.message_count,
            "two routes to one session cannot answer with two numbers"
        );
        assert_eq!(
            by_id.message_count, 1,
            "and the number they agree on is the members that are still members"
        );
        assert_eq!(
            (by_id.started_at_ms, by_id.ended_at_ms),
            (listed.started_at_ms, listed.ended_at_ms),
            "the window is derived from the same population, so it agrees too"
        );
    }

    /// **A folded Session leaves the listing and its total.**
    ///
    /// `MergePlan::declare` has no `role` check, so a `role =
    /// 'collection'` row can be folded like any other. Whether it
    /// should be is a question for `declare`; what this holds is that a
    /// composite that *has* been folded stops being a tile — including
    /// in the total beside it, which is the number a paginating client
    /// reads.
    #[tokio::test]
    async fn a_folded_collection_leaves_the_sessions_listing_and_its_total() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = composite(persona, "sess.keeper", at(1_700_000_000_000));
        let headstone = composite(persona, "sess.duplicate", at(1_700_000_030_000));
        for asset in [&keeper, &headstone] {
            repo.save(asset).await.unwrap();
        }

        let before = repo.list_sessions(&AssetQuery::default()).await.unwrap();
        assert_eq!(before.items.len(), 2, "two tiles before the fold");
        assert_eq!(before.total, Some(2));

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let after = repo.list_sessions(&AssetQuery::default()).await.unwrap();
        assert_eq!(
            after.items.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            vec![
                asterism_core::domain::value::SessionId::new(keeper.id.as_uuid().to_string())
                    .unwrap()
            ],
            "a folded composite is not a session to open"
        );
        assert_eq!(
            after.total,
            Some(1),
            "and the total says the same thing the page does"
        );
    }

    /// **The container's cover.** It is the earliest member's, so the
    /// fixture folds exactly that one.
    ///
    /// Both members carry a cover, which is what makes the answer move
    /// rather than vanish: a `None` here would also be produced by a
    /// query that broke outright.
    #[tokio::test]
    async fn a_folded_first_member_stops_being_the_containers_cover() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let container = composite(persona, "sess.cover", at(1_700_000_000_000));
        let opening = member(
            persona,
            &container,
            "/msgs/opening.md",
            at(1_700_000_060_000),
        );
        let reply = member(persona, &container, "/msgs/reply.md", at(1_700_000_120_000));
        repo.save(&container).await.unwrap();
        for (asset, cover) in [(&opening, "the opening line"), (&reply, "the later reply")] {
            repo.save(asset).await.unwrap();
            repo.set_cover(&asset.id, &CoverText::new(cover.to_string()).unwrap())
                .await
                .unwrap();
        }

        assert_eq!(
            repo.first_member_cover(&container.id).await.unwrap(),
            Some("the opening line".to_string()),
            "the earliest member titles the container before the fold"
        );

        fold_into(&isle, &opening.id, &reply.id).await;

        assert_eq!(
            repo.first_member_cover(&container.id).await.unwrap(),
            Some("the later reply".to_string()),
            "a container cannot read as a message that is no longer inside it"
        );
    }

    /// **The cover backfill's work list.** A headstone's `cover` stays
    /// NULL forever and nothing deletes the row, so a folded container
    /// left in this queue is not merely useless work — it is permanent
    /// work, taking a slot on every pass from a container whose cover
    /// somebody would see.
    #[tokio::test]
    async fn a_folded_container_leaves_the_cover_backfill_queue() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let waiting = composite(persona, "sess.waiting", at(1_700_000_000_000));
        let headstone = composite(persona, "sess.duplicate", at(1_700_000_030_000));
        for asset in [&waiting, &headstone] {
            repo.save(asset).await.unwrap();
        }

        assert_eq!(
            repo.containers_without_cover(10)
                .await
                .unwrap()
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([waiting.id, headstone.id]),
            "both are uncovered containers before the fold"
        );

        fold_into(&isle, &headstone.id, &waiting.id).await;

        assert_eq!(
            repo.containers_without_cover(10).await.unwrap(),
            vec![waiting.id],
            "covering a redirect produces nothing anybody can see"
        );
    }

    /// **The constellation around a card.** `candidates_near` feeds
    /// `plan_edges`, so a headstone left in it draws a second card for
    /// a picture already on screen — the very duplicate somebody
    /// resolved by folding it.
    #[tokio::test]
    async fn candidates_near_does_not_return_a_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let target = item(persona, "/pics/target.png");
        let neighbour = item(persona, "/pics/neighbour.png");
        let headstone = item(persona, "/pics/copy-of-neighbour.png");
        for asset in [&target, &neighbour, &headstone] {
            repo.save(asset).await.unwrap();
        }

        let near = |repo: SqliteAssetRepository, target: Asset| async move {
            repo.candidates_near(&target, 10)
                .await
                .unwrap()
                .into_iter()
                .map(|asset| asset.id)
                .collect::<std::collections::HashSet<_>>()
        };

        assert_eq!(
            near(repo.clone(), target.clone()).await,
            std::collections::HashSet::from([neighbour.id, headstone.id]),
            "both neighbours are in the window before the fold"
        );

        fold_into(&isle, &headstone.id, &neighbour.id).await;

        assert_eq!(
            near(repo.clone(), target.clone()).await,
            std::collections::HashSet::from([neighbour.id]),
            "the resolved copy is the keeper's content, not a neighbour of the target"
        );
    }

    /// **The two background sweeps.** Neither has a visible symptom
    /// today — nothing displays what they produce for a headstone — and
    /// that is why they are worth pinning: the next reader to surface
    /// either result would inherit the bug rather than write it.
    ///
    /// A headstone is never restored and never deleted, so a row left
    /// in one of these scans is not a wasted pass, it is a permanent
    /// resident of the work list.
    #[tokio::test]
    async fn the_provenance_and_dimension_sweeps_skip_a_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut keeper = item(persona, "/pics/keeper.png");
        let mut headstone = item(persona, "/pics/copy.png");
        for asset in [&mut keeper, &mut headstone] {
            asset.extra = serde_json::json!({ "_trace": { "resolved": false } });
            repo.save(asset).await.unwrap();
        }

        assert_eq!(
            repo.unresolved_provenance_ids(10)
                .await
                .unwrap()
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([keeper.id, headstone.id]),
            "both claims are pending before the fold"
        );
        assert_eq!(
            repo.scan_dims_candidates(DimsScope::Unlooked, None, 10)
                .await
                .unwrap()
                .len(),
            2,
            "and both rows are unlooked-at"
        );

        fold_into(&isle, &headstone.id, &keeper.id).await;

        assert_eq!(
            repo.unresolved_provenance_ids(10).await.unwrap(),
            vec![keeper.id],
            "a claim on a redirect names nothing a person can open"
        );
        assert_eq!(
            repo.scan_dims_candidates(DimsScope::Unlooked, None, 10)
                .await
                .unwrap()
                .into_iter()
                .map(|candidate| candidate.asset_id)
                .collect::<Vec<_>>(),
            vec![keeper.id],
            "and no card would ever display the measurement"
        );
    }

    /// **Teeth for the constant: a correlated read of a container's
    /// members that does not name [`MEMBER_POPULATION`] may not
    /// exist.**
    ///
    /// A shared definition only holds if the next query finds it. Nine
    /// of these subqueries were written in this crate before the
    /// constant existed, each one re-deriving "who is inside this
    /// container" from memory, and all nine remembered the trash and
    /// forgot the fold. Naming the rule fixed the nine. This is what
    /// fixes the tenth, which nobody is going to write next to the
    /// other nine.
    ///
    /// Read off the two files' own source rather than out of a
    /// database — the same trade `migrations::
    /// every_step_is_named_for_the_version_it_produces` makes: names
    /// rather than meanings, and no build dependency.
    ///
    /// Test modules are cut off first. A fixture's whole job is to
    /// create the row the production rule excludes, so holding it to
    /// that rule would leave it unable to set up the state the test
    /// asserts about.
    #[test]
    fn every_member_query_names_the_member_population() {
        /// The `container_id` comparisons this rule does **not**
        /// govern, each with the question it asks instead.
        ///
        /// Keyed by the statement rather than by a line number, so
        /// moving code does not move the exception, and adding one
        /// means naming the query and saying why. Adding an entry with
        /// no reason on it fails below — a list that takes a blank one
        /// is nine forgotten filters again, only written down.
        const EXCEPT: &[(&str, &str)] = &[
            (
                "SELECT COUNT(*) FROM asset WHERE container_id = ?1",
                "session::delete_if_empty asks the inverse question — is anything at all \
                 still pointing here — and a trashed or folded member is exactly what has \
                 to count, because deleting the composite would orphan it.",
            ),
            (
                "UPDATE asset SET container_id = ?2, updated_at = ?3 \
                 WHERE container_id = ?1 AND id <> ?2",
                "fold_one's own child re-point, which is the fold mechanism producing the \
                 state this rule describes rather than reading under it.",
            ),
            (
                "asset.container_id = ?",
                "QueryParts::build carries the fold axis (and the caller's trash axis) over \
                 the member table itself under the alias `asset`, so the drill-down states \
                 this rule in the alias its outer query already uses.",
            ),
        ];
        const NEEDLE: &str = "container_id = ";

        /// Everything before `mod tests`, with whitespace runs — the
        /// `\` line continuations these SQL literals are written with
        /// included — collapsed to one space, so a statement reads as
        /// one string whatever shape it has in the file.
        fn production(source: &str) -> String {
            let (before_tests, _) = source
                .split_once("#[cfg(test)]\nmod tests {")
                .expect("each file keeps its tests in one `mod tests`");
            let mut out = String::with_capacity(before_tests.len());
            let mut pending = false;
            for ch in before_tests.chars() {
                if ch.is_whitespace() || ch == '\\' {
                    pending = true;
                    continue;
                }
                if pending && !out.is_empty() {
                    out.push(' ');
                }
                pending = false;
                out.push(ch);
            }
            out
        }

        for (sql, reason) in EXCEPT {
            assert!(
                reason.split_whitespace().count() >= 8,
                "the exception for `{sql}` carries no reason. An exception is a sentence \
                 about which question that statement asks instead"
            );
        }

        let mut used = vec![false; EXCEPT.len()];
        let mut missing: Vec<String> = Vec::new();
        let mut governed = 0usize;

        for (file, source) in [
            ("repo/asset.rs", include_str!("asset.rs")),
            ("repo/session.rs", include_str!("session.rs")),
        ] {
            let text = production(source);
            let mut here = 0usize;
            let mut at = 0usize;
            while let Some(offset) = text[at..].find(NEEDLE) {
                let start = at + offset;
                let rest = &text[start + NEEDLE.len()..];
                at = start + NEEDLE.len();

                // Only a comparison inside a statement. `container_id =
                // Some(container.id)` is Rust, and `container_id =
                // excluded.container_id` is the upsert writing the
                // column rather than reading by it.
                let term: String = rest
                    .chars()
                    .take_while(|c| !matches!(c, ' ' | ',' | ')' | '"'))
                    .collect();
                let compares = term.starts_with('?')
                    || term.split_once('.').is_some_and(|(alias, column)| {
                        column == "id" && alias.chars().all(|c| c.is_alphanumeric() || c == '_')
                    });
                if !compares {
                    continue;
                }

                if let Some(index) = EXCEPT.iter().position(|(sql, _)| {
                    text.match_indices(sql)
                        .any(|(from, _)| from <= start && start < from + sql.len())
                }) {
                    used[index] = true;
                    continue;
                }

                here += 1;
                let tail = rest[term.len()..].trim_start();
                if !tail.starts_with("AND {MEMBER_POPULATION}") {
                    missing.push(format!(
                        "{file}: `… container_id = {term} {} …`",
                        &tail[..tail.len().min(48)]
                    ));
                }
            }
            assert!(
                here > 0,
                "{file} yielded no governed statement. The scan reading nothing is how \
                 this test goes quiet instead of red"
            );
            governed += here;
        }

        // A floor, not a fixture: it may rise when a member query is
        // added, and its job is to fail if the reader above ever stops
        // matching the way these statements are written.
        assert!(
            governed >= 8,
            "only {governed} governed statements found, against 8 when this was written"
        );
        for ((sql, _), used) in EXCEPT.iter().zip(used) {
            assert!(
                used,
                "nothing matches the exception for `{sql}` any more. A stale exception is \
                 a hole waiting for a query to be written back into it"
            );
        }
        assert!(
            missing.is_empty(),
            "a correlated read of a container's members that does not name \
             MEMBER_POPULATION. Splice the constant, or name the statement in the \
             exception list with the question it asks instead: {missing:#?}"
        );
    }

    /// The other half of the rule: a read that *names* a row returns
    /// it. This is not leniency — it is the mechanism. A stale
    /// `asset:<uuid>` claim resolves by id, and a `#fragment`
    /// reference or a re-import resolves by locator; both have to
    /// reach the headstone to learn where they were redirected.
    ///
    /// "Reach" is the word doing the work on the locator side. `Any`
    /// stops there, because its question is which row stands at the
    /// address. `Live` walks on — asserted here beside the `Any` read
    /// of the same locator, so neither answer can be read as the
    /// lookup having one behaviour.
    #[tokio::test]
    async fn a_headstone_stays_reachable_by_id_and_by_locator() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        for asset in [&keeper, &headstone] {
            repo.save(asset).await.unwrap();
        }
        fold_into(&isle, &headstone.id, &keeper.id).await;

        let by_id = repo
            .find(&headstone.id)
            .await
            .unwrap()
            .expect("a headstone is still a row, and the id still names it");
        assert_eq!(
            by_id.folded_into,
            Some(keeper.id),
            "and it says where it went"
        );
        assert_eq!(by_id.fold_policy, FoldPolicy::Auto);

        let by_locator = |scope| {
            let repo = &repo;
            let persona = &persona;
            async move {
                repo.find_by_source(
                    persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc("/pics/copy.png"),
                    scope,
                )
                .await
                .unwrap()
                .expect("the locator is still known, whichever scope asks")
            }
        };

        let stored = by_locator(SourceLookupScope::Any).await;
        assert_eq!(
            stored.id, headstone.id,
            "the locator stays with the row that was imported from it"
        );
        assert_eq!(stored.folded_into, Some(keeper.id));

        let live = by_locator(SourceLookupScope::Live).await;
        assert_eq!(
            live.id, keeper.id,
            "after the ruling, this path names the row a person can open"
        );
        assert_eq!(
            live.source.locator,
            loc("/pics/keeper.png"),
            "…and it is the keeper's own row that comes back, not a re-labelled headstone"
        );

        // The keeper is untouched by its side of the fold.
        let keeper_row = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(keeper_row.folded_into, None);
    }

    /// A chain: `A → B` and then `B → C`, which is what resolving two
    /// pairs one after the other leaves behind — the second fold does
    /// not rewrite the headstone that was already pointing at `B`.
    ///
    /// `B` is asserted to be the answer *before* the second fold, so a
    /// walk that stopped at the first hop would have to fail here rather
    /// than pass over a fixture where one hop and two agree.
    #[tokio::test]
    async fn a_locator_resolves_along_the_whole_fold_chain() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let a = item(persona, "/pics/a.png");
        let b = item(persona, "/pics/b.png");
        let c = item(persona, "/pics/c.png");
        for asset in [&a, &b, &c] {
            repo.save(asset).await.unwrap();
        }

        let held = |locator: &'static str| {
            let repo = &repo;
            let persona = &persona;
            async move {
                repo.find_by_source(
                    persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc(locator),
                    SourceLookupScope::Live,
                )
                .await
                .unwrap()
                .map(|asset| asset.id)
            }
        };

        repo.fold_into(&a.id, &b.id).await.unwrap();
        assert_eq!(held("/pics/a.png").await, Some(b.id), "one hop");

        repo.fold_into(&b.id, &c.id).await.unwrap();
        assert_eq!(
            held("/pics/a.png").await,
            Some(c.id),
            "two hops: the middle row is a headstone now, and A's path names what survived"
        );
        assert_eq!(
            held("/pics/b.png").await,
            Some(c.id),
            "and so does B's own path"
        );
        assert_eq!(
            held("/pics/c.png").await,
            Some(c.id),
            "…while the keeper's path is answered by the keeper, with no walk involved"
        );
    }

    /// Where the chain ends decides whether the locator is held at all.
    ///
    /// A keeper in the trash is the trash rule reached one hop later:
    /// `Live` passes over a trashed row found directly, and passing over
    /// one found through a fold is the same sentence. `Any` still
    /// answers, which is what keeps this about the chain's end rather
    /// than about the row having disappeared.
    #[tokio::test]
    async fn a_locator_whose_keeper_is_in_the_trash_is_held_by_nothing_live() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        for asset in [&keeper, &headstone] {
            repo.save(asset).await.unwrap();
        }
        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        let live = || {
            let repo = &repo;
            let persona = &persona;
            async move {
                repo.find_by_source(
                    persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc("/pics/copy.png"),
                    SourceLookupScope::Live,
                )
                .await
                .unwrap()
                .map(|asset| asset.id)
            }
        };
        assert_eq!(
            live().await,
            Some(keeper.id),
            "while the keeper is live the path resolves to it"
        );

        repo.trash(&keeper.id, Utc::now()).await.unwrap();
        assert_eq!(
            live().await,
            None,
            "with the keeper thrown away no row a person can open holds this path, \
             so an import of it may mint"
        );
        assert!(
            repo.find_by_source(
                &persona,
                &SourceKind::new(SourceKind::FS).unwrap(),
                &loc("/pics/copy.png"),
                SourceLookupScope::Any,
            )
            .await
            .unwrap()
            .is_some(),
            "the headstone itself did not go anywhere — `Any` still finds it standing there"
        );
    }

    /// A chain that leads nowhere, and one that leads in a circle.
    ///
    /// Neither is writable through a verb (`fold_one` refuses a keeper
    /// that is folded, and only a purge can remove one), so the fixture
    /// writes the column directly — which is also the only way the
    /// states arise in the wild. What is asserted is that the ingest
    /// lookup **terminates** and reports the path as held by nothing,
    /// rather than returning a dead id or spinning.
    #[tokio::test]
    async fn a_broken_fold_chain_answers_with_nothing_instead_of_a_dead_row() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let dangling = item(persona, "/pics/dangling.png");
        let round_a = item(persona, "/pics/round-a.png");
        let round_b = item(persona, "/pics/round-b.png");
        for asset in [&dangling, &round_a, &round_b] {
            repo.save(asset).await.unwrap();
        }

        let live = |locator: &'static str| {
            let repo = &repo;
            let persona = &persona;
            async move {
                repo.find_by_source(
                    persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc(locator),
                    SourceLookupScope::Live,
                )
                .await
                .unwrap()
                .map(|asset| asset.id)
            }
        };
        // Every one of them answers with itself first, so the `None`s
        // below are the chain and not the fixture.
        assert_eq!(live("/pics/dangling.png").await, Some(dangling.id));
        assert_eq!(live("/pics/round-a.png").await, Some(round_a.id));

        let gone = AssetId::new();
        fold_into(&isle, &dangling.id, &gone).await;
        assert_eq!(
            live("/pics/dangling.png").await,
            None,
            "a fold pointing at a row that is not there holds no live locator"
        );

        // `A → B → A`. The cycle guard is what ends this; the hop
        // ceiling would too, but with a different diagnosis.
        fold_into(&isle, &round_a.id, &round_b.id).await;
        fold_into(&isle, &round_b.id, &round_a.id).await;
        assert_eq!(
            live("/pics/round-a.png").await,
            None,
            "a chain that comes back to a row it already passed ends the walk"
        );
    }

    /// The hop ceiling, exercised on a chain one link past it.
    ///
    /// The shorter chain beside it is the disagreement the assertion
    /// needs: a walk that gave up early — or one that never gave up and
    /// hung — would not produce this pair of answers.
    #[tokio::test]
    async fn a_fold_chain_past_the_hop_ceiling_is_not_followed() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        // One row per link, `n` folded into `n + 1`, so the row at the
        // head is `chain.len() - 1` hops from the tail.
        let build = |len: usize, prefix: &'static str| {
            let repo = &repo;
            async move {
                let mut chain = Vec::new();
                for i in 0..len {
                    let asset = item(persona, &format!("/pics/{prefix}-{i}.png"));
                    repo.save(&asset).await.unwrap();
                    chain.push(asset);
                }
                // Forward order, and it is the only one that works: the
                // verb refuses a keeper that is already folded, so each
                // link has to be stood while the row ahead of it is
                // still live. A skipped link would leave a shorter chain
                // and quietly move the ceiling.
                for pair in chain.windows(2) {
                    let outcome = repo.fold_into(&pair[0].id, &pair[1].id).await.unwrap();
                    assert!(
                        matches!(outcome, FoldOutcome::Folded(_)),
                        "the fixture must actually build the chain: {outcome:?}"
                    );
                }
                chain
            }
        };

        let reachable = build(FOLD_RESOLUTION_HOPS + 1, "within").await;
        let beyond = build(FOLD_RESOLUTION_HOPS + 2, "beyond").await;

        let live = |locator: String| {
            let repo = &repo;
            let persona = &persona;
            async move {
                repo.find_by_source(
                    persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc(&locator),
                    SourceLookupScope::Live,
                )
                .await
                .unwrap()
                .map(|asset| asset.id)
            }
        };

        assert_eq!(
            live("/pics/within-0.png".to_string()).await,
            Some(reachable.last().unwrap().id),
            "exactly {FOLD_RESOLUTION_HOPS} hops is still followed to the end"
        );
        assert_eq!(
            live("/pics/beyond-0.png".to_string()).await,
            None,
            "one link further is not, and the caller is told nothing live holds the path"
        );
        // …and not because that chain has no end: its last row is live
        // and answers to its own path. The `None` above is the walk
        // stopping, not a missing keeper.
        let end = beyond.last().unwrap();
        assert_eq!(
            live(format!("/pics/beyond-{}.png", FOLD_RESOLUTION_HOPS + 1)).await,
            Some(end.id)
        );
    }

    /// The walk's own verdict for the row at `from`, read the way
    /// `find_by_source` reads it: the row first, then its `folded_into`
    /// as the first hop.
    ///
    /// Reached directly because the lookup flattens every dead end to
    /// one `None`, and half of what is asserted below is that the walk
    /// tells them apart underneath that.
    async fn walk_from(isle: &AsyncIsle, from: &AssetId) -> FoldResolution {
        let id = *from.as_uuid();
        isle.call(move |conn| {
            let row = conn.query_row(
                &format!("SELECT {} FROM asset WHERE id = ?1", AssetRow::COLUMNS),
                params![id],
                AssetRow::from_row,
            )?;
            let keeper = row
                .folded_into
                .expect("the fixture must stand a headstone to walk from");
            resolve_fold_chain(conn, &row, keeper)
        })
        .await
        .unwrap()
    }

    /// The stored row behind an id, for the tests that hand one to
    /// something taking `&AssetRow` instead of going through the port.
    async fn row_of(isle: &AsyncIsle, id: &AssetId) -> AssetRow {
        let id = *id.as_uuid();
        isle.call(move |conn| {
            conn.query_row(
                &format!("SELECT {} FROM asset WHERE id = ?1", AssetRow::COLUMNS),
                params![id],
                AssetRow::from_row,
            )
        })
        .await
        .unwrap()
    }

    /// Collects the `event` field of everything emitted while it is the
    /// thread's subscriber.
    ///
    /// The field and not the rendered line: `event` is the name a
    /// diagnosis is searched for by, so asserting on it is asserting on
    /// the thing operators use, and it cannot pass because some other
    /// part of a message happened to contain the string. The cost is one
    /// `Visit` impl that keeps a single key.
    #[derive(Clone, Default)]
    struct EventNames(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl EventNames {
        fn seen(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventNames {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Name<'a>(&'a mut Option<String>);
            impl tracing::field::Visit for Name<'_> {
                fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                    if field.name() == "event" {
                        *self.0 = Some(value.to_string());
                    }
                }
                // Every other field is somebody's id and none of them is
                // what this is watching.
                fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
            }
            let mut name = None;
            event.record(&mut Name(&mut name));
            if let Some(name) = name {
                self.0.lock().unwrap().push(name);
            }
        }
    }

    /// **A dead end is said out loud, in its own words.**
    ///
    /// The typed verdict is asserted above, but the verdict is not what
    /// anyone reads: the lookup answers `None` for all of them, so a
    /// `report` with an empty body would leave a corrupt `folded_into`
    /// looking exactly like a path nobody has imported, and the next
    /// sweep would mint over it in silence. That is the state the fold
    /// rules forbid in as many words, and until here nothing failed when
    /// the warning stopped being emitted.
    ///
    /// The verdicts come from the real walk over real rows, so the two
    /// names below are the ones a broken chain actually produces. Only
    /// the reporting is invoked from the test's own thread — a scoped
    /// subscriber reaches no further than the thread it is set on, and
    /// the lookup does its reading inside the isle's actor. That also
    /// leaves one warning without a witness: the candidate-ceiling
    /// `diag.asset.source_candidates_past_ceiling` in `find_by_source`
    /// is emitted inside that closure, and reaching it would take a
    /// global subscriber — which every other test in this binary would
    /// then be running under.
    #[tokio::test]
    async fn a_broken_chain_names_its_diagnosis_in_the_log() {
        use tracing_subscriber::layer::SubscriberExt as _;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let dangling = item(persona, "/pics/dangling.png");
        let round_a = item(persona, "/pics/round-a.png");
        let round_b = item(persona, "/pics/round-b.png");
        let keeper = item(persona, "/pics/keeper.png");
        for asset in [&dangling, &round_a, &round_b, &keeper] {
            repo.save(asset).await.unwrap();
        }
        let gone = AssetId::new();
        fold_into(&isle, &dangling.id, &gone).await;
        fold_into(&isle, &round_a.id, &round_b.id).await;
        fold_into(&isle, &round_b.id, &round_a.id).await;

        let dangling_row = row_of(&isle, &dangling.id).await;
        let dangling_verdict = walk_from(&isle, &dangling.id).await;
        let round_row = row_of(&isle, &round_a.id).await;
        let round_verdict = walk_from(&isle, &round_a.id).await;
        let keeper_row = row_of(&isle, &keeper.id).await;
        // The fixture is two different findings before anything is said
        // about them — otherwise one repeated name below would pass.
        assert!(
            matches!(dangling_verdict, FoldResolution::Dangling(at) if at == *gone.as_uuid()),
            "{dangling_verdict:?}"
        );
        assert!(
            matches!(round_verdict, FoldResolution::Cycle(_)),
            "{round_verdict:?}"
        );

        let log = EventNames::default();
        tracing::subscriber::with_default(tracing_subscriber::registry().with(log.clone()), || {
            dangling_verdict.report(&dangling_row);
            round_verdict.report(&round_row);
            // The two that are meant to be silent, said here so the
            // silence is asserted rather than assumed: an answer and an
            // ordinary absence are not findings, and a `report` that
            // warned about them would bury the two that are.
            FoldResolution::Trashed.report(&dangling_row);
            FoldResolution::Resolved(keeper_row).report(&dangling_row);
        });

        assert_eq!(
            log.seen(),
            vec![
                "diag.asset.fold_chain_dangling".to_string(),
                "diag.asset.fold_chain_cycle".to_string(),
            ],
            "each dead end is reported under its own name, and only the dead ends are"
        );
    }

    /// **A cycle and a ceiling are two different findings**, and the
    /// walk returns which one it hit rather than one `None` for both.
    ///
    /// This is what holds the cycle guard in place. Delete `seen` and
    /// the loop still ends — after sixteen hops around a two-row circle
    /// — so every assertion phrased as "the lookup answers with
    /// nothing" survives its removal. Here the circle is asserted to
    /// come back `Cycle`, one row deep, while a genuinely over-long
    /// chain beside it comes back `TooLong`: with the guard gone the
    /// first of those becomes the second.
    #[tokio::test]
    async fn a_walk_says_whether_it_hit_a_cycle_or_the_ceiling() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let round_a = item(persona, "/pics/round-a.png");
        let round_b = item(persona, "/pics/round-b.png");
        for asset in [&round_a, &round_b] {
            repo.save(asset).await.unwrap();
        }
        // No verb writes a circle (`fold_one` refuses a keeper that is
        // itself folded), so the column is written directly — which is
        // also the only way the state arises in the wild.
        fold_into(&isle, &round_a.id, &round_b.id).await;
        fold_into(&isle, &round_b.id, &round_a.id).await;
        let verdict = walk_from(&isle, &round_a.id).await;
        assert!(
            matches!(verdict, FoldResolution::Cycle(at) if at == *round_a.id.as_uuid()),
            "the walk came back to the row it started at: {verdict:?}"
        );

        // A chain one link past the ceiling, built with the verb, in
        // forward order because each link has to be stood while the row
        // ahead of it is still live.
        let mut chain = Vec::new();
        for i in 0..FOLD_RESOLUTION_HOPS + 2 {
            let asset = item(persona, &format!("/pics/long-{i}.png"));
            repo.save(&asset).await.unwrap();
            chain.push(asset);
        }
        for pair in chain.windows(2) {
            let outcome = repo.fold_into(&pair[0].id, &pair[1].id).await.unwrap();
            assert!(
                matches!(outcome, FoldOutcome::Folded(_)),
                "the fixture must actually build the chain: {outcome:?}"
            );
        }
        let verdict = walk_from(&isle, &chain[0].id).await;
        assert!(
            matches!(verdict, FoldResolution::TooLong),
            "a chain past the ceiling is a different finding from a circle: {verdict:?}"
        );
        // …and one link shorter is no finding at all, so the ceiling is
        // where it says it is rather than anywhere below.
        let verdict = walk_from(&isle, &chain[1].id).await;
        assert!(
            matches!(verdict, FoldResolution::Resolved(ref row) if row.id == *chain.last().unwrap().id.as_uuid()),
            "exactly {FOLD_RESOLUTION_HOPS} hops still resolves: {verdict:?}"
        );
    }

    /// **A dead end sends the locator to the row behind it**, and only a
    /// locator with nothing but dead ends on it is held by nothing live.
    ///
    /// `OnDuplicate::Separate` puts several rows on one locator on
    /// purpose, and a fold writes no `trashed_at` — so the headstone
    /// stays live and stays the *oldest* of them, which is the one the
    /// lookup reaches first, every time, for ever. A lookup that ended
    /// at that dead end would report the path as unregistered on every
    /// sweep and mint another row on every sweep, never once seeing the
    /// rows it had already minted.
    ///
    /// The two answers are in disagreement by construction: while the
    /// keeper is live the *first* candidate answers, and only once its
    /// chain dies does the second. An implementation that always
    /// preferred the newest row would fail the first assertion, and one
    /// that stopped at the first dead end would fail the second.
    #[tokio::test]
    async fn a_dead_end_candidate_hands_the_locator_to_the_row_behind_it() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        // The candidate order is `created_at, id`, so the fixture states
        // the instants rather than leaving them to two `Utc::now()`
        // calls that may land in one millisecond and hand the tie to an
        // id comparison nobody meant to assert about.
        let mut older = item(persona, "/pics/filed-twice.png");
        older.created_at = at(1_700_000_000_000);
        let mut newer = item(persona, "/pics/filed-twice.png");
        newer.created_at = at(1_700_000_060_000);
        let keeper = item(persona, "/pics/keeper.png");
        // The older row is *saved last* on purpose — arrival order and
        // `created_at` order disagree, so a lookup that dropped
        // `ORDER BY created_at, id` would reach the newer row first and
        // fail below, rather than agree with the sort by accident.
        for asset in [&keeper, &newer, &older] {
            repo.save(asset).await.unwrap();
        }
        repo.fold_into(&older.id, &keeper.id).await.unwrap();

        let live = || {
            let repo = &repo;
            let persona = &persona;
            async move {
                repo.find_by_source(
                    persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc("/pics/filed-twice.png"),
                    SourceLookupScope::Live,
                )
                .await
                .unwrap()
                .map(|asset| asset.id)
            }
        };

        assert_eq!(
            live().await,
            Some(keeper.id),
            "the oldest row holding the locator answers, through its fold"
        );

        repo.trash(&keeper.id, Utc::now()).await.unwrap();
        assert_eq!(
            live().await,
            Some(newer.id),
            "with that chain dead the question moves to the next row holding the locator, \
             which is live and answers for itself"
        );

        // And when the last of them dies too, nothing live holds the
        // path — which is the state that makes minting right, and the
        // one an ingest would otherwise never reach.
        repo.trash(&newer.id, Utc::now()).await.unwrap();
        assert_eq!(live().await, None, "every candidate is a dead end now");
        assert_eq!(
            repo.find_by_source(
                &persona,
                &SourceKind::new(SourceKind::FS).unwrap(),
                &loc("/pics/filed-twice.png"),
                SourceLookupScope::Any,
            )
            .await
            .unwrap()
            .map(|asset| asset.id),
            Some(older.id),
            "…while storage still says who is standing at that address"
        );
    }

    /// **A fold written across two libraries is not followed.**
    ///
    /// Nothing on the write path stops one being written: `MergePlan::
    /// declare` weighs id sets and `fold_one` has no persona term, so a
    /// hand-run merge can point one persona's headstone at another
    /// persona's row. The automatic detector cannot produce it — it
    /// groups by `(persona_id, digest)` — which is exactly why the
    /// fixture uses the merge verb.
    ///
    /// Following it would be a leak, not just a wrong id: the answer
    /// carries the other row's locator, title and labels back to a
    /// caller scoped to a persona that may not see any of them. So the
    /// walk stops, the locator goes unheld on this side, and the other
    /// persona — asserted here as the control, so that a lookup broken
    /// into always answering `None` cannot pass — still answers for its
    /// own row.
    #[tokio::test]
    async fn a_fold_leaving_the_persona_is_not_followed() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let mine = seed_persona(&isle).await;
        let theirs = seed_persona(&isle).await;

        // One path, two libraries — the state
        // `two_personas_holding_one_path_are_two_first_imports` declares
        // legitimate.
        let ours = item(mine, "/shared/pic.png");
        let hers = item(theirs, "/shared/pic.png");
        for asset in [&ours, &hers] {
            repo.save(asset).await.unwrap();
        }
        let outcome = repo.fold_into(&ours.id, &hers.id).await.unwrap();
        assert!(
            matches!(outcome, FoldOutcome::Folded(_)),
            "the verb writes a cross-persona fold, which is why this test exists: {outcome:?}"
        );

        let live = |persona: PersonaId| {
            let repo = &repo;
            async move {
                repo.find_by_source(
                    &persona,
                    &SourceKind::new(SourceKind::FS).unwrap(),
                    &loc("/shared/pic.png"),
                    SourceLookupScope::Live,
                )
                .await
                .unwrap()
                .map(|asset| asset.id)
            }
        };

        assert_eq!(
            live(mine).await,
            None,
            "my headstone points out of my library, so nothing I can open holds this path"
        );
        assert_eq!(
            live(theirs).await,
            Some(hers.id),
            "…and the other library still answers for its own row, which is what says the \
             lookup stopped rather than stopped working"
        );

        let verdict = walk_from(&isle, &ours.id).await;
        assert!(
            matches!(verdict, FoldResolution::OtherPersona(id) if id == *hers.id.as_uuid()),
            "the walk names the reason, and it is not a purged keeper: {verdict:?}"
        );
    }

    /// A ruling is durable, and `save` is not allowed to undo it: the
    /// resolution verb owns both fold columns, so a metadata
    /// round-trip (`find` → edit → `save`) carrying stale values back
    /// must change neither. Without this, resurrecting a headstone
    /// would take nothing more than renaming it.
    #[tokio::test]
    async fn save_cannot_resurrect_a_headstone_or_undo_a_keep() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        for asset in [&keeper, &headstone] {
            repo.save(asset).await.unwrap();
        }

        // Both entities were read before either column was written —
        // the realistic shape of a lost update.
        let mut stale_headstone = repo.find(&headstone.id).await.unwrap().unwrap();
        let mut stale_keeper = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(stale_headstone.folded_into, None);
        assert_eq!(stale_keeper.fold_policy, FoldPolicy::Auto);

        fold_into(&isle, &headstone.id, &keeper.id).await;
        let keeper_uuid = *keeper.id.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "UPDATE asset SET fold_policy = 'keep' WHERE id = ?1",
                params![keeper_uuid],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        stale_headstone.title = Some("renamed".into());
        stale_keeper.title = Some("also renamed".into());
        repo.save(&stale_headstone).await.unwrap();
        repo.save(&stale_keeper).await.unwrap();

        let headstone_after = repo.find(&headstone.id).await.unwrap().unwrap();
        assert_eq!(headstone_after.title.as_deref(), Some("renamed"));
        assert_eq!(
            headstone_after.folded_into,
            Some(keeper.id),
            "the fold survived a whole-row save"
        );
        let keeper_after = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(keeper_after.title.as_deref(), Some("also renamed"));
        assert_eq!(
            keeper_after.fold_policy,
            FoldPolicy::Keep,
            "and so did the human ruling"
        );
        assert_eq!(
            repo.list(&AssetQuery::default()).await.unwrap().items.len(),
            1,
            "the listing never saw the headstone come back"
        );
    }

    // ---- the hash lookup ----------------------------------------
    //
    // `find_by_content_hash` is the read conflict detection runs once a
    // fingerprint lands. Everything below pins one of its documented
    // decisions; each fixture first shows the row it is about being
    // *found*, so an assertion cannot pass over a set that never
    // contained it.

    /// Seeds one item with its primary material already fingerprinted,
    /// at a stated occurrence time. Time is explicit because the port
    /// promises oldest-first, and `Utc::now()` twice inside one
    /// millisecond would let the tie-break carry an assertion the sort
    /// is supposed to carry.
    async fn hashed_item(
        repo: &SqliteAssetRepository,
        persona: PersonaId,
        locator: &str,
        digest: &str,
        occurred_at: DateTime<Utc>,
    ) -> Asset {
        let mut asset = item(persona, locator);
        asset.occurred_at = occurred_at;
        repo.save(&asset).await.unwrap();
        repo.set_material_fingerprint(&asset.id, 0, &file_axis(digest))
            .await
            .unwrap();
        asset
    }

    fn at(ms: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(ms).unwrap()
    }

    /// The key is `(persona_id, digest)`. Both halves are load-bearing:
    /// the same bytes under a second persona are a different question
    /// (folding across personas is manual only), and the answer comes
    /// back oldest first so the caller can read the incumbent off the
    /// front.
    ///
    /// The older row is *saved second* on purpose — arrival order and
    /// occurrence order disagree, so an implementation that returned
    /// rows in insertion order would fail here rather than agree by
    /// accident.
    #[tokio::test]
    async fn the_hash_lookup_answers_within_one_persona_oldest_first() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let neighbour = seed_persona(&isle).await;

        let digest = "sha256:aaaa";
        let newer = hashed_item(&repo, persona, "/pics/copy.png", digest, at(2_000)).await;
        let older = hashed_item(&repo, persona, "/pics/original.png", digest, at(1_000)).await;
        // Same bytes, other persona, and a row that must never appear.
        let elsewhere =
            hashed_item(&repo, neighbour, "/other/original.png", digest, at(1_500)).await;

        let hits = repo
            .find_by_content_hash(&persona, DuplicateAxis::Artefact, digest)
            .await
            .unwrap();
        assert_eq!(
            hits.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![older.id, newer.id],
            "both sharers of the digest, oldest first"
        );
        assert!(
            !hits.iter().any(|a| a.id == elsewhere.id),
            "a persona boundary is not crossed by a hash"
        );
        // And the neighbour sees its own row and only its own.
        let theirs = repo
            .find_by_content_hash(&neighbour, DuplicateAxis::Artefact, digest)
            .await
            .unwrap();
        assert_eq!(
            theirs.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![elsewhere.id]
        );

        // The entity is hydrated, not a bare id: the caller decides
        // with `fold_policy` / `occurred_at` / the material in hand.
        assert_eq!(hits[0].fold_policy, FoldPolicy::Auto);
        assert_eq!(
            hits[0].materials.first().unwrap().content_hash.as_deref(),
            Some(digest)
        );

        // A digest nobody carries is an empty answer, not an error —
        // that is the shape "nothing else holds these bytes" has.
        assert!(
            repo.find_by_content_hash(&persona, DuplicateAxis::Artefact, "sha256:bbbb")
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// The values that live in the hash column without standing for
    /// bytes are refused, not answered. Every fragment in a
    /// conversation log shares the one `unhashable:` marker and every
    /// empty file shares the empty digest, so answering honestly would
    /// return the whole corpus as one match — and answering "no rows"
    /// would be indistinguishable from "these bytes are unique".
    #[tokio::test]
    async fn the_hash_lookup_refuses_a_value_that_is_not_a_digest() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        for (i, marker) in [UNHASHABLE, UNHASHABLE, UNHASHABLE, EMPTY, EMPTY]
            .into_iter()
            .enumerate()
        {
            hashed_item(
                &repo,
                persona,
                &format!("/logs/session-{i}.jsonl"),
                marker,
                at(1_000 + i as i64),
            )
            .await;
        }

        // The rows are there and they do share the value — without this
        // the refusal below would be indistinguishable from an empty
        // table.
        let sharing = isle
            .call(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM material WHERE content_hash = ?1",
                    params![UNHASHABLE],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(sharing, 3, "three rows would have matched the marker");

        for value in [UNHASHABLE, EMPTY, "phash:0f0f", "", "sha256"] {
            let err = repo
                .find_by_content_hash(&persona, DuplicateAxis::Artefact, value)
                .await
                .expect_err("only a real digest may be looked up");
            assert!(
                matches!(err, DomainError::Validation(_)),
                "{value:?} was rejected as {err:?}, not as a validation error"
            );
        }

        // The refusal follows the domain rule rather than a second
        // list, so a value that rule accepts is accepted here.
        assert!(content_hash::is_duplicate_key(
            DuplicateAxis::Artefact,
            "sha256:aaaa"
        ));
        assert!(
            repo.find_by_content_hash(&persona, DuplicateAxis::Artefact, "sha256:aaaa")
                .await
                .is_ok()
        );
    }

    /// The key is `(persona, digest)` **at `ord = 0`**. A material at a
    /// higher ord is a secondary original of its asset (the RAW beside
    /// the JPEG); it matching somebody's primary is not two of the same
    /// asset, and folding on it would throw away the row that owns the
    /// other resource.
    #[tokio::test]
    async fn the_hash_lookup_ignores_a_secondary_material() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = "sha256:aaaa";
        let primary = hashed_item(&repo, persona, "/pics/a.png", digest, at(1_000)).await;

        // A second asset whose *secondary* material carries the same
        // bytes; its own primary hashes to something else.
        let mut pair = item(persona, "/pics/b.jpg");
        pair.occurred_at = at(2_000);
        let mut secondary = asterism_core::domain::material::Material::primary(
            loc("/pics/b.raw"),
            Some(2),
            Utc::now(),
        );
        secondary.ord = 1;
        pair.materials.push(secondary);
        repo.save(&pair).await.unwrap();
        repo.set_material_fingerprint(&pair.id, 0, &file_axis("sha256:bbbb"))
            .await
            .unwrap();
        repo.set_material_fingerprint(&pair.id, 1, &file_axis(digest))
            .await
            .unwrap();

        // The fixture really did write the digest at ord 1 — otherwise
        // the assertion below would hold over a row that never matched.
        let secondary_rows = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM material WHERE ord = 1 AND content_hash = ?1",
                    params!["sha256:aaaa"],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(secondary_rows, 1);

        let hits = repo
            .find_by_content_hash(&persona, DuplicateAxis::Artefact, digest)
            .await
            .unwrap();
        assert_eq!(
            hits.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![primary.id],
            "the secondary original is not a duplicate of a primary"
        );
    }

    /// A headstone is gone from every path that enumerates, and this
    /// enumerates. It is also what keeps a third copy working: the
    /// keeper still carries the hash, so the newcomer raises its
    /// conflict against a row that exists instead of against a dead one
    /// it could then be folded into.
    #[tokio::test]
    async fn the_hash_lookup_forgets_a_folded_row() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = "sha256:aaaa";
        let keeper = hashed_item(&repo, persona, "/pics/keeper.png", digest, at(1_000)).await;
        let headstone = hashed_item(&repo, persona, "/pics/copy.png", digest, at(2_000)).await;

        let before = repo
            .find_by_content_hash(&persona, DuplicateAxis::Artefact, digest)
            .await
            .unwrap();
        assert_eq!(before.len(), 2, "both are found before the fold");

        fold_into(&isle, &headstone.id, &keeper.id).await;

        let after = repo
            .find_by_content_hash(&persona, DuplicateAxis::Artefact, digest)
            .await
            .unwrap();
        assert_eq!(
            after.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![keeper.id],
            "the folded row is not offered as a partner again"
        );
        // Its material still holds the digest — the row was excluded by
        // the fold rule, not by having lost its hash.
        let headstone_row = repo.find(&headstone.id).await.unwrap().unwrap();
        assert_eq!(
            headstone_row
                .materials
                .first()
                .unwrap()
                .content_hash
                .as_deref(),
            Some(digest)
        );
    }

    /// A trashed row **is** returned — deliberately unlike the
    /// duplicate report, which is a work list and skips what is already
    /// on its way out. This is a lookup by key, and a trashed row holds
    /// the bytes as firmly as it holds its locator (the reasoning
    /// `find_by_source` already applies on the path axis). Hiding it
    /// would make re-importing something the user threw away look
    /// unique.
    #[tokio::test]
    async fn the_hash_lookup_still_reports_a_trashed_holder() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = "sha256:aaaa";
        let discarded =
            hashed_item(&repo, persona, "/pics/thrown-away.png", digest, at(1_000)).await;
        let arrival = hashed_item(&repo, persona, "/pics/again.png", digest, at(2_000)).await;
        repo.trash(&discarded.id, Utc::now()).await.unwrap();

        let hits = repo
            .find_by_content_hash(&persona, DuplicateAxis::Artefact, digest)
            .await
            .unwrap();
        assert_eq!(
            hits.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![discarded.id, arrival.id],
            "the trash still holds these bytes"
        );
        assert!(
            hits[0].trashed_at.is_some(),
            "and the caller can tell which kind of holder it is"
        );

        // The contrast that makes the choice deliberate rather than an
        // oversight: the report the user acts on does drop it.
        let groups = repo
            .list_duplicate_groups(Some(&persona), DuplicateAxis::Artefact, 50)
            .await
            .unwrap();
        assert!(
            groups.is_empty(),
            "the work list stays quiet about a pair that is half in the trash"
        );
    }

    /// The plan behind the decision recorded in
    /// [`find_by_content_hash`]: the digest side is served by
    /// `idx_material_content_hash` (V41), so no index was added for this
    /// lookup. The statement is fetched from
    /// [`content_hash_lookup_sql`] rather than retyped, so a rewrite of
    /// the query is measured by this test instead of drifting past it.
    ///
    /// **Both axes**, since the axis chooses the column and so chooses
    /// the index. The content axis has the same shape of index on its
    /// own column (`idx_material_content_region_hash`, V55) — asserting
    /// only the artefact one would let the second axis run the lookup
    /// as a table scan on every fingerprint, unnoticed.
    ///
    /// The failure message carries the whole plan: when this breaks,
    /// what is wanted is the plan SQLite actually chose, not the fact
    /// that it changed.
    #[tokio::test]
    async fn the_hash_lookup_is_served_by_the_content_hash_index() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        // Measured 2026-08-04, SQLite as bundled by rusqlite:
        //
        //   SEARCH m USING COVERING INDEX idx_material_content_hash (content_hash=?)
        //   SEARCH asset USING INDEX sqlite_autoindex_asset_1 (id=?)
        //   USE TEMP B-TREE FOR ORDER BY
        //
        // The order of the first two lines is the whole point: the
        // digest is the outer loop and the asset rows are primary-key
        // hits off it. Written `WHERE asset.id IN (SELECT … )` the same
        // query planned the other way round —
        // `SEARCH asset USING INDEX idx_asset_persona_occurred
        // (persona_id=?)` — walking every asset of the persona on a
        // lookup that runs once per fingerprint.
        for (axis, index, probe) in [
            (
                DuplicateAxis::Artefact,
                "idx_material_content_hash",
                "sha256:aaaa",
            ),
            (
                DuplicateAxis::Content,
                "idx_material_content_region_hash",
                "cr1-sha256:aaaa",
            ),
        ] {
            let sql = content_hash_lookup_sql(axis);
            let plan: Vec<String> = isle
                .call(move |conn| {
                    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                    // Both placeholders are bound: the statement plans
                    // with the same shape it runs with.
                    stmt.query_map(params![Uuid::now_v7(), probe], |r| r.get::<_, String>(3))?
                        .collect::<Result<_, _>>()
                })
                .await
                .unwrap();
            let plan_text = plan.join("\n");
            assert!(
                plan[0].contains(&format!("SEARCH m USING COVERING INDEX {index}")),
                "the {} digest is no longer the outer loop:\n{plan_text}",
                axis.as_str()
            );
            assert!(
                plan[1].contains("SEARCH asset USING INDEX sqlite_autoindex_asset_1"),
                "the asset side is no longer a primary-key hit:\n{plan_text}"
            );
            assert!(
                !plan_text.contains("SCAN "),
                "something turned into a scan:\n{plan_text}"
            );
        }
    }

    /// The plan behind V57: the live grid listing (`page_index` with the
    /// desktop default — a persona filter, `LiveOnly`, `occurred_at
    /// DESC`) must be served **entirely from the covering index**. The
    /// WHERE clause comes from the real `QueryParts::build` and the
    /// column list from the real `IndexRow::COLUMNS`, so a predicate or
    /// column added to the listing without being added to the index
    /// breaks here instead of silently re-introducing the per-hit
    /// table-page lookup V17 was measured to cost (~2.0 s cold at 110k
    /// rows) and V57 was measured to have lost.
    #[tokio::test]
    async fn the_live_grid_listing_is_served_by_the_covering_index() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let query = AssetQuery {
            persona_id: Some(PersonaId::from_uuid(Uuid::now_v7())),
            ..AssetQuery::default()
        };
        let parts = QueryParts::build(&query);
        let select_sql = format!(
            "SELECT {} FROM asset {} ORDER BY occurred_at DESC LIMIT ? OFFSET ?",
            IndexRow::COLUMNS,
            parts.where_sql,
        );
        let plan: Vec<String> = isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {select_sql}"))?;
                let mut params = parts.params;
                params.push(Value::Integer(200_000));
                params.push(Value::Integer(0));
                stmt.query_map(rusqlite::params_from_iter(params), |r| {
                    r.get::<_, String>(3)
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        let plan_text = plan.join("\n");
        // Measured 2026-08-05, SQLite as bundled by rusqlite:
        //
        //   SEARCH asset USING COVERING INDEX idx_asset_persona_occurred_cover (persona_id=?)
        //
        // Before V57 the same statement planned as `SEARCH asset USING
        // INDEX idx_asset_persona_occurred (persona_id=?)` — a seek
        // index, one table-page lookup per returned row.
        assert!(
            plan_text.contains("USING COVERING INDEX idx_asset_persona_occurred_cover"),
            "the live listing left the covering index:\n{plan_text}"
        );
    }

    // ---- the declared duplicate strategy -------------------------

    /// The declaration made at registration has to still be there when
    /// the fingerprint lands, which is minutes or hours later and in
    /// another process's worker. That is the whole of this subtask, so
    /// it is what this pins: every value survives the round trip, and
    /// silence survives as silence.
    #[tokio::test]
    async fn a_declared_strategy_lands_and_absence_stays_absent() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        for (locator, declared) in [
            ("/pics/ask.png", OnDuplicate::Ask),
            ("/pics/fold.png", OnDuplicate::Fold),
            ("/pics/separate.png", OnDuplicate::Separate),
        ] {
            let mut asset = item(persona, locator);
            asset.on_duplicate = Some(declared);
            repo.save(&asset).await.unwrap();

            let stored = repo.find(&asset.id).await.unwrap().unwrap();
            assert_eq!(
                stored.on_duplicate,
                Some(declared),
                "{locator} was registered with {declared:?} and read back as something else"
            );
        }

        // Undeclared stays undeclared. `Ask` is what an undeclared
        // registration will *resolve to* while it is the only default
        // there is — but resolving is the detector's job, and a row that
        // stored the answer could never pick up a lane default later.
        let silent = item(persona, "/pics/silent.png");
        assert_eq!(silent.on_duplicate, None, "the fixture declares nothing");
        repo.save(&silent).await.unwrap();
        let stored = repo.find(&silent.id).await.unwrap().unwrap();
        assert_eq!(
            stored.on_duplicate, None,
            "an undeclared registration must not read back as a request to ask"
        );
        // …and that absence is a NULL column, not the string "ask"
        // written through a mapping that happened to invert twice.
        let raw: Option<String> = isle
            .call({
                let id = *silent.id.as_uuid();
                move |conn| {
                    conn.query_row(
                        "SELECT on_duplicate FROM asset WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                }
            })
            .await
            .unwrap();
        assert_eq!(raw, None);
    }

    /// `save` writes the column on INSERT and never on UPDATE: the row
    /// records what the caller declared *at registration*, and no verb
    /// re-declares it. Without this, any read-modify-write path could
    /// restate a past intention as a present one — and the entity is
    /// hydrated everywhere, so every metadata edit is such a path.
    #[tokio::test]
    async fn save_does_not_restate_a_declaration() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut registered = item(persona, "/pics/declared.png");
        registered.on_duplicate = Some(OnDuplicate::Separate);
        repo.save(&registered).await.unwrap();

        // A caller reads the asset, changes its mind about the strategy
        // along with something it is allowed to change, and saves.
        let mut edited = repo.find(&registered.id).await.unwrap().unwrap();
        assert_eq!(edited.on_duplicate, Some(OnDuplicate::Separate));
        edited.title = Some("renamed".into());
        edited.on_duplicate = Some(OnDuplicate::Fold);
        repo.save(&edited).await.unwrap();

        let after = repo.find(&registered.id).await.unwrap().unwrap();
        assert_eq!(
            after.title.as_deref(),
            Some("renamed"),
            "the edit the caller was entitled to make did land"
        );
        assert_eq!(
            after.on_duplicate,
            Some(OnDuplicate::Separate),
            "the declaration is what registration said, not what a later save says"
        );

        // The same rule from the other side: a save cannot introduce a
        // declaration onto a row that never carried one.
        let silent = item(persona, "/pics/still-silent.png");
        repo.save(&silent).await.unwrap();
        let mut edited = repo.find(&silent.id).await.unwrap().unwrap();
        edited.on_duplicate = Some(OnDuplicate::Fold);
        repo.save(&edited).await.unwrap();
        assert_eq!(
            repo.find(&silent.id).await.unwrap().unwrap().on_duplicate,
            None,
            "a strategy cannot be back-dated onto an undeclared registration"
        );
    }

    // ---- the conflict queue --------------------------------------

    /// Builds a question about two rows without going through
    /// detection — the storage half is what these tests are about.
    fn raise(persona: PersonaId, newcomer: &Asset, incumbent: &Asset) -> DuplicateConflict {
        DuplicateConflict::raise(
            persona,
            newcomer.id,
            incumbent.id,
            DuplicateAxis::Artefact,
            "sha256:aaaa",
            None,
            at(3_000),
        )
        .unwrap()
    }

    /// One pair, one question — no matter how many times, or from which
    /// end, the match is observed.
    ///
    /// The mirror case is the one worth having: the backfill walk
    /// fingerprints rows in id order, so which of two copies is the
    /// "newcomer" depends on which one the walk reached first. Keyed on
    /// the ordered pair, a re-walk from the other side would put the
    /// same two rows in front of the user a second time.
    #[tokio::test]
    async fn one_pair_queues_once_however_often_it_is_detected() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = "sha256:aaaa";
        let older = hashed_item(&repo, persona, "/pics/original.png", digest, at(1_000)).await;
        let newer = hashed_item(&repo, persona, "/pics/copy.png", digest, at(2_000)).await;

        assert!(
            repo.record_duplicate_conflict(&raise(persona, &newer, &older))
                .await
                .unwrap(),
            "the first detection is news"
        );
        assert!(
            !repo
                .record_duplicate_conflict(&raise(persona, &newer, &older))
                .await
                .unwrap(),
            "re-running the fingerprint asks nothing new"
        );
        assert!(
            !repo
                .record_duplicate_conflict(&raise(persona, &older, &newer))
                .await
                .unwrap(),
            "the same pair from the other end is the same question"
        );

        let open = repo
            .list_open_duplicate_conflicts(Some(&persona), 50)
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(
            (open[0].newcomer, open[0].incumbent),
            (newer.id, older.id),
            "the row keeps the direction the first detection recorded"
        );
        assert_eq!(open[0].axis, DuplicateAxis::Artefact);
        assert_eq!(open[0].content_hash, digest);
        assert!(open[0].is_open());
        assert_eq!(
            open[0].fold_exclusion, None,
            "nobody asked this pair to fold, so nothing was declined"
        );

        // A different axis is a different question about the same pair:
        // "every byte agreed" and "the decoded result agrees" are not
        // the same finding, and one may be resolved without the other.
        let mut content_axis = raise(persona, &newer, &older);
        content_axis.axis = DuplicateAxis::Content;
        // …and this one was a fold the lineage rule declined, so the
        // reason travels with it. Asserted on the second row rather
        // than the first so the round trip is measured against a
        // column that is `None` on a sibling row — a mapping that
        // dropped it would otherwise agree with the default.
        content_axis.fold_exclusion = Some(FoldExclusion::Lineage);
        assert!(
            repo.record_duplicate_conflict(&content_axis).await.unwrap(),
            "the axis is part of the key"
        );
        let both = repo
            .list_open_duplicate_conflicts(Some(&persona), 50)
            .await
            .unwrap();
        assert_eq!(both.len(), 2);
        let reason_of = |axis: DuplicateAxis| {
            both.iter()
                .find(|c| c.axis == axis)
                .expect("both axes are on the queue")
                .fold_exclusion
        };
        assert_eq!(
            reason_of(DuplicateAxis::Content),
            Some(FoldExclusion::Lineage)
        );
        assert_eq!(reason_of(DuplicateAxis::Artefact), None);

        // The persona filter is the sidebar's, as everywhere else.
        let neighbour = seed_persona(&isle).await;
        assert!(
            repo.list_open_duplicate_conflicts(Some(&neighbour), 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            repo.list_open_duplicate_conflicts(None, 50)
                .await
                .unwrap()
                .len(),
            2,
            "and no filter means every persona"
        );
    }

    /// A question about a row that is in the trash or has been folded
    /// away is not asked — and, for the trash, starts being asked again
    /// when the row comes back.
    ///
    /// That reversibility is why the rule is a join rather than a
    /// column: writing "resolved: the row went away" onto the queue row
    /// would have to be un-written by whatever restores the asset, which
    /// is a verb that knows nothing about this table.
    #[tokio::test]
    async fn a_question_stops_being_asked_while_a_side_is_gone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let digest = "sha256:aaaa";
        let keeper = hashed_item(&repo, persona, "/pics/keeper.png", digest, at(1_000)).await;
        let copy = hashed_item(&repo, persona, "/pics/copy.png", digest, at(2_000)).await;
        repo.record_duplicate_conflict(&raise(persona, &copy, &keeper))
            .await
            .unwrap();

        async fn open(repo: &SqliteAssetRepository, persona: &PersonaId) -> usize {
            repo.list_open_duplicate_conflicts(Some(persona), 50)
                .await
                .unwrap()
                .len()
        }
        assert_eq!(
            open(&repo, &persona).await,
            1,
            "the question is on the queue"
        );

        // In the trash: not worth interrupting anyone over, the row is
        // already on its way out.
        repo.trash(&copy.id, Utc::now()).await.unwrap();
        assert_eq!(open(&repo, &persona).await, 0);

        // Restored: the question is live again, from the same row.
        repo.restore(&copy.id).await.unwrap();
        assert_eq!(
            open(&repo, &persona).await,
            1,
            "the queue row was never destroyed, only filtered"
        );

        // Folded: answered structurally, and one-way — a headstone is
        // not a thing to compare.
        fold_into(&isle, &copy.id, &keeper.id).await;
        assert_eq!(open(&repo, &persona).await, 0);

        let rows: i64 = isle
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM duplicate_conflict", params![], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(
            rows, 1,
            "the record that a conflict was raised outlives both filters"
        );
    }

    // ---- the fold verb -------------------------------------------
    //
    // Every test below asserts **before the fold** that the row it is
    // about to follow really is attached to the headstone, then folds,
    // then asserts where it ended up. The pre-assertion is not
    // ceremony: an assertion that only reads the post-fold state passes
    // just as happily over a fixture that was attached to the keeper
    // all along, and would keep passing with the re-point deleted.

    /// Seeds one Group under `persona`.
    async fn seed_bucket(isle: &AsyncIsle, persona: PersonaId, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let pid = *persona.as_uuid();
        let name = name.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO bucket (id, persona_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 0, 0)",
                params![id, pid, name],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        id
    }

    /// Files `asset` into `bucket` at `position`.
    async fn file_in_bucket(isle: &AsyncIsle, asset: &AssetId, bucket: Uuid, position: i64) {
        let asset = *asset.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset_bucket (asset_id, bucket_id, added_at, position) \
                 VALUES (?1, ?2, 0, ?3)",
                params![asset, bucket, position],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// `(bucket_id, position)` pairs an asset is filed under.
    async fn filing_of(isle: &AsyncIsle, asset: &AssetId) -> Vec<(Uuid, i64)> {
        let asset = *asset.as_uuid();
        isle.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT bucket_id, position FROM asset_bucket \
                  WHERE asset_id = ?1 ORDER BY bucket_id",
            )?;
            stmt.query_map(params![asset], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await
        .unwrap()
    }

    /// Seeds a tag and links it to `asset`.
    async fn seed_tag(isle: &AsyncIsle, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let name = name.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO tag (id, name) VALUES (?1, ?2)",
                params![id, name],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        id
    }

    async fn link_tag(isle: &AsyncIsle, asset: &AssetId, tag: Uuid) {
        let asset = *asset.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset_tag (asset_id, tag_id) VALUES (?1, ?2)",
                params![asset, tag],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn tags_of(isle: &AsyncIsle, asset: &AssetId) -> Vec<Uuid> {
        let asset = *asset.as_uuid();
        isle.call(move |conn| {
            let mut stmt =
                conn.prepare("SELECT tag_id FROM asset_tag WHERE asset_id = ?1 ORDER BY tag_id")?;
            stmt.query_map(params![asset], |row| row.get(0))?.collect()
        })
        .await
        .unwrap()
    }

    /// Writes one edge straight at the table — the repository's edge
    /// port refuses a self-loop, and some of these fixtures need shapes
    /// only a fold can produce.
    async fn seed_edge(isle: &AsyncIsle, from: &AssetId, to: &AssetId, kind: &str, label: &str) {
        let (from, to) = (*from.as_uuid(), *to.as_uuid());
        let (kind, label) = (kind.to_string(), label.to_string());
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO edge (id, from_asset, to_asset, kind, label, weight) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0.5)",
                params![Uuid::now_v7(), from, to, kind, label],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Every edge in the table as `(from, to, kind)`.
    async fn all_edges(isle: &AsyncIsle) -> Vec<(Uuid, Uuid, String)> {
        isle.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT from_asset, to_asset, kind FROM edge \
                  ORDER BY from_asset, to_asset, kind",
            )?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .await
        .unwrap()
    }

    /// Every column of one asset row, positionally — the shape a
    /// "nothing about the keeper changed" assertion needs. Reading it
    /// through `AssetRow::COLUMNS` means a column added later is
    /// compared too, without this test being edited.
    async fn raw_row(isle: &AsyncIsle, id: &AssetId) -> Vec<Value> {
        let uuid = *id.as_uuid();
        isle.call(move |conn| {
            conn.query_row(
                &format!("SELECT {} FROM asset WHERE id = ?1", AssetRow::COLUMNS),
                params![uuid],
                |row| {
                    (0..row.as_ref().column_count())
                        .map(|i| row.get::<_, Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                },
            )
        })
        .await
        .unwrap()
    }

    async fn container_of(isle: &AsyncIsle, child: &AssetId) -> Option<Uuid> {
        let child = *child.as_uuid();
        isle.call(move |conn| {
            conn.query_row(
                "SELECT container_id FROM asset WHERE id = ?1",
                params![child],
                |row| row.get(0),
            )
        })
        .await
        .unwrap()
    }

    /// Every column of `asset` has to be in exactly one of the three
    /// lists — combined, kept-and-recorded, or left alone.
    ///
    /// The point is the column that does not exist yet. Add one and it
    /// belongs to none of the three, and the choice of what a fold
    /// should do with it gets made here, deliberately, instead of being
    /// inherited from whichever list happened to be nearest.
    ///
    /// # Against the table, not against a `SELECT` list
    ///
    /// The other side of this assertion used to be `AssetRow::COLUMNS`,
    /// which is a hand-written list of the columns the *reader* wants.
    /// It passed while omitting five columns, because both sides
    /// omitted the same five: a column reached the guard by being
    /// remembered in a string, and one nobody typed there was invisible
    /// to it. `PRAGMA table_info` is the table itself, so a column
    /// reaches the guard by existing.
    #[tokio::test]
    async fn the_three_column_groups_cover_the_table() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let columns: Vec<String> = isle
            .call(|conn| {
                let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('asset')")?;
                stmt.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        assert!(
            columns.len() > 30,
            "the PRAGMA answered about a table that is not there: {columns:?}"
        );

        let mut named: Vec<&str> = MERGED_COLUMNS
            .iter()
            .map(|(column, _)| *column)
            .chain(KEPT_COLUMNS.iter().copied())
            .chain(UNTOUCHED_COLUMNS.iter().copied())
            .collect();
        let mut table: Vec<&str> = columns.iter().map(String::as_str).collect();
        assert_eq!(
            named.len(),
            table.len(),
            "a column is in two lists, or in none: {named:?} against {table:?}"
        );
        named.sort_unstable();
        table.sort_unstable();
        assert_eq!(
            named, table,
            "the fold rules and the table have drifted apart"
        );
    }

    /// Teeth for the guard above: the `SELECT` list it used to be held
    /// to is **not** the table, and the columns that differ are exactly
    /// the ones the old spelling could not see.
    ///
    /// Without this, "point it at the table" is a change no assertion
    /// distinguishes from the one it replaced.
    ///
    /// `external_key` used to be one of the five and is now read: the
    /// column had no reader while a UNIQUE index was the only thing that
    /// looked at it, and routing a source-stated id onto it made a
    /// standing `None` on the entity a lie. The four `has_*` flags stay
    /// unread, so the gap this asserts is still a real one.
    #[tokio::test]
    async fn the_read_list_is_narrower_than_the_table() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let columns: Vec<String> = isle
            .call(|conn| {
                let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('asset')")?;
                stmt.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        let read: Vec<&str> = AssetRow::COLUMNS.split(',').map(str::trim).collect();
        let mut missing: Vec<&str> = columns
            .iter()
            .map(String::as_str)
            .filter(|column| !read.contains(column))
            .collect();
        missing.sort_unstable();
        assert_eq!(
            missing,
            // `dims_probed_at` joins the four `has_*` flags for the same
            // reason they are here: it is written by one walk and read by
            // that walk's own predicate, and no entity read wants it. A
            // column the reader does not select is exactly the kind the
            // fold rules have to name for themselves.
            vec![
                "dims_probed_at",
                "has_code",
                "has_link",
                "has_mermaid",
                "has_table"
            ],
            "these are the columns a fold rule has to name explicitly, \
             because no reader's SELECT list mentions them"
        );
    }

    // ---- coded pixel dimensions (V69) ----------------------------
    //
    // Every fixture below uses a width that differs from its height: the
    // two columns are independent `Option<u32>`s, so a square fixture
    // passes a transposed read or write on either side.

    /// The two columns as the row holds them — `Value`, not
    /// `Option<i64>`, so `NULL` and `0` are two different answers here
    /// instead of one.
    async fn stored_dims(isle: &AsyncIsle, id: &AssetId) -> (Value, Value) {
        let uuid = *id.as_uuid();
        isle.call(move |conn| {
            conn.query_row(
                "SELECT width_px, height_px FROM asset WHERE id = ?1",
                params![uuid],
                |row| Ok((row.get::<_, Value>(0)?, row.get::<_, Value>(1)?)),
            )
        })
        .await
        .unwrap()
    }

    /// Writes the two columns straight, bypassing `save`.
    ///
    /// The only way to stage a value the entity cannot hold: `u32` →
    /// `i64` is lossless over the whole range, so a fixture built through
    /// the repository can never produce an out-of-range row and a test
    /// that tried would assert nothing.
    async fn force_stored_dims(isle: &AsyncIsle, id: &AssetId, width: i64, height: i64) {
        let uuid = *id.as_uuid();
        isle.call(move |conn| {
            let touched = conn.execute(
                "UPDATE asset SET width_px = ?1, height_px = ?2 WHERE id = ?3",
                params![width, height, uuid],
            )?;
            assert_eq!(touched, 1, "the fixture row is not there");
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A measured pair survives `save` → `find`, and a `(0, 0)` pair is
    /// carried as a stated zero rather than folded into absence.
    ///
    /// The zero half is the direction the "absent is not zero" rule does
    /// **not** cover: `None` → `NULL` is asserted one test down, and
    /// without this one a `map(|v| if v == 0 { None } else { .. })`
    /// anywhere on the road would pass.
    #[tokio::test]
    async fn a_measured_pair_round_trips_and_a_zero_pair_stays_a_zero() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut measured = item(persona, "/pics/measured.png");
        measured.width_px = Some(1920);
        measured.height_px = Some(1080);
        let mut zeroed = item(persona, "/pics/zeroed.png");
        zeroed.width_px = Some(0);
        zeroed.height_px = Some(0);
        repo.save(&measured).await.unwrap();
        repo.save(&zeroed).await.unwrap();

        let read = repo.find(&measured.id).await.unwrap().unwrap();
        assert_eq!(
            (read.width_px, read.height_px),
            (Some(1920), Some(1080)),
            "the pair came back transposed or lost"
        );
        assert_eq!(
            stored_dims(&isle, &measured.id).await,
            (Value::Integer(1920), Value::Integer(1080)),
            "and the columns hold the two numbers in that order"
        );

        let read = repo.find(&zeroed.id).await.unwrap().unwrap();
        assert_eq!(
            (read.width_px, read.height_px),
            (Some(0), Some(0)),
            "a stated zero is a statement; nothing on this road may read \
             it as 'nobody measured'"
        );
        assert_eq!(
            stored_dims(&isle, &zeroed.id).await,
            (Value::Integer(0), Value::Integer(0)),
            "and it reaches the columns as 0, not NULL"
        );
    }

    /// An asset nobody measured stores `NULL`, and never `0`.
    ///
    /// `0` would be a measurement: it sorts ahead of every real value on
    /// an ascending axis and there is no way back from it to "nobody
    /// looked". Read as `Value` so the assertion can tell the two apart —
    /// an `Option<i64>` getter would report `Some(0)` and `None`
    /// faithfully, but a `row.get::<_, i64>` would not.
    #[tokio::test]
    async fn an_unmeasured_asset_stores_null_rather_than_zero() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let unmeasured = item(persona, "/pics/unmeasured.png");
        assert_eq!(
            (unmeasured.width_px, unmeasured.height_px),
            (None, None),
            "a fresh entity states nothing"
        );
        repo.save(&unmeasured).await.unwrap();

        assert_eq!(
            stored_dims(&isle, &unmeasured.id).await,
            (Value::Null, Value::Null),
            "an unmeasured asset must not claim a zero-pixel resolution"
        );
        // …and the row is still readable, which is what a library that
        // predates the column needs (every one of its rows is this row).
        let read = repo.find(&unmeasured.id).await.unwrap().unwrap();
        assert_eq!((read.width_px, read.height_px), (None, None));
    }

    /// **The arriving value wins, including when it is nothing.**
    ///
    /// A re-ingest whose probe failed replaces a measurement with `NULL`.
    /// That is a choice, not an oversight: dimensions come out of the same
    /// probe pass as `duration_ms` and `file_size_bytes`, and giving these
    /// two columns a `COALESCE` would make one pass's results follow two
    /// different overwrite rules. Pinned here so changing it has to be
    /// deliberate.
    #[tokio::test]
    async fn a_save_that_states_no_dimensions_clears_the_measured_ones() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut asset = item(persona, "/pics/reingested.png");
        asset.width_px = Some(3840);
        asset.height_px = Some(2160);
        repo.save(&asset).await.unwrap();
        assert_eq!(
            stored_dims(&isle, &asset.id).await,
            (Value::Integer(3840), Value::Integer(2160)),
            "the row starts out measured, or the overwrite below proves \
             nothing"
        );

        asset.width_px = None;
        asset.height_px = None;
        repo.save(&asset).await.unwrap();

        assert_eq!(
            stored_dims(&isle, &asset.id).await,
            (Value::Null, Value::Null),
            "the upsert leaves the incoming value standing — a probe that \
             failed erases what a probe that worked had written"
        );
    }

    /// A stored value outside `u32` is a **read error**, not a cast.
    ///
    /// Both ends, because both are silent under `as u32`: `4294967296`
    /// becomes `0` — a value that reads as a measurement and sorts ahead
    /// of every real one — and `-1` becomes `4294967295`. The rows are
    /// staged with SQL for the reason `force_stored_dims` states.
    #[tokio::test]
    async fn a_stored_dimension_outside_u32_is_refused_rather_than_cast() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let over = item(persona, "/pics/over.png");
        let under = item(persona, "/pics/under.png");
        repo.save(&over).await.unwrap();
        repo.save(&under).await.unwrap();
        // Readable before the columns are corrupted, so the failures
        // below are about the values and not about the fixture.
        assert!(repo.find(&over.id).await.unwrap().is_some());

        force_stored_dims(&isle, &over.id, 4_294_967_296, 1_080).await;
        force_stored_dims(&isle, &under.id, -1, 1_080).await;

        let err = repo
            .find(&over.id)
            .await
            .expect_err("4294967296 pixels is not 0 pixels");
        assert!(
            err.to_string().contains("width_px"),
            "the error names the column: {err}"
        );
        let err = repo
            .find(&under.id)
            .await
            .expect_err("-1 pixels is not 4294967295 pixels");
        assert!(err.to_string().contains("width_px"), "{err}");
    }

    /// The keeper's dimensions stand and the headstone's are written
    /// down — the run-time consequence of putting the two columns in
    /// [`KEPT_COLUMNS`].
    ///
    /// Note what the note can and cannot say: the walk judges one column
    /// at a time, so this fixture (which differs on both) leaves two
    /// entries, and a pair differing only in height would leave one —
    /// with nothing recording that the number was half of a resolution.
    /// That is a property of the note's shape, not of this list.
    #[tokio::test]
    async fn a_fold_keeps_the_keepers_dimensions_and_records_the_headstones() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut keeper = item(persona, "/pics/keeper.png");
        keeper.width_px = Some(1920);
        keeper.height_px = Some(1080);
        let mut headstone = item(persona, "/pics/copy.png");
        headstone.width_px = Some(1280);
        headstone.height_px = Some(720);
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();
        assert_ne!(
            stored_dims(&isle, &keeper.id).await,
            stored_dims(&isle, &headstone.id).await,
            "the two rows disagree before the fold, or every rule below \
             passes with its implementation deleted"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        let after = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(
            (after.width_px, after.height_px),
            (Some(1920), Some(1080)),
            "a fold has no basis for combining two measurements, so the \
             keeper's stand"
        );

        let note = absorbed_note(&repo, &keeper.id).await;
        let discarded = note.get("discarded").expect("the note lists what was left");
        assert_eq!(
            discarded.get("width_px").and_then(|v| v.as_i64()),
            Some(1280),
            "the resolution nobody will see again is unreadable: {note}"
        );
        assert_eq!(
            discarded.get("height_px").and_then(|v| v.as_i64()),
            Some(720),
            "{note}"
        );
    }

    // ---- the duplicate re-scan's walk ----------------------------

    /// **The two material walks partition the table.**
    ///
    /// `scan_unhashed_materials` finds work by asking whether the
    /// fingerprint columns are empty; this one asks the same question
    /// and takes the other answer. If they ever stop being complements,
    /// a row falls into the gap — hashed by nobody and re-derived by
    /// nobody — and nothing else in the system would notice, because
    /// each walk terminates on its own predicate quite happily.
    ///
    /// Asserted as a partition rather than as two independent contents:
    /// disjoint *and* covering, over a fixture that holds one of each
    /// kind.
    #[tokio::test]
    async fn the_two_material_walks_partition_the_table() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let hashed = item(persona, "/pics/hashed.png");
        let marked = item(persona, "/notes/no-bytes.md");
        let pending = item(persona, "/pics/pending.png");
        for asset in [&hashed, &marked, &pending] {
            repo.save(asset).await.unwrap();
        }
        repo.set_material_fingerprint(&hashed.id, 0, &file_axis("sha256:aaaa"))
            .await
            .unwrap();
        // A marker is an answer, not an absence — the row belongs to the
        // fingerprinted side even though no digest was taken.
        repo.set_material_fingerprint(
            &marked.id,
            0,
            &file_axis(asterism_core::domain::content_hash::UNHASHABLE),
        )
        .await
        .unwrap();

        let done: Vec<_> = repo
            .scan_fingerprinted_materials(None, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.asset_id)
            .collect();
        let todo: Vec<_> = repo
            .scan_unhashed_materials(None, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.asset_id)
            .collect();

        let mut both = done.clone();
        both.retain(|id| todo.contains(id));
        assert!(both.is_empty(), "a row is in both walks: {both:?}");

        let mut all = done.clone();
        all.extend(todo.iter().copied());
        all.sort_by_key(|id| *id.as_uuid());
        let mut want = vec![hashed.id, marked.id, pending.id];
        want.sort_by_key(|id| *id.as_uuid());
        assert_eq!(all, want, "a row is in neither walk");

        assert!(done.contains(&marked.id), "a marker is an answered axis");
        assert!(todo.contains(&pending.id));
    }

    /// The re-scan hands back the digests the row holds, unchanged.
    ///
    /// It is what `detect_duplicate` compares, and it has to be what the
    /// hashing pass handed the same function — otherwise the two
    /// derivations of one conflict could disagree about what was
    /// measured.
    #[tokio::test]
    async fn the_rescan_carries_the_stored_fingerprint_verbatim() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let asset = item(persona, "/pics/one.png");
        repo.save(&asset).await.unwrap();
        let written = file_axis("sha256:abcdef");
        repo.set_material_fingerprint(&asset.id, 0, &written)
            .await
            .unwrap();

        let page = repo.scan_fingerprinted_materials(None, 50).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].ord, 0);
        assert_eq!(page[0].fingerprint, written);
    }

    /// The cursor is the composite `(asset_id, ord)`, exclusive.
    ///
    /// An `asset_id`-only cursor would skip the remaining `ord > 0`
    /// materials of an asset a page boundary cut through — the same trap
    /// the hashing walk documents, and the reason both carry the pair.
    #[tokio::test]
    async fn the_rescan_pages_forward_without_skipping_a_cut_asset() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        // Three materials on one asset. `item` builds the primary only,
        // so the siblings are copies of it at the next two positions —
        // `set_material_fingerprint` updates a row and does not mint
        // one, so the rows have to exist before the digests do.
        let mut asset = item(persona, "/pics/multi.png");
        let primary = asset.materials[0].clone();
        for ord in 1..3u32 {
            asset.materials.push(Material {
                ord,
                ..primary.clone()
            });
        }
        repo.save(&asset).await.unwrap();
        for ord in 0..3u32 {
            repo.set_material_fingerprint(
                &asset.id,
                ord,
                &file_axis(&format!("sha256:{ord}{ord}")),
            )
            .await
            .unwrap();
        }

        let first = repo.scan_fingerprinted_materials(None, 2).await.unwrap();
        assert_eq!(
            first.iter().map(|m| m.ord).collect::<Vec<_>>(),
            vec![0, 1],
            "the page bounds the read"
        );
        // Cut mid-asset. An id-only cursor would resume past the whole
        // asset and lose `ord = 2`.
        let next = repo
            .scan_fingerprinted_materials(Some((&asset.id, 1)), 50)
            .await
            .unwrap();
        assert_eq!(
            next.iter().map(|m| m.ord).collect::<Vec<_>>(),
            vec![2],
            "the cursor resumes inside the asset it cut"
        );
    }

    // ---- the dimension backfill's walk (V71) ---------------------

    /// `dims_probed_at` as the row holds it.
    async fn stored_probe_stamp(isle: &AsyncIsle, id: &AssetId) -> Value {
        let uuid = *id.as_uuid();
        isle.call(move |conn| {
            conn.query_row(
                "SELECT dims_probed_at FROM asset WHERE id = ?1",
                params![uuid],
                |row| row.get(0),
            )
        })
        .await
        .unwrap()
    }

    /// **A row nobody could measure leaves the walk anyway.**
    ///
    /// This is the whole reason `dims_probed_at` exists, and the one
    /// property that separates this walk from a `width_px IS NULL`
    /// predicate. A text note has no dimensions and never will; offered
    /// again on the next pass it would be re-read forever, and the
    /// startup seed would make "forever" mean "every launch".
    ///
    /// The measured row is in the fixture as the control: both leave,
    /// and they leave for different reasons.
    #[tokio::test]
    async fn a_probe_that_measured_nothing_still_retires_the_row() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let note = item(persona, "/notes/plain.md");
        let picture = item(persona, "/pics/shot.png");
        repo.save(&note).await.unwrap();
        repo.save(&picture).await.unwrap();

        let page = repo
            .scan_dims_candidates(DimsScope::Unlooked, None, 10)
            .await
            .unwrap();
        assert_eq!(page.len(), 2, "both rows start out unlooked-at");

        let now = chrono::Utc::now();
        repo.record_dims_probe(
            &note.id,
            DimsProbe::NothingToMeasure,
            DimsWritePolicy::FillOnly,
            now,
        )
        .await
        .unwrap();
        repo.record_dims_probe(
            &picture.id,
            DimsProbe::Measured(4000, 1000),
            DimsWritePolicy::FillOnly,
            now,
        )
        .await
        .unwrap();

        assert!(
            repo.scan_dims_candidates(DimsScope::Unlooked, None, 10)
                .await
                .unwrap()
                .is_empty(),
            "a probed row is out of the walk whatever it measured"
        );
        // The note is out *and* still unmeasured — the stamp is the
        // thing that retired it, not a stand-in dimension.
        assert_eq!(
            stored_dims(&isle, &note.id).await,
            (Value::Null, Value::Null),
            "no measurement was invented to end the walk"
        );
        assert!(matches!(
            stored_probe_stamp(&isle, &note.id).await,
            Value::Integer(_)
        ));
        assert_eq!(
            stored_dims(&isle, &picture.id).await,
            (Value::Integer(4000), Value::Integer(1000)),
            "and the measured row kept its pair, unswapped"
        );
    }

    /// The cursor walks forward and the page bounds the read.
    #[tokio::test]
    async fn the_walk_pages_forward_from_its_cursor() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut ids = Vec::new();
        for n in 0..5 {
            let row = item(persona, &format!("/pics/{n}.png"));
            repo.save(&row).await.unwrap();
            ids.push(row.id);
        }
        // Ordered by id (UUID v7 = time-ordered), which is what makes a
        // cursor resumable rather than a re-scan.
        ids.sort_by_key(|id| *id.as_uuid());

        let first = repo
            .scan_dims_candidates(DimsScope::Unlooked, None, 2)
            .await
            .unwrap();
        let first_ids: Vec<_> = first.iter().map(|r| r.asset_id).collect();
        assert_eq!(first_ids, ids[..2], "the page bounds the read");

        let next = repo
            .scan_dims_candidates(DimsScope::Unlooked, Some(&ids[1]), 10)
            .await
            .unwrap();
        let next_ids: Vec<_> = next.iter().map(|r| r.asset_id).collect();
        assert_eq!(
            next_ids,
            ids[2..],
            "the cursor is exclusive — a page boundary must not re-offer \
             its own last row, nor skip the one after it"
        );
    }

    /// The walk reads the asset's own locator, which is the artefact the
    /// importer measured at ingest.
    ///
    /// Measuring anything else — a derived material, a thumbnail — would
    /// put a second meaning into one column.
    #[tokio::test]
    async fn the_walk_hands_back_the_assets_own_locator() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let row = item(persona, "/pics/original.png");
        repo.save(&row).await.unwrap();

        let page = repo
            .scan_dims_candidates(DimsScope::Unlooked, None, 10)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].locator, row.source.locator);
    }

    /// A pair that arrived while the walk was in flight is not
    /// overwritten, and the row still leaves the walk.
    ///
    /// The race is real: the scan and the write are separate awaits, and
    /// an ingest can fill the columns in between. What that ingest wrote
    /// came off the artefact at import time, which is the better
    /// evidence — the backfill is the fallback, not the authority.
    #[tokio::test]
    async fn a_probe_does_not_overwrite_dimensions_that_arrived_meanwhile() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut row = item(persona, "/pics/raced.png");
        row.width_px = Some(4000);
        row.height_px = Some(1000);
        repo.save(&row).await.unwrap();

        repo.record_dims_probe(
            &row.id,
            DimsProbe::Measured(16, 9),
            DimsWritePolicy::FillOnly,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        assert_eq!(
            stored_dims(&isle, &row.id).await,
            (Value::Integer(4000), Value::Integer(1000)),
            "the ingest measurement stands"
        );
        assert!(
            repo.scan_dims_candidates(DimsScope::Unlooked, None, 10)
                .await
                .unwrap()
                .is_empty(),
            "and the row is out of the walk regardless"
        );
    }

    /// The stamp is not a modification of the asset.
    ///
    /// `updated_at` is the paging axis for differential sync
    /// (`ListAssetsQuery::updated_from_ms`). Moving it here would make
    /// every consumer re-fetch the whole library because the server
    /// audited it — a change to the record of a read, reported as a
    /// change to the thing read.
    #[tokio::test]
    async fn recording_a_probe_does_not_move_updated_at() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let row = item(persona, "/pics/stamped.png");
        repo.save(&row).await.unwrap();
        let before = repo.find(&row.id).await.unwrap().unwrap().updated_at;

        repo.record_dims_probe(
            &row.id,
            DimsProbe::Measured(4000, 1000),
            DimsWritePolicy::FillOnly,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let after = repo.find(&row.id).await.unwrap().unwrap().updated_at;
        assert_eq!(before, after, "a read is not a write to the asset");
    }

    /// **An unreadable artefact leaves no trace, so it comes back.**
    ///
    /// The one outcome that writes nothing. A volume that was not
    /// mounted when the pass ran must not have retired its whole library
    /// permanently — the row stays in every scope it was in, and a later
    /// pass reaches it.
    ///
    /// Both scopes are checked because a stamp would remove it from one
    /// and a written pair from the other; leaving no trace has to mean
    /// both.
    #[tokio::test]
    async fn an_unreadable_artefact_records_nothing_and_stays_in_every_scope() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let row = item(persona, "/pics/on-a-drive-that-is-out.png");
        repo.save(&row).await.unwrap();

        repo.record_dims_probe(
            &row.id,
            DimsProbe::Unreadable,
            DimsWritePolicy::Overwrite,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        assert_eq!(
            stored_probe_stamp(&isle, &row.id).await,
            Value::Null,
            "a failed read is not an answer, so it leaves no stamp"
        );
        assert_eq!(
            stored_dims(&isle, &row.id).await,
            (Value::Null, Value::Null)
        );
        for scope in [DimsScope::Unlooked, DimsScope::Unmeasured, DimsScope::All] {
            let page = repo.scan_dims_candidates(scope, None, 10).await.unwrap();
            assert_eq!(page.len(), 1, "{scope:?} must still offer the row");
        }
    }

    /// The three scopes select three different sets.
    ///
    /// Asserted together because the value of each is what the others
    /// leave out: `Unlooked` is the pass that terminates, `Unmeasured`
    /// re-offers rows it retired, and `All` reaches rows that already
    /// have an answer.
    #[tokio::test]
    async fn each_scope_selects_its_own_set() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let fresh = item(persona, "/pics/never-looked-at.png");
        let note = item(persona, "/notes/looked-at-no-dims.md");
        let measured = item(persona, "/pics/measured.png");
        for row in [&fresh, &note, &measured] {
            repo.save(row).await.unwrap();
        }
        let now = chrono::Utc::now();
        repo.record_dims_probe(
            &note.id,
            DimsProbe::NothingToMeasure,
            DimsWritePolicy::FillOnly,
            now,
        )
        .await
        .unwrap();
        repo.record_dims_probe(
            &measured.id,
            DimsProbe::Measured(4000, 1000),
            DimsWritePolicy::FillOnly,
            now,
        )
        .await
        .unwrap();

        let ids_in = |scope| {
            let repo = repo.clone();
            async move {
                let mut ids: Vec<_> = repo
                    .scan_dims_candidates(scope, None, 10)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|r| r.asset_id)
                    .collect();
                ids.sort_by_key(|id| *id.as_uuid());
                ids
            }
        };
        let sorted = |mut ids: Vec<AssetId>| {
            ids.sort_by_key(|id| *id.as_uuid());
            ids
        };

        assert_eq!(
            ids_in(DimsScope::Unlooked).await,
            vec![fresh.id],
            "Unlooked skips everything already probed — that is what lets \
             the startup pass finish"
        );
        assert_eq!(
            ids_in(DimsScope::Unmeasured).await,
            sorted(vec![fresh.id, note.id]),
            "Unmeasured re-offers the probed-but-unmeasured row"
        );
        assert_eq!(
            ids_in(DimsScope::All).await,
            sorted(vec![fresh.id, note.id, measured.id]),
            "All reaches the row that already has an answer"
        );
    }

    /// **`Overwrite` replaces a stored measurement; `FillOnly` cannot.**
    ///
    /// The distinction the whole write-policy parameter exists for. With
    /// only `FillOnly` — the shape this replaced — a caller asking to
    /// re-measure reads the artefact and changes nothing, which looks
    /// exactly like success.
    #[tokio::test]
    async fn overwrite_replaces_a_measurement_and_fill_only_does_not() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut row = item(persona, "/pics/replaced-on-disk.png");
        row.width_px = Some(4000);
        row.height_px = Some(1000);
        repo.save(&row).await.unwrap();

        repo.record_dims_probe(
            &row.id,
            DimsProbe::Measured(1600, 900),
            DimsWritePolicy::FillOnly,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(
            stored_dims(&isle, &row.id).await,
            (Value::Integer(4000), Value::Integer(1000)),
            "FillOnly leaves a stored measurement alone"
        );

        repo.record_dims_probe(
            &row.id,
            DimsProbe::Measured(1600, 900),
            DimsWritePolicy::Overwrite,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(
            stored_dims(&isle, &row.id).await,
            (Value::Integer(1600), Value::Integer(900)),
            "Overwrite is what makes a re-measure mean anything"
        );

        // …and it clears, too: a re-measure that came back empty is an
        // answer, not a reason to keep the old one.
        repo.record_dims_probe(
            &row.id,
            DimsProbe::NothingToMeasure,
            DimsWritePolicy::Overwrite,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(
            stored_dims(&isle, &row.id).await,
            (Value::Null, Value::Null),
            "the caller asked, and this is what came back"
        );
    }

    /// The columns of one asset row that differ between two reads, by
    /// name. Built from `AssetRow::COLUMNS`, so it keeps naming
    /// whatever the table grows.
    fn changed_columns(before: &[Value], after: &[Value]) -> Vec<String> {
        AssetRow::COLUMNS
            .split(',')
            .map(str::trim)
            .zip(before.iter().zip(after.iter()))
            .filter(|(_, (before, after))| before != after)
            .map(|(column, _)| column.to_string())
            .collect()
    }

    /// The losing row becomes a headstone pointing at the keeper, and
    /// the keeper's columns move **only where a rule says so**.
    ///
    /// This is the shape the structural wave's "no column of the keeper
    /// changed" assertion turned into. Deleting it would have left the
    /// boundary unguarded; a bare allow-list would leave a column added
    /// next year free to join the merge silently. So the fixture
    /// disagrees on every combinable column and the assertion is an
    /// **equality** on the set that moved: a column that starts being
    /// merged shows up here uninvited, and one that stops being merged
    /// goes missing.
    #[tokio::test]
    async fn a_fold_changes_only_the_columns_a_rule_names() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut keeper = item(persona, "/pics/keeper.png");
        keeper.labels = vec![Label::new("keeper-label").unwrap()];
        keeper.keywords = vec![Keyword::new("keeper-word").unwrap()];
        keeper.rating = Some(2);
        keeper.register_note = Some(RegisterNote::new("what the keeper says").unwrap());
        let mut headstone = item(persona, "/pics/copy.png");
        // Every combinable column disagrees, and so do several that a
        // rule refuses to combine — a fixture where the two sides
        // already agree would pass with the merge deleted.
        headstone.labels = vec![Label::new("copy-label").unwrap()];
        headstone.keywords = vec![Keyword::new("copy-word").unwrap()];
        headstone.rating = Some(5);
        headstone.register_note = Some(RegisterNote::new("what the copy says").unwrap());
        headstone.visibility = Visibility::Restricted {
            sharing: vec!["alice".into()],
        };
        headstone.title = Some("the copy".into());
        headstone.cover = Some(CoverText::new("a cover only the copy has").unwrap());
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        let before = raw_row(&isle, &keeper.id).await;
        assert_ne!(
            raw_row(&isle, &headstone.id).await,
            before,
            "the two rows have to disagree before the fold means anything"
        );
        assert_eq!(
            repo.find(&headstone.id).await.unwrap().unwrap().folded_into,
            None,
            "the fixture starts with two live rows"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert!(
            matches!(outcome, FoldOutcome::Folded(_)),
            "the fold was refused: {outcome:?}"
        );

        assert_eq!(
            repo.find(&headstone.id).await.unwrap().unwrap().folded_into,
            Some(keeper.id),
            "the headstone points at the keeper"
        );

        let mut changed = changed_columns(&before, &raw_row(&isle, &keeper.id).await);
        changed.sort();
        let mut allowed = vec![
            // Combined by a rule.
            "labels",
            "keywords",
            "vis_sharing",
            "vis_restricted",
            "rating",
            "register_note",
            // The note about what was *not* taken, and the stamp that
            // tells a differential sync any of this happened.
            "extra",
            "updated_at",
        ];
        allowed.sort();
        assert_eq!(
            changed, allowed,
            "the set of columns a fold moves is not the set the rules name"
        );
    }

    /// The locator stays with the headstone and is not copied onto the
    /// keeper: the row that holds a Source value is the one that
    /// was imported from there, and a re-arrival at that path has to
    /// resolve *through* the headstone for the duplicate to stay
    /// resolved. Handing the value to the keeper would take it from the
    /// row that answers to it.
    ///
    /// Asked in the `Any` scope, which is the one that reports storage
    /// as it stands. `Live` resolves the redirection this test is about
    /// and would answer with the keeper for either arrangement of the
    /// column — it cannot tell them apart, so it cannot be the witness
    /// here.
    #[tokio::test]
    async fn a_fold_leaves_the_locator_with_the_headstone() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        let by_path = repo
            .find_by_source(
                &persona,
                &SourceKind::new(SourceKind::FS).unwrap(),
                &loc("/pics/copy.png"),
                SourceLookupScope::Any,
            )
            .await
            .unwrap()
            .expect("the path is still known");
        assert_eq!(by_path.id, headstone.id, "the headstone still holds it");
        assert_eq!(
            repo.find(&keeper.id).await.unwrap().unwrap().source.locator,
            loc("/pics/keeper.png"),
            "the keeper did not inherit the other path"
        );
    }

    /// Edges naming the headstone name the keeper afterwards — on both
    /// sides, since the storage is directional and `IdenticalTo` writes
    /// the incumbent on the `to` side.
    #[tokio::test]
    async fn a_fold_repoints_the_edges_of_the_folded_row() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        let neighbour = item(persona, "/pics/neighbour.png");
        for asset in [&keeper, &headstone, &neighbour] {
            repo.save(asset).await.unwrap();
        }
        seed_edge(&isle, &headstone.id, &neighbour.id, "derived_from", "out").await;
        seed_edge(&isle, &neighbour.id, &headstone.id, "reference", "in").await;

        assert_eq!(
            all_edges(&isle).await,
            {
                let mut want = vec![
                    (
                        *headstone.id.as_uuid(),
                        *neighbour.id.as_uuid(),
                        "derived_from".to_string(),
                    ),
                    (
                        *neighbour.id.as_uuid(),
                        *headstone.id.as_uuid(),
                        "reference".to_string(),
                    ),
                ];
                want.sort();
                want
            },
            "both edges start on the row about to be folded"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                edges_repointed: 2,
                // The pair disagrees about one kept column — its
                // locator, which no two rows can share.
                values_discarded: 1,
                ..FoldReport::default()
            })
        );

        let mut want = vec![
            (
                *keeper.id.as_uuid(),
                *neighbour.id.as_uuid(),
                "derived_from".to_string(),
            ),
            (
                *neighbour.id.as_uuid(),
                *keeper.id.as_uuid(),
                "reference".to_string(),
            ),
        ];
        want.sort();
        assert_eq!(all_edges(&isle).await, want);
    }

    /// The two shapes a re-point cannot take: the pair's own edge
    /// (which would become a keeper→keeper self-loop the `edge` table
    /// has no constraint against) and an edge whose `(from, to, kind)`
    /// the keeper already holds. Neither may abort the fold, and
    /// neither may vanish without a record.
    #[tokio::test]
    async fn a_fold_drops_the_edges_it_cannot_move_and_writes_down_what_they_claimed() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        let neighbour = item(persona, "/pics/neighbour.png");
        for asset in [&keeper, &headstone, &neighbour] {
            repo.save(asset).await.unwrap();
        }
        // The detection edge: "this newcomer holds the same bytes".
        seed_edge(&isle, &headstone.id, &keeper.id, "identical_to", "artefact").await;
        // A claim both rows already make about the same neighbour.
        seed_edge(
            &isle,
            &headstone.id,
            &neighbour.id,
            "derived_from",
            "from the copy",
        )
        .await;
        seed_edge(
            &isle,
            &keeper.id,
            &neighbour.id,
            "derived_from",
            "from the keeper",
        )
        .await;

        assert_eq!(all_edges(&isle).await.len(), 3, "three edges to start");

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                edges_dropped: 2,
                values_discarded: 1,
                ..FoldReport::default()
            }),
            "one self-loop and one collision, neither of them re-pointed"
        );

        assert_eq!(
            all_edges(&isle).await,
            vec![(
                *keeper.id.as_uuid(),
                *neighbour.id.as_uuid(),
                "derived_from".to_string()
            )],
            "no keeper→keeper self-loop, and the keeper's own edge survived"
        );

        // What the dropped rows claimed is on the headstone's `_trace`.
        let folded = repo.find(&headstone.id).await.unwrap().unwrap();
        let note = folded
            .extra
            .get(asterism_core::domain::provenance::TRACE_KEY)
            .and_then(|t| t.get("fold"))
            .expect("the fold left a note");
        assert_eq!(
            note.get("keeper").and_then(|v| v.as_str()),
            Some(keeper.id.to_string().as_str())
        );
        let dropped = note
            .get("dropped_edges")
            .and_then(|v| v.as_array())
            .expect("the note lists them");
        let kinds: Vec<&str> = dropped
            .iter()
            .filter_map(|e| e.get("kind").and_then(|v| v.as_str()))
            .collect();
        assert!(
            kinds.contains(&"identical_to") && kinds.contains(&"derived_from"),
            "both dropped claims are named: {dropped:?}"
        );
        let labels: Vec<&str> = dropped
            .iter()
            .filter_map(|e| e.get("label").and_then(|v| v.as_str()))
            .collect();
        assert!(
            labels.contains(&"artefact") && labels.contains(&"from the copy"),
            "the labels went with them: {dropped:?}"
        );
    }

    /// Group membership moves, and a Group the keeper was already filed
    /// in keeps the keeper's own position rather than the headstone's.
    #[tokio::test]
    async fn a_fold_moves_group_membership_without_disturbing_what_the_keeper_had() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        let only_headstone = seed_bucket(&isle, persona, "reference shots").await;
        let shared = seed_bucket(&isle, persona, "everything").await;
        file_in_bucket(&isle, &headstone.id, only_headstone, 7).await;
        file_in_bucket(&isle, &headstone.id, shared, 3).await;
        file_in_bucket(&isle, &keeper.id, shared, 1).await;

        assert_eq!(
            filing_of(&isle, &keeper.id).await,
            vec![(shared, 1)],
            "the keeper is in one Group before the fold"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                buckets_moved: 1,
                // The locator, and the position the headstone held in
                // the Group both rows were filed in.
                values_discarded: 2,
                ..FoldReport::default()
            }),
            "only the Group it was not already in counts as moved"
        );

        let mut want = vec![(only_headstone, 7), (shared, 1)];
        want.sort();
        let mut got = filing_of(&isle, &keeper.id).await;
        got.sort();
        assert_eq!(
            got, want,
            "the keeper gained the other Group and kept its own position in the shared one"
        );
        assert!(
            filing_of(&isle, &headstone.id).await.is_empty(),
            "a headstone is filed nowhere"
        );
    }

    /// Cards filed inside the folded row are re-filed inside the
    /// keeper. Without this they would hang off a container the grid
    /// does not show, which is the same as being gone.
    #[tokio::test]
    async fn a_fold_refiles_the_children_of_the_folded_row() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/sessions/keeper");
        let headstone = item(persona, "/sessions/copy");
        let mut child = item(persona, "/sessions/copy/msg-1.md");
        child.container_id = Some(headstone.id);
        for asset in [&keeper, &headstone, &child] {
            repo.save(asset).await.unwrap();
        }

        assert_eq!(
            container_of(&isle, &child.id).await,
            Some(*headstone.id.as_uuid()),
            "the child starts inside the row about to be folded"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                children_repointed: 1,
                values_discarded: 1,
                ..FoldReport::default()
            })
        );
        assert_eq!(
            container_of(&isle, &child.id).await,
            Some(*keeper.id.as_uuid()),
            "the child is now inside the keeper"
        );
    }

    /// Tags move, and a tag both rows carried does not become two rows
    /// (the link table's key would refuse the second one, aborting the
    /// fold, if the write were not `OR IGNORE`).
    #[tokio::test]
    async fn a_fold_moves_the_tag_links_without_duplicating_them() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        let only_headstone = seed_tag(&isle, "dusk").await;
        let shared = seed_tag(&isle, "sunset").await;
        link_tag(&isle, &headstone.id, only_headstone).await;
        link_tag(&isle, &headstone.id, shared).await;
        link_tag(&isle, &keeper.id, shared).await;

        assert_eq!(
            tags_of(&isle, &keeper.id).await,
            vec![shared],
            "the keeper carries one tag before the fold"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                tags_moved: 1,
                values_discarded: 1,
                ..FoldReport::default()
            }),
            "only the tag it did not already carry counts as moved"
        );

        let mut want = vec![only_headstone, shared];
        want.sort();
        let mut got = tags_of(&isle, &keeper.id).await;
        got.sort();
        assert_eq!(got, want);
        assert!(
            tags_of(&isle, &headstone.id).await.is_empty(),
            "the links moved rather than being copied"
        );
    }

    /// Snapshot membership is **not** moved. A snapshot is
    /// addressed by the hash of its member set, so swapping a member
    /// would silently change what a frozen selection was — the dispatch
    /// history that names it would then describe a run over assets that
    /// were never in it.
    #[tokio::test]
    async fn a_fold_never_rewrites_a_snapshot() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        let snapshot = Uuid::now_v7();
        let (pid, member) = (*persona.as_uuid(), *headstone.id.as_uuid());
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO snapshot (id, persona_id, content_hash, created_at) \
                 VALUES (?1, ?2, 'sha256:frozen', 0)",
                params![snapshot, pid],
            )?;
            conn.execute(
                "INSERT INTO snapshot_asset (snapshot_id, asset_id, position) \
                 VALUES (?1, ?2, 0)",
                params![snapshot, member],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let members = |isle: AsyncIsle| async move {
            isle.call(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT asset_id FROM snapshot_asset ORDER BY position")?;
                stmt.query_map([], |row| row.get::<_, Uuid>(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .unwrap()
        };
        assert_eq!(
            members(isle.clone()).await,
            vec![*headstone.id.as_uuid()],
            "the snapshot names the row about to be folded"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        assert_eq!(
            members(isle.clone()).await,
            vec![*headstone.id.as_uuid()],
            "a content-addressed member set must not be edited by a fold"
        );
    }

    /// The re-read the fold does on its way in, one refusal at a time.
    /// Each case has to leave the database exactly as it found it —
    /// "refused" and "half folded" are not distinguishable to a caller
    /// that only checks for an error.
    #[tokio::test]
    async fn the_fold_guard_refuses_a_headstone_a_folded_keeper_and_a_trashed_keeper() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let already = item(persona, "/pics/already-folded.png");
        let candidate = item(persona, "/pics/candidate.png");
        let trashed = item(persona, "/pics/trashed-keeper.png");
        for asset in [&keeper, &already, &candidate, &trashed] {
            repo.save(asset).await.unwrap();
        }
        // Something the fold would move if it ran at all.
        let bucket = seed_bucket(&isle, persona, "watched").await;
        file_in_bucket(&isle, &candidate.id, bucket, 0).await;

        // (a) the row to fold is already a headstone.
        fold_into(&isle, &already.id, &keeper.id).await;
        assert_eq!(
            repo.fold_into(&already.id, &keeper.id).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::AlreadyFolded)
        );

        // (b) the keeper is itself a headstone — folding into it would
        // build a chain every reader would have to walk.
        assert_eq!(
            repo.fold_into(&candidate.id, &already.id).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::KeeperFolded)
        );

        // (c) the keeper is in the trash, where retention deletes it.
        repo.trash(&trashed.id, Utc::now()).await.unwrap();
        assert_eq!(
            repo.fold_into(&candidate.id, &trashed.id).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::KeeperTrashed)
        );

        // …and none of the three wrote anything.
        assert_eq!(
            repo.find(&candidate.id).await.unwrap().unwrap().folded_into,
            None,
            "a refused fold left a headstone behind"
        );
        assert_eq!(
            filing_of(&isle, &candidate.id).await,
            vec![(bucket, 0)],
            "a refused fold moved the filing anyway"
        );
        assert_eq!(
            filing_of(&isle, &keeper.id).await,
            vec![],
            "a refused fold filed something onto the keeper"
        );

        // Degenerate input is a refusal too, not a row that folds into
        // itself.
        assert_eq!(
            repo.fold_into(&candidate.id, &candidate.id).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::SameAsset)
        );
        let ghost = AssetId::new();
        assert_eq!(
            repo.fold_into(&ghost, &keeper.id).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::Missing)
        );
        assert_eq!(
            repo.fold_into(&candidate.id, &ghost).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::KeeperMissing)
        );
    }

    /// Running the same fold twice is safe. There is no retry in this
    /// job engine, but a backfill, a re-enqueue after a restart, or a
    /// person clicking twice all produce the second run — and the
    /// second one must not move anything a third party has since filed
    /// on the keeper.
    #[tokio::test]
    async fn folding_the_same_pair_twice_leaves_the_second_run_with_nothing_to_do() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();
        let bucket = seed_bucket(&isle, persona, "shots").await;
        file_in_bucket(&isle, &headstone.id, bucket, 4).await;

        assert!(matches!(
            repo.fold_into(&headstone.id, &keeper.id).await.unwrap(),
            FoldOutcome::Folded(_)
        ));
        let after_first = raw_row(&isle, &keeper.id).await;
        let filing_after_first = filing_of(&isle, &keeper.id).await;

        assert_eq!(
            repo.fold_into(&headstone.id, &keeper.id).await.unwrap(),
            FoldOutcome::Skipped(FoldRefusal::AlreadyFolded),
            "the second run recognises the fold that already happened"
        );
        assert_eq!(raw_row(&isle, &keeper.id).await, after_first);
        assert_eq!(filing_of(&isle, &keeper.id).await, filing_after_first);
    }

    // ---- the field rules -----------------------------------------
    //
    // One test per rule, never one test for all of them: a combined
    // assertion tells you the merge broke and leaves you to find out
    // which half. Each fixture also asserts that the two rows
    // **disagree before the fold** — over two rows that already agree,
    // every rule below passes with its implementation deleted.

    /// What the keeper wrote down about the last row it absorbed.
    async fn absorbed_note(repo: &SqliteAssetRepository, keeper: &AssetId) -> serde_json::Value {
        let asset = repo.find(keeper).await.unwrap().unwrap();
        asset
            .extra
            .get(asterism_core::domain::provenance::TRACE_KEY)
            .and_then(|trace| trace.get("absorbed"))
            .and_then(|absorbed| absorbed.as_array())
            .and_then(|entries| entries.last())
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "the keeper records nothing about what it absorbed: {}",
                    asset.extra
                )
            })
    }

    /// Sets are combined, and a value both rows carried does not become
    /// two. The keeper's own order survives, so a card does not
    /// re-shuffle its badges because somebody resolved a duplicate.
    #[tokio::test]
    async fn a_fold_unions_the_labels_and_the_keywords() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut keeper = item(persona, "/pics/keeper.png");
        keeper.labels = vec![Label::new("dusk").unwrap()];
        keeper.keywords = vec![Keyword::new("sunset").unwrap()];
        let mut headstone = item(persona, "/pics/copy.png");
        headstone.labels = vec![Label::new("dusk").unwrap(), Label::new("print").unwrap()];
        headstone.keywords = vec![Keyword::new("beach").unwrap()];
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        let before = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(before.labels.len(), 1, "the keeper starts with one label");
        assert_eq!(
            repo.find(&headstone.id)
                .await
                .unwrap()
                .unwrap()
                .keywords
                .len(),
            1,
            "and the other row carries a keyword it does not have"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        let after = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(
            after
                .labels
                .iter()
                .map(|l| l.as_str().to_string())
                .collect::<Vec<_>>(),
            vec!["dusk".to_string(), "print".to_string()],
            "the shared label is not doubled, and the keeper's order stands"
        );
        assert_eq!(
            after
                .keywords
                .iter()
                .map(|k| k.as_str().to_string())
                .collect::<Vec<_>>(),
            vec!["sunset".to_string(), "beach".to_string()]
        );
    }

    /// A row **already stored** with the same label twice is read back
    /// with one of them, on both projections the grid uses.
    ///
    /// The write side drops repeats now (`dedup_labels`, called from
    /// `AssetService::add` / `update_meta` and the dispatch reify path),
    /// so this row is written through `save` directly — that is the only
    /// way to reproduce what a database written before the guard, or by
    /// hand SQL, still holds. The repeat is not adjacent and the list is
    /// not in sorted order, so a dedup that only looked at neighbours or
    /// that sorted first would show up here rather than pass.
    #[tokio::test]
    async fn a_stored_repeat_is_read_back_once() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut legacy = item(persona, "/pics/legacy.png");
        legacy.labels = vec![
            Label::new("zebra").unwrap(),
            Label::new("alpha").unwrap(),
            Label::new("zebra").unwrap(),
        ];
        repo.save(&legacy).await.unwrap();
        // What the column holds is unchanged — reading is not a repair.
        let stored: String = {
            let id = *legacy.id.as_uuid();
            isle.call(move |conn| {
                conn.query_row("SELECT labels FROM asset WHERE id = ?1", params![id], |r| {
                    r.get::<_, String>(0)
                })
            })
            .await
            .unwrap()
        };
        assert_eq!(
            stored, r#"["zebra","alpha","zebra"]"#,
            "the fixture has to be a row that really carries the repeat"
        );

        let entity = repo.find(&legacy.id).await.unwrap().unwrap();
        assert_eq!(
            entity.labels.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
            vec!["zebra", "alpha"],
            "the entity read drops the second copy and keeps the first"
        );

        let query = AssetQuery {
            persona_id: Some(persona),
            limit: 10,
            ..Default::default()
        };
        let card = repo
            .page(&query)
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.id == legacy.id)
            .expect("the row is on the page");
        assert_eq!(
            card.labels.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
            vec!["zebra", "alpha"],
            "and so does the card projection the grid renders chips from"
        );
    }

    /// `rating` takes the larger of the two — but `0` is "nobody rated
    /// this", not a low score, so it never wins and never overwrites a
    /// real one.
    #[tokio::test]
    async fn a_fold_takes_the_higher_rating_and_never_a_zero() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let rated = |locator: &str, rating: u8| {
            let mut asset = item(persona, locator);
            asset.rating = Some(rating);
            asset
        };
        // (a) the keeper is rated, the folded row is not.
        let keeper = rated("/pics/rated-keeper.png", 3);
        let unrated = rated("/pics/unrated-copy.png", 0);
        // (b) the other way round.
        let blank = rated("/pics/blank-keeper.png", 0);
        let scored = rated("/pics/scored-copy.png", 4);
        // (c) both rated. Without this pair the rule is only ever asked
        // about `0` against a real score, and "larger wins" would be
        // indistinguishable from "smaller wins" — the two cases above
        // both come down to `None`, and pass either way.
        let high = rated("/pics/high-keeper.png", 4);
        let low = rated("/pics/low-copy.png", 2);
        for asset in [&keeper, &unrated, &blank, &scored, &high, &low] {
            repo.save(asset).await.unwrap();
        }
        assert_eq!(
            (
                repo.find(&keeper.id).await.unwrap().unwrap().rating,
                repo.find(&unrated.id).await.unwrap().unwrap().rating,
                repo.find(&blank.id).await.unwrap().unwrap().rating,
                repo.find(&scored.id).await.unwrap().unwrap().rating,
                repo.find(&high.id).await.unwrap().unwrap().rating,
                repo.find(&low.id).await.unwrap().unwrap().rating,
            ),
            (Some(3), Some(0), Some(0), Some(4), Some(4), Some(2)),
            "all three pairs start out disagreeing"
        );

        repo.fold_into(&unrated.id, &keeper.id).await.unwrap();
        repo.fold_into(&scored.id, &blank.id).await.unwrap();
        repo.fold_into(&low.id, &high.id).await.unwrap();

        assert_eq!(
            repo.find(&keeper.id).await.unwrap().unwrap().rating,
            Some(3),
            "an unrated duplicate must not pull a rated keeper down to zero"
        );
        assert_eq!(
            repo.find(&blank.id).await.unwrap().unwrap().rating,
            Some(4),
            "and a real rating fills an unrated keeper"
        );
        assert_eq!(
            repo.find(&high.id).await.unwrap().unwrap().rating,
            Some(4),
            "between two real ratings the larger one is the pair's"
        );
    }

    /// Two notes become one note, keeper first, separated by a blank
    /// line. The same text on both sides is not doubled — a fold that
    /// re-ran, or two rows that carried the same sentence, must not
    /// grow the paragraph.
    #[tokio::test]
    async fn a_fold_joins_the_register_notes_without_doubling_an_identical_one() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let noted = |locator: &str, note: &str| {
            let mut asset = item(persona, locator);
            asset.register_note = Some(RegisterNote::new(note).unwrap());
            asset
        };
        let keeper = noted("/pics/keeper.png", "shot on the roof");
        let headstone = noted("/pics/copy.png", "sent by aya");
        // The second pair says the same thing twice.
        let echo_keeper = noted("/pics/echo.png", "same sentence");
        let echo_copy = noted("/pics/echo-copy.png", "same sentence");
        for asset in [&keeper, &headstone, &echo_keeper, &echo_copy] {
            repo.save(asset).await.unwrap();
        }
        assert_ne!(
            repo.find(&keeper.id)
                .await
                .unwrap()
                .unwrap()
                .register_note
                .map(|n| n.as_str().to_string()),
            repo.find(&headstone.id)
                .await
                .unwrap()
                .unwrap()
                .register_note
                .map(|n| n.as_str().to_string()),
            "the first pair has two different notes to join"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        repo.fold_into(&echo_copy.id, &echo_keeper.id)
            .await
            .unwrap();

        assert_eq!(
            repo.find(&keeper.id)
                .await
                .unwrap()
                .unwrap()
                .register_note
                .unwrap()
                .as_str(),
            "shot on the roof\n\nsent by aya",
            "neither half of the note is dropped"
        );
        assert_eq!(
            repo.find(&echo_keeper.id)
                .await
                .unwrap()
                .unwrap()
                .register_note
                .unwrap()
                .as_str(),
            "same sentence",
            "identical text is one sentence, not two"
        );
    }

    /// Visibility only ever tightens. `vis_restricted` is an `OR`, so
    /// folding a restricted row into an open one closes the keeper —
    /// the opposite direction would publish something somebody had
    /// restricted, which no duplicate resolution is allowed to do.
    #[tokio::test]
    async fn a_fold_restricts_a_keeper_that_absorbed_a_restricted_row() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/open-keeper.png");
        let mut headstone = item(persona, "/pics/restricted-copy.png");
        headstone.visibility = Visibility::Restricted {
            sharing: vec!["alice".into()],
        };
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();
        assert_eq!(
            repo.find(&keeper.id).await.unwrap().unwrap().visibility,
            Visibility::Open,
            "the keeper is open before the fold"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        assert_eq!(
            repo.find(&keeper.id).await.unwrap().unwrap().visibility,
            Visibility::Restricted {
                sharing: vec!["alice".into()]
            },
            "the restricted side of the pair decides"
        );
    }

    /// The sharing list is a set like the others: the keeper keeps
    /// everyone it already shared with and gains the rest, once each.
    #[tokio::test]
    async fn a_fold_unions_the_sharing_list() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let shared_with = |locator: &str, subjects: &[&str]| {
            let mut asset = item(persona, locator);
            asset.visibility = Visibility::Restricted {
                sharing: subjects.iter().map(|s| (*s).to_string()).collect(),
            };
            asset
        };
        let keeper = shared_with("/pics/keeper.png", &["alice"]);
        let headstone = shared_with("/pics/copy.png", &["aya", "alice"]);
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();
        assert_eq!(
            repo.find(&keeper.id).await.unwrap().unwrap().visibility,
            Visibility::Restricted {
                sharing: vec!["alice".into()]
            },
            "the keeper starts out shared with one subject"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        assert_eq!(
            repo.find(&keeper.id).await.unwrap().unwrap().visibility,
            Visibility::Restricted {
                sharing: vec!["alice".into(), "aya".into()]
            },
            "the shared subject is not listed twice, and the new one is there"
        );
    }

    /// A column no rule combines: the keeper's value stands, and what
    /// it beat is **readable afterwards**. Losing the write-down is the
    /// failure this guards — the fold still looks right from the
    /// keeper, and the other answer is simply gone.
    #[tokio::test]
    async fn a_fold_keeps_the_keepers_own_values_and_writes_down_what_it_dropped() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper_box = item(persona, "/sessions/keeper-box");
        let headstone_box = item(persona, "/sessions/copy-box");
        let mut keeper = item(persona, "/pics/keeper.png");
        keeper.container_id = Some(keeper_box.id);
        keeper.title = Some("the one we keep".into());
        let mut headstone = item(persona, "/pics/copy.png");
        headstone.container_id = Some(headstone_box.id);
        headstone.title = Some("the copy".into());
        for asset in [&keeper_box, &headstone_box, &keeper, &headstone] {
            repo.save(asset).await.unwrap();
        }
        assert_ne!(
            repo.find(&keeper.id).await.unwrap().unwrap().container_id,
            repo.find(&headstone.id)
                .await
                .unwrap()
                .unwrap()
                .container_id,
            "the two rows are filed in different composites before the fold"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        let after = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(
            after.container_id,
            Some(keeper_box.id),
            "the keeper stays where it was filed"
        );
        assert_eq!(after.title.as_deref(), Some("the one we keep"));

        let note = absorbed_note(&repo, &keeper.id).await;
        assert_eq!(
            note.get("from").and_then(|v| v.as_str()),
            Some(headstone.id.to_string().as_str()),
            "the note names the row it came from: {note}"
        );
        let discarded = note.get("discarded").expect("the note lists what was left");
        assert_eq!(
            discarded.get("container_id").and_then(|v| v.as_str()),
            Some(headstone_box.id.to_string().as_str()),
            "the composite the folded row was filed in is unreadable: {note}"
        );
        assert_eq!(
            discarded.get("title").and_then(|v| v.as_str()),
            Some("the copy"),
            "the name nobody will see again is unreadable: {note}"
        );
        // The note records the column value it discarded, verbatim — the
        // walk that builds it reads `KEPT_COLUMNS` off the row and knows
        // nothing about what any one of them encodes. So the locator
        // appears in the form the column holds, which is now the tagged
        // one; rendering it for a reader here would put the locator
        // encoding inside the fold note's generic value walk.
        assert_eq!(
            discarded.get("source_locator").and_then(|v| v.as_str()),
            Some(crate::sqlite::stored_locator("/pics/copy.png").as_str()),
        );
    }

    /// Hand arrangement inside a Group the keeper was already filed in.
    /// The keeper's own position stands — but unlike a losing column,
    /// the row that held the other position is deleted, so without the
    /// note the number is gone rather than merely hidden.
    #[tokio::test]
    async fn a_fold_records_the_group_position_it_displaced() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();
        let shared = seed_bucket(&isle, persona, "everything").await;
        file_in_bucket(&isle, &keeper.id, shared, 1).await;
        file_in_bucket(&isle, &headstone.id, shared, 7).await;
        assert_eq!(
            (
                filing_of(&isle, &keeper.id).await,
                filing_of(&isle, &headstone.id).await
            ),
            (vec![(shared, 1)], vec![(shared, 7)]),
            "the two rows sit at different places in the same Group"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        assert_eq!(
            filing_of(&isle, &keeper.id).await,
            vec![(shared, 1)],
            "the keeper's own arrangement stands"
        );
        let note = absorbed_note(&repo, &keeper.id).await;
        let positions = note
            .get("positions_not_taken")
            .and_then(|v| v.as_array())
            .expect("the note lists the arrangement it displaced");
        assert_eq!(positions.len(), 1, "{note}");
        assert_eq!(
            positions[0].get("bucket").and_then(|v| v.as_str()),
            Some(shared.to_string().as_str())
        );
        assert_eq!(
            positions[0].get("position").and_then(|v| v.as_i64()),
            Some(7),
            "the position the deleted row held is not readable anywhere: {note}"
        );
    }

    /// Attribution is never transplanted, not even into an empty
    /// column.
    ///
    /// `NULL` there means **unrecorded**, not "the owner" and not "the
    /// same person" (`domain::attribution`, V47). Moving the folded
    /// row's assertion onto the keeper would state that somebody
    /// authored a row they never said anything about — and once
    /// written, that is indistinguishable from an assertion they did
    /// make. The claim is kept instead as what it is: something the
    /// *other* row's registration said, recorded against that row's id.
    #[tokio::test]
    async fn a_fold_never_moves_an_assertion_onto_the_keeper() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item_by(
            persona,
            "/pics/copy.png",
            &AttributionContext::asserted(
                Some(Author::Subject("alice".into())),
                Some(OperatorRef::new("claude-code").unwrap()),
            )
            .unwrap(),
        );
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();
        let before = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(
            (before.author(), before.operator_ai()),
            (None, None),
            "nobody has stated anything about the keeper"
        );
        let stated = repo.find(&headstone.id).await.unwrap().unwrap();
        assert_eq!(
            (stated.author(), stated.operator_ai()),
            (
                Some(&Author::Subject("alice".into())),
                Some(&OperatorRef::new("claude-code").unwrap())
            ),
            "and the row about to be folded carries a real assertion"
        );

        repo.fold_into(&headstone.id, &keeper.id).await.unwrap();

        let after = repo.find(&keeper.id).await.unwrap().unwrap();
        assert_eq!(
            (after.author(), after.operator_ai()),
            (None, None),
            "a fold minted an assertion nobody made"
        );

        // Not taken is not the same as not recorded: whose claim it
        // was, and which row they made it about, both stay readable.
        let note = absorbed_note(&repo, &keeper.id).await;
        assert_eq!(
            note.get("from").and_then(|v| v.as_str()),
            Some(headstone.id.to_string().as_str())
        );
        let discarded = note.get("discarded").expect("the note lists what was left");
        assert_eq!(
            discarded.get("author_kind").and_then(|v| v.as_str()),
            Some("subject"),
            "{note}"
        );
        assert_eq!(
            discarded.get("author_subject").and_then(|v| v.as_str()),
            Some("alice"),
            "{note}"
        );
        assert_eq!(
            discarded.get("operator_ai").and_then(|v| v.as_str()),
            Some("claude-code"),
            "{note}"
        );
        // And the row that made the assertion still holds it — the
        // headstone is never deleted, so the claim has a home.
        assert_eq!(
            repo.find(&headstone.id).await.unwrap().unwrap().author(),
            Some(&Author::Subject("alice".into())),
        );
    }

    /// Comments move onto the keeper and stay in one chronology: they
    /// keep their own timestamps, so the two sets interleave the way
    /// they were written rather than arriving as an appended block.
    #[tokio::test]
    async fn a_fold_moves_the_comments_into_one_chronology() {
        use asterism_core::domain::asset_comment::{AssetComment, CommentAuthor};
        use asterism_core::domain::repository::AssetCommentRepository;

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let comments = crate::sqlite::repo::SqliteAssetCommentRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        // Written alternately, so an implementation that appended one
        // asset's comments after the other's would be visible here.
        for (asset, body, when) in [
            (&keeper, "first, on the keeper", 1_000),
            (&headstone, "second, on the copy", 2_000),
            (&keeper, "third, on the keeper", 3_000),
            (&headstone, "fourth, on the copy", 4_000),
        ] {
            comments
                .save(&AssetComment::new(asset.id, CommentAuthor::User, body, at(when)).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(
            comments.list_by_asset(&keeper.id).await.unwrap().len(),
            2,
            "half the thread is on the row about to be folded"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                comments_moved: 2,
                values_discarded: 1,
                ..FoldReport::default()
            })
        );

        assert_eq!(
            comments
                .list_by_asset(&keeper.id)
                .await
                .unwrap()
                .into_iter()
                .map(|c| c.body)
                .collect::<Vec<_>>(),
            vec![
                "first, on the keeper".to_string(),
                "second, on the copy".to_string(),
                "third, on the keeper".to_string(),
                "fourth, on the copy".to_string(),
            ],
            "the two halves have to read as one conversation"
        );
        assert!(
            comments
                .list_by_asset(&headstone.id)
                .await
                .unwrap()
                .is_empty(),
            "the comments moved rather than being copied"
        );
    }

    /// A thread pinned to the folded card follows it to the keeper.
    /// Left behind it would hang off a card no listing shows, which is
    /// the same as being gone.
    #[tokio::test]
    async fn a_fold_reanchors_a_thread_pinned_to_the_folded_card() {
        use asterism_core::domain::repository::ThreadRepository;
        use asterism_core::domain::thread::{Thread, ThreadAnchor};

        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let threads = crate::sqlite::repo::SqliteThreadRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let headstone = item(persona, "/pics/copy.png");
        repo.save(&keeper).await.unwrap();
        repo.save(&headstone).await.unwrap();

        let pinned = Thread::new(
            ThreadAnchor::Card(headstone.id),
            "what to do with this one",
            at(1_000),
        )
        .unwrap();
        threads.save(&pinned).await.unwrap();
        assert_eq!(
            threads
                .list_by_anchor(&ThreadAnchor::Card(keeper.id), false)
                .await
                .unwrap()
                .len(),
            0,
            "the thread is on the other card before the fold"
        );

        let outcome = repo.fold_into(&headstone.id, &keeper.id).await.unwrap();
        assert_eq!(
            outcome,
            FoldOutcome::Folded(FoldReport {
                threads_reanchored: 1,
                values_discarded: 1,
                ..FoldReport::default()
            })
        );

        assert_eq!(
            threads
                .list_by_anchor(&ThreadAnchor::Card(keeper.id), false)
                .await
                .unwrap()
                .into_iter()
                .map(|t| t.id)
                .collect::<Vec<_>>(),
            vec![pinned.id],
            "the thread follows the card that survived"
        );
        assert!(
            threads
                .list_by_anchor(&ThreadAnchor::Card(headstone.id), false)
                .await
                .unwrap()
                .is_empty(),
            "and does not stay behind on the headstone as well"
        );
    }

    // ---- the manual merge ----------------------------------------
    //
    // Every fold here is the same `fold_one` the tests above already
    // hold to the column rules and the structure moves, so nothing
    // below re-states them. What is new is the three things the verb
    // adds: N rows instead of one, one transaction around all of them,
    // and `dry_run`. Each test is about one of those.

    /// Every row of the plan, as the database holds it — the shape a
    /// "nothing at all happened" assertion needs. `filing_of` and
    /// `tags_of` are separate because the rows they read are not on
    /// `asset` at all, and a merge that moved a Group membership without
    /// touching a column would slip past a row-only comparison.
    async fn merge_snapshot(
        isle: &AsyncIsle,
        ids: &[&AssetId],
    ) -> Vec<(Vec<Value>, Vec<(Uuid, i64)>, Vec<Uuid>)> {
        let mut out = Vec::new();
        for id in ids {
            out.push((
                raw_row(isle, id).await,
                filing_of(isle, id).await,
                tags_of(isle, id).await,
            ));
        }
        out
    }

    /// Three rows ruled one thing: all three become headstones pointing
    /// at the keeper, and everything that hung off them is on the keeper
    /// afterwards.
    ///
    /// The `register_note` assertion is the order decision made visible
    /// (see the port doc): the paragraphs come out in the order the plan
    /// lists the rows, which is why that order is the caller's to
    /// choose. Reverse the plan and this assertion fails — which is the
    /// point of writing it down rather than leaving the sequence to
    /// whatever the loop happened to do.
    #[tokio::test]
    async fn a_merge_of_three_folds_all_three_and_gathers_them_on_the_keeper() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut keeper = item(persona, "/pics/keeper.png");
        keeper.register_note = Some(RegisterNote::new("K").unwrap());
        let mut first = item(persona, "/pics/one.png");
        first.register_note = Some(RegisterNote::new("A").unwrap());
        let mut second = item(persona, "/pics/two.png");
        second.register_note = Some(RegisterNote::new("B").unwrap());
        let mut third = item(persona, "/pics/three.png");
        third.register_note = Some(RegisterNote::new("C").unwrap());
        let neighbour = item(persona, "/pics/neighbour.png");
        for asset in [&keeper, &first, &second, &third, &neighbour] {
            repo.save(asset).await.unwrap();
        }

        // Filing: one Group the keeper is already in (its own position
        // stands), and two it is not.
        let alpha = seed_bucket(&isle, persona, "alpha").await;
        let beta = seed_bucket(&isle, persona, "beta").await;
        let gamma = seed_bucket(&isle, persona, "gamma").await;
        file_in_bucket(&isle, &keeper.id, alpha, 0).await;
        file_in_bucket(&isle, &first.id, alpha, 5).await;
        file_in_bucket(&isle, &first.id, beta, 1).await;
        file_in_bucket(&isle, &second.id, gamma, 2).await;

        // Tags: one the keeper already carries, two it does not.
        let shared = seed_tag(&isle, "shared").await;
        let only_second = seed_tag(&isle, "only-second").await;
        let only_third = seed_tag(&isle, "only-third").await;
        link_tag(&isle, &keeper.id, shared).await;
        link_tag(&isle, &first.id, shared).await;
        link_tag(&isle, &second.id, only_second).await;
        link_tag(&isle, &third.id, only_third).await;

        seed_edge(&isle, &second.id, &neighbour.id, "derived_from", "out").await;
        seed_edge(&isle, &neighbour.id, &third.id, "reference", "in").await;

        let plan = MergePlan::declare(
            keeper.id,
            vec![first.id, second.id, third.id],
            &[keeper.id, first.id, second.id, third.id],
        )
        .unwrap();
        let outcome = repo.merge_into(&plan, false).await.unwrap();

        assert_eq!(
            outcome.folded,
            vec![first.id, second.id, third.id],
            "all three rows were folded, in the plan's order"
        );
        assert!(outcome.already_folded.is_empty());
        assert_eq!(outcome.refusals, vec![]);
        assert!(outcome.committed, "the merge was kept");
        assert_eq!(outcome.totals.buckets_moved, 2, "beta and gamma, not alpha");
        assert_eq!(outcome.totals.tags_moved, 2, "the shared tag is not moved");
        assert_eq!(outcome.totals.edges_repointed, 2);

        for folded in [&first, &second, &third] {
            assert_eq!(
                repo.find(&folded.id).await.unwrap().unwrap().folded_into,
                Some(keeper.id),
                "every row of the plan points at the keeper"
            );
        }

        let mut filing = filing_of(&isle, &keeper.id).await;
        filing.sort();
        let mut want = vec![(alpha, 0), (beta, 1), (gamma, 2)];
        want.sort();
        assert_eq!(filing, want, "the keeper kept its own place in alpha");

        let mut tags = tags_of(&isle, &keeper.id).await;
        tags.sort();
        let mut want_tags = vec![shared, only_second, only_third];
        want_tags.sort();
        assert_eq!(tags, want_tags);

        let mut edges = vec![
            (
                *keeper.id.as_uuid(),
                *neighbour.id.as_uuid(),
                "derived_from".to_string(),
            ),
            (
                *neighbour.id.as_uuid(),
                *keeper.id.as_uuid(),
                "reference".to_string(),
            ),
        ];
        edges.sort();
        assert_eq!(all_edges(&isle).await, edges);

        assert_eq!(
            repo.find(&keeper.id)
                .await
                .unwrap()
                .unwrap()
                .register_note
                .map(|note| note.as_str().to_string()),
            Some("K\n\nA\n\nB\n\nC".to_string()),
            "the notes read in the order the plan listed the rows"
        );
    }

    /// A plan that does not account for every row somebody looked at is
    /// refused, and the database is untouched.
    ///
    /// Written to run the merge when the plan is accepted, which reads
    /// oddly until you ask what it is guarding: with the `members` check
    /// gone the declaration below **is** accepted, the merge runs, and
    /// the rows change. Asserting only `is_err()` would leave that
    /// version of the code passing this test.
    #[tokio::test]
    async fn a_plan_that_leaves_a_row_unaccounted_for_is_refused_and_writes_nothing() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let folded = item(persona, "/pics/one.png");
        let bystander = item(persona, "/pics/two.png");
        for asset in [&keeper, &folded, &bystander] {
            repo.save(asset).await.unwrap();
        }
        let bucket = seed_bucket(&isle, persona, "watched").await;
        file_in_bucket(&isle, &folded.id, bucket, 3).await;

        let before = merge_snapshot(&isle, &[&keeper.id, &folded.id, &bystander.id]).await;

        // Three rows were looked at; the plan rules on two. The third is
        // either "leave it alone" or "I missed it", and the verb must
        // not have to guess which.
        match MergePlan::declare(
            keeper.id,
            vec![folded.id],
            &[keeper.id, folded.id, bystander.id],
        ) {
            Err(refused) => assert!(
                refused.to_string().contains(&bystander.id.to_string()),
                "the refusal should name the row nobody ruled on: {refused}"
            ),
            Ok(plan) => {
                repo.merge_into(&plan, false).await.unwrap();
            }
        }

        assert_eq!(
            merge_snapshot(&isle, &[&keeper.id, &folded.id, &bystander.id]).await,
            before,
            "a merge ran on a plan that does not add up"
        );
    }

    /// The dry run's counts are the run's counts, and the dry run leaves
    /// nothing behind.
    ///
    /// Both halves matter and neither implies the other: a preview
    /// computed by a second route can be exactly as harmless while
    /// reporting a number the real run never produces, which is the
    /// failure this is here to catch.
    #[tokio::test]
    async fn a_dry_run_reports_what_the_run_does_and_writes_nothing() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let mut keeper = item(persona, "/pics/keeper.png");
        keeper.labels = vec![Label::new("keeper-label").unwrap()];
        let mut first = item(persona, "/pics/one.png");
        first.labels = vec![Label::new("one-label").unwrap()];
        first.rating = Some(4);
        let mut second = item(persona, "/pics/two.png");
        second.register_note = Some(RegisterNote::new("what the second says").unwrap());
        let neighbour = item(persona, "/pics/neighbour.png");
        for asset in [&keeper, &first, &second, &neighbour] {
            repo.save(asset).await.unwrap();
        }
        let bucket = seed_bucket(&isle, persona, "watched").await;
        file_in_bucket(&isle, &first.id, bucket, 7).await;
        let tag = seed_tag(&isle, "kept").await;
        link_tag(&isle, &second.id, tag).await;
        seed_edge(&isle, &first.id, &neighbour.id, "derived_from", "out").await;

        let plan = MergePlan::declare(
            keeper.id,
            vec![first.id, second.id],
            &[keeper.id, first.id, second.id],
        )
        .unwrap();

        let before = merge_snapshot(&isle, &[&keeper.id, &first.id, &second.id]).await;
        let predicted = repo.merge_into(&plan, true).await.unwrap();
        assert!(!predicted.committed, "a dry run keeps nothing");
        assert_eq!(
            predicted.folded,
            vec![first.id, second.id],
            "the preview reached both rows, in the plan's order"
        );
        assert!(
            predicted.totals.tags_moved > 0 && predicted.totals.buckets_moved > 0,
            "a preview of nothing would match a run of nothing: {predicted:?}"
        );
        assert_eq!(
            merge_snapshot(&isle, &[&keeper.id, &first.id, &second.id]).await,
            before,
            "the dry run wrote something"
        );

        let done = repo.merge_into(&plan, false).await.unwrap();
        assert_eq!(
            done,
            MergeOutcome {
                committed: true,
                ..predicted
            },
            "the run and its preview disagree"
        );
    }

    /// One refused row abandons the whole merge — including the rows
    /// that were folded before the refusal was reached.
    ///
    /// The refused row is in the **middle** deliberately: with a
    /// transaction per row the first row's fold is already committed by
    /// the time the second is refused, and the person is left with a set
    /// smaller than the one they ruled over.
    #[tokio::test]
    async fn a_refusal_anywhere_in_a_merge_leaves_every_row_untouched() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let first = item(persona, "/pics/one.png");
        let elsewhere = item(persona, "/pics/two.png");
        let third = item(persona, "/pics/three.png");
        let other_keeper = item(persona, "/pics/other-keeper.png");
        for asset in [&keeper, &first, &elsewhere, &third, &other_keeper] {
            repo.save(asset).await.unwrap();
        }
        let bucket = seed_bucket(&isle, persona, "watched").await;
        file_in_bucket(&isle, &first.id, bucket, 2).await;

        // Somebody else already ruled on this row, and ruled differently.
        fold_into(&isle, &elsewhere.id, &other_keeper.id).await;

        let before = merge_snapshot(&isle, &[&keeper.id, &first.id, &third.id]).await;

        let plan = MergePlan::declare(
            keeper.id,
            vec![first.id, elsewhere.id, third.id],
            &[keeper.id, first.id, elsewhere.id, third.id],
        )
        .unwrap();
        let outcome = repo.merge_into(&plan, false).await.unwrap();

        assert_eq!(
            outcome.refusals,
            vec![(elsewhere.id, FoldRefusal::AlreadyFolded)],
            "the refusal names the row it is about"
        );
        assert!(!outcome.committed, "a refused merge is not kept");
        assert_eq!(
            merge_snapshot(&isle, &[&keeper.id, &first.id, &third.id]).await,
            before,
            "a row folded before the refusal stayed folded"
        );
        for untouched in [&first, &third] {
            assert_eq!(
                repo.find(&untouched.id).await.unwrap().unwrap().folded_into,
                None,
                "a refused merge left a headstone behind"
            );
        }
        assert_eq!(
            repo.find(&elsewhere.id).await.unwrap().unwrap().folded_into,
            Some(other_keeper.id),
            "and did not move the row somebody else had already ruled on"
        );
    }

    /// A row already folded into **this** keeper is the plan already
    /// being true for that row, so it is counted rather than refused and
    /// the rest of the merge goes through.
    ///
    /// The same replay this repository allows a single fold: a second
    /// click, a retried request, or a set assembled from a panel a fold
    /// job has since overtaken. Refusing it would make the person re-rule
    /// a set that is already partly settled.
    #[tokio::test]
    async fn a_row_already_folded_into_this_keeper_is_counted_not_refused() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let keeper = item(persona, "/pics/keeper.png");
        let settled = item(persona, "/pics/one.png");
        let pending = item(persona, "/pics/two.png");
        for asset in [&keeper, &settled, &pending] {
            repo.save(asset).await.unwrap();
        }
        repo.fold_into(&settled.id, &keeper.id).await.unwrap();

        let plan = MergePlan::declare(
            keeper.id,
            vec![settled.id, pending.id],
            &[keeper.id, settled.id, pending.id],
        )
        .unwrap();
        let outcome = repo.merge_into(&plan, false).await.unwrap();

        assert_eq!(outcome.refusals, vec![], "{outcome:?}");
        assert_eq!(outcome.already_folded, vec![settled.id]);
        assert_eq!(
            outcome.folded,
            vec![pending.id],
            "only the row that was not settled yet"
        );
        assert!(outcome.committed, "the rest of the ruling was carried out");
        assert_eq!(
            repo.find(&pending.id).await.unwrap().unwrap().folded_into,
            Some(keeper.id)
        );
    }

    // --- tag_match: how multi-select tag chips combine -----------------

    /// Three assets over two tags, arranged so the two combinators
    /// **disagree**: one asset carries both tags, one carries only the
    /// first, one only the second. Asking for `[a, b]` under `Any`
    /// returns all three and under `All` returns one, so neither
    /// assertion can pass by falling through to the other's answer.
    async fn seed_tag_pair_fixture(
        isle: &AsyncIsle,
        repo: &SqliteAssetRepository,
        persona: PersonaId,
    ) -> (Uuid, Uuid, AssetId, AssetId, AssetId) {
        let travel = seed_tag(isle, "travel").await;
        let summer = seed_tag(isle, "summer").await;

        let both = item(persona, "/pics/both.png");
        let travel_only = item(persona, "/pics/travel.png");
        let summer_only = item(persona, "/pics/summer.png");
        repo.save(&both).await.unwrap();
        repo.save(&travel_only).await.unwrap();
        repo.save(&summer_only).await.unwrap();

        link_tag(isle, &both.id, travel).await;
        link_tag(isle, &both.id, summer).await;
        link_tag(isle, &travel_only.id, travel).await;
        link_tag(isle, &summer_only.id, summer).await;

        (travel, summer, both.id, travel_only.id, summer_only.id)
    }

    async fn ids_for_tags(
        repo: &SqliteAssetRepository,
        persona: PersonaId,
        tags: &[Uuid],
        tag_match: TagMatch,
    ) -> Vec<AssetId> {
        let query = AssetQuery {
            persona_id: Some(persona),
            tag_ids: tags.iter().copied().map(TagId::from_uuid).collect(),
            tag_match,
            limit: 100,
            ..Default::default()
        };
        let mut ids: Vec<_> = repo
            .page(&query)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|c| c.id)
            .collect();
        ids.sort();
        ids
    }

    /// `All` is an intersection: only the asset carrying **every**
    /// requested tag survives. The two single-tag assets are what make
    /// this a real assertion — without them the fixture would answer the
    /// same under either combinator.
    #[tokio::test]
    async fn tag_match_all_requires_every_tag() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (travel, summer, both, travel_only, _summer_only) =
            seed_tag_pair_fixture(&isle, &repo, persona).await;

        assert_eq!(
            ids_for_tags(&repo, persona, &[travel, summer], TagMatch::All).await,
            vec![both],
            "only the asset carrying both tags passes an intersection"
        );
        // One tag: the combinator has nothing to intersect, so `All`
        // must not narrow further than the single predicate.
        let mut single = vec![both, travel_only];
        single.sort();
        assert_eq!(
            ids_for_tags(&repo, persona, &[travel], TagMatch::All).await,
            single,
            "a single-tag filter means the same under either combinator"
        );

        driver.shutdown().await.unwrap();
    }

    /// The default stays the union it has always been. Pinned against
    /// the same fixture the intersection narrows, so a regression that
    /// made `All` the default would fail here rather than quietly
    /// shrinking every existing multi-tag filter.
    #[tokio::test]
    async fn tag_match_default_stays_any_of() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (travel, summer, both, travel_only, summer_only) =
            seed_tag_pair_fixture(&isle, &repo, persona).await;

        let mut all_three = vec![both, travel_only, summer_only];
        all_three.sort();

        // The default the `..Default::default()` above supplies.
        assert_eq!(AssetQuery::default().tag_match, TagMatch::Any);
        assert_eq!(
            ids_for_tags(&repo, persona, &[travel, summer], TagMatch::Any).await,
            all_three,
            "the union returns every asset carrying either tag"
        );

        driver.shutdown().await.unwrap();
    }

    // --- text_match: the Query-side text predicate (V58) --------------

    /// Saves an asset with a body, through the same two adapters the
    /// app writes through (body cache + text index).
    async fn seed_asset_with_body(
        isle: &AsyncIsle,
        repo: &SqliteAssetRepository,
        persona: PersonaId,
        locator: &str,
        body: &str,
    ) -> asterism_core::domain::value::AssetId {
        use asterism_core::domain::repository::{AssetBodyRepository, AssetIndexer, IndexDoc};
        let asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            None,
            chrono::Utc::now(),
            &nobody(),
        );
        repo.save(&asset).await.unwrap();
        crate::sqlite::repo::SqliteAssetBodyRepository::new(isle.clone())
            .upsert(&asset.id, body)
            .await
            .unwrap();
        crate::sqlite::repo::SqliteAssetTextIndex::new(isle.clone())
            .upsert(&IndexDoc {
                asset_id: asset.id,
                persona_id: persona,
                text: Some(body.to_string()),
            })
            .await
            .unwrap();
        asset.id
    }

    async fn ids_matching(
        repo: &SqliteAssetRepository,
        persona: PersonaId,
        text: &str,
    ) -> Vec<asterism_core::domain::value::AssetId> {
        let query = AssetQuery {
            persona_id: Some(persona),
            text_match: Some(text.to_string()),
            limit: 100,
            ..Default::default()
        };
        let mut ids: Vec<_> = repo
            .page(&query)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|c| c.id)
            .collect();
        ids.sort();
        ids
    }

    /// The predicate means "the body contains this string" at **every**
    /// query length. The two branches behind it (trigram index at 3+
    /// characters, `LIKE` below that) are an implementation detail, so
    /// this pins that they answer the same question — including the case
    /// the word-segmented alternative would have got wrong, where the
    /// term sits inside a longer run of characters with no boundary
    /// around it.
    #[tokio::test]
    async fn text_match_is_substring_at_every_query_length() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let kuroneko = seed_asset_with_body(&isle, &repo, persona, "a.md", "黒猫がいた").await;
        let inu = seed_asset_with_body(&isle, &repo, persona, "b.md", "犬の写真").await;
        let test_case =
            seed_asset_with_body(&isle, &repo, persona, "c.md", "regression テストケース").await;

        // 1 character, below the trigram floor: still a substring, and
        // still finds the term buried inside 黒猫.
        assert_eq!(
            ids_matching(&repo, persona, "猫").await,
            vec![kuroneko],
            "a 1-character term must find the asset whose body contains it"
        );
        // 2 characters, still below the floor.
        assert_eq!(ids_matching(&repo, persona, "犬の").await, vec![inu]);
        // 3 characters and up, served by the trigram index — same
        // meaning, and `スト` living inside `テストケース` counts.
        assert_eq!(
            ids_matching(&repo, persona, "テスト").await,
            vec![test_case]
        );
        assert_eq!(
            ids_matching(&repo, persona, "ストケ").await,
            vec![test_case],
            "a term spanning a word boundary is still a substring"
        );
        // A term nothing carries.
        assert!(ids_matching(&repo, persona, "unrelated").await.is_empty());

        driver.shutdown().await.unwrap();
    }

    /// FTS5 has a query syntax and `LIKE` has wildcards. A person
    /// searching for `50%` or `a-b` is searching for that text, so both
    /// branches must treat the term as a literal rather than as an
    /// expression — the failure mode otherwise is an error or, worse, a
    /// different answer that looks plausible.
    #[tokio::test]
    async fn text_match_treats_the_term_as_literal_text() {
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let discount = seed_asset_with_body(&isle, &repo, persona, "a.md", "up to 50% off").await;
        let hyphen = seed_asset_with_body(&isle, &repo, persona, "b.md", "issue a-b filed").await;
        let quoted = seed_asset_with_body(&isle, &repo, persona, "c.md", r#"he said "no""#).await;
        let underscore = seed_asset_with_body(&isle, &repo, persona, "d.md", "snake_case").await;

        // `%` is a LIKE wildcard on the short branch.
        assert_eq!(ids_matching(&repo, persona, "0%").await, vec![discount]);
        // `_` matches any single character in LIKE; as a literal it must
        // not match `snakeXcase`-shaped text, and here it must find the
        // asset that really has one.
        assert_eq!(ids_matching(&repo, persona, "e_c").await, vec![underscore]);
        // `-` and `"` are FTS5 query syntax on the trigram branch.
        assert_eq!(ids_matching(&repo, persona, "a-b").await, vec![hyphen]);
        assert_eq!(ids_matching(&repo, persona, r#""no""#).await, vec![quoted]);
        // `OR` is an FTS5 operator; as text it matches nothing here.
        assert!(
            ids_matching(&repo, persona, "off OR filed")
                .await
                .is_empty()
        );

        driver.shutdown().await.unwrap();
    }

    /// Removing an asset from the index must stop it matching. The FTS
    /// row is addressed through `asset_fts_key`, so this is also what
    /// pins that the mapping is maintained rather than leaked.
    #[tokio::test]
    async fn removing_from_the_text_index_stops_the_match() {
        use asterism_core::domain::repository::AssetIndexer;
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let id = seed_asset_with_body(&isle, &repo, persona, "a.md", "sunset over water").await;
        assert_eq!(ids_matching(&repo, persona, "sunset").await, vec![id]);

        crate::sqlite::repo::SqliteAssetTextIndex::new(isle.clone())
            .remove(&id)
            .await
            .unwrap();
        assert!(
            ids_matching(&repo, persona, "sunset").await.is_empty(),
            "an unindexed asset must stop matching the predicate"
        );

        driver.shutdown().await.unwrap();
    }

    /// Re-indexing the same asset must replace its document, not add a
    /// second one — otherwise a re-import would make the asset match its
    /// old body forever.
    #[tokio::test]
    async fn reindexing_replaces_the_document() {
        use asterism_core::domain::repository::AssetIndexer;
        let (isle, driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;

        let id = seed_asset_with_body(&isle, &repo, persona, "a.md", "first body").await;
        crate::sqlite::repo::SqliteAssetTextIndex::new(isle.clone())
            .upsert(&asterism_core::domain::repository::IndexDoc {
                asset_id: id,
                persona_id: persona,
                text: Some("second body".into()),
            })
            .await
            .unwrap();

        assert!(
            ids_matching(&repo, persona, "first").await.is_empty(),
            "the replaced body must stop matching"
        );
        assert_eq!(ids_matching(&repo, persona, "second").await, vec![id]);

        driver.shutdown().await.unwrap();
    }
}
