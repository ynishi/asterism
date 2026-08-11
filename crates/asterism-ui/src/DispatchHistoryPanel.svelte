<script lang="ts">
  // DispatchHistoryPanel — the dispatch-history drawer entry. The
  // bottom status chip grew from a passive `KindGauge` band into a
  // live "dispatch
  // history" entry: click it, the drawer slides in with a filterable
  // list of past `dispatch_job` rows for the active persona, and
  // clicking a row opens the SnapshotView on that job's frozen
  // input.
  //
  // 0-prop by design: the panel reads
  // `dispatchCatalog.history` / `.historyOpen` /
  // `.historyStateFilter` and `activeFilter.activePersona`
  // directly. The App-side `$effect` orchestrates the load
  // (persona flip / drawer open / filter change → catalog.history.load
  // with the current arg tuple).
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { personaName } from "./lib/formatters";

  const STATE_FILTERS: Array<{ slug: string | null; label: string }> = [
    { slug: null, label: "all" },
    { slug: "running", label: "running" },
    { slug: "done", label: "done" },
    { slug: "failed", label: "failed" },
  ];

  function fmtDate(ms: number): string {
    const d = new Date(ms);
    if (!Number.isFinite(d.getTime())) return "";
    // Keep it terse — the drawer is a scanning surface, not a report.
    return d.toISOString().slice(5, 16).replace("T", " ");
  }

  function stateBadge(state: string): string {
    switch (state) {
      case "pending":
        return "◌";
      case "running":
        return "▸";
      case "done":
        return "✓";
      case "failed":
        return "✕";
      case "cancelled":
        return "⊘";
      default:
        return "·";
    }
  }
</script>

{#if dispatchCatalog.historyOpen}
  <!-- Backdrop absorbs outside-click; the drawer itself
       stopPropagation so an interior click never closes. -->
  <div
    class="drawer-backdrop"
    onclick={() => dispatchCatalog.closeHistory()}
    role="button"
    tabindex="-1"
    aria-label="Close dispatch history"
  >
    <aside
      class="drawer"
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-label="Dispatch history"
    >
      <header class="drawer-head">
        <h3>Dispatch history</h3>
        <button
          class="drawer-close"
          onclick={() => dispatchCatalog.closeHistory()}
          aria-label="Close"
        >✕</button>
      </header>

      <div class="drawer-filters" role="tablist">
        {#each STATE_FILTERS as f (f.label)}
          <button
            type="button"
            class="drawer-filter"
            class:active={dispatchCatalog.historyStateFilter === f.slug}
            onclick={() => (dispatchCatalog.historyStateFilter = f.slug)}
            role="tab"
            aria-selected={dispatchCatalog.historyStateFilter === f.slug}
          >{f.label}</button>
        {/each}
      </div>

      {#if activeFilter.activePersona === null}
        <p class="drawer-empty">
          Pick a single persona to see its dispatch history.
        </p>
      {:else if dispatchCatalog.history.loading}
        <p class="drawer-empty">loading…</p>
      {:else if dispatchCatalog.history.error}
        <p class="drawer-empty drawer-error">
          Load failed: {dispatchCatalog.history.error}
        </p>
      {:else if dispatchCatalog.history.data.length === 0}
        <p class="drawer-empty">
          No dispatch jobs for {personaName(activeFilter.activePersona)}
          {dispatchCatalog.historyStateFilter
            ? `in state “${dispatchCatalog.historyStateFilter}”`
            : ""}.
        </p>
      {:else}
        <ul class="drawer-list" role="list">
          {#each dispatchCatalog.history.data as job (job.id)}
            <button
              type="button"
              class="drawer-row"
              class:row-done={job.state === "done"}
              class:row-failed={job.state === "failed" || job.state === "cancelled"}
              class:row-running={job.state === "running"}
              onclick={() => dispatchCatalog.openSnapshot(job.snapshot_id)}
              title="Open the frozen input for this dispatch"
            >
              <span class="row-state" aria-hidden="true">
                {stateBadge(job.state)}
              </span>
              <span class="row-title">
                {job.exporter_slug} · {job.action}
                <!-- Who asked for it. Only when asserted: an absent
                     operator is unrecorded, and a placeholder here
                     would claim the row was driven by hand. -->
                {#if job.operator_ai}
                  <span class="row-operator">· operator {job.operator_ai}</span>
                {/if}
                {#if job.state === "done"}
                  <span class="row-out">→ {job.output_asset_ids.length} asset(s)</span>
                {:else if job.state_message}
                  <span class="row-msg">· {job.state_message}</span>
                {/if}
              </span>
              <span class="row-time">{fmtDate(job.created_at_ms)}</span>
            </button>
          {/each}
        </ul>
      {/if}
    </aside>
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(23, 22, 42, 0.35);
    z-index: 60;
    display: flex;
    justify-content: flex-end;
  }
  .drawer {
    width: min(28rem, 100vw);
    height: 100%;
    background: #fbfbff;
    box-shadow: -6px 0 24px rgba(23, 22, 42, 0.25);
    display: flex;
    flex-direction: column;
    font-family: inherit;
  }
  .drawer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.7rem 1rem 0.5rem;
    border-bottom: 1px solid #e6e5f0;
  }
  .drawer-head h3 {
    margin: 0;
    font-size: 0.95rem;
    color: #2f2b5a;
  }
  .drawer-close {
    background: transparent;
    border: none;
    color: #7a76c9;
    font-size: 0.95rem;
    cursor: pointer;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
  }
  .drawer-close:hover {
    background: #ecebfa;
  }
  .drawer-filters {
    display: flex;
    gap: 0.3rem;
    padding: 0.55rem 1rem;
    border-bottom: 1px solid #efedfa;
  }
  .drawer-filter {
    padding: 0.15rem 0.6rem;
    font-size: 0.72rem;
    font-family: inherit;
    color: #7a76c9;
    background: #f0effc;
    border: 1px solid #d9d5f2;
    border-radius: 999px;
    cursor: pointer;
  }
  .drawer-filter.active {
    background: #7a76c9;
    color: #fff;
    border-color: #7a76c9;
  }
  .drawer-empty {
    padding: 1rem;
    color: #8a87ab;
    font-size: 0.8rem;
  }
  .drawer-error {
    color: #b05656;
  }
  .drawer-list {
    list-style: none;
    padding: 0;
    margin: 0;
    overflow-y: auto;
  }
  .drawer-row {
    display: grid;
    grid-template-columns: 1.2rem 1fr auto;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    text-align: left;
    padding: 0.55rem 1rem;
    border: none;
    border-bottom: 1px solid #efedfa;
    background: transparent;
    color: #2f2b5a;
    font-family: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .drawer-row:hover {
    background: #f5f4ff;
  }
  .drawer-row.row-done .row-state {
    color: #4a8f78;
  }
  .drawer-row.row-failed .row-state {
    color: #b05656;
  }
  .drawer-row.row-running .row-state {
    color: #b57a55;
  }
  .row-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-out {
    color: #4a8f78;
    margin-left: 0.35rem;
  }
  .row-msg {
    color: #8a87ab;
    margin-left: 0.35rem;
  }
  .row-operator {
    color: #8a86b8;
    margin-left: 0.35rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.72rem;
  }
  .row-time {
    color: #9a97b0;
    font-size: 0.72rem;
    font-variant-numeric: tabular-nums;
  }
</style>
