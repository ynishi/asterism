// Grid comparator tests — and the drift detector for the two-sided sort
// contract.
//
// The backend re-implements this comparator in
// `crates/asterism-core/src/domain/sort_eval.rs` so Query Groups can
// freeze the grid order into `asset_bucket.position`. Two independent
// implementations of one ordering drift silently unless something reads
// both. Nothing can execute Rust from vitest, so the mechanism here is a
// **shared fixture, mirrored test names**: every `it(...)` below carries
// the name of its `sort_eval.rs` counterpart, over the same fixture
// values, so the two files diff by eye and a one-sided edit shows up as
// a missing or renamed case.
//
// The second describe block covers string collation, where the two
// sides used to disagree in four classes and now agree in all but one.
// It asserts the *same* orders `sort_eval.rs`'s `collation_parity`
// module asserts, so a one-sided change to either comparator breaks a
// named pair rather than quietly re-opening the gap.
//
// # What vitest can and cannot say about the shipped runtime
//
// vitest runs on Node (V8 + full ICU); the app runs in WKWebView
// (JavaScriptCore). Those are different ICU builds, so a passing
// assertion here is not automatically true on the device. It was
// checked directly: `Intl.Collator("en")` over a 76-entry corpus
// (Latin / accents / kana / half-width / Han / CJK extensions / astral
// / emoji / symbols / the sentinel) produced byte-identical order on
// `jsc` and on Node. That equality is an empirical fact about today's
// two ICU versions, not a spec guarantee — if the collation cases here
// ever start failing on device but passing in CI, that assumption is
// the first thing to re-measure.
import { describe, expect, it } from "vitest";
import {
  TAIL_SENTINEL,
  buildCardCmp,
  firstUserLabel,
  sortCards,
  type CardSortLookups,
  type SortableCard,
} from "./card-cmp";
import { ROOT_COLLATION_TAG, textComparator } from "./collation";
import type { SortTarget } from "../stores/filter.svelte";

// --- fixtures (mirrors `sort_eval.rs` `asset()` / `ctx()`) -------------

type Card = SortableCard & { id: string };

function card(
  id: string,
  persona: string,
  modality: string,
  occurred: number,
  created: number,
  labels: string[] = [],
  groups: string[] = [],
  cover: string | null = null,
): Card {
  return {
    id,
    persona_id: persona,
    modality,
    occurred_at_ms: occurred,
    created_at_ms: created,
    labels,
    group_ids: groups,
    // Only the `group` + `ordered` cases care; they set it through
    // `filedAt` so every other fixture line stays readable.
    primary_group_position: null,
    cover,
  };
}

// `card` filed into one group at a known slot — the fixture shape for the
// manual-arrangement axis. Mirrors `sort_eval.rs` `filed_at`.
function filedAt(
  id: string,
  group: string,
  position: number,
  occurred: number,
): Card {
  return {
    ...card(id, "pa", "dialogue", occurred, 1, [], [group]),
    primary_group_position: position,
  };
}

// `card` carrying the three metric columns. Mirrors `sort_eval.rs`
// `measured`; `null` on any of them means the row does not carry that
// value (a still image has no length, a row whose bytes were never
// recorded has no size, a row nobody probed has no dimensions), which is
// the third state all three axes push to the tail.
function measured(
  id: string,
  durationMs: number | null,
  fileSizeBytes: number | null,
  pixelCount: number | null,
  occurred: number,
): Card {
  return {
    ...card(id, "pa", "dialogue", occurred, 1),
    duration_ms: durationMs,
    file_size_bytes: fileSizeBytes,
    pixel_count: pixelCount,
  };
}

const PERSONA_NAMES: Record<string, string> = {
  pa: "Aiko",
  pb: "Ben",
  pc: "Cara",
};
// Display order deliberately differs from alphabetical: pc, pa, pb.
const PERSONA_ORDER = ["pc", "pa", "pb"];
const MODALITY_ORDER = ["dialogue", "journal", "media"];
const GROUP_NAMES: Record<string, string> = { g1: "Alpha", g2: "Beta" };

const lookups: CardSortLookups = {
  personaName: (id) => PERSONA_NAMES[id] ?? "?",
  personaDisplayOrder: (id) => {
    const i = PERSONA_ORDER.indexOf(id);
    return i < 0 ? PERSONA_ORDER.length : i;
  },
  modalityRank: (slug) => {
    const i = MODALITY_ORDER.indexOf(slug);
    return i < 0 ? MODALITY_ORDER.length : i;
  },
  primaryGroupName: (groupIds) => {
    const gid = groupIds[0];
    if (!gid) return TAIL_SENTINEL;
    return GROUP_NAMES[gid] ?? TAIL_SENTINEL;
  },
  // Root collation — the same default the backend's `SortSpec.collation:
  // None` selects. Passing `null` (rather than a literal tag) keeps the
  // fixture honest about which value the app actually ships.
  compareText: textComparator(null),
};

function ids(cards: readonly Card[]): string[] {
  return cards.map((c) => c.id);
}

// --- axis parity -------------------------------------------------------

describe("buildCardCmp — axis behaviour mirrored in sort_eval.rs", () => {
  // sort_eval.rs: `occurred_at_default_is_desc`. Both sides sort DESC
  // explicitly now. The UI used to return `null` here and lean on the
  // server's page order, which held only while the page was unfiltered:
  // narrowing to one Group switches the backend to `asset_bucket.position`
  // (`SqliteAssetRepository::page`), and the shortcut then passed a
  // hand-arranged sequence through untouched under a `Sort: Occurred`
  // label. The fixture is the twin's, unsorted on input, so a comparator
  // that declined the axis would hand back `a, b, c`.
  it("occurred_at_default_is_desc", () => {
    const cards = [
      card("a", "pa", "dialogue", 100, 1),
      card("b", "pa", "dialogue", 300, 1),
      card("c", "pa", "dialogue", 200, 1),
    ];
    expect(ids(sortCards(cards, "occurred_at", "updated", false, lookups))).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  // sort_eval.rs: `occurred_at_reversed_is_asc`
  it("occurred_at_reversed_is_asc", () => {
    const cards = [
      card("b", "pa", "dialogue", 300, 1),
      card("c", "pa", "dialogue", 200, 1),
      card("a", "pa", "dialogue", 100, 1),
    ];
    expect(ids(sortCards(cards, "occurred_at", "updated", true, lookups))).toEqual([
      "a",
      "c",
      "b",
    ]);
  });

  // sort_eval.rs: `created_at_desc_then_reversed`
  it("created_at_desc_then_reversed", () => {
    const cards = [
      card("a", "pa", "dialogue", 10, 100),
      card("b", "pa", "dialogue", 10, 300),
      card("c", "pa", "dialogue", 10, 200),
    ];
    expect(ids(sortCards(cards, "created_at", "updated", false, lookups))).toEqual([
      "b",
      "c",
      "a",
    ]);
    expect(ids(sortCards(cards, "created_at", "updated", true, lookups))).toEqual([
      "a",
      "c",
      "b",
    ]);
  });

  // sort_eval.rs: `persona_ordered_follows_display_order`
  it("persona_ordered_follows_display_order", () => {
    const cards = [
      card("a", "pa", "dialogue", 10, 1),
      card("b", "pb", "dialogue", 10, 1),
      card("c", "pc", "dialogue", 10, 1),
    ];
    expect(ids(sortCards(cards, "persona", "ordered", false, lookups))).toEqual([
      "c",
      "a",
      "b",
    ]);
    expect(ids(sortCards(cards, "persona", "ordered", true, lookups))).toEqual([
      "b",
      "a",
      "c",
    ]);
  });

  // sort_eval.rs: `persona_alpha_follows_name`
  it("persona_alpha_follows_name", () => {
    const cards = [
      card("c", "pc", "dialogue", 10, 1),
      card("a", "pa", "dialogue", 10, 1),
      card("b", "pb", "dialogue", 10, 1),
    ];
    expect(ids(sortCards(cards, "persona", "alpha", false, lookups))).toEqual([
      "a",
      "b",
      "c",
    ]);
  });

  // sort_eval.rs: `persona_updated_ranks_by_bucket_recency`
  it("persona_updated_ranks_by_bucket_recency", () => {
    const cards = [
      card("b1", "pb", "dialogue", 200, 1),
      card("a1", "pa", "dialogue", 100, 1),
      card("a2", "pa", "dialogue", 500, 1),
    ];
    expect(ids(sortCards(cards, "persona", "updated", false, lookups))).toEqual([
      "a2",
      "a1",
      "b1",
    ]);
  });

  // sort_eval.rs: `modality_ordered_follows_canonical_order`
  it("modality_ordered_follows_canonical_order", () => {
    const cards = [
      card("m", "pa", "media", 10, 1),
      card("d", "pa", "dialogue", 10, 1),
      card("j", "pa", "journal", 10, 1),
    ];
    expect(ids(sortCards(cards, "modality", "ordered", false, lookups))).toEqual([
      "d",
      "j",
      "m",
    ]);
  });

  // sort_eval.rs: `modality_unknown_slug_sorts_to_tail`
  it("modality_unknown_slug_sorts_to_tail", () => {
    const cards = [
      card("u", "pa", "zzz_unknown", 10, 1),
      card("d", "pa", "dialogue", 10, 1),
    ];
    expect(ids(sortCards(cards, "modality", "ordered", false, lookups))).toEqual([
      "d",
      "u",
    ]);
  });

  // sort_eval.rs: `tag_alpha_uses_first_user_label`
  it("tag_alpha_uses_first_user_label", () => {
    const cards = [
      card("none", "pa", "dialogue", 10, 1),
      card("beta", "pa", "dialogue", 10, 1, ["beta"]),
      card("alpha", "pa", "dialogue", 10, 1, ["persona:sys", "alpha"]),
    ];
    expect(ids(sortCards(cards, "tag", "alpha", false, lookups))).toEqual([
      "alpha",
      "beta",
      "none",
    ]);
  });

  // sort_eval.rs: `tag_updated_ranks_by_tag_recency`.
  // `beta` bucket max = 400, `alpha` bucket max = 100 — recency puts beta
  // first while the name order puts alpha first, so the two options
  // cannot return the same list the way they used to.
  it("tag_updated_ranks_by_tag_recency", () => {
    const cards = [
      card("a1", "pa", "dialogue", 100, 1, ["alpha"]),
      card("b1", "pa", "dialogue", 400, 1, ["beta"]),
      card("b2", "pa", "dialogue", 50, 1, ["beta"]),
    ];
    expect(ids(sortCards(cards, "tag", "updated", false, lookups))).toEqual([
      "b1",
      "b2",
      "a1",
    ]);
    expect(ids(sortCards(cards, "tag", "alpha", false, lookups))).toEqual([
      "a1",
      "b1",
      "b2",
    ]);
  });

  // sort_eval.rs: `group_alpha_buckets_by_name`
  it("group_alpha_buckets_by_name", () => {
    const cards = [
      card("u", "pa", "dialogue", 10, 1),
      card("beta", "pa", "dialogue", 10, 1, [], ["g2"]),
      card("alpha", "pa", "dialogue", 10, 1, [], ["g1"]),
    ];
    expect(ids(sortCards(cards, "group", "alpha", false, lookups))).toEqual([
      "alpha",
      "beta",
      "u",
    ]);
  });

  // sort_eval.rs: `group_ordered_follows_the_hand_arrangement`.
  // Every card pulls occurrence-descending against its slot, so an
  // `alpha` result (which falls to that tie-break) and an `ordered`
  // result cannot coincide by accident.
  it("group_ordered_follows_the_hand_arrangement", () => {
    const cards = [
      filedAt("a1", "g1", 1, 400),
      filedAt("b1", "g2", 1, 300),
      filedAt("a2", "g1", 0, 200),
      filedAt("b2", "g2", 0, 100),
    ];
    expect(ids(sortCards(cards, "group", "ordered", false, lookups))).toEqual([
      "a2",
      "a1",
      "b2",
      "b1",
    ]);
    expect(ids(sortCards(cards, "group", "alpha", false, lookups))).toEqual([
      "a1",
      "a2",
      "b1",
      "b2",
    ]);
  });

  // sort_eval.rs: `group_ordered_puts_unfiled_cards_last`
  it("group_ordered_puts_unfiled_cards_last", () => {
    const cards = [
      card("u", "pa", "dialogue", 500, 1),
      filedAt("a2", "g1", 1, 100),
      filedAt("a1", "g1", 0, 200),
    ];
    expect(ids(sortCards(cards, "group", "ordered", false, lookups))).toEqual([
      "a1",
      "a2",
      "u",
    ]);
  });

  // sort_eval.rs: `group_updated_ranks_by_group_recency`
  it("group_updated_ranks_by_group_recency", () => {
    const cards = [
      card("a1", "pa", "dialogue", 100, 1, [], ["g1"]),
      card("b1", "pa", "dialogue", 400, 1, [], ["g2"]),
      card("b2", "pa", "dialogue", 50, 1, [], ["g2"]),
    ];
    expect(ids(sortCards(cards, "group", "updated", false, lookups))).toEqual([
      "b1",
      "b2",
      "a1",
    ]);
  });

  // sort_eval.rs: `cover_alpha_case_insensitive`
  it("cover_alpha_case_insensitive", () => {
    const cards = [
      card("z", "pa", "dialogue", 10, 1, [], [], "Zebra"),
      card("a", "pa", "dialogue", 10, 1, [], [], "apple"),
      card("none", "pa", "dialogue", 10, 1, [], [], null),
    ];
    expect(ids(sortCards(cards, "cover", "alpha", false, lookups))).toEqual([
      "none",
      "a",
      "z",
    ]);
  });

  // --- duration / file size -------------------------------------------
  //
  // One fixture serves both axes, the same three rows `sort_eval.rs`
  // `metric_fixture` builds:
  //
  // | id | duration | size  | occurred |
  // |----|----------|-------|----------|
  // | a  |     1 s  | 2 MB  |     100  |
  // | b  |   120 s  | 0.5MB |     200  |
  // | c  |    30 s  | 9 MB  |     300  |
  //
  // The `occurred_at DESC` tie-break — what a branch that never ran
  // answers with — reads `c, b, a`. Longest-first reads `b, c, a`,
  // shortest-first `a, c, b`, largest-first `c, a, b`, smallest-first
  // `b, a, c`. None of the four is the tie-break order and no two of
  // them coincide, so a branch reading the neighbouring column fails
  // rather than agreeing by luck.
  //
  // These axes are not in the grid's Sort dropdown yet — the rows the
  // grid sorts carry neither column (see the `duration` branch in
  // `card-cmp.ts`). The cases are the reason that stays a data question
  // rather than also becoming a semantics question: the comparison is
  // pinned to its backend twin now, whichever way the rows are settled.
  //
  // The pixel counts stay absent here, and the resolution axis gets its
  // own fixture below, for the reason the twin states: three rows have
  // six orderings and the tie-break plus the four expectations already
  // claim five of them.
  const metricFixture = () => [
    measured("a", 1_000, 2_000_000, null, 100),
    measured("b", 120_000, 500_000, null, 200),
    measured("c", 30_000, 9_000_000, null, 300),
  ];

  // sort_eval.rs: `pixel_fixture`. Largest-first reads `p, r, q` and
  // smallest-first `q, r, p`; the tie-break is `r, q, p`, the length axis
  // `p, q, r` / `r, q, p` and the size axis `q, p, r` / `r, p, q`. None
  // of those is either expectation, so a branch reading a neighbouring
  // column fails rather than agreeing by luck.
  const pixelFixture = () => [
    measured("p", 3_000, 5_000_000, 12_000_000, 100),
    measured("q", 2_000, 9_000_000, 2_000_000, 200),
    measured("r", 1_000, 1_000, 8_000_000, 300),
  ];

  // sort_eval.rs: `duration_default_is_longest_first`
  it("duration_default_is_longest_first", () => {
    expect(
      ids(sortCards(metricFixture(), "duration", "updated", false, lookups)),
    ).toEqual(["b", "c", "a"]);
  });

  // sort_eval.rs: `duration_reversed_is_shortest_first`
  it("duration_reversed_is_shortest_first", () => {
    expect(
      ids(sortCards(metricFixture(), "duration", "updated", true, lookups)),
    ).toEqual(["a", "c", "b"]);
  });

  // sort_eval.rs: `file_size_default_is_largest_first`
  it("file_size_default_is_largest_first", () => {
    expect(
      ids(sortCards(metricFixture(), "file_size", "updated", false, lookups)),
    ).toEqual(["c", "a", "b"]);
  });

  // sort_eval.rs: `file_size_reversed_is_smallest_first`
  it("file_size_reversed_is_smallest_first", () => {
    expect(
      ids(sortCards(metricFixture(), "file_size", "updated", true, lookups)),
    ).toEqual(["b", "a", "c"]);
  });

  // sort_eval.rs: `pixels_default_is_largest_first`
  it("pixels_default_is_largest_first", () => {
    expect(
      ids(sortCards(pixelFixture(), "pixels", "updated", false, lookups)),
    ).toEqual(["p", "r", "q"]);
  });

  // sort_eval.rs: `pixels_reversed_is_smallest_first`
  it("pixels_reversed_is_smallest_first", () => {
    expect(
      ids(sortCards(pixelFixture(), "pixels", "updated", true, lookups)),
    ).toEqual(["q", "r", "p"]);
  });

  // sort_eval.rs: `assets_with_no_length_sort_last_in_both_directions`.
  // The lengthless card is the newest of the three, so it leads the
  // tie-break order; and if "no length" were modelled as zero it would
  // lead the shortest-first page. Both readings are wrong — "the
  // shortest clip here" is not answered with a still image — so each is
  // a different sequence from the expectation.
  it("assets_with_no_length_sort_last_in_both_directions", () => {
    const cards = [
      measured("still", null, 4_000_000, null, 900),
      measured("long", 120_000, 1_000, null, 100),
      measured("short", 5_000, 2_000, null, 200),
    ];
    expect(ids(sortCards(cards, "duration", "updated", false, lookups))).toEqual([
      "long",
      "short",
      "still",
    ]);
    expect(ids(sortCards(cards, "duration", "updated", true, lookups))).toEqual([
      "short",
      "long",
      "still",
    ]);
  });

  // sort_eval.rs: `assets_with_no_recorded_size_sort_last_in_both_directions`.
  // The same tail rule one column over; `unsized` is again the newest
  // row, so a branch that never ran would put it first.
  it("assets_with_no_recorded_size_sort_last_in_both_directions", () => {
    const cards = [
      measured("unsized", 10_000, null, null, 900),
      measured("big", 1_000, 9_000_000, null, 100),
      measured("small", 2_000, 1_000, null, 200),
    ];
    expect(ids(sortCards(cards, "file_size", "updated", false, lookups))).toEqual([
      "big",
      "small",
      "unsized",
    ]);
    expect(ids(sortCards(cards, "file_size", "updated", true, lookups))).toEqual([
      "small",
      "big",
      "unsized",
    ]);
  });

  // sort_eval.rs: `assets_with_no_measured_dimensions_sort_last_in_both_directions`.
  // The same tail rule again; `unmeasured` is the newest row, so a branch
  // that never ran would put it first.
  it("assets_with_no_measured_dimensions_sort_last_in_both_directions", () => {
    const cards = [
      measured("unmeasured", 10_000, 4_000_000, null, 900),
      measured("wide", 1_000, 1_000, 9_000_000, 100),
      measured("narrow", 2_000, 2_000, 1_000_000, 200),
    ];
    expect(ids(sortCards(cards, "pixels", "updated", false, lookups))).toEqual([
      "wide",
      "narrow",
      "unmeasured",
    ]);
    expect(ids(sortCards(cards, "pixels", "updated", true, lookups))).toEqual([
      "narrow",
      "wide",
      "unmeasured",
    ]);
  });

  // sort_eval.rs: `a_measured_zero_is_a_value_and_not_the_absent_state`.
  // A count of zero orders as the smallest picture rather than tailing
  // with the unmeasured rows. `zero` and `unmeasured` are the two newest
  // rows, so a branch folding one into the other still puts `zero` at the
  // tail of smallest-first — where the expectation has it leading.
  it("a_measured_zero_is_a_value_and_not_the_absent_state", () => {
    const cards = [
      measured("zero", 1, 1, 0, 900),
      measured("unmeasured", 2, 2, null, 800),
      measured("small", 3, 3, 1_000, 100),
      measured("big", 4, 4, 9_000_000, 200),
    ];
    expect(ids(sortCards(cards, "pixels", "updated", true, lookups))).toEqual([
      "zero",
      "small",
      "big",
      "unmeasured",
    ]);
    expect(ids(sortCards(cards, "pixels", "updated", false, lookups))).toEqual([
      "big",
      "small",
      "zero",
      "unmeasured",
    ]);
  });

  // sort_eval.rs: `equal_metrics_fall_to_the_shared_tie_breaks`. Equal
  // keys — and the all-absent case, which is what a library of stills
  // looks like on the length axis — fall to `occurred_at DESC` rather
  // than to an arbitrary permutation. The sizes are set, and set to an
  // order (`b, c, a` largest-first) that is neither the expectation nor
  // the occurrence order, so the case keeps its teeth against a length
  // branch that reads the size column.
  //
  // The twin's rows are the same values in the order `b, a, c`; here
  // they are handed over in id order. That is documented divergence #1
  // (`sort_eval.rs` module docs): the backend appends an `id` tie-break
  // for a deterministic `position`, while this side ends its chain at
  // `occurred_at` and leans on `Array.prototype.sort` stability. `a` and
  // `b` share a timestamp, so the two implementations agree on their
  // relative order only because the input presents them in that order —
  // stating it here rather than letting the permutation read as drift.
  it("equal_metrics_fall_to_the_shared_tie_breaks", () => {
    const sameLength = [
      measured("a", 60_000, 1, null, 100),
      measured("b", 60_000, 3, null, 100),
      measured("c", 60_000, 2, null, 500),
    ];
    expect(ids(sortCards(sameLength, "duration", "updated", false, lookups))).toEqual([
      "c",
      "a",
      "b",
    ]);
    const noLength = [
      measured("a", null, 1, null, 100),
      measured("b", null, 3, null, 100),
      measured("c", null, 2, null, 500),
    ];
    expect(ids(sortCards(noLength, "duration", "updated", false, lookups))).toEqual([
      "c",
      "a",
      "b",
    ]);
  });

  // sort_eval.rs: `order_is_inert_on_the_metric_axes`. The target is the
  // ordering on all three axes; `order` is the placeholder every branch
  // ignores, as on the timestamp axes.
  it("order_is_inert_on_the_metric_axes", () => {
    for (const order of ["alpha", "ordered", "updated"] as const) {
      expect(
        ids(sortCards(metricFixture(), "duration", order, false, lookups)),
        `order ${order} moved the length axis`,
      ).toEqual(["b", "c", "a"]);
      expect(
        ids(sortCards(metricFixture(), "file_size", order, false, lookups)),
        `order ${order} moved the size axis`,
      ).toEqual(["c", "a", "b"]);
      expect(
        ids(sortCards(pixelFixture(), "pixels", order, false, lookups)),
        `order ${order} moved the resolution axis`,
      ).toEqual(["p", "r", "q"]);
    }
  });

  // Absent property and explicit `null` are the same statement — the row
  // does not carry the value. The grid's light rows set both keys now
  // (`indexToLightCard` forwards them), so this shape reaches the
  // comparator from fixtures and from any caller that builds a partial
  // card rather than from the grid; either way it must not read as
  // "zero".
  it("a missing metric property reads as absent, not as zero", () => {
    const cards = [
      // No `duration_ms` key at all — `card()` never sets one.
      card("plain", "pa", "dialogue", 900, 1),
      measured("long", 120_000, 1_000, null, 100),
    ];
    expect(ids(sortCards(cards, "duration", "updated", false, lookups))).toEqual([
      "long",
      "plain",
    ]);
    expect(ids(sortCards(cards, "duration", "updated", true, lookups))).toEqual([
      "long",
      "plain",
    ]);
  });

  // --- relevance (frontend-only axis, no `sort_eval.rs` twin) ---------
  //
  // The rank comes from `search_asset_ids`, not from a card field, so
  // there is nothing for the backend comparator to mirror. The
  // fixture puts the two
  // ranked cards at the *oldest* timestamps, so rank order and the
  // default `occurred_at` DESC order are exact opposites — a comparator
  // that ignored the map, or one that happened to leave the incoming
  // order alone, cannot produce the expected sequence.
  const RANKED = [
    card("r1", "pa", "dialogue", 100, 1),
    card("r2", "pa", "dialogue", 200, 1),
    card("u1", "pa", "dialogue", 400, 1),
    card("u2", "pa", "dialogue", 300, 1),
  ];
  // Best match first: `r1` is position 0.
  const RANK = new Map<string, number>([
    ["r1", 0],
    ["r2", 1],
  ]);

  it("relevance puts ranked cards first, in rank order", () => {
    expect(
      ids(sortCards(RANKED, "relevance", "ordered", false, lookups, RANK)),
    ).toEqual(["r1", "r2", "u1", "u2"]);
  });

  it("relevance keeps out-of-shortlist cards behind, in the default order", () => {
    // `u1` / `u2` are not in the map: the retriever stopped at its
    // ceiling, but the exact predicate still selected them. They keep
    // `occurred_at` DESC among themselves rather than being dropped or
    // interleaved with the ranked run.
    const out = sortCards(RANKED, "relevance", "ordered", false, lookups, RANK);
    expect(ids(out).slice(2)).toEqual(["u1", "u2"]);
  });

  it("relevance without a rank map falls back to the default axis", () => {
    expect(
      ids(sortCards(RANKED, "relevance", "ordered", false, lookups, null)),
    ).toEqual(["u1", "u2", "r2", "r1"]);
  });

  it("relevance reverses whole, like every other axis", () => {
    // `\` flips the primary key, so the unranked block leads and the
    // ranked run reads worst-match-first. The tie-break is never
    // reversed, so `u1` / `u2` stay newest-first inside their block.
    expect(
      ids(sortCards(RANKED, "relevance", "ordered", true, lookups, RANK)),
    ).toEqual(["u1", "u2", "r2", "r1"]);
  });

  // No axis on the union declines to sort. This replaces the old
  // `msg_count_falls_back_to_server_order` pair (and its `sort_eval.rs`
  // twin): that axis was on both unions with a comparator on neither, so
  // picking it produced the order the caller would have got by picking
  // nothing, and it has been removed from the union and from `SortSpec`
  // rather than given a branch.
  //
  // The `Record<SortTarget, true>` shape is what keeps this honest:
  // adding a union member without listing it here is a type error, and
  // listing one with no branch in `buildCardCmp` fails the assertion —
  // which is exactly the state that went unnoticed before.
  it("every sort target has a comparator", () => {
    const targets: Record<SortTarget, true> = {
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
      // Answers with the default axis when no rank map is passed, which
      // is still a comparator — the null return is reserved for "this
      // build has no branch for the axis at all".
      relevance: true,
    };
    const noRecency = {
      persona: new Map<string, number>(),
      modality: new Map<string, number>(),
      tag: new Map<string, number>(),
      group: new Map<string, number>(),
    };
    for (const target of Object.keys(targets) as SortTarget[]) {
      expect(
        buildCardCmp<Card>(target, "updated", 1, noRecency, lookups),
        `${target} has no comparator branch — it would silently keep server order`,
      ).not.toBeNull();
    }
  });

  it("firstUserLabel skips internal namespaces and falls to the sentinel", () => {
    expect(firstUserLabel(["persona:sys", "journal_kind:emo", "real"])).toBe("real");
    expect(firstUserLabel(["persona:sys"])).toBe(TAIL_SENTINEL);
    expect(firstUserLabel([])).toBe(TAIL_SENTINEL);
  });
});

// --- collation parity --------------------------------------------------

// Builds a lookups object that differs from the shared fixture only in
// which collation it compares under, so a case can state "the same rows,
// the same axis, a different search parameter".
function lookupsWithCollation(collation: string | null): CardSortLookups {
  return { ...lookups, compareText: textComparator(collation) };
}

describe("JS ↔ Rust collation parity", () => {
  // Each case is the twin of a `collation_parity::*` test in
  // `sort_eval.rs`, over the same fixture strings. Both sides now assert
  // the same order — except `cjk extension`, which is the one documented
  // residual and asserts the opposite on purpose.

  it("case is tertiary, letter is primary", () => {
    const cards = [
      card("upper", "pa", "dialogue", 10, 1, ["Zebra"]),
      card("lower", "pa", "dialogue", 10, 1, ["apple"]),
    ];
    expect(ids(sortCards(cards, "tag", "alpha", false, lookups))).toEqual([
      "lower",
      "upper",
    ]);
  });

  it("accents are a secondary difference", () => {
    // Both sides lower-case the cover key, so case is out of the picture
    // and the accent is what is being tested: 'é' stays beside 'e'
    // instead of sorting past 'f'.
    const cards = [
      card("accented", "pa", "dialogue", 10, 1, [], [], "Édition"),
      card("plain", "pa", "dialogue", 10, 1, [], [], "Future"),
    ];
    expect(ids(sortCards(cards, "cover", "alpha", false, lookups))).toEqual([
      "accented",
      "plain",
    ]);
  });

  it("kana follows gojuon across scripts", () => {
    // The highest-impact case: covers are Japanese in practice. 'ア' is
    // the first kana and 'ん' the last; hiragana and katakana share
    // primary weights, so the script boundary does not enter into it.
    const cards = [
      card("n", "pa", "dialogue", 10, 1, [], [], "んご"),
      card("a", "pa", "dialogue", 10, 1, [], [], "アイ"),
    ];
    expect(ids(sortCards(cards, "cover", "alpha", false, lookups))).toEqual([
      "a",
      "n",
    ]);
  });

  it("astral label stays before the sentinel", () => {
    // `TAIL_SENTINEL` (U+FFFF) parks unlabelled cards last. ICU weights
    // the noncharacter above every assigned code point, so an emoji
    // label (U+1F642) sorts before "no tag" on both sides — the grid and
    // the frozen `position` agree on where the tail is.
    const cards = [
      card("emoji", "pa", "dialogue", 10, 1, ["🙂 mood"]),
      card("untagged", "pa", "dialogue", 10, 1, []),
    ];
    expect(ids(sortCards(cards, "tag", "alpha", false, lookups))).toEqual([
      "emoji",
      "untagged",
    ]);
  });

  it("ascii only keys are unchanged", () => {
    const cards = [
      card("c", "pa", "dialogue", 10, 1, ["cherry"]),
      card("a", "pa", "dialogue", 10, 1, ["apple"]),
      card("b", "pa", "dialogue", 10, 1, ["banana"]),
    ];
    expect(ids(sortCards(cards, "tag", "alpha", false, lookups))).toEqual([
      "a",
      "b",
      "c",
    ]);
  });

  it("cjk extension is the residual divergence", () => {
    // KNOWN LIMITATION. ICU interleaves the Han extension blocks among
    // the URO, ICU4X puts the URO first. `𠮷` is Ext B (U+20BB7) and
    // `日本` is URO, so this side says `𠮷野` < `日本` and the backend
    // says the reverse — the twin case in `sort_eval.rs` asserts
    // `["uro", "extb"]`. Everyday Japanese is URO and agrees; only
    // rare-kanji forms reach this.
    const cards = [
      card("uro", "pa", "dialogue", 10, 1, [], [], "日本"),
      card("extb", "pa", "dialogue", 10, 1, [], [], "𠮷野"),
    ];
    expect(ids(sortCards(cards, "cover", "alpha", false, lookups))).toEqual([
      "extb",
      "uro",
    ]);
  });

  it("collation is a search parameter", () => {
    // The knob is per-search, not a module constant: the same rows on
    // the same axis reorder once the query names a tailoring. Swedish
    // moves `ä` past `z` at the primary level, so the difference
    // survives the default tertiary strength. (Japanese would show
    // nothing here — CLDR `ja` demotes the hiragana/katakana difference
    // to quaternary, so at tertiary the two compare equal.)
    const cards = [
      card("umlaut", "pa", "dialogue", 10, 1, [], [], "Ärlig"),
      card("z", "pa", "dialogue", 10, 1, [], [], "Zebra"),
    ];
    expect(ids(sortCards(cards, "cover", "alpha", false, lookups))).toEqual([
      "umlaut",
      "z",
    ]);
    expect(
      ids(sortCards(cards, "cover", "alpha", false, lookupsWithCollation("sv"))),
    ).toEqual(["z", "umlaut"]);
  });

  it("root is reachable by tag but not by und", () => {
    // Why `ROOT_COLLATION_TAG` is `"en"` and not `"und"`: `und` is not a
    // supported locale, so ECMA-402 resolves it to the host default and
    // the "fixed" collation would silently vary by machine. This is the
    // assertion that would fail if a future engine started supporting
    // `und` properly — at which point the mapping should be revisited.
    expect(Intl.Collator.supportedLocalesOf(["und"])).toEqual([]);
    expect(Intl.Collator.supportedLocalesOf([ROOT_COLLATION_TAG])).not.toEqual(
      [],
    );
    // `"root"` is not even well-formed.
    expect(() => new Intl.Collator("root")).toThrow(RangeError);
  });
});
