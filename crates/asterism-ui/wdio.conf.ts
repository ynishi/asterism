// WebdriverIO config for the desktop e2e suite (`just ui-e2e`).
//
// These specs exist for the questions the HTTP API cannot answer. The
// backend can say what is in the database and, since the sort axis
// reached the wire, what order a filter produces — but not whether a
// drag actually lands on the row under the pointer, whether an
// affordance lights up for a move the drop would refuse, or whether a
// panel renders what it was handed. Those live in the webview.
//
// # Why the app carries its own driver
//
// macOS ships no WKWebView WebDriver, so `tauri-driver` cannot reach
// this app from outside. The `embedded` provider (the default on every
// platform) runs a W3C server *inside* the binary instead — which is
// why the app has to be built with the `wdio` cargo feature and the
// matching capability. A default build has neither, so nothing here can
// point at a shipped app by accident.
//
// # Profile
//
// The app runs against the disposable `dev` profile with its own port,
// never Dogfood. An e2e suite that clicked through the User's real
// library would be a data-loss bug wearing a test's clothes.

import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "../..");

// Per-run screenshot dir (see card-trash.spec.ts `snapStage`). Created
// in `onPrepare` (launcher process) and handed to the worker through
// the environment, because the worker re-evaluates this module and
// would otherwise mint a second timestamp. Capped at the newest 10 runs
// so the trail cannot quietly eat the disk — this dir holds disposable
// machine artifacts, same tier as the tee'd logs next to it.
const screensRoot = path.join(repoRoot, "workspace/test-logs/e2e-screens");

// Built by `just ui-e2e` — plain `cargo build --features wdio`, not a
// bundle. The binary reads the Vite output from `frontendDist`, so the
// recipe runs `npm run build` first.
const appBinary = path.join(repoRoot, "target/debug/asterism-ui");

// The app's only window, and the handle the embedded driver knows it
// by. `tauri.conf.json` declares `app.windows: []` and the window is
// built programmatically under the label `"main"`
// (`src-tauri/src/lib.rs:165`); the e2e capability grants `wdio:*` to
// that same label (`src-tauri/tauri.e2e.conf.json:12-14`).
//
// Label and handle are the same string here, which is what makes the
// opt-out below safe to write as a literal: the embedded WebDriver
// answers `GET /window/handles` with `webview_windows().keys()` and
// resolves `POST /window` by looking the handle up in that same map
// [measured 2026-08-11, tauri-plugin-wdio-webdriver 1.2.0
// src/server/handlers/window.rs:61 and :113].
const WINDOW_LABEL = "main";

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./e2e/**/*.spec.ts"],
  maxInstances: 1,

  // Without this, `browserName: "tauri"` is just an unknown browser and
  // wdio tries to start Chrome.
  services: ["@wdio/tauri-service"],

  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": {
        application: appBinary,
        // A port of its own: the suite must not collide with a Dev app
        // the developer already has open, and must not be able to reach
        // the Dogfood port at all.
        args: ["--port", "19899"],
      },
      "wdio:tauriServiceOptions": {
        // **The key the service spawns from.** `tauri:options.args`
        // above only reaches a debug log inside the service; the child
        // process is spawned from `appArgs`
        // [measured 2026-08-05, @wdio/tauri-service dist/esm/index.js
        // `startEmbeddedDriver` → `spawnTauriApp(appBinaryPath,
        // options.appArgs, env)`].
        //
        // Until this key existed the port above was a comment: the app
        // fell back to the profile default (dev = 18989), so the "port
        // of its own" the note promises was never in effect and a run
        // started while `just dev` was open would fail to bind. In a
        // window (not `--headless`) that failure is a warning and the
        // app keeps going without HTTP, which is why no e2e spec ever
        // reported it — none of them talk to the app over HTTP.
        appArgs: ["--port", "19899"],
        env: {
          ASTERISM_PROFILE: "dev",
          ASTERISM_HOME: path.join(repoRoot, "workspace/runtime/e2e"),
        },
      },
    },
  ],

  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  // The app opens a real window and a real SQLite core; the first spec
  // in a run pays for both. 300 s, not 120: card-trash.spec budgets its
  // own per-step ceilings (~200 s worst case, see its header) and needs
  // its `finally` fixture-restore to run inside the mocha budget — a
  // mocha timeout mid-test is what leaves the e2e profile trashed.
  mochaOpts: { ui: "bdd", timeout: 300_000 },

  // Opt out of the service's per-command auto-focus.
  //
  // **What is being opted out of.** The service hooks `beforeCommand`
  // and, for `getTitle` / `findElement` / `findElements` / `$` / `$$` /
  // `elementClick`, asks the app for its window states over
  // `core.invoke` before letting the command through
  // [@wdio/tauri-service dist/esm/index.js:3891-3898 (the hook), :2964
  // (the command list), :2914 (`getWindowStates`)]. This webview
  // exposes no `core.invoke` — the app does not enable
  // `withGlobalTauri` — so the injected script busy-waits and throws
  // "Tauri core.invoke not available after 5s timeout" [:3053-3057].
  // The service logs that and carries on, which makes it a silent ~5 s
  // surcharge on every element command, and on every poll of a wait
  // that uses one.
  //
  // **Why it has to go through `browser.tauri`.** The check returns
  // immediately once the session id is in `userSwitchedWindowCache`
  // [:2959]. Exactly one function writes that set —
  // `switchWindowByLabel` [:2848] — and the only way in is
  // `browser.tauri.switchWindow(label)` [:4095-4100].
  //
  // The built-in `browser.switchWindow()` is a different function and
  // never reaches the service at all: handed a handle equal to the
  // current one it returns on the spot, and otherwise it calls
  // `switchToWindow` directly [webdriverio/build/index.js:5713-5719].
  // The hook that stood here until 2026-08-11 called exactly that with
  // `getWindowHandles()[0]`, so it was a no-op wearing a comment that
  // claimed the tax was gone. It never was: `card-trash.spec.ts` timed
  // a single `$(sel).click()` — five to six taxed commands — at 25-30 s
  // against its own 30 s step ceilings, which is the 2026-08-01 flake.
  //
  // **Why `beforeSuite`.** `browser.tauri` is installed by the
  // service's own `before` [:3808]. Config-file hooks are registered
  // ahead of service hooks [@wdio/config/build/node/index.js:339 vs
  // @wdio/runner/build/index.js:696-700] and every `before` is started
  // concurrently under one `Promise.all`
  // [@wdio/utils/build/index.js:942-965], so at the top of a config
  // `before` the property does not exist yet. `beforeSuite` is a
  // root-suite `beforeAll` [@wdio/mocha-framework/build/index.js:243]:
  // it runs once, after every `before` hook has resolved.
  //
  // **Cost.** The validation step inside `switchWindowByLabel`
  // (`listWindowLabels`, :2833) goes through `core.invoke` too, so the
  // opt-out pays the 5 s timeout once and is then waved through
  // [:2838-2843]; the switch itself proceeds and the cache entry
  // survives because the label is a valid handle (see `WINDOW_LABEL`).
  //
  // **If it stops working** nothing fails, it gets slow: the ~5 s
  // surcharge returns to every element command and card-trash walks
  // back into its ceilings. The `console.warn` below is the only
  // notice, so correlate it with a suite that suddenly takes minutes.
  beforeSuite: async () => {
    // The service augments the browser object at runtime but ships no
    // ambient declaration for it, so name the one method we call.
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
        `[wdio.conf] auto-focus opt-out failed (${String(error)}) — every ` +
          `element command now pays the ~5 s core.invoke timeout.`,
      );
    }
  },

  onPrepare: () => {
    const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    const dir = path.join(screensRoot, stamp);
    fs.mkdirSync(dir, { recursive: true });
    process.env.E2E_SCREENS_DIR = dir;
    const runs = fs
      .readdirSync(screensRoot)
      .filter((name) => !name.startsWith("."))
      .sort();
    for (const old of runs.slice(0, Math.max(0, runs.length - 10))) {
      fs.rmSync(path.join(screensRoot, old), { recursive: true, force: true });
    }
  },

  // One frame at the moment of death, named after the test. The
  // per-stage trail (card-trash.spec.ts `snapStage`) shows the path
  // here; this shows the destination. `takeScreenshot` is untaxed.
  afterTest: async (test, _context, result) => {
    const dir = process.env.E2E_SCREENS_DIR;
    if (!dir || result.passed) return;
    try {
      const safe = test.title.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 80);
      await browser.saveScreenshot(path.join(dir, `FAIL_${safe}.png`));
    } catch {
      // Best-effort: diagnostics must not cascade a failure.
    }
  },
};
