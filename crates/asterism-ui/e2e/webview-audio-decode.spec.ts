/**
 * What the packaged webview does with each audio format the map names.
 *
 * `KNOWN_AUDIO_MIMES` is a closed list of formats this app records, and
 * recording one hands the row a player: `MimeType::media` answers
 * `Audio` for the whole family, so `render_policy` takes a named audio
 * row out of its conservative arm and the detail pane mounts an
 * `<audio>` element. The waveform above it opens on a second gate — the
 * asset's *modality* rather than its mime — which is why the rows below
 * state both, and why a spec that stated only the mime would measure a
 * decode that never started. Whether the webview can decode the bytes
 * underneath either promise is a different question from whether the
 * map can name the format.
 *
 * It has to be asked here rather than in Rust. The decoder is
 * WKWebView's, the waveform is drawn from that same decoder
 * (`DetailPane.svelte` resolves an `OfflineAudioContext` and calls
 * `decodeAudioData`), and a real window is the only place either one
 * answers. That is what this suite is for — the questions the HTTP API
 * cannot answer — and it is why the video side's equivalent claim
 * (`VideoFormat::webview_cannot_play`) carries a measurement in its doc
 * rather than a derivation.
 *
 * # What this file claims
 *
 * For every format the scanner can deliver, seeded from the bytes
 * `scripts/gen-test-fixtures.py` generates: the pane mounts the audio
 * surface, the `<audio>` element parses the container's header, and the
 * waveform draws from decoded samples. Measured on 2026-09-08 against
 * WKWebView 605.1.15, that is what all eight do — nothing this app can
 * name is refused — and asserting it is what turns the measurement into
 * something that stays true rather than something somebody remembers.
 *
 * The row per format is still printed, because a failure here is a
 * statement about a system decoder and the table is what makes it
 * legible: which format, which half, and what the element or the
 * waveform said instead.
 *
 * **If a format ever is refused**, that is a fact about WKWebView
 * rather than a defect in this tree, and the decision it opens — a
 * transcoded rendition the way video does it, or a surface that says
 * what it cannot do — belongs where `VideoFormat::webview_cannot_play`
 * makes its claim, not in this file. What this file would do is stop
 * asserting the uniform answer and carry the exception per case.
 *
 * # What it deliberately does not claim
 *
 * **Nothing about playback past the first frames.** `preload="metadata"`
 * is what the pane asks for, so a container that parses its header and
 * fails later reads as loadable here. Distinguishing those needs audio
 * output, which a headless-ish CI window would not have either.
 *
 * **Nothing about chapters or marks.** Those hang off the same element
 * and have their own spec (`chapter-band.spec.ts`), which asserts them
 * against a placeholder file precisely because it makes no claim about
 * decoding. The two files split that way on purpose.
 *
 * Shape follows the other specs in this directory, and the helpers
 * (`snapStage` / `stage` / `pollUntil`, the `api` seeder, `repoRoot`)
 * are duplicated rather than shared for the reason `metric-sort.spec.ts`
 * gives: each file's budgets and failure modes are documented against
 * its own steps. Every query goes through `browser.execute`, because
 * the tauri service charges roughly six seconds to each element
 * command, and the `__name` shim is reinstalled after every load or the
 * driver's stringified callbacks die on a `ReferenceError`.
 *
 * # Why the filename sorts last
 *
 * These rows are real media, so seeding them starts real work: the
 * bytes are hashed, and each material enters the chapter walk, which
 * spawns an ffmpeg. That work is paid **once** — both walks stamp what
 * they finish and offer a row once whatever came back
 * ([`JobKind::ChapterScan`], "the band is the stamp") — so the cost
 * lands in the run that first seeds a fixture and not in later ones.
 * Sorting last is what keeps that one-time cost off the clock of a spec
 * being timed, and it is the whole of the reason: the rows stay,
 * additively, the way every other fixture in this directory does.
 *
 * # An unexplained neighbour
 *
 * `card-trash`'s first context-menu step is intermittent on the machine
 * this was written on, and its rate moves with the presence of this
 * file in a way nothing here accounts for [measured 2026-09-08]: five
 * failures in six runs with this file in the directory, and a pass in
 * the one run without it — but it failed with this spec sorted first
 * and sorted last, it failed once when run alone with this spec not
 * executing at all, and purging these rows between runs (tried, then
 * reverted) changed nothing. That spec carries the tightest budgets
 * here and documents an earlier timing flake of its own.
 *
 * Recorded rather than diagnosed, and deliberately not fixed from this
 * side: a remedy aimed at a mechanism nobody has established is how a
 * suite acquires machinery that protects against nothing.
 */
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

// --- budgets -------------------------------------------------------
const DRIVER_MS = 15_000;
const GRID_MS = 20_000;
const COLD_MS = 60_000;
const SEED_MS = 60_000;
/** One second of audio, already on disk, decoded locally. The ceiling
 *  is for a decoder that never answers at all, not for a slow one. */
const SETTLE_MS = 20_000;
const POLL_GAP_MS = 250;

// --- selectors -----------------------------------------------------
const DETAIL_PANEL = ".detail-panel";
const DETAIL_CLOSE = ".detail-close";
const AUDIO_SURFACE = ".detail-media-audio";
const AUDIO_EL = ".detail-media-audio audio";
const WAVEFORM_CANVAS = ".detail-media-audio .waveform-canvas";
const WAVEFORM_PLACEHOLDER = ".detail-media-audio .waveform-placeholder";

// --- the formats under measurement ---------------------------------
const PACK_ID = "e2e-audio-decode";
const PERSONA_NAME = "Audio decode";

/**
 * One case per answer the pipeline can produce, which is not the same
 * as one per extension.
 *
 * The mime is what the app records and what `KNOWN_AUDIO_MIMES` closes
 * over, so every entry in that list needs a row here. `.opus` earns a
 * second row against `.ogg` anyway: both record `audio/ogg` — a mime
 * names a container — while the codec inside differs, and the codec is
 * what a decoder either has or does not. Reading them as one case would
 * let a webview that carries Opus and not Vorbis answer for both.
 *
 * Two extensions have no row of their own, each for the same reason:
 * `.oga` is the Ogg container `.ogg` already covers, and `.aif` is the
 * second spelling of AIFF. Both reach a mime a case above already
 * measures, and nothing downstream can tell either from its sibling.
 *
 * The bytes come from `scripts/gen-test-fixtures.py` by way of the
 * audio importer's fixture directory, so what is measured here is what
 * that parser reads back in its own tests.
 */
const CASES = [
  { ext: "mp3", mime: "audio/mpeg", fixture: "tone.mp3" },
  { ext: "wav", mime: "audio/wav", fixture: "tone.wav" },
  { ext: "m4a", mime: "audio/mp4", fixture: "tone.m4a" },
  { ext: "flac", mime: "audio/flac", fixture: "tone.flac" },
  { ext: "ogg", mime: "audio/ogg", fixture: "tone.ogg" },
  { ext: "opus", mime: "audio/ogg", fixture: "tone.opus" },
  { ext: "aac", mime: "audio/aac", fixture: "tone.aac" },
  { ext: "aiff", mime: "audio/aiff", fixture: "tone.aiff" },
] as const;

/** The fixtures are one second at 44.1 kHz. Stated at ingest the way
 *  the importers state it, rather than probed: this spec is measuring
 *  the webview, and a row whose duration came back from ffprobe would
 *  make the probe a dependency of that measurement. */
const DURATION_MS = 1_000;

// --- seeding over the app's own HTTP -------------------------------
const HTTP_PORT = Number(process.env.E2E_APP_PORT ?? 19899);
const BASE_URL = `http://127.0.0.1:${HTTP_PORT}`;

async function api<T>(
  method: string,
  route: string,
  body?: unknown,
): Promise<T> {
  const response = await fetch(`${BASE_URL}${route}`, {
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

interface PersonaDto {
  id: string;
  pack_id: string | null;
}

interface CardDto {
  id: string;
  cover: string | null;
  modality: string | null;
}

interface PageDto {
  items: CardDto[];
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
 * Copies one generated fixture to where the seeded row will point.
 *
 * Copied rather than pointed at in place, because `workspace/` is a
 * symlink to the main checkout in every worktree while
 * `crates/…/tests/fixtures/` is not: a row addressed at the crate path
 * would name the worktree it was seeded from, and outlive it. The rows
 * outlive the run — an asset pointing at nothing fails its hash job on
 * every start from then on — so the stable side is the right side.
 *
 * Re-copied whenever the sizes disagree, so regenerating the fixtures
 * reaches this measurement rather than leaving it on last year's bytes.
 */
function placeFixture(root: string, name: string): string {
  const source = path.join(
    root,
    "crates/asterism-importer-audio/tests/fixtures",
    name,
  );
  if (!fs.existsSync(source)) {
    throw new Error(
      `missing generated fixture ${source} — regenerate with ` +
        "python3 scripts/gen-test-fixtures.py",
    );
  }
  const dir = path.join(root, "workspace/runtime/e2e-fixtures/audio-decode");
  fs.mkdirSync(dir, { recursive: true });
  const dest = path.join(dir, name);
  const stale =
    !fs.existsSync(dest) || fs.statSync(dest).size !== fs.statSync(source).size;
  if (stale) fs.copyFileSync(source, dest);
  return dest;
}

/**
 * Brings the profile to the state the measurement needs, and returns
 * the asset id per case.
 *
 * Additive: find, restore if trashed, create only what is missing —
 * the shape `chapter-band.spec.ts` and `metric-sort` use, for the same
 * reason. A row already carrying the right cover is left alone rather
 * than rewritten.
 *
 * The one exception is a row whose modality disagrees, which is purged
 * and reseeded rather than left. That is not a fixture somebody may
 * have authored — the whole row is this file's — and a row without the
 * modality cannot answer the question this spec asks, so leaving it
 * would mean measuring a decode that never starts.
 */
async function ensureFixtures(): Promise<Map<string, string>> {
  await api<unknown>("GET", "/asterism/health").catch((err) => {
    throw new Error(
      `the app is not serving HTTP on ${BASE_URL} (${String(err)}). ` +
        "The fixtures are seeded over that port, so this run cannot continue. " +
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
  const trashed = (
    await api<PageDto>(
      "GET",
      `/asterism/assets?persona_id=${encodeURIComponent(persona.id)}&trash=trashed&limit=500`,
    )
  ).items;

  const root = repoRoot();
  const ids = new Map<string, string>();

  for (const probe of CASES) {
    const cover = `e2e-audio-decode-${probe.ext}`;
    const standing = live.items.find((c) => c.cover === cover);
    let assetId = standing?.id ?? "";

    // A row whose modality is not `audio` cannot answer this spec's
    // question — the waveform effect gates on the *modality* while the
    // player gates on the mime, so such a row mounts a player and never
    // starts a decode, and its measurement would read "no waveform" for
    // every format including the ones the decoder handles. Purged and
    // reseeded rather than left in place, because unlike a chapter band
    // there is nothing here a person could have authored: the whole row
    // is this file's fixture.
    if (standing && standing.modality !== "audio") {
      await api<unknown>("POST", "/asterism/assets/trash", {
        asset_id: standing.id,
      });
      await api<unknown>("POST", "/asterism/assets/purge", {
        asset_id: standing.id,
      });
      assetId = "";
    }

    if (assetId === "") {
      const buried = trashed.find(
        (c) => c.cover === cover && c.modality === "audio",
      );
      if (buried) {
        await api<unknown>("POST", "/asterism/assets/restore", {
          asset_id: buried.id,
        });
        assetId = buried.id;
      } else {
        const created = await api<{ id: string }>(
          "POST",
          "/asterism/assets/add",
          {
            persona_id: persona.id,
            source_kind: "fs",
            locator: placeFixture(root, probe.fixture),
            // Stated, and load-bearing: the pane opens two gates and they
            // read different fields. The player is mounted off the mime
            // (`guess_mime` → `MimeType::Audio` → `media`), while the
            // waveform's effect asks the asset's modality. A row with the
            // mime and no modality gets the player and never starts a
            // decode — which is the state this spec would then measure as
            // "no waveform" for every format. The audio importer states
            // `audio` here, so this is what a real import looks like.
            modality: "audio",
            occurred_at_ms: 1_700_000_000_000,
            labels: ["e2e-audio-decode"],
            register_note: null,
            platform: null,
            file_size_bytes: null,
            duration_ms: DURATION_MS,
            width_px: null,
            height_px: null,
            extra_json: null,
            cover_hint: cover,
          },
        );
        assetId = created.id;
      }
    }
    ids.set(probe.ext, assetId);
  }
  return ids;
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
          `${String(shotSeq).padStart(3, "0")}_ad_${failed ? "FAIL_" : ""}${safe}.png`,
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
        timer = setTimeout(
          () => reject(new Error(`no answer within ${ms}ms`)),
          ms,
        );
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

const NAME_SHIM =
  "window.__name = window.__name || function (target) { return target; };";

interface Sidebar {
  present: boolean;
  marked: boolean;
  fixtureRowPresent: boolean;
  fixtureRowActive: boolean;
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
        marked:
          (window as unknown as { __audioDecodeMark?: boolean })
            .__audioDecodeMark === true,
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
      (window as unknown as { __audioDecodeMark?: boolean }).__audioDecodeMark =
        true;
    })
    .catch(() => undefined);
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
      const all = list
        ? list.querySelector("li:not(.persona-row) button")
        : null;
      if (all instanceof HTMLElement) {
        all.click();
        return true;
      }
      return false;
    })
    .catch(() => false);
}

async function gridHolds(assetId: string): Promise<boolean> {
  return browser
    .execute(
      (query: string) => document.querySelector(query) !== null,
      `.grid-wrapper .card[data-asset-id="${assetId}"]`,
    )
    .catch(() => false);
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

/**
 * What the two decoders have said so far about the open asset.
 *
 * `settled` is the whole reason this is one read rather than two: the
 * element and the waveform answer independently and at their own pace,
 * and a table printed before both have finished would record "not yet"
 * as "no".
 */
interface Probe {
  surfacePresent: boolean;
  audioPresent: boolean;
  /** `HTMLMediaElement.readyState`: 0 nothing, 1+ metadata parsed. */
  readyState: number;
  /** `HTMLMediaElement.networkState`: 3 is NETWORK_NO_SOURCE. */
  networkState: number;
  /** `MediaError.code`, 4 being MEDIA_ERR_SRC_NOT_SUPPORTED — the
   *  refusal this measurement is looking for. */
  errorCode: number | null;
  /** Seconds, as the element reports them. `NaN` reaches here as null. */
  duration: number | null;
  waveform: WaveformState;
  waveformNote: string;
  settled: boolean;
}

/** What the wrap around the canvas is showing. Four of the five are
 *  `DetailPane.svelte`'s own branches — `canvas` read by the element's
 *  presence and the other three by the placeholder's text. `absent` is
 *  this file's own fifth: no placeholder and no canvas, which is not a
 *  state the pane renders and so is a read that arrived too early. */
type WaveformState = "canvas" | "decoding" | "unavailable" | "none" | "absent";

async function readProbe(): Promise<Probe> {
  return browser
    .execute(
      (
        surfaceSel: string,
        audioSel: string,
        canvasSel: string,
        placeholderSel: string,
      ) => {
        const surface = document.querySelector(surfaceSel);
        const el = document.querySelector(audioSel);
        const audio = el instanceof HTMLAudioElement ? el : null;
        const canvas = document.querySelector(canvasSel);
        const placeholder = document.querySelector(placeholderSel);
        const note = (placeholder?.textContent ?? "").trim();
        const waveform:
          "canvas" | "decoding" | "unavailable" | "none" | "absent" = canvas
          ? "canvas"
          : placeholder === null
            ? "absent"
            : note.startsWith("decoding")
              ? "decoding"
              : note.startsWith("waveform unavailable")
                ? "unavailable"
                : "none";
        const duration =
          audio && Number.isFinite(audio.duration) ? audio.duration : null;
        // Settled means both sides have reached an answer, and for the
        // waveform an answer is a drawn canvas or a stated failure.
        // `none` is neither: it is the branch that renders before the
        // effect has run, so treating it as settled would record "this
        // format has no waveform" for a decode that had not started —
        // which is exactly what the first run of this spec reported,
        // for every case, because the rows it seeded carried no
        // modality and the effect never fired.
        const elementSettled =
          audio !== null &&
          (audio.readyState >= 1 ||
            audio.error !== null ||
            audio.networkState === 3);
        const waveformSettled =
          waveform === "canvas" || waveform === "unavailable";
        return {
          surfacePresent: surface !== null,
          audioPresent: audio !== null,
          readyState: audio?.readyState ?? -1,
          networkState: audio?.networkState ?? -1,
          errorCode: audio?.error?.code ?? null,
          duration,
          waveform,
          waveformNote: note,
          settled: elementSettled && waveformSettled,
        };
      },
      AUDIO_SURFACE,
      AUDIO_EL,
      WAVEFORM_CANVAS,
      WAVEFORM_PLACEHOLDER,
    )
    .catch(() => ({
      surfacePresent: false,
      audioPresent: false,
      readyState: -1,
      networkState: -1,
      errorCode: null,
      duration: null,
      waveform: "absent" as const,
      waveformNote: "",
      settled: false,
    }));
}

async function detailPresent(): Promise<boolean> {
  return browser
    .execute(
      (query: string) => document.querySelector(query) !== null,
      DETAIL_PANEL,
    )
    .catch(() => false);
}

/** One line per case, in the order they were measured. Logged as it is
 *  taken *and* collected for the block `after` prints: the block is the
 *  table a person reads, and the per-case line is what survives a run
 *  that dies before `after` gets to it. */
function reportRow(ext: string, mime: string, probe: Probe): string {
  const play =
    probe.errorCode !== null
      ? `refused (MediaError ${probe.errorCode})`
      : probe.readyState >= 1
        ? `loaded (readyState ${probe.readyState})`
        : `no answer (readyState ${probe.readyState}, network ${probe.networkState})`;
  const wave =
    probe.waveform === "canvas"
      ? "drawn"
      : probe.waveform === "unavailable"
        ? `refused ${probe.waveformNote.replace(/^waveform unavailable\s*/, "")}`
        : probe.waveform === "none"
          ? "never started"
          : probe.waveform;
  const secs = probe.duration === null ? "—" : `${probe.duration.toFixed(2)}s`;
  return `  .${ext.padEnd(5)} ${mime.padEnd(11)} element: ${play.padEnd(30)} waveform: ${wave.padEnd(28)} duration: ${secs}`;
}

// --- the run -------------------------------------------------------

describe("what the packaged webview decodes", () => {
  let personaId = "";
  let ids = new Map<string, string>();
  const measured: string[] = [];

  before(async () => {
    const trail: string[] = [];

    await stage(trail, "install __name shim", DRIVER_MS, () =>
      browser.execute(NAME_SHIM),
    );
    await pollUntil(
      trail,
      "app window paints",
      COLD_MS,
      async () => (await readSidebar("")).present,
      "the app never painted its sidebar",
    );

    ids = await stage(trail, "seed fixtures over HTTP", SEED_MS, () =>
      ensureFixtures(),
    );
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
    await stage(trail, "reinstall __name shim", DRIVER_MS, () =>
      browser.execute(NAME_SHIM),
    );

    // Narrow to the fixture persona. The grid is virtualised, so over a
    // whole profile a fixture card is not guaranteed to be in the DOM
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
  });

  after(async () => {
    // Leave the app the way the specs behind this one expect to find
    // it: no open pane over the grid, no persona filter. Swallowed on
    // purpose — cleanup must never replace the error that got us here.
    await closeDetail().catch(() => false);
    await clearPersonaFilter().catch(() => false);
    // The point of the run, gathered: the per-case lines are already
    // out, interleaved with driver noise, and a table is what they are
    // not until they sit together.
    console.log(
      [
        "",
        "  webview decode measurement — one row per format:",
        ...measured,
        "",
      ].join("\n"),
    );
  });

  for (const probe of CASES) {
    it(`settles on .${probe.ext} (${probe.mime})`, async () => {
      const trail: string[] = [];
      const assetId = ids.get(probe.ext) ?? "";
      expect(assetId).not.toBe("");

      await pollUntil(
        trail,
        `grid holds the .${probe.ext} card`,
        GRID_MS,
        () => gridHolds(assetId),
        `the .${probe.ext} fixture never reached the grid`,
      );
      await stage(trail, `open the .${probe.ext} card`, DRIVER_MS, () =>
        openCard(assetId),
      );
      await pollUntil(
        trail,
        `.${probe.ext} detail pane opens`,
        GRID_MS,
        () => detailPresent(),
        `clicking the .${probe.ext} card did not open the detail pane`,
      );

      // The surface is the claim `KNOWN_AUDIO_MIMES` makes: a format
      // the map names reaches the audio branch of the pane, with a
      // player mounted. Waited for separately from the decode answers
      // below because it is the precondition for them — a pane with no
      // player would fail the three assertions at the end of this test
      // without saying which of the two went wrong.
      await pollUntil(
        trail,
        `.${probe.ext} mounts the audio surface`,
        GRID_MS,
        async () => (await readProbe()).audioPresent,
        `.${probe.ext} recorded ${probe.mime} but the pane mounted no audio player`,
      );

      // Then let both decoders finish. A format the webview refuses
      // settles too — with an error rather than with metadata — so the
      // timeout here means "neither answered", which is a third state
      // and is reported as one.
      const settled = await stage(
        trail,
        `.${probe.ext} decoders settle`,
        SETTLE_MS + DRIVER_MS,
        async () => {
          const deadline = Date.now() + SETTLE_MS;
          for (;;) {
            const seen = await readProbe();
            if (seen.settled || Date.now() >= deadline) return seen;
            await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
          }
        },
      );

      const row = reportRow(probe.ext, probe.mime, settled);
      measured.push(row);
      console.log(row);

      // The measured answer, asserted per half so a failure names which
      // decoder changed its mind rather than only that something did.
      expect(settled.errorCode).toBe(null);
      expect(settled.readyState).toBeGreaterThanOrEqual(1);
      expect(settled.waveform).toBe("canvas");

      await stage(trail, `close the .${probe.ext} pane`, DRIVER_MS, () =>
        closeDetail(),
      );
    });
  }
});
