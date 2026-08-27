<script lang="ts">
  // SharedLinesPanel — the lines a team hosts, in their own drawer.
  //
  // The separation is the requirement (#148 decision 16): shared lines
  // list here rather than mixed into the local ones, because they come
  // from somewhere else and a surface that hid that would be claiming
  // one library where there are two. Nothing on this panel is a copy of
  // anything — each read is a request to the server, and when the
  // connection goes the panel goes empty rather than stale.
  //
  // 0-prop by design, like DispatchHistoryPanel: it reads
  // `sharedCatalog` and `activeFilter.activePersona` directly, and the
  // App only mounts it.
  //
  // The team id is typed in because the member's client has no verb
  // for "the teams I am in". What a picker would change when that verb
  // lands, and what it would not, is argued in the catalog's header
  // rather than restated here.
  //
  // What this panel reads from that header is `phase`: there is nobody
  // to ask, there is nobody chosen to ask about, or there is. The three
  // branches below are those three, and the empty list belongs to the
  // last of them alone — under either of the others it would be
  // answering for a team on nobody's behalf.
  import { sharedCatalog } from "./lib/stores/shared.svelte";
  import { activeFilter } from "./lib/stores/filter.svelte";

  let baseUrl = $state("http://127.0.0.1:8787");
  let login = $state("");
  let password = $state("");

  // Publishing asks for more than a click, and all of it is init-time.
  let publishLineId = $state("");
  let publishName = $state("");
  let reenact = $state(false);

  const STRATEGY = "mainline-first";

  async function connect(event: Event) {
    event.preventDefault();
    await sharedCatalog.connect(baseUrl, login, password);
    password = "";
    if (sharedCatalog.teamId) await sharedCatalog.lines.load({ teamId: sharedCatalog.teamId });
  }

  async function look(event: Event) {
    event.preventDefault();
    await sharedCatalog.lines.load({ teamId: sharedCatalog.teamId });
  }

  async function publish(event: Event) {
    event.preventDefault();
    await sharedCatalog.publish(publishLineId, publishName, STRATEGY, reenact);
    publishLineId = "";
    publishName = "";
    reenact = false;
  }
</script>

{#if sharedCatalog.open}
  <!-- Backdrop absorbs outside-click and Escape; the drawer itself
       stopPropagation so an interior click never closes. -->
  <div
    class="drawer-backdrop"
    onclick={() => sharedCatalog.closePanel()}
    onkeydown={(e) => e.key === "Escape" && sharedCatalog.closePanel()}
    role="button"
    tabindex="-1"
    aria-label="Close shared lines"
  >
    <div
      class="drawer"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      tabindex="-1"
      aria-label="Shared lines"
    >
      <header class="drawer-head">
        <h3>Shared lines</h3>
        <button
          class="drawer-close"
          onclick={() => sharedCatalog.closePanel()}
          aria-label="Close"
        >✕</button>
      </header>

      <!-- These lines are on a team's server. They are read from it
           every time this panel opens; nothing here is kept. -->
      <p class="drawer-note">
        Hosted by a team, and read from it. Nothing on this panel is
        stored on this machine — cloning is how you take a copy.
      </p>

      {#if sharedCatalog.phase === "disconnected"}
        <form class="drawer-form" onsubmit={connect}>
          <label>
            Server
            <input type="url" bind:value={baseUrl} required />
          </label>
          <label>
            Login
            <input type="text" bind:value={login} required autocomplete="username" />
          </label>
          <label>
            Password
            <input
              type="password"
              bind:value={password}
              required
              autocomplete="current-password"
            />
          </label>
          <button type="submit">Connect</button>
        </form>
      {:else}
        <div class="drawer-session">
          <span>Signed in as {sharedCatalog.session}</span>
          <button type="button" onclick={() => sharedCatalog.disconnect()}>
            Disconnect
          </button>
        </div>

        <form class="drawer-form" onsubmit={look}>
          <label>
            Team
            <input
              type="text"
              bind:value={sharedCatalog.teamId}
              placeholder="team id"
              required
            />
          </label>
          <button type="submit">List its lines</button>
        </form>

        {#if sharedCatalog.said}
          <p class="drawer-said">{sharedCatalog.said}</p>
        {/if}

        {#if sharedCatalog.phase === "no-team"}
          <p class="drawer-empty">
            Name a team above to see the lines it hosts.
          </p>
        {:else if sharedCatalog.lines.loading}
          <p class="drawer-empty">loading…</p>
        {:else if sharedCatalog.lines.error}
          <p class="drawer-empty drawer-error">
            Could not read the team's lines: {sharedCatalog.lines.error}
          </p>
        {:else if sharedCatalog.lines.data.length === 0}
          <p class="drawer-empty">This team hosts no lines.</p>
        {:else}
          <ul class="drawer-list" role="list">
            {#each sharedCatalog.lines.data as line (line.id)}
              <li>
                <button
                  type="button"
                  class="drawer-row"
                  class:active={sharedCatalog.selected === line.id}
                  onclick={() => sharedCatalog.show(line.id)}
                  title="Read what is on this line"
                >
                  <span class="row-title">{line.name}</span>
                  <span class="row-standing">{line.standing}</span>
                </button>

                {#if sharedCatalog.selected === line.id}
                  <!-- How the chain reads is the visible difference
                       between the two seedings: published as it stands
                       is one change point however long the private
                       line was, re-enacted is as many as it had. -->
                  {#if sharedCatalog.changePoints !== null}
                    <p class="drawer-chain">
                      {sharedCatalog.changePoints} change point{sharedCatalog.changePoints ===
                      1
                        ? ""
                        : "s"} since this line began
                    </p>
                  {/if}
                  {#if sharedCatalog.states.loading}
                    <p class="drawer-empty">loading…</p>
                  {:else if sharedCatalog.states.error}
                    <p class="drawer-empty drawer-error">
                      {sharedCatalog.states.error}
                    </p>
                  {:else if sharedCatalog.onTheLine.length === 0}
                    <p class="drawer-empty">Nothing is on this line.</p>
                  {:else}
                    <ul class="drawer-entries" role="list">
                      {#each sharedCatalog.onTheLine as entry (entry.entry_id)}
                        <li class="entry">
                          <span class="entry-name">{entry.name ?? entry.entry_id}</span>
                          <button
                            type="button"
                            disabled={activeFilter.activePersona === null}
                            title={activeFilter.activePersona === null
                              ? "Pick a single persona to clone into"
                              : "Take a detached copy into this library"}
                            onclick={() =>
                              sharedCatalog.clone(
                                entry.entry_id,
                                activeFilter.activePersona!,
                              )}
                          >Clone</button>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/if}
              </li>
            {/each}
          </ul>
        {/if}

        <!-- Publishing. The re-enactment is chosen here or never:
             a line seeded with its current state cannot be given its
             history afterwards.

             Behind `ready` for the same reason the list above is: it
             seeds a line on the team in the field, and with no team
             named it would be offering to publish to nobody. -->
        {#if sharedCatalog.phase === "ready"}
          <form class="drawer-form drawer-publish" onsubmit={publish}>
            <h4>Publish a line of mine</h4>
            <label>
              Local line
              <input
                type="text"
                bind:value={publishLineId}
                placeholder="line id"
                required
              />
            </label>
            <label>
              Call it
              <input type="text" bind:value={publishName} required />
            </label>
            <label class="drawer-check">
              <input type="checkbox" bind:checked={reenact} />
              Re-enact the whole chain
            </label>
            <p class="drawer-cost">
              {#if reenact}
                The team's line will be <strong>re-enacted</strong>: one
                change point for each of mine, every act stamped to me
                rather than to whoever made the work, and every content
                the line ever named sent — including what has since been
                replaced. Work logs and conversations do not go.
              {:else}
                The team gets what the line holds now, as a single change
                point. Choose re-enactment before publishing if you want
                the chain; it cannot be added to the line afterwards.
              {/if}
            </p>
            <button type="submit">Publish</button>
          </form>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 60;
    border: 0;
    padding: 0;
  }
  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    height: 100%;
    width: min(30rem, 92vw);
    overflow-y: auto;
    background: var(--panel-bg, #1b1b1e);
    color: var(--panel-fg, #e8e8ea);
    box-shadow: -0.5rem 0 1.5rem rgba(0, 0, 0, 0.4);
    padding: 1rem 1.15rem 2rem;
    box-sizing: border-box;
  }
  .drawer-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .drawer-head h3 {
    margin: 0;
    font-size: 1rem;
  }
  .drawer-close {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.9rem;
  }
  .drawer-note,
  .drawer-cost {
    font-size: 0.78rem;
    opacity: 0.72;
    line-height: 1.45;
  }
  .drawer-said {
    font-size: 0.8rem;
    border-left: 2px solid currentColor;
    padding-left: 0.5rem;
    opacity: 0.85;
  }
  .drawer-form {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    margin: 0.9rem 0;
  }
  .drawer-form label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.78rem;
  }
  .drawer-check {
    flex-direction: row !important;
    align-items: center;
    gap: 0.4rem;
  }
  .drawer-publish {
    border-top: 1px solid rgba(255, 255, 255, 0.12);
    padding-top: 0.9rem;
  }
  .drawer-publish h4 {
    margin: 0;
    font-size: 0.85rem;
  }
  .drawer-session {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.78rem;
    opacity: 0.85;
  }
  .drawer-empty {
    font-size: 0.8rem;
    opacity: 0.7;
  }
  .drawer-error {
    color: #ff9d9d;
  }
  .drawer-list,
  .drawer-entries {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .drawer-row {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 0.5rem;
    background: none;
    border: 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    color: inherit;
    cursor: pointer;
    padding: 0.45rem 0.1rem;
    text-align: left;
    font-size: 0.82rem;
  }
  .drawer-row.active {
    font-weight: 600;
  }
  .row-standing {
    opacity: 0.6;
    font-size: 0.72rem;
  }
  .entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.25rem 0 0.25rem 0.8rem;
    font-size: 0.78rem;
  }
  .entry-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .drawer-chain {
    font-size: 0.72rem;
    opacity: 0.6;
    margin: 0.2rem 0 0.2rem 0.8rem;
  }
</style>
