// Merge dialog state machine tests.
//
// What these pin is the order the two calls are allowed to happen in,
// not the markup. The dialog is DOM and this suite runs in node, so
// every rule below would be unchecked if it lived in the component —
// and these are the rules worth being wrong about, because the second
// call is a fold and a fold does not come back:
//
//   - a commit only after a preview of the plan now on screen,
//   - changing the keeper throws that preview away,
//   - the two calls carry the same plan and differ only in the flag,
//   - the rows named are the rows that were on screen when the ruling
//     started, not the ones selected by the time it was confirmed.
//
// The api choke point is mocked; the catalog method and the state
// machine both run for real.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MergeAssetsCommand, MergeAssetsDto } from "../../bindings";
import { api } from "../api";
import {
  mergeDialog,
  mergeRowsLine,
  mergeTotalLines,
  mergeWarningNote,
} from "./merge-dialog.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

function dto(overrides: Partial<MergeAssetsDto> = {}): MergeAssetsDto {
  return {
    keeper_id: "a",
    folded_ids: ["b", "c"],
    already_folded_ids: [],
    refusals: [],
    warnings: [],
    totals: {
      edges_repointed: 0,
      edges_dropped: 0,
      buckets_moved: 0,
      children_repointed: 0,
      tags_moved: 0,
      comments_moved: 0,
      threads_reanchored: 0,
      columns_merged: 0,
      values_discarded: 0,
    },
    committed: true,
    ...overrides,
  };
}

/** The command the mock was called with on its `n`th call. */
function sentCommand(n: number): MergeAssetsCommand {
  const args = apiMock.mock.calls[n]?.[1] as { command: MergeAssetsCommand };
  return args.command;
}

/** Opens a ruling over a, b, c keeping a, and previews it. */
async function previewed(preview: MergeAssetsDto = dto({ committed: false })) {
  mergeDialog.start(["a", "b", "c"]);
  mergeDialog.chooseKeeper("a");
  apiMock.mockResolvedValueOnce(preview);
  await mergeDialog.runPreview();
}

beforeEach(() => {
  apiMock.mockReset();
  mergeDialog.close();
});

describe("what a preview says", () => {
  // The rule that keeps a preview honest about scope: every count the
  // DTO carries has to be sayable. A field with no label would be a
  // number the backend reported and the dialog dropped, on an
  // operation with no undo. (The `Record<keyof MergeTotalsDto, …>` type
  // is the other half — a new field stops the build — but this is what
  // catches a label added and then never reached.)
  it("can name every count the backend reports", () => {
    const totals = dto().totals;
    const allOnes = Object.fromEntries(
      Object.keys(totals).map((k) => [k, 1]),
    ) as typeof totals;
    expect(mergeTotalLines(allOnes)).toHaveLength(Object.keys(totals).length);
  });

  it("drops the zeroes", () => {
    const lines = mergeTotalLines({ ...dto().totals, tags_moved: 3 });
    expect(lines).toEqual([{ label: "tags moved", count: 3 }]);
  });

  it("says what is already true separately from what will change", () => {
    // Not counted in: a row already folded into this keeper is the plan
    // already holding, and somebody who ticked three rows and reads
    // "2 rows fold" has to be told where the third went.
    const line = mergeRowsLine(dto({ folded_ids: ["b", "c"], already_folded_ids: ["d"] }));
    expect(line).toContain("2 rows fold");
    expect(line).toContain("1 more is already folded into it");
  });

  it("reports an unrecognised warning verbatim", () => {
    expect(mergeWarningNote("something-new")).toContain("something-new");
  });
});

describe("mergeDialog — the plan", () => {
  it("folds everything but the keeper, in the order the panel drew them", () => {
    mergeDialog.start(["z", "a", "m"]);
    mergeDialog.chooseKeeper("a");
    // Not sorted: the merge folds in this order and the keeper's note
    // is concatenated in it.
    expect(mergeDialog.discardIds).toEqual(["z", "m"]);
  });

  it("holds the rows that were on screen when the ruling started", async () => {
    const selection = ["a", "b"];
    mergeDialog.start(selection);
    // The panel's selection is live. A row ticked after the dialog
    // opened was not part of what the person ruled over.
    selection.push("c");
    mergeDialog.chooseKeeper("a");
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await mergeDialog.runPreview();
    expect(sentCommand(0).member_ids).toEqual(["a", "b"]);
  });

  it("cannot preview a ruling with no second row", () => {
    mergeDialog.start(["a"]);
    mergeDialog.chooseKeeper("a");
    // `MergePlan::declare` refuses this command; the button that would
    // send it is off before it gets there.
    expect(mergeDialog.canPreview).toBe(false);
  });

  it("cannot preview before a survivor is named", () => {
    mergeDialog.start(["a", "b"]);
    expect(mergeDialog.canPreview).toBe(false);
  });
});

describe("mergeDialog — preview before commit", () => {
  it("previews with dry_run true", async () => {
    await previewed();
    expect(apiMock).toHaveBeenCalledTimes(1);
    expect(apiMock.mock.calls[0][0]).toBe("merge_assets");
    expect(sentCommand(0)).toEqual({
      keeper_id: "a",
      discard_ids: ["b", "c"],
      member_ids: ["a", "b", "c"],
      dry_run: true,
    });
    expect(mergeDialog.phase).toBe("preview");
  });

  it("refuses to commit what was never previewed", async () => {
    mergeDialog.start(["a", "b"]);
    mergeDialog.chooseKeeper("a");
    expect(mergeDialog.canCommit).toBe(false);
    const result = await mergeDialog.commit();
    expect(result).toBeNull();
    // The point of the rule: no call reached the backend. The preview
    // is the only moment the warnings are computed, so a commit that
    // skipped it would fold rows nobody was warned about.
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("throws the preview away when the keeper changes", async () => {
    await previewed();
    expect(mergeDialog.canCommit).toBe(true);
    mergeDialog.chooseKeeper("b");
    // The preview described keeping `a`. Confirming it now would run a
    // fold the screen never described.
    expect(mergeDialog.preview).toBeNull();
    expect(mergeDialog.canCommit).toBe(false);
    expect(mergeDialog.phase).toBe("choosing");
  });

  it("commits the same plan with the flag flipped", async () => {
    await previewed();
    apiMock.mockResolvedValueOnce(dto());
    await mergeDialog.commit();
    expect(apiMock).toHaveBeenCalledTimes(2);
    const { dry_run: previewFlag, ...previewPlan } = sentCommand(0);
    const { dry_run: commitFlag, ...commitPlan } = sentCommand(1);
    expect(commitPlan).toEqual(previewPlan);
    expect(previewFlag).toBe(true);
    expect(commitFlag).toBe(false);
  });

  it("exposes the warnings the preview came back with", async () => {
    await previewed(
      dto({
        committed: false,
        warnings: [{ keeper_id: "a", headstone_id: "b", kind: "lineage" }],
      }),
    );
    // The commit branch returns none by contract, so this is the only
    // copy there will be.
    expect(mergeDialog.preview?.warnings).toHaveLength(1);
  });
});

describe("mergeDialog — outcomes", () => {
  it("answers the committed DTO and closes", async () => {
    await previewed();
    const committed = dto();
    apiMock.mockResolvedValueOnce(committed);
    const result = await mergeDialog.commit();
    // The caller's one job with this return value is to re-read the
    // panel, so a committed run is the only thing that is not null.
    expect(result).toEqual(committed);
    expect(mergeDialog.phase).toBe("closed");
    expect(mergeDialog.isOpen).toBe(false);
    expect(mergeDialog.members).toEqual([]);
  });

  it("stays open on a refusal and keeps what was refused", async () => {
    await previewed();
    const refused = dto({
      folded_ids: [],
      refusals: [{ asset_id: "b", reason: "the keeper is in the trash" }],
      committed: false,
    });
    apiMock.mockResolvedValueOnce(refused);
    const result = await mergeDialog.commit();
    // A refusal is a 200 with `committed: false`, not a throw — and
    // nothing was written, so the caller must not reload as if it had.
    expect(result).toBeNull();
    expect(mergeDialog.phase).toBe("refused");
    expect(mergeDialog.isOpen).toBe(true);
    expect(mergeDialog.refusal?.refusals).toHaveLength(1);
    expect(mergeDialog.error).toBeNull();
  });

  it("keeps the preview when the commit call throws", async () => {
    await previewed();
    apiMock.mockRejectedValueOnce(new Error("backend gone"));
    const result = await mergeDialog.commit();
    expect(result).toBeNull();
    expect(mergeDialog.error).toContain("backend gone");
    // Back where it was: pressing confirm again must not make somebody
    // run the dry run a second time to get back to the same screen.
    expect(mergeDialog.phase).toBe("preview");
    expect(mergeDialog.canCommit).toBe(true);
  });

  it("drops back to choosing when the preview call throws", async () => {
    mergeDialog.start(["a", "b"]);
    mergeDialog.chooseKeeper("a");
    apiMock.mockRejectedValueOnce(
      new Error("member_ids does not account for every declared member"),
    );
    await mergeDialog.runPreview();
    expect(mergeDialog.error).toContain("does not account for every declared member");
    expect(mergeDialog.preview).toBeNull();
    expect(mergeDialog.phase).toBe("choosing");
    expect(mergeDialog.canCommit).toBe(false);
  });

  it("leaves nothing behind for the next ruling", async () => {
    await previewed(
      dto({
        committed: false,
        warnings: [{ keeper_id: "a", headstone_id: "b", kind: "lineage" }],
      }),
    );
    mergeDialog.close();
    mergeDialog.start(["x", "y"]);
    // A keeper or a preview carried over from the last ruling would be
    // a screen describing rows this one is not about.
    expect(mergeDialog.keeperId).toBeNull();
    expect(mergeDialog.preview).toBeNull();
    expect(mergeDialog.refusal).toBeNull();
    expect(mergeDialog.error).toBeNull();
    expect(mergeDialog.canCommit).toBe(false);
  });
});
