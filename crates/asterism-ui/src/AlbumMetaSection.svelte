<script lang="ts">
  // AlbumMetaSection — the statements somebody made about one asset, and
  // the controls to add, correct or take one back.
  //
  // Owns: the draft row (name / value), the pending flag, and the local
  // error for a refused write. Owns no data — the statements arrive as a
  // prop because they ride in the asset detail the pane already fetched,
  // so a catalog here would be a second copy of something already loaded
  // (the prop budget is spent on the two that cannot be
  // read from a store, plus the callback the owner has to react to).
  //
  // Extracted rather than inlined into DetailPane for the usual
  // reason: the pane is 3 k lines and the roadmap is to shrink it. The
  // edit state below is self-contained, so it does not have to live
  // there — and out here it is reachable by the component test layer.
  //
  // Deliberately not here: the search side. Finding rows *by* a recorded
  // value is a filter over the whole library, so its home is the filter
  // surface, not a per-asset panel.
  import { api } from "./lib/api";
  import { albumMetaKeyProblem, type AlbumMetaStatement } from "./lib/album-meta";
  import { fmtDateTime } from "./lib/formatters";
  import type { AssetDto } from "./bindings";

  let {
    assetId,
    statements,
    onChanged,
  }: {
    assetId: string;
    statements: AlbumMetaStatement[];
    // The verb answers with the whole row, so the owner takes the new
    // asset rather than being told to refetch what it was just handed.
    onChanged: (asset: AssetDto) => void;
  } = $props();

  let draftKey = $state("");
  let draftValue = $state("");
  let pending = $state<string | null>(null);
  let error = $state<string | null>(null);

  // The name is checked before the round trip; the value is not, because
  // the only rule on it is "not blank", which the button already reads.
  let keyProblem = $derived(
    draftKey.trim().length === 0 ? null : albumMetaKeyProblem(draftKey),
  );
  let canSubmit = $derived(
    pending === null &&
      draftKey.trim().length > 0 &&
      draftValue.trim().length > 0 &&
      keyProblem === null,
  );

  async function declare(key: string, value: string | null) {
    pending = key;
    error = null;
    try {
      const asset = await api<AssetDto>("asset_declare_meta", {
        command: {
          asset_id: assetId,
          key,
          // Absent, not empty: the server reads a missing value as the
          // retraction and refuses `""` outright, so the two spellings
          // must not be allowed to collapse on the way out.
          value: value ?? null,
          operator_ai: null,
        },
      });
      onChanged(asset);
      if (value !== null) {
        draftKey = "";
        draftValue = "";
      }
    } catch (e) {
      error = String(e);
      console.warn("[albumMeta] declare failed:", e);
    } finally {
      pending = null;
    }
  }

  function submit(event: Event) {
    event.preventDefault();
    if (!canSubmit) return;
    void declare(draftKey.trim(), draftValue.trim());
  }
</script>

<dt>Stated</dt>
<dd class="album-meta">
  {#if statements.length > 0}
    <ul class="album-meta-list">
      {#each statements as statement (statement.key)}
        <li class="album-meta-row">
          <span class="album-meta-key">{statement.key}</span>
          <span class="album-meta-value">{statement.value}</span>
          <span class="album-meta-provenance">
            <!-- How it arrived, because a value the caller handed over
                 and one somebody typed later are different evidence
                 about the same name. -->
            {#if statement.source}<span class="album-meta-source">{statement.source}</span>{/if}
            {#if statement.operator}<span class="album-meta-operator">via {statement.operator}</span>{/if}
            {#if statement.declaredAtMs !== null}
              <span class="album-meta-when">{fmtDateTime(statement.declaredAtMs)}</span>
            {/if}
          </span>
          <button
            class="album-meta-remove"
            disabled={pending !== null}
            title="Take this statement back"
            aria-label={`Take back ${statement.key}`}
            onclick={() => void declare(statement.key, null)}
          >
            ×
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <div class="album-meta-empty">nothing stated yet</div>
  {/if}

  <form class="album-meta-form" onsubmit={submit}>
    <input
      class="album-meta-input album-meta-input-key"
      type="text"
      placeholder="name"
      aria-label="Statement name"
      bind:value={draftKey}
    />
    <input
      class="album-meta-input album-meta-input-value"
      type="text"
      placeholder="what it says"
      aria-label="Statement value"
      bind:value={draftValue}
    />
    <button type="submit" disabled={!canSubmit}>State</button>
  </form>
  {#if keyProblem}
    <div class="album-meta-problem">{keyProblem}</div>
  {/if}
  {#if error}
    <div class="album-meta-error">{error}</div>
  {/if}
</dd>

<style>
  .album-meta-list {
    list-style: none;
    margin: 0 0 0.4rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .album-meta-row {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    font-size: 0.82rem;
  }
  .album-meta-key {
    font-weight: 600;
    opacity: 0.8;
  }
  .album-meta-value {
    word-break: break-all;
  }
  .album-meta-provenance {
    display: inline-flex;
    gap: 0.35rem;
    margin-left: auto;
    font-size: 0.72rem;
    opacity: 0.55;
    white-space: nowrap;
  }
  .album-meta-remove {
    background: none;
    border: none;
    cursor: pointer;
    opacity: 0.5;
    padding: 0 0.2rem;
  }
  .album-meta-remove:hover:not(:disabled) {
    opacity: 1;
  }
  .album-meta-empty {
    font-size: 0.78rem;
    opacity: 0.5;
    margin-bottom: 0.4rem;
  }
  .album-meta-form {
    display: flex;
    gap: 0.3rem;
  }
  .album-meta-input {
    min-width: 0;
    font-size: 0.8rem;
    padding: 0.15rem 0.3rem;
  }
  .album-meta-input-key {
    flex: 0 1 8rem;
  }
  .album-meta-input-value {
    flex: 1 1 auto;
  }
  .album-meta-problem,
  .album-meta-error {
    font-size: 0.72rem;
    margin-top: 0.25rem;
    opacity: 0.8;
  }
</style>
