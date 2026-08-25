<script lang="ts">
  // SourceTypeRow — one row on the detail pane: what the asset's
  // digital source type rests on, and the control to assert it (#108).
  //
  // Three states, straight off the read: *asserted* (the term plus who
  // said it and when, with Edit and Retract), *evidence* (the term the
  // container establishes, read-only, with a guarded Override…), and
  // *unknown* ("container declares nothing" and an Assert… control).
  // The read keeps a fourth distinction the storage keeps: a container
  // not yet fingerprinted is "not yet read", not "declares nothing",
  // and the row says which.
  //
  // The select is closed over the five IPTC terms — short names shown,
  // URI on the option's title — ordered by expected frequency, because
  // the backend refuses unknown terms anyway and a free-text field
  // would only manufacture refusals. Deliberately not a modal in the
  // export flow: an assertion is a fact about the asset, not about one
  // export (#108's own scoping).
  //
  // Owns its data, unlike AlbumMetaSection next door: the two halves
  // (evidence and assertion) ride a dedicated read the pane's detail
  // fetch does not carry, so the row fetches on the asset it is shown
  // for and refetches after its own writes. The declare verb answers
  // with the whole asset row, which is handed up through `onChanged`
  // so the pane's cached copy of `extra` does not go stale.
  import { api } from "./lib/api";
  import { fmtDateTime } from "./lib/formatters";
  import type { AssetDto, AssetSourceTypeDto } from "./bindings";

  let {
    assetId,
    onChanged,
  }: {
    assetId: string;
    onChanged: (asset: AssetDto) => void;
  } = $props();

  // The five terms the backend accepts, in expected-frequency order
  // (#108). The URI rides the option title so the short name stays the
  // visible spelling; the server stores the URI whichever arrives.
  const TERMS = [
    "digitalCapture",
    "humanEdits",
    "trainedAlgorithmicMedia",
    "compositeWithTrainedAlgorithmicMedia",
    "algorithmicMedia",
  ] as const;
  const TERM_URI_PREFIX = "http://cv.iptc.org/newscodes/digitalsourcetype/";

  let reading = $state<AssetSourceTypeDto | null>(null);
  let editing = $state(false);
  let draft = $state<string>("");
  let pending = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    const id = assetId;
    editing = false;
    error = null;
    reading = null;
    void api<AssetSourceTypeDto>("asset_source_type", { assetId: id }).then(
      (dto) => {
        if (assetId === id) reading = dto;
      },
      (e) => {
        if (assetId === id) error = String(e);
      },
    );
  });

  function beginEdit() {
    draft = reading?.asserted?.source_type ?? reading?.evidence ?? TERMS[0];
    editing = true;
    error = null;
  }

  async function declare(term: string | null) {
    // Captured so a declare resolving after the pane moved on cannot
    // write the old asset's reading into the new row — the same guard
    // the fetch above carries.
    const id = assetId;
    pending = true;
    error = null;
    try {
      const asset = await api<AssetDto>("asset_declare_source_type", {
        command: {
          asset_id: id,
          // Absent, not empty: the server reads a missing term as the
          // retraction and refuses anything it does not define.
          source_type: term,
          operator_ai: null,
        },
      });
      onChanged(asset);
      const dto = await api<AssetSourceTypeDto>("asset_source_type", {
        assetId: id,
      });
      if (assetId === id) {
        editing = false;
        reading = dto;
      }
    } catch (e) {
      error = String(e);
      console.warn("[sourceType] declare failed:", e);
    } finally {
      pending = false;
    }
  }
</script>

<dt>Source type</dt>
<dd class="source-type">
  {#if reading === null && error === null}
    <span class="source-type-quiet">…</span>
  {:else if editing}
    <span class="source-type-edit">
      <select
        class="source-type-select"
        aria-label="Digital source type"
        value={draft}
        onchange={(e) => (draft = e.currentTarget.value)}
        disabled={pending}
      >
        {#each TERMS as term (term)}
          <option value={term} title={TERM_URI_PREFIX + term}>{term}</option>
        {/each}
      </select>
      <button disabled={pending} onclick={() => void declare(draft)}>
        Assert
      </button>
      <button disabled={pending} onclick={() => (editing = false)}>
        Cancel
      </button>
    </span>
  {:else if reading?.asserted}
    <span
      class="source-type-term"
      title={TERM_URI_PREFIX + reading.asserted.source_type}
    >
      {reading.asserted.source_type}
    </span>
    <span class="source-type-provenance">
      asserted{#if reading.asserted.operator}&nbsp;via {reading.asserted.operator}{/if}{#if reading.asserted.declared_at_ms !== null}&nbsp;{fmtDateTime(reading.asserted.declared_at_ms)}{/if}
    </span>
    <span class="source-type-actions">
      <button disabled={pending} onclick={beginEdit}>Edit</button>
      <button
        disabled={pending}
        title="Take the assertion back — the container's own evidence stands again"
        onclick={() => void declare(null)}
      >
        Retract
      </button>
    </span>
  {:else if reading?.evidence}
    <span class="source-type-term" title={TERM_URI_PREFIX + reading.evidence}>
      {reading.evidence}
    </span>
    <span class="source-type-provenance">from the container</span>
    <span class="source-type-actions">
      <button
        disabled={pending}
        title="Assert a different term over the container's — yours to state, yours to answer for"
        onclick={beginEdit}
      >
        Override…
      </button>
    </span>
  {:else}
    <span class="source-type-quiet">
      {#if reading?.evidence_pending}container not yet read{:else}container
        declares nothing{/if}
    </span>
    <span class="source-type-actions">
      <button disabled={pending} onclick={beginEdit}>Assert…</button>
    </span>
  {/if}
  {#if error}
    <div class="source-type-error">{error}</div>
  {/if}
</dd>

<style>
  .source-type {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    flex-wrap: wrap;
    font-size: 0.82rem;
  }
  .source-type-term {
    word-break: break-all;
  }
  .source-type-provenance {
    font-size: 0.72rem;
    opacity: 0.55;
    white-space: nowrap;
  }
  .source-type-quiet {
    font-size: 0.78rem;
    opacity: 0.5;
  }
  .source-type-actions {
    display: inline-flex;
    gap: 0.3rem;
    margin-left: auto;
  }
  .source-type-actions button,
  .source-type-edit button {
    font-size: 0.72rem;
    background: none;
    border: none;
    cursor: pointer;
    opacity: 0.6;
    padding: 0 0.2rem;
  }
  .source-type-actions button:hover:not(:disabled),
  .source-type-edit button:hover:not(:disabled) {
    opacity: 1;
  }
  .source-type-edit {
    display: inline-flex;
    gap: 0.3rem;
    align-items: center;
  }
  .source-type-select {
    min-width: 0;
    font-size: 0.8rem;
    padding: 0.15rem 0.3rem;
  }
  .source-type-error {
    flex-basis: 100%;
    font-size: 0.72rem;
    margin-top: 0.25rem;
    opacity: 0.8;
  }
</style>
