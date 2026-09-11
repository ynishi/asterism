<script lang="ts">
  // ActiveFilters — the grid-top band that surfaces every engaged
  // filter axis as one removable chip row.
  // Replaces the sidebar-local "Active filters" chip band, which showed
  // three axes and left the rest to be found in the sidebar, with one
  // row carrying whichever are engaged — so the whole filter can be
  // read and cleared next to the grid it produces. The template below
  // is the list; an enumeration here would be a second one to keep.
  //
  // Data is read 0-prop from the shared stores:
  //   - `activeFilter`   — every selection axis + its carried names
  //   - `personaName()`  — persona id → label (reads personaCatalog)
  //   - `modalityCatalog.labelOf` — modality slug → label (master-driven)
  // Tag / group chip labels come from `activeFilter.active*Names`,
  // populated atomically with the id sets, so no catalog join is
  // needed here.
  //
  // The three callback props are App-owned grid side effects (the
  // one prop category the 0-prop rule allows): search-clear and reset both
  // flush the App-side search debounce timer + reload, and save opens
  // the App-owned custom-prompt modal. Every other chip's clear
  // mutates `activeFilter` directly, because the App reload `$effect`
  // already tracks the field it clears.
  //
  // Two chips carry a *mode* rather than a selection:
  //   - the AND checkbox flips `tagMatchAll` (OR ⇄ AND over the tag
  //     chips). It appears only from the second tag on, because with one
  //     chip the two compositions are the same question — the state
  //     itself survives dropping back to one, so re-adding a tag does
  //     not silently lose the choice. Mutates the store directly for the
  //     same reason the per-axis clears do.
  //   - the search chip leads with ✦ / 🔍 to say which domain answered:
  //     ranked candidates vs the exact matching set. The chip is shown
  //     and cleared identically either way; only the glyph differs.
  //
  // The 🎲 chip is a third kind again: not a filter and not a mode but a
  // view state — the grid is showing a random handful of whatever the
  // other chips select. It earns a chip because it changes what
  // the grid means more than any single axis does, and because the row
  // is where a user looks to find out why the grid is not a listing.
  import { activeFilter, viewerTimeZone } from "./lib/stores/filter.svelte";
  import { personaName } from "./lib/formatters";
  import { modalityCatalog } from "./lib/stores/modality.svelte";
  // Colour chips need the swatch ink + label the sidebar uses, so the
  // two surfaces name a bucket the same way.
  import { colorCatalog } from "./lib/stores/color.svelte";

  interface Props {
    onClearSearch: () => void;
    onReset: () => void;
    // "Save as Group" — the W5 successor of the old
    // `onSaveQuery`: persists the current filter chips + Sorter as a
    // Query Group under the active persona.
    onSaveAsGroup: () => void;
  }

  let { onClearSearch, onReset, onSaveAsGroup }: Props = $props();

  let searchTrimmed = $derived(activeFilter.searchText.trim());

  let anyFilterActive = $derived(
    searchTrimmed.length > 0
      || activeFilter.activePersona !== null
      || activeFilter.activeModality !== null
      || activeFilter.activeFormat !== null
      || activeFilter.activeColor !== null
      || activeFilter.activeLabel !== null
      || activeFilter.activeTagIds.size > 0
      || activeFilter.activeGroupIds.size > 0
      || activeFilter.activeSessionId !== null
      // The calendar filter earns a chip where the metric bands do not,
      // and the difference is how much it hides. A size band trims the
      // library; a week of it removes nearly all of it, so a grid that
      // looks empty needs a line on screen saying what emptied it —
      // the same argument the 🎲 chip below carries.
      || activeFilter.hasDayFilter()
      // Without this the band would stay hidden on a bare 🎲 draw — the
      // one state where the grid least resembles a listing and most
      // needs a line saying why.
      || activeFilter.discoverRandom,
  );

  function modalityLabel(slug: string): string {
    return modalityCatalog.labelOf(slug);
  }

  // The Inbox toggle stores the literal label `"inbox"`; other label
  // axes (Voice roles) reuse the same field. Render a friendly glyph
  // for the Inbox case and the raw slug otherwise.
  function labelChipText(label: string): string {
    return label === "inbox" ? "📥 inbox" : label;
  }

  // The calendar filter in as few words as it can honestly be put. A
  // recognised span names itself against its date; a day-of-year names
  // the month and day; anything else — a restored rule the picker did
  // not draw — shows the range, because calling an arbitrary range "a
  // day" would be a claim about the filter that is not true. The zone
  // is appended only when it is not the viewer's own, which is the one
  // case it says something.
  function dayChipText(): string {
    const zone =
      activeFilter.dayTimeZone === viewerTimeZone() ? "" : ` (${activeFilter.dayTimeZone})`;
    const doy = activeFilter.dayOfYear;
    if (doy !== null) {
      const pad = (n: number) => String(n).padStart(2, "0");
      return `${pad(doy.month)}-${pad(doy.day)} · every year${zone}`;
    }
    const date = activeFilter.jumpDate();
    const span = activeFilter.jumpSpan();
    if (date !== null && span !== null) return `${date} · ${span}${zone}`;
    return `${activeFilter.dayRangeText()}${zone}`;
  }
</script>

{#if anyFilterActive}
  <div class="active-filters-band" role="group" aria-label="Active filters">
    <span class="afb-lead">Filters</span>

    {#if activeFilter.activePersona !== null}
      <button
        type="button"
        class="afb-chip persona"
        onclick={() => (activeFilter.activePersona = null)}
        title="Clear persona filter"
      >
        ◈ {personaName(activeFilter.activePersona)} <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if activeFilter.activeModality !== null}
      <button
        type="button"
        class="afb-chip modality"
        onclick={() => (activeFilter.activeModality = null)}
        title="Clear modality filter"
      >
        ▤ {modalityLabel(activeFilter.activeModality)} <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if activeFilter.activeFormat !== null}
      <button
        type="button"
        class="afb-chip modality"
        onclick={() => (activeFilter.activeFormat = null)}
        title="Clear format filter"
      >
        ▣ {activeFilter.activeFormat} <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if activeFilter.activeColor !== null}
      <button
        type="button"
        class="afb-chip modality"
        onclick={() => (activeFilter.activeColor = null)}
        title="Clear colour filter"
      >
        <span
          class="afb-swatch"
          style="background: {colorCatalog.hexOf(activeFilter.activeColor)}"
        ></span>
        {colorCatalog.labelOf(activeFilter.activeColor)}
        <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if activeFilter.hasDayFilter()}
      <button
        type="button"
        class="afb-chip day"
        onclick={() => activeFilter.clearJump()}
        title="Clear the date filter"
      >
        ⌖ {dayChipText()} <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if activeFilter.activeLabel !== null}
      <button
        type="button"
        class="afb-chip label"
        onclick={() => (activeFilter.activeLabel = null)}
        title="Clear label filter"
      >
        {labelChipText(activeFilter.activeLabel)} <span class="afb-x">✕</span>
      </button>
    {/if}

    {#each Array.from(activeFilter.activeTagNames) as [tid, tname] (tid)}
      <button
        type="button"
        class="afb-chip tag"
        onclick={() => activeFilter.removeTag(tid)}
        title="Remove #{tname} from filter"
      >
        # {tname} <span class="afb-x">✕</span>
      </button>
    {/each}

    {#if activeFilter.activeTagNames.size >= 2}
      <label
        class="afb-tagmatch"
        title="Require every tag (AND) instead of any (OR)"
      >
        <input type="checkbox" bind:checked={activeFilter.tagMatchAll} />
        AND
      </label>
    {/if}

    {#each Array.from(activeFilter.activeGroupNames) as [gid, gname] (gid)}
      <button
        type="button"
        class="afb-chip group"
        onclick={() => activeFilter.removeGroup(gid)}
        title="Remove ~{gname} from filter"
      >
        ~ {gname} <span class="afb-x">✕</span>
      </button>
    {/each}

    {#if activeFilter.activeSessionId && activeFilter.activeSessionLabel}
      <button
        type="button"
        class="afb-chip session"
        onclick={() => activeFilter.clearSession()}
        title="Clear session filter"
      >
        ⇱ {activeFilter.activeSessionLabel} <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if searchTrimmed.length > 0}
      <button
        type="button"
        class="afb-chip search"
        onclick={onClearSearch}
        title={activeFilter.searchFuzzy
          ? "Clear search text (fuzzy — ranked candidates)"
          : "Clear search text (exact — the matching set)"}
      >
        {activeFilter.searchFuzzy ? "✦" : "🔍"}
        “{searchTrimmed}” <span class="afb-x">✕</span>
      </button>
    {/if}

    {#if activeFilter.discoverRandom}
      <button
        type="button"
        class="afb-chip search"
        onclick={() => activeFilter.toggleDiscoverRandom()}
        title="Stop drawing at random and show the listing again"
      >
        🎲 random <span class="afb-x">✕</span>
      </button>
    {/if}

    <span class="afb-controls">
      {#if activeFilter.activePersona !== null}
        <button
          type="button"
          class="afb-ctrl"
          onclick={onSaveAsGroup}
          title="Save current filter + sort as a Query Group (materialised members refresh on rule / data change)"
          aria-label="Save current filter as a Query Group"
        >⊕ Save as Group</button>
      {/if}
      <button
        type="button"
        class="afb-ctrl afb-clear-all"
        onclick={onReset}
        title="Clear every active filter"
      >
        Clear all
      </button>
    </span>
  </div>
{/if}

<style>
  .active-filters-band {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    padding: 0.4rem 0.6rem;
    margin: 0 0 0.5rem;
    background: var(--surface-raised);
    border: 1px solid var(--accent-line);
    border-radius: 8px;
  }

  .afb-lead {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--accent-ink);
    margin-right: 0.15rem;
  }

  .afb-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.12rem 0.55rem;
    font-size: 0.72rem;
    font-family: inherit;
    /* Neutral, so that each axis below reads as a hue against it. The
       base wore the accent until the palette landed, which left the
       persona axis — also accent — indistinguishable from a chip
       carrying no axis at all. */
    color: var(--ink-secondary);
    background: var(--surface-hover);
    border: 1px solid var(--line);
    border-radius: 999px;
    cursor: pointer;
  }
  .afb-chip:hover {
    background: var(--surface-active);
    color: var(--ink);
  }

  /* Per-axis palettes so the axes read apart at a glance. Kept in the
     same hue families as the sidebar chips they replace. */
  .afb-chip.group {
    background: var(--success-surface);
    border-color: var(--success-line);
    color: var(--success-ink);
  }
  .afb-chip.group:hover {
    background: var(--success-surface-strong);
    color: var(--success-ink);
  }
  .afb-chip.session {
    background: var(--surface-hover);
    border-color: var(--warning-line);
    color: var(--warning-ink);
  }
  .afb-chip.session:hover {
    background: var(--warning-surface);
    color: var(--warning-ink);
  }
  .afb-chip.persona {
    background: var(--accent-surface);
    border-color: var(--accent-line);
    color: var(--accent-ink);
  }
  .afb-chip.persona:hover {
    background: var(--accent-surface-strong);
    color: var(--accent-ink);
  }
  .afb-chip.modality {
    background: var(--accent-surface);
    border-color: var(--cat-orchid);
    color: var(--cat-orchid);
  }
  .afb-chip.modality:hover {
    background: var(--accent-surface-strong);
    color: var(--cat-orchid);
  }
  .afb-chip.label {
    background: var(--danger-surface);
    border-color: var(--danger-line);
    color: var(--danger-ink);
  }
  .afb-chip.label:hover {
    background: var(--danger-surface-strong);
    color: var(--danger-ink);
  }
  /* The date chip takes the persona chip's accent, which is the
     sidebar's "this is the axis in force" ink. It is the strongest
     narrowing on the row and the one most likely to be why the grid
     looks empty, so it reads at the same weight as the persona. */
  .afb-chip.day {
    background: var(--accent-surface);
    border-color: var(--accent-line);
    color: var(--accent-ink);
  }
  .afb-chip.day:hover {
    background: var(--accent-surface-strong);
    border-color: var(--accent-line-strong);
  }
  .afb-chip.search {
    background: var(--info-surface);
    border-color: var(--info-line);
    color: var(--info-ink);
  }
  .afb-chip.search:hover {
    background: var(--info-surface-strong);
    color: var(--info-ink);
  }

  /* Reads as a qualifier on the tag chips it follows, not as another
     chip: no pill, no remove affordance. */
  .afb-tagmatch {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    font-size: 0.68rem;
    letter-spacing: 0.03em;
    color: var(--accent-ink);
    cursor: pointer;
    user-select: none;
  }

  .afb-tagmatch input {
    margin: 0;
    width: 0.75rem;
    height: 0.75rem;
    cursor: pointer;
  }

  .afb-x {
    font-size: 0.6rem;
    opacity: 0.6;
  }

  /* Stands in for the glyph the other chips lead with (◈ / ▤ / ▣).
     The catalogue runs from White to Black, so whichever end the app
     is dark or light at, one of them meets the chip it sits on; the
     hairline is what keeps that one an edge rather than a hole. */
  .afb-swatch {
    display: inline-block;
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 2px;
    border: 1px solid var(--line-strong);
    vertical-align: -1px;
  }

  .afb-controls {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin-left: auto;
  }

  .afb-ctrl {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 0.1rem 0.4rem;
    font-size: 0.72rem;
    font-family: inherit;
    color: var(--accent-ink);
    cursor: pointer;
    line-height: 1.3;
  }
  .afb-ctrl:hover {
    background: var(--accent-surface);
    border-color: var(--accent-line);
  }
  .afb-clear-all {
    color: var(--danger-ink);
  }
  .afb-clear-all:hover {
    background: var(--danger-surface);
    border-color: var(--danger-line);
  }
</style>
