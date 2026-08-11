// Trashing a card from the grid, without a drag.
//
// The backend half of this is already covered elsewhere: `trash_asset`
// sets `trashed_at` and every subsequent query filters on it. What no
// HTTP assertion can say is whether the grid *offers* the action at all,
// and — since 2026-08-01 — whether it offers it in the right place.
//
// The action used to be a 🗑 in the card's hover strip, and its sibling
// "Delete forever" a 🔥 next to it on the trash side. Both moved into
// the card context menu: opened deliberately, last entry, separated,
// in destructive tone. That is the standard shape (Apple HIG; none of
// the eight library apps surveyed put removal behind hover), and it is
// what this spec asserts — placement included, because "the app can
// trash a card" would pass just as well with the button back under the
// pointer where a mis-aim reaches it.
//
// Two later specs cover the rest of the removal surface, and both are
// here rather than in a file of their own because they need the same
// fixture discipline: the trash-side toolbar (which only has half a
// meaning against an empty trash, so each one trashes a card first and
// owes it back), and the Undo toast that every trash gesture arms.
//
// The Empty Trash button is exercised **as far as its confirmation and
// no further**. Cancel is clicked; the destructive button is not named
// by any selector in this file. The suite runs against a profile on
// disk, and answering that question the other way would delete every
// trashed asset on it with nothing left to restore from.
//
// # Every wait is bounded, every failure names its step, and DOM reads
// # do not go through the element API
//
// Three findings, in the order they were made, because the third
// explains the first two.
//
// **One.** The first version chained waits of 10-20 s each — and
// `setTrashView` re-ran a 20 s `waitForExist` on *every poll* of its
// own `waitUntil` — so the happy path could want more than mocha's
// whole per-test budget without any single step being stuck. That
// produced the worst possible failure: a bare mocha `Timeout` with no
// step named, and a `finally` cut off mid-recovery, leaving the
// fixture's one asset in the trash for the next run to find. Fixed by
// `stage()`: every await is raced against a ceiling, and the error
// carries a breadcrumb trail of the steps that already passed.
//
// **Two.** With that in place the failures started naming themselves,
// and they landed on a step that made no sense — sometimes the first
// read of a card id, later `strip no longer carries a trash icon`,
// never `read menu`. Raising the ceilings from 4 s to 15 s moved the
// symptom without removing it.
//
// **Three.** The reason is in `readDom` below, and it is worth reading
// before touching anything here: in this environment **every element
// command costs ~6 s** and `browser.execute` costs nothing. The steps
// that failed were the ones spending two element commands against a
// one-command budget; the step that never failed was the one that was
// already a single `execute`. So DOM reads go through `readDom` /
// `existsInPage` / `pollUntil`, and only clicks remain element
// commands. Reintroducing a `$(...)` query — or a `waitForExist` — is
// how the flake comes back.
//
// # What this spec does not assert, and why
//
// That a *pointer* can reach a menu entry. The driver's pointer input is
// a `new MouseEvent` handed to `dispatchEvent`
// (`tauri-plugin-wdio-webdriver`), and a synthetic event never moves the
// engine's own hover state; `HTMLElement.click()` fires a handler
// regardless of `pointer-events`. So the menu is opened by dispatching
// `contextmenu` and its entries are clicked through the DOM. Both are
// honest about what they cover: the entry exists, it is where it should
// be, and it does the thing. Neither can vouch for hit-testing.
//
// That gap is smaller than it was. The old spec had to assert around the
// hover reveal — a strip at `opacity: 0` that WebKit would not reveal
// via `:focus-within` either, because it only matches the focus
// pseudo-classes while the document has focus and the window driving
// this suite is not the key window (measured 2026-08-01: `focusIsIcon:
// true` next to `stripFocusWithin: false`). A menu has no reveal to
// work around; it is in the DOM or it is not.
//
// That the menu opens on the *first* dispatch. `openCardMenu` retries,
// because a human whose right-click produced nothing presses again and
// a spec that failed on the first miss would hold the app to something
// stricter than it promises. The retry is not silent: a menu that
// needed more than one dispatch prints a `console.warn` with the
// per-attempt diagnostics and adds a breadcrumb saying which attempt
// won, so "it stopped opening on the first try" stays visible instead
// of being absorbed into a green tick. If that line starts appearing,
// it is a finding, not noise.
//
// # Why this restores what it trashes
//
// The suite runs against a profile that lives on disk
// (`workspace/runtime/e2e`), not a fresh database per run. A spec that
// trashed an asset and walked away would shrink the fixture every time
// it ran, and would eventually be asserting against an empty grid. The
// undo goes through the UI rather than a command, because the webview
// is built without `withGlobalTauri` — there is no `invoke` handle to
// reach from `browser.execute`, and adding one so a test could take a
// shortcut would widen the app's surface for the test's convenience.
//
// Restore is still the ↩︎ icon in the strip on the trash side (it is
// not destructive, so it did not move), which is why `clickCardIcon`
// survives the rewrite.

import { browser, $ } from "@wdio/globals";
import path from "node:path";

/**
 * Screenshot trail. `wdio.conf.ts` `onPrepare` creates a per-run dir
 * and exports it as `E2E_SCREENS_DIR`; every completed `stage()` drops
 * one PNG there, numbered in execution order. The point is liveness and
 * legibility from outside: watching the dir answers "is it running or
 * hung, and what is the UI doing right now", and after a failure the
 * trail shows what the window looked like at each step.
 *
 * `takeScreenshot` is not on the tax list (see `readDom`'s header), so
 * this costs a plain round-trip, not 6 s. Best-effort by design: a
 * screenshot must never be the reason a test fails or stalls, so the
 * call is raced against its own small ceiling and all errors are eaten.
 */
const SCREENS_DIR = process.env.E2E_SCREENS_DIR;
let shotSeq = 0;
async function snapStage(name: string, failed = false): Promise<void> {
  if (!SCREENS_DIR) return;
  shotSeq += 1;
  const safe = name.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 60);
  const file = path.join(
    SCREENS_DIR,
    `${String(shotSeq).padStart(3, "0")}_${failed ? "FAIL_" : ""}${safe}.png`,
  );
  try {
    await Promise.race([
      browser.saveScreenshot(file),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
  } catch {
    // Liveness aid only.
  }
}

const RESTORE_ICON = '[aria-label="Restore from trash"]';
/** The removal tier is the only destructive-toned entry in either
 *  menu, which makes the class a stabler handle than the label — the
 *  label carries a count when the selection is a multi-select. */
const MENU_DANGER_ITEM = ".card-menu .card-menu-item-danger";
/** Stable in both view states; the label text flips between ○ and ◉. */
const TRASH_VIEW_TOGGLE = 'aside.sidebar button[title^="Show trashed items"]';

// --- budgets -------------------------------------------------------
//
// Sized against mocha's 300 s per-test ceiling (`wdio.conf.ts`), and
// the property being bought is narrower than "the sum fits": the
// *first* step to stall throws inside its own budget and names itself,
// which ends the test long before mocha's clock matters. The original
// numbers (10-20 s, nested) were large enough that a single stall plus
// ordinary latency could reach mocha first — and then mocha, not the
// step, wrote the error.
//
// The floors stay generous even though DOM reads are now untaxed (see
// `readDom`), for two reasons: the three remaining element commands
// still pay ~6 s each unless the `wdio.conf.ts` opt-out lands, and a
// budget that is merely *sufficient* is cheap here — nothing waits for
// its ceiling on a healthy run. What is not cheap is a ceiling smaller
// than a step's real cost, which is what produced the 2026-08-01 flake
// (`read first card id`, then `strip no longer carries a trash icon`:
// two taxed commands each, against a one-command budget).

/** One driver round-trip. Sized for a *taxed* command, so it is
 *  generous for the `execute` calls that make up most of the file. */
const DRIVER_MS = 15_000;
/** Something already on screen has to be found. */
const PRESENT_MS = 15_000;
/** The grid has to change after a backend round-trip: a row leaves,
 *  a row arrives, the trash view repaints. */
const GRID_MS = 20_000;
/** How long one `contextmenu` dispatch gets to produce a menu before
 *  the next dispatch. Deliberately short: the menu is a synchronous
 *  render away from a handler that ran, so a long wait here buys
 *  nothing and delays both the retry and the diagnosis. */
const MENU_ATTEMPT_MS = 2_000;
/** Two consecutive identical reads of the grid's shape, this far
 *  apart, count as "settled". */
const SETTLE_GAP_MS = 500;
/** Gap between `execute`-based polls. Short because those are the
 *  untaxed commands — see the note on `readDom`. */
const POLL_GAP_MS = 250;

/**
 * Everything the page-side probe needs, installed once.
 *
 * Written as a *string*, and in ES5 (`var`, `function`), for one
 * reason: a string is the single script shape the compiler never
 * rewrites. Every other script in this file is an anonymous arrow in
 * argument position for the same seam (see the note on `openCardMenu`),
 * but this one has to declare a reusable listener — the exact shape
 * esbuild's name preservation would turn into `__name(rec, "rec")` —
 * so it stays out of the compiler's reach entirely.
 *
 * The `__name` shim rides along, since it must be in place before any
 * function-typed script runs.
 *
 * Three collectors, all writing into whichever `__cardMenuProbe` object
 * is current, and all installed exactly once so nothing ever has to be
 * removed (a removable listener would need a named reference):
 *
 *   * a MutationObserver on `document.body`, which is the only way to
 *     catch a `.card-menu` that appears and disappears faster than the
 *     driver polls;
 *   * capture-phase listeners for the events that can close the menu,
 *     so a close has a cause attached rather than being inferred;
 *   * nothing else — the card's own `.selected` class is read at
 *     dispatch and read-back time, which needs no listener.
 */
const INSTALL_PROBE = `
  window.__name = window.__name || function (target) { return target; };
  if (!window.__cardMenuProbeInstalled) {
    window.__cardMenuProbeInstalled = true;
    window.__cardMenuProbe = null;
    window.__cardMenuProbeNode = null;
    new MutationObserver(function (records) {
      var p = window.__cardMenuProbe;
      if (!p) { return; }
      for (var i = 0; i < records.length; i++) {
        var added = records[i].addedNodes;
        for (var a = 0; a < added.length; a++) {
          var n = added[a];
          if (n.nodeType === 1 && n.classList && n.classList.contains("card-menu")) {
            p.appeared = true;
            p.appearedAt = Date.now() - p.t0;
          }
        }
        var gone = records[i].removedNodes;
        for (var b = 0; b < gone.length; b++) {
          var m = gone[b];
          if (m.nodeType === 1 && m.classList && m.classList.contains("card-menu")) {
            p.removed = true;
            p.removedAt = Date.now() - p.t0;
          }
        }
      }
    }).observe(document.body, { childList: true, subtree: true });
    var rec = function (e) {
      var p = window.__cardMenuProbe;
      if (!p || p.events.length >= 20) { return; }
      p.events.push(e.type + "@" + (Date.now() - p.t0) + "ms");
    };
    window.addEventListener("click", rec, true);
    window.addEventListener("mousedown", rec, true);
    window.addEventListener("keydown", rec, true);
    window.addEventListener("contextmenu", rec, true);
  }
  return true;
`;

/**
 * Runs one step with a hard ceiling and records it on success.
 *
 * The ceiling is a `Promise.race`, which is the only bound available
 * for a raw driver call — `browser.execute` and `.click()` carry no
 * timeout of their own, and a driver that never answers would
 * otherwise sit there until mocha killed the whole test. The losing
 * promise is not cancelled (nothing here can cancel it); it is
 * abandoned, which is fine because a failure ends the test anyway.
 *
 * On failure the error carries the step name and the trail of steps
 * that already passed, so one run locates a stall instead of reporting
 * that time ran out somewhere.
 */
async function stage<T>(
  trail: string[],
  name: string,
  ms: number,
  work: () => Promise<T>,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const value = await Promise.race([
      work(),
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(`no answer within ${ms}ms`)), ms);
      }),
    ]);
    trail.push(name);
    console.log(`[stage] ${name}`);
    await snapStage(name);
    return value;
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    await snapStage(name, true);
    throw new Error(
      `step "${name}" failed: ${why}\n` +
        `  completed before it: ${trail.length > 0 ? trail.join(" → ") : "(none)"}`,
    );
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

function cardSelector(assetId: string) {
  return `.grid-wrapper .card[data-asset-id="${assetId}"]`;
}

/**
 * Everything the specs below ask about the DOM, in one round trip.
 *
 * # Why this is a script and not a pile of `$(...)` calls
 *
 * Measured 2026-08-01 from `ui-e2e-run6.log`: every WebDriver command
 * in the list below costs **~6 seconds** in this environment, and the
 * cost is charged before the command runs.
 *
 * `@wdio/tauri-service` installs a `beforeCommand` hook
 * (`dist/esm/index.js:3891`) that calls `ensureActiveWindowFocus`
 * (:2955), which asks the app for its window states through
 * `core.invoke('plugin:wdio|get_window_states')` (:2916). That invoke
 * is not answered by this build, so it times out after 5 s and logs
 * `Failed to get window states` — once per command, which is the 6 s
 * WARN cadence in the log. It is not a background loop; it is a tax.
 *
 * The tax applies to exactly this list (:2964):
 *
 *     ['getTitle', 'findElement', 'findElements', '$', '$$', 'elementClick']
 *
 * `execute` is **not** on it. So one `browser.execute` reading twenty
 * things is free, while `await $(sel).isExisting()` — which is a
 * `findElement` followed by a `findElements` — costs ~12 s, and
 * `waitForExist` pays the toll again on *every poll*.
 *
 * That is the whole flake. `step "strip no longer carries a trash
 * icon" failed: no answer within 15000ms` was two taxed commands
 * (~12 s + jitter) against a 15 s ceiling, which is why it passed on
 * one run and failed on the next three with nothing changed. The step
 * that never failed — `live: read menu` — is the one that was already
 * a single `execute`.
 *
 * So: **queries go through `readDom` / `existsInPage`; only clicks
 * stay as element commands**, because a click is the one thing an
 * in-page `el.click()` would quietly change the meaning of.
 */
interface DomSnapshot {
  sidebarPresent: boolean;
  cardCount: number;
  firstCardId: string;
  /** For the asset under test; `false` when the id passed in is `""`. */
  cardPresent: boolean;
  stripHasTrashIcon: boolean;
  menuOpen: boolean;
  trashView: boolean;
}

async function readDom(assetId: string): Promise<DomSnapshot> {
  return browser
    .execute(
      (cardQuery: string, toggleQuery: string) => {
        const cards = document.querySelectorAll(".grid-wrapper .card");
        const card = document.querySelector(cardQuery);
        const toggle = document.querySelector(toggleQuery);
        return {
          sidebarPresent: document.querySelector("aside.sidebar") !== null,
          cardCount: cards.length,
          firstCardId:
            cards.length > 0 ? (cards[0].getAttribute("data-asset-id") ?? "") : "",
          cardPresent: card !== null,
          stripHasTrashIcon:
            card !== null && card.querySelector('[aria-label="Move to trash"]') !== null,
          menuOpen: document.querySelector(".card-menu") !== null,
          trashView: toggle !== null && toggle.classList.contains("active"),
        };
      },
      // `cardSelector("")` is valid CSS that matches nothing, which is
      // exactly what a caller who only wants the global fields means.
      cardSelector(assetId),
      TRASH_VIEW_TOGGLE,
    )
    .catch(() => ({
      // A script that cannot run yet (very early in a cold start) is
      // "nothing is there", not a failure — the caller is polling.
      sidebarPresent: false,
      cardCount: 0,
      firstCardId: "",
      cardPresent: false,
      stripHasTrashIcon: false,
      menuOpen: false,
      trashView: false,
    }));
}

/** One untaxed existence check for a selector `readDom` does not cover. */
async function existsInPage(selector: string): Promise<boolean> {
  return browser
    .execute((query: string) => document.querySelector(query) !== null, selector)
    .catch(() => false);
}

/**
 * Polls a condition built from `execute` calls, which cost nothing, so
 * the interval can be short and the ceiling honest.
 *
 * Replaces `waitForExist` / `browser.waitUntil` wherever the condition
 * is a DOM read — both of those poll through `findElements`, and at
 * ~6 s per poll a two-poll wait already blows a 15 s budget.
 */
async function pollUntil(
  trail: string[],
  name: string,
  ms: number,
  check: () => Promise<boolean>,
  message: string,
) {
  await stage(trail, name, ms + DRIVER_MS, async () => {
    const deadline = Date.now() + ms;
    for (;;) {
      if (await check()) {
        return;
      }
      if (Date.now() >= deadline) {
        throw new Error(`${message} (polled for ${ms}ms)`);
      }
      await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
    }
  });
}

/**
 * Clicks an icon in the floating strip.
 *
 * No wait on the reveal, and no hit test before the click. Four runs
 * went into establishing that neither is reachable here — see the
 * header — and a wait on a condition the environment cannot produce is
 * a guaranteed timeout, not a safety net.
 *
 * Addressed by selector rather than by a stored handle: the grid is
 * virtualised, so a handle taken even one command earlier can point at
 * a node the row has already replaced.
 */
async function clickCardIcon(trail: string[], assetId: string, iconSelector: string) {
  const sel = `${cardSelector(assetId)} ${iconSelector}`;
  // The wait is untaxed; only the click itself is a driver command.
  await pollUntil(
    trail,
    `${iconSelector} appears`,
    PRESENT_MS,
    () => existsInPage(sel),
    `icon ${iconSelector} never appeared on asset ${assetId}`,
  );
  await stage(trail, `click ${iconSelector}`, DRIVER_MS + PRESENT_MS, () => $(sel).click());
}

/** The id of the first card the grid is showing, or `""`. Re-derived
 *  from a selector on every use rather than kept as a handle: the grid
 *  is virtualised, so a handle taken one command earlier can point at a
 *  node the row has already replaced. */
async function firstCardId(trail: string[], name: string): Promise<string> {
  return stage(trail, name, DRIVER_MS, async () => (await readDom("")).firstCardId);
}

/**
 * Waits until the grid stops changing shape.
 *
 * The grid is virtualised and keeps working after its first card
 * paints: light index rows hydrate, thumbs resolve, an `$effect` on the
 * active filter can reload. A `contextmenu` dispatched into the middle
 * of that lands on a node the render is about to replace, and a node
 * that is being swapped out is not reliably wired to Svelte's delegated
 * listener — which is hypothesis (a) on `openCardMenu`, and the reason
 * only the *first* interaction of a cold run was failing.
 *
 * "Settled" is deliberately weak: two identical reads of (card count,
 * first card id) half a second apart. It cannot prove the grid is done,
 * and it is not the only defence — the retry in `openCardMenu` covers
 * a re-render that starts later. It removes the common case cheaply.
 */
async function waitForGridSettled(trail: string[]) {
  // Ceiling deliberately above the polling deadline below: this check
  // is advisory, so the only thing that should be able to fail it is a
  // driver that stopped answering — not the loop reaching its own end.
  await stage(trail, "grid settles", GRID_MS + DRIVER_MS, async () => {
    let previous = "";
    const deadline = Date.now() + GRID_MS;
    while (Date.now() < deadline) {
      const dom = await readDom("");
      const shape = `${dom.cardCount}:${dom.firstCardId}`;
      if (dom.cardCount > 0 && shape === previous) {
        return;
      }
      previous = shape;
      await new Promise((resolve) => setTimeout(resolve, SETTLE_GAP_MS));
    }
    // Not fatal: an unsettled grid is what the retry exists for, and a
    // hard failure here would turn a slow machine into a red run.
    console.warn(
      `[card-trash] grid never settled within ${GRID_MS}ms (last shape ${previous}) — ` +
        "continuing; openCardMenu will retry if a dispatch misses",
    );
  });
}

/** One entry of the open context menu, in DOM order. */
interface MenuEntry {
  /** `item` = a clickable entry, `sep` = the rule above the removal
   *  tier, `head` = the "N selected" count line. */
  kind: string;
  text: string;
  danger: boolean;
}

/**
 * # Nothing in an in-page callback may carry a name
 *
 * No named function, no class, and no arrow assigned to a `const` —
 * only anonymous arrows in argument position, and no nested function at
 * all if it can be avoided. The reason is a seam between two layers that
 * each behave correctly on their own:
 *
 *   * The specs are compiled with esbuild name preservation, which
 *     rewrites anything nameable as `__name(fn, "fn")` and defines
 *     `__name` once at module scope.
 *   * `@wdio/tauri-service` replaces `browser.execute` and, on this
 *     driver provider, sends the callback to `executeAsync` as a
 *     *string* (`script.toString()`).
 *
 * WebdriverIO prepends its `__name` polyfill only for function-typed
 * scripts, so the string path arrives in the page with the helper
 * missing and module scope gone. The result is
 * `ReferenceError: Can't find variable: __name` — reported as a driver
 * error with no line, which is why the first two versions of this spec
 * failed opaquely. `before` installs a shim for the same reason.
 *
 * Two further properties hold for every callback below, each one a
 * failure the earlier attempts hit:
 *   * **Selectors in, data out.** Nothing crosses the boundary as an
 *     element reference, so a re-render between two calls cannot turn a
 *     measurement into a driver-side exception with no detail.
 *   * **Every fault path returns.** A script that throws inside this
 *     driver surfaces as "A JavaScript exception occurred" with the
 *     cause discarded, so a cause is carried out as data instead.
 *
 * ---
 *
 * What the probe below records, and why a dispatch needed one.
 *
 * On 2026-08-01 the *first* interaction of a cold run began failing at
 * "menu opens" while the identical call in the next test, same session,
 * opened the menu and drove a full round trip. Raising the ceiling from
 * 4 s to 15 s moved the failure but did not remove it: fifteen seconds
 * of nothing is not slowness, it is a dispatch that produced no menu.
 *
 * Three explanations fit that, and reading `.card-menu` alone cannot
 * tell them apart:
 *
 *   (a) the handler never ran — the node was found by
 *       `querySelector` but was not (or was no longer) wired to
 *       Svelte's delegated `contextmenu` listener, e.g. mid-re-render
 *       while the virtualised grid settles after first paint;
 *   (b) the menu opened and something closed it again before the
 *       driver's next poll — App closes it on any click reaching
 *       `svelte:window`, and the tauri-service is polling the window
 *       every 6 s with its own machinery;
 *   (c) the handler ran and the menu never rendered.
 *
 * The probe separates them without touching app code, using a signal
 * the app already produces: `openCardMenu` retargets the grid selection
 * before it opens anything, so `.card.selected` says whether the
 * handler ran at all. Together with a MutationObserver that catches a
 * `.card-menu` which appears and disappears between polls, and a
 * capture-phase log of the clicks / keys that arrived in the window:
 *
 *   appeared=false selectedAfter=false → (a) the handler never ran
 *   appeared=false selectedAfter=true  → (c) ran, never rendered
 *   appeared=true  removed=true        → (b), and `events` names what
 *                                        arrived in the gap
 *
 * `sameNode` / `stillConnected` test the re-render half of (a)
 * directly: they say whether the node that was dispatched on is still
 * the node the selector finds, and whether it is still in the document.
 *
 * One caveat on reading the output: the `.selected` discriminator is
 * clean only on the first attempt. A later attempt inherits the
 * selection an earlier one left behind, so its line reads
 * `selected true → true` and cannot separate (a) from (c) on its own.
 * That is why every attempt is printed rather than just the last.
 */
async function openCardMenu(trail: string[], assetId: string, label: string) {
  const diagnostics: string[] = [];
  let attempts = 0;

  await stage(trail, `${label}: menu opens`, PRESENT_MS + DRIVER_MS, async () => {
    const deadline = Date.now() + PRESENT_MS;

    // Retry, because a human whose right-click produced nothing presses
    // again — a spec that gave up on the first miss would be asserting
    // something stricter than the app promises. What it must not do is
    // absorb the miss silently, so every failed attempt is kept and
    // reported: on an eventual pass through `console.warn` and an extra
    // breadcrumb, on a failure as the whole list. A regression to
    // "never opens on the first try" therefore still shows up in the
    // run output instead of hiding behind a green tick.
    while (Date.now() < deadline) {
      attempts += 1;

      const armed: MenuProbe = await browser.execute((cardQuery: string) => {
        const probe = {
          t0: Date.now(),
          cardFound: false,
          connectedAtDispatch: false,
          selectedBefore: false,
          dispatched: false,
          appeared: false,
          appearedAt: -1,
          removed: false,
          removedAt: -1,
          events: [] as string[],
        };
        // The observer and the capture-phase listeners are installed
        // once, in `before`; they read whichever probe is current, so
        // arming is just replacing the object they write into. Doing it
        // in the same call as the dispatch leaves no gap where a menu
        // could open unobserved.
        (window as unknown as { __cardMenuProbe: unknown }).__cardMenuProbe = probe;
        (window as unknown as { __cardMenuProbeNode: unknown }).__cardMenuProbeNode = null;

        const card = document.querySelector(cardQuery);
        if (!card) {
          return probe;
        }
        probe.cardFound = true;
        probe.connectedAtDispatch = card.isConnected;
        probe.selectedBefore = card.classList.contains("selected");
        (window as unknown as { __cardMenuProbeNode: unknown }).__cardMenuProbeNode = card;

        card.dispatchEvent(
          new MouseEvent("contextmenu", {
            bubbles: true,
            cancelable: true,
            clientX: 120,
            clientY: 120,
          }),
        );
        probe.dispatched = true;
        return probe;
      }, cardSelector(assetId));

      // Short per-attempt window: the menu is a synchronous render away
      // from a handler that ran, so waiting longer here only delays the
      // next attempt and the diagnosis. Polled with `execute` rather
      // than `waitForExist`, which would pay the ~6 s window-focus tax
      // on every poll (see `readDom`) and make this window meaningless.
      let opened = false;
      const attemptDeadline = Date.now() + MENU_ATTEMPT_MS;
      for (;;) {
        opened = await existsInPage(".card-menu");
        if (opened || Date.now() >= attemptDeadline) {
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
      }

      if (opened) {
        if (attempts > 1) {
          const note =
            `[card-trash] ${label}: context menu needed ${attempts} dispatches ` +
            `for asset ${assetId}. Earlier attempts:\n  ${diagnostics.join("\n  ")}`;
          console.warn(note);
          trail.push(`${label}: menu opened on attempt ${attempts}`);
        }
        return;
      }

      diagnostics.push(`attempt ${attempts}: ${describeProbe(armed, await readProbe(assetId))}`);
    }

    throw new Error(
      `context menu never opened for asset ${assetId} after ${attempts} dispatches\n  ` +
        diagnostics.join("\n  "),
    );
  });
}

/** What the in-page probe collects between arming and reading. */
interface MenuProbe {
  t0: number;
  cardFound: boolean;
  connectedAtDispatch: boolean;
  selectedBefore: boolean;
  dispatched: boolean;
  appeared: boolean;
  appearedAt: number;
  removed: boolean;
  removedAt: number;
  events: string[];
}

/** The probe's state now, plus what the DOM says about the card. */
interface ProbeRead {
  probe: MenuProbe | null;
  menuNow: boolean;
  selectedAfter: boolean;
  sameNode: boolean;
  stillConnected: boolean;
  cardCount: number;
}

async function readProbe(assetId: string): Promise<ProbeRead> {
  return browser.execute((cardQuery: string) => {
    const held = window as unknown as {
      __cardMenuProbe: MenuProbe | null;
      __cardMenuProbeNode: Element | null;
    };
    const p = held.__cardMenuProbe;
    const card = document.querySelector(cardQuery);
    return {
      probe: p,
      menuNow: Boolean(document.querySelector(".card-menu")),
      selectedAfter: card !== null && card.classList.contains("selected"),
      sameNode: card !== null && card === held.__cardMenuProbeNode,
      stillConnected:
        held.__cardMenuProbeNode !== null && held.__cardMenuProbeNode.isConnected,
      cardCount: document.querySelectorAll(".grid-wrapper .card").length,
    };
  }, cardSelector(assetId));
}

/** One line per attempt, ordered so the discriminating fields come
 *  first — see the three-way split on `openCardMenu`. */
function describeProbe(armed: MenuProbe, read: ProbeRead): string {
  const live = read.probe;
  const verdict = !armed.cardFound
    ? "no card matched the selector"
    : !live
      ? "probe went missing (page reloaded?)"
      : live.appeared && live.removed
        ? "(b) menu opened then closed"
        : live.appeared
          ? "menu opened but the driver did not see it"
          : read.selectedAfter
            ? "(c) handler ran, menu never rendered"
            : "(a) handler never ran";
  return [
    verdict,
    `dispatched=${armed.dispatched}`,
    `connectedAtDispatch=${armed.connectedAtDispatch}`,
    `appeared=${live ? live.appeared : "?"}@${live ? live.appearedAt : "?"}ms`,
    `removed=${live ? live.removed : "?"}@${live ? live.removedAt : "?"}ms`,
    `menuNow=${read.menuNow}`,
    `selected ${armed.selectedBefore} → ${read.selectedAfter}`,
    `sameNode=${read.sameNode}`,
    `stillConnected=${read.stillConnected}`,
    `cards=${read.cardCount}`,
    `events=[${live ? live.events.join(", ") : ""}]`,
  ].join(" ");
}

/** Reads the open menu in one pass. Empty array = no menu. */
async function readCardMenu(trail: string[], label: string): Promise<MenuEntry[]> {
  return stage(trail, `${label}: read menu`, DRIVER_MS, () =>
    browser.execute(() => {
      const menu = document.querySelector(".card-menu");
      if (!menu) {
        return [] as MenuEntry[];
      }
      // Direct children only: the fold-out submenus (`.card-menu-sub`)
      // are their own lists, and flattening them would put a Modality
      // row between the reflex actions and the removal tier — which is
      // exactly the ordering claim being made here.
      return Array.from(menu.children).map((el) => ({
        kind: el.classList.contains("card-menu-sep")
          ? "sep"
          : el.classList.contains("card-menu-head")
            ? "head"
            : el.classList.contains("card-menu-item")
              ? "item"
              : "other",
        text: (el.textContent ?? "").replace(/\s+/g, " ").trim(),
        danger: el.classList.contains("card-menu-item-danger"),
      }));
    }),
  );
}

/**
 * Dismisses the menu without acting on it.
 *
 * Escape first, dispatched at the window: that is the app's own
 * documented close (the interaction-mode stack pops `cardMenu`), and it
 * does not depend on a synthetic click bubbling to a `svelte:window`
 * handler. The outside click is kept as a second path — the two fail
 * for different reasons, and the fixture-restoring code below must not
 * be blocked by a menu that would not close.
 */
async function closeCardMenu(trail: string[], label: string) {
  await stage(trail, `${label}: dismiss menu`, DRIVER_MS + PRESENT_MS, async () => {
    await browser.execute(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    const deadline = Date.now() + MENU_ATTEMPT_MS;
    let triedClick = false;
    for (;;) {
      if (!(await existsInPage(".card-menu"))) {
        return;
      }
      if (Date.now() >= deadline) {
        if (triedClick) {
          throw new Error("context menu closed by neither Escape nor an outside click");
        }
        // Second path: an outside click, which App turns into a close
        // via its window listener. The two fail for different reasons,
        // and the fixture-restoring code below must not be blocked by a
        // menu that would not close.
        triedClick = true;
        await browser.execute(() => {
          document.body.click();
        });
      }
      await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
    }
  });
}

/** The trash toolbar's three buttons, flattened — one `execute`, no
 *  nested function to name (see `openCardMenu` on why). Absent
 *  buttons read as `present: false` rather than throwing, because
 *  "the toolbar is not there" is one of the things asked about. */
interface ToolbarSnapshot {
  present: boolean;
  restorePresent: boolean;
  restoreDisabled: boolean;
  restoreText: string;
  purgePresent: boolean;
  purgeDisabled: boolean;
  purgeText: string;
  emptyPresent: boolean;
  emptyDisabled: boolean;
  emptyText: string;
}

const ABSENT_TOOLBAR: ToolbarSnapshot = {
  present: false,
  restorePresent: false,
  restoreDisabled: false,
  restoreText: "",
  purgePresent: false,
  purgeDisabled: false,
  purgeText: "",
  emptyPresent: false,
  emptyDisabled: false,
  emptyText: "",
};

async function readTrashToolbar(): Promise<ToolbarSnapshot> {
  return browser
    .execute(() => {
      const bar = document.querySelector(".trash-toolbar");
      const restore = document.querySelector(
        ".trash-toolbar-restore",
      ) as HTMLButtonElement | null;
      const purge = document.querySelector(
        ".trash-toolbar-purge",
      ) as HTMLButtonElement | null;
      const empty = document.querySelector(
        ".trash-toolbar-empty",
      ) as HTMLButtonElement | null;
      return {
        present: bar !== null,
        restorePresent: restore !== null,
        restoreDisabled: restore !== null && restore.disabled,
        restoreText: (restore?.textContent ?? "").replace(/\s+/g, " ").trim(),
        purgePresent: purge !== null,
        purgeDisabled: purge !== null && purge.disabled,
        purgeText: (purge?.textContent ?? "").replace(/\s+/g, " ").trim(),
        emptyPresent: empty !== null,
        emptyDisabled: empty !== null && empty.disabled,
        emptyText: (empty?.textContent ?? "").replace(/\s+/g, " ").trim(),
      };
    })
    .catch(() => ABSENT_TOOLBAR);
}

/** The confirm modal, read the same way. `confirmIsDanger` is the
 *  destructive tone: the class is only on the confirm button when the
 *  caller asked for it. */
interface ConfirmSnapshot {
  open: boolean;
  title: string;
  body: string;
  confirmLabel: string;
  confirmIsDanger: boolean;
}

async function readConfirm(): Promise<ConfirmSnapshot> {
  return browser
    .execute(() => {
      const panel = document.querySelector(".confirm-panel");
      const title = document.querySelector(".confirm-title");
      const body = document.querySelector(".confirm-body");
      const danger = document.querySelector(".confirm-btn.danger");
      return {
        open: panel !== null,
        title: (title?.textContent ?? "").replace(/\s+/g, " ").trim(),
        body: (body?.textContent ?? "").replace(/\s+/g, " ").trim(),
        confirmLabel: (danger?.textContent ?? "").replace(/\s+/g, " ").trim(),
        confirmIsDanger: danger !== null,
      };
    })
    .catch(() => ({
      open: false,
      title: "",
      body: "",
      confirmLabel: "",
      confirmIsDanger: false,
    }));
}

/**
 * Dismisses a stray confirm through the page realm — the `finally`
 * cleanup tool, not the assertion path. The test itself clicks Cancel
 * with the element API, because that click is a gesture under test
 * (it exercises the backdrop/panel z-order a mispositioned overlay
 * would break). Cleanup takes the in-page route instead: it has to
 * work even when the app can no longer respond to anything — the
 * 2026-08-01 scheduler-death bug (see interaction/mode.svelte.ts
 * `push`) left a modal on screen that took down every test and spec
 * after it, and a cleanup that costs a taxed driver command per
 * attempt is also just slower for no added claim.
 */
async function cancelConfirm(): Promise<void> {
  await browser
    .execute(() => {
      const el = document.querySelector(".confirm-btn.ghost");
      if (el instanceof HTMLElement) el.click();
    })
    .catch(() => undefined);
}

/** The undo toast. */
interface ToastSnapshot {
  present: boolean;
  message: string;
  actionLabel: string;
}

async function readUndoToast(): Promise<ToastSnapshot> {
  return browser
    .execute(() => {
      const toast = document.querySelector(".undo-toast");
      const action = document.querySelector(".undo-toast-action");
      return {
        present: toast !== null,
        message: (
          document.querySelector(".undo-toast-message")?.textContent ?? ""
        )
          .replace(/\s+/g, " ")
          .trim(),
        actionLabel: (action?.textContent ?? "").replace(/\s+/g, " ").trim(),
      };
    })
    .catch(() => ({ present: false, message: "", actionLabel: "" }));
}

/**
 * Trashes the first live card through the menu entry and waits for it
 * to leave — the fixture step both new specs open with, since a trash
 * toolbar with an empty trash can assert only half of itself.
 *
 * Returns the id, which the caller owes back to the live side.
 */
async function trashFirstLiveCard(trail: string[]): Promise<string> {
  const assetId = await firstCardId(trail, "read first card id");
  if (assetId === "") {
    throw new Error("no card on the live side to trash");
  }
  await openCardMenu(trail, assetId, "live");
  await stage(trail, "click Move to Trash", DRIVER_MS + PRESENT_MS, () =>
    $(MENU_DANGER_ITEM).click(),
  );
  await waitGone(
    trail,
    assetId,
    "card leaves the live grid",
    "card stayed in the live grid after Move to Trash",
  );
  return assetId;
}

async function waitGone(trail: string[], assetId: string, name: string, message: string) {
  await pollUntil(
    trail,
    name,
    GRID_MS,
    async () => !(await readDom(assetId)).cardPresent,
    message,
  );
}

/**
 * Whether the grid is showing the trash.
 *
 * Read off the toggle's own `active` class rather than its label or
 * title: the title is identical in both states, and the label flips
 * between two glyphs that are easy to mistype. Comes back with the rest
 * of `readDom`, so asking costs nothing.
 */
async function inTrashView(): Promise<boolean> {
  return (await readDom("")).trashView;
}

/** Idempotent — clicking a toggle that is already where you want it is
 *  how a suite ends up on the wrong side of the trash. */
async function setTrashView(trail: string[], on: boolean) {
  const name = `trash view → ${on ? "on" : "off"}`;
  await pollUntil(
    trail,
    `${name}: toggle present`,
    PRESENT_MS,
    () => existsInPage(TRASH_VIEW_TOGGLE),
    "sidebar trash toggle never appeared",
  );
  if ((await inTrashView()) === on) {
    trail.push(`${name} (already there)`);
    return;
  }
  await stage(trail, `${name}: click toggle`, DRIVER_MS + PRESENT_MS, () =>
    $(TRASH_VIEW_TOGGLE).click(),
  );
  await pollUntil(
    trail,
    name,
    GRID_MS,
    async () => (await inTrashView()) === on,
    `trash view never turned ${on ? "on" : "off"}`,
  );
}

/**
 * Puts one asset back on the live side, from wherever it currently is.
 *
 * Used as cleanup rather than as an assertion: it makes no claim that
 * restore works — the spec asserts that separately — it only refuses to
 * leave the fixture spent if something above it failed midway.
 *
 * Its own trail, so a cleanup failure reports how far cleanup got
 * rather than borrowing the test's breadcrumbs.
 */
async function ensureLive(assetId: string) {
  const trail: string[] = [];
  // A menu left open would swallow the first click below.
  const initial = await stage(trail, "cleanup: read dom", DRIVER_MS, () => readDom(assetId));
  if (initial.menuOpen) {
    await closeCardMenu(trail, "cleanup").catch(() => undefined);
  }
  await setTrashView(trail, false);
  if ((await stage(trail, "cleanup: on live side?", DRIVER_MS, () => readDom(assetId)))
    .cardPresent) {
    return;
  }
  await setTrashView(trail, true);
  if ((await stage(trail, "cleanup: on trash side?", DRIVER_MS, () => readDom(assetId)))
    .cardPresent) {
    await clickCardIcon(trail, assetId, RESTORE_ICON);
    await waitGone(
      trail,
      assetId,
      "cleanup: restore lands",
      `cleanup restore did not take for asset ${assetId}`,
    );
  }
  await setTrashView(trail, false);
}

/**
 * Brings the profile back to a state the suite can run against.
 *
 * The suite runs on a profile that lives on disk, and this spec is the
 * only one that moves an asset out of the live set. A failure between
 * the trash click and the restore therefore does not just fail a run —
 * it consumes the fixture, and every later run dies in `before` with
 * "no cards" instead of naming the original defect. That happened
 * twice: once before this existed, and again on 2026-08-01 when mocha
 * cut off the `finally` that would have restored.
 *
 * Two deliberate limits. It only runs when the live side is *empty*, so
 * it cannot quietly undo a deliberately trashed asset in a profile that
 * still has something to test with. And it restores exactly one card —
 * the minimum that makes the suite runnable — rather than emptying the
 * trash, which would be a much larger thing to do on the strength of a
 * guess about why the live side is bare.
 */
async function healFixture(trail: string[]) {
  await setTrashView(trail, true);

  // Short: the view switch is already confirmed by `setTrashView`, so
  // this is only waiting on the trash grid to paint. If nothing is
  // there by now, there is nothing to recover and the caller should say
  // so rather than spend the remaining budget hoping.
  const anyTrashed = await pollUntil(
    trail,
    "heal: trash grid paints",
    GRID_MS,
    async () => (await readDom("")).cardCount > 0,
    "nothing in the trash to recover",
  ).then(
    () => true,
    () => false,
  );

  if (anyTrashed) {
    const id = await firstCardId(trail, "heal: read trashed card id");
    if (id) {
      await clickCardIcon(trail, id, RESTORE_ICON);
      await waitGone(
        trail,
        id,
        "heal: restore lands",
        `recovery restore did not take for asset ${id}`,
      );
    }
  }

  await setTrashView(trail, false);
}

describe("card trash action", () => {
  before(async () => {
    const trail: string[] = [];

    // The `__name` shim goes in before anything else, unchanged and on
    // its own: it has to be in place before the first function-typed
    // script runs, and the probe install below cannot be that first
    // thing because its MutationObserver needs `document.body` to be
    // worth watching. Idempotent, and `INSTALL_PROBE` re-asserts it.
    await stage(trail, "install __name shim", DRIVER_MS, () =>
      browser.execute(
        "window.__name = window.__name || function (target) { return target; };",
      ),
    );

    // The window and the SQLite open are the slow half of a cold start,
    // and only the first spec in a run pays for them — so this one wait
    // is long on purpose while everything after it is not. Polled with
    // `execute`, which also means a script that cannot run yet reads as
    // "not ready" instead of costing a taxed `findElement` per poll.
    await pollUntil(
      trail,
      "app window paints",
      60_000,
      async () => (await readDom("")).sidebarPresent,
      "the app never painted its sidebar",
    );

    await stage(trail, "install page probe", DRIVER_MS, () =>
      browser.execute(INSTALL_PROBE),
    );

    // The grid paints after the first page load resolves. Every spec
    // below needs a card, so a profile with none is a setup failure and
    // should read as one rather than as a silent pass — but "none on
    // the live side" is also what a previous run's mid-flight failure
    // leaves behind, and that is recoverable. Try, heal, try again;
    // only the second failure is real.
    const live = await pollUntil(
      trail,
      "live grid paints",
      30_000,
      async () => (await readDom("")).cardCount > 0,
      "no cards on the live side",
    ).then(
      () => true,
      () => false,
    );

    if (!live) {
      await healFixture(trail);
      await pollUntil(
        trail,
        "live grid paints after recovery",
        GRID_MS,
        async () => (await readDom("")).cardCount > 0,
        "no cards on the live side, and nothing in the trash to restore — " +
          "the e2e profile has no assets at all",
      );
    }

    await waitForGridSettled(trail);
  });

  it("offers Move to Trash as the last entry of a live card's context menu", async () => {
    const trail: string[] = [];
    const assetId = await firstCardId(trail, "read first card id");
    expect(assetId).not.toBe("");

    try {
      await openCardMenu(trail, assetId, "live");
      const entries = await readCardMenu(trail, "live");
      expect(entries.length).toBeGreaterThan(1);

      const last = entries[entries.length - 1];
      const beforeLast = entries[entries.length - 2];

      // All four claims in one object so a failure prints the whole
      // shape rather than stopping at the first — the placement is the
      // point of this spec, and "which part of it moved" is the thing
      // worth reading in the diff.
      expect({
        lastIsItem: last.kind,
        lastSaysTrash: last.text.includes("Move to Trash"),
        lastIsDestructive: last.danger,
        separatedFromTheRest: beforeLast.kind,
      }).toEqual({
        lastIsItem: "item",
        lastSaysTrash: true,
        lastIsDestructive: true,
        separatedFromTheRest: "sep",
      });

      // The other half of the regrammar, and the reason the entry moved
      // at all: it is no longer in the hover strip. Absence from the
      // card's own DOM needs no reveal to observe.
      const stripStillHasIt = await stage(
        trail,
        "strip no longer carries a trash icon",
        DRIVER_MS,
        async () => (await readDom(assetId)).stripHasTrashIcon,
      );
      expect(stripStillHasIt).toBe(false);
    } finally {
      await closeCardMenu([], "live cleanup").catch(() => undefined);
    }
  });

  it("takes the card out of the live grid when the menu entry is picked", async () => {
    const trail: string[] = [];
    const assetId = await firstCardId(trail, "read first card id");
    expect(assetId).not.toBe("");

    try {
      await openCardMenu(trail, assetId, "live");
      // A real element click, not an in-page `el.click()`: `elementClick`
      // is one of the taxed commands (see `readDom`), but this is the
      // gesture under test and routing it through the DOM would quietly
      // change what the assertion covers. The tax is affordable here
      // because it is now one of only three taxed commands in the file.
      await stage(trail, "click Move to Trash", DRIVER_MS + PRESENT_MS, () =>
        $(MENU_DANGER_ITEM).click(),
      );
      await waitGone(
        trail,
        assetId,
        "card leaves the live grid",
        "card stayed in the live grid after Move to Trash",
      );

      // --- undo, so the fixture survives the run -------------------
      await setTrashView(trail, true);
      await pollUntil(
        trail,
        "card arrives on the trash side",
        GRID_MS,
        async () => (await readDom(assetId)).cardPresent,
        "trashed card never appeared on the trash side",
      );

      // The withholding half: the menu that offered "Move to Trash" a
      // moment ago must not offer it here — a trashed card is not
      // re-trashable, the same shape the sidebar Trash row uses when it
      // drops its `data-drop-kind` on this side. What it offers instead
      // is the pair that does apply, with the irreversible one last.
      await openCardMenu(trail, assetId, "trash");
      const trashEntries = await readCardMenu(trail, "trash");
      const trashLast = trashEntries[trashEntries.length - 1];
      expect({
        offersMoveToTrash: trashEntries.some((e) => e.text.includes("Move to Trash")),
        offersRestore: trashEntries.some((e) => e.text.includes("Restore")),
        lastSaysDeleteForever: trashLast.text.includes("Delete Forever"),
        lastIsDestructive: trashLast.danger,
      }).toEqual({
        offersMoveToTrash: false,
        offersRestore: true,
        lastSaysDeleteForever: true,
        lastIsDestructive: true,
      });
      // Dismissed rather than used: "Delete Forever" is the one action
      // in the app with no way back, and a suite that runs against a
      // profile on disk must not be the thing that exercises it.
      await closeCardMenu(trail, "trash");

      await clickCardIcon(trail, assetId, RESTORE_ICON);
      await waitGone(
        trail,
        assetId,
        "card leaves the trash side",
        "card stayed on the trash side after restore",
      );

      await setTrashView(trail, false);
      await pollUntil(
        trail,
        "live grid repaints",
        GRID_MS,
        async () => (await readDom("")).cardCount > 0,
        "live grid never came back after leaving the trash view",
      );
    } finally {
      // Three things have to be true for the next spec and the next
      // run, whichever line above threw: no menu is left open, the
      // asset is back on the live side, and the grid is showing that
      // side. The last is not tidiness — the sidebar Trash row
      // withholds `data-drop-kind` while the trash is on screen, so a
      // spec abandoned in the trash view takes `drop-targets.spec.ts`
      // down with it in the same session.
      //
      // This only gets to run if the body left budget on the clock,
      // which is what the constants at the top of the file are for.
      // Cleanup failures are swallowed on purpose: this must never
      // replace the error that got us here with one about cleanup.
      await ensureLive(assetId).catch(() => undefined);
    }
  });

  it("puts a toolbar on the trash side whose two per-selection actions wait for one", async () => {
    const trail: string[] = [];
    let assetId = "";

    try {
      assetId = await trashFirstLiveCard(trail);
      await setTrashView(trail, true);
      await pollUntil(
        trail,
        "card arrives on the trash side",
        GRID_MS,
        async () => (await readDom(assetId)).cardPresent,
        "trashed card never appeared on the trash side",
      );

      // Nothing selected. Restore / Delete Forever have no set to act
      // on and say so; Empty Trash does not read a selection at all,
      // and the trash is demonstrably non-empty (the card above), so
      // its enabled state in this same read is what keeps the two
      // disabled ones from being a claim about a toolbar that is
      // simply inert.
      const idle = await stage(trail, "read toolbar (no selection)", DRIVER_MS, () =>
        readTrashToolbar(),
      );
      expect({
        toolbar: idle.present,
        restore: idle.restorePresent,
        purge: idle.purgePresent,
        empty: idle.emptyPresent,
        restoreDisabled: idle.restoreDisabled,
        purgeDisabled: idle.purgeDisabled,
        emptyDisabled: idle.emptyDisabled,
      }).toEqual({
        toolbar: true,
        restore: true,
        purge: true,
        empty: true,
        restoreDisabled: true,
        purgeDisabled: true,
        emptyDisabled: false,
      });

      // With a selection they light up and carry its size. The
      // selection comes from the context menu's own retarget (App
      // exclusive-selects the card it opens on), which is also the
      // only way to select a card here that does not open the detail
      // pane. Read while the menu is open, deliberately: the read is
      // about the toolbar, and waiting for the close would make the
      // assertion depend on the close path too.
      await openCardMenu(trail, assetId, "trash");
      const chosen = await stage(trail, "read toolbar (one selected)", DRIVER_MS, () =>
        readTrashToolbar(),
      );
      expect({
        restoreDisabled: chosen.restoreDisabled,
        purgeDisabled: chosen.purgeDisabled,
        restoreCountsIt: chosen.restoreText.includes("(1)"),
        purgeCountsIt: chosen.purgeText.includes("(1)"),
      }).toEqual({
        restoreDisabled: false,
        purgeDisabled: false,
        restoreCountsIt: true,
        purgeCountsIt: true,
      });
      await closeCardMenu(trail, "trash");

      // Empty Trash asks before it acts — and this suite goes exactly
      // that far. The confirm is clicked *Cancel*: the profile lives
      // on disk, and a run that answered the other way would delete
      // every trashed asset on it with nothing to restore from. The
      // danger button is never addressed by any selector below.
      await stage(trail, "click Empty Trash", DRIVER_MS + PRESENT_MS, () =>
        $(".trash-toolbar-empty").click(),
      );
      await pollUntil(
        trail,
        "confirm opens",
        PRESENT_MS,
        async () => (await readConfirm()).open,
        "Empty Trash did not ask for confirmation",
      );
      const ask = await stage(trail, "read confirm", DRIVER_MS, () => readConfirm());
      expect({
        title: ask.title,
        label: ask.confirmLabel,
        destructiveTone: ask.confirmIsDanger,
        saysItIsFinal: ask.body.includes("cannot be undone"),
        // The filter caveat is the one thing about this command a user
        // cannot infer from the button: it ignores whatever the grid
        // is filtered to.
        saysItIgnoresTheFilter: ask.body.includes("filter"),
      }).toEqual({
        title: "Empty Trash?",
        label: "Empty Trash",
        destructiveTone: true,
        saysItIsFinal: true,
        saysItIgnoresTheFilter: true,
      });

      await stage(trail, "click Cancel", DRIVER_MS + PRESENT_MS, () =>
        $(".confirm-btn.ghost").click(),
      );
      let closedByClick = false;
      for (let i = 0; i < 24 && !closedByClick; i++) {
        closedByClick = !(await readConfirm()).open;
        if (!closedByClick) await browser.pause(POLL_GAP_MS);
      }
      if (!closedByClick) {
        // Forensics for the 2026-08-01 stuck-Cancel investigation.
        // Three links could break independently: event delivery (the
        // direct/capture listeners below), the delegated click handler,
        // or the render flush. Escape exercises the same store through
        // a non-delegated `svelte:window` keydown listener, so a modal
        // that Escape *can* close narrows the break to click
        // delegation; one Escape cannot close means the store or the
        // flush is dead.
        const forensics = await browser
          .execute(() => {
            const out = {
              panels: document.querySelectorAll(".confirm-panel").length,
              btnFound: false,
              directListenerFired: false,
              docCaptureSawClick: false,
              activeElement: document.activeElement
                ? `${document.activeElement.tagName}.${document.activeElement.className}`
                : "(none)",
            };
            const el = document.querySelector(".confirm-btn.ghost");
            if (el instanceof HTMLElement) {
              out.btnFound = true;
              el.addEventListener(
                "click",
                () => {
                  out.directListenerFired = true;
                },
                { once: true, capture: true },
              );
              document.addEventListener(
                "click",
                () => {
                  out.docCaptureSawClick = true;
                },
                { once: true, capture: true },
              );
              el.click();
            }
            return out;
          })
          .catch((err) => ({ probeFailed: String(err) }));
        await browser
          .execute(() => {
            window.dispatchEvent(
              new KeyboardEvent("keydown", {
                key: "Escape",
                bubbles: true,
                cancelable: true,
              }),
            );
          })
          .catch(() => undefined);
        let closedByEscape = false;
        for (let i = 0; i < 16 && !closedByEscape; i++) {
          closedByEscape = !(await readConfirm()).open;
          if (!closedByEscape) await browser.pause(POLL_GAP_MS);
        }
        await snapStage("confirm closes", true);
        throw new Error(
          `the confirm modal stayed open after Cancel; ` +
            `forensics=${JSON.stringify(forensics)} closedByEscape=${closedByEscape}`,
        );
      }
      trail.push("confirm closes");
      // Cancelled means nothing happened — including to the card that
      // was sitting in the trash while the question was on screen.
      const survived = await stage(trail, "card survived the cancel", DRIVER_MS, () =>
        readDom(assetId),
      );
      expect(survived.cardPresent).toBe(true);
    } finally {
      // A confirm abandoned open blocks every driver click after it
      // (see `cancelConfirm`) — dismiss it before anything else, or
      // the fixture restore below cannot press the trash toggle and
      // the stuck modal takes the next spec down too (observed
      // 2026-08-01: one stuck Cancel failed the two tests after it).
      await cancelConfirm();
      // Same three obligations as the spec above, and the same reason
      // the last one matters: a run abandoned in the trash view takes
      // `drop-targets.spec.ts` down with it.
      if (assetId !== "") {
        await ensureLive(assetId).catch(() => undefined);
      } else {
        await setTrashView([], false).catch(() => undefined);
      }
    }
  });

  it("offers an Undo on the trash gesture, and taking it puts the card back", async () => {
    const trail: string[] = [];
    let assetId = "";

    try {
      assetId = await trashFirstLiveCard(trail);

      await pollUntil(
        trail,
        "undo toast appears",
        PRESENT_MS,
        async () => (await readUndoToast()).present,
        "trashing a card offered no Undo",
      );
      const toast = await stage(trail, "read undo toast", DRIVER_MS, () =>
        readUndoToast(),
      );
      expect({
        action: toast.actionLabel,
        saysWhatHappened: toast.message.includes("Trash"),
      }).toEqual({
        action: "Undo",
        saysWhatHappened: true,
      });

      // The one click in this file that goes through the DOM rather
      // than the element API, and the reason is arithmetic: a taxed
      // `$(sel).click()` is a `findElement` plus an `elementClick`,
      // ~12 s in this environment (see `readDom`), while the toast is
      // gone after 8 by design. An element click could therefore only
      // ever assert the timer. The handler under test is the same one
      // either way; what this cannot vouch for is hit-testing, which
      // is already true of every menu entry here.
      await stage(trail, "click Undo", DRIVER_MS, () =>
        browser.execute(() => {
          const btn = document.querySelector(".undo-toast-action");
          if (btn instanceof HTMLElement) btn.click();
          return btn !== null;
        }),
      );

      await pollUntil(
        trail,
        "card returns to the live grid",
        GRID_MS,
        async () => (await readDom(assetId)).cardPresent,
        "Undo did not bring the card back to the live grid",
      );
      // Taking the offer withdraws it — a toast still offering Undo
      // after the undo ran is a second restore waiting to be clicked.
      await pollUntil(
        trail,
        "undo toast leaves",
        PRESENT_MS,
        async () => !(await readUndoToast()).present,
        "the Undo toast stayed on screen after it was taken",
      );
    } finally {
      if (assetId !== "") {
        await ensureLive(assetId).catch(() => undefined);
      }
    }
  });
});
