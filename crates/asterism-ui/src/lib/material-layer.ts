// material-layer — the reading of a material's chapter bands, separated
// from the component that renders them.
//
// A band (`MaterialLayer`) is one reading of how a material is divided,
// and it says who produced that reading: the file's own declaration, a
// person's, or a job's. The chapters hang off the band rather than off
// the asset, which is what lets re-reading a file replace its own list
// without touching the one a person wrote.
//
// Four facts of that wire shape decide almost everything below, and each
// is easy to get wrong in a way no type catches:
//
//   * **A band has no display name.** It is described by what it *is* —
//     `(origin, role)` — and a surface derives a caption from the pair
//     (`material_layer.rs`, "No display name"). A caption stored beside
//     the pair would be the half that drifts.
//   * **A chapter's `id` is not stable across a re-scan.** Re-reading a
//     file replaces an imported band's rows wholesale, so the row a
//     reader was looking at comes back with a new id. `(layer_id, ord)`
//     is what names the same section before and after, and it is what
//     the `{#each}` keys on.
//   * **`end_ms: null` means the section states no end**, not "runs to
//     the end of the media" (`chapter_mark.rs` / `TimelineSpan`). It is
//     rendered as an absent end, never as a computed one.
//   * **Only a `user` band may be written into.** The service refuses a
//     hand edit of an imported or machine band
//     (`material_layer_service::require_user_owned`), so the surface
//     asks the same question before offering the affordance — an absent
//     button is a better answer than a rejected write.
//
// These live here rather than in `MaterialChapters.svelte` for the
// reason `material-mark.ts` gives: vitest runs on Node and cannot mount
// a component (`vite.config.ts` `test.environment: "node"`), and the
// command builders are one half of a two-sided contract whose other half
// is Rust — a test that pins the shape this side sends is the only thing
// that fails when the two drift.
import { fmtDurationMs } from "./formatters";
import type {
  ChapterMarkDto,
  CreateMaterialLayerCommand,
  EditChapterMarkCommand,
  MaterialLayerViewDto,
  PostChapterMarkCommand,
} from "../bindings";

/// The role slug of a band that holds chapters. The other role
/// (`annotation`) holds notes and is read through `list_material_marks`
/// by `MaterialMarks.svelte`; this surface never shows one.
export const STRUCTURE_ROLE = "structure";

/// The origin slug of a band a person owns — the only one a hand edit
/// may touch.
export const USER_ORIGIN = "user";

/// The bands of an asset that hold chapters, in the order the backend
/// handed them over.
///
/// `list_material_layers` returns every band over the material,
/// annotation ones included (their `chapters` is always empty). Filtering
/// by role here rather than asking the backend for a subset keeps the one
/// call that the panel needs: the bands to choose between and the
/// contents of the chosen one arrive at the same moment.
///
/// The order is the backend's (`ord`, then id). Re-sorting would put a
/// second opinion about display order in the client.
export function structureBands(
  views: readonly MaterialLayerViewDto[],
): MaterialLayerViewDto[] {
  return views.filter((v) => v.layer.role === STRUCTURE_ROLE);
}

/// The caption for a band, derived from the `(origin, role)` pair.
///
/// Total over the wire, not over the enum: `origin` and `role` arrive as
/// strings, and a build that reads a row written by a newer one gets a
/// slug it has no variant for. That case falls through to the pair
/// itself, which is at least true, rather than to a caption that claims
/// the band is something it is not.
export function bandLabel(origin: string, role: string): string {
  if (role === STRUCTURE_ROLE) {
    switch (origin) {
      case "imported":
        return "From the file";
      case USER_ORIGIN:
        return "Yours";
      case "machine":
        return "Detected";
    }
  } else if (role === "annotation") {
    switch (origin) {
      case "imported":
        return "Notes from the file";
      case USER_ORIGIN:
        return "Your notes";
      case "machine":
        return "Detected notes";
    }
  }
  return `${origin} ${role}`;
}

/// Whether a person may write into a band of this origin.
///
/// Mirrors `require_user_owned`: imported bands are replaced by reading
/// the file again and machine bands by re-running the job, so neither
/// takes a hand edit.
export function bandEditable(origin: string): boolean {
  return origin === USER_ORIGIN;
}

/// Which band the panel should show: the one already open if it is still
/// there, otherwise the default one, otherwise the first, otherwise
/// nothing.
///
/// Keeping the current choice across a reload is what makes "add a
/// chapter" not throw the reader back to the file's band. The default is
/// only consulted when the current id is gone — which happens when the
/// band was deleted, or when this is the first load and there is no
/// current id at all.
export function pickBandId(
  bands: readonly MaterialLayerViewDto[],
  current: string | null,
): string | null {
  if (current !== null && bands.some((b) => b.layer.id === current)) return current;
  const preferred = bands.find((b) => b.layer.is_default) ?? bands[0];
  return preferred ? preferred.layer.id : null;
}

/// The keys an `{#each}` over a band's chapters should use, positionally.
///
/// **Not the chapter's `id`.** Re-reading a material replaces an imported
/// band's rows, so the same section arrives with a different id and a
/// keyed block would tear down and rebuild every row — losing focus and
/// scroll for a list whose content did not change. `(layer_id, ord)` is
/// the pair that names the same section across that replacement.
///
/// Returns the whole list rather than one key at a time because that
/// pair is **not unique**, and a Svelte key must be. `ord` is the band's
/// stated reading order, but nothing enforces distinctness: the schema's
/// `idx_chapter_mark_layer_ord` is a plain index, the column carries
/// `DEFAULT 0`, and a container is free to declare its sections without
/// one — so a whole band can arrive as `ord = 0`. A repeated key is not
/// a doubled row: Svelte answers `each_key_duplicate` by throwing while
/// reconciling, which takes down the pane the list sits in. That is the
/// failure `DetailPane.test.ts` was written for after a row carried the
/// same label twice on 2026-07-20, and this is the same shape one layer
/// down.
///
/// Repeats are therefore disambiguated by their position *among the
/// repeats* — the first `layer:0` stays `layer:0`, the second becomes
/// `layer:0#1`. That keeps the property the pair was chosen for: the
/// suffix is a function of the order the band states, not of the ids,
/// so a re-scan that renumbers every `id` still produces the same key
/// list for the same sections.
export function chapterRowKeys(chapters: readonly ChapterMarkDto[]): string[] {
  const seen = new Map<string, number>();
  return chapters.map((c) => {
    const base = `${c.layer_id}:${c.ord}`;
    const nth = seen.get(base) ?? 0;
    seen.set(base, nth + 1);
    return nth === 0 ? base : `${base}#${nth}`;
  });
}

/// The `ord` a chapter appended to this band should carry.
///
/// One past the highest in the band, rather than the row count: a band
/// whose middle row was deleted has a gap, and reusing an `ord` that is
/// still in the band would make two rows share a key.
export function nextChapterOrd(chapters: readonly ChapterMarkDto[]): number {
  let max = -1;
  for (const c of chapters) {
    if (Number.isFinite(c.ord) && c.ord > max) max = c.ord;
  }
  return max + 1;
}

/// Builds the command that opens a band of one's own over an asset's
/// primary material.
///
/// `material_ord: null` names the primary original — the axis
/// `asset.duration_ms` measures and `HTMLMediaElement.currentTime`
/// reports (`material_layer.rs` `PRIMARY_MATERIAL_ORD`). The pane plays
/// that one, so it is the one it marks.
///
/// `ord` is one past the bands already there, for the same reason
/// `nextChapterOrd` is: it decides display order, and a repeat would
/// leave two bands with no stated order between them.
export function buildCreateBandCommand(
  assetId: string,
  bands: readonly MaterialLayerViewDto[],
): CreateMaterialLayerCommand {
  let max = -1;
  for (const b of bands) {
    if (Number.isFinite(b.layer.ord) && b.layer.ord > max) max = b.layer.ord;
  }
  return {
    asset_id: assetId,
    material_ord: null,
    role: STRUCTURE_ROLE,
    ord: max + 1,
  };
}

/// Builds the command that adds a section at the playhead, or `null`
/// when there is nothing to add.
///
/// `end_ms: null` is a section that states no end, which is what the
/// panel can honestly say: the person marked where a section *starts*,
/// and nothing was said about where it stops. An end computed from the
/// next chapter would be the panel inventing a claim the person did not
/// make.
///
/// An empty label is legal here, unlike a mark's body — a container is
/// free to declare an untitled section and a person is free to write
/// one (`chapter_mark.rs` `validate`). What is refused is a position
/// that is not one.
export function buildPostChapterCommand(
  layerId: string,
  startMs: number,
  label: string,
  chapters: readonly ChapterMarkDto[],
): PostChapterMarkCommand | null {
  if (!Number.isFinite(startMs) || startMs < 0) return null;
  return {
    layer_id: layerId,
    start_ms: Math.round(startMs),
    end_ms: null,
    label: label.trim(),
    ord: nextChapterOrd(chapters),
  };
}

/// Builds the command that retitles a section, or `null` when the title
/// is the one it already has.
///
/// `start_ms: null` is the load-bearing field: it means "leave the
/// section where it is". Sending the stored start back instead would
/// travel through the same arm that a move does and re-validate a span
/// nobody asked to change — and, because `end_ms` is only read when
/// `start_ms` is present, would quietly restate the end as well
/// (`material_layer_service::edit_chapter_mark`).
export function buildRenameCommand(
  chapter: ChapterMarkDto,
  label: string,
): EditChapterMarkCommand | null {
  const trimmed = label.trim();
  if (trimmed === chapter.label) return null;
  return {
    layer_id: chapter.layer_id,
    chapter_id: chapter.id,
    label: trimmed,
    start_ms: null,
    end_ms: null,
    ord: null,
  };
}

/// Builds the command that moves a section to a new start, or `null`
/// when it is already there.
///
/// Carries the label and the end it already had. The label because the
/// field is not optional — an edit always states a title, and the one it
/// states here is the unchanged one. The end because `end_ms` is read
/// only alongside `start_ms`, so omitting it while moving would silently
/// convert a section with a stated end into one without.
///
/// Keeping the end can make the request invalid: dragging a start past
/// its own end is an inverted interval, which `TimelineSpan::new`
/// refuses. That refusal is surfaced rather than avoided — dropping the
/// end here to make the write succeed would discard a stated fact to
/// spare the caller an error message.
export function buildMoveCommand(
  chapter: ChapterMarkDto,
  startMs: number,
): EditChapterMarkCommand | null {
  if (!Number.isFinite(startMs) || startMs < 0) return null;
  const rounded = Math.round(startMs);
  if (rounded === chapter.start_ms) return null;
  return {
    layer_id: chapter.layer_id,
    chapter_id: chapter.id,
    label: chapter.label,
    start_ms: rounded,
    end_ms: chapter.end_ms,
    ord: null,
  };
}

/// How a section's extent reads: its start alone when it states no end,
/// `start – end` when it states one.
///
/// The absent end is printed as absent. `end_ms: null` says the section
/// declares no end, and the two readings a computed end would invite —
/// "runs to the next chapter", "runs to the end of the media" — are both
/// claims the data does not make.
export function chapterRangeLabel(startMs: number, endMs: number | null): string {
  const start = fmtDurationMs(startMs);
  return endMs === null ? start : `${start} – ${fmtDurationMs(endMs)}`;
}

/// The sentence to show in place of a chapter list, or `null` when there
/// are chapters to show.
///
/// The two empty cases are different facts and are never given the same
/// words:
///
///   * **No structure band at all** — nobody has looked. The material
///     has not been read for chapters (or was imported before it was
///     read for them), and what the file declares is still unknown.
///   * **A band holding no chapters** — somebody looked and the answer
///     was "none". For an imported band that is the file's own statement
///     that it declares no divisions; for a band of one's own it is
///     simply an empty list waiting to be written.
///
/// Collapsing the two into "No chapters" would report an unasked
/// question as an answered one.
export function chapterListNote(
  bands: readonly MaterialLayerViewDto[],
  active: MaterialLayerViewDto | null,
): string | null {
  if (bands.length === 0) return "This material has not been read for chapters.";
  if (active === null) return "This material has not been read for chapters.";
  if (active.chapters.length > 0) return null;
  return active.layer.origin === USER_ORIGIN
    ? "No sections in this band yet."
    : "This file declares no chapters.";
}
