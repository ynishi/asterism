// A pursuit from open to close, through the real backend, in the
// webview.
//
// `lib/stores/forge.test.ts` owns what the work half decides — that a
// pursuit cut from the head asks nothing about what it collides with,
// that a rule declining to resolve is an outcome and not a failure,
// that closing work re-reads the line and drops its chain. All of it
// runs against mocked `api` and `mutate`, which is the same hole
// `forge-line.spec.ts` was written for: a unit test asserts that the
// store called `"push_forge_round"` with `{pursuitId, command}`, a
// shape its own author wrote down twice. A webview spec is the only
// place a command's name, its arguments and the shape of its answer
// are checked against the app that has to answer them.
//
// # Why one `it` again
//
// The order is the model's. A round needs a pursuit, a pursuit needs a
// line, and what a satisfied close puts on the line is only visible
// because the round before it asked for something. None of these is a
// provocation that can be set up some other way without seeding the
// thing the spec is meant to prove.
//
// # What this one proves that the line spec could not
//
// The release count against real content. `forge-line.spec.ts` says so
// itself: its line holds nothing, so its discard reports zero and no
// asset moves. This one puts an asset on a line through a round, and
// the discard at the end is the first time anything has checked that
// the number in that notice is the number of assets the line was
// holding.
//
// # Why it leaves the fixture as it found it
//
// The line is created and discarded within the spec, for the reason the
// line spec gives — a destructive verb provoked against seeded fixture
// is usable once. The asset never moves: holding is a prohibition on
// deleting the bytes rather than a place they go, so what the discard
// releases is the line's claim on it, and that count is what the last
// assertion reads.
import { browser } from "@wdio/globals";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 20_000;
const COLD_MS = 60_000;
const POLL_GAP_MS = 250;

/** A name nothing else in the fixture answers to. */
const LINE_NAME = "e2e-forge-pursuit";
const WORK_TITLE = "e2e first round";

const FORGE_ROW = 'aside.sidebar button[title^="Lines on this machine"]';
const DRAWER = '[role="dialog"][aria-label="Forge"]';

/** Same shape `forge-line.spec.ts` uses, and for its reason: a raw
 *  driver call carries no timeout, so the bound has to be a race, and
 *  on failure the error names the step plus what already passed. */
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
    return value;
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(
      `step "${name}" failed: ${why}\n` +
        `  completed before it: ${trail.length > 0 ? trail.join(" → ") : "(none)"}`,
    );
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** Polls a condition built from `execute`, which is untaxed — a
 *  `findElement` poll costs ~6 s and blows any honest ceiling. */
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
      if (await check()) return;
      if (Date.now() > deadline) throw new Error(message);
      await new Promise((r) => setTimeout(r, POLL_GAP_MS));
    }
  });
}

interface WorkSnapshot {
  drawerPresent: boolean;
  /** Line names in the open section, in order. */
  open: string[];
  /** Line names under Archived. */
  archived: string[];
  /** The selected line's header, if one is selected. */
  heading: string;
  /** Titles of the work listed against it, open first. */
  work: string[];
  /** The title in the work header, when one piece of work is showing. */
  working: string;
  /** How many rounds the log holds. */
  rounds: number;
  /** Operation labels across every round, in order. */
  ops: string[];
  /** The contents tab's count line. */
  onTheLine: string;
  /** The discard notice, if it is showing. */
  released: string;
}

/** One `execute` for the whole drawer: every read this spec makes is a
 *  field of this, so a step costs one driver call rather than one per
 *  question. */
function readDrawer(): Promise<WorkSnapshot> {
  return browser.execute(() => {
    const empty = {
      drawerPresent: false,
      open: [] as string[],
      archived: [] as string[],
      heading: "",
      work: [] as string[],
      working: "",
      rounds: 0,
      ops: [] as string[],
      onTheLine: "",
      released: "",
    };
    const drawer = document.querySelector('[role="dialog"][aria-label="Forge"]');
    if (drawer === null) return empty;
    const nav = drawer.querySelector('nav[aria-label="Lines"]');
    const lists = nav === null ? [] : Array.from(nav.querySelectorAll("ul"));
    const namesIn = (ul: Element | undefined) =>
      ul === undefined
        ? []
        : Array.from(ul.querySelectorAll("button")).map(
            (b) => b.textContent?.trim() ?? "",
          );
    const detail = drawer.querySelector(".line");
    const counts = Array.from(drawer.querySelectorAll(".line > .quiet")).map(
      (p) => p.textContent?.trim() ?? "",
    );
    return {
      drawerPresent: true,
      open: namesIn(lists[0]),
      archived: namesIn(lists[1]),
      heading: detail?.querySelector("h3")?.textContent?.trim() ?? "",
      work: Array.from(drawer.querySelectorAll(".work-list button")).map(
        (b) => b.textContent?.trim() ?? "",
      ),
      working:
        drawer.querySelector(".work-head strong")?.textContent?.trim() ?? "",
      rounds: drawer.querySelectorAll(".rounds > li").length,
      ops: Array.from(drawer.querySelectorAll(".ops .op-name")).map(
        (s) => s.textContent?.trim() ?? "",
      ),
      onTheLine: counts.filter((t) => t.endsWith("on the line"))[0] ?? "",
      released: drawer.querySelector(".released")?.textContent?.trim() ?? "",
    };
  });
}

/** Clicks by selector through `execute`. `$().click()` is a taxed
 *  driver call and this spec makes a dozen of them. */
function clickIn(selector: string): Promise<boolean> {
  return browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    if (el === null) return false;
    (el as HTMLElement).click();
    return true;
  }, selector);
}

/** Clicks the button inside the drawer whose label is exactly this. */
function clickLabelled(within: string, label: string): Promise<boolean> {
  return browser.execute(
    (scope: string, wanted: string) => {
      const root = document.querySelector(scope);
      if (root === null) return false;
      const button = Array.from(root.querySelectorAll("button")).filter(
        (b) => (b.textContent?.trim() ?? "") === wanted,
      )[0];
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    within,
    label,
  );
}

describe("a pursuit against a line", () => {
  before(async () => {
    const trail: string[] = [];
    await stage(trail, "install __name shim", DRIVER_MS, () =>
      browser.execute(
        "window.__name = window.__name || function (target) { return target; };",
      ),
    );
    await pollUntil(
      trail,
      "app window paints",
      COLD_MS,
      async () =>
        browser.execute(() => document.querySelector("aside.sidebar") !== null),
      "the app never painted its sidebar",
    );
    await pollUntil(
      trail,
      "the grid has a card to select",
      COLD_MS,
      async () =>
        browser.execute(
          () => document.querySelector(".card[data-asset-id]") !== null,
        ),
      "no card ever painted, so there is nothing to put on a line",
    );
  });

  it("opens work, puts a selection on the line, and closes it", async () => {
    const trail: string[] = [];

    // The selection comes first, and it has to: the drawer is an
    // overlay, so the grid is not reachable once the forge is up. A
    // meta-click is the gesture that toggles selection rather than
    // opening the detail pane — `el.click()` carries no modifier, so
    // the event is built rather than dispatched off the element.
    await stage(trail, "select one card", DRIVER_MS, () =>
      browser.execute(() => {
        const card = document.querySelector(".card[data-asset-id]");
        if (card === null) return false;
        card.dispatchEvent(
          new MouseEvent("click", { bubbles: true, metaKey: true }),
        );
        return true;
      }),
    );

    await stage(trail, "open the forge", DRIVER_MS, () => clickIn(FORGE_ROW));
    await pollUntil(
      trail,
      "the drawer paints",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).drawerPresent,
      "the forge drawer never appeared",
    );

    await stage(trail, "fill the new-line form", DRIVER_MS, () =>
      browser.execute((name: string) => {
        const form = document.querySelector(
          '[role="dialog"][aria-label="Forge"] form.new-line',
        );
        if (form === null) return false;
        const input = form.querySelector("input");
        const select = form.querySelector("select");
        if (input === null || select === null) return false;
        const rule = Array.from(select.options).filter(
          (o) => o.value !== "",
        )[0];
        if (rule === undefined) return false;
        // Svelte binds on `input` / `change`, so setting `.value` alone
        // would leave the component's state on the old one.
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        select.value = rule.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        return true;
      }, LINE_NAME),
    );
    await stage(trail, "open the line", DRIVER_MS, () =>
      clickIn(`${DRAWER} form.new-line button`),
    );
    await pollUntil(
      trail,
      "the line reaches the list",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).open.includes(LINE_NAME),
      "the opened line never appeared in the list",
    );

    await stage(trail, "select the line", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} nav[aria-label="Lines"]`, LINE_NAME),
    );
    await pollUntil(
      trail,
      "the line's own panel paints",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).heading === LINE_NAME,
      "selecting the line did not put it in the header",
    );

    // The header button is the one #180 left disabled. Pressing it is
    // what proves the work tab is reachable the way the design says.
    await stage(trail, "press open a pursuit", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} .line header`, "open a pursuit"),
    );
    await stage(trail, "fill the new-work form", DRIVER_MS, () =>
      browser.execute((title: string) => {
        const form = document.querySelector(
          '[role="dialog"][aria-label="Forge"] form.new-work',
        );
        if (form === null) return false;
        const input = form.querySelector("input");
        if (input === null) return false;
        input.value = title;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        return true;
      }, WORK_TITLE),
    );
    await stage(trail, "open the pursuit", DRIVER_MS, () =>
      clickIn(`${DRAWER} form.new-work button`),
    );
    await pollUntil(
      trail,
      "the work paints with nothing asked for",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        return drawer.working === WORK_TITLE && drawer.rounds === 0;
      },
      "opening the pursuit did not put it on screen",
    );

    // The selection becomes a round. One operation, named from the
    // asset's locator — which is the read `hydrate_cards` answers and
    // the reason the name is not blank.
    await stage(trail, "add the selection", DRIVER_MS, () =>
      browser.execute(() => {
        const compose = document.querySelector(
          '[role="dialog"][aria-label="Forge"] .compose button',
        ) as HTMLButtonElement | null;
        if (compose === null || compose.disabled) return false;
        compose.click();
        return true;
      }),
    );
    await pollUntil(
      trail,
      "the round reaches the log",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        return drawer.rounds === 1 && drawer.ops.length === 1;
      },
      "the round never appeared in the work's log",
    );

    // Nothing is on the line yet. A round is a request, and the panel
    // has to be able to say so — if this read finds the entry already
    // there, `push` is landing something and the model is not what the
    // screen was built against.
    await stage(trail, "look at the contents", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} .tabs`, "on the line"),
    );
    await pollUntil(
      trail,
      "the line still holds nothing",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).onTheLine === "0 on the line",
      "the line held something before the work was closed",
    );

    await stage(trail, "back to the work", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} .tabs`, "work"),
    );
    await stage(trail, "close it satisfied", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} .close`, "close · put it on the line"),
    );
    await pollUntil(
      trail,
      "the work reports it ended",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('[role="dialog"][aria-label="Forge"] .work-head')
              ?.textContent?.includes("satisfied") ?? false,
        ),
      "closing the work did not change what it says about itself",
    );

    // And now it is on the line. This is the only step that proves a
    // close lands what the rounds asked for, which is the one thing
    // every read before it deliberately does not do.
    await stage(trail, "look at the contents again", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} .tabs`, "on the line"),
    );
    await pollUntil(
      trail,
      "the entry is on the line",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).onTheLine === "1 on the line",
      "the satisfied close did not put the round's entry on the line",
    );

    // Clean up through the panel, which is also the assertion the line
    // spec could not make: the discard names what it released, and now
    // there is something to release.
    await stage(trail, "archive it", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} .verbs`, "archive"),
    );
    await pollUntil(
      trail,
      "the line moves to archived",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).archived.includes(LINE_NAME),
      "archiving did not move the line into the archived section",
    );
    await stage(trail, "select the archived line", DRIVER_MS, () =>
      clickLabelled(`${DRAWER} nav[aria-label="Lines"]`, LINE_NAME),
    );
    await stage(trail, "press discard", DRIVER_MS, () =>
      clickIn(`${DRAWER} .verbs button.danger`),
    );
    await stage(trail, "confirm the discard", DRIVER_MS, () =>
      clickLabelled("body", "Discard Forever"),
    );
    await pollUntil(
      trail,
      "the notice names the asset it released",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        return (
          !drawer.open.includes(LINE_NAME) &&
          !drawer.archived.includes(LINE_NAME) &&
          drawer.released.startsWith("Discarded. 1 asset released")
        );
      },
      "the discard did not report the one asset the line was holding",
    );

    await stage(trail, "close the drawer", DRIVER_MS, () =>
      clickIn(`${DRAWER} .drawer-close`),
    );
  });
});
