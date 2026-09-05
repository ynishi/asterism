<script lang="ts">
  // SidebarSearch — extracted from App.svelte (2026-07-20 Phase C wave
  // 1). Owns the top-of-sidebar search input. The "Active filters"
  // header + chip band that used to live here moved to the grid-top
  // `ActiveFilters.svelte` so every filter axis
  // (Persona / Modality / label / Tags / Groups / Session / search)
  // shows in one place next to the grid — this component no longer
  // renders chips, only the search box.
  //
  // Everything it reads is pulled straight from the `activeFilter`
  // store; the two callback props route back to App.svelte's reload
  // orchestration — the debounce timer + loadAssets path stay in App
  // (reaction ownership). `onSearchDebounce` fires on
  // every key stroke (App wraps `loadAssets()` in the 250 ms
  // debouncer); `onSearchImmediate` skips the debouncer (clear button
  // + Escape + the mode toggle).
  //
  // The leading button switches which domain the box talks to:
  // ✦ = Retrieval, ranked
  // candidates that make no claim about covering the library; 🔍 =
  // `ListAssetsQuery.text_match`, an exact predicate that composes with
  // the chips and counts / sorts / saves like any other axis. Both write
  // the same `searchText`, so flipping the mode re-asks the question the
  // user already typed — hence the immediate reload rather than the
  // debounced one; there is no keystroke burst to coalesce.
  import { activeFilter } from "./lib/stores/filter.svelte";

  interface Props {
    onSearchDebounce: () => void;
    onSearchImmediate: () => void;
  }

  let { onSearchDebounce, onSearchImmediate }: Props = $props();

  function handleInput(event: Event) {
    activeFilter.searchText = (event.target as HTMLInputElement).value;
    onSearchDebounce();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      activeFilter.searchText = "";
      onSearchImmediate();
    }
  }

  function handleClearButton() {
    activeFilter.searchText = "";
    onSearchImmediate();
  }

  function toggleSearchMode() {
    activeFilter.searchFuzzy = !activeFilter.searchFuzzy;
    onSearchImmediate();
  }

  // Says what the button *is* first, then what clicking does — the glyph
  // alone reads as an action to some users and as a state to others.
  let modeTitle = $derived(
    activeFilter.searchFuzzy
      ? "Fuzzy — ranked candidates (click for exact)"
      : "Exact — the matching set (click for fuzzy)",
  );
</script>

<div class="search-wrap">
  <button
    type="button"
    class="search-mode"
    class:exact={!activeFilter.searchFuzzy}
    onclick={toggleSearchMode}
    aria-pressed={!activeFilter.searchFuzzy}
    title={modeTitle}
  >{activeFilter.searchFuzzy ? "✦" : "🔍"}</button>
  <input
    id="sidebar-search-input"
    class="search"
    type="text"
    placeholder="Search cover / labels…"
    value={activeFilter.searchText}
    oninput={handleInput}
    onkeydown={handleKeydown}
  />
  {#if activeFilter.searchText.length > 0}
    <button class="search-clear" onclick={handleClearButton} aria-label="Clear search">✕</button>
  {/if}
</div>

<style>
  .search-wrap {
    position: relative;
    margin-bottom: 0.5rem;
  }

  /* Sits where the magnifier normally would, inside the field's own
     left inset — the mode is a property of this box, not a control
     floating next to it. */
  .search-mode {
    position: absolute;
    left: 0.15rem;
    top: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    padding: 0;
    background: none;
    border: none;
    font-size: 0.8rem;
    line-height: 1;
    color: var(--accent-ink);
    cursor: pointer;
  }

  .search-mode:hover {
    color: var(--ink);
  }

  .search-mode.exact {
    color: var(--info-ink);
  }

  .search {
    width: 100%;
    padding: 0.35rem 1.5rem 0.35rem 1.75rem;
    font-size: 0.8rem;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: var(--surface-raised);
    box-sizing: border-box;
  }

  .search:focus {
    outline: none;
    border-color: var(--accent-line-strong);
  }

  .search-clear {
    position: absolute;
    right: 0.25rem;
    top: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.4rem;
    padding: 0;
    background: none;
    border: none;
    font-size: 0.75rem;
    color: var(--ink-faint);
    cursor: pointer;
    line-height: 1;
  }

  .search-clear:hover {
    color: var(--ink-secondary);
  }
</style>
