<script lang="ts">
  // ColorList — the "Color" sidebar section.
  // An "● all" row + one row per palette swatch present on the current
  // persona's top-level assets, with counts on the right.
  //
  // Colour is a derived fact of the image (the dominant-colour palette
  // quantised into a closed swatch set), orthogonal to both the
  // semantic Modality axis and the FORMAT facet — the three compose as
  // independent filters.
  //
  // The "all" row carries no count on purpose: an asset holds up to
  // five swatches, so a sum would exceed the number of assets. The
  // per-swatch counts are exact (each counts assets, not entries).
  //
  // State (0-prop, catalog-driven):
  //   - colorCatalog.counts / .labelOf / .hexOf
  //   - activeFilter.activeColor
  //
  // Selection mutates activeFilter.activeColor directly; the App-side
  // reload `$effect` picks up the change.
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { colorCatalog } from "./lib/stores/color.svelte";
</script>

<h2>Color</h2>
<ul>
  <li>
    <button
      class:active={activeFilter.activeColor === null}
      onclick={() => (activeFilter.activeColor = null)}
    >
      ● all
    </button>
  </li>
  {#each colorCatalog.counts.data as entry (entry.key)}
    <li>
      <button
        class:active={activeFilter.activeColor === entry.key}
        onclick={() => (activeFilter.activeColor = entry.key)}
      >
        <span class="swatch" style="background: {colorCatalog.hexOf(entry.key)}"
        ></span>
        {colorCatalog.labelOf(entry.key)}
        <span class="sidebar-count">{entry.count}</span>
      </button>
    </li>
  {/each}
</ul>

<style>
  /* Mirror `.sidebar h2` / `ul` / `button` / `.sidebar-count`
     cascade — same duplication pattern as ModalityList / FormatList.
     Kept in sync until the whole sidebar graduates out of App
     (wave 9). */
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

  /* The swatch sits where the ○ bullet sits in the sibling sections,
     so the rows line up. A hairline border keeps White visible against
     the sidebar. */
  .swatch {
    display: inline-block;
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 2px;
    border: 1px solid rgba(0, 0, 0, 0.15);
    vertical-align: -1px;
    margin-right: 0.15rem;
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
