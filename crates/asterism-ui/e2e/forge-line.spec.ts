// A line's whole lifecycle, through the real backend, in the webview.
//
// `lib/stores/forge.test.ts` already owns most of what this panel does:
// which reads a write invalidates, that closing drops the selection and
// everything it produced, that an entry off the line is not offered as
// contents. All of it runs against mocked `api` and `mutate`.
//
// That mocking is exactly the hole. A unit test asserts the catalog
// called `"rename_forge_line"` with `{lineId, command: {line_id, name}}`
// — a shape its own author wrote down twice, once in the store and once
// in the assertion. If the Tauri command takes a different argument
// name, if a command does not exist under that name at all, or if the
// DTO it answers with is not the shape `bindings.ts` claims, every test
// in the package stays green and the panel is dead on arrival. Seven
// commands reached that way in #180, none of them exercised anywhere
// before this spec.
//
// # Why the whole lifecycle in one `it`
//
// Because the verbs are ordered by the model rather than by taste. A
// discard is reachable only from an archived line (`Line`'s standing),
// so "archive then discard" is not two independent provocations that
// happen to run in sequence — the second cannot be provoked without the
// first. Splitting them would mean seeding a line in some other way for
// the second half, which is the thing this spec exists not to do.
//
// # Why this leaves the fixture as it found it
//
// The line is created through the panel and discarded through the
// panel, so the profile carries one more line only while the spec runs.
// That matters here more than it usually would: a discard is
// destructive by design, and a spec that provoked it against seeded
// fixture would be usable exactly once. Creating what it destroys is
// what makes it repeatable.
//
// It also means the assets stay put. A discard releases what a line
// held back to the library; this line holds nothing, so the count it
// reports is zero and no asset moves. `forge-pursuit.spec.ts` is where
// that count is checked against real content, because putting content
// on a line is what a pursuit is for.
import { browser } from "@wdio/globals";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 20_000;
const COLD_MS = 60_000;
const POLL_GAP_MS = 250;

/// A name nothing else in the fixture answers to, and unique because `clickLineNamed` clicks the first button
/// reading it and `Name` carries no claim of uniqueness — the model
/// says so. A fixed name meant this spec selected, renamed, archived
/// and discarded whichever line happened to be first, which stopped
/// being its own the moment a run left one behind. The fixture held
/// five, all called `e2e-forge-line`, put there by the rename this
/// spec was not performing (see "answer the prompt"). With a name
/// nothing else answers to, the discard at the end takes what this run
/// made and the pile stops growing.
const RUN = Date.now();
const LINE_NAME = `e2e-forge-line-${RUN}`;
const RENAMED = `e2e-forge-line-${RUN} renamed`;

const FORGE_ROW = 'aside.sidebar button[title^="Lines on this machine"]';
const DRAWER = '[role="dialog"][aria-label="Forge"]';

/**
 * Runs one step with a hard ceiling and records it on success. Same
 * shape `refusal.spec.ts` uses, and for its reason: a raw driver call
 * carries no timeout, so the bound has to be a race, and on failure the
 * error names the step plus what already passed.
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

interface PanelSnapshot {
  drawerPresent: boolean;
  /** Line names in the open section, in order. */
  open: string[];
  /** Line names under Archived. */
  archived: string[];
  /** The selected line's header, if one is selected. */
  heading: string;
  standing: string;
  /** The discard notice, if it is showing. */
  released: string;
}

/** One `execute` for the whole panel: every read this spec makes is a
 *  field of this, so a step costs one driver call rather than one per
 *  question. */
function readPanel(): Promise<PanelSnapshot> {
  return browser.execute(() => {
    const drawer = document.querySelector('[role="dialog"][aria-label="Forge"]');
    if (drawer === null) {
      return {
        drawerPresent: false,
        open: [],
        archived: [],
        heading: "",
        standing: "",
        released: "",
      };
    }
    const nav = drawer.querySelector('nav[aria-label="Lines"]');
    const lists = nav === null ? [] : Array.from(nav.querySelectorAll("ul"));
    const namesIn = (ul: Element | undefined) =>
      ul === undefined
        ? []
        : Array.from(ul.querySelectorAll("button")).map(
            (b) => b.textContent?.trim() ?? "",
          );
    const detail = drawer.querySelector(".line");
    return {
      drawerPresent: true,
      open: namesIn(lists[0]),
      archived: namesIn(lists[1]),
      heading: detail?.querySelector("h3")?.textContent?.trim() ?? "",
      standing:
        detail?.querySelector("header .quiet")?.textContent?.trim() ?? "",
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

/** Clicks the line button whose label is exactly this. */
function clickLineNamed(name: string): Promise<boolean> {
  return browser.execute((wanted: string) => {
    const nav = document.querySelector(
      '[role="dialog"][aria-label="Forge"] nav[aria-label="Lines"]',
    );
    if (nav === null) return false;
    const button = Array.from(nav.querySelectorAll("button")).filter(
      (b) => (b.textContent?.trim() ?? "") === wanted,
    )[0];
    if (button === undefined) return false;
    (button as HTMLElement).click();
    return true;
  }, name);
}

describe("a line's lifecycle", () => {
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
  });

  it("opens, renames, archives, reopens and discards through the real backend", async () => {
    const trail: string[] = [];

    await stage(trail, "open the forge", DRIVER_MS, () => clickIn(FORGE_ROW));
    await pollUntil(
      trail,
      "the drawer paints",
      ROUND_TRIP_MS,
      async () => (await readPanel()).drawerPresent,
      "the forge drawer never appeared",
    );

    // Create. The rule comes from `list_forge_strategies`, so choosing
    // the first option also proves that read answered with something.
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
        // Svelte binds on `input` / `change`, so setting `.value`
        // alone would leave the component's state on the old one.
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        select.value = rule.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        return true;
      }, LINE_NAME),
    );
    await stage(trail, "open the line", DRIVER_MS, () =>
      clickIn('[role="dialog"][aria-label="Forge"] form.new-line button'),
    );
    await pollUntil(
      trail,
      "the line reaches the list",
      ROUND_TRIP_MS,
      async () => (await readPanel()).open.includes(LINE_NAME),
      "the opened line never appeared in the list",
    );

    // Select it, which reads its contents — a second command, and the
    // one whose answer the two deriveds are built on.
    await stage(trail, "select the line", DRIVER_MS, () =>
      clickLineNamed(LINE_NAME),
    );
    await pollUntil(
      trail,
      "the line's own panel paints",
      ROUND_TRIP_MS,
      async () => (await readPanel()).heading === LINE_NAME,
      "selecting the line did not put it in the header",
    );

    // Rename. The prompt is the App's, mounted once and reached from
    // here through the store — so this proves the panel's use of it as
    // well as the command.
    await stage(trail, "start the rename", DRIVER_MS, () =>
      clickIn('[role="dialog"][aria-label="Forge"] .verbs button'),
    );
    // The prompt by its own class, and committed through its own OK.
    //
    // This asked for `.prompt-modal input, [role="dialog"] input[type="text"]`
    // and then submitted `input.closest("form")`. Neither half found the
    // prompt: the class is `prompt-panel`, and the drawer is itself a
    // `[role="dialog"]` mounted before it — so the match was the
    // *new-line* Name field, and the form submitted was the new-line
    // form. Every run of this spec opened a rename it never answered
    // and created a second line called "…renamed" instead, which is
    // why the fixture carried five of them and a prompt nobody could
    // press past. The poll below passed on that new line's name.
    //
    // There is no form to submit either: `PromptModal` commits from a
    // button and from Enter, so this presses the button.
    await stage(trail, "answer the prompt", DRIVER_MS, () =>
      browser.execute((name: string) => {
        const input = document.querySelector(
          ".prompt-panel input.prompt-input",
        ) as HTMLInputElement | null;
        if (input === null) return false;
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        const ok = document.querySelector(
          ".prompt-panel .prompt-btn.primary",
        ) as HTMLElement | null;
        if (ok === null) return false;
        ok.click();
        return true;
      }, RENAMED),
    );
    await pollUntil(
      trail,
      "the new name reaches the list",
      ROUND_TRIP_MS,
      async () => (await readPanel()).open.includes(RENAMED),
      "the rename did not reach the list",
    );

    // Archive, and the standing moves. Nothing lands on an archived
    // line, and it is the only standing a discard can be reached from.
    await stage(trail, "select the renamed line", DRIVER_MS, () =>
      clickLineNamed(RENAMED),
    );
    await stage(trail, "archive it", DRIVER_MS, () =>
      browser.execute(() => {
        const verbs = document.querySelectorAll(
          '[role="dialog"][aria-label="Forge"] .verbs button',
        );
        const archive = Array.from(verbs).filter(
          (b) => b.textContent?.trim() === "archive",
        )[0];
        if (archive === undefined) return false;
        (archive as HTMLElement).click();
        return true;
      }),
    );
    await pollUntil(
      trail,
      "the line moves to archived",
      ROUND_TRIP_MS,
      async () => (await readPanel()).archived.includes(RENAMED),
      "archiving did not move the line into the archived section",
    );

    // Reopen, and back it comes. The pair is worth asserting because
    // the two are one toggle in the model and two commands on the wire.
    await stage(trail, "select it again", DRIVER_MS, () =>
      clickLineNamed(RENAMED),
    );
    await stage(trail, "reopen it", DRIVER_MS, () =>
      browser.execute(() => {
        const verbs = document.querySelectorAll(
          '[role="dialog"][aria-label="Forge"] .verbs button',
        );
        const reopen = Array.from(verbs).filter(
          (b) => b.textContent?.trim() === "reopen",
        )[0];
        if (reopen === undefined) return false;
        (reopen as HTMLElement).click();
        return true;
      }),
    );
    await pollUntil(
      trail,
      "the line comes back to open",
      ROUND_TRIP_MS,
      async () => (await readPanel()).open.includes(RENAMED),
      "reopening did not move the line back",
    );

    // Discard, which needs the archived standing again and the confirm
    // modal after it. The notice it leaves is the only place the
    // released assets are ever named.
    await stage(trail, "select it once more", DRIVER_MS, () =>
      clickLineNamed(RENAMED),
    );
    await stage(trail, "archive before discarding", DRIVER_MS, () =>
      browser.execute(() => {
        const verbs = document.querySelectorAll(
          '[role="dialog"][aria-label="Forge"] .verbs button',
        );
        const archive = Array.from(verbs).filter(
          (b) => b.textContent?.trim() === "archive",
        )[0];
        if (archive === undefined) return false;
        (archive as HTMLElement).click();
        return true;
      }),
    );
    await pollUntil(
      trail,
      "archived again",
      ROUND_TRIP_MS,
      async () => (await readPanel()).archived.includes(RENAMED),
      "the line did not archive before the discard",
    );
    await stage(trail, "select the archived line", DRIVER_MS, () =>
      clickLineNamed(RENAMED),
    );
    await stage(trail, "press discard", DRIVER_MS, () =>
      browser.execute(() => {
        const discard = document.querySelector(
          '[role="dialog"][aria-label="Forge"] .verbs button.danger',
        );
        if (discard === null) return false;
        (discard as HTMLElement).click();
        return true;
      }),
    );
    await stage(trail, "confirm the discard", DRIVER_MS, () =>
      browser.execute(() => {
        const buttons = document.querySelectorAll("button");
        const confirm = Array.from(buttons).filter(
          (b) => b.textContent?.trim() === "Discard Forever",
        )[0];
        if (confirm === undefined) return false;
        (confirm as HTMLElement).click();
        return true;
      }),
    );

    await pollUntil(
      trail,
      "the line is gone and the notice says so",
      ROUND_TRIP_MS,
      async () => {
        const panel = await readPanel();
        return (
          !panel.open.includes(RENAMED) &&
          !panel.archived.includes(RENAMED) &&
          panel.released.startsWith("Discarded.")
        );
      },
      "the discard did not remove the line, or left no notice",
    );

    // Closing ends the question: the notice answers about a line that
    // no longer exists, so it does not survive into the next open.
    await stage(trail, "close the drawer", DRIVER_MS, () =>
      clickIn(`${DRAWER} .drawer-close`),
    );
    await stage(trail, "reopen the forge", DRIVER_MS, () => clickIn(FORGE_ROW));
    await pollUntil(
      trail,
      "the notice did not survive the close",
      ROUND_TRIP_MS,
      async () => {
        const panel = await readPanel();
        return panel.drawerPresent && panel.released === "";
      },
      "the discard notice was still on screen after closing and reopening",
    );

    await stage(trail, "close it again", DRIVER_MS, () =>
      clickIn(`${DRAWER} .drawer-close`),
    );
  });
});
