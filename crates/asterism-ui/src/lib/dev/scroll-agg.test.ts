// scroll-agg unit tests.
//
// Same standard as `thumb-perf.test.ts`: what is worth pinning is what
// a wrong number would look like in a bench result, because every one
// of these is plausible on its own.
//
//   - a percentile computed by interpolation reports a wait nobody
//     had, and an off-by-one at the top end quietly returns the
//     second-slowest fetch as p95 — the exact number the "5 s bar"
//     verdict is read off;
//   - the empty window has to stay `null` rather than `0`, or a cycle
//     during which nothing loaded averages into the curve as the
//     fastest one in the run;
//   - `over_5s_count` must be strictly *over*, so the bar means what
//     the issue says;
//   - `wasted_rate` is the size of the missing stale-cancel, so it is
//     counted off `visibleAtResolve` and nothing else.
import { describe, expect, it } from "vitest";
import {
  aggregateWindow,
  percentile,
  residencyOf,
  SLOW_MS,
} from "./scroll-agg";
import type { ThumbOutcome, ThumbPerfEntry } from "./thumb-perf";

function entry(
  durationMs: number,
  outcome: ThumbOutcome = "hit",
  visibleAtResolve = true,
  retryCount = 0,
): ThumbPerfEntry {
  return {
    assetId: `a-${durationMs}-${outcome}`,
    sizePx: 256,
    requestedAtMs: 1_000,
    resolvedAtMs: 1_000 + durationMs,
    outcome,
    retryCount,
    visibleAtResolve,
  };
}

describe("percentile", () => {
  it("is nearest-rank, so it returns a value that was measured", () => {
    const sorted = [10, 20, 30, 40];
    // ceil(0.5 * 4) = 2 → the 2nd smallest. Interpolation would say 25,
    // which no fetch took.
    expect(percentile(sorted, 0.5)).toBe(20);
    expect(percentile(sorted, 0.95)).toBe(40);
    expect(percentile(sorted, 0)).toBe(10);
    expect(percentile(sorted, 1)).toBe(40);
  });

  it("has no answer for an empty sample", () => {
    expect(percentile([], 0.5)).toBe(null);
  });

  it("returns the single value for a one-element sample", () => {
    expect(percentile([7], 0.5)).toBe(7);
    expect(percentile([7], 0.95)).toBe(7);
  });

  it("reaches the top element for a sample large enough to round down", () => {
    // 100 samples: ceil(0.95 * 100) = 95 → the 95th smallest, i.e. the
    // 6th slowest. A rank computed with floor would return the 96th.
    const sorted = Array.from({ length: 100 }, (_v, i) => i + 1);
    expect(percentile(sorted, 0.95)).toBe(95);
    expect(percentile(sorted, 0.99)).toBe(99);
    expect(percentile(sorted, 1)).toBe(100);
  });
});

describe("aggregateWindow", () => {
  it("reports nothing measurable for an empty window", () => {
    const agg = aggregateWindow([]);
    expect(agg.count).toBe(0);
    // Null, not zero: a window with no fetches is not a fast window.
    expect(agg.p50_ms).toBe(null);
    expect(agg.p95_ms).toBe(null);
    expect(agg.max_ms).toBe(null);
    expect(agg.mean_ms).toBe(null);
    expect(agg.wasted_rate).toBe(null);
    expect(agg.over_5s_count).toBe(0);
    expect(agg.wasted_count).toBe(0);
    expect(agg.missing_count).toBe(0);
    expect(agg.retried_count).toBe(0);
    expect(agg.by_outcome).toEqual({ hit: 0, retried: 0, missing: 0, dead: 0 });
  });

  it("collapses a one-entry window onto that entry", () => {
    const agg = aggregateWindow([entry(120)]);
    expect(agg.count).toBe(1);
    expect(agg.p50_ms).toBe(120);
    expect(agg.p95_ms).toBe(120);
    expect(agg.max_ms).toBe(120);
    expect(agg.mean_ms).toBe(120);
    expect(agg.wasted_rate).toBe(0);
  });

  it("orders the sample itself rather than trusting arrival order", () => {
    // Entries arrive in resolve order, which is not duration order.
    const agg = aggregateWindow([entry(900), entry(100), entry(500), entry(300)]);
    expect(agg.p50_ms).toBe(300);
    expect(agg.max_ms).toBe(900);
    expect(agg.mean_ms).toBe(450);
  });

  it("counts the 5 s bar strictly above it", () => {
    const agg = aggregateWindow([
      entry(SLOW_MS - 1),
      entry(SLOW_MS),
      entry(SLOW_MS + 1),
      entry(SLOW_MS * 2),
    ]);
    // Exactly 5,000 ms is not "over 5 s".
    expect(agg.over_5s_count).toBe(2);
  });

  it("counts waste off the visibility flag and turns it into a rate", () => {
    const agg = aggregateWindow([
      entry(100, "hit", true),
      entry(200, "hit", false),
      entry(300, "retried", false),
      entry(400, "missing", true),
    ]);
    expect(agg.wasted_count).toBe(2);
    expect(agg.wasted_rate).toBe(0.5);
  });

  it("keeps the outcome split and hoists the missing count", () => {
    const agg = aggregateWindow([
      entry(10, "hit"),
      entry(20, "retried", true, 2),
      entry(30, "missing"),
      entry(40, "missing"),
      entry(50, "dead"),
    ]);
    expect(agg.by_outcome).toEqual({ hit: 1, retried: 1, missing: 2, dead: 1 });
    expect(agg.missing_count).toBe(2);
    expect(agg.retried_count).toBe(1);
  });

  it("clamps a backwards clock instead of letting it drag the sample", () => {
    const backwards: ThumbPerfEntry = {
      ...entry(0),
      requestedAtMs: 5_000,
      resolvedAtMs: 4_000,
    };
    const agg = aggregateWindow([backwards, entry(100)]);
    expect(agg.max_ms).toBe(100);
    expect(agg.p50_ms).toBe(0);
    expect(agg.mean_ms).toBe(50);
  });
});

describe("residencyOf", () => {
  it("carries the catalog counters and the ring's own bookkeeping", () => {
    // `dropped > 0` is the field that keeps a saturated window from
    // reading as a quiet one — the aggregate above only ever sees what
    // survived the ring.
    expect(
      residencyOf({
        entries: [],
        capacity: 20_000,
        recorded: 20_005,
        dropped: 5,
        blobUrlCount: 812,
        missingCount: 3,
        deadCount: 1,
      }),
    ).toEqual({
      blobUrlCount: 812,
      missingCount: 3,
      deadCount: 1,
      recorded: 20_005,
      dropped: 5,
      capacity: 20_000,
    });
  });
});
