// Shared filter state — the "what the user is currently selecting" axis.
// All UI surfaces that read or mutate the active filter (Sidebar / PersonaStrip
// / SavedQueryModal / Grid header / DetailPane in the future) read from
// this single class instance instead of receiving 15+ props.
//
// Module-boundary rule: this store owns *selections* and
// the names carried alongside them. Catalog data (tag counts, group
// summaries, dir tree, expand state) lives in dedicated stores. Adding
// catalog fields here would collapse the mesh back into a god-store.
//
// Reaction ownership: mutation methods only touch state.
// Reload wiring (loadAssets / loadTagCounts / ...) stays in App.svelte
// `$effect` blocks that track the same fields via `.size` / identity —
// dependency tracking works transparently through the class boundary.
//
// Two fields here are *how a predicate is read*, not *what is selected*
// (they belong to the unified filter band):
//   - `searchFuzzy` picks which domain the search box talks to —
//     Retrieval (ranked candidates, `search_assets`) or Query
//     (`ListAssetsQuery.text_match`, an exact set predicate).
//   - `tagMatchAll` picks how the tag chips compose — OR (default) or
//     AND (`ListAssetsQuery.tag_match`).
// They ride here rather than in App-local state because every surface
// that renders or persists the filter (band toggle, chip row, Query
// Group rule, URL) has to agree on them.
//
// A third pair (`discoverRandom` / `randomNonce`) is *neither* — it is a
// view state that replaces the grid's listing with a random draw.
// It sits here because the same surfaces have to see it (the
// chip row shows it, the count row phrases it, the reload effect keys on
// it), but unlike the two above it is never written to a Query Group or
// the URL: see the field docs.
//
// This module also owns the **unit boundary** for the two metric bands
// (playback length / stored size): the state is held in the units the
// sidebar shows (seconds, MB) and `metricBands()` is the single place
// that turns them into the wire's milliseconds and bytes, with
// `restoreQueryGroup` the single place going back. Components read and
// write the display numbers and never multiply — a second conversion at
// a callsite would be a second definition of what "5 MB" means.

import { SvelteMap, SvelteSet } from "svelte/reactivity";

// Sort axes mirror the App.svelte types (kept in sync manually until
// the sort UI is extracted). `SortTarget` picks the dimension,
// `SortOrder` picks direction inside that dimension, `sortReverse`
// flips whichever order came out.
//
// Every member here has a comparator branch in `../sort/card-cmp`. A
// `msg_count` member used to sit on this union with no branch on either
// side, so selecting it sorted by nothing; it was dropped along with the
// `SortTarget::MsgCount` wire variant when the grid retired it from the
// picker (asset-model v4 P3). The backend enum carries two members this
// union does not (`updated_at` / `rating`) — that direction is inert
// rather than misleading, since the grid never asks for them.
// `duration` / `file_size` were in that list too and are on the union
// now, with a comparator each in `../sort/card-cmp` and an entry each in
// the grid's Sort dropdown. The dropdown withheld them for one wave
// while the rows the grid sorts carried neither column — a data gap
// rather than a vocabulary one, closed by putting both on the index row
// and forwarding them in `indexToLightCard`.
// `relevance` is **frontend-only** and has no wire token. It orders the
// exact page by the Retrieval rank (`search_asset_ids`), which is the
// second composition form of the retrieval / query split:
// membership stays exact, the sequence comes from a retriever that does
// not promise the same answer twice. So it cannot be frozen into a Query
// Group and the backend `SortTarget` deliberately does not carry
// it — a rule naming it would be refused at the wire, which is the
// guard, not a gap.
export type SortTarget =
  | "occurred_at"
  | "created_at"
  | "persona"
  | "modality"
  | "tag"
  | "group"
  | "cover"
  // Playback length (`asset.duration_ms`) and stored size
  // (`asset.file_size_bytes`). Continuous axes rather than buckets, so
  // the grid streams them headerless the way `cover` does.
  | "duration"
  | "file_size"
  // Total pixel count (`width_px * height_px`), continuous like the two
  // above. The *product*, because the columns behind it are coded
  // dimensions recorded before orientation is applied — a "widest first"
  // axis would read an upright phone capture as a landscape one.
  | "pixels"
  | "relevance";

export type SortOrder = "alpha" | "ordered" | "updated";

// Runtime half of the union, for the two boundaries that receive a sort
// axis as an unchecked string: the URL query (`url-adapter`) and a stored
// Query Group rule (`restoreQueryGroup` below). Declared here rather than
// beside either caller so the list and the type are edited together, and
// derived from a `Record<SortTarget, true>` so the guarantee runs both
// ways: dropping a union member without dropping the key is a type
// error, and adding a member without adding the key is one too. (A
// one-way `readonly SortTarget[]` only caught the first — a new axis
// missing from the list would have been silently demoted to the default
// at both boundaries, the exact "accepted then dropped" shape the
// msg_count retirement removed.)
const SORT_TARGET_KEYS = {
  occurred_at: true,
  created_at: true,
  persona: true,
  modality: true,
  tag: true,
  group: true,
  cover: true,
  duration: true,
  file_size: true,
  pixels: true,
  // Accepted at both string boundaries. The URL may name it (the axis
  // is a view state worth restoring); a stored Query Group cannot,
  // because neither writer puts it there — `saveAsQueryGroup` and
  // `updateQueryFromFilter` fall back to the default axis while it is
  // selected. A hand-edited rule that names it anyway restores an axis
  // with no rank map, which `buildCardCmp` answers in the default order.
  relevance: true,
} satisfies Record<SortTarget, true>;

export const SORT_TARGETS: readonly SortTarget[] = Object.keys(
  SORT_TARGET_KEYS,
) as SortTarget[];

export function isSortTarget(v: string): v is SortTarget {
  // `Object.hasOwn`, not `in`: `in` walks the prototype chain, so
  // `"toString"` would pass as a sort axis.
  return Object.hasOwn(SORT_TARGET_KEYS, v);
}

const SORT_ORDER_KEYS = {
  alpha: true,
  ordered: true,
  updated: true,
} satisfies Record<SortOrder, true>;

export const SORT_ORDERS: readonly SortOrder[] = Object.keys(
  SORT_ORDER_KEYS,
) as SortOrder[];

export function isSortOrder(v: string): v is SortOrder {
  return Object.hasOwn(SORT_ORDER_KEYS, v);
}

// Sessions ceased to be a separate view in the session-model refine.
// Dialog modality now
// shows Session tiles inside the Messages grid directly; a "Show
// messages" toggle interleaves Message tiles when the user wants
// the drilled view. Any persisted `"sessions"` value migrates back
// to `"messages"` on hydrate (see url-adapter.ts).
export type ViewMode = "messages" | "groups";

// The two unit conversions between what the sidebar shows and what the
// wire carries. `ListAssetsQuery` takes raw milliseconds and raw bytes
// ("the raw unit, whatever a client chooses to show it in", see the
// field docs); asking someone to type 120000 to mean two minutes is not
// a question a sidebar should ask.
//
// These live here, and only here — `metricBands()` below is the one
// place either factor is applied on the way out and `restoreQueryGroup`
// the one place on the way back. A component that did its own
// arithmetic would be a second definition of what "5 MB" means.
const MS_PER_SECOND = 1000;
// 1024-based, matching `fmtBytes` in `lib/formatters.ts`: the card meta
// row already prints `1.5 MB` under that reading, and a sidebar band
// using 10^6 would mean something different by the same name.
const BYTES_PER_MB = 1024 * 1024;
// 10^6, and deliberately **not** the 1024-based factor beside it. A
// megapixel is a decimal million everywhere the word is used — camera
// bodies, phone spec sheets, print-size tables — so a 12 MP sensor is
// 12,000,000 pixels and not 12,582,912. The two constants disagreeing is
// the correct state: they are different units that happen to share a
// prefix, and unifying them would make one of the two sidebar rows lie.
const PIXELS_PER_MP = 1_000_000;

// Wire form of the three metric bands (`ListAssetsQuery` field names), so
// the query builders can spread it without restating the mapping. Named
// apart from the `MetricBands.svelte` section that edits them: this is
// the query fragment, that is the control.
export type MetricBandQuery = {
  duration_min_ms: number | null;
  duration_max_ms: number | null;
  size_min_bytes: number | null;
  size_max_bytes: number | null;
  pixels_min: number | null;
  pixels_max: number | null;
};

// `value * factor`, keeping `null` (= "this end is open") as `null`.
// Whole units in, whole units out: the inputs step in seconds and MB, so
// the product is exact and a saved rule reads back as the number that
// was typed.
function scaleBand(value: number | null, factor: number): number | null {
  return value === null ? null : Math.round(value * factor);
}

// The inverse, for a rule coming back off the wire. Not rounded — a
// hand-written rule may name a band that is not a whole second or a
// whole MB, and rounding it here would restore a filter that selects a
// different set than the group holds.
function unscaleBand(value: number | null | undefined, factor: number): number | null {
  return value === null || value === undefined ? null : value / factor;
}

class Filter {
  activePersona = $state<string | null>(null);
  activeModality = $state<string | null>(null);
  // FORMAT facet (asset-model v4): a mime top-level type ("image" /
  // "video" / "audio" / "text"). Orthogonal to `activeModality`
  // (semantic classification) — format is a fact of the material,
  // not a user classification. Persisted into Query Group rules like
  // every other axis (v4 P3 carry closed 2026-08-03).
  activeFormat = $state<string | null>(null);
  // COLOR facet: one palette swatch slug ("red" / "blue" / "white" /
  // …). Like `activeFormat` this is a derived fact of the asset, not a
  // user classification, and it composes with every other axis. Single
  // selection — "red or blue" is not a question the sidebar asks yet.
  activeColor = $state<string | null>(null);
  activeLabel = $state<string | null>(null);
  activeTagIds = new SvelteSet<string>();
  activeTagNames = new SvelteMap<string, string>();
  activeGroupIds = new SvelteSet<string>();
  activeGroupNames = new SvelteMap<string, string>();
  activeSessionId = $state<string | null>(null);
  activeSessionLabel = $state<string | null>(null);
  searchText = $state("");

  /**
   * Playback-length band, in **whole seconds**, and stored-size band, in
   * **whole MB** — the units the sidebar shows. `null` = that end is
   * open. `metricBands()` is what turns them into the wire's ms / bytes.
   *
   * Held in display units rather than wire units so the number in the
   * state is the number in the input: a store carrying `120000` while
   * the box reads `120` gives every reader of this class two answers to
   * "what is the band".
   *
   * Naming either end excludes rows whose column is NULL — a still image
   * has no length and a row with no recorded size has nothing to place
   * in a size band (`ListAssetsQuery::duration_min_ms`). That is the
   * backend's rule; it is restated here because it is the one thing
   * about these fields that is not obvious from their names.
   *
   * An inverted band (`min > max`) is a validation error on the wire,
   * not an empty page, and it is deliberately not pre-checked here: the
   * grid keeps its last good page and the error reaches the status line,
   * which is an honest report. Silently withholding the query would
   * leave the sidebar showing a filter the grid is not under.
   */
  durationMinSec = $state<number | null>(null);
  durationMaxSec = $state<number | null>(null);
  sizeMinMb = $state<number | null>(null);
  sizeMaxMb = $state<number | null>(null);

  /**
   * Resolution band, in **whole megapixels** — the unit the sidebar
   * shows, held here for the same reason the two above are: the number
   * in the state is the number in the input.
   *
   * The band is over the *total pixel count*, not over width or height.
   * The columns behind it hold coded dimensions taken before orientation
   * is applied, so a portrait photo is stored as a landscape pair and a
   * band over either side answers backwards for it. The product is what
   * survives the rotation, which is why the sidebar asks "how many
   * pixels" rather than "how wide".
   *
   * Naming either end excludes rows nobody measured — everything
   * ingested before the columns existed, and every material with no
   * pixels to count.
   */
  pixelsMinMp = $state<number | null>(null);
  pixelsMaxMp = $state<number | null>(null);

  /**
   * `true` = ✦ fuzzy: `searchText` goes to Retrieval (`search_assets`),
   * which answers with ranked candidates and no claim of covering the
   * library. `false` = 🔍 exact: the same text goes to
   * `ListAssetsQuery.text_match`, a set predicate that composes with the
   * chips, counts exactly, sorts, and can be frozen into a Query Group
   * (a rule may only carry deterministic predicates).
   *
   * Defaults to `true` so the box behaves as it always has. Deliberately
   * left alone by `reset()`: like `searchText` itself, the mode belongs
   * to the search box, and clearing the chips should not silently move
   * the user's next query to a different domain.
   */
  searchFuzzy = $state(true);

  /**
   * `false` = the tag chips compose with OR (an asset needs any active
   * tag), `true` = AND (it needs every one). Maps to
   * `ListAssetsQuery.tag_match` (`"any"` / `"all"`). Meaningless with
   * fewer than two tags, which is why `ActiveFilters` only offers the
   * checkbox from the second chip on.
   */
  tagMatchAll = $state(false);

  /**
   * `true` = the grid shows a random handful out of the current filter
   * instead of its listing — the sidebar's "🎲 Random".
   * The chips still narrow;
   * only what the grid does with the resulting set changes.
   *
   * **Never persisted.** Neither `saveAsQueryGroup` nor the URL adapter
   * writes it, for the reason `persistableSort()` drops the `relevance`
   * axis: the draw is not reproducible, so freezing it would record a
   * state nothing can restore. Reloading the app, or reopening a saved
   * group, lands on the ordinary listing — which is the honest answer to
   * "what was I looking at", since the picks are gone either way.
   */
  discoverRandom = $state(false);

  /**
   * Bumped to draw again. The catalog folds it into its fetch key, so
   * `randomNonce++` is the whole of "reshuffle" — an identical request
   * that the key nevertheless treats as new, which is exactly right for
   * a wire that answers differently every time.
   *
   * Not persisted either (see `discoverRandom`); a restored counter
   * would name a draw that no longer exists.
   */
  randomNonce = $state(0);

  sortTarget = $state<SortTarget>("occurred_at");
  sortOrder = $state<SortOrder>("updated");
  sortReverse = $state(false);
  // BCP-47 tag for the collation the alphabetical axes compare under —
  // the language knob for sorting, carried with the query rather than
  // in global settings. `null` = CLDR root, the only value the UI and
  // the backend comparator are pinned to reproduce (`lib/sort/collation.ts`,
  // `asterism-core::domain::sort_eval`). Mirrors `SortSpec.collation`.
  sortCollation = $state<string | null>(null);
  viewMode = $state<ViewMode>("messages");

  /**
   * `true` = the grid shows the trash instead of the live set.
   *
   * A view mode rather than a filter chip: it flips which side of
   * `trashed_at` every query reads (`currentFilter()` maps it to the
   * wire `trash` selector), so it composes with the persona / modality
   * / tag filters instead of replacing them — "the trash, for this
   * persona" is a question worth being able to ask.
   *
   * Deliberately not persisted to the URL adapter: landing in the trash
   * after a restart, with destructive actions on screen, is not a state
   * a user should be restored into.
   */
  trashView = $state(false);

  // Reset selection-shape fields. `searchText` is intentionally left
  // untouched: its clear path goes through the debounced input handler
  // in App.svelte (onSearchInput / clearSearch), which owns the timer
  // and the immediate reload. Callers that want a full wipe should
  // call `filter.reset()` then App.svelte's `clearSearch()`.
  //
  // `tagMatchAll` *is* reset, unlike `searchFuzzy`: it qualifies the tag
  // set this method clears, so leaving it on would apply an invisible
  // AND to whatever tags the user picks next.
  //
  // `discoverRandom` is reset for a different reason: it replaces the
  // grid's listing wholesale, so leaving it on would answer "clear
  // everything" with a random draw over the now-empty filter — the one
  // state where the user is least likely to read it as a filter effect.
  reset() {
    this.activePersona = null;
    this.activeModality = null;
    this.activeFormat = null;
    this.activeColor = null;
    this.activeLabel = null;
    this.activeTagIds.clear();
    this.activeTagNames.clear();
    this.activeGroupIds.clear();
    this.activeGroupNames.clear();
    this.activeSessionId = null;
    this.activeSessionLabel = null;
    this.tagMatchAll = false;
    this.durationMinSec = null;
    this.durationMaxSec = null;
    this.sizeMinMb = null;
    this.sizeMaxMb = null;
    this.pixelsMinMp = null;
    this.pixelsMaxMp = null;
    this.discoverRandom = false;
  }

  /**
   * The three metric bands in wire units. The only place the second /
   * MB / MP → ms / byte / pixel conversion happens on the way out; both
   * query builders (the grid's `currentFilter()` and the Query Group
   * rule) spread this rather than converting at the callsite.
   */
  metricBands(): MetricBandQuery {
    return {
      duration_min_ms: scaleBand(this.durationMinSec, MS_PER_SECOND),
      duration_max_ms: scaleBand(this.durationMaxSec, MS_PER_SECOND),
      size_min_bytes: scaleBand(this.sizeMinMb, BYTES_PER_MB),
      size_max_bytes: scaleBand(this.sizeMaxMb, BYTES_PER_MB),
      pixels_min: scaleBand(this.pixelsMinMp, PIXELS_PER_MP),
      pixels_max: scaleBand(this.pixelsMaxMp, PIXELS_PER_MP),
    };
  }

  /** `true` while any metric band names at least one end. */
  hasMetricBand(): boolean {
    return (
      this.durationMinSec !== null ||
      this.durationMaxSec !== null ||
      this.sizeMinMb !== null ||
      this.sizeMaxMb !== null ||
      this.pixelsMinMp !== null ||
      this.pixelsMaxMp !== null
    );
  }

  /**
   * Flips the random draw on or off.
   *
   * Turning it on drops a ✦ fuzzy query: Retrieval would then be
   * answering the grid twice — once as a ranked shortlist and once as a
   * shuffle over it — and neither number on screen could say which set
   * the user is looking at. A 🔍 exact query is left alone on purpose:
   * that text is a `WHERE` clause like any chip, so it narrows the pool
   * the picks come out of, which is a composition that means something
   * ("something random out of the ones containing 'rooftop'").
   */
  toggleDiscoverRandom() {
    this.discoverRandom = !this.discoverRandom;
    if (this.discoverRandom && this.searchFuzzy) this.searchText = "";
  }

  /**
   * Draws again. The catalog keys its fetch on this counter, so bumping
   * it is the whole gesture — the request itself is unchanged, and the
   * wire answers differently anyway.
   */
  reshuffle() {
    this.randomNonce += 1;
  }

  /**
   * The text the Retrieval branch should receive — empty in 🔍 exact
   * mode, because there the text is a set predicate carried by
   * `textMatch()` instead and must not be answered twice.
   *
   * Lives here rather than inline in App.svelte so that "which domain
   * gets the text" is one expression with one test, not a rule restated
   * at each callsite. `switchToExactSearch` (the shortlist escape hatch)
   * changes the branch purely by flipping `searchFuzzy` through these.
   */
  retrievalText(): string {
    return this.searchFuzzy ? this.searchText : "";
  }

  /**
   * The exact text predicate for `ListAssetsQuery.text_match` — null in
   * ✦ fuzzy mode, and null for whitespace-only input in either mode.
   * Trimmed, because a set predicate should not depend on the spaces
   * around the word.
   */
  textMatch(): string | null {
    const t = this.searchText.trim();
    return !this.searchFuzzy && t.length > 0 ? t : null;
  }

  /**
   * The sort a Query Group rule may carry — the current one, unless it
   * is `relevance`, which falls back to the default axis.
   *
   * `✦ Relevance` orders by a Retrieval rank, and Retrieval does not
   * promise the same answer twice. A rule is a persistent
   * definition whose order gets frozen into `asset_bucket.position`, so
   * saving it would record a sequence nobody can reproduce — the same
   * reason a ✦ query's *text* stays out of a rule. The backend
   * enum carries no `relevance` token either, so a rule naming it would
   * be refused at the wire; this makes the writers emit something the
   * group can actually be evaluated with instead.
   *
   * Collation is not included: the two writers disagree about it today
   * (App persists it, `GroupsSection` does not) and that gap is tracked
   * separately — this method exists to make the *axis* rule single.
   */
  persistableSort(): { target: SortTarget; order: SortOrder; reverse: boolean } {
    if (this.sortTarget === "relevance") {
      return { target: "occurred_at", order: "updated", reverse: false };
    }
    return {
      target: this.sortTarget,
      order: this.sortOrder,
      reverse: this.sortReverse,
    };
  }

  addTag(tag: { id: string; name: string }) {
    if (!this.activeTagIds.has(tag.id)) {
      this.activeTagIds.add(tag.id);
      this.activeTagNames.set(tag.id, tag.name);
    }
  }

  removeTag(tagId: string) {
    this.activeTagIds.delete(tagId);
    this.activeTagNames.delete(tagId);
  }

  // Sidebar click: idempotent membership flip. Same-tag click removes,
  // a fresh tag adds. OR semantic is enforced downstream by the domain
  // query (an asset needs at least one of the active tags). Symmetric
  // with `toggleGroup` above.
  toggleTag(tag: { id: string; name: string }) {
    if (this.activeTagIds.has(tag.id)) {
      this.removeTag(tag.id);
    } else {
      this.addTag(tag);
    }
  }

  clearTags() {
    this.activeTagIds.clear();
    this.activeTagNames.clear();
  }

  toggleGroup(g: { id: string; name: string }) {
    if (this.activeGroupIds.has(g.id)) {
      this.activeGroupIds.delete(g.id);
      this.activeGroupNames.delete(g.id);
    } else {
      this.activeGroupIds.add(g.id);
      this.activeGroupNames.set(g.id, g.name);
    }
  }

  removeGroup(id: string) {
    this.activeGroupIds.delete(id);
    this.activeGroupNames.delete(id);
  }

  clearGroups() {
    this.activeGroupIds.clear();
    this.activeGroupNames.clear();
  }

  clearSession() {
    this.activeSessionId = null;
    this.activeSessionLabel = null;
  }

  // Restores a Query Group's saved rule into the active filter — the
  // W3b/W5 successor of `restoreSavedQuery` for the "Expand query
  // into filter" affordance. Reads the
  // v1 `query_json` blob straight off the Group's DTO (parsed shape:
  // `{ v, filter, sort, search_text }`), applies each field to the
  // matching selection state, then writes the Sorter state.
  // Returns `false` when the stored blob is corrupt / unsupported —
  // toast surfacing stays the caller's job.
  restoreQueryGroup(queryJson: string): boolean {
    let rule: {
      v?: number;
      filter?: {
        persona_id?: string | null;
        modality?: string | null;
        format?: string | null;
        color?: string | null;
        tag_ids?: string[];
        // `"any"` / `"all"`, typed loose for the same reason `sort.target`
        // is: this is stored JSON, and a rule frozen before the knob
        // existed carries neither value.
        tag_match?: string | null;
        group_ids?: string[];
        session_id?: string | null;
        label?: string | null;
        // Wire units (ms / bytes / raw pixel count), converted back to
        // the sidebar's seconds / MB / MP below. Absent in every rule
        // frozen before the bands existed; those read back as "both ends
        // open", which is the set they were saved as.
        duration_min_ms?: number | null;
        duration_max_ms?: number | null;
        size_min_bytes?: number | null;
        size_max_bytes?: number | null;
        pixels_min?: number | null;
        pixels_max?: number | null;
      };
      sort?: {
        // `string`, not the unions: this is parsed JSON that may have
        // been frozen by an older build, so the axis is validated below
        // rather than asserted here.
        target: string;
        order: string;
        reverse: boolean;
        // Absent in every rule written before the collation knob
        // existed; those read back as root, matching the backend's
        // `#[serde(default)]`.
        collation?: string | null;
      };
      search_text?: string | null;
    };
    try {
      rule = JSON.parse(queryJson);
    } catch {
      return false;
    }
    if (rule.v !== 1) return false;
    const f = rule.filter ?? {};
    if (f.persona_id && this.activePersona !== f.persona_id) {
      this.activePersona = f.persona_id;
    }
    this.activeModality = f.modality ?? null;
    // FORMAT / COLOR facets are part of the rule since 2026-08-03 (v4
    // P3 carry closed). Rules frozen before that carry neither field
    // and read back as `null` — the same exact-restore the old
    // unconditional clear gave them.
    this.activeFormat = f.format ?? null;
    this.activeColor = f.color ?? null;
    this.activeTagIds.clear();
    for (const id of f.tag_ids ?? []) this.activeTagIds.add(id);
    // Absent in rules frozen before the knob existed; those read back as
    // OR, matching the backend's `TagMatch::default()`.
    this.tagMatchAll = f.tag_match === "all";
    this.activeGroupIds.clear();
    for (const id of f.group_ids ?? []) this.activeGroupIds.add(id);
    this.activeSessionId = f.session_id ?? null;
    this.activeLabel = f.label ?? null;
    // The inverse of `metricBands()`, and the only place the wire →
    // display conversion happens.
    this.durationMinSec = unscaleBand(f.duration_min_ms, MS_PER_SECOND);
    this.durationMaxSec = unscaleBand(f.duration_max_ms, MS_PER_SECOND);
    this.sizeMinMb = unscaleBand(f.size_min_bytes, BYTES_PER_MB);
    this.sizeMaxMb = unscaleBand(f.size_max_bytes, BYTES_PER_MB);
    this.pixelsMinMp = unscaleBand(f.pixels_min, PIXELS_PER_MP);
    this.pixelsMaxMp = unscaleBand(f.pixels_max, PIXELS_PER_MP);
    this.searchText = rule.search_text ?? "";
    // A rule's text is an exact predicate by construction: only the 🔍
    // side is persistable and the backend evaluates the stored
    // rule through `text_match` (`query_group_service.rs`). Restoring it
    // into the ✦ box would run the group's own definition through a
    // different domain and show a different set than the group holds.
    // A rule without text leaves the mode alone — there is nothing to
    // disagree about, and the user's current box keeps its behaviour.
    if ((rule.search_text ?? "").length > 0) {
      this.searchFuzzy = false;
    }
    if (rule.sort) {
      // The blob is stored JSON: its `target` / `order` are typed above
      // but unchecked at runtime, and a rule frozen under an axis this
      // build no longer has (`msg_count`, retired with the grid's
      // Session tiles) would otherwise put an off-union value into
      // `sortTarget`. `ORDER_OPTIONS[target]` in App.svelte is then
      // `undefined` and the sorter effect throws, which takes down more
      // than the one stale group. Fall back to the default axis the same
      // way the URL adapter does with an unknown token.
      this.sortTarget = isSortTarget(rule.sort.target) ? rule.sort.target : "occurred_at";
      this.sortOrder = isSortOrder(rule.sort.order) ? rule.sort.order : "updated";
      this.sortReverse = rule.sort.reverse;
      this.sortCollation = rule.sort.collation ?? null;
    }
    return true;
  }
}

// Exported as `activeFilter` rather than `filter` because App.svelte
// already uses `filter` as a local variable name inside `currentFilter`
// / search helpers (a query-shape object). Keeping the singleton under
// a distinct name avoids shadowing at the callsite.
export const activeFilter = new Filter();
