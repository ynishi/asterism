/**
 * Chapter bands in the detail pane — the one surface that can say a
 * chapter reached the screen.
 *
 * What this file claims: an asset whose material carries a band of
 * chapters opens a pane that lists those sections, in the band's order,
 * with the extent each one states, under a caption naming whose band it
 * is — and, because the band is the person's own, with the affordances
 * that write into it.
 *
 * What it deliberately does not claim:
 *
 *   * **Nothing about the file's own declaration.** An `imported` band
 *     is minted by the scan job reading a container, and there is no
 *     route that creates one — deliberately, since a hand-made
 *     "imported" band would be a lie about where its contents came
 *     from. The read-only rendering of such a band (no compose box, no
 *     title fields, no delete) is asserted in `MaterialChapters.test.ts`
 *     against a mounted component, where the origin can simply be
 *     stated. What is left for this file is the half that a component
 *     test cannot reach: that the panel is wired into the pane at all,
 *     for a real asset, over the real IPC.
 *   * **Nothing about playback.** The fixture's bytes are a placeholder
 *     (see `writeFixtureFile`) — the `<audio>` element will not load it
 *     and the waveform will report itself unavailable. Neither is read
 *     by the chapter panel, which is driven by the asset's
 *     `duration_ms` and the band rows alone. Seeding real audio would
 *     add a dependency on the fixture generator for an assertion this
 *     spec does not make.
 *
 * Shape follows the other specs in this directory, and the helpers
 * (`snapStage` / `stage` / `pollUntil`, the `api` seeder, `repoRoot`)
 * are duplicated rather than shared for the reason `metric-sort.spec.ts`
 * gives: each file's budgets and failure modes are documented against
 * its own steps.
 *
 * Two conventions from `card-trash.spec.ts` are load-bearing here as
 * well. Every query goes through `browser.execute`, because the tauri
 * service charges roughly six seconds to each element command, and the
 * `__name` shim is reinstalled after every load or the driver's
 * stringified callbacks die on a `ReferenceError`.
 *
 * This spec runs second of four in the shared session (alphabetically,
 * after `card-trash`), so it owes the ones behind it a closed detail
 * pane and no persona filter — `after` restores both.
 */
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

// --- budgets -------------------------------------------------------
const DRIVER_MS = 15_000;
const PRESENT_MS = 15_000;
const GRID_MS = 20_000;
const COLD_MS = 60_000;
const SEED_MS = 60_000;
const POLL_GAP_MS = 250;

// --- selectors -----------------------------------------------------
const PANEL = ".chapter-panel";
const DETAIL_PANEL = ".detail-panel";
const DETAIL_CLOSE = ".detail-close";

// --- the fixture ---------------------------------------------------
const PACK_ID = "e2e-chapter-bands";
const PERSONA_NAME = "Chapter bands";
const COVER = "e2e-chapter-band-fixture";
/** Ten minutes. Only its sign matters to the panel (a material with no
 *  duration has no timeline to divide), but the ruler places ticks as a
 *  fraction of it, so it has to outlast the sections below. */
const DURATION_MS = 600_000;

interface FixtureChapter {
  startMs: number;
  endMs: number | null;
  label: string;
  ord: number;
}

/**
 * The band this spec writes, and the two shapes a section can take.
 *
 * The first states both ends. The second states only a start, which is
 * the case worth having on screen: `end_ms: null` means the section
 * declares no end, and the panel must print it as absent rather than
 * running it to the next section or to the end of the material.
 */
const CHAPTERS: FixtureChapter[] = [
  { startMs: 0, endMs: 90_000, label: "Cold open", ord: 0 },
  { startMs: 90_000, endMs: null, label: "Main theme", ord: 1 },
];

/** What the two rows above should read as, in the band's order. */
const EXPECTED_TIMES = ["0:00 – 1:30", "1:30"];
const EXPECTED_TITLES = ["Cold open", "Main theme"];

// --- seeding over the app's own HTTP -------------------------------
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
}

interface CardDto {
  id: string;
  cover: string | null;
  duration_ms?: number | null;
}

interface PageDto {
  items: CardDto[];
}

interface LayerDto {
  id: string;
  origin: string;
  role: string;
}

interface ChapterDto {
  id: string;
  start_ms: number;
  end_ms: number | null;
  label: string;
  ord: number;
}

interface LayerViewDto {
  layer: LayerDto;
  chapters: ChapterDto[];
}

/**
 * The repo root, found by walking up from the working directory until
 * the e2e profile dir is underfoot. Walked rather than assumed for the
 * reason `metric-sort.spec.ts` gives — where wdio is launched from is
 * the recipe's business.
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
 * Writes the file the seeded row points at.
 *
 * Named `.m4a` because the extension is what decides the row's mime,
 * and the mime is what decides that the pane shows a player at all
 * (`guess_mime` → `MimeType::Audio` → the audio branch, which is where
 * the chapter panel is mounted). The bytes themselves are a placeholder:
 * nothing this spec asserts decodes them, and the row is created with
 * its `duration_ms` stated rather than probed.
 *
 * It exists at all for the reason the other fixtures do — the rows
 * outlive the run, and an asset pointing at nothing fails its hash job
 * on every start from then on.
 */
function writeFixtureFile(root: string): string {
  const dir = path.join(root, "workspace/runtime/e2e-fixtures/chapter-band");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "chaptered.m4a");
  if (!fs.existsSync(file)) {
    fs.writeFileSync(
      file,
      "placeholder for e2e/chapter-band.spec.ts — not decodable audio; the row " +
        "states its own duration and the chapter panel reads the band, not the bytes\n",
      "utf8",
    );
  }
  return file;
}

/**
 * Brings the profile to the state the assertions need, and returns the
 * asset id.
 *
 * Additive by construction: find, restore if trashed, create only what
 * is missing. Like `metric-sort`, it refuses to repair a band that
 * disagrees with the table above — a fixture quietly rewritten is a
 * fixture that stops testing what its name says.
 */
async function ensureFixture(): Promise<string> {
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
  let assetId = live.items.find((c) => c.cover === COVER)?.id ?? "";

  if (assetId === "") {
    // Missing from the live side. It may be a row a failed run left in
    // the trash — restoring it is right, and adding a second copy of it
    // would not be.
    const trashed = (
      await api<PageDto>(
        "GET",
        `/asterism/assets?persona_id=${encodeURIComponent(persona.id)}&trash=trashed&limit=500`,
      )
    ).items;
    const buried = trashed.find((c) => c.cover === COVER);
    if (buried) {
      await api<unknown>("POST", "/asterism/assets/restore", { asset_id: buried.id });
      assetId = buried.id;
    } else {
      const created = await api<{ id: string }>("POST", "/asterism/assets/add", {
        persona_id: persona.id,
        source_kind: "fs",
        locator: writeFixtureFile(repoRoot()),
        modality: null,
        occurred_at_ms: 1_700_000_000_000,
        labels: ["e2e-chapter-fixture"],
        register_note: null,
        platform: null,
        file_size_bytes: 1_024,
        // Stated at ingest, as the importers state it. Without a
        // duration the material has no timeline and the panel — by
        // design — does not appear at all.
        duration_ms: DURATION_MS,
        width_px: null,
        height_px: null,
        extra_json: null,
        cover_hint: COVER,
      });
      assetId = created.id;
    }
  }

  const views = await api<LayerViewDto[]>(
    "GET",
    `/asterism/assets/${encodeURIComponent(assetId)}/material-layers`,
  );
  const mine = views.find(
    (v) => v.layer.role === "structure" && v.layer.origin === "user",
  );

  if (mine) {
    const carried = mine.chapters
      .map((c) => `${c.ord}:${c.start_ms}:${c.end_ms ?? "-"}:${c.label}`)
      .join("|");
    const wanted = CHAPTERS.map(
      (c) => `${c.ord}:${c.startMs}:${c.endMs ?? "-"}:${c.label}`,
    ).join("|");
    if (carried !== wanted) {
      throw new Error(
        `the e2e profile already holds a chapter band (layer ${mine.layer.id}) on the ` +
          `fixture asset carrying "${carried}", but this spec's table says "${wanted}". ` +
          `Nothing here rewrites it: delete that band (or the "${PERSONA_NAME}" persona) ` +
          "and re-run to reseed.",
      );
    }
    return assetId;
  }

  const layer = await api<LayerDto>(
    "POST",
    `/asterism/assets/${encodeURIComponent(assetId)}/material-layers`,
    { asset_id: assetId, material_ord: null, role: "structure", ord: 0 },
  );
  for (const c of CHAPTERS) {
    await api<ChapterDto>(
      "POST",
      `/asterism/material-layers/${encodeURIComponent(layer.id)}/chapter-marks`,
      {
        layer_id: layer.id,
        start_ms: c.startMs,
        end_ms: c.endMs,
        label: c.label,
        ord: c.ord,
      },
    );
  }
  return assetId;
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
          `${String(shotSeq).padStart(3, "0")}_cb_${failed ? "FAIL_" : ""}${safe}.png`,
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
// the reasons are in `card-trash.spec.ts`, and the `__name` shim
// installed after each load is the other half of the first one.

const NAME_SHIM = "window.__name = window.__name || function (target) { return target; };";

interface Sidebar {
  present: boolean;
  fixtureRowPresent: boolean;
  fixtureRowActive: boolean;
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
          (window as unknown as { __chapterBandMark?: boolean }).__chapterBandMark === true,
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
      (window as unknown as { __chapterBandMark?: boolean }).__chapterBandMark = true;
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
 *  found relative to a real persona row, so it cannot pick up the first
 *  entry of one of the sidebar's other lists. */
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

/** Opens the detail pane on one card. A plain click is what the pane
 *  answers to; the modifier arms are selection, and the swallow after a
 *  marquee or a drag is never armed by a synthetic event. */
async function openCard(assetId: string): Promise<boolean> {
  return browser
    .execute((query: string) => {
      const card = document.querySelector(query);
      if (card instanceof HTMLElement) {
        card.click();
        return true;
      }
      return false;
    }, `.grid-wrapper .card[data-asset-id="${assetId}"]`)
    .catch(() => false);
}

async function closeDetail(): Promise<boolean> {
  return browser
    .execute((query: string) => {
      const btn = document.querySelector(query);
      if (btn instanceof HTMLElement) {
        btn.click();
        return true;
      }
      return false;
    }, DETAIL_CLOSE)
    .catch(() => false);
}

interface Panel {
  detailPresent: boolean;
  panelPresent: boolean;
  /** Each band chip's caption. A band has no name of its own, so this
   *  is the pair `(origin, role)` as the surface renders it. */
  bands: string[];
  /** The extent each row states, in the band's order. */
  times: string[];
  /** Each row's title. A band one owns puts them in fields, so the
   *  value is read where the text would otherwise be. */
  titles: string[];
  /** Whether the surface that writes into a band is offered. */
  composePresent: boolean;
  /** The sentence shown in place of a list, when there is one. */
  note: string;
}

async function readPanel(): Promise<Panel> {
  return browser
    .execute(
      (panelQuery: string, detailQuery: string) => {
        const panel = document.querySelector(panelQuery);
        const rows = panel ? Array.from(panel.querySelectorAll(".chapter-row")) : [];
        return {
          detailPresent: document.querySelector(detailQuery) !== null,
          panelPresent: panel !== null,
          bands: panel
            ? Array.from(panel.querySelectorAll(".chapter-band")).map((el) =>
                (el.textContent ?? "").trim().replace(/\s+/g, " "),
              )
            : [],
          times: rows.map((row) => {
            const el = row.querySelector(".chapter-time");
            return (el && el.textContent ? el.textContent : "").trim();
          }),
          titles: rows.map((row) => {
            const input = row.querySelector("input.chapter-title-input");
            if (input instanceof HTMLInputElement) return input.value;
            const span = row.querySelector(".chapter-title");
            return (span && span.textContent ? span.textContent : "").trim();
          }),
          composePresent: panel ? panel.querySelector(".chapter-compose") !== null : false,
          note: panel
            ? Array.from(panel.querySelectorAll(".chapter-note"))
                .map((el) => (el.textContent ?? "").trim())
                .join(" ")
            : "",
        };
      },
      PANEL,
      DETAIL_PANEL,
    )
    .catch(() => ({
      detailPresent: false,
      panelPresent: false,
      bands: [] as string[],
      times: [] as string[],
      titles: [] as string[],
      composePresent: false,
      note: "",
    }));
}

// --- the run -------------------------------------------------------

describe("chapter bands in the detail pane", () => {
  let personaId = "";
  let assetId = "";

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

    assetId = await stage(trail, "seed fixture over HTTP", SEED_MS, () => ensureFixture());
    const personas = await api<PersonaDto[]>("GET", "/asterism/personas");
    personaId = personas.find((p) => p.pack_id === PACK_ID)?.id ?? "";
    expect(personaId).not.toBe("");

    // Reload, and reload *clean*: a persona registered after startup is
    // not in the sidebar the app already painted, and the search string
    // carries the whole filter state, so dropping it starts this spec
    // from defaults whatever ran before it in the session.
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

    // Narrow to the fixture persona. The grid is virtualised, so over a
    // whole profile the fixture card is not guaranteed to be in the DOM
    // at all, and a card that is not there cannot be clicked.
    await pollUntil(
      trail,
      "fixture persona row appears",
      GRID_MS,
      async () => (await readSidebar(personaId)).fixtureRowPresent,
      `the sidebar never listed the "${PERSONA_NAME}" persona`,
    );
    await stage(trail, "click the fixture persona", DRIVER_MS, () =>
      clickPersonaRow(personaId),
    );
    await pollUntil(
      trail,
      "fixture persona is active",
      GRID_MS,
      async () => (await readSidebar(personaId)).fixtureRowActive,
      "clicking the fixture persona row did not select it",
    );
    await pollUntil(
      trail,
      "grid holds the fixture card",
      GRID_MS,
      async () => (await readGridIds()).includes(assetId),
      "the grid never showed the fixture asset",
    );
  });

  after(async () => {
    // Leave the app the way the specs behind this one expect to find
    // it: no open pane over the grid, no persona filter. Swallowed on
    // purpose — cleanup must never replace the error that got us here.
    await closeDetail().catch(() => false);
    await clearPersonaFilter().catch(() => false);
  });

  it("lists the band's sections, with the extent each one states", async () => {
    const trail: string[] = [];
    try {
      await stage(trail, "open the fixture card", DRIVER_MS, () => openCard(assetId));
      await pollUntil(
        trail,
        "detail pane opens",
        PRESENT_MS,
        async () => (await readPanel()).detailPresent,
        "clicking the fixture card never opened the detail pane",
      );
      await pollUntil(
        trail,
        "chapter panel appears",
        PRESENT_MS,
        async () => (await readPanel()).panelPresent,
        "the detail pane never showed a chapter panel — the band is read over IPC " +
          "after the pane opens, so this is either the read failing or the panel " +
          "not being mounted on the audio body",
      );

      const panel = await stage(trail, "read the chapter panel", DRIVER_MS, () =>
        readPanel(),
      );

      // One object per claim group, so a failure prints the whole shape
      // rather than stopping at the first field.
      expect({
        times: panel.times,
        titles: panel.titles,
        note: panel.note,
      }).toEqual({
        // The second row states no end and reads as its start alone.
        // "1:30 – 10:00" here would mean the panel had invented an end
        // out of the material's duration.
        times: EXPECTED_TIMES,
        titles: EXPECTED_TITLES,
        // A band with rows in it shows no note; the two empty cases the
        // note distinguishes are asserted in `MaterialChapters.test.ts`.
        note: "",
      });

      // The band is the person's own, so it is captioned as such and
      // offers the surface that writes into it. `bands` is the whole
      // list on purpose: an unexpected second chip is something an
      // assertion should show, not hide.
      //
      // That the list is exactly `["Yours"]` rests on something this
      // spec does not state anywhere else, so it is stated here.
      // Ingest enqueues a `chapter_scan` for the row
      // (`asset_service.rs`), and that job is what mints an `imported`
      // band — a second chip, captioned "From the file". It does not,
      // because the seeded file is the placeholder text
      // `writeFixtureFile` writes: the probe answers `Unreadable`, the
      // handler leaves the material for a later pass, and no band is
      // written. So the exact match holds by way of the fixture's bytes
      // being undecodable, not by way of the scan being absent.
      //
      // The consequence for whoever changes the fixture: seeding real
      // audio here — a `chaptered.m4a` from `gen-test-fixtures.py`, say
      // — makes this assertion fail with both chips listed, and that
      // failure is correct. Extend the expectation to both rather than
      // reaching for the placeholder again. Their order is the listing
      // order (`material_ord`, `role`, `ord`, `id`), and since the two
      // would agree on the first three it falls to the ids — so read it
      // off the failure rather than predicting it here.
      expect({
        bands: panel.bands,
        composePresent: panel.composePresent,
      }).toEqual({
        bands: ["Yours"],
        composePresent: true,
      });
    } finally {
      await closeDetail().catch(() => false);
    }
  });
});
