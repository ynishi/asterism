<script lang="ts">
  // CardActionIcons — the Eagle-style Floating icon strip that sits
  // inside the card on hover. Extracted from the inline block that
  // used to live under `App.svelte` (grid Messages card, line 4724-
  // 4757 pre-refactor) so both the Messages grid Card and the
  // SessionsView tile can consume one uniform Floating.
  //
  // Contract:
  //   * `hasNote` / `hasThread` / `hasConstellation` — filled-state
  //     inputs. Filled tone signals "already has content".
  //   * `showConstellation` — Session tile passes `false` (Session has
  //     no per-asset constellation grouping).
  //   * `onNoteClick` / `onThreadClick` / `onConstellationClick` —
  //     click callbacks. Component fires them raw; the caller is the
  //     one that should `event.stopPropagation()` (the outer card is
  //     also a click target).
  //   * `onConstellationHoverEnter` — the ✦ icon's aim-hover opener
  //     (grid card path only; Session tile leaves it unset).
  //   * `onOverlayEnter` / `onOverlayLeave` — pointer grace so a hop
  //     from the card onto a spawned overlay does not blink it out.
  //
  // CSS: the class names (`.card-action-icons` / `.card-action-icon`
  // / `.filled`) plus the `.card:hover` reveal cascade, its
  // `:focus-within` keyboard counterpart, and the
  // `.content.clean-mode .card .card-action-icons` hide rule are all
  // declared here with `:global(...)`. The parent card is *outside*
  // this component's scoped tree, so a scoped selector would never
  // match; `:global` is the only way to keep the hover reveal cascade
  // firing from either the Messages grid `.card` or the SessionsView
  // `.card.session-tile`.
  //   * `trashMode` — the grid is showing the trash. The note / thread
  //     / constellation icons give way to Restore, because content
  //     actions on a trashed card are noise and mixing them in invites
  //     the wrong click.
  //
  // # No destructive action lives in this strip
  //
  // The strip is hover chrome: it appears under the pointer, over the
  // card, with no click already committed to it. That is the wrong
  // place for anything that removes a card from where the user put it
  // — a mis-aimed pointer lands on a control that was not on screen
  // when the movement started. Both destructive actions were here
  // once (a 🗑 on the live side, a 🔥 "Delete forever" on the trash
  // side) and both moved out to the card context menu, which is the
  // standard place for them: opened deliberately, destructive entry
  // last, in destructive tone (Apple HIG; none of the eight surveyed
  // library apps put delete behind hover).
  //
  // What stays is non-destructive: Note / Thread / Constellation open
  // panels, and ↩︎ Restore *undoes* a removal. Adding a destructive
  // icon back here is a regression, not a convenience.
  interface Props {
    hasNote: boolean;
    hasThread: boolean;
    hasConstellation?: boolean;
    showConstellation?: boolean;
    trashMode?: boolean;
    onNoteClick: (ev: MouseEvent) => void;
    onThreadClick: (ev: MouseEvent) => void;
    onConstellationClick?: (ev: MouseEvent) => void;
    onConstellationHoverEnter?: (ev: MouseEvent) => void;
    onRestoreClick?: (ev: MouseEvent) => void;
    onOverlayEnter?: () => void;
    onOverlayLeave?: () => void;
  }

  let {
    hasNote,
    hasThread,
    hasConstellation = false,
    showConstellation = true,
    trashMode = false,
    onNoteClick,
    onThreadClick,
    onConstellationClick,
    onConstellationHoverEnter,
    onRestoreClick,
    onOverlayEnter,
    onOverlayLeave,
  }: Props = $props();
</script>

<div
  class="card-action-icons"
  onmouseenter={() => onOverlayEnter?.()}
  onmouseleave={() => onOverlayLeave?.()}
>
  {#if trashMode}
    <button
      type="button"
      class="card-action-icon"
      title="Restore"
      aria-label="Restore from trash"
      onclick={(e) => onRestoreClick?.(e)}
    >↩︎</button>
    <!-- Delete forever is NOT here — see the note at the top of this
         file. It lives at the bottom of the card context menu. -->
  {:else}
  <button
    type="button"
    class="card-action-icon"
    class:filled={hasNote}
    title="Note"
    aria-label="Open note"
    onclick={onNoteClick}
  >📝</button>
  <button
    type="button"
    class="card-action-icon"
    class:filled={hasThread}
    title="Thread"
    aria-label="Open thread"
    onclick={onThreadClick}
  >💬</button>
  {#if showConstellation}
    <!-- ✦ sits at the strip's right end so the aim target is nearest
         the card-right-anchored burst panel it opens (pointer travel
         ≈ 0). -->
    <button
      type="button"
      class="card-action-icon"
      class:filled={hasConstellation}
      title="Constellation"
      aria-label="Open constellation"
      onmouseenter={(e) => onConstellationHoverEnter?.(e)}
      onclick={(e) => onConstellationClick?.(e)}
    >✦</button>
  {/if}
  {/if}
</div>

<style>
  /* Card action-icon strip — Eagle-style floating menu inside the
     card. Hidden until the pointer settles on the card so it does
     not add visual noise to the default grid state. Each icon is
     a hover target that opens its own overlay; `.filled` indicates
     the target already has content.

     `:global` here because the parent `.card` (Messages grid or
     Sessions tile) lives outside this component's scope; scoped
     Svelte hashes would prevent the `.card:hover .card-action-icons`
     reveal from ever matching. */
  :global(.card-action-icons) {
    position: absolute;
    right: 6px;
    bottom: 6px;
    display: flex;
    gap: 2px;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s;
    background: rgba(255, 255, 255, 0.85);
    border-radius: 999px;
    padding: 2px 4px;
    box-shadow: 0 1px 3px rgba(23, 22, 42, 0.15);
    z-index: 2;
  }
  :global(.card:hover .card-action-icons) {
    opacity: 1;
    pointer-events: auto;
  }
  /* The keyboard half of the same reveal. These are real buttons with
     no `tabindex="-1"`, so they have always been in the tab order —
     which meant Tab could land on a control at `opacity: 0` and Enter
     could fire it with nothing on screen to say where the focus was
     (`pointer-events` gates the pointer, never the keyboard). Kept
     after the destructive icons left the strip: the remaining ones
     still open panels and restore rows, and an invisible focus target
     is a defect regardless of what it does.
     Scoped to the strip rather than `.card:focus-within` on purpose:
     the card itself carries `tabindex="0"`, and focusing the card is
     not a request for its actions. */
  :global(.card-action-icons:focus-within) {
    opacity: 1;
    pointer-events: auto;
  }
  :global(.card-action-icon) {
    background: transparent;
    border: none;
    font-size: 0.85rem;
    padding: 3px 6px;
    cursor: pointer;
    opacity: 0.45;
    line-height: 1;
    border-radius: 999px;
    transition: opacity 0.08s, background 0.08s, transform 0.08s;
  }
  :global(.card-action-icon:hover) {
    opacity: 1;
    background: rgba(88, 80, 255, 0.12);
    transform: scale(1.12);
  }
  :global(.card-action-icon.filled) {
    opacity: 0.95;
  }
  /* Focus lands on one icon, not the strip: the reveal above brings
     the strip up, this brings the focused glyph to full strength so
     the UA focus ring has something legible inside it. */
  :global(.card-action-icon:focus-visible) {
    opacity: 1;
  }
  /* No `.danger` tone in this cascade, deliberately: the strip holds
     no destructive action to tone (see the note at the top). The
     destructive tone lives on `.card-menu-item-danger` in App.svelte,
     next to the entry it warns about. */
  /* Clean-mode hides the strip together with the other card chrome.
     `.content.clean-mode` also lives outside this component so the
     rule ships with a `:global` wrapper too. */
  :global(.content.clean-mode .card .card-action-icons) {
    display: none !important;
  }
</style>
