//! Sort specification DTO — the wire form of the grid's sort axis.
//!
//! Historically `sort_json` was an opaque `String` blob owned entirely
//! by the UI (`SavedQuery.sort_json`, see
//! `asterism_core::domain::saved_query`). The backend had no type and no
//! evaluator: SQL order was hard-wired to `occurred_at DESC` (or a single
//! group's `position`). The Query Group model materialises a query's
//! members
//! into `asset_bucket` rows with a frozen `position`, which means the
//! backend must now *evaluate* the sort itself. This module gives that
//! sort a real type; the evaluator lives in
//! `asterism_core::domain::sort_eval` (behaviour stays in the core, shape
//! stays in the leaf contract crate).
//!
//! # UI correspondence (drift watch)
//!
//! These enums mirror the TypeScript unions `SortTarget` / `SortOrder`
//! in `crates/asterism-ui/src/lib/stores/filter.svelte.ts` and the
//! `{ target, order, reverse }` object `saveCurrentQuery` builds in
//! `crates/asterism-ui/src/App.svelte` (symbol references on purpose —
//! line numbers here drifted the moment either file moved). The serde
//! `rename_all = "snake_case"` reproduces the exact string tokens the UI
//! writes into `sort_json`, so a round-trip through this type is
//! byte-identical to what `saveCurrentQuery` persists. Keep the variant
//! set in lock-step with the UI union; a new axis on either side without
//! the other is the classic drift bug this module is documented against.

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// Which asset dimension the grid sorts on.
///
/// Mirrors `SortTarget` in
/// `crates/asterism-ui/src/lib/stores/filter.svelte.ts`. The
/// `snake_case` wire tokens are `occurred_at` / `created_at` /
/// `updated_at` / `persona` / `modality` / `tag` / `group` / `cover` /
/// `rating` / `duration` / `file_size` / `pixels`.
///
/// [`UpdatedAt`](Self::UpdatedAt) and [`Rating`](Self::Rating) are the
/// variants the UI union does not carry: each was wired server-side
/// first (the grid's own comparator has no branch for either), so such a
/// spec is answerable over the wire and inert client-side. They are
/// named here rather than left implicit so the drift is a documented gap
/// instead of a surprise.
///
/// [`Duration`](Self::Duration) and [`FileSize`](Self::FileSize) were in
/// that list until the UI union took them: `card-cmp.ts` carries both
/// branches now and `card-cmp.test.ts` pins them against the evaluator
/// below. The grid's Sort dropdown withheld them for one wave after
/// that, for a reason one layer down from this enum — the index rows it
/// sorts carried neither column, a row-shape question rather than an
/// axis one. Both are on the row now
/// (`AssetIndexEntryDto::duration_ms` / `file_size_bytes`) and the
/// picker offers them, so the two unions and the two comparators agree
/// end to end.
///
/// A `msg_count` token used to sit on both unions with a comparator on
/// neither: the UI declined it (`buildCardCmp` returned `null`) and the
/// evaluator answered `Equal`, so naming the axis produced the order the
/// caller would have got by naming nothing. The grid retired it from the
/// picker when the Session tiles left the grid (asset-model v4 P3), and
/// the token went with it — a spec naming it now fails to parse, which
/// is the same answer a misspelled axis gets and the only honest one for
/// an axis nothing implements. A `query_json` v1 blob frozen with it
/// fails its own refresh loudly (`RefreshAllOutcome::failures`, one
/// group, keep-going) rather than materialising a position under a
/// different order; re-saving the group from the picker mints a live
/// axis. One legacy path stays quiet: the V19 migration transcribing
/// pre-Query-Group `saved_query.sort_json` reads with
/// `unwrap_or_default()`, so a retired token there falls to the default
/// axis — which orders identically to what `MsgCount`'s `Equal`
/// comparator produced, so the silence is harmless there and only there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortTarget {
    /// Occurrence timestamp (the asset's own event time).
    OccurredAt,
    /// Ingest timestamp (when Asterism recorded the row).
    CreatedAt,
    /// Last-modification timestamp (`asset.updated_at`) — most recently
    /// changed first in its natural direction, the same reading as the
    /// other two time axes.
    ///
    /// This is the paging axis for differential sync: a consumer asking
    /// `updated_from_ms` for "what changed since I last looked" needs the
    /// answer in change order, or its cursor advances past rows it has
    /// not seen. Ordering that page by occurrence or ingest time
    /// interleaves it with material whose stamps have nothing to do with
    /// the question.
    ///
    /// The stamp moves on metadata edits, provenance writes, the cover /
    /// flags / keyword pipeline writes and Session renames — but not on
    /// trash, tagging, group filing or comments. A page ordered on this
    /// axis is therefore in "order of the changes this column records",
    /// which is narrower than "order of the changes"; see
    /// `ListAssetsQuery::updated_from_ms` for the exhaustive list.
    ///
    /// [`SortOrder`] is inert here, as on the other timestamp axes.
    UpdatedAt,
    /// Persona bucket.
    Persona,
    /// Primary modality slug.
    Modality,
    /// First user-visible label (tag axis).
    Tag,
    /// Primary group (first `group_ids` entry, resolved to a name).
    Group,
    /// Card cover text.
    Cover,
    /// Star rating. Natural direction is best-first (`5 … 1`), matching
    /// the `rating DESC` index the schema carries for it
    /// (`idx_asset_persona_rating`); `reverse` reads it worst-first.
    /// Unrated assets sort to the tail in **both** directions — see the
    /// evaluator for why an unrated card leading a "worst first" page
    /// would be a wrong answer rather than a stylistic one.
    ///
    /// [`SortOrder`] is inert here, like it is on the two timestamp axes:
    /// the target already is the ordering.
    Rating,
    /// Playback length (`asset.duration_ms`). Natural direction is
    /// longest first; `reverse` reads it shortest first.
    ///
    /// Assets carrying no length sort to the tail in **both** directions,
    /// as unrated ones do on [`Rating`](Self::Rating) and for the same
    /// kind of reason: leading a "shortest first" page with a still image
    /// answers "what is the shortest clip here" with something that is
    /// not a clip, which is a wrong answer rather than an untidy one. A
    /// stand-in `0` would instead park it at one end and flip sides under
    /// `reverse`.
    ///
    /// Not a video-only axis — audio carries the same column.
    ///
    /// [`SortOrder`] is inert here, as on [`Rating`](Self::Rating): the
    /// target already is the ordering.
    Duration,
    /// Stored size (`asset.file_size_bytes`). Natural direction is
    /// largest first; `reverse` reads it smallest first. Rows with no
    /// recorded size sort to the tail in **both** directions, same rule
    /// and same reason as [`Duration`](Self::Duration), and [`SortOrder`]
    /// is inert here too.
    FileSize,
    /// Total pixel count (`asset.width_px * asset.height_px`). Natural
    /// direction is largest first; `reverse` reads it smallest first.
    /// Unmeasured rows tail in **both** directions and [`SortOrder`] is
    /// inert, same rules as the two metric axes above.
    ///
    /// The **product**, not either side, and not an aspect ratio: the two
    /// columns hold coded dimensions taken before orientation is applied,
    /// so a portrait photo and a landscape one of the same sensor are the
    /// same pair in a different order. Their product is what survives
    /// that, which is why this axis exists and a "widest first" one does
    /// not. See [`ListAssetsQuery::pixels_min`] for the full reading.
    ///
    /// [`ListAssetsQuery::pixels_min`]: crate::query::ListAssetsQuery::pixels_min
    Pixels,
}

/// Direction of ordering *inside* the chosen [`SortTarget`].
///
/// Mirrors `SortOrder` in
/// `crates/asterism-ui/src/lib/stores/filter.svelte.ts:34`. Not every
/// order is meaningful for every target (the UI gates the combinations
/// via `ORDER_OPTIONS`, `App.svelte`); the evaluator treats
/// unsupported combinations exactly as the UI comparator does — e.g.
/// `Tag` + `Ordered` is not offered, and a spec naming it reads as
/// [`Alpha`](Self::Alpha).
///
/// Four targets carry no choice at all: on [`SortTarget::OccurredAt`] /
/// [`SortTarget::CreatedAt`] / [`SortTarget::UpdatedAt`] the target *is*
/// the ordering, and [`SortTarget::Cover`] has a single reading. The
/// field is not nullable, so those specs still name an order — a
/// placeholder that every comparator branch ignores. The UI hides the
/// dropdown when the option list has one entry rather than displaying
/// it: `Sort: Occurred` beside `Order: updated` reads as a promise to
/// order by modification time, which is a different axis
/// ([`SortTarget::UpdatedAt`]) rather than a direction within this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Alphabetical on the axis key.
    Alpha,
    /// Domain-native display order: persona sidebar order, modality
    /// canonical rank, and — on [`SortTarget::Group`] — the hand
    /// arrangement, i.e. `asset_bucket.position` within each group
    /// (`AssetCardDto::primary_group_position`). Falls back to
    /// alphabetical where no explicit order exists.
    Ordered,
    /// Most-recently-touched first — ranks buckets by the maximum
    /// `occurred_at_ms` across the slice being sorted.
    ///
    /// "Touched" is occurrence, not modification: this order ranks
    /// buckets by `occurred_at_ms` and never reads `updated_at`.
    /// Modification time is a *target* ([`SortTarget::UpdatedAt`]), not a
    /// direction within one, so the two never meet — `Sort: Persona` +
    /// `Order: updated` still means "the persona whose newest *event* is
    /// newest", not "the persona edited last".
    ///
    /// The order only means anything on the bucketing targets (persona /
    /// modality / tag / group), where it decides which bucket leads.
    /// Elsewhere it is the placeholder described on this enum's docs.
    Updated,
}

/// Serialised sort axis (`sort_json` payload, and the `sort` field of
/// the Query Group `query_json` v1 blob).
///
/// Corresponds to the `{ target, order, reverse, collation }` object
/// built at `crates/asterism-ui/src/App.svelte:2380`.
///
/// Not `Copy`: [`collation`](Self::collation) carries an owned tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct SortSpec {
    /// The dimension to sort on.
    pub target: SortTarget,
    /// The direction inside that dimension.
    pub order: SortOrder,
    /// Flips whichever order the `(target, order)` pair produced. Applied
    /// to the primary key only — the `occurred_at DESC` tie-break is
    /// never reversed (matches `App.svelte`'s `dir` / `tie` split).
    pub reverse: bool,
    /// BCP-47 tag selecting the collation the alphabetical axes compare
    /// under — the language knob for sorting, carried as a **search
    /// parameter** rather than global state.
    ///
    /// Asterism has no settings store (there is no `app_setting` table;
    /// the only per-persona precedent is `persona_theme`), and collation
    /// is not a global property anyway: it belongs to the query whose
    /// order gets frozen into `asset_bucket.position`. Putting the tag
    /// here means each Query Group persists the collation its `position`
    /// was materialised under, and any caller can inject a different one
    /// without a service-level constant.
    ///
    /// `None` = CLDR **root** collation: language-independent, and the
    /// only value that is reproducible across the two comparators (see
    /// `asterism_core::domain::sort_eval` for the measured parity).
    /// Anything else is a tailoring and only agrees with the UI as far
    /// as the two ICU builds agree.
    ///
    /// `#[serde(default)]` keeps every `query_json` written before this
    /// field existed parseable — they read back as root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
}

impl Default for SortSpec {
    /// The grid's default axis: newest occurrence first, unreversed,
    /// root collation. Mirrors the UI defaults (`sortTarget =
    /// "occurred_at"`, `sortOrder = "updated"`, `sortReverse = false`,
    /// `sortCollation = null`, `filter.svelte.ts`).
    fn default() -> Self {
        Self {
            target: SortTarget::OccurredAt,
            order: SortOrder::Updated,
            reverse: false,
            collation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_tokens_match_ui() {
        // The exact strings the UI persists into sort_json.
        let cases = [
            (SortTarget::OccurredAt, "\"occurred_at\""),
            (SortTarget::CreatedAt, "\"created_at\""),
            (SortTarget::UpdatedAt, "\"updated_at\""),
            (SortTarget::Persona, "\"persona\""),
            (SortTarget::Modality, "\"modality\""),
            (SortTarget::Tag, "\"tag\""),
            (SortTarget::Group, "\"group\""),
            (SortTarget::Cover, "\"cover\""),
            (SortTarget::Rating, "\"rating\""),
            (SortTarget::Duration, "\"duration\""),
            (SortTarget::FileSize, "\"file_size\""),
            (SortTarget::Pixels, "\"pixels\""),
        ];
        for (variant, token) in cases {
            assert_eq!(serde_json::to_string(&variant).unwrap(), token);
            assert_eq!(serde_json::from_str::<SortTarget>(token).unwrap(), variant);
        }
    }

    /// The retired axis is refused, not answered.
    ///
    /// `msg_count` was on both unions with a comparator on neither, so a
    /// spec naming it was accepted and then ordered by nothing. Removing
    /// the variant routes it into the same rejection a misspelled axis
    /// gets — asserted here because "the variant is gone" and "the wire
    /// says so" are different facts, and the second is the one callers
    /// (and a stale `query_json`) actually meet.
    #[test]
    fn retired_msg_count_token_is_rejected() {
        let err = serde_json::from_str::<SortTarget>("\"msg_count\"")
            .expect_err("a retired axis must not parse");
        // Same shape serde reports for a typo, which is the point: the
        // caller is told the axis is unknown either way.
        assert!(
            err.to_string().contains("msg_count"),
            "the error should name the token it refused, got {err}"
        );
        assert!(
            serde_json::from_str::<SortSpec>(
                r#"{"target":"msg_count","order":"updated","reverse":false}"#
            )
            .is_err(),
            "a whole spec carrying the retired axis must fail too"
        );
    }

    #[test]
    fn order_tokens_match_ui() {
        let cases = [
            (SortOrder::Alpha, "\"alpha\""),
            (SortOrder::Ordered, "\"ordered\""),
            (SortOrder::Updated, "\"updated\""),
        ];
        for (variant, token) in cases {
            assert_eq!(serde_json::to_string(&variant).unwrap(), token);
            assert_eq!(serde_json::from_str::<SortOrder>(token).unwrap(), variant);
        }
    }

    #[test]
    fn sortspec_round_trip_matches_ui_shape() {
        // The literal object App.svelte:2380 writes. Root collation is
        // absent from the wire form on both sides (the UI spreads the
        // key in only when set, this side skips serializing `None`), so
        // the default spec still round-trips byte-for-byte.
        let json = r#"{"target":"occurred_at","order":"updated","reverse":false}"#;
        let spec: SortSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec, SortSpec::default());
        assert_eq!(spec.collation, None);
        assert_eq!(serde_json::to_string(&spec).unwrap(), json);
    }

    #[test]
    fn collation_round_trips_when_set() {
        let json = r#"{"target":"cover","order":"alpha","reverse":false,"collation":"sv"}"#;
        let spec: SortSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.collation.as_deref(), Some("sv"));
        assert_eq!(serde_json::to_string(&spec).unwrap(), json);
    }

    #[test]
    fn collation_is_optional_for_pre_existing_query_json() {
        // Every `query_json` written before the knob existed omits the
        // key; those groups must keep parsing and land on root rather
        // than failing the rule parse (which would strand the group).
        let json = r#"{"target":"tag","order":"alpha","reverse":true}"#;
        let spec: SortSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.collation, None);
        assert!(spec.reverse);
    }
}
