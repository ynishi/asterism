//! Response DTOs — outputs shared between the application services, the
//! TypeScript bindings, and any future MCP tool responses.
//!
//! Conversion from domain types happens in `asterism-core`
//! (`application::mapping`); this crate is a leaf and knows nothing about
//! the domain types.

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// Per-persona identity signal — the avatar / short bio / role
/// tag the archive UI surfaces on the sidebar Profile card. Kept
/// separate from `PersonaThemeDto` because identity metadata and
/// UI chrome evolve on independent commit cadences.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PersonaProfileDto {
    /// Owner persona id.
    pub persona_id: String,
    /// Portrait / thumbnail asset id (must reference an image asset
    /// already imported into asterism).
    pub avatar_asset_id: Option<String>,
    /// One-line description of who this persona is inside asterism.
    pub bio_short: Option<String>,
    /// Free-form role tag chip.
    pub role_tag: Option<String>,
    /// Last-change timestamp (unix epoch ms).
    pub updated_at_ms: i64,
}

/// Per-persona visual chrome — currently holds the wallpaper asset
/// reference. Returned by `get_persona_theme` and echoed back by
/// `set_persona_theme` so the UI can flip its CSS variables without
/// a follow-up query.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PersonaThemeDto {
    /// Owner persona id.
    pub persona_id: String,
    /// Image asset id backing the wallpaper. `None` means no
    /// wallpaper is set even though the theme row exists.
    pub wallpaper_asset_id: Option<String>,
    /// Last-change timestamp (unix epoch ms).
    pub updated_at_ms: i64,
}

/// Persona payload.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PersonaDto {
    /// Persona id (UUID hyphenated).
    pub id: String,
    /// Optional natural key from an external persona pack.
    pub pack_id: Option<String>,
    /// Display name.
    pub name: String,
    /// Accent colour applied in the sidebar.
    pub accent_color: Option<String>,
    /// Sort order in the sidebar.
    pub display_order: i64,
    /// Whether the persona is archived.
    pub archived: bool,
    /// Ingest timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Last modification timestamp (unix epoch ms).
    pub updated_at_ms: i64,
}

/// Wire default for [`AssetCardDto::role`]. A payload that predates
/// the field can only describe an item — containers were not listable
/// through the card path before the field existed.
fn default_role() -> String {
    "item".to_string()
}

/// Wire default for the `media` fields. A payload that predates the
/// field says nothing about a player, and `none` is the reading that
/// promises nothing — the same conservative direction `render_policy`
/// takes for an unknown mime.
fn default_media() -> String {
    "none".to_string()
}

/// Lightweight card representation used on the grid (wire form of
/// `AssetCard`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetCardDto {
    /// Asset id.
    pub id: String,
    /// Persona bucket id.
    pub persona_id: String,
    /// Semantic classification slug (`None` = unclassified — the
    /// normal state for conversation messages and containers).
    pub modality: Option<String>,
    /// Occurrence timestamp (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Cover text (`None` while the `cover_gen` job is still pending;
    /// UIs should show a placeholder).
    pub cover: Option<String>,
    /// Labels rendered as badges on the card.
    pub labels: Vec<String>,
    /// Original artefact size (weight signal). Also the `FileSize` sort
    /// axis's key — see [`AssetIndexEntryDto::file_size_bytes`] for why
    /// the light row carries it too.
    pub file_size_bytes: Option<u64>,
    /// Playback length in ms for time-bounded material (video, audio),
    /// the `Duration` sort axis's key.
    ///
    /// `None` says the row has no measured length, which covers both
    /// "does not play" (a still image) and "nobody probed it" — the card
    /// cannot tell those apart and neither can the sort, which tails
    /// absent rows in both directions rather than reading them as zero.
    ///
    /// The value already existed on [`AssetDto`], the detail payload.
    /// It is here as well because the card projection is what the grid
    /// sorts and what the light index row widens into: a card without it
    /// meant the axis compared absent values on every row, whatever the
    /// index carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Total pixel count (`width_px * height_px`) — the `Pixels` sort
    /// axis's key, with the same three-valued reading as `duration_ms`:
    /// `None` says nothing measured this row's dimensions, not that it
    /// has no area.
    ///
    /// **The product, and not the pair.** The two columns behind it hold
    /// *coded* dimensions, taken before orientation is applied, so
    /// rendering them here as "1920 × 1080" would label an upright phone
    /// video with a landscape size. The card carries the one figure that
    /// survives that rotation and nothing a caller could mistake for a
    /// displayed shape; the detail payload ([`AssetDto::width_px`]) is
    /// where the raw pair lives, with the caveat attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_count: Option<u64>,
    /// Format fact of the primary material (`image/png`,
    /// `text/plain`, …; `None` = unknown). The card-level answer to
    /// "is this an image?" — asset-model v4 moved that question off
    /// the modality axis onto the material layer.
    pub mime: Option<String>,
    /// Which inline player these bytes call for — `image` / `video` /
    /// `audio` / `none`, the [`MediaKind`] slug.
    ///
    /// Projected so the UI stops deriving it. `mime` above is the raw
    /// fact and answering "is this an image?" from it means repeating
    /// `render_policy`'s rules in TypeScript, which is what five
    /// `startsWith("image/")` sites were doing — they had no way to
    /// learn that an unnamed `image/*` subtype still tiles, because
    /// that decision lives in `MimeType`.
    pub media: String,
    /// Where the original artefact is, **rendered for display** — the
    /// path of a file, the container of a record, the address of a
    /// remote, the name of a logical locator. Passed to
    /// `convertFileSrc()` on the frontend for image cards; retained
    /// as a plain string for other formats so downstream views
    /// (detail panels, jump-to-source links) can reuse it.
    ///
    /// Not the stored form, deliberately. Every consumer renders this —
    /// a basename label, a tooltip, a clipboard copy — and none reads it
    /// back, so the wire type is not coupled to the storage encoding
    /// and does not change when that does.
    pub source_locator: String,
    /// Group ids the asset is filed into (m:n `asset_bucket`). The
    /// UI uses this for the `Group` sort axis and the group-lane
    /// tile counts without an extra round trip per card.
    pub group_ids: Vec<String>,
    /// The card's slot inside its primary group — `asset_bucket.position`
    /// for `group_ids[0]`. `None` when the card is unfiled.
    ///
    /// This is the hand arrangement, and it is the only way the grid can
    /// show it: the repository sorts by `position` only when the filter
    /// names exactly one group, so without the value on the card the
    /// arrangement is invisible under every other filter shape (and the
    /// UI cannot tell a position-ordered page from a time-ordered one).
    /// Feeds `Group` + `ordered` on both comparators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_group_position: Option<i64>,
    /// Ingest timestamp (unix epoch ms) — when Asterism recorded
    /// the row. Used by the grid's "Added" sort so a payload with a
    /// future / historical `occurred_at_ms` can still be surfaced
    /// in arrival order.
    pub created_at_ms: i64,
    /// Last-modification timestamp (unix epoch ms) — the
    /// **differential-sync cursor**: the value a consumer hands straight
    /// back as
    /// [`ListAssetsQuery::updated_from_ms`](crate::query::ListAssetsQuery::updated_from_ms)
    /// to ask "what changed since I last looked". Both ends of that
    /// window are inclusive, so replaying the highest stamp from a page
    /// re-delivers the rows sitting exactly on it — process idempotently
    /// rather than assuming each row arrives once.
    ///
    /// Carried on the card so a sync loop closes inside the list
    /// response. Without it the cursor is only obtainable by fetching
    /// each row's full detail, which is an N+1 against the very path
    /// that exists to avoid one.
    ///
    /// The set of writes that advance the stamp is narrower than "any
    /// change to the asset" — trash, tagging, group filing and comments
    /// leave it untouched. The window field's docs carry the exhaustive
    /// list.
    pub updated_at_ms: i64,
    /// Star rating 0-5 (`None` = unrated). Rendered as the industry-
    /// standard 5-star widget on the card head; keyboard `0`-`5`
    /// mutates the hovered / selected card's rating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    /// Dominant colour palette — up to 5 lowercase `"#rrggbb"`
    /// strings extracted by the `thumb_gen` job. `None` for
    /// non-image assets or rows whose thumbnail has not been
    /// processed yet. Rendered as a colour strip on the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
    /// `true` when the asset carries a non-empty `register_note`.
    /// Renders a 📝 badge on the card head.
    #[serde(default)]
    pub has_note: bool,
    /// `true` when the asset has at least one `AssetComment`.
    /// Renders a 💬 badge on the card head.
    #[serde(default)]
    pub has_thread: bool,
    /// Structural role slug — `"item"` or `"collection"`. Lets the UI
    /// tell a container apart from an item on the read path: a
    /// collection owns no material, so the item card renders it as a
    /// nameless coverless placeholder. The grid filters these out at
    /// the query layer; this field is what makes the distinction
    /// visible anywhere the two can still mix (search, saved queries).
    #[serde(default = "default_role")]
    pub role: String,
    /// Hand-given name (`None` when never named). A container owns no
    /// material, so this is the only text the grid can put on its card
    /// until a cover is generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Live assets filed inside this one. `0` for every item; for a
    /// container it is the card's headline number.
    #[serde(default)]
    pub member_count: u64,
    /// Search-only: BM25 score assigned by the full-text index.
    /// `None` on the grid / detail read paths where rank is
    /// irrelevant. Populated by `AssetService::search`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Search-only: highlighted snippet extracted from the asset's
    /// body around the query terms (HTML with `<b>` tags around the
    /// matches). `None` when the body had no material for the query
    /// or on non-search paths. Rendered by the UI's hover-burst
    /// preview lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Attribution kind — `"owner"` (no subject) or `"subject"`
    /// (subject in [`AssetCardDto::author_subject`]). Same pair
    /// [`AssetDto::author_kind`] carries, denormalised onto the card so
    /// the grid / detail read path answers "who is this by" without a
    /// second round trip. Absent means **unrecorded**, which is not
    /// "the owner wrote it".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// Subject token this asset is attributed to. Present only
    /// alongside `author_kind = "subject"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// Agent that performed the operation (`claude-code`, `codex`,
    /// `asterism-ui`, …) — an open slug, absent when unrecorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Paginated grid page.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetPageDto {
    /// Cards in the current page.
    pub items: Vec<AssetCardDto>,
    /// Echo of the requested offset.
    pub offset: u64,
    /// Echo of the requested limit.
    pub limit: u64,
    /// Total row count after the filter (adapters may skip and leave
    /// `None`).
    pub total: Option<u64>,
}

/// One page of a **retrieval** — the ranked shortlist, narrowed by the
/// filter and cut to a page.
///
/// A separate type from [`AssetPageDto`] because it cannot answer the
/// question that one's `total` answers. Retrieval looks at a bounded
/// number of candidates and returns the closest of them, so "how many
/// assets match" is not a number it holds. Reusing the page shape meant
/// the count of *survivors of the shortlist* travelled under the name
/// `total` and was rendered as a library-wide count — a wrong number
/// with nothing marking it as wrong.
///
/// The exhaustive, countable question is the Query side's: the same
/// text as `ListAssetsQuery::text_match` answers it exactly.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RetrievedPageDto {
    /// Cards in the current page, best match first.
    pub items: Vec<AssetCardDto>,
    /// Echo of the requested offset.
    pub offset: u64,
    /// Echo of the requested limit.
    pub limit: u64,
    /// How many candidates passed the filter. An answer about the
    /// shortlist, **not** about the library: it can only ever be as
    /// large as `candidates_considered`.
    pub matched: u64,
    /// How many candidates the retriever looked at before filtering.
    pub candidates_considered: u64,
    /// The shortlist filled to its ceiling, so assets beyond it were
    /// never offered to the filter and cannot appear here however deep
    /// the caller pages. Presenting `matched` as a complete count is
    /// wrong whenever this is set — and the honest phrasing ("the top
    /// N") is fine to use either way.
    pub truncated: bool,
}

/// A retrieval reduced to **order**: the ranked ids and nothing else.
///
/// The second composition form: membership
/// stays with the Query side (exact, countable, no ceiling) and only the
/// *sequence* comes from Retrieval. So this carries no cards, no paging
/// and no counts of the library — a caller pairs it with a page it
/// already holds and uses the rank to sort what is on screen.
///
/// Not a set. `ids` is best-first and stops at the candidate ceiling, so
/// an id missing from it means "not in the top N we looked at", never
/// "does not match". Treating it as a membership answer is the mistake
/// [`RetrievedPageDto`]'s missing `total` exists to prevent.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RetrievedIdsDto {
    /// Candidate ids in rank order, best first. Already narrowed by the
    /// request's filter, so every entry is one the caller may show.
    pub ids: Vec<String>,
    /// How many candidates the retriever looked at before filtering.
    /// Always ≥ `ids.len()`, and the honest denominator for any
    /// "ranked" phrasing on screen.
    pub candidates_considered: u32,
    /// The shortlist filled to its ceiling, so assets beyond it were
    /// never ranked and cannot appear in `ids` — the order is a hint
    /// about the top of the library, not about all of it.
    pub truncated: bool,
}

/// A random handful drawn from the set a filter describes — the answer to
/// [`RandomAssetsQuery`](crate::query::RandomAssetsQuery).
///
/// **Order is the shuffle itself and carries no meaning.** Nothing here
/// is reproducible: the same request answers with different picks, in a
/// different sequence, every time (Retrieval promises
/// no determinism, and this is a Retrieval-shaped read that happens to be
/// implemented in SQL). Freezing this state into a saved query or a URL
/// would freeze something that cannot be restored.
///
/// It is also **not** an enumeration: `items` stops at the requested `k`
/// however large the set is, so it can never be paged through to reach
/// the rest. The set's size is answerable, though, and `set_total` says
/// it exactly — the filter is a SQL predicate, so this is the Query
/// side's kind of count, not the shortlist-bounded number
/// [`RetrievedPageDto`] deliberately withholds.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SampledPageDto {
    /// The picks, in the order the shuffle produced them.
    pub items: Vec<AssetCardDto>,
    /// How many picks came back — `items.len()`, stated so a caller can
    /// phrase "N picks from M" without counting the array. Smaller than
    /// the requested `k` only when the whole set is smaller.
    pub picked: u32,
    /// Exact size of the set the picks were drawn from: a `COUNT(*)` over
    /// the same predicate, with no ceiling and no shortlist behind it.
    pub set_total: u64,
}

/// Index-only wire form for 6-figure grids.
///
/// Same sort / filter surface as `AssetCardDto` minus the heavy
/// per-row fields (cover, source_locator, snippet, score). The
/// frontend fetches this eagerly (~150 bytes/row × 100 k = ~15 MB IPC)
/// to drive the full-page virtualised scroll, then hydrates the visible
/// viewport slice through `cards_by_ids`.
///
/// `file_size_bytes` was named on that dropped list until the metric
/// axes landed, and `duration_ms` had never been on the row at all.
/// Both are here now, and the exception is not "these two happened to
/// be cheap": it is the rule this type states one field down — **an
/// axis the index cannot express is an axis the grid cannot offer**.
/// Size and length are axes (`SortTarget::FileSize` /
/// `SortTarget::Duration`) and the grid sorts these rows, so withholding
/// them made the picker withhold the axes. They cost two nullable
/// integers, the weight class of the timestamps already here — the
/// dropped three are unbounded text and a per-query score.
///
/// Cover text and source locator stay dropped for the same rule read the
/// other way: neither is an axis this projection can serve. Cover-text
/// sort is deliberately unsupported on index rows (the full cover only
/// exists on the hydrated card) and the locator is a render input, not
/// an ordering key.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetIndexEntryDto {
    /// Asset id (UUID text).
    pub id: String,
    /// Persona bucket id.
    pub persona_id: String,
    /// Semantic classification slug (`None` = unclassified).
    pub modality: Option<String>,
    /// Occurrence timestamp (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Labels — feeds the tag-axis sort and the content-flag chip
    /// filter without a hydration round-trip.
    pub labels: Vec<String>,
    /// Groups the asset is filed into (m:n `asset_bucket`).
    pub group_ids: Vec<String>,
    /// Slot inside the primary group — see
    /// [`AssetCardDto::primary_group_position`]. Carried on the light row
    /// because the grid sorts on index rows and hydrates only the
    /// viewport; leaving it off would make `Group` + `ordered` collapse
    /// on exactly the pages the index path exists for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_group_position: Option<i64>,
    /// Ingest timestamp (unix epoch ms) — feeds the "Added" sort.
    pub created_at_ms: i64,
    /// Last-modification timestamp (unix epoch ms) — the
    /// differential-sync cursor, fed straight back as
    /// [`ListAssetsQuery::updated_from_ms`](crate::query::ListAssetsQuery::updated_from_ms),
    /// and the key the `updated_at` sort axis orders on. See
    /// [`AssetCardDto::updated_at_ms`] for the inclusivity caveat and
    /// for which writes actually move the stamp.
    ///
    /// On the light row because the client sorts and paginates over
    /// index rows: an axis the index cannot express is an axis the grid
    /// cannot offer.
    pub updated_at_ms: i64,
    /// Playback length in ms — the `Duration` axis's key. `None` for
    /// material that does not play (a still image) and for material
    /// nothing has measured; the two are the same statement here, and
    /// both tail the sort in either direction rather than reading as
    /// zero seconds. See [`AssetCardDto::duration_ms`].
    ///
    /// On the light row for the reason `updated_at_ms` states one field
    /// up, and the type doc says why that reason overrides the payload
    /// budget for this field and not for cover / locator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Original artefact size — the `FileSize` axis's key, carried for
    /// the same reason and with the same three-valued reading as
    /// `duration_ms`. Also the value the card shows once hydrated, so
    /// the light row and its hydrated self now agree on it instead of
    /// the light row claiming `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size_bytes: Option<u64>,
    /// Total pixel count — the `Pixels` axis's key, carried for the same
    /// reason and with the same three-valued reading as the two fields
    /// above. See [`AssetCardDto::pixel_count`] for why this is the
    /// product rather than the two dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_count: Option<u64>,
    /// Structural role slug (`"item"` / `"collection"`) — the two
    /// render through different card paths, so the light row has to
    /// carry it or a container paints blank until hydration.
    #[serde(default = "default_role")]
    pub role: String,
}

/// Paginated index page (sibling of `AssetPageDto`).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetIndexPageDto {
    /// Index entries in the current page.
    pub items: Vec<AssetIndexEntryDto>,
    /// Echo of the requested offset.
    pub offset: u64,
    /// Echo of the requested limit.
    pub limit: u64,
    /// Total row count after the filter.
    pub total: Option<u64>,
}

/// Full asset payload used on the detail view.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetDto {
    /// Asset id.
    pub id: String,
    /// Persona bucket id.
    pub persona_id: String,
    /// Ingest source slug.
    pub source_kind: String,
    /// Where the original artefact is, rendered for display — the path
    /// of a file, the container of a record, the address of a remote,
    /// the name of a logical locator.
    ///
    /// Not the stored form. Nothing reads this value back, so it stays
    /// the spelling a person recognises rather than following the
    /// storage encoding wherever that goes.
    pub locator: String,
    /// Original artefact size.
    pub file_size_bytes: Option<u64>,
    /// Originating platform (human-readable name).
    pub platform: Option<String>,
    /// Format fact of the primary material (`image/png`,
    /// `text/plain`, …; `None` = unknown). Asset-model v4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// Which inline player these bytes call for — `image` / `video` /
    /// `audio` / `none`. Same reason as the card's: the detail view
    /// was deriving it from `mime` with its own `startsWith` chain.
    #[serde(default = "default_media")]
    pub media: String,
    /// Fingerprint of the primary material's bytes — the value the
    /// duplicate report groups on (`asterism_core::domain::content_hash`).
    ///
    /// Three states, and telling them apart is the whole use of the
    /// field:
    ///
    /// - `sha256:<hex>` — a real digest: these bytes were read, and
    ///   this is what they hash to.
    /// - `unhashable:no-bytes` — the marker for a material that can
    ///   never have a digest, because there are no bytes to read: a
    ///   record addressed *inside* a container file, or a locator that
    ///   is not on this disk. It reaches the wire **verbatim** rather
    ///   than folded into absence — "we looked and there is nothing to
    ///   read" is an answer, "nobody has looked yet" is not, and a
    ///   consumer waiting for a hash that is never coming is the bug
    ///   collapsing them produces.
    /// - Absent — not computed yet (hashing runs after ingest, in a
    ///   job), **or** the entity was not hydrated on the path that
    ///   built this payload. The same two-way silence
    ///   [`AssetDto::mime`] carries: `None` is "unknown", never "not
    ///   applicable".
    ///
    /// No state here is evidence that the asset is *not* a duplicate.
    /// A digest says which rows match; absence says nobody answered the
    /// question — reading it as uniqueness is the one wrong inference
    /// available. (The predicate this used to name,
    /// `is_hashable_locator`, is gone: the question is now which shape
    /// the locator is, and "not a local file" means "no fingerprint for
    /// this one" — never "known unique".)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Semantic classification slug (`None` = unclassified).
    pub modality: Option<String>,
    /// Free-form labels.
    pub labels: Vec<String>,
    /// Occurrence timestamp (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Composition membership — the hyphenated UUID of the composite
    /// Asset this row belongs to (session-model v2). `None` = top-level
    /// (not inside any composite). Replaces the old `session_id` field:
    /// membership is now modality-agnostic and expressed through the
    /// composite Asset, not a Dialog-only session key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    /// User-authored name — the primary surface for a composite Asset
    /// (a Session's title). `None` for ordinary assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Constellation-edge grouping key for non-Dialog modalities
    /// (tape / journal / image / future slot). Set by importers that
    /// used to write `session_id` for the same purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// Structural role slug — `"item"` or `"collection"`. The detail
    /// view needs it to decide what "the content of this asset" means:
    /// an item's content is its body, a container's is its members.
    #[serde(default = "default_role")]
    pub role: String,
    /// Card cover text.
    pub cover: Option<String>,
    /// Raw keywords extracted by the auto-tag pipeline.
    pub keywords: Vec<String>,
    /// Register / tone annotation.
    pub register_note: Option<String>,
    /// Visibility flag: `false` = open, `true` = restricted.
    pub visibility_restricted: bool,
    /// Sharing list when the asset is restricted; empty when open.
    pub visibility_sharing: Vec<String>,
    /// Duration for time-bounded assets.
    pub duration_ms: Option<u64>,
    /// Pixel width of the **stored bytes** — the coded dimension, with no
    /// orientation applied, so this is not necessarily the width a viewer
    /// shows. An image tagged EXIF Orientation 5-8 displays transposed
    /// and still reports its landscape pair here; a reader that wants
    /// display dimensions combines this with `extra.orientation`. See
    /// [`AddAssetCommand::width_px`](crate::command::AddAssetCommand::width_px)
    /// for the whole rule.
    ///
    /// `None` = nobody measured it, never `0`.
    ///
    /// Detail only. The card and index projections deliberately do not
    /// carry it: they carry `duration_ms` because a sort axis reads it,
    /// and there is no dimensions axis yet.
    ///
    /// Read straight through: the pair-or-nothing rule is asserted on the
    /// write side, so a row that some other writer left half-filled is
    /// reported here as it stands rather than being repaired or hidden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
    /// Pixel height of the stored bytes, on the terms
    /// [`AssetDto::width_px`] states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
    /// Star rating 0-5 (`None` = unrated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    /// Dominant colour palette — up to 5 lowercase `"#rrggbb"`
    /// strings. `None` for non-image assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
    /// Source-specific extension bag (JSON string).
    pub extra_json: Option<String>,
    /// Ingest timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Last modification timestamp (unix epoch ms).
    pub updated_at_ms: i64,
    /// Attribution kind — `"owner"` (no subject) or `"subject"`
    /// (subject present in [`AssetDto::author_subject`]). Absent means
    /// **unrecorded**: nobody asserted an author, which is deliberately
    /// not the same as "the owner wrote it".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// Subject token this asset is attributed to — the same token
    /// `viewer_subject` and the sharing list carry. Present only
    /// alongside `author_kind = "subject"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// Agent that performed the operation (`claude-code`, `codex`,
    /// `asterism-ui`, …) — an open slug, absent when unrecorded.
    /// Caller-asserted, like `viewer_subject`; authentication is a
    /// hosted-time transport concern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
    /// Channel the attribution above arrived through —
    /// `"owner-surface"` (the owner's own app), `"asserted"` (an HTTP /
    /// MCP caller stating its own), or `"authenticated"`. Absent when
    /// nothing is recorded, and on rows written before the channel was
    /// tracked.
    ///
    /// **Read direction only.** The value is derived from the entry
    /// point that served the write; no command carries it inward,
    /// because a caller that could name its own channel could name any
    /// of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributed_via: Option<String>,
    /// Duplicate strategy declared when this asset was registered —
    /// `"ask"` / `"fold"` / `"separate"`, absent when nobody declared
    /// one (see [`crate::command::OnDuplicate`]). Projected as the slug
    /// rather than the enum, like `role` and `author_kind`: every
    /// closed set on this DTO reaches the wire as its stored token.
    ///
    /// On the wire because it is otherwise write-only: a caller that
    /// declared `fold` has no other way to see whether the server took
    /// it, and nothing on the row can be used to work it out. The two
    /// columns beside it on the fold axis ([`folded_into`](Self::folded_into)
    /// / [`fold_policy`](Self::fold_policy)) are here for the same
    /// reason and are described on their own fields.
    ///
    /// The grid projections do not carry it
    /// ([`AssetCardDto`] / [`AssetIndexEntryDto`]): it sorts nothing,
    /// filters nothing and is not drawn, so it would be a column paid
    /// for on every card in a page of 200 000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_duplicate: Option<String>,
    /// The keeper this row was folded into — absent on a live row,
    /// present on a headstone.
    ///
    /// **The one field that says a read is answering about a row that is
    /// no longer part of the library.** Reads by id deliberately return
    /// a headstone (that is what makes an old reference resolvable at
    /// all — see
    /// `asterism_core::domain::asset::Asset::folded_into`), so without
    /// this field `GET /assets/{id}` and the `asset_get` tool answer a
    /// stale id with a complete-looking record and no hint that
    /// somebody ruled it a duplicate. An agent that picked the id out
    /// of a note written last month has no other way to find that out.
    ///
    /// A reader that wants the row this one became follows it: the
    /// value is the id to fetch, and the chain is at most one hop in
    /// practice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folded_into: Option<String>,
    /// Whether this row may be folded at all — `"auto"` (nobody has
    /// ruled) or `"keep"` (a person decided it is its own thing).
    /// Projected as the stored slug, like `role` and `on_duplicate`.
    ///
    /// Not an absence and never absent: every row starts at `"auto"`,
    /// which is a real answer and not a missing one. It is on the wire
    /// because `"keep"` is a **human ruling** — "these two are different
    /// things" — and a caller looking at two rows that hold identical
    /// bytes is owed the fact that somebody has already looked at them
    /// and said so. Without it the only readable trace of that ruling is
    /// a closed row on a queue nothing outside the duplicates panel
    /// reads.
    #[serde(default = "default_fold_policy")]
    pub fold_policy: String,
}

/// `fold_policy` for a payload written before the field existed. `auto`
/// is the column's own default — "nobody has ruled" — so a record that
/// never carried the field reads back as one nobody has ruled on.
fn default_fold_policy() -> String {
    "auto".to_string()
}

/// A single channel tag.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TagDto {
    /// Tag id.
    pub id: String,
    /// Channel name.
    pub name: String,
    /// Classification axis slug (`period` / `counterpart` / `mood` /
    /// `scene` / `platform` / `modality`; `None` = unclassified).
    pub axis: Option<String>,
}

/// One `session` row — the Dialog-modality 1st-class entity.
///
/// Wire form of `asterism_core::domain::session::Session`. `title` /
/// `note` / `cover_hint` are the user-editable metadata (all
/// `Option<String>`, default absent); `external_key` is the raw
/// importer-supplied identifier the find-or-create path resolves
/// against; the aggregates (`started_at_ms` / `ended_at_ms` /
/// `message_count`) are derived from the participating dialogue
/// assets and refreshed by P1b's `SessionRebuild` job.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SessionDto {
    /// Session id — hyphenated UUID v7 (also the value written into
    /// each participating asset's `session_id` column after V26).
    pub id: String,
    /// Owning persona.
    pub persona_id: String,
    /// Importer-visible identifier the find-or-create path resolves
    /// against (e.g. Claude Code session UUID, JSONL file stem).
    pub external_key: String,
    /// User-facing title. `None` = untitled.
    pub title: Option<String>,
    /// Free-form user note about the Session.
    pub note: Option<String>,
    /// Cover hint (typically the first message excerpt).
    pub cover_hint: Option<String>,
    /// Earliest occurrence time among participating assets (unix
    /// epoch ms).
    pub started_at_ms: i64,
    /// Latest occurrence time among participating assets (unix epoch
    /// ms).
    pub ended_at_ms: i64,
    /// Count of participating dialogue assets.
    pub message_count: u64,
    /// Creation time (unix epoch ms).
    pub created_at_ms: i64,
    /// Last-updated time (unix epoch ms).
    pub updated_at_ms: i64,
}

/// Paginated response for the Sessions view. Items are
/// [`SessionDto`] rows sourced from the `session` table
/// (Dialog-modality 1st-class entity); the SessionsView
/// tile grid renders straight from these.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SessionPageDto {
    /// Items in the requested page.
    pub items: Vec<SessionDto>,
    /// Total number of Sessions belonging to the queried persona
    /// (independent of the page window).
    pub total: Option<u64>,
    /// Requested offset.
    pub offset: u64,
    /// Requested limit.
    pub limit: u64,
}

/// A user-curated Group (bucket) — the hand-picked twin of a Tag.
///
/// Groups are persona-scoped: `(persona_id, name)` is unique on the
/// storage side. See `asterism_core::domain::group::Group` for the
/// domain rationale ("hand-picked" vs Tag's "organically labelled").
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct GroupDto {
    /// Group id.
    pub id: String,
    /// Owner persona id.
    pub persona_id: String,
    /// Human-facing name (unique per persona).
    pub name: String,
    /// Optional freeform description.
    pub description: Option<String>,
    /// Sidebar dir this group is filed under (`None` = root level).
    pub dir_id: Option<String>,
    /// `"manual"` (hand-curated) or `"query"` (rule-defined
    /// membership). Drives the sidebar icon,
    /// drop-target gating, and reorder availability.
    pub kind: String,
    /// The `query_json` v1 rule when `kind == "query"`; `None` for
    /// manual groups. Feeds the "expand query into the filter bar"
    /// affordance.
    pub query_json: Option<String>,
    /// Birth record: the Snapshot this Group was promoted from,
    /// `None` for directly-created groups. Feeds the "promoted from"
    /// provenance entry that opens the Snapshot view.
    pub origin_snapshot_id: Option<String>,
    /// When the last query-group refresh ran (unix epoch ms); `None`
    /// for manual groups and never-refreshed query groups (W4-b
    /// failure signal).
    pub last_refresh_at_ms: Option<i64>,
    /// `"ok"` / `"failed"` outcome of the last refresh; `None` = never
    /// refreshed. Drives the sidebar staleness chip.
    pub last_refresh_status: Option<String>,
    /// Failure text of the last refresh (`None` on success) — surfaced
    /// as the chip tooltip.
    pub last_refresh_error: Option<String>,
    /// Creation time (unix epoch ms).
    pub created_at_ms: i64,
    /// Last-update time (unix epoch ms).
    pub updated_at_ms: i64,
}

/// A sidebar organisation folder. Dirs contain dirs and groups —
/// never assets — and are a pure navigation axis: selecting a dir is
/// expanded client-side into the group ids beneath it, so the asset
/// filter wire shape (`group_ids` OR) is unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DirDto {
    /// Dir id.
    pub id: String,
    /// Owner persona id.
    pub persona_id: String,
    /// Parent dir id (`None` = root level).
    pub parent_id: Option<String>,
    /// Human-facing name (unique among siblings).
    pub name: String,
    /// Manual sort hint among siblings (ties broken by name).
    pub position: i64,
    /// Creation time (unix epoch ms).
    pub created_at_ms: i64,
    /// Last-update time (unix epoch ms).
    pub updated_at_ms: i64,
}

/// One Group-in-Group connection (Are.na channel-in-channel). The
/// client receives the full persona-scoped list and assembles the
/// nesting graph (child bands, descendant expansion) itself.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct GroupLinkDto {
    /// The containing group.
    pub parent_group_id: String,
    /// The contained group.
    pub child_group_id: String,
    /// Hand-arranged order among the parent's child groups.
    pub position: i64,
}

/// A group paired with the number of distinct assets attached, used
/// to render the sidebar Groups section and chip count badges.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct GroupSummaryDto {
    /// The group itself.
    pub group: GroupDto,
    /// Distinct-asset count.
    pub asset_count: u64,
}

/// A tag paired with the number of assets currently attached to it,
/// used to render the sidebar Tags section and chip count badges.
///
/// Aggregated by `list_tag_counts` — persona-scoped when the caller
/// supplies a persona id, global otherwise. Tags with zero assets in
/// the query scope are omitted so the sidebar does not list dead
/// channels.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TagCountDto {
    /// The tag itself.
    pub tag: TagDto,
    /// Distinct-asset count within the query scope.
    pub asset_count: u64,
}

/// A single constellation edge (payload for the hover-burst).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EdgeDto {
    /// Edge id.
    pub id: String,
    /// Asset the burst starts from.
    pub from_asset_id: String,
    /// Asset the burst lands on.
    pub to_asset_id: String,
    /// Edge-kind slug (`time_proximity` / `keyword_overlap` /
    /// `co_presence` / `cadence` / `reference`).
    pub kind: String,
    /// Human-readable relationship label (for example `same-session`).
    pub label: Option<String>,
    /// Weight used to sort the top-N burst list.
    pub weight: Option<f64>,
}

/// One hover-burst item — an edge paired with the card it lands on.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ConstellationItemDto {
    /// Relationship metadata (kind / label / weight).
    pub edge: EdgeDto,
    /// Burst target card (already filtered to what the viewer can see).
    pub card: AssetCardDto,
    /// Which side of the underlying edge the queried asset sat on:
    /// `"outgoing"` (queried asset was `edge.from`), `"incoming"`
    /// (queried asset was `edge.to`), or `"both"` (two symmetric
    /// edges were collapsed into a single confirmed link). The UI
    /// uses this hint for styling — the burst destination itself is
    /// always [`card`](Self::card), no matter which side we came in
    /// from.
    pub direction: String,
}

/// Composite response for `GET /asterism/assets/{id}/provenance` —
/// the provenance ("derived from") mini-graph the detail view surfaces.
///
/// Semantics (aligned with the write path in
/// `dispatch_service::reify_derived`, which writes
/// `ConstellationEdge { from = derived_asset, to = parent, kind =
/// DerivedFrom }`):
///
/// - `ancestors` — cards this asset was *derived from* (Selection
///   inputs that seeded the dispatch producing this asset). Empty
///   for imported assets that never went through a dispatch.
/// - `descendants` — cards *derived from* this asset (this asset
///   sat as a Selection input for some later dispatch). Empty for
///   leaves that were never re-dispatched.
///
/// MVP scope: 1 hop in each direction, capped at `limit`. Deeper
/// lineage traversal is a follow-up (each side can be re-fetched
/// against the picked card to walk further).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ProvenanceViewDto {
    /// The asset the view is rooted at (echo of the URL path).
    pub asset_id: String,
    /// Cards this asset was derived from (1 hop up).
    pub ancestors: Vec<AssetCardDto>,
    /// Cards derived from this asset (1 hop down).
    pub descendants: Vec<AssetCardDto>,
}

/// One asset in a multi-hop lineage walk.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LineageNodeDto {
    /// The asset itself.
    pub card: AssetCardDto,
    /// Hops from the queried asset. `0` is the asset itself,
    /// positive numbers are ancestors (what it came from), negative
    /// numbers are descendants (what came out of it).
    ///
    /// Signed rather than split into two lists because a chain is one
    /// line through the queried asset, and the sign is what says which
    /// way each node sits on it.
    pub depth: i32,
    /// The dispatch that produced this asset, when it was produced by
    /// one (`extra._dispatch.dispatch_id`). This is the hop's identity
    /// — the sequence of these along a chain is what says which route
    /// an artefact travelled.
    pub dispatch_id: Option<String>,
}

/// One `derived_from` link in a lineage walk. Direction is
/// child → parent, matching the stored edge.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LineageEdgeDto {
    /// The derived asset.
    pub from_asset_id: String,
    /// The asset it was derived from.
    pub to_asset_id: String,
    /// Edge label (`dispatch-<slug>` for a reified output,
    /// `correlated-ingest` for a declared one) — how the link came to
    /// be recorded.
    pub label: Option<String>,
}

/// Multi-hop `derived_from` lineage around one asset.
///
/// The 1-hop [`ProvenanceViewDto`] answers "what is next to this";
/// this answers "what route did this take", which is the question a
/// chain that left the machine and came back poses.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LineageViewDto {
    /// The asset the walk started from (echo of the URL path).
    pub asset_id: String,
    /// Every asset reached, including the starting one (`depth = 0`).
    pub nodes: Vec<LineageNodeDto>,
    /// Every link between the returned nodes.
    pub edges: Vec<LineageEdgeDto>,
    /// Assets with no further ancestors — where the chain begins.
    /// More than one is normal: an export of N assets gives its
    /// output N parents.
    pub roots: Vec<String>,
    /// Dispatch ids seen along the ancestor side, ordered by depth.
    /// The backbone of the chain — "this went out through these
    /// exports, in this order".
    pub dispatch_ids: Vec<String>,
    /// `true` when the walk hit its depth or node budget and stopped
    /// early. A picture that silently omits half a chain is worse
    /// than one that says it did.
    pub truncated: bool,
}

/// Where a video's preview rendition stands
/// (`GET /asterism/assets/{id}/video-preview`).
///
/// Some formats the embedded webview cannot display (VP9 WebM,
/// Matroska — measured); for those the player uses a
/// transcoded H.264 MP4 rendition cached beside the profile database.
/// This DTO is how the pane finds out whether one exists yet.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct VideoPreviewDto {
    /// `ready` (rendition exists — `path` is set) / `pending` (a
    /// transcode is queued or running — poll again) / `not_needed`
    /// (the original plays natively — use its own locator) /
    /// `failed` (the last transcode died — `detail` says why).
    pub status: String,
    /// Absolute path of the rendition file when `status == "ready"`.
    pub path: Option<String>,
    /// Failure reason when `status == "failed"`.
    pub detail: Option<String>,
}

/// Composite response for the detail view (asset + tags + edges).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetDetailDto {
    /// The asset itself.
    pub asset: AssetDto,
    /// Attached tags.
    pub tags: Vec<TagDto>,
    /// Top constellation edges (weight-descending).
    pub edges: Vec<EdgeDto>,
}

/// Full source text of one asset, resolved from the original
/// artefact on disk (the DB itself only stores the 200-char cover
/// snippet). `text = None` means the source could not be read — the
/// UI falls back to the cover.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetTextDto {
    /// Asset id.
    pub asset_id: String,
    /// Full body text (`None` on read/extract failure).
    pub text: Option<String>,
}

/// Job status payload.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct JobDto {
    /// Job id.
    pub id: String,
    /// Job-kind slug (`asset_add` / `auto_tag` / `cover_gen` /
    /// `edge_rebuild` / `persona_import` / `index_rebuild`).
    pub kind: String,
    /// Lifecycle-state slug (`pending` / `running` / `completed` /
    /// `failed` / `cancelled`).
    pub state: String,
    /// Items processed so far.
    pub progress_current: u64,
    /// Total item count (`None` = indeterminate).
    pub progress_total: Option<u64>,
    /// Latest progress message.
    pub progress_message: Option<String>,
    /// Enqueue timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Latest state-change timestamp (unix epoch ms).
    pub updated_at_ms: i64,
}

/// Per-kind slice of the background-jobs table (wire form of the
/// infra-side snapshot; the UI banner consumes it via `jobs_stats`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, SchemaBridge)]
pub struct JobKindSnapshotDto {
    /// Every persisted row for this kind.
    pub total: u64,
    /// Rows in `Done` state.
    pub done: u64,
    /// Rows in `Pending` state (queued, not picked up yet).
    pub pending: u64,
    /// Rows in `Running` state (actively worked on).
    pub running: u64,
    /// Rows in `Failed` state.
    pub failed: u64,
}

/// One telemetry event row (wire form of the local `event_log`
/// table). Newest-first listings feed the UI and agent-side usage
/// aggregation over the HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EventDto {
    /// Event id (UUID hyphenated).
    pub id: String,
    /// Event kind (open slug, e.g. `persona_switch`).
    pub kind: String,
    /// When the event was recorded (unix epoch ms, server-stamped).
    pub occurred_at_ms: i64,
    /// Persona in scope when the event fired (`None` = no persona
    /// context).
    pub persona_id: Option<String>,
    /// User-perceived duration of the measured interaction, if any.
    pub duration_ms: Option<i64>,
    /// Extension bag serialised as JSON (opaque to the schema).
    pub payload_json: Option<String>,
}

/// The envelope every observation carries, whatever stream it is in.
///
/// Repeated by value in each stream's DTO rather than nested or
/// flattened: these are read with `curl` by whoever is investigating
/// the application, and a flat object is what that reader wants. The
/// six fields are the same six in every stream table, so the repetition
/// is a transcription, not a second definition.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ObservationDto {
    /// Which stream this row came from (`action` / `job` / `diag` /
    /// `perf`). Present only on the union listing, where it is the
    /// thing that tells the four apart.
    pub stream: String,
    /// Record id (UUID hyphenated).
    pub id: String,
    /// When it was recorded (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Data profile the record was produced against.
    pub env: String,
    /// Namespaced event name (`job.cover_gen.failed`), which identifies
    /// the record's type and therefore the shape of `attrs_json`.
    pub event: String,
    /// Event-specific detail as a JSON object, `None` when the record
    /// carried none.
    pub attrs_json: Option<String>,
    /// Ties records emitted from one user action together, when the
    /// writer supplied it.
    pub correlation_id: Option<String>,
}

/// One persisted diagnostic (`GET /asterism/diag`).
///
/// The read side of the `DiagLog` stream. Separate from [`EventDto`]
/// for the same reason the tables are separate: an event measures an
/// interaction, a diagnostic explains a decision the application made
/// on its own.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DiagDto {
    /// Record id (UUID hyphenated).
    pub id: String,
    /// When it was recorded (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Data profile the record was produced against.
    pub env: String,
    /// Namespaced event name (`diag.search.commit_failed`).
    pub event: String,
    /// `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE`.
    pub level: String,
    /// Emitting module path (`asterism_core::application::…`).
    pub target: String,
    /// Human-readable text.
    pub message: String,
    /// Structured fields as a JSON object, `None` when the record
    /// carried only a message.
    pub attrs_json: Option<String>,
    /// Correlation id, when the writer supplied one.
    pub correlation_id: Option<String>,
}

/// One persisted timing (`GET /asterism/perf`).
///
/// The read side of the `PerfLog` stream. Written in development only,
/// and kept for days rather than months: its value is in the aggregate
/// over a session, not in any individual row.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PerfDto {
    /// Record id (UUID hyphenated).
    pub id: String,
    /// When it was recorded (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Data profile the record was produced against.
    pub env: String,
    /// Namespaced event name (`perf.list_index`).
    pub event: String,
    /// Operation being timed — the axis every perf question groups by.
    pub op: String,
    /// How long it took.
    pub duration_ms: i64,
    /// The rest of the breakdown as a JSON object.
    pub attrs_json: Option<String>,
    /// Correlation id, when the writer supplied one.
    pub correlation_id: Option<String>,
}

/// One job run (`GET /asterism/jobs/log`).
///
/// The read side of the `JobLog` stream: one row per run, written at
/// completion. The queue's own table keeps state rather than history,
/// so this is the only place that answers "how long did this take" and
/// "how often does it fail".
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct JobLogDto {
    /// Record id (UUID hyphenated).
    pub id: String,
    /// When the run finished (unix epoch ms).
    pub occurred_at_ms: i64,
    /// Data profile the record was produced against.
    pub env: String,
    /// Namespaced event name (`job.cover_gen.failed`).
    pub event: String,
    /// Queue task id. Always present — a run is a run *of* something,
    /// so the writer defaults it rather than omitting it.
    pub task_id: String,
    /// Job-kind slug (`cover_gen`, `auto_tag`, …).
    pub job_kind: String,
    /// `completed` / `failed` / `skipped` / `panicked`.
    pub outcome: String,
    /// Which claim of this task the run was. Counts claims, not
    /// retries — above 1 means a previous claim was reclaimed.
    pub attempt: i64,
    /// How long the run took, absent when it never reached its end.
    pub duration_ms: Option<i64>,
    /// Run detail as a JSON object.
    pub attrs_json: Option<String>,
    /// Correlation id, when the writer supplied one.
    pub correlation_id: Option<String>,
}

/// Snapshot of the background-jobs table used by the UI progress
/// banner (status roll-up + per-kind breakdown).
#[derive(Debug, Clone, Default, Serialize, Deserialize, SchemaBridge)]
pub struct JobsSnapshotDto {
    /// Every persisted row, regardless of status.
    pub total: u64,
    /// Rows in `Done` state.
    pub done: u64,
    /// Rows in `Pending` state (queued, not picked up yet).
    pub pending: u64,
    /// Rows in `Running` state (actively worked on).
    pub running: u64,
    /// Rows in `Failed` state (retry / dead-letter path).
    pub failed: u64,
    /// Per-kind breakdown so the banner can show
    /// `cover_gen 512/512, edge_rebuild 200/500 pending 300`.
    pub by_kind: std::collections::BTreeMap<String, JobKindSnapshotDto>,
}

/// One exporter invocation against a frozen [`SnapshotDto`].
///
/// The wire form mirrors `asterism_core::domain::forge::dispatch::DispatchJob`:
/// `state` is the slug (`pending` / `running` / `done` / `failed` /
/// `cancelled`); `output_asset_ids` is populated atomically with the
/// transition to `done`. `state_message` carries the failure /
/// cancellation reason for terminal error states, `progress_*` fields
/// carry the running-state hint.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DispatchDto {
    /// Dispatch id (UUID hyphenated).
    pub id: String,
    /// Snapshot whose frozen asset_ids seeded this dispatch.
    pub snapshot_id: String,
    /// Persona bucket.
    pub persona_id: String,
    /// Pursuit this round is filed under (#29). `None` only on rows
    /// that predate the stamp's backfill invariant; moved only by the
    /// restamp verb, never by a state save.
    pub pursuit_id: Option<String>,
    /// Exporter slug (`comfy` / `gemini` / `vdsl` / `alc-sd-bake`).
    pub exporter_slug: String,
    /// Action string handed to the exporter (`img2img` / `txt2img` /
    /// `lora_bake` / …).
    pub action: String,
    /// Exporter-specific parameters (opaque JSON string).
    pub params_json: String,
    /// Lifecycle-state slug (`pending` / `running` / `done` /
    /// `failed` / `cancelled`).
    pub state: String,
    /// Human-readable state annotation: failure reason for `failed`,
    /// cancellation reason for `cancelled`, latest progress message
    /// for `running`, `None` otherwise.
    pub state_message: Option<String>,
    /// Running-state progress hint: discrete step count.
    pub progress_current: Option<u64>,
    /// Running-state progress hint: expected total.
    pub progress_total: Option<u64>,
    /// Reified derived Asset ids. Empty until the state is `done`.
    pub output_asset_ids: Vec<String>,
    /// Creation timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Last-updated timestamp (unix epoch ms).
    pub updated_at_ms: i64,
    /// Wall-clock time the job reached a terminal state (`None`
    /// while still pending / running).
    pub completed_at_ms: Option<i64>,
    /// Group frozen into the snapshot at dispatch time (`None` for a
    /// direct dispatch of a volatile selection) — P4 "then vs now"
    /// provenance.
    pub source_group_id: Option<String>,
    /// Query rule frozen alongside a query-group dispatch (`None`
    /// otherwise) — the freeze's reproduction material.
    pub source_query_json: Option<String>,
    /// Agent that requested the dispatch (`claude-code`, `codex`,
    /// `asterism-ui`, …) — the same open slug
    /// [`AssetDto::operator_ai`] carries, stamped onto the reified
    /// outputs when the job completes. Absent means **unrecorded**;
    /// caller-asserted, never authenticated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// An immutable, content-addressed freeze of an ordered asset set —
/// the git-tree analogue behind every dispatch and promote. Opened from
/// its referencing
/// event (a dispatch-history row or a Group's "promoted from" chip),
/// never listed or managed on its own.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SnapshotDto {
    /// Snapshot id (UUID hyphenated).
    pub id: String,
    /// Owner persona id.
    pub persona_id: String,
    /// SHA-256 fingerprint of the ordered member ids — the dedupe key
    /// within a persona.
    pub content_hash: String,
    /// Frozen member ids in freeze order.
    pub asset_ids: Vec<String>,
    /// Creation time of the canonical row (unix epoch ms).
    pub created_at_ms: i64,
}

/// The minted unit of work (#29): one line of generation and curation
/// toward an intent. The row is thin and immutable — `standing` is
/// derived on read from the lifecycle events, never stored.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PursuitDto {
    /// Pursuit id (UUID hyphenated) — minted, never derived from
    /// content.
    pub id: String,
    /// Owner persona id.
    pub persona_id: String,
    /// Pursuit this one was spawned from (`None` for a root). Set at
    /// creation, immutable.
    pub parent_id: Option<String>,
    /// Short human label (`None` for an anonymous, implicitly minted
    /// pursuit — display names for those are synthesized, not stored).
    pub title: Option<String>,
    /// One short free-text slot.
    pub note: Option<String>,
    /// Live standing, derived from the latest lifecycle event:
    /// `open` / `closed_satisfied` / `closed_abandoned`.
    pub standing: String,
    /// Creation time (unix epoch ms).
    pub created_at_ms: i64,
}

/// One lifecycle fact about a pursuit (#29): a close or a reopen,
/// append-only. A repeat close is a new fact; standing re-derives.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PursuitEventDto {
    /// Event id (UUID hyphenated).
    pub id: String,
    /// Pursuit the fact is about.
    pub pursuit_id: String,
    /// `closed_satisfied` / `closed_abandoned` / `reopened`.
    pub kind: String,
    /// `closed_satisfied` only: the kept set frozen at close (`None`
    /// there means "concluded with nothing kept" — a defined state).
    pub snapshot_id: Option<String>,
    /// One short free-text slot.
    pub note: Option<String>,
    /// When the fact was recorded (unix epoch ms).
    pub created_at_ms: i64,
}

/// One pursuit, opened up (#29): the thin row plus everything the
/// record correlates to it — all of it derived at read time, none of
/// it stored on the pursuit.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PursuitViewDto {
    /// The pursuit itself, standing included.
    pub pursuit: PursuitDto,
    /// Its rounds — the dispatch jobs stamped with it, oldest first.
    pub rounds: Vec<DispatchDto>,
    /// Its returns — asset ids whose ingest note resolved to one of
    /// the rounds (the dispatch join) or to the pursuit directly (the
    /// claim lane), ingest order. What a round minted in-library is
    /// not here: those ride on the round's own `output_asset_ids`
    /// above — returns are what came back from outside. Ids rather
    /// than cards: the asset surfaces already know how to open an id,
    /// and this view answers membership, not display.
    pub returns: Vec<String>,
    /// The lifecycle facts, oldest first.
    pub events: Vec<PursuitEventDto>,
}

/// One thing an exporter produced, ready for the core to reify
/// into a new Asset.
///
/// Moved here from `asterism-dispatch-sdk` so the SDK does not
/// have to depend on `asterism-core` and the core does not have
/// to depend on the SDK — the type is genuinely shared boundary
/// data (SDK returns it, core consumes it, UI displays the
/// resulting Asset).
///
/// **No `SchemaBridge` derive** — the `extra` field is a
/// `serde_json::Value`, which schema-bridge does not know how to
/// render (see this crate's lib doc). The wire representation
/// still round-trips via serde; the frontend consumes results
/// through the reified `AssetDto` rather than this shape, so no TS
/// gen is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedDto {
    /// Primary modality slug for the new asset.
    pub modality: String,
    /// Location the exporter wrote the artefact to (filesystem
    /// path, URL, DB row reference).
    pub locator: String,
    /// When the artefact came into being (RFC 3339 in wire form).
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    /// Optional short cover text shown on the grid card.
    /// Truncated to [`DERIVED_COVER_MAX_CHARS`] by the core.
    pub cover_hint: Option<String>,
    /// Optional register-note (tone / status chip). Truncated to
    /// [`DERIVED_REGISTER_MAX_CHARS`] by the core.
    pub register_note: Option<String>,
    /// Free-form labels.
    pub labels: Vec<String>,
    /// Original artefact size on disk when cheaply known.
    pub file_size_bytes: Option<u64>,
    /// Duration for time-bounded artefacts (video / audio).
    pub duration_ms: Option<u64>,
    /// Exporter-specific extension bag. Serialised to
    /// `Asset::extra` — the exporter typically records what would
    /// otherwise be lost information (prompt, seed, workflow ref,
    /// sampler, checkpoint, sample images list, …).
    pub extra: serde_json::Value,
    /// Optional sub-batch id for streaming exporters that emit
    /// multiple harvests over the same handle (LoRA train that
    /// hands back periodic sample images plus a final
    /// safetensors). MVP exporters leave this `None`.
    pub batch_hint: Option<String>,
}

/// Maximum cover hint length in Unicode scalar values (matches
/// `asterism-importer-sdk::COVER_MAX_CHARS`).
pub const DERIVED_COVER_MAX_CHARS: usize = 200;
/// Maximum register-note preview length (matches
/// `asterism-importer-sdk::REGISTER_MAX_CHARS`).
pub const DERIVED_REGISTER_MAX_CHARS: usize = 80;

/// One comment attached to an Asset.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetCommentDto {
    /// Comment id (UUID hyphenated).
    pub id: String,
    /// Asset the comment is attached to.
    pub asset_id: String,
    /// `"user"` or `"persona"`.
    pub author_kind: String,
    /// Persona id when `author_kind = "persona"`; `None` otherwise.
    pub author_persona_id: Option<String>,
    /// Free-form body.
    pub body: String,
    /// Post timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Last edit timestamp (unix epoch ms); `None` for pristine
    /// posts.
    pub edited_at_ms: Option<i64>,
}

/// One mark placed into an Asset's material — the coordinate space its
/// content carries, rather than the asset as a catalogue entry.
///
/// The anchor arrives flattened: a `anchor_kind` slug plus the columns
/// that kind populates. The alternative — a tagged sub-object — would
/// put the wire shape at odds with both the storage row and the
/// `AssetCommentDto` beside it, for a payload that is two integers wide.
///
/// `body` / `author_kind` / `author_persona_id` / `created_at_ms` /
/// `edited_at_ms` deliberately carry the same names and types as on
/// [`AssetCommentDto`]: one note vocabulary, spelled once.
///
/// **The band the mark sits in is not on this shape.** Every mark
/// belongs to a [`MaterialLayerDto`] — an annotation band, resolved for
/// the caller when the mark is placed — but no surface asks which one:
/// a person clicking a timeline is answering "where", not "in which of
/// my passes over this file", and the marks read back through
/// `list_material_marks` are the asset's, not one band's. Adding
/// `layer_id` here would be putting a field on the wire (and into the
/// generated bindings, which are tracked) before anything reads it, and
/// a field nobody reads is one nobody notices going wrong. It arrives
/// when a surface offers a second annotation band to choose between.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MaterialMarkDto {
    /// Mark id (UUID hyphenated).
    pub id: String,
    /// Asset whose material the mark points into.
    pub asset_id: String,
    /// Which coordinate space the mark is anchored in. `"temporal"` is
    /// the only kind this build places.
    pub anchor_kind: String,
    /// Start of the anchor on the playback timeline, in milliseconds
    /// from the presentation origin.
    ///
    /// Required when `anchor_kind = "temporal"`. It is optional here
    /// because a second coordinate space (a rectangle on an image
    /// plane, say) leaves the column empty — the wire shape is kept the
    /// same shape as the storage row, where the same column is nullable
    /// for the same reason.
    pub start_ms: Option<i64>,
    /// Exclusive end of the anchor. `None` on a temporal mark names an
    /// instant rather than "to the end of the media".
    pub end_ms: Option<i64>,
    /// `"user"` or `"persona"`.
    pub author_kind: String,
    /// Persona id when `author_kind = "persona"`; `None` otherwise.
    pub author_persona_id: Option<String>,
    /// Free-form body.
    pub body: String,
    /// When the mark was placed (unix epoch ms).
    pub created_at_ms: i64,
    /// Last edit timestamp (unix epoch ms); `None` while untouched.
    pub edited_at_ms: Option<i64>,
}

/// One band of marks over an Asset's material — which reading of the
/// content this is, and who produced it.
///
/// `origin` is `"imported"` (read out of the material itself: the
/// chapter list the container declares), `"user"` (written by the person
/// running Asterism) or `"machine"` (derived by a job). It is the axis
/// that decides whether a band may be edited: the write verbs accept
/// `"user"` and refuse the other two, because an imported band is
/// replaced by reading the file again and a machine band by running its
/// job again, so a hand edit into either is lost at the next run.
///
/// `role` is `"structure"` (a reading of how the material is divided —
/// holds [`ChapterMarkDto`]) or `"annotation"` (notes fastened to
/// positions — holds [`MaterialMarkDto`]). The two hold different rows,
/// so a chapter verb aimed at an annotation band is refused rather than
/// silently writing something that band's readers cannot see.
///
/// **No display name.** A band is described by what it *is* —
/// `(origin, role)` — and a surface renders that pair. A stored caption
/// beside it would be a second answer to one question, and the caption
/// is the one that would drift.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MaterialLayerDto {
    /// Layer id (UUID hyphenated).
    pub id: String,
    /// Asset whose material the band is over.
    pub asset_id: String,
    /// Which of that asset's originals the band is over. `0` is the
    /// primary one — the axis `duration_ms` measures and the player
    /// reports positions on.
    pub material_ord: u32,
    /// `"imported"` / `"user"` / `"machine"`.
    pub origin: String,
    /// `"structure"` / `"annotation"`.
    pub role: String,
    /// Whether this is the band a surface shows, and the one a new mark
    /// lands in, when the caller names no other. At most one per
    /// `(asset_id, material_ord, role)`.
    pub is_default: bool,
    /// Display order within `(asset_id, material_ord, role)`.
    pub ord: u32,
}

/// One named section of a material — an entry in a chapter list.
///
/// Not a [`MaterialMarkDto`]. A mark is a note fastened to a position
/// ("look at this"); a chapter is a claim about how the material is
/// *divided* ("this section starts here"). Two differences follow from
/// that and are visible on this shape:
///
/// - **`start_ms` is not optional and there is no `anchor_kind`.** A
///   chapter is a section of a playback timeline by construction, where
///   a mark names the coordinate space it is anchored in and leaves the
///   columns of the other spaces empty.
/// - **`label` may be empty, and `end_ms` may be absent.** Both are
///   ordinary container output: MP4's `chpl` declares start times only
///   (a chapter ends where the next begins, which is a fact about other
///   rows), and plenty of files declare untitled sections. Refusing
///   either would mean an import drops a section the file really has or
///   invents a title for it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ChapterMarkDto {
    /// Chapter id (UUID hyphenated). **Not stable across a re-read of
    /// the material**: replacing an imported band's contents mints new
    /// rows, so a consumer that has to remember one section keys on
    /// `(layer_id, ord)` rather than on this.
    pub id: String,
    /// Band the chapter belongs to. Always a `"structure"` band.
    pub layer_id: String,
    /// Start of the section on the playback timeline, in milliseconds
    /// from the presentation origin.
    pub start_ms: i64,
    /// Exclusive end of the section. `None` means the file stated no
    /// end — the section starts here and the next one's start is where
    /// it stops.
    pub end_ms: Option<i64>,
    /// The section's title as the container declares it. **Empty is
    /// legal.**
    pub label: String,
    /// Reading order within the band. Carried rather than derived from
    /// `start_ms`, because a container is free to declare its sections
    /// in an order of its own and the list a person reads is the one
    /// the file states.
    pub ord: u32,
}

/// One band together with the chapters in it — what an asset-level read
/// of the layer model returns per band.
///
/// Bundled rather than left to a second call per band because the
/// surface that shows a chapter list needs both halves at once (the
/// bands to choose between, and the contents of the chosen one), and an
/// asset carries single-digit bands: the round trips saved are the point
/// and the payload is the same rows either way.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MaterialLayerViewDto {
    /// The band itself.
    pub layer: MaterialLayerDto,
    /// The sections in it, in reading order. **Always empty for an
    /// `"annotation"` band** — those hold notes, which are read through
    /// the material-marks route. An empty list on a `"structure"` band
    /// is a different statement: the band exists and declares no
    /// sections (a file that was read and turned out to have none),
    /// which is not the same as the asset having no band at all (never
    /// read).
    pub chapters: Vec<ChapterMarkDto>,
}

/// Thread anchor — what a `ThreadDto` hangs off of.
///
/// `anchor_kind`:
/// - `"app_global"` — Home-tab Inbox / free-form threads. `anchor_id`
///   is `None`.
/// - `"snapshot"` — anchored to a Snapshot. `anchor_id` is the
///   Snapshot uuid.
/// - `"query_group"` — anchored to a query Group. `anchor_id` is the
///   Group uuid.
/// - `"card"` — anchored to an Asset (per-card conversation lane).
///   `anchor_id` is the Asset uuid.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ThreadAnchorDto {
    /// `"app_global"` / `"snapshot"` / `"query_group"` / `"card"`.
    pub kind: String,
    /// Referenced entity id. `None` when `kind = "app_global"`.
    pub id: Option<String>,
}

/// One reference chip embedded in a `MessageDto` body.
///
/// `kind`:
/// - `"card"` — asset uuid.
/// - `"snapshot"` — snapshot uuid.
/// - `"query_group"` — group uuid.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MessageRefDto {
    /// `"card"` / `"snapshot"` / `"query_group"`.
    pub kind: String,
    /// Referenced entity id.
    pub id: String,
}

/// Thread container payload.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ThreadDto {
    /// Thread id (UUID hyphenated).
    pub id: String,
    /// Display label.
    pub title: String,
    /// Anchor (what the thread hangs off of).
    pub anchor: ThreadAnchorDto,
    /// Creation timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Last mutation timestamp (unix epoch ms). Reflects both title
    /// / archive changes and appended messages.
    pub updated_at_ms: i64,
    /// Post timestamp of the newest attached message (unix epoch
    /// ms). `None` for an empty Thread.
    pub last_message_at_ms: Option<i64>,
    /// Number of messages currently attached.
    pub message_count: u32,
    /// Soft-hide flag. Archived threads are excluded from default
    /// listings.
    pub archived: bool,
}

/// One appended entry in a Thread.
///
/// `author_kind`:
/// - `"human"` — the person running Asterism. `author_name` /
///   `author_persona_id` are both `None`.
/// - `"claude_code"` — the Claude Code CLI / harness.
///   `author_name` / `author_persona_id` are both `None`.
/// - `"agent"` — a named agent worker. `author_name` carries the
///   agent slug (e.g. `"lint-bot"`); `author_persona_id` is
///   `None`.
/// - `"persona"` — a persona-tagged author. `author_persona_id`
///   carries the persona uuid; `author_name` is `None`.
///
/// `role` is `"note"` / `"action"` / `"system"` (see
/// [`crate::command::AppendMessageCommand`] for semantics).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MessageDto {
    /// Message id (UUID hyphenated).
    pub id: String,
    /// Owning Thread.
    pub thread_id: String,
    /// `"human"` / `"claude_code"` / `"agent"` / `"persona"`.
    pub author_kind: String,
    /// Agent slug when `author_kind = "agent"`; `None` otherwise.
    pub author_name: Option<String>,
    /// Persona id when `author_kind = "persona"`; `None` otherwise.
    pub author_persona_id: Option<String>,
    /// `"note"` / `"action"` / `"system"`.
    pub role: String,
    /// Free-form body (markdown).
    pub body: String,
    /// Reference chips (may be empty).
    pub refs: Vec<MessageRefDto>,
    /// Post timestamp (unix epoch ms).
    pub created_at_ms: i64,
}

/// One row of a sidebar count aggregation — `(key, asset_count)`.
///
/// The `key` string carries different id shapes depending on the
/// axis being counted (persona UUID for
/// `list_persona_asset_counts`, modality slug for
/// `list_modality_asset_counts`); the UI resolves it against the
/// axis's own name map.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AssetCountEntryDto {
    /// The bucket key — a persona uuid, a modality slug, or
    /// whatever the endpoint documents.
    pub key: String,
    /// Number of distinct assets in this bucket.
    pub count: u64,
}

/// Which fingerprint a duplicate finding is about.
///
/// The wire half of `asterism_core::domain::duplicate_conflict::DuplicateAxis`,
/// declared here for the reason [`OnDuplicate`](crate::command::OnDuplicate)
/// is: this crate is a leaf, the arrow to `asterism-core` points the
/// other way, and this is the crate the TypeScript bindings and the MCP
/// schemas are generated from. The `snake_case` tokens are the same on
/// both sides and the conversions are exhaustive matches, so a further
/// axis stops compiling until both sets answer for it.
///
/// An enum rather than a free string because the reader that most needs
/// the closed set is the panel: it draws one control per axis and has to
/// label a group with the agreement it reports. Rendered as a union in
/// the generated bindings, which is what keeps the UI from restating the
/// vocabulary in TypeScript — and a restatement is how a fourth spelling
/// (`"File"`, `"content-region"`) gets compared against on one side only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateAxis {
    /// Every byte of the artefact (`sha256:` over the whole file).
    ///
    /// Spelled `artefact` on the wire, matching the domain's name for
    /// it and the value V64 rewrote into every stored `axis`. It said
    /// `file` before that migration; nothing answers to the old token,
    /// on this side or in `DuplicateAxis::parse` on the other.
    Artefact,
    /// Only the bytes that decide the decoded result
    /// (`cr1-sha256:` over the content region).
    Content,
    /// Only the metadata the container carries about the artefact
    /// (`m1-sha256:` over the canonical key → value rendering) — the
    /// exact complement of the axis above.
    ///
    /// Neither this nor `Content` implies the other: two frames off one
    /// workflow differing only by a seed agree here and not there, and
    /// one picture re-exported with a caption written in agrees there
    /// and not here.
    Meta,
}

/// One set of live assets that share a fingerprint on one axis.
///
/// Members are ordered oldest first, which is the order a "keep the
/// first one, trash the rest" reading expects. The group is only
/// reported when it has two or more members — a fingerprint held by
/// one asset is not a finding.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DuplicateGroupDto {
    /// Which fingerprint the members agreed on: `"artefact"` (every byte
    /// of the artefact) or `"content"` (only the bytes that decide the
    /// decoded result).
    ///
    /// Both are computed now, and a report answers about one of them —
    /// the caller names it on the request and gets it echoed on every
    /// group. Two groups over the same assets mean different things,
    /// and a panel that ran them together would be claiming an
    /// agreement nothing measured.
    pub axis: DuplicateAxis,
    /// The fingerprint the members share, **on `axis`**: `sha256:<hex>`
    /// on the artefact axis, `cr1-sha256:<hex>` on the content one.
    ///
    /// The name stays `content_hash` while the value changes shape,
    /// which is worth being explicit about because the sibling decision
    /// on the storage side went the other way (the artefact digest kept the
    /// `content_hash` column and the region digest got a new one,
    /// `content_region_hash`). A column is per-axis and had to be; this
    /// field is *the key of this group*, and the group already says
    /// which axis it is. Splitting it into two optional fields would
    /// give every reader a null to handle and every writer a chance to
    /// fill the wrong one, to express something `axis` states directly.
    /// `DuplicateConflictDto::content_hash` carries the same value on
    /// the same terms, so the two surfaces stay one vocabulary.
    ///
    /// The algorithm tag travels with the value, so keys from the two
    /// axes cannot collide and a reader holding a bare key can still
    /// tell which question it answers.
    pub content_hash: String,
    /// The duplicate assets, oldest first.
    pub members: Vec<AssetCardDto>,
}

/// One unanswered "are these two the same thing?" question.
///
/// Raised when a fingerprint lands on bytes another asset in the same
/// persona already holds and the registering caller's strategy was to
/// ask (the default). What the pair *did* agree on is recorded
/// separately and permanently as an `identical_to` edge; this is the
/// part of the event that stops existing once somebody answers.
///
/// Both sides arrive as full cards rather than ids. A panel showing a
/// pair has to draw two thumbnails, two titles and two locators to be
/// worth showing at all, so returning ids would buy nothing but a
/// second round-trip per row.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DuplicateConflictDto {
    /// Queue row id — what
    /// `POST /asterism/duplicates/conflicts/resolve` names.
    pub id: String,
    /// Which fingerprint agreed, read as on [`DuplicateGroupDto::axis`].
    /// A queue row is about one pair on one axis, so unlike the report
    /// this is not selected by the caller — it is what detection found.
    pub axis: DuplicateAxis,
    /// The digest the two share.
    pub content_hash: String,
    /// The row whose arrival raised the question — the younger of the
    /// two. Which side this is depends on when each was fingerprinted,
    /// not on which one a person should keep.
    pub newcomer: AssetCardDto,
    /// The row that already held these bytes: the oldest holder, and
    /// the one an automatic fold would have kept.
    pub incumbent: AssetCardDto,
    /// Why an automatic fold was declined, when one was asked for:
    /// `"lineage"` (the two are connected through `derived_from`, or
    /// the graph was too large to say otherwise) or `"dispatch"` (at
    /// least one is the output of an export run).
    ///
    /// `null` is the ordinary case and means no automatic fold was
    /// declined — nobody asked for one. It is **not** "no reason
    /// known".
    ///
    /// A slug rather than a sentence: every other closed vocabulary on
    /// this wire is one, and a rendered warning baked in here would be
    /// a UI string travelling through the transport, re-worded in the
    /// contract crate every time the panel's wording changed. The two
    /// values are documented above; the caller writes the sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold_exclusion: Option<String>,
    /// When the match was observed (unix epoch ms).
    pub detected_at_ms: i64,
}

/// What one answered question ended up saying.
///
/// Returned by the resolution verb rather than an empty 200, because
/// the two answers have different consequences and a caller that
/// cannot tell which one it just recorded cannot report it either.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DuplicateResolutionDto {
    /// The queue row that was closed.
    pub conflict_id: String,
    /// `"folded"` (ruled one thing) or `"kept"` (ruled two).
    pub resolution: String,
    /// When the answer was recorded (unix epoch ms).
    pub resolved_at_ms: i64,
    /// On `"folded"`: the asset that stays, carrying the pair's tags,
    /// groups, comments and edges once the fold runs. `null` on
    /// `"kept"` — nothing was folded, so nothing was kept *instead of*
    /// anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper_id: Option<String>,
    /// On `"folded"`: the asset that becomes a headstone pointing at
    /// the keeper. `null` on `"kept"`.
    ///
    /// The fold itself runs as a queued job, so this names what *will*
    /// be folded; the row is still live when this returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headstone_id: Option<String>,
}

/// What the manual merge verb saw and did — the answer to
/// [`MergeAssetsCommand`](crate::command::MergeAssetsCommand), for both
/// the preview and the real thing.
///
/// # One shape for `dry_run` and the commit
///
/// The two branches of the command return the **same** DTO, and that is
/// not incidental. The port doc on
/// [`merge_into`](../../asterism_core/domain/repository/trait.AssetRepository.html#tymethod.merge_into)
/// records why: the preview is a prediction of *this* call, and every
/// number it reports (folded ids, refusals, per-column counts) comes
/// from the statements that would have been kept — not from a second
/// implementation that estimates alongside. Splitting the DTO would make
/// that contract unhelpable on the wire: a caller would read two shapes
/// off the same verb and, at the exact point where the two must agree,
/// have no way to say so. The single field that distinguishes a run from
/// a preview — [`committed`](Self::committed) — is what the port doc
/// keeps them tellable-apart on.
///
/// This is also how a panel following a preview reads the answer back.
/// The `false` returned by `dry_run` is the *same DTO shape* the `true`
/// returned by the commit is, so the code that rendered "these five rows
/// will fold into this one, and this exclusion warns why the machine
/// would not have done it" runs unchanged over the confirmed run —
/// which is the affordance the wire layer is there to give the caller.
///
/// # `warnings` is the whole reason a preview is worth running
///
/// The exclusions that stop an *automatic* fold (lineage, dispatch
/// product) are deliberately not binding on the manual merge verb — a
/// person looking at the rows can see what the rule was protecting
/// against. They are read anyway, on the dry run, so
/// the panel can say what those rules would have said before the person
/// overrode them. On the commit branch [`warnings`](Self::warnings) is
/// always empty, since the caller has already seen them.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeAssetsDto {
    /// The row that stayed — the same id the caller sent as
    /// `MergeAssetsCommand::keeper_id`, echoed for callers that read the
    /// answer separately from the request they sent.
    pub keeper_id: String,
    /// Rows that were folded into the keeper on this call, in fold
    /// order — the same order
    /// [`MergePlan::discard`](../../asterism_core/domain/merge_plan/struct.MergePlan.html#method.discard)
    /// lists them in, so the `register_note` paragraphs and the
    /// keeper's `_trace.absorbed` entries afterwards read in it. Carried
    /// as ids rather than counted for the reason the port doc on
    /// [`MergeOutcome::folded`](../../asterism_core/domain/repository/struct.MergeOutcome.html#structfield.folded)
    /// gives.
    ///
    /// On `dry_run` these are the rows that *would* have folded; on the
    /// commit they are the rows that *did*. [`committed`](Self::committed)
    /// says which of the two this is.
    pub folded_ids: Vec<String>,
    /// Rows that were already folded into this same keeper when the call
    /// ran, in the order they were reached. Carried separately from
    /// [`folded_ids`](Self::folded_ids) because they are the plan already
    /// being true for those rows rather than a fold that happened here —
    /// the state after the call is exactly the state the plan declares
    /// either way. See
    /// [`MergeOutcome::already_folded`](../../asterism_core/domain/repository/struct.MergeOutcome.html#structfield.already_folded)
    /// for why this is not a refusal.
    pub already_folded_ids: Vec<String>,
    /// Rows the merge could not touch and the reason each one was
    /// refused, in the order they were reached. **Non-empty means
    /// nothing was written** ([`committed`](Self::committed) is
    /// `false`) — a merge is a decision about the whole set, and
    /// executing some other set is the worst way for it to fail. The
    /// caller re-reads the panel, rules again, and the refusals here
    /// name which rows the ruling has to be re-made against.
    pub refusals: Vec<MergeRefusalDto>,
    /// Rules that would have stopped an *automatic* fold of a pair
    /// somewhere in this merge — populated only on `dry_run`, and
    /// **always empty** on the commit branch.
    ///
    /// The point is not to refuse the merge (a person looking at the
    /// rows overrides the rules on purpose) but
    /// to say what the rules were protecting before the override
    /// happens, so the panel can put "these two share a lineage — going
    /// through with this loses the record that A was derived from B" in
    /// front of the person who is about to click confirm.
    ///
    /// Empty on the commit branch by design: the caller has already
    /// seen the warnings on the preview, and a commit that recomputed
    /// them would be a second read of the graph for a report the caller
    /// is not being handed a choice on. If a panel wants the warnings
    /// on the confirmed run too, it keeps the ones the preview returned
    /// and shows them beside the commit's answer.
    pub warnings: Vec<MergeWarningDto>,
    /// Totals of every fold that went through, summed across the whole
    /// merge — a straight field-by-field port of
    /// [`FoldReport`](../../asterism_core/domain/repository/struct.FoldReport.html).
    ///
    /// On `dry_run` these are the counts the merge *would* have
    /// written; on the commit they are the counts it *did*. The two
    /// come from the same statements, per the port doc, so they agree
    /// unless the world moved between the two calls.
    pub totals: MergeTotalsDto,
    /// **The only field that tells a `dry_run` DTO apart from a commit
    /// one.**
    ///
    /// `false` on `dry_run` and `false` on a refused merge — the two
    /// are the same event at the storage layer, and a caller that only
    /// looked at [`folded_ids`](Self::folded_ids) would read a
    /// prediction as a result. `true` only when every fold in the plan
    /// went through and the transaction was kept.
    ///
    /// See
    /// [`MergeOutcome::committed`](../../asterism_core/domain/repository/struct.MergeOutcome.html#structfield.committed)
    /// for the storage-layer counterpart.
    pub committed: bool,
}

/// One row a fold could not touch, and why.
///
/// The reason is a slug rather than a sentence for the same reason
/// [`DuplicateConflictDto::fold_exclusion`] is: every closed vocabulary
/// on this wire is one, and a rendered warning baked in here would be a
/// UI string travelling through the transport, re-worded in the contract
/// crate every time the panel's wording changed. The slugs come from
/// [`FoldRefusal::as_str`](../../asterism_core/domain/repository/enum.FoldRefusal.html#method.as_str)
/// — `"already folded"`, `"no such asset"`, `"no such keeper"`,
/// `"the keeper is itself folded"`, `"the keeper is in the trash"`,
/// `"an asset cannot be folded into itself"` — the caller writes the
/// sentence.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeRefusalDto {
    /// The row the fold was refused for.
    pub asset_id: String,
    /// Slug of the refusal, from
    /// [`FoldRefusal::as_str`](../../asterism_core/domain/repository/enum.FoldRefusal.html#method.as_str).
    pub reason: String,
}

/// One rule that would have declined an *automatic* fold of a pair, if
/// this pair had reached detection.
///
/// The manual merge verb overrides these rules on purpose — that is the
/// whole point of a person's ruling — but a panel drawing the
/// [`dry_run`](crate::command::MergeAssetsCommand::dry_run) has to say
/// what a rule was protecting before the person overrides it. That is
/// what this is: one pair, one exclusion, and the words for which one.
///
/// `keeper_id` and `headstone_id` are both here rather than one of them,
/// because a merge folds several rows into one keeper and a caller
/// reading a list of warnings has to be able to say **which pair each
/// one is about**. A warning that named only the discard would still be
/// readable ("this row shares a lineage with the keeper"), but a
/// warning list is being drawn beside a panel that lists the whole set,
/// and pointing at both ends is what lets the caller mark the pair
/// rather than the row.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeWarningDto {
    /// The row that stays after the merge — always the same for a given
    /// merge, echoed on every warning so a caller reading the list does
    /// not have to track it back to
    /// [`MergeAssetsDto::keeper_id`](MergeAssetsDto::keeper_id).
    pub keeper_id: String,
    /// The row this warning is about, from the discard side of the
    /// plan.
    pub headstone_id: String,
    /// Which rule declined the pair —
    /// [`FoldExclusion::as_str`](../../asterism_core/domain/duplicate_conflict/enum.FoldExclusion.html#method.as_str)
    /// = `"lineage"` (the two are connected through `derived_from`, or
    /// the graph was too large to say otherwise) or `"dispatch"` (at
    /// least one is the output of an export run).
    pub kind: String,
}

/// Row-count totals of a merge, a field-by-field port of
/// [`FoldReport`](../../asterism_core/domain/repository/struct.FoldReport.html)
/// — see that type for what each column means and how the two
/// deliberately-doubled counts
/// ([`columns_merged`](Self::columns_merged) and
/// [`values_discarded`](Self::values_discarded)) are read.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MergeTotalsDto {
    /// Edges that now name the keeper on the side that named the
    /// headstone.
    pub edges_repointed: u64,
    /// Edges that could not move and were removed instead: the pair's
    /// own edges and those whose `(from, to, kind)` the keeper already
    /// had.
    pub edges_dropped: u64,
    /// Group memberships the keeper gained.
    pub buckets_moved: u64,
    /// Rows that were filed inside a headstone and are now filed inside
    /// the keeper.
    pub children_repointed: u64,
    /// Tag links the keeper gained (excluding tags it already carried).
    pub tags_moved: u64,
    /// Comments that hung off a headstone and now hang off the keeper.
    pub comments_moved: u64,
    /// Threads anchored on a headstone's card that are now anchored on
    /// the keeper's.
    pub threads_reanchored: u64,
    /// Columns of the keeper the merge rewrote — a **write count**, not
    /// a distinct-column count. Three folds each contributing to
    /// `labels` add three.
    pub columns_merged: u64,
    /// Values a headstone held where the keeper's own stood — same
    /// counting rule as [`columns_merged`](Self::columns_merged).
    pub values_discarded: u64,
}

/// The duplicate report (`GET /asterism/duplicates`).
///
/// Carries two counts alongside the groups because an empty report has
/// three very different meanings — "there are no duplicates", "nothing
/// has been fingerprinted yet", and "the content axis has never looked
/// at these files" — and only the numbers can tell them apart.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DuplicateReportDto {
    /// Groups sharing a fingerprint on the requested axis, newest group
    /// first. Every group repeats the axis it was grouped on.
    pub groups: Vec<DuplicateGroupDto>,
    /// Materials with no answer recorded yet. Non-zero means the
    /// report is still incomplete.
    ///
    /// Files that can never be fingerprinted (a record inside a
    /// conversation log, a remote locator) are **not** counted — they
    /// carry an explicit "no bytes to read" marker, so the number
    /// converges to zero on a healthy library instead of sitting at
    /// the conversation corpus's size forever. What keeps it non-zero
    /// is a file that was unreadable when the job ran: an unplugged
    /// disk, a moved original.
    pub unhashed_count: u64,
    /// Materials the **content axis has no reading of**: rows still
    /// carrying `unsupported:not-walked`.
    ///
    /// The marker is written over every pre-existing row by the
    /// migration that adds the column and cleared by the next step of
    /// the same chain, which reads the files. Both run before the
    /// application serves anything, so what is counted here is not a
    /// backlog waiting to start — it is the originals that step could
    /// **not open**: files moved or deleted since they were imported, or
    /// a disk that was not connected during the upgrade.
    ///
    /// Reported on every request, not only content-axis ones, so a
    /// caller can tell before switching axis that the answer over there
    /// omits this many rows.
    ///
    /// This is not a smaller `unhashed_count`. That one counts rows with
    /// no answer at all and converges to zero on its own as the
    /// fingerprint walk runs. This one counts rows with an answer that
    /// happens to be "these bytes were never read", and **it does not
    /// move on its own** — the files have to come back. A caller that
    /// renders it as a progress bar is describing a number that is not
    /// going anywhere; one that omits it because the other count is zero
    /// turns "some of your originals are missing" into "your library has
    /// no content duplicates".
    ///
    /// Not persona-scoped — see the port
    /// (`AssetRepository::unwalked_material_count`).
    pub unwalked_count: u64,
}

/// One row of the Modality master (`GET /asterism/modalities`).
///
/// Two-layer model: `slug` / `label` / `sort_order` / `hidden` /
/// `cover_template` are the open, editable identity + presentation
/// metadata; `kind` is the single reference into the closed
/// `ContentKind` behaviour set. `asset_count` is the number of assets
/// currently carrying the slug (feeds the sidebar badge and the
/// delete guard).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ModalityDefDto {
    /// Open modality slug (primary key).
    pub slug: String,
    /// Display name.
    pub label: String,
    /// Whether assets of this classification read as terminal
    /// transcripts — the one display question the semantic axis
    /// decides. Everything else (thumbnail, media player, "is this
    /// text") comes from the material's mime.
    #[serde(default)]
    pub terminal: bool,
    /// Sidebar sort rank.
    pub sort_order: i64,
    /// Whether the modality is hidden from the default listing.
    pub hidden: bool,
    /// Cover-template slug; `None` = the generic first-line template.
    pub cover_template: Option<String>,
    /// Number of assets currently carrying this slug.
    pub asset_count: u64,
}

/// One registered series Strategy — a rule for reading "made the same
/// way" out of a material's metadata (`GET /asterism/series-strategies`).
///
/// The five rule fields are what the derivation reads; the three
/// provenance fields are what the row says about itself. `include` and
/// `exclude` keep the nesting the rule was written with — a list of
/// paths, each a list of segments — because a flattened `["vdsl.script"]`
/// would be one path naming a keyword nothing carries, and the two spell
/// alike enough that a reader would not notice.
///
/// `system` is provenance and not permission: it says a migration seeded
/// the row, and a seeded row is editable and deletable like any other.
/// `created_at_ms` / `updated_at_ms` are here for the same reason they
/// are in the schema — equal stamps are how a later migration tells a
/// pristine seed from one somebody took over.
///
/// **No group projection.** Which materials a rule put on which key is a
/// different question with a different shape, and no surface asks it yet;
/// see the `series` domain module for the constraint the query that
/// eventually does will have to satisfy.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SeriesStrategyDto {
    /// Strategy id (UUID hyphenated) — what derived rows are filed
    /// under, and what the `PATCH` / `DELETE` paths name.
    pub id: String,
    /// Display name. Never read by the derivation, so a rename moves no
    /// key.
    pub name: String,
    /// The one media type this rule is written against.
    pub applies_to: String,
    /// Decoder token: `none` / `raw_json` / `base64_json` / `exif`.
    pub decode: String,
    /// Sub-trees to keep, outermost segment first. Empty means the whole
    /// of the container's metadata.
    pub include: Vec<Vec<String>>,
    /// Sub-trees to drop, applied after `include` and rooted the same
    /// way.
    pub exclude: Vec<Vec<String>>,
    /// Whether a migration seeded this row.
    pub system: bool,
    /// When the rule was registered (unix epoch ms).
    pub created_at_ms: i64,
    /// When it was last written (unix epoch ms). Equal to
    /// `created_at_ms` until something edits it.
    pub updated_at_ms: i64,
}

/// A named `(filter, sort)` snapshot pinned in the sidebar next to
/// Selections and Groups. Restores to the grid's active filter chips
/// + Sorter toolbar without freezing asset ids.
///
/// `filter_json` is a serialised `ListAssetsQuery`; `sort_json` is a
/// serialised `SortSpec`. The wire form matches
/// [`CreateSavedQueryCommand`](crate::command::CreateSavedQueryCommand) —
/// string blobs, because `schema-bridge` cannot codegen
/// `serde_json::Value`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SavedQueryDto {
    /// SavedQuery id (UUID hyphenated).
    pub id: String,
    /// Persona bucket.
    pub persona_id: String,
    /// Human-facing name (unique per persona).
    pub name: String,
    /// `ListAssetsQuery` serialised as JSON.
    pub filter_json: String,
    /// `SortSpec` (target / order / reverse) serialised as JSON.
    pub sort_json: String,
    /// Sidebar position (ascending).
    pub position: i64,
    /// Creation timestamp (unix epoch ms).
    pub created_at_ms: i64,
    /// Last-updated timestamp (unix epoch ms).
    pub updated_at_ms: i64,
}

/// One layer contributing a value to a setting.
///
/// The listing returns every layer that has a value, not just the one in
/// force, so a client can show what the effective value is shadowing —
/// the same reason `git config --show-origin` prints each scope rather
/// than only the winner. A settings screen that shows only the winner
/// cannot explain why a control reads the way it does, and cannot offer
/// a meaningful way back to the layer underneath.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SettingLayerDto {
    /// Layer name: `default` / `env` / `stored`.
    pub source: String,
    /// This layer's value as JSON text — canonicalised unless
    /// `rejected` is set, in which case it is the raw text that failed.
    pub value_json: String,
    /// Where the value comes from when there is something to name — the
    /// variable name for `env`. `None` for `default` and `stored`.
    pub origin: Option<String>,
    /// Why this layer contributes nothing, when it does not (an
    /// unparseable export, a stored value outside the key's range).
    /// `None` for a layer that is in play.
    ///
    /// Rejected layers are listed so a client can show that a value was
    /// supplied and discarded, with the reason. A rejected layer is
    /// never the one in force.
    pub rejected: Option<String>,
}

/// One application setting, resolved through the whole layer stack
/// (`GET /asterism/settings`).
///
/// The row carries the value in force (`value_json` / `source`), the
/// whole chain it came from (`layers`, which includes the code default
/// and any rejected layer), and the material a settings UI needs to
/// render a control without a second round trip: the declared `kind`,
/// the `min` / `max` bound, and the `env_var` that seeds the key.
///
/// Values are JSON text rather than a tagged union because
/// `schema-bridge` cannot codegen one; the consumer parses according to
/// `kind`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SettingDto {
    /// Registry key (`ui.clean_mode`, `jobs.concurrency`, …).
    pub key: String,
    /// Declared value shape: `bool` / `int` / `text`.
    pub kind: String,
    /// Value in force, as JSON text. Always equals the last entry of
    /// `layers` whose `rejected` is `None`.
    pub value_json: String,
    /// Which layer supplied `value_json`: `default` / `env` / `stored`.
    pub source: String,
    /// Every layer that supplied a value, ordered lowest precedence
    /// first, including rejected ones. The last non-rejected entry is
    /// the one in force; entries before it are what it shadows.
    /// Never empty — the code default always contributes and is never
    /// rejected, so `layers[0]` is the default.
    pub layers: Vec<SettingLayerDto>,
    /// Environment variable that seeds this key, when it has one. Named
    /// even while nothing is exported, so a client can tell the user
    /// which variable would apply.
    pub env_var: Option<String>,
    /// Inclusive lower bound for an `int` key; `None` = unbounded.
    /// Present so a client can render the control's constraint, not so
    /// it can enforce it — the backend rejects out-of-range writes
    /// regardless of which caller made them.
    pub min: Option<i64>,
    /// Inclusive upper bound for an `int` key; `None` = unbounded.
    pub max: Option<i64>,
    /// One-line description for the settings UI.
    pub summary: String,
}
