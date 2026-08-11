// material-mark — the arithmetic behind the MaterialMark surface,
// separated from the component that renders it.
//
// A mark is a note at a position inside what an asset holds, not a note
// about the asset (that is `asset_comment`). Today the only anchor the
// backend accepts is `temporal`: a point on the playback timeline, in
// milliseconds from its origin. The three things that turn a
// `<video>` / `<audio>` element into that number — and the one thing
// that turns it back into a screen position — live here rather than in
// `MaterialMarks.svelte`, because vitest runs on Node and cannot mount
// a component (`vite.config.ts` `test.environment: "node"`).
//
// The command builder is here for the same reason the grid comparator
// was pulled out of the component: `PostMaterialMarkCommand` is one
// half of a two-sided contract, and the other half
// (`material_mark_service::post`) is Rust. A test that pins the shape
// this side sends is the only thing that fails when the two drift.
import type { MaterialMarkDto, PostMaterialMarkCommand } from "../bindings";

/// Whether an asset's material has a timeline to mark at all.
///
/// `duration_ms` is the backend's answer to "do these bytes run for a
/// while": a still image has none, and a video whose probe failed has
/// none either. The service refuses a temporal anchor in both cases
/// (`material_mark_service.rs`, the `temporal` arm), so the UI asks the
/// same question before offering the surface — a disabled compose box
/// is a better answer than a rejected post.
export function hasTimeline(durationMs: number | null | undefined): boolean {
  return typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs > 0;
}

/// The playback position a media element is at, in whole milliseconds.
///
/// `HTMLMediaElement.currentTime` is seconds as a float and is `NaN`
/// before metadata loads; both of those reach the backend as a
/// `start_ms` that fails the `start_ms >= 0` CHECK, so they are folded
/// to 0 here. Rounding (not truncating) keeps a mark placed at the
/// frame the user was looking at rather than up to a millisecond
/// before it.
export function positionMsFromMedia(currentTimeSec: number): number {
  if (!Number.isFinite(currentTimeSec) || currentTimeSec <= 0) return 0;
  return Math.round(currentTimeSec * 1000);
}

/// Where a mark sits along the timeline, as a 0..1 fraction, or `null`
/// when there is no timeline to sit on.
///
/// Clamped at both ends on purpose. `duration_ms` is what the importer
/// probed and `start_ms` is what the player reported, and the two
/// disagree by a frame or so on some containers; a tick drawn at
/// 100.3% would leave the ruler instead of resting on its end.
export function markRatio(
  startMs: number | null,
  durationMs: number | null | undefined,
): number | null {
  if (!hasTimeline(durationMs)) return null;
  if (startMs === null || !Number.isFinite(startMs)) return null;
  return Math.min(1, Math.max(0, startMs / (durationMs as number)));
}

/// Builds the post command for a point mark, or `null` when there is
/// nothing to post.
///
/// Three of the fields are constants at this surface and are spelled
/// out rather than defaulted, because each is a decision:
///   * `anchor_kind: "temporal"` — the only variant the backend knows.
///   * `end_ms: null` — an instant. The detail pane offers no interval
///     UI, and `None` names the moment itself, not "from here to the
///     end" (`material_mark.rs` `TimelineSpan`).
///   * `author_kind: "user"` — the pane's mark composer posts as the
///     person driving it. A persona posts marks through the MCP tool,
///     which carries its own attribution.
export function buildPostCommand(
  assetId: string,
  startMs: number,
  body: string,
): PostMaterialMarkCommand | null {
  const trimmed = body.trim();
  if (trimmed.length === 0) return null;
  if (!Number.isFinite(startMs) || startMs < 0) return null;
  return {
    asset_id: assetId,
    anchor_kind: "temporal",
    start_ms: Math.round(startMs),
    end_ms: null,
    author_kind: "user",
    author_persona_id: null,
    body: trimmed,
  };
}

/// The mark a viewer is inside of right now — the last one at or before
/// the playhead — or `null` when the playhead sits before every mark.
///
/// Reads the list in the order it arrives. The repository already
/// returns `start_ms` ascending (`ORDER BY start_ms, id`), and re-sorting
/// here would put a second ordering opinion in the client, which is the
/// thing the design says not to do.
export function currentMarkId(
  marks: readonly MaterialMarkDto[],
  positionMs: number,
): string | null {
  let hit: string | null = null;
  for (const m of marks) {
    if (m.start_ms === null) continue;
    if (m.start_ms <= positionMs) hit = m.id;
    else break;
  }
  return hit;
}
