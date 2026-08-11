// `restoreQueryGroup` — the boundary where a stored Query Group rule
// (`query_json` v1) becomes active filter state.
//
// The blob is JSON written by an older build, so its `sort.target` is an
// unchecked string however it is typed at the parse site. That matters
// because the axis vocabulary shrank: `msg_count` was removed from the
// union and from the `SortSpec` wire enum after the grid retired it from
// the picker (asset-model v4 P3, `sort.rs`), and a rule frozen while it
// still existed would otherwise land an off-union value in `sortTarget`.
// `ORDER_OPTIONS[target]` in App.svelte is `undefined` for such a value
// and the sorter effect throws on it, so one stale group would break the
// grid rather than itself.
import { beforeEach, describe, expect, it } from "vitest";
import { activeFilter } from "./filter.svelte";

function rule(sort: Record<string, unknown>): string {
  return JSON.stringify({
    v: 1,
    filter: { persona_id: null, modality: null, tag_ids: [], group_ids: [] },
    sort,
    search_text: null,
  });
}

describe("restoreQueryGroup — sort axis validation", () => {
  beforeEach(() => {
    activeFilter.reset();
    activeFilter.sortTarget = "occurred_at";
    activeFilter.sortOrder = "updated";
    activeFilter.sortReverse = false;
    // `reset()` deliberately leaves the search box alone (text + mode
    // belong to it), so the singleton needs them cleared here.
    activeFilter.searchText = "";
    activeFilter.searchFuzzy = true;
  });

  it("applies an axis this build still has", () => {
    // The control: without it the case below could pass against a
    // restore that ignores `sort` entirely.
    expect(
      activeFilter.restoreQueryGroup(
        rule({ target: "group", order: "ordered", reverse: true }),
      ),
    ).toBe(true);
    expect(activeFilter.sortTarget).toBe("group");
    expect(activeFilter.sortOrder).toBe("ordered");
    expect(activeFilter.sortReverse).toBe(true);
  });

  it("falls back to the default axis for a retired one", () => {
    // Starts from a non-default axis so "fell back" and "never wrote"
    // are distinguishable.
    activeFilter.sortTarget = "cover";
    activeFilter.sortOrder = "alpha";
    expect(
      activeFilter.restoreQueryGroup(
        rule({ target: "msg_count", order: "updated", reverse: false }),
      ),
    ).toBe(true);
    expect(activeFilter.sortTarget).toBe("occurred_at");
  });

  it("falls back for an order the union does not carry", () => {
    expect(
      activeFilter.restoreQueryGroup(
        rule({ target: "persona", order: "by_vibes", reverse: false }),
      ),
    ).toBe(true);
    expect(activeFilter.sortTarget).toBe("persona");
    expect(activeFilter.sortOrder).toBe("updated");
  });

  // FORMAT / COLOR are rule fields since 2026-08-03 (v4 P3 carry
  // closed): a rule that carries them restores them, and a rule frozen
  // before they existed clears them — either way the restored filter
  // is exactly the saved query, never a blend with whatever facet was
  // active before the click.
  it("restores the FORMAT / COLOR facets a rule carries", () => {
    expect(
      activeFilter.restoreQueryGroup(
        JSON.stringify({
          v: 1,
          filter: {
            persona_id: "p1",
            format: "video",
            color: "red",
            tag_ids: [],
            group_ids: [],
          },
          sort: { target: "occurred_at", order: "updated", reverse: false },
          search_text: null,
        }),
      ),
    ).toBe(true);
    expect(activeFilter.activeFormat).toBe("video");
    expect(activeFilter.activeColor).toBe("red");
  });

  it("clears the facets for a rule frozen before they existed", () => {
    activeFilter.activeFormat = "image";
    activeFilter.activeColor = "blue";
    expect(
      activeFilter.restoreQueryGroup(
        rule({ target: "occurred_at", order: "updated", reverse: false }),
      ),
    ).toBe(true);
    expect(activeFilter.activeFormat).toBeNull();
    expect(activeFilter.activeColor).toBeNull();
  });

  // A rule's text is an exact predicate by construction (only 🔍 text is
  // persistable, and the backend evaluates the stored rule through
  // `text_match`), so restoring one has to land the box in exact mode.
  // Starting from the default `true` is what makes the assertion mean
  // "the restore wrote it" rather than "nothing touched it".
  it("puts the search box in exact mode for a rule that carries text", () => {
    activeFilter.searchFuzzy = true;
    expect(
      activeFilter.restoreQueryGroup(
        JSON.stringify({
          v: 1,
          filter: { persona_id: "p1", tag_ids: [], group_ids: [] },
          sort: { target: "occurred_at", order: "updated", reverse: false },
          search_text: "stargazing",
        }),
      ),
    ).toBe(true);
    expect(activeFilter.searchText).toBe("stargazing");
    expect(activeFilter.searchFuzzy).toBe(false);
  });

  it("restores the AND tag composition a rule carries", () => {
    expect(
      activeFilter.restoreQueryGroup(
        JSON.stringify({
          v: 1,
          filter: {
            persona_id: "p1",
            tag_ids: ["t1", "t2"],
            tag_match: "all",
            group_ids: [],
          },
          sort: { target: "occurred_at", order: "updated", reverse: false },
          search_text: null,
        }),
      ),
    ).toBe(true);
    expect(activeFilter.tagMatchAll).toBe(true);
  });

  // A rule frozen before either knob existed must restore as the plain
  // defaults, not as a blend with whatever the box was doing before the
  // click. Both fields are set away from their default first so "read
  // back as default" and "was never written" stay distinguishable — and
  // note the two defaults differ in kind: `tagMatchAll` is forced back
  // to OR (the rule's tag set is the whole story), while `searchFuzzy`
  // is left alone because a textless rule says nothing about the mode.
  it("reads a rule without either knob as OR, leaving the search mode alone", () => {
    activeFilter.tagMatchAll = true;
    activeFilter.searchFuzzy = true;
    expect(
      activeFilter.restoreQueryGroup(
        rule({ target: "occurred_at", order: "updated", reverse: false }),
      ),
    ).toBe(true);
    expect(activeFilter.tagMatchAll).toBe(false);
    expect(activeFilter.searchFuzzy).toBe(true);
  });

  // The mode split itself, both halves at once — the invariant being
  // pinned is that the text reaches exactly one domain. If both returned
  // the text the same query would be answered twice; if neither did, a
  // typed query would silently do nothing.
  it("hands the text to exactly one domain per mode", () => {
    activeFilter.searchText = "  stargazing  ";

    activeFilter.searchFuzzy = true;
    expect(activeFilter.retrievalText()).toBe("  stargazing  ");
    expect(activeFilter.textMatch()).toBeNull();

    activeFilter.searchFuzzy = false;
    expect(activeFilter.retrievalText()).toBe("");
    expect(activeFilter.textMatch()).toBe("stargazing"); // trimmed

    // Whitespace alone is not a predicate on either side.
    activeFilter.searchText = "   ";
    expect(activeFilter.textMatch()).toBeNull();
  });

  // --- metric bands: the unit boundary -------------------------------
  //
  // The sidebar holds seconds, MB and MP; the wire takes milliseconds,
  // bytes and a raw pixel count, and the store is the only place any
  // factor is applied. These cases pin both directions, because a
  // conversion that exists in one place can still be wrong in it.

  it("converts the bands to the wire units", () => {
    activeFilter.durationMinSec = 30;
    activeFilter.durationMaxSec = 120;
    activeFilter.sizeMinMb = 1;
    activeFilter.sizeMaxMb = 5;
    activeFilter.pixelsMinMp = 2;
    activeFilter.pixelsMaxMp = 12;
    expect(activeFilter.metricBands()).toEqual({
      duration_min_ms: 30_000,
      duration_max_ms: 120_000,
      // 1024-based, matching `fmtBytes` — the card meta row prints MB
      // under that reading, so the band has to mean the same thing.
      size_min_bytes: 1_048_576,
      size_max_bytes: 5_242_880,
      // 10^6, and deliberately not the factor one line up. A megapixel
      // is a decimal million wherever the word is used, so a 12 MP body
      // is 12,000,000 pixels; borrowing the binary factor here would
      // make the row mean 12,582,912 under the same label.
      pixels_min: 2_000_000,
      pixels_max: 12_000_000,
    });
  });

  it("keeps an open end open rather than sending a zero", () => {
    // `0 s` and "no lower bound" are different requests: the first
    // excludes every still image (naming an end drops NULL rows), the
    // second excludes nothing. A conversion that read `null` as `0`
    // would silently make the sidebar's empty box a filter.
    activeFilter.durationMinSec = null;
    activeFilter.durationMaxSec = 60;
    activeFilter.sizeMinMb = null;
    activeFilter.sizeMaxMb = null;
    activeFilter.pixelsMinMp = null;
    activeFilter.pixelsMaxMp = null;
    expect(activeFilter.metricBands()).toEqual({
      duration_min_ms: null,
      duration_max_ms: 60_000,
      size_min_bytes: null,
      size_max_bytes: null,
      pixels_min: null,
      pixels_max: null,
    });
    expect(activeFilter.hasMetricBand()).toBe(true);
    activeFilter.durationMaxSec = null;
    expect(activeFilter.hasMetricBand()).toBe(false);
    // The resolution row counts towards the same flag — the note and the
    // clear button under the section are keyed on it, so an axis missing
    // here would leave a live band with nothing saying so.
    activeFilter.pixelsMinMp = 8;
    expect(activeFilter.hasMetricBand()).toBe(true);
    activeFilter.pixelsMinMp = null;
    expect(activeFilter.hasMetricBand()).toBe(false);
  });

  it("round-trips a band through a stored rule", () => {
    expect(
      activeFilter.restoreQueryGroup(
        JSON.stringify({
          v: 1,
          filter: {
            persona_id: "p1",
            tag_ids: [],
            group_ids: [],
            duration_min_ms: 30_000,
            duration_max_ms: 120_000,
            size_min_bytes: 1_048_576,
            size_max_bytes: 5_242_880,
            pixels_min: 2_000_000,
            pixels_max: 12_000_000,
          },
          sort: { target: "occurred_at", order: "updated", reverse: false },
          search_text: null,
        }),
      ),
    ).toBe(true);
    // Back in display units, exactly the numbers that were typed — the
    // inputs step in whole seconds / whole MB / whole MP, so the round
    // trip is not approximate.
    expect(activeFilter.durationMinSec).toBe(30);
    expect(activeFilter.durationMaxSec).toBe(120);
    expect(activeFilter.sizeMinMb).toBe(1);
    expect(activeFilter.sizeMaxMb).toBe(5);
    expect(activeFilter.pixelsMinMp).toBe(2);
    expect(activeFilter.pixelsMaxMp).toBe(12);
    expect(activeFilter.metricBands()).toEqual({
      duration_min_ms: 30_000,
      duration_max_ms: 120_000,
      size_min_bytes: 1_048_576,
      size_max_bytes: 5_242_880,
      pixels_min: 2_000_000,
      pixels_max: 12_000_000,
    });
  });

  it("clears the bands for a rule frozen before they existed", () => {
    // Same shape as the FORMAT / COLOR case: a restored filter is the
    // saved query, never a blend with the band that happened to be set
    // before the click — and a leftover band would silently narrow the
    // group's own set.
    activeFilter.durationMinSec = 10;
    activeFilter.sizeMaxMb = 2;
    activeFilter.pixelsMinMp = 8;
    expect(
      activeFilter.restoreQueryGroup(
        rule({ target: "occurred_at", order: "updated", reverse: false }),
      ),
    ).toBe(true);
    expect(activeFilter.durationMinSec).toBeNull();
    expect(activeFilter.sizeMaxMb).toBeNull();
    expect(activeFilter.pixelsMinMp).toBeNull();
  });

  it("supports the three metric sort axes at the string boundary", () => {
    // They are on the union, so a stored rule naming one restores it
    // rather than falling back — the axis is answerable, and the wire
    // enum carries the same tokens (`SortTarget::Duration` / `FileSize`
    // / `Pixels`).
    for (const target of ["duration", "file_size", "pixels"] as const) {
      expect(
        activeFilter.restoreQueryGroup(
          rule({ target, order: "updated", reverse: true }),
        ),
      ).toBe(true);
      expect(activeFilter.sortTarget).toBe(target);
      expect(activeFilter.sortReverse).toBe(true);
    }
  });

  // The restore still reports success: the rest of the rule (persona /
  // modality / tags / groups / search text) is intact, and refusing the
  // whole group over an axis that has a sane default would strand it.
  it("keeps the rest of a rule whose axis was retired", () => {
    expect(
      activeFilter.restoreQueryGroup(
        JSON.stringify({
          v: 1,
          filter: {
            persona_id: "p1",
            modality: "image",
            tag_ids: ["t1"],
            group_ids: [],
          },
          sort: { target: "msg_count", order: "updated", reverse: false },
          search_text: "stargazing",
        }),
      ),
    ).toBe(true);
    expect(activeFilter.activePersona).toBe("p1");
    expect(activeFilter.activeModality).toBe("image");
    expect(Array.from(activeFilter.activeTagIds)).toEqual(["t1"]);
    expect(activeFilter.searchText).toBe("stargazing");
  });
});

// 🎲 Random. Two rules, and both are about what the
// draw is allowed to disturb: it may not run beside a ✦ query (two
// Retrieval-shaped answers, one grid, no way to say which is on screen),
// and it must not survive a "clear everything".
describe("discoverRandom", () => {
  beforeEach(() => {
    activeFilter.reset();
    activeFilter.searchText = "";
    activeFilter.searchFuzzy = true;
  });

  it("drops a ✦ query when the draw starts", () => {
    activeFilter.searchText = "rooftop";
    activeFilter.searchFuzzy = true;

    activeFilter.toggleDiscoverRandom();

    expect(activeFilter.discoverRandom).toBe(true);
    expect(activeFilter.searchText).toBe("");
    // Turning it back off does not restore the text — the box is empty
    // because the query was abandoned, not hidden.
    activeFilter.toggleDiscoverRandom();
    expect(activeFilter.discoverRandom).toBe(false);
    expect(activeFilter.searchText).toBe("");
  });

  it("keeps a 🔍 query, which narrows the pool instead", () => {
    activeFilter.searchText = "rooftop";
    activeFilter.searchFuzzy = false;

    activeFilter.toggleDiscoverRandom();

    expect(activeFilter.discoverRandom).toBe(true);
    expect(activeFilter.searchText).toBe("rooftop");
    // Still a predicate on the wire, so the picks come out of the
    // matching set rather than the whole library.
    expect(activeFilter.textMatch()).toBe("rooftop");
    expect(activeFilter.retrievalText()).toBe("");
  });

  it("is cleared by reset(), and the draw counter is the only reshuffle", () => {
    activeFilter.toggleDiscoverRandom();
    expect(activeFilter.discoverRandom).toBe(true);

    const before = activeFilter.randomNonce;
    activeFilter.reshuffle();
    expect(activeFilter.randomNonce).toBe(before + 1);

    activeFilter.reset();
    expect(activeFilter.discoverRandom).toBe(false);
  });
});
