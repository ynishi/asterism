// assetPageCatalog unit tests.
// The api choke point and the dev-only perfBaseline (which touches
// `window` at module init under DEV) are mocked; the catalog's own
// machinery — index→light-card widening, fetch-key skip, stale
// guard, last-good-on-error, hydration batch, patchCard — runs for
// real. The catalog is a singleton without a test reset, so every
// test uses a unique fetch key to stay independent.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssetCardDto } from "../../bindings";
import { api } from "../api";
import { assetPageCatalog } from "./asset-page.svelte";
import { activeFilter } from "./filter.svelte";
import {
  TAIL_SENTINEL,
  sortCards,
  type CardSortLookups,
} from "../sort/card-cmp";
import { textComparator } from "../sort/collation";

vi.mock("../api", () => ({ api: vi.fn() }));
vi.mock("../dev/perf-baseline", () => ({
  perfBaseline: {
    now: () => 0,
    begin: () => null,
    stamp: () => {},
    end: () => {},
    measureToPaint: () => {},
    samples: () => [],
    clear: () => {},
  },
}));

const apiMock = vi.mocked(api);

function idxEntry(id: string) {
  return {
    id,
    persona_id: "p1",
    modality: "image",
    occurred_at_ms: 1,
    labels: ["l"],
    group_ids: ["g"],
    created_at_ms: 2,
    // The index row carries the modification stamp and
    // `indexToLightCard` forwards it, so leaving it out here would put
    // `undefined` on every light card the catalog builds under test.
    updated_at_ms: 3,
    // Same for the two metric columns: the row carries them so the grid
    // can sort on length and size (`AssetIndexEntryDto`).
    duration_ms: 4,
    file_size_bytes: 5,
  };
}

// One index row with named metrics, for the cases that are about the
// two sort keys rather than about the fetch machinery.
function measuredEntry(
  id: string,
  durationMs: number | null,
  fileSizeBytes: number | null,
  occurredAtMs: number,
) {
  return {
    ...idxEntry(id),
    duration_ms: durationMs,
    file_size_bytes: fileSizeBytes,
    occurred_at_ms: occurredAtMs,
  };
}

function idxPage(ids: string[]) {
  return { items: ids.map(idxEntry), offset: 0, limit: 10, total: ids.length };
}

function fullCard(id: string): AssetCardDto {
  return {
    ...idxEntry(id),
    cover: "cover text",
    file_size_bytes: 10,
    source_locator: "/tmp/x.png",
    rating: 3,
    palette: null,
    has_note: true,
    has_thread: false,
    score: null,
    snippet: null,
  } as AssetCardDto;
}

describe("assetPageCatalog", () => {
  beforeEach(() => {
    apiMock.mockReset();
    assetPageCatalog.error = null;
    // `activeFilter` is a singleton shared by every test in this file.
    activeFilter.searchText = "";
    activeFilter.searchFuzzy = true;
  });

  it("widens index entries into light cards (placeholder fields)", async () => {
    apiMock.mockResolvedValueOnce(idxPage(["a1"]));
    const ok = await assetPageCatalog.loadPage({
      filter: { f: 1 },
      searchText: "",
      key: "k-widen",
    });
    expect(ok).toBe(true);
    expect(apiMock).toHaveBeenCalledWith("list_asset_index", {
      query: { f: 1 },
    });
    const card = assetPageCatalog.page!.items[0];
    expect(card.id).toBe("a1");
    expect(card.cover).toBeNull();
    expect(card.source_locator).toBe("");
    expect(card.rating).toBeNull();
    expect(card.has_note).toBe(false);
  });

  // The sort keys are the half that must *not* be placeholdered: the
  // grid orders these light rows itself, so a key the widening drops is
  // an axis that silently answers in arrival order. `updated_at_ms` is
  // the precedent; length and size joined it when the picker gained the
  // two metric axes.
  it("forwards the index row's sort keys instead of placeholdering them", async () => {
    apiMock.mockResolvedValueOnce({
      items: [measuredEntry("m1", 90_000, 4_096, 7)],
      offset: 0,
      limit: 10,
      total: 1,
    });
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-metrics",
    });
    const card = assetPageCatalog.page!.items[0];
    expect(card.duration_ms).toBe(90_000);
    expect(card.file_size_bytes).toBe(4_096);
    expect(card.updated_at_ms).toBe(3);
  });

  // An absent metric has to survive the widening as absent. Folding it
  // into `0` would satisfy shortest-first and put a still image at the
  // head of longest-first, which is the one reading the null exists to
  // prevent (`sort_eval::absent_last_desc`).
  it("keeps an unmeasured metric absent rather than zeroing it", async () => {
    apiMock.mockResolvedValueOnce({
      items: [measuredEntry("m2", null, null, 7)],
      offset: 0,
      limit: 10,
      total: 1,
    });
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-metrics-absent",
    });
    const card = assetPageCatalog.page!.items[0];
    expect(card.duration_ms).toBeNull();
    expect(card.file_size_bytes).toBeNull();
  });

  // The forwarding assertions above name the fields; this one names the
  // consequence, over a fixture where the length axis and the arrival
  // order are *opposites*. Drop `duration_ms` from `indexToLightCard`
  // and every row compares as absent, so the comparator falls through to
  // its `occurred_at DESC` tie-break and answers `arrival` — the shape
  // that got `msg_count` retired, and the reason the picker withheld
  // this axis until the row carried the column.
  it("gives the length axis real keys to order the page by", async () => {
    const lookups: CardSortLookups = {
      personaName: (id) => id,
      personaDisplayOrder: () => 0,
      modalityRank: () => 0,
      primaryGroupName: () => TAIL_SENTINEL,
      compareText: textComparator(null),
    };
    apiMock.mockResolvedValueOnce({
      // As the server hands them over: `occurred_at` DESC.
      items: [
        measuredEntry("brief", 1_000, 2_000_000, 300),
        measuredEntry("still", null, 7_000_000, 200),
        measuredEntry("feature", 120_000, 500_000, 100),
      ],
      offset: 0,
      limit: 10,
      total: 3,
    });
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-metrics-axis",
    });
    const items = assetPageCatalog.page!.items;
    const arrival = items.map((c) => c.id);
    expect(arrival).toEqual(["brief", "still", "feature"]);

    const byLength = sortCards(items, "duration", "updated", false, lookups);
    // Longest first, with the row that has no length at the tail.
    expect(byLength.map((c) => c.id)).toEqual(["feature", "brief", "still"]);
    expect(byLength.map((c) => c.id)).not.toEqual(arrival);

    // The size axis reads the other column, and over this fixture it
    // answers a third sequence — so neither axis can be passing by
    // borrowing the other's key.
    const bySize = sortCards(items, "file_size", "updated", false, lookups);
    expect(bySize.map((c) => c.id)).toEqual(["still", "brief", "feature"]);
  });

  it("skips the round-trip when the fetch key is unchanged", async () => {
    apiMock.mockResolvedValueOnce(idxPage(["a1"]));
    await assetPageCatalog.loadPage({ filter: {}, searchText: "", key: "k-skip" });
    apiMock.mockClear();
    const ok = await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-skip",
    });
    expect(ok).toBe(true);
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("routes non-empty search text to search_assets", async () => {
    apiMock.mockResolvedValueOnce({
      items: [fullCard("s1")],
      offset: 0,
      limit: 10,
      matched: 1,
      candidates_considered: 40,
      truncated: false,
    });
    await assetPageCatalog.loadPage({
      filter: { f: 2 },
      searchText: "  hello ",
      key: "k-search",
    });
    expect(apiMock).toHaveBeenCalledWith("search_assets", {
      query: { text: "hello", filter: { f: 2 } },
    });
    expect(assetPageCatalog.page!.items[0].id).toBe("s1");
  });

  // 🔍 exact search: App puts the text on
  // `filter.text_match` and hands the catalog an empty `searchText`, so
  // the branch taken here must be the listing one. If the text also
  // reached this argument the same query would be answered by Retrieval
  // — ranked candidates, capped at K, `page.total` null — which is the
  // exact claim the exact mode exists to avoid making.
  it("stays on the listing branch when the text rides on filter.text_match", async () => {
    apiMock.mockResolvedValueOnce(idxPage(["e1"]));
    const ok = await assetPageCatalog.loadPage({
      filter: { f: 3, text_match: "hello" },
      searchText: "",
      key: "k-text-match",
    });
    expect(ok).toBe(true);
    expect(apiMock).toHaveBeenCalledWith("list_asset_index", {
      query: { f: 3, text_match: "hello" },
    });
    expect(apiMock).not.toHaveBeenCalledWith(
      "search_assets",
      expect.anything(),
    );
    expect(assetPageCatalog.retrieval).toBeNull();
    expect(assetPageCatalog.page!.total).toBe(1);
  });

  // The shortlist escape hatch (App's `switchToExactSearch`, offered
  // beside the "more beyond the shortlist" hint): flipping `searchFuzzy`
  // to false is the whole mechanism, and it has to move the very next
  // fetch off the capped Retrieval branch onto the exact one. Driven
  // through the store's own `retrievalText()` / `textMatch()` — the same
  // two expressions App feeds the catalog — so this pins the production
  // mapping rather than a copy of it. The button's click wiring itself
  // is App-internal and has no component test to sit in.
  it("moves the next fetch from the shortlist to the exact set when the mode flips", async () => {
    activeFilter.searchText = "cat";
    activeFilter.searchFuzzy = true;

    // Fuzzy: text goes to Retrieval, and the answer is a capped shortlist.
    apiMock.mockResolvedValueOnce({
      items: [fullCard("esc-1")],
      offset: 0,
      limit: 10,
      matched: 1,
      candidates_considered: 500,
      truncated: true,
    });
    await assetPageCatalog.loadPage({
      filter: { text_match: activeFilter.textMatch() },
      searchText: activeFilter.retrievalText(),
      key: "k-escape-fuzzy",
    });
    expect(apiMock).toHaveBeenCalledWith("search_assets", {
      query: { text: "cat", filter: { text_match: null } },
    });
    expect(assetPageCatalog.retrieval?.truncated).toBe(true);

    // The escape hatch, in full: one field flips.
    activeFilter.searchFuzzy = false;
    expect(activeFilter.retrievalText()).toBe("");
    expect(activeFilter.textMatch()).toBe("cat");

    apiMock.mockReset();
    apiMock.mockResolvedValueOnce(idxPage(["esc-1", "esc-2"]));
    await assetPageCatalog.loadPage({
      filter: { text_match: activeFilter.textMatch() },
      searchText: activeFilter.retrievalText(),
      key: "k-escape-exact",
    });
    expect(apiMock).toHaveBeenCalledWith("list_asset_index", {
      query: { text_match: "cat" },
    });
    expect(apiMock).not.toHaveBeenCalledWith(
      "search_assets",
      expect.anything(),
    );
    // No shortlist numbers left to phrase a count with, and a real total.
    expect(assetPageCatalog.retrieval).toBeNull();
    expect(assetPageCatalog.page!.total).toBe(2);
  });

  // The retrieval path has no library-wide count, so `page.total` must
  // stay empty: it is what the count line reads, and a number there
  // would be rendered as "N item(s)" — a claim about the library made
  // from a measurement of the shortlist.
  it("leaves page.total empty on the retrieval path and reports shortlist numbers separately", async () => {
    apiMock.mockResolvedValueOnce({
      items: [fullCard("s1"), fullCard("s2")],
      offset: 0,
      limit: 10,
      matched: 2,
      candidates_considered: 500,
      truncated: true,
    });
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "cat",
      key: "k-retrieval",
    });
    expect(assetPageCatalog.page!.total).toBeNull();
    expect(assetPageCatalog.retrieval).toEqual({
      matched: 2,
      candidatesConsidered: 500,
      truncated: true,
    });
  });

  // Going back to a listing must clear the shortlist numbers, or the
  // count line keeps phrasing an exact page as "N of the top M".
  it("clears the retrieval numbers when the next fetch is a listing", async () => {
    apiMock.mockResolvedValueOnce({
      items: [fullCard("s1")],
      offset: 0,
      limit: 10,
      matched: 1,
      candidates_considered: 3,
      truncated: false,
    });
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "cat",
      key: "k-r1",
    });
    expect(assetPageCatalog.retrieval).not.toBeNull();

    apiMock.mockResolvedValueOnce(idxPage(["a1"]));
    await assetPageCatalog.loadPage({ filter: {}, searchText: "", key: "k-l1" });
    expect(assetPageCatalog.retrieval).toBeNull();
    expect(assetPageCatalog.page!.total).toBe(1);
  });

  // 🎲 Random: a third branch, and the one that must
  // not be mistaken for a page. It answers with the picks plus the size
  // of the pool they came from — no `total`, because there is nothing to
  // page through, and the count line reads `total` to decide whether to
  // print "N item(s)".
  it("routes the random draw to random_assets and reports the pool separately", async () => {
    apiMock.mockResolvedValueOnce({
      items: [fullCard("rnd-1"), fullCard("rnd-2")],
      picked: 2,
      set_total: 4321,
    });
    const ok = await assetPageCatalog.loadPage({
      filter: { f: 9 },
      searchText: "",
      random: true,
      key: "k-random-1",
    });
    expect(ok).toBe(true);
    expect(apiMock).toHaveBeenCalledWith("random_assets", {
      query: { filter: { f: 9 }, k: 100 },
    });
    expect(apiMock).not.toHaveBeenCalledWith(
      "list_asset_index",
      expect.anything(),
    );
    expect(assetPageCatalog.page!.items.map((i) => i.id)).toEqual([
      "rnd-1",
      "rnd-2",
    ]);
    // The pool's size lives where it can be phrased as one, and the
    // field that would be rendered as a library count stays empty.
    expect(assetPageCatalog.page!.total).toBeNull();
    expect(assetPageCatalog.random).toEqual({ picked: 2, setTotal: 4321 });
    expect(assetPageCatalog.retrieval).toBeNull();
  });

  // "Draw again" is a request whose arguments are identical to the last
  // one — the fetch-key cache would swallow it if the nonce were not in
  // the key. The key is composed App-side; this drives the same shape
  // (`fetchKey` puts the nonce in) to pin that a changed nonce reaches
  // the wire and an unchanged one does not.
  it("re-draws when the nonce moves and skips when it does not", async () => {
    const draw = (ids: string[]) => ({
      items: ids.map(fullCard),
      picked: ids.length,
      set_total: 10,
    });
    const key = (nonce: number) => JSON.stringify({ f: {}, q: "", r: nonce });

    apiMock.mockResolvedValueOnce(draw(["first"]));
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      random: true,
      key: key(0),
    });
    expect(assetPageCatalog.page!.items[0].id).toBe("first");

    // Same nonce: nothing moved, so nothing is asked.
    apiMock.mockClear();
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      random: true,
      key: key(0),
    });
    expect(apiMock).not.toHaveBeenCalled();

    // Bumped nonce: a fresh draw, with a genuinely different answer.
    apiMock.mockResolvedValueOnce(draw(["second"]));
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      random: true,
      key: key(1),
    });
    expect(apiMock).toHaveBeenCalledWith("random_assets", {
      query: { filter: {}, k: 100 },
    });
    expect(assetPageCatalog.page!.items[0].id).toBe("second");
  });

  // Turning the draw off has to put the listing's numbers back, or the
  // count line keeps saying "N picks from M" over an exhaustive page.
  it("clears the random numbers when the next fetch is a listing", async () => {
    apiMock.mockResolvedValueOnce({
      items: [fullCard("rnd-x")],
      picked: 1,
      set_total: 7,
    });
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      random: true,
      key: "k-random-off-1",
    });
    expect(assetPageCatalog.random).not.toBeNull();

    apiMock.mockResolvedValueOnce(idxPage(["a1"]));
    await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      random: false,
      key: "k-random-off-2",
    });
    expect(assetPageCatalog.random).toBeNull();
    expect(assetPageCatalog.page!.total).toBe(1);
  });

  it("clears the hydration cache when the fetch key changes", async () => {
    assetPageCatalog.hydration.set("h-old", fullCard("h-old"));
    apiMock.mockResolvedValueOnce(idxPage(["a1"]));
    await assetPageCatalog.loadPage({ filter: {}, searchText: "", key: "k-clear" });
    expect(assetPageCatalog.hydration.size).toBe(0);
  });

  it("keeps the last-good page on fetch error and surfaces it", async () => {
    apiMock.mockResolvedValueOnce(idxPage(["good"]));
    await assetPageCatalog.loadPage({ filter: {}, searchText: "", key: "k-good" });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    apiMock.mockRejectedValueOnce(new Error("net down"));
    const ok = await assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-bad",
    });
    expect(ok).toBe(false);
    expect(assetPageCatalog.page!.items[0].id).toBe("good"); // not blanked
    expect(assetPageCatalog.error).toContain("net down");
    warn.mockRestore();
  });

  it("drops a superseded in-flight load", async () => {
    let resolveFirst!: (v: unknown) => void;
    apiMock.mockImplementationOnce(
      () => new Promise((res) => (resolveFirst = res)),
    );
    const p1 = assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-stale-1",
    });
    apiMock.mockResolvedValueOnce(idxPage(["newer"]));
    const p2 = assetPageCatalog.loadPage({
      filter: {},
      searchText: "",
      key: "k-stale-2",
    });
    expect(await p2).toBe(true);
    resolveFirst(idxPage(["older"]));
    expect(await p1).toBe(false);
    expect(assetPageCatalog.page!.items[0].id).toBe("newer");
  });

  it("reload() re-runs the last query (key-skip semantics preserved)", async () => {
    apiMock.mockResolvedValueOnce(idxPage(["r1"]));
    await assetPageCatalog.loadPage({ filter: {}, searchText: "", key: "k-reload" });
    apiMock.mockClear();
    expect(await assetPageCatalog.reload()).toBe(true);
    expect(apiMock).not.toHaveBeenCalled(); // same key → cache hit
  });

  it("patchCard reflects onto the page item and the hydration hit", async () => {
    apiMock.mockResolvedValueOnce(idxPage(["pc1"]));
    await assetPageCatalog.loadPage({ filter: {}, searchText: "", key: "k-patch" });
    assetPageCatalog.hydration.set("pc1", fullCard("pc1"));
    assetPageCatalog.patchCard("pc1", { rating: 5, has_thread: true });
    expect(assetPageCatalog.page!.items[0].rating).toBe(5);
    const hit = assetPageCatalog.hydration.get("pc1")!;
    expect(hit.rating).toBe(5);
    expect(hit.has_thread).toBe(true);
    expect(hit.cover).toBe("cover text"); // untouched fields survive
  });

  it("hydratedCard queues light cards and swaps in the batch result", async () => {
    vi.useFakeTimers();
    try {
      apiMock.mockResolvedValueOnce([fullCard("hy1")]);
      const light = { ...fullCard("hy1"), cover: null, source_locator: "" };
      const first = assetPageCatalog.hydratedCard(light);
      expect(first).toBe(light); // immediate paint with the light card
      await vi.advanceTimersByTimeAsync(50); // 40 ms debounce flush
      expect(apiMock).toHaveBeenCalledWith("hydrate_cards", {
        ids: ["hy1"],
        viewerSubject: null,
      });
      const second = assetPageCatalog.hydratedCard(light);
      expect(second.cover).toBe("cover text");
    } finally {
      vi.useRealTimers();
    }
  });

  it("hydratedCard leaves fully-populated (search-mode) cards alone", () => {
    const full = fullCard("full-1");
    expect(assetPageCatalog.hydratedCard(full)).toBe(full);
    expect(apiMock).not.toHaveBeenCalled();
  });

  // Trash-view restore / purge remove a row that genuinely left the
  // side being shown. Dropping it locally instead of refetching is
  // what keeps a 6-figure grid from repainting on every click, so the
  // bookkeeping has to be right: the count follows the row out, and
  // the hydration entry goes with it (a later page holding the same
  // id must not paint the stale card).
  it("dropItem removes the row, decrements total, and clears its hydration", async () => {
    const page = {
      items: [fullCard("drop-1"), fullCard("drop-2")],
      offset: 0,
      limit: 10,
      total: 2,
    };
    apiMock.mockResolvedValueOnce(page);
    await assetPageCatalog.loadPage({
      filter: { f: "drop" },
      searchText: "",
      viewerSubject: null,
      key: "drop-key",
    } as never);
    assetPageCatalog.hydration.set("drop-1", fullCard("drop-1"));

    assetPageCatalog.dropItem("drop-1");
    expect(assetPageCatalog.page?.items.map((i) => i.id)).toEqual(["drop-2"]);
    expect(assetPageCatalog.page?.total).toBe(1);
    expect(assetPageCatalog.hydration.has("drop-1")).toBe(false);

    // Unknown id is a no-op — no phantom decrement.
    assetPageCatalog.dropItem("not-here");
    expect(assetPageCatalog.page?.total).toBe(1);
    expect(assetPageCatalog.page?.items.length).toBe(1);
  });

  // The hydration batch is debounced, so a restore / purge can land
  // while a request for that very id is already in flight. Without a
  // guard the response re-inserts the cache entry `dropItem` just
  // removed, and a later page paints the stale card.
  it("dropItem survives a hydration batch that was already in flight", async () => {
    vi.useFakeTimers();
    try {
      const page = {
        items: [fullCard("inflight-1")],
        offset: 0,
        limit: 10,
        total: 1,
      };
      apiMock.mockResolvedValueOnce(page);
      await assetPageCatalog.loadPage({
        filter: { f: "inflight" },
        searchText: "",
        viewerSubject: null,
        key: "inflight-key",
      } as never);

      // Queue a hydration, then drop the row before the batch resolves.
      apiMock.mockResolvedValueOnce([fullCard("inflight-1")]);
      assetPageCatalog.ensureCardHydrated("inflight-1");
      assetPageCatalog.dropItem("inflight-1");
      await vi.advanceTimersByTimeAsync(50);

      expect(assetPageCatalog.hydration.has("inflight-1")).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
