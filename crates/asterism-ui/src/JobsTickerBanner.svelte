<script lang="ts">
  // JobsTickerBanner — extracted from App.svelte (2026-07-21
  // Phase C wave C). Renders the live job-pipeline ticker (one
  // gauge chip per active kind: pending / running / failed
  // counters), gated on `dispatchCatalog.activeKindGauges.length
  // > 0`. Reads the derived directly so idle kinds vanish and
  // the banner disappears once the queue drains.
  //
  // Not owned: the 3-s `$effect` poll that drives
  // `refreshJobsSnapshot()` on the store — App keeps that
  // effect (component lifecycle surface) and the store owns the
  // state it writes into.
  //
  // Consumes:
  //   - dispatchCatalog.activeKindGauges                    (store)
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
</script>

<!--
  Bottom status chip (W6). Always renders — a stationary
  "Dispatch history" entry that opens the DispatchHistoryPanel drawer
  when the queue is idle, and enriches itself with the live gauge
  chips when work is in flight. Making the banner itself the button
  keeps the affordance discoverable without adding another surface.
-->
<button
  type="button"
  class="jobs-ticker"
  class:jobs-ticker-active={dispatchCatalog.activeKindGauges.length > 0}
  onclick={() => dispatchCatalog.openHistory()}
  aria-label="Open dispatch history"
  title="Dispatch history"
>
  <span
    class="jobs-ticker-dot"
    class:jobs-ticker-dot-idle={dispatchCatalog.activeKindGauges.length === 0}
  ></span>
  {#if dispatchCatalog.activeKindGauges.length === 0}
    <span class="jobs-ticker-kind">Dispatch history</span>
  {:else}
    <!--
      Nested live region — the outer <button> can't carry
      role="status" because a button is already an interactive
      landmark. Wrapping the gauge chips (only rendered when there
      is work in flight) preserves the SR announcements the
      pre-button banner had.
    -->
    <span class="jobs-ticker-gauges" role="status" aria-live="polite">
      {#each dispatchCatalog.activeKindGauges as g (g.kind)}
        <span class="jobs-ticker-chip">
          <span class="jobs-ticker-kind">{g.kind}</span>
          <span class="jobs-ticker-gauge">
            {g.done.toLocaleString()} / {g.total.toLocaleString()}
          </span>
          {#if g.pending > 0}
            <span class="jobs-ticker-sub">pending {g.pending.toLocaleString()}</span>
          {/if}
          {#if g.running > 0}
            <span class="jobs-ticker-sub">running {g.running.toLocaleString()}</span>
          {/if}
          {#if g.failed > 0}
            <span class="jobs-ticker-sub jobs-ticker-failed">failed {g.failed.toLocaleString()}</span>
          {/if}
        </span>
      {/each}
    </span>
  {/if}
</button>

<style>
  /* Muted colours + a soft pulsing dot so the banner is
     discoverable without stealing focus from the grid. Chips
     share the sidebar's tag chip treatment. Copied verbatim
     from App.svelte in wave C — same duplication policy as the
     other extracted sections. */

  .jobs-ticker {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0 0.55rem;
    padding: 0.25rem 0.35rem;
    background: #f5f4fb;
    border: 1px solid #ecebfa;
    border-radius: 6px;
    color: #7a76c9;
    font-size: 0.72rem;
    flex-wrap: wrap;
    width: 100%;
    text-align: left;
    font-family: inherit;
    cursor: pointer;
  }
  .jobs-ticker:hover {
    background: #ecebfa;
    border-color: #d9d5f2;
  }

  .jobs-ticker-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #7a76c9;
    animation: jobs-ticker-pulse 1.4s ease-in-out infinite;
  }
  /* Idle state: no pulse — the chip is a static entry point when the
     queue is drained. The dot stays as a visual cue for
     consistency. */
  .jobs-ticker-dot.jobs-ticker-dot-idle {
    animation: none;
    opacity: 0.5;
  }

  @keyframes jobs-ticker-pulse {
    0%, 100% { opacity: 0.4; }
    50% { opacity: 1; }
  }

  .jobs-ticker-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    background: #fff;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    padding: 0.05rem 0.4rem;
  }

  .jobs-ticker-kind {
    font-weight: 500;
  }

  .jobs-ticker-gauge {
    font-weight: 600;
    color: #4d488a;
    padding-right: 0.15rem;
  }

  .jobs-ticker-sub {
    color: #888;
    font-size: 0.68rem;
    padding: 0 0.15rem;
  }

  .jobs-ticker-failed {
    color: #b04a4a;
  }
</style>
