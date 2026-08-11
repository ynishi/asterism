// Thumb-fetch instrumentation for the grid / import benches.
//
// One record per `ensureThumb` fetch: when it was first asked for, when
// it resolved, how it resolved, how many retries it took, and whether
// the card that asked for it was still in the DOM at that moment. The
// last field is the measurement the "no stale cancel" finding needs —
// a thumb that lands for a tile the user has already scrolled past is
// decode work nobody sees, and counting it is the only way to say how
// much of it there is.
//
// The module records; it does not aggregate. `dump()` hands the driver
// the raw entries and the three catalog counts (blob URLs held,
// negative-cached misses, dead originals) so the arithmetic happens on
// the driver side, where a run's numbers can be re-derived after the
// fact. `stats()` is a console convenience — counts, nothing that
// would tempt a caller to publish it as the result.
//
// Everything no-ops outside the gate — a production build pays
// nothing, not even the DOM probe. `makeThumbPerf` is exported so the
// disabled shape can be tested from a suite that necessarily runs with
// `DEV` true.
//
// The gate is `DEV` **or** an explicit `VITE_BENCH=1`, which is wider
// than `perf-baseline.ts`'s and for one reason: the bench driver runs
// the app the way `just ui-e2e` does — `tauri build --debug`, whose
// frontend is a *production* Vite build loaded from `frontendDist`. On
// a `DEV`-only gate every measurement in this module would be a no-op
// in exactly the build the bench measures. `VITE_BENCH` is opt-in at
// the build command (`bench-scroll` in the Justfile) and set nowhere
// else, so a shipped build still has `DEV` false, `VITE_BENCH`
// undefined, and the disabled shape — `perfGateEnabled` exists as a
// named function so that last sentence is a test rather than a claim.

// How a fetch ended.
//
// `hit` / `retried` are the same success seen from the retry budget:
// `retried` means the first attempt found no thumb and a backoff
// round produced one, which is the placeholder duration the bench is
// about. `missing` is the budget running out (the catalog negative-
// caches the key), `dead` is the command itself failing.
export type ThumbOutcome = "hit" | "retried" | "missing" | "dead";

// One resolved fetch.
export interface ThumbPerfEntry {
  assetId: string;
  sizePx: number;
  // When the *first* attempt for this key started — not the retry
  // that happened to succeed. The tile shows a placeholder for the
  // whole span, so that is what the span has to cover.
  requestedAtMs: number;
  resolvedAtMs: number;
  outcome: ThumbOutcome;
  retryCount: number;
  visibleAtResolve: boolean;
}

// The stamp `begin` hands back to the call site, so the entry can be
// written at the point the fetch ends without the catalog having to
// carry any per-key bookkeeping of its own.
export interface ThumbAttempt {
  requestedAtMs: number;
  retryCount: number;
}

// Frontend-side residency counters, read off the catalog at dump
// time.
export interface ThumbCatalogCounts {
  blobUrlCount: number;
  missingCount: number;
  deadCount: number;
}

export interface ThumbPerfDump extends ThumbCatalogCounts {
  entries: ThumbPerfEntry[];
  // Ring capacity, so a driver can tell "20,000 entries" from
  // "20,000 entries and an unknown number discarded".
  capacity: number;
  // Every record ever pushed, discarded ones included.
  recorded: number;
  dropped: number;
}

export interface ThumbPerfStats extends ThumbCatalogCounts {
  entries: number;
  recorded: number;
  dropped: number;
  byOutcome: Record<ThumbOutcome, number>;
  // Resolved while the asking card was no longer in the DOM.
  wasted: number;
}

export interface ThumbPerf {
  // `false` in a production build — the call sites skip building an
  // entry at all rather than handing one to a sink that drops it.
  readonly enabled: boolean;
  now(): number;
  // Stamps the start of a fetch for `(assetId, sizePx)`. Called on
  // every attempt: the first call fixes `requestedAtMs`, later ones
  // return that same stamp with the retry count advanced.
  begin(assetId: string, sizePx: number): ThumbAttempt;
  // Whether a card for this asset is in the DOM right now.
  visible(assetId: string): boolean;
  record(entry: ThumbPerfEntry): void;
  // Late-bound because the catalog owns the counters and this module
  // must not import it (the catalog imports this one).
  bindCounts(source: () => ThumbCatalogCounts): void;
  dump(): ThumbPerfDump;
  stats(): ThumbPerfStats;
  reset(): void;
}

// Ring size. 20,000 entries is a ten-minute scroll session at the
// rates the L preset produces, and bounded so a dev session that is
// left open does not grow without limit.
export const CAPACITY = 20_000;

// The selector the grid card carries (`App.svelte`, `.card` +
// `data-asset-id`). Exported so the test pins the exact string: a
// silently non-matching selector would report every resolve as
// wasted, which is a plausible-looking number and a wrong one.
export function cardSelector(assetId: string): string {
  const escaped =
    typeof CSS !== "undefined" && typeof CSS.escape === "function"
      ? CSS.escape(assetId)
      : assetId.replace(/["\\]/g, "\\$&");
  return `.card[data-asset-id="${escaped}"]`;
}

const now = (): number =>
  typeof performance !== "undefined" ? performance.now() : Date.now();

const ZERO_ATTEMPT: ThumbAttempt = Object.freeze({
  requestedAtMs: 0,
  retryCount: 0,
});

function emptyOutcomes(): Record<ThumbOutcome, number> {
  return { hit: 0, retried: 0, missing: 0, dead: 0 };
}

export function makeThumbPerf(enabled: boolean): ThumbPerf {
  if (!enabled) {
    return {
      enabled: false,
      now,
      begin: () => ZERO_ATTEMPT,
      visible: () => false,
      record: () => {},
      bindCounts: () => {},
      dump: () => ({
        entries: [],
        capacity: CAPACITY,
        recorded: 0,
        dropped: 0,
        blobUrlCount: 0,
        missingCount: 0,
        deadCount: 0,
      }),
      stats: () => ({
        entries: 0,
        recorded: 0,
        dropped: 0,
        byOutcome: emptyOutcomes(),
        wasted: 0,
        blobUrlCount: 0,
        missingCount: 0,
        deadCount: 0,
      }),
      reset: () => {},
    };
  }

  // Ring buffer: fixed-size array plus a write head, so a long session
  // costs a constant amount of memory and the oldest entries are the
  // ones that go.
  const ring: Array<ThumbPerfEntry | undefined> = new Array(CAPACITY);
  let head = 0;
  let recorded = 0;
  // In-flight stamps, keyed like the catalog's own caches. Bounded by
  // the number of concurrent fetches: every terminal outcome deletes
  // its key, and the retry path deliberately keeps it (the fetch has
  // not ended yet).
  const pending = new Map<string, ThumbAttempt>();
  let counts: () => ThumbCatalogCounts = () => ({
    blobUrlCount: 0,
    missingCount: 0,
    deadCount: 0,
  });

  const key = (assetId: string, sizePx: number) => `${assetId}@${sizePx}`;

  const entries = (): ThumbPerfEntry[] => {
    if (recorded < CAPACITY) {
      return ring.slice(0, recorded) as ThumbPerfEntry[];
    }
    // Full: the oldest entry sits at the write head.
    return [
      ...(ring.slice(head) as ThumbPerfEntry[]),
      ...(ring.slice(0, head) as ThumbPerfEntry[]),
    ];
  };

  const perf: ThumbPerf = {
    enabled: true,
    now,
    begin(assetId, sizePx) {
      const k = key(assetId, sizePx);
      const existing = pending.get(k);
      if (existing) {
        existing.retryCount += 1;
        return existing;
      }
      const fresh: ThumbAttempt = { requestedAtMs: now(), retryCount: 0 };
      pending.set(k, fresh);
      return fresh;
    },
    visible(assetId) {
      if (typeof document === "undefined") return false;
      try {
        return document.querySelector(cardSelector(assetId)) !== null;
      } catch {
        // A selector the engine refuses is a measurement failure, not
        // a reason to take the app down inside a thumb continuation.
        return false;
      }
    },
    record(entry) {
      pending.delete(key(entry.assetId, entry.sizePx));
      ring[head] = entry;
      head = (head + 1) % CAPACITY;
      recorded += 1;
    },
    bindCounts(source) {
      counts = source;
    },
    dump() {
      const list = entries();
      return {
        entries: list,
        capacity: CAPACITY,
        recorded,
        dropped: recorded - list.length,
        ...counts(),
      };
    },
    stats() {
      const list = entries();
      const byOutcome = emptyOutcomes();
      let wasted = 0;
      for (const entry of list) {
        byOutcome[entry.outcome] += 1;
        if (!entry.visibleAtResolve) wasted += 1;
      }
      return {
        entries: list.length,
        recorded,
        dropped: recorded - list.length,
        byOutcome,
        wasted,
        ...counts(),
      };
    },
    reset() {
      ring.fill(undefined);
      head = 0;
      recorded = 0;
      pending.clear();
    },
  };

  if (typeof window !== "undefined") {
    // The driver's handle. Only the three verbs a run needs — the
    // recording API stays a module import so nothing can push a
    // fabricated entry in from the console.
    (
      window as unknown as {
        __ASTERISM_THUMB_PERF__?: Pick<ThumbPerf, "dump" | "stats" | "reset">;
      }
    ).__ASTERISM_THUMB_PERF__ = {
      dump: () => perf.dump(),
      stats: () => perf.stats(),
      reset: () => perf.reset(),
    };
  }

  return perf;
}

// The subset of `import.meta.env` the gate reads. Narrow on purpose:
// `ImportMetaEnv` carries an index signature, so a typo in the flag
// name would type-check against it and silently never open the gate.
export interface PerfGateEnv {
  DEV: boolean;
  VITE_BENCH?: string | undefined;
}

/**
 * Whether the instrumentation records.
 *
 * `VITE_BENCH` is compared against the literal `"1"` — Vite inlines the
 * build-time environment as strings, so a truthiness check would also
 * open the gate for `VITE_BENCH=0` and `VITE_BENCH=false`.
 */
export function perfGateEnabled(env: PerfGateEnv): boolean {
  return env.DEV || env.VITE_BENCH === "1";
}

export const thumbPerf: ThumbPerf = makeThumbPerf(perfGateEnabled(import.meta.env));
