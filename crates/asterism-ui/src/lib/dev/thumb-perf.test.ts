// thumb-perf unit tests.
//
// What is worth pinning is what a wrong number would look like, because
// every one of these failure modes produces a plausible bench result:
//
//   - the ring must drop the *oldest* entries and say how many it
//     dropped (a silently truncated tail reads as "the session got
//     faster towards the end");
//   - a retry chain is one fetch, so `requestedAtMs` must stay on the
//     first attempt (resetting it per retry erases exactly the
//     placeholder duration the bench asks for);
//   - the disabled build must record nothing at all (a production
//     build paying for a 20,000-entry ring is a regression in the
//     thing being measured);
//   - the card selector must be the one `App.svelte` actually emits —
//     a non-matching selector reports every resolve as wasted, which
//     is a believable number and a false one.
//
// `import.meta.env.DEV` is true under vitest, so the enabled shape is
// what a plain import gives; the disabled shape is reached through the
// exported factory.
//
// The DOM probe is tested against a stub `document` installed on
// `globalThis` rather than under jsdom: the suite runs in vitest's node
// environment (vite.config.ts) and the repo convention is that
// window-dependent surface stays untested until a DOM env is
// deliberately added (see `lib/diag.test.ts`). The stub keeps the
// assertion on the part that can silently rot — the selector string.
import { afterEach, describe, expect, it } from "vitest";
import {
  CAPACITY,
  cardSelector,
  makeThumbPerf,
  perfGateEnabled,
  type ThumbOutcome,
} from "./thumb-perf";

function entry(
  perf: ReturnType<typeof makeThumbPerf>,
  assetId: string,
  outcome: ThumbOutcome = "hit",
  visibleAtResolve = true,
) {
  perf.record({
    assetId,
    sizePx: 256,
    requestedAtMs: 0,
    resolvedAtMs: 1,
    outcome,
    retryCount: 0,
    visibleAtResolve,
  });
}

const realDocument = (globalThis as { document?: unknown }).document;

afterEach(() => {
  if (realDocument === undefined) {
    delete (globalThis as { document?: unknown }).document;
  } else {
    (globalThis as { document?: unknown }).document = realDocument;
  }
});

describe("thumbPerf ring", () => {
  it("keeps the newest CAPACITY entries and reports what it dropped", () => {
    const perf = makeThumbPerf(true);
    for (let i = 0; i < CAPACITY + 5; i += 1) entry(perf, `a-${i}`);

    const dump = perf.dump();
    expect(dump.entries.length).toBe(CAPACITY);
    expect(dump.recorded).toBe(CAPACITY + 5);
    expect(dump.dropped).toBe(5);
    expect(dump.capacity).toBe(CAPACITY);
    // Oldest-first, oldest five gone — the order is what a driver
    // slices a time window out of.
    expect(dump.entries[0]!.assetId).toBe("a-5");
    expect(dump.entries[CAPACITY - 1]!.assetId).toBe(`a-${CAPACITY + 4}`);
  });

  it("reset empties the buffer and the in-flight stamps", () => {
    const perf = makeThumbPerf(true);
    entry(perf, "a-1");
    perf.begin("a-2", 256);
    perf.reset();

    expect(perf.dump().entries).toEqual([]);
    expect(perf.dump().recorded).toBe(0);
    expect(perf.dump().dropped).toBe(0);
    // The stamp is gone too, so the next fetch of `a-2` starts a new
    // attempt rather than inheriting a retry count from before reset.
    expect(perf.begin("a-2", 256).retryCount).toBe(0);
  });

  it("holds one requestedAt per key across retries, per size", () => {
    const perf = makeThumbPerf(true);
    const first = perf.begin("a-1", 256);
    const retry = perf.begin("a-1", 256);
    expect(retry.requestedAtMs).toBe(first.requestedAtMs);
    expect(retry.retryCount).toBe(1);
    expect(perf.begin("a-1", 256).retryCount).toBe(2);

    // 256 and 512 are different fetches of the same asset.
    expect(perf.begin("a-1", 512).retryCount).toBe(0);

    // Recording ends the fetch: the next `begin` is a fresh attempt.
    entry(perf, "a-1");
    expect(perf.begin("a-1", 256).retryCount).toBe(0);
  });

  it("stats counts outcomes, waste and the bound catalog counters", () => {
    const perf = makeThumbPerf(true);
    perf.bindCounts(() => ({
      blobUrlCount: 7,
      missingCount: 2,
      deadCount: 1,
    }));
    entry(perf, "a-1", "hit", true);
    entry(perf, "a-2", "retried", false);
    entry(perf, "a-3", "missing", false);
    entry(perf, "a-4", "dead", true);

    const stats = perf.stats();
    expect(stats.entries).toBe(4);
    expect(stats.byOutcome).toEqual({
      hit: 1,
      retried: 1,
      missing: 1,
      dead: 1,
    });
    expect(stats.wasted).toBe(2);
    expect(stats.blobUrlCount).toBe(7);
    expect(perf.dump().missingCount).toBe(2);
    expect(perf.dump().deadCount).toBe(1);
  });
});

describe("thumbPerf gate", () => {
  it("records nothing when disabled", () => {
    const perf = makeThumbPerf(false);
    expect(perf.enabled).toBe(false);
    entry(perf, "a-1");
    perf.bindCounts(() => ({
      blobUrlCount: 9,
      missingCount: 9,
      deadCount: 9,
    }));

    const dump = perf.dump();
    expect(dump.entries).toEqual([]);
    expect(dump.recorded).toBe(0);
    // Not even the bound counters: the disabled shape holds no
    // reference to the catalog at all.
    expect(dump.blobUrlCount).toBe(0);
    expect(perf.stats().byOutcome).toEqual({
      hit: 0,
      retried: 0,
      missing: 0,
      dead: 0,
    });
    // No stamp bookkeeping either — every attempt reads as the first.
    expect(perf.begin("a-1", 256).retryCount).toBe(0);
    expect(perf.begin("a-1", 256).retryCount).toBe(0);
    expect(perf.visible("a-1")).toBe(false);
  });

  // The gate decides which builds pay for any of the above, and the
  // two halves fail in opposite directions: a gate that stays shut
  // under `VITE_BENCH=1` makes every bench run report zero fetches
  // (a green-looking result with no data behind it), and a gate that
  // opens without it puts a 20,000-entry ring and a `querySelector`
  // per thumb into the shipped app. Both are pinned here rather than
  // inferred from `thumbPerf`, whose own gate is fixed at import time
  // by whatever `import.meta.env` vitest provides.
  it("opens for a dev build", () => {
    expect(perfGateEnabled({ DEV: true })).toBe(true);
  });

  it("opens for a production build built with VITE_BENCH=1", () => {
    expect(perfGateEnabled({ DEV: false, VITE_BENCH: "1" })).toBe(true);
  });

  it("stays shut for an ordinary production build", () => {
    expect(perfGateEnabled({ DEV: false })).toBe(false);
    expect(perfGateEnabled({ DEV: false, VITE_BENCH: undefined })).toBe(false);
    // Vite inlines the flag as a string, so the values a shell would
    // produce for "off" must not read as truthy.
    expect(perfGateEnabled({ DEV: false, VITE_BENCH: "0" })).toBe(false);
    expect(perfGateEnabled({ DEV: false, VITE_BENCH: "false" })).toBe(false);
    expect(perfGateEnabled({ DEV: false, VITE_BENCH: "" })).toBe(false);
  });
});

describe("thumbPerf visibility probe", () => {
  it("asks the DOM for the card the grid actually renders", () => {
    const asked: string[] = [];
    (globalThis as { document?: unknown }).document = {
      querySelector: (selector: string) => {
        asked.push(selector);
        return selector.includes("on-screen") ? {} : null;
      },
    };

    const perf = makeThumbPerf(true);
    expect(perf.visible("on-screen")).toBe(true);
    expect(perf.visible("scrolled-past")).toBe(false);
    // The selector is the contract with `App.svelte`'s card markup.
    expect(asked[0]).toBe('.card[data-asset-id="on-screen"]');
    expect(cardSelector("x")).toBe('.card[data-asset-id="x"]');
  });

  it("treats a selector the engine refuses as not visible", () => {
    (globalThis as { document?: unknown }).document = {
      querySelector: () => {
        throw new Error("bad selector");
      },
    };
    // A measurement failure must not take down a thumb continuation.
    expect(makeThumbPerf(true).visible("a-1")).toBe(false);
  });

  it("reports not-visible when there is no document at all", () => {
    delete (globalThis as { document?: unknown }).document;
    expect(makeThumbPerf(true).visible("a-1")).toBe(false);
  });
});
