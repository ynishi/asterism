<script lang="ts">
  // TagList — extracted from App.svelte (2026-07-20 Phase C wave 5a).
  // Owns the "Tags" sidebar section: OR-mode heading, free-text filter
  // input (when expanded), the "● all" row, one row per tag with a
  // checkbox + count, and the "show top / show all (N)" + "+N more"
  // pagination affordances.
  //
  // State:
  //   - tagCatalog.counts.data                       (store)
  //   - activeFilter.activeTagIds               (store)
  //   - tagsExpanded / tagsFilter / tagsRenderCap (local — ephemeral
  //     sidebar-section UX, App does not need to reason about it)
  //
  // Selection uses `activeFilter.toggleTag()` — the App-side reload
  // `$effect` picks up the change transparently.
  //
  // CSS: this component duplicates the `.tags-list` / `.tag-name` /
  // `.tag-count` / `.tags-toggle` / `.tags-empty` / `.tags-filter` /
  // `.tags-active-count` rules from App.svelte because Svelte scoped
  // CSS does not reach across component boundaries. App keeps its
  // copy for the Groups sidebar section (which reuses the same class
  // names) — same duplication pattern as ModalityList (wave 4).
  import type { TagCountDto } from "./bindings";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { tagCatalog } from "./lib/stores/tag.svelte";

  // Sidebar space budget: top-N by count, "show all" expands into a
  // scrollable capped list. Numbers are section-local knobs, not
  // routing-level constants, so they live with the template.
  const TAGS_SIDEBAR_TOP_N = 12;
  const TAGS_PAGE_STEP = 500;

  let tagsExpanded = $state(false);
  // Free-text filter for the expanded list. `show all` on a large tag
  // corpus (10k+ tags is a realistic case) flat-renders a huge DOM
  // tree and freezes the browser; the filter narrows the slice before
  // render, and the hard cap below keeps the sidebar responsive even
  // without a filter query.
  let tagsFilter = $state("");
  // How many filtered rows the sidebar is currently allowed to
  // render. Bumped by TAGS_PAGE_STEP every time the User clicks "+N
  // more". Reset to the initial page whenever the filter query flips.
  let tagsRenderCap = $state(TAGS_PAGE_STEP);

  let visibleTagCounts = $derived.by<{ rows: TagCountDto[]; truncated: number }>(() => {
    if (!tagsExpanded) {
      return { rows: tagCatalog.counts.data.slice(0, TAGS_SIDEBAR_TOP_N), truncated: 0 };
    }
    const q = tagsFilter.trim().toLowerCase();
    const filtered = q
      ? tagCatalog.counts.data.filter((tc) => tc.tag.name.toLowerCase().includes(q))
      : tagCatalog.counts.data;
    const truncated = Math.max(0, filtered.length - tagsRenderCap);
    return { rows: filtered.slice(0, tagsRenderCap), truncated };
  });

  // Reset the render cap whenever the filter query changes so a
  // narrower query doesn't carry a giant cap into a small list, and
  // a widened / cleared query starts from the base page again.
  $effect(() => {
    void tagsFilter;
    tagsRenderCap = TAGS_PAGE_STEP;
  });
</script>

<h2>Tags {#if activeFilter.activeTagIds.size > 0}<span class="tags-active-count">
  · {activeFilter.activeTagIds.size} OR</span>{/if}</h2>
{#if tagsExpanded && tagCatalog.counts.data.length > TAGS_SIDEBAR_TOP_N}
  <input
    class="tags-filter"
    type="text"
    placeholder="filter tags…"
    bind:value={tagsFilter}
  />
{/if}
<ul class="tags-list">
  <li>
    <button
      class:active={activeFilter.activeTagIds.size === 0}
      onclick={() => activeFilter.clearTags()}
    >
      ● all
    </button>
  </li>
  {#if tagCatalog.counts.data.length === 0}
    <li class="tags-empty">no tags yet</li>
  {:else}
    {#each visibleTagCounts.rows as tc (tc.tag.id)}
      <li>
        <button
          class:active={activeFilter.activeTagIds.has(tc.tag.id)}
          onclick={() => activeFilter.toggleTag(tc.tag)}
          title={tc.tag.axis ? `axis: ${tc.tag.axis}` : "unclassified"}
        >
          <span class="tag-name">
            {activeFilter.activeTagIds.has(tc.tag.id) ? "☑" : "☐"} {tc.tag.name}
          </span>
          <span class="tag-count">{tc.asset_count}</span>
        </button>
      </li>
    {/each}
    {#if tagsExpanded && visibleTagCounts.truncated > 0}
      <li>
        <button
          class="tags-toggle"
          onclick={() => (tagsRenderCap += TAGS_PAGE_STEP)}
          title={`Show ${Math.min(TAGS_PAGE_STEP, visibleTagCounts.truncated)} more tags (refine with the filter to skip the scroll)`}
        >
          +{visibleTagCounts.truncated} more · load
          {Math.min(TAGS_PAGE_STEP, visibleTagCounts.truncated)} more
        </button>
      </li>
    {/if}
    {#if tagCatalog.counts.data.length > TAGS_SIDEBAR_TOP_N}
      <li>
        <button class="tags-toggle" onclick={() => {
          tagsExpanded = !tagsExpanded;
          if (!tagsExpanded) {
            tagsFilter = "";
            tagsRenderCap = TAGS_PAGE_STEP;
          }
        }}>
          {tagsExpanded ? "show top" : `show all (${tagCatalog.counts.data.length})`}
        </button>
      </li>
    {/if}
  {/if}
</ul>

<style>
  /* Sidebar heading (same cascade as `.sidebar h2` in App.svelte).
     Kept in sync until the whole sidebar graduates out of App
     (wave 9). */
  h2 {
    font-size: 0.75rem;
    color: var(--ink-muted);
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
    color: var(--ink-secondary);
    cursor: pointer;
    width: 100%;
    text-align: left;
    border-radius: 4px;
    font-family: inherit;
  }
  button:hover {
    background: var(--surface-hover);
  }
  button.active {
    color: var(--ink);
    font-weight: 600;
    background: var(--surface-active);
  }

  /* Tags-specific two-column row (name + count). */
  .tags-list button {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.4rem;
  }
  .tags-filter {
    width: 100%;
    box-sizing: border-box;
    padding: 0.28rem 0.5rem;
    margin: 0.2rem 0 0.35rem;
    font-size: 0.8rem;
    border: 1px solid var(--accent-line);
    border-radius: 4px;
    background: var(--accent-surface);
    color: inherit;
    outline: none;
  }
  .tags-filter:focus {
    border-color: var(--accent-line-strong);
    background: var(--surface-raised);
  }
  .tag-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag-count {
    font-size: 0.6rem;
    color: var(--accent-ink-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .tags-toggle {
    font-size: 0.6rem;
    color: var(--accent-ink);
    font-style: italic;
  }
  .tags-empty {
    font-size: 0.65rem;
    color: var(--ink-faint);
    padding: 0.2rem 0.4rem;
    list-style: none;
  }
  .tags-active-count {
    font-size: 0.6rem;
    color: var(--accent-ink);
    font-weight: normal;
  }
</style>
