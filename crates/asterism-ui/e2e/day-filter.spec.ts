// The "Occurred on" sidebar section, in a real window.
//
// The calendar filter is pinned twice already: `day_filter_e2e.rs` runs
// six fixtures through the HTTP and MCP paths and `jump-filter.test.ts`
// runs the store through vitest. What neither can answer is the
// question `wdio.conf.ts` says this suite exists for: whether typing a
// date into the section narrows the grid, whether the span buttons and
// the every-year toggle change what it holds, and whether the chip in
// the filter band clears it. That is the whole content of this file.
//
// # The fixture, and the zone the expectations are read in
//
// Six rows, all under a persona of their own, stamped in March 2019 so
// they sort below whatever else the profile holds and stay away from
// `card-trash.spec.ts`, which trashes and restores "the first card".
// The instants are chosen around Tokyo's 14 March 2019 — both edges of
// it, the instant that opens the 15th, one row whose importer had no
// occurrence, one row that says where it happened — but the
// expectations are **not** written in Tokyo. The day the section
// selects is opened in the viewer's zone, and the viewer is whatever
// zone the window resolves (`viewerTimeZone()` in the filter store), so
// the spec reads that zone out of the page and works each row's day out
// under it, by the rule the server applies: an `import`-sourced row's
// time is its arrival, every other row's is its occurrence; a row with a
// zone of its own is read on its own local day and the viewer's zone
// never enters (`asterism_core::domain::asset_zone`).
//
// Two of the sets below are literals rather than derivations — the
// week and the month containing 14 March 2019 are `[03-11, 03-18)` and
// `[03-01, 04-01)` — so the store's Monday-first week rule is checked
// against a written date and not against a second copy of itself.
//
// The vacuity guards are asserted, not assumed. A day that showed every
// row, or a toggle that changed nothing, would pass a `toEqual` while
// the section did nothing, which is the shape of this repo's two
// earlier vacuous ordering assertions. So the day set has to be a
// proper, non-empty subset of the six; the week set has to be a proper
// superset of the day set; and the every-year set has to reach the row
// this file keeps in 2018 for exactly that purpose.
//
// # Where the rows come from, and what is not asserted
//
// Seeding is over the app's own loopback HTTP surface, additive and
// idempotent, on the terms `metric-sort.spec.ts` sets: find by cover
// text, verify against the table, restore from the trash, create only
// what is missing, and stop rather than repair a row that disagrees.
//
// A Query Group round trip (save the filter, change it, restore it) is
// not driven here: no spec in this directory drives the group menu, and
// `jump-filter.test.ts` pins the restore path over the store. The
// gesture is a `change` event on the date box and clicks on the
// buttons, the way `metric-sort.spec.ts` drives its `<select>`, and for
// the same reason: the native date picker is out of this driver's
// reach, so what a green run says is that the app narrows the grid when
// the box's value changes.

import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

// --- budgets -------------------------------------------------------
//
// Same shape and the same reasoning as `metric-sort.spec.ts`.

/** One driver round-trip. */
const DRIVER_MS = 15_000;
/** The grid has to repaint after a filter change. */
const GRID_MS = 20_000;
/** A cold start: the window, the SQLite open, the first page load. */
const COLD_MS = 60_000;
/** Two identical reads of the grid, this far apart, count as settled;
 *  outside the filter-reload debounce (`App.svelte`). */
const SETTLE_GAP_MS = 400;
/** Gap between `execute`-based polls. */
const POLL_GAP_MS = 250;

// --- the fixture ---------------------------------------------------

interface FixtureRow {
  readonly name: string;
  /** `occurred_at_ms`, a UTC instant. */
  readonly occurredMs: number;
  /** The rung the stamp came from. `import` means the row's time is its
   *  arrival, which nothing here can write down — it is read back off
   *  the created row. */
  readonly source: "unknown" | "import";
  /** The zone the row says it happened in, or none. */
  readonly timeZone: string | null;
}

/** 2019-03-13T15:00:00Z — Tokyo's 14 March 2019 opens here. */
const TOKYO_DAY_OPENS = Date.UTC(2019, 2, 13, 15, 0, 0);
/** 24 hours in ms. */
const DAY_MS = 24 * 60 * 60 * 1000;

const FIXTURE: readonly FixtureRow[] = [
  // Both edges of Tokyo's 14 March, and the instant that opens its
  // 15th. Which of the three a viewer's 14 March holds depends on the
  // viewer's zone, which is the point: the expectation is derived per
  // zone, and no zone can hold `early` and `next` on one day, since
  // they are exactly 24 hours apart.
  { name: "early", occurredMs: TOKYO_DAY_OPENS, source: "unknown", timeZone: null },
  { name: "late", occurredMs: TOKYO_DAY_OPENS + DAY_MS - 1000, source: "unknown", timeZone: null },
  { name: "next", occurredMs: TOKYO_DAY_OPENS + DAY_MS, source: "unknown", timeZone: null },
  // Its column says noon on the 14th, its source says the stamp is the
  // import moment, so its day is the day this spec first seeded it —
  // never a day in 2019.
  {
    name: "imported",
    occurredMs: Date.UTC(2019, 2, 14, 3, 0, 0),
    source: "import",
    timeZone: null,
  },
  // 01:00 on the 14th in Tokyo, which is still the 13th in UTC and
  // everywhere west of UTC+8. The row says Tokyo, so it is read on the
  // 14th wherever the viewer is.
  {
    name: "zoned",
    occurredMs: Date.UTC(2019, 2, 13, 16, 0, 0),
    source: "unknown",
    timeZone: "Asia/Tokyo",
  },
  // The same month and day a year earlier, so "same day, every year"
  // has a row to reach that the day alone does not. 11:00Z is the 14th
  // for every viewer from UTC-11 to UTC+12.
  {
    name: "earlier-year",
    occurredMs: Date.UTC(2018, 2, 14, 11, 0, 0),
    source: "unknown",
    timeZone: null,
  },
];

/** The date the section is sent to, and the two spans around it as
 *  half-open literals. 2019-03-14 is a Thursday; ISO weeks open on
 *  Monday the 11th. */
const PICKED = "2019-03-14";
const WEEK = { from: "2019-03-11", until: "2019-03-18" };
const MONTH = { from: "2019-03-01", until: "2019-04-01" };

const PACK_ID = "e2e-day-filter";
const PERSONA_NAME = "E2E Day Filter";

function coverOf(name: string): string {
  return `e2e-day:${name}`;
}

// --- HTTP, for the fixture only ------------------------------------

const HTTP_PORT = Number(process.env.E2E_APP_PORT ?? 19899);
const BASE_URL = `http://127.0.0.1:${HTTP_PORT}`;

async function api<T>(method: string, route: string, body?: unknown): Promise<T> {
  const response = await fetch(`${BASE_URL}${route}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(
      `${method} ${route} → ${response.status} ${(await response.text()).slice(0, 400)}`,
    );
  }
  return (await response.json()) as T;
}

interface PersonaDto {
  id: string;
  pack_id: string | null;
  name: string;
}

interface CardDto {
  id: string;
  cover: string | null;
  occurred_at_ms: number;
  occurred_source: string;
  time_zone: string | null;
  created_at_ms: number;
}

interface PageDto {
  items: CardDto[];
}

/** One seeded row as the profile holds it: the instant the filter
 *  resolves to, and the zone it is read in. */
interface SeededRow {
  readonly name: string;
  readonly id: string;
  readonly resolvedMs: number;
  readonly timeZone: string | null;
}

function repoRoot(): string {
  let dir = process.cwd();
  for (;;) {
    if (fs.existsSync(path.join(dir, "workspace/runtime/e2e"))) return dir;
    const up = path.dirname(dir);
    if (up === dir) {
      throw new Error(
        `could not find a repo root above ${process.cwd()} holding ` +
          "workspace/runtime/e2e — the e2e profile the suite runs against",
      );
    }
    dir = up;
  }
}

function writeFixtureFile(root: string, row: FixtureRow): string {
  const dir = path.join(root, "workspace/runtime/e2e-fixtures/day-filter");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `${row.name}.md`);
  if (!fs.existsSync(file)) {
    fs.writeFileSync(
      file,
      `# ${row.name}\n\nFixture row for e2e/day-filter.spec.ts.\n` +
        `occurred_at_ms=${row.occurredMs} occurred_source=${row.source} ` +
        `time_zone=${row.timeZone ?? "none"}\n`,
      "utf8",
    );
  }
  return file;
}

/** The row's resolved instant, by the rule the server applies. */
function resolvedOf(card: CardDto): number {
  return card.occurred_source === "import" ? card.created_at_ms : card.occurred_at_ms;
}

function seededOf(name: string, card: CardDto): SeededRow {
  return { name, id: card.id, resolvedMs: resolvedOf(card), timeZone: card.time_zone };
}

/**
 * Brings the profile to the state the assertions need. Additive by
 * construction, and refuses to repair a row that disagrees with the
 * table — see the file header.
 */
async function ensureFixture(): Promise<SeededRow[]> {
  await api<unknown>("GET", "/asterism/health").catch((err) => {
    throw new Error(
      `the app is not serving HTTP on ${BASE_URL} (${String(err)}). ` +
        "The fixture is seeded over that port, so this run cannot continue.",
    );
  });

  const personas = await api<PersonaDto[]>("GET", "/asterism/personas");
  const persona =
    personas.find((p) => p.pack_id === PACK_ID) ??
    (await api<PersonaDto>("POST", "/asterism/personas/register", {
      name: PERSONA_NAME,
      pack_id: PACK_ID,
    }));

  const live = await api<PageDto>(
    "GET",
    `/asterism/assets?persona_id=${encodeURIComponent(persona.id)}&limit=500`,
  );
  let trashed: CardDto[] | null = null;
  const root = repoRoot();
  const seeded: SeededRow[] = [];

  for (const row of FIXTURE) {
    const cover = coverOf(row.name);
    const seen = live.items.find((c) => c.cover === cover);
    if (seen) {
      const carried = {
        occurred_at_ms: seen.occurred_at_ms,
        occurred_source: seen.occurred_source,
        time_zone: seen.time_zone,
      };
      const wanted = {
        occurred_at_ms: row.occurredMs,
        occurred_source: row.source,
        time_zone: row.timeZone,
      };
      if (JSON.stringify(carried) !== JSON.stringify(wanted)) {
        throw new Error(
          `the e2e profile already holds a fixture row "${row.name}" (asset ${seen.id}) ` +
            `carrying ${JSON.stringify(carried)}, but this spec's table says ` +
            `${JSON.stringify(wanted)}. Nothing here rewrites it: delete that asset (or the ` +
            `"${PERSONA_NAME}" persona) and re-run to reseed.`,
        );
      }
      seeded.push(seededOf(row.name, seen));
      continue;
    }

    if (trashed === null) {
      trashed = (
        await api<PageDto>(
          "GET",
          `/asterism/assets?persona_id=${encodeURIComponent(persona.id)}&trash=trashed&limit=500`,
        )
      ).items;
    }
    const buried = trashed.find((c) => c.cover === cover);
    if (buried) {
      await api<unknown>("POST", "/asterism/assets/restore", { asset_id: buried.id });
      seeded.push(seededOf(row.name, buried));
      continue;
    }

    const created = await api<CardDto>("POST", "/asterism/assets/add", {
      persona_id: persona.id,
      source_kind: "fs",
      locator: writeFixtureFile(root, row),
      modality: null,
      occurred_at_ms: row.occurredMs,
      occurred_source: row.source,
      time_zone: row.timeZone,
      labels: ["e2e-day-fixture"],
      register_note: null,
      platform: null,
      file_size_bytes: null,
      duration_ms: null,
      width_px: null,
      height_px: null,
      extra_json: null,
      cover_hint: cover,
    });
    seeded.push(seededOf(row.name, created));
  }

  return seeded;
}

// --- the calendar, in the viewer's zone ----------------------------

/**
 * The calendar day `ms` falls on in `zone`, as `YYYY-MM-DD`. `en-CA`
 * because its numeric date form is that string already.
 */
function localDay(ms: number, zone: string): string {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: zone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date(ms));
}

/** The day a seeded row is read on: its own zone when it has one, the
 *  viewer's otherwise. */
function dayOf(row: SeededRow, viewerZone: string): string {
  return localDay(row.resolvedMs, row.timeZone ?? viewerZone);
}

function namesWhere(rows: readonly SeededRow[], keep: (day: string) => boolean): string[] {
  return rows
    .filter((row) => keep(dayOf(row, viewerZone)))
    .map((row) => row.name)
    .sort();
}

// --- screenshots, stages, polling ----------------------------------

const SCREENS_DIR = process.env.E2E_SCREENS_DIR;
let shotSeq = 0;

async function snapStage(name: string, failed = false): Promise<void> {
  if (!SCREENS_DIR) return;
  shotSeq += 1;
  const safe = name.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 60);
  try {
    await Promise.race([
      browser.saveScreenshot(
        path.join(
          SCREENS_DIR,
          `${String(shotSeq).padStart(3, "0")}_df_${failed ? "FAIL_" : ""}${safe}.png`,
        ),
      ),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
  } catch {
    // Liveness aid only.
  }
}

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
      if (Date.now() >= deadline) {
        throw new Error(`${message} (polled for ${ms}ms)`);
      }
      await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
    }
  });
}

// --- page reads and gestures ---------------------------------------
//
// Every callback below is an anonymous arrow in argument position, and
// returns rather than throws on a fault path; the reasons are in
// `card-trash.spec.ts` (`openCardMenu`, `readDom`), and the `__name`
// shim installed after each load is the other half of the first one.

const NAME_SHIM = "window.__name = window.__name || function (target) { return target; };";

const DATE_INPUT = 'aside.sidebar .jump input[type="date"]';
const SPAN_BUTTONS = "aside.sidebar .jump-spans button";
const EVERY_YEAR = "aside.sidebar .jump-every-year input";
const DAY_CHIP = ".active-filters-band .afb-chip.day";

interface Section {
  present: boolean;
  date: string;
  /** The label of the span button carrying `.active`, or "". */
  activeSpan: string;
  everyYear: boolean;
  chipPresent: boolean;
  chipText: string;
}

async function readSection(): Promise<Section> {
  return browser
    .execute(
      (dateQuery: string, spanQuery: string, everyQuery: string, chipQuery: string) => {
        const input = document.querySelector(dateQuery);
        const spans = Array.from(document.querySelectorAll(spanQuery));
        const active = spans.find((b) => b.classList.contains("active"));
        const every = document.querySelector(everyQuery);
        const chip = document.querySelector(chipQuery);
        return {
          present: input !== null,
          date: input instanceof HTMLInputElement ? input.value : "",
          activeSpan: active ? (active.textContent ?? "").trim() : "",
          everyYear: every instanceof HTMLInputElement ? every.checked : false,
          chipPresent: chip !== null,
          chipText: chip ? (chip.textContent ?? "").replace(/\s+/g, " ").trim() : "",
        };
      },
      DATE_INPUT,
      SPAN_BUTTONS,
      EVERY_YEAR,
      DAY_CHIP,
    )
    .catch(() => ({
      present: false,
      date: "",
      activeSpan: "",
      everyYear: false,
      chipPresent: false,
      chipText: "",
    }));
}

/** Types a date the way the engine leaves the box after a pick: assign,
 *  then `change`, which is the event the section commits on. */
async function typeDate(iso: string): Promise<boolean> {
  return browser
    .execute(
      (query: string, value: string) => {
        const input = document.querySelector(query);
        if (!(input instanceof HTMLInputElement)) return false;
        input.value = value;
        input.dispatchEvent(new Event("change", { bubbles: true }));
        return input.value === value;
      },
      DATE_INPUT,
      iso,
    )
    .catch(() => false);
}

/** Clicks the span button whose label is `label`. */
async function clickSpan(label: string): Promise<boolean> {
  return browser
    .execute(
      (query: string, wanted: string) => {
        const button = Array.from(document.querySelectorAll(query)).find(
          (b) => (b.textContent ?? "").trim() === wanted,
        );
        if (!(button instanceof HTMLButtonElement) || button.disabled) return false;
        button.click();
        return true;
      },
      SPAN_BUTTONS,
      label,
    )
    .catch(() => false);
}

async function clickEveryYear(): Promise<boolean> {
  return browser
    .execute((query: string) => {
      const box = document.querySelector(query);
      if (!(box instanceof HTMLInputElement) || box.disabled) return false;
      box.click();
      return true;
    }, EVERY_YEAR)
    .catch(() => false);
}

async function clickChip(): Promise<boolean> {
  return browser
    .execute((query: string) => {
      const chip = document.querySelector(query);
      if (!(chip instanceof HTMLElement)) return false;
      chip.click();
      return true;
    }, DAY_CHIP)
    .catch(() => false);
}

async function readGridIds(): Promise<string[]> {
  return browser
    .execute(
      (query: string) =>
        Array.from(document.querySelectorAll(query)).map(
          (el) => el.getAttribute("data-asset-id") ?? "",
        ),
      ".grid-wrapper .card",
    )
    .catch(() => [] as string[]);
}

async function readViewerZone(): Promise<string> {
  return browser
    .execute(() => Intl.DateTimeFormat().resolvedOptions().timeZone ?? "")
    .catch(() => "");
}

interface Sidebar {
  present: boolean;
  fixtureRowPresent: boolean;
  fixtureRowActive: boolean;
  marked: boolean;
}

function personaRowSelector(personaId: string): string {
  return `aside.sidebar li.persona-row[data-persona-id="${personaId}"]`;
}

async function readSidebar(personaId: string): Promise<Sidebar> {
  return browser
    .execute((query: string) => {
      const row = document.querySelector(query);
      const btn = row ? row.querySelector("button") : null;
      return {
        present: document.querySelector("aside.sidebar") !== null,
        fixtureRowPresent: row !== null,
        fixtureRowActive: btn !== null && btn.classList.contains("active"),
        marked: (window as unknown as { __dayFilterMark?: boolean }).__dayFilterMark === true,
      };
    }, personaRowSelector(personaId))
    .catch(() => ({
      present: false,
      fixtureRowPresent: false,
      fixtureRowActive: false,
      marked: false,
    }));
}

async function markPage(): Promise<void> {
  await browser
    .execute(() => {
      (window as unknown as { __dayFilterMark?: boolean }).__dayFilterMark = true;
    })
    .catch(() => undefined);
}

async function clickPersonaRow(personaId: string): Promise<boolean> {
  return browser
    .execute((query: string) => {
      const row = document.querySelector(query);
      const btn = row ? row.querySelector("button") : null;
      if (btn instanceof HTMLElement) {
        btn.click();
        return true;
      }
      return false;
    }, personaRowSelector(personaId))
    .catch(() => false);
}

async function clearPersonaFilter(): Promise<boolean> {
  return browser
    .execute(() => {
      const row = document.querySelector("aside.sidebar li.persona-row");
      const list = row ? row.parentElement : null;
      const all = list ? list.querySelector("li:not(.persona-row) button") : null;
      if (all instanceof HTMLElement) {
        all.click();
        return true;
      }
      return false;
    })
    .catch(() => false);
}

// --- the run -------------------------------------------------------

let seeded: SeededRow[] = [];
let viewerZone = "";

function asNames(ids: readonly string[]): string[] {
  return ids.map((id) => seeded.find((row) => row.id === id)?.name ?? `?${id.slice(0, 8)}`).sort();
}

/**
 * Waits for the grid's card set to move off `previous` and stop moving,
 * then reports it in fixture names, sorted. Two identical reads
 * `SETTLE_GAP_MS` apart that differ from `previous` count as settled;
 * on the deadline it hands back what it last saw rather than throwing,
 * so a wrong set fails as a diff and not as a timeout.
 */
async function settledSet(trail: string[], name: string, previous: readonly string[]) {
  const before = [...previous].sort().join(",");
  return stage(trail, name, GRID_MS + DRIVER_MS, async () => {
    let last = "";
    const deadline = Date.now() + GRID_MS;
    for (;;) {
      const names = asNames(await readGridIds());
      const shape = names.join(",");
      if (shape !== before && shape === last) return names;
      if (Date.now() >= deadline) return names;
      last = shape;
      await new Promise((resolve) => setTimeout(resolve, SETTLE_GAP_MS));
    }
  });
}

/** Sends the section to `iso` on the day span and waits for the grid. */
async function jumpToDay(trail: string[], iso: string, previous: readonly string[]) {
  const typed = await stage(trail, `type ${iso}`, DRIVER_MS, () => typeDate(iso));
  expect({ step: `date box accepts ${iso}`, typed }).toEqual({
    step: `date box accepts ${iso}`,
    typed: true,
  });
  return settledSet(trail, `grid under ${iso}`, previous);
}

describe("occurred-on day filter", () => {
  let personaId = "";
  let all: string[] = [];

  before(async () => {
    const trail: string[] = [];
    await stage(trail, "install __name shim", DRIVER_MS, () => browser.execute(NAME_SHIM));
    await pollUntil(
      trail,
      "app window paints",
      COLD_MS,
      async () => (await readSidebar("")).present,
      "the app never painted its sidebar",
    );

    seeded = await stage(trail, "seed fixture over HTTP", 60_000, () => ensureFixture());
    const personas = await api<PersonaDto[]>("GET", "/asterism/personas");
    personaId = personas.find((p) => p.pack_id === PACK_ID)?.id ?? "";
    expect(personaId).not.toBe("");
    all = seeded.map((row) => row.name).sort();

    // The zone the window resolves is the zone the expectations are
    // built in; an empty answer would make every derived set wrong in
    // a way that reads as a filter defect, so it is refused here.
    viewerZone = await stage(trail, "read the viewer's zone", DRIVER_MS, () => readViewerZone());
    expect({ viewerZone: viewerZone.length > 0 ? "resolved" : "" }).toEqual({
      viewerZone: "resolved",
    });
    console.log(`[day-filter] viewer zone ${viewerZone}`);

    // Reload clean, for the reasons `metric-sort.spec.ts` gives: a
    // persona registered after startup is not in the painted sidebar,
    // and the search string carries the filter state.
    await markPage();
    await stage(trail, "reload with a clean filter state", DRIVER_MS, () =>
      browser.execute(() => {
        history.replaceState(history.state, "", window.location.pathname);
        window.location.reload();
      }),
    );
    await pollUntil(
      trail,
      "app window repaints",
      COLD_MS,
      async () => {
        const dom = await readSidebar("");
        return dom.present && !dom.marked;
      },
      "the app never came back with a fresh document after the reload",
    );
    await stage(trail, "reinstall __name shim", DRIVER_MS, () => browser.execute(NAME_SHIM));

    await pollUntil(
      trail,
      "fixture persona row appears",
      GRID_MS,
      async () => (await readSidebar(personaId)).fixtureRowPresent,
      `the sidebar never listed the "${PERSONA_NAME}" persona`,
    );
    await stage(trail, "click the fixture persona", DRIVER_MS, () => clickPersonaRow(personaId));
    await pollUntil(
      trail,
      "fixture persona is active",
      GRID_MS,
      async () => (await readSidebar(personaId)).fixtureRowActive,
      "clicking the fixture persona row did not select it",
    );

    // Every row on screen before any day is picked: the narrowing
    // below is only a narrowing if this is where it starts from.
    await pollUntil(
      trail,
      "grid holds the six fixture rows",
      GRID_MS,
      async () => asNames(await readGridIds()).join(",") === all.join(","),
      "the grid never showed exactly the six fixture rows",
    );
    const section = await readSection();
    expect({ sectionPresent: section.present, chipBefore: section.chipPresent }).toEqual({
      sectionPresent: true,
      chipBefore: false,
    });
  });

  // Every test starts from all six rows and no day filter, so the
  // `previous` set each one hands `settledSet` is the set actually on
  // screen — a stale read could otherwise be mistaken for a settled
  // one when a test inherited the last one's filter.
  beforeEach(async () => {
    const trail: string[] = [];
    if ((await readSection()).chipPresent) {
      await stage(trail, "clear the day filter", DRIVER_MS, () => clickChip());
    }
    await pollUntil(
      trail,
      "grid holds every fixture row again",
      GRID_MS,
      async () => asNames(await readGridIds()).join(",") === all.join(","),
      "the grid did not return to the six fixture rows",
    );
  });

  after(async () => {
    // Leave the app the way the other specs expect to find it: no day
    // filter, no persona filter. Swallowed on purpose.
    await clickChip().catch(() => false);
    await clearPersonaFilter().catch(() => false);
  });

  it("keeps a fixture whose day sets are proper subsets of each other", () => {
    // The vacuity guard, read off the table under the viewer's zone
    // rather than off the app: a day that held every row, or a span
    // that added nothing, could pass the grid assertions below while
    // the section did nothing.
    const day = namesWhere(seeded, (d) => d === PICKED);
    const week = namesWhere(seeded, (d) => d >= WEEK.from && d < WEEK.until);
    const month = namesWhere(seeded, (d) => d >= MONTH.from && d < MONTH.until);
    const everyYear = namesWhere(seeded, (d) => d.endsWith(PICKED.slice(4)));
    const subset = (a: string[], b: string[]) => a.every((n) => b.includes(n));
    expect({
      viewerZone,
      dayNonEmpty: day.length > 0,
      dayNarrows: day.length < all.length,
      zonedReadOnItsOwnDay: day.includes("zoned"),
      importedReadOnArrival: !week.includes("imported"),
      weekWidens: subset(day, week) && week.length > day.length,
      monthHoldsWeek: subset(week, month),
      everyYearReachesTheEarlierYear: subset(day, everyYear) && everyYear.includes("earlier-year"),
    }).toEqual({
      viewerZone,
      dayNonEmpty: true,
      dayNarrows: true,
      zonedReadOnItsOwnDay: true,
      importedReadOnArrival: true,
      weekWidens: true,
      monthHoldsWeek: true,
      everyYearReachesTheEarlierYear: true,
    });
  });

  it("narrows the grid to the picked day, and shows the chip", async () => {
    const trail: string[] = [];
    const got = await jumpToDay(trail, PICKED, all);
    expect(got).toEqual(namesWhere(seeded, (d) => d === PICKED));

    const section = await stage(trail, "read the section", DRIVER_MS, () => readSection());
    expect({
      date: section.date,
      activeSpan: section.activeSpan,
      everyYear: section.everyYear,
      chipPresent: section.chipPresent,
      chipNamesTheDay: section.chipText.includes(`${PICKED} · day`),
    }).toEqual({
      date: PICKED,
      activeSpan: "Day",
      everyYear: false,
      chipPresent: true,
      chipNamesTheDay: true,
    });
  });

  it("reads a zoned row on its own day, not the viewer's", async () => {
    // The zoned row's instant is the 13th in UTC and everywhere west of
    // UTC+8; the row says Tokyo, so the 13th must not hold it wherever
    // the viewer is, and the 14th must. Derived under the viewer's zone
    // like every other set, so in a zone at or east of UTC+8 the first
    // half is the same set the unzoned rule would give — the second
    // half still holds the row on the 14th.
    const trail: string[] = [];
    const before = await jumpToDay(trail, PICKED, all);
    const got = await jumpToDay(trail, "2019-03-13", before);
    expect(got).toEqual(namesWhere(seeded, (d) => d === "2019-03-13"));
    expect({ zonedOnTheThirteenth: got.includes("zoned") }).toEqual({
      zonedOnTheThirteenth: false,
    });
  });

  it("widens to the week and the month containing the day", async () => {
    const trail: string[] = [];
    const day = await jumpToDay(trail, PICKED, all);

    const weekTook = await stage(trail, "click Week", DRIVER_MS, () => clickSpan("Week"));
    expect({ step: "Week is clickable", took: weekTook }).toEqual({
      step: "Week is clickable",
      took: true,
    });
    const week = await settledSet(trail, "grid under the week", day);
    expect(week).toEqual(namesWhere(seeded, (d) => d >= WEEK.from && d < WEEK.until));
    const afterWeek = await readSection();
    // The box shows the day the range opens on — the Monday — not the
    // day that was typed.
    expect({ date: afterWeek.date, activeSpan: afterWeek.activeSpan }).toEqual({
      date: WEEK.from,
      activeSpan: "Week",
    });

    // Back to a day, then the month — not month from week: the month's
    // set equals the week's for this fixture, and `settledSet` waits
    // for a change. Clicking Day rather than retyping the date, because
    // a date change keeps the span the picker already has; and Day
    // opens the date the box shows, which is the Monday.
    const dayTook = await stage(trail, "click Day", DRIVER_MS, () => clickSpan("Day"));
    expect({ step: "Day is clickable", took: dayTook }).toEqual({
      step: "Day is clickable",
      took: true,
    });
    const monday = await settledSet(trail, "grid under the Monday", week);
    expect(monday).toEqual(namesWhere(seeded, (d) => d === WEEK.from));
    const monthTook = await stage(trail, "click Month", DRIVER_MS, () => clickSpan("Month"));
    expect({ step: "Month is clickable", took: monthTook }).toEqual({
      step: "Month is clickable",
      took: true,
    });
    const month = await settledSet(trail, "grid under the month", monday);
    expect(month).toEqual(namesWhere(seeded, (d) => d >= MONTH.from && d < MONTH.until));
    expect((await readSection()).date).toBe(MONTH.from);
  });

  it("drops the year with the every-year toggle, and puts it back", async () => {
    const trail: string[] = [];
    const day = await jumpToDay(trail, PICKED, all);

    const on = await stage(trail, "tick every year", DRIVER_MS, () => clickEveryYear());
    expect({ step: "toggle is clickable", took: on }).toEqual({
      step: "toggle is clickable",
      took: true,
    });
    const everyYear = await settledSet(trail, "grid under 03-14, every year", day);
    expect(everyYear).toEqual(namesWhere(seeded, (d) => d.endsWith(PICKED.slice(4))));
    const ticked = await readSection();
    expect({
      everyYear: ticked.everyYear,
      activeSpan: ticked.activeSpan,
      chipNamesEveryYear: ticked.chipText.includes("03-14 · every year"),
    }).toEqual({ everyYear: true, activeSpan: "", chipNamesEveryYear: true });

    // Off again lands on the day in the current year, which holds none
    // of the fixture — the year that was dropped is not remembered, and
    // the section says so by opening the day where the picker showed
    // it.
    const off = await stage(trail, "untick every year", DRIVER_MS, () => clickEveryYear());
    expect({ step: "toggle is clickable again", took: off }).toEqual({
      step: "toggle is clickable again",
      took: true,
    });
    const thisYear = await settledSet(trail, "grid under 03-14 this year", everyYear);
    const today = new Date().getFullYear();
    expect(thisYear).toEqual(namesWhere(seeded, (d) => d === `${today}-03-14`));
    expect((await readSection()).date).toBe(`${today}-03-14`);
  });

  it("clears the filter from the chip and shows every row again", async () => {
    const trail: string[] = [];
    const day = await jumpToDay(trail, PICKED, all);
    const clicked = await stage(trail, "click the chip", DRIVER_MS, () => clickChip());
    expect({ step: "chip is clickable", took: clicked }).toEqual({
      step: "chip is clickable",
      took: true,
    });
    const cleared = await settledSet(trail, "grid after the chip", day);
    expect(cleared).toEqual(all);
    const section = await readSection();
    expect({ chipPresent: section.chipPresent, date: section.date }).toEqual({
      chipPresent: false,
      date: "",
    });
  });
});
