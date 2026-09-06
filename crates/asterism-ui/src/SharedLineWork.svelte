<script lang="ts">
  // SharedLineWork — a piece of work against a team's line, open to close.
  //
  // The shared plane's half of `ForgeWork`, and a second component
  // rather than that one taking props. #148 decision 16 keeps the two
  // catalogs apart because their sources differ, and `ForgeWork` is
  // bound to `forgeCatalog` in every branch it has — line, work,
  // rounds, cards, collisions, conversations. What the two surfaces
  // genuinely share is the fold, and that is shared:
  // `lib/forge-projection.ts` is one copy, used by both.
  //
  // What is the same. A round is a request and nothing lands until a
  // close with `satisfied`; the log is the editor, so a correction is
  // another round; the rows are what a close would leave, folded from
  // the line and the work together because neither log answers it
  // alone. Every one of those is the model's rather than a screen's,
  // and the model is mirrored (decision 19).
  //
  // What is different, and each difference is a verb this plane does
  // not have rather than a choice made here:
  //
  // **Nothing new can be added.** `ForgeWork` builds a round out of a
  // grid selection, and a grid selection is local assets. Putting one
  // on a team's line is content entering the team — `enter_content`,
  // scoped to open work by #148 decision 5 — which is the promotion,
  // #198's sibling and out of its scope. So the verbs here are the
  // three that name entries the line already holds: rename, remove,
  // and the add that puts a removal back, which names the content the
  // row already carries rather than anything new to the team.
  // `replace` is absent because it has nothing to name: choosing what
  // an entry becomes means choosing from the team's own content, and
  // this plane has no read of that to choose from.
  //
  // **The two answers above the rows, read the way `ForgeWork` reads
  // them (#211).** How many landings have arrived since the work was
  // cut, and what it collides with — the team server mirrored both
  // routes from the day the forge was hosted, and until this issue the
  // desktop carried no verb for either, so a close could be refused
  // here by something this surface never showed. `sharedCatalog`'s
  // `collisions` and `behind` are the same two `Resource`s
  // `forgeCatalog` keeps, over the same two commands mirrored path for
  // path (decision 19); this component draws them the same way,
  // without the one action `ForgeWork` offers under them — this plane
  // carries no verb that lets a line's rule settle a collision, so the
  // list is read-only here.
  //
  // **A refusal's reason is read the way `ForgeWork` reads it.** The
  // forge's conflicts answer with `blocked` / `raced` / `settled` /
  // `clashes`, `TeamsClientError::Refused` holds the token on the way
  // off the wire, and `teams_error` in `commands.rs` now keeps it
  // rather than dropping it under `..` — a shared refusal crosses as
  // the same `UiError::Conflict { message, reason }` a local one does,
  // and `whatToDo` below reads it the way `ForgeWork`'s does. `blocked`
  // reads differently on this plane: there is no rule to ask to settle
  // a collision from here, only the list above to look at, and no
  // archive/reopen verb on a shared line at all.
  //
  // **No pictures, and no conversations.** An entry names content the
  // team holds, which this machine has no copy of — the contents tab
  // is where cloning takes one. And the member's client does not carry
  // the thread verbs, which the catalog's header records.
  // No `line` prop, unlike `ForgeWork`. Every write here names the
  // line through the catalog's own `selected`, which is where the
  // catalog itself reads it — a prop would be a second answer to a
  // question the catalog already answers, and the panel would be the
  // place they could disagree.
  import { sharedCatalog } from "./lib/stores/shared.svelte";
  import type { ForgeProjectedEntry } from "./lib/forge-projection";
  import { promptCatalog } from "./lib/stores/prompt.svelte";
  import type { ForgeOpDto } from "./bindings";
  import { endingWord } from "./lib/formatters";
  import ForgeRoundLog from "./ForgeRoundLog.svelte";

  const work = $derived(sharedCatalog.work);
  const ended = $derived(work !== null && work.close !== null);
  const projected = $derived(sharedCatalog.projection);
  const clashing = $derived(sharedCatalog.wouldClash);

  // The new-work form and the note carried into a close. Component
  // state, because a half-typed sentence is nobody else's business.
  let newTitle = $state("");
  let newNote = $state("");
  let opening = $state(false);
  let closeNote = $state("");
  let closing = $state(false);

  // What the last close left to say that the toast does not — the
  // action behind a reason. `ForgeWork`'s own field, by the same name
  // and for the same one write: every other one here still catches
  // into nothing, because `mutate` has already put the refusal on
  // screen and none of them refuse for a reason this surface can act
  // on.
  let said = $state<string | null>(null);

  // Every write but the close is awaited inside a `catch` that does
  // nothing, and the nothing is the point: `mutate` has already put
  // the refusal on screen, and this plane has no second half to add to
  // it for these. What the catch buys is that a refusal is not also an
  // unhandled rejection, which `mutate` re-throws for callers that roll
  // something back. These have nothing to roll back: every write here
  // reaches the catalog before it touches anything on screen, so a
  // refused one leaves the surface exactly as it was.
  async function open(event: Event) {
    event.preventDefault();
    opening = true;
    try {
      await sharedCatalog.openPursuit(newTitle.trim(), newNote.trim());
      newTitle = "";
      newNote = "";
    } catch {
      // Said by `mutate`.
    } finally {
      opening = false;
    }
  }

  /** One operation, its own round — the log is the editor. */
  async function ask(op: ForgeOpDto) {
    try {
      await sharedCatalog.pushRound([op], "");
    } catch {
      // Said by `mutate`.
    }
  }

  async function rename(row: ForgeProjectedEntry) {
    const name = await promptCatalog.open(
      "Name this entry",
      "a name",
      row.name ?? "",
    );
    if (name === null || name.trim() === "") return;
    await ask({
      entry_id: row.entryId,
      kind: "rename",
      content_asset_id: null,
      name: name.trim(),
    });
  }

  async function remove(row: ForgeProjectedEntry) {
    await ask({
      entry_id: row.entryId,
      kind: "remove",
      content_asset_id: null,
      name: null,
    });
  }

  // Putting a removal back is adding *that* entry, which is why this
  // sends the row's own id rather than minting one: an entry that comes
  // back under a new id is a new arrival, and the record would say so.
  // It needs something to hold, so a row the line never filled has no
  // way back and is not offered one — and this plane has nothing to
  // give it, for the reason `replace` is absent: choosing content for
  // a row means choosing from the team's own, and there is no read of
  // that here.
  async function putBack(row: ForgeProjectedEntry) {
    if (row.assetId === null) return;
    await ask({
      entry_id: row.entryId,
      kind: "add",
      content_asset_id: row.assetId,
      name: row.name ?? row.entryId,
    });
  }

  // The action behind a reason, read the way `ForgeWork`'s `whatToDo`
  // reads it — same four tokens, `blocked`'s message the one that
  // differs, for the reason the header gives.
  function whatToDo(error: unknown): string | null {
    const reason = (error as { reason?: string })?.reason;
    switch (reason) {
      case "blocked":
        return "Something has to change first — look at what this collides with, above. If the list above is empty, the line itself has been archived, which nothing on this screen can undo.";
      case "raced":
        return "A landing arrived while this was being written. Closing again will usually take.";
      case "settled":
        return "This work has already ended.";
      case "clashes":
        return "The line would end up with two entries under one name. Rename one of them below and close again.";
      default:
        return null;
    }
  }

  async function end(outcome: "satisfied" | "abandoned") {
    closing = true;
    said = null;
    try {
      await sharedCatalog.closePursuit(outcome, closeNote.trim());
      closeNote = "";
    } catch (error) {
      // `mutate` has already put the refusal on screen; this adds the
      // half it does not read. The note is kept, because a close that
      // was refused is one somebody will press again.
      said = whatToDo(error);
    } finally {
      closing = false;
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }
</script>

{#if sharedCatalog.working === null}
  <!-- No work open: what there is against this line, and the form that
       starts some. Read with the line rather than on demand — the
       catalog's `show` loads all three of a line's answers together. -->
  {#if sharedCatalog.pursuits.loading}
    <p class="quiet">Reading…</p>
  {:else if sharedCatalog.pursuits.error}
    <p class="quiet error">
      Could not read the work against this line: {sharedCatalog.pursuits.error}
    </p>
  {:else}
    {#if sharedCatalog.openWork.length > 0}
      <ul class="work-list">
        {#each sharedCatalog.openWork as item (item.id)}
          <li>
            <button onclick={() => sharedCatalog.selectPursuit(item.id)}>
              <span>{item.title ?? "(untitled)"}</span>
              <span class="quiet">{when(item.opened_at_ms)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="quiet">No work open against this line.</p>
    {/if}

    {#if sharedCatalog.endedWork.length > 0}
      <h4>Closed</h4>
      <ul class="work-list ended">
        {#each sharedCatalog.endedWork as item (item.id)}
          <li>
            <button onclick={() => sharedCatalog.selectPursuit(item.id)}>
              <span>{item.title ?? "(untitled)"}</span>
              <span class="quiet">{item.close ? endingWord(item.close.outcome) : ""}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <!-- Nothing is copied first. Decision 10: working on a shared line
         needs no clone, so this is the verb a member reaches for and
         the clone button on the contents tab is for taking a copy
         home. -->
    <form class="new-work" onsubmit={open}>
      <h4>Open work</h4>
      <label>
        Title
        <input type="text" bind:value={newTitle} placeholder="optional" />
      </label>
      <label>
        Why
        <input type="text" bind:value={newNote} placeholder="optional" />
      </label>
      <button type="submit" disabled={opening}>
        {opening ? "Opening…" : "Open"}
      </button>
    </form>
  {/if}
{:else if sharedCatalog.pursuits.loading && work === null}
  <p class="quiet">Reading…</p>
{:else if work === null && sharedCatalog.pursuits.error}
  <!-- The list emptied because the read failed, which is the other way
       `Resource` answers with nothing: it puts the reason on `.error`
       and the data back to its initial. Asked before the sentence
       below, because that one is a claim about what the line has and
       this is a read that never got an answer. -->
  <p class="quiet error">
    Could not re-read the work against this line: {sharedCatalog.pursuits
      .error}
    <button type="button" onclick={() => sharedCatalog.clearWork()}>
      back to all work
    </button>
  </p>
{:else if work === null}
  <!-- Selected out of the list, and the read that emptied it succeeded,
       so the list stopped carrying it — somebody else's write moved
       what this line has. -->
  <p class="quiet">
    That work is no longer in this line's list.
    <button type="button" onclick={() => sharedCatalog.clearWork()}>
      back to all work
    </button>
  </p>
{:else}
  <header class="work-head">
    <button class="back" onclick={() => sharedCatalog.clearWork()}>
      ← all work
    </button>
    <strong>{work.title ?? "(untitled)"}</strong>
    <span class="quiet">
      {work.close === null ? "open" : endingWord(work.close.outcome)}
    </span>
  </header>

  {#if work.note !== null}
    <p class="quiet note">{work.note}</p>
  {/if}

  {#if !ended}
    <!-- Both answers are about work that can still move, on
         `ForgeWork`'s own reasoning: ended work collides with nothing
         anybody can act on, and a satisfied close leaves it behind its
         own landing — a true number about a question nobody is asking
         any more (#211). -->
    {#if sharedCatalog.behind.data.length > 0}
      <p class="quiet">
        {sharedCatalog.behind.data.length}
        {sharedCatalog.behind.data.length === 1 ? "landing" : "landings"} on the
        line since this was cut.
      </p>
    {/if}

    {#if sharedCatalog.collisions.data.length > 0}
      <div class="collisions">
        <h4>Collides with the line</h4>
        <ul>
          {#each sharedCatalog.collisions.data as hit (hit.entry_id + hit.axis)}
            <li>
              <span>
                {projected.find((row) => row.entryId === hit.entry_id)?.name ??
                  hit.entry_id}
              </span>
              <span class="quiet">{hit.axis}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  {/if}

  <!-- Only for work that can still move. The sentence is about a close
       that has not happened, and ended work has none coming — an
       abandoned pursuit can leave two entries under one name, with
       nothing left to refuse it. -->
  {#if !ended && clashing.length > 0}
    <p class="quiet warn">
      Two entries would be named {clashing.join(", ")}. A line holds one live
      entry per name, so closing this will be refused until one is renamed.
    </p>
  {/if}

  {#if said !== null}
    <p class="said">
      {said}
      <button type="button" onclick={() => (said = null)}>dismiss</button>
    </p>
  {/if}

  {#if !ended}
    <!-- What closing would leave, and where the verbs are pressed. Only
         for work that can still move: an abandoned close lands nothing,
         and after a satisfied one the contents tab is the answer.

         Rows this work takes off the line stay listed and dimmed, with
         the name the line has for them rather than anything the work
         said on the way off — that is what a removal leaves, and it is
         what putting one back has to put back. -->
    <h4>The line, as this would leave it</h4>
    <ul class="projected">
      {#each projected as row (row.entryId)}
        <li class:gone={!row.alive}>
          <span class="op-name">{row.name ?? "(unnamed)"}</span>
          <span class="row-verbs">
            <!-- `stated` rather than `alive`, which is the distinction
                 `ForgeProjectedEntry` is defined on: this asks what a
                 close will look at, and a rename on a row the work has
                 said absent is not among it. -->
            {#if row.stated !== "absent"}
              <button type="button" onclick={() => rename(row)}>rename</button>
            {/if}
            {#if row.alive}
              <button type="button" onclick={() => remove(row)}>remove</button>
            {:else if row.assetId !== null}
              <button type="button" onclick={() => putBack(row)}>put back</button>
            {/if}
          </span>
        </li>
      {/each}
    </ul>
    {#if projected.length === 0}
      <p class="quiet">Nothing on the line, and nothing asked for yet.</p>
    {/if}

    <!-- Said where somebody would otherwise look for the control: this
         surface can move what the line holds and cannot bring anything
         to it. -->
    <p class="quiet">
      Adding content to a team's line is a promotion, which starts from
      an asset rather than from here. The verbs above name entries this
      line already holds.
    </p>
  {/if}

  <!-- The log, factored out to `ForgeRoundLog` (#217): identical to
       `ForgeWork`'s copy except for the "say something" verb, which
       this plane does not carry (no `onTalkAboutRound`/`onTalkAboutOp`
       — the member's client has no thread commands), and the divider
       colour, which the two files never shared either. -->
  <ForgeRoundLog
    rounds={work.rounds}
    {projected}
    dividerColor="var(--wash-up-strong)"
  />

  {#if !ended}
    <div class="close">
      <label>
        Note
        <input type="text" bind:value={closeNote} placeholder="optional" />
      </label>
      <button type="button" disabled={closing} onclick={() => end("satisfied")}>
        close · put it on the line
      </button>
      <button type="button" disabled={closing} onclick={() => end("abandoned")}>
        close · abandon
      </button>
    </div>
  {:else if work.close !== null}
    <p class="quiet">
      Closed {when(work.close.at_ms)}{work.close.note !== null
        ? ` — ${work.close.note}`
        : ""}
    </p>
  {/if}
{/if}

<style>
  h4 {
    margin: 0.9rem 0 0.3rem;
    font-size: 0.82rem;
    font-weight: 500;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.78rem;
    margin: 0.3rem 0;
  }
  .error {
    color: var(--danger-ink);
    opacity: 1;
  }
  .warn {
    border-left: 2px solid var(--warning-line);
    padding-left: 0.5rem;
  }
  .collisions {
    border-left: 2px solid var(--danger-line);
    padding-left: 0.5rem;
    margin: 0.5rem 0;
  }
  .collisions li {
    display: flex;
    gap: 0.5rem;
    font-size: 0.78rem;
  }
  .said {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.78rem;
    border-left: 2px solid var(--line-strong);
    padding-left: 0.5rem;
  }
  .said button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    opacity: 0.7;
    padding: 0;
    text-decoration: underline;
  }
  .work-list button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    display: flex;
    gap: 0.6rem;
    padding: 0.25rem 0;
    text-align: left;
    width: 100%;
  }
  .work-list.ended button {
    opacity: 0.6;
  }
  .work-head {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    margin-bottom: 0.3rem;
  }
  .back {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    opacity: 0.75;
    padding: 0;
  }
  .note {
    font-style: italic;
  }
  .projected li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.78rem;
    padding: 0.15rem 0;
  }
  /* Off the line after this lands, drawn the way the contents tab draws
     what a line let go. */
  .projected li.gone {
    opacity: 0.55;
    text-decoration: line-through;
  }
  .row-verbs {
    display: flex;
    gap: 0.3rem;
    margin-left: auto;
  }
  .row-verbs button,
  .close button,
  .quiet button {
    background: none;
    border: 1px solid var(--line-strong);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    padding: 0.1rem 0.4rem;
  }
  .op-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-top: 0.6rem;
    flex-wrap: wrap;
  }
  .close button {
    font-size: 0.78rem;
    padding: 0.15rem 0.5rem;
  }
  .new-work {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 1rem;
    padding-top: 0.7rem;
    border-top: 1px solid var(--line);
  }
  .new-work label,
  .close label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    opacity: 0.85;
  }
  .new-work button {
    align-self: flex-start;
    background: none;
    border: 1px solid var(--line-strong);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    padding: 0.15rem 0.6rem;
  }
</style>
