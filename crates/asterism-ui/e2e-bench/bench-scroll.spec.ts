// The scroll bench: what a thumb costs while the grid is moving.
//
// Run by `just bench-scroll`, never by `just ui-e2e` — the two suites
// have separate spec globs and separate configs for that reason (see
// `wdio.bench.conf.ts`). The questions it answers are the
// scroll-latency ones: window aggregation, stale-cancel accounting,
// and cold vs warm cache.
//
// Two phases, because they answer different questions:
//
//   **A — one way, at a constant speed.** Top to bottom once at ~1.5
//   viewports per second, never waiting for a tile to arrive. That is
//   the steady-state distribution (item 2) and the placeholder
//   duration (item 4): every fetch it produces is one a user scrolling
//   at a normal pace would have produced.
//
//   **B — thrash-scroll.** N minutes of seeded random jumps within ±3
//   viewports, which is what "browsing" looks like and what the
//   reference app degrades under (item 3). Sampled in 60-second
//   windows with a `reset()` after each, so the output is a *curve*
//   rather than one average over a run whose first minute and last
//   minute are the comparison being asked for. Residency (blob URLs
//   held, negative-cached misses) and process RSS ride along on every
//   window (item 5).
//
// # The guard, and why it is in `before`
//
// This drives a real app against a real profile on disk. Pointed at
// the Dogfood profile it would be a ten-minute scripted scroll through
// the User's library — harmless in itself, and a completely wrong
// measurement reported as a real one. `wdio.bench.conf.ts` passes
// `ASTERISM_PROFILE=bench`, but an env var says what was *asked for*;
// the persona list says what was *opened*. So the run refuses to start
// unless every persona in the sidebar carries the bench prefix. In
// `before`, because a failing `before` skips every `it` in the
// describe — the scenario must not scroll a single pixel to find out
// it is in the wrong library.
//
// The same hook asserts `window.__ASTERISM_THUMB_PERF__`. Its absence
// means the frontend was built without `VITE_BENCH=1`, in which case
// the instrumentation is a no-op (see `thumb-perf.ts`) and every
// number below would be a confident zero.
//
// # Cost model
//
// Inherited wholesale from `e2e/card-trash.spec.ts`: in this
// environment every *element* command (`$`, `$$`, `elementClick`, …)
// pays a ~6 s window-focus tax and `browser.execute` pays none. This
// file therefore contains no element commands at all — the two clicks
// it needs (a persona row, a group row) go through the DOM. That
// weakens them in the way that spec documents: an in-page `click()`
// fires the handler without proving anything about hit-testing. Which
// is the right trade here, because the gesture is not what is being
// measured; it is how the measurement gets set up.
//
// # Known limits
//
//   * A scripted scroll is not a trackpad. That is accepted:
//     run-to-run comparability is the property being bought, and
//     the cross-app comparison is confined to the two bars (placeholder
//     duration, and the count over 5 s).
//   * A window's `dump()` crosses the wire as raw entries — up to
//     20,000 objects. That is deliberate (aggregation belongs on the
//     driver side, where a run can be re-derived), and it is why the
//     ring is capped: `residency.dropped > 0` in the output means a
//     window saturated it and the aggregate under-counts.
//   * RSS is sampled with `ps`. The Tauri app's own processes are
//     identifiable by name; the WKWebView content processes are not
//     attributable to one app from `comm` alone, so they are reported
//     separately and labelled machine-wide.

import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import {
  aggregateWindow,
  residencyOf,
  type ResidencySample,
  type ScrollWindowAgg,
} from "../src/lib/dev/scroll-agg";
import type { ThumbPerfDump, ThumbPerfStats } from "../src/lib/dev/thumb-perf";

// --- knobs ---------------------------------------------------------
//
// Everything the recipe can vary, plus the two constants that are part
// of the corpus contract rather than of this run
// (`asterism-benchgen/src/model.rs`: personas are `bench-persona-N`,
// the full-population group is `bench-mega`).

function numberEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (raw === undefined || raw.trim() === "") return fallback;
  const value = Number(raw);
  return Number.isFinite(value) ? value : fallback;
}

const PERSONA_PREFIX = "bench-persona-";
const PERSONA = (process.env.BENCH_PERSONA ?? "bench-persona-0").trim();
const GROUP = (process.env.BENCH_GROUP ?? "bench-mega").trim();
const SCROLL_SEED = numberEnv("BENCH_SCROLL_SEED", 42);
/** Recorded in the result file so a number can be tied back to the
 *  corpus it was measured against. */
const CORPUS_SEED = numberEnv("BENCH_CORPUS_SEED", 42);

/** Phase A speed, in viewports per second. */
const ONE_WAY_VIEWPORTS_PER_S = 1.5;
/** Phase A is bounded: at this speed a 10k-asset group is minutes of
 *  scrolling, and an unexpectedly large one would otherwise eat the
 *  whole mocha budget before phase B ever ran. Hitting the ceiling is
 *  recorded (`stopped_early`), never swallowed. */
const ONE_WAY_MAX_S = numberEnv("BENCH_ONE_WAY_MAX_SECONDS", 300);
/** Interval between `scrollTop` writes. The position is computed from
 *  elapsed wall time, not accumulated per step, so driver jitter
 *  changes the smoothness and not the speed. */
const STEP_GAP_MS = 80;

/** Phase B jump distance, in viewports.
 *
 *  The move that matters is the one that lands **entirely outside**
 *  what is on screen. A viewport is what the grid paints at once; five
 *  to ten of them is "flick the scrollbar a few hundred cards", which
 *  is simply what handling a few thousand images looks like. Every
 *  such jump asks for a fresh screenful, and nothing cancels the
 *  screenful left behind (`thumb.svelte.ts:202` — the retry is a bare
 *  `setTimeout`, no `AbortController` anywhere in the store).
 *
 *  The previous shape was ±3 viewports, which mostly landed back
 *  inside the cache: its fetches dried up after four minutes and the
 *  run reported "no degradation" [measured 2026-08-05, bench-scroll-v1].
 *  That was a measurement of a cache, not of the operation people
 *  complain about.
 */
const JUMP_VIEWPORTS_MIN = numberEnv("BENCH_JUMP_VIEWPORTS_MIN", 5);
const JUMP_VIEWPORTS_MAX = numberEnv("BENCH_JUMP_VIEWPORTS_MAX", 10);
/** Pause after each jump.
 *
 *  Deliberately shorter than a thumb takes to arrive. Someone
 *  scrolling a few thousand images does not wait for the tiles before
 *  flicking again, and that overlap is the thing under test: the
 *  requests from the previous screenful are still in flight, still
 *  retrying on a backoff that runs 250/500/1000/2000/4000 ms
 *  (`thumb.svelte.ts:200`), and the new screenful queues up behind
 *  them.
 */
const JUMP_GAP_MS = numberEnv("BENCH_JUMP_GAP_MS", 400);
/** Share of jumps that land anywhere in the group instead of a fixed
 *  distance away — the scrollbar-drag to somewhere else entirely. */
const FAR_JUMP_RATE = numberEnv("BENCH_FAR_JUMP_RATE", 0.15);
/** How many jumps phase B makes. */
const TOTAL_JUMPS = numberEnv("BENCH_JUMPS", 200);
/** Window boundary, **in jumps rather than seconds**.
 *
 *  The curve asked for is "how long does the Nth screenful take",
 *  which is a function of how many screenfuls have been requested and
 *  not of how long the app has been open. Sampling on a clock mixes
 *  fast windows (many jumps) with slow ones (few) and averages the
 *  shape away.
 */
const JUMPS_PER_WINDOW = numberEnv("BENCH_JUMPS_PER_WINDOW", 10);

const SCREENS_DIR = process.env.BENCH_SCREENS_DIR;
const RESULTS_DIR = process.env.BENCH_RESULTS_DIR;
const PORT = process.env.BENCH_PORT ?? "19898";

// --- small node-side helpers ---------------------------------------

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * mulberry32 — 32-bit seeded PRNG.
 *
 * The point of seeding is that two runs make the same jumps, so a
 * changed curve is a changed app rather than a differently shuffled
 * one. Ten lines is the whole reason not to take a dependency.
 */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

async function snap(name: string): Promise<void> {
  if (!SCREENS_DIR) return;
  const safe = name.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 60);
  try {
    await Promise.race([
      browser.saveScreenshot(path.join(SCREENS_DIR, `${safe}.png`)),
      sleep(5_000),
    ]);
  } catch {
    // Liveness aid only — never a reason to lose a ten-minute run.
  }
}

/** One RSS reading. `null` totals mean `ps` could not be read, which
 *  is reported rather than defaulted to zero: a memory curve with
 *  invented zeros in it is worse than one with gaps. */
interface RssSample {
  at_ms: number;
  app_rss_kb: number | null;
  app_pids: number[];
  /** Machine-wide: WKWebView content processes cannot be attributed to
   *  one app from `comm`, so this is every one on the box. Useful as a
   *  trend on an otherwise idle machine, not as an absolute. */
  webkit_rss_kb: number | null;
  webkit_pids: number[];
  error?: string;
}

function sampleRss(): RssSample {
  const at_ms = Date.now();
  try {
    const out = execFileSync("ps", ["-axo", "pid,rss,comm"], {
      encoding: "utf8",
      timeout: 10_000,
      maxBuffer: 8 * 1024 * 1024,
    });
    let appKb = 0;
    let webkitKb = 0;
    const appPids: number[] = [];
    const webkitPids: number[] = [];
    for (const line of out.split("\n").slice(1)) {
      const m = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line);
      if (!m) continue;
      const pid = Number(m[1]);
      const rss = Number(m[2]);
      const comm = m[3] ?? "";
      if (comm.includes("asterism-ui") || comm.includes("Asterism")) {
        appKb += rss;
        appPids.push(pid);
      } else if (comm.includes("WebKit.WebContent") || comm.includes("com.apple.WebKit")) {
        webkitKb += rss;
        webkitPids.push(pid);
      }
    }
    return {
      at_ms,
      app_rss_kb: appKb,
      app_pids: appPids,
      webkit_rss_kb: webkitKb,
      webkit_pids: webkitPids,
    };
  } catch (err) {
    return {
      at_ms,
      app_rss_kb: null,
      app_pids: [],
      webkit_rss_kb: null,
      webkit_pids: [],
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

// --- in-page reads -------------------------------------------------
//
// Every callback below is an anonymous arrow in argument position,
// declares no named function, takes selectors / numbers in and returns
// data out, and returns rather than throwing on a fault path. All four
// are load-bearing under this driver — the reasons are laid out in
// `e2e/card-trash.spec.ts` (`openCardMenu`, `readDom`), and the
// `__name` shim `before` installs is the other half of the first one.

interface SidebarRow {
  id: string;
  name: string;
  active: boolean;
}

interface Shell {
  sidebar: boolean;
  perfHandle: boolean;
  personas: SidebarRow[];
  cardCount: number;
}

/**
 * The sidebar's persona list, plus whether the instrumentation is
 * there at all.
 *
 * The name is assembled from the row button's *direct text nodes*
 * only. `textContent` would drag in the count badge and the avatar
 * fallback bullet, and a name read as `"○ bench-persona-0 12345"`
 * would fail the prefix guard for a reason that has nothing to do with
 * the profile.
 */
async function readShell(): Promise<Shell> {
  return browser
    .execute(() => {
      const personas = Array.from(
        document.querySelectorAll("aside.sidebar li.persona-row"),
      ).map((li) => {
        const btn = li.querySelector("button");
        let label = "";
        if (btn) {
          for (const node of Array.from(btn.childNodes)) {
            if (node.nodeType === 3) label += node.nodeValue ?? "";
          }
        }
        return {
          id: li.getAttribute("data-persona-id") ?? "",
          name: label.replace(/[○●]/g, "").replace(/\s+/g, " ").trim(),
          active: btn !== null && btn.classList.contains("active"),
        };
      });
      return {
        sidebar: document.querySelector("aside.sidebar") !== null,
        perfHandle:
          typeof (window as unknown as { __ASTERISM_THUMB_PERF__?: unknown })
            .__ASTERISM_THUMB_PERF__ === "object",
        personas,
        cardCount: document.querySelectorAll(".grid-wrapper .card").length,
      };
    })
    .catch(() => ({
      // A script that cannot run yet is "nothing is there", not a
      // failure — every caller is polling.
      sidebar: false,
      perfHandle: false,
      personas: [] as SidebarRow[],
      cardCount: 0,
    }));
}

/** The sidebar's group rows, named the same careful way (`.tag-name`
 *  carries the checkbox glyph and the kind icon as siblings of the
 *  name text node). Dir rows share `.group-row` and have no
 *  `.group-main-btn`, so they come back with an empty name — indexes
 *  stay aligned with the click below, which queries the same list. */
async function readGroups(): Promise<SidebarRow[]> {
  return browser
    .execute(() => {
      return Array.from(document.querySelectorAll(".group-row")).map((li) => {
        const btn = li.querySelector("button.group-main-btn");
        const nameEl = btn ? btn.querySelector(".tag-name") : null;
        let label = "";
        if (nameEl) {
          for (const node of Array.from(nameEl.childNodes)) {
            if (node.nodeType === 3) label += node.nodeValue ?? "";
          }
        }
        return {
          id: li.getAttribute("data-drop-id") ?? "",
          name: label.replace(/[☑☐]/g, "").replace(/\s+/g, " ").trim(),
          active: btn !== null && btn.classList.contains("active"),
        };
      });
    })
    .catch(() => [] as SidebarRow[]);
}

/** Clicks the Nth row of a list, addressed by index because the label
 *  arithmetic above must not be duplicated inside a second callback
 *  where it could drift. The index is re-derived from a read taken
 *  immediately before, and the caller confirms the click by polling
 *  the row's `active` flag rather than trusting the return. */
async function clickRow(listSelector: string, index: number, button: string): Promise<boolean> {
  return browser
    .execute(
      (query: string, at: number, btnQuery: string) => {
        const rows = document.querySelectorAll(query);
        const row = rows[at];
        const btn = row ? row.querySelector(btnQuery) : null;
        if (btn instanceof HTMLElement) {
          btn.click();
          return true;
        }
        return false;
      },
      listSelector,
      index,
      button,
    )
    .catch(() => false);
}

interface ScrollerInfo {
  found: boolean;
  scrollTop: number;
  maxScroll: number;
  viewport: number;
  connected: boolean;
}

/**
 * Finds the element `virtua` scrolls and stashes it on `window`.
 *
 * Found by measurement rather than by selector: `VList` renders its own
 * scroll container inside `.grid-wrapper` with no stable class of ours
 * on it, so the deepest descendant with something to scroll is the
 * honest way to name it — and a selector that silently stopped
 * matching would produce a run where every `scrollTop` write went
 * nowhere and every window came back empty.
 */
async function resolveScroller(): Promise<ScrollerInfo> {
  return browser
    .execute(() => {
      const wrapper = document.querySelector(".grid-wrapper");
      const miss = {
        found: false,
        scrollTop: 0,
        maxScroll: 0,
        viewport: 0,
        connected: false,
      };
      if (!wrapper) return miss;
      let best: Element | null = null;
      let bestRange = 0;
      for (const el of [wrapper, ...Array.from(wrapper.querySelectorAll("*"))]) {
        const range = el.scrollHeight - el.clientHeight;
        if (range > bestRange) {
          best = el;
          bestRange = range;
        }
      }
      if (!best) return miss;
      (window as unknown as { __benchScroller?: Element }).__benchScroller = best;
      return {
        found: true,
        scrollTop: best.scrollTop,
        maxScroll: best.scrollHeight - best.clientHeight,
        viewport: best.clientHeight,
        connected: best.isConnected,
      };
    })
    .catch(() => ({
      found: false,
      scrollTop: 0,
      maxScroll: 0,
      viewport: 0,
      connected: false,
    }));
}

/** Writes `scrollTop` and reads the geometry back in the same trip —
 *  the list grows as rows hydrate, so `maxScroll` is not a constant. */
async function scrollTo(top: number): Promise<ScrollerInfo> {
  return browser
    .execute((to: number) => {
      const el = (window as unknown as { __benchScroller?: Element })
        .__benchScroller;
      if (!el || !el.isConnected) {
        return {
          found: false,
          scrollTop: 0,
          maxScroll: 0,
          viewport: 0,
          connected: false,
        };
      }
      el.scrollTop = to;
      return {
        found: true,
        scrollTop: el.scrollTop,
        maxScroll: el.scrollHeight - el.clientHeight,
        viewport: el.clientHeight,
        connected: true,
      };
    }, top)
    .catch(() => ({
      found: false,
      scrollTop: 0,
      maxScroll: 0,
      viewport: 0,
      connected: false,
    }));
}

/** Re-resolves once if the stashed node was replaced. */
async function scrollToResilient(top: number): Promise<ScrollerInfo> {
  const first = await scrollTo(top);
  if (first.found) return first;
  await resolveScroller();
  return scrollTo(top);
}

const EMPTY_DUMP: ThumbPerfDump = {
  entries: [],
  capacity: 0,
  recorded: 0,
  dropped: 0,
  blobUrlCount: 0,
  missingCount: 0,
  deadCount: 0,
};

async function perfReset(): Promise<void> {
  await browser
    .execute(() => {
      const handle = (
        window as unknown as {
          __ASTERISM_THUMB_PERF__?: { reset: () => void };
        }
      ).__ASTERISM_THUMB_PERF__;
      if (handle) handle.reset();
      return true;
    })
    .catch(() => false);
}

async function perfDump(): Promise<ThumbPerfDump> {
  return browser
    .execute(() => {
      const handle = (
        window as unknown as {
          __ASTERISM_THUMB_PERF__?: { dump: () => unknown };
        }
      ).__ASTERISM_THUMB_PERF__;
      return handle ? handle.dump() : null;
    })
    .then((value) => (value as ThumbPerfDump | null) ?? EMPTY_DUMP)
    .catch(() => EMPTY_DUMP);
}

async function perfStats(): Promise<ThumbPerfStats | null> {
  return browser
    .execute(() => {
      const handle = (
        window as unknown as {
          __ASTERISM_THUMB_PERF__?: { stats: () => unknown };
        }
      ).__ASTERISM_THUMB_PERF__;
      return handle ? handle.stats() : null;
    })
    .then((value) => (value as ThumbPerfStats | null) ?? null)
    .catch(() => null);
}

// --- result file ---------------------------------------------------

interface CycleResult {
  /** Jump number at the *end* of this window — the x-axis. */
  jump: number;
  window_ms: number;
  agg: ScrollWindowAgg;
  residency: ResidencySample;
  /** The module's own counters, kept as a cross-check on the
   *  driver-side arithmetic. */
  stats: ThumbPerfStats | null;
  rss: RssSample;
  scroll_top: number;
  max_scroll: number;
  /** Cards the grid had mounted when this window closed. This is the
   *  "viewport" the jump distance is measured in, read rather than
   *  assumed — the whole scenario is defined relative to it. */
  cards_on_screen: number;
  /** Jumps taken in this window, split by kind. Recorded because the
   *  mix is what makes one window's numbers comparable to another's —
   *  a window that happened to draw few far jumps sees a smaller
   *  working set, and that is a fact about the draw, not the app. */
  far_jumps: number;
  near_jumps: number;
}

interface OneWayResult {
  duration_ms: number;
  viewports_per_s: number;
  start_max_scroll: number;
  end_scroll_top: number;
  end_max_scroll: number;
  /** How much of the list the pass actually covered. Below 1 with
   *  `stopped_early` true means the ceiling cut it short — the numbers
   *  are still a valid sample of a moving grid, but they are not "the
   *  whole group". */
  covered_fraction: number;
  stopped_early: boolean;
  steps: number;
  agg: ScrollWindowAgg;
  residency: ResidencySample;
  stats: ThumbPerfStats | null;
  rss: RssSample;
}

function writeResult(body: unknown): string | null {
  if (!RESULTS_DIR) {
    console.warn("[bench-scroll] BENCH_RESULTS_DIR unset — result not written");
    return null;
  }
  const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  const file = path.join(RESULTS_DIR, `${stamp}-scroll.json`);
  fs.mkdirSync(RESULTS_DIR, { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(body, null, 2)}\n`, "utf8");
  return file;
}

// --- the run -------------------------------------------------------

describe("grid scroll bench", () => {
  before(async () => {
    // The shim first and on its own: it has to be in place before any
    // function-typed script reaches the page (see the file header's
    // pointer to `card-trash.spec.ts`).
    await browser.execute(
      "window.__name = window.__name || function (target) { return target; };",
    );

    // Cold start: a real window and a real SQLite open, against a
    // profile with 110,000 rows in it.
    const deadline = Date.now() + 120_000;
    let shell = await readShell();
    while (!shell.sidebar && Date.now() < deadline) {
      await sleep(500);
      shell = await readShell();
    }
    if (!shell.sidebar) {
      throw new Error("the app never painted its sidebar within 120 s");
    }

    // Persona rows arrive with the first catalog load, a little after
    // the shell.
    const listDeadline = Date.now() + 60_000;
    while (shell.personas.length === 0 && Date.now() < listDeadline) {
      await sleep(500);
      shell = await readShell();
    }

    await snap("00_guard");

    // --- the guard ------------------------------------------------
    //
    // Reported as one object so a wrong profile prints what it found
    // rather than stopping at the first assertion. Every branch below
    // ends the run before a single `scrollTop` is written.
    const names = shell.personas.map((p) => p.name);
    const foreign = names.filter((n) => !n.startsWith(PERSONA_PREFIX));
    if (names.length === 0) {
      throw new Error(
        "no personas in the sidebar — the bench profile looks unseeded " +
          "(run `just bench-seed-l` first)",
      );
    }
    if (foreign.length > 0) {
      throw new Error(
        `refusing to run: ${foreign.length} of ${names.length} personas do not ` +
          `carry the \`${PERSONA_PREFIX}\` prefix, so this is not the bench ` +
          `profile (dogfood / dev?). Found: ${JSON.stringify(names.slice(0, 12))}`,
      );
    }
    if (!names.includes(PERSONA)) {
      throw new Error(
        `persona \`${PERSONA}\` is not in this profile. Found: ` +
          JSON.stringify(names),
      );
    }
    if (!shell.perfHandle) {
      throw new Error(
        "window.__ASTERISM_THUMB_PERF__ is absent — the frontend was not " +
          "built with VITE_BENCH=1, so every measurement would be zero " +
          "(build via `just bench-scroll`, not `just ui-e2e`)",
      );
    }
    console.log(
      `[bench-scroll] guard ok: ${names.length} bench personas, ` +
        `instrumentation present, port ${PORT}`,
    );
  });

  it(`scrolls ${GROUP} one way, then jumps ${TOTAL_JUMPS}×${JUMP_VIEWPORTS_MIN}-${JUMP_VIEWPORTS_MAX} viewports`, async () => {
    // --- pick the persona ----------------------------------------
    const shell = await readShell();
    const personaIndex = shell.personas.findIndex((p) => p.name === PERSONA);
    if (personaIndex < 0) throw new Error(`persona \`${PERSONA}\` vanished from the sidebar`);
    if (!(await clickRow("aside.sidebar li.persona-row", personaIndex, "button"))) {
      throw new Error(`could not click the row for persona \`${PERSONA}\``);
    }
    {
      const deadline = Date.now() + 30_000;
      for (;;) {
        const rows = (await readShell()).personas;
        if (rows.some((p) => p.name === PERSONA && p.active)) break;
        if (Date.now() >= deadline) {
          throw new Error(`persona \`${PERSONA}\` never became the active filter`);
        }
        await sleep(250);
      }
    }

    // --- turn on the group filter --------------------------------
    //
    // The group list reloads on the persona change, so the row is
    // waited for rather than read once.
    let groups: SidebarRow[] = [];
    {
      const deadline = Date.now() + 60_000;
      for (;;) {
        groups = await readGroups();
        if (groups.some((g) => g.name === GROUP)) break;
        if (Date.now() >= deadline) {
          throw new Error(
            `group \`${GROUP}\` never appeared under persona \`${PERSONA}\`. ` +
              `Found: ${JSON.stringify(groups.map((g) => g.name).filter((n) => n !== ""))}`,
          );
        }
        await sleep(500);
      }
    }
    const groupIndex = groups.findIndex((g) => g.name === GROUP);
    if (!(await clickRow(".group-row", groupIndex, "button.group-main-btn"))) {
      throw new Error(`could not click the row for group \`${GROUP}\``);
    }
    {
      const deadline = Date.now() + 30_000;
      for (;;) {
        if ((await readGroups()).some((g) => g.name === GROUP && g.active)) break;
        if (Date.now() >= deadline) {
          throw new Error(`group \`${GROUP}\` never became active`);
        }
        await sleep(250);
      }
    }

    // --- wait for the grid ---------------------------------------
    {
      const deadline = Date.now() + 120_000;
      for (;;) {
        if ((await readShell()).cardCount > 0) break;
        if (Date.now() >= deadline) {
          throw new Error(
            `no cards painted for ${PERSONA} / ${GROUP} within 120 s`,
          );
        }
        await sleep(500);
      }
    }
    await snap("01_filtered");

    let scroller = await resolveScroller();
    if (!scroller.found || scroller.maxScroll <= 0) {
      throw new Error(
        `no scrollable element under .grid-wrapper (maxScroll=${scroller.maxScroll}) — ` +
          "the filtered set may be smaller than one viewport",
      );
    }
    console.log(
      `[bench-scroll] scroller: viewport ${scroller.viewport}px, ` +
        `range ${scroller.maxScroll}px`,
    );

    // --- phase A: one way, constant speed ------------------------
    await scrollToResilient(0);
    // Let the top settle before the clock starts: fetches triggered by
    // the jump back to 0 belong to the setup, not to the pass.
    await sleep(1_000);
    await perfReset();

    const oneWayStartMax = scroller.maxScroll;
    const oneWayStart = Date.now();
    let steps = 0;
    let stoppedEarly = false;
    for (;;) {
      const elapsedS = (Date.now() - oneWayStart) / 1000;
      if (elapsedS > ONE_WAY_MAX_S) {
        stoppedEarly = true;
        break;
      }
      // Position from elapsed wall time, so round-trip jitter changes
      // the step size and not the speed.
      const target = elapsedS * ONE_WAY_VIEWPORTS_PER_S * scroller.viewport;
      const reached = await scrollToResilient(Math.min(target, scroller.maxScroll));
      steps += 1;
      if (!reached.found) {
        throw new Error("the grid scroller went away mid-pass");
      }
      scroller = reached;
      if (target >= reached.maxScroll) break;
      await sleep(STEP_GAP_MS);
    }
    const oneWayMs = Date.now() - oneWayStart;
    // The tail of the pass is still resolving; the design says not to
    // wait for tiles, but a dump taken in the same millisecond as the
    // last write would drop the fetches the last screen just started.
    await sleep(2_000);

    const oneWayDump = await perfDump();
    const oneWayStats = await perfStats();
    const oneWay: OneWayResult = {
      duration_ms: oneWayMs,
      viewports_per_s: ONE_WAY_VIEWPORTS_PER_S,
      start_max_scroll: oneWayStartMax,
      end_scroll_top: scroller.scrollTop,
      end_max_scroll: scroller.maxScroll,
      covered_fraction:
        scroller.maxScroll > 0 ? scroller.scrollTop / scroller.maxScroll : 0,
      stopped_early: stoppedEarly,
      steps,
      agg: aggregateWindow(oneWayDump.entries),
      residency: residencyOf(oneWayDump),
      stats: oneWayStats,
      rss: sampleRss(),
    };
    console.log(
      `[bench-scroll] one-way: ${oneWay.agg.count} fetches, ` +
        `p50 ${oneWay.agg.p50_ms}ms p95 ${oneWay.agg.p95_ms}ms ` +
        `over5s ${oneWay.agg.over_5s_count} wasted ${oneWay.agg.wasted_rate}` +
        (stoppedEarly ? ` (STOPPED EARLY at ${ONE_WAY_MAX_S}s)` : ""),
    );
    await snap("02_one_way_done");

    // --- phase B: jump repetition, sampled every N jumps ----------
    //
    // Each jump lands a whole screenful away, at least. What is being
    // measured is how the Nth screenful compares to the first: the
    // requests behind the previous jumps are never cancelled
    // (`thumb.svelte.ts:202`), each one retries on a 250→4000 ms
    // backoff, and a card whose budget runs out is added to
    // `#missingThumb` and never fetched again (`:210-213`). Whether
    // that queue keeps up under repeated flicking is the question.
    const random = mulberry32(SCROLL_SEED);
    const cycles: CycleResult[] = [];
    await perfReset();
    const phaseBStart = Date.now();
    let windowStart = phaseBStart;

    let farJumps = 0;
    let nearJumps = 0;

    for (let jump = 1; jump <= TOTAL_JUMPS; jump += 1) {
      // The kind is drawn first and unconditionally, so the seeded
      // stream advances the same way whichever branch runs — a seed
      // reproduces a run rather than a branch history.
      const far = random() < FAR_JUMP_RATE;
      let target: number;
      if (far) {
        // Uniform over the whole group: dragging the scrollbar to
        // somewhere else entirely.
        target = random() * scroller.maxScroll;
        farJumps += 1;
      } else {
        // A fixed several screenfuls away, either direction. The
        // distance is what guarantees the landing spot shares nothing
        // with what was just on screen.
        const viewports =
          JUMP_VIEWPORTS_MIN + random() * (JUMP_VIEWPORTS_MAX - JUMP_VIEWPORTS_MIN);
        const direction = random() < 0.5 ? -1 : 1;
        target = Math.min(
          Math.max(0, scroller.scrollTop + direction * viewports * scroller.viewport),
          scroller.maxScroll,
        );
        nearJumps += 1;
      }
      const reached = await scrollToResilient(target);
      if (!reached.found) throw new Error("the grid scroller went away mid-run");
      scroller = reached;

      // Not long enough for the tiles to arrive — that is the point.
      await sleep(JUMP_GAP_MS);

      if (jump % JUMPS_PER_WINDOW === 0 || jump === TOTAL_JUMPS) {
        const windowMs = Date.now() - windowStart;
        const dump = await perfDump();
        const stats = await perfStats();
        const cardsOnScreen = (await readShell()).cardCount;
        await perfReset();
        windowStart = Date.now();
        const cycle: CycleResult = {
          jump,
          window_ms: windowMs,
          agg: aggregateWindow(dump.entries),
          residency: residencyOf(dump),
          stats,
          rss: sampleRss(),
          scroll_top: scroller.scrollTop,
          max_scroll: scroller.maxScroll,
          cards_on_screen: cardsOnScreen,
          far_jumps: farJumps,
          near_jumps: nearJumps,
        };
        cycles.push(cycle);
        farJumps = 0;
        nearJumps = 0;
        console.log(
          `[bench-scroll] jump ${jump}/${TOTAL_JUMPS}: ` +
            `${cycle.agg.count} fetches, p50 ${cycle.agg.p50_ms}ms ` +
            `p95 ${cycle.agg.p95_ms}ms over5s ${cycle.agg.over_5s_count} ` +
            `missing ${cycle.residency.missingCount} ` +
            `blobs ${cycle.residency.blobUrlCount} ` +
            `cards ${cardsOnScreen} ` +
            `rss ${cycle.rss.app_rss_kb}kB ` +
            `(${cycle.far_jumps}far/${cycle.near_jumps}near)`,
        );
        await snap(`03_jump_${String(jump).padStart(4, "0")}`);
      }
    }

    const file = writeResult({
      // v2: phase B mixes uniform whole-group jumps into the walk
      // (`FAR_JUMP_RATE`). A v1 file's later windows measured a warm
      // neighbourhood cache, so the two are not comparable series.
      // v3: phase B is jump repetition sampled per N jumps, not a
      // ±3-viewport walk sampled per minute. A v1/v2 file's later
      // windows measured a warm neighbourhood cache; these measure the
      // Nth screenful. Not the same series.
      schema: "bench-scroll-v3",
      generated_at: new Date().toISOString(),
      seed_corpus: CORPUS_SEED,
      persona: PERSONA,
      group: GROUP,
      scroll_seed: SCROLL_SEED,
      jumps: TOTAL_JUMPS,
      jumps_per_window: JUMPS_PER_WINDOW,
      jump_viewports: [JUMP_VIEWPORTS_MIN, JUMP_VIEWPORTS_MAX],
      jump_gap_ms: JUMP_GAP_MS,
      far_jump_rate: FAR_JUMP_RATE,
      one_way: oneWay,
      cycles,
      env: { port: PORT, profile: "bench" },
    });
    console.log(`[bench-scroll] wrote ${file ?? "(nothing)"}`);

    // The assertions are about the *run*, not about the app: a bench
    // that produced no measurement must not report success, and the
    // verdict on the numbers themselves belongs to whoever reads the
    // file (the bars are for a person).
    expect(oneWay.agg.count).toBeGreaterThan(0);
    if (TOTAL_JUMPS > 0) {
      expect(cycles.length).toBeGreaterThan(0);
    }
    expect(file).not.toBe(null);
  });
});
