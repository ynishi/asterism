<script lang="ts">
  // Shared between `ForgePanel` and `SharedLinesPanel` (#217): three
  // instances of the same row of buttons, one active, that differed
  // only because nobody had unified them, not because any of the
  // difference was intentional. `ForgePanel`'s gap, border colour,
  // button padding and lack of dimming on the inactive tabs are the
  // values kept here; `SharedLinesPanel`'s two tab rows now render the
  // same way — the visible change this brings, matching the shape
  // `SharedLinesPanel`'s own header already argued for (decision 19:
  // "the three tabs are the forge's three answers about one line").
  //
  // What stays out of this component is any prop for gap, colour,
  // padding or size: a caller position its own outer margin with a
  // wrapper element, the way `ForgePanel` and both of `SharedLinesPanel`'s
  // rows do below, rather than this component growing a knob for
  // something that no longer varies between callers.
  //
  // ARIA: `ForgePanel`'s row already carried `role="tablist"` /
  // `role="tab"` / `aria-selected` — activating a tab here only ever
  // reveals a sibling panel already on screen (lazily loading it, in
  // `ForgePanel`'s `history` and both of `SharedLinesPanel`'s `roster`
  // and `ledger`), never a navigation or a write — which is the WAI-ARIA
  // tabs pattern's own precondition. `SharedLinesPanel`'s two rows had
  // no ARIA at all; they get the same role here rather than a prop
  // deciding whether they should.
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
    <button role="tab" aria-selected={t.key === active} onclick={t.onSelect}>
      {t.label}
    </button>
  {/each}
</div>

<style>
  .tab-strip {
    display: flex;
    gap: 0.8rem;
    border-bottom: 1px solid rgba(128, 128, 128, 0.3);
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
