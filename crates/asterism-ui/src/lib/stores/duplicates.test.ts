// duplicatesCatalog conflict tests. The api choke
// point is mocked; the catalog's own machinery — the two Resources,
// the answered-set bookkeeping and the command shapes — runs for real.
//
// What these pin is the panel's honesty, not its markup: an empty work
// list that means "not looked yet", a warning that has to appear when
// the automatic fold declined a pair, a `kept` answer that never
// carries a keeper, and a refused answer that leaves the question on
// screen. The panel itself is DOM and this suite runs in node, so any
// of those rules that lived in the component would be untested.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AssetCardDto,
  DuplicateAxis,
  DuplicateConflictDto,
  DuplicateGroupDto,
  MergeAssetsCommand,
  MergeAssetsDto,
} from "../../bindings";
import { api } from "../api";
import {
  contentBacklogNote,
  duplicatesCatalog,
  foldExclusionNote,
  groupKey,
  noConflictsLine,
  noGroupsLine,
} from "./duplicates.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

function side(id: string, locator: string): DuplicateConflictDto["newcomer"] {
  return {
    id,
    persona_id: "p1",
    modality: "image",
    occurred_at_ms: 1,
    cover: null,
    labels: [],
    file_size_bytes: 10,
    duration_ms: null,
    pixel_count: null,
    mime: "image/png",
    media: "image",
    source_locator: locator,
    group_ids: [],
    primary_group_position: null,
    created_at_ms: 1,
    updated_at_ms: 1,
    rating: null,
    palette: null,
    has_note: false,
    has_thread: false,
    role: "asset",
    title: null,
    member_count: 0,
    score: null,
    snippet: null,
    author_kind: null,
    author_subject: null,
    operator_ai: null,
  } as DuplicateConflictDto["newcomer"];
}

function conflict(id: string, exclusion: string | null = null): DuplicateConflictDto {
  return {
    id,
    axis: "artefact",
    content_hash: `hash-${id}`,
    newcomer: side(`${id}-new`, `/inbox/${id}.png`),
    incumbent: side(`${id}-old`, `/library/${id}.png`),
    fold_exclusion: exclusion,
    detected_at_ms: 100,
  };
}

/** Puts rows on both Resources without going through a fetch. */
function seed(conflicts: DuplicateConflictDto[], unhashedCount = 0) {
  duplicatesCatalog.conflicts.data = conflicts;
  duplicatesCatalog.report.data = {
    groups: [],
    unhashed_count: unhashedCount,
    unreadable_count: 0,
    unwalked_count: 0,
  };
}

/** A report as the backend would hand it back. */
function report(groups: DuplicateGroupDto[], unwalkedCount = 0) {
  return {
    groups,
    unhashed_count: 0,
    unreadable_count: 0,
    unwalked_count: unwalkedCount,
  };
}

function group(axis: DuplicateAxis, contentHash: string, ids: string[]): DuplicateGroupDto {
  return {
    axis,
    content_hash: contentHash,
    members: ids.map((id) => side(id, `/pics/${id}.png`) as AssetCardDto),
  };
}

describe("duplicatesCatalog conflicts", () => {
  beforeEach(() => {
    apiMock.mockReset();
    duplicatesCatalog.reset();
  });

  it("reads both endpoints when the panel asks", async () => {
    apiMock.mockImplementation(async (cmd: string) =>
      cmd === "list_duplicate_groups" ? report([]) : [],
    );
    await duplicatesCatalog.load("p1");
    expect(apiMock).toHaveBeenCalledWith("list_duplicate_groups", {
      personaId: "p1",
      axis: "artefact",
      limit: null,
    });
    expect(apiMock).toHaveBeenCalledWith("list_duplicate_conflicts", {
      personaId: "p1",
      limit: null,
    });
  });

  // The trap the report's `unhashed` note was written for, applied to
  // the work list: an empty list before anything has been fingerprinted
  // is not the same answer as an empty list after.
  it("says nothing-yet when files are still to be fingerprinted", () => {
    seed([], 3);
    expect(duplicatesCatalog.openConflicts).toEqual([]);
    expect(noConflictsLine(duplicatesCatalog.unhashed)).toBe(
      "Nothing to answer yet — 3 files still to fingerprint.",
    );
  });

  it("says nothing-to-answer once the fingerprinting is done", () => {
    seed([], 0);
    expect(noConflictsLine(duplicatesCatalog.unhashed)).toBe(
      "Nothing to answer — no duplicate questions are open.",
    );
  });

  // A failed report read falls back to `unhashed_count: 0`, which is
  // indistinguishable from a finished one — so the panel passes null
  // rather than that zero, and the line stops claiming completeness.
  it("does not claim the fingerprinting is done when the backlog is unknown", () => {
    expect(noConflictsLine(null)).toContain("could not be read");
  });

  it("warns on an excluded pair, and hands the decision over", () => {
    const lineage = foldExclusionNote("lineage");
    expect(lineage).toContain("connected by lineage");
    expect(lineage).toContain("you can still fold it by hand");
    const dispatch = foldExclusionNote("dispatch");
    expect(dispatch).toContain("export run");
    expect(dispatch).toContain("you can still fold it by hand");
  });

  it("says nothing when no rule declined the pair", () => {
    expect(foldExclusionNote(null)).toBeNull();
  });

  // A slug the UI has not been taught is still a pair something held
  // back. Reporting it verbatim beats swallowing the warning.
  it("reports an unrecognised exclusion instead of dropping it", () => {
    expect(foldExclusionNote("wormhole")).toContain("(wormhole)");
  });

  it("folding sends the chosen keeper", async () => {
    seed([conflict("c1")]);
    apiMock.mockResolvedValueOnce({
      conflict_id: "c1",
      resolution: "folded",
      resolved_at_ms: 1,
      keeper_id: "c1-old",
      headstone_id: "c1-new",
    });
    await duplicatesCatalog.foldConflict("c1", "c1-old");
    expect(apiMock).toHaveBeenCalledWith("resolve_duplicate_conflict", {
      command: { conflict_id: "c1", resolution: "folded", keeper_id: "c1-old" },
    });
    expect(duplicatesCatalog.openConflicts).toEqual([]);
  });

  it("ruling the pair apart sends no keeper", async () => {
    seed([conflict("c2")]);
    apiMock.mockResolvedValueOnce({
      conflict_id: "c2",
      resolution: "kept",
      resolved_at_ms: 1,
      keeper_id: null,
      headstone_id: null,
    });
    await duplicatesCatalog.keepConflictApart("c2");
    expect(apiMock).toHaveBeenCalledWith("resolve_duplicate_conflict", {
      command: { conflict_id: "c2", resolution: "kept", keeper_id: null },
    });
    expect(duplicatesCatalog.openConflicts).toEqual([]);
  });

  // Neither answer is destructive here — `kept` changes nothing and
  // `folded` only queues a job — so dropping the row optimistically
  // would hide a refusal behind a list that looks resolved.
  it("keeps the question listed when the answer was refused", async () => {
    seed([conflict("c3")]);
    apiMock.mockRejectedValueOnce(new Error("already resolved as kept"));
    await expect(duplicatesCatalog.foldConflict("c3", "c3-old")).rejects.toThrow(
      "already resolved as kept",
    );
    expect(duplicatesCatalog.openConflicts.map((c) => c.id)).toEqual(["c3"]);
    expect(duplicatesCatalog.answered.has("c3")).toBe(false);
  });

  // Switching axis is a refetch, not a relabel. A toggle that changed
  // the heading without re-reading — or re-read the axis it was
  // leaving — would show one fingerprint's groups under the other's
  // name, and the panel's button trashes files.
  it("re-reads the report on the axis it was switched to", async () => {
    apiMock.mockImplementation(async (cmd: string) =>
      cmd === "list_duplicate_groups" ? report([]) : [],
    );
    await duplicatesCatalog.load("p1");
    apiMock.mockClear();

    await duplicatesCatalog.setAxis("content", "p1");
    expect(duplicatesCatalog.axis).toBe("content");
    expect(apiMock).toHaveBeenCalledWith("list_duplicate_groups", {
      personaId: "p1",
      axis: "content",
      limit: null,
    });
    // The queue is not axis-scoped — its rows each carry the axis
    // detection found them on — so switching must not re-read it.
    expect(apiMock).not.toHaveBeenCalledWith("list_duplicate_conflicts", expect.anything());

    // …and a plain reload afterwards stays on the chosen axis rather
    // than snapping back to the default.
    apiMock.mockClear();
    await duplicatesCatalog.load("p1");
    expect(apiMock).toHaveBeenCalledWith("list_duplicate_groups", {
      personaId: "p1",
      axis: "content",
      limit: null,
    });
  });

  // The trap the whole `unwalked` field exists for. Rows with no
  // content reading are in no content group, which is indistinguishable
  // from "no duplicate" unless the count is shown. The values are
  // computed by the column's own migration, so a non-zero count is not
  // pending work — it is originals that could not be opened, and the
  // note has to say that rather than imply a queue that will drain.
  it("tells an empty content report apart from originals it could not read", async () => {
    apiMock.mockResolvedValueOnce(report([], 4_601));
    await duplicatesCatalog.setAxis("content", null);

    expect(duplicatesCatalog.groups).toEqual([]);
    expect(duplicatesCatalog.unwalked).toBe(4_601);

    const note = contentBacklogNote(duplicatesCatalog.unwalked);
    expect(note).toContain("4601");
    expect(note).toContain("could not be read on this axis");
    // The second half: what would change the number. A count with no
    // next step reads as a progress bar, and this one does not move on
    // its own — the files have to come back.
    expect(note).toContain("Putting those files back");
    // …and it must not describe the pass as something still to start.
    expect(note).not.toContain("started on purpose");

    // The "found nothing" half stays a separate sentence, and says
    // which question found nothing.
    expect(noGroupsLine("content")).toContain("same picture");
    expect(noGroupsLine("artefact")).toContain("byte-identical");
  });

  // Singular is a separate sentence in this note, and it is the one a
  // real library is most likely to see (one moved original).
  it("names a single missing original in the singular", () => {
    const note = contentBacklogNote(1);
    expect(note).toContain("1 file could not be read");
    expect(note).toContain("the original was not where");
    expect(note).toContain("Putting that file back");
  });

  it("says nothing about the backlog when there is none", () => {
    expect(contentBacklogNote(0)).toBeNull();
  });

  // A failed report falls back to `unwalked_count: 0`, which reads as a
  // fully walked library. The panel passes null instead, and the line
  // stops claiming coverage it cannot vouch for.
  it("does not claim the library is walked when the backlog is unknown", () => {
    expect(contentBacklogNote(null)).toContain("could not be read");
  });

  // The list key has to survive the list changing axis underneath it,
  // and the panel's "already resolved" set outlives a toggle. Keyed on
  // the digest alone, a group resolved on one axis would vanish from
  // the other — a claim about a pair that nobody made.
  it("keys a group by its axis as well as its digest", () => {
    const shared = "sha256:aaaa";
    expect(groupKey(group("artefact", shared, ["a", "b"]))).not.toBe(
      groupKey(group("content", shared, ["a", "b"])),
    );
    expect(groupKey(group("artefact", shared, ["a", "b"]))).toBe(
      groupKey(group("artefact", shared, ["a", "b"])),
    );
  });

  it("drops only the answered question from the list", async () => {
    seed([conflict("c4"), conflict("c5")]);
    apiMock.mockResolvedValueOnce({
      conflict_id: "c4",
      resolution: "kept",
      resolved_at_ms: 1,
      keeper_id: null,
      headstone_id: null,
    });
    await duplicatesCatalog.keepConflictApart("c4");
    expect(duplicatesCatalog.openConflicts.map((c) => c.id)).toEqual(["c5"]);
  });
});

// The manual merge verb is reachable from the same
// catalog. What these pin is the contract between the store and the
// panel driving it: the command shape sent, the DTO returned, and the
// deliberate absence of local mutation on either branch. Panel button
// wiring is not exercised here — that lives on the DOM side, out of
// this suite's reach.
describe("duplicatesCatalog.mergeAssets", () => {
  beforeEach(() => {
    apiMock.mockReset();
    duplicatesCatalog.reset();
  });

  /** A fully-populated DTO the caller might read after a successful commit. */
  function commitDto(overrides: Partial<MergeAssetsDto> = {}): MergeAssetsDto {
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

  function mergeCommand(dry_run: boolean): MergeAssetsCommand {
    return {
      keeper_id: "a",
      discard_ids: ["b", "c"],
      member_ids: ["a", "b", "c"],
      dry_run,
    };
  }

  // The verb name has to match what the Tauri handler is registered
  // as (`commands::merge_assets`) — a mismatch would be a silent
  // 404-shape on the wire. The command is passed as-is inside `command`,
  // the same envelope `resolve_duplicate_conflict` uses.
  it("sends the command through the `merge_assets` verb", async () => {
    apiMock.mockResolvedValueOnce(commitDto());
    const command = mergeCommand(false);
    await duplicatesCatalog.mergeAssets(command);
    expect(apiMock).toHaveBeenCalledWith("merge_assets", { command });
  });

  it("returns the DTO to the caller unchanged", async () => {
    const backend = commitDto({
      warnings: [{ keeper_id: "a", headstone_id: "b", kind: "lineage" }],
      committed: false,
    });
    apiMock.mockResolvedValueOnce(backend);
    const returned = await duplicatesCatalog.mergeAssets(mergeCommand(true));
    expect(returned).toEqual(backend);
  });

  // A refused merge (keeper trashed, keeper itself folded, …) comes
  // back as a shape, not an exception. `refusals` on the response is
  // the field the caller re-reads, `committed: false` is what stops it
  // being confused with a run.
  it("returns refusals on the DTO rather than throwing", async () => {
    apiMock.mockResolvedValueOnce(
      commitDto({
        folded_ids: [],
        refusals: [
          { asset_id: "b", reason: "the keeper is in the trash" },
          { asset_id: "c", reason: "the keeper is in the trash" },
        ],
        committed: false,
      }),
    );
    const result = await duplicatesCatalog.mergeAssets(mergeCommand(false));
    expect(result.refusals).toHaveLength(2);
    expect(result.committed).toBe(false);
    // …and the store did not treat a refused merge as an answered one.
    // The `answered` set is conflict_id-keyed and manual merge has no
    // conflict_id, so it must not have gained anything either way.
    expect(duplicatesCatalog.answered.size).toBe(0);
  });

  // The store deliberately does not mutate itself on a successful
  // commit. `answered` is conflict_id-keyed and manual merge has no
  // conflict_id; the report groups reflect the fold on the next
  // `load`, and the caller (the panel) drives the reload.
  it("does not mutate the answered set on a successful commit", async () => {
    seed([conflict("c1")]);
    apiMock.mockResolvedValueOnce(commitDto());
    await duplicatesCatalog.mergeAssets(mergeCommand(false));
    expect(duplicatesCatalog.answered.size).toBe(0);
    // The open conflicts list is untouched — it holds queue rows keyed
    // by conflict_id, none of which the merge verb answered.
    expect(duplicatesCatalog.openConflicts.map((c) => c.id)).toEqual(["c1"]);
  });

  // A rejected API call (a validation refusal from `MergePlan::declare`
  // — bad member set) reaches the caller as a throw. Callers wanting
  // to distinguish "the plan itself is malformed" from "the fold could
  // not run" read the shape (`refusals`) versus the exception at their
  // own level; the store does not swallow either.
  it("propagates a rejected api call", async () => {
    apiMock.mockRejectedValueOnce(new Error("member_ids does not account for every declared member"));
    await expect(duplicatesCatalog.mergeAssets(mergeCommand(true))).rejects.toThrow(
      "does not account for every declared member",
    );
  });
});
