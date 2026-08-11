<script lang="ts">
  // DiscoverSection — the sidebar block directly under the search box:
  // the ways into the library that are *not* "I know what I am looking
  // for".
  //
  // Both entries put their result in the ordinary grid rather than a
  // modal, so thumbnails, Quick Look, rating and drag keep working on
  // whatever the exploration turned up — a modal would have had to
  // re-earn every one of those.
  //
  //   - 🎲 Random — a toggle. While lit the grid shows a random handful
  //     out of the current filter instead of its listing; the chips
  //     still narrow, and the count line says how large the pool was.
  //     Holds state (the chip band shows it, `reset()` clears it).
  //   - ✦ Search — not a state at all: it flips the box to ✦ fuzzy and
  //     puts the cursor in it. It exists because the Retrieval side had
  //     no entry that names itself; the toggle inside the search box is
  //     only findable once you already know the box has modes.
  //
  // 0-prop for data: the store is read directly. The
  // one prop is an App-owned grid side effect — the allowed category —
  // because flipping the draw has to flush App's pending search
  // debounce, which lives there with the timer.
  import { activeFilter } from "./lib/stores/filter.svelte";

  interface Props {
    /// Reload the grid now, cancelling any queued search reload. Called
    /// after the random toggle flips, since that both changes the branch
    /// the grid fetches and may have cleared a fuzzy query.
    onReloadNow: () => void;
  }

  let { onReloadNow }: Props = $props();

  function toggleRandom() {
    activeFilter.toggleDiscoverRandom();
    onReloadNow();
  }

  function focusSearch() {
    activeFilter.searchFuzzy = true;
    const box = document.getElementById(
      "sidebar-search-input",
    ) as HTMLInputElement | null;
    box?.focus();
    box?.select();
  }
</script>

<h2>Discover</h2>
<ul>
  <li>
    <button
      class:active={activeFilter.discoverRandom}
      onclick={toggleRandom}
      aria-pressed={activeFilter.discoverRandom}
      title="Show a random handful out of the current filter. Draw again from the count line; the picks are never saved."
    >
      🎲 Random
    </button>
  </li>
  <li>
    <button
      onclick={focusSearch}
      title="Search by nearness — ranked candidates, not an exhaustive set"
    >
      ✦ Search
    </button>
  </li>
</ul>

<style>
  /* Sidebar cascade, duplicated from `.sidebar` in App.svelte — the
     same pattern every extracted section follows (ModalityList wave 4,
     TagList wave 5a, GroupsSection wave 5b), because Svelte scopes the
     App copy to App's own markup. */
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
</style>
