<script lang="ts">
  // SnapshotView — the shared component behind every "opened
  // snapshot" affordance: a dispatch-history row click, a Group
  // detail's "promoted from N assets" chip, whatever else W6+ wires
  // in. Snapshots are content objects (no list / rename /
  // delete surface); this panel is the only way to look inside one.
  //
  // Opens whenever `dispatchCatalog.snapshotOpenId` is non-null.
  // Loads the snapshot metadata + member cards in parallel; the
  // caller (App) mints the two `$effect` reloads.
  //
  // Actions:
  //   - Re-select: rehydrate the frozen ids into the grid
  //     multi-select (P2 first half) via `gridSelection.restore`.
  //   - Re-dispatch: invoke `redispatch` with this snapshot's most
  //     recent dispatch id (P2). The command replays the
  //     same exporter / action / params / frozen input against the
  //     shared snapshot row (content_hash dedupe). Enabled only
  //     when a prior dispatch exists.
  //   - Dispatch… (W6-b): orphan snapshots (no prior job) get a
  //     fresh-dispatch fold-out instead — exporter picker + action +
  //     params JSON → `create_dispatch`.
  //   - Group ify (promote): mint a hand-owned Group from the
  //     frozen membership via `promote_snapshot_to_group`.
  //     Prompts for the group name inline.

  import { invoke } from "@tauri-apps/api/core";
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
  import { gridSelection } from "./lib/stores/grid-selection.svelte";
  import { settingsCatalog } from "./lib/stores/settings.svelte";
  import type {
    AssetCardDto,
    DispatchDto,
    PromoteSnapshotToGroupResult,
    SnapshotDto,
  } from "./bindings";

  interface Props {
    // "Promote" wraps `customPrompt` + a name; App owns the modal
    // shell so components stay prompt-free.
    onPromptName: (title: string, placeholder: string) => Promise<string | null>;
    // Toast surface (invoke failure / promote result).
    onFlash: (msg: string, ms?: number) => void;
    // Sidebar count refresh after a promote materialises a Group.
    onLoadGroupCounts: () => void;
  }

  let { onPromptName, onFlash, onLoadGroupCounts }: Props = $props();

  let snapshot = $state<SnapshotDto | null>(null);
  let members = $state<AssetCardDto[]>([]);
  let originJob = $state<DispatchDto | null>(null); // for the [Re-dispatch] action
  let loading = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  // Fresh-dispatch form (W6-b): an orphan snapshot (promoted but never
  // dispatched) has no prior job to replay, so Re-dispatch is dead.
  // The fold-out offers a first dispatch via `create_dispatch` —
  // exporter picker + action + params. Only rendered while
  // `originJob === null` (P2 stays replay-only when a prior
  // run exists).
  let dispatchFormOpen = $state(false);
  let exporterOptions = $state<string[]>([]);
  let dispatchExporter = $state("");
  let dispatchAction = $state("");
  let dispatchParams = $state("");
  // Generation guard — the id alone cannot tell an
  // `open(A) → close → open(A)` sequence from a single open, so a
  // stale first-load could overwrite the fresh second-load. Every
  // effect run bumps this and each async resolution checks it,
  // matching what `Resource` does internally.
  let openGen = 0;

  // Reload whenever the open id flips. Nested `$effect` (not
  // `$derived.by`) because the state writes need to be sequential
  // and error / loading flags reset per open.
  $effect(() => {
    const id = dispatchCatalog.snapshotOpenId;
    const gen = ++openGen;
    if (id === null) {
      snapshot = null;
      members = [];
      originJob = null;
      loading = false;
      error = null;
      dispatchFormOpen = false;
      // Drop the drafted action / params — carrying them across
      // snapshots invites dispatching snapshot B with snapshot A's
      // half-typed intent. The exporter pick (static slug) stays.
      dispatchAction = "";
      dispatchParams = "";
      return;
    }
    loading = true;
    error = null;
    snapshot = null;
    members = [];
    originJob = null;
    dispatchFormOpen = false;
    dispatchAction = "";
    dispatchParams = "";
    (async () => {
      try {
        // Two round-trips in parallel — metadata + members. Cards are
        // returned in frozen `position` order (snapshot_members).
        const [snap, cards, jobs] = await Promise.all([
          invoke<SnapshotDto>("get_snapshot", { id }),
          invoke<AssetCardDto[]>("snapshot_members", { id }),
          // list_dispatch by snapshot_id — pick the latest job that
          // references this freeze, so Re-dispatch has an id to
          // replay.
          invoke<DispatchDto[]>("list_dispatch", {
            personaId: null,
            snapshotId: id,
            stateSlug: null,
            limit: 8,
          }),
        ]);
        if (gen !== openGen) return; // superseded by a newer open (incl. same-id re-open)
        snapshot = snap;
        members = cards;
        originJob = jobs[0] ?? null;
      } catch (e) {
        if (gen === openGen) error = String(e);
      } finally {
        if (gen === openGen) loading = false;
      }
    })();
  });

  function fmtDate(ms: number): string {
    const d = new Date(ms);
    if (!Number.isFinite(d.getTime())) return "";
    return d.toISOString().slice(5, 16).replace("T", " ");
  }

  function reselect() {
    if (!snapshot) return;
    // Delegate to `gridSelection.restore` so any future step added
    // to the restore pipeline (search clear on persona flip, etc.)
    // applies here without drift.
    gridSelection.restore(snapshot);
    dispatchCatalog.closeSnapshot();
  }

  async function redispatch() {
    if (!originJob || busy) return;
    busy = true;
    try {
      const dto = await invoke<DispatchDto>("redispatch", {
        command: { dispatch_id: originJob.id },
      });
      dispatchCatalog.beginDispatch(
        dto.id,
        `Re-dispatching · ${dto.id.slice(0, 8)}`,
      );
      void dispatchCatalog.pollDispatch(dto.id);
      dispatchCatalog.closeSnapshot();
    } catch (e) {
      onFlash(`Re-dispatch failed: ${String(e)}`, 6000);
    } finally {
      busy = false;
    }
  }

  async function toggleDispatchForm() {
    dispatchFormOpen = !dispatchFormOpen;
    if (dispatchFormOpen && exporterOptions.length === 0) {
      try {
        exporterOptions = await invoke<string[]>("list_exporters");
        if (dispatchExporter === "" && exporterOptions.length > 0) {
          dispatchExporter = exporterOptions[0];
        }
      } catch {
        exporterOptions = [];
      }
    }
    if (dispatchFormOpen) prefillParamsFor();
  }

  // Text of the skeleton this component last generated. Used to tell
  // "untouched prefill" from "the user's draft", so switching exporters
  // can retract a skeleton without ever discarding typed input.
  let lastPrefill = "";

  // Seeds the params textarea with the exporter's known defaults so the
  // user edits a skeleton instead of recalling the schema.
  //
  // Two rules keep this from eating input: it only fills a box that is
  // empty, and it only clears a box still byte-identical to the
  // skeleton it produced. Anything the user typed is left alone —
  // including when they switch exporters, where the alternative
  // (leaving a comfy payload behind) would silently submit it as the
  // next exporter's params.
  //
  // Comfy is the one exporter with a configured default today
  // (`dispatch.comfy.endpoint`). The other fields are left as obvious
  // placeholders rather than invented values — `workflow` in particular
  // is a whole ComfyUI graph that only the user has, so an unedited
  // skeleton is expected to fail at the backend, not to run.
  function prefillParamsFor(): void {
    if (dispatchExporter.trim() !== "comfy") {
      if (dispatchParams === lastPrefill) {
        dispatchParams = "";
        lastPrefill = "";
      }
      return;
    }
    if (dispatchParams.trim().length > 0) return;
    const endpoint = settingsCatalog.text(
      "dispatch.comfy.endpoint",
      "http://127.0.0.1:8188",
    );
    lastPrefill = JSON.stringify(
      { endpoint, workflow: {}, input_slot: "" },
      null,
      2,
    );
    dispatchParams = lastPrefill;
  }

  async function dispatchFresh() {
    if (!snapshot || busy) return;
    const exporter = dispatchExporter.trim();
    const action = dispatchAction.trim();
    if (exporter.length === 0 || action.length === 0) {
      onFlash("Pick an exporter and an action first");
      return;
    }
    busy = true;
    try {
      const dto = await invoke<DispatchDto>("create_dispatch", {
        command: {
          snapshot_id: snapshot.id,
          exporter_slug: exporter,
          action,
          // Empty string = `{}` server-side; invalid JSON surfaces as
          // a Validation error via the flash below.
          params_json: dispatchParams.trim(),
        },
      });
      dispatchCatalog.beginDispatch(
        dto.id,
        `Dispatching · ${dto.id.slice(0, 8)}`,
      );
      void dispatchCatalog.pollDispatch(dto.id);
      dispatchCatalog.closeSnapshot();
    } catch (e) {
      onFlash(`Dispatch failed: ${String(e)}`, 6000);
    } finally {
      busy = false;
    }
  }

  async function promote() {
    if (!snapshot || busy) return;
    const name = await onPromptName(
      "Promote snapshot to Group",
      "unique per persona",
    );
    if (!name || !name.trim()) return;
    busy = true;
    try {
      const result = await invoke<PromoteSnapshotToGroupResult>(
        "promote_snapshot_to_group",
        {
          command: {
            snapshot_id: snapshot.id,
            name: name.trim(),
            description: null,
            dir_id: null,
          },
        },
      );
      onFlash(
        `Promoted · Group “${result.name}” · ${result.asset_count} asset(s)`,
      );
      onLoadGroupCounts();
      dispatchCatalog.closeSnapshot();
    } catch (e) {
      onFlash(`Promote failed: ${String(e)}`, 6000);
    } finally {
      busy = false;
    }
  }
</script>

<!--
  Snapshot view — a fixed overlay so it does not fight the sidebar /
  grid layout. Zero-prop for state; the caller only threads modal +
  toast primitives that live App-side per design.
-->
{#if dispatchCatalog.snapshotOpenId !== null}
  <div
    class="snap-backdrop"
    onclick={() => dispatchCatalog.closeSnapshot()}
    role="button"
    tabindex="-1"
    aria-label="Close snapshot"
  >
    <div
      class="snap-panel"
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-label="Snapshot"
    >
      <header class="snap-head">
        <div class="snap-title">
          Snapshot
          {#if snapshot}
            · <code>{snapshot.id.slice(0, 8)}</code>
          {/if}
        </div>
        <button
          class="snap-close"
          onclick={() => dispatchCatalog.closeSnapshot()}
          aria-label="Close"
        >✕</button>
      </header>

      {#if loading}
        <p class="snap-empty">loading…</p>
      {:else if error}
        <p class="snap-empty snap-error">Load failed: {error}</p>
      {:else if snapshot}
        <div class="snap-meta">
          <span class="snap-meta-item">
            frozen {members.length} of {snapshot.asset_ids.length}
            {#if members.length !== snapshot.asset_ids.length}
              <span class="snap-hint">
                · {snapshot.asset_ids.length - members.length} member(s) no longer resolvable
              </span>
            {/if}
          </span>
          <span class="snap-meta-item">
            at {fmtDate(snapshot.created_at_ms)}
          </span>
          {#if originJob}
            <span class="snap-meta-item">
              via {originJob.exporter_slug} · {originJob.action}
            </span>
            <!-- Which agent asked for that run. Only when asserted:
                 unrecorded is not "a human did it", so nothing is
                 rendered in its place. -->
            {#if originJob.operator_ai}
              <span class="snap-meta-item">
                operator {originJob.operator_ai}
              </span>
            {/if}
          {/if}
        </div>

        <div class="snap-actions" role="group" aria-label="Snapshot actions">
          <button
            type="button"
            class="snap-btn"
            onclick={reselect}
            disabled={busy || members.length === 0}
            title="Rehydrate this freeze back into the grid multi-select"
          >Re-select</button>
          <button
            type="button"
            class="snap-btn"
            onclick={redispatch}
            disabled={busy || originJob === null}
            title={originJob
              ? `Re-run ${originJob.exporter_slug} · ${originJob.action} against this freeze`
              : "No prior dispatch to replay"}
          >Re-dispatch</button>
          {#if originJob === null}
            <button
              type="button"
              class="snap-btn"
              onclick={toggleDispatchForm}
              disabled={busy || members.length === 0}
              title="No prior run to replay — dispatch this freeze through an exporter (W6-b)"
            >{dispatchFormOpen ? "▾" : "▸"} Dispatch…</button>
          {/if}
          <button
            type="button"
            class="snap-btn snap-btn-primary"
            onclick={promote}
            disabled={busy || members.length === 0}
            title="Materialise a hand-owned Group from this freeze's members"
          >Group-ify</button>
        </div>

        {#if dispatchFormOpen && originJob === null}
          <div class="snap-dispatch-form" role="group" aria-label="New dispatch">
            {#if exporterOptions.length === 0}
              <p class="snap-hint">No exporters registered in this build.</p>
            {:else}
              <div class="snap-dispatch-row">
                <select
                  class="snap-dispatch-select"
                  bind:value={dispatchExporter}
                  onchange={prefillParamsFor}
                >
                  {#each exporterOptions as slug (slug)}
                    <option value={slug}>{slug}</option>
                  {/each}
                </select>
                <input
                  class="snap-dispatch-action"
                  type="text"
                  placeholder="action, e.g. write"
                  bind:value={dispatchAction}
                />
                <button
                  type="button"
                  class="snap-btn snap-btn-primary"
                  onclick={dispatchFresh}
                  disabled={busy || dispatchExporter.trim().length === 0 || dispatchAction.trim().length === 0}
                >Run</button>
              </div>
              <textarea
                class="snap-dispatch-params"
                rows="2"
                placeholder={'params JSON — empty = {}'}
                bind:value={dispatchParams}
              ></textarea>
            {/if}
          </div>
        {/if}

        {#if members.length === 0}
          <p class="snap-empty">
            No live members — every frozen asset has been deleted.
          </p>
        {:else}
          <ul class="snap-members" role="list">
            {#each members as card (card.id)}
              <li class="snap-member">
                <span class="snap-modality">{card.modality}</span>
                <span class="snap-cover">
                  {card.cover ?? card.source_locator.split("/").pop() ?? card.id.slice(0, 8)}
                </span>
                <span class="snap-time">{fmtDate(card.occurred_at_ms)}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .snap-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(23, 22, 42, 0.45);
    z-index: 65;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .snap-panel {
    width: min(46rem, 96vw);
    max-height: 84vh;
    background: #fbfbff;
    border-radius: 8px;
    box-shadow: 0 12px 32px rgba(23, 22, 42, 0.35);
    display: flex;
    flex-direction: column;
    font-family: inherit;
  }
  .snap-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.7rem 1rem 0.5rem;
    border-bottom: 1px solid #e6e5f0;
  }
  .snap-title {
    font-size: 0.95rem;
    color: #2f2b5a;
  }
  .snap-title code {
    font-family: monospace;
    color: #7a76c9;
    background: #f0effc;
    padding: 0.05rem 0.35rem;
    border-radius: 4px;
  }
  .snap-close {
    background: transparent;
    border: none;
    color: #7a76c9;
    font-size: 0.95rem;
    cursor: pointer;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
  }
  .snap-close:hover {
    background: #ecebfa;
  }
  .snap-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    padding: 0.5rem 1rem;
    color: #6b6795;
    font-size: 0.78rem;
    border-bottom: 1px solid #efedfa;
  }
  .snap-hint {
    color: #b05656;
  }
  .snap-actions {
    display: flex;
    gap: 0.4rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid #efedfa;
  }
  .snap-btn {
    padding: 0.28rem 0.75rem;
    font-size: 0.78rem;
    font-family: inherit;
    color: #4a4770;
    background: #f0effc;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    cursor: pointer;
  }
  .snap-btn:hover:not(:disabled) {
    background: #e2ddf9;
    color: #2f2b5a;
  }
  .snap-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .snap-btn-primary {
    background: #7a76c9;
    color: #fff;
    border-color: #7a76c9;
  }
  .snap-btn-primary:hover:not(:disabled) {
    background: #5f5abd;
    color: #fff;
  }
  /* Fresh-dispatch fold-out (W6-b) — compact exporter / action /
     params row under the action strip; only rendered for orphan
     snapshots (no prior job to replay). */
  .snap-dispatch-form {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.5rem 1rem 0.6rem;
    border-bottom: 1px solid #efedfa;
    background: #f6f5fd;
  }
  .snap-dispatch-row {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  .snap-dispatch-select,
  .snap-dispatch-action {
    padding: 0.28rem 0.5rem;
    font-size: 0.78rem;
    font-family: inherit;
    color: #2f2b5a;
    background: #fff;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
  }
  .snap-dispatch-action {
    flex: 1;
    min-width: 0;
  }
  .snap-dispatch-params {
    width: 100%;
    resize: vertical;
    padding: 0.35rem 0.5rem;
    font-size: 0.75rem;
    font-family: monospace;
    color: #2f2b5a;
    background: #fff;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    box-sizing: border-box;
  }
  .snap-dispatch-action:focus,
  .snap-dispatch-params:focus {
    outline: none;
    border-color: #7a76c9;
  }
  .snap-empty {
    padding: 1rem;
    color: #8a87ab;
    font-size: 0.8rem;
  }
  .snap-error {
    color: #b05656;
  }
  .snap-members {
    list-style: none;
    padding: 0;
    margin: 0;
    overflow-y: auto;
    max-height: 50vh;
  }
  .snap-member {
    display: grid;
    grid-template-columns: 5rem 1fr auto;
    gap: 0.5rem;
    align-items: baseline;
    padding: 0.35rem 1rem;
    border-bottom: 1px solid #f4f2ff;
    font-size: 0.78rem;
    color: #2f2b5a;
  }
  .snap-modality {
    color: #7a76c9;
    font-variant: small-caps;
    letter-spacing: 0.03em;
  }
  .snap-cover {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .snap-time {
    color: #9a97b0;
    font-size: 0.72rem;
    font-variant-numeric: tabular-nums;
  }
</style>
