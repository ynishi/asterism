// Modality catalog — sidebar-facing state for the Modality master.
// Modality is now a
// backend-authoritative row set (the `modality` master table +
// `ModalityService` CRUD), not a frontend-declared enum: the (slug,
// label, kind, sort_order, hidden) tuples are fetched via
// `list_modalities` and every consumer resolves label / kind / rank
// off the fetched rows. Behaviour never branches on the slug directly
// — the `kind` reference into the closed `ContentKind` set is the only
// thing jobs / UI / import read.
//
// Scope:
//   - `list`: Resource over `list_modalities` — the ordered master
//     rows (hidden included), each carrying its live `asset_count`.
//     Ordering + identity are backend-authoritative; this store never
//     mutates row contents except through the optimistic `reorder`
//     path (which re-reads on failure).
//   - `counts`: Resource over `list_modality_asset_counts` — the
//     persona-scoped per-slug tally that drives the sidebar badge.
//     Kept alongside `list` (whose `asset_count` is a cross-persona
//     total) so the sidebar count still narrows with the active
//     persona, and so a slug carried by assets but absent from the
//     master (importer escape hatch) can be surfaced at the tail.
//   - `visible` / `all` / `kindOf` / `labelOf` / `rank` /
//     `slugForKind` / `countBySlug`: cheap derivations
//     rebuilt on each fetch.
//
// Reload wiring: App-side `$effect` owns the reload chain — persona
// change → `loadCounts`, and `load` is called once on mount. Same
// reaction-ownership rule as personaCatalog + activeFilter.
// The catalog never decides when to reload itself.

import type {
  AssetCountEntryDto,
  CreateModalityCommand,
  ModalityDefDto,
  UpdateModalityCommand,
} from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

// Reserved key the backend reports rows carrying no modality under,
// and the sentinel a client sends back to filter for exactly those.
// Mirrors `asterism_core::domain::asset::UNCLASSIFIED_MODALITY`; the
// `!` prefix is a shape the `Modality` newtype rejects, so it can
// never collide with a real master slug.
export const UNCLASSIFIED_MODALITY = "!unclassified";

// The `ContentKind` set lived here as a UI mirror. It is gone: the
// format kinds moved to the material's mime (`render_policy`) and what
// was left — "does this read as a terminal transcript?" — is a bool on
// the master row, edited as a checkbox rather than picked from a list.

class ModalityCatalog {
  list = new Resource(
    () => api<ModalityDefDto[]>("list_modalities"),
    [] as ModalityDefDto[],
    "modalityCatalog.list",
  );

  // `trash` follows the grid — see `personaCatalog.counts`. Args are a
  // tuple because `Resource` passes exactly one value through.
  counts = new Resource(
    ([personaId, trash]: [string | null, string]) =>
      api<AssetCountEntryDto[]>("list_modality_asset_counts", { personaId, trash }),
    [] as AssetCountEntryDto[],
    "modalityCatalog.counts",
  );

  // slug → master row. Rebuilt whenever `list.data` reassigns so
  // `kindOf` / `labelOf` resolve in O(1) from templates and effects.
  bySlug = $derived.by(() => {
    const m = new SvelteMap<string, ModalityDefDto>();
    for (const row of this.list.data) m.set(row.slug, row);
    return m;
  });

  // Persona-scoped per-slug count map for the sidebar badge.
  countBySlug = $derived.by(() => {
    const m = new SvelteMap<string, number>();
    for (const e of this.counts.data) m.set(e.key, e.count);
    return m;
  });

  // Deliberately no `totalCount`: summing this section's buckets is
  // not the size of the grid, because an unclassified row carries no
  // modality and therefore no bucket. The "● all" row reads
  // `personaCatalog.scopedTotal` instead — see ModalityList.

  // Sidebar / filter / move-menu list: the hidden-excluded master rows
  // (already ordered by `sort_order` then `slug` on the backend) with
  // any unregistered slugs — present in the persona count set but not
  // the master (importer escape hatch) — appended at the tail so they
  // stay reachable rather than vanishing (design P2: surfacing
  // unregistered slugs).
  visible = $derived.by(() => {
    const rows = this.list.data.filter((m) => !m.hidden);
    const known = new Set(this.list.data.map((m) => m.slug));
    const extra: ModalityDefDto[] = [];
    for (const e of this.counts.data) {
      if (e.count > 0 && !known.has(e.key)) {
        extra.push({
          slug: e.key,
          // The unclassified bucket is a real row the user clicks, not
          // an unregistered slug that leaked out of an importer, so it
          // gets a name instead of its raw sentinel.
          label: e.key === UNCLASSIFIED_MODALITY ? "Unclassified" : e.key,
          terminal: false,
          sort_order: Number.MAX_SAFE_INTEGER,
          hidden: false,
          cover_template: null,
          asset_count: e.count,
        });
      }
    }
    extra.sort((a, b) => a.slug.localeCompare(b.slug));
    return [...rows, ...extra];
  });

  // Full master row set (hidden included) for the DetailPane modality
  // <select>. Ordered by the backend.
  all = $derived(this.list.data);

  // Slugs carried by assets (persona-scoped counts) but absent from the
  // master — the importer escape hatch surface. Same detection as the
  // `visible` tail, exposed separately so the settings UI can offer a
  // one-click "Register" ({ slug, count } per entry, slug-sorted).
  unregistered = $derived.by(() => {
    const known = new Set(this.list.data.map((m) => m.slug));
    const out: { slug: string; count: number }[] = [];
    for (const e of this.counts.data) {
      if (e.count > 0 && !known.has(e.key)) {
        out.push({ slug: e.key, count: e.count });
      }
    }
    out.sort((a, b) => a.slug.localeCompare(b.slug));
    return out;
  });

  // Rank map for asset sorting (grid / sessions "modality + ordered").
  // Follows the master `sort_order` ordering; unregistered slugs drop
  // to the end (falling back to alphabetical among themselves via the
  // stable sort on the caller side).
  #rankBySlug = $derived.by(() => {
    const m = new SvelteMap<string, number>();
    this.list.data.forEach((row, i) => m.set(row.slug, i));
    return m;
  });

  // Whether assets of this classification read as terminal
  // transcripts — the one display question the master answers.
  // Unregistered / unclassified → `false`.
  isTerminal(slug: string | null | undefined): boolean {
    if (slug == null) return false;
    return this.bySlug.get(slug)?.terminal ?? false;
  }

  // Human label for a modality slug. Unregistered → the slug itself;
  // `null` (unclassified) → "".
  labelOf(slug: string | null | undefined): string {
    if (slug == null) return "";
    return this.bySlug.get(slug)?.label ?? slug;
  }

  // Sort rank for a modality slug. Unregistered / unclassified → end
  // of the list.
  rank(slug: string | null | undefined): number {
    if (slug == null) return this.list.data.length;
    return this.#rankBySlug.get(slug) ?? this.list.data.length;
  }

  async load(): Promise<void> {
    await this.list.load(undefined);
  }

  async loadCounts(
    personaId: string | null,
    trash: "live" | "trashed" = "live",
  ): Promise<void> {
    await this.counts.load([personaId, trash]);
  }

  // Persists a drag-reorder: rewrites `sort_order` to the given slug
  // order via `update_modality`, optimistically reflecting the new
  // order locally first and re-reading the authoritative rows on any
  // failure. Rows absent from `orderedSlugs` (e.g. hidden ones the
  // sidebar never surfaced) keep their relative order after the
  // reordered prefix. localStorage order (`asterism.modality_order.v1`)
  // is retired — the master `sort_order` is the SoT.
  async reorder(orderedSlugs: string[]): Promise<void> {
    const bySlug = new Map(this.list.data.map((m) => [m.slug, m]));
    const seen = new Set(orderedSlugs);
    const ordered: ModalityDefDto[] = [];
    for (const slug of orderedSlugs) {
      const row = bySlug.get(slug);
      if (row) ordered.push(row);
    }
    for (const row of this.list.data) {
      if (!seen.has(row.slug)) ordered.push(row);
    }
    const next = ordered.map((row, i) => ({ ...row, sort_order: i }));
    const changed = next.filter(
      (row) => bySlug.get(row.slug)?.sort_order !== row.sort_order,
    );
    // Optimistic local reflect.
    this.list.data = next;
    try {
      await Promise.all(
        changed.map((row) =>
          api<ModalityDefDto>("update_modality", {
            command: { slug: row.slug, sort_order: row.sort_order },
          }),
        ),
      );
    } catch {
      // A write failed — drop the optimistic order and re-read the
      // authoritative rows.
      await this.load();
    }
  }

  // Settings-UI CRUD. Unlike `reorder` (drag path, optimistic) these
  // stay authoritative-re-read only: the management screen mutates one
  // row at a time and a fresh `list_modalities` after each write is
  // cheap + never diverges. Errors propagate to
  // the caller (component) which owns message display.
  async create(cmd: CreateModalityCommand): Promise<void> {
    await api<ModalityDefDto>("create_modality", { command: cmd });
    await this.load();
  }

  async update(
    slug: string,
    patch: Partial<Omit<UpdateModalityCommand, "slug">>,
  ): Promise<void> {
    await api<ModalityDefDto>("update_modality", {
      command: { slug, ...patch },
    });
    await this.load();
  }

  async remove(slug: string): Promise<void> {
    await api<void>("delete_modality", { command: { slug } });
    await this.load();
  }
}

// Exported as `modalityCatalog` for parallelism with `personaCatalog`.
export const modalityCatalog = new ModalityCatalog();
