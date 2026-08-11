// MaterialMark UI arithmetic — and the shape pin for the post command.
//
// Two kinds of assertion live here and they are worth telling apart.
//
// The `hasTimeline` / `positionMsFromMedia` / `markRatio` cases are
// about values a media element actually produces: `NaN` before
// metadata, a negative from a seek that landed short, a `currentTime`
// that runs a frame past the probed duration. Each of those reaches the
// backend as a `start_ms` that fails a CHECK, or reaches the ruler as a
// tick drawn off its end.
//
// The `buildPostCommand` case is a contract pin. Its counterpart is
// `material_mark_service::post` in Rust, which reads `anchor_kind`,
// requires `start_ms` on the temporal arm, and pairs `author_kind` with
// `author_persona_id`. Nothing here can execute that arm, so the test
// asserts the whole object rather than a field at a time: a field
// renamed or dropped on either side shows up as a failing equality
// instead of a post rejected at runtime.
import { describe, expect, it } from "vitest";
import {
  buildPostCommand,
  currentMarkId,
  hasTimeline,
  markRatio,
  positionMsFromMedia,
} from "./material-mark";
import type { MaterialMarkDto } from "../bindings";

function mark(id: string, startMs: number | null): MaterialMarkDto {
  return {
    id,
    asset_id: "asset-1",
    anchor_kind: "temporal",
    start_ms: startMs,
    end_ms: null,
    author_kind: "user",
    author_persona_id: null,
    body: `body ${id}`,
    created_at_ms: 0,
    edited_at_ms: null,
  };
}

describe("hasTimeline", () => {
  it("accepts a positive duration", () => {
    expect(hasTimeline(1)).toBe(true);
    expect(hasTimeline(210_000)).toBe(true);
  });

  it("rejects the still image (no duration recorded)", () => {
    expect(hasTimeline(null)).toBe(false);
    expect(hasTimeline(undefined)).toBe(false);
  });

  it("rejects a zero duration — nothing to place a mark on", () => {
    expect(hasTimeline(0)).toBe(false);
  });

  it("rejects a non-finite duration", () => {
    expect(hasTimeline(Number.NaN)).toBe(false);
    expect(hasTimeline(Number.POSITIVE_INFINITY)).toBe(false);
  });
});

describe("positionMsFromMedia", () => {
  it("converts seconds to whole milliseconds", () => {
    expect(positionMsFromMedia(12.345)).toBe(12345);
  });

  it("rounds to the nearest millisecond rather than truncating", () => {
    expect(positionMsFromMedia(1.2346)).toBe(1235);
  });

  it("folds NaN to zero — currentTime before metadata loads", () => {
    expect(positionMsFromMedia(Number.NaN)).toBe(0);
  });

  it("folds a negative position to zero", () => {
    // start_ms >= 0 is a CHECK on the table; a negative would be
    // rejected at the far end of the round trip.
    expect(positionMsFromMedia(-0.5)).toBe(0);
  });

  it("keeps the origin at zero", () => {
    expect(positionMsFromMedia(0)).toBe(0);
  });
});

describe("markRatio", () => {
  it("places a mark proportionally along the timeline", () => {
    expect(markRatio(30_000, 120_000)).toBe(0.25);
  });

  it("returns null when the asset has no timeline", () => {
    expect(markRatio(30_000, null)).toBeNull();
    expect(markRatio(30_000, 0)).toBeNull();
  });

  it("returns null for a mark with no position (a non-temporal anchor)", () => {
    expect(markRatio(null, 120_000)).toBeNull();
  });

  it("clamps a position past the probed duration to the ruler's end", () => {
    // The player and the importer's probe disagree by a frame on some
    // containers; the tick belongs on the end, not past it.
    expect(markRatio(120_400, 120_000)).toBe(1);
  });

  it("clamps a negative position to the ruler's start", () => {
    expect(markRatio(-5, 120_000)).toBe(0);
  });
});

describe("buildPostCommand", () => {
  it("builds a point mark with the anchor the backend accepts", () => {
    expect(buildPostCommand("asset-1", 30_000, "here")).toEqual({
      asset_id: "asset-1",
      anchor_kind: "temporal",
      start_ms: 30_000,
      end_ms: null,
      author_kind: "user",
      author_persona_id: null,
      body: "here",
    });
  });

  it("trims the body it sends", () => {
    expect(buildPostCommand("asset-1", 0, "  spaced  ")?.body).toBe("spaced");
  });

  it("refuses a body that is only whitespace", () => {
    // The domain's `validate()` rejects an empty body after a Rust
    // trim; refusing here keeps that from being a round trip.
    expect(buildPostCommand("asset-1", 0, "   ")).toBeNull();
    expect(buildPostCommand("asset-1", 0, "")).toBeNull();
  });

  it("refuses a position that could not be stored", () => {
    expect(buildPostCommand("asset-1", -1, "here")).toBeNull();
    expect(buildPostCommand("asset-1", Number.NaN, "here")).toBeNull();
  });

  it("rounds a fractional position", () => {
    expect(buildPostCommand("asset-1", 1234.6, "here")?.start_ms).toBe(1235);
  });
});

describe("currentMarkId", () => {
  const marks = [mark("a", 0), mark("b", 10_000), mark("c", 25_000)];

  it("names the last mark at or before the playhead", () => {
    expect(currentMarkId(marks, 12_000)).toBe("b");
  });

  it("counts a mark the playhead has exactly reached", () => {
    expect(currentMarkId(marks, 10_000)).toBe("b");
  });

  it("names the final mark once the playhead is past all of them", () => {
    expect(currentMarkId(marks, 99_000)).toBe("c");
  });

  it("returns null when the playhead precedes every mark", () => {
    expect(currentMarkId([mark("b", 10_000)], 500)).toBeNull();
  });

  it("returns null for an empty list", () => {
    expect(currentMarkId([], 500)).toBeNull();
  });

  it("skips a mark with no position instead of counting it as zero", () => {
    // A spatial anchor would carry start_ms = null. None exist today,
    // but reading one as "at the origin" would make it the answer for
    // every playhead position.
    expect(currentMarkId([mark("s", null), mark("b", 10_000)], 500)).toBeNull();
  });
});
