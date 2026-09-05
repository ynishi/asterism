<script lang="ts">
  // PersonaStrip — extracted from App.svelte (2026-07-20 Phase C
  // wave 3). Owns the "Persona" sidebar section: the "● all" row +
  // one row per persona, each carrying an avatar mini, an asset
  // count, and (on the active row) a wallpaper-clear affordance.
  //
  // State comes through four stores + a handful of App-owned props:
  //   - personaCatalog.list / .countById / .totalCount  (store)
  //   - activeFilter.activePersona                       (store)
  //   - themeCatalog.theme                               (store; the
  //     wallpaper-clear affordance reads it directly since wave 8a)
  //   - profileCatalog.profiles / avatarUrl              (store; the
  //     avatar-mini reads the map + blob-URL cache directly since
  //     wave 8b)
  //   - onPersonaHoverEnter / Leave                      (prop; profile
  //     card open/close is App-owned. W1 hover regrammar: the open
  //     fires from the row's ⓘ aim target — hover or click — not
  //     from the row body; row leave still drives the grace close)
  //   - onClearWallpaper                                 (prop; theme
  //     mutation runs invoke + refetch in App)
  //
  // Persona selection is a direct `activeFilter.activePersona = ...`
  // mutation. The App-side `$effect` block that tracks that field
  // still fires `loadAssets` / `loadTagCounts` etc. transparently
  // (reaction ownership).
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { personaCatalog } from "./lib/stores/personas.svelte";
  import { profileCatalog } from "./lib/stores/profile.svelte";
  import { themeCatalog } from "./lib/stores/theme.svelte";

  interface Props {
    // `explicit` = true for a real click on the ⓘ — App lets it
    // bypass the hover-suppression guard (mirrors the card icons'
    // guarded-hover / unguarded-click split).
    onPersonaHoverEnter: (personaId: string, ev: MouseEvent, explicit?: boolean) => void;
    onPersonaHoverLeave: () => void;
    onClearWallpaper: () => void;
  }

  let {
    onPersonaHoverEnter,
    onPersonaHoverLeave,
    onClearWallpaper,
  }: Props = $props();
</script>

<h2>Persona</h2>
<ul>
  <li>
    <button
      class:active={activeFilter.activePersona === null}
      onclick={() => (activeFilter.activePersona = null)}
    >
      ● all
      <span class="sidebar-count">{personaCatalog.totalCount}</span>
    </button>
  </li>
  {#each personaCatalog.list.data as persona (persona.id)}
    <li
      class="persona-row"
      data-persona-id={persona.id}
      onmouseleave={onPersonaHoverLeave}
    >
      <button
        class:active={activeFilter.activePersona === persona.id}
        onclick={() => (activeFilter.activePersona = persona.id)}
      >
        {#if profileCatalog.avatarUrl(profileCatalog.profiles.get(persona.id)?.avatar_asset_id)}
          <img
            class="persona-avatar-mini"
            src={profileCatalog.avatarUrl(profileCatalog.profiles.get(persona.id)?.avatar_asset_id) ?? ""}
            alt=""
          />
        {:else}
          ○
        {/if}
        {persona.name}
        <span class="sidebar-count">{personaCatalog.countById.get(persona.id) ?? 0}</span>
      </button>
      <!-- ⓘ aim target (W1 hover regrammar): revealed on row hover
           in place of the count badge; pointing at it opens the
           profile card immediately (click works too, for keyboard /
           touch reachability). The row body itself never opens. -->
      <button
        class="persona-info"
        onmouseenter={(e) => onPersonaHoverEnter(persona.id, e)}
        onclick={(e) => { e.stopPropagation(); onPersonaHoverEnter(persona.id, e, true); }}
        title="Profile"
        aria-label="Show profile card"
      >ⓘ</button>
      {#if activeFilter.activePersona === persona.id && themeCatalog.theme?.wallpaper_asset_id}
        <button
          class="persona-wallpaper-clear"
          onclick={onClearWallpaper}
          title="Clear wallpaper"
          aria-label="Clear wallpaper"
        >▨×</button>
      {/if}
    </li>
  {/each}
</ul>

<style>
  /* Mirror the App-side `.sidebar h2` / `.sidebar ul` / `.sidebar
     button` / `.sidebar-count` cascade — the scoped-CSS namespace
     switch when this section moved into its own component severed
     the ancestor selector match. Kept in sync with the sibling
     rules in App.svelte until the whole `<aside class="sidebar">`
     graduates out of App as well. */
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

  /* Right-aligned count badge inside the row button — floated span
     so the base `text-align: left; width: 100%` stays untouched. */
  .sidebar-count {
    float: right;
    font-size: 0.7rem;
    color: var(--accent-ink-dim);
    font-variant-numeric: tabular-nums;
    padding-left: 0.4rem;
  }
  button.active .sidebar-count {
    color: var(--accent-ink);
  }

  /* ⓘ aim target — absolutely positioned over the count badge's
     spot at the row's right edge; invisible until the row is
     hovered, then swaps in for the count (the count fades out via
     the rule below). Keyboard focus also reveals it (WCAG 3.2.7 —
     hover must not be the only reachability path). */
  .persona-row {
    position: relative;
  }
  .persona-info {
    position: absolute;
    right: 0.15rem;
    top: 0.12rem;
    display: inline-block;
    width: auto;
    padding: 0 0.25rem;
    border: none;
    background: none;
    color: var(--accent-ink);
    font-size: 0.8rem;
    cursor: pointer;
    opacity: 0;
  }
  .persona-row:hover .persona-info,
  .persona-info:focus-visible {
    opacity: 1;
  }
  .persona-row:hover .sidebar-count {
    opacity: 0;
  }
  .persona-info:hover {
    color: var(--accent-ink);
    background: var(--surface-hover);
    border-radius: 3px;
  }

  /* Round mini-avatar inside the row label. Falls through to the
     plain "○" bullet in the template when no avatar is set. */
  .persona-avatar-mini {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    object-fit: cover;
    vertical-align: middle;
    margin-right: 0.35em;
  }

  /* Wallpaper-clear affordance — appears next to the active persona
     row when a theme wallpaper is set. Small ▨× glyph on a muted
     chip so the row's primary button keeps focus. Override the
     parent-scoped `button` reset so the chip sizes independently
     of the row-wide 100% width. */
  .persona-wallpaper-clear {
    /* Sits above the hover-revealed absolute ⓘ so the clear
       affordance stays clickable on the active row (LOW-2). */
    position: relative;
    z-index: 1;
    display: inline-block;
    width: auto;
    margin-left: 0.35rem;
    padding: 0 0.35rem;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: var(--surface-raised);
    cursor: pointer;
    font-size: 0.75rem;
    color: var(--ink-secondary);
  }
  .persona-wallpaper-clear:hover {
    background: var(--danger-surface);
    color: var(--danger-ink);
  }
</style>
