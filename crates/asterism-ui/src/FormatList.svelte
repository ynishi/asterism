<script lang="ts">
  // FormatList — the "Format" sidebar section (asset-model v4 P3).
  // An "● all" row + one row per mime top-level type present on the
  // current persona's top-level assets, with counts on the right.
  //
  // Format is a fact of the material (image / video / audio / text),
  // orthogonal to the semantic Modality axis above it — the two
  // sections compose as independent filters.
  //
  // State (0-prop, catalog-driven):
  //   - formatCatalog.counts / .labelOf
  //   - personaCatalog.scopedTotal   ("all" = the grid population, the
  //     same number MODALITY prints; a row whose material carries no
  //     mime has no format bucket but is still in the grid)
  //   - activeFilter.activeFormat
  //
  // Selection mutates activeFilter.activeFormat directly; the
  // App-side reload `$effect` picks up the change.
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { formatCatalog } from "./lib/stores/format.svelte";
  import { personaCatalog } from "./lib/stores/personas.svelte";
</script>

<h2>Format</h2>
<ul>
  <li>
    <button
      class:active={activeFilter.activeFormat === null}
      onclick={() => (activeFilter.activeFormat = null)}
    >
      ● all
      <span class="sidebar-count">{personaCatalog.scopedTotal}</span>
    </button>
  </li>
  {#each formatCatalog.counts.data as entry (entry.key)}
    <li>
      <button
        class:active={activeFilter.activeFormat === entry.key}
        onclick={() => (activeFilter.activeFormat = entry.key)}
      >
        ○ {formatCatalog.labelOf(entry.key)}
        <span class="sidebar-count">{entry.count}</span>
      </button>
    </li>
  {/each}
</ul>

<style>
  /* Mirror `.sidebar h2` / `ul` / `button` / `.sidebar-count`
     cascade — same duplication pattern as ModalityList. Kept in sync
     until the whole sidebar graduates out of App (wave 9). */
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
