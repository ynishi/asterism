// Working a team's line from the app: open a pursuit, push a round,
// close it, and see the line move — over a real `teams-server`.
//
// `teams-connect.spec.ts` next door walks the plane's three phases and
// its three tabs against an empty team. This one is about the team that
// has something on it, which is a different fixture and therefore a
// different spec: `onPrepare` seeds a second team with a line holding
// one entry, and everything below happens inside that line's own frame.
//
// # Why it needs a server rather than a mock
//
// `lib/stores/shared.test.ts` owns what the catalog does with the four
// verbs — that a round is not followed by a re-read of the contents,
// that a satisfied close is, that the fold shows a rename before it
// lands. All of it against mocked `api` and `mutate`, which is the hole
// this fills: those tests assert that the catalog called
// `"push_shared_round"` with a shape their own author wrote down twice.
// This is where the work verbs — `shared_line_pursuits`,
// `open_shared_pursuit`, `push_shared_round`, `close_shared_pursuit` —
// meet a server rather than a mock, and where `shared_line_states`
// answers for a line that actually holds something.
//
// # What it walks
//
// The line's frame beside the list it was picked from, its three tabs,
// and then the work: open a pursuit, rename the seeded entry, close as satisfied,
// and read the new name back off the contents tab. The rename is the
// operation under test because it is the one this plane can perform
// end to end — adding content is the promotion, which #198 leaves to
// its sibling.
//
// The last assertion is the one that matters most. A round that was
// never pushed, a close that was refused, or a contents read that
// answered from something kept would all leave `cut-01` on the line;
// only the whole chain working end to end renames it.
//
// # What it meets, and what it leaves
//
// The specs share one app process, so this one meets a window
// `teams-connect.spec.ts` has already used — the config says why the
// order is that way round and not the other. It connects if the window
// is not connected and takes over the session if it is, and it
// disconnects at the end, which is what keeps the arrangement from
// depending on which spec ran before it.
//
// It leaves the drawer open, which `teams-promote.spec.ts` meets and
// closes: an overlay's backdrop covers the sidebar, and a spec typing
// under one waits on a driver that retries rather than refuses.
//
// Of the team's own it leaves the rename, which is the point: the
// fixture's second team exists for this spec and goes with the
// database `onComplete` removes.
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 20_000;
const COLD_MS = 60_000;
const POLL_GAP_MS = 250;

const SHARED_ROW = 'aside.sidebar button[title^="A team\'s lines"]';
const DRAWER = '[role="dialog"][aria-label="Team"]';
/** The tabs under a team, and the tabs under a line, which are two
 *  strips. Both sit in a `.drawer-tabs` wrapper that positions a
 *  `TabStrip` (#217) — the wrapper no longer carries the row's own
 *  look, only its outer margin. Only the inner one carries
 *  `.line-tabs`, and the outer one is the first in the document, which
 *  is what `querySelector` answers with. */
const TEAM_TABS = `${DRAWER} .drawer-tabs`;
const LINE_TABS = `${DRAWER} .line-tabs`;

/** What this spec renames the seeded entry to. The name it starts
 *  with is the fixture's, and arrives as `entryName`. */
const RENAMED = "cut-02";

/** What `onPrepare` put up for this spec, or a failure saying it did not. */
function fixture(): {
  baseUrl: string;
  login: string;
  password: string;
  teamId: string;
  lineName: string;
  entryName: string;
} {
  const read = (name: string): string => {
    const value = process.env[name];
    if (!value) {
      throw new Error(
        `${name} is not set — \`onPrepare\` in wdio.teams.conf.ts is what ` +
          `provides it, so this spec was run through the wrong config.`,
      );
    }
    return value;
  };
  return {
    baseUrl: read("E2E_TEAMS_BASE_URL"),
    login: read("E2E_TEAMS_LOGIN"),
    password: read("E2E_TEAMS_PASSWORD"),
    teamId: read("E2E_TEAMS_WORK_ID"),
    lineName: read("E2E_TEAMS_WORK_LINE"),
    entryName: read("E2E_TEAMS_WORK_ENTRY"),
  };
}

/** Runs one step with a hard ceiling and records it on success. */
async function stage<T>(
  trail: string[],
  what: string,
  ms: number,
  run: () => Promise<T>,
): Promise<T> {
  const start = Date.now();
  try {
    const value = await Promise.race([
      run(),
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error(`timed out after ${ms} ms`)), ms),
      ),
    ]);
    trail.push(`${what} (${Date.now() - start} ms)`);
    return value;
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(
      `${what}: ${why}\n  passed already: ${trail.join(" -> ") || "(nothing)"}`,
    );
  }
}

/** Polls a probe until it answers true, reporting what it last saw. */
async function pollUntil(
  probe: () => Promise<boolean>,
  what: string,
  ms: number,
): Promise<void> {
  const deadline = Date.now() + ms;
  for (;;) {
    if (await probe()) return;
    if (Date.now() > deadline) {
      throw new Error(`${what} (polled for ${ms} ms)`);
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
  }
}

/** The drawer's visible text, or null when it is not mounted. */
function drawerText(): Promise<string | null> {
  return browser.execute((sel: string) => {
    const drawer = document.querySelector(sel);
    return drawer === null ? null : (drawer.textContent ?? "");
  }, DRAWER);
}

/** Clicks whatever the selector finds, and throws when it finds none. */
async function clickIn(selector: string): Promise<void> {
  const hit = await browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    if (el === null) return false;
    (el as HTMLElement).click();
    return true;
  }, selector);
  if (!hit) throw new Error(`nothing matched ${selector}`);
}

/**
 * Presses a control by the word on it, inside one container.
 *
 * By label rather than by position, for the reason
 * `teams-connect.spec.ts` gives at its own: position is a fact about
 * how many controls exist, so a fourth verb landing beside three would
 * move an assertion onto a different one with nothing failing to say
 * so. The container is a parameter here because this spec presses two
 * different tab strips and the rows' own verbs.
 */
async function clickLabelled(container: string, label: string): Promise<void> {
  const hit = await browser.execute(
    (sel: string, want: string) => {
      const scope = document.querySelector(sel);
      if (scope === null) return false;
      const button = Array.from(scope.querySelectorAll("button")).find(
        (candidate) => (candidate.textContent ?? "").trim() === want,
      );
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    container,
    label,
  );
  if (!hit) throw new Error(`nothing in ${container} reads "${label}"`);
}

/**
 * Presses the first control inside `container` whose text carries
 * `text`.
 *
 * Beside `clickLabelled` rather than instead of it, because a row is
 * not a label: the line rows below carry a name and a standing in two
 * spans, so their text is the two with the markup's whitespace between
 * them and an exact match never lands.
 */
async function clickCarrying(container: string, text: string): Promise<void> {
  const hit = await browser.execute(
    (sel: string, want: string) => {
      const scope = document.querySelector(sel);
      if (scope === null) return false;
      const button = Array.from(scope.querySelectorAll("button")).find(
        (candidate) => (candidate.textContent ?? "").includes(want),
      );
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    container,
    text,
  );
  if (!hit) throw new Error(`nothing in ${container} carries "${text}"`);
}

/** Types into a field the way a person does — see the sibling spec. */
async function fill(selector: string, value: string): Promise<void> {
  const field = await $(selector);
  await field.waitForExist({ timeout: DRIVER_MS });
  await field.setValue(value);
}

/** One frame, named for the step, on the path a person will read. */
async function snap(name: string): Promise<void> {
  const dir = process.env.E2E_TEAMS_SCREENS_DIR;
  if (!dir) return;
  try {
    fs.mkdirSync(dir, { recursive: true });
    await browser.saveScreenshot(path.join(dir, `${name}.png`));
  } catch {
    // Diagnostics must not cascade a failure.
  }
}

describe("work against a team's line", () => {
  it("opens a pursuit, pushes a round, and closes it onto the line", async () => {
    const trail: string[] = [];
    const { baseUrl, login, password, teamId, lineName, entryName } = fixture();

    await stage(trail, "the app paints its sidebar", COLD_MS, () =>
      pollUntil(
        () =>
          browser.execute(() => document.querySelector("aside.sidebar") !== null),
        "the app never painted its sidebar",
        COLD_MS,
      ),
    );

    await stage(trail, "open the shared-lines drawer", DRIVER_MS, async () => {
      await clickIn(SHARED_ROW);
      await pollUntil(
        async () => (await drawerText()) !== null,
        "the drawer never mounted",
        DRIVER_MS,
      );
    });

    // Connect, unless the window already is. A session outlives a spec
    // — it lives in the backend for as long as the window does — so
    // this asks what it found rather than assuming it found nothing.
    await stage(trail, "connect to the team server", ROUND_TRIP_MS, async () => {
      if (((await drawerText()) ?? "").includes("Signed in as")) return;
      await fill(`${DRAWER} form input[type="url"]`, baseUrl);
      await fill(`${DRAWER} form input[type="text"]`, login);
      await fill(`${DRAWER} form input[type="password"]`, password);
      await clickIn(`${DRAWER} form button[type="submit"]`);
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("Signed in as"),
        "the drawer never reported a session",
        ROUND_TRIP_MS,
      );
    });

    await stage(trail, "name the seeded team", ROUND_TRIP_MS, async () => {
      // By id, which sits behind a disclosure in the rail (#217): the
      // account was made for this run and is not a member of the
      // seeded team, so the list holds nothing to press.
      // Press the toggle only when the form is not already showing: the
      // panel outlives one spec, and a press against an open form
      // closes it.
      if (!(await $(`${DRAWER} .by-id`).isExisting())) {
        await clickIn(`${DRAWER} .by-id-toggle`);
      }
      await fill(`${DRAWER} .by-id input[type="text"]`, teamId);
      await clickIn(`${DRAWER} .by-id button[type="submit"]`);
      // Which tab is showing is inherited rather than chosen: the panel
      // is mounted for the window's lifetime, so it is still on
      // whichever tab the last spec left it on — `members`, as it
      // happens. Pressing `lines` is what makes the next assertion
      // about the lines rather than about a tab this spec never
      // visited.
      await clickLabelled(TEAM_TABS, "lines");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes(lineName),
        "the team's lines were never read",
        ROUND_TRIP_MS,
      );
    });
    await snap("10-work-team-read");

    // Selecting a line opens its frame in the body beside the list, so
    // the way back is what says the frame is open — and the publish
    // form, which the body holds when no line is, has to be gone with
    // it.
    await stage(trail, "open the line", ROUND_TRIP_MS, async () => {
      // `.lines` rather than `.drawer-list`: the drawer holds two
      // lists that share the styling since #202 put the teams to pick
      // from above these, and the looser selector finds the first.
      await clickCarrying(`${DRAWER} .drawer-list.lines`, lineName);
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("the team's lines"),
        "opening a line did not show its frame",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (text.includes("Publish a line of mine")) {
        throw new Error("the publish form followed the reader into a line");
      }
      // The contents tab is where a line opens, and the seeded entry is
      // what it holds.
      if (!text.includes(entryName)) {
        throw new Error(`the line did not show what it holds: ${text}`);
      }
    });
    await snap("11-line-open");

    await stage(trail, "the line's history says what landed", ROUND_TRIP_MS, async () => {
      await clickLabelled(LINE_TABS, "history");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("genesis · "),
        "the history never showed the line's beginning",
        ROUND_TRIP_MS,
      );
      // The point the fixture landed, opened. A row is phrased from the
      // axes it states rather than as a verb, which is what the model
      // stores — so this is the sentence a landing produces and not a
      // word the screen chose.
      await clickCarrying(`${DRAWER} .chain`, "row");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("existence → present"),
        "the change point never said what it moved",
        ROUND_TRIP_MS,
      );
    });
    await snap("12-line-history");

    await stage(trail, "no work is open against the line yet", DRIVER_MS, async () => {
      await clickLabelled(LINE_TABS, "work");
      await pollUntil(
        async () =>
          ((await drawerText()) ?? "").includes("No work open against this line."),
        "the work tab did not report an unworked line",
        DRIVER_MS,
      );
    });

    await stage(trail, "open a pursuit against it", ROUND_TRIP_MS, async () => {
      // The work tab's own form: title, then why. Both optional to the
      // command; filled here so the log says something a reader can
      // recognise.
      await fill(`${DRAWER} .new-work input[type="text"]`, "rename the cut");
      await clickLabelled(`${DRAWER} .new-work`, "Open");
      await pollUntil(
        async () =>
          ((await drawerText()) ?? "").includes("The line, as this would leave it"),
        "opening work did not land on the work itself",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (!text.includes("Nothing asked for yet.")) {
        throw new Error(`new work already carried rounds: ${text}`);
      }
    });
    await snap("13-work-open");

    // The round. A rename is pressed on the row, answered in the
    // prompt modal — which is mounted outside the drawer, so the fill
    // below is not scoped to it.
    await stage(trail, "rename the entry, which is one round", ROUND_TRIP_MS, async () => {
      await clickLabelled(`${DRAWER} .projected`, "rename");
      // `.prompt-input` rather than a `[role="dialog"] input`: the
      // drawer is a dialog too and comes first in the document, so the
      // looser selector fills the team field.
      await fill(".prompt-panel .prompt-input", RENAMED);
      await clickIn(".prompt-panel .prompt-btn.primary");
      await pollUntil(
        async () => {
          const text = (await drawerText()) ?? "";
          return text.includes("1 operation") && text.includes(RENAMED);
        },
        "the round never reached the log",
        ROUND_TRIP_MS,
      );
    });
    await snap("14-round-pushed");

    // Nothing has landed. The rows show what a close would leave, and
    // the contents tab still answers with what the line holds — which
    // is the distinction the whole model rests on.
    await stage(trail, "the line has not moved yet", ROUND_TRIP_MS, async () => {
      await clickLabelled(LINE_TABS, "on the line");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes(entryName),
        "the contents tab lost what the line holds",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (text.includes(RENAMED)) {
        throw new Error(
          "the contents tab showed a name no close had landed — a round is a " +
            "request, and nothing reaches the line until a satisfied close",
        );
      }
    });

    await stage(trail, "close it onto the line", ROUND_TRIP_MS, async () => {
      await clickLabelled(LINE_TABS, "work");
      await clickLabelled(`${DRAWER} .close`, "close · put it on the line");
      await pollUntil(
        async () =>
          ((await drawerText()) ?? "").includes(
            "what the work asked for is on the line",
          ),
        "the close was never reported",
        ROUND_TRIP_MS,
      );
    });
    await snap("15-closed");

    // The whole point, read back off the line rather than off the work:
    // the contents are re-read after a satisfied close, and what comes
    // back carries the new name.
    await stage(trail, "the line holds the new name", ROUND_TRIP_MS, async () => {
      await clickLabelled(LINE_TABS, "on the line");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes(RENAMED),
        "the landed rename never reached the contents tab",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (text.includes(entryName)) {
        throw new Error(
          `the line still holds the old name beside the new one: ${text}`,
        );
      }
    });
    await snap("16-landed");

    // And the way back, which is what makes replacing the list a
    // navigation rather than a trap.
    await stage(trail, "back to the team's lines", DRIVER_MS, async () => {
      await clickIn(`${DRAWER} .line-head .back`);
      await pollUntil(
        async () => {
          const text = (await drawerText()) ?? "";
          return text.includes("Publish a line of mine");
        },
        "the way back did not return to the list",
        DRIVER_MS,
      );
    });

    // Put the window back the way it was met. The session is the one
    // thing this spec leaves in it that a later one would meet, and
    // disconnecting is also the assertion that the view empties rather
    // than going stale.
    await stage(trail, "disconnect empties the view", ROUND_TRIP_MS, async () => {
      await clickIn(`${DRAWER} .drawer-session button`);
      await pollUntil(
        async () => !((await drawerText()) ?? "").includes("Signed in as"),
        "the drawer kept the session after disconnecting",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (text.includes(lineName)) {
        throw new Error(
          "the drawer kept answering for the team after the connection went",
        );
      }
    });
  });
});
