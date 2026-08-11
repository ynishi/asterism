//! Query DTOs — inputs for read-side operations.
//!
//! `viewer_subject` is the raw input for visibility enforcement:
//! `None` means the owner view (everything visible), and `Some(subject)`
//! means a restricted subject view (persona view, and so on).

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

use crate::sort::SortSpec;

/// Custom deserialiser for `tag_ids` that accepts both a JSON array
/// (native form, used by Tauri IPC and HTTP POST bodies) and a
/// comma-separated string (HTTP GET query string, since
/// `serde_urlencoded` cannot map repeated keys to `Vec`). Empty /
/// missing input produces an empty `Vec` so the caller can treat it
/// as "no filter" uniformly.
fn deserialize_tag_ids<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Vec(Vec<String>),
        String(String),
    }
    let raw = Option::<StringOrVec>::deserialize(deserializer)?;
    Ok(match raw {
        None => Vec::new(),
        Some(StringOrVec::Vec(v)) => v,
        Some(StringOrVec::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
    })
}

/// Accepts a [`SortSpec`] as either a nested object (JSON transports) or
/// a JSON-encoded string (HTTP `GET` query strings, where
/// `serde_urlencoded` sees every value as a scalar). A malformed string is
/// an error rather than a silent `None`: a caller asking for an axis it
/// spelled wrong must not be answered with the arrival order.
fn deserialize_sort<'de, D>(deserializer: D) -> Result<Option<SortSpec>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SpecOrString {
        Spec(SortSpec),
        String(String),
    }
    Ok(match Option::<SpecOrString>::deserialize(deserializer)? {
        None => None,
        Some(SpecOrString::Spec(spec)) => Some(spec),
        Some(SpecOrString::String(raw)) if raw.trim().is_empty() => None,
        Some(SpecOrString::String(raw)) => {
            Some(serde_json::from_str(&raw).map_err(serde::de::Error::custom)?)
        }
    })
}

/// How the entries of [`ListAssetsQuery::tag_ids`] combine.
///
/// Two readings of the same chip row, and the caller says which:
/// "anything carrying one of these" widens as chips are added, "only
/// what carries all of these" narrows. Both are wanted often enough that
/// picking one and calling it the meaning of a multi-select would be
/// wrong half the time.
///
/// [`Any`](Self::Any) is the default, so a caller that never heard of
/// this field keeps the OR semantic it already had.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum TagMatch {
    /// Union — an asset passes when it carries **at least one** of the
    /// requested tags.
    #[default]
    Any,
    /// Intersection — an asset passes only when it carries **every** one
    /// of the requested tags.
    All,
}

/// Filter and pagination parameters for the asset grid.
///
/// `#[serde(default)]` lets an HTTP `GET` query string omit fields — the
/// server falls back to [`ListAssetsQuery::default`] (`limit = 200`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct ListAssetsQuery {
    /// Requesting subject (`None` = owner view = everything visible).
    pub viewer_subject: Option<String>,
    /// Optional persona filter.
    pub persona_id: Option<String>,
    /// Optional modality filter.
    pub modality: Option<String>,
    /// Lower bound on occurrence time (unix epoch ms, inclusive).
    pub occurred_from_ms: Option<i64>,
    /// Upper bound on occurrence time (unix epoch ms, exclusive).
    pub occurred_until_ms: Option<i64>,
    /// Lower bound on ingest time (`asset.created_at`, unix epoch ms,
    /// inclusive) — when the row entered *this* library, as opposed to
    /// [`occurred_from_ms`](Self::occurred_from_ms), which is when the
    /// thing it records happened. Importing a decade-old folder puts ten
    /// years between the two axes.
    ///
    /// **Both ends of the ingest and modification windows are
    /// inclusive**, the `_until_` end included. That diverges from
    /// `occurred_until_ms` (exclusive) deliberately: these four are
    /// cursor fields for differential sync, where a caller hands back a
    /// timestamp the server gave it rather than a day boundary it
    /// computed itself, and a half-open upper end would drop the row
    /// sitting exactly on the cursor.
    ///
    /// An inverted pair (`_from_` above `_until_`) is **not** rejected;
    /// it returns an empty page. That is a deliberate divergence from
    /// the rating band, which does reject `rating_min > rating_max`, and
    /// the reason is symmetry rather than merit: `occurred_from_ms` /
    /// `occurred_until_ms` have never validated their pair, and a caller
    /// mixes all three windows in one request. One inverted window
    /// answering `400` while the next answers an empty page is the
    /// harder rule to predict. The empty page is genuinely uninformative
    /// here, so this is a cost carried for consistency, not a free
    /// choice.
    pub created_from_ms: Option<i64>,
    /// Upper bound on ingest time (unix epoch ms, **inclusive** — see
    /// [`created_from_ms`](Self::created_from_ms)).
    pub created_until_ms: Option<i64>,
    /// Lower bound on last-modification time (`asset.updated_at`, unix
    /// epoch ms, inclusive) — the differential-sync cursor: "everything
    /// that changed since I last looked". Inclusivity and the
    /// non-rejection of an inverted pair are as described on
    /// [`created_from_ms`](Self::created_from_ms).
    ///
    /// **What the column actually tracks** is narrower than "any change
    /// to the asset", so a consumer building on this needs the list.
    /// Writes that advance `updated_at`:
    /// - metadata edits through `AssetService::update_meta` — labels,
    ///   memo, cover text, rating, modality, title
    /// - provenance declaration and re-resolution
    ///   (`declare_provenance`, `reresolve_unresolved`)
    /// - the pipeline's own column writes: `set_cover`,
    ///   `set_content_flags`, `set_keywords`
    /// - Session (composite asset) metadata edit and rename
    ///
    /// Writes that leave it untouched, and which this filter therefore
    /// **cannot** report:
    /// - **trash / restore**, single and per-persona — only
    ///   `trashed_at` moves. A consumer mirroring deletions has to ask
    ///   for `trash: "any"` and compare that column itself.
    /// - **tag attach / detach / rename / merge / delete** — write
    ///   `asset_tag` (and the `tag` row itself) only. The channel
    ///   administration verbs are in this list for the same reason
    ///   the per-asset ones are: a tag is a fact on the join table,
    ///   so folding two channels together can change what an asset
    ///   is tagged with while its stamp stands still
    /// - **group filing, unfiling and reordering**, Query-Group
    ///   materialisation included — writes `asset_bucket` only
    /// - **comments** — write `asset_comment` only
    /// - **palette extraction** (`set_palette`) and **material content
    ///   hashing** — write `asset.palette` / `material` with no stamp
    ///
    /// That is a record of current behaviour, not an endorsement of it.
    /// A missing stamp is a write-side defect; fixing one widens what
    /// this filter reports without changing what it means.
    ///
    /// # Using it as a cursor safely
    ///
    /// The stamp a caller replays comes from
    /// [`AssetCardDto::updated_at_ms`](crate::dto::AssetCardDto::updated_at_ms)
    /// (or its index-row twin), and two properties of it decide how the
    /// loop has to be written:
    ///
    /// **Rows are re-delivered.** The lower bound is inclusive, so
    /// handing back the highest stamp from the previous page returns
    /// every row sitting exactly on that instant again — and a bulk edit
    /// stamps many rows the same millisecond, so "again" can be a whole
    /// page rather than one row. A consumer must be idempotent per row
    /// (upsert by id, not append). Advancing to `stamp + 1` to avoid the
    /// replay is worse: it silently drops every row that shares the
    /// boundary millisecond and was not on the page.
    ///
    /// **The stamp is wall clock, not commit order.** Each writer takes
    /// `Utc::now()` *before* its write lands, so under concurrent writers
    /// a row committed later can carry an earlier stamp than one already
    /// returned — a row can appear below a cursor that has moved past it,
    /// and be missed forever. There is no sequence column to fall back
    /// on. The mitigation is to overlap: keep the cursor a margin behind
    /// the newest stamp seen (comfortably wider than the slowest write's
    /// clock-to-commit gap) and accept the extra re-delivery, which the
    /// idempotence above already absorbs. A clock adjustment on the host
    /// moves the whole axis and has the same shape, at a larger scale.
    pub updated_from_ms: Option<i64>,
    /// Upper bound on last-modification time (unix epoch ms,
    /// **inclusive** — see [`created_from_ms`](Self::created_from_ms)).
    pub updated_until_ms: Option<i64>,
    /// Channel-tag filter. [`tag_match`](Self::tag_match) says how the
    /// entries combine — **OR** by default, so an asset needs to carry
    /// at least one of these tags to pass. An empty vector disables the
    /// tag filter.
    ///
    /// Wire representation is dual so both transports work:
    /// - **JSON** (Tauri IPC / HTTP POST bodies) — a real array
    ///   (`"tag_ids": ["aa", "bb"]`).
    /// - **HTTP GET query string** — comma-separated
    ///   (`?tag_ids=aa,bb`), because `serde_urlencoded` (the
    ///   deserialiser axum's `Query` extractor uses) does not
    ///   support repeated keys → `Vec`.
    #[serde(default, deserialize_with = "deserialize_tag_ids")]
    pub tag_ids: Vec<String>,
    /// How [`tag_ids`](Self::tag_ids) combine. `any` = an asset carrying
    /// any one of them passes (OR, the default); `all` = only an asset
    /// carrying every one of them passes (AND). Meaningless when
    /// `tag_ids` is empty — there is nothing to combine, and neither
    /// value adds a clause.
    ///
    /// The axis is tags only. `group_ids` stays OR whatever this says:
    /// the two are separate filters, and a value named for one of them
    /// silently redefining the other is the surprise this note exists
    /// to rule out.
    #[serde(default)]
    pub tag_match: TagMatch,
    /// User-curated `Group` filter — same OR semantic and dual JSON /
    /// CSV wire form as `tag_ids`. A group is the hand-picked twin
    /// of a tag; the shared `deserialize_tag_ids` helper works
    /// verbatim because it does no id-shape validation.
    #[serde(default, deserialize_with = "deserialize_tag_ids")]
    pub group_ids: Vec<String>,
    /// Optional session drill filter — the composite Asset id whose
    /// members to list (drilling from a Session tile into its messages).
    /// session-model v2: the wire name stays `session_id` for
    /// back-compat, but the server maps it to the `container_id` axis.
    /// Empty / missing means "no session filter".
    pub session_id: Option<String>,
    /// Optional label filter — matches when the asset's `labels`
    /// array contains this literal (see
    /// `asterism_core::domain::asset::AssetQuery::label`).
    pub label: Option<String>,
    /// Optional body-text filter — matches when the asset's resolved
    /// body **contains this string**. No word boundaries and no
    /// dictionary: `スト` matches `テスト`, `猫` matches `黒猫`.
    ///
    /// This is a predicate like every other field here, which is the
    /// whole point of it: the result is a set, so it can be counted,
    /// ordered on any axis, paged to any depth, and used to define a
    /// Query Group's membership. It is *not* the fuzzy search box —
    /// that one asks `retrieve_assets` for a ranked shortlist and
    /// promises no completeness (`SearchAssetsQuery`).
    ///
    /// Trimmed before use; empty or whitespace-only means "no filter".
    pub text_match: Option<String>,
    /// Format facet filter — a mime top-level type (`image` / `video`
    /// / `audio` / `text`) matched as a prefix against the primary
    /// material's format fact. The sidebar FORMAT section drives it.
    pub format: Option<String>,
    /// Colour facet filter — one swatch slug (`red` / `blue` /
    /// `white` / …, the closed set `asterism_core::domain::color`
    /// defines). Matches when any entry of the asset's dominant-colour
    /// palette quantises into that bucket. An unknown slug is a
    /// validation error rather than a silent "no filter": showing the
    /// whole grid would read as "this colour matches everything".
    /// The sidebar COLOR section drives it.
    pub color: Option<String>,
    /// Lower bound on the star rating, inclusive (accepted range `0..=5`;
    /// anything else is a validation error rather than a clamp — a caller
    /// asking for `rating_min=7` is asking for something the axis cannot
    /// mean).
    ///
    /// **Unrated assets (`rating IS NULL`) are excluded as soon as either
    /// bound is named**, `rating_max` included: "at most 2 stars" is a
    /// question about assets that carry a rating, and an unrated asset
    /// carries no opinion to compare. Reaching the unrated set is the
    /// no-bounds case (or a future dedicated facet), not a wide range.
    pub rating_min: Option<u8>,
    /// Upper bound on the star rating, inclusive. Same range rule and the
    /// same NULL-exclusion semantics as [`rating_min`](Self::rating_min),
    /// with one extra rejection: `rating_max = 0` is a validation error,
    /// because stored ratings are `1..=5` (a zero on the write side clears
    /// the rating) so that band can never match. `rating_min > rating_max`
    /// is likewise an error rather than an empty page, because an empty
    /// page reads as "nothing is rated in that band".
    pub rating_max: Option<u8>,
    /// Lower bound on playback length in **milliseconds**, inclusive —
    /// the raw unit, whatever a client chooses to show it in.
    ///
    /// **Assets with no length (`duration_ms IS NULL`) are excluded as
    /// soon as either end is named**, `duration_max_ms` included: "under
    /// two minutes" is a question about material that plays, and a still
    /// image has no length to compare. A video whose container the
    /// importer could not probe drops out the same way — the parser
    /// leaves the column unset and records the miss in its `extra`
    /// (`asterism-importer-video`'s `mp4_probe_seen` /
    /// `matroska_probe_seen`), and being absent from a length band states
    /// that honestly, whereas admitting it as `0` would put it at the
    /// head of "shortest first" as though something had been measured.
    ///
    /// Not a video-only facet: the column holds however long the material
    /// plays for, so audio answers the same question.
    ///
    /// Unsigned because a negative length is not a narrower request but a
    /// malformed one — it fails at the transport instead of travelling to
    /// a domain check. `duration_min_ms > duration_max_ms` is likewise a
    /// validation error rather than an empty page, matching the rating
    /// band: an empty page reads as a claim about the library ("nothing
    /// is that long") rather than about the request.
    pub duration_min_ms: Option<u64>,
    /// Upper bound on playback length (milliseconds, inclusive). Same
    /// NULL-exclusion, unit and inversion rules as
    /// [`duration_min_ms`](Self::duration_min_ms).
    pub duration_max_ms: Option<u64>,
    /// Lower bound on stored size in **bytes**, inclusive — the raw unit;
    /// the MB a UI renders is that side's presentation.
    ///
    /// **Assets with no recorded size (`file_size_bytes IS NULL`) are
    /// excluded as soon as either end is named**, for the reason the
    /// length band gives: a row whose bytes were never recorded has
    /// nothing to place in a size band, and a stand-in `0` would make it
    /// the smallest thing in the library.
    ///
    /// `size_min_bytes > size_max_bytes` is a validation error, not an
    /// empty page.
    pub size_min_bytes: Option<u64>,
    /// Upper bound on stored size (bytes, inclusive). Same NULL-exclusion
    /// and inversion rules as [`size_min_bytes`](Self::size_min_bytes).
    pub size_max_bytes: Option<u64>,
    /// Lower bound on **total pixel count** (`width_px * height_px`),
    /// inclusive — the raw count; the megapixels a UI shows are that
    /// side's presentation, the same split the two bands above make.
    ///
    /// **The count, not the two sides.** Width and height are stored as
    /// the *coded* pixel dimensions of the byte stream, before any
    /// orientation is applied
    /// (`asterism_importer_sdk::footprint`), so a photo shot in portrait
    /// with EXIF Orientation 6 is held as a landscape pair, and a phone
    /// video shot upright is held as 1920x1080 with the display matrix
    /// nowhere on the row. Their **product is invariant under that
    /// rotation**, which is what makes it the one resolution question
    /// answerable from what is actually measured. Bands over width and
    /// height separately, or presets like "1080p", would read the coded
    /// pair as if it were the displayed one and answer backwards for
    /// exactly the upright material a phone library is full of.
    ///
    /// **Rows with no measured dimensions are excluded as soon as either
    /// end is named**, for the reason the length and size bands give: the
    /// pair is written together or not at all
    /// (`AssetService::add` refuses a half-written pair), so an
    /// unmeasured row has no count to place in a band, and a stand-in `0`
    /// would make it the smallest picture in the library.
    ///
    /// A stored `0` — a side actually measured as zero — is a real value
    /// and stays in the band, unlike an unmeasured row. Nothing in the
    /// column's contract promises a picture has area.
    ///
    /// `pixels_min > pixels_max` is a validation error, not an empty
    /// page.
    pub pixels_min: Option<u64>,
    /// Upper bound on total pixel count (inclusive). Same NULL-exclusion
    /// and inversion rules as [`pixels_min`](Self::pixels_min).
    pub pixels_max: Option<u64>,
    /// Which side of the trash to read: `"live"` (default), `"trashed"`
    /// (the trash view), or `"any"`.
    ///
    /// Omitted / `null` means live, so a client that predates the trash
    /// cannot accidentally surface trashed assets. Unknown values are a
    /// validation error rather than a silent fallback — a typo here
    /// would otherwise quietly show the wrong side of a destructive
    /// feature.
    pub trash: Option<String>,
    /// AlbumMeta name filter — rows carrying a statement filed under
    /// this key (`asterism_core::domain::album_meta`).
    ///
    /// This is the *filter* half of AlbumMeta, and the separation is the
    /// design rather than an implementation detail: a recorded
    /// identifier stays a statement, and looking rows up by one is a
    /// secondary index over asset identity, never a replacement for it.
    /// Nothing here promises the answer is one row.
    ///
    /// Checked against the same shape the write side accepts, so a key
    /// no statement could have been filed under is a validation error
    /// rather than an empty page — the same rule `rating_max = 0`
    /// follows, for the same reason: an always-empty answer is a fact
    /// about the request, not about the corpus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_meta_key: Option<String>,
    /// AlbumMeta value filter — rows carrying this exact value, under
    /// [`album_meta_key`](Self::album_meta_key) when one is named and
    /// under any name otherwise.
    ///
    /// Naming a value alone is supported on purpose: somebody holding a
    /// generator's reference usually does not know which name it was
    /// filed under. Matching is exact, not prefix or substring — free
    /// text is what the search path is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_meta_value: Option<String>,
    /// Sort axis to order the filtered set by, evaluated **server-side**.
    ///
    /// `None` keeps the repository's own arrival order — the manual
    /// arrangement (`asset_bucket.position`) when the filter names exactly
    /// one Group, `occurred_at DESC` otherwise. That is what every client
    /// got before this field existed, and it is what the desktop UI still
    /// asks for: it fetches the whole index eagerly and applies the same
    /// comparator itself (`lib/sort/card-cmp.ts`), so the axis never
    /// reached the wire.
    ///
    /// Naming an axis here runs [`asterism_core::domain::sort_eval`] — the
    /// backend twin of that comparator — over the whole filtered set
    /// before pagination, so an HTTP caller can reproduce exactly what the
    /// grid shows for a given `Sort` / `Order` pick. Without it the
    /// ordering was only observable by driving the UI, which is how a pair
    /// of ordering defects reached a release unnoticed (see 727c6a3).
    ///
    /// Wire representation is dual, same reason as
    /// [`tag_ids`](Self::tag_ids):
    /// - **JSON** — a real object
    ///   (`"sort": {"target":"group","order":"ordered","reverse":false}`).
    /// - **HTTP GET query string** — the same object JSON-encoded into one
    ///   value (`?sort=%7B%22target%22%3A%22group%22...%7D`), because
    ///   `serde_urlencoded` cannot express a nested struct.
    #[serde(default, deserialize_with = "deserialize_sort")]
    pub sort: Option<SortSpec>,
    /// Page offset.
    pub offset: u64,
    /// Page size (the server may clamp against a hard upper bound).
    pub limit: u64,
}

impl Default for ListAssetsQuery {
    fn default() -> Self {
        Self {
            viewer_subject: None,
            persona_id: None,
            modality: None,
            occurred_from_ms: None,
            occurred_until_ms: None,
            // All four `None` = no ingest / modification window asked
            // for. A default window would turn every client that never
            // set the field into a partial reader of its own library.
            created_from_ms: None,
            created_until_ms: None,
            updated_from_ms: None,
            updated_until_ms: None,
            tag_ids: Vec::new(),
            // OR: the semantic every caller had before the field
            // existed, and the one a multi-select widens under.
            tag_match: TagMatch::Any,
            group_ids: Vec::new(),
            session_id: None,
            label: None,
            text_match: None,
            format: None,
            color: None,
            // Both `None` = no rating band asked for, which is the only
            // way unrated assets stay in the result set.
            rating_min: None,
            rating_max: None,
            // Both `None` = no AlbumMeta filter. Naming only the value
            // is a supported request, so neither field implies the
            // other.
            album_meta_key: None,
            album_meta_value: None,
            // Same shape one axis over: no band asked for is the only
            // state in which rows carrying no length / no recorded size
            // (stills, unprobed containers) stay in the result set.
            duration_min_ms: None,
            duration_max_ms: None,
            size_min_bytes: None,
            size_max_bytes: None,
            // Same again on the resolution axis: no band asked for is the
            // only state in which rows nobody measured (everything
            // ingested before V69, every non-visual material) stay in the
            // result set.
            pixels_min: None,
            pixels_max: None,
            // `None` = live side; see the field doc for why the trash
            // view has to be asked for explicitly.
            trash: None,
            // `None` = the repository's arrival order, not a default axis:
            // asking for no sort and asking for `occurred_at DESC` are
            // different requests (the single-Group case arrives in its
            // manual arrangement).
            sort: None,
            offset: 0,
            limit: 200,
        }
    }
}

/// Full-text / fuzzy search. Shares the same filter and pagination shape
/// as [`ListAssetsQuery`].
///
/// Results are ranked by relevance (BM25) and that ranking **is** the
/// order: [`filter.sort`](ListAssetsQuery::sort) is refused with a
/// validation error rather than accepted and dropped. Sorted listings are
/// the list path's job (`asset_list` / `POST /asterism/assets`), which
/// takes the same filter surface. The unblock point, if search ever gains
/// an axis, is the guard in `AssetService::search`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct SearchAssetsQuery {
    /// Query text.
    pub text: String,
    /// Filter + pagination — the list query verbatim, including its
    /// `sort` field. Naming an axis there is a `400` on this path (see
    /// the type docs); every other field narrows exactly as it does on
    /// the list path.
    pub filter: ListAssetsQuery,
}

/// A random handful out of the set a filter describes — the "🎲 Random"
/// entry of the sidebar's Discover section.
///
/// The filter is [`ListAssetsQuery`] verbatim, so every chip the grid has
/// lit narrows the pool the picks come from. What comes back is a
/// *sample*, not a page: it does not enumerate the set, it cannot be
/// paged through, and asking the same question twice answers differently.
///
/// [`filter.sort`](ListAssetsQuery::sort) is refused with a validation
/// error for the same reason the search path refuses it: the order here
/// *is* the shuffle, so an axis would have to discard it. Sorted listings
/// are the list path's job.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct RandomAssetsQuery {
    /// Filter — the list query verbatim. Naming an axis in `sort` is a
    /// `400` on this path (see the type docs); `limit` / `offset` are
    /// ignored, because a sample has no pages. Use `k` for the size.
    pub filter: ListAssetsQuery,
    /// How many picks to draw. `None` takes the service default (100);
    /// values outside `1..=500` are clamped rather than refused — every
    /// number in that request still names the same question, only wider
    /// or narrower than the implementation will serve.
    pub k: Option<u32>,
}

/// Detail view (asset + tags + constellation edges).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct GetAssetDetailQuery {
    /// Target asset id.
    pub asset_id: String,
    /// Requesting subject (`None` = owner view).
    pub viewer_subject: Option<String>,
}

/// Job status lookup (for progress polling; push updates travel over the
/// `job:progress:{id}` event channel).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct GetJobStatusQuery {
    /// Target job id.
    pub job_id: String,
}

/// Telemetry event listing — newest first. All filters are optional so
/// an HTTP `GET /asterism/events` query string can express "everything
/// recent" (`limit` alone), one kind, or a time window. Consumed by
/// the UI and by agents aggregating usage summaries over the HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[serde(default)]
pub struct ListEventsQuery {
    /// Optional event-kind filter (exact match on the open slug,
    /// e.g. `persona_switch`).
    pub kind: Option<String>,
    /// Lower bound on `occurred_at` (unix epoch ms, inclusive).
    pub since_ms: Option<i64>,
    /// Upper bound on `occurred_at` (unix epoch ms, exclusive).
    pub until_ms: Option<i64>,
    /// Page size (the server clamps against a hard upper bound).
    pub limit: u32,
}

impl Default for ListEventsQuery {
    fn default() -> Self {
        Self {
            kind: None,
            since_ms: None,
            until_ms: None,
            limit: 500,
        }
    }
}

/// Severity of a persisted diagnostic — the closed set `tracing` can
/// produce, and the only values `ListDiagQuery::min_level` accepts.
///
/// A closed type rather than a string comparison: severity is an
/// ordering, callers filter on it, and a value outside the set has no
/// meaningful rank. Keeping that knowledge in one place is what lets
/// the reader turn `min_level` into an exact SQL predicate instead of
/// guessing, and lets `GET /asterism/diag/levels` publish the accepted
/// values rather than leaving a caller to discover them by trial.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, SchemaBridge,
)]
#[serde(rename_all = "lowercase")]
pub enum DiagLevel {
    /// Most verbose.
    Trace,
    /// Diagnostic detail.
    Debug,
    /// Normal operational record (the default floor for persistence).
    Info,
    /// Something was skipped, ignored, or fell back.
    Warn,
    /// Most severe.
    Error,
}

impl DiagLevel {
    /// Every level, ascending by severity. The order is the ordering —
    /// `min_level` semantics and the published listing both read it
    /// from here.
    pub const ALL: [DiagLevel; 5] = [
        DiagLevel::Trace,
        DiagLevel::Debug,
        DiagLevel::Info,
        DiagLevel::Warn,
        DiagLevel::Error,
    ];

    /// Wire / storage spelling. Uppercase because that is what
    /// `tracing::Level` writes into `diag_log.level`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    /// Parses a caller-supplied level, case-insensitively.
    ///
    /// `Err` carries the accepted values, so a typo answers itself
    /// without the caller having to look anything up.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let needle = raw.trim().to_ascii_uppercase();
        Self::ALL
            .into_iter()
            .find(|level| level.as_str() == needle)
            .ok_or_else(|| {
                let accepted: Vec<&str> = Self::ALL.iter().map(|l| l.as_str()).collect();
                format!(
                    "unknown level {raw:?}; expected one of {}",
                    accepted.join(", ")
                )
            })
    }

    /// This level and everything more severe — the set `min_level`
    /// selects, ordered most severe first.
    pub fn at_least(self) -> Vec<DiagLevel> {
        let mut out: Vec<DiagLevel> = Self::ALL.into_iter().filter(|l| *l >= self).collect();
        out.reverse();
        out
    }
}

/// Diagnostic listing — newest first, the read side of `diag_log`.
///
/// HTTP-only by design (`GET /asterism/diag`). Diagnostics are for
/// whoever is investigating the application, not for the application to
/// show its user, so this deliberately has no Tauri command and no
/// generated TypeScript binding. Reaching it means opening a terminal,
/// which is the right amount of friction.
///
/// Filters mirror [`ListEventsQuery`] so the two logs are queried the
/// same way, with `level` / `target` standing in for `kind`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[serde(default)]
pub struct ListDiagQuery {
    /// Minimum severity, inclusive. `None` = every level. Accepted
    /// values are [`DiagLevel::ALL`], case-insensitive; anything else
    /// is a `400` naming the accepted set. `GET /asterism/diag/levels`
    /// publishes the same list.
    ///
    /// A *minimum* rather than an exact match because the question is
    /// almost always "show me the bad ones", and asking for exactly
    /// `WARN` would hide the `ERROR` above it.
    ///
    /// Typed as a `String` because it arrives from a query string;
    /// [`DiagLevel::parse`] turns it into the closed type at the
    /// boundary, so no comparison downstream works on raw text.
    pub min_level: Option<String>,
    /// Substring match on the emitting module path
    /// (`asterism_core::application`). Narrows a noisy log to one area.
    pub target: Option<String>,
    /// Lower bound on `occurred_at` (unix epoch ms, inclusive).
    pub since_ms: Option<i64>,
    /// Upper bound on `occurred_at` (unix epoch ms, exclusive).
    pub until_ms: Option<i64>,
    /// Page size (the server clamps against a hard upper bound).
    ///
    /// Exact: every filter is applied in the query, so a page shorter
    /// than `limit` means there were no more matching records in the
    /// requested window — not that the server stopped looking.
    pub limit: u32,
}

impl Default for ListDiagQuery {
    fn default() -> Self {
        Self {
            min_level: None,
            target: None,
            since_ms: None,
            until_ms: None,
            limit: 500,
        }
    }
}

/// Timing listing — newest first, the read side of `perf_log`.
///
/// HTTP-only, like the rest of the observation reads.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[serde(default)]
pub struct ListPerfQuery {
    /// Exact match on the operation (`list_index`). Exact rather than a
    /// substring: `op` is a closed set the writers choose, and the
    /// question is "how does *this* operation behave".
    pub op: Option<String>,
    /// Lower bound on `occurred_at` (unix epoch ms, inclusive).
    pub since_ms: Option<i64>,
    /// Upper bound on `occurred_at` (unix epoch ms, exclusive).
    pub until_ms: Option<i64>,
    /// Page size (the server clamps against a hard upper bound).
    pub limit: u32,
}

impl Default for ListPerfQuery {
    fn default() -> Self {
        Self {
            op: None,
            since_ms: None,
            until_ms: None,
            limit: 500,
        }
    }
}

/// Job-run listing — newest first, the read side of `job_log`.
///
/// HTTP-only, like the rest of the observation reads.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[serde(default)]
pub struct ListJobLogQuery {
    /// Exact match on the job-kind slug (`cover_gen`).
    pub job_kind: Option<String>,
    /// Exact match on the outcome (`failed`). Not a minimum: outcomes
    /// are unordered categories, so "at least failed" would mean
    /// nothing.
    pub outcome: Option<String>,
    /// Lower bound on `occurred_at` (unix epoch ms, inclusive).
    pub since_ms: Option<i64>,
    /// Upper bound on `occurred_at` (unix epoch ms, exclusive).
    pub until_ms: Option<i64>,
    /// Page size (the server clamps against a hard upper bound).
    pub limit: u32,
}

impl Default for ListJobLogQuery {
    fn default() -> Self {
        Self {
            job_kind: None,
            outcome: None,
            since_ms: None,
            until_ms: None,
            limit: 500,
        }
    }
}

/// Cross-stream listing — newest first, over the `observation` view.
///
/// The single-timeline read the four separate tables would otherwise
/// cost. Returns the shared envelope only: a stream's own columns are
/// the reason it is a separate table, so asking for them means asking
/// that stream's own endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
#[serde(default)]
pub struct ListObservationsQuery {
    /// Restrict to one stream. `None` = all four, interleaved.
    ///
    /// Typed as a `String` because it arrives from a query string; the
    /// reader parses it into the closed domain type at the boundary, so
    /// an unknown name is a `400` naming the accepted set rather than a
    /// filter that silently matches nothing.
    /// `GET /asterism/observations/streams` publishes that set.
    pub stream: Option<String>,
    /// Lower bound on `occurred_at` (unix epoch ms, inclusive).
    pub since_ms: Option<i64>,
    /// Upper bound on `occurred_at` (unix epoch ms, exclusive).
    pub until_ms: Option<i64>,
    /// Page size (the server clamps against a hard upper bound).
    pub limit: u32,
}

impl Default for ListObservationsQuery {
    fn default() -> Self {
        Self {
            stream: None,
            since_ms: None,
            until_ms: None,
            limit: 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sort::{SortOrder, SortTarget};

    /// The two transports carry the axis differently — a nested object over
    /// JSON, one encoded value over a `GET` query string — and both have to
    /// land on the same spec, or "reproduce what the grid shows" only works
    /// from one of them.
    ///
    /// The query-string case is expressed as a *string-valued* field rather
    /// than through `serde_urlencoded`: a scalar string is exactly what a
    /// form deserialiser hands the field, and asserting on that keeps the
    /// test on this crate's own contract instead of on which form crate
    /// axum happens to use.
    #[test]
    fn sort_accepts_object_and_encoded_string() {
        let from_object: ListAssetsQuery = serde_json::from_str(
            r#"{"offset":0,"limit":10,
                "sort":{"target":"group","order":"ordered","reverse":false}}"#,
        )
        .unwrap();
        let from_scalar: ListAssetsQuery = serde_json::from_str(
            r#"{"offset":0,"limit":10,
                "sort":"{\"target\":\"group\",\"order\":\"ordered\",\"reverse\":false}"}"#,
        )
        .unwrap();

        let spec = from_object.sort.expect("object form parsed");
        assert_eq!(spec.target, SortTarget::Group);
        assert_eq!(spec.order, SortOrder::Ordered);
        assert!(!spec.reverse);
        assert_eq!(from_scalar.sort, Some(spec));
    }

    /// Omitted and blank both mean "no axis named" — the repository's
    /// arrival order, which is a different answer from any axis.
    #[test]
    fn sort_absent_or_blank_is_none() {
        let omitted: ListAssetsQuery = serde_json::from_str(r#"{"offset":0,"limit":10}"#).unwrap();
        assert_eq!(omitted.sort, None);
        let blank: ListAssetsQuery =
            serde_json::from_str(r#"{"offset":0,"limit":10,"sort":"  "}"#).unwrap();
        assert_eq!(blank.sort, None);
    }

    /// The rating band is a plain numeric pair — no dual deserialiser the
    /// way `tag_ids` and `sort` need one. This fixture pins the JSON
    /// transport (IPC / POST bodies); the `GET` query-string form rides
    /// the same serde integer path as the other plain numeric fields
    /// (`occurred_from_ms` et al.) and is not separately fixtured.
    #[test]
    fn rating_bounds_parse_from_numbers() {
        let parsed: ListAssetsQuery =
            serde_json::from_str(r#"{"offset":0,"limit":10,"rating_min":3,"rating_max":5}"#)
                .unwrap();
        assert_eq!(parsed.rating_min, Some(3));
        assert_eq!(parsed.rating_max, Some(5));
    }

    /// Omitted means "no band asked for", which is the only state that
    /// keeps unrated assets in the answer. A default that named a band
    /// would hide every unrated row from every client that never set the
    /// field.
    #[test]
    fn rating_bounds_default_to_absent() {
        let omitted: ListAssetsQuery = serde_json::from_str(r#"{"offset":0,"limit":10}"#).unwrap();
        assert_eq!(omitted.rating_min, None);
        assert_eq!(omitted.rating_max, None);
        assert_eq!(ListAssetsQuery::default().rating_min, None);
        assert_eq!(ListAssetsQuery::default().rating_max, None);
    }

    /// The length and size bands are plain numeric pairs like the rating
    /// one. The fixture pins that all four land on distinct fields: a
    /// copy-paste aiming the size pair at the duration column would still
    /// parse, and would answer a byte question in milliseconds.
    #[test]
    fn duration_and_size_bounds_parse_from_numbers() {
        let parsed: ListAssetsQuery = serde_json::from_str(
            r#"{"offset":0,"limit":10,
                "duration_min_ms":1000,"duration_max_ms":120000,
                "size_min_bytes":1024,"size_max_bytes":1048576}"#,
        )
        .unwrap();
        assert_eq!(parsed.duration_min_ms, Some(1_000));
        assert_eq!(parsed.duration_max_ms, Some(120_000));
        assert_eq!(parsed.size_min_bytes, Some(1_024));
        assert_eq!(parsed.size_max_bytes, Some(1_048_576));
    }

    /// Omitted means "no band asked for", the only state that keeps rows
    /// carrying neither column — stills, unprobed containers — in the
    /// answer. A default band would hide them from every client that
    /// never set the field.
    #[test]
    fn duration_and_size_bounds_default_to_absent() {
        let omitted: ListAssetsQuery = serde_json::from_str(r#"{"offset":0,"limit":10}"#).unwrap();
        for value in [
            omitted.duration_min_ms,
            omitted.duration_max_ms,
            omitted.size_min_bytes,
            omitted.size_max_bytes,
        ] {
            assert_eq!(value, None);
        }
        let default = ListAssetsQuery::default();
        assert_eq!(default.duration_min_ms, None);
        assert_eq!(default.duration_max_ms, None);
        assert_eq!(default.size_min_bytes, None);
        assert_eq!(default.size_max_bytes, None);
    }

    /// The reason both bands are unsigned: a negative length or size is
    /// not a narrower request but a malformed one, so it is refused at
    /// the transport and never reaches a domain check that would have to
    /// decide what it meant.
    #[test]
    fn negative_duration_or_size_is_refused_at_the_wire() {
        for body in [
            r#"{"offset":0,"limit":10,"duration_min_ms":-1}"#,
            r#"{"offset":0,"limit":10,"duration_max_ms":-1}"#,
            r#"{"offset":0,"limit":10,"size_min_bytes":-1}"#,
            r#"{"offset":0,"limit":10,"size_max_bytes":-1}"#,
        ] {
            assert!(
                serde_json::from_str::<ListAssetsQuery>(body).is_err(),
                "a negative bound must not parse: {body}"
            );
        }
    }

    /// The ingest / modification windows are plain integer pairs on the
    /// same serde path as `occurred_from_ms`. The fixture pins that all
    /// four are wired to distinct field names — a copy-paste that pointed
    /// two of them at one column would still parse, and would answer a
    /// `created_*` question with the `updated_*` window.
    #[test]
    fn ingest_and_modification_windows_parse_independently() {
        let parsed: ListAssetsQuery = serde_json::from_str(
            r#"{"offset":0,"limit":10,
                "created_from_ms":10,"created_until_ms":20,
                "updated_from_ms":30,"updated_until_ms":40}"#,
        )
        .unwrap();
        assert_eq!(parsed.created_from_ms, Some(10));
        assert_eq!(parsed.created_until_ms, Some(20));
        assert_eq!(parsed.updated_from_ms, Some(30));
        assert_eq!(parsed.updated_until_ms, Some(40));
    }

    /// Omitted means "no window", the only state in which a client sees
    /// its whole library. A default window would silently truncate every
    /// caller that predates these fields.
    #[test]
    fn time_windows_default_to_absent() {
        let omitted: ListAssetsQuery = serde_json::from_str(r#"{"offset":0,"limit":10}"#).unwrap();
        for value in [
            omitted.created_from_ms,
            omitted.created_until_ms,
            omitted.updated_from_ms,
            omitted.updated_until_ms,
        ] {
            assert_eq!(value, None);
        }
        let default = ListAssetsQuery::default();
        assert_eq!(default.created_from_ms, None);
        assert_eq!(default.created_until_ms, None);
        assert_eq!(default.updated_from_ms, None);
        assert_eq!(default.updated_until_ms, None);
    }

    /// The combinator's wire tokens, and the default an omitting caller
    /// gets. Omitted must read as `any`: every client written before the
    /// field existed sends no `tag_match`, and defaulting to `all` would
    /// silently narrow each of their multi-tag filters to an
    /// intersection.
    #[test]
    fn tag_match_defaults_to_any_and_parses_lowercase() {
        let omitted: ListAssetsQuery = serde_json::from_str(r#"{"offset":0,"limit":10}"#).unwrap();
        assert_eq!(omitted.tag_match, TagMatch::Any);
        assert_eq!(ListAssetsQuery::default().tag_match, TagMatch::Any);

        for (variant, token) in [(TagMatch::Any, "\"any\""), (TagMatch::All, "\"all\"")] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), token);
            assert_eq!(serde_json::from_str::<TagMatch>(token).unwrap(), variant);
        }
        let asked: ListAssetsQuery =
            serde_json::from_str(r#"{"offset":0,"limit":10,"tag_match":"all"}"#).unwrap();
        assert_eq!(asked.tag_match, TagMatch::All);
        // An unknown combinator is refused rather than answered with the
        // default — a typo would otherwise widen the filter in silence.
        assert!(
            serde_json::from_str::<ListAssetsQuery>(
                r#"{"offset":0,"limit":10,"tag_match":"both"}"#
            )
            .is_err()
        );
    }

    /// A misspelled axis is an error, not a silent fallback: answering it
    /// with the arrival order would look like the sort ran.
    #[test]
    fn sort_rejects_an_unknown_axis() {
        let parsed = serde_json::from_str::<ListAssetsQuery>(
            r#"{"offset":0,"limit":10,
                "sort":"{\"target\":\"colour\",\"order\":\"alpha\",\"reverse\":false}"}"#,
        );
        assert!(parsed.is_err(), "unknown target must not parse: {parsed:?}");
    }
}
