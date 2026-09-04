// Shared display formatters. One
// canonical implementation of the pure display helpers that App /
// DetailPane / SessionsView all reach for. Before wave B these
// lived on App and were threaded through as callback props — the
// utility half of the DetailPane 25-prop signature. Now every
// consumer imports directly, so the props vanish and there's a
// single home for the format rules (bytes → KB/MB rounding,
// ms → mm:ss / hh:mm:ss cascade, etc.).
//
// Not owned here:
//   - `detailSrc(locator, assetId)` — reads the App-side thumb
//     cache (`thumbUrls` / `thumbTick` / `ensureThumb`) so it
//     isn't pure. Stays App-side + threaded through as a prop
//     until the thumb cache gets its own catalog (candidate for
//     the future gridCatalog / thumbCache extraction).

import DOMPurify from "dompurify";
import { marked } from "marked";
import type { AssetDetailDto } from "../bindings";
import { personaCatalog } from "./stores/personas.svelte";

// Persona id → name resolver. Reads `personaCatalog.nameById`,
// which is a $derived `SvelteMap` on the store — templates that
// call this inside a reactive context re-render as expected when
// the catalog list reassigns.
export function personaName(id: string): string {
  return personaCatalog.nameById.get(id) ?? "?";
}

// Shown for a note whose Persona author has been purged. The body
// survives the author (schema V68); the identity does not, because
// the row that answered "which Persona" is the row the User deleted.
export const DELETED_PERSONA_LABEL = "(deleted persona)";

// Byline for the note vocabulary `AssetCommentDto` and
// `MaterialMarkDto` share — `author_kind` plus a nullable
// `author_persona_id`. Three surfaces render it (the card thread, the
// session hover, the material marks list), so the three-way reading
// lives here rather than being spelled out at each of them.
//
// The two nulls are different facts and read differently:
//
//   - `author_persona_id == null` on a persona note is the purged
//     author. There is no id to look up, and never will be.
//   - a non-null id the catalog cannot name is `personaName`'s "?" —
//     the catalog has not loaded it, which is a transient miss rather
//     than a deletion. Saying "(deleted persona)" there would report a
//     purge that did not happen.
export function noteAuthorLabel(
  authorKind: string | null,
  authorPersonaId: string | null,
): string {
  if (authorKind === "user") return "You";
  if (authorPersonaId == null) return DELETED_PERSONA_LABEL;
  return personaName(authorPersonaId);
}

// Formats a duration in ms as `mm:ss` (or `hh:mm:ss` past an
// hour). Used by the video / audio detail meta rows so the raw
// millisecond count reads at a glance.
export function fmtDurationMs(ms: number | null): string {
  if (ms == null || !Number.isFinite(ms)) return "?";
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (v: number) => v.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

// Formats a stored pixel pair as `4000 × 1000`, or `null` when the row
// carries no measurement. Used by the image / video detail meta rows.
//
// Returning `null` rather than a placeholder is the point: "nobody
// measured this" is not a dimension, and the caller drops the whole row
// instead of printing a `—` that reads as an answer. The pair is written
// together or not at all (`AssetService::add` refuses a half), so a
// single side arriving here means something wrote past that gate — shown
// as absent rather than as `4000 × ?`.
//
// **The stored pair is the coded one — the byte stream's own dimensions,
// before any orientation is applied.** A photo shot upright with EXIF
// Orientation 6 reads as landscape here, which is what the row beside
// this one (`Orientation`) exists to explain. Video has no such row and
// no display metadata anywhere on the asset, so an upright phone clip
// reads as `1920 × 1080` with nothing to qualify it; that is a gap in
// what the importer measures, not something this formatter can fix.
// Multiplying the pair out is the one reading that survives the
// rotation, which is why the grid's facet uses the product and this row
// does not pretend to.
export function fmtDimensions(
  width: number | null | undefined,
  height: number | null | undefined,
): string | null {
  if (width == null || height == null) return null;
  if (!Number.isFinite(width) || !Number.isFinite(height)) return null;
  return `${width} × ${height}`;
}

export function fmtBytes(n: number | null): string {
  if (n == null) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

// What a pursuit close's outcome reads as, in the words the close
// buttons themselves use (#220) — `satisfied` and `abandoned` are the
// domain's own tokens, not the screen's, so a row or a header printing
// one verbatim is a third vocabulary next to "Closed" and "close · …".
// Shared by `ForgeWork` and `SharedLineWork` rather than duplicated:
// the two close buttons' own text is `close · put it on the line` and
// `close · abandon`, and a second copy of this mapping is a second
// place those two strings could drift out of step with it. A token
// that is not one of the two answers itself, honestly, rather than
// being guessed at.
export function endingWord(outcome: string): string {
  if (outcome === "satisfied") return "closed · put it on the line";
  if (outcome === "abandoned") return "closed · abandon";
  return outcome;
}

export function fmtDateTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} `
    + `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

// EXIF and other source-specific fields ride on the asset as a
// JSON string; parse defensively so a malformed payload does not
// break the panel.
export function parseExtra(
  asset: AssetDetailDto["asset"],
): Record<string, unknown> {
  if (!asset.extra_json) return {};
  try {
    return JSON.parse(asset.extra_json) as Record<string, unknown>;
  } catch {
    return {};
  }
}

// Markdown → sanitized HTML. Sources are local imports, but they
// still pass through DOMPurify so a stray <script> inside an old
// log cannot execute in the webview.
export function renderMarkdown(text: string): string {
  const html = marked.parse(text, {
    async: false,
    gfm: true,
    breaks: true,
  });
  return DOMPurify.sanitize(html);
}

// Text render modes shared by DetailPane and QuickLook — same chip
// strip semantics in both surfaces so muscle memory carries over.
export type DetailMode = "md" | "raw" | "html" | "term";

// Sniff a reasonable initial render mode for a fetched body. The
// `term` ContentKind (and tape / term-log labels) short-circuit to
// term, an HTML prologue routes to html, an ANSI escape hints term;
// everything else falls back to md. Takes the resolved `ContentKind`
// slug rather than the modality slug so this module stays free of any
// store dependency — the caller resolves `kind` off `modalityCatalog`.
export function pickDetailMode(
  text: string | null,
  kind?: string | null,
  labels?: string[] | null,
): DetailMode {
  if (
    kind === "term" ||
    labels?.includes("tape") ||
    labels?.includes("term-log")
  ) {
    return "term";
  }
  if (!text) return "md";
  const head = text.slice(0, 512).trimStart().toLowerCase();
  if (
    head.startsWith("<!doctype") ||
    head.startsWith("<html") ||
    head.startsWith("<body") ||
    head.startsWith("<svg")
  ) {
    return "html";
  }
  // eslint-disable-next-line no-control-regex
  if (/\x1b\[[\d;]*[a-z]/i.test(text)) return "term";
  return "md";
}
