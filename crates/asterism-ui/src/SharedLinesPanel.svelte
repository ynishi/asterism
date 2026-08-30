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
  // Tabs rather than one column, and the reasoning is the catalog's:
  // the lines a team hosts, its roster and its ledger are three answers
  // about one team, so moving between them changes the question rather
  // than the subject. Two of the three are here; the roster is a later
  // child of #171 and lands beside them.
  //
  // The connection and the team sit *above* the tabs, because they are
  // what the tabs are answers about. Publishing sits *inside* the lines
  // tab, because it seeds a line and a line is what that tab is for.
  import { sharedCatalog } from "./lib/stores/shared.svelte";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { fmtDateTime } from "./lib/formatters";

  let baseUrl = $state("http://127.0.0.1:8787");
  let login = $state("");
  let password = $state("");

  // The field is this component's, not the catalog's.
  //
  // Bound straight to `sharedCatalog.teamId` it changed the team every
  // read is made against as somebody typed, so a ledger walk started on
  // one team would continue against another — the next page requested
  // from team B with team A's cursor, and its answer appended to team
  // A's list. Naming a team is a submit, and `lookAt` is what a submit
  // reaches; between the two the catalog does not move.
  //
  // Seeded from the catalog, because a connection outlives this drawer
  // and reopening it should show the team it was last looking at.
  let teamField = $state(sharedCatalog.teamId);

  let tab = $state<"lines" | "roster" | "ledger">("lines");
  // One event's payload open at a time, by `event_id`.
  let openPayload = $state<string | null>(null);

  // Publishing asks for more than a click, and all of it is init-time.
  let publishLineId = $state("");
  let publishName = $state("");
  let reenact = $state(false);

  const STRATEGY = "mainline-first";

  async function connect(event: Event) {
    event.preventDefault();
    await sharedCatalog.connect(baseUrl, login, password);
    password = "";
    if (teamField) await sharedCatalog.lookAt(teamField);
  }

  async function look(event: Event) {
    event.preventDefault();
    // Everything naming a team has to let go of is `lookAt`'s, written
    // once there rather than at each caller.
    await sharedCatalog.lookAt(teamField);
    if (tab === "ledger") await sharedCatalog.readLedgerPage();
  }

  // The ledger reads on demand rather than beside the lines: it answers
  // what the team did, which is a question asked apart from working
  // with what it holds.
  async function toLedger() {
    tab = "ledger";
    if (!sharedCatalog.ledgerRead) await sharedCatalog.readLedgerPage();
  }

  // The roster reads on demand for the same reason: who is in a team
  // is a question about the team rather than about the work.
  async function toRoster() {
    tab = "roster";
    if (sharedCatalog.roster.data === null) {
      await sharedCatalog.roster.load({ teamId: sharedCatalog.teamId });
    }
  }

  // Founding a team names it too. Somebody who just made one wants to
  // be looking at it, and the alternative is copying an id out of a
  // message into the field directly above.
  async function makeTeam() {
    const teamId = await sharedCatalog.createTeam();
    teamField = teamId;
    await sharedCatalog.lookAt(teamId);
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
              bind:value={teamField}
              placeholder="team id"
              required
            />
          </label>
          <button type="submit">List its lines</button>
        </form>

        <!-- Founding a team sits beside the field rather than on a tab,
             because every tab is an answer about the team named above
             and this is the one act about no team in particular.
             Beside the field in every phase with a connection, not only
             in `no-team`: naming a team is a one-way trip on this
             surface — the field is `required`, so there is no way back
             to having named none — and an offer that only appeared
             there would be an offer somebody could take exactly once
             per window. -->
        <button type="button" class="make-team" onclick={makeTeam}>
          Start a team of your own
        </button>

        {#if sharedCatalog.said}
          <p class="drawer-said">{sharedCatalog.said}</p>
        {/if}

        {#if sharedCatalog.phase === "no-team"}
          <p class="drawer-empty">
            Name a team above to see the lines it hosts.
          </p>
        {:else}
          <nav class="drawer-tabs" aria-label="What to read about this team">
            <button
              type="button"
              class:active={tab === "lines"}
              onclick={() => (tab = "lines")}
            >lines</button>
            <button
              type="button"
              class:active={tab === "roster"}
              onclick={toRoster}
            >members</button>
            <button
              type="button"
              class:active={tab === "ledger"}
              onclick={toLedger}
            >ledger</button>
          </nav>
        {/if}

        {#if sharedCatalog.phase === "no-team" || tab !== "lines"}
          <!-- The lines list is what this chain renders, and this arm
               is what keeps it off the ledger. The publish form below
               carries its own condition. -->
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
        {#if sharedCatalog.phase === "ready" && tab === "lines"}
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

        {#if sharedCatalog.phase === "ready" && tab === "roster"}
          <!-- Ids rather than names, and the note says so where a
               reader would otherwise compare this tab with the ledger
               and wonder. A membership row holds a user id and a role;
               the name on a ledger event is a snapshot the act took,
               and there is no equivalent to read here. -->
          <p class="drawer-note">
            Who is in this team. Accounts are shown by id — a membership
            row carries no name, unlike an act in the ledger, which
            keeps the name it read when it happened.
          </p>

          {#if sharedCatalog.roster.loading}
            <p class="drawer-empty">loading…</p>
          {:else if sharedCatalog.roster.error}
            <p class="drawer-empty drawer-error">
              Could not read the team's roster: {sharedCatalog.roster.error}
            </p>
          {:else if sharedCatalog.roster.data === null}
            <p class="drawer-empty">Nothing read yet.</p>
          {:else}
            <ul class="drawer-list roster" role="list">
              {#each sharedCatalog.roster.data.members as member (member.user_id)}
                <li class="member" class:you={member.user_id === sharedCatalog.session}>
                  <span class="member-id">{member.user_id}</span>
                  <span class="member-role">
                    {member.role}{#if member.user_id === sharedCatalog.session}
                      &nbsp;· you{/if}
                  </span>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}

        {#if sharedCatalog.phase === "ready" && tab === "ledger"}
          <p class="drawer-note">
            What this team did, and in what capacity. Oldest first, and
            the names are as they read when each act was recorded.
          </p>

          {#if sharedCatalog.ledgerError}
            <p class="drawer-empty drawer-error">
              Could not read the team's ledger: {sharedCatalog.ledgerError}
            </p>
          {/if}

          {#if sharedCatalog.ledger.length > 0}
            <ul class="drawer-list ledger" role="list">
              <!-- `kind` and `payload_json` are rendered as stored, and
                   that is a decision rather than an omission. The kinds
                   are namespaced and versioned by the server and
                   `forge.*` is still growing them, so a screen mapping
                   each to a sentence would be a second place every new
                   kind has to be learned — going stale where nobody is
                   looking, which is the trap #148 decision 14 names for
                   the projection. It costs a reader some fluency, and
                   means a kind this screen has never seen still arrives
                   intact. -->
              {#each sharedCatalog.ledger as event (event.event_id)}
                <li class="event">
                  <div class="event-head">
                    <span class="event-kind">{event.kind}</span>
                    <span class="event-when"
                      >{fmtDateTime(event.occurred_at_ms)}</span
                    >
                  </div>
                  <div class="event-who">
                    {event.actor_display_name}
                    <!-- The capacity, not just the name. An admin acting
                         inside a team without a membership row is stamped
                         as one and never disguised as a member (#83 §1). -->
                    <span class="event-kind-of-actor">{event.actor_kind}</span>
                  </div>
                  <button
                    type="button"
                    class="event-payload-toggle"
                    onclick={() =>
                      (openPayload =
                        openPayload === event.event_id ? null : event.event_id)}
                  >
                    {openPayload === event.event_id ? "hide" : "what it says"}
                  </button>
                  {#if openPayload === event.event_id}
                    <pre class="event-payload">{event.payload_json}</pre>
                  {/if}
                </li>
              {/each}
            </ul>
          {:else if sharedCatalog.ledgerRead && !sharedCatalog.ledgerLoading}
            <!-- Unreachable against a server behaving itself: founding a
                 team appends its own event, so a team that answered with
                 nothing answered wrongly. -->
            <p class="drawer-empty drawer-error">
              This team's ledger came back empty, which should not be
              possible — creating a team records itself.
            </p>
          {/if}

          <!-- The foot. A null cursor is not an end: the read says only
               that nothing lay past here when the page was taken, and a
               ledger has no final page. So neither branch below claims
               one. -->
          <div class="ledger-foot">
            {#if sharedCatalog.ledgerLoading}
              <p class="drawer-empty">reading…</p>
            {:else if !sharedCatalog.ledgerRead}
              <!-- Nothing has come back, so there is nothing to say
                   about what lies past it. A page that failed lands
                   here, under the error above. -->
              <button type="button" onclick={() => sharedCatalog.readLedgerPage()}>
                Read the ledger
              </button>
            {:else if sharedCatalog.ledgerCursor !== null}
              <button type="button" onclick={() => sharedCatalog.readLedgerPage()}>
                Read more
              </button>
            {:else}
              <button type="button" onclick={() => sharedCatalog.readLedgerPage()}>
                Ask again
              </button>
              <span class="drawer-empty">
                Nothing more had been recorded when this was read.
              </span>
            {/if}
          </div>
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
  .drawer-tabs {
    display: flex;
    gap: 0.15rem;
    margin: 0.8rem 0 0.2rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.12);
  }
  .drawer-tabs button {
    background: none;
    border: 0;
    border-bottom: 2px solid transparent;
    color: inherit;
    cursor: pointer;
    font-size: 0.8rem;
    padding: 0.3rem 0.55rem;
    opacity: 0.6;
  }
  .drawer-tabs button.active {
    opacity: 1;
    border-bottom-color: currentColor;
  }
  .make-team {
    background: none;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.78rem;
    padding: 0.35rem 0.6rem;
  }
  .roster .member {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    padding: 0.4rem 0.1rem;
    font-size: 0.78rem;
  }
  .roster .member.you {
    font-weight: 600;
  }
  .member-id {
    font-family: ui-monospace, monospace;
    font-size: 0.72rem;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .member-role {
    opacity: 0.6;
    font-size: 0.72rem;
    white-space: nowrap;
  }
  .ledger .event {
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    padding: 0.45rem 0.1rem;
    font-size: 0.78rem;
  }
  .event-head {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .event-kind {
    font-family: ui-monospace, monospace;
    font-size: 0.72rem;
  }
  .event-when,
  .event-kind-of-actor {
    opacity: 0.6;
    font-size: 0.72rem;
  }
  .event-who {
    display: flex;
    gap: 0.4rem;
    align-items: baseline;
    opacity: 0.85;
  }
  .event-payload-toggle {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    opacity: 0.6;
    padding: 0.1rem 0;
  }
  .event-payload {
    font-size: 0.7rem;
    margin: 0.2rem 0 0;
    padding: 0.35rem 0.45rem;
    background: rgba(255, 255, 255, 0.05);
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .ledger-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding-top: 0.7rem;
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
