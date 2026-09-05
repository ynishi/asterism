<script lang="ts">
  // Shared between `ForgePanel` and `SharedLinesPanel` (#217): three
  // instances of the same row of buttons, one active, that differed
  // only because nobody had unified them, not because any of the
  // difference was intentional. `ForgePanel`'s gap, border colour,
  // button padding, font size and lack of dimming on the inactive tabs
  // are the values kept here; `SharedLinesPanel`'s two tab rows now
  // render the same way, which is a visible change on that plane — no
  // more dimmed inactive tabs, a different border colour and gap, and
  // a font size no longer set explicitly (`ForgePanel`'s row never set
  // one either).
  //
  // What stays out of this component is any prop for gap, colour,
  // padding or size: a caller position its own outer margin with a
  // wrapper element, the way `ForgePanel` and both of `SharedLinesPanel`'s
  // rows do below, rather than this component growing a knob for
  // something that no longer varies between callers.
  //
  // ARIA: `ForgePanel`'s row already carried `role="tablist"` /
  // `role="tab"` / `aria-selected`, with no `aria-label`;
  // `SharedLinesPanel`'s two rows carried an `aria-label` (kept here as
  // `ariaLabel`) but neither role nor `aria-selected`. Both get all of
  // it now rather than a prop deciding which caller earns which half:
  // activating a tab here only ever reveals a sibling panel already on
  // screen (lazily loading it, in `ForgePanel`'s `history` and both of
  // `SharedLinesPanel`'s `roster` and `ledger`), never a navigation or
  // a write — which is the WAI-ARIA tabs pattern's own precondition.
  interface Tab {
    key: string;
    label: string;
    onSelect: () => void;
  }

  interface Props {
    tabs: Tab[];
    active: string;
    ariaLabel: string;
  }

  let { tabs, active, ariaLabel }: Props = $props();
</script>

<div class="tab-strip" role="tablist" aria-label={ariaLabel}>
  {#each tabs as t (t.key)}
    <button
      type="button"
      role="tab"
      aria-selected={t.key === active}
      onclick={t.onSelect}
    >
      {t.label}
    </button>
  {/each}
</div>

<style>
  .tab-strip {
    display: flex;
    gap: 0.8rem;
    border-bottom: 1px solid var(--line);
  }
  .tab-strip button {
    background: none;
    border: 0;
    border-bottom: 2px solid transparent;
    color: inherit;
    cursor: pointer;
    padding: 0.3rem 0;
  }
  .tab-strip button[aria-selected="true"] {
    border-bottom-color: currentColor;
  }
</style>
