<script lang="ts">
  // PromoteToTeam — handing this asset to a team, from the pane that is
  // showing it.
  //
  // The act #66 exists for, and the one way content reaches a team's
  // line: #148 decision 5 gives it exactly one entry point, a verb
  // scoped to open work, so the team never holds an asset that is not
  // attached to work.
  //
  // # Why it is here and not on the shared-lines drawer
  //
  // `shared.svelte.ts` argues the placement under "A promotion does not
  // start here" and leaves this surface the rest. What it does not
  // consider is the arrangement the local plane uses: `ForgeWork`
  // builds a round out of the grid selection, and the drawer could have
  // taken a promotion the same way, making one gesture where this makes
  // two. What decided against it is decision 7 — a `TeamAsset` is
  // minted per promotion — so a selection of five behind one press is
  // five acts that cannot be pressed twice, and nothing on the way
  // would have said so.
  //
  // # The target is picked here, and written through (#219)
  //
  // The three ids a promotion needs — team, line, pursuit — are
  // `sharedCatalog`'s, and this surface used to only read them: a
  // person set them up in the drawer, closed it, found the asset, and
  // pressed — and the pane's own answer when the drawer was not set up
  // was a sentence sending the person there. The destination is picked
  // here now, on the asset.
  //
  // What did not change is that there is one place naming the three.
  // The pickers below do not hold a team, a line or a pursuit of their
  // own — each writes the catalog (`lookAt`, `show`, `selectPursuit`,
  // `clearWork`) and reads it back, so the drawer follows: opened
  // after a promotion, it is already on that team, that line, that
  // work. A picker holding copies was the second place that could
  // disagree about where an asset was going, and this is not that.
  //
  // The target moves while this pane is mounted, and what the pickers
  // draw between a pick and the read that answers it is the catalog's
  // to say: `show` drops the last line's states, chain and work before
  // it reads the next, so no row here ever offers one line's work
  // against another, and `requireOpenWork` refuses the pairing if a
  // caller ever assembles it by hand. What the catalog says about a
  // read itself is its `answered` — an empty list before the read
  // lands is nobody having asked, not "none" — and every claim this
  // surface draws from a resource waits on that resource's: the rows,
  // the two empties, whether the team on is off the list, and the
  // Promote press, which waits on the Work row's.
  //
  // # Work is opened, not assumed
  //
  // A promotion needs open work, and this pane does not open one for
  // it. `Promotion::pursuit_id` on the client says why: a pursuit is
  // the record that a person chose to start work, and one opened as a
  // step of a promotion the team then refuses is a record of a
  // decision nobody made — an orphan a refused Tx would leave, with no
  // verb that takes it back without recording a second decision.
  //
  // So opening work for this entry is its own act, pressed on its
  // own: "Open work for this" calls the same `openPursuit` the
  // shared-lines drawer's own form does, titled with what the entry is
  // being called, and only once it has landed does Promote appear.
  // Choosing an existing piece of open work skips that press. Either
  // way the decision to have work open is the person's, made before
  // the promotion is, which is what makes a refused promotion cost
  // nothing beyond itself.
  //
  // With no session the drawer is still where a connection is made,
  // and the button opens it. #219 asked for the connect form in place;
  // that form carries the provider hand-off and the stored-sign-in
  // sentences, and a second copy here would be a copy to drift. What
  // this surface does instead is try the silent resume the drawer
  // would (#204), so a window that remembers its server offers a
  // target without the drawer ever opening.
  //
  // # What travels is said before it goes
  //
  // #171 asks for this in as many words. The rule is decision 4's and
  // the client's promotion module states it; what is new here is that
  // a person reads it, in their own words, **above** the control rather
  // than beside the result — it is a thing to know before pressing.
  //
  // # What this does not pre-empt
  //
  // A Collection and a multi-material asset are refused by the client,
  // each with a message naming which of the two it is and why the
  // team's shape for it is undecided (decision 3). This surface does
  // not ask those questions first: the refusal's message is the whole
  // answer, and a second copy of that rule here would be a copy to
  // drift. `mutate` puts it on screen.
  import { untrack } from "svelte";
  import { sharedCatalog } from "./lib/stores/shared.svelte";
  import type { PromotedAssetDto } from "./bindings";

  let { assetId, defaultName }: { assetId: string; defaultName: string } =
    $props();

  // What the entry answers to on the line — the caller's, not the
  // asset's title. A line's names are the team's namespace, and what
  // somebody called a thing in their own library is not automatically
  // what it should be called on somebody else's line. Seeded from the
  // title because that is the likeliest answer, and editable because it
  // is a guess.
  let named = $state("");
  let sending = $state(false);
  let promoted = $state<PromotedAssetDto | null>(null);

  // Reset on the asset rather than on the name, so a pane moved to
  // another asset does not show the last one's outcome under it — and
  // so that renaming the asset in the pane above does not overwrite a
  // half-typed name here. That is why the seed is read untracked: this
  // effect is about which asset is showing.
  //
  $effect(() => {
    void assetId;
    promoted = null;
    named = untrack(() => defaultName);
  });

  // What a target needs — a session, silently resumed if there is one
  // to resume, the teams, the lines and the work — is asked for when
  // the asset changes and whenever the target's preconditions do: the
  // session, the team on, the line on. Not on the asset alone: a
  // disconnect drops every read and keeps the team, and a pane that
  // asked only per asset would sit on "this team hosts no lines" for
  // a team it never re-read. The call is untracked because what it
  // reads it also writes; the three preconditions are read tracked so
  // that changing one runs it.
  $effect(() => {
    void assetId;
    void sharedCatalog.session;
    void sharedCatalog.teamId;
    void sharedCatalog.selected;
    void untrack(() => sharedCatalog.readyForPromotion());
  });

  // The outcome is about the target it was sent to. The target moves
  // without the pane moving now, so a change of team, line or work
  // takes the last outcome down with it rather than leaving its ids
  // under a target they never went to. A promotion itself touches
  // none of the three — the work has to be open already (#219) — so
  // this effect does not fire across a promote, and the outcome it
  // reports stays up.
  $effect(() => {
    void sharedCatalog.teamId;
    void sharedCatalog.selected;
    void sharedCatalog.working;
    promoted = null;
  });

  const teams = $derived(sharedCatalog.teams.data);
  // The team the catalog is on may be one the list does not hold: the
  // instance admin reaches a team by id without a membership row.
  // Offered as itself rather than dropped, so the pane never shows a
  // team other than the one the catalog is on. A claim about the list,
  // so it waits on the list: until the teams have answered, the team
  // on is neither in the list nor off it, and the row says only which
  // team it is.
  const teamOffList = $derived(
    sharedCatalog.teams.answered &&
      sharedCatalog.teamId !== "" &&
      !teams.some((team) => team.team_id === sharedCatalog.teamId),
  );
  // On a team while the teams list has not answered — a window the
  // catalog itself makes, since a disconnect keeps the team and drops
  // the reads made for it.
  const teamUnread = $derived(
    !sharedCatalog.teams.answered && sharedCatalog.teamId !== "",
  );
  const line = $derived(
    sharedCatalog.lines.data.find((one) => one.id === sharedCatalog.selected) ??
      null,
  );
  const work = $derived(sharedCatalog.work);
  const ended = $derived(work !== null && work.close !== null);
  // What the work picker shows: the open pursuit the catalog is on, or
  // nothing when it is on none — or on one that has ended, which this
  // surface does not offer and so reads as none. Empty means "not
  // chosen yet", not "let the write open one" — there is no such
  // choice here (#219).
  const workChoice = $derived(work !== null && !ended ? work.id : "");
  // No team on, or no line on it, and a read that failed says so
  // rather than reading as an empty answer — the catalog's header
  // refuses that merge, and the drawer draws the error first too.
  // And a read that has not answered is not an empty answer either:
  // every claim here drawn from a resource waits on its `answered`,
  // so "no teams" and "no lines" are only ever said of a list that
  // was asked for.
  const noTeams = $derived(
    sharedCatalog.teams.answered && !sharedCatalog.teams.error &&
      teams.length === 0 && !teamOffList,
  );
  const noLines = $derived(
    sharedCatalog.phase === "ready" && sharedCatalog.lines.answered &&
      !sharedCatalog.lines.error && sharedCatalog.lines.data.length === 0,
  );

  async function pickTeam(event: Event) {
    const teamId = (event.currentTarget as HTMLSelectElement).value;
    if (teamId === "" || teamId === sharedCatalog.teamId) return;
    await sharedCatalog.lookAt(teamId);
  }

  async function pickLine(event: Event) {
    const lineId = (event.currentTarget as HTMLSelectElement).value;
    if (lineId === "" || lineId === sharedCatalog.selected) return;
    await sharedCatalog.show(lineId);
  }

  function pickWork(event: Event) {
    const pursuitId = (event.currentTarget as HTMLSelectElement).value;
    if (pursuitId === "") sharedCatalog.clearWork();
    else sharedCatalog.selectPursuit(pursuitId);
  }

  let opening = $state(false);

  // Opens work for this entry, on its own press: the decision to have
  // work open belongs to the person, made before the promotion is, and
  // this is the same `openPursuit` the shared-lines drawer's own form
  // calls. Titled with what the entry is being called, since that is
  // what the work exists to carry — editable up there, not asked twice
  // here.
  async function openWorkForThis(event: Event) {
    event.preventDefault();
    opening = true;
    try {
      await sharedCatalog.openPursuit(named.trim(), "");
    } catch {
      // Said by `mutate`, the same as the drawer's own form.
    } finally {
      opening = false;
    }
  }

  async function promote(event: Event) {
    event.preventDefault();
    sending = true;
    try {
      promoted = await sharedCatalog.promote(assetId, named.trim());
    } catch {
      // Two kinds of failure arrive here and neither wants anything
      // added. A refusal from the team is already on screen — `mutate`
      // puts it there, and the client's message names which refusal it
      // is. A refusal from the catalog's own guards is not, and cannot
      // be: those fire when a caller skipped what this surface checks
      // above, so what they say is for a stack trace rather than for a
      // person. Caught so that neither becomes an unhandled rejection.
    } finally {
      sending = false;
    }
  }
</script>

<section class="promote">
  <h4>Hand to a team</h4>

  {#if sharedCatalog.session === null}
    <p class="quiet">
      Not connected to a team server. The team drawer is where a
      connection is made.
    </p>
    <button type="button" onclick={() => sharedCatalog.openPanel()}>
      open the team drawer
    </button>
  {:else}
    <!-- The target, top down: which team, which of its lines, which
         work against that line. Each row is the catalog's answer, and
         changing one writes the catalog — which is why the rows below
         it re-read rather than being cleared here. -->
    <div class="target">
      <label>
        Team
        {#if !sharedCatalog.teams.answered && !teamUnread}
          <span class="quiet">reading your teams…</span>
        {:else if sharedCatalog.teams.error && teams.length === 0}
          <span class="quiet">
            Could not read your teams: {sharedCatalog.teams.error}
          </span>
        {:else if noTeams}
          <span class="quiet">
            You are not a member of any team on this server. Founding
            one, or opening one by id, starts from the team
            drawer.
          </span>
        {:else}
          <select class="pick-team" value={sharedCatalog.teamId} onchange={pickTeam}>
            <option value="" disabled>choose…</option>
            {#each teams as team (team.team_id)}
              <option value={team.team_id} title={team.team_id}>{team.name ?? team.team_id} · {team.role}</option>
            {/each}
            {#if teamOffList}
              <option value={sharedCatalog.teamId}
                >{sharedCatalog.teamId} · opened by id</option
              >
            {:else if teamUnread}
              <!-- The team on, by id alone: how it was reached — a
                   membership row, or by id without one — is the
                   list's to say, and the list has not answered. -->
              <option value={sharedCatalog.teamId}>{sharedCatalog.teamId}</option>
            {/if}
          </select>
        {/if}
      </label>

      {#if sharedCatalog.phase === "ready"}
        <label>
          Line
          {#if !sharedCatalog.lines.answered}
            <span class="quiet">reading the team's lines…</span>
          {:else if sharedCatalog.lines.error}
            <span class="quiet">
              Could not read the team's lines: {sharedCatalog.lines.error}
            </span>
          {:else if noLines}
            <span class="quiet">
              This team hosts no lines. Publishing one starts from the
              team drawer.
            </span>
          {:else}
            <select
              class="pick-line"
              value={sharedCatalog.selected ?? ""}
              onchange={pickLine}
            >
              <option value="" disabled>choose…</option>
              {#each sharedCatalog.lines.data as one (one.id)}
                <option value={one.id}>{one.name} · {one.standing}</option>
              {/each}
            </select>
          {/if}
        </label>
      {/if}

      {#if line !== null}
        <label>
          Work
          <!-- Nothing offered until the line's work has been read: an
               empty list before the read answers is not "no open
               work", it is nobody having asked yet, which the
               catalog's `answered` keeps apart from the answer
               (#219). Only existing work is a choice here — opening
               one is its own act, below. -->
          {#if !sharedCatalog.pursuits.answered}
            <span class="quiet">reading the work against this line…</span>
          {:else if sharedCatalog.pursuits.error}
            <span class="quiet">
              Could not read the work against this line:
              {sharedCatalog.pursuits.error}
            </span>
          {:else if sharedCatalog.openWork.length === 0}
            <span class="quiet">Nothing open against this line yet.</span>
          {:else}
            <select class="pick-work" value={workChoice} onchange={pickWork}>
              <option value="" disabled>choose…</option>
              {#each sharedCatalog.openWork as item (item.id)}
                <option value={item.id}>{item.title ?? "(untitled work)"}</option>
              {/each}
            </select>
          {/if}
        </label>
      {/if}
    </div>

    {#if noTeams || noLines}
      <button type="button" onclick={() => sharedCatalog.openPanel()}>
        open the team drawer
      </button>
    {:else if line !== null && sharedCatalog.pursuits.answered && !sharedCatalog.pursuits.error}
      <label class="call-it">
        Call it, on the line
        <input type="text" bind:value={named} required />
      </label>

      {#if workChoice === ""}
        <!-- No work chosen: the decision to have one open is the
             person's, pressed on its own rather than folded into
             Promote (#219) — see the header for why. -->
        <p class="quiet travels">
          A promotion goes onto open work. Pick a piece above, or open
          one for this entry.
        </p>
        <form class="open-work" onsubmit={openWorkForThis}>
          <button type="submit" disabled={opening || named.trim() === ""}>
            {opening ? "Opening…" : "Open work for this"}
          </button>
        </form>
      {:else}
        <p class="quiet where">
          Onto <strong>{work?.title ?? "(untitled work)"}</strong>, against
          <strong>{line.name}</strong>.
        </p>

        <!-- Before the control, because it is a thing to know before
             pressing rather than after. -->
        <p class="quiet travels">
          What goes: the file itself, and the marks you wrote on it. What
          stays: thumbnails, anything indexed from the file, and marks the
          import or a machine made — the team can make those again.
        </p>

        <form class="do-promote" onsubmit={promote}>
          <button type="submit" disabled={sending || named.trim() === ""}>
            {sending ? "Handing over…" : "Promote"}
          </button>
        </form>
      {/if}
    {/if}

    {#if promoted !== null}
      <!-- What only the promotion knows. `sharedCatalog.said` carries
           the sentence; this carries the three facts a person may want
           to check afterwards. Outside the form's own branch: the
           promotion's last act is a re-read of the work, and a re-read
           that failed must not take the answer down with it — the
           entry was pushed whether or not the list could be read
           back. -->
        <dl class="outcome">
          {#if promoted.already_promoted}
            <div>
              <dt>Sent</dt>
              <dd>
                Nothing. This machine had already promoted it onto this line —
                which says what this machine did rather than what the team
                holds.
              </dd>
            </div>
            <!-- Labelled for what it is on this path. The client hashes
                 before it reads the relation, so this is the file as it
                 is now — not what the team took, which was hashed
                 whenever the first promotion happened. -->
            <div>
              <dt>This file, now</dt>
              <dd class="mono">{promoted.digest}</dd>
            </div>
          {:else}
            <div>
              <dt>Entry</dt>
              <dd class="mono">{promoted.entry_id}</dd>
            </div>
            <div>
              <dt>The team's copy</dt>
              <dd class="mono">{promoted.team_asset_id ?? "—"}</dd>
            </div>
            <div>
              <dt>Digest</dt>
              <dd class="mono">{promoted.digest}</dd>
            </div>
          {/if}
          <div>
            <dt>Already there</dt>
            <dd>
              <!-- Three states, and the third is not "no". Nobody asked on
                   a repeat, because nothing was going to be sent. -->
              {#if promoted.bytes_already_held === null}
                not asked
              {:else if promoted.bytes_already_held}
                the team already held these bytes
              {:else}
                the team did not have these bytes
              {/if}
            </dd>
          </div>
        </dl>
    {/if}
  {/if}
</section>

<style>
  .promote {
    border-top: 1px solid var(--line);
    margin-top: 0.9rem;
    padding-top: 0.7rem;
  }
  h4 {
    margin: 0 0 0.3rem;
    font-size: 0.82rem;
    font-weight: 500;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.78rem;
    line-height: 1.45;
    margin: 0.3rem 0;
  }
  .target {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin: 0.4rem 0;
  }
  .target select {
    width: 100%;
    box-sizing: border-box;
    font-size: 0.78rem;
  }
  .where strong {
    opacity: 1;
    font-weight: 600;
  }
  .travels {
    border-left: 2px solid var(--line);
    padding-left: 0.5rem;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 0.5rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    opacity: 0.85;
  }
  button {
    align-self: flex-start;
    background: none;
    border: 1px solid var(--line);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.78rem;
    padding: 0.15rem 0.6rem;
  }
  .outcome {
    margin: 0.6rem 0 0;
    font-size: 0.75rem;
  }
  .outcome div {
    display: flex;
    gap: 0.5rem;
    padding: 0.1rem 0;
  }
  .outcome dt {
    flex: 0 0 7rem;
    opacity: 0.6;
  }
  .outcome dd {
    margin: 0;
  }
  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.7rem;
    overflow-wrap: anywhere;
  }
</style>
