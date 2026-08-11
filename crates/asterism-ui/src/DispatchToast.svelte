<script lang="ts">
  // DispatchToast — extracted from App.svelte (2026-07-21 Phase
  // C wave C). Fixed-position toast at the bottom of the layout
  // that mirrors `dispatchCatalog.status`. Gated on
  // `status !== null`; the store auto-clears via `flash(msg,
  // ms)` (fade after `ms`) or `pollDispatch` (fade after the
  // 6-s terminal-state grace).
  //
  // Consumes:
  //   - dispatchCatalog.status                              (store)
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
</script>

{#if dispatchCatalog.status !== null}
  <div class="dispatch-toast" role="status" aria-live="polite">
    {dispatchCatalog.status}
  </div>
{/if}

<style>
  /* Copied verbatim from App.svelte in wave C — same
     duplication policy as the other extracted overlays. */

  .dispatch-toast {
    position: fixed;
    bottom: 5rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.5rem 1rem;
    background: #1f1e33;
    color: #e9e7ff;
    border-radius: 6px;
    font-size: 0.85rem;
    box-shadow: 0 6px 18px rgba(23, 22, 42, 0.3);
    z-index: 45;
    max-width: 60ch;
  }
</style>
