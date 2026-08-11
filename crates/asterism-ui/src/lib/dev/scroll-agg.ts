// Window aggregation for the scroll bench.
//
// `thumb-perf.ts` records and refuses to aggregate; this is the other
// half. It turns one `dump()` — the raw fetches since the last
// `reset()` — into the handful of numbers the run is read by, and it
// lives here rather than inside the wdio spec for one reason: a
// spec-local aggregate is only ever exercised by a ten-minute run
// against a seeded 110k-row profile, so an off-by-one in the
// percentile would be indistinguishable from the app being slower.
// Here it is a vitest away.
//
// # What the fields mean
//
// The bars come from the question this bench exists to answer:
//
//   * `p50_ms` / `p95_ms` / `max_ms` — how long a tile showed a
//     placeholder. Measured from the *first* attempt for the key
//     (`requestedAtMs`), so a retry chain reads as the one wait the
//     user actually sat through.
//   * `over_5s_count` — fetches that took more than five seconds.
//     That is the "unusable" bar: the point at which the reference
//     app stops being usable while scrolling.
//   * `wasted_rate` — share of fetches whose asking card had already
//     left the DOM when they landed. The app has no stale-cancel
//     (no `AbortController` anywhere in the thumb path), so this is
//     the size of that hole, not a proxy for it.
//   * `by_outcome` / `missing_count` — a run whose p95 improved
//     because more fetches gave up early is not a faster run.
//
// # Reading a window with nothing in it
//
// Every quantile is `null`, never `0`. A zero would sort and average
// like a real measurement and would read as "instant" — which is the
// one thing an empty window does not mean. Counts stay `0` and rates
// stay `null` for the same reason.

// Type-only, and it has to stay that way: `thumb-perf.ts` builds its
// singleton at module scope off `import.meta.env` and pokes `window`,
// neither of which exists in the Node process running the wdio spec
// that imports this file. `import type` is erased outright under
// `isolatedModules`.
import type { ThumbOutcome, ThumbPerfDump, ThumbPerfEntry } from "./thumb-perf";

/** The "unusable while scrolling" bar. */
export const SLOW_MS = 5_000;

export interface ScrollWindowAgg {
  /** Fetches that resolved inside the window. */
  count: number;
  /** Nearest-rank percentiles over `resolvedAtMs - requestedAtMs`. */
  p50_ms: number | null;
  p95_ms: number | null;
  max_ms: number | null;
  mean_ms: number | null;
  /** Resolved slower than `SLOW_MS`. */
  over_5s_count: number;
  /** Resolved after the asking card had left the DOM. */
  wasted_count: number;
  /** `wasted_count / count`, or `null` for an empty window. */
  wasted_rate: number | null;
  by_outcome: Record<ThumbOutcome, number>;
  /** `by_outcome.missing` — the retry budget ran out and the catalog
   *  negative-cached the key. Hoisted because it is one of the bars. */
  missing_count: number;
  /** Fetches that needed at least one retry round. */
  retried_count: number;
}

/** What the frontend was still holding at dump time (issue item 5). */
export interface ResidencySample {
  blobUrlCount: number;
  missingCount: number;
  deadCount: number;
  /** Ring bookkeeping, so a window that overflowed says so rather
   *  than looking like a quiet one. */
  recorded: number;
  dropped: number;
  capacity: number;
}

function emptyOutcomes(): Record<ThumbOutcome, number> {
  return { hit: 0, retried: 0, missing: 0, dead: 0 };
}

/**
 * Nearest-rank percentile over an already-sorted ascending array.
 *
 * Nearest rank rather than interpolation: the value returned is one a
 * tile actually waited, which is what a bar like "p95 over 5 s" is a
 * claim about. `q` is a fraction in `[0, 1]`.
 */
export function percentile(sortedAsc: number[], q: number): number | null {
  if (sortedAsc.length === 0) return null;
  const rank = Math.ceil(q * sortedAsc.length);
  const index = Math.min(sortedAsc.length - 1, Math.max(0, rank - 1));
  return sortedAsc[index] ?? null;
}

/**
 * Aggregates one window of recorded fetches.
 *
 * Takes the entries rather than the dump so a caller can slice a
 * window out of a longer capture; `residencyOf` covers the counters
 * that only exist on the dump.
 */
export function aggregateWindow(entries: readonly ThumbPerfEntry[]): ScrollWindowAgg {
  const durations: number[] = [];
  const byOutcome = emptyOutcomes();
  let wasted = 0;
  let overSlow = 0;
  let retried = 0;
  let total = 0;

  for (const entry of entries) {
    byOutcome[entry.outcome] += 1;
    if (!entry.visibleAtResolve) wasted += 1;
    if (entry.retryCount > 0) retried += 1;
    // Clamped at zero: a negative span would mean the clock moved
    // backwards between the two stamps, and letting it into the sort
    // would drag every percentile below it.
    const ms = Math.max(0, entry.resolvedAtMs - entry.requestedAtMs);
    if (ms > SLOW_MS) overSlow += 1;
    durations.push(ms);
    total += ms;
  }

  durations.sort((a, b) => a - b);
  const count = durations.length;

  return {
    count,
    p50_ms: percentile(durations, 0.5),
    p95_ms: percentile(durations, 0.95),
    max_ms: count > 0 ? durations[count - 1]! : null,
    mean_ms: count > 0 ? total / count : null,
    over_5s_count: overSlow,
    wasted_count: wasted,
    wasted_rate: count > 0 ? wasted / count : null,
    by_outcome: byOutcome,
    missing_count: byOutcome.missing,
    retried_count: retried,
  };
}

/** The catalog-side counters carried on a dump. */
export function residencyOf(dump: ThumbPerfDump): ResidencySample {
  return {
    blobUrlCount: dump.blobUrlCount,
    missingCount: dump.missingCount,
    deadCount: dump.deadCount,
    recorded: dump.recorded,
    dropped: dump.dropped,
    capacity: dump.capacity,
  };
}
