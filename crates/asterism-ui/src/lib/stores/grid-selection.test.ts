// gridSelection unit tests.
// The store is a singleton; each test rebuilds the fields it reads.
import { beforeEach, describe, expect, it } from "vitest";
import type { SnapshotDto } from "../../bindings";
import { activeFilter } from "./filter.svelte";
import { gridSelection } from "./grid-selection.svelte";

function sel(personaId: string, assetIds: string[]): SnapshotDto {
  return {
    id: "snap-1",
    persona_id: personaId,
    content_hash: "h",
    asset_ids: assetIds,
    created_at_ms: 0,
  } as SnapshotDto;
}

describe("gridSelection.restore", () => {
  beforeEach(() => {
    activeFilter.reset();
    gridSelection.selectedIds.clear();
    gridSelection.lastAnchorId = null;
  });

  it("flips the persona filter to the row's owner", () => {
    activeFilter.activePersona = "p-other";
    gridSelection.restore(sel("p-owner", ["a1"]));
    expect(activeFilter.activePersona).toBe("p-owner");
  });

  it("replaces the multi-select and sets the anchor to the last id", () => {
    gridSelection.selectedIds.add("stale");
    gridSelection.restore(sel("p1", ["a1", "a2", "a3"]));
    expect(Array.from(gridSelection.selectedIds)).toEqual(["a1", "a2", "a3"]);
    expect(gridSelection.selectedIds.has("stale")).toBe(false);
    expect(gridSelection.lastAnchorId).toBe("a3");
  });

  it("clears the anchor when the selection row is empty", () => {
    gridSelection.lastAnchorId = "old";
    gridSelection.restore(sel("p1", []));
    expect(gridSelection.selectedIds.size).toBe(0);
    expect(gridSelection.lastAnchorId).toBeNull();
  });
});
