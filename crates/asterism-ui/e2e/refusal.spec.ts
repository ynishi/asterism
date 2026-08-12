// A refusal that came from the backend, on screen, in the webview.
//
// The unit tests already own most of this. `lib/mutate.test.ts` proves
// that a rejected `invoke` reaches `undoToastCatalog.refuse` with the
// message the call site named and the detail `detailOf` shaped, and a
// component test can prove `UndoToast.svelte` renders that pair. What
// neither can prove is the one seam they both stand on: **that the
// value a refused Tauri command actually rejects with has the shape
// `detailOf` reads**. A unit test constructs that value itself, so it
// agrees with whatever its author believed the boundary produces.
//
// The boundary is not obvious. `UiError` is
// `#[serde(tag = "kind", content = "message")]` (`src-tauri/src/
// error.rs`), so what crosses is neither the `Display` string nor a
// struct with the variant's own prose — it is `{kind, message}` where
// `message` is the *inner* string, the `#[error("…: {0}")]` prefix
// having stayed on the Rust side. `detailOf` is written to that fact
// and reads `kind` for exactly one variant. If serde's shape changed,
// if the IPC layer wrapped the rejection, or if the plugin stringified
// it on the way through, every unit test in the package would stay
// green and the user would get a toast with no reason under it. This
// spec is the only place that asks the real backend.
//
// # Why `delete_dir`
//
// A provocation for this has to satisfy three things at once: it must
// be refused by the real backend rather than by a client-side guard, it
// must be a *destructive verb* (the issue's Verification asks for at
// least one), and it must change nothing when it is refused — because
// the suite runs against a profile on disk and a provocation that
// consumed fixture would be usable once.
//
// `DirRepo::delete` (`asterism-infra/src/sqlite/repo/dir.rs:241-288`)
// is all three. It refuses a non-empty dir with `Conflict("dir is not
// empty — move or delete its contents first")`, and it refuses it
// *before* the `DELETE` runs: the occupancy query and the delete are
// one call, and the non-empty verdict returns out of it. So the
// gesture is repeatable, and the fixture is exactly as large after this
// spec as before it. The alternatives are worse on the third count:
// trash and purge succeed, which is the point of them.
//
// # The fixture seeds itself, and the success path is the cleanup
//
// The dir under test is created by `before`, over the app's own
// loopback HTTP surface (`POST /asterism/dirs/create` — the same origin
// `metric-sort.spec.ts` gives its five assets): one parent, one child
// dir inside it. A single run on a fresh profile therefore goes green;
// a spec that needed a hand-staged profile, or a second run, would be
// reporting on the machine it happened to be on rather than on the
// code.
//
// Seeding buys the other half of the question for free. Deleting an
// **empty** dir *succeeds* — which is exactly what makes clicking ✕ on
// an unproven dir dangerous — so after the refusal is asserted, the
// second test runs the same gesture where it must succeed: the child
// (provably empty), then the emptied parent, asserting that success
// stays silent. That pair is the refusal's control group, and it is
// also the cleanup — the profile ends the run with the dirs it started
// with. `after` keeps an HTTP backstop for the mid-failure case, so a
// red run cannot leave rows behind for `drop-targets.spec.ts` to find
// on the next one.
//
// The non-emptiness is still proven from the DOM before the ✕ is
// clicked, seeded or not: the ids `before` holds are a claim about
// what it created, and the click is only ever made on a dir whose
// contents are on screen.
//
// # How the DOM proves a dir is not empty
//
// `GroupsSection.svelte` renders the tree flat: every row is an `li` in
// one `ul.tags-list` carrying its own `--depth`, and an expanded dir is
// followed by its children at `--depth + 1` (`dirNode`, :602-661).
// What renders one level in is exactly two lists — `dirChildren` (dirs
// whose `parent_id` is this one) and `groupsByDir` (groups whose
// `dir_id` is this one) — and when both are empty the disclosure
// renders one `li.dir-empty` saying "empty" instead.
//
// Those are the same two tables the backend counts: the occupancy query
// is `EXISTS(SELECT 1 FROM dir WHERE parent_id = ?) OR EXISTS(SELECT 1
// FROM bucket WHERE dir_id = ?)`. So a row on screen at `depth + 1`
// that is not the empty marker means a row on disk, and the refusal is
// certain before the click.
//
// The implication runs one way only, and this spec stays on the side
// where it holds. A dir that *looks* empty can still refuse: the
// listing hides groups belonging to a trashed persona, which
// `dir.rs:259-269` detects and answers with a different `Conflict`
// sentence. That is why an `li.dir-empty` disqualifies a candidate
// outright instead of being treated as "probably deletable" — and why
// the assertion below names the sentence it expects. A run that came
// back with the trashed-persona sentence instead would fail here, and
// that failure is a fact about the profile, not about this change.
//
// Only groups filed *in* the dir count. Nested child groups render one
// level further in and only when their parent group is expanded, so the
// search never expands a group — a `depth + 1` row is a direct child by
// construction.
//
// `data-dir-id` on the dir row exists for this spec (`GroupsSection
// .svelte`, note above `dirNode`): the id was already on the inner
// `.dir-name` button, but a row is what has to be read and clicked.
//
// # Environment
//
// Three constraints hold for everything in this directory, and
// `card-trash.spec.ts` is where they are documented in full:
//
//   1. **Every element command costs ~6 s** — the window-focus tax the
//      tauri service charges on `$`, `$$`, `findElement(s)` and
//      `elementClick`, and on every poll of a `waitForExist`.
//      `browser.execute` is untaxed. So all DOM reads here go through
//      one `execute` returning data, and the only element commands in
//      the file are the two clicks that are the gesture under test.
//   2. **Nothing in an in-page callback may carry a name** — no named
//      function, no class, no arrow assigned to a `const`. The specs
//      are compiled with esbuild name preservation and the service
//      sends callbacks to the driver as strings, so a nameable function
//      arrives in the page as `__name(fn, "fn")` with `__name`
//      undefined. `before` installs the shim for the same reason.
//   3. **Every wait is bounded and names itself** — `stage()` races each
//      step against a ceiling and carries the trail of steps that
//      already passed, so a stall is located in one run instead of
//      arriving as a bare mocha `Timeout`.
//
// `snapStage` / `stage` / `pollUntil` are copied rather than imported,
// which is the convention in this directory (`metric-sort.spec.ts`
// says why: each spec's budgets are documented against its own steps,
// and sharing them would mean editing a neighbour's).
//
// # What this does not assert
//
// That a *pointer* can reach the ✕ or the dismiss. The driver's click
// is a synthetic event (`tauri-plugin-wdio-webdriver`), which fires a
// handler regardless of hit-testing — the same limit every spec here
// carries. What it does assert is that the handler is wired, that the
// refusal survived the IPC boundary with the backend's own words, that
// nothing was deleted, and that the dismiss button takes the message
// away.
//
// Ordering: the specs run alphabetically in one session and this is the
// last of them, but it does not rely on that. It leaves no toast on
// screen, no modal open, collapses whatever it expanded, and removes
// the two dirs it seeded.

import { browser, $ } from "@wdio/globals";
import path from "node:path";

/**
 * Screenshot trail, as in `card-trash.spec.ts`: `wdio.conf.ts`
 * `onPrepare` exports a per-run dir and every completed `stage()` drops
 * one numbered PNG into it, so a run is legible from outside while it
 * happens and after it fails. Best-effort by design — a screenshot must
 * never be why a test fails, so it is raced against its own ceiling and
 * all errors are eaten.
 */
const SCREENS_DIR = process.env.E2E_SCREENS_DIR;
let shotSeq = 0;
async function snapStage(name: string, failed = false): Promise<void> {
  if (!SCREENS_DIR) return;
  shotSeq += 1;
  const safe = name.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 60);
  const file = path.join(
    SCREENS_DIR,
    `${String(shotSeq).padStart(3, "0")}_${failed ? "FAIL_" : ""}${safe}.png`,
  );
  try {
    await Promise.race([
      browser.saveScreenshot(file),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
  } catch {
    // Liveness aid only.
  }
}

// --- budgets -------------------------------------------------------
//
// Sized against mocha's 300 s per-test ceiling, and against the tax:
// generous enough that nothing waits for its ceiling on a healthy run,
// small enough that the first step to stall throws inside its own
// budget and names itself rather than letting mocha write the error.

/** One driver round-trip, sized for a *taxed* command so it is
 *  generous for the `execute` calls that make up most of the file. */
const DRIVER_MS = 15_000;
/** Something already on screen has to be found. */
const PRESENT_MS = 15_000;
/** A backend round trip has to complete and the result has to render:
 *  the IPC call, the SQLite occupancy query, the rejection, and the
 *  toast. Longer than `PRESENT_MS` because it includes all four. */
const ROUND_TRIP_MS = 20_000;
/** The window and the SQLite open, paid once by the first spec in a
 *  run — and by this one when it is run alone. */
const COLD_MS = 60_000;
/** Gap between `execute`-based polls. Short because those are untaxed. */
const POLL_GAP_MS = 250;

/**
 * Runs one step with a hard ceiling and records it on success.
 *
 * The ceiling is a `Promise.race`, the only bound available for a raw
 * driver call: neither `browser.execute` nor `.click()` carries a
 * timeout of its own. On failure the error names the step and lists the
 * steps that already passed, so one run locates a stall.
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

/**
 * Polls a condition built from `execute` calls, which cost nothing, so
 * the interval can be short and the ceiling honest. Replaces
 * `waitForExist` / `browser.waitUntil` wherever the condition is a DOM
 * read — both of those poll through `findElements`, and at ~6 s per
 * poll a two-poll wait already blows a 15 s budget.
 */
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
      if (await check()) {
        return;
      }
      if (Date.now() >= deadline) {
        throw new Error(`${message} (polled for ${ms}ms)`);
      }
      await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
    }
  });
}

/** One dir row, and what it currently discloses. */
interface DirRow {
  id: string;
  depth: number;
  /** Rows rendered one level in from this one, not counting the
   *  `empty` placeholder. `0` means either collapsed or empty — which
   *  of the two is what `emptyMarker` says. */
  childRows: number;
  /** The `li.dir-empty` placeholder, which renders only when the dir is
   *  expanded *and* holds neither a group nor a child dir. */
  emptyMarker: boolean;
}

interface SidebarSnapshot {
  present: boolean;
  dirs: DirRow[];
}

/**
 * Every dir row and its disclosure state, in one untaxed round trip.
 *
 * Scoped to the `ul.tags-list` that holds dir rows: `TagList.svelte`
 * renders a second list with the same class in the same `aside`, and a
 * document-order walk that ran across the boundary between them would
 * read one list's rows as another's children.
 *
 * "Selectors in, data out" — nothing crosses as an element reference,
 * so a re-render between two calls cannot turn a measurement into a
 * driver-side exception with no detail.
 */
async function readSidebar(): Promise<SidebarSnapshot> {
  return browser
    .execute(() => {
      const lists = Array.from(
        document.querySelectorAll<HTMLElement>("aside.sidebar ul.tags-list"),
      );
      const held = lists.filter((ul) => ul.querySelector(".dir-row") !== null)[0] ?? null;
      const rows =
        held === null
          ? []
          : Array.from(held.querySelectorAll<HTMLElement>(":scope > li")).map((li) => ({
              dirId: li.getAttribute("data-dir-id"),
              // Rows without the custom property (the "● all" row, the
              // error row) read as `NaN` and are treated as a boundary
              // below rather than as depth 0.
              depth: Number.parseInt(li.style.getPropertyValue("--depth"), 10),
              empty: li.classList.contains("dir-empty"),
            }));
      const dirs: DirRow[] = [];
      for (let i = 0; i < rows.length; i += 1) {
        const row = rows[i];
        if (row.dirId === null || Number.isNaN(row.depth)) continue;
        let childRows = 0;
        let emptyMarker = false;
        // The subtree is the run of following siblings deeper than this
        // row; direct children are the ones exactly one level in.
        for (let j = i + 1; j < rows.length; j += 1) {
          const next = rows[j];
          const depth = Number.isNaN(next.depth) ? -1 : next.depth;
          if (depth <= row.depth) break;
          if (depth !== row.depth + 1) continue;
          if (next.empty) emptyMarker = true;
          else childRows += 1;
        }
        dirs.push({ id: row.dirId, depth: row.depth, childRows, emptyMarker });
      }
      return {
        present: document.querySelector("aside.sidebar") !== null,
        dirs,
      };
    })
    .catch(() => ({ present: false, dirs: [] as DirRow[] }));
}

/** The dir row with this id, or `undefined`. Re-derived on every use
 *  rather than kept as a handle: a row is replaced whenever the dir
 *  listing reloads. */
function rowOf(snapshot: SidebarSnapshot, dirId: string): DirRow | undefined {
  return snapshot.dirs.filter((d) => d.id === dirId)[0];
}

function dirRowSelector(dirId: string) {
  return `aside.sidebar li.dir-row[data-dir-id="${dirId}"]`;
}

/** The refusal toast, read the same untaxed way. `role` and
 *  `aria-live` come along because "reaches the screen" for a message
 *  about something that did not happen means reaching a screen reader
 *  too, and those two attributes are the whole of that claim. */
interface RefusalSnapshot {
  present: boolean;
  role: string;
  live: string;
  message: string;
  detail: string;
  dismissPresent: boolean;
}

const ABSENT_REFUSAL: RefusalSnapshot = {
  present: false,
  role: "",
  live: "",
  message: "",
  detail: "",
  dismissPresent: false,
};

const DISMISS_REFUSAL = '.refusal-toast [aria-label="Dismiss this message"]';

async function readRefusal(): Promise<RefusalSnapshot> {
  return browser
    .execute((dismissQuery: string) => {
      const toast = document.querySelector(".refusal-toast");
      if (toast === null) {
        return {
          present: false,
          role: "",
          live: "",
          message: "",
          detail: "",
          dismissPresent: false,
        };
      }
      return {
        present: true,
        role: toast.getAttribute("role") ?? "",
        live: toast.getAttribute("aria-live") ?? "",
        message: (toast.querySelector(".refusal-toast-message")?.textContent ?? "")
          .replace(/\s+/g, " ")
          .trim(),
        // Absent when the backend gave no reason, which is a real state
        // (`detailOf` returns null) and reads as "" rather than
        // throwing — the assertion is about which of the two happened.
        detail: (toast.querySelector(".refusal-toast-detail")?.textContent ?? "")
          .replace(/\s+/g, " ")
          .trim(),
        dismissPresent: document.querySelector(dismissQuery) !== null,
      };
    }, DISMISS_REFUSAL)
    .catch(() => ABSENT_REFUSAL);
}

/**
 * Takes a refusal down through the page realm — the cleanup tool, not
 * the assertion path. The test dismisses with a driver click, because
 * that click is part of what the issue asks to be shown working; this
 * one only refuses to leave a sticky message on screen for whatever
 * runs next if something above it threw. A refusal has no timer
 * (`stores/undo-toast.svelte.ts`), so nothing takes it away on its own.
 */
async function clearRefusal(): Promise<void> {
  await browser
    .execute((dismissQuery: string) => {
      const el = document.querySelector(dismissQuery);
      if (el instanceof HTMLElement) el.click();
    }, DISMISS_REFUSAL)
    .catch(() => undefined);
}

/** Clicks a dir's disclosure triangle in the page. Not the gesture
 *  under test — it is how the spec *reads* what a dir holds — so it
 *  takes the untaxed route rather than spending ~12 s per candidate. */
async function toggleDir(dirId: string): Promise<void> {
  await browser
    .execute((query: string) => {
      const el = document.querySelector(query);
      if (el instanceof HTMLElement) el.click();
    }, `${dirRowSelector(dirId)} .dir-toggle`)
    .catch(() => undefined);
}

// --- fixture seeding, over loopback HTTP ---------------------------
//
// The same surface `metric-sort.spec.ts` seeds through, for the same
// reason: it is the app's own public write path (the importers use it),
// so nothing here writes SQLite behind the running core's back. The
// port is the one `wdio.conf.ts` pins through `appArgs`.

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
  name: string;
}

interface DirDto {
  id: string;
  persona_id: string;
  parent_id: string | null;
  name: string;
}

/** Sibling-unique names, which is what makes find-or-create idempotent:
 *  a rerun after a mid-failure adopts the rows the last run left
 *  instead of erroring on the unique constraint. */
const PARENT_NAME = "refusal-fixture";
const CHILD_NAME = "refusal-fixture contents";

/**
 * Creates (or adopts) the parent + child pair and returns their ids.
 *
 * Any persona will do: the sidebar opens unfiltered (`activePersona`
 * starts `null`, which lists every persona's dirs), so this borrows
 * whichever persona exists — in practice the one `metric-sort.spec.ts`
 * registered — and registers one only on a profile that has none.
 */
async function seedDirPair(): Promise<{ parentId: string; childId: string }> {
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
    personas[0] ??
    (await api<PersonaDto>("POST", "/asterism/personas/register", {
      name: "refusal fixture",
      pack_id: null,
    }));
  const dirs = await api<DirDto[]>("GET", "/asterism/dirs");
  const parent =
    dirs.find((d) => d.parent_id === null && d.name === PARENT_NAME) ??
    (await api<DirDto>("POST", "/asterism/dirs/create", {
      persona_id: persona.id,
      parent_id: null,
      name: PARENT_NAME,
    }));
  const child =
    dirs.find((d) => d.parent_id === parent.id && d.name === CHILD_NAME) ??
    (await api<DirDto>("POST", "/asterism/dirs/create", {
      persona_id: persona.id,
      parent_id: parent.id,
      name: CHILD_NAME,
    }));
  return { parentId: parent.id, childId: child.id };
}

/** Deletes a seeded dir, tolerating "already gone" and "still refused":
 *  the success-path test is the intended remover, this is the
 *  mid-failure backstop in `after`. */
async function deleteDirQuietly(dirId: string): Promise<void> {
  await api<unknown>("POST", "/asterism/dirs/delete", { dir_id: dirId }).catch(
    () => undefined,
  );
}

/** The trash-view toggle. `App.svelte`'s sidebar `$effect` watches
 *  `activeFilter.trashView` and refetches the dir listing on every
 *  flip, so two in-page clicks (on, then off) are this spec's "reload
 *  the sidebar" — dirs created over HTTP after the window painted have
 *  no other route onto the screen. Not the gesture under test, so both
 *  clicks take the untaxed route. */
const TRASH_VIEW_TOGGLE = 'aside.sidebar button[title^="Show trashed items"]';

async function nudgeDirReload(): Promise<void> {
  await browser
    .execute((query: string) => {
      const el = document.querySelector(query);
      if (el instanceof HTMLElement) el.click();
    }, TRASH_VIEW_TOGGLE)
    .catch(() => undefined);
}

/** The dirs this spec expanded, so `after` can put the sidebar back the
 *  shape it found it in. */
const expandedHere: string[] = [];

/** The seeded pair. The parent is the gesture's target, the child is
 *  what makes the refusal certain, and the success-path test removes
 *  both. */
let parentDirId = "";
let childDirId = "";

describe("refused write", () => {
  before(async () => {
    const trail: string[] = [];

    // The `__name` shim first and on its own: it has to be in place
    // before the first function-typed script runs. Idempotent.
    await stage(trail, "install __name shim", DRIVER_MS, () =>
      browser.execute(
        "window.__name = window.__name || function (target) { return target; };",
      ),
    );

    // The window and the SQLite open are the slow half of a cold start.
    // Polled with `execute`, so a script that cannot run yet reads as
    // "not ready" instead of costing a taxed `findElement` per poll.
    await pollUntil(
      trail,
      "app window paints",
      COLD_MS,
      async () => (await readSidebar()).present,
      "the app never painted its sidebar",
    );

    // Seed over HTTP, then nudge the sidebar into refetching: the dir
    // listing loaded with the first paint, before the rows below
    // existed.
    const seeded = await stage(trail, "seed the dir pair over HTTP", ROUND_TRIP_MS, () =>
      seedDirPair(),
    );
    parentDirId = seeded.parentId;
    childDirId = seeded.childId;

    await stage(trail, "reload the dir listing (trash view on)", DRIVER_MS, () =>
      nudgeDirReload(),
    );
    await stage(trail, "reload the dir listing (trash view off)", DRIVER_MS, () =>
      nudgeDirReload(),
    );

    await pollUntil(
      trail,
      "seeded folder reaches the sidebar",
      ROUND_TRIP_MS,
      async () => rowOf(await readSidebar(), parentDirId) !== undefined,
      "the seeded dir never appeared in the sidebar dir listing",
    );

    // Disclose it, so the child is on screen and the non-emptiness the
    // click relies on is proven from the DOM rather than from the ids
    // above.
    if ((rowOf(await readSidebar(), parentDirId)?.childRows ?? 0) === 0) {
      await stage(trail, "disclose the seeded folder", DRIVER_MS, () =>
        toggleDir(parentDirId),
      );
      expandedHere.push(parentDirId);
      await pollUntil(
        trail,
        "the seeded folder discloses its contents",
        PRESENT_MS,
        async () => (rowOf(await readSidebar(), parentDirId)?.childRows ?? 0) > 0,
        "the seeded folder disclosed no contents, though a child dir was just created in it",
      );
    }
  });

  after(async () => {
    // Backstop for a run that died between the seed and the
    // success-path test, which is the intended remover of the pair: the
    // next run's `drop-targets.spec.ts` asserts over every dir row it
    // finds, and must not inherit these. Child first — the parent
    // refuses while it holds one. Both tolerate "already gone".
    if (childDirId !== "") await deleteDirQuietly(childDirId);
    if (parentDirId !== "") await deleteDirQuietly(parentDirId);
    // Leave the sidebar the shape it was found in. Best-effort: a
    // cleanup failure must not replace whatever error got us here.
    for (const id of expandedHere) {
      const row = rowOf(await readSidebar(), id);
      if (row !== undefined && (row.childRows > 0 || row.emptyMarker)) {
        await toggleDir(id);
      }
    }
  });

  it("puts the backend's own reason on screen, deletes nothing, and goes when dismissed", async () => {
    const trail: string[] = [];

    try {
      // Re-read immediately before the click rather than trusting
      // `before`: the contents are what make the refusal certain, and
      // anything that emptied the dir in between would turn this
      // gesture into a successful delete.
      const armed = await stage(trail, "folder still holds something", DRIVER_MS, () =>
        readSidebar(),
      );
      const held = rowOf(armed, parentDirId);
      expect(held?.childRows ?? 0).toBeGreaterThan(0);
      const heldCount = held?.childRows ?? 0;

      // A real element click, not an in-page `el.click()`: this is the
      // gesture under test and routing it through the DOM would quietly
      // change what the assertion covers. `.group-delete` carries no
      // hover reveal (`GroupsSection.svelte` styles it plain), so there
      // is nothing to wait for first.
      await stage(trail, "click ✕ on the folder", DRIVER_MS + PRESENT_MS, () =>
        $(`${dirRowSelector(parentDirId)} .group-delete`).click(),
      );

      await pollUntil(
        trail,
        "refusal reaches the document",
        ROUND_TRIP_MS,
        async () => (await readRefusal()).present,
        "delete_dir was refused and nothing said so on screen — the failure reached " +
          "the console and stopped there, which is the defect this issue is about",
      );

      const refusal = await stage(trail, "read the refusal", DRIVER_MS, () => readRefusal());

      // One object so a failure prints the whole shape. The three
      // claims are independent: `message` is the call site's verb
      // phrase, `detail` is the backend's sentence having survived
      // serde and `detailOf`, and the two attributes are what makes it
      // an interruption rather than a decoration.
      //
      // `detail` is matched on "not empty" rather than on the whole
      // sentence: the wording is the backend's to change, and pinning
      // it here would make a copy edit in `dir.rs` a failing e2e run.
      // What must not change is that the *reason* arrives at all.
      expect({
        message: refusal.message,
        detailCarriesTheReason: refusal.detail.includes("not empty"),
        role: refusal.role,
        live: refusal.live,
        offersDismiss: refusal.dismissPresent,
      }).toEqual({
        message: "Could not delete this folder.",
        detailCarriesTheReason: true,
        role: "alert",
        live: "assertive",
        offersDismiss: true,
      });

      // Refused means nothing happened — to the dir, and to what it
      // holds. `deleteDir` collapses the row only on success, so both
      // halves are still readable here.
      const survived = await stage(trail, "nothing was deleted", DRIVER_MS, () =>
        readSidebar(),
      );
      const after = rowOf(survived, parentDirId);
      expect({
        folderIsStillThere: after !== undefined,
        contentsAreStillThere: after?.childRows ?? 0,
      }).toEqual({
        folderIsStillThere: true,
        contentsAreStillThere: heldCount,
      });

      // The way out. A refusal is sticky by design — no timer, so it
      // outlives the gesture that raised it — which makes this button
      // the only thing that takes it down. Clicked through the driver
      // for the same reason the ✕ was: it is part of what the issue
      // asks to be shown working, and there is no timer to race here.
      await stage(trail, "click Dismiss", DRIVER_MS + PRESENT_MS, () =>
        $(DISMISS_REFUSAL).click(),
      );
      await pollUntil(
        trail,
        "refusal leaves",
        PRESENT_MS,
        async () => !(await readRefusal()).present,
        "the refusal stayed on screen after its dismiss button was pressed",
      );
    } finally {
      // Sticky message, shared window: anything left here is still on
      // screen for whatever runs next.
      await clearRefusal();
    }
  });

  it("stays silent when the same verb succeeds", async () => {
    const trail: string[] = [];

    // The control group: the refusal above must be the backend
    // speaking, not a toast that any ✕ produces. The child dir is
    // empty, so the same gesture on it must succeed, take the row out,
    // and put nothing on screen. Then the parent — empty now — goes the
    // same way, which is the proof that the refusal above was about the
    // contents and about nothing else. It is also the cleanup: the
    // profile ends this suite holding the dirs it held before it.
    try {
      for (const dirId of [childDirId, parentDirId]) {
        await stage(trail, `click ✕ on ${dirId}`, DRIVER_MS + PRESENT_MS, () =>
          $(`${dirRowSelector(dirId)} .group-delete`).click(),
        );
        await pollUntil(
          trail,
          `folder ${dirId} leaves the sidebar`,
          ROUND_TRIP_MS,
          async () => rowOf(await readSidebar(), dirId) === undefined,
          "deleting an empty folder did not remove its row",
        );
        // Read *after* the row left: the removal is the proof the
        // backend answered, so a refusal read now is a statement about
        // this delete rather than a race against it.
        const quiet = await stage(trail, "no refusal for a success", DRIVER_MS, () =>
          readRefusal(),
        );
        expect(quiet.present).toBe(false);
      }
    } finally {
      await clearRefusal();
    }
  });
});
