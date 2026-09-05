<script lang="ts">
  // MergeDialog — the screen a person confirms a manual merge on.
  //
  // The act is the irreversible one. "Keep this, trash the rest" in the
  // report next door moves rows to the trash and they come back; this
  // folds them, leaving one row and a marker in place of each of the
  // others, and no button anywhere undoes that. Everything about this
  // dialog is arranged around that difference: a survivor has to be
  // named rather than defaulted, a preview has to come back before
  // confirm is live, and the preview is drawn as counts of what will
  // move rather than as a reassuring summary.
  //
  // # Where the rules live
  //
  // Not here. `mergeDialog` (lib/stores/merge-dialog.svelte.ts) owns the
  // order the two calls happen in — that a commit follows a preview of
  // *this* plan, that changing the keeper throws the preview away, that
  // the rows named are the rows that were on screen when the ruling
  // started. Those are the rules worth being wrong about and this file
  // is DOM, which the test suite (node) cannot reach. What is here is
  // markup and the one call the store deliberately does not make: the
  // reload, which needs the persona the panel knows.
  //
  // # Why `cards` is a prop
  //
  // The store holds ids, because ids are what the command carries and
  // what a test can state a plan in without building a card fixture per
  // row. Drawing needs the rows themselves, and the panel already has
  // them. So the ids stay authoritative — the list below iterates
  // `mergeDialog.members`, not the prop — and the prop is only ever a
  // lookup for a thumbnail and a name. A row the lookup misses is drawn
  // by id rather than dropped: it is in the plan either way, and a
  // dialog that showed four rows while folding five would be lying at
  // the exact moment it must not.
  import type { AssetCardDto } from "./bindings";
  import { fmtBytes } from "./lib/formatters";
  import {
    mergeDialog,
    mergeRowsLine,
    mergeTotalLines,
    mergeWarningNote,
  } from "./lib/stores/merge-dialog.svelte";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";

  interface Props {
    /** The rows the panel drew, for looking the plan's ids up in. */
    cards: AssetCardDto[];
    /**
     * The fold went through and these rows left the live set — the
     * panel reloads and App drops them from the grid selection. Fires
     * only on a committed run: a refusal writes nothing (see the store).
     */
    onCommitted: (foldedIds: string[]) => void;
  }

  let { cards, onCommitted }: Props = $props();

  let byId = $derived(new Map(cards.map((c) => [c.id, c])));
  let rows = $derived(mergeDialog.members.map((id) => ({ id, card: byId.get(id) ?? null })));
  let busy = $derived(mergeDialog.phase === "previewing" || mergeDialog.phase === "committing");

  function label(id: string, card: AssetCardDto | null): string {
    if (card === null) return id;
    const parts = card.source_locator.split("/").filter((p) => p.length > 0);
    return parts.slice(-2).join("/") || card.source_locator;
  }

  async function confirm() {
    const result = await mergeDialog.commit();
    if (result !== null) onCommitted(result.folded_ids);
  }
</script>

<div class="mrg-backdrop" role="presentation"></div>
<section class="mrg-dialog" aria-label="Merge these into one">
  <header>
    <h2>Merge into one</h2>
    <button class="mrg-close" onclick={() => mergeDialog.close()} disabled={busy} aria-label="Cancel merge">
      ✕
    </button>
  </header>

  <p class="mrg-lead">
    One row survives and the rest become markers pointing at it. Their
    tags, comments and group filings move onto the survivor. This one
    does not come back — the trash does not hold a folded row.
  </p>

  <fieldset class="mrg-rows">
    <legend>Which one stays</legend>
    {#each rows as row (row.id)}
      <label class="mrg-row" class:keeper={mergeDialog.keeperId === row.id}>
        <input
          type="radio"
          name="merge-keeper"
          value={row.id}
          checked={mergeDialog.keeperId === row.id}
          disabled={busy}
          onchange={() => mergeDialog.chooseKeeper(row.id)}
        />
        {#if row.card !== null}
          <img src={thumbCatalog.thumbSrc(row.card)} alt={row.card.cover ?? ""} />
        {/if}
        <span class="mrg-row-text">
          <span class="mrg-name" title={row.card?.source_locator ?? row.id}>
            {label(row.id, row.card)}
          </span>
          <span class="mrg-meta">
            {row.card?.file_size_bytes != null ? fmtBytes(row.card.file_size_bytes) : "size unknown"}
          </span>
        </span>
      </label>
    {/each}
  </fieldset>

  {#if mergeDialog.error}
    <p class="mrg-error">{mergeDialog.error}</p>
  {/if}

  {#if mergeDialog.phase === "previewing"}
    <p class="mrg-note">working out what this would do…</p>
  {/if}

  {#if mergeDialog.preview !== null}
    {@const preview = mergeDialog.preview}
    <div class="mrg-preview">
      <p class="mrg-note">{mergeRowsLine(preview)}</p>
      <!--
        Warnings are computed on the dry run and nowhere else (the
        application verb returns none on the commit branch, because by
        then they have been read), so this is the only place they can
        appear — and it is deliberately above the confirm button.
      -->
      {#each preview.warnings as warning (warning.headstone_id)}
        <p class="mrg-warning">{mergeWarningNote(warning.kind)}</p>
      {/each}
      {#if mergeTotalLines(preview.totals).length > 0}
        <ul class="mrg-totals">
          {#each mergeTotalLines(preview.totals) as line (line.label)}
            <li>{line.count} {line.label}</li>
          {/each}
        </ul>
      {:else}
        <p class="mrg-note">Nothing else moves — the rows carry no tags, comments or filings between them.</p>
      {/if}
    </div>
  {/if}

  {#if mergeDialog.phase === "committing"}
    <p class="mrg-note">merging…</p>
  {/if}

  {#if mergeDialog.refusal !== null}
    {@const refusal = mergeDialog.refusal}
    <!--
      A refusal is the backend's answer, not a failure of the call: it
      arrives as a 200 with `committed: false`, and nothing was written.
      Each line names a row and why, which is what somebody needs to
      rule again — so the dialog stays open around it.
    -->
    <div class="mrg-refusal">
      <p class="mrg-error">Nothing was merged. The rows were rejected:</p>
      <ul>
        {#each refusal.refusals as item (item.asset_id)}
          <li>
            <span class="mrg-name">{label(item.asset_id, byId.get(item.asset_id) ?? null)}</span>
            — {item.reason}
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <div class="mrg-actions">
    <button class="mrg-btn" onclick={() => mergeDialog.close()} disabled={busy}>Cancel</button>
    {#if mergeDialog.canCommit}
      <button class="mrg-btn danger" onclick={() => void confirm()} disabled={busy}>
        Merge — this does not come back
      </button>
    {:else}
      <!--
        Preview is the only way through to confirm. The button above
        does not exist until one has come back for the plan on screen,
        which is why picking a different survivor puts this one back.
      -->
      <button class="mrg-btn" onclick={() => void mergeDialog.runPreview()} disabled={!mergeDialog.canPreview}>
        {mergeDialog.keeperId === null ? "Pick the one that stays" : "Show me what this would do"}
      </button>
    {/if}
  </div>
</section>

<style>
  .mrg-backdrop {
    position: fixed;
    inset: 0;
    background: var(--wash-down);
    z-index: 50;
  }

  .mrg-dialog {
    position: fixed;
    top: 8vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(560px, 92vw);
    max-height: 80vh;
    overflow-y: auto;
    background: var(--surface-raised);
    border-radius: 8px;
    box-shadow: 0 14px 44px var(--shadow-color);
    padding: 1rem 1.25rem 1.25rem;
    z-index: 51;
  }

  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  h2 {
    font-size: 1rem;
    margin: 0 0 0.25rem;
  }

  .mrg-close {
    background: none;
    border: none;
    font-size: 0.9rem;
    color: var(--ink-muted);
    cursor: pointer;
  }
  .mrg-close:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .mrg-lead {
    margin: 0 0 0.75rem;
    font-size: 0.8rem;
    color: var(--ink-secondary);
  }

  .mrg-rows {
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 0.5rem 0.6rem;
    margin: 0 0 0.75rem;
    background: var(--surface-raised);
  }

  legend {
    font-size: 0.75rem;
    color: var(--ink);
    padding: 0 0.3rem;
  }

  .mrg-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.3rem 0.2rem;
    border-radius: 4px;
    cursor: pointer;
  }
  .mrg-row:hover {
    background: var(--surface-hover);
  }
  .mrg-row.keeper {
    background: var(--surface-hover);
  }

  .mrg-row img {
    width: 56px;
    height: 40px;
    object-fit: cover;
    border-radius: 3px;
    background: var(--surface-hover);
  }

  .mrg-row-text {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
    font-size: 0.72rem;
    color: var(--ink-secondary);
  }

  .mrg-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mrg-meta {
    color: var(--ink-faint);
  }

  .mrg-preview,
  .mrg-refusal {
    border-top: 1px solid var(--line);
    padding-top: 0.6rem;
    margin-bottom: 0.5rem;
  }

  .mrg-note {
    font-size: 0.78rem;
    color: var(--ink-secondary);
    margin: 0 0 0.4rem;
  }

  .mrg-warning {
    font-size: 0.75rem;
    color: var(--warning-ink);
    margin: 0 0 0.4rem;
  }

  .mrg-error {
    font-size: 0.78rem;
    color: var(--danger-ink);
    margin: 0 0 0.4rem;
  }

  .mrg-totals,
  .mrg-refusal ul {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.75rem;
    color: var(--ink-secondary);
  }

  .mrg-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
    margin-top: 0.75rem;
  }

  .mrg-btn {
    font-family: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--surface-hover);
    cursor: pointer;
  }
  .mrg-btn:hover:enabled {
    background: var(--surface-active);
  }
  .mrg-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .mrg-btn.danger {
    border-color: var(--danger-fill);
    background: var(--danger-surface);
    color: var(--danger-ink);
  }
  .mrg-btn.danger:hover:enabled {
    background: var(--danger-surface);
  }
</style>
