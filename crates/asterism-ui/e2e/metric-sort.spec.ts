// Picking Length, Size and Pixels in the Sort dropdown, in a real
// window.
//
// Everything else about these three axes is already pinned elsewhere.
// `sorted_list_e2e.rs` runs the fixture below through the backend and
// `card-cmp.test.ts` runs the comparator through vitest; between them
// the ordering rule — largest first in the natural direction, rows with
// no value at the tail in *both* directions — is covered twice over.
// What neither can answer is the question `wdio.conf.ts` says this
// suite exists for: whether picking the option in the panel reorders
// the grid. That is the whole content of this file.
//
// The claim is narrow on purpose. The grid's own fetch carries no sort
// spec (`App.svelte:currentFilter` has no `sort` field), so the server
// hands back arrival order and `card-cmp.ts` alone decides what the
// window shows. An assertion here therefore fails when the comparator
// stops working, which is the property the subtask's mutation check
// asks for.
//
// # The fixture, and the two vacuous assertions that shaped it
//
// This repo has shipped an ordering assertion that passed while the
// axis did nothing, twice: a `Cover` axis over rows whose covers were
// all `None`, and an ordering assertion under a single-Group filter
// where arrival order already *was* `asset_bucket.position`. Both
// passed because the axis under test agreed with the default. So the
// fixture is built to disagree with it, and the disagreement is
// asserted rather than assumed — see the first `it`, which reads the
// table and refuses a shape that could pass vacuously:
//
//   * length order and size order are a derangement of each other (no
//     row keeps its index between them),
//   * the row with no length and the row with no size are *different*
//     rows, so tailing the unmeasured row on one axis cannot be
//     satisfied by the other axis's gap,
//   * arrival order (occurred_at DESC) matches none of the four axis
//     orders.
//
// The numbers are copied verbatim from `sorted_list_e2e.rs`'s
// `metric axes` fixture, so the sequence this window produces is
// directly comparable to the one the HTTP path produced. Only the
// timestamps differ: they are moved back to 2019 so these five rows
// sort *below* whatever else the profile holds, which keeps
// `card-trash.spec.ts` — which trashes and restores "the first card" —
// away from the fixture.
//
// # Where the five rows come from
//
// The other two specs read the profile they are handed. This one
// cannot: no ambient profile carries a row with a playback length of
// exactly two minutes and a stored size of exactly half a megabyte,
// and a spec that asserted over whatever happened to be there would be
// asserting over an unknown. So it seeds, through the app's own
// loopback HTTP surface (`POST /asterism/assets/add`, the same command
// the importers use) rather than by writing SQLite behind the running
// core.
//
// Seeding is idempotent and additive. The rows are found by their
// cover text, verified against the table, and only the missing ones are
// created — a second run adds nothing, and a row that a failed run left
// in the trash is restored rather than duplicated. Nothing here trashes,
// purges or deletes: the fixture becomes part of the e2e profile the
// same way its existing assets are, and `workspace/runtime/e2e` is the
// disposable profile by construction (`wdio.conf.ts`), never Dogfood.
//
// A row that exists under the fixture's cover text but disagrees with
// the table stops the run instead of being repaired. Rewriting it would
// mean deciding, unattended, that the profile is wrong and this file is
// right; a fixture that is not what the spec thinks it is deserves a
// person.
//
// # Cost model, and why every interaction goes through the DOM
//
// Inherited from `e2e/card-trash.spec.ts`: in this environment every
// *element* command (`$`, `$$`, `elementClick`, …) pays a ~6 s
// window-focus tax and `browser.execute` pays none. That file keeps the
// gesture under test as a real element click and routes only its reads
// through the page. Here even the gesture cannot be a real click, and
// not because of the tax: WKWebView opens a native popup for a
// `<select>`, and this driver's pointer input is a synthetic
// `dispatchEvent` (see `card-trash.spec.ts`'s header) which cannot
// reach a native menu at all. So the option is chosen the way the
// engine itself would leave things after a pick — assign `value`,
// dispatch `input` + `change` — and the honest reading of a green run
// is: the app reorders the grid when the picker's value changes. It
// does not vouch for the popup being reachable with a mouse.
//
// # What this does not assert
//
// The band inputs (`duration_min_ms` and friends). Those are a filter,
// not an ordering, they live in a different sidebar section, and the
// HTTP path already covers them. This file is about the two options in
// the Sort dropdown.

import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

// --- budgets -------------------------------------------------------
//
// Same shape and the same reasoning as `card-trash.spec.ts`: every
// await is raced against a ceiling so the *first* step to stall names
// itself, instead of mocha's 300 s clock reporting that time ran out
// somewhere. The floors are generous because nothing waits for its
// ceiling on a healthy run, and a ceiling below a step's real cost is
// what produced the 2026-08-01 flake.

/** One driver round-trip. */
const DRIVER_MS = 15_000;
/** Something already on screen has to be found. */
const PRESENT_MS = 15_000;
/** The grid has to repaint after a filter change or a re-sort. */
const GRID_MS = 20_000;
/** A cold start: the window, the SQLite open, the first page load. */
const COLD_MS = 60_000;
/** Two identical reads of the grid's card order, this far apart, count
 *  as "settled". Long enough to sit outside the filter-reload debounce
 *  (`App.svelte` `FILTER_RELOAD_DEBOUNCE_MS`), so a repaint in flight
 *  cannot be mistaken for a finished one. */
const SETTLE_GAP_MS = 400;
/** Gap between `execute`-based polls — the untaxed commands. */
const POLL_GAP_MS = 250;

// --- the fixture ---------------------------------------------------

/** One seeded row. `null` is a state under test, not a placeholder: a
 *  still image has no playback length, an original whose bytes were
 *  never recorded has no size, and a text note has no dimensions. All
 *  three must tail in both directions. */
interface FixtureRow {
  readonly name: string;
  readonly durationMs: number | null;
  readonly sizeBytes: number | null;
  /** Coded pixel pair, or `null` for a row nothing measured.
   *
   *  **Every pair is non-square.** The two columns are independent
   *  `Option<u32>`s end to end, so nothing in the types stops a
   *  transposed read or write; a square fixture would report the same
   *  product either way round and check nothing. */
  readonly pixels: readonly [number, number] | null;
  readonly occurredMs: number;
}

/**
 * Five rows, from `sorted_list_e2e.rs`'s `metric axes` fixture, plus a
 * resolution column this side adds.
 *
 * Read the columns against each other rather than down: `feature` is
 * the longest, the *smallest* file and the second largest picture;
 * `clip` is the largest file and the smallest picture. That is the
 * disagreement the axes are being tested for, and the first `it` below
 * refuses to run if a later edit takes it away — pairwise, so a fourth
 * axis has to face the same bar.
 */
const FIXTURE: readonly FixtureRow[] = [
  // `brief` is the row with no dimensions. Deliberately *not* `still` or
  // `unsized`: each axis's gap sits on a different row, so tailing the
  // unmeasured one on one axis can never be satisfied by another axis's
  // gap. `feature` is the longest, the smallest file and the second
  // largest picture — the disagreement all three axes are tested for.
  {
    name: "brief",
    durationMs: 1_000,
    sizeBytes: 2_000_000,
    pixels: null,
    occurredMs: 1_550_000_000_000,
  },
  {
    name: "feature",
    durationMs: 120_000,
    sizeBytes: 500_000,
    pixels: [1_920, 1_080],
    occurredMs: 1_550_000_001_000,
  },
  {
    name: "clip",
    durationMs: 30_000,
    sizeBytes: 9_000_000,
    pixels: [640, 480],
    occurredMs: 1_550_000_002_000,
  },
  {
    name: "still",
    durationMs: null,
    sizeBytes: 7_000_000,
    pixels: [4_000, 3_000],
    occurredMs: 1_550_000_003_000,
  },
  {
    name: "unsized",
    durationMs: 60_000,
    sizeBytes: null,
    pixels: [1_280, 720],
    occurredMs: 1_550_000_004_000,
  },
];

// The five orders, written out rather than computed. Deriving them here
// would mean re-implementing the comparator in the test and then
// checking the app against that re-implementation — the assertion would
// survive any change the two made together. These are literals, they
// match what the HTTP path produced for the same rows, and the fixture
// guard below checks them back against the table.

/** occurred_at DESC — what the grid shows on the default axis, and what
 *  an axis that quietly did nothing would leave behind. */
const ARRIVAL = ["unsized", "still", "clip", "feature", "brief"];
/** Length, natural direction. `still` has none and tails. */
const LENGTH_LONGEST_FIRST = ["feature", "unsized", "clip", "brief", "still"];
/** Reversed. `still` tails *again* — the absent case is not multiplied
 *  by the direction, or "Shortest first" would open on a still image. */
const LENGTH_SHORTEST_FIRST = ["brief", "clip", "unsized", "feature", "still"];
/** Size, natural direction. `unsized` has none and tails. */
const SIZE_LARGEST_FIRST = ["clip", "still", "brief", "feature", "unsized"];
/** Reversed, `unsized` still tailing. */
const SIZE_SMALLEST_FIRST = ["feature", "brief", "still", "clip", "unsized"];
/** Resolution, natural direction — ordered by the **product**, which is
 *  the one reading that survives a rotation (the columns hold coded
 *  dimensions, taken before orientation). `brief` has none and tails. */
const PIXELS_LARGEST_FIRST = ["still", "feature", "unsized", "clip", "brief"];
/** Reversed, `brief` still tailing. */
const PIXELS_SMALLEST_FIRST = ["clip", "unsized", "feature", "still", "brief"];

/** Natural key of the fixture persona. Unique when present, so finding
 *  it is how a second run knows not to register another. */
const PACK_ID = "e2e-metric-axes";
const PERSONA_NAME = "E2E Metric Axes";

/** Cover text is the fixture's handle: it survives on the card
 *  (`AssetCardDto.cover`), it is set at ingest (`cover_hint`, which
 *  makes the `cover_gen` job skip the row), and it reads on a
 *  screenshot. Prefixed so nothing else in a profile can collide. */
function coverOf(name: string): string {
  return `e2e-metric:${name}`;
}

// --- HTTP, for the fixture only ------------------------------------
//
// The port is the one `wdio.conf.ts` hands the app through
// `wdio:tauriServiceOptions.appArgs`. A run that cannot reach it is a
// run whose app never bound the port — usually because another core
// already holds it — and in a window that failure is only a warning
// (see the conf's note), so nothing else in the suite would notice. It
// is fatal here, and says so.

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
  duration_ms?: number | null;
  file_size_bytes?: number | null;
  /** The `Pixels` axis's key — the product, not the pair. The card
   *  never carries the two sides; see `AssetCardDto::pixel_count`. */
  pixel_count?: number | null;
}

interface PageDto {
  items: CardDto[];
}

/**
 * The repo root, found by walking up from the working directory until
 * the e2e profile dir is underfoot.
 *
 * Walked rather than assumed: `just ui-e2e` runs wdio from
 * `crates/asterism-ui`, but that is the recipe's business and not
 * something a spec should encode. The marker is the profile
 * `wdio.conf.ts` points the app at, so finding it also confirms the two
 * halves are talking about the same tree.
 */
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

/**
 * Writes the file a seeded row points at.
 *
 * The ingest path does not require the locator to exist — the hash runs
 * later, as a job — so this is not load-bearing for the assertions. It
 * is here because the rows outlive the run: a fixture asset pointing at
 * nothing would fail its hash job on every start from then on, and
 * someone would eventually have to work out why.
 *
 * Contents differ per row so the duplicate detector never folds two of
 * them together.
 */
function writeFixtureFile(root: string, row: FixtureRow): string {
  const dir = path.join(root, "workspace/runtime/e2e-fixtures/metric-sort");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `${row.name}.md`);
  if (!fs.existsSync(file)) {
    fs.writeFileSync(
      file,
      `# ${row.name}\n\nFixture row for e2e/metric-sort.spec.ts.\n` +
        `duration_ms=${row.durationMs} file_size_bytes=${row.sizeBytes}\n`,
      "utf8",
    );
  }
  return file;
}

/**
 * Brings the profile to the state the assertions need, and returns
 * `asset id → fixture name`.
 *
 * Additive by construction: find, verify, restore if trashed, create
 * only what is missing. The one thing it refuses to do is repair a row
 * that disagrees with the table — see the file header.
 */
async function ensureFixture(): Promise<Map<string, string>> {
  await api<unknown>("GET", "/asterism/health").catch((err) => {
    throw new Error(
      `the app is not serving HTTP on ${BASE_URL} (${String(err)}). ` +
        "The fixture is seeded over that port, so this run cannot continue. " +
        "A bind failure in a window is only a warning — check whether another " +
        "core already holds the port.",
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
  const idToName = new Map<string, string>();

  for (const row of FIXTURE) {
    const cover = coverOf(row.name);
    const seen = live.items.find((c) => c.cover === cover);
    if (seen) {
      const carried = {
        duration_ms: seen.duration_ms ?? null,
        file_size_bytes: seen.file_size_bytes ?? null,
        pixel_count: seen.pixel_count ?? null,
      };
      const wanted = {
        duration_ms: row.durationMs,
        file_size_bytes: row.sizeBytes,
        // The card carries the product; the seed states the pair.
        pixel_count: row.pixels ? row.pixels[0] * row.pixels[1] : null,
      };
      if (JSON.stringify(carried) !== JSON.stringify(wanted)) {
        throw new Error(
          `the e2e profile already holds a fixture row "${row.name}" (asset ${seen.id}) ` +
            `carrying ${JSON.stringify(carried)}, but this spec's table says ` +
            `${JSON.stringify(wanted)}. Nothing here rewrites it: delete that asset (or the ` +
            `"${PERSONA_NAME}" persona) and re-run to reseed.`,
        );
      }
      idToName.set(seen.id, row.name);
      continue;
    }

    // Missing from the live side. It may be a row a failed run left in
    // the trash — restoring it is right, and adding a second copy of it
    // would not be.
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
      idToName.set(buried.id, row.name);
      continue;
    }

    const created = await api<{ id: string }>("POST", "/asterism/assets/add", {
      persona_id: persona.id,
      source_kind: "fs",
      locator: writeFixtureFile(root, row),
      // Unclassified. The axes are orthogonal to modality by design
      // — a length is a fact about the material, not a
      // claim that the row is a video — and leaving it off keeps the
      // fixture out of the sidebar's modality counts.
      modality: null,
      occurred_at_ms: row.occurredMs,
      labels: ["e2e-metric-fixture"],
      register_note: null,
      platform: null,
      file_size_bytes: row.sizeBytes,
      duration_ms: row.durationMs,
      // Stated at ingest, exactly as the importers state it. The
      // fixture files are markdown, so the startup dimension walk
      // measures nothing in them — and that is harmless here precisely
      // because that pass writes under `FillOnly`, which leaves a value
      // already on the row alone. A pass that overwrote would erase
      // this fixture on the next launch.
      width_px: row.pixels ? row.pixels[0] : null,
      height_px: row.pixels ? row.pixels[1] : null,
      extra_json: null,
      cover_hint: cover,
    });
    idToName.set(created.id, row.name);
  }

  return idToName;
}

// --- screenshots, stages, polling ----------------------------------
//
// Same three helpers as `card-trash.spec.ts`, and duplicated rather
// than shared for the reason each spec in this directory is
// self-contained: extracting them would mean editing that file, whose
// budgets and failure modes are documented against its own steps.

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
          `${String(shotSeq).padStart(3, "0")}_ms_${failed ? "FAIL_" : ""}${safe}.png`,
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

// --- page reads ----------------------------------------------------
//
// Every callback below is an anonymous arrow in argument position,
// takes selectors in and returns data out, and returns rather than
// throws on a fault path. All three are load-bearing under this driver;
// the reasons are in `card-trash.spec.ts` (`openCardMenu`, `readDom`),
// and the `__name` shim installed after each load is the other half of
// the first one.

const NAME_SHIM = "window.__name = window.__name || function (target) { return target; };";

interface PickerOption {
  value: string;
  label: string;
}

interface Picker {
  /** Two selects = the axis picker is on screen. Zero = the panel is
   *  showing one of its "something else owns the order" notices
   *  (🎲 Shuffle / ⌕ Relevance), which is a different state and not one
   *  any assertion here can be read against. */
  selects: number;
  targetValue: string;
  targetOptions: PickerOption[];
  orderValue: string;
  orderOptions: PickerOption[];
}

const ABSENT_PICKER: Picker = {
  selects: 0,
  targetValue: "",
  targetOptions: [],
  orderValue: "",
  orderOptions: [],
};

async function readPicker(): Promise<Picker> {
  return browser
    .execute((query: string) => {
      // Both selects read inline rather than through a shared local
      // helper: a `const` holding an arrow is nameable, and esbuild's
      // name preservation turns those into `__name(fn, "fn")` inside a
      // script this driver sends as a *string* (see the note above).
      const found = document.querySelectorAll(query);
      const target = found[0];
      const order = found[1];
      return {
        selects: found.length,
        targetValue: target instanceof HTMLSelectElement ? target.value : "",
        targetOptions:
          target instanceof HTMLSelectElement
            ? Array.from(target.options).map((o) => ({
                value: o.value,
                label: (o.textContent ?? "").replace(/\s+/g, " ").trim(),
              }))
            : [],
        orderValue: order instanceof HTMLSelectElement ? order.value : "",
        orderOptions:
          order instanceof HTMLSelectElement
            ? Array.from(order.options).map((o) => ({
                value: o.value,
                label: (o.textContent ?? "").replace(/\s+/g, " ").trim(),
              }))
            : [],
      };
    }, ".sort-picker select")
    .catch(() => ABSENT_PICKER);
}

/**
 * Chooses an option the way the engine leaves a `<select>` after a
 * native pick: assign, then `input` + `change`. Svelte's `bind:value`
 * listens for `change` (the axis select) and the direction select is a
 * plain `onchange` handler, so both are driven by the same two events.
 *
 * Returns `false` when the option is not on the list — which is a
 * finding, not a retry condition, so the caller asserts on it.
 */
async function chooseOption(at: number, value: string): Promise<boolean> {
  return browser
    .execute(
      (query: string, index: number, wanted: string) => {
        const el = document.querySelectorAll(query)[index];
        if (!(el instanceof HTMLSelectElement)) return false;
        if (!Array.from(el.options).some((o) => o.value === wanted)) return false;
        el.value = wanted;
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
        return el.value === wanted;
      },
      ".sort-picker select",
      at,
      value,
    )
    .catch(() => false);
}

/** The grid's card ids, in the order the DOM carries them. */
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

interface Sidebar {
  present: boolean;
  fixtureRowPresent: boolean;
  fixtureRowActive: boolean;
  /** The marker `markPage()` leaves on `window`. It is what tells a
   *  reload apart from a reload that has not started yet: the document
   *  standing in front of the driver a moment after `location.reload()`
   *  is still the old one, sidebar and all, so waiting on the sidebar
   *  alone would pass without anything having happened. */
  marked: boolean;
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
        marked:
          (window as unknown as { __metricSortMark?: boolean }).__metricSortMark === true,
      };
    }, personaRowSelector(personaId))
    .catch(() => ({
      present: false,
      fixtureRowPresent: false,
      fixtureRowActive: false,
      marked: false,
    }));
}

/** Stamps the current document so the reload below can be observed
 *  rather than assumed. A fresh document carries no such property. */
async function markPage(): Promise<void> {
  await browser
    .execute(() => {
      (window as unknown as { __metricSortMark?: boolean }).__metricSortMark = true;
    })
    .catch(() => undefined);
}

function personaRowSelector(personaId: string): string {
  return `aside.sidebar li.persona-row[data-persona-id="${personaId}"]`;
}

/** Clicks a sidebar row's own button (the first one — `.persona-info`
 *  is the ⓘ that opens the profile card and must not be hit). */
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

/** Clears the persona filter through the list's own "● all" row —
 *  found relative to a real persona row, so it cannot pick up the
 *  first entry of one of the sidebar's other lists. */
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

/** `asset id → fixture name`, filled by `before`. */
let idToName = new Map<string, string>();

/** The grid's order, in fixture names. Ids the fixture does not know
 *  come back as `?<prefix>` rather than being dropped: an unexpected
 *  card is something an assertion should show, not hide. */
function asNames(ids: readonly string[]): string[] {
  return ids.map((id) => idToName.get(id) ?? `?${id.slice(0, 8)}`);
}

/**
 * Waits for the card order to stop moving, then reports it in fixture
 * names.
 *
 * Deliberately not "wait until it equals what I expect": that turns a
 * wrong order into a timeout with no diff, and the wrong order is
 * exactly what this file exists to show. Settling and then asserting
 * means a gutted comparator fails as
 * `expected [feature, …] received [unsized, …]` on the first attempt.
 *
 * "Settled" is two identical reads `SETTLE_GAP_MS` apart. On reaching
 * the deadline it hands back what it last saw rather than throwing, for
 * the same reason.
 */
async function settledOrder(trail: string[], name: string): Promise<string[]> {
  return stage(trail, name, GRID_MS + DRIVER_MS, async () => {
    let previous = "";
    const deadline = Date.now() + GRID_MS;
    for (;;) {
      const ids = await readGridIds();
      const shape = ids.join(",");
      if (ids.length > 0 && shape === previous) return asNames(ids);
      if (Date.now() >= deadline) return asNames(ids);
      previous = shape;
      await new Promise((resolve) => setTimeout(resolve, SETTLE_GAP_MS));
    }
  });
}

/** Picks an axis and a direction, confirming each landed before moving
 *  on — the direction select's option list is re-rendered when the axis
 *  changes, so setting both in one breath can address a stale list. */
async function pickSort(trail: string[], label: string, target: string, order: string) {
  const axisTook = await stage(trail, `${label}: pick axis ${target}`, DRIVER_MS, () =>
    chooseOption(0, target),
  );
  expect({ step: `${label}: axis option ${target} exists`, took: axisTook }).toEqual({
    step: `${label}: axis option ${target} exists`,
    took: true,
  });
  await pollUntil(
    trail,
    `${label}: axis reads ${target}`,
    PRESENT_MS,
    async () => (await readPicker()).targetValue === target,
    `the Sort select never settled on ${target}`,
  );

  const orderTook = await stage(trail, `${label}: pick order ${order}`, DRIVER_MS, () =>
    chooseOption(1, order),
  );
  expect({ step: `${label}: order option ${order} exists`, took: orderTook }).toEqual({
    step: `${label}: order option ${order} exists`,
    took: true,
  });
  await pollUntil(
    trail,
    `${label}: order reads ${order}`,
    PRESENT_MS,
    async () => (await readPicker()).orderValue === order,
    `the Order select never settled on ${order}`,
  );
}

describe("metric sort axes", () => {
  let personaId = "";

  before(async () => {
    const trail: string[] = [];

    // The window and the SQLite open are the slow half of a cold start.
    // Polled with `execute` so a script that cannot run yet reads as
    // "not ready" instead of costing a taxed `findElement` per poll.
    await stage(trail, "install __name shim", DRIVER_MS, () => browser.execute(NAME_SHIM));
    await pollUntil(
      trail,
      "app window paints",
      COLD_MS,
      async () => (await readSidebar("")).present,
      "the app never painted its sidebar",
    );

    idToName = await stage(trail, "seed fixture over HTTP", 60_000, () => ensureFixture());
    const personas = await api<PersonaDto[]>("GET", "/asterism/personas");
    personaId = personas.find((p) => p.pack_id === PACK_ID)?.id ?? "";
    expect(personaId).not.toBe("");

    // Reload, and reload *clean*. Two things ride on it: a persona
    // registered after startup is not in the sidebar the app already
    // painted, and the search string carries the whole filter state
    // (`url-adapter.ts`), so dropping it starts this spec from defaults
    // whatever ran before it in the session. Done on every run rather
    // than only when something was seeded, so the path a fresh profile
    // takes is the path every run takes.
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
        // Both halves: a fresh document (the mark is gone) that has
        // painted its shell. Either alone passes too early.
        return dom.present && !dom.marked;
      },
      "the app never came back with a fresh document after the reload",
    );
    await stage(trail, "reinstall __name shim", DRIVER_MS, () => browser.execute(NAME_SHIM));

    // Narrow to the fixture persona. Not tidiness: the grid is
    // virtualised, so over a whole profile the five rows are not all
    // guaranteed to be in the DOM at once, and an ordering read off a
    // window of a longer list is not the ordering.
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

    // The grid must hold the five fixture rows and nothing else. A
    // sixth card here means the profile carries something under this
    // persona that the table does not describe, and every ordering
    // assertion below would be about a set the spec cannot name.
    await pollUntil(
      trail,
      "grid holds the five fixture rows",
      GRID_MS,
      async () => {
        const ids = await readGridIds();
        return ids.length === FIXTURE.length && ids.every((id) => idToName.has(id));
      },
      "the grid never showed exactly the five fixture rows",
    );
  });

  after(async () => {
    // Leave the app the way the other specs expect to find it: the
    // default axis, and no persona filter. Swallowed on purpose —
    // cleanup must never replace the error that got us here.
    await chooseOption(0, "occurred_at").catch(() => false);
    await chooseOption(1, "updated:asc").catch(() => false);
    await clearPersonaFilter().catch(() => false);
  });

  it("keeps a fixture whose axes disagree with each other and with arrival order", () => {
    // The guard against the two vacuous assertions this repo has
    // already shipped (see the file header). It reads the table rather
    // than the app, so an edit that collapses two of the orders fails
    // here — loudly, and before any of the grid assertions get the
    // chance to pass for the wrong reason.
    const orders = {
      arrival: ARRIVAL,
      lengthDesc: LENGTH_LONGEST_FIRST,
      lengthAsc: LENGTH_SHORTEST_FIRST,
      sizeDesc: SIZE_LARGEST_FIRST,
      sizeAsc: SIZE_SMALLEST_FIRST,
      pixelsDesc: PIXELS_LARGEST_FIRST,
      pixelsAsc: PIXELS_SMALLEST_FIRST,
    };
    const names = FIXTURE.map((r) => r.name);

    // Each order is a permutation of the fixture, and no two of the
    // five are the same sequence.
    const entries = Object.entries(orders);
    for (const [label, order] of entries) {
      expect({ order: label, rows: [...order].sort() }).toEqual({
        order: label,
        rows: [...names].sort(),
      });
    }
    const distinct = new Set(entries.map(([, order]) => order.join(">")));
    expect({ orders: entries.length, distinctSequences: distinct.size }).toEqual({
      orders: entries.length,
      distinctSequences: entries.length,
    });

    // The three axes do not merely differ — **no row keeps its place**
    // between any pair of them. A fixture where two agreed about most
    // rows could let a broken axis pass on the strength of the ones they
    // share. Checked pairwise so adding a fourth axis has to face the
    // same bar rather than only the pair somebody remembered.
    for (const [label, a, b] of [
      ["length vs size", LENGTH_LONGEST_FIRST, SIZE_LARGEST_FIRST],
      ["length vs pixels", LENGTH_LONGEST_FIRST, PIXELS_LARGEST_FIRST],
      ["size vs pixels", SIZE_LARGEST_FIRST, PIXELS_LARGEST_FIRST],
    ] as const) {
      const held = a.filter((name, at) => b[at] === name);
      expect({ pair: label, rowsHoldingTheirPlace: held }).toEqual({
        pair: label,
        rowsHoldingTheirPlace: [],
      });
    }

    // The three gaps are on three different rows, so tailing the
    // unmeasured row on one axis can never be satisfied by another
    // axis's gap.
    const noLength = FIXTURE.filter((r) => r.durationMs === null).map((r) => r.name);
    const noSize = FIXTURE.filter((r) => r.sizeBytes === null).map((r) => r.name);
    const noPixels = FIXTURE.filter((r) => r.pixels === null).map((r) => r.name);
    expect({ noLength, noSize, noPixels }).toEqual({
      noLength: ["still"],
      noSize: ["unsized"],
      noPixels: ["brief"],
    });
    expect(new Set([noLength[0], noSize[0], noPixels[0]]).size).toBe(3);

    // Every measured pair is non-square. A square one would read the
    // same transposed, so a swapped write or read would pass every
    // ordering assertion below.
    const square = FIXTURE.filter((r) => r.pixels && r.pixels[0] === r.pixels[1]).map(
      (r) => r.name,
    );
    expect({ squareFixtures: square }).toEqual({ squareFixtures: [] });

    // And the literals agree with the table they claim to describe:
    // each direction is monotone over the rows that carry a value, with
    // the valueless one last.
    // The resolution axis reads the *product*, so its literal is checked
    // against `w * h` rather than against either column — an ordering
    // that happened to be monotone in width would not be the axis.
    const valueOf = (name: string, key: "durationMs" | "sizeBytes" | "pixels") => {
      const row = FIXTURE.find((r) => r.name === name);
      if (!row) return null;
      if (key !== "pixels") return row[key];
      return row.pixels ? row.pixels[0] * row.pixels[1] : null;
    };
    for (const [label, order, key] of [
      ["length longest first", LENGTH_LONGEST_FIRST, "durationMs"],
      ["length shortest first", LENGTH_SHORTEST_FIRST, "durationMs"],
      ["size largest first", SIZE_LARGEST_FIRST, "sizeBytes"],
      ["size smallest first", SIZE_SMALLEST_FIRST, "sizeBytes"],
      ["pixels largest first", PIXELS_LARGEST_FIRST, "pixels"],
      ["pixels smallest first", PIXELS_SMALLEST_FIRST, "pixels"],
    ] as const) {
      const values = order.map((name) => valueOf(name, key));
      const measured = values.filter((v): v is number => v !== null);
      const descending = label.includes("longest") || label.includes("largest");
      const monotone = measured.every(
        (v, at) => at === 0 || (descending ? measured[at - 1] > v : measured[at - 1] < v),
      );
      expect({ order: label, monotone, absentAtTail: values[values.length - 1] }).toEqual({
        order: label,
        monotone: true,
        absentAtTail: null,
      });
    }
  });

  it("offers Length, Size and Pixels in the Sort dropdown", async () => {
    const trail: string[] = [];
    const picker = await stage(trail, "read the sort picker", DRIVER_MS, () => readPicker());

    // Both selects on screen: the panel swaps them for a notice when a
    // draw or a search owns the order, and a missing option would then
    // mean "the picker is not here" rather than "the option is gone".
    expect({ selects: picker.selects }).toEqual({ selects: 2 });

    const axes = picker.targetOptions.filter(
      (o) => o.value === "duration" || o.value === "file_size" || o.value === "pixels",
    );
    // The order is the dropdown's, so this pins where a new axis lands
    // as well as that it is there.
    expect(axes).toEqual([
      { value: "duration", label: "Length" },
      { value: "file_size", label: "Size" },
      // "Pixels", not "Resolution": the axis orders on a count, and the
      // columns behind it are coded dimensions — "Resolution" invites
      // the 1920x1080 reading, which is the pair this axis does not
      // offer.
      { value: "pixels", label: "Pixels" },
    ]);
  });

  it("orders the grid by playback length, longest first, and not by arrival", async () => {
    const trail: string[] = [];
    // Establish the arrival order in this very window first, rather
    // than trusting the literal: "the axis produced something other
    // than arrival order" is only worth asserting if arrival order is
    // what the same five cards show on the default axis. Setting it
    // explicitly also makes the test independent of what the previous
    // one left the picker on.
    await pickSort(trail, "default", "occurred_at", "updated:asc");
    const arrival = await settledOrder(trail, "order under Occurred / Newest first");
    expect(arrival).toEqual(ARRIVAL);

    await pickSort(trail, "length", "duration", "updated:asc");
    const order = await settledOrder(trail, "order under Length / Longest first");

    // The direction is named in the option text, not inferred: the
    // metric axes borrow the `updated` wire token and rename both
    // choices, so a value alone does not say which way it goes.
    const picker = await readPicker();
    expect(picker.orderOptions).toEqual([
      { value: "updated:asc", label: "Longest first" },
      { value: "updated:desc", label: "Shortest first" },
    ]);

    expect(order).toEqual(LENGTH_LONGEST_FIRST);
    // The vacuity check, asserted rather than implied: an axis that did
    // nothing would leave the arrival order in place, and that is what
    // both of this repo's earlier ordering mistakes looked like.
    expect({ isArrivalOrder: order.join(">") === arrival.join(">") }).toEqual({
      isArrivalOrder: false,
    });
    // The row with no measured length tails rather than reading as zero.
    expect(order[order.length - 1]).toBe("still");
  });

  it("reverses to shortest first, with the unmeasured row still at the tail", async () => {
    const trail: string[] = [];
    await pickSort(trail, "length", "duration", "updated:asc");
    const longest = await settledOrder(trail, "order under Longest first");
    await pickSort(trail, "length reversed", "duration", "updated:desc");
    const shortest = await settledOrder(trail, "order under Shortest first");

    expect(shortest).toEqual(LENGTH_SHORTEST_FIRST);
    // Not merely "reversed": the tail row is in the same place in both
    // directions, which is what stops "Shortest first" opening on a
    // still image. So the two orders are each other's mirror over the
    // measured rows only.
    expect({
      tail: shortest[shortest.length - 1],
      measuredMirrored:
        shortest.slice(0, -1).join(">") === longest.slice(0, -1).reverse().join(">"),
      isArrivalOrder: shortest.join(">") === ARRIVAL.join(">"),
    }).toEqual({ tail: "still", measuredMirrored: true, isArrivalOrder: false });
  });

  it("orders the grid by stored size, largest first, and not by arrival", async () => {
    const trail: string[] = [];
    await pickSort(trail, "size", "file_size", "updated:asc");
    const order = await settledOrder(trail, "order under Size / Largest first");

    const picker = await readPicker();
    expect(picker.orderOptions).toEqual([
      { value: "updated:asc", label: "Largest first" },
      { value: "updated:desc", label: "Smallest first" },
    ]);

    expect(order).toEqual(SIZE_LARGEST_FIRST);
    expect({
      isArrivalOrder: order.join(">") === ARRIVAL.join(">"),
      // And not the length order either: the two axes are a
      // derangement of each other in this fixture, so an axis wired to
      // the wrong column lands here rather than in a green run.
      isLengthOrder: order.join(">") === LENGTH_LONGEST_FIRST.join(">"),
      tail: order[order.length - 1],
    }).toEqual({ isArrivalOrder: false, isLengthOrder: false, tail: "unsized" });
  });

  it("reverses to smallest first, with the sizeless row still at the tail", async () => {
    const trail: string[] = [];
    await pickSort(trail, "size", "file_size", "updated:asc");
    const largest = await settledOrder(trail, "order under Largest first");
    await pickSort(trail, "size reversed", "file_size", "updated:desc");
    const smallest = await settledOrder(trail, "order under Smallest first");

    expect(smallest).toEqual(SIZE_SMALLEST_FIRST);
    expect({
      tail: smallest[smallest.length - 1],
      measuredMirrored:
        smallest.slice(0, -1).join(">") === largest.slice(0, -1).reverse().join(">"),
      isArrivalOrder: smallest.join(">") === ARRIVAL.join(">"),
    }).toEqual({ tail: "unsized", measuredMirrored: true, isArrivalOrder: false });
  });

  it("orders the grid by pixel count, largest first, and not by the other axes", async () => {
    const trail: string[] = [];
    await pickSort(trail, "pixels", "pixels", "updated:asc");
    const order = await settledOrder(trail, "order under Pixels / Largest first");

    // "Largest" and "Smallest", the same words the size axis uses:
    // "Highest" would suggest a vertical measurement and "Widest" would
    // name one coded side, which is the reading this axis exists to
    // avoid.
    const picker = await readPicker();
    expect(picker.orderOptions).toEqual([
      { value: "updated:asc", label: "Largest first" },
      { value: "updated:desc", label: "Smallest first" },
    ]);

    expect(order).toEqual(PIXELS_LARGEST_FIRST);
    // Against all three of the orders it could have been mistaken for.
    // The fixture is a derangement across the axes, so an axis wired to
    // the wrong column lands here rather than in a green run — and
    // `pixel_count` is a *derived* value on the row, so a projection
    // that forgot to compute it would leave every card comparing absent
    // and answer in arrival order.
    expect({
      isArrivalOrder: order.join(">") === ARRIVAL.join(">"),
      isLengthOrder: order.join(">") === LENGTH_LONGEST_FIRST.join(">"),
      isSizeOrder: order.join(">") === SIZE_LARGEST_FIRST.join(">"),
      tail: order[order.length - 1],
    }).toEqual({
      isArrivalOrder: false,
      isLengthOrder: false,
      isSizeOrder: false,
      // The row nothing measured, tailing rather than reading as zero.
      tail: "brief",
    });
  });

  it("reverses to smallest first, with the dimensionless row still at the tail", async () => {
    const trail: string[] = [];
    await pickSort(trail, "pixels", "pixels", "updated:asc");
    const largest = await settledOrder(trail, "order under Pixels / Largest first");
    await pickSort(trail, "pixels reversed", "pixels", "updated:desc");
    const smallest = await settledOrder(trail, "order under Pixels / Smallest first");

    expect(smallest).toEqual(PIXELS_SMALLEST_FIRST);
    // `brief` holds the tail in *both* directions. A stand-in `0` would
    // park it at one end and flip it to the other here, which is the
    // failure the three-valued reading exists to prevent — and the one
    // this axis is most exposed to, since a measured `0 x 0` is a legal
    // value the column can hold.
    expect({
      tail: smallest[smallest.length - 1],
      measuredMirrored:
        smallest.slice(0, -1).join(">") === largest.slice(0, -1).reverse().join(">"),
      isArrivalOrder: smallest.join(">") === ARRIVAL.join(">"),
    }).toEqual({ tail: "brief", measuredMirrored: true, isArrivalOrder: false });
  });
});
