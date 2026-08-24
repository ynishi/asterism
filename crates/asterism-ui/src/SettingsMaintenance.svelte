<script lang="ts">
  // Maintenance — the library-wide repair verbs (#136). Each button is
  // a thin caller of one Tauri command; three of the four enqueue jobs
  // and return task ids, and their progress reaches the jobs ticker
  // with no wiring here. `organize_by_location` is the exception: it runs
  // synchronously and answers with a summary, so the button stays busy
  // for the whole pass — a multi-minute wait on a large library, which
  // the hint says out loud.
  import type {
    OrganizeByLocationCommand,
    OrganizeByLocationResult,
  } from "./bindings";
  import { api } from "./lib/api";

  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  // `unmeasured` is the default scope because it is the "the situation
  // changed" answer — it fills blanks and replaces nothing. `all` is
  // the only scope that overwrites, and picking it is deliberate.
  let dimsScope = $state("unmeasured");
  let organizeBaseDir = $state("");

  function errMsg(e: unknown): string {
    if (typeof e === "string") return e;
    if (e && typeof e === "object" && "message" in e) {
      return String((e as { message: unknown }).message);
    }
    return String(e);
  }

  // One gate for the whole section: these are library-wide passes, and
  // two of them racing (a remeasure under a rescan) is a load nobody
  // asked for. Serialising here does not serialise the queue — enqueued
  // jobs still run on their own — it only stops this panel stacking
  // requests.
  async function run(fn: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    error = null;
    notice = null;
    try {
      await fn();
    } catch (e) {
      error = errMsg(e);
    } finally {
      busy = false;
    }
  }

  async function rebuildIndex(): Promise<void> {
    await run(async () => {
      await api<string>("rebuild_index");
      notice = "Index rebuild enqueued — progress shows in the jobs ticker.";
    });
  }

  async function rescanDuplicates(): Promise<void> {
    await run(async () => {
      await api<string>("rescan_duplicates");
      notice = "Duplicate rescan enqueued — new conflicts land in the queue.";
    });
  }

  async function remeasureDims(): Promise<void> {
    await run(async () => {
      await api<string[]>("remeasure_dims", {
        assetIds: [],
        scope: dimsScope,
      });
      notice = `Dimension remeasure (${dimsScope}) enqueued.`;
    });
  }

  async function organizeByLocation(): Promise<void> {
    await run(async () => {
      const base = organizeBaseDir.trim();
      const command: OrganizeByLocationCommand = {
        persona_id: null,
        base_dir: base === "" ? null : base,
      };
      const result = await api<OrganizeByLocationResult>(
        "organize_by_location",
        { command },
      );
      notice =
        `Organized ${result.assets_organized} assets into ` +
        `${result.groups_created} groups under ${result.dirs_created} dirs ` +
        `(${result.skipped} skipped).`;
    });
  }
</script>

<h4>Maintenance</h4>

{#if error}
  <p class="maint-error">{error}</p>
{/if}
{#if notice}
  <p class="maint-notice">{notice}</p>
{/if}

<ul class="maint-list">
  <li class="maint-row">
    <div class="maint-main">
      <span class="maint-label">Search index</span>
      <span class="maint-hint">
        Re-index every asset body into full-text search. Safe to re-run;
        rows indexed by the current reading are skipped.
      </span>
    </div>
    <button
      class="maint-action"
      disabled={busy}
      title="Enqueue a batch index rebuild job"
      onclick={rebuildIndex}
    >
      Rebuild
    </button>
  </li>
  <li class="maint-row">
    <div class="maint-main">
      <span class="maint-label">Duplicates</span>
      <span class="maint-hint">
        Re-derive duplicate conflicts from stored fingerprints. Detection
        only — nothing is merged.
      </span>
    </div>
    <button
      class="maint-action"
      disabled={busy}
      title="Enqueue a whole-library duplicate rescan job"
      onclick={rescanDuplicates}
    >
      Rescan
    </button>
  </li>
  <li class="maint-row">
    <div class="maint-main">
      <span class="maint-label">Dimensions</span>
      <span class="maint-hint">
        Re-read artefacts and rewrite width / height. Only the “all” scope
        replaces existing measurements.
      </span>
    </div>
    <select
      class="maint-scope"
      bind:value={dimsScope}
      disabled={busy}
      title="unlooked / unmeasured fill blanks; all overwrites"
    >
      <option value="unlooked">unlooked</option>
      <option value="unmeasured">unmeasured</option>
      <option value="all">all</option>
    </select>
    <button
      class="maint-action"
      disabled={busy}
      title="Enqueue a dimension remeasure job for the chosen scope"
      onclick={remeasureDims}
    >
      Remeasure
    </button>
  </li>
  <li class="maint-row">
    <div class="maint-main">
      <span class="maint-label">Organize by location</span>
      <span class="maint-hint">
        Backfill: file existing assets under a Dir tree derived from where
        they came from. Runs synchronously — a large library takes minutes.
      </span>
    </div>
    <input
      class="maint-base"
      type="text"
      bind:value={organizeBaseDir}
      disabled={busy}
      placeholder="base dir (optional)"
      title="Path prefix stripped from source locators; assets outside it are ignored"
    />
    <button
      class="maint-action"
      disabled={busy}
      title="Run the backfill now and report the summary"
      onclick={organizeByLocation}
    >
      Organize
    </button>
  </li>
</ul>

<style>
  .maint-list {
    list-style: none;
    margin: 0 0 1rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
  }
  .maint-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .maint-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }
  .maint-label {
    font-size: 0.82rem;
    color: #334;
  }
  .maint-hint {
    font-size: 0.75rem;
    font-style: italic;
    color: #999;
  }
  .maint-action {
    flex: none;
    font-size: 0.78rem;
    padding: 0.25rem 0.7rem;
    cursor: pointer;
  }
  .maint-action:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .maint-scope {
    flex: none;
    font-size: 0.78rem;
  }
  .maint-base {
    flex: none;
    width: 11rem;
    font-size: 0.78rem;
    padding: 0.2rem 0.4rem;
  }
  .maint-error {
    color: var(--danger, #e2665b);
    font-size: 0.78rem;
  }
  .maint-notice {
    color: #567;
    font-size: 0.78rem;
  }
</style>
