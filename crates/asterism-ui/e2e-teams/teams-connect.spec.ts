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
// Five commands are reached here — `team_server_session`,
// `connect_team_server`, `list_shared_lines`, `team_ledger_page` and,
// through the disconnect at the end, `disconnect_team_server` — none of
// them exercised anywhere before. That is the whole reason this suite
// needs a server rather than a mock. The reads behind a selected line,
// and both writes, wait for a fixture that has something on it.
//
// # What it walks
//
// The three phases, in the order a person meets them: nobody to ask,
// somebody to ask and no team named, and a team whose lines are read.
// The fixture's team is empty, so the third arrives at "This team hosts
// no lines." — which is the assertion that matters most here, because
// it is the sentence that used to appear in the second phase too.
//
// Then across the tabs. The ledger is the one read on this plane whose
// answer cannot legitimately be empty: founding a team appends its own
// event, so an empty ledger is a server misbehaving rather than a team
// nothing has happened to. And its foot is checked for the word it
// must not say — with one event and a page size well above it there is
// no cursor, which is exactly the branch that has to offer to ask
// again rather than announce an end.
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
 * Presses a tab by the word on it.
 *
 * By label rather than by position, because position is a fact about
 * how many tabs exist: this spec named them `:first-child` and
 * `:last-child` while there were two, and the roster landing between
 * them would have moved one of those onto a different surface with
 * nothing failing to say so.
 */
async function clickTab(label: string): Promise<void> {
  const hit = await browser.execute(
    (sel: string, want: string) => {
      const nav = document.querySelector(sel);
      if (nav === null) return false;
      const button = Array.from(nav.querySelectorAll("button")).find(
        (candidate) => (candidate.textContent ?? "").trim() === want,
      );
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    `${DRAWER} .drawer-tabs`,
    label,
  );
  if (!hit) throw new Error(`no tab reads "${label}"`);
}

/**
 * Presses the first control inside `container` carrying `text`.
 *
 * For rows rather than labels: a team row is an id and a role in two
 * spans, so its text is the two with the markup's whitespace between
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
      if (!text.includes("Pick a team above")) {
        throw new Error(`the drawer did not ask for a team; it read: ${text}`);
      }
      if (text.includes("Publish a line of mine")) {
        throw new Error("the drawer offered to publish to no team");
      }
      // The way out of having no team, offered wherever there is a
      // connection rather than only here.
      if (!text.includes("Start a team of your own")) {
        throw new Error("no way out of having no team");
      }
    });

    // Picked rather than typed, which is what a person does now that
    // something answers "the teams I am in" (#202). The account was
    // made for this run, so every team in the list is one the fixture
    // or this spec founded — nothing else could be in it. Typing an id
    // still works, and the specs that name a team this window has not
    // been on still do it that way.
    await stage(trail, "pick the team and read its lines", ROUND_TRIP_MS, async () => {
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes(teamId),
        "the account's own teams were never listed",
        ROUND_TRIP_MS,
      );
      await clickCarrying(`${DRAWER} .teams`, teamId);
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

    // The roster. The fixture's team was founded by the account this
    // window is signed in as, so it holds exactly one row and that row
    // is the reader's — which is the case the "you" marking exists for.
    await stage(trail, "read the team's roster", ROUND_TRIP_MS, async () => {
      await clickTab("members");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("owner"),
        "the roster never showed a member",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (!text.includes("· you")) {
        throw new Error(`the roster did not mark the reader's own row: ${text}`);
      }
      // Why ids and not names, said where a reader would compare this
      // tab with the ledger and wonder.
      if (!text.includes("carries no name")) {
        throw new Error("the roster did not say why it shows ids");
      }
      if (text.includes("Publish a line of mine")) {
        throw new Error("the publish form followed the reader onto the roster");
      }
    });
    await snap("04-roster");

    // The ledger. Neither read on this plane can legitimately answer
    // with nothing — a team is created with a founding owner and with
    // the event recording that — so an empty answer from either is a
    // server misbehaving rather than a team nothing has happened to.
    await stage(trail, "read the team's ledger", ROUND_TRIP_MS, async () => {
      await clickTab("ledger");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("teams.team.created"),
        "the ledger never showed the event that founded the team",
        ROUND_TRIP_MS,
      );
      const text = (await drawerText()) ?? "";
      if (text.includes("came back empty")) {
        throw new Error("the ledger reported itself empty");
      }
      // The foot never claims an end. With one event and a page size
      // well above it the cursor is null, which is the branch that has
      // to say "ask again" rather than "that was everything".
      if (!text.includes("Ask again")) {
        throw new Error(`the ledger's foot did not offer to ask again: ${text}`);
      }
      // Publishing belongs to a line, so it is not on this tab.
      if (text.includes("Publish a line of mine")) {
        throw new Error("the publish form followed the reader onto the ledger");
      }
    });
    await snap("05-ledger");

    await stage(trail, "an event says what it carries", DRIVER_MS, async () => {
      await clickIn(`${DRAWER} .ledger .event-payload-toggle`);
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("hide"),
        "the payload never opened",
        DRIVER_MS,
      );
    });
    await snap("06-payload");

    await stage(trail, "the lines tab is still there", DRIVER_MS, async () => {
      await clickTab("lines");
      await pollUntil(
        async () =>
          ((await drawerText()) ?? "").includes("This team hosts no lines."),
        "going back to the lines tab did not show the lines",
        DRIVER_MS,
      );
    });

    // Founding one, which is the only write on this walk. Pressed
    // rather than merely asserted present: a control nobody presses is
    // a control nobody has checked. It lands the reader on the team it
    // just made — a roster of one, and no lines.
    //
    // Pressed from the members tab on purpose. Founding drops what the
    // on-demand tabs held, because what they held was about the team
    // named before; a tab that did not ask again would sit on its
    // unread state under a tab the reader never left.
    await stage(trail, "found a team and land on it", ROUND_TRIP_MS, async () => {
      await clickTab("members");
      await clickIn(`${DRAWER} .make-team`);
      // Which team this window is on is the marked row in the picker,
      // not whether an id appears in the drawer: since #202 the drawer
      // lists every team the account is in, so the old id is on screen
      // whether or not it is the one being read.
      //
      // Polled rather than checked once, and the two conditions are
      // one wait rather than two: `createTeam` says "Created team"
      // before it re-reads the list, and `makeTeam` names the new team
      // only after that returns — so the sentence arrives while the
      // marked row is still the previous team's, and a check that ran
      // there would fail saying the reader was left behind when they
      // were about to be moved.
      const markedTeam = async (): Promise<string | null> =>
        browser.execute((sel: string) => {
          const row = document.querySelector(sel);
          return row === null ? null : (row.textContent ?? "");
        }, `${DRAWER} .teams .drawer-row.active`);
      await pollUntil(
        async () => {
          if (!((await drawerText()) ?? "").includes("Created team ")) {
            return false;
          }
          const on = await markedTeam();
          return on !== null && !on.includes(teamId);
        },
        "founding a team did not land the reader on the team it made",
        ROUND_TRIP_MS,
      );
      const after = (await drawerText()) ?? "";
      if (after.includes("Nothing read yet")) {
        throw new Error(
          "the members tab kept its unread state after the team under it changed",
        );
      }
    });
    await snap("08-founded");

    // Still on the members tab, which never moved: the roster showing
    // now is the new team's, read because founding one dropped the
    // last.
    //
    // "· you" alone, not "owner · you": the markup puts a non-breaking
    // space before it, so the joined form never matches what
    // `textContent` returns.
    await stage(trail, "the new team holds only its founder", ROUND_TRIP_MS, async () => {
      await pollUntil(
        async () => {
          const text = (await drawerText()) ?? "";
          return text.includes("owner") && text.includes("· you");
        },
        "the founder is not in the team they founded",
        ROUND_TRIP_MS,
      );
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
    await snap("09-disconnected");
  });
});
