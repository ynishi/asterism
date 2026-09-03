// The roster's writes, driven in a window against a real
// `teams-server` (#210): invite somebody, move their role both ways,
// take them out, leave a team of somebody else's, and delete one of
// your own.
//
// `teams-connect.spec.ts` next door walks the plane's three phases and
// its three tabs and reads the roster; this one writes to it, which is
// a different fixture and therefore a different spec — the same split
// `forge-line.spec.ts` and `forge-pursuit.spec.ts` make on the local
// plane. Two of these verbs end the reader's relationship with a team,
// so a spec sharing a team with its neighbours could not drive them at
// all.
//
// # Why it needs a window rather than the store's tests
//
// `lib/stores/shared.test.ts` owns what the catalog does with these
// verbs, and its own describe blocks are the list. All of it against
// mocked `api` and `mutate`, which is the hole this fills: those tests
// assert that the catalog called `"invite_team_member"` with a shape
// their own author wrote down twice. Nothing there presses a control,
// so nothing there answers whether the controls exist, whether an
// owner is shown them, whether a confirmation stands between a person
// and a write that cannot be undone, or whether the form's id reaches
// a server that accepts it.
//
// # Which teams it uses, and why each
//
// **One it founds itself**, for the writes that need a roster the
// reader owns. Founding is the shortest way to be an owner, and the
// team goes with the database `onComplete` removes.
//
// **One the fixture's second account founded**, for leaving. The last
// owner cannot go by either verb, and founding a team makes you its
// only one — so a team to leave has to come from somewhere else, and
// `onPrepare` has the second account found it and invite this one.
//
// It touches neither team the other specs use.
import { browser, $ } from "@wdio/globals";
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
  otherId: string;
  leaveTeamId: string;
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
    otherId: read("E2E_TEAMS_OTHER_ID"),
    leaveTeamId: read("E2E_TEAMS_LEAVE_ID"),
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

/** Clicks whatever the selector finds, and throws where it finds none. */
async function clickIn(selector: string): Promise<void> {
  const hit = await browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    if (el === null) return false;
    (el as HTMLElement).click();
    return true;
  }, selector);
  if (!hit) throw new Error(`nothing matched ${selector}`);
}

/** Presses a tab by the word on it, for the reason its sibling gives. */
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

/** Presses the first control inside `container` carrying `text`. */
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
 * Presses a button on the row carrying a given text.
 *
 * `clickCarrying` takes the first button in a container, which is the
 * wrong tool on a list whose rows offer the same verb: an owner's
 * remove sits on every row but their own, so "the first one" would be
 * whichever row the server happened to list first.
 */
async function clickOnRow(
  container: string,
  rowText: string,
  buttonText: string,
): Promise<void> {
  const hit = await browser.execute(
    (sel: string, row: string, want: string) => {
      const found = Array.from(document.querySelectorAll(sel)).find(
        (candidate) => (candidate.textContent ?? "").includes(row),
      );
      if (found === undefined) return false;
      const button = Array.from(found.querySelectorAll("button")).find(
        (candidate) => (candidate.textContent ?? "").trim() === want,
      );
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    container,
    rowText,
    buttonText,
  );
  if (!hit) {
    throw new Error(`no "${buttonText}" on the row carrying "${rowText}"`);
  }
}

/** Reads a row's text, so a role can be asserted without the whole tab. */
function rowText(container: string, rowText: string): Promise<string | null> {
  return browser.execute(
    (sel: string, row: string) => {
      const found = Array.from(document.querySelectorAll(sel)).find(
        (candidate) => (candidate.textContent ?? "").includes(row),
      );
      return found === undefined ? null : (found.textContent ?? "");
    },
    container,
    rowText,
  );
}

/** Types into a field the way a person does, for `bind:value`'s sake. */
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

const ROSTER_ROW = `${DRAWER} .roster li`;

describe("the roster's writes", () => {
  it("lets somebody in, moves their role, and takes them out", async () => {
    const trail: string[] = [];
    const { baseUrl, login, password, otherId } = fixture();

    await stage(trail, "the app paints its sidebar", COLD_MS, () =>
      pollUntil(
        () =>
          browser.execute(
            () => document.querySelector("aside.sidebar") !== null,
          ),
        "the app never painted its sidebar",
        COLD_MS,
      ),
    );

    await stage(trail, "open the shared-lines drawer", DRIVER_MS, async () => {
      if ((await drawerText()) !== null) return;
      await clickIn(SHARED_ROW);
      await pollUntil(
        async () => (await drawerText()) !== null,
        "the drawer never mounted",
        DRIVER_MS,
      );
    });

    // Connect, unless the window already is. A session outlives a spec,
    // so this asks what it found rather than assuming it found nothing.
    await stage(
      trail,
      "connect to the team server",
      ROUND_TRIP_MS,
      async () => {
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
      },
    );

    // A team of its own, because all but the leave below is an owner's
    // and founding is the shortest way to be one.
    await stage(trail, "found a team to work on", ROUND_TRIP_MS, async () => {
      await clickIn(`${DRAWER} .make-team`);
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("Created team"),
        "founding a team never reported an id",
        ROUND_TRIP_MS,
      );
    });

    await stage(trail, "read its roster", ROUND_TRIP_MS, async () => {
      await clickTab("members");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("· you"),
        "the roster never showed the founder's own row",
        ROUND_TRIP_MS,
      );
    });
    await snap("roster-01-founded");

    // The controls are an owner's, and this reader is one. What a
    // member is shown is not asserted anywhere: the panel has no test
    // file, and the store's tests cannot answer it, holding no
    // controls. So this pins that they exist, and nothing pins their
    // absence.
    await stage(
      trail,
      "the owner is offered the writes",
      DRIVER_MS,
      async () => {
        const text = (await drawerText()) ?? "";
        for (const control of [
          "Let somebody in",
          "Invite",
          "leave",
          "Delete this team",
        ]) {
          if (!text.includes(control)) {
            throw new Error(`the roster tab offered no ${control}: ${text}`);
          }
        }
      },
    );

    // Against the rows rather than the drawer's whole text. Each write
    // reports itself into a status line inside the drawer that names
    // the account it acted on, so a probe reading the drawer can pass
    // on the report rather than on the roster. It bites hardest on the
    // removal, where the id the probe waits to lose is the one the
    // report has just gained: that probe never passes, which is how it
    // turned up. Here it would take a failed re-read to go wrong, the
    // store setting the line only after the read returns — a narrower
    // hazard, and the same fix.
    await stage(trail, "let somebody in", ROUND_TRIP_MS, async () => {
      await fill(`${DRAWER} .drawer-invite input[type="text"]`, otherId);
      await clickIn(`${DRAWER} .drawer-invite button[type="submit"]`);
      await pollUntil(
        async () => (await rowText(ROSTER_ROW, otherId)) !== null,
        "the invited account never reached the roster",
        ROUND_TRIP_MS,
      );
    });
    await snap("roster-02-invited");

    // Both ways, because the pair is one control that swaps: a row
    // showing promote after a demote is the assertion that the write
    // landed and the re-read saw it.
    await stage(trail, "make them an owner", ROUND_TRIP_MS, async () => {
      await clickOnRow(ROSTER_ROW, otherId, "promote");
      await pollUntil(
        async () =>
          ((await rowText(ROSTER_ROW, otherId)) ?? "").includes("owner"),
        "the promoted member never read back as an owner",
        ROUND_TRIP_MS,
      );
    });

    await stage(trail, "put them back to a member", ROUND_TRIP_MS, async () => {
      await clickOnRow(ROSTER_ROW, otherId, "demote");
      await pollUntil(
        async () => {
          const row = (await rowText(ROSTER_ROW, otherId)) ?? "";
          return row.includes("member") && !row.includes("owner");
        },
        "the demoted owner never read back as a member",
        ROUND_TRIP_MS,
      );
    });
    await snap("roster-03-role-moved");

    // Removing asks first. The modal focuses Cancel, so the
    // destructive answer is one deliberate move away — and a spec that
    // did not make that move would pass against a screen that never
    // removed anybody.
    await stage(trail, "take them out again", ROUND_TRIP_MS, async () => {
      await clickOnRow(ROSTER_ROW, otherId, "remove");
      await pollUntil(
        async () =>
          browser.execute(
            () => document.querySelector(".confirm-panel") !== null,
          ),
        "removing a member asked nothing",
        DRIVER_MS,
      );
      await clickCarrying(".confirm-panel", "Remove");
      await pollUntil(
        async () => (await rowText(ROSTER_ROW, otherId)) === null,
        "the removed account stayed on the roster",
        ROUND_TRIP_MS,
      );
    });
    await snap("roster-04-removed");
  });

  it("leaves a team of somebody else's, and deletes one of its own", async () => {
    const trail: string[] = [];
    const { leaveTeamId } = fixture();

    // The window is connected and on the team the first test founded,
    // which is the one to delete. Its roster holds one row — the
    // reader's own, the invited account having been removed — so the
    // control on that row is `leave` rather than `remove`.
    await stage(
      trail,
      "the reader's own row offers leaving",
      DRIVER_MS,
      async () => {
        await clickTab("members");
        await pollUntil(
          async () => ((await drawerText()) ?? "").includes("· you"),
          "the roster never showed the reader's own row",
          ROUND_TRIP_MS,
        );
        const text = (await drawerText()) ?? "";
        if (!text.includes("leave")) {
          throw new Error(`the reader's own row offered no leave: ${text}`);
        }
      },
    );

    // Delete first, while this team is the one named. It takes the
    // ledger with it, so it asks — the same confirmation removing a
    // member asks, and the same reason for pressing rather than
    // falling into the answer.
    await stage(
      trail,
      "delete the team it founded",
      ROUND_TRIP_MS,
      async () => {
        await clickIn(`${DRAWER} .delete-team`);
        await pollUntil(
          async () =>
            browser.execute(
              () => document.querySelector(".confirm-panel") !== null,
            ),
          "deleting a team asked nothing",
          DRIVER_MS,
        );
        await clickCarrying(".confirm-panel", "Delete");
        await pollUntil(
          async () => ((await drawerText()) ?? "").includes("Deleted team"),
          "the deletion never reported back",
          ROUND_TRIP_MS,
        );
      },
    );
    await snap("roster-05-deleted");

    // And the team it did not found, which is the one it can leave:
    // the last owner cannot go, and founding makes you the only one.
    await stage(
      trail,
      "name the team it was invited to",
      ROUND_TRIP_MS,
      async () => {
        // By id, behind its disclosure in the rail (#217).
        if (!(await $(`${DRAWER} .by-id`).isExisting())) {
          await clickIn(`${DRAWER} .by-id-toggle`);
        }
        await fill(`${DRAWER} .by-id input[type="text"]`, leaveTeamId);
        await clickIn(`${DRAWER} .by-id button[type="submit"]`);
        await clickTab("members");
        await pollUntil(
          async () => ((await drawerText()) ?? "").includes("· you"),
          "the invited team's roster never showed the reader",
          ROUND_TRIP_MS,
        );
      },
    );

    // A member rather than an owner here, so the row offers leaving
    // and nothing else — no step down, because there is nothing to
    // step down from.
    await stage(trail, "leave it", ROUND_TRIP_MS, async () => {
      const own = (await rowText(ROSTER_ROW, "· you")) ?? "";
      if (own.includes("demote")) {
        throw new Error(`a member's own row offered a step down: ${own}`);
      }
      await clickOnRow(ROSTER_ROW, "· you", "leave");
      await pollUntil(
        async () =>
          browser.execute(
            () => document.querySelector(".confirm-panel") !== null,
          ),
        "leaving asked nothing",
        DRIVER_MS,
      );
      await clickCarrying(".confirm-panel", "Leave");
      await pollUntil(
        async () => ((await drawerText()) ?? "").includes("You have left"),
        "leaving never reported back",
        ROUND_TRIP_MS,
      );
    });
    await snap("roster-06-left");

    // The drawer closes behind this spec, because an overlay left open
    // is what the others hand each other.
    // Through its own control rather than the sidebar row that opened
    // it — the drawer is an overlay with a backdrop over everything,
    // which is what `teams-promote.spec.ts` says where it does the
    // same.
    await stage(trail, "close the drawer", DRIVER_MS, async () => {
      await clickIn(`${DRAWER} .drawer-close`);
      await pollUntil(
        async () => (await drawerText()) === null,
        "the drawer never closed",
        DRIVER_MS,
      );
    });
  });
});
