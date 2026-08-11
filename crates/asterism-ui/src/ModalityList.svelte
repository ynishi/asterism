<script lang="ts">
  // ModalityList — extracted from App.svelte (2026-07-20 Phase C
  // wave 4). Owns the "Modality" sidebar section: an "● all" row +
  // one row per modality with the persona-scoped asset count on the
  // right.
  //
  // State (0-prop, catalog-driven — the Modality
  // axis is now backend-authoritative):
  //   - modalityCatalog.visible      (ordered master rows + any
  //     unregistered slugs at the tail)
  //   - modalityCatalog.countBySlug
  //   - personaCatalog.scopedTotal   ("all" = the grid population, not
  //     the sum of this section's own buckets — unclassified rows have
  //     no modality bucket, so summing them read 237 against a grid of
  //     264)
  //   - activeFilter.activeModality
  //
  // Modality selection mutates activeFilter.activeModality directly;
  // the App-side reload `$effect` picks up the change transparently.
  //
  // Drop target: a row carries `data-drop-kind="modality"` and its slug,
  // and that is the whole registration. The drag helper resolves what
  // is under the pointer and App routes the drop — no handlers here,
  // because pointer capture means this element never sees the pointer
  // anyway (lib/interaction/drag.svelte.ts).
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { cardDrag } from "./lib/interaction/drag.svelte";
  import { modalityCatalog, UNCLASSIFIED_MODALITY } from "./lib/stores/modality.svelte";
  import { personaCatalog } from "./lib/stores/personas.svelte";

  // Registered rows only. The Unclassified bucket is browsable but not
  // a destination — its key is a sentinel the backend rejects — and an
  // unregistered slug is not somewhere to file things either.
  function accepts(slug: string): boolean {
    return (
      slug !== UNCLASSIFIED_MODALITY &&
      modalityCatalog.all.some((m) => m.slug === slug)
    );
  }
</script>

<h2>Modality</h2>
<ul>
  <li>
    <button
      class:active={activeFilter.activeModality === null}
      onclick={() => (activeFilter.activeModality = null)}
    >
      ● all
      <span class="sidebar-count">{personaCatalog.scopedTotal}</span>
    </button>
  </li>
  {#each modalityCatalog.visible as row (row.slug)}
    <li
      class:drop-target={cardDrag.isOver("modality", row.slug)}
      data-drop-kind={accepts(row.slug) ? "modality" : undefined}
      data-drop-id={accepts(row.slug) ? row.slug : undefined}
    >
      <button
        class:active={activeFilter.activeModality === row.slug}
        onclick={() => (activeFilter.activeModality = row.slug)}
      >
        ○ {row.label}
        <span class="sidebar-count">{modalityCatalog.countBySlug.get(row.slug) ?? 0}</span>
      </button>
    </li>
  {/each}
</ul>

<style>
  /* Mirror `.sidebar h2` / `ul` / `button` / `.sidebar-count`
     cascade — same duplication pattern as SidebarSearch (wave 1) and
     PersonaStrip (wave 3). Kept in sync until the whole sidebar
     graduates out of App (wave 9). */
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

  /* Drop target while a card is over the row — the same affordance the
     Groups section uses, so "drag a card onto a sidebar row to file
     it" reads the same way on both axes. */
  li.drop-target {
    outline: 2px dashed #b5b1e2;
    outline-offset: -2px;
    border-radius: 4px;
    background: #f2f1fb;
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

  .sidebar-count {
    float: right;
    font-size: 0.7rem;
    color: #b5b1e2;
    font-variant-numeric: tabular-nums;
    padding-left: 0.4rem;
  }
  button.active .sidebar-count {
    color: #7a7594;
  }
</style>
