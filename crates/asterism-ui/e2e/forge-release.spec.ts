// A release from the verb to the host, through the real backend, in
// the webview.
//
// `lib/stores/release.test.ts` owns what the catalog decides — that a
// write re-reads what it moved, that a parked run is not polled, that
// an empty file list is not a finished release. All of it runs against
// mocked `api` and `mutate`, which is the same hole the two forge specs
// beside this were written for: a unit test asserts the store called
// `"release_forge_change_point"` with a shape its own author wrote down
// twice. Whether a command of that name exists, takes those arguments
// and answers with what `bindings.ts` claims is a question only a
// webview spec against the real binary can ask. What this walk puts
// through that question: `release_forge_change_point`,
// `get_forge_release`, `list_forge_releases_of_change_point`,
// `release_output_dir`, `list_transfer_profiles`,
// `read_transfer_profile`, `send_forge_release`,
// `list_forge_release_sends` and `get_dispatch`.
//
// # Why one long walk
//
// The order is the model's, not this file's taste. A release names a
// change point, a change point comes from a pursuit closed satisfied,
// and a pursuit needs a line and something picked in the grid. A
// release whose fold has no live entry is refused outright, so there is
// no shortcut that seeds a releasable point without doing the work that
// makes one. And a send needs a release whose files have actually been
// written, which is a dispatch this spec has to wait out.
//
// # What this proves that no assertion below the webview can
//
// That the stamp rows reach a screen and say what the record says. A
// build with no certificate configured reports the manifest half
// skipped for `no_signing_identity` on every copy — and the whole point
// of the wording work in `ReleaseView` is that this reads as "not
// signed — no certificate configured" rather than as a failure. Nothing
// but a rendered row can check that.
//
// # Why the destination is `file://`
//
// The same reason `forge_send_e2e` sends there: an SSH server inside
// this process would answer for that server rather than for the app.
// What this spec is asking about sits above the transport — that a
// profile on disk is listed, picked, sent with, and that the host's
// per-file answers come back as rows — and `file://` is a destination a
// profile may name, which the transfer adapter's own `Scheme` says.
//
// # Where the profile comes from
//
// This spec writes it, into the directory the setting resolves to with
// nothing set — `$ASTERISM_HOME/transfer`. That settles #280's open
// question, and it is the better of the two options rather than the
// lazier: it needs no settings write, so the run exercises the empty
// default that every machine has before anybody changes a setting,
// which is the path that would otherwise be covered by nothing. A
// fixture under a temporary directory would have tested the app's
// ability to read a directory somebody had already pointed it at, and
// left the default untested.
//
// The file is written fresh each run under a fixed name and removed at
// the end. A leftover from a failed run is harmless — it is a valid
// profile aimed inside this suite's own disposable home, and the next
// run overwrites it — which is deliberately unlike the *lines* these
// specs leave, where a leftover carries the name the next run looks for.
//
// # Why this one seeds its own asset
//
// Its subject reads an asset's *bytes*, where the forge specs beside it
// work on rows: `forge-pursuit.spec.ts` puts a card on a line and never
// opens the file behind it, so a row pointing at nothing passes it. A
// release copies those bytes and stamps the copy, so the file has to be
// there.
//
// It is not, for the fixtures already in this profile. The e2e home is
// `workspace/runtime/e2e` under the *main* checkout — `workspace/` is a
// symlink, so every worktree's run shares one database — while each
// spec's fixture file is written under the worktree that seeded it. A
// row seeded from a worktree that has since been removed points at a
// path that no longer exists, and `ensureFixture` in the specs that own
// those rows only creates what is *missing*, so the row survives and
// the file does not. The first run of this spec found exactly that: the
// release's copy failed with `No such file or directory` on a locator
// under `.worktrees/material-layers`.
//
// So this seeds a row against the file it has just written, matched by
// locator rather than by name: a stale row for another worktree's path
// is simply not this one, and a fresh row is created beside it. That
// makes the spec independent of which worktree ran last, which is the
// property the failure above says it needs.
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 20_000;
// A release and a send each start a dispatch and wait for the job
// engine to run it. That is a real pipeline rather than a round trip,
// and the poll below is what this spec spends most of its time in.
const RUN_MS = 60_000;
const COLD_MS = 60_000;
const POLL_GAP_MS = 250;

const here = path.dirname(fileURLToPath(import.meta.url));
// wdio runs with `crates/asterism-ui` as its working directory; the e2e
// home is the one `wdio.conf.ts` hands the app, and both halves have to
// agree or this spec writes its profile where nothing reads it.
const REPO_ROOT = path.resolve(here, "../../..");
const E2E_HOME = path.join(REPO_ROOT, "workspace/runtime/e2e");
const PROFILE_DIR = path.join(E2E_HOME, "transfer");
const PROFILE_NAME = "e2e-file-destination";
const PROFILE_FILE = path.join(PROFILE_DIR, `${PROFILE_NAME}.json`);

const RUN = Date.now();
const LINE_NAME = `e2e-forge-release-${RUN}`;
const LINE_PREFIX = "e2e-forge-release";
const WORK_TITLE = "e2e release round";
const DESTINATION = "an e2e destination";
// Where the send puts the files. Under this suite's own home so that
// nothing it writes escapes the disposable profile.
const SENT_DIR = path.join(E2E_HOME, "e2e-sent", String(RUN));

const FORGE_ROW = 'aside.sidebar button[title^="Lines on this machine"]';
const DRAWER = '[role="dialog"][aria-label="Forge"]';
const RELEASE = '[role="dialog"][aria-label="Release"]';

// --- the asset this releases, seeded over the app's own HTTP --------
//
// Same port and the same shape `chapter-band.spec.ts` uses; that file
// carries the argument for seeding this way rather than through the UI.
const HTTP_PORT = Number(process.env.E2E_APP_PORT ?? 19899);
const BASE_URL = `http://127.0.0.1:${HTTP_PORT}`;
const PACK_ID = "e2e-forge-release";
const PERSONA_NAME = "Forge releases";
const COVER = "e2e-forge-release-fixture";

/// CRC-32 as PNG spells it, over a chunk's type and body.
///
/// Written out rather than taken from a crate, for the reason
/// `asterism-exporter-transfer` writes its own CSV quoting: the whole
/// of what this file needs from CRC-32 is one table and one loop, and
/// nothing here reads a checksum back.
const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c;
  }
  return table;
})();

function crc32(bytes: Buffer): number {
  let c = -1;
  for (const byte of bytes) {
    c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  }
  return (c ^ -1) >>> 0;
}

/// One PNG chunk: length, type, body, CRC over type and body.
function pngChunk(type: string, body: Buffer): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(body.length);
  const typed = Buffer.concat([Buffer.from(type, "ascii"), body]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(typed));
  return Buffer.concat([length, typed, crc]);
}

/// A 1×1 8-bit RGBA PNG whose one pixel is fully transparent.
///
/// **Built here rather than held as a literal, and that is the point.**
/// It was a base64 constant, described in this comment as one fully
/// transparent pixel. It was not: the IDAT inflated to
/// `01 ff 00 00 7f` — a Sub filter and a half-opaque red pixel — so the
/// statement about where the bytes came from rested on a description
/// they contradicted, which is the one thing a provenance note may not
/// do. Bytes nobody in this repository can read are bytes nobody can
/// check.
///
/// Constructed, there is nothing to be wrong about: signature, `IHDR`
/// (1×1, 8-bit, colour type 6), one `IDAT` holding a single scanline of
/// filter type 0 followed by four zero channels, and `IEND`. Every byte
/// originates here, so no licence, notice or attribution travels with
/// it and PUBLIC_DEVELOPMENT.md's question about third-party material
/// has no subject.
///
/// A real container rather than a placeholder named `.png`, because
/// what the rows this spec reads are *about* is what the disclosure
/// writer did to the copy, and it has to open the file to do anything
/// at all.
function onePixelPng(): Buffer {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(1, 0); // width
  ihdr.writeUInt32BE(1, 4); // height
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type 6 — truecolour with alpha
  ihdr[10] = 0; // compression: deflate, the only one PNG defines
  ihdr[11] = 0; // filter method: adaptive, the only one PNG defines
  ihdr[12] = 0; // not interlaced
  // The scanline: one filter byte (0 = None) and R, G, B, A all zero.
  const idat = zlib.deflateSync(Buffer.from([0, 0, 0, 0, 0]));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", idat),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

async function http<T>(method: string, route: string, body?: unknown): Promise<T> {
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

interface PersonaRow {
  id: string;
  pack_id: string | null;
}
interface CardRow {
  id: string;
  cover: string | null;
  source_locator: string;
}
interface PageRow {
  items: CardRow[];
}

/// Writes the file the seeded row points at, under this worktree.
function writeFixtureFile(): string {
  const dir = path.join(REPO_ROOT, "workspace/runtime/e2e-fixtures/forge-release");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "key-visual.png");
  // Rewritten every run rather than only when missing. This is the one
  // spec that reads the bytes, and a zero-length or truncated leftover
  // from an interrupted run would fail the stamp rather than the copy —
  // a slower failure to read than simply writing 70 bytes again.
  fs.writeFileSync(file, onePixelPng());
  return file;
}

/// Brings the profile to the state this spec needs and returns the
/// asset id.
///
/// Matched on the locator, not the cover: a row carrying this cover but
/// pointing into a worktree that no longer exists is not a row this
/// spec can release, and reusing it is what the first run did before it
/// failed on the missing file.
async function ensureFixture(): Promise<{ assetId: string; personaId: string }> {
  const file = writeFixtureFile();
  await http<unknown>("GET", "/asterism/health").catch((err) => {
    throw new Error(
      `the app is not serving HTTP on ${BASE_URL} (${String(err)}). ` +
        "This spec seeds the asset it releases over that port, so it cannot continue.",
    );
  });

  const personas = await http<PersonaRow[]>("GET", "/asterism/personas");
  const persona =
    personas.find((p) => p.pack_id === PACK_ID) ??
    (await http<PersonaRow>("POST", "/asterism/personas/register", {
      name: PERSONA_NAME,
      pack_id: PACK_ID,
    }));

  const live = await http<PageRow>(
    "GET",
    `/asterism/assets?persona_id=${encodeURIComponent(persona.id)}&limit=500`,
  );
  const mine = live.items.find((card) => card.source_locator === file);
  if (mine !== undefined) return { assetId: mine.id, personaId: persona.id };

  const created = await http<{ id: string }>("POST", "/asterism/assets/add", {
    persona_id: persona.id,
    source_kind: "fs",
    locator: file,
    modality: null,
    occurred_at_ms: 1_700_000_000_000,
    labels: ["e2e-forge-release-fixture"],
    register_note: null,
    platform: null,
    file_size_bytes: fs.statSync(file).size,
    duration_ms: null,
    width_px: 1,
    height_px: 1,
    extra_json: null,
    cover_hint: COVER,
  });
  return { assetId: created.id, personaId: persona.id };
}

/// Per-stage frames, the way `card-trash.spec.ts` leaves them: `ui-e2e`
/// is the only recipe that produces something an agent can look at, and
/// the frames are that. A screenshot must never be why a test fails, so
/// the call is raced against its own ceiling and every error is eaten.
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

/** Same shape the two forge specs use: a raw driver call carries no
 *  timeout, so the bound has to be a race, and on failure the error
 *  names the step plus what already passed. */
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
  message: string | (() => string),
) {
  await stage(trail, name, ms + DRIVER_MS, async () => {
    const deadline = Date.now() + ms;
    for (;;) {
      if (await check()) return;
      if (Date.now() > deadline) {
        throw new Error(typeof message === "string" ? message : message());
      }
      await new Promise((r) => setTimeout(r, POLL_GAP_MS));
    }
  });
}

/** Clicks by selector through `execute`, and fails when it hits
 *  nothing — a click in the page answers with a boolean, and a boolean
 *  nobody reads is a step that passes for pressing thin air. */
async function press(selector: string): Promise<void> {
  const hit = await browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    if (el === null) return false;
    (el as HTMLElement).click();
    return true;
  }, selector);
  if (!hit) throw new Error(`nothing to press at ${selector}`);
}

/** Clicks the button inside `within` whose label is exactly `label`. */
async function pressLabelled(within: string, label: string): Promise<void> {
  const hit = await browser.execute(
    (scope: string, wanted: string) => {
      const root = document.querySelector(scope);
      if (root === null) return false;
      const button = Array.from(root.querySelectorAll("button")).filter(
        (b) => (b.textContent?.trim() ?? "") === wanted,
      )[0];
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    within,
    label,
  );
  if (!hit) throw new Error(`no button reading "${label}" inside ${within}`);
}

interface ReleaseSnapshot {
  /** Whether the release drawer is up. */
  present: boolean;
  /** Whatever the header says about when and where. */
  meta: string;
  /** One entry per file row, whitespace-collapsed. */
  files: string[];
  /** What the drawer says instead of file rows, when it has none. This
   *  is the sentence that explains a release with nothing on it — a run
   *  still going, one that finished empty, or one that failed carrying
   *  its own reason — so a poll that times out reports it rather than
   *  just saying the rows never came. */
  note: string;
  /** One entry per send row. */
  sends: string[];
  /** The per-file lines of whichever send is expanded. */
  attempt: string[];
  /** Profile rows in the send form: what each says, and whether it can
   *  be picked. The two together, because a row that cannot be picked
   *  is one carrying a parse error, and the error is the thing worth
   *  reporting when this spec's own profile is the one refused. */
  profiles: { name: string; usable: boolean; text: string }[];
  /** What the app refused, if anything. Outside the drawer, and read
   *  anyway: `mutate` puts every refusal there and a spec that reads
   *  only the drawer sees a write silently not happening. */
  refusal: string;
}

/** One `execute` for the whole drawer, so a step costs one driver call
 *  rather than one per question. */
function readRelease(): Promise<ReleaseSnapshot> {
  return browser.execute(() => {
    const flat = (el: Element | null | undefined) =>
      (el?.textContent ?? "").replace(/\s+/g, " ").trim();
    const all = (root: Element, sel: string) =>
      Array.from(root.querySelectorAll(sel)).map((el) => flat(el));
    const refusal = flat(document.querySelector(".refusal-toast"));
    const panel = document.querySelector('[role="dialog"][aria-label="Release"]');
    if (panel === null) {
      return {
        present: false,
        meta: "",
        files: [] as string[],
        note: "",
        sends: [] as string[],
        attempt: [] as string[],
        profiles: [] as { name: string; usable: boolean; text: string }[],
        refusal,
      };
    }
    return {
      present: true,
      meta: flat(panel.querySelector(".rel-meta")),
      files: all(panel, ".rel-files > li"),
      note: all(panel, ".rel-section .rel-empty").join(" | "),
      sends: all(panel, ".rel-sends .rel-send-head"),
      attempt: all(panel, ".rel-attempt-files > li"),
      profiles: Array.from(panel.querySelectorAll(".rel-profiles > li")).map(
        (li) => {
          const radio = li.querySelector(
            'input[type="radio"]',
          ) as HTMLInputElement | null;
          return {
            name: radio?.value ?? "",
            usable: radio !== null && !radio.disabled,
            text: flat(li),
          };
        },
      ),
      refusal,
    };
  });
}

/** Every line this spec's earlier runs left behind, this run's aside. */
async function sweepLeftovers(trail: string[]): Promise<void> {
  for (let round = 0; round < 20; round += 1) {
    const stale = await browser.execute((prefix: string, mine: string) => {
      const nav = document.querySelector(
        '[role="dialog"][aria-label="Forge"] nav[aria-label="Lines"]',
      );
      if (nav === null) return [] as string[];
      return Array.from(nav.querySelectorAll("button"))
        .map((b) => b.textContent?.trim() ?? "")
        .filter((name) => name.startsWith(prefix) && name !== mine);
    }, LINE_PREFIX, LINE_NAME);
    if (stale.length === 0) return;
    await dropLine(trail, stale[0], `sweep ${stale[0]}`);
  }
  throw new Error("more leftovers than this sweep is willing to remove");
}

/** Archives and discards the named line, closing any open work first.
 *
 *  Work first, and the model is why: a drop takes the history the work
 *  was cut from, so it refuses while any is open rather than leaving a
 *  log against nothing. */
async function dropLine(
  trail: string[],
  name: string,
  label: string,
): Promise<void> {
  await stage(trail, `${label}: select`, DRIVER_MS, () =>
    pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, name),
  );
  await stage(trail, `${label}: to the work`, DRIVER_MS, () =>
    pressLabelled(`${DRAWER} .tabs`, "work"),
  );
  for (let piece = 0; piece < 10; piece += 1) {
    const anyOpen = await browser.execute(
      () =>
        document.querySelector(
          '[role="dialog"][aria-label="Forge"] .work-list:not(.ended) button',
        ) !== null,
    );
    if (!anyOpen) break;
    await stage(trail, `${label}: open the piece`, DRIVER_MS, () =>
      press(`${DRAWER} .work-list:not(.ended) button`),
    );
    await pollUntil(
      trail,
      `${label}: the piece paints`,
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document.querySelector(
              '[role="dialog"][aria-label="Forge"] .work-head',
            ) !== null,
        ),
      "the piece of work never opened",
    );
    await stage(trail, `${label}: abandon it`, DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .close`, "close · abandon"),
    );
    await stage(trail, `${label}: back to the list`, DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .work-head`, "← all work"),
    );
  }
  const archived = await browser.execute((wanted: string) => {
    const nav = document.querySelector(
      '[role="dialog"][aria-label="Forge"] nav[aria-label="Lines"]',
    );
    const lists = nav === null ? [] : Array.from(nav.querySelectorAll("ul"));
    return Array.from(lists[1]?.querySelectorAll("button") ?? []).some(
      (b) => (b.textContent?.trim() ?? "") === wanted,
    );
  }, name);
  if (!archived) {
    await stage(trail, `${label}: archive`, DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .verbs`, "archive"),
    );
    await stage(trail, `${label}: select it again`, DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, name),
    );
  }
  await stage(trail, `${label}: discard`, DRIVER_MS, () =>
    press(`${DRAWER} .verbs button.danger`),
  );
  await stage(trail, `${label}: confirm`, DRIVER_MS, () =>
    pressLabelled("body", "Discard Forever"),
  );
  await pollUntil(
    trail,
    `${label}: it is gone`,
    ROUND_TRIP_MS,
    async () =>
      browser.execute((wanted: string) => {
        const nav = document.querySelector(
          '[role="dialog"][aria-label="Forge"] nav[aria-label="Lines"]',
        );
        if (nav === null) return false;
        return !Array.from(nav.querySelectorAll("button")).some(
          (b) => (b.textContent?.trim() ?? "") === wanted,
        );
      }, name),
    `${name} did not go`,
  );
}

describe("a release, and where it went", () => {
  /// The asset this run releases, and the persona it belongs to, both
  /// seeded in `before`.
  let assetId = "";
  let personaId = "";

  before(async () => {
    // The profile, and the directory it aims at. Written from Node
    // because the spec process is the only side of this with a
    // filesystem — the webview has none, and the app deliberately does
    // not write profiles.
    fs.mkdirSync(PROFILE_DIR, { recursive: true });
    fs.mkdirSync(SENT_DIR, { recursive: true });
    fs.writeFileSync(
      PROFILE_FILE,
      JSON.stringify(
        {
          endpoint: `file://${SENT_DIR}`,
          // One column, rendered from the item the send binds. Enough
          // to prove the sidecar is written; what an agency's column
          // set looks like is that agency's business and is not in this
          // tree.
          sidecar: {
            filename: "metadata.csv",
            columns: [
              { header: "Filename", template: "{{item.remote_name}}" },
            ],
          },
        },
        null,
        2,
      ),
    );

    const trail: string[] = [];
    await stage(trail, "install __name shim", DRIVER_MS, () =>
      browser.execute(
        "window.__name = window.__name || function (target) { return target; };",
      ),
    );
    await pollUntil(
      trail,
      "app window paints",
      COLD_MS,
      async () =>
        browser.execute(() => document.querySelector("aside.sidebar") !== null),
      "the app never painted its sidebar",
    );
    await pollUntil(
      trail,
      "the grid has a card to select",
      COLD_MS,
      async () =>
        browser.execute(
          () => document.querySelector(".card[data-asset-id]") !== null,
        ),
      "no card ever painted, so there is nothing to put on a line",
    );
    const seeded = await stage(
      trail,
      "seed the asset to release",
      ROUND_TRIP_MS,
      () => ensureFixture(),
    );
    assetId = seeded.assetId;
    personaId = seeded.personaId;

    // Ask the backend what it makes of the profile this run just wrote,
    // before any of it is driven through a screen.
    //
    // Two things this settles at the only moment they are cheap to
    // settle. Whether the directory the app resolves is the directory
    // this spec wrote into — the two derive it independently, one from
    // `$ASTERISM_HOME` and one from `import.meta.url`, and a mismatch
    // would otherwise surface as a profile that is simply not listed.
    // And, if it is listed, the parser's own sentence about it: a
    // profile refused here is refused for a reason, and reading that
    // reason off a disabled radio three steps later says only that
    // something was wrong.
    await stage(trail, "the backend can send with the profile", ROUND_TRIP_MS, async () => {
      const listed = await http<{
        directory: string;
        profiles: { name: string; error: string | null; scheme: string | null }[];
      }>("GET", "/asterism/forge/transfer-profiles");
      const mine = listed.profiles.find((row) => row.name === PROFILE_NAME);
      if (mine === undefined) {
        throw new Error(
          `the app reads profiles from ${listed.directory}, and this run wrote ` +
            `${PROFILE_FILE}. It saw ${JSON.stringify(listed.profiles.map((p) => p.name))}.`,
        );
      }
      // `!= null`, so an absent field and a null one read alike. The
      // contract now always sends one; a strict comparison here was
      // half of what made the picker refuse every profile.
      if (mine.error != null) {
        throw new Error(
          `the profile this run wrote was refused: ${mine.error}\n` +
            `  file: ${PROFILE_FILE}\n` +
            `  content: ${fs.readFileSync(PROFILE_FILE, "utf8")}`,
        );
      }
    });

    // Reload, and reload *clean*. A persona registered after startup is
    // not in the sidebar the app already painted — the first run of
    // this spec asked for the seeded card in a grid that had been
    // loaded before the row existed, and the sidebar still said three
    // personas. The search string carries the whole filter state, so
    // dropping it also starts this spec from defaults whatever ran
    // before it in the session. `chapter-band.spec.ts` does both, for
    // both of these reasons.
    await stage(trail, "mark the page", DRIVER_MS, () =>
      browser.execute(() => {
        (window as unknown as { __releaseMark?: boolean }).__releaseMark = true;
      }),
    );
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
      async () =>
        browser.execute(() => {
          const fresh =
            (window as unknown as { __releaseMark?: boolean }).__releaseMark !==
            true;
          // Both halves: a fresh document, and one that has painted its
          // shell. Either alone passes too early.
          return fresh && document.querySelector("aside.sidebar") !== null;
        }),
      "the app never came back with a fresh document after the reload",
    );
    await stage(trail, "reinstall __name shim", DRIVER_MS, () =>
      browser.execute(
        "window.__name = window.__name || function (target) { return target; };",
      ),
    );
  });

  after(() => {
    // What this run wrote, taken back out. The line is dropped inside
    // the test; these are the two things outside the app's own record.
    try {
      fs.rmSync(PROFILE_FILE, { force: true });
      fs.rmSync(SENT_DIR, { recursive: true, force: true });
    } catch {
      // The home is disposable; a leftover here costs nothing.
    }
  });

  it("writes a change point out, reads the stamps, and sends it", async () => {
    const trail: string[] = [];

    // Narrow to this spec's own persona before looking for its card.
    // The grid is virtualised, so over a whole profile the card is not
    // guaranteed to be in the DOM at all — and a card that is not there
    // cannot be clicked.
    await pollUntil(
      trail,
      "the fixture persona row appears",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          (query: string) => document.querySelector(query) !== null,
          `aside.sidebar li.persona-row[data-persona-id="${personaId}"]`,
        ),
      `the sidebar never listed the "${PERSONA_NAME}" persona`,
    );
    await stage(trail, "narrow to the fixture persona", DRIVER_MS, () =>
      browser.execute((query: string) => {
        // The row's own button, which is the first one: `.persona-info`
        // is the ⓘ that opens the profile card and must not be hit.
        const row = document.querySelector(query);
        const button = row ? row.querySelector("button") : null;
        if (!(button instanceof HTMLElement)) return false;
        button.click();
        return true;
      }, `aside.sidebar li.persona-row[data-persona-id="${personaId}"]`),
    );
    await pollUntil(
      trail,
      "the seeded card is in the grid",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          (id: string) =>
            document.querySelector(`.card[data-asset-id="${id}"]`) !== null,
          assetId,
        ),
      "the seeded asset never appeared in the grid",
    );

    // A selection first: the drawer is an overlay, so the grid is not
    // reachable once the forge is up. A meta-click is the gesture that
    // toggles selection rather than opening the detail pane.
    //
    // This card and not the first one in the grid. The first is
    // whatever another spec seeded, and the release copies its bytes —
    // which is how the first run of this spec came to fail on a locator
    // under a worktree that had been removed.
    await stage(trail, "select the seeded card", DRIVER_MS, () =>
      browser.execute((id: string) => {
        const card = document.querySelector(`.card[data-asset-id="${id}"]`);
        if (card === null) return false;
        card.dispatchEvent(
          new MouseEvent("click", { bubbles: true, metaKey: true }),
        );
        return true;
      }, assetId),
    );

    await stage(trail, "open the forge", DRIVER_MS, () => press(FORGE_ROW));
    await pollUntil(
      trail,
      "the drawer paints",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document.querySelector('[role="dialog"][aria-label="Forge"]') !==
            null,
        ),
      "the forge drawer never appeared",
    );
    await sweepLeftovers(trail);

    // A line, a pursuit, and one entry landed on it — which is the
    // shortest road to a change point that folds to something, and a
    // release whose fold has no live entry is refused.
    await stage(trail, "fill the new-line form", DRIVER_MS, () =>
      browser.execute((name: string) => {
        const form = document.querySelector(
          '[role="dialog"][aria-label="Forge"] form.new-line',
        );
        const input = form?.querySelector("input");
        const select = form?.querySelector("select");
        if (!form || !input || !select) return false;
        const rule = Array.from(select.options).filter((o) => o.value !== "")[0];
        if (rule === undefined) return false;
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        select.value = rule.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        return true;
      }, LINE_NAME),
    );
    await stage(trail, "open the line", DRIVER_MS, () =>
      press(`${DRAWER} form.new-line button`),
    );
    await pollUntil(
      trail,
      "the line reaches the list",
      ROUND_TRIP_MS,
      async () =>
        browser.execute((wanted: string) => {
          const nav = document.querySelector(
            '[role="dialog"][aria-label="Forge"] nav[aria-label="Lines"]',
          );
          if (nav === null) return false;
          return Array.from(nav.querySelectorAll("button")).some(
            (b) => (b.textContent?.trim() ?? "") === wanted,
          );
        }, LINE_NAME),
      "the opened line never appeared in the list",
    );
    await stage(trail, "select the line", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} nav[aria-label="Lines"]`, LINE_NAME),
    );
    await stage(trail, "press open work", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .line header`, "open work"),
    );
    await stage(trail, "fill the new-work form", DRIVER_MS, () =>
      browser.execute((title: string) => {
        const form = document.querySelector(
          '[role="dialog"][aria-label="Forge"] form.new-work',
        );
        const input = form?.querySelector("input");
        if (!form || !input) return false;
        input.value = title;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        return true;
      }, WORK_TITLE),
    );
    await stage(trail, "open the pursuit", DRIVER_MS, () =>
      press(`${DRAWER} form.new-work button`),
    );
    await stage(trail, "add the selection", DRIVER_MS, () =>
      press(`${DRAWER} .compose button`),
    );
    await pollUntil(
      trail,
      "the round reaches the log",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document.querySelectorAll(
              '[role="dialog"][aria-label="Forge"] .rounds > li',
            ).length === 1,
        ),
      "the round never appeared in the work's log",
    );
    await stage(trail, "close it satisfied", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .close`, "close · put it on the line"),
    );
    await pollUntil(
      trail,
      "the work reports it ended",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('[role="dialog"][aria-label="Forge"] .work-head')
              ?.textContent?.includes("put it on the line") ?? false,
        ),
      "closing the work did not land it",
    );

    // The chain now holds the point this releases.
    await stage(trail, "read the history", DRIVER_MS, () =>
      pressLabelled(`${DRAWER} .tabs`, "history"),
    );
    await pollUntil(
      trail,
      "the change point is on the chain",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(
          () =>
            document.querySelectorAll(
              '[role="dialog"][aria-label="Forge"] .chain > li',
            ).length === 1,
        ),
      "the satisfied close left no change point",
    );

    // The verb is disabled until this screen can say which persona the
    // frozen set belongs to, which it learns from the card the library
    // answers for. Waiting for it here rather than pressing through is
    // the difference between asserting the button works and asserting
    // it exists.
    await pollUntil(
      trail,
      "write out is offered",
      ROUND_TRIP_MS,
      async () =>
        browser.execute(() => {
          const button = document.querySelector(
            '[role="dialog"][aria-label="Forge"] .write-out',
          ) as HTMLButtonElement | null;
          return button !== null && !button.disabled;
        }),
      "the change point never became releasable — no card said which persona it is",
    );
    await stage(trail, "write it out", DRIVER_MS, () =>
      press(`${DRAWER} .write-out`),
    );

    // The drawer opens on a release that has been recorded and whose
    // files are still being written — the honest first frame, and the
    // one the poll fills.
    await pollUntil(
      trail,
      "the release drawer paints",
      ROUND_TRIP_MS,
      async () => (await readRelease()).present,
      "writing out did not open the release drawer",
    );

    // The stamps. Unsigned is what this build produces, so every copy
    // reports the manifest half skipped — and the wording is the thing
    // under test: absence stated as absence, not as a failure.
    let sawFiles = "";
    await pollUntil(
      trail,
      "every copy reports both halves of its stamp",
      RUN_MS,
      async () => {
        const view = await readRelease();
        sawFiles = `files=${JSON.stringify(view.files)} note=${JSON.stringify(view.note)}`;
        return (
          view.files.length > 0 &&
          view.files.every(
            (row) =>
              // Both halves reported, whatever each one says. What the
              // XMP half *said* is the writer's business and depends on
              // the container; that it is stated at all is this
              // screen's.
              row.includes("XMP:") &&
              // The manifest half is the acceptance criterion itself:
              // with no certificate configured every row says the
              // manifest was not signed, and says so for that reason
              // rather than as a failure.
              row.includes("manifest: not signed — no certificate configured"),
          )
        );
      },
      () => `the file rows never said what became of each copy; last saw ${sawFiles}`,
    );
    await snapStage("release-before-the-send");

    // The send. The profile was written before the app started and is
    // read from the directory the empty setting resolves to.
    // By class rather than by label: the caret in front of the words
    // flips with the fold-out's own state, so matching the text would
    // be matching a control's current shape rather than the control.
    await stage(trail, "open the send form", DRIVER_MS, () =>
      press(`${RELEASE} .rel-send-toggle`),
    );
    let sawProfiles = "";
    await pollUntil(
      trail,
      "the profile is listed and usable",
      ROUND_TRIP_MS,
      async () => {
        const view = await readRelease();
        sawProfiles = JSON.stringify(view.profiles);
        // Usable, not merely listed. A profile that does not parse is
        // listed too — that is the point of the row — so a check that
        // only looked for the name passed on a row carrying a parse
        // error and left the failure to show up two steps later as a
        // disabled button.
        return view.profiles.some(
          (row) => row.name === PROFILE_NAME && row.usable,
        );
      },
      () =>
        `the profile this run wrote was not listed as usable; last saw ${sawProfiles}`,
    );
    // Svelte binds a radio group on `change` and a text field on
    // `input`, so both are set the way the component listens for rather
    // than by `click()` — and the step answers with what went wrong
    // instead of a boolean nobody reads, which is how the previous run
    // recorded "pick the profile" as done while picking nothing.
    await stage(trail, "pick the profile and name the send", DRIVER_MS, async () => {
      const why = await browser.execute(
        (wanted: string, label: string) => {
          const panel = document.querySelector(
            '[role="dialog"][aria-label="Release"]',
          );
          if (panel === null) return "no release panel";
          const radios = Array.from(
            panel.querySelectorAll('.rel-profiles input[type="radio"]'),
          ) as HTMLInputElement[];
          const radio = radios.find((el) => el.value === wanted);
          if (radio === undefined) {
            return `no radio for ${wanted}; saw ${JSON.stringify(radios.map((r) => r.value))}`;
          }
          if (radio.disabled) return `the radio for ${wanted} is disabled`;
          radio.checked = true;
          radio.dispatchEvent(new Event("change", { bubbles: true }));
          const field = panel.querySelector(
            ".rel-field input",
          ) as HTMLInputElement | null;
          if (field === null) return "no destination field";
          field.value = label;
          field.dispatchEvent(new Event("input", { bubbles: true }));
          return "";
        },
        PROFILE_NAME,
        DESTINATION,
      );
      if (why !== "") throw new Error(why);
    });
    // Not `press`: `el.click()` on a disabled button does nothing and
    // reports that the element was there, so the step passes for a form
    // that refused the gesture. That is what happened on the run before
    // this check existed — "send it" recorded as done, no send, no
    // refusal, and nothing on screen to say which.
    await stage(trail, "send it", DRIVER_MS, async () => {
      const state = await browser.execute(() => {
        const button = document.querySelector(
          '[role="dialog"][aria-label="Release"] .rel-btn-primary',
        ) as HTMLButtonElement | null;
        if (button === null) return "no send button";
        if (button.disabled) {
          const panel = document.querySelector(
            '[role="dialog"][aria-label="Release"]',
          );
          const picked = (
            panel?.querySelector(
              '.rel-profiles input[type="radio"]:checked',
            ) as HTMLInputElement | null
          )?.value;
          const label = (
            panel?.querySelector(".rel-field input") as HTMLInputElement | null
          )?.value;
          return `send is disabled (picked=${JSON.stringify(picked ?? null)} label=${JSON.stringify(label ?? null)})`;
        }
        button.click();
        return "";
      });
      if (state !== "") throw new Error(state);
    });

    // The send is a row, and it says what the host did with each file.
    let sawSends = "";
    await pollUntil(
      trail,
      "the send is listed with its counts",
      RUN_MS,
      async () => {
        const view = await readRelease();
        sawSends = `sends=${JSON.stringify(view.sends)} refusal=${JSON.stringify(view.refusal)}`;
        return view.sends.some(
          (row) => row.includes(DESTINATION) && row.includes("put"),
        );
      },
      () => `the send never reported what it put; last saw ${sawSends}`,
    );
    await stage(trail, "expand the send", DRIVER_MS, () =>
      press(`${RELEASE} .rel-sends .rel-send-head`),
    );
    let sawAttempt = "";
    await pollUntil(
      trail,
      "one line per file, with the host's answer",
      ROUND_TRIP_MS,
      async () => {
        const view = await readRelease();
        sawAttempt = JSON.stringify(view.attempt);
        return (
          view.attempt.length > 0 &&
          view.attempt.every((row) => row.includes("sent"))
        );
      },
      () => `the send's per-file rows never arrived; last saw ${sawAttempt}`,
    );
    await snapStage("release-after-the-send");

    // The bytes are actually on the far side, which is the one claim no
    // reading of the app's own screen can make for it.
    await stage(trail, "the files are on the far side", DRIVER_MS, async () => {
      const landed = fs.readdirSync(SENT_DIR);
      if (!landed.includes("metadata.csv")) {
        throw new Error(`no sidecar beside the files: ${landed.join(", ")}`);
      }
      if (landed.length < 2) {
        throw new Error(`nothing but the sidecar arrived: ${landed.join(", ")}`);
      }
    });

    // Clean up through the panel, as the two specs beside this do: a
    // destructive verb provoked against seeded fixture is usable once,
    // so this discards what it made.
    await stage(trail, "close the release", DRIVER_MS, () =>
      press(`${RELEASE} .rel-close`),
    );
    await dropLine(trail, LINE_NAME, "cleanup");
    await stage(trail, "close the drawer", DRIVER_MS, () =>
      press(`${DRAWER} .drawer-close`),
    );
  });
});
