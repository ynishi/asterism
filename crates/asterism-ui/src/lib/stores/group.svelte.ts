// Group catalog — sidebar-facing state for user-curated buckets
// (Groups), the organisation Dirs tree, and the group-in-group edge
// list.
// Three flat backend lists collapse into one store because their
// tree derivations (`childGroupsByParent` / `dirChildren` /
// `groupsByDir`) all cross two of the three axes at once — splitting
// them into three stores would just move the join into every
// consumer. Each axis rides its own Resource: three fetch state
// machines, one domain store.
//
// Scope:
//   - `counts`: Resource over `list_groups` (`GroupSummaryDto[]`,
//     read via `counts.data`). Persona-scoped (invoke takes
//     `personaId`, "all" returns cross-persona rows).
//   - `dirs`: Resource over `list_dirs` (flat `DirDto[]` parent_id
//     list, read via `dirs.data`).
//   - `links`: Resource over `list_group_links` (flat
//     `GroupLinkDto[]` channel-in-channel edges, read via
//     `links.data`).
//   - `nameById`: id → name map over `counts.data`. Hot path for the
//     sort comparator (110k cards resolving `card.group_ids[0]`) and
//     the URL-hydrate name-fill effect in App.svelte.
//   - `dirChildren` / `groupsByDir` / `childGroupsByParent`: tree
//     shape derived once per data reassignment; template consumers
//     use `.get(parentId) ?? []` to render one level at a time.
//
// Deliberately NOT owned here:
//   - `expandedDirs` / `expandedGroups` (ephemeral disclosure UX).
//   - `newGroupName` / `newDirName` / `groupCreateError` / `dirError`
//     (form input state, App-owned until the GroupsSection component
//     lands in wave 5b-2).
//   - `renamingDirId` / `renamingGroupId` / `renameDraft` (inline
//     rename UX, same reasoning as above).
//   - `soloGroupId` (cross-cuts `activeFilter.activeGroupIds` +
//     `activeFilter.viewMode`; App keeps it as a query-side derived).
//
// Reload wiring: App-side `$effect` still owns the persona-change →
// `loadCounts` / `loadDirs` / `loadLinks` chain via thin wrappers.

import type { DirDto, GroupLinkDto, GroupSummaryDto } from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

// Root-level parent key. `DirDto.parent_id` and `GroupSummaryDto.
// group.dir_id` both use `null` for the top level; the maps below
// key by `""` so every bucket stays string-keyed.
const ROOT = "";

class GroupCatalog {
  counts = new Resource(
    (personaId: string | null) =>
      api<GroupSummaryDto[]>("list_groups", { personaId }),
    [] as GroupSummaryDto[],
    "groupCatalog.counts",
  );

  dirs = new Resource(
    (personaId: string | null) => api<DirDto[]>("list_dirs", { personaId }),
    [] as DirDto[],
    "groupCatalog.dirs",
  );

  links = new Resource(
    (personaId: string | null) =>
      api<GroupLinkDto[]>("list_group_links", { personaId }),
    [] as GroupLinkDto[],
    "groupCatalog.links",
  );

  nameById = $derived.by(() => {
    const m = new SvelteMap<string, string>();
    for (const gc of this.counts.data) m.set(gc.group.id, gc.group.name);
    return m;
  });

  // id → kind lookup — the sidebar reads it to skip drop-target /
  // reorder chrome for query groups (their membership is rule-driven,
  // so hand-editing it is gated). The backend command layer is the final
  // gate; the UI mirror is purely for chrome and fast-fail UX.
  kindById = $derived.by(() => {
    const m = new SvelteMap<string, "manual" | "query">();
    for (const gc of this.counts.data) {
      m.set(gc.group.id, gc.group.kind === "query" ? "query" : "manual");
    }
    return m;
  });

  isQueryGroup(id: string): boolean {
    return this.kindById.get(id) === "query";
  }

  dirChildren = $derived.by(() => {
    const map = new Map<string, DirDto[]>();
    for (const d of this.dirs.data) {
      const key = d.parent_id ?? ROOT;
      const bucket = map.get(key) ?? [];
      bucket.push(d);
      map.set(key, bucket);
    }
    return map;
  });

  groupsByDir = $derived.by(() => {
    const map = new Map<string, GroupSummaryDto[]>();
    for (const gc of this.counts.data) {
      const key = gc.group.dir_id ?? ROOT;
      const bucket = map.get(key) ?? [];
      bucket.push(gc);
      map.set(key, bucket);
    }
    return map;
  });

  // Parent group id → ordered child summaries (position order comes
  // from the backend link list).
  childGroupsByParent = $derived.by(() => {
    const byId = new Map(this.counts.data.map((gc) => [gc.group.id, gc]));
    const map = new Map<string, GroupSummaryDto[]>();
    for (const link of this.links.data) {
      const child = byId.get(link.child_group_id);
      if (!child) continue;
      const bucket = map.get(link.parent_group_id) ?? [];
      bucket.push(child);
      map.set(link.parent_group_id, bucket);
    }
    return map;
  });

  async loadCounts(personaId: string | null): Promise<void> {
    await this.counts.load(personaId);
  }

  async loadDirs(personaId: string | null): Promise<void> {
    await this.dirs.load(personaId);
  }

  async loadLinks(personaId: string | null): Promise<void> {
    await this.links.load(personaId);
  }
}

// Exported as `groupCatalog` for parallelism with `personaCatalog` /
// `modalityCatalog` / `tagCatalog`.
export const groupCatalog = new GroupCatalog();
