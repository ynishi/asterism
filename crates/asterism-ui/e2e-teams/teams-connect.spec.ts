// The team plane, from the connect form to a team's lines, through a
// real `teams-server`.
//
// `lib/stores/shared.test.ts` owns what the catalog does — that a
// served-through view reads on open, that disconnecting empties it,
// that an entry off the line is not offered as contents, and which of
// the three phases the frame is in. All of it against mocked `api` and
// `mutate`, which is the hole this fills: those tests assert that the
// catalog called `"list_shared_lines"` with `{teamIdRaw}`, a shape
// their own author wrote down twice. If the command takes a different
// argument name, does not exist under that name, or answers with a
// shape `bindings.ts` does not claim, every one of them stays green and
// the panel is dead on arrival.
//
// Four commands are reached here — `team_server_session`,
// `connect_team_server`, `list_shared_lines` and, through the
// disconnect at the end, `disconnect_team_server` — none of them
// exercised anywhere before. That is the whole reason this suite needs
// a server rather than a mock. The reads behind a selected line, and
// both writes, wait for a fixture that has something on it.
//
// # What it walks
//
// The three phases, in the order a person meets them: nobody to ask,
// somebody to ask and no team named, and a team whose lines are read.
// The fixture's team is empty, so the third ends on "This team hosts no
// lines." — which is the assertion that matters most here, because it
// is the sentence that used to appear in the second phase too.
//
// # What it leaves of the team's
//
// Nothing. The account and the team live in a database `onPrepare`
// creates and `onComplete` removes. What does survive a run is the
// app's own profile directory — opening a home creates it and stamps
// its marker before any store is touched — and the retained
// screenshots. Neither holds anything this spec put there: connecting
// is the only write it performs, and that one lives in the window.
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 20_000;
const COLD_MS = 60_000;
const POLL_GAP_MS = 250;

const SHARED_ROW = 'aside.sidebar button[title^="Lines a team hosts"]';
const DRAWER = '[role="dialog"][aria-label="Shared lines"]';

/** What `onPrepare` put up, or a failure that says it did not. */
function fixture(): {
  baseUrl: string;
  login: string;
  password: string;
  teamId: string;
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
    teamId: read("E2E_TEAMS_ID"),
  };
}

/**
 * Runs one step with a hard ceiling and records it on success. Same
 * shape `forge-line.spec.ts` uses, and for its reason: a raw driver
 * call carries no timeout, so the bound has to be a race, and on
 * failure the error names the step plus what already passed.
 */
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

/**
 * Clicks whatever the selector finds, and says whether it found one.
 *
 * It throws where a caller ignores the answer, because a press against
 * nothing recorded as a pass is what `forge-line.spec.ts` found in its
 * own helper: every step after it asserts about a screen that never
 * changed.
 */
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
 * Types into a field the way a person does.
 *
 * Not `element.value = …`: the panel binds with Svelte's `bind:value`,
 * which listens for the input event a real keystroke raises and never
 * hears an assignment. A field set the other way looks filled in a
 * screenshot and submits empty.
 */
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

describe("the team plane", () => {
  it("connects, names a team, and reads what it hosts", async () => {
    const trail: string[] = [];
    const { baseUrl, login, password, teamId } = fixture();

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
    await snap("01-drawer-open");

    // Phase one: there is nobody to ask, so the drawer is a connection
    // form and nothing else. An empty list here would be a claim about
    // a team this window has never spoken to.
    await stage(trail, "the drawer asks for a connection", DRIVER_MS, async () => {
      const text = (await drawerText()) ?? "";
      if (text.includes("This team hosts no lines.")) {
        throw new Error(
          "the drawer reported an empty team before connecting to one",
        );
      }
      await $(`${DRAWER} form input[type="url"]`).waitForExist({
        timeout: DRIVER_MS,
      });
    });

    await stage(trail, "connect to the team server", ROUND_TRIP_MS, async () => {
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
    await snap("02-connected");

    // Phase two: somebody to ask, nothing named to ask about. The
    // sentence under test is the one that must *not* be here.
    await stage(trail, "connected with no team named", DRIVER_MS, async () => {
      const text = (await drawerText()) ?? "";
      if (text.includes("This team hosts no lines.")) {
        throw new Error(
          "the drawer answered for a team before one was named — the two " +
            "kinds of empty are merged again (#190)",
        );
      }
      if (!text.includes("Name a team above")) {
        throw new Error(`the drawer did not ask for a team; it read: ${text}`);
      }
      if (text.includes("Publish a line of mine")) {
        throw new Error("the drawer offered to publish to no team");
      }
    });

    await stage(trail, "name the team and read its lines", ROUND_TRIP_MS, async () => {
      await fill(`${DRAWER} form input[type="text"]`, teamId);
      await clickIn(`${DRAWER} form button[type="submit"]`);
      await pollUntil(
        async () =>
          ((await drawerText()) ?? "").includes("This team hosts no lines."),
        "the team's lines were never read",
        ROUND_TRIP_MS,
      );
    });
    await snap("03-team-read");

    // The fixture's team is empty, so this is the honest answer — and
    // now it is the answer to a question that was actually asked.
    await stage(trail, "the empty team is offered publishing", DRIVER_MS, async () => {
      const text = (await drawerText()) ?? "";
      if (!text.includes("Publish a line of mine")) {
        throw new Error("a named team did not offer publishing");
      }
    });

    await stage(trail, "disconnect empties the view", ROUND_TRIP_MS, async () => {
      await clickIn(`${DRAWER} .drawer-session button`);
      await pollUntil(
        async () => {
          const text = (await drawerText()) ?? "";
          return !text.includes("Signed in as");
        },
        "the drawer kept the session after disconnecting",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (text.includes("This team hosts no lines.")) {
        throw new Error(
          "the drawer kept answering for the team after the connection went",
        );
      }
    });
    await snap("04-disconnected");
  });
});
