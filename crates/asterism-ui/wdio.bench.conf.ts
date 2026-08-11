// WebdriverIO config for the scroll bench (`just bench-scroll`).
//
// A sibling of `wdio.conf.ts`, not a variant of it. The e2e suite asks
// whether a gesture is wired and finishes in a couple of minutes; this
// one drives a scripted scroll for ten and reports latency
// distributions. They share the embedded-WebDriver mechanics — the `wdio` cargo
// feature, the `browserName: "tauri"` capability, the auto-focus
// opt-out — and disagree about everything a run touches, which is why
// this is a second file rather than a flag on the first: a bench spec
// that could be picked up by `just ui-e2e` would put a ten-minute run
// inside the check loop.
//
// # Four deliberate differences from `wdio.conf.ts`
//
// **The profile.** `ASTERISM_PROFILE=bench` and — the part that is
// easy to get wrong — **no `ASTERISM_HOME`**. The e2e suite pins its
// home to a directory in the repo; the bench must land on
// `~/.asterism/profiles/bench`, the profile `just bench-seed-l`
// populates, so it lets the app's own path resolution find it. That
// also keeps the `.asterism-profile` marker check in play, which is
// the thing standing between a bench run and the Dogfood database.
// The scenario re-checks the profile from the inside before it scrolls
// (see `bench-scroll.spec.ts`), because an env var says what was
// asked for and the persona list says what was opened.
//
// **The port.** 19898: not the e2e suite's 19899, not Dogfood's 8989,
// not the bench backend's 28989. A bench run must not be able to
// attach to something already open.
//
// **The clock.** One `it` covers the whole scenario, so mocha's budget
// is derived from `BENCH_SCROLL_MINUTES` rather than fixed. A default
// 300 s ceiling would kill a ten-minute run at its third cycle.
//
// **The output.** `workspace/bench-results/` is created here and
// handed to the worker through the environment, the same way
// `wdio.conf.ts` hands over its screenshot dir — the worker
// re-evaluates this module and would otherwise mint a second
// timestamp.

import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "../..");

/** A port of its own — see the header. */
const BENCH_PORT = "19898";

// Kept apart from `e2e-screens` so the retention sweep below cannot
// reach the e2e trail, and so a bench run's frames are legible as a
// bench run's.
const screensRoot = path.join(repoRoot, "workspace/test-logs/bench-screens");
const resultsRoot = path.join(repoRoot, "workspace/bench-results");

// Same binary `just ui-e2e` builds, with `VITE_BENCH=1` added to the
// frontend half so `thumb-perf.ts` records (see its header: the e2e
// build's frontend is a production Vite build, where a `DEV`-only gate
// is a no-op).
const appBinary = path.join(repoRoot, "target/debug/asterism-ui");

/** Same window, same reasoning as `wdio.conf.ts` — the app builds one
 *  window labelled `"main"` (`src-tauri/src/lib.rs:165`) and the
 *  embedded driver uses labels as handles. */
const WINDOW_LABEL = "main";

const jumps = Number(process.env.BENCH_JUMPS ?? "200");
const jumpGapMs = Number(process.env.BENCH_JUMP_GAP_MS ?? "400");
// Whole-scenario budget: the jump phase plus the one-way pass, the
// cold start, and the aggregation. The margin is deliberately large —
// nothing waits for this ceiling on a healthy run, and a ceiling that
// lands mid-scenario produces a bare mocha timeout with no result file
// written, which is the one failure mode that wastes the whole run.
//
// The jump phase is bounded by count, not by clock, so the estimate is
// per-jump: the gap plus a generous allowance for the round-trip and
// the per-window dump. A jump that takes longer than that allowance is
// exactly the finding the run is after, so the allowance is padded
// rather than tight.
const perJumpMs =
  (Number.isFinite(jumpGapMs) && jumpGapMs > 0 ? jumpGapMs : 400) + 2_000;
const mochaTimeoutMs =
  (Number.isFinite(jumps) && jumps > 0 ? jumps : 200) * perJumpMs + 15 * 60_000;

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./e2e-bench/**/*.spec.ts"],
  maxInstances: 1,

  services: ["@wdio/tauri-service"],

  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": {
        application: appBinary,
        args: ["--port", BENCH_PORT],
      },
      "wdio:tauriServiceOptions": {
        // **The one the service actually spawns with.** `tauri:options.args`
        // above reaches only a debug log inside the service
        // (`onPrepare` validates it and drops it); the child process is
        // spawned from `wdio:tauriServiceOptions.appArgs`
        // [measured 2026-08-05, @wdio/tauri-service dist/esm/index.js
        // `startEmbeddedDriver` → `spawnTauriApp(appBinaryPath,
        // options.appArgs, env)`].
        //
        // Without it the app fell back to the profile's default port
        // (bench = 28989) and `--port 19898` was a comment. Nothing
        // noticed while the specs only drove the UI; the first one to
        // *talk* to the app over HTTP got `Connection refused`.
        //
        // Both keys are kept in sync deliberately: the service's own
        // `createTauriCapabilities` helper writes both, so a reader
        // comparing this conf to the service's output finds the same
        // shape.
        appArgs: ["--port", BENCH_PORT],
        env: {
          // Profile only. `ASTERISM_HOME` is withheld on purpose —
          // see the header.
          ASTERISM_PROFILE: "bench",
        },
      },
    },
  ],

  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: mochaTimeoutMs },

  // Identical to `wdio.conf.ts`, and for the identical reason: without
  // it every `getTitle` / `findElement` / `findElements` / `$` / `$$` /
  // `elementClick` pays a 5 s "Tauri core.invoke not available"
  // timeout, because the service asks the app for its window states
  // before each of those and this webview exposes no `core.invoke`. The
  // full argument — which cache, which call reaches it, and why the
  // built-in `browser.switchWindow()` does not — is written out once in
  // `wdio.conf.ts` above its own copy of this hook; the two must not
  // drift.
  //
  // Two things carry over verbatim and are worth restating here. The
  // only writer of the service's `userSwitchedWindowCache` is
  // `switchWindowByLabel`, reachable solely via
  // `browser.tauri.switchWindow(label)` [@wdio/tauri-service
  // dist/esm/index.js:2848, :4095-4100] — the built-in `switchWindow()`
  // never touches it. And this has to be `beforeSuite`, not `before`,
  // because `browser.tauri` is installed by the service's own `before`
  // [:3808], which starts *after* a config-file `before` begins
  // [@wdio/utils/build/index.js:942-965].
  //
  // The stakes differ from the e2e suite's. The scenario issues
  // thousands of `browser.execute` calls, which were never taxed, so
  // losing this makes the handful of element commands around them slow
  // rather than wrong — but a bench that silently pays 5 s per element
  // command is reporting latency for a run it perturbed, so the
  // `console.warn` matters more here than the wall clock does.
  beforeSuite: async () => {
    const tauri = (
      browser as unknown as {
        tauri?: { switchWindow(label: string): Promise<void> };
      }
    ).tauri;
    try {
      if (!tauri) {
        throw new Error("browser.tauri is missing (service before() hook)");
      }
      await tauri.switchWindow(WINDOW_LABEL);
    } catch (error) {
      console.warn(
        `[wdio.bench.conf] auto-focus opt-out failed (${String(error)}) — ` +
          `every element command now pays the ~5 s core.invoke timeout.`,
      );
    }
  },

  onPrepare: () => {
    const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    const dir = path.join(screensRoot, stamp);
    fs.mkdirSync(dir, { recursive: true });
    process.env.BENCH_SCREENS_DIR = dir;

    // The result files are the point of the run, so unlike the
    // screenshots they are never swept.
    fs.mkdirSync(resultsRoot, { recursive: true });
    process.env.BENCH_RESULTS_DIR = resultsRoot;
    process.env.BENCH_PORT = BENCH_PORT;

    const runs = fs
      .readdirSync(screensRoot)
      .filter((name) => !name.startsWith("."))
      .sort();
    for (const old of runs.slice(0, Math.max(0, runs.length - 10))) {
      fs.rmSync(path.join(screensRoot, old), { recursive: true, force: true });
    }
  },

  afterTest: async (test, _context, result) => {
    const dir = process.env.BENCH_SCREENS_DIR;
    if (!dir || result.passed) return;
    try {
      const safe = test.title.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 80);
      await browser.saveScreenshot(path.join(dir, `FAIL_${safe}.png`));
    } catch {
      // Diagnostics must not cascade a failure.
    }
  },
};
