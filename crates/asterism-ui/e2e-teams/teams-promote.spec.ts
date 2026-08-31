// Handing an asset to a team, from the pane that is showing it —
// through a real `teams-server`, over the wire the client speaks.
//
// `lib/stores/shared.test.ts` owns what the catalog does with the verb:
// that it names the team, the line and the work it holds, that the work
// is re-read afterwards and the contents are not, that a repeat says
// nothing was sent. All of it against mocked `api` and `mutate`, which
// is the hole this fills — those tests assert that the catalog called
// `"promote_asset_to_team"` with a shape their own author wrote down
// twice. This is where the command exists, the content verb takes the
// bytes, and the round the promotion pushes reaches a line somebody
// else could read.
//
// # What it needs that the other two specs do not
//
// An asset. The teams profile is its own `ASTERISM_HOME` and carries
// no content of its own — it persists between runs, but nothing seeds
// it the way the e2e suite's profile is seeded, so a run that needed
// an asset and found one was looking at what an earlier run of this
// spec left. This seeds over the app's own loopback HTTP surface —
// `POST /asterism/assets/add`, the command the importers use — exactly
// as `e2e/metric-sort.spec.ts` seeds the rows it measures. Seeding is
// idempotent: the persona and the asset are found by their natural key
// and their cover text, and only what is missing is created.
//
// **A promotion of it is new on every run even so.** `onPrepare` makes
// a fresh team per run, and a promotion is keyed on the team, the line
// and the asset — so the repeat path the store tests cover is not
// reachable from here, and the assertion that bytes were sent holds on
// the second run as on the first.
//
// # What it meets, and what it leaves
//
// It runs last, so it meets a window the other two have used. What
// mattered was not the session — each spec disconnects — but the
// shared-lines drawer, which `teams-work.spec.ts` leaves open: its
// backdrop covers everything, and a spec that types into the sidebar
// under it waits for a driver that retries rather than refuses. So
// this one closes the drawer before it touches anything, and
// disconnects at the end.
//
// # What it walks
//
// Open work on the team's line, then leave the drawer for the grid:
// promoting is an act about an asset, and the pane showing that asset
// is where the verb sits (#171). What it checks at the end is that the
// round reached the work — read back through the drawer, not from what
// the write answered, because the answer a screen shows and the answer
// a server holds are the two things this suite exists to tell apart.
//
// The pane's disclosure is asserted before the press, because #148
// decision 4 is a rule about what a person is told *before* handing
// something over, and a sentence that drifted out of the screen would
// leave the promotion no less correct and the person no less
// uninformed.
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 20_000;
const COLD_MS = 60_000;
const POLL_GAP_MS = 250;
/** A page read plus the seed that has to be in it. Wider than a
 *  round trip because it is two: the search reload, and the thumbnail
 *  work the grid does behind it. */
const GRID_RELOAD_MS = 30_000;

const SHARED_ROW = 'aside.sidebar button[title^="Lines a team hosts"]';
const DRAWER = '[role="dialog"][aria-label="Shared lines"]';
const TEAM_TABS = `${DRAWER} .drawer-tabs`;
const LINE_TABS = `${DRAWER} .line-tabs`;
const PROMOTE = ".detail-panel .promote";

/** The fixture's own persona and asset, found by these rather than by
 *  position — a profile that already carries them is reused. */
const PACK_ID = "e2e-promote";
const PERSONA_NAME = "e2e promote";
const COVER = "e2e-promote-fixture";

/** What the entry is called on the team's line. */
const NAMED = "handed-over";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "../../..");

interface PersonaDto {
  id: string;
  pack_id: string | null;
  name: string;
}

interface CardDto {
  id: string;
  cover: string | null;
  /** Where the material is. Read because a row outlives the path it
   *  remembers — see `ensureAsset`. */
  source_locator: string;
}

interface PageDto {
  items: CardDto[];
}

/** What `onPrepare` put up, or a failure that says it did not. */
function fixture(): {
  baseUrl: string;
  login: string;
  password: string;
  teamId: string;
  lineName: string;
  appUrl: string;
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
    appUrl: read("E2E_TEAMS_APP_URL"),
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

/** The visible text under a selector, or null when it is not mounted. */
function textOf(selector: string): Promise<string | null> {
  return browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    return el === null ? null : (el.textContent ?? "");
  }, selector);
}

/**
 * The same text with every run of whitespace squashed to one space.
 *
 * For asserting on prose. `textContent` carries the markup's own line
 * breaks and indentation, so a sentence the source wraps arrives with a
 * newline inside it and a plain `includes` of the sentence never
 * matches — which is the same trap `teams-connect.spec.ts` records for
 * `&nbsp;`, met from the other side. Squashing means the assertion is
 * about the sentence rather than about where the file happened to wrap
 * it.
 */
async function proseOf(selector: string): Promise<string> {
  return ((await textOf(selector)) ?? "").replace(/\s+/g, " ").trim();
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

/** Presses the control inside `container` whose text is `label`. */
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

/** Types into a field the way a person does — see the sibling specs. */
async function fill(selector: string, value: string): Promise<void> {
  const field = await $(selector);
  await field.waitForExist({ timeout: DRIVER_MS });
  await field.setValue(value);
}

/**
 * Brings the promotion surface into the frame.
 *
 * It is the last thing in the detail pane's meta column, which is long
 * — so a screenshot taken without this shows the top of the column and
 * none of what the stage was about. The assertions read `textContent`
 * and never needed it; the retained frames are for a person.
 */
async function scrollTo(selector: string): Promise<void> {
  await browser.execute((sel: string) => {
    document.querySelector(sel)?.scrollIntoView({ block: "center" });
  }, selector);
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

/**
 * Whether the bytes a card names are still on disk.
 *
 * A card carries its locator in the display spelling — `to_display`,
 * which for a file is the bare path and for anything else is a URL or
 * a name. So this is `existsSync` of the string, and a locator that is
 * not a path answers false, which is the honest answer to the question
 * being asked: a promotion reads the material's bytes off disk.
 *
 * Stored, the same locator is a typed shape (`{"kind":"file",…}`).
 * Reading the card's spelling as that one is what an earlier draft of
 * this did, and it answered "gone" for every row — including the ones
 * it had just written.
 */
function materialIsThere(displayLocator: string): boolean {
  return fs.existsSync(displayLocator);
}

async function api<T>(
  appUrl: string,
  method: string,
  route: string,
  body?: unknown,
): Promise<T> {
  const response = await fetch(`${appUrl}${route}`, {
    method,
    headers:
      body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(
      `${method} ${route} → ${response.status} ${(await response.text()).slice(0, 400)}`,
    );
  }
  return (await response.json()) as T;
}

/**
 * One asset in this profile's library, created if it is not there.
 *
 * Seeded over the app's own HTTP surface rather than by writing SQLite
 * behind the running core, which is `e2e/metric-sort.spec.ts`'s rule
 * and for its reason: the command the importers use is the one that
 * builds a row this app can read.
 *
 * The material is a real file on disk because a promotion reads its
 * bytes: the client hashes at promote time rather than trusting a
 * stored digest, for the reason its own `hash_at_promote_time` gives.
 */
async function ensureAsset(appUrl: string): Promise<string> {
  await api<unknown>(appUrl, "GET", "/asterism/health").catch((err) => {
    throw new Error(
      `the app is not serving HTTP on ${appUrl} (${String(err)}). The fixture ` +
        `asset is seeded over that port, so this run cannot continue.`,
    );
  });

  const personas = await api<PersonaDto[]>(appUrl, "GET", "/asterism/personas");
  const persona =
    personas.find((one) => one.pack_id === PACK_ID) ??
    (await api<PersonaDto>(appUrl, "POST", "/asterism/personas/register", {
      name: PERSONA_NAME,
      pack_id: PACK_ID,
    }));

  const live = await api<PageDto>(
    appUrl,
    "GET",
    `/asterism/assets?persona_id=${encodeURIComponent(persona.id)}&limit=500`,
  );
  // A row whose material is still there is the one to reuse. One
  // whose material is gone is not: a promotion reads the bytes, so
  // reusing it fails at the read rather than at anything this spec is
  // about. That is a state the profile has actually been left in —
  // the rows outlive the paths they remember — and repairing it is
  // the fixture's job rather than a person's.
  const ours = live.items.filter((card) => card.cover === COVER);
  const seen = ours.find((card) => materialIsThere(card.source_locator));
  if (seen) return seen.id;

  // Trashed rather than left beside the new row. The seeded material
  // is the same bytes every time, so a live row pointing at a missing
  // copy of them folds with the one about to be added and the grid
  // shows a card whose asset id is not the one this spec then looks
  // for. Trashing is reversible and this profile is disposable.
  for (const dead of ours) {
    await api<unknown>(appUrl, "POST", "/asterism/assets/trash", {
      asset_id: dead.id,
      comment: "e2e-teams: its material is gone; reseeding",
    });
  }

  // Through `realpath`, and that is the whole of why this line is not
  // a `path.join`. `workspace/` is a symlink into the main checkout,
  // so every worktree writes this file to the same place — but the
  // path *this* worktree reaches it by runs through its own symlink,
  // and that is what the asset row would remember. The row outlives
  // the worktree: a later run from anywhere else finds a locator
  // through a directory that no longer exists, and the promotion
  // fails reading bytes rather than doing anything this spec is about.
  // Measured on 2026-09-01, one worktree after the row was seeded.
  const dir = fs.realpathSync(
    (() => {
      const under = path.join(repoRoot, "workspace/runtime/e2e-fixtures/promote");
      fs.mkdirSync(under, { recursive: true });
      return under;
    })(),
  );
  const file = path.join(dir, "promoted.md");
  if (!fs.existsSync(file)) {
    fs.writeFileSync(
      file,
      "# promoted\n\nFixture material for e2e-teams/teams-promote.spec.ts.\n" +
        "Its bytes are what the content verb carries to the team.\n",
      "utf8",
    );
  }

  const created = await api<{ id: string }>(
    appUrl,
    "POST",
    "/asterism/assets/add",
    {
      persona_id: persona.id,
      source_kind: "fs",
      locator: file,
      modality: null,
      occurred_at_ms: Date.now(),
      labels: ["e2e-promote-fixture"],
      register_note: null,
      platform: null,
      file_size_bytes: fs.statSync(file).size,
      duration_ms: null,
      width_px: null,
      height_px: null,
      extra_json: null,
      cover_hint: COVER,
    },
  );
  return created.id;
}

describe("promoting an asset to a team", () => {
  it("opens work, hands an asset over, and finds the round on it", async () => {
    const trail: string[] = [];
    const { baseUrl, login, password, teamId, lineName, appUrl } = fixture();

    await stage(trail, "the app paints its sidebar", COLD_MS, () =>
      pollUntil(
        () =>
          browser.execute(() => document.querySelector("aside.sidebar") !== null),
        "the app never painted its sidebar",
        COLD_MS,
      ),
    );

    // The drawer is an overlay with a backdrop over everything, and
    // the spec before this one left it open — the panel's open state
    // is the catalog's, and the app process is shared. Anything typed
    // into the sidebar under it is typed into a covered element, which
    // this driver retries rather than refuses: the stage below hit its
    // own ceiling with the driver still waiting. So the window is put
    // back to what a person would be looking at before the fixture is
    // touched.
    await stage(trail, "start from a window with nothing over it", DRIVER_MS, async () => {
      if ((await textOf(DRAWER)) === null) return;
      await clickIn(`${DRAWER} .drawer-close`);
      await pollUntil(
        async () => (await textOf(DRAWER)) === null,
        "the shared-lines drawer would not close",
        DRIVER_MS,
      );
    });

    const assetId = await stage(trail, "seed one asset to promote", ROUND_TRIP_MS, () =>
      ensureAsset(appUrl),
    );

    // The grid read its page before the seed existed. Typing into the
    // sidebar's search and clearing it is the gesture that reloads —
    // the clear skips the debounce, which is what `App.svelte` keeps
    // `reloadSearchImmediate` for.
    await stage(trail, "the grid reads again and shows it", GRID_RELOAD_MS, async () => {
      await fill("#sidebar-search-input", "promoted");
      await clickIn(".search-clear");
      await pollUntil(
        async () =>
          browser.execute(
            (sel: string) => document.querySelector(sel) !== null,
            `.grid-wrapper .card[data-asset-id="${assetId}"]`,
          ),
        "the seeded asset never appeared in the grid",
        GRID_RELOAD_MS,
      );
    });
    await snap("20-seeded");

    await stage(trail, "connect and name the team", ROUND_TRIP_MS, async () => {
      await clickIn(SHARED_ROW);
      await pollUntil(
        async () => (await textOf(DRAWER)) !== null,
        "the drawer never mounted",
        DRIVER_MS,
      );
      if (!((await textOf(DRAWER)) ?? "").includes("Signed in as")) {
        await fill(`${DRAWER} form input[type="url"]`, baseUrl);
        await fill(`${DRAWER} form input[type="text"]`, login);
        await fill(`${DRAWER} form input[type="password"]`, password);
        await clickIn(`${DRAWER} form button[type="submit"]`);
        await pollUntil(
          async () => ((await textOf(DRAWER)) ?? "").includes("Signed in as"),
          "the drawer never reported a session",
          ROUND_TRIP_MS,
        );
      }
      await fill(`${DRAWER} form input[type="text"]`, teamId);
      await clickIn(`${DRAWER} form button[type="submit"]`);
      await clickLabelled(TEAM_TABS, "lines");
      await pollUntil(
        async () => ((await textOf(DRAWER)) ?? "").includes(lineName),
        "the team's lines were never read",
        ROUND_TRIP_MS,
      );
    });

    // Work first, because content enters against open work and nothing
    // else — #148 decision 5, and the reason the pane refuses when
    // there is none.
    await stage(trail, "open work on the line", ROUND_TRIP_MS, async () => {
      // `.lines` rather than `.drawer-list`: two lists share that
      // class since #202, and the looser selector finds the teams.
      await clickCarrying(`${DRAWER} .drawer-list.lines`, lineName);
      await pollUntil(
        async () => ((await textOf(DRAWER)) ?? "").includes("the team's lines"),
        "opening a line did not show its frame",
        ROUND_TRIP_MS,
      );
      await clickLabelled(LINE_TABS, "work");
      await fill(`${DRAWER} .new-work input[type="text"]`, "take this one");
      await clickLabelled(`${DRAWER} .new-work`, "Open");
      await pollUntil(
        async () =>
          ((await textOf(DRAWER)) ?? "").includes("The line, as this would leave it"),
        "opening work did not land on the work itself",
        ROUND_TRIP_MS,
      );
    });
    await snap("21-work-open");

    // Out of the drawer and onto the asset. The drawer holds the three
    // ids while it is closed, which is what lets the pane promote to
    // work opened here.
    await stage(trail, "open the asset's pane", DRIVER_MS, async () => {
      await clickIn(`${DRAWER} .drawer-close`);
      await clickIn(`.grid-wrapper .card[data-asset-id="${assetId}"]`);
      await pollUntil(
        async () => (await textOf(PROMOTE)) !== null,
        "the detail pane never showed the promotion surface",
        DRIVER_MS,
      );
    });
    await scrollTo(PROMOTE);
    await snap("22-pane-open");

    await stage(trail, "it says what travels before it goes", DRIVER_MS, async () => {
      const text = await proseOf(PROMOTE);
      // Decision 4, on the screen rather than only in the client.
      if (!text.includes("What goes")) {
        throw new Error(`the pane did not say what travels: ${text}`);
      }
      if (!text.includes("What stays")) {
        throw new Error(`the pane did not say what stays home: ${text}`);
      }
      // And which work it would go onto, which is the thing a person
      // has to be able to check before pressing.
      if (!text.includes("take this one")) {
        throw new Error(`the pane did not name the open work: ${text}`);
      }
      if (!text.includes(lineName)) {
        throw new Error(`the pane did not name the line: ${text}`);
      }
    });

    await stage(trail, "hand it over", ROUND_TRIP_MS, async () => {
      await fill(`${PROMOTE} input[type="text"]`, NAMED);
      await clickLabelled(PROMOTE, "Promote");
      // A refusal is shown by `mutate` as a toast and by nothing else
      // — the surface's own catch adds nothing to it — so the wait is
      // for either answer. Without this, a refused promotion times out
      // saying only that no digest arrived, which is the least useful
      // half of what the screen is showing.
      await pollUntil(
        async () =>
          (await proseOf(PROMOTE)).includes("sha256:") ||
          (await textOf(".refusal-toast")) !== null,
        "the promotion neither reported a digest nor a refusal",
        ROUND_TRIP_MS,
      );
      const refused = await proseOf(".refusal-toast");
      if (refused !== "") {
        throw new Error(`the promotion was refused: ${refused}`);
      }
      const text = await proseOf(PROMOTE);
      // The team is new this run, so this promotion is new: the bytes
      // were sent, and the answer must not read as a repeat.
      if (text.includes("Nothing.")) {
        throw new Error(
          `a first promotion onto a fresh team reported itself a repeat: ${text}`,
        );
      }
      if (!text.includes("the team did not have these bytes")) {
        throw new Error(
          `the have-check answered something else — a fresh team held these ` +
            `bytes already, or the answer was not shown: ${text}`,
        );
      }
    });
    await scrollTo(PROMOTE);
    await snap("23-promoted");

    // Read back through the drawer rather than off the answer the
    // write returned: the round is on the team's server, and this is
    // the only assertion that says so.
    await stage(trail, "the round is on the work", ROUND_TRIP_MS, async () => {
      await clickIn(".detail-panel .detail-close");
      await clickIn(SHARED_ROW);
      await pollUntil(
        async () => {
          const text = (await textOf(DRAWER)) ?? "";
          return text.includes(NAMED) && text.includes("1 operation");
        },
        "the promotion's round never reached the work log",
        ROUND_TRIP_MS,
      );
    });
    await snap("24-round-on-work");

    await stage(trail, "disconnect empties the view", ROUND_TRIP_MS, async () => {
      await clickIn(`${DRAWER} .drawer-session button`);
      await pollUntil(
        async () => !((await textOf(DRAWER)) ?? "").includes("Signed in as"),
        "the drawer kept the session after disconnecting",
        ROUND_TRIP_MS,
      );
    });
  });
});
