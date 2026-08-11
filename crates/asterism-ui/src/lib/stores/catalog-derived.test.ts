// Catalog derived-map tests. Feeds rows straight
// into the Resource `data` fields (no fetch, no Tauri) and pins the
// tree / lookup derivations. Singleton state is reset per test via
// `Resource.reset()` — the H1 hardening that makes these testable.
import { beforeEach, describe, expect, it } from "vitest";
import type { DirDto, GroupLinkDto, GroupSummaryDto, TagCountDto } from "../../bindings";
import { groupCatalog } from "./group.svelte";
import { tagCatalog } from "./tag.svelte";

function gc(id: string, name: string, dirId: string | null): GroupSummaryDto {
  return {
    group: {
      id,
      persona_id: "p1",
      name,
      dir_id: dirId,
      created_at_ms: 0,
      updated_at_ms: 0,
    },
    asset_count: 0,
  } as GroupSummaryDto;
}

function dir(id: string, name: string, parentId: string | null): DirDto {
  return {
    id,
    persona_id: "p1",
    name,
    parent_id: parentId,
    created_at_ms: 0,
    updated_at_ms: 0,
  } as DirDto;
}

function link(parent: string, child: string, position: number): GroupLinkDto {
  return {
    parent_group_id: parent,
    child_group_id: child,
    position,
  } as GroupLinkDto;
}

describe("tagCatalog.nameById", () => {
  beforeEach(() => tagCatalog.counts.reset());

  it("maps tag id to name over counts.data", () => {
    tagCatalog.counts.data = [
      { tag: { id: "t1", name: "alpha" }, asset_count: 3 },
      { tag: { id: "t2", name: "beta" }, asset_count: 1 },
    ] as TagCountDto[];
    expect(tagCatalog.nameById.get("t1")).toBe("alpha");
    expect(tagCatalog.nameById.get("t2")).toBe("beta");
    expect(tagCatalog.nameById.size).toBe(2);
  });
});

describe("groupCatalog tree derivations", () => {
  beforeEach(() => {
    groupCatalog.counts.reset();
    groupCatalog.dirs.reset();
    groupCatalog.links.reset();
  });

  it("buckets dirs and groups under their parent (null → ROOT '')", () => {
    groupCatalog.dirs.data = [
      dir("d1", "root-dir", null),
      dir("d2", "child-dir", "d1"),
    ];
    groupCatalog.counts.data = [
      gc("g1", "in-root", null),
      gc("g2", "in-d1", "d1"),
      gc("g3", "also-d1", "d1"),
    ];
    expect(groupCatalog.dirChildren.get("")?.map((d) => d.id)).toEqual(["d1"]);
    expect(groupCatalog.dirChildren.get("d1")?.map((d) => d.id)).toEqual(["d2"]);
    expect(groupCatalog.groupsByDir.get("")?.map((g) => g.group.id)).toEqual(["g1"]);
    expect(groupCatalog.groupsByDir.get("d1")?.map((g) => g.group.id)).toEqual([
      "g2",
      "g3",
    ]);
  });

  it("resolves child groups per parent in link order, skipping unknown ids", () => {
    groupCatalog.counts.data = [
      gc("parent", "P", null),
      gc("c1", "C1", null),
      gc("c2", "C2", null),
    ];
    groupCatalog.links.data = [
      link("parent", "c1", 0),
      link("parent", "c2", 1),
      link("parent", "ghost", 2), // no summary row → skipped
    ];
    expect(
      groupCatalog.childGroupsByParent.get("parent")?.map((g) => g.group.id),
    ).toEqual(["c1", "c2"]);
  });

  it("reset() drops rows and the derivations follow", () => {
    groupCatalog.counts.data = [gc("g1", "one", null)];
    expect(groupCatalog.nameById.get("g1")).toBe("one");
    groupCatalog.counts.reset();
    expect(groupCatalog.nameById.size).toBe(0);
  });
});
