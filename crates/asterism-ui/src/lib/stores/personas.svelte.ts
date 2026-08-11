// Personas catalog — sidebar-facing state for the persona list + its
// per-persona asset counts.
//
// Scope of this store:
//   - `list`: Resource over `list_personas` — read rows via
//     `list.data`, failure state via `.error` (the old
//     `{ok, error}` return union is gone; App's status line reads
//     the Resource fields instead). Ordering + identity
//     are backend-authoritative — this store never mutates row
//     contents, only replaces the array on reload.
//   - `counts`: Resource over `list_persona_asset_counts`.
//     Persona-agnostic (never re-scoped by activePersona) — same
//     rule that group / tag "all-assets" counting follows.
//   - `countById` / `totalCount`: derivations rebuilt whenever
//     `counts.data` reassigns. Cheap because both persona and
//     modality buckets stay under ~100 rows.
//
// Deliberately NOT owned here:
//   - `personaTheme` / `personaWallpaperUrl` — `themeCatalog`.
//   - `personaProfiles` — `profileCatalog`.
//
// Reaction ownership: `load()` / `loadCounts()`
// only touch this store's own state + await backend invokes. App-side
// `$effect` blocks that read `.list.data.length` etc. still fire the
// downstream reloads transparently.

import type { AssetCountEntryDto, PersonaDto } from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";
import { activeFilter } from "./filter.svelte";
import { Resource } from "./_resource.svelte";

class PersonaCatalog {
  list = new Resource(
    () => api<PersonaDto[]>("list_personas"),
    [] as PersonaDto[],
    "personaCatalog.list",
  );

  // `trash` follows the grid: a live count beside a trash grid
  // describes the other half of the app, and clicking the chip then
  // filters the trash by a number that was never about it.
  counts = new Resource(
    (trash: string) => api<AssetCountEntryDto[]>("list_persona_asset_counts", { trash }),
    [] as AssetCountEntryDto[],
    "personaCatalog.counts",
  );

  // Reactive lookup maps for the sidebar count spans. Built once per
  // fetch and consumed via `.get()` in templates so a persona list
  // with dozens of rows resolves each count in O(1).
  countById = $derived.by(() => {
    const m = new SvelteMap<string, number>();
    for (const e of this.counts.data) m.set(e.key, e.count);
    return m;
  });

  // Total-assets number rendered on the "● all" row of the Persona
  // section. Sum of every persona bucket — persona-agnostic totals.
  totalCount = $derived(
    this.counts.data.reduce((acc, e) => acc + e.count, 0),
  );

  // Size of the grid population under the current persona scope.
  //
  // Every sidebar section's "● all" row prints this, because "all"
  // means "do not narrow on *this* axis" — so the number has to be
  // the same one the grid lands on once the axis is cleared. Summing
  // a section's own buckets does not answer that: MODALITY has no
  // bucket for unclassified rows, so its own sum read 237 against a
  // grid of 264. The backend already counts one shared population
  // (`GRID_POPULATION`), and the persona counts are its per-bucket
  // partition, which makes them the total for every axis.
  scopedTotal = $derived(
    activeFilter.activePersona === null
      ? this.totalCount
      : (this.countById.get(activeFilter.activePersona) ?? 0),
  );

  // Persona id → name lookup. Rebuilt whenever the list reassigns
  // (a full re-fetch), read from every card label / sort compare
  // in the grid so O(1) lookup matters — `personas.find()` used to
  // pin the main thread for seconds while switching sort under a
  // full 110 k asset dataset.
  nameById = $derived.by(() => {
    const m = new SvelteMap<string, string>();
    for (const p of this.list.data) m.set(p.id, p.name);
    return m;
  });

  async load(): Promise<void> {
    await this.list.load(undefined);
  }

  async loadCounts(trash: "live" | "trashed" = "live"): Promise<void> {
    await this.counts.load(trash);
  }
}

// Exported as `personaCatalog` (not `personas`) so callers can keep
// using `persona` as a loop variable in {#each} blocks without
// shadowing the singleton.
export const personaCatalog = new PersonaCatalog();
