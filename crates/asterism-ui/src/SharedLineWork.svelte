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
  // and the add that puts a removal back, which names content the line
  // is already holding for that entry rather than anything new.
  // `replace` is absent because it has nothing to name: choosing what
  // an entry becomes means choosing from the team's own content, and
  // this plane has no read of that to choose from.
  //
  // **The two answers above the rows are not read.** `ForgeWork` shows
  // how many landings have arrived since the work was cut and what it
  // collides with; the team server mirrors both routes and the desktop
  // has no command for either. So a close can be refused here by
  // something this surface never showed. That is a gap and is written
  // down as one — what closes it is two more commands, not a decision.
  //
  // **A refusal is its message and nothing more.** `ForgeWork` reads a
  // `reason` off the error and says what to do about it. A refusal
  // from a team server carries no such field on any arm — `teams_error`
  // in `commands.rs` maps every refusal it can name to the server's
  // own sentence — so there is nothing to key on, and inventing advice
  // from the prose would be guessing at which refusal arrived.
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
  import type { ForgeOpDto, ForgeRoundDto } from "./bindings";

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

  // Every write here is awaited inside a `catch` that does nothing,
  // and the nothing is the point: `mutate` has already put the refusal
  // on screen, and this plane has no second half to add to it — the
  // header says why there is no action behind a reason here. What the
  // catch buys is that a refusal is not also an unhandled rejection,
  // which `mutate` re-throws for callers that roll something back.
  // These have nothing to roll back: the list is re-read either way.
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
  // way back and is not offered one — and on this plane it could not
  // be given one either, since that would be naming content.
  async function putBack(row: ForgeProjectedEntry) {
    if (row.assetId === null) return;
    await ask({
      entry_id: row.entryId,
      kind: "add",
      content_asset_id: row.assetId,
      name: row.name ?? row.entryId,
    });
  }

  async function end(outcome: "satisfied" | "abandoned") {
    closing = true;
    try {
      await sharedCatalog.closePursuit(outcome, closeNote.trim());
      closeNote = "";
    } catch {
      // Said by `mutate`. The note is kept, because a close that was
      // refused is one somebody will press again.
    } finally {
      closing = false;
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  /** What to call an entry an operation names — the op's own name when
   *  it carries one, and otherwise whatever the fold has for it. */
  function opName(op: ForgeOpDto): string {
    return (
      op.name ??
      projected.find((row) => row.entryId === op.entry_id)?.name ??
      op.entry_id
    );
  }

  function summarise(round: ForgeRoundDto): string {
    return `${round.ops.length} ${round.ops.length === 1 ? "operation" : "operations"}`;
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
      <h4>Ended</h4>
      <ul class="work-list ended">
        {#each sharedCatalog.endedWork as item (item.id)}
          <li>
            <button onclick={() => sharedCatalog.selectPursuit(item.id)}>
              <span>{item.title ?? "(untitled)"}</span>
              <span class="quiet">{item.close?.outcome}</span>
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
      <h4>Open a pursuit</h4>
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
      {work.close === null ? "open" : work.close.outcome}
    </span>
  </header>

  {#if work.note !== null}
    <p class="quiet note">{work.note}</p>
  {/if}

  <!-- Only for work that can still move. The sentence is about a close
       that has not happened, and on ended work there is none to refuse
       — an abandoned pursuit can carry two names for one entry and
       nothing is waiting to reject it. -->
  {#if !ended && clashing.length > 0}
    <p class="quiet warn">
      Two entries would be named {clashing.join(", ")}. A line holds one live
      entry per name, so closing this will be refused until one is renamed.
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

  <!-- The log, oldest first: the order the chain holds and the order
       somebody reads a piece of work in. -->
  <h4>What was asked for</h4>
  <ol class="rounds">
    {#each work.rounds as round (round.id)}
      <li>
        <p class="round-head">
          <span>{summarise(round)}</span>
          <span class="quiet">{when(round.at_ms)}</span>
        </p>
        {#if round.note !== null}
          <p class="quiet note">{round.note}</p>
        {/if}
        <ul class="ops">
          {#each round.ops as op (op.entry_id + op.kind)}
            <li>
              <!-- The verb is stated rather than read off what moved:
                   an operation carries one, which a change row does
                   not. -->
              <span class="kind">{op.kind}</span>
              <span class="op-name">{opName(op)}</span>
            </li>
          {/each}
        </ul>
      </li>
    {/each}
  </ol>

  {#if work.rounds.length === 0}
    <p class="quiet">Nothing asked for yet.</p>
  {/if}

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
      Ended {when(work.close.at_ms)}{work.close.note !== null
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
  ul,
  ol {
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
    color: #ff9d9d;
    opacity: 1;
  }
  .warn {
    border-left: 2px solid rgba(220, 170, 90, 0.7);
    padding-left: 0.5rem;
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
    border: 1px solid rgba(255, 255, 255, 0.25);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    padding: 0.1rem 0.4rem;
  }
  .rounds > li {
    border-top: 1px solid rgba(255, 255, 255, 0.14);
    padding: 0.4rem 0;
  }
  .round-head {
    display: flex;
    gap: 0.6rem;
    margin: 0;
    font-size: 0.78rem;
  }
  .ops li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.78rem;
    padding: 0.15rem 0;
  }
  .kind {
    min-width: 4rem;
    opacity: 0.75;
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
    border-top: 1px solid rgba(255, 255, 255, 0.14);
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
    border: 1px solid rgba(255, 255, 255, 0.25);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    padding: 0.15rem 0.6rem;
  }
</style>
