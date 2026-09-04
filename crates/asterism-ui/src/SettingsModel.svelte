<script lang="ts">
  // Model — the tag model section of the settings stack (#130).
  //
  // Two reads, two verbs, and one badge. The encoder half is
  // infrastructure: it ships with the app, there is nothing to choose,
  // and the only honest states are its identity or "no model bound".
  // The head half is what a person manages — train one from your own
  // rulings, or install the one your team published.
  //
  // Both verbs enqueue a job and answer with a task id, and the thing
  // worth reading is the *verdict* the job reaches: promoted or not,
  // installed or refused. That sentence is built in the handler and
  // arrives on `job:progress:{task_id}`, so the panel subscribes to
  // the id it was handed and shows what it gets, verbatim. Rewording
  // it here would put a second author on a message whose whole value
  // is being the backend's own words — a refusal in particular, which
  // names the encoder mismatch or the shape that failed.
  //
  // The restart badge is server-derived (`restart_required`), not a
  // flag this component sets when a train wins: the head is bound once
  // at startup, so what asks for a relaunch is what would bind next
  // launch differing from what is bound now, and that stays true across
  // a reopened panel and a reloaded webview.
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { HeadStatusDto, VisualModelStatusDto } from "./bindings";
  import { api } from "./lib/api";

  let encoder = $state<VisualModelStatusDto | null>(null);
  let head = $state<HeadStatusDto | null>(null);
  let loadError = $state<string | null>(null);

  let busy = $state(false);
  let error = $state<string | null>(null);
  // The last thing a job said about itself, as it said it.
  let verdict = $state<string | null>(null);

  let artifactText = $state("");

  const hasEncoder = $derived(encoder !== null && encoder.model_id !== null);

  function errMsg(e: unknown): string {
    if (typeof e === "string") return e;
    if (e && typeof e === "object" && "message" in e) {
      return String((e as { message: unknown }).message);
    }
    return String(e);
  }

  async function load(): Promise<void> {
    try {
      encoder = await api<VisualModelStatusDto>("visual_model_status");
      head = await api<HeadStatusDto>("head_status");
      loadError = null;
    } catch (e) {
      loadError = errMsg(e);
    }
  }

  $effect(() => {
    void load();
  });

  // Waits for the job behind `taskId` to say what became of it, then
  // drops both subscriptions — the panel is not a job monitor, it is
  // asking one question about one task.
  //
  // Two ways out, because the per-task event is delivered best-effort
  // (`ProgressEmitter`): the run's own final step, and the `jobs:tick`
  // broadcast that fires after every job of a kind. The tick carries no
  // message, so it settles nothing about the verdict — what it does is
  // keep a dropped event from leaving the section busy until the panel
  // is reopened. It is emitted after the per-task one, so the ordinary
  // path still reads the verdict first.
  async function follow(taskId: string): Promise<void> {
    const drop: UnlistenFn[] = [];
    await new Promise<void>((resolve) => {
      listen<{ current?: number; total?: number | null; message?: string }>(
        `job:progress:${taskId}`,
        (evt) => {
          const message = evt.payload?.message;
          if (message) verdict = message;
          const total = evt.payload?.total ?? null;
          const current = evt.payload?.current ?? 0;
          if (total !== null && current >= total) resolve();
        },
      ).then((fn) => drop.push(fn));
      listen<{ kind?: string }>("jobs:tick", (evt) => {
        const kind = evt.payload?.kind ?? "";
        if (kind === "head_train" || kind === "head_pull") resolve();
      }).then((fn) => drop.push(fn));
    });
    for (const unlisten of drop) unlisten();
    // The pointer may have moved: re-read rather than infer.
    await load();
  }

  // One gate for the section: training and installing both end at the
  // same pointer, and two of them in flight is a race over which head
  // is promoted.
  async function run(fn: () => Promise<string>): Promise<void> {
    if (busy) return;
    busy = true;
    error = null;
    verdict = null;
    try {
      await follow(await fn());
    } catch (e) {
      error = errMsg(e);
    } finally {
      busy = false;
    }
  }

  async function train(): Promise<void> {
    await run(() => api<string>("train_tag_head"));
  }

  async function pull(): Promise<void> {
    // Client-side checking stops at "there is an object to send".
    // Whether the artifact may score here — the encoder it was trained
    // against, its row widths, its keys — is the job's answer, and its
    // refusal is the message worth reading, so nothing is duplicated
    // here to pre-empt it.
    let artifact: unknown;
    try {
      artifact = JSON.parse(artifactText);
    } catch (e) {
      error = `That is not JSON: ${errMsg(e)}`;
      return;
    }
    if (artifact === null || typeof artifact !== "object" || Array.isArray(artifact)) {
      error = "Paste the head artifact itself — a JSON object.";
      return;
    }
    await run(() => api<string>("pull_tag_head", { artifact }));
  }

  function stamp(ms: number): string {
    return new Date(ms).toLocaleString();
  }
</script>

<h4>Model</h4>

{#if loadError}
  <p class="model-error">{loadError}</p>
{/if}
{#if error}
  <p class="model-error">{error}</p>
{/if}
{#if verdict}
  <p class="model-verdict">{verdict}</p>
{/if}

<dl class="model-facts">
  <dt>Encoder</dt>
  <dd>
    {#if encoder && encoder.model_id}
      <span class="model-id">{encoder.model_id}</span>
      <span class="model-dim">
        {encoder.dim} dimensions · preprocess rev {encoder.preprocess_ver}
      </span>
    {:else}
      <span class="model-none">
        No model bound — tag suggestions and training are off.
      </span>
    {/if}
  </dd>

  <dt>Head</dt>
  <dd>
    {#if head}
      <span class="model-id">{head.bound ?? "zero-shot"}</span>
      {#if head.restart_required}
        <span class="model-badge" title="Takes effect on the next launch"
          >restart</span
        >
      {/if}
      {#if head.promoted && head.promoted !== head.bound}
        <span class="model-hint">
          staged: {head.promoted}{head.run
            ? ""
            : " — its artifact cannot be read, so the next launch falls back to zero-shot"}
        </span>
      {/if}
      {#if head.run}
        <span class="model-hint">
          {head.run.trained_tags} tag(s) trained on {head.run.rulings_used} ruling(s);
          held-out {head.run.held_out} — this head {head.run.candidate_correct}
          vs zero-shot {head.run.baseline_correct}, trained
          {stamp(head.run.trained_at_ms)}
        </span>
        {#if encoder && encoder.model_id && head.run.model_id !== encoder.model_id}
          <span class="model-hint">
            trained against {head.run.model_id} — a head scores only against
            the vectors it learned from, so the next launch falls back to
            zero-shot
          </span>
        {/if}
      {/if}
    {:else}
      <span class="model-none">…</span>
    {/if}
  </dd>

  <dt>Rulings</dt>
  <dd>
    {#if head}
      <span class="model-hint">
        {head.readiness.rulings} ruling(s) across {head.readiness
          .tags_with_rulings} tag(s); {head.readiness.tags_ready} clear the
        training floor of {head.readiness.min_rulings_per_class} per class.
      </span>
    {:else}
      <span class="model-none">…</span>
    {/if}
  </dd>
</dl>

<ul class="model-list">
  <li class="model-row">
    <div class="model-main">
      <span class="model-label">Train a head</span>
      <span class="model-hint">
        Learn from your own accepted and rejected suggestions. A new head
        is staged only if it beats zero-shot on held-out rulings.
      </span>
    </div>
    <button
      class="model-action"
      disabled={busy || !hasEncoder}
      title="Enqueue a training run over the rulings under the bound encoder"
      onclick={train}
    >
      Train now
    </button>
  </li>
  <li class="model-row model-row-stacked">
    <div class="model-main">
      <span class="model-label">Pull the team's head</span>
      <span class="model-hint">
        Fetch <code>GET /teams/heads/registry</code> with your own session and
        paste the JSON here. It is verified against this encoder before
        anything is installed.
      </span>
    </div>
    <textarea
      class="model-artifact"
      bind:value={artifactText}
      disabled={busy || !hasEncoder}
      rows="3"
      placeholder="the head artifact, as fetched"
    ></textarea>
    <button
      class="model-action"
      disabled={busy || !hasEncoder || artifactText.trim() === ""}
      title="Verify and install the pasted head, then stage it"
      onclick={pull}
    >
      Install
    </button>
  </li>
</ul>

<style>
  .model-facts {
    display: grid;
    grid-template-columns: 6rem 1fr;
    gap: 0.25rem 0.6rem;
    margin: 0 0 0.8rem;
    font-size: 0.78rem;
  }
  .model-facts dt {
    color: #99a;
  }
  .model-facts dd {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }
  .model-id {
    color: #334;
  }
  .model-dim,
  .model-hint {
    font-size: 0.75rem;
    font-style: italic;
    color: #999;
  }
  .model-none {
    font-size: 0.75rem;
    color: #999;
  }
  .model-badge {
    align-self: flex-start;
    font-size: 0.66rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.55;
    border: 1px solid currentColor;
    border-radius: 3px;
    padding: 0 0.25rem;
  }
  .model-list {
    list-style: none;
    margin: 0 0 1rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
  }
  .model-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .model-row-stacked {
    align-items: flex-end;
  }
  .model-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }
  .model-label {
    font-size: 0.82rem;
    color: #334;
  }
  .model-artifact {
    flex: none;
    width: 12rem;
    font-size: 0.72rem;
    font-family: inherit;
    padding: 0.2rem 0.4rem;
    resize: vertical;
  }
  .model-action {
    flex: none;
    font-size: 0.78rem;
    padding: 0.25rem 0.7rem;
    cursor: pointer;
  }
  .model-action:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .model-error {
    color: var(--danger, #e2665b);
    font-size: 0.78rem;
  }
  .model-verdict {
    color: #567;
    font-size: 0.78rem;
  }
</style>
