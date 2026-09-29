// The card menu's "Copy to…" prompt, from the typed directory to the
// exporter's answer on screen.
//
// # What this is for
//
// Until #306 the prompt ran its own test on the directory — "starts
// with `/`" — and refused anything else with a toast of its own before
// a request was made. The exporter it stood in front of asks the
// platform instead (`resolve_output_dir`, `asterism-exporter-file`), so
// on Windows the app refused `C:\…`, a directory the exporter would
// have written to. The check is gone and the exporter's rule is the
// only one; this spec is what says the webview no longer has one of its
// own, and that a refusal still reaches the user in words.
//
// So the question each case asks is not "is this path good" — that is
// the exporter's to answer and its unit tests ask it — but **whose
// answer is on screen**. The UI's own refusal began "Dispatch aborted";
// the exporter's arrives through `pollDispatch` as
// `failed: backend rejected the request: …`. A case whose toast is the
// second one proves the request left the webview.
//
// # The cases, and what each platform says to them
//
// A Windows-shaped path is not absolute on unix, so on macOS the
// exporter refuses the drive and UNC forms — and that refusal, in the
// exporter's words and naming the path, is the pass condition: it could
// not exist if the webview had stopped the request. On Windows the same
// shapes are the ones that must succeed, so there the drive forms point
// at a real directory under the repo and are expected to finish; the
// UNC form names a host that does not resolve and is only asked to
// reach the exporter. Those expectations are what the Windows build
// owes; macOS cannot exercise them.
//
// A relative path is refused on every platform, and the exporter's
// message is what the user reads. A unix absolute path under the repo
// is the control: the whole dispatch runs and a file lands on disk.
// `~` forms are left out on purpose — the only directory they reach is
// the real home of whoever runs the suite.
//
// # Fixture
//
// One persona of its own and one asset pointing at a file this spec
// writes, both seeded over the app's loopback HTTP surface the way
// `metric-sort.spec.ts` seeds its rows, found rather than re-created on
// a second run. The persona filter is narrowed to it so the one card is
// in the (virtualised) grid. Every case copies that one selection, so
// the Snapshot each dispatch freezes is the same deduped one; the
// refused cases leave failed dispatch rows in the e2e profile, which is
// the cost the app's comment names. The directory the control writes
// is removed in `after`.
//
// # Environment
//
// The constraints `card-trash.spec.ts` documents hold here: every
// element command pays the window-focus tax, so every read and click is
// one untaxed `execute`; nothing inside an in-page callback carries a
// name; and every wait is bounded and names itself. The helpers are
// copied rather than imported, the convention in this directory.
//
// What this does not assert is that a pointer reaches the menu entry or
// the OK button — the clicks are synthetic, the same limit every spec
// here carries.

import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

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
          `${String(shotSeq).padStart(3, "0")}_co_${failed ? "FAIL_" : ""}${safe}.png`,
        ),
      ),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
  } catch {
    // Liveness aid only.
  }
}

/** One driver round-trip, sized for a taxed command. */
const DRIVER_MS = 15_000;
/** Something already on screen has to be found. */
const PRESENT_MS = 15_000;
/** The grid has to load after a reload or a filter change. */
const GRID_MS = 20_000;
/** The window and the SQLite open, paid by the first spec in a run. */
const COLD_MS = 60_000;
/** A dispatch has to be enqueued, picked up by the job worker, and
 *  polled to a terminal state. `pollDispatch` polls every 1.5 s. */
const DISPATCH_MS = 60_000;
/** Gap between `execute`-based polls. */
const POLL_GAP_MS = 250;
/** How many `contextmenu` dispatches one menu open gets. */
const MENU_ATTEMPTS = 4;

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
}

const PACK_ID = "e2e-copy-output-dir";
const PERSONA_NAME = "E2E Copy Output Dir";
const COVER = "e2e-copy-output-dir:source";

/** The repo root, found by walking up to the e2e profile dir — the
 *  same marker `metric-sort.spec.ts` walks to, and for its reason. */
function repoRoot(): string {
  let dir = process.cwd();
  for (;;) {
    if (fs.existsSync(path.join(dir, "workspace/runtime/e2e"))) return dir;
    const up = path.dirname(dir);
    if (up === dir) {
      throw new Error(
        `could not find a repo root above ${process.cwd()} holding workspace/runtime/e2e`,
      );
    }
    dir = up;
  }
}

/** Seeds (or finds) the persona and its one asset; returns both ids.
 *  The source file is written every run, since the copy reads it. */
async function ensureFixture(root: string): Promise<{ personaId: string; assetId: string }> {
  await api<unknown>("GET", "/asterism/health").catch((err) => {
    throw new Error(
      `the app is not serving HTTP on ${BASE_URL} (${String(err)}); the fixture is seeded ` +
        "over that port. A bind failure in a window is only a warning — check whether " +
        "another core already holds the port.",
    );
  });

  const dir = path.join(root, "workspace/runtime/e2e-fixtures/copy-output-dir");
  fs.mkdirSync(dir, { recursive: true });
  const source = path.join(dir, "source.md");
  fs.writeFileSync(source, "# source\n\nFixture row for e2e/copy-output-dir.spec.ts.\n", "utf8");

  const personas = await api<PersonaDto[]>("GET", "/asterism/personas");
  const persona =
    personas.find((p) => p.pack_id === PACK_ID) ??
    (await api<PersonaDto>("POST", "/asterism/personas/register", {
      name: PERSONA_NAME,
      pack_id: PACK_ID,
    }));
  const q = `persona_id=${encodeURIComponent(persona.id)}&limit=500`;

  const live = await api<{ items: CardDto[] }>("GET", `/asterism/assets?${q}`);
  const seen = live.items.find((c) => c.cover === COVER);
  if (seen) return { personaId: persona.id, assetId: seen.id };

  const trashed = await api<{ items: CardDto[] }>("GET", `/asterism/assets?${q}&trash=trashed`);
  const buried = trashed.items.find((c) => c.cover === COVER);
  if (buried) {
    await api<unknown>("POST", "/asterism/assets/restore", { asset_id: buried.id });
    return { personaId: persona.id, assetId: buried.id };
  }

  const created = await api<{ id: string }>("POST", "/asterism/assets/add", {
    persona_id: persona.id,
    source_kind: "fs",
    locator: source,
    modality: null,
    occurred_at_ms: Date.UTC(2026, 0, 1),
    labels: ["e2e-copy-output-dir-fixture"],
    register_note: null,
    platform: null,
    file_size_bytes: null,
    duration_ms: null,
    width_px: null,
    height_px: null,
    extra_json: null,
    cover_hint: COVER,
  });
  return { personaId: persona.id, assetId: created.id };
}

// --- page reads and gestures ---------------------------------------

const NAME_SHIM = "window.__name = window.__name || function (target) { return target; };";

function personaRowSelector(personaId: string): string {
  return `aside.sidebar li.persona-row[data-persona-id="${personaId}"]`;
}

interface Sidebar {
  present: boolean;
  rowPresent: boolean;
  rowActive: boolean;
  marked: boolean;
}

async function readSidebar(personaId: string): Promise<Sidebar> {
  return browser
    .execute((query: string) => {
      const row = document.querySelector(query);
      const btn = row ? row.querySelector("button") : null;
      return {
        present: document.querySelector("aside.sidebar") !== null,
        rowPresent: row !== null,
        rowActive: btn !== null && btn.classList.contains("active"),
        marked:
          (window as unknown as { __copyOutputDirMark?: boolean }).__copyOutputDirMark === true,
      };
    }, personaRowSelector(personaId))
    .catch(() => ({ present: false, rowPresent: false, rowActive: false, marked: false }));
}

async function clickSelector(query: string): Promise<boolean> {
  return browser
    .execute((q: string) => {
      const el = document.querySelector(q);
      if (el instanceof HTMLElement) {
        el.click();
        return true;
      }
      return false;
    }, query)
    .catch(() => false);
}

async function inPage(query: string): Promise<boolean> {
  return browser
    .execute((q: string) => document.querySelector(q) !== null, query)
    .catch(() => false);
}

/** The toast's text, or `null` when no toast is up. */
async function readToast(): Promise<string | null> {
  return browser
    .execute(() => {
      const el = document.querySelector(".dispatch-toast");
      return el ? (el.textContent ?? "").replace(/\s+/g, " ").trim() : null;
    })
    .catch(() => null);
}

/** Opens the card menu on the fixture card. Retried because a
 *  `contextmenu` dispatched while the grid re-renders can land on a node
 *  that is on its way out — `card-trash.spec.ts` diagnoses that case. */
async function openCardMenu(trail: string[], label: string, assetId: string): Promise<void> {
  await stage(trail, `${label}: open the card menu`, DRIVER_MS * MENU_ATTEMPTS, async () => {
    for (let attempt = 1; attempt <= MENU_ATTEMPTS; attempt++) {
      await browser.execute((q: string) => {
        const card = document.querySelector(q);
        if (card) {
          card.dispatchEvent(
            new MouseEvent("contextmenu", {
              bubbles: true,
              cancelable: true,
              clientX: 120,
              clientY: 120,
            }),
          );
        }
      }, `.grid-wrapper .card[data-asset-id="${assetId}"]`);
      const deadline = Date.now() + 3_000;
      while (Date.now() < deadline) {
        if (await inPage(".card-menu")) return;
        await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
      }
    }
    throw new Error(`no .card-menu after ${MENU_ATTEMPTS} contextmenu dispatches`);
  });
}

/** Clicks the menu's "Copy to…" entry. Returns why it could not, or
 *  `null` when it did. The entry is disabled while a dispatch is still
 *  being polled, so the caller retries until it is not. */
async function clickCopyTo(): Promise<string | null> {
  return browser
    .execute(() => {
      const items = Array.from(document.querySelectorAll(".card-menu .card-menu-item"));
      const entry = items.find((el) => (el.textContent ?? "").includes("Copy to"));
      if (!(entry instanceof HTMLButtonElement)) return "no Copy to… entry in the card menu";
      if (entry.disabled) return "the Copy to… entry is disabled";
      entry.click();
      return null;
    })
    .catch((err: unknown) => `execute failed: ${String(err)}`);
}

/** Types into the prompt the way `bind:value` reads it, then presses
 *  its OK. Returns what the input held when OK was pressed. */
async function answerPrompt(value: string): Promise<string | null> {
  return browser
    .execute((typed: string) => {
      const input = document.querySelector(".prompt-panel .prompt-input");
      const ok = document.querySelector(".prompt-panel .prompt-btn.primary");
      if (!(input instanceof HTMLInputElement) || !(ok instanceof HTMLElement)) return null;
      input.value = typed;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      const held = input.value;
      ok.click();
      return held;
    }, value)
    .catch(() => null);
}

// --- the cases -----------------------------------------------------

type Expect =
  /** The exporter refused it as not absolute, naming the marker. */
  | { kind: "refusedAsRelative" }
  /** The dispatch finished and a file is in `dir`. */
  | { kind: "done"; dir: string }
  /** Reached the exporter; its verdict is not this spec's to fix. */
  | { kind: "reachedExporter" };

interface Case {
  label: string;
  typed: string;
  /** A string the toast must carry for the answer to be this case's —
   *  the path's own distinctive segment. */
  marker: string;
  expect: Expect;
}

const onWindows = process.platform === "win32";

function cases(root: string, stamp: string): Case[] {
  const outRoot = path.join(root, "workspace/runtime/e2e-copy-out", stamp);
  const out: Case[] = [];

  // Drive + root, both separators. On Windows these are real
  // directories under the repo; elsewhere they are the shape alone.
  if (onWindows) {
    const drive = path.win32.join(outRoot, "drive-backslash");
    out.push({
      label: "Windows drive path, backslashes",
      typed: drive,
      marker: "drive-backslash",
      expect: { kind: "done", dir: drive },
    });
    const fwd = path.win32.join(outRoot, "drive-forward");
    out.push({
      label: "Windows drive path, forward slashes",
      typed: fwd.replace(/\\/g, "/"),
      marker: "drive-forward",
      expect: { kind: "done", dir: fwd },
    });
  } else {
    out.push({
      label: "Windows drive path, backslashes",
      typed: "C:\\asterism-e2e-drive-backslash\\out",
      marker: "asterism-e2e-drive-backslash",
      expect: { kind: "refusedAsRelative" },
    });
    out.push({
      label: "Windows drive path, forward slashes",
      typed: "C:/asterism-e2e-drive-forward/out",
      marker: "asterism-e2e-drive-forward",
      expect: { kind: "refusedAsRelative" },
    });
  }

  out.push({
    label: "UNC path",
    typed: "\\\\asterism-e2e-unc.invalid\\share\\out",
    marker: "asterism-e2e-unc.invalid",
    expect: onWindows ? { kind: "reachedExporter" } : { kind: "refusedAsRelative" },
  });

  out.push({
    label: "relative path",
    typed: "asterism-e2e-relative/out",
    marker: "asterism-e2e-relative",
    expect: { kind: "refusedAsRelative" },
  });

  if (!onWindows) {
    const unix = path.join(outRoot, "unix-absolute");
    out.push({
      label: "unix absolute path",
      typed: unix,
      marker: "unix-absolute",
      expect: { kind: "done", dir: unix },
    });
  }
  return out;
}

/** The UI's own refusal, as the removed check worded it. */
const UI_ABORT = "Dispatch aborted";
/** What `pollDispatch` shows for a dispatch the exporter refused. */
const EXPORTER_REFUSAL = "failed: backend rejected the request:";
const NOT_ABSOLUTE = "output_dir must be an absolute path";

describe("copy selection output directory", () => {
  const root = repoRoot();
  const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
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

    ({ personaId, assetId } = await stage(trail, "seed fixture over HTTP", 60_000, () =>
      ensureFixture(root),
    ));

    // Reload clean: a persona registered after startup is not in the
    // painted sidebar, and the URL carries the filter state of
    // whatever spec ran before this one.
    await browser
      .execute(() => {
        (window as unknown as { __copyOutputDirMark?: boolean }).__copyOutputDirMark = true;
      })
      .catch(() => undefined);
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
      async () => (await readSidebar(personaId)).rowPresent,
      `the sidebar never listed the "${PERSONA_NAME}" persona`,
    );
    await stage(trail, "click the fixture persona", DRIVER_MS, () =>
      clickSelector(`${personaRowSelector(personaId)} button`),
    );
    await pollUntil(
      trail,
      "fixture persona is active",
      GRID_MS,
      async () => (await readSidebar(personaId)).rowActive,
      "clicking the fixture persona row did not select it",
    );
    await pollUntil(
      trail,
      "fixture card is in the grid",
      GRID_MS,
      () => inPage(`.grid-wrapper .card[data-asset-id="${assetId}"]`),
      "the fixture card never appeared in the grid",
    );
  });

  after(async () => {
    // Close anything left open and clear the persona filter, then remove
    // what the control case wrote. Swallowed: cleanup must not replace
    // the error that got us here.
    await browser
      .execute(() => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      })
      .catch(() => undefined);
    await browser
      .execute(() => {
        const row = document.querySelector("aside.sidebar li.persona-row");
        const list = row ? row.parentElement : null;
        const all = list ? list.querySelector("li:not(.persona-row) button") : null;
        if (all instanceof HTMLElement) all.click();
      })
      .catch(() => undefined);
    try {
      fs.rmSync(path.join(root, "workspace/runtime/e2e-copy-out", stamp), {
        recursive: true,
        force: true,
      });
    } catch {
      // Best-effort.
    }
  });

  for (const c of cases(root, stamp)) {
    it(`${c.label}: ${JSON.stringify(c.typed)} goes to the exporter, not a webview check`, async () => {
      const trail: string[] = [];

      // A previous case's dispatch may still be in its poll; the entry
      // is disabled until it ends. Waiting on the entry itself, not on
      // a toast, is what makes the order of cases irrelevant.
      await openCardMenu(trail, c.label, assetId);
      const refusals = new Set<string>();
      await stage(trail, `${c.label}: click Copy to…`, DISPATCH_MS + DRIVER_MS, async () => {
        const deadline = Date.now() + DISPATCH_MS;
        for (;;) {
          const why = await clickCopyTo();
          if (why === null) return;
          refusals.add(why);
          if (Date.now() >= deadline) {
            throw new Error(`could not pick Copy to…: ${[...refusals].join("; ")}`);
          }
          await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
        }
      });

      await pollUntil(
        trail,
        `${c.label}: prompt opens`,
        PRESENT_MS,
        () => inPage(".prompt-panel .prompt-input"),
        "the directory prompt never opened",
      );
      const held = await stage(trail, `${c.label}: type and press OK`, DRIVER_MS, () =>
        answerPrompt(c.typed),
      );
      expect(held).toBe(c.typed);

      // Every toast seen from here on, in order, so a failure shows the
      // whole sequence rather than the last frame.
      const seen: string[] = [];
      let terminal: string | null = null;
      await pollUntil(
        trail,
        `${c.label}: the dispatch ends`,
        DISPATCH_MS,
        async () => {
          const text = await readToast();
          if (text !== null && seen[seen.length - 1] !== text) seen.push(text);
          if (text === null) return false;
          if (text.startsWith(UI_ABORT)) {
            terminal = text;
            return true;
          }
          const mine = text.includes(c.marker);
          if (text.startsWith("failed:") && mine) {
            terminal = text;
            return true;
          }
          // A `Done` counts only after this case's own "Dispatching"
          // toast: the previous case's terminal toast lingers for 6 s,
          // and `pollDispatch` holds "Dispatching" for at least 1.5 s.
          const begun = seen.some((t) => t.startsWith("Dispatching"));
          if (text.startsWith("Done") && begun && c.expect.kind !== "refusedAsRelative") {
            terminal = text;
            return true;
          }
          return false;
        },
        `no terminal toast for this case; toasts seen: ${JSON.stringify(seen)}`,
      );
      const final = terminal as string | null;

      // The one thing every case owes: the webview did not answer.
      expect({ case: c.label, uiAbortSeen: seen.some((t) => t.startsWith(UI_ABORT)) }).toEqual({
        case: c.label,
        uiAbortSeen: false,
      });
      // That the request was made is what each branch below asserts:
      // the exporter's refusal, or a finished dispatch, exists only for
      // a request that left the webview.
      switch (c.expect.kind) {
        case "refusedAsRelative":
          expect(final).toContain(EXPORTER_REFUSAL);
          expect(final).toContain(NOT_ABSOLUTE);
          expect(final).toContain(c.marker);
          break;
        case "done": {
          expect(final ?? "").toMatch(/^Done/);
          const files = fs.existsSync(c.expect.dir) ? fs.readdirSync(c.expect.dir) : [];
          expect({ case: c.label, dir: c.expect.dir, wroteSomething: files.length > 0 }).toEqual({
            case: c.label,
            dir: c.expect.dir,
            wroteSomething: true,
          });
          break;
        }
        case "reachedExporter":
          expect(final ?? "").toMatch(/^(Done|failed: backend rejected the request:)/);
          break;
      }
      await snapStage(`${c.label}: final toast`);
    });
  }
});
