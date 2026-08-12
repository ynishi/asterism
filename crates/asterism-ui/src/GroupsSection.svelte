<script lang="ts">
  // GroupsSection — extracted from App.svelte (2026-07-21 Phase C
  // wave 5b-2). Owns the Groups + Dirs sidebar section: the
  // Groups heading, "● all" root drop-target, the recursive
  // {#snippet dirNode} / {#snippet groupRow} tree, inline rename +
  // create / delete inputs, and every sidebar-side drag / drop
  // handler pair (group ↔ dir, group ↔ group, root drop, dir ↔ dir).
  //
  // Cross-boundary state:
  //   - `dragAssetId` (read-only prop) — set by the grid card
  //     ondragstart in App; groupAccepts / onGroupDrop read it to
  //     recognise a "card dropped onto a group" gesture (Are.na
  //     "drop into a channel").
  //   - `dragOverGroupId` (bindable prop) — normally set / cleared
  //     by GroupsSection's own dragenter / dragleave / drop
  //     handlers; App also clears it defensively from
  //     `onCardDragEnd` in case the browser skips the row's
  //     dragleave on drag cancel.
  //   - `onToggleDirFocus(dirId)` — the sidebar dir-row click
  //     toggles App's dir-focus lane above the grid; the actual
  //     lane / breadcrumb rendering stays in App because the lane
  //     is grid-adjacent, not sidebar-internal.
  //   - `onLoadAssets` — invoked after `add_asset_to_group` so the
  //     grid reloads (if the drop destination is the active
  //     filter). The detail-pane cache purge goes straight through
  //     `assetPageCatalog.invalidateDetail` (was the
  //     `onInvalidateDetailAsset` prop before wave ①).
  //
  // State strictly owned here (out of App's reach):
  //   - form inputs: `newGroupName`, `groupCreateError`,
  //     `newDirName`, `dirError`
  //   - disclosure sets: `expandedDirs`, `expandedGroups`
  //   - inline rename: `renamingDirId`, `renamingGroupId`,
  //     `renameDraft`
  //   - sidebar drag: `dragGroupId`, `dragDirId`, `dragOverDirId`
  //
  // `updateQueryFromFilter` writes the same `query_json` v1 shape as
  // App's `saveAsQueryGroup`, including its two predicate-mode fields:
  // `filter.tag_match` ("any" / "all") and a `search_text` that is
  // written only in 🔍 exact mode — a ✦ fuzzy query is not reproducible
  // and so cannot define a persistent set. The sort goes through
  // `activeFilter.persistableSort()` for the same reason on the order
  // axis: `✦ Relevance` is a retriever's sequence, so it drops to the
  // default axis rather than being frozen. The fields this rule
  // omits that App's writes (format / color / collation) are a
  // pre-existing gap, tracked separately.
  //
  // Backend I/O (`invoke`) sits alongside the handlers because a
  // sidebar mutation → catalog reload chain is a single unit; the
  // stores expose the reload primitives (`groupCatalog.loadCounts`
  // etc.) so no zero-arg wrapper needs to survive in App just to
  // bridge into this component.
  //
  // CSS: the group / dir cascade (`.group-row` / `.dir-row` /
  // `.dir-toggle` / `.group-edit` / `.group-delete` /
  // `.drop-target-group` / `.drop-target-root` / `.rename-input` /
  // `.dir-name` / `.nest-badge` / `.nest-mark` / `.group-create` /
  // `.group-error` / `.dir-empty`) plus the shared "row + count"
  // vocabulary (`.tags-list` / `.tag-name` / `.tag-count` /
  // `.tags-empty` / `.tags-active-count`) is duplicated in
  // the scoped style block below because Svelte scoped CSS does not reach across
  // component boundaries. App keeps its copies for the Groups /
  // Dirs use of the shared classes it still renders in other
  // sections (Tags / Selections / SavedQueries) — same pattern as
  // TagList (wave 5a) and ModalityList (wave 4).
  import type {
    DirDto,
    GroupDto,
    GroupSummaryDto,
    UpdateQueryGroupQueryCommand,
  } from "./bindings";
  import { invoke } from "@tauri-apps/api/core";
  import { mutate } from "./lib/mutate";
  import { SvelteSet } from "svelte/reactivity";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
  import { groupCatalog } from "./lib/stores/group.svelte";
  import {
    beginDrag,
    cardDrag,
    type DragSource,
    type DropTarget,
  } from "./lib/interaction/drag.svelte";
  import { interaction } from "./lib/interaction/mode.svelte";

  interface Props {
    onToggleDirFocus: (dirId: string) => void;
  }

  let { onToggleDirFocus }: Props = $props();

  // Root-parent sentinel for drop-target handling. Duplicated (not
  // imported) from `groupCatalog`'s internal `ROOT` because the
  // constant is a private detail of the store's tree assembly.
  const ROOT = "";

  // --- Create form state
  let newGroupName = $state("");
  let groupCreateError = $state("");
  let newDirName = $state("");
  let dirError = $state("");

  // --- Disclosure state (dir subtree open/close, group nesting
  // subtree open/close). Reactive Set primitives from
  // svelte/reactivity so `.add()` / `.delete()` reactively update
  // every template read of `.has(id)`.
  let expandedDirs = new SvelteSet<string>();
  let expandedGroups = new SvelteSet<string>();

  // --- Inline rename state (at most one row in edit mode at a time).
  let renamingDirId = $state<string | null>(null);
  let renamingGroupId = $state<string | null>(null);
  let renameDraft = $state("");

  // --- Sidebar drag payload. Everything carried — a card, a group, a
  // dir — lives in the shared pointer drag store, so these are reads
  // rather than state of their own.
  const dragGroupId = $derived(cardDrag.sourceOf("group"));

  // --- Group context menu. Two kinds of entries share one overlay:
  // query rules ("Expand query into filter" / "Update query",
  // kind === "query" only) and the promote birth record (W6-a
  // "promoted from · <id>" → Snapshot view, any group whose
  // `origin_snapshot_id` is stamped). A manual group with no origin
  // has nothing to show and does not open the menu.
  let queryMenu = $state<{
    x: number;
    y: number;
    group: GroupSummaryDto["group"];
  } | null>(null);
  let queryMenuError = $state("");

  function openQueryMenu(event: MouseEvent, group: GroupSummaryDto["group"]) {
    // Always suppress the WKWebView native context menu on a group
    // row, regardless of kind, so the sidebar's right-click behaviour
    // is uniform (mirrors App's cardMenu convention). Groups with
    // neither a stored rule nor a birth record then short-circuit —
    // the menu would be empty.
    event.preventDefault();
    if (group.kind !== "query" && group.origin_snapshot_id === null) return;
    const menuW = 260;
    const menuH = 180;
    const x = Math.min(event.clientX, window.innerWidth - menuW - 4);
    const y = Math.min(event.clientY, window.innerHeight - menuH - 4);
    queryMenu = { x, y, group };
    queryMenuError = "";
    // Register on the interaction stack (W5 Esc SoT): App's Escape
    // switch sees "queryMenu" on top and defers to this component's
    // own listener instead of clearing the grid selection.
    interaction.push("queryMenu");
  }

  function closeQueryMenu() {
    queryMenu = null;
    interaction.remove("queryMenu");
  }

  function openProvenanceSnapshot(group: GroupSummaryDto["group"]) {
    // W6-a: the "promoted from" entry is one of the two sanctioned
    // Snapshot entrances (the other is the dispatch-history row); both
    // land in the same shared SnapshotView via the catalog.
    if (group.origin_snapshot_id === null) return;
    dispatchCatalog.openSnapshot(group.origin_snapshot_id);
    closeQueryMenu();
  }

  function expandQueryIntoFilter(group: GroupSummaryDto["group"]) {
    // Explicit action: the stored rule replaces the current filter chips
    // + Sorter. `restoreQueryGroup` handles the v1 blob parse + the
    // per-field writes; a corrupt blob surfaces inline instead of a
    // silent no-op.
    const blob = group.query_json;
    if (!blob) {
      queryMenuError = "query group has no stored rule";
      return;
    }
    const ok = activeFilter.restoreQueryGroup(blob);
    if (!ok) {
      queryMenuError = "stored rule is corrupt (unsupported v or bad JSON)";
      return;
    }
    closeQueryMenu();
  }

  async function updateQueryFromFilter(group: GroupSummaryDto["group"]) {
    // "Update query": rewrite the rule from the current filter.
    // Uses the exact same query_json v1 shape as App's
    // `saveAsQueryGroup` (raw group_ids, first-class search_text, no
    // UI-derived paginate fields) so the eval Job reads the two paths
    // identically. Persona semantics: the group's persona is
    // authoritative (the backend evaluates against
    // `group.persona_id` and does not re-validate the rule's own
    // persona_id) — the rule stamps `group.persona_id`, not the
    // ambient filter's active persona, so an ambient "all personas"
    // view or a mid-menu persona flip cannot corrupt the stored rule.
    // The activePersona===null gate stays as a UX hint (filter chips
    // are cross-persona; the user probably meant to narrow first).
    if (activeFilter.activePersona === null) {
      queryMenuError = "pick a single persona to base the rule on";
      return;
    }
    if (activeFilter.activePersona !== group.persona_id) {
      queryMenuError = "current filter persona differs from the group's persona";
      return;
    }
    const searchText = activeFilter.searchText.trim();
    const rule = {
      v: 1,
      filter: {
        persona_id: group.persona_id,
        modality: activeFilter.activeModality,
        occurred_from_ms: null,
        occurred_until_ms: null,
        tag_ids: Array.from(activeFilter.activeTagIds),
        tag_match: activeFilter.tagMatchAll ? "all" : "any",
        group_ids: Array.from(activeFilter.activeGroupIds),
        session_id: activeFilter.activeSessionId,
        label: activeFilter.activeLabel,
        // Length / size bands, in the wire's ms / bytes
        // (`activeFilter.metricBands()` owns the conversion). Written
        // here as well as in `saveAsQueryGroup` because a rule that
        // dropped them would freeze a wider set than the filter the user
        // is looking at while re-writing the group from it.
        ...activeFilter.metricBands(),
      },
      // `✦ Relevance` falls back to the default axis here, same as in
      // `saveAsQueryGroup`: the order comes from a retriever that does
      // not promise the same answer twice, so it cannot define the
      // frozen sequence of a persistent group.
      sort: activeFilter.persistableSort(),
      // 🔍 exact text only, same rule as `saveAsQueryGroup`: a ✦ fuzzy
      // query is not reproducible, so it cannot define a persistent set.
      search_text:
        !activeFilter.searchFuzzy && searchText.length > 0 ? searchText : null,
    };
    const command: UpdateQueryGroupQueryCommand = {
      group_id: group.id,
      query_json: JSON.stringify(rule),
    };
    try {
      await invoke("update_query_group_query", { command });
      // The backend re-evaluates synchronously — refresh the
      // sidebar counts so the new member count paints.
      await groupCatalog.loadCounts(activeFilter.activePersona);
      closeQueryMenu();
    } catch (err) {
      queryMenuError = String(
        (err as { message?: string })?.message ?? err,
      );
    }
  }

  // --- Backend I/O ---------------------------------------------------

  async function createGroup() {
    const name = newGroupName.trim();
    if (name.length === 0 || activeFilter.activePersona === null) {
      groupCreateError = activeFilter.activePersona === null
        ? "pick a persona first"
        : "name required";
      return;
    }
    try {
      await invoke<GroupDto>("create_group", {
        command: { persona_id: activeFilter.activePersona, name, description: null },
      });
      newGroupName = "";
      groupCreateError = "";
      await groupCatalog.loadCounts(activeFilter.activePersona);
    } catch (error) {
      groupCreateError = String(
        (error as { message?: string })?.message ?? error,
      );
    }
  }

  /// Moves the group to the trash. Reversible: the membership and its
  /// drag order stay on disk, and the member assets are untouched — a
  /// Group is a filing, not a container. Restoring is a backend verb
  /// (`restore_group`) that has no sidebar affordance yet.
  async function trashGroup(id: string) {
    try {
      await mutate("trash_group", { command: { group_id: id } }, "trash this group");
      // Drop the filter if the trashed group was active.
      activeFilter.removeGroup(id);
      await groupCatalog.loadCounts(activeFilter.activePersona);
      await groupCatalog.loadLinks(activeFilter.activePersona);
    } catch (error) {
      console.warn("trash_group failed", error);
    }
  }

  async function createDir() {
    const name = newDirName.trim();
    if (name.length === 0 || activeFilter.activePersona === null) {
      dirError = activeFilter.activePersona === null
        ? "pick a persona first"
        : "name required";
      return;
    }
    try {
      await invoke<DirDto>("create_dir", {
        command: { persona_id: activeFilter.activePersona, parent_id: null, name },
      });
      newDirName = "";
      dirError = "";
      await groupCatalog.loadDirs(activeFilter.activePersona);
    } catch (error) {
      dirError = String((error as { message?: string })?.message ?? error);
    }
  }

  async function deleteDir(id: string) {
    try {
      await mutate("delete_dir", { command: { dir_id: id } }, "delete this folder");
      expandedDirs.delete(id);
      dirError = "";
      await groupCatalog.loadDirs(activeFilter.activePersona);
    } catch {
      // Deliberately silent here: `mutate` has already put the refusal
      // on screen. Typically "dir is not empty — move or delete its
      // contents first", which is a refused *operation*, not a field
      // the user can correct in place. `dirError` keeps the other
      // half — "name required" belongs beside the input it is about,
      // and the form's own failures stay there.
    }
  }

  // --- Inline rename (shared by dirs and groups; at most one row is
  // in edit mode at a time).
  function startRenameDir(dir: DirDto) {
    renamingGroupId = null;
    renamingDirId = dir.id;
    renameDraft = dir.name;
  }

  function startRenameGroup(group: GroupDto) {
    renamingDirId = null;
    renamingGroupId = group.id;
    renameDraft = group.name;
  }

  function cancelRename() {
    renamingDirId = null;
    renamingGroupId = null;
    renameDraft = "";
  }

  async function commitRename() {
    const name = renameDraft.trim();
    if (name.length === 0) {
      cancelRename();
      return;
    }
    try {
      if (renamingDirId !== null) {
        await invoke("rename_dir", {
          command: { dir_id: renamingDirId, name },
        });
        await groupCatalog.loadDirs(activeFilter.activePersona);
      } else if (renamingGroupId !== null) {
        const id = renamingGroupId;
        await invoke("rename_group", { command: { group_id: id, name } });
        if (activeFilter.activeGroupNames.has(id)) activeFilter.activeGroupNames.set(id, name);
        await groupCatalog.loadCounts(activeFilter.activePersona);
      }
      dirError = "";
    } catch (error) {
      dirError = String((error as { message?: string })?.message ?? error);
    } finally {
      cancelRename();
    }
  }

  function onRenameKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      commitRename();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancelRename();
    }
  }

  // --- Disclosure toggles
  function toggleDirExpand(id: string) {
    if (expandedDirs.has(id)) expandedDirs.delete(id);
    else expandedDirs.add(id);
  }

  function toggleGroupExpand(id: string) {
    if (expandedGroups.has(id)) expandedGroups.delete(id);
    else expandedGroups.add(id);
  }

  // --- Sidebar drops. A dir row (or the ROOT heading) accepts a group
  // (file it under the dir) or another dir (re-parent). Both arrive
  // through the pointer drag helper, so nothing here registers a DOM
  // handler — the rows carry `data-drop-kind="dir"` and this decides
  // what a drop means.
  async function dropOntoDir(source: DragSource, dirId: string) {
    try {
      if (source.kind === "group") {
        await invoke("move_group_to_dir", {
          command: {
            group_id: source.id,
            dir_id: dirId === ROOT ? null : dirId,
          },
        });
        await groupCatalog.loadCounts(activeFilter.activePersona);
      } else if (source.kind === "dir") {
        await invoke("move_dir", {
          command: {
            dir_id: source.id,
            new_parent_id: dirId === ROOT ? null : dirId,
          },
        });
        await groupCatalog.loadDirs(activeFilter.activePersona);
      }
      if (dirId !== ROOT) expandedDirs.add(dirId);
      dirError = "";
    } catch (error) {
      // Typically a cycle rejection or a sibling-name collision.
      dirError = String((error as { message?: string })?.message ?? error);
    }
  }

  // --- Nesting: connect the dragged group into the drop-target
  // group (Are.na channel-in-channel), backed by `link_group`.
  async function linkGroups(parentId: string, childId: string) {
    try {
      await invoke("link_group", {
        command: { parent_group_id: parentId, child_group_id: childId },
      });
      await groupCatalog.loadLinks(activeFilter.activePersona);
      dirError = "";
    } catch (error) {
      // Cycle / cross-persona rejections surface inline.
      dirError = String((error as { message?: string })?.message ?? error);
    }
  }

  // --- Nesting, undone: disconnect the child from the parent it is
  // rendered under. The drag gesture that creates a link had no
  // inverse, so a mis-drop was permanent from the sidebar's side even
  // though the command layer has always supported the removal — the
  // nested row now carries its own way out.
  async function unlinkGroup(parentId: string, childId: string) {
    try {
      await mutate(
        "unlink_group",
        { command: { parent_group_id: parentId, child_group_id: childId } },
        "unlink these groups",
      );
      await groupCatalog.loadLinks(activeFilter.activePersona);
      dirError = "";
    } catch {
      // Same reasoning as `deleteDir`: `mutate` has the refusal on
      // screen, and this is an operation rather than a field. It is
      // also the same verb `App.svelte` calls, which now says the same
      // thing on both paths.
    }
  }

  // Sidebar Group entry as a drop target. Two payloads land here:
  // a grid card (the Are.na "drop into a channel" gesture → add the
  // asset) or another sidebar group row (→ connect it as a nested
  // collection). HTML5 requires preventDefault() on BOTH dragenter
  // and dragover for the element to be a valid drop target; skipping
  // dragenter is why the browser rejects the drop (and never fires
  // drop) even though dragover looks correct.
  // A group row accepts a drop only when it makes sense on that
  // group's kind:
  //   - Query groups: never — their membership is the materialised
  //     result of the stored rule, and the command layer rejects
  //     add_asset_to_group / link_group with kind='query' as parent.
  //   - Manual groups: accept a card (Are.na "drop into a channel")
  //     or another sidebar group (nest as a child collection). The
  //     child slot on a nesting link stays kind-agnostic so a manual
  //     group can pull a query group's materialised members through
  //     its bucket_link closure.
  function groupAccepts(groupId: string): boolean {
    if (groupCatalog.isQueryGroup(groupId)) return false;
    if (dragGroupId !== null && dragGroupId === groupId) return false;
    return true;
  }

  // A group row is both a drag source (drop it on another group to nest
  // it) and a drop target (`data-drop-kind="group"`). Card drops land
  // in App's router; group-onto-group nesting is handled here since the
  // link is this section's own concern.
  function onSidebarDropTarget(target: DropTarget, source: DragSource) {
    if (target.kind === "dir") {
      void dropOntoDir(source, target.id);
      return;
    }
    // Group into group = nest it (Are.na channel-in-channel).
    if (target.kind === "group" && source.kind === "group") {
      void linkGroups(target.id, source.id);
    }
  }
</script>

<!-- `parentId` is the Group this row is rendered *under* (null at the
     top level). It carries two things a boolean `nested` could not: the
     `⊂` mark, and the id `unlinkGroup` needs to undo the connection. -->
{#snippet groupRow(gc: GroupSummaryDto, depth: number, parentId: string | null)}
  {@const childGroups = groupCatalog.childGroupsByParent.get(gc.group.id) ?? []}
  {@const isQuery = gc.group.kind === "query"}
  <li
    class="group-row"
    class:group-row-query={isQuery}
    style="--depth: {depth}"
    class:drop-target-group={cardDrag.isOver("group", gc.group.id)}
    data-drop-kind={groupAccepts(gc.group.id) ? "group" : undefined}
    data-drop-id={groupAccepts(gc.group.id) ? gc.group.id : undefined}
  >
    {#if childGroups.length > 0}
      <button
        class="dir-toggle"
        onclick={() => toggleGroupExpand(gc.group.id)}
        title="{childGroups.length} nested group(s)"
      >
        {expandedGroups.has(gc.group.id) ? "▾" : "▸"}
      </button>
    {/if}
    {#if renamingGroupId === gc.group.id}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="rename-input"
        autofocus
        bind:value={renameDraft}
        onkeydown={onRenameKeydown}
        onblur={commitRename}
      />
    {:else}
      <button
        class="group-main-btn"
        class:active={activeFilter.activeGroupIds.has(gc.group.id)}
        onpointerdown={(e) =>
          beginDrag(e, { kind: "group", id: gc.group.id }, onSidebarDropTarget)}
        onclick={() => activeFilter.toggleGroup(gc.group)}
        oncontextmenu={(e) => openQueryMenu(e, gc.group)}
        title={gc.group.description ?? (isQuery ? "rule-defined query group — right-click to expand or update the rule" : "user-curated bucket")}
      >
        <span class="tag-name">
          {#if parentId !== null}<span class="nest-mark">⊂</span>{/if}
          {activeFilter.activeGroupIds.has(gc.group.id) ? "☑" : "☐"}
          <!-- kind icon differentiates hand-curated buckets from
               rule-defined query groups. ▤ = manual,
               ⌘ = query (a stored rule that the eval Job materialises). -->
          <span class="group-kind" class:group-kind-query={isQuery}
            title={isQuery ? "Query Group — membership defined by a stored rule" : "Manual Group — hand-curated bucket"}
          >{isQuery ? "⌘" : "▤"}</span>
          {#if isQuery && gc.group.last_refresh_status === "failed"}
            <!-- W4-b failure signal: the last evaluate failed, so
                 the materialised membership is stale. The tooltip
                 carries the stamped error body. -->
            <span class="group-stale"
              title={`Last refresh failed — membership may be stale${gc.group.last_refresh_error ? `: ${gc.group.last_refresh_error}` : ""}`}
            >⚠</span>
          {/if}
          {gc.group.name}
          {#if childGroups.length > 0}
            <span class="nest-badge">⊃{childGroups.length}</span>
          {/if}
        </span>
        <span class="tag-count">{gc.asset_count}</span>
      </button>
      {#if parentId !== null}
        <button
          class="group-unlink"
          onclick={() => unlinkGroup(parentId, gc.group.id)}
          title="Move this group out of its parent (the group and its members stay)"
        >
          ⊄
        </button>
      {/if}
      <button
        class="group-edit"
        onclick={() => startRenameGroup(gc.group)}
        title="Rename this group"
      >
        ✎
      </button>
      <button
        class="group-delete"
        onclick={() => trashGroup(gc.group.id)}
        title="Move this group to the trash (members are kept)"
      >
        ✕
      </button>
    {/if}
  </li>
  {#if childGroups.length > 0 && expandedGroups.has(gc.group.id)}
    {#each childGroups as cgc (cgc.group.id)}
      {@render groupRow(cgc, depth + 1, gc.group.id)}
    {/each}
  {/if}
{/snippet}

{#snippet dirNode(dir: DirDto, depth: number)}
  <!-- `data-dir-id` is on the row as well as on the `.dir-name` button
       below (where the drag router reads it) so the row itself is
       addressable: `e2e/refusal.spec.ts` has to read what a dir
       discloses and then click that same row's ✕, and without an id on
       the `li` both of those go through `:has()`. -->
  <li
    class="group-row dir-row"
    data-dir-id={dir.id}
    style="--depth: {depth}"
    class:drop-target-group={cardDrag.isOver("dir", dir.id)}
  >
    <button class="dir-toggle" onclick={() => toggleDirExpand(dir.id)}>
      {expandedDirs.has(dir.id) ? "▾" : "▸"}
    </button>
    {#if renamingDirId === dir.id}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="rename-input"
        autofocus
        bind:value={renameDraft}
        onkeydown={onRenameKeydown}
        onblur={commitRename}
      />
    {:else}
      <button
        class="dir-name"
        data-drop-kind="dir"
        data-drop-id={dir.id}
        onpointerdown={(e) =>
          beginDrag(e, { kind: "dir", id: dir.id }, onSidebarDropTarget)}
        onclick={() => onToggleDirFocus(dir.id)}
        title="Click: toggle every group inside as the filter. Drop a group here to file it."
      >
        <span class="tag-name">▣ {dir.name}</span>
      </button>
      <button
        class="group-edit"
        onclick={() => startRenameDir(dir)}
        title="Rename this dir"
      >
        ✎
      </button>
      <button
        class="group-delete"
        onclick={() => deleteDir(dir.id)}
        title="Delete this dir (must be empty)"
      >
        ✕
      </button>
    {/if}
  </li>
  {#if expandedDirs.has(dir.id)}
    {#each groupCatalog.dirChildren.get(dir.id) ?? [] as child (child.id)}
      {@render dirNode(child, depth + 1)}
    {/each}
    {#each groupCatalog.groupsByDir.get(dir.id) ?? [] as gc (gc.group.id)}
      <!-- Filed in a dir is not nested in a *group*: no `⊂`, and the way
           out is the `● all` drop target, not `⊄`. -->
      {@render groupRow(gc, depth + 1, null)}
    {/each}
    {#if (groupCatalog.dirChildren.get(dir.id) ?? []).length === 0 && (groupCatalog.groupsByDir.get(dir.id) ?? []).length === 0}
      <li class="tags-empty dir-empty" style="--depth: {depth + 1}">empty</li>
    {/if}
  {/if}
{/snippet}

<h2>Groups {#if activeFilter.activeGroupIds.size > 0}<span class="tags-active-count">
  · {activeFilter.activeGroupIds.size} OR</span>{/if}</h2>
<ul class="tags-list">
  <!-- Rejections from every structural write (link / unlink / file into
       a dir / move / rename / delete) land here, so this sits at the top
       of the list and outside the persona gate below. It used to be the
       last row of the create-form block, which meant a cycle rejection
       was invisible unless a persona happened to be selected *and* the
       sidebar happened to be scrolled to the bottom — the drop looked
       like it silently did nothing. -->
  {#if dirError}
    <li class="group-error">
      {dirError}
      <button class="group-error-dismiss" onclick={() => (dirError = "")} title="Dismiss">✕</button>
    </li>
  {/if}
  <li>
    <button
      class:active={activeFilter.activeGroupIds.size === 0}
      class:drop-target-root={cardDrag.isOver("dir", ROOT)}
      data-drop-kind="dir"
      data-drop-id={ROOT}
      onclick={() => activeFilter.clearGroups()}
      title="Click: clear the group filter. Drop a group here to move it back to the root."
    >
      ● all
    </button>
  </li>
  {#if groupCatalog.counts.data.length === 0 && groupCatalog.dirs.data.length === 0}
    <li class="tags-empty">no groups yet</li>
  {:else}
    {#each groupCatalog.dirChildren.get(ROOT) ?? [] as dir (dir.id)}
      {@render dirNode(dir, 0)}
    {/each}
    {#each groupCatalog.groupsByDir.get(ROOT) ?? [] as gc (gc.group.id)}
      {@render groupRow(gc, 0, null)}
    {/each}
  {/if}
  {#if activeFilter.activePersona !== null}
    <li class="group-create">
      <input
        type="text"
        placeholder="＋ new group"
        bind:value={newGroupName}
        onkeydown={(e) => e.key === "Enter" && createGroup()}
      />
      {#if newGroupName.trim().length > 0}
        <button onclick={createGroup}>add</button>
      {/if}
    </li>
    <li class="group-create">
      <input
        type="text"
        placeholder="＋ new dir"
        bind:value={newDirName}
        onkeydown={(e) => e.key === "Enter" && createDir()}
      />
      {#if newDirName.trim().length > 0}
        <button onclick={createDir}>add</button>
      {/if}
    </li>
    {#if groupCreateError}
      <li class="group-error">{groupCreateError}</li>
    {/if}
  {:else}
    <li class="tags-empty">pick a persona to create groups</li>
  {/if}
</ul>

<!-- Query-group context menu overlay. Fixed position so it
     escapes the sidebar's own overflow clipping; stopPropagation on
     click so the window-level onclick close does not fire on menu
     item picks. Escape / outside click / action pick all close. -->
{#if queryMenu}
  <div
    class="query-menu"
    style={`left: ${queryMenu.x}px; top: ${queryMenu.y}px;`}
    onclick={(e) => e.stopPropagation()}
    role="menu"
  >
    <div class="query-menu-head">{queryMenu.group.name}</div>
    {#if queryMenu.group.kind === "query"}
      <button
        type="button"
        class="query-menu-item"
        onclick={() => expandQueryIntoFilter(queryMenu!.group)}
      >
        ▸ Expand query into filter
      </button>
      <button
        type="button"
        class="query-menu-item"
        onclick={() => updateQueryFromFilter(queryMenu!.group)}
        title="Rewrite this group's rule from the current filter chips + Sorter state"
      >
        ↻ Update query from current filter
      </button>
    {/if}
    {#if queryMenu.group.origin_snapshot_id !== null}
      <button
        type="button"
        class="query-menu-item"
        onclick={() => openProvenanceSnapshot(queryMenu!.group)}
        title="Open the frozen Snapshot this group was promoted from"
      >
        ◇ Promoted from · {queryMenu.group.origin_snapshot_id.slice(0, 8)}
      </button>
    {/if}
    {#if queryMenuError}
      <div class="query-menu-error">{queryMenuError}</div>
    {/if}
  </div>
{/if}

<svelte:window
  onkeydown={(e) => { if (e.key === "Escape" && queryMenu) closeQueryMenu(); }}
  onclick={() => queryMenu && closeQueryMenu()}
/>

<style>
  /* Sidebar heading (mirrors `.sidebar h2` in App.svelte). Same
     duplication pattern as ModalityList (wave 4) and TagList
     (wave 5a). */
  h2 {
    font-size: 0.75rem;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 1rem 0 0.25rem;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  button {
    background: none;
    border: none;
    padding: 0.2rem 0.3rem;
    font-size: 0.85rem;
    color: #555;
    cursor: pointer;
    width: 100%;
    text-align: left;
    border-radius: 4px;
    font-family: inherit;
  }
  button:hover {
    background: #efefe9;
  }
  button.active {
    color: #111;
    font-weight: 600;
    background: #eceae2;
  }

  /* Shared row cascade (name + count, empty state, active-count
     badge). App keeps a copy for its remaining sections that still
     use these classes. */
  .tags-list button {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.4rem;
  }
  .tag-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag-count {
    font-size: 0.6rem;
    color: #b5b1e2;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .tags-empty {
    font-size: 0.65rem;
    color: #999;
    padding: 0.2rem 0.4rem;
    list-style: none;
  }
  .tags-active-count {
    font-size: 0.6rem;
    color: #9a96d9;
    font-weight: normal;
  }

  /* Sidebar group row: button + trailing edit / delete affordance
     sit side by side so a group can be renamed / removed without a
     right-click. */
  .group-row {
    display: flex;
    gap: 0.2rem;
    align-items: stretch;
    padding-left: calc(var(--depth, 0) * 0.75rem);
  }
  .group-row > button:first-child {
    flex: 1;
  }
  /* Highlight a Group entry while a card is being dragged over it —
     signals "drop here to add to this group". Distinct hue from the
     grid drop-target so users can tell which target axis is armed.
     `!important` on the button background so the inner button's
     hover / active style doesn't visually cancel the row-level
     highlight. */
  .group-row.drop-target-group {
    background: #eef7f4 !important;
    outline: 2px solid #7ab89a;
    outline-offset: -2px;
    border-radius: 4px;
  }
  .group-row.drop-target-group > button {
    background: transparent !important;
  }

  /* Kind indicator — the sidebar reads `group.kind` and stamps the
     row. Manual groups get the neutral
     bucket glyph; query groups a distinct token + tinted color so they
     read apart at a glance. `.group-row-query` also relaxes hover
     affordances that suggest hand editing (drop-target chrome is
     already blocked at the JS layer via `groupCatalog.isQueryGroup`). */
  .group-kind {
    display: inline-block;
    margin-right: 0.25em;
    color: #8f89b6;
    font-size: 0.85em;
  }
  .group-kind.group-kind-query {
    color: #b57a55;
  }
  /* W4-b staleness chip — the last refresh of this query group
     failed; the tooltip carries the stamped error text. */
  .group-stale {
    display: inline-block;
    margin-right: 0.25em;
    color: #c0392b;
    font-size: 0.85em;
  }
  /* Target the name button explicitly — a nested query group renders
     the disclosure chevron (`.dir-toggle`) as the row's first child,
     so `> button:first-child` would tint the chevron instead of the
     label. `.group-main-btn` is present on every group name button
     (with or without nested children). */
  .group-row-query > button.group-main-btn {
    color: #5a4a3a;
  }
  .group-row-query > button.group-main-btn .tag-name {
    font-style: italic;
  }

  /* Query-group context menu (right-click on a kind='query' row).
     Fixed-positioned so it escapes the sidebar's clipping; the sizing
     tracks the two-action + optional error footprint (~180px tall). */
  .query-menu {
    position: fixed;
    z-index: 1000;
    min-width: 240px;
    background: #fff;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    box-shadow: 0 6px 18px rgba(58, 50, 130, 0.14);
    padding: 0.35rem 0;
    font-family: inherit;
  }
  .query-menu-head {
    padding: 0.2rem 0.7rem 0.35rem;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #9a97b0;
    border-bottom: 1px solid #efedfa;
    margin-bottom: 0.3rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .query-menu-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.35rem 0.7rem;
    font-size: 0.78rem;
    font-family: inherit;
    color: #4a4770;
    background: transparent;
    border: none;
    cursor: pointer;
  }
  .query-menu-item:hover {
    background: #f0effc;
    color: #2f2b5a;
  }
  .query-menu-error {
    padding: 0.35rem 0.7rem;
    margin-top: 0.25rem;
    border-top: 1px solid #f0d5d5;
    color: #b05656;
    font-size: 0.72rem;
    white-space: normal;
  }

  .group-delete {
    padding: 0 0.35rem;
    font-size: 0.55rem;
    color: #b0a5cf;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .group-delete:hover {
    color: #d47272;
  }

  /* Rename affordance (dirs + groups) — hidden-ish until hover so
     the row stays quiet. */
  .group-edit {
    padding: 0 0.2rem;
    font-size: 0.55rem;
    color: #c9c4e4;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .group-edit:hover {
    color: #7a76c9;
  }

  /* WebKit (Tauri's WKWebView) does not initiate an HTML5 drag from
     form controls even with draggable="true" — grid cards work
     because they are <div>s. The -webkit-user-drag override opts the
     sidebar row buttons back into dragging. */
  .tags-list button[draggable="true"] {
    -webkit-user-drag: element;
  }
  .dir-empty {
    padding-left: calc(var(--depth, 0) * 0.75rem + 1.3rem);
  }

  /* Nesting markers on group rows: ⊃N = contains N nested groups
     (visible even when collapsed), ⊂ = this row is rendered as a
     nested child under its parent. */
  /* Un-nest affordance — only rendered on a row that sits under a
     parent group. Same weight as ✎ / ✕, tinted with the nesting green
     so it reads as the inverse of the `⊂` mark next to the name rather
     than as another delete. */
  .group-unlink {
    padding: 0 0.2rem;
    font-size: 0.6rem;
    color: #7ab89a;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .group-unlink:hover {
    color: #4f9b78;
  }

  .nest-badge {
    margin-left: 0.25rem;
    font-size: 0.6rem;
    color: #7ab89a;
    font-variant-numeric: tabular-nums;
  }
  .nest-mark {
    color: #7ab89a;
    font-size: 0.7rem;
    margin-right: 0.1rem;
  }

  /* Dir rows: disclosure triangle + folder-ish name. */
  .dir-toggle {
    flex: 0 0 auto;
    width: 1.1rem;
    padding: 0.2rem 0;
    text-align: center;
    font-size: 0.6rem;
    color: #9a96d9;
  }
  .dir-row > .dir-name {
    flex: 1;
  }
  .dir-name .tag-name {
    color: #6f6c9c;
    font-weight: 500;
  }

  /* Inline rename input replaces the row label. */
  .rename-input {
    flex: 1;
    min-width: 0;
    padding: 0.15rem 0.35rem;
    font-size: 0.75rem;
    border: 1px solid #8a86ff;
    border-radius: 4px;
    background: #fff;
    font-family: inherit;
  }
  .rename-input:focus {
    outline: none;
  }

  /* Root drop target (the "● all" button) while a sidebar row is
     being dragged. */
  .drop-target-root {
    background: #eef7f4 !important;
    outline: 2px solid #7ab89a;
    outline-offset: -2px;
  }

  /* Sidebar "＋ new group / dir" inline creators. */
  .group-create {
    display: flex;
    gap: 0.3rem;
    padding: 0.3rem 0.4rem 0.15rem;
    list-style: none;
  }
  .group-create input {
    flex: 1;
    padding: 0.15rem 0.35rem;
    font-size: 0.7rem;
    border: 1px solid #d9d5f2;
    border-radius: 4px;
    background: #fff;
    font-family: inherit;
  }
  .group-create button {
    font-size: 0.65rem;
    padding: 0.1rem 0.5rem;
    background: #7a76c9;
    color: #fff;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
  }
  .group-error {
    padding: 0.15rem 0.5rem;
    font-size: 0.6rem;
    color: #d47272;
    list-style: none;
  }
  /* A rejection stays until it is read: the next successful write clears
     it, and this gives the reader a way out in the meantime. */
  .group-error-dismiss {
    padding: 0 0.25rem;
    font-size: 0.55rem;
    color: #d47272;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .group-error-dismiss:hover {
    color: #b0a5cf;
  }
</style>
