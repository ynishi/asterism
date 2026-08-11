// encodeToSearch format pin. `encodeToSearch` and
// `decodeFromSearch` are deliberately window-free (see url-adapter.ts
// header) so the URL format and the round trip can be pinned without a
// DOM environment. `hydrateFromURL` / `syncToURL` are the thin
// `window.location` wrappers over them and stay untested until a DOM env
// is deliberately added.
import { beforeEach, describe, expect, it } from "vitest";
import { activeFilter } from "./filter.svelte";
import { decodeFromSearch, encodeToSearch } from "./url-adapter";

function resetFilter() {
  activeFilter.reset();
  activeFilter.searchText = "";
  activeFilter.searchFuzzy = true;
  activeFilter.viewMode = "messages";
  activeFilter.sortTarget = "occurred_at";
  activeFilter.sortOrder = "updated";
  activeFilter.sortReverse = false;
}

describe("encodeToSearch", () => {
  beforeEach(resetFilter);

  it("returns an empty string for the all-default state", () => {
    expect(encodeToSearch()).toBe("");
  });

  it("serialises every non-default axis", () => {
    activeFilter.activePersona = "persona-1";
    activeFilter.activeModality = "image";
    activeFilter.addTag({ id: "t1", name: "alpha" });
    activeFilter.addTag({ id: "t2", name: "beta" });
    activeFilter.activeGroupIds.add("g1");
    activeFilter.searchText = "  hello  ";
    activeFilter.viewMode = "groups";
    const qs = new URLSearchParams(encodeToSearch());
    expect(qs.get("p")).toBe("persona-1");
    expect(qs.get("m")).toBe("image");
    expect(qs.get("t")).toBe("t1,t2");
    expect(qs.get("g")).toBe("g1");
    expect(qs.get("s")).toBe("hello"); // trimmed
    expect(qs.get("v")).toBe("groups");
    expect(qs.get("sort")).toBeNull(); // still default
  });

  // Every facet the sidebar can set has to survive a deep link. A
  // facet once shipped without this and was silently absent from the
  // URL — the same shape of omission that left it out of the reload
  // effect, where selecting a row changed nothing at all.
  it("carries the derived facets (format / color)", () => {
    activeFilter.activeFormat = "image";
    activeFilter.activeColor = "blue";
    const qs = new URLSearchParams(encodeToSearch());
    expect(qs.get("fmt")).toBe("image");
    expect(qs.get("col")).toBe("blue");
  });

  it("drops the sort param at default and encodes non-default + reverse", () => {
    activeFilter.sortTarget = "persona";
    activeFilter.sortOrder = "alpha";
    expect(new URLSearchParams(encodeToSearch()).get("sort")).toBe(
      "persona:alpha",
    );
    activeFilter.sortReverse = true;
    expect(new URLSearchParams(encodeToSearch()).get("sort")).toBe(
      "persona:alpha:r",
    );
  });

  it("encodes reverse-only sort (target/order still default)", () => {
    activeFilter.sortReverse = true;
    expect(new URLSearchParams(encodeToSearch()).get("sort")).toBe(
      "occurred_at:updated:r",
    );
  });

  // The predicate-mode axes: a deep link
  // that carried the chips but not these would reproduce the same
  // selection read a different way — the exact set instead of ranked
  // candidates, OR instead of AND.
  it("carries the predicate modes only when non-default", () => {
    expect(new URLSearchParams(encodeToSearch()).get("sm")).toBeNull();
    expect(new URLSearchParams(encodeToSearch()).get("tm")).toBeNull();
    activeFilter.searchFuzzy = false;
    activeFilter.tagMatchAll = true;
    const qs = new URLSearchParams(encodeToSearch());
    expect(qs.get("sm")).toBe("e");
    expect(qs.get("tm")).toBe("all");
  });
});

describe("encode → decode round trip", () => {
  beforeEach(resetFilter);

  // `relevance` is a frontend-only axis — it has no
  // wire token and cannot be stored in a Query Group. The URL is a
  // different boundary: it carries view state, and `isSortTarget` is
  // the validator on both, so the axis rides it with no adapter change.
  // Pinned because "no change needed" is only true while the guard is
  // the shared union.
  it("carries the relevance axis through the URL", () => {
    activeFilter.sortTarget = "relevance";
    activeFilter.sortOrder = "ordered";
    const qs = encodeToSearch();
    expect(new URLSearchParams(qs).get("sort")).toBe("relevance:ordered");

    resetFilter();
    expect(activeFilter.sortTarget).toBe("occurred_at"); // control

    decodeFromSearch(qs);
    expect(activeFilter.sortTarget).toBe("relevance");
    expect(activeFilter.sortOrder).toBe("ordered");
  });

  it("restores the predicate modes through the URL", () => {
    activeFilter.searchText = "stargazing";
    activeFilter.searchFuzzy = false;
    activeFilter.addTag({ id: "t1", name: "alpha" });
    activeFilter.addTag({ id: "t2", name: "beta" });
    activeFilter.tagMatchAll = true;
    const qs = encodeToSearch();

    resetFilter();
    expect(activeFilter.searchFuzzy).toBe(true); // control: back at the defaults
    expect(activeFilter.tagMatchAll).toBe(false);

    decodeFromSearch(qs);
    expect(activeFilter.searchFuzzy).toBe(false);
    expect(activeFilter.tagMatchAll).toBe(true);
    expect(activeFilter.searchText).toBe("stargazing");
    expect(Array.from(activeFilter.activeTagIds)).toEqual(["t1", "t2"]);
  });

  // A link written by the build before these keys existed has to decode
  // as the defaults rather than inheriting whatever the running session
  // had set — hydrate runs once on mount over live singleton state.
  it("decodes a link without the mode keys as the defaults", () => {
    activeFilter.searchFuzzy = false;
    activeFilter.tagMatchAll = true;
    decodeFromSearch("?p=persona-1&t=t1,t2");
    expect(activeFilter.searchFuzzy).toBe(true);
    expect(activeFilter.tagMatchAll).toBe(false);
    expect(activeFilter.activePersona).toBe("persona-1");
  });
});
