<script lang="ts">
  // ReleaseView — one release, and what became of every copy it wrote.
  //
  // Opens whenever `releaseCatalog.openId` is non-null, which the forge
  // panel's history tab sets: a change point is written out from the row
  // that names it, and everything after that verb is this drawer's. Why
  // writing out and sending are two verbs rather than one dialog with a
  // destination on it is argued in `lib/stores/release.svelte.ts`, beside
  // the reads it makes; this header says only what this component adds.
  //
  // Mounted once by App, like `SnapshotView` beside it, and zero-prop
  // for state. A release is a thing you open from somewhere else, and
  // the somewhere else is a row in a chain that stays on screen behind
  // this.
  //
  // # Three sections, and the order is the order things happen
  //
  // **Files**: one row per copy, with both halves of its stamp. Per-file
  // rather than one summary for the release, for the reason the record
  // is per-file: a build with no certificate reports the manifest half
  // skipped on every file and a container that cannot take a packet
  // reports it on one, and a summary would have to pick which of those
  // to say.
  //
  // **Sends**: one row per send, each expanding to one line per file
  // with what the host answered. A partial send is labelled by its
  // refusal count and never rounded to done — `lib/attempt-record.ts`
  // holds that rule, because the rows belong to every dispatch rather
  // than to this screen.
  //
  // **Send…**: a profile picked from a directory, a label, and the
  // button. Under it, one sentence about the step this does not take.
  //
  // # The manifest's three readings, in a neutral register
  //
  // "stamped", "not signed — no certificate configured", "failed — …".
  // Absence is stated as absence: a file that was not signed is not a
  // file that failed, and the wording keeps them apart because the two
  // lead somewhere completely different. This follows C2PA's own
  // register ("Trusted" / "Valid" / "No Content Credentials found")
  // rather than any wording found in a shipping app — none was, and the
  // sentence here is this repository's own.
  //
  // # Where the files sit, and why the control copies rather than opens
  //
  // The header names the directory the copies were written into, taken
  // from the rows themselves rather than from the setting: the setting
  // says where the *next* release goes, and this release went where it
  // went. The control beside it copies that path.
  //
  // It copies rather than revealing in a file manager, and that is the
  // settled answer to #280's open question rather than a stopgap. Three
  // things decide it. Linux has no single file manager to reveal in, so
  // a fallback has to exist on every platform that has one. Reaching one
  // from the webview needs a capability this app's `default.json` does
  // not grant and an npm package its lockfile does not carry — a change
  // to the app's permission surface, bought for a convenience. And one
  // behaviour everywhere is a behaviour the e2e can assert; a reveal is
  // a window nothing can read. So the button says what it does.
  import { api } from "./lib/api";
  import { releaseCatalog, isTerminal } from "./lib/stores/release.svelte";
  import { readAttempt, attemptSummary } from "./lib/attempt-record";
  import type { ForgeReleaseFileDto, ForgeSendDto } from "./bindings";

  interface Props {
    // Toast surface. The drawer's own writes go through `mutate`, which
    // puts a refusal on screen by itself; this is for the things that
    // are not writes at all, like whether a copy to the clipboard took.
    onFlash: (msg: string, ms?: number) => void;
  }

  let { onFlash }: Props = $props();

  // One send open at a time. Its per-file lines are the wall of table
  // the design warns about when every row stands open at once.
  let openSend = $state<string | null>(null);
  let sendFormOpen = $state(false);
  let destination = $state("");
  let pickedProfile = $state("");
  let rawProfile = $state("");
  let sending = $state(false);

  const release = $derived(releaseCatalog.release.data);
  const profiles = $derived(releaseCatalog.profiles.data);

  // Every copy this release wrote was written into one directory, and
  // the rows are where that is recorded. Derived from the first row
  // rather than stored: a release carries no output directory of its
  // own, and the setting behind the verb answers about the next release
  // rather than this one.
  const wroteInto = $derived.by(() => {
    const first = release?.files[0]?.path;
    if (first === undefined) return null;
    const cut = Math.max(first.lastIndexOf("/"), first.lastIndexOf("\\"));
    return cut > 0 ? first.slice(0, cut) : null;
  });

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  function basename(path: string): string {
    const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return cut >= 0 ? path.slice(cut + 1) : path;
  }

  /// What the IPTC/XMP half of a stamp says.
  ///
  /// The tokens `detail` carries for a skip are stable words rather
  /// than sentences (`no_signing_identity` and its siblings), so they
  /// are shown as they are rather than translated: a word this build
  /// has never seen would otherwise arrive as a guess, and the one
  /// reading worth spelling out is spelled out below.
  function xmpReads(file: ForgeReleaseFileDto): string {
    if (file.xmp.state === "written") return "written";
    if (file.xmp.state === "skipped") {
      return `not written${file.xmp.detail ? ` — ${file.xmp.detail}` : ""}`;
    }
    return `failed${file.xmp.detail ? ` — ${file.xmp.detail}` : ""}`;
  }

  /// What the signed-manifest half says, in three readings.
  ///
  /// `no_signing_identity` is the one this build produces on every file
  /// — the composition root builds the disclosure writer unsigned — so
  /// it is the one reading written out as a sentence rather than shown
  /// as a token.
  function manifestReads(file: ForgeReleaseFileDto): string {
    if (file.manifest.state === "written") return "stamped";
    if (file.manifest.state === "skipped") {
      if (file.manifest.detail === "no_signing_identity") {
        return "not signed — no certificate configured";
      }
      return `not signed${file.manifest.detail ? ` — ${file.manifest.detail}` : ""}`;
    }
    return `failed${file.manifest.detail ? ` — ${file.manifest.detail}` : ""}`;
  }

  function stampClass(state: string): string {
    if (state === "written") return "ok";
    return state === "failed" ? "bad" : "quiet";
  }

  async function copyPath(path: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(path);
      onFlash("Path copied");
    } catch {
      // The clipboard can be refused by the webview, and saying so is
      // better than a button that looks like it worked.
      onFlash("Could not reach the clipboard", 6000);
    }
  }

  async function toggleSendForm(): Promise<void> {
    sendFormOpen = !sendFormOpen;
    if (sendFormOpen && !releaseCatalog.profiles.answered) {
      await releaseCatalog.profiles.load(undefined);
    }
  }

  /// Whether a profile can be sent with.
  ///
  /// `== null` rather than `=== null`, so an absent field and a null
  /// one are the same answer. The contract says the wire always carries
  /// one — `TransferProfileDto` explains why it stopped leaving them
  /// out — and this is the belt: a strict comparison against `null` is
  /// what made every profile in this picker unpickable the first time,
  /// and the check is in one place now rather than at each of the three
  /// sites that ask.
  function usable(profile: { error: string | null }): boolean {
    return profile.error == null;
  }

  /// The profile text the send goes out with.
  ///
  /// A picked profile is read from the file by name, and the raw box is
  /// the one-off. The picked one wins when both are filled: choosing
  /// from the list is the deliberate gesture, and a half-typed draft
  /// left in the box below it is not a second answer.
  const chosen = $derived(
    profiles?.profiles.find((row) => row.name === pickedProfile) ?? null,
  );

  const canSend = $derived(
    !sending &&
      destination.trim().length > 0 &&
      ((chosen !== null && usable(chosen)) ||
        (chosen === null && rawProfile.trim().length > 0)),
  );

  async function doSend(): Promise<void> {
    if (release === null || !canSend) return;
    sending = true;
    try {
      // A picked profile is read from its file here rather than held in
      // the list: the list carries a summary built to be rendered, and
      // the send needs the profile whole — including the auth block the
      // summary deliberately leaves out.
      //
      // **Its own arm, because it is not a `mutate`.** This read goes
      // through `api`, which hands a failure back and says nothing —
      // right for a read a `Resource` normalises, wrong here, where the
      // failure means the send did not happen. The first version of
      // this caught the whole body and left the comment "`mutate` has
      // already put the refusal on screen", which is true of the send
      // and false of this: a profile that could not be read produced a
      // button press with no send, no row and nothing said, which is
      // exactly the defect `lib/mutate.ts` exists to prevent.
      let body: string;
      try {
        body =
          chosen !== null
            ? await api<string>("read_transfer_profile", { name: chosen.name })
            : rawProfile.trim();
      } catch (err) {
        onFlash(`Could not read the profile “${chosen?.name}”: ${String(err)}`, 8000);
        return;
      }
      await releaseCatalog.send(release.id, destination.trim(), body);
      destination = "";
      rawProfile = "";
      sendFormOpen = false;
    } catch {
      // The send itself. `mutate` has already put the refusal on
      // screen; the form stays open with what was typed still in it.
    } finally {
      sending = false;
    }
  }

</script>

<!--
  A fixed overlay, like `SnapshotView`: it comes up over the forge
  drawer rather than instead of it, so closing it leaves the chain
  exactly where it was.
-->
{#if releaseCatalog.openId !== null}
  <div
    class="rel-backdrop"
    onclick={() => releaseCatalog.close()}
    onkeydown={(e) => e.key === "Escape" && releaseCatalog.close()}
    role="button"
    tabindex="-1"
    aria-label="Close the release"
  >
    <div
      class="rel-panel"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      tabindex="-1"
      aria-label="Release"
    >
      <header class="rel-head">
        <div class="rel-title">
          Release
          {#if release}
            · <code>{release.id.slice(0, 8)}</code>
          {/if}
        </div>
        <button
          class="rel-close"
          onclick={() => releaseCatalog.close()}
          aria-label="Close"
        >✕</button>
      </header>

      {#if releaseCatalog.release.loading && release === null}
        <p class="rel-empty">Reading…</p>
      {:else if releaseCatalog.release.error !== null}
        <p class="rel-empty rel-error">{releaseCatalog.release.error}</p>
      {:else if release}
        <div class="rel-meta">
          <span>written out {when(release.at_ms)}</span>
          <span>by {release.actor_id}</span>
          {#if wroteInto !== null}
            <span class="rel-dir">
              into <code>{wroteInto}</code>
              <button type="button" onclick={() => copyPath(wroteInto)}>
                copy path
              </button>
            </span>
          {/if}
        </div>

        <!-- Files. An empty list is two different states and the run is
             what tells them apart: still going, or finished having
             written nothing. Saying "no files" during the first would be
             wrong about a release whose copies are on their way. -->
        <section class="rel-section" aria-label="What left">
          <h3>Files</h3>
          {#if release.files.length === 0}
            {#if releaseCatalog.releaseSettled}
              <!-- A parked run that wrote nothing is two answers, and
                   the run says which. "Finished and wrote no files" for
                   a run that reached the end, the state and its own
                   message for one that did not — because a release with
                   no rows after a failure is explained by the failure,
                   and a screen that called that "finished" would be
                   hiding the only sentence worth reading. -->
              {#if releaseCatalog.releaseRun?.state === "done"}
                <p class="rel-empty">The run finished and wrote no files.</p>
              {:else}
                <p class="rel-empty rel-error">
                  The run {releaseCatalog.releaseRun?.state ?? "stopped"} and wrote
                  no files{releaseCatalog.releaseRun?.state_message
                    ? ` — ${releaseCatalog.releaseRun.state_message}`
                    : ""}.
                </p>
              {/if}
            {:else}
              <p class="rel-empty">
                The copies are being written
                {#if releaseCatalog.releaseRun}
                  · {releaseCatalog.releaseRun.state}
                {/if}
              </p>
            {/if}
          {:else}
            <ul class="rel-files" role="list">
              {#each release.files as file (file.path)}
                <li>
                  <span class="rel-name" title={file.path}>
                    {basename(file.path)}
                  </span>
                  <span class={`rel-half ${stampClass(file.xmp.state)}`}>
                    XMP: {xmpReads(file)}
                  </span>
                  <span class={`rel-half ${stampClass(file.manifest.state)}`}>
                    manifest: {manifestReads(file)}
                  </span>
                  {#if file.prompt_dropped || file.system_dropped}
                    <!-- What the packet could not carry. Said because a
                         disclosure that dropped half of itself is a fact
                         about the file somebody is about to hand over. -->
                    <span class="rel-half quiet">
                      dropped to fit: {file.prompt_dropped ? "prompt" : ""}{file.prompt_dropped &&
                      file.system_dropped
                        ? ", "
                        : ""}{file.system_dropped ? "system" : ""}
                    </span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <!-- Sends. -->
        <section class="rel-section" aria-label="Where it went">
          <h3>Sends</h3>
          {#if releaseCatalog.sends.data.length === 0}
            <p class="rel-empty">Not sent anywhere yet.</p>
          {:else}
            <ul class="rel-sends" role="list">
              {#each releaseCatalog.sends.data as sent (sent.id)}
                {@render sendRow(sent)}
              {/each}
            </ul>
          {/if}

          <button
            type="button"
            class="rel-btn rel-send-toggle"
            onclick={toggleSendForm}
          >{sendFormOpen ? "▾" : "▸"} Send…</button>

          {#if sendFormOpen}
            {@render sendForm()}
          {/if}
        </section>
      {/if}
    </div>
  </div>
{/if}

{#snippet sendRow(sent: ForgeSendDto)}
  {@const run = releaseCatalog.runOf(sent)}
  {@const record = readAttempt(run?.attempt_json ?? null)}
  <li>
    <button
      class="rel-send-head"
      aria-expanded={openSend === sent.id}
      onclick={() => (openSend = openSend === sent.id ? null : sent.id)}
    >
      <span class="rel-caret">{openSend === sent.id ? "▾" : "▸"}</span>
      <span class="rel-dest">{sent.destination}</span>
      <span class="quiet">{when(sent.at_ms)}</span>
      <span class="quiet">{run ? run.state : "reading…"}</span>
      <span class={record && record.refused > 0 ? "bad" : "quiet"}>
        {attemptSummary(record)}
      </span>
    </button>
    {#if openSend === sent.id}
      <div class="rel-attempt">
        {#if record === null}
          <p class="rel-empty">
            {run && isTerminal(run.state)
              ? "The run recorded nothing."
              : "The run has not reported yet."}
          </p>
        {:else}
          {#if record.endpoint !== null}
            <p class="quiet rel-endpoint">to <code>{record.endpoint}</code></p>
          {/if}
          {#if record.refusedBefore !== null}
            <!-- Refused before any file moved: one sentence about the
                 call rather than a row per file, which is the other
                 shape the adapter writes. -->
            <p class="bad">{record.refusedBefore}</p>
          {/if}
          {#if record.files.length > 0}
            <ul class="rel-attempt-files" role="list">
              {#each record.files as row (row.source)}
                <li>
                  <span class="rel-name" title={row.source}>{row.name}</span>
                  <span class={row.outcome === "sent" ? "ok" : "bad"}>
                    {row.outcome}
                  </span>
                  <span class="quiet">
                    {row.answer ?? (row.bytes !== null ? `${row.bytes} bytes` : "")}
                  </span>
                </li>
              {/each}
            </ul>
          {/if}
          {#if record.sidecar !== null}
            <p class="quiet">
              sidecar {record.sidecar.name} ·
              {record.sidecar.outcome}{record.sidecar.answer
                ? ` — ${record.sidecar.answer}`
                : ""}
            </p>
          {/if}
        {/if}
      </div>
    {/if}
  </li>
{/snippet}

{#snippet sendForm()}
  <div class="rel-send-form" role="group" aria-label="Send this release">
    {#if releaseCatalog.profiles.loading}
      <p class="rel-empty">Reading the profiles…</p>
    {:else if profiles !== null}
      <p class="quiet">
        Profiles in <code>{profiles.directory}</code>
      </p>
      {#if profiles.profiles.length === 0}
        <!-- Nothing creates this directory, so an empty list is the
             state before the first profile is written rather than a
             failure. It says where one goes. -->
        <p class="rel-empty">
          No profile here yet. A profile is one JSON file in that
          directory; <code>asterism-server schema print
          exporter:transfer:params</code> prints a runnable example.
        </p>
      {:else}
        <ul class="rel-profiles" role="list">
          {#each profiles.profiles as profile (profile.name)}
            <li class:unusable={!usable(profile)}>
              <label>
                <input
                  type="radio"
                  name="transfer-profile"
                  value={profile.name}
                  bind:group={pickedProfile}
                  disabled={!usable(profile)}
                />
                <span class="rel-profile-name">{profile.name}</span>
              </label>
              {#if usable(profile)}
                <span class="quiet">
                  {profile.scheme}{profile.host ? `://${profile.host}` : "://"}{profile.directory}
                </span>
              {:else}
                <!-- Listed with its reason rather than dropped: a file
                     the app stopped mentioning is one somebody edits
                     blind. It cannot be picked. -->
                <span class="bad">{profile.error}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    {/if}

    <label class="rel-field">
      Destination label
      <input type="text" bind:value={destination} placeholder="what to remember this send by" />
    </label>

    <!-- The one-off. A profile is the reusable answer and this is for
         the send that is not going to happen twice; the picked one wins
         when both are filled. -->
    <label class="rel-field">
      or a profile, as JSON
      <textarea rows="3" bind:value={rawProfile} disabled={chosen !== null}
      ></textarea>
    </label>

    <button
      type="button"
      class="rel-btn rel-btn-primary"
      onclick={doSend}
      disabled={!canSend}
    >{sending ? "Sending…" : "Send"}</button>

    <!-- The step this does not take. Named because an uploader that
         says nothing about it is one somebody assumes has taken it, and
         no uploader anywhere ticks this box. No agency is named: which
         agency asks for what is that agency's business and changes on
         its schedule, and a name here would be this tree carrying
         somebody else's policy. -->
    <p class="quiet rel-note">
      The generative-AI declaration is on the agency's own submission
      form. Nothing here ticks it.
    </p>
  </div>
{/snippet}

<style>
  .rel-backdrop {
    position: fixed;
    inset: 0;
    background: var(--wash-down);
    z-index: 70;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 0;
    padding: 0;
  }
  .rel-panel {
    width: min(46rem, 96vw);
    max-height: 84vh;
    overflow-y: auto;
    background: var(--surface-raised);
    border-radius: 8px;
    box-shadow: 0 12px 32px var(--shadow-color-strong);
    display: flex;
    flex-direction: column;
    font-family: inherit;
    color: var(--ink);
  }
  .rel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.7rem 1rem 0.5rem;
    border-bottom: 1px solid var(--accent-line);
  }
  .rel-title {
    font-size: 0.95rem;
  }
  code {
    font-family: monospace;
    color: var(--accent-ink);
    background: var(--accent-surface);
    padding: 0.05rem 0.35rem;
    border-radius: 4px;
  }
  .rel-close {
    background: transparent;
    border: none;
    color: var(--accent-ink);
    font-size: 0.95rem;
    cursor: pointer;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
  }
  .rel-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    padding: 0.5rem 1rem;
    color: var(--accent-ink);
    font-size: 0.78rem;
    border-bottom: 1px solid var(--accent-line);
  }
  .rel-dir {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
  }
  .rel-dir button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
    font-size: 0.72rem;
  }
  .rel-section {
    padding: 0.6rem 1rem 0.8rem;
    border-bottom: 1px solid var(--accent-line);
  }
  .rel-section h3 {
    margin: 0 0 0.4rem;
    font-size: 0.85rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rel-files > li,
  .rel-attempt-files > li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    padding: 0.25rem 0;
    border-bottom: 1px solid var(--accent-line);
    font-size: 0.76rem;
  }
  .rel-name {
    min-width: 11rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rel-half {
    font-size: 0.74rem;
  }
  .ok {
    color: var(--accent-ink);
  }
  .bad {
    color: var(--danger-ink);
  }
  .quiet {
    color: var(--accent-ink);
    opacity: 0.8;
    font-size: 0.74rem;
  }
  .rel-send-head {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    padding: 0.3rem 0;
    text-align: left;
    width: 100%;
    font-size: 0.78rem;
  }
  .rel-caret {
    min-width: 1rem;
  }
  .rel-dest {
    min-width: 8rem;
  }
  .rel-attempt {
    margin: 0 0 0.4rem 1.2rem;
    border-left: 1px solid var(--accent-line);
    padding-left: 0.6rem;
  }
  .rel-endpoint {
    margin: 0.2rem 0;
  }
  .rel-send-form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 0.5rem;
    padding: 0.6rem;
    border: 1px solid var(--accent-line);
    border-radius: 6px;
    background: var(--accent-surface);
  }
  .rel-profiles > li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.5rem;
    padding: 0.2rem 0;
    font-size: 0.76rem;
  }
  .rel-profiles > li.unusable .rel-profile-name {
    opacity: 0.6;
    text-decoration: line-through;
  }
  .rel-profiles label {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
  }
  .rel-field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.74rem;
    color: var(--accent-ink);
  }
  .rel-field input,
  .rel-field textarea {
    font-family: inherit;
    font-size: 0.78rem;
    color: var(--ink);
    background: var(--surface-raised);
    border: 1px solid var(--accent-line);
    border-radius: 6px;
    padding: 0.28rem 0.5rem;
    box-sizing: border-box;
    width: 100%;
  }
  .rel-field textarea {
    font-family: monospace;
    resize: vertical;
  }
  .rel-btn {
    padding: 0.28rem 0.75rem;
    font-size: 0.78rem;
    font-family: inherit;
    color: var(--accent-ink);
    background: var(--accent-surface);
    border: 1px solid var(--accent-line);
    border-radius: 6px;
    cursor: pointer;
    align-self: flex-start;
  }
  .rel-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .rel-btn-primary {
    background: var(--accent-fill);
    color: var(--accent-on-fill);
    border-color: var(--accent-line-strong);
  }
  .rel-note {
    margin: 0;
  }
  .rel-empty {
    padding: 0.4rem 0;
    color: var(--accent-ink);
    font-size: 0.78rem;
  }
  .rel-error {
    color: var(--danger-ink);
  }
</style>
