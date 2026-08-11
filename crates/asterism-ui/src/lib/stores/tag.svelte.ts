// Tag catalog — sidebar-facing state for the per-tag asset counts.
// Symmetric with `modalityCatalog` and `personaCatalog`: catalog
// rows + trivial name-lookup derived, nothing about "what the user
// is selecting" (that stays on `activeFilter`, per the module-boundary
// rule).
//
// Scope:
//   - `counts`: Resource over `list_tag_counts` — read rows via
//     `counts.data`, in-flight / failure state via `.loading` /
//     `.error` (error policy + stale-response guard live on the
//     Resource primitive). Persona-scoped — the invoke
//     accepts a `personaId` argument, so switching persona narrows
//     the tally to that slice; the "all" persona returns
//     cross-persona totals.
//   - `nameById`: id → name lookup rebuilt once per `counts.data`
//     reassignment. Used by the sort comparator hot path (10k+
//     tags) and the URL-hydrate name-fill effect in App.svelte.
//
// Deliberately NOT owned here:
//   - Sidebar expand / free-text filter / render cap (`tagsExpanded`
//     / `tagsFilter` / `tagsRenderCap`). These are ephemeral view
//     state whose lifetime is the sidebar section, so they live on
//     `TagList.svelte` alongside the template that reads them.
//
// Reload wiring: App-side `$effect` still owns the persona-change →
// `loadCounts` chain via a thin wrapper. Same reaction-ownership
// rule as `personaCatalog` / `modalityCatalog` / `activeFilter`.

import type { TagCountDto } from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

class TagCatalog {
  counts = new Resource(
    (personaId: string | null) =>
      api<TagCountDto[]>("list_tag_counts", { personaId }),
    [] as TagCountDto[],
    "tagCatalog.counts",
  );

  nameById = $derived.by(() => {
    const m = new SvelteMap<string, string>();
    for (const tc of this.counts.data) m.set(tc.tag.id, tc.tag.name);
    return m;
  });

  async loadCounts(personaId: string | null): Promise<void> {
    await this.counts.load(personaId);
  }
}

// Exported as `tagCatalog` for parallelism with `personaCatalog` /
// `modalityCatalog` (avoids shadowing local `tag` loop variables in
// {#each} blocks).
export const tagCatalog = new TagCatalog();
