//! Sort evaluation — the backend port of the UI grid comparator.
//!
//! The UI sorts the asset grid entirely client-side in
//! `crates/asterism-ui/src/lib/sort/card-cmp.ts`: `buildCardCmp`
//! produces a JS compare-fn from the `(target, order, reverse)`
//! [`SortSpec`], `sortCards` applies it, and `computeBucketRecency`
//! feeds the "most-recently-touched" orders. Query Groups must freeze
//! this exact
//! ordering into `asset_bucket.position`, so the comparator is
//! re-implemented here on the backend and the members are materialised
//! in the resulting order.
//!
//! Entry point: [`sort_asset_ids`] — takes the resolved id set (as
//! [`SortableAsset`] rows) plus the ancillary lookups the UI reads from
//! its catalogs ([`SortContext`]) and returns the ordered ids that W1b's
//! materialize pipeline writes as `position 0, 1, 2, …`.
//!
//! # Faithfulness to the UI (drift watch)
//!
//! Each branch cites the `card-cmp.ts` symbol it mirrors — by name, not
//! by line, because the comparator previously lived inside `App.svelte`
//! and every citation here rotted as that component grew (the extraction
//! into its own module is what made the references stable, and made the
//! parity test below possible at all).
//!
//! The paired test file is `card-cmp.test.ts`: its cases carry the same
//! names as the `tests` module here, over the same fixture values, so a
//! one-sided edit surfaces as a missing or renamed case on the other
//! side. `collation_parity` in both files pins the string ordering.
//!
//! One deliberate, documented divergence exists because the backend
//! cannot rely on the UI's ambient state:
//!
//! 0. **The `Rating` and `UpdatedAt` axes have no UI half at all.** Both
//!    were wired server-side first, so `card-cmp.ts` has no `rating` or
//!    `updated_at` branch and no paired case exists for their tests
//!    below. `UpdatedAt` in particular exists for a caller the grid is
//!    not — an API consumer paging a differential sync — so its parity
//!    gap may well be permanent. Documented (see [`SortTarget::Rating`],
//!    [`SortTarget::UpdatedAt`]), not an omission the parity discipline
//!    missed.
//!
//!    `Duration` and `FileSize` landed backend-first the same way and no
//!    longer share that gap: `card-cmp.ts` carries both branches and
//!    `card-cmp.test.ts` holds the paired cases (same names, same
//!    fixture values as the `duration / file size` block below), so the
//!    two comparators are pinned to each other. The layer below the
//!    comparator has since caught up as well — the index row carries
//!    both columns (`AssetIndex::duration_ms` /
//!    `file_size_bytes`), `indexToLightCard` forwards them, and the
//!    grid's Sort dropdown offers the two axes. Before that the picker
//!    withheld them: the rows it sorts carried neither column, so
//!    picking one client-side compared absent values throughout and
//!    answered in tie-break order.
//!
//! 1. **A final `id` tie-break is appended.** The UI leans on
//!    `Array.prototype.sort` stability + server order to break ties. The
//!    materialized `position` must be deterministic (it feeds
//!    `content_hash`), so after the axis key and the shared
//!    `occurred_at DESC` tie-break, ids break any remaining tie
//!    ascending.
//!
//! # String collation
//!
//! The alphabetical axes compare through ICU collation on both sides:
//! `Intl.Collator` in the UI, ICU4X [`Collator`] here. Which collation
//! is a **search parameter** — [`SortSpec::collation`], a BCP-47 tag
//! that rides in the query group's `query_json` — not a constant in
//! this module and not global settings state. `None` selects CLDR root.
//!
//! Root is the default because it is the only value the two ICU builds
//! reproduce. `Intl.Collator` cannot name root: `"root"` is not a valid
//! language tag (`RangeError`) and `"und"` is not a supported locale, so
//! ECMA-402 resolves it to the *host* default — on a Japanese Mac,
//! `ja-JP`. The UI therefore asks for `"en"`, which CLDR leaves
//! untailored and which is consequently root. See
//! `card-cmp.ts` `ROOT_COLLATION_TAG` for that mapping.
//!
//! Measured parity over a 76-entry corpus (Latin / accents / kana /
//! half-width / Han / CJK extensions / astral / emoji / symbols / the
//! sentinel), comparing this evaluator's root order against
//! `Intl.Collator("en")` on both JavaScriptCore (the WKWebView engine
//! the app actually runs in) and Node (the engine vitest runs in):
//!
//! * JavaScriptCore and Node agree on **all 76** — which is what makes
//!   the vitest-side parity test meaningful evidence about the shipped
//!   runtime rather than about Node.
//! * ICU4X root agrees on **73 of 76**.
//!
//! That is not a one-off measurement: the corpus and both orders live in
//! `fixtures/collation/` and are re-checked by three consumers — the
//! `collation_parity` tests below (ICU4X), `collation-parity.test.ts`
//! (Node), and `just collation-jsc` (JavaScriptCore). The last one is
//! what keeps "Node stands in for the shipped engine" an assertion
//! rather than an assumption; `fixtures/collation/README.md` has the
//! arrangement.
//!
//! The four classes that used to disagree under [`str::cmp`] — case
//! (`apple` < `Zebra`), accents (`édition` < `future`), kana (`アイ` <
//! `んご`) and astral-vs-[`TAIL_SENTINEL`] — all agree now. The last of
//! those was never merely an ordering difference: under code-point order
//! an emoji label (U+1F642) sorted *after* the U+FFFF sentinel, so a
//! frozen group materialised a tail the grid would never render. ICU
//! weights U+FFFF above every assigned code point, restoring the
//! sentinel's contract.
//!
//! ## Known limitation — CJK extension ideographs
//!
//! The residual 3 of 76 are Han **extension** ideographs (Ext A
//! U+3400–, Ext B U+20000–: `㐀` `𠀀` `𠮷`). ICU interleaves the
//! extension blocks among the URO; ICU4X places the URO first and the
//! extensions after it. Everyday Japanese lives in the URO
//! (U+4E00–9FFF) and agrees; the divergence is reachable only by a
//! label or cover whose first differing character is a rare-kanji form
//! such as `𠮷野` or `𩸽`. Pinned rather than fixed — closing it would
//! need a hand-written tailoring, and it is cheaper to let a rare-kanji
//! group's frozen tail sit one slot away from where the grid draws it.
//! `collation_parity::cjk_extension_is_the_residual_divergence` holds
//! the case, paired with the same name in `card-cmp.test.ts`.
//!
//! `cover` is lower-cased before comparison to match the UI
//! (`buildCardCmp`'s `cover` branch); the other alpha axes compare raw,
//! also matching the UI (persona / modality / tag / group do **not**
//! lower-case).

use std::cmp::Ordering;
use std::collections::HashMap;

use asterism_contract::sort::{SortOrder, SortSpec, SortTarget};
use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale_core::Locale;

use crate::error::DomainError;

/// Sentinel key for assets with no user-visible label / no group, so they
/// cluster at the tail. Mirrors `card-cmp.ts` `TAIL_SENTINEL`, read by
/// its `firstUserLabel` and by App's `primaryGroupName`.
///
/// ICU weights the U+FFFF noncharacter above every assigned code point,
/// so under [`collator_for`] the "cluster at the tail" property holds
/// for every key including astral-plane ones. It did **not** hold under
/// the previous code-point comparison — see the module-level notes.
const TAIL_SENTINEL: &str = "\u{FFFF}";

/// Builds the collator the alphabetical axes compare under, from the
/// search parameter [`SortSpec::collation`].
///
/// `None` → CLDR root (locale-independent; the value the UI comparator
/// is pinned against). `Some(tag)` → that tailoring, and it is the
/// caller's business whether the UI was asked for the same one.
///
/// Fails loud on an unparseable tag rather than silently falling back:
/// a query group that names a collation it does not get would freeze a
/// `position` nobody can reproduce.
///
/// Returns the borrowed form: with the `compiled_data` feature the CLDR
/// tables are baked into the binary, so the collator borrows `'static`
/// data and costs no allocation to build per evaluation.
pub fn collator_for(spec: &SortSpec) -> Result<CollatorBorrowed<'static>, DomainError> {
    let prefs = match spec.collation.as_deref() {
        None => CollatorPreferences::default(),
        Some(tag) => {
            let locale: Locale = tag.parse().map_err(|e| {
                DomainError::Validation(format!(
                    "sort collation is not a valid BCP-47 tag: {tag} ({e})"
                ))
            })?;
            CollatorPreferences::from(&locale)
        }
    };
    Collator::try_new(prefs, CollatorOptions::default()).map_err(|e| {
        DomainError::Validation(format!(
            "no collation data for {:?}: {e}",
            spec.collation.as_deref().unwrap_or("und")
        ))
    })
}

/// Fallback persona display name, mirroring `personaName`'s `?? "?"`
/// (`crates/asterism-ui/src/lib/formatters.ts`).
const UNKNOWN_PERSONA_NAME: &str = "?";

/// Label prefixes the UI hides from the tag axis
/// (`card-cmp.ts` `INTERNAL_LABEL_PREFIXES`).
const INTERNAL_LABEL_PREFIXES: [&str; 2] = ["persona:", "journal_kind:"];

/// The per-asset attributes the comparator reads.
///
/// This is a min-superset of
/// [`AssetIndexEntryDto`](asterism_contract::dto::AssetIndexEntryDto)
/// plus `cover` (the index entry omits cover, but the `Cover` axis needs
/// it). W1b builds these from whatever row shape its filter query
/// returns; keeping a dedicated struct decouples the evaluator from any
/// single wire DTO.
#[derive(Debug, Clone)]
pub struct SortableAsset {
    /// Asset id — the value returned in the ordered output.
    pub id: String,
    /// Persona bucket id (persona axis + recency).
    pub persona_id: String,
    /// Primary modality slug (modality axis + recency).
    pub modality: String,
    /// Occurrence timestamp; the universal tie-break (DESC) and the
    /// recency signal.
    pub occurred_at_ms: i64,
    /// Ingest timestamp (the `created_at` axis).
    pub created_at_ms: i64,
    /// Last-modification timestamp (the `updated_at` axis) — a plain
    /// `i64` like the other two time keys, because every row has one:
    /// the column is `NOT NULL` and starts equal to `created_at`, so
    /// there is no "never modified" third state to keep apart.
    pub updated_at_ms: i64,
    /// Labels, in priority order (tag axis reads the first user label).
    pub labels: Vec<String>,
    /// Group ids the asset is filed into; the group axis resolves the
    /// first entry to a name.
    pub group_ids: Vec<String>,
    /// Slot inside the primary group (`asset_bucket.position` for
    /// `group_ids[0]`), or `None` when unfiled. Read only by `Group` +
    /// `Ordered`, which is the hand arrangement.
    pub primary_group_position: Option<i64>,
    /// Card cover text (`Cover` axis). `None` renders as the empty
    /// string, matching the UI's `a.cover ?? ""`.
    pub cover: Option<String>,
    /// Star rating 0-5, `None` when unrated (`Rating` axis). Kept as an
    /// `Option` rather than folded into a number here: the axis treats
    /// "unrated" as a third state that sits outside the ordering, and a
    /// stand-in value (0 or 6) would place it at one end and therefore
    /// flip sides under `reverse`.
    pub rating: Option<u8>,
    /// Playback length in milliseconds, `None` for material that does not
    /// play — a still image, or a container the importer could not probe
    /// (`Duration` axis). An `Option` for the reason [`rating`](Self::rating)
    /// is one: a stand-in `0` would lead a "shortest first" page with
    /// something nobody measured.
    ///
    /// Signed to match the column as SQLite hands it back (`INTEGER`);
    /// the domain query's bounds are `u64` because a negative *request*
    /// is malformed, but a stored value only ever gets compared here.
    pub duration_ms: Option<i64>,
    /// Stored size in bytes, `None` when the row carries no recorded size
    /// (`FileSize` axis). Same three-valued treatment and same signedness
    /// as [`duration_ms`](Self::duration_ms).
    pub file_size_bytes: Option<i64>,
    /// Total pixel count (`width_px * height_px`), `None` when nothing
    /// measured the row's dimensions (`Pixels` axis). Same three-valued
    /// treatment and same signedness as [`duration_ms`](Self::duration_ms).
    ///
    /// Already multiplied out by whoever built this row: the axis orders
    /// on the product, and holding the two sides here would invite a
    /// comparator branch that reads one of them — which would be ordering
    /// by a coded width, a number that says nothing about how large the
    /// picture looks.
    pub pixel_count: Option<i64>,
}

/// Ancillary lookups the UI comparator reads from its catalogs, supplied
/// by the caller so the pure evaluator stays free of I/O.
///
/// Built once per evaluation run. The three "rank / order" inputs are
/// index-based: position in the slice = rank, and anything not in the
/// slice falls to `len` (the tail), mirroring the App-side bindings of
/// `card-cmp.ts` `CardSortLookups`: `personaDisplayOrder`
/// (`findIndex(...) < 0 ? len : idx`) and `modalityRank`
/// (`modalityCatalog.rank`).
#[derive(Debug, Clone, Default)]
pub struct SortContext {
    persona_names: HashMap<String, String>,
    persona_order: HashMap<String, usize>,
    persona_count: usize,
    modality_ranks: HashMap<String, usize>,
    modality_count: usize,
    group_names: HashMap<String, String>,
}

impl SortContext {
    /// Assembles the lookup context.
    ///
    /// * `persona_order` — persona ids in sidebar display order (index =
    ///   rank). Doubles as the tail-rank count for unknown personas.
    /// * `persona_names` — persona id → display name (missing →
    ///   [`UNKNOWN_PERSONA_NAME`]).
    /// * `modality_order` — modality slugs in canonical sidebar order
    ///   (index = rank; missing → tail).
    /// * `group_names` — group id → name (missing / no group →
    ///   [`TAIL_SENTINEL`]).
    pub fn new(
        persona_order: &[String],
        persona_names: HashMap<String, String>,
        modality_order: &[String],
        group_names: HashMap<String, String>,
    ) -> Self {
        let persona_order_map = persona_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        let modality_ranks = modality_order
            .iter()
            .enumerate()
            .map(|(i, slug)| (slug.clone(), i))
            .collect();
        Self {
            persona_names,
            persona_order: persona_order_map,
            persona_count: persona_order.len(),
            modality_ranks,
            modality_count: modality_order.len(),
            group_names,
        }
    }

    fn persona_name(&self, id: &str) -> &str {
        self.persona_names
            .get(id)
            .map(String::as_str)
            .unwrap_or(UNKNOWN_PERSONA_NAME)
    }

    fn persona_display_order(&self, id: &str) -> usize {
        self.persona_order
            .get(id)
            .copied()
            .unwrap_or(self.persona_count)
    }

    fn modality_rank(&self, slug: &str) -> usize {
        self.modality_ranks
            .get(slug)
            .copied()
            .unwrap_or(self.modality_count)
    }

    /// First group id resolved to a name; sentinel when unfiled / unknown.
    /// Mirrors App's `primaryGroupName` binding.
    fn primary_group_name<'a>(&'a self, asset: &SortableAsset) -> &'a str {
        match asset.group_ids.first() {
            None => TAIL_SENTINEL,
            Some(gid) => self
                .group_names
                .get(gid)
                .map(String::as_str)
                .unwrap_or(TAIL_SENTINEL),
        }
    }
}

/// Per-bucket "last touched" times over the slice being sorted, feeding
/// the `Updated` orders. Mirrors `card-cmp.ts` `computeBucketRecency`:
/// each map holds `max(occurred_at_ms)` per persona / modality / group
/// name across exactly the assets under sort.
struct BucketRecency {
    persona: HashMap<String, i64>,
    modality: HashMap<String, i64>,
    tag: HashMap<String, i64>,
    group: HashMap<String, i64>,
}

impl BucketRecency {
    fn compute(assets: &[SortableAsset], ctx: &SortContext) -> Self {
        let mut persona: HashMap<String, i64> = HashMap::new();
        let mut modality: HashMap<String, i64> = HashMap::new();
        let mut tag: HashMap<String, i64> = HashMap::new();
        let mut group: HashMap<String, i64> = HashMap::new();
        for a in assets {
            let t = a.occurred_at_ms;
            bump(&mut persona, &a.persona_id, t);
            bump(&mut modality, &a.modality, t);
            // Tag recency is keyed by the same first-user-label the tag
            // axis buckets on, matching the UI
            // (`tag.set(firstUserLabel(c.labels), t)`).
            bump(&mut tag, first_user_label(&a.labels), t);
            // Group recency is keyed by the resolved name, matching the
            // UI (`group.set(primaryGroupName(c.group_ids), t)`).
            bump(&mut group, ctx.primary_group_name(a), t);
        }
        Self {
            persona,
            modality,
            tag,
            group,
        }
    }
}

fn bump(map: &mut HashMap<String, i64>, key: &str, t: i64) {
    map.entry(key.to_string())
        .and_modify(|cur| {
            if t > *cur {
                *cur = t;
            }
        })
        .or_insert(t);
}

/// First label that is not an internal (`persona:` / `journal_kind:`)
/// prefix; the tail sentinel when none. Mirrors `firstUserLabel`
/// (`card-cmp.ts` `firstUserLabel`).
fn first_user_label(labels: &[String]) -> &str {
    for l in labels {
        if !INTERNAL_LABEL_PREFIXES.iter().any(|p| l.starts_with(p)) {
            return l;
        }
    }
    TAIL_SENTINEL
}

/// Applies `reverse` to a primary-key ordering. `Equal` is invariant, so
/// reversed keys still fall through to the (never-reversed) tie-breaks —
/// matching the `dir` / `tie` split in the UI comparator.
fn apply_reverse(ord: Ordering, reverse: bool) -> Ordering {
    if reverse { ord.reverse() } else { ord }
}

/// The three-valued numeric ordering shared by [`SortTarget::Rating`],
/// [`SortTarget::Duration`], [`SortTarget::FileSize`] and
/// [`SortTarget::Pixels`]: largest first in the natural direction, and
/// rows carrying no value at the tail in **both** directions.
///
/// The absent state sits outside the ordering rather than at one end of
/// it, which is why `reverse` is applied to the value comparison alone
/// instead of to the whole result. Folding "absent" into a number — `0`,
/// or a maximum — would place it at one end and flip it to the other
/// under `reverse`, so a "shortest first" page would open on a still
/// image and a "worst first" page on the cards nobody has judged. Each
/// of those answers a different question from the one asked.
///
/// `Equal` for the both-absent case, so those rows fall through to the
/// shared tie-breaks and stay in a deterministic order among themselves.
fn absent_last_desc<T: Ord>(a: Option<T>, b: Option<T>, reverse: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => apply_reverse(y.cmp(&x), reverse),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

/// Evaluates the primary axis key for one pair, before tie-breaks.
/// Returns `Equal` for the "fall to tie-break" cases.
///
/// Every [`SortTarget`] has a branch here, and the match is exhaustive
/// without a catch-all arm on purpose: an axis with no branch would
/// answer in tie-break order while claiming to sort, which is what
/// `msg_count` did on both sides until it was retired from the wire (see
/// [`SortTarget`]). A new variant must fail to compile here rather than
/// degrade quietly.
fn primary_cmp(
    spec: &SortSpec,
    a: &SortableAsset,
    b: &SortableAsset,
    ctx: &SortContext,
    rec: &BucketRecency,
    coll: &CollatorBorrowed<'_>,
) -> Ordering {
    let rev = spec.reverse;
    match spec.target {
        // Natural (unreversed) direction is DESC, same as the UI's
        // `occurred_at` branch. That branch used to decline the axis and
        // let the server's page order stand; it sorts explicitly now,
        // because the server order is `asset_bucket.position` whenever the
        // page is narrowed to one Group, and the two are not the same
        // sequence. `order` is inert on this target.
        SortTarget::OccurredAt => apply_reverse(b.occurred_at_ms.cmp(&a.occurred_at_ms), rev),
        // `dir * (b.created - a.created)` → natural DESC.
        SortTarget::CreatedAt => apply_reverse(b.created_at_ms.cmp(&a.created_at_ms), rev),
        // Most-recently-changed first, the same natural DESC reading as
        // the other two time axes. No UI branch exists for this target
        // (see `SortTarget::UpdatedAt`); the axis is here because
        // differential sync pages on it — a consumer walking
        // `updated_from_ms` forward needs the page ordered by the column
        // its cursor advances along, or the cursor skips rows it never
        // received. `order` is inert.
        SortTarget::UpdatedAt => apply_reverse(b.updated_at_ms.cmp(&a.updated_at_ms), rev),
        SortTarget::Persona => match spec.order {
            // `dir * (order_a - order_b)` → asc by display order.
            SortOrder::Ordered => apply_reverse(
                ctx.persona_display_order(&a.persona_id)
                    .cmp(&ctx.persona_display_order(&b.persona_id)),
                rev,
            ),
            // `dir * (rb - ra)` → DESC recency.
            SortOrder::Updated => {
                let ra = rec.persona.get(&a.persona_id).copied().unwrap_or(0);
                let rb = rec.persona.get(&b.persona_id).copied().unwrap_or(0);
                apply_reverse(rb.cmp(&ra), rev)
            }
            // `dir * name_a.localeCompare(name_b)` → asc.
            SortOrder::Alpha => apply_reverse(
                coll.compare(
                    ctx.persona_name(&a.persona_id),
                    ctx.persona_name(&b.persona_id),
                ),
                rev,
            ),
        },
        SortTarget::Modality => match spec.order {
            // `dir * (rank_a - rank_b)` → asc.
            SortOrder::Ordered => apply_reverse(
                ctx.modality_rank(&a.modality)
                    .cmp(&ctx.modality_rank(&b.modality)),
                rev,
            ),
            // `dir * (rb - ra)` → DESC recency.
            SortOrder::Updated => {
                let ra = rec.modality.get(&a.modality).copied().unwrap_or(0);
                let rb = rec.modality.get(&b.modality).copied().unwrap_or(0);
                apply_reverse(rb.cmp(&ra), rev)
            }
            // `dir * a.modality.localeCompare(b.modality)` → asc on the raw
            // slug. Slugs are `[a-z0-9_-]`, so this is the one alpha branch
            // whose result the collation choice cannot change; it goes
            // through the collator anyway so every alpha axis has one
            // comparison path.
            SortOrder::Alpha => apply_reverse(coll.compare(&a.modality, &b.modality), rev),
        },
        // Tag axis: the same two readings the other bucketing targets
        // offer. `Updated` used to share the `Alpha` comparison on both
        // sides, so picking between them changed nothing.
        SortTarget::Tag => {
            let ka = first_user_label(&a.labels);
            let kb = first_user_label(&b.labels);
            match spec.order {
                // `dir * (rb - ra)` → DESC recency by tag.
                SortOrder::Updated => {
                    let ra = rec.tag.get(ka).copied().unwrap_or(0);
                    let rb = rec.tag.get(kb).copied().unwrap_or(0);
                    apply_reverse(rb.cmp(&ra), rev)
                }
                // `Ordered` is not offered on this axis (`ORDER_OPTIONS`);
                // a spec that names it reads as alphabetical, which is
                // what the UI's `else` branch does with it.
                SortOrder::Alpha | SortOrder::Ordered => apply_reverse(coll.compare(ka, kb), rev),
            }
        }
        SortTarget::Group => {
            let ka = ctx.primary_group_name(a);
            let kb = ctx.primary_group_name(b);
            match spec.order {
                // `dir * (rb - ra)` → DESC recency by group name.
                SortOrder::Updated => {
                    let ra = rec.group.get(ka).copied().unwrap_or(0);
                    let rb = rec.group.get(kb).copied().unwrap_or(0);
                    apply_reverse(rb.cmp(&ra), rev)
                }
                SortOrder::Alpha => apply_reverse(coll.compare(ka, kb), rev),
                // The hand arrangement. Buckets stay in name order — the
                // Groups themselves have no cross-group sequence a card
                // carries — and inside each one the cards take their
                // `asset_bucket.position`, which is what the drag gesture
                // writes. Unfiled cards have no slot; they already sort
                // into the tail bucket by name, so `i64::MAX` only orders
                // them among themselves and then the tie-break takes over.
                //
                // This is the one axis that used to be a lie: `ordered`
                // shared the `alpha` branch, so the arrangement was
                // unreachable from the picker and only ever visible as the
                // incidental arrival order of a single-Group page.
                SortOrder::Ordered => {
                    let by_name = coll.compare(ka, kb);
                    if by_name != Ordering::Equal {
                        return apply_reverse(by_name, rev);
                    }
                    let pa = a.primary_group_position.unwrap_or(i64::MAX);
                    let pb = b.primary_group_position.unwrap_or(i64::MAX);
                    apply_reverse(pa.cmp(&pb), rev)
                }
            }
        }
        // `(a.cover ?? "").toLowerCase()` alpha.
        SortTarget::Cover => {
            let ka = a.cover.as_deref().unwrap_or("").to_lowercase();
            let kb = b.cover.as_deref().unwrap_or("").to_lowercase();
            apply_reverse(coll.compare(&ka, &kb), rev)
        }
        // Star rating, best-first in its natural direction — the reading
        // the schema's `rating DESC` index is built for, and the one a
        // star widget implies ("show me the good ones"). Unrated rows go
        // last in *both* directions; see [`absent_last_desc`] for why
        // that is not a tidiness choice. `order` is inert, as on the
        // timestamp axes.
        SortTarget::Rating => absent_last_desc(a.rating, b.rating, rev),
        // Playback length, longest first in its natural direction. The
        // absent state here is wider than the rating axis's: a still
        // image has no length by nature, and a video whose container the
        // importer could not probe has none by accident — neither placed
        // anywhere in the band would be a true statement, so both go to
        // the tail. Not a video-only axis; audio carries the same column.
        // `order` is inert.
        SortTarget::Duration => absent_last_desc(a.duration_ms, b.duration_ms, rev),
        // Stored size, largest first in its natural direction, with rows
        // carrying no recorded size at the tail. Same shape and same
        // reasoning one column over; `order` is inert here too.
        SortTarget::FileSize => absent_last_desc(a.file_size_bytes, b.file_size_bytes, rev),
        // Total pixel count, largest first in its natural direction, with
        // unmeasured rows at the tail. The absent state is the widest of
        // the three: everything ingested before the columns existed, and
        // every material that has no pixels to count. `order` is inert.
        //
        // A row measured as `0` is *not* absent — it sorts as the smallest
        // real picture. Nothing in the column's contract says a measured
        // side has to be positive, and quietly folding zero into "not
        // measured" would hide the one case where a parser did answer.
        SortTarget::Pixels => absent_last_desc(a.pixel_count, b.pixel_count, rev),
    }
}

/// Total comparator: primary axis key, then the shared `occurred_at DESC`
/// tie-break (`card-cmp.ts` `tie`, never reversed), then a final
/// `id` tie-break for deterministic `position` materialization.
fn compare(
    spec: &SortSpec,
    a: &SortableAsset,
    b: &SortableAsset,
    ctx: &SortContext,
    rec: &BucketRecency,
    coll: &CollatorBorrowed<'_>,
) -> Ordering {
    primary_cmp(spec, a, b, ctx, rec, coll)
        .then_with(|| b.occurred_at_ms.cmp(&a.occurred_at_ms))
        .then_with(|| a.id.cmp(&b.id))
}

/// Orders the resolved id set exactly as the grid would render it under
/// `spec`, returning the ids in materialize order (`position 0, 1, 2, …`).
///
/// `assets` is the full slice under sort (recency buckets are computed
/// over precisely these rows, matching the UI's `bucketRecency` derived
/// from `filteredBase`). The output length equals `assets.len()`.
///
/// The collator is built once per call from [`SortSpec::collation`]
/// ([`collator_for`]), so an unusable tag fails here rather than
/// freezing an unreproducible `position`.
pub fn sort_asset_ids(
    spec: &SortSpec,
    assets: &[SortableAsset],
    ctx: &SortContext,
) -> Result<Vec<String>, DomainError> {
    let coll = collator_for(spec)?;
    let rec = BucketRecency::compute(assets, ctx);
    let mut order: Vec<usize> = (0..assets.len()).collect();
    order.sort_by(|&i, &j| compare(spec, &assets[i], &assets[j], ctx, &rec, &coll));
    Ok(order.into_iter().map(|i| assets[i].id.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- fixtures -------------------------------------------------------

    #[allow(clippy::too_many_arguments)] // test fixture builder — flat args keep cases readable
    fn asset(
        id: &str,
        persona: &str,
        modality: &str,
        occurred: i64,
        created: i64,
        labels: &[&str],
        groups: &[&str],
        cover: Option<&str>,
    ) -> SortableAsset {
        SortableAsset {
            id: id.into(),
            persona_id: persona.into(),
            modality: modality.into(),
            occurred_at_ms: occurred,
            created_at_ms: created,
            // Only the `UpdatedAt` cases care; they go through `touched`
            // so every other fixture line stays readable. Equal across
            // the default fixtures, which is exactly what makes a
            // cross-wired `UpdatedAt` branch fall to the tie-break and
            // fail its own cases rather than borrowing another axis.
            updated_at_ms: 0,
            labels: labels.iter().map(|s| s.to_string()).collect(),
            group_ids: groups.iter().map(|s| s.to_string()).collect(),
            // Only the `Group` + `Ordered` cases care; they set it through
            // `filed_at` so every other fixture line stays readable.
            primary_group_position: None,
            cover: cover.map(String::from),
            // Only the `Rating` cases care; they go through `rated` so
            // every other fixture line stays readable.
            rating: None,
            // Likewise for the three metric axes, which build through
            // `measured`. `None` on all of them is also the state every
            // other case wants: a row with no length, no recorded size
            // and no measured dimensions is what an unprobed row looks
            // like here.
            duration_ms: None,
            file_size_bytes: None,
            pixel_count: None,
        }
    }

    /// [`asset`] filed into one group at a known slot — the fixture shape
    /// for the manual-arrangement axis.
    fn filed_at(id: &str, group: &str, position: i64, occurred: i64) -> SortableAsset {
        SortableAsset {
            primary_group_position: Some(position),
            ..asset(id, "pa", "dialogue", occurred, 1, &[], &[group], None)
        }
    }

    /// [`asset`] with all three time stamps placed independently — the
    /// fixture shape for the modification axis.
    ///
    /// The three arguments exist so a case can put occurrence, ingest and
    /// modification order in genuine disagreement. A row saved by the
    /// application carries stamps that move together, and a fixture built
    /// that way would pass against a comparator reading any of the three.
    fn touched(id: &str, occurred: i64, created: i64, updated: i64) -> SortableAsset {
        SortableAsset {
            updated_at_ms: updated,
            ..asset(id, "pa", "dialogue", occurred, created, &[], &[], None)
        }
    }

    /// [`asset`] carrying a star rating (or `None` for unrated) at a
    /// known occurrence time — the fixture shape for the star axis.
    fn rated(id: &str, rating: Option<u8>, occurred: i64) -> SortableAsset {
        SortableAsset {
            rating,
            ..asset(id, "pa", "dialogue", occurred, 1, &[], &[], None)
        }
    }

    /// [`asset`] carrying a playback length, a stored size and a pixel
    /// count (any of them `None` for material that has none) at a known
    /// occurrence time — the fixture shape for the three metric axes.
    ///
    /// All three metrics are set from one call so a case can put them in
    /// disagreement: the longest clip being the smallest file being the
    /// largest picture is what separates the branches from each other,
    /// and a fixture where the three ran together would pass against any
    /// of them.
    fn measured(
        id: &str,
        duration_ms: Option<i64>,
        file_size_bytes: Option<i64>,
        pixel_count: Option<i64>,
        occurred: i64,
    ) -> SortableAsset {
        SortableAsset {
            duration_ms,
            file_size_bytes,
            pixel_count,
            ..asset(id, "pa", "dialogue", occurred, 1, &[], &[], None)
        }
    }

    fn ctx() -> SortContext {
        let mut persona_names = HashMap::new();
        persona_names.insert("pa".to_string(), "Aiko".to_string());
        persona_names.insert("pb".to_string(), "Ben".to_string());
        persona_names.insert("pc".to_string(), "Cara".to_string());
        let mut group_names = HashMap::new();
        group_names.insert("g1".to_string(), "Alpha".to_string());
        group_names.insert("g2".to_string(), "Beta".to_string());
        // Display order deliberately differs from alphabetical: pc, pa, pb.
        SortContext::new(
            &["pc".into(), "pa".into(), "pb".into()],
            persona_names,
            &["dialogue".into(), "journal".into(), "media".into()],
            group_names,
        )
    }

    /// Shadows the public entry point with an unwrapping form so the
    /// case bodies read as `sort_asset_ids(...) == [...]`. The `Result`
    /// only carries "that collation tag is unusable", which every case
    /// but `collation_is_a_search_parameter` has no interest in.
    fn sort_asset_ids(spec: &SortSpec, assets: &[SortableAsset], ctx: &SortContext) -> Vec<String> {
        super::sort_asset_ids(spec, assets, ctx).expect("root collation is always available")
    }

    fn spec(target: SortTarget, order: SortOrder, reverse: bool) -> SortSpec {
        SortSpec {
            target,
            order,
            reverse,
            // Root — the collation the parity test pins against.
            collation: None,
        }
    }

    // --- occurred_at ----------------------------------------------------

    #[test]
    fn occurred_at_default_is_desc() {
        let assets = vec![
            asset("a", "pa", "dialogue", 100, 1, &[], &[], None),
            asset("b", "pa", "dialogue", 300, 1, &[], &[], None),
            asset("c", "pa", "dialogue", 200, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::OccurredAt, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["b", "c", "a"]);
    }

    #[test]
    fn occurred_at_reversed_is_asc() {
        let assets = vec![
            asset("a", "pa", "dialogue", 100, 1, &[], &[], None),
            asset("b", "pa", "dialogue", 300, 1, &[], &[], None),
            asset("c", "pa", "dialogue", 200, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::OccurredAt, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["a", "c", "b"]);
    }

    #[test]
    fn occurred_at_tie_breaks_on_id() {
        // Equal occurred → deterministic id ascending (backend addition).
        let assets = vec![
            asset("y", "pa", "dialogue", 100, 1, &[], &[], None),
            asset("x", "pa", "dialogue", 100, 1, &[], &[], None),
            asset("z", "pa", "dialogue", 100, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::OccurredAt, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["x", "y", "z"]);
    }

    // --- created_at -----------------------------------------------------

    #[test]
    fn created_at_desc_then_reversed() {
        let assets = vec![
            asset("a", "pa", "dialogue", 10, 100, &[], &[], None),
            asset("b", "pa", "dialogue", 10, 300, &[], &[], None),
            asset("c", "pa", "dialogue", 10, 200, &[], &[], None),
        ];
        let desc = sort_asset_ids(
            &spec(SortTarget::CreatedAt, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(desc, vec!["b", "c", "a"]);
        let asc = sort_asset_ids(
            &spec(SortTarget::CreatedAt, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(asc, vec!["a", "c", "b"]);
    }

    // --- updated_at -----------------------------------------------------

    /// Most-recently-changed first, and `reverse` reads it oldest-change
    /// first.
    ///
    /// The fixture puts the three time axes in three different orders:
    /// occurrence ascends `a, b, c`, ingest descends, modification runs
    /// `b, a, c`. Only the modification order produces the expected
    /// answer, so a branch reading either neighbouring column — or
    /// falling through to the `occurred_at DESC` tie-break — fails.
    #[test]
    fn updated_at_default_is_most_recently_changed_first() {
        let assets = vec![
            touched("a", 100, 300, 200),
            touched("b", 200, 200, 300),
            touched("c", 300, 100, 100),
        ];
        let desc = sort_asset_ids(
            &spec(SortTarget::UpdatedAt, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(
            desc,
            vec!["b", "a", "c"],
            "modification order, which agrees with neither occurrence nor ingest order"
        );
        let asc = sort_asset_ids(
            &spec(SortTarget::UpdatedAt, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(asc, vec!["c", "a", "b"]);
    }

    /// Equal modification stamps fall to the shared tie-break chain
    /// (`occurred_at DESC`, then id). This is what makes the axis usable
    /// as a sync cursor: rows written in the same millisecond still come
    /// back in a fixed order, so a consumer replaying that stamp sees the
    /// same page rather than an arbitrary permutation of it.
    #[test]
    fn equal_modification_stamps_fall_to_the_shared_tie_breaks() {
        let assets = vec![
            touched("a", 100, 1, 500),
            touched("b", 300, 1, 500),
            touched("c", 200, 1, 500),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::UpdatedAt, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["b", "c", "a"]);
    }

    /// `order` is inert on this axis, as documented on
    /// [`SortTarget::UpdatedAt`] — the target already is the ordering.
    #[test]
    fn order_is_inert_on_the_modification_axis() {
        let assets = vec![
            touched("a", 100, 300, 200),
            touched("b", 200, 200, 300),
            touched("c", 300, 100, 100),
        ];
        for order in [SortOrder::Alpha, SortOrder::Ordered, SortOrder::Updated] {
            let out = sort_asset_ids(&spec(SortTarget::UpdatedAt, order, false), &assets, &ctx());
            assert_eq!(
                out,
                vec!["b", "a", "c"],
                "order {order:?} changed the answer"
            );
        }
    }

    // --- persona --------------------------------------------------------

    #[test]
    fn persona_ordered_follows_display_order() {
        // Display order is pc, pa, pb. occurred equal so only axis matters.
        let assets = vec![
            asset("a", "pa", "dialogue", 10, 1, &[], &[], None),
            asset("b", "pb", "dialogue", 10, 1, &[], &[], None),
            asset("c", "pc", "dialogue", 10, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Persona, SortOrder::Ordered, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["c", "a", "b"]);
        let rev = sort_asset_ids(
            &spec(SortTarget::Persona, SortOrder::Ordered, true),
            &assets,
            &ctx(),
        );
        assert_eq!(rev, vec!["b", "a", "c"]);
    }

    #[test]
    fn persona_alpha_follows_name() {
        // Names: pa=Aiko, pb=Ben, pc=Cara → alpha asc = pa, pb, pc.
        let assets = vec![
            asset("c", "pc", "dialogue", 10, 1, &[], &[], None),
            asset("a", "pa", "dialogue", 10, 1, &[], &[], None),
            asset("b", "pb", "dialogue", 10, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Persona, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn persona_updated_ranks_by_bucket_recency() {
        // pa bucket max occurred = 500 (via a2), pb max = 200. So pa first.
        let assets = vec![
            asset("b1", "pb", "dialogue", 200, 1, &[], &[], None),
            asset("a1", "pa", "dialogue", 100, 1, &[], &[], None),
            asset("a2", "pa", "dialogue", 500, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Persona, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        // pa bucket (a2 desc then a1) before pb bucket.
        assert_eq!(out, vec!["a2", "a1", "b1"]);
    }

    // --- modality -------------------------------------------------------

    #[test]
    fn modality_ordered_follows_canonical_order() {
        // Canonical: dialogue < journal < media.
        let assets = vec![
            asset("m", "pa", "media", 10, 1, &[], &[], None),
            asset("d", "pa", "dialogue", 10, 1, &[], &[], None),
            asset("j", "pa", "journal", 10, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Modality, SortOrder::Ordered, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["d", "j", "m"]);
    }

    #[test]
    fn modality_unknown_slug_sorts_to_tail() {
        let assets = vec![
            asset("u", "pa", "zzz_unknown", 10, 1, &[], &[], None),
            asset("d", "pa", "dialogue", 10, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Modality, SortOrder::Ordered, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["d", "u"]);
    }

    // --- tag ------------------------------------------------------------

    #[test]
    fn tag_alpha_uses_first_user_label() {
        // Internal prefixes skipped; empty labels sink to the tail.
        let assets = vec![
            asset("none", "pa", "dialogue", 10, 1, &[], &[], None),
            asset("beta", "pa", "dialogue", 10, 1, &["beta"], &[], None),
            asset(
                "alpha",
                "pa",
                "dialogue",
                10,
                1,
                &["persona:sys", "alpha"],
                &[],
                None,
            ),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Tag, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["alpha", "beta", "none"]);
    }

    #[test]
    fn tag_updated_ranks_by_tag_recency() {
        // `beta` bucket max = 400, `alpha` bucket max = 100 — so recency
        // puts beta first while the name order puts alpha first. The two
        // options used to return the same list, which made the choice
        // between them inert.
        let assets = vec![
            asset("a1", "pa", "dialogue", 100, 1, &["alpha"], &[], None),
            asset("b1", "pa", "dialogue", 400, 1, &["beta"], &[], None),
            asset("b2", "pa", "dialogue", 50, 1, &["beta"], &[], None),
        ];
        let by_updated = sort_asset_ids(
            &spec(SortTarget::Tag, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        // beta bucket first (recency 400), b1 before b2 by occurred desc.
        assert_eq!(by_updated, vec!["b1", "b2", "a1"]);

        let by_alpha = sort_asset_ids(
            &spec(SortTarget::Tag, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        assert_eq!(by_alpha, vec!["a1", "b1", "b2"]);
    }

    // --- group ----------------------------------------------------------

    #[test]
    fn group_alpha_buckets_by_name() {
        // g1=Alpha, g2=Beta, unfiled → tail sentinel.
        let assets = vec![
            asset("u", "pa", "dialogue", 10, 1, &[], &[], None),
            asset("beta", "pa", "dialogue", 10, 1, &[], &["g2"], None),
            asset("alpha", "pa", "dialogue", 10, 1, &[], &["g1"], None),
        ];
        let by_alpha = sort_asset_ids(
            &spec(SortTarget::Group, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        assert_eq!(by_alpha, vec!["alpha", "beta", "u"]);
    }

    #[test]
    fn group_ordered_follows_the_hand_arrangement() {
        // Two buckets, each hand-arranged against occurrence order: g1
        // (Alpha) holds a2 before a1, g2 (Beta) holds b2 before b1. Every
        // card shares an `occurred_at`-descending pull in the opposite
        // direction, so an `alpha` result (which falls to that tie-break)
        // and an `ordered` result cannot coincide by accident.
        let assets = vec![
            filed_at("a1", "g1", 1, 400),
            filed_at("b1", "g2", 1, 300),
            filed_at("a2", "g1", 0, 200),
            filed_at("b2", "g2", 0, 100),
        ];
        let ordered = sort_asset_ids(
            &spec(SortTarget::Group, SortOrder::Ordered, false),
            &assets,
            &ctx(),
        );
        assert_eq!(ordered, vec!["a2", "a1", "b2", "b1"]);

        // `alpha` keeps the buckets but reads occurrence-first inside
        // them — the axis the arrangement is distinct from.
        let alpha = sort_asset_ids(
            &spec(SortTarget::Group, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        assert_eq!(alpha, vec!["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn group_ordered_puts_unfiled_cards_last() {
        // No slot → tail. The unfiled bucket already sorts last by name;
        // this pins that the `i64::MAX` stand-in does not disturb it.
        let assets = vec![
            asset("u", "pa", "dialogue", 500, 1, &[], &[], None),
            filed_at("a2", "g1", 1, 100),
            filed_at("a1", "g1", 0, 200),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Group, SortOrder::Ordered, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["a1", "a2", "u"]);
    }

    #[test]
    fn group_updated_ranks_by_group_recency() {
        // Beta (g2) bucket max = 400, Alpha (g1) bucket max = 100.
        let assets = vec![
            asset("a1", "pa", "dialogue", 100, 1, &[], &["g1"], None),
            asset("b1", "pa", "dialogue", 400, 1, &[], &["g2"], None),
            asset("b2", "pa", "dialogue", 50, 1, &[], &["g2"], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Group, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        // g2 bucket first (recency 400), b1 before b2 by occurred desc.
        assert_eq!(out, vec!["b1", "b2", "a1"]);
    }

    // --- cover ----------------------------------------------------------

    #[test]
    fn cover_alpha_case_insensitive() {
        let assets = vec![
            asset("z", "pa", "dialogue", 10, 1, &[], &[], Some("Zebra")),
            asset("a", "pa", "dialogue", 10, 1, &[], &[], Some("apple")),
            asset("none", "pa", "dialogue", 10, 1, &[], &[], None),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Cover, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        // "" (None) < "apple" < "zebra" (lower-cased).
        assert_eq!(out, vec!["none", "a", "z"]);
    }

    // --- rating ---------------------------------------------------------
    //
    // Every fixture here crosses the star order against occurrence: the
    // tie-break is `occurred_at DESC`, so a comparator branch that never
    // ran would answer in occurrence order, and each expectation below is
    // a different sequence from that.

    #[test]
    fn rating_default_is_best_first() {
        // Occurrence ascends as the rating descends, so occurrence order
        // is [r1, r5, r3] — nothing like the expectation.
        let assets = vec![
            rated("r3", Some(3), 100),
            rated("r5", Some(5), 200),
            rated("r1", Some(1), 300),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Rating, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["r5", "r3", "r1"]);
    }

    #[test]
    fn rating_reversed_is_worst_first() {
        let assets = vec![
            rated("r3", Some(3), 100),
            rated("r5", Some(5), 200),
            rated("r1", Some(1), 300),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Rating, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["r1", "r3", "r5"]);
    }

    #[test]
    fn unrated_sorts_last_in_both_directions() {
        // The unrated card is the newest of the three, so it leads the
        // tie-break order; and if "unrated" were modelled as zero stars
        // it would lead the reversed page. Both readings are wrong: it
        // has to sit at the tail either way.
        let assets = vec![
            rated("none", None, 900),
            rated("r4", Some(4), 100),
            rated("r2", Some(2), 200),
        ];
        let best_first = sort_asset_ids(
            &spec(SortTarget::Rating, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(best_first, vec!["r4", "r2", "none"]);

        let worst_first = sort_asset_ids(
            &spec(SortTarget::Rating, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(worst_first, vec!["r2", "r4", "none"]);
    }

    #[test]
    fn equal_ratings_fall_to_the_shared_tie_breaks() {
        // Same star count → `occurred_at DESC`, then id ascending.
        let assets = vec![
            rated("b", Some(4), 100),
            rated("a", Some(4), 100),
            rated("c", Some(4), 500),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Rating, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out, vec!["c", "a", "b"]);
    }

    #[test]
    fn order_is_inert_on_the_rating_axis() {
        // Documented on `SortTarget::Rating`: the target is the ordering,
        // and the `order` field is the placeholder every branch ignores.
        let assets = vec![
            rated("r1", Some(1), 300),
            rated("none", None, 400),
            rated("r5", Some(5), 100),
        ];
        for order in [SortOrder::Alpha, SortOrder::Ordered, SortOrder::Updated] {
            let out = sort_asset_ids(&spec(SortTarget::Rating, order, false), &assets, &ctx());
            assert_eq!(
                out,
                vec!["r5", "r1", "none"],
                "order {order:?} moved the axis"
            );
        }
    }

    // --- duration / file size -------------------------------------------
    //
    // One fixture serves both axes, built so that four orders are four
    // different sequences:
    //
    // | id | duration | size  | occurred |
    // |----|----------|-------|----------|
    // | a  |     1 s  | 2 MB  |     100  |
    // | b  |   120 s  | 0.5MB |     200  |
    // | c  |    30 s  | 9 MB  |     300  |
    //
    // The `occurred_at DESC` tie-break — which is what a branch that
    // never ran would answer with — reads `c, b, a`. Longest-first reads
    // `b, c, a`, shortest-first `a, c, b`, largest-first `c, a, b` and
    // smallest-first `b, a, c`. None of the four is the tie-break order,
    // and no two of them coincide, so a branch reading the neighbouring
    // column (the shape a copy-paste between these two arms produces)
    // fails rather than agreeing by luck.
    //
    // The pixel counts stay absent here, and the resolution axis gets its
    // own fixture below: three rows have six orderings, and the tie-break
    // plus the four expectations above already claim five of them, so
    // there is no room left for a pair the `Pixels` branch could disagree
    // with. All-absent still keeps this fixture's teeth against a length
    // or size branch that reads the pixel column — that answers `Equal`
    // and falls to the tie-break, which is none of the four.
    fn metric_fixture() -> Vec<SortableAsset> {
        vec![
            measured("a", Some(1_000), Some(2_000_000), None, 100),
            measured("b", Some(120_000), Some(500_000), None, 200),
            measured("c", Some(30_000), Some(9_000_000), None, 300),
        ]
    }

    /// Three rows for the resolution axis, built so that every wrong
    /// answer is a different sequence from the right one.
    ///
    /// | id | length | size  | pixels | occurred |
    /// |----|--------|-------|--------|----------|
    /// | p  |   3 s  | 5 MB  |  12 MP |    100   |
    /// | q  |   2 s  | 9 MB  |   2 MP |    200   |
    /// | r  |   1 s  | ~0 MB |   8 MP |    300   |
    ///
    /// Largest-first reads `p, r, q` and smallest-first `q, r, p`. The
    /// tie-break a branch that never ran would answer with is `r, q, p`;
    /// the length axis reads `p, q, r` / `r, q, p` and the size axis
    /// `q, p, r` / `r, p, q`. None of those four is either expectation, so
    /// a `Pixels` arm reading a neighbouring column — the shape a
    /// copy-paste between these arms produces — fails rather than
    /// agreeing by luck.
    fn pixel_fixture() -> Vec<SortableAsset> {
        vec![
            measured("p", Some(3_000), Some(5_000_000), Some(12_000_000), 100),
            measured("q", Some(2_000), Some(9_000_000), Some(2_000_000), 200),
            measured("r", Some(1_000), Some(1_000), Some(8_000_000), 300),
        ]
    }

    #[test]
    fn duration_default_is_longest_first() {
        let out = sort_asset_ids(
            &spec(SortTarget::Duration, SortOrder::Updated, false),
            &metric_fixture(),
            &ctx(),
        );
        assert_eq!(out, vec!["b", "c", "a"]);
    }

    #[test]
    fn duration_reversed_is_shortest_first() {
        let out = sort_asset_ids(
            &spec(SortTarget::Duration, SortOrder::Updated, true),
            &metric_fixture(),
            &ctx(),
        );
        assert_eq!(out, vec!["a", "c", "b"]);
    }

    #[test]
    fn file_size_default_is_largest_first() {
        let out = sort_asset_ids(
            &spec(SortTarget::FileSize, SortOrder::Updated, false),
            &metric_fixture(),
            &ctx(),
        );
        assert_eq!(out, vec!["c", "a", "b"]);
    }

    #[test]
    fn file_size_reversed_is_smallest_first() {
        let out = sort_asset_ids(
            &spec(SortTarget::FileSize, SortOrder::Updated, true),
            &metric_fixture(),
            &ctx(),
        );
        assert_eq!(out, vec!["b", "a", "c"]);
    }

    #[test]
    fn pixels_default_is_largest_first() {
        let out = sort_asset_ids(
            &spec(SortTarget::Pixels, SortOrder::Updated, false),
            &pixel_fixture(),
            &ctx(),
        );
        assert_eq!(out, vec!["p", "r", "q"]);
    }

    #[test]
    fn pixels_reversed_is_smallest_first() {
        let out = sort_asset_ids(
            &spec(SortTarget::Pixels, SortOrder::Updated, true),
            &pixel_fixture(),
            &ctx(),
        );
        assert_eq!(out, vec!["q", "r", "p"]);
    }

    /// A row with no length sits at the tail whichever way the axis is
    /// read.
    ///
    /// The lengthless card is the newest of the three, so it leads the
    /// tie-break order; and if "no length" were modelled as zero
    /// milliseconds it would lead the shortest-first page. Both readings
    /// are wrong — "the shortest clip here" is not answered with a still
    /// image — so the fixture makes each of them a different sequence
    /// from the expectation.
    #[test]
    fn assets_with_no_length_sort_last_in_both_directions() {
        let assets = vec![
            measured("still", None, Some(4_000_000), None, 900),
            measured("long", Some(120_000), Some(1_000), None, 100),
            measured("short", Some(5_000), Some(2_000), None, 200),
        ];
        let longest_first = sort_asset_ids(
            &spec(SortTarget::Duration, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(longest_first, vec!["long", "short", "still"]);

        let shortest_first = sort_asset_ids(
            &spec(SortTarget::Duration, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(shortest_first, vec!["short", "long", "still"]);
    }

    /// The same tail rule one column over. `unsized` is again the newest
    /// row, so a branch that never ran would put it first.
    #[test]
    fn assets_with_no_recorded_size_sort_last_in_both_directions() {
        let assets = vec![
            measured("unsized", Some(10_000), None, None, 900),
            measured("big", Some(1_000), Some(9_000_000), None, 100),
            measured("small", Some(2_000), Some(1_000), None, 200),
        ];
        let largest_first = sort_asset_ids(
            &spec(SortTarget::FileSize, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(largest_first, vec!["big", "small", "unsized"]);

        let smallest_first = sort_asset_ids(
            &spec(SortTarget::FileSize, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(smallest_first, vec!["small", "big", "unsized"]);
    }

    /// The same tail rule on the resolution axis. `unmeasured` is again
    /// the newest row, so a branch that never ran would put it first.
    ///
    /// The absent set is widest here: not just material with no pixels,
    /// but everything ingested before the dimension columns existed.
    #[test]
    fn assets_with_no_measured_dimensions_sort_last_in_both_directions() {
        let assets = vec![
            measured("unmeasured", Some(10_000), Some(4_000_000), None, 900),
            measured("wide", Some(1_000), Some(1_000), Some(9_000_000), 100),
            measured("narrow", Some(2_000), Some(2_000), Some(1_000_000), 200),
        ];
        let largest_first = sort_asset_ids(
            &spec(SortTarget::Pixels, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(largest_first, vec!["wide", "narrow", "unmeasured"]);

        let smallest_first = sort_asset_ids(
            &spec(SortTarget::Pixels, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(smallest_first, vec!["narrow", "wide", "unmeasured"]);
    }

    /// A count of zero is a **value**, not the absent state: it orders as
    /// the smallest picture rather than tailing with the unmeasured rows.
    ///
    /// The distinction is the one thing the `Pixels` axis could most
    /// easily get wrong, because the two states look alike from a
    /// distance and `0` is what a stand-in for "unknown" would be spelled
    /// as. The fixture separates them: `zero` and `unmeasured` are the two
    /// newest rows, so a branch folding zero into absent would still put
    /// `zero` at the tail of smallest-first — where the expectation has it
    /// leading.
    ///
    /// Nothing in the column's contract promises a measured side is
    /// positive, so a `0` here reports that a parser answered, which is a
    /// different fact from nobody having asked.
    #[test]
    fn a_measured_zero_is_a_value_and_not_the_absent_state() {
        let assets = vec![
            measured("zero", Some(1), Some(1), Some(0), 900),
            measured("unmeasured", Some(2), Some(2), None, 800),
            measured("small", Some(3), Some(3), Some(1_000), 100),
            measured("big", Some(4), Some(4), Some(9_000_000), 200),
        ];
        let smallest_first = sort_asset_ids(
            &spec(SortTarget::Pixels, SortOrder::Updated, true),
            &assets,
            &ctx(),
        );
        assert_eq!(smallest_first, vec!["zero", "small", "big", "unmeasured"]);

        let largest_first = sort_asset_ids(
            &spec(SortTarget::Pixels, SortOrder::Updated, false),
            &assets,
            &ctx(),
        );
        assert_eq!(largest_first, vec!["big", "small", "zero", "unmeasured"]);
    }

    /// Equal keys — and the all-absent case, which is what a library of
    /// stills looks like on the length axis — fall to the shared
    /// tie-break chain (`occurred_at DESC`, then id ascending) rather
    /// than to an arbitrary permutation.
    ///
    /// The sizes are set, and set to an order (`b, c, a` largest-first)
    /// that is neither the expectation nor the occurrence order, so the
    /// case keeps its teeth against a length branch that reads the size
    /// column: without that, three equal lengths and three sizes rising
    /// with id would agree with the tie-break by accident.
    #[test]
    fn equal_metrics_fall_to_the_shared_tie_breaks() {
        let same_length = vec![
            measured("b", Some(60_000), Some(3), None, 100),
            measured("a", Some(60_000), Some(1), None, 100),
            measured("c", Some(60_000), Some(2), None, 500),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Duration, SortOrder::Updated, false),
            &same_length,
            &ctx(),
        );
        assert_eq!(out, vec!["c", "a", "b"]);

        let no_length = vec![
            measured("b", None, Some(3), None, 100),
            measured("a", None, Some(1), None, 100),
            measured("c", None, Some(2), None, 500),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Duration, SortOrder::Updated, false),
            &no_length,
            &ctx(),
        );
        assert_eq!(out, vec!["c", "a", "b"]);
    }

    /// `order` is inert on all three metric axes, as documented on
    /// [`SortTarget::Duration`], [`SortTarget::FileSize`] and
    /// [`SortTarget::Pixels`] — the target already is the ordering, and
    /// `order` is the placeholder every branch ignores.
    #[test]
    fn order_is_inert_on_the_metric_axes() {
        let assets = metric_fixture();
        let pixels = pixel_fixture();
        for order in [SortOrder::Alpha, SortOrder::Ordered, SortOrder::Updated] {
            let by_length =
                sort_asset_ids(&spec(SortTarget::Duration, order, false), &assets, &ctx());
            assert_eq!(
                by_length,
                vec!["b", "c", "a"],
                "order {order:?} moved the length axis"
            );
            let by_size =
                sort_asset_ids(&spec(SortTarget::FileSize, order, false), &assets, &ctx());
            assert_eq!(
                by_size,
                vec!["c", "a", "b"],
                "order {order:?} moved the size axis"
            );
            // Its own fixture, for the reason `metric_fixture` states:
            // the resolution axis has no spare ordering there.
            let by_pixels =
                sort_asset_ids(&spec(SortTarget::Pixels, order, false), &pixels, &ctx());
            assert_eq!(
                by_pixels,
                vec!["p", "r", "q"],
                "order {order:?} moved the resolution axis"
            );
        }
    }

    // --- structural -----------------------------------------------------

    #[test]
    fn output_is_permutation_of_input() {
        let assets = vec![
            asset("a", "pb", "media", 10, 5, &["x"], &["g2"], Some("q")),
            asset("b", "pa", "dialogue", 20, 3, &[], &[], None),
            asset("c", "pc", "journal", 15, 8, &["y"], &["g1"], Some("r")),
        ];
        let out = sort_asset_ids(
            &spec(SortTarget::Persona, SortOrder::Alpha, false),
            &assets,
            &ctx(),
        );
        assert_eq!(out.len(), 3);
        let mut sorted = out.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let out = sort_asset_ids(
            &spec(SortTarget::OccurredAt, SortOrder::Updated, false),
            &[],
            &ctx(),
        );
        assert!(out.is_empty());
    }

    // --- collation drift ------------------------------------------------
    //
    // The paired half of `card-cmp.test.ts`'s `collation drift` block:
    // same fixtures, same case names, opposite expectations. Each case
    // asserts what *this* side does; the TS file asserts what the UI
    // does. Reading them together is the drift detector — and if the two
    // sides are ever unified on one collator, both files must be edited,
    // which is the property that keeps the divergence from going quiet.
    /// The backend half of the collation contract. Every case here has a
    /// same-named twin in `card-cmp.ts`'s `collation parity` block over
    /// the same fixture strings, and both sides now assert the **same**
    /// resulting order — the pair used to assert opposite ones, which is
    /// what this wave closed. Editing one side alone breaks the other.
    ///
    /// The UI-side expectations were taken from `Intl.Collator("en")` on
    /// JavaScriptCore, the engine WKWebView runs, not from Node alone.
    mod collation_parity {
        use super::*;

        #[test]
        fn case_is_tertiary_letter_is_primary() {
            // apple < Zebra: the letter difference outranks the case one.
            let assets = vec![
                asset("upper", "pa", "dialogue", 10, 1, &["Zebra"], &[], None),
                asset("lower", "pa", "dialogue", 10, 1, &["apple"], &[], None),
            ];
            let out = sort_asset_ids(
                &spec(SortTarget::Tag, SortOrder::Alpha, false),
                &assets,
                &ctx(),
            );
            assert_eq!(out, vec!["lower", "upper"]);
        }

        #[test]
        fn accents_are_a_secondary_difference() {
            // édition < future: 'é' is a secondary difference on 'e', so
            // it stays next to 'e' instead of sorting past 'f'.
            let assets = vec![
                asset(
                    "accented",
                    "pa",
                    "dialogue",
                    10,
                    1,
                    &[],
                    &[],
                    Some("Édition"),
                ),
                asset("plain", "pa", "dialogue", 10, 1, &[], &[], Some("Future")),
            ];
            let out = sort_asset_ids(
                &spec(SortTarget::Cover, SortOrder::Alpha, false),
                &assets,
                &ctx(),
            );
            assert_eq!(out, vec!["accented", "plain"]);
        }

        #[test]
        fn kana_follows_gojuon_across_scripts() {
            // アイ < んご: kana sort by gojūon with hiragana and katakana
            // sharing primary weights, so the script boundary is invisible
            // to the primary key. The highest-impact case — covers are
            // Japanese in practice.
            let assets = vec![
                asset("n", "pa", "dialogue", 10, 1, &[], &[], Some("んご")),
                asset("a", "pa", "dialogue", 10, 1, &[], &[], Some("アイ")),
            ];
            let out = sort_asset_ids(
                &spec(SortTarget::Cover, SortOrder::Alpha, false),
                &assets,
                &ctx(),
            );
            assert_eq!(out, vec!["a", "n"]);
        }

        #[test]
        fn astral_label_stays_before_the_sentinel() {
            // The sentinel's whole job: unlabelled rows last. ICU weights
            // U+FFFF above every assigned code point, so an emoji label
            // (U+1F642) sorts before "no tag" — the grid and the frozen
            // `position` agree on where the tail is.
            let assets = vec![
                asset("emoji", "pa", "dialogue", 10, 1, &["🙂 mood"], &[], None),
                asset("untagged", "pa", "dialogue", 10, 1, &[], &[], None),
            ];
            let out = sort_asset_ids(
                &spec(SortTarget::Tag, SortOrder::Alpha, false),
                &assets,
                &ctx(),
            );
            assert_eq!(out, vec!["emoji", "untagged"]);
        }

        #[test]
        fn ascii_only_keys_are_unchanged() {
            // The control case: ASCII agreed even under code-point order,
            // so it must still agree now.
            let assets = vec![
                asset("c", "pa", "dialogue", 10, 1, &["cherry"], &[], None),
                asset("a", "pa", "dialogue", 10, 1, &["apple"], &[], None),
                asset("b", "pa", "dialogue", 10, 1, &["banana"], &[], None),
            ];
            let out = sort_asset_ids(
                &spec(SortTarget::Tag, SortOrder::Alpha, false),
                &assets,
                &ctx(),
            );
            assert_eq!(out, vec!["a", "b", "c"]);
        }

        #[test]
        fn cjk_extension_is_the_residual_divergence() {
            // KNOWN LIMITATION (module docs): ICU interleaves the Han
            // extension blocks among the URO, ICU4X puts the URO first.
            // `𠮷` is Ext B (U+20BB7), `日本` is URO — so ICU says
            // `𠮷` < `日本` and this side says the reverse. The twin case
            // in `card-cmp.test.ts` asserts the opposite order on purpose;
            // it is the one pair that still does.
            let assets = vec![
                asset("uro", "pa", "dialogue", 10, 1, &[], &[], Some("日本")),
                asset("extb", "pa", "dialogue", 10, 1, &[], &[], Some("𠮷野")),
            ];
            let out = sort_asset_ids(
                &spec(SortTarget::Cover, SortOrder::Alpha, false),
                &assets,
                &ctx(),
            );
            assert_eq!(out, vec!["uro", "extb"]);
        }

        #[test]
        fn collation_is_a_search_parameter() {
            // The knob is per-search, not per-service: the same rows on
            // the same axis order differently once the query names a
            // tailoring. Swedish is the demonstrator because it moves
            // `ä` past `z` at the *primary* level, so the difference
            // survives the default tertiary strength.
            //
            // Japanese would not show anything here: CLDR `ja` demotes
            // the hiragana / katakana distinction to quaternary, so at
            // tertiary the two scripts compare Equal and the id
            // tie-break decides — measured, not assumed.
            let assets = vec![
                asset("umlaut", "pa", "dialogue", 10, 1, &[], &[], Some("Ärlig")),
                asset("z", "pa", "dialogue", 10, 1, &[], &[], Some("Zebra")),
            ];
            let root = spec(SortTarget::Cover, SortOrder::Alpha, false);
            assert_eq!(sort_asset_ids(&root, &assets, &ctx()), vec!["umlaut", "z"]);

            let sv = SortSpec {
                collation: Some("sv".into()),
                ..root
            };
            assert_eq!(sort_asset_ids(&sv, &assets, &ctx()), vec!["z", "umlaut"]);
        }

        /// The shared corpus and the two frozen orders
        /// (`fixtures/collation/`, read by the vitest half and by
        /// `just collation-jsc` as well).
        mod fixtures {
            pub const CORPUS: &str = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/collation/corpus.txt"
            ));
            /// `Intl.Collator("en")` order — what the grid shows.
            pub const GOLDEN_ICU: &str = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/collation/golden-icu.txt"
            ));
            /// ICU4X root order — what gets frozen into `position`.
            pub const GOLDEN_ICU4X: &str = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/collation/golden-icu4x.txt"
            ));

            pub fn lines(raw: &str) -> Vec<&str> {
                raw.lines().filter(|l| !l.is_empty()).collect()
            }
        }

        /// Han extension ideographs, the one documented place the two
        /// collations disagree. Ext A (`㐀`) and Ext B (`𠀀` / `𠮷`).
        const CJK_EXTENSION: [&str; 3] = ["㐀", "𠀀", "𠮷"];

        #[test]
        fn root_reproduces_the_icu4x_golden_order() {
            // The whole corpus, not a two-row fixture: this is what
            // actually pins the collation, and it is the same file the
            // vitest half and the `jsc` recipe read.
            let corpus = fixtures::lines(fixtures::CORPUS);
            let expected = fixtures::lines(fixtures::GOLDEN_ICU4X);
            let coll = collator_for(&SortSpec::default()).expect("root collator");
            let mut got = corpus.clone();
            got.sort_by(|a, b| coll.compare(a, b));
            assert_eq!(got, expected);
        }

        #[test]
        fn the_two_goldens_differ_only_in_cjk_extensions() {
            // The parity statement itself, from this side. The vitest
            // half asserts the same thing over the same files, so
            // neither language can widen the gap on its own.
            let icu = fixtures::lines(fixtures::GOLDEN_ICU);
            let icu4x = fixtures::lines(fixtures::GOLDEN_ICU4X);
            let drop = |xs: &[&str]| -> Vec<String> {
                xs.iter()
                    .filter(|s| !CJK_EXTENSION.contains(s))
                    .map(|s| s.to_string())
                    .collect()
            };
            assert_eq!(drop(&icu4x), drop(&icu));
            // ...and they do still differ, so this cannot pass by the
            // divergence having vanished without the fixtures being
            // regenerated.
            assert_ne!(icu4x, icu);
        }

        #[test]
        fn fixtures_are_consistent_with_each_other() {
            // Guards the fixture files themselves — a hand-edit that
            // drops or duplicates a line would otherwise surface as a
            // confusing ordering failure above.
            let mut corpus = fixtures::lines(fixtures::CORPUS);
            let n = corpus.len();
            corpus.sort_unstable();
            corpus.dedup();
            assert_eq!(corpus.len(), n, "corpus.txt has duplicate entries");
            for golden in [fixtures::GOLDEN_ICU, fixtures::GOLDEN_ICU4X] {
                let mut g = fixtures::lines(golden);
                g.sort_unstable();
                assert_eq!(g, corpus, "golden is not a permutation of the corpus");
            }
        }

        #[test]
        fn nothing_in_the_corpus_sorts_past_the_sentinel() {
            // The property `TAIL_SENTINEL` exists for, stated over the
            // whole corpus rather than one emoji pair.
            assert_eq!(
                fixtures::lines(fixtures::GOLDEN_ICU4X).last(),
                Some(&TAIL_SENTINEL)
            );
            assert_eq!(
                fixtures::lines(fixtures::GOLDEN_ICU).last(),
                Some(&TAIL_SENTINEL)
            );
        }

        #[test]
        fn unusable_collation_tag_fails_loud() {
            // A query group that names a collation it would not get must
            // not quietly freeze a `position` under a different one.
            let bad = SortSpec {
                collation: Some("not a tag".into()),
                ..spec(SortTarget::Tag, SortOrder::Alpha, false)
            };
            let err = super::super::sort_asset_ids(&bad, &[], &ctx())
                .expect_err("an unparseable tag must not fall back silently");
            assert!(matches!(err, DomainError::Validation(_)), "got {err:?}");
        }
    }
}
