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
// # Why each `it` is one long walk
//
// The order is the model's. A round needs a pursuit, a pursuit needs a
// line, and what a satisfied close puts on the line is only visible
// because the round before it asked for something. None of these is a
// provocation that can be set up some other way without seeding the
// thing the spec is meant to prove. The conversation in the middle
// hangs off the round for the same reason: an anchor is resolved
// against the work rather than taken on trust, so there is no round to
// talk about until one has been written.
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

/// What every line this spec has ever made is called, and what this
/// run's is called.
///
/// A name unique to the run, because the first thing a failed run
/// leaves behind is its line — and the second thing is a spec that
/// picks the leftover instead of what it just made. That happened: a
/// run failed before its discard, the next one archived its own line
/// and then discarded nothing, because "the first button reading
/// e2e-forge-pursuit" was the corpse of the run before.
///
/// `Name` carries no claim of uniqueness and the model says so, so the
/// uniqueness has to be here rather than assumed of the forge.
const LINE_PREFIX = "e2e-forge-pursuit";
const RUN = Date.now();
const LINE_NAME = `${LINE_PREFIX}-${RUN}`;
/// The second test's line, which outlives one close and is worked
/// again. Same prefix, so one sweep answers for both.
const SECOND_LINE = `${LINE_PREFIX}-${RUN}-again`;
const WORK_TITLE = "e2e first round";
const RENAMED_ENTRY = "renamed by the second pass";

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
/// `message` may be a function, which is how a poll reports what it
/// actually saw: a string is built before the polling starts and can
/// only describe what was expected.
async function pollUntil(
  trail: string[],
  name: string,
  ms: number,
  check: () => Promise<boolean>,
  message: string | (() => string),
) {
  await stage(trail, name, ms + DRIVER_MS, async () => {
    const deadline = Date.now() + ms;
    for (;;) {
      if (await check()) return;
      if (Date.now() > deadline) {
        throw new Error(typeof message === "string" ? message : message());
      }
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
  /** The title in the work header, when one piece of work is showing. */
  working: string;
  /** How many rounds the log holds. */
  rounds: number;
  /** Operation labels across every round, in order. */
  ops: string[];
  /// The names on the rows a close would leave, in order — the fold of
  /// the line and the work, which is what the second test changes.
  projected: string[];
  /** What the conversation surface is about, when one is open. */
  talkAbout: string;
  /** What each message says now, in order. */
  said: string[];
  /** How many messages say they have been corrected. */
  corrected: number;
  /** The contents tab's count line. */
  onTheLine: string;
  /** The discard notice, if it is showing. */
  released: string;
  /// What the app refused, if anything.
  ///
  /// Outside the drawer, and read anyway: `mutate` puts every refusal
  /// here and nowhere else, so a spec that only reads the drawer sees a
  /// write silently not happening. That is what the sweep hit — a
  /// discard pressed, confirmed, and refused, with the reason on screen
  /// the whole time in a place nothing was looking at.
  refusal: string;
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
      working: "",
      rounds: 0,
      ops: [] as string[],
      projected: [] as string[],
      talkAbout: "",
      said: [] as string[],
      corrected: 0,
      onTheLine: "",
      released: "",
      refusal: "",
    };
    // Read before the early return: a refusal outlives the drawer, and
    // the drawer being gone is one of the things a refusal explains.
    const refusal =
      document.querySelector(".refusal-toast")?.textContent?.trim() ?? "";
    const drawer = document.querySelector('[role="dialog"][aria-label="Forge"]');
    if (drawer === null) return { ...empty, refusal };
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
      working:
        drawer.querySelector(".work-head strong")?.textContent?.trim() ?? "",
      rounds: drawer.querySelectorAll(".rounds > li").length,
      // Verb and name together: an operation carries its verb, unlike a
      // change row, and a test that reads only the name cannot tell a
      // replace from the add before it.
      ops: Array.from(drawer.querySelectorAll(".ops li")).map((li) =>
        (li.textContent ?? "").replace(/\s+/g, " ").trim(),
      ),
      projected: Array.from(drawer.querySelectorAll(".projected .op-name")).map(
        (s) => s.textContent?.trim() ?? "",
      ),
      talkAbout: drawer.querySelector(".talk h4")?.textContent?.trim() ?? "",
      said: Array.from(drawer.querySelectorAll(".talk .said")).map(
        (p) => p.textContent?.trim() ?? "",
      ),
      corrected: Array.from(drawer.querySelectorAll(".talk .by")).filter((p) =>
        (p.textContent ?? "").includes("corrected"),
      ).length,
      onTheLine: counts.filter((t) => t.endsWith("on the line"))[0] ?? "",
      // Whitespace-collapsed, because this one is read as a sentence
      // rather than for a word: the count and its noun are two
      // expressions in the markup with a line break between them, so
      // `textContent` has "1\n        asset" where a reader sees "1
      // asset". `forge-line.spec.ts` compares only the first word and
      // never met this.
      released: (drawer.querySelector(".released")?.textContent ?? "")
        .replace(/\s+/g, " ")
        .trim(),
      refusal,
    };
  });
}

/// Writes a PNG of the window under `workspace/`, which is gitignored.
///
/// Every assertion here reads `textContent`, and text that is present
/// says nothing about whether it can be reached: this spec passed while
/// the add control was a disabled button telling somebody to go and
/// select in a grid the drawer was covering. A picture is what shows
/// that, and nobody had looked at one.
/// The path is relative to wdio's working directory, which is
/// `crates/asterism-ui` — two levels under the `workspace/` this writes
/// to, and `saveScreenshot` does not create a directory it is handed.
async function shot(name: string): Promise<void> {
  await browser.saveScreenshot(`../../workspace/forge-${name}.png`);
}

/// Clicks by selector through `execute`, and **fails when it hits
/// nothing**. `$().click()` is a taxed driver call and this spec makes
/// a dozen of them, so the click is done in the page — but a click in
/// the page answers with a boolean, and a boolean nobody reads is a
/// step that passes for pressing thin air. This spec did that: "press
/// discard" found no discard button, returned false, and was recorded
/// as done.
///
/// It also cannot see an overlay. `el.click()` reaches an element under
/// a modal that a person could not press, which is why `noModal` is
/// asserted rather than assumed.
async function press(selector: string): Promise<void> {
  const hit = await browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    if (el === null) return false;
    (el as HTMLElement).click();
    return true;
  }, selector);
  if (!hit) throw new Error(`nothing to press at ${selector}`);
}

/// Clicks the button inside `within` whose label is exactly `label`,
/// and fails when there is none, for `press`'s reason.
async function pressLabelled(within: string, label: string): Promise<void> {
  const hit = await browser.execute(
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
  if (!hit) throw new Error(`no button reading "${label}" inside ${within}`);
}

/// Every line this spec's earlier runs left behind, this run's aside.
async function leftovers(): Promise<string[]> {
  const drawer = await readDrawer();
  return [...drawer.open, ...drawer.archived].filter(
    (name) => name.startsWith(LINE_PREFIX) && name !== LINE_NAME,
  );
}

/// Discards them, oldest first, before this run makes its own.
///
/// A run that fails before its cleanup leaves a line, and the fixture
/// showed six of them. They are not inert: they carry the name the next
/// run looks for, and the spec that finds one instead of its own line
/// archives the right thing and then discards nothing. Unique names
/// stop the confusion; this stops the pile.
///
/// Only this spec's own prefix. `e2e-forge-line` belongs to the spec
/// next door and its leftovers are that spec's to answer for.
async function sweepLeftovers(trail: string[]): Promise<void> {
  for (let round = 0; round < 20; round += 1) {
    const stale = await leftovers();
    if (stale.length === 0) return;
    const name = stale[0];
    const open = (await readDrawer()).open.includes(name);
    await stage(trail, `sweep: select ${name}`, DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, name),
    );

    // Work first, and the model is why: a drop takes the history the
    // work was cut from, so it refuses while any is open rather than
    // leaving a log against nothing. A leftover is exactly the line
    // whose work never got closed, so every one of these needs it.
    await stage(trail, "sweep: to the work", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "work"),
    );
    for (let piece = 0; piece < 10; piece += 1) {
      const anyOpen = await browser.execute(
        () =>
          document.querySelector(
            '[role="dialog"][aria-label="Forge"] .work-list:not(.ended) button',
          ) !== null,
      );
      if (!anyOpen) break;
      await stage(trail, "sweep: open the piece", DRIVER_MS, () =>
        press(`${DRAWER} .work-list:not(.ended) button`),
      );
      await pollUntil(
        trail,
        "sweep: the piece paints",
        ROUND_TRIP_MS,
        async () =>
          browser.execute(
            () =>
              document.querySelector(
                '[role="dialog"][aria-label="Forge"] .work-head',
              ) !== null,
          ),
        "the piece of work never opened",
      );
      await stage(trail, "sweep: abandon it", DRIVER_MS, () =>
        pressLabelled(`${DRAWER} .close`, "close · abandon"),
      );
      await pollUntil(
        trail,
        "sweep: it ended",
        ROUND_TRIP_MS,
        async () =>
          browser.execute(
            () =>
              document
                .querySelector('[role="dialog"][aria-label="Forge"] .work-head')
                ?.textContent?.includes("closed · abandon") ?? false,
          ),
        "abandoning the piece of work did not take",
      );
      await stage(trail, "sweep: back to the list", DRIVER_MS, () =>
        pressLabelled(`${DRAWER} .work-head`, "← all work"),
      );
    }

    if (open) {
      // A discard is reachable only from an archived line, which is the
      // model's order rather than the screen's.
      await stage(trail, "sweep: archive it", DRIVER_MS, () =>
        pressLabelled(`${DRAWER} .verbs`, "archive"),
      );
      await stage(trail, "sweep: select it again", DRIVER_MS, () =>
        pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, name),
      );
    }
    await stage(trail, "sweep: discard it", DRIVER_MS, () =>
      press(`${DRAWER} .verbs button.danger`),
    );
    await stage(trail, "sweep: confirm", DRIVER_MS, () =>
      pressLabelled("body", "Discard Forever"),
    );
    // By count rather than by name: several leftovers can share one,
    // and `includes` would still be true with one of them gone.
    let saw = "";
    await pollUntil(
      trail,
      "sweep: one fewer",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        const covering = await browser.execute(() =>
          Array.from(
            document.querySelectorAll('[role="dialog"], .prompt-panel, .confirm-panel'),
          )
            .filter((el) => el.getAttribute("aria-label") !== "Forge")
            .map((el) => el.textContent?.trim().slice(0, 30) ?? ""),
        );
        saw =
          `open=[${drawer.open.join("|")}] archived=[${drawer.archived.join("|")}] ` +
          `heading=${JSON.stringify(drawer.heading)} released=${JSON.stringify(drawer.released)} ` +
          `refusal=${JSON.stringify(drawer.refusal)} covering=${JSON.stringify(covering)}`;
        return (
          [...drawer.open, ...drawer.archived].filter(
            (n) => n.startsWith(LINE_PREFIX) && n !== LINE_NAME,
          ).length < stale.length
        );
      },
      () => `${name} did not go; last saw ${saw}`,
    );
  }
  throw new Error("more leftovers than this sweep is willing to remove");
}

/// Opens a pursuit against the selected line and waits for it.
///
/// Both tests do this and the second does it twice, which is the whole
/// point of the second: a line is worked more than once.
async function openWorkTitled(trail: string[], title: string): Promise<void> {
  await stage(trail, `open work: ${title}`, DRIVER_MS, () =>
    pressLabelled(`${DRAWER} .line header`, "open work"),
  );
  await stage(trail, "fill the new-work form", DRIVER_MS, () =>
    browser.execute((wanted: string) => {
      const form = document.querySelector(
        '[role="dialog"][aria-label="Forge"] form.new-work',
      );
      const input = form?.querySelector("input");
      if (!form || !input) return false;
      input.value = wanted;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      return true;
    }, title),
  );
  await stage(trail, "open it", DRIVER_MS, () =>
    press(`${DRAWER} form.new-work button`),
  );
  await pollUntil(
    trail,
    `${title} is showing`,
    ROUND_TRIP_MS,
    async () => (await readDrawer()).working === title,
    `opening ${title} did not put it on screen`,
  );
}

/// Ends the work being shown and waits for it to say so.
async function closeWork(trail: string[], label: string): Promise<void> {
  await stage(trail, `press ${label}`, DRIVER_MS, () =>
    pressLabelled(`${DRAWER} .close`, label),
  );
  await pollUntil(
    trail,
    "the work reports it ended",
    ROUND_TRIP_MS,
    async () =>
      browser.execute(() => {
        const head = document.querySelector(
          '[role="dialog"][aria-label="Forge"] .work-head',
        );
        const said = head?.textContent ?? "";
        return said.includes("closed");
      }),
    "closing the work did not change what it says about itself",
  );
}

/// Fails if anything is covering the drawer.
///
/// A modal left open by an earlier spec covered every screen this one
/// captured, and every assertion still passed, because `execute` clicks
/// through it. A person would have been able to press nothing at all.
/// So the state a picture would have shown is asserted here instead of
/// being left to whoever looks at one.
async function noModal(where: string): Promise<void> {
  const open = await browser.execute(() => {
    const dialogs = Array.from(
      document.querySelectorAll('[role="dialog"], .prompt-modal, .confirm-modal'),
    ).filter((el) => el.getAttribute("aria-label") !== "Forge");
    return dialogs.map((el) => el.textContent?.trim().slice(0, 40) ?? "");
  });
  if (open.length > 0) {
    throw new Error(
      `something is covering the drawer at ${where}: ${JSON.stringify(open)}`,
    );
  }
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

    await stage(trail, "open the forge", DRIVER_MS, () => press(FORGE_ROW));
    await pollUntil(
      trail,
      "the drawer paints",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).drawerPresent,
      "the forge drawer never appeared",
    );
    // Before anything is read off the screen. A modal left open by an
    // earlier spec is invisible to every assertion here and fatal to a
    // person, and this is the first moment the drawer exists to be
    // covered.
    await stage(trail, "nothing is covering the drawer", DRIVER_MS, () =>
      noModal("the drawer just opened"),
    );
    await sweepLeftovers(trail);

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
      press(`${DRAWER} form.new-line button`),
    );
    await pollUntil(
      trail,
      "the line reaches the list",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).open.includes(LINE_NAME),
      "the opened line never appeared in the list",
    );

    await stage(trail, "select the line", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, LINE_NAME),
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
    await stage(trail, "press open work", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .line header`, "open work"),
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
      press(`${DRAWER} form.new-work button`),
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
    await shot("work-opened");

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
    await shot("round-pushed");

    // Say something about the round, and then correct it. The
    // correction is the half worth driving through the real backend:
    // the model keeps what was said first and every revision, and a
    // surface that rendered only the latest would misreport somebody.
    await stage(trail, "open a conversation about the round", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .round-head`, "say something"),
    );
    await pollUntil(
      trail,
      "the conversation surface paints",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).talkAbout === "Said about this round",
      "pressing say something did not open the conversation surface",
    );
    await stage(trail, "start it", DRIVER_MS, () =>
      browser.execute(() => {
        const form = document.querySelector(
          '[role="dialog"][aria-label="Forge"] .talk form.start',
        );
        const input = form?.querySelector("input");
        if (!form || !input) return false;
        input.value = "first thing said";
        input.dispatchEvent(new Event("input", { bubbles: true }));
        form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
        return true;
      }),
    );
    await pollUntil(
      trail,
      "what was said comes back",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).said.includes("first thing said"),
      "the conversation never showed what was said in it",
    );
    await stage(trail, "start a correction", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .talk`, "correct"),
    );
    await stage(trail, "save the correction", DRIVER_MS, () =>
      browser.execute(() => {
        const input = document.querySelector(
          '[role="dialog"][aria-label="Forge"] .talk li form input',
        ) as HTMLInputElement | null;
        const form = input?.closest("form");
        if (!input || !form) return false;
        input.value = "what I meant";
        input.dispatchEvent(new Event("input", { bubbles: true }));
        form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
        return true;
      }),
    );
    await pollUntil(
      trail,
      "the message says it was corrected",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        return drawer.said.includes("what I meant") && drawer.corrected === 1;
      },
      "the correction did not reach the message, or left no trace of itself",
    );
    await shot("conversation-corrected");
    await stage(trail, "close the conversation", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .talk header`, "close"),
    );

    // Nothing is on the line yet. A round is a request, and the panel
    // has to be able to say so — if this read finds the entry already
    // there, `push` is landing something and the model is not what the
    // screen was built against.
    await stage(trail, "look at the contents", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "on the line"),
    );
    await pollUntil(
      trail,
      "the line still holds nothing",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).onTheLine === "0 on the line",
      "the line held something before the work was closed",
    );

    await stage(trail, "back to the work", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "work"),
    );
    await stage(trail, "close it satisfied", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .close`, "close · put it on the line"),
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
              ?.textContent?.includes("put it on the line") ?? false,
        ),
      "closing the work did not change what it says about itself",
    );

    // And now it is on the line. This is the only step that proves a
    // close lands what the rounds asked for, which is the one thing
    // every read before it deliberately does not do.
    await stage(trail, "look at the contents again", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "on the line"),
    );
    await pollUntil(
      trail,
      "the entry is on the line",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).onTheLine === "1 on the line",
      "the satisfied close did not put the round's entry on the line",
    );

    // And the entry can be *seen*, which is a different question from
    // whether it is listed.
    //
    // A tile passes by showing a thumbnail or by saying what the thing
    // is. What it may not do is stay an empty box, which is what it did
    // for twenty seconds when this check was first written: the fixture
    // asset this run picks is a recording, `thumbById` answers a miss
    // with a transparent pixel, and "no picture yet" and "no picture
    // ever" were the same grey square.
    let tile = "";
    await pollUntil(
      trail,
      "the entry has something to look at",
      ROUND_TRIP_MS,
      async () => {
        tile = await browser.execute(() => {
          // Through the button, which is the tile's frame rather than
          // its content: what a person sees is the thumbnail or the
          // word inside it.
          const cell = document.querySelector(
            '[role="dialog"][aria-label="Forge"] .entries li > :first-child',
          );
          const inner =
            cell?.tagName === "BUTTON" ? cell.firstElementChild : cell;
          if (!inner) return "(no tile)";
          if (inner.tagName === "IMG") {
            return `img:${(inner as HTMLImageElement).src.slice(0, 11)}`;
          }
          // By class rather than by className: Svelte appends a
          // per-component scoping class, so the attribute is never the
          // string the markup wrote.
          const said = inner.textContent?.trim() ?? "";
          return `${inner.classList.contains("kind") ? "kind" : "blank"}:${said}`;
        });
        return (
          tile.startsWith("img:blob:") ||
          tile.startsWith("img:asset:") ||
          /^kind:\S/.test(tile)
        );
      },
      () =>
        `the entry on the line is an empty box: the tile is ${JSON.stringify(tile)}`,
    );
    await shot("landed-on-the-line");

    // Clean up through the panel, which is also the assertion the line
    // spec could not make: the discard names what it released, and now
    // there is something to release.
    await stage(trail, "archive it", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .verbs`, "archive"),
    );
    await pollUntil(
      trail,
      "the line moves to archived",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).archived.includes(LINE_NAME),
      "archiving did not move the line into the archived section",
    );
    await stage(trail, "select the archived line", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, LINE_NAME),
    );
    await stage(trail, "press discard", DRIVER_MS, () =>
      press(`${DRAWER} .verbs button.danger`),
    );
    await stage(trail, "confirm the discard", DRIVER_MS, () =>
      pressLabelled("body", "Discard Forever"),
    );
    // What this last poll saw, if it never comes true. A conjunction of
    // three that fails as one boolean says only that something went
    // wrong; the first run of this spec failed here and the three
    // candidates — the line still listed, no notice at all, a count
    // that is not one — want three different fixes.
    let lastSeen = "";
    await pollUntil(
      trail,
      "the notice names the asset it released",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        lastSeen =
          `open=[${drawer.open.join("|")}] archived=[${drawer.archived.join("|")}] ` +
          `released=${JSON.stringify(drawer.released)}`;
        return (
          !drawer.open.includes(LINE_NAME) &&
          !drawer.archived.includes(LINE_NAME) &&
          drawer.released.startsWith("Discarded. 1 asset released")
        );
      },
      () =>
        `the discard did not report the one asset the line was holding; last saw ${lastSeen}`,
    );

    await stage(trail, "close the drawer", DRIVER_MS, () =>
      press(`${DRAWER} .drawer-close`),
    );
  });

  // A second piece of work against a line that already holds
  // something, which is where three of the four verbs first get
  // pressed.
  //
  // The test above opens a pursuit against an empty line and adds to
  // it, and stops. `replace`, `rename` and `remove` name an entry that
  // already exists — the model lets that be one this work added a
  // moment ago, so it is the *script* above that never reaches them
  // rather than the model — and they were about to ship never having
  // run against the real backend once. This is the loop a person
  // actually works: put something on a line, come back later, change
  // what is there, and land that too.
  //
  // It also drives the way back from a step-aside, because a second
  // round needs a pick and the drawer is what is covering the grid.
  // That gesture had no way back at all when it was first written.
  it("comes back to a line that has contents, and changes them", async () => {
    const trail: string[] = [];

    await stage(trail, "pick two cards", DRIVER_MS, () =>
      browser.execute(() => {
        const cards = Array.from(
          document.querySelectorAll(".card[data-asset-id]"),
        ).slice(0, 2);
        if (cards.length < 2) return false;
        for (const card of cards) {
          card.dispatchEvent(
            new MouseEvent("click", { bubbles: true, metaKey: true }),
          );
        }
        return true;
      }),
    );

    await stage(trail, "open the forge", DRIVER_MS, () => press(FORGE_ROW));
    await pollUntil(
      trail,
      "the drawer paints",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).drawerPresent,
      "the forge drawer never appeared",
    );
    await sweepLeftovers(trail);

    await stage(trail, "fill the new-line form", DRIVER_MS, () =>
      browser.execute((name: string) => {
        const form = document.querySelector(
          '[role="dialog"][aria-label="Forge"] form.new-line',
        );
        const input = form?.querySelector("input");
        const select = form?.querySelector("select");
        if (!form || !input || !select) return false;
        const rule = Array.from(select.options).filter((o) => o.value !== "")[0];
        if (rule === undefined) return false;
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        select.value = rule.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        return true;
      }, SECOND_LINE),
    );
    await stage(trail, "open the line", DRIVER_MS, () =>
      press(`${DRAWER} form.new-line button`),
    );
    await pollUntil(
      trail,
      "the line reaches the list",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).open.includes(SECOND_LINE),
      "the opened line never appeared in the list",
    );
    await stage(trail, "select it", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, SECOND_LINE),
    );

    // First landing: two entries arrive from the grid.
    await openWorkTitled(trail, "first pass");
    await stage(trail, "add both", DRIVER_MS, () =>
      press(`${DRAWER} .compose button`),
    );
    await pollUntil(
      trail,
      "both are asked for",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).projected.length === 2,
      "the round did not put two entries in the projection",
    );
    await closeWork(trail, "close · put it on the line");
    await stage(trail, "look at the contents", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "on the line"),
    );
    await pollUntil(
      trail,
      "two are on the line",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).onTheLine === "2 on the line",
      "the first close did not land both entries",
    );

    // Second pursuit, cut from a line that now holds something. The
    // projection is seeded from the line rather than from this work,
    // which is the case the fold exists for.
    await openWorkTitled(trail, "second pass");
    await pollUntil(
      trail,
      "the line's own entries are what it starts from",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).projected.length === 2,
      "a pursuit against a line with contents started from nothing",
    );
    const before = (await readDrawer()).projected;

    // Rename. The prompt is the App's, reached through the row.
    await stage(trail, "rename the first entry", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .projected li:first-child`, "rename"),
    );
    await stage(trail, "answer the prompt", DRIVER_MS, () =>
      browser.execute((name: string) => {
        const input = document.querySelector(
          ".prompt-panel input.prompt-input",
        ) as HTMLInputElement | null;
        const ok = document.querySelector(
          ".prompt-panel .prompt-btn.primary",
        ) as HTMLElement | null;
        if (input === null || ok === null) return false;
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        ok.click();
        return true;
      }, RENAMED_ENTRY),
    );
    await pollUntil(
      trail,
      "the new name is what the line would say",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).projected.includes(RENAMED_ENTRY),
      "the rename never reached the projection",
    );

    // Replace, which needs exactly one asset picked — and the grid is
    // behind the drawer, so this is the step-aside round trip.
    await stage(trail, "step aside to pick", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .compose`, "pick in the grid — this steps aside"),
    );
    await pollUntil(
      trail,
      "the way back is on screen",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () => document.querySelector('[aria-label="The forge is waiting"]') !== null,
        ),
      "stepping aside left nothing on screen to come back with",
    );
    await stage(trail, "pick one card", DRIVER_MS, () =>
      browser.execute(() => {
        const card = document.querySelectorAll(".card[data-asset-id]")[2];
        if (card === undefined) return false;
        card.dispatchEvent(
          new MouseEvent("click", { bubbles: true, metaKey: true }),
        );
        return true;
      }),
    );
    await stage(trail, "come back", DRIVER_MS, () =>
      pressLabelled('[aria-label="The forge is waiting"]', "back to the forge"),
    );
    await pollUntil(
      trail,
      "it comes back to the same work",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).working === "second pass",
      "coming back did not land on the work it stepped aside from",
    );
    await stage(trail, "replace the second entry", DRIVER_MS, () =>
      pressLabelled(
        `${DRAWER} .projected li:nth-child(2)`,
        "replace with the selected",
      ),
    );
    await pollUntil(
      trail,
      "the round holds a replace",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).ops.some((op) => op.startsWith("replace")),
      "the replace never reached the log",
    );

    // Remove, and put it back, and remove it again. The middle step is
    // what proves an entry comes back under its own id rather than as a
    // new arrival.
    await stage(trail, "remove the first entry", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .projected li:first-child`, "remove"),
    );
    await pollUntil(
      trail,
      "it reads as leaving",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document.querySelector(
              '[role="dialog"][aria-label="Forge"] .projected li.gone',
            ) !== null,
        ),
      "the removal did not show on the row",
    );
    await stage(trail, "put it back", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .projected li.gone`, "put back"),
    );
    await pollUntil(
      trail,
      "it is on the line again, and still one entry",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        return (
          drawer.projected.length === before.length &&
          !(await browser.execute(
            () =>
              document.querySelector(
                '[role="dialog"][aria-label="Forge"] .projected li.gone',
              ) !== null,
          ))
        );
      },
      "putting it back either did not take or arrived as a second entry",
    );
    await stage(trail, "remove it for good", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .projected li:first-child`, "remove"),
    );

    // And land the lot.
    await closeWork(trail, "close · put it on the line");
    await stage(trail, "look at the contents again", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "on the line"),
    );
    await pollUntil(
      trail,
      "one is on the line, renamed and refilled",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).onTheLine === "1 on the line",
      "the second close did not leave exactly the entry it kept",
    );
    await shot("second-pass-landed");

    // What the line calls an entry is not what the asset is called, so
    // "what is this actually" is a question the forge raises and cannot
    // answer. The detail pane answers it, and comes up *over* the
    // drawer rather than instead of it — which is the whole reason the
    // tile does not have to step aside first.
    await stage(trail, "open the entry properly", DRIVER_MS, () =>
      press(`${DRAWER} .entries .tile`),
    );
    await pollUntil(
      trail,
      "the detail pane is over the drawer",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(() => {
          const pane = document.querySelector(".detail-backdrop .detail-panel");
          const drawer = document.querySelector(
            '[role="dialog"][aria-label="Forge"]',
          );
          return pane !== null && drawer !== null;
        }),
      "pressing a tile did not open the detail pane, or closed the drawer",
    );
    await shot("detail-over-the-drawer");
    await stage(trail, "close the detail", DRIVER_MS, () =>
      press(".detail-backdrop .detail-close"),
    );
    await pollUntil(
      trail,
      "the line is where it was left",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        return drawer.drawerPresent && drawer.heading === SECOND_LINE;
      },
      "closing the detail did not leave the forge where it was",
    );

    // The chain records both landings, and the genesis is not one of
    // them.
    await stage(trail, "read the history", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "history"),
    );
    await pollUntil(
      trail,
      "two change points",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document.querySelectorAll(
              '[role="dialog"][aria-label="Forge"] .chain > li',
            ).length === 2,
        ),
      "the chain does not hold one point per close",
    );

    // Clean up: both pursuits ended, so the drop is not refused.
    await stage(trail, "archive it", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .verbs`, "archive"),
    );
    await pollUntil(
      trail,
      "it moves to archived",
      ROUND_TRIP_MS,
      async () => (await readDrawer()).archived.includes(SECOND_LINE),
      "archiving did not move the line",
    );
    await stage(trail, "select it there", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, SECOND_LINE),
    );
    await stage(trail, "discard it", DRIVER_MS, () =>
      press(`${DRAWER} .verbs button.danger`),
    );
    await stage(trail, "confirm", DRIVER_MS, () =>
      pressLabelled("body", "Discard Forever"),
    );
    // Three, and not the one entry left on the line: what a drop
    // releases is the union of every content the chain named and every
    // content the work named — `discard.rs` says so, and says why it is
    // one answer rather than two. This line named three assets across
    // two pursuits (two added, one put in by the replace), and the
    // entry removed at the end does not take its content out of that
    // union.
    let sawReleased = "";
    await pollUntil(
      trail,
      "it is gone and the notice says what it released",
      ROUND_TRIP_MS,
      async () => {
        const drawer = await readDrawer();
        sawReleased = drawer.released;
        return (
          !drawer.open.includes(SECOND_LINE) &&
          !drawer.archived.includes(SECOND_LINE) &&
          drawer.released.startsWith("Discarded. 3 assets released")
        );
      },
      () =>
        `the discard did not report the three assets this line named; the notice read ${JSON.stringify(sawReleased)}`,
    );

    await stage(trail, "close the drawer", DRIVER_MS, () =>
      press(`${DRAWER} .drawer-close`),
    );
  });
});
