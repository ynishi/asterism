// Asset page catalog — the messages-view grid page + viewport
// hydration cache. Owns everything the
// grid needs to answer "which cards are on the page and what do
// they look like right now": the fetched `AssetPageDto`, the
// index→light-card widening, the viewport hydration machinery,
// in-place card patches after mutations, and the detail-pane
// invalidation signal.
//
// NOT on the Resource primitive (escape hatch — reasons):
//   - error policy diverges: a transient fetch failure must keep
//     the last-good page on screen (Resource resets to initial,
//     which would blank a 6-figure grid on a hiccup).
//   - the fetch is bespoke: two invoke shapes (index vs search),
//     a fetch-key skip cache, and a hydration-clear side effect
//     on key change. The generation guard / loading / error
//     fields below follow the same policy shape.
//
// Scope:
//   - `page`: current `AssetPageDto` (`null` before first load).
//     List mode items are **light cards** — `cover` /
//     `source_locator` / `mime` etc. are placeholders until hydration
//     fills the viewport slice. The sort keys are not placeholdered:
//     whatever the client orders on has to be real on these rows, so
//     `indexToLightCard` forwards every axis the index carries
//     (timestamps, group slot, and now length + size). Search mode
//     items arrive fully hydrated (top-K + snippet + score).
//   - `loadPage(query)`: fetches `random_assets` (`query.random`),
//     `list_asset_index` (empty search) or `search_assets` (text).
//     Skips the round-trip when the
//     caller's `key` matches the last accepted fetch — refetching
//     110 k assets on every view toggle pinned the main thread.
//     Resolves `true` iff the page is fresh (cache hit or accepted
//     write). Query composition (`currentFilter()` + fetch key) is
//     App-side; the catalog never reads `activeFilter` itself.
//   - `reload()`: re-runs the last `loadPage` args (same key-skip
//     semantics). Lets components trigger the App-composed reload
//     path without a callback prop.
//   - `rankOrder` / `loadRankOrder(query)` / `clearRankOrder()`: the
//     `✦ Relevance` axis's key — asset id → rank position, fetched
//     from `search_asset_ids`. An *ordering* hint layered on an exact
//     page, never a membership answer: the page it
//     orders comes from the list path and is unaffected. App decides
//     when it applies (relevance axis + 🔍 exact + non-empty text).
//   - `random`: set while `page` came from the random draw, `null`
//     otherwise. Twin of `retrieval` — the numbers that are true on that
//     path (`picked` / `setTotal`) with none of a listing's (`total` is
//     `null`, because the draw enumerates nothing).
//   - Hydration: `hydratedCard(card)` returns the cached hydrated
//     form or queues a 40 ms-debounced `hydrate_cards` batch and
//     returns the light card. `hydration` / `hydrationTick` back
//     the cache; the in-flight queue is a plain Set because
//     `hydratedCard` runs during template evaluation (writing
//     reactive state there is illegal — profile catalog precedent).
//   - `patchCard(assetId, patch)`: reflects a mutation (rating /
//     has_note / has_thread / labels) onto the in-memory page item
//     + hydration cache so the card re-renders without a grid
//     refetch.
//   - `invalidations` + `invalidateDetail(assetId)`: monotonic
//     signal consumed by DetailPane to purge its per-asset LRU
//     cache after out-of-band changes (drag-drop into a group).
//
// Deliberately NOT owned here:
//   - `fetchKey()` / `currentFilter()` composition — App-side
//     (crosses activeFilter + group closure + content flags).
//   - reload orchestration ($effect wiring) — App-side.
//   - sort / content-flag / recent-drop derivations
//     (`filteredBase` and friends) — App-side for now; they
//     cross-cut reader texts + drag-drop UX state that belongs to
//     later waves.

import type {
  AssetCardDto,
  AssetIndexEntryDto,
  AssetIndexPageDto,
  AssetPageDto,
  RetrievedIdsDto,
  RetrievedPageDto,
  SampledPageDto,
} from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";
import { perfBaseline } from "../dev/perf-baseline";

export interface RankOrderQuery {
  /// `currentFilter()` result — sent verbatim as the retrieval's filter
  /// half, so the rank is computed over the same narrowing the page was.
  filter: Record<string, unknown>;
  /// Raw search text (the catalog trims it).
  text: string;
  /// Cache key composed App-side, same shape as `AssetPageQuery.key`
  /// plus the axis. Skips the round-trip when nothing moved.
  key: string;
}

export interface AssetPageQuery {
  /// `currentFilter()` result — passed opaque to the backend, with one
  /// field read here: `trash` decides the provenance recorded in
  /// `pageIsTrash`, which is what destructive affordances key off.
  filter: Record<string, unknown> & { trash?: string };
  /// Raw search text (the catalog trims it). Ignored on the random
  /// branch, which App keeps empty anyway.
  searchText: string;
  /// `true` = draw a random handful out of `filter` instead of listing
  /// it (the sidebar's 🎲 Random). Takes precedence over `searchText`.
  /// Absent reads as `false` — the two older branches are unchanged, and
  /// a caller that says nothing gets the listing it always got.
  random?: boolean;
  /// Cache key composed App-side from (filter, trimmed search, draw
  /// nonce). Skips the round-trip when nothing moved — which is why the
  /// nonce has to be in it: "draw again" is a request whose *arguments*
  /// are identical to the last one.
  key: string;
}

// How many picks one draw asks for. Mirrors the backend's own default
// (`RANDOM_PICKS_DEFAULT`); sent explicitly so the number the grid is
// built around is visible on this side too, rather than being whatever
// the server happens to prefer.
const RANDOM_PICKS = 100;

// Widens an index entry into an `AssetCardDto`-shaped object so the
// downstream sort / filter / render code sees one uniform shape.
// Fields absent from the index are placeholders — the render helper
// `hydratedCard` swaps in the real values as hydration populates
// the viewport cache.
function indexToLightCard(idx: AssetIndexEntryDto): AssetCardDto {
  return {
    id: idx.id,
    persona_id: idx.persona_id,
    modality: idx.modality,
    occurred_at_ms: idx.occurred_at_ms,
    cover: null,
    labels: idx.labels,
    // Carried, not placeholdered: `file_size`, `duration` and `pixels`
    // sort on these, and sorting happens over these light rows. `null`
    // here meant every row compared as "no value", so the axes answered
    // in `occurred_at DESC` — the shape that got `msg_count` retired.
    // The index row carries the three columns for exactly this reason
    // (`AssetIndexEntryDto`), so forwarding them is the whole of what
    // makes the picker able to offer the axes.
    file_size_bytes: idx.file_size_bytes,
    duration_ms: idx.duration_ms,
    pixel_count: idx.pixel_count,
    mime: null,
    // Placeholder until hydration: the index carries no format, and
    // `"none"` is the reading that promises no player. Anything else
    // would make an unhydrated tile claim a renderer it has no basis
    // for.
    media: "none",
    source_locator: "",
    group_ids: idx.group_ids,
    // Carried, not placeholdered: `Group` + `ordered` sorts on it, and
    // sorting happens over these light rows — a `null` here would flatten
    // the hand arrangement on exactly the pages this path serves.
    primary_group_position: idx.primary_group_position,
    created_at_ms: idx.created_at_ms,
    // Carried: the index row already knows the modification stamp, and a
    // placeholder would make the light row disagree with its hydrated
    // self on the value API consumers page their sync on.
    updated_at_ms: idx.updated_at_ms,
    rating: null,
    palette: null,
    has_note: false,
    has_thread: false,
    // Carried on the index row: the grid lists both roles, and the two
    // render through different card paths, so guessing "item" here
    // would paint every container blank until hydration caught up.
    role: idx.role,
    title: null,
    member_count: 0,
    score: null,
    snippet: null,
    // Placeholders: attribution is not on the index row (it drives no
    // sort or filter axis), so a light card reads as unrecorded until
    // `hydratedCard` swaps in the fetched card. That is why the
    // attribution chips only appear on hydrated rows — and why they are
    // rendered conditionally rather than as "—", which would make the
    // pre-hydration state look like an answer.
    author_kind: null,
    author_subject: null,
    operator_ai: null,
  };
}

// 40 ms debounce coalesces a burst of scroll into one
// `hydrate_cards` round-trip (the VList paints ~20-40 rows).
const HYDRATION_BATCH_MS = 40;

class AssetPageCatalog {
  page = $state<AssetPageDto | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);
  /**
   * Set while `page` came from the retrieval path, `null` while it came
   * from the list path.
   *
   * The two paths answer different questions and only one of them has a
   * total: retrieval looks at a bounded shortlist, so its count is "how
   * many of the candidates we looked at survived the filter", not "how
   * many assets match". Kept beside `page` rather than folded into it
   * so a caller cannot read a shortlist count as a library count —
   * `page.total` is `null` on this path, and the number worth showing
   * lives here with the context that makes it true.
   */
  retrieval = $state<
    { matched: number; candidatesConsidered: number; truncated: boolean } | null
  >(null);
  /**
   * `true` when `page` came back from the trashed side.
   *
   * Separate from the filter's `trashView` on purpose: the toggle is
   * instantaneous and the fetch is not, so anything that offers a
   * destructive action must key off *this* — otherwise Restore /
   * Delete-forever appear over live rows during the round trip, and
   * over live rows indefinitely if the fetch fails and the last-good
   * page is kept.
   */
  pageIsTrash = $state(false);
  /**
   * Set while `page` came from the random draw, `null` otherwise.
   *
   * Same separation as `retrieval`, for a related reason: the draw
   * answers nothing about *which* assets the filter holds, only how many
   * (`setTotal`, exact) and how many of them are on screen (`picked`).
   * `page.total` is `null` on this path so the ordinary count line
   * cannot render a number that would read as "the grid shows N of N".
   */
  random = $state<{ picked: number; setTotal: number } | null>(null);
  /**
   * Asset id → position in the retriever's answer (0 = best match), or
   * `null` when no rank is in play.
   *
   * The `✦ Relevance` axis's key. Deliberately separate from `page`:
   * this does not decide *what* is on screen — the page is the exact
   * (Query-side) answer and stays so — only in which sequence the grid
   * draws it. Ids absent from the map are not
   * non-matches; they are rows the shortlist did not reach, and the
   * comparator keeps them in the default order behind the ranked ones.
   *
   * A plain `Map` rebuilt whole on each fetch (frontend discipline #3):
   * nothing mutates single entries.
   */
  rankOrder = $state<Map<string, number> | null>(null);
  /**
   * How wide the net was for the current `rankOrder` — `null` whenever
   * `rankOrder` is. Feeds the count line's "ranked first" hint, which
   * has to say what it is a fraction of.
   */
  rankInfo = $state<{ candidatesConsidered: number; truncated: boolean } | null>(
    null,
  );
  #generation = 0;
  #key = "";
  #rankGeneration = 0;
  #rankKey = "";
  #lastQuery: AssetPageQuery | null = null;

  // Viewport hydration cache — keyed by asset id so cards scrolled
  // back into view render instantly. `hydrationTick` is the
  // subscription signal for `hydratedCard` (bumped once per batch
  // instead of relying on per-key SvelteMap reads).
  hydration = new SvelteMap<string, AssetCardDto>();
  hydrationTick = $state(0);
  #hydrationQueue = new Set<string>();
  #hydrationTimer: ReturnType<typeof setTimeout> | undefined;
  /// Ids removed by `dropItem` since the last successful fetch. Guards
  /// the in-flight hydration batch from re-caching a card the user just
  /// restored or purged; cleared whenever a fresh page lands, because
  /// at that point the server's answer supersedes any local removal.
  #dropped = new Set<string>();

  // Out-of-band change signal for DetailPane's per-asset cache.
  // `tick` is monotonic so identical `id` triggers a fresh purge.
  invalidations = $state<{ id: string | null; tick: number }>({
    id: null,
    tick: 0,
  });

  invalidateDetail(assetId: string): void {
    this.invalidations = { id: assetId, tick: this.invalidations.tick + 1 };
  }

  /**
   * Removes one asset from the current page without refetching.
   *
   * For trash-view restore / purge: the row genuinely no longer belongs
   * to the side being shown, and refetching a 6-figure page to learn
   * that would repaint the whole grid on every single click. The stored
   * `total` is decremented so the count stays honest, and the hydration
   * cache entry goes too so a later page holding the same id cannot
   * paint a stale card.
   *
   * The id is also pulled out of the pending hydration queue: a batch
   * already in flight would otherwise resolve and re-insert the cache
   * entry this just deleted (~40 ms window), quietly breaking the
   * guarantee above.
   */
  dropItem(assetId: string): void {
    if (this.page === null) return;
    const items = this.page.items.filter((item) => item.id !== assetId);
    if (items.length === this.page.items.length) return;
    this.page = {
      ...this.page,
      items,
      total: this.page.total === null ? null : Math.max(0, this.page.total - 1),
    };
    this.hydration.delete(assetId);
    this.#hydrationQueue.delete(assetId);
    this.#dropped.add(assetId);
    this.hydrationTick += 1;
  }

  async loadPage(query: AssetPageQuery): Promise<boolean> {
    this.#lastQuery = query;
    // Fetch-key cache: skip when the effect fired purely because
    // the view flipped and the filter itself hasn't changed since
    // the last successful fetch.
    if (this.page && this.#key === query.key) return true;
    const gen = ++this.#generation;
    this.loading = true;
    const bl = perfBaseline.begin("loadAssets");
    try {
      const text = query.searchText.trim();
      let result: AssetPageDto;
      let retrieval: AssetPageCatalog["retrieval"] = null;
      let random: AssetPageCatalog["random"] = null;
      if (query.random) {
        // Random draw: the server returns full cards (drawn by id, then
        // hydrated), so no hydration step is needed here either.
        const invokeT0 = perfBaseline.now();
        const drawn = await api<SampledPageDto>("random_assets", {
          query: { filter: query.filter, k: RANDOM_PICKS },
        });
        perfBaseline.stamp(bl, "invoke:random_assets", invokeT0, {
          items: drawn.items.length,
          total: drawn.set_total,
        });
        // The hydration cache is keyed by asset id and the draw crosses
        // filters freely, so clear it on a key change for the same
        // reason the list branch does.
        if (this.#key !== query.key) {
          this.hydration.clear();
          this.hydrationTick += 1;
        }
        // `total: null` on purpose, as on the retrieval branch: the
        // picks are not a page of anything, so a total here would be
        // read as "N of N shown". The pool's real size travels in
        // `random.setTotal`, where the count line can phrase it as what
        // it is — the set the picks came *from*.
        result = {
          items: drawn.items,
          offset: 0,
          limit: RANDOM_PICKS,
          total: null,
        };
        random = { picked: drawn.picked, setTotal: drawn.set_total };
      } else if (text.length === 0) {
        // List mode: 6-figure grids go through the index-only
        // endpoint. Payload is ~10x smaller than the equivalent
        // card page; the frontend hydrates the visible slice as
        // the VList paints.
        const invokeT0 = perfBaseline.now();
        const idxPage = await api<AssetIndexPageDto>("list_asset_index", {
          query: query.filter,
        });
        perfBaseline.stamp(bl, "invoke:list_asset_index", invokeT0, {
          items: idxPage.items.length,
          total: idxPage.total,
        });
        // Clear the cache when the filter set changes so stale
        // hydrations (e.g. cover from a prior persona filter) do
        // not bleed through. Keeping it across identical keys is
        // fine.
        if (this.#key !== query.key) {
          this.hydration.clear();
          this.hydrationTick += 1;
        }
        result = {
          items: idxPage.items.map(indexToLightCard),
          offset: idxPage.offset,
          limit: idxPage.limit,
          total: idxPage.total,
        };
      } else {
        // Retrieval mode: server returns full cards (ranked shortlist +
        // snippet + score), no hydration step needed.
        const invokeT0 = perfBaseline.now();
        const found = await api<RetrievedPageDto>("search_assets", {
          query: { text, filter: query.filter },
        });
        perfBaseline.stamp(bl, "invoke:search_assets", invokeT0, {
          items: found.items.length,
          total: found.matched,
        });
        // `total: null` on purpose. The shortlist has no library-wide
        // count to put there, and leaving the field empty is what stops
        // the count line from rendering one; the numbers that *are* true
        // travel in `retrieval` below.
        result = {
          items: found.items,
          offset: found.offset,
          limit: found.limit,
          total: null,
        };
        retrieval = {
          matched: found.matched,
          candidatesConsidered: found.candidates_considered,
          truncated: found.truncated,
        };
      }
      if (gen !== this.#generation) return false; // superseded in flight
      this.page = result;
      this.retrieval = retrieval;
      this.random = random;
      this.#key = query.key;
      this.#dropped.clear();
      // Provenance of what is actually on screen, not what the filter
      // currently asks for. Destructive affordances key off this: the
      // toggle flips instantly, the fetch does not, and an icon strip
      // that leads the data would offer "delete forever" on a live
      // card. On a failed fetch the last-good page stays, so this stays
      // with it (see the catch branch).
      this.pageIsTrash = query.filter?.trash === "trashed";
      this.error = null;
      return true;
    } catch (e) {
      if (gen !== this.#generation) return false;
      // Keep the last-good page (see file head) — only surface the
      // error.
      this.error = String(e);
      console.warn("[assetPageCatalog.page] load failed:", e);
      return false;
    } finally {
      if (gen === this.#generation) this.loading = false;
      perfBaseline.end(bl);
    }
  }

  /**
   * Fetches the Retrieval rank for the current page's filter + text
   * (`search_asset_ids`) and stores it as an ordering hint.
   *
   * Same generation / key-skip policy as `loadPage`: a superseded
   * response is dropped rather than applied, and an unchanged key does
   * not re-ask. A failure clears the rank instead of keeping the last
   * one — a stale rank is worse than none here, because the grid would
   * order the *new* page by the *old* query's idea of relevance and
   * nothing on screen would say so.
   */
  async loadRankOrder(query: RankOrderQuery): Promise<boolean> {
    const text = query.text.trim();
    if (text.length === 0) {
      this.clearRankOrder();
      return false;
    }
    if (this.rankOrder !== null && this.#rankKey === query.key) return true;
    const gen = ++this.#rankGeneration;
    try {
      const found = await api<RetrievedIdsDto>("search_asset_ids", {
        query: { text, filter: query.filter },
      });
      if (gen !== this.#rankGeneration) return false; // superseded in flight
      const map = new Map<string, number>();
      found.ids.forEach((id, i) => map.set(id, i));
      this.rankOrder = map;
      this.rankInfo = {
        candidatesConsidered: found.candidates_considered,
        truncated: found.truncated,
      };
      this.#rankKey = query.key;
      return true;
    } catch (e) {
      if (gen !== this.#rankGeneration) return false;
      console.warn("[assetPageCatalog.rankOrder] load failed:", e);
      this.clearRankOrder();
      return false;
    }
  }

  /** Drops the rank hint — the grid falls back to the chosen field axis. */
  clearRankOrder(): void {
    // Bump the generation so a fetch already in flight cannot land on
    // top of the clear (the axis may have been switched away mid-round
    // trip, and re-ranking then would be a visible jump).
    this.#rankGeneration += 1;
    this.#rankKey = "";
    if (this.rankOrder !== null) this.rankOrder = null;
    if (this.rankInfo !== null) this.rankInfo = null;
  }

  async reload(): Promise<boolean> {
    if (this.#lastQuery === null) return false;
    return await this.loadPage(this.#lastQuery);
  }

  // Drops the fetch-key cache so the next `loadPage` / `loadAssets`
  // refetches even when the filter is unchanged. Needed after an
  // in-place mutation removes rows the *active* filter selects on —
  // e.g. graduating assets out of the Inbox while the `inbox` label
  // filter is engaged, or landing a freshly-added memo into the
  // current view. Without this the key-skip in `loadPage` returns the
  // stale page. `patchCard` covers in-view edits; this covers
  // membership changes.
  invalidateKey(): void {
    this.#key = "";
  }

  ensureCardHydrated(id: string): void {
    if (this.hydration.has(id) || this.#hydrationQueue.has(id)) return;
    this.#hydrationQueue.add(id);
    if (this.#hydrationTimer !== undefined) return;
    this.#hydrationTimer = setTimeout(async () => {
      this.#hydrationTimer = undefined;
      const batch = Array.from(this.#hydrationQueue);
      this.#hydrationQueue.clear();
      if (batch.length === 0) return;
      try {
        const cards = await api<AssetCardDto[]>("hydrate_cards", {
          ids: batch,
          viewerSubject: null,
        });
        for (const c of cards) {
          // A batch that was already in flight when the card was
          // restored / purged must not put it back in the cache.
          if (this.#dropped.has(c.id)) continue;
          this.hydration.set(c.id, c);
        }
        this.hydrationTick += 1;
      } catch (err) {
        console.warn("hydrate_cards failed", err);
      }
    }, HYDRATION_BATCH_MS);
  }

  // Render-side accessor: returns the hydrated card when the
  // viewport cache has it, otherwise fires a queued hydrate and
  // returns the light card (so the tile paints immediately with
  // placeholders). Called from inside the VList row template so
  // only the visible ~40 cards trigger a fetch.
  hydratedCard(card: AssetCardDto): AssetCardDto {
    void this.hydrationTick;
    const hit = this.hydration.get(card.id);
    if (hit !== undefined) return hit;
    // Search-mode cards arrive fully populated — cover is non-null
    // — so no hydration is needed. List-mode index-widened cards
    // have `cover === null` and `source_locator === ""`.
    if (card.cover === null && card.source_locator === "") {
      this.ensureCardHydrated(card.id);
    }
    return card;
  }

  // Reflect a mutation onto the in-memory page + hydration cache so
  // the card re-renders without an extra grid fetch.
  patchCard(assetId: string, patch: Partial<AssetCardDto>): void {
    if (this.page) {
      for (const it of this.page.items) {
        if (it.id === assetId) Object.assign(it, patch);
      }
    }
    const hit = this.hydration.get(assetId);
    if (hit) this.hydration.set(assetId, { ...hit, ...patch });
    this.hydrationTick += 1;
  }
}

export const assetPageCatalog = new AssetPageCatalog();
