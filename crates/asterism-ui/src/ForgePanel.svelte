<script lang="ts">
  // ForgePanel — this machine's lines, and how each got there.
  //
  // The frame #170's four surfaces attach to. Why it is shaped this way
  // — the ratio it follows, what the model forbids, where each of those
  // surfaces lands on it — is in `lib/stores/forge.svelte.ts`, beside
  // the reads it makes. This header says only what this component adds.
  //
  // 0-prop: it reads `forgeCatalog` directly and takes nothing from
  // whoever mounts it. Nobody mounts it yet — where the forge sits in
  // the app is one of the two questions the store's doc leaves open,
  // and putting it somewhere would answer that by implementation rather
  // than by decision. It loads its own list on mount, so wherever it
  // lands it works without an orchestrating effect beside it.
  //
  // Two tabs rather than two panels. Contents and history are answers
  // about one line from one place, and a person moving between them is
  // changing the question rather than the subject — so the line stays
  // named in the header while the body below it swaps.
  //
  // Nothing here writes. `[open a pursuit]` is disabled and says why:
  // a button that looks live and does nothing is worse than one that
  // states what it is waiting for.
  import { forgeCatalog } from "./lib/stores/forge.svelte";

  let tab = $state<"contents" | "history">("contents");
  let showOffTheLine = $state(false);

  $effect(() => {
    void forgeCatalog.lines.load();
  });

  const open = $derived(
    forgeCatalog.lines.data.filter((line) => line.standing === "open"),
  );
  const archived = $derived(
    forgeCatalog.lines.data.filter((line) => line.standing !== "open"),
  );
  const current = $derived(
    forgeCatalog.lines.data.find((line) => line.id === forgeCatalog.selected) ??
      null,
  );

  async function select(lineId: string) {
    forgeCatalog.selected = lineId;
    showOffTheLine = false;
    // The chain goes with the line it belongs to. Without this, opening
    // the history tab after switching lines renders the previous line's
    // chain under this one's name — the store's header calls that out
    // for writes, and a selection reaches it the same way.
    forgeCatalog.history.reset();
    await forgeCatalog.states.load({ lineId });
    if (tab === "history") await forgeCatalog.history.load({ lineId });
  }

  // The chain loads on demand rather than beside the contents: it
  // answers a question asked after the work rather than during it.
  async function toHistory() {
    tab = "history";
    if (forgeCatalog.selected !== null && forgeCatalog.history.data === null) {
      await forgeCatalog.history.load({ lineId: forgeCatalog.selected });
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }
</script>

<section class="forge" aria-label="Forge">
  <nav class="lines" aria-label="Lines">
    <h3>Lines</h3>
    {#if forgeCatalog.lines.loading}
      <p class="quiet">Reading…</p>
    {:else if forgeCatalog.lines.error !== null}
      <p class="quiet">{forgeCatalog.lines.error}</p>
    {:else if forgeCatalog.lines.data.length === 0}
      <p class="quiet">No line on this machine yet.</p>
    {/if}

    <ul>
      {#each open as line (line.id)}
        <li>
          <button
            class:selected={line.id === forgeCatalog.selected}
            onclick={() => select(line.id)}
          >{line.name}</button>
        </li>
      {/each}
    </ul>

    {#if archived.length > 0}
      <!-- A section rather than a filter: a discard is only reachable
           from an archived line, so hiding these would hide exactly the
           ones somebody is about to drop. -->
      <h4>Archived</h4>
      <ul>
        {#each archived as line (line.id)}
          <li>
            <button
              class:selected={line.id === forgeCatalog.selected}
              onclick={() => select(line.id)}
            >{line.name}</button>
          </li>
        {/each}
      </ul>
    {/if}
  </nav>

  <div class="line">
    {#if current === null}
      <p class="quiet">Select a line.</p>
    {:else}
      <header>
        <h3>{current.name}</h3>
        <span class="quiet">{current.standing} · {current.strategy_id}</span>
        <button type="button" disabled title="Opening work is #170's second child">
          open a pursuit
        </button>
      </header>

      <div class="tabs" role="tablist">
        <button
          role="tab"
          aria-selected={tab === "contents"}
          onclick={() => (tab = "contents")}
        >on the line</button>
        <button role="tab" aria-selected={tab === "history"} onclick={toHistory}>
          history
        </button>
      </div>

      {#if tab === "contents"}
        {#if forgeCatalog.states.loading}
          <p class="quiet">Reading…</p>
        {:else}
          <ul class="entries">
            {#each forgeCatalog.onTheLine as entry (entry.entry_id)}
              <li>{entry.name ?? "(unnamed)"}</li>
            {/each}
          </ul>
          <p class="quiet">{forgeCatalog.onTheLine.length} on the line</p>

          {#if forgeCatalog.offTheLine.length > 0}
            <!-- Findable, and drawn so it cannot be read as contents.
                 "Off the line" rather than "taken off": the wire says
                 only that the line does not hold these now, and an
                 entry named without ever being added reads the same as
                 one a change point removed. -->
            <button
              class="disclose"
              onclick={() => (showOffTheLine = !showOffTheLine)}
            >
              {forgeCatalog.offTheLine.length} off the line
              {showOffTheLine ? "▾" : "▸"}
            </button>
            {#if showOffTheLine}
              <ul class="entries gone">
                {#each forgeCatalog.offTheLine as entry (entry.entry_id)}
                  <li>{entry.name ?? "(unnamed)"}</li>
                {/each}
              </ul>
            {/if}
          {/if}
        {/if}
      {:else if forgeCatalog.history.loading}
        <p class="quiet">Reading…</p>
      {:else if forgeCatalog.history.data === null}
        <p class="quiet">No history read yet.</p>
      {:else}
        <ol class="chain">
          {#each [...forgeCatalog.history.data.changes].reverse() as point (point.id)}
            <li>
              <span class="rows">{point.table.length} rows</span>
              <span>{point.actor_id}</span>
              <span class="quiet">{when(point.at_ms)}</span>
            </li>
          {/each}
        </ol>
        <!-- The genesis is not a change point, and the model keeps the
             two apart. Folding it into the chain would claim something
             the record does not. -->
        <p class="quiet genesis">
          genesis · {when(forgeCatalog.history.data.genesis_at_ms)}
        </p>
      {/if}
    {/if}
  </div>
</section>

<style>
  .forge {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
  }
  .lines {
    flex: 0 0 12rem;
  }
  .line {
    flex: 1 1 auto;
    min-width: 0;
  }
  h3,
  h4 {
    margin: 0 0 0.4rem;
    font-size: 0.9rem;
  }
  h4 {
    margin-top: 0.9rem;
    font-weight: 500;
  }
  ul,
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .lines button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    padding: 0.2rem 0;
    text-align: left;
    width: 100%;
  }
  .lines button.selected {
    font-weight: 600;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
  }
  header button {
    margin-left: auto;
  }
  .tabs {
    display: flex;
    gap: 0.8rem;
    margin: 0.6rem 0;
    border-bottom: 1px solid rgba(128, 128, 128, 0.3);
  }
  .tabs button {
    background: none;
    border: 0;
    border-bottom: 2px solid transparent;
    color: inherit;
    cursor: pointer;
    padding: 0.3rem 0;
  }
  .tabs button[aria-selected="true"] {
    border-bottom-color: currentColor;
  }
  .entries.gone li {
    opacity: 0.55;
    border-left: 2px dashed currentColor;
    padding-left: 0.4rem;
  }
  .chain li {
    display: flex;
    gap: 0.6rem;
    padding: 0.25rem 0;
  }
  .rows {
    min-width: 4.5rem;
  }
  .disclose {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    padding: 0.3rem 0;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.78rem;
    margin: 0.3rem 0;
  }
  .genesis {
    border-top: 1px solid rgba(128, 128, 128, 0.3);
    padding-top: 0.4rem;
  }
</style>
