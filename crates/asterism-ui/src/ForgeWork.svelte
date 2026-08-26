<script lang="ts">
  // ForgeWork — a piece of work against a line, from open to close.
  //
  // The half of the forge that proposes. `ForgePanel` shows what a line
  // says and how it got there; this shows what somebody is asking it to
  // say, which is a separate log the model keeps apart — the reasoning
  // is in `lib/stores/forge.svelte.ts`, beside the reads. This header
  // says what this surface decides for itself.
  //
  // **Nothing here is on the line.** A round is a request, and the only
  // moment anything lands is a close with `satisfied`. So the rounds
  // below are drawn as a log rather than as contents, and the panel's
  // contents tab is the place that answers what the line holds.
  //
  // **The log is the editor.** There is no staging area between picking
  // images and writing a round: pressing add writes one. A correction
  // is another round, which is what the model stores anyway — a staging
  // area would be a second, private copy of a log that already exists,
  // and the first thing it would need is a way to say what is in it.
  //
  // **Content comes from a grid selection; the other three verbs are
  // pressed on an entry.** The grid is where a person picks images, and
  // `gridSelection` is the same set dispatch and snapshot read. The
  // forge is an overlay, so the picking happens before it opens. What
  // that gesture cannot express is which existing entry to rename,
  // refill or take off — those name something the work or the line
  // already has, so they sit on the row rather than in the grid.
  //
  // **The projection is what those rows are.** A person deciding
  // whether to close needs to see what closing would leave, which
  // neither log answers on its own. The fold is `forgeCatalog`'s, made
  // where both reads are; what it means here is that the rows are a
  // picture of a line that has not moved, and the two answers above
  // them are how far that assumption has drifted.
  //
  // **Where a refusal carries a reason, the reason is not the action.**
  // `mutate` puts the message on screen and reads no further, and what
  // to do about each reason is set out on `close_forge_pursuit` in
  // `asterism-server`'s `http` module. Not every refusal has one — a
  // close that would change nothing is a validation refusal and its
  // message is the whole answer.
  import { api } from "./lib/api";
  import { forgeCatalog } from "./lib/stores/forge.svelte";
  import type { ForgeProjectedEntry } from "./lib/stores/forge.svelte";
  import { gridSelection } from "./lib/stores/grid-selection.svelte";
  import { promptCatalog } from "./lib/stores/prompt.svelte";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";
  import { baseName } from "./lib/basename";
  import type {
    AssetCardDto,
    ForgeLineDto,
    ForgeOpDto,
    ForgeRoundDto,
  } from "./bindings";

  let { line }: { line: ForgeLineDto } = $props();

  const work = $derived(forgeCatalog.pursuit.data);
  const ended = $derived(work !== null && work.close !== null);

  // What a close would leave, and which names would be on the line
  // twice if it did. Both are folds over the two logs and both live in
  // the catalog, beside the reads they are made of.
  const projected = $derived(forgeCatalog.projection);
  const clashing = $derived(forgeCatalog.wouldClash);

  // The new-work form, and the note carried into a close. Component
  // state because a half-typed sentence is nobody else's business.
  let newTitle = $state("");
  let newNote = $state("");
  let opening = $state(false);
  let closeNote = $state("");
  let closing = $state(false);
  let adding = $state(false);

  // What the last write left to say that nothing else says. `mutate`
  // shows every refusal itself; what it does not show is the action
  // behind a reason, a rule that declined without failing, or a
  // selection that named ids the library did not answer for. Cleared by
  // the next write, because it answers about that one.
  let said = $state<string | null>(null);

  async function open(event: Event) {
    event.preventDefault();
    opening = true;
    said = null;
    try {
      await forgeCatalog.openPursuit(
        line.id,
        newTitle.trim() === "" ? null : newTitle.trim(),
        newNote.trim() === "" ? null : newNote.trim(),
      );
      newTitle = "";
      newNote = "";
    } finally {
      opening = false;
    }
  }

  // The selection becomes a round.
  //
  // The cards are read rather than taken from the grid's page: what is
  // selected outlives a page, and a light card carries a placeholder
  // where `source_locator` will be. `hydrate_cards` drops ids the
  // library does not have or will not show, so the count is checked
  // rather than assumed — adding four of five silently is the kind of
  // quiet loss the forge exists to make impossible.
  //
  // The entry id is minted here rather than by the model, which
  // `ForgeOpDto.entry_id` requires and explains.
  async function addSelection() {
    if (work === null) return;
    const ids = [...gridSelection.selectedIds];
    if (ids.length === 0) return;
    adding = true;
    said = null;
    try {
      const cards = await api<AssetCardDto[]>("hydrate_cards", {
        ids,
        viewerSubject: null,
      });
      if (cards.length === 0) {
        said = "The library did not answer for any of the selected assets.";
        return;
      }
      const ops: ForgeOpDto[] = cards.map((card) => ({
        entry_id: crypto.randomUUID(),
        kind: "add",
        content_asset_id: card.id,
        name: baseName(card.source_locator) || card.id,
      }));
      await forgeCatalog.pushRound(work.id, ops, null);
      if (cards.length < ids.length) {
        said = `${ids.length - cards.length} of ${ids.length} selected assets were not added — the library did not answer for them.`;
      }
      // The pick is consumed by the round, which is the app's
      // convention for an operation over a selection. Said after the
      // count above, because that sentence is about what was picked and
      // the picking is over.
      gridSelection.clear();
    } finally {
      adding = false;
    }
  }

  /** One operation, its own round. A correction is a round like any
   *  other, which is what makes a rename here the same kind of thing as
   *  the add that named it wrongly. */
  async function ask(op: ForgeOpDto) {
    if (work === null) return;
    said = null;
    await forgeCatalog.pushRound(work.id, [op], null);
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

  // Refilling takes exactly one asset, so it is offered only when the
  // grid holds exactly one. The alternative — refill with the first of
  // several — picks on somebody's behalf which of their selection this
  // entry becomes.
  //
  // The id is read back for the same reason `addSelection` reads its
  // cards: a selected id the library will not answer for is one the
  // round would name and the write would refuse, and it costs one call
  // to find out here instead.
  async function replace(row: ForgeProjectedEntry) {
    const ids = [...gridSelection.selectedIds];
    if (ids.length !== 1) return;
    said = null;
    const cards = await api<AssetCardDto[]>("hydrate_cards", {
      ids,
      viewerSubject: null,
    });
    if (cards.length === 0) {
      said = "The library did not answer for the selected asset.";
      return;
    }
    await ask({
      entry_id: row.entryId,
      kind: "replace",
      content_asset_id: cards[0].id,
      name: null,
    });
    gridSelection.clear();
  }

  async function remove(row: ForgeProjectedEntry) {
    await ask({
      entry_id: row.entryId,
      kind: "remove",
      content_asset_id: null,
      name: null,
    });
  }

  // Undoing a removal is adding back *that* entry, which is why this
  // sends the row's own id rather than minting one: an entry that comes
  // back under a new id is a new arrival, and the record would say so.
  // It needs something to hold, so a row the line never filled has no
  // way back and is not offered one.
  async function putBack(row: ForgeProjectedEntry) {
    if (row.assetId === null) return;
    await ask({
      entry_id: row.entryId,
      kind: "add",
      content_asset_id: row.assetId,
      name: row.name ?? row.entryId,
    });
  }

  // What the rule did, said in terms of what is left. Writing nothing
  // has two causes — the rule declining, and there being nothing to
  // settle — and the collisions re-read alongside it is what tells them
  // apart.
  async function settle() {
    if (work === null) return;
    said = null;
    const wrote = await forgeCatalog.resolve(work.id);
    if (wrote) return;
    said =
      forgeCatalog.collisions.data.length === 0
        ? "Nothing left to settle."
        : "This line's rule leaves these to a person. Ask for something else on the rows below, and the collision goes with it.";
  }

  // The action behind a reason. `blocked` is the one that needs two,
  // because it arrives for two situations and only `mutate`'s message
  // says which — `http.rs` names them where it names the reasons.
  function whatToDo(error: unknown): string | null {
    const reason = (error as { reason?: string })?.reason;
    switch (reason) {
      case "blocked":
        return "Something has to change first — resolve what this collides with, or reopen the line if it is archived.";
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
    if (work === null) return;
    closing = true;
    said = null;
    try {
      await forgeCatalog.closePursuit(
        work.id,
        line.id,
        outcome,
        closeNote.trim() === "" ? null : closeNote.trim(),
      );
      closeNote = "";
    } catch (error) {
      // `mutate` has already put the refusal on screen; this adds the
      // half it does not read.
      said = whatToDo(error);
    } finally {
      closing = false;
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  /** What to call an entry an operation names. The op's own name when
   *  it carries one, and otherwise whatever the fold has for it — which
   *  may be a name another round of this work asked for rather than one
   *  the line has ever said. */
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

{#if forgeCatalog.working === null}
  <!-- No work open: what there is, and the form that starts some. -->
  {#if forgeCatalog.pursuits.loading}
    <p class="quiet">Reading…</p>
  {:else}
    {#if forgeCatalog.openWork.length > 0}
      <ul class="work-list">
        {#each forgeCatalog.openWork as item (item.id)}
          <li>
            <button onclick={() => forgeCatalog.selectPursuit(item.id)}>
              <span>{item.title ?? "(untitled)"}</span>
              <span class="quiet">{when(item.opened_at_ms)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="quiet">No work open against this line.</p>
    {/if}

    {#if forgeCatalog.endedWork.length > 0}
      <!-- Dimmed and listed apart, for the reason the store's `pursuits`
           read keeps them at all. -->
      <h4>Ended</h4>
      <ul class="work-list ended">
        {#each forgeCatalog.endedWork as item (item.id)}
          <li>
            <button onclick={() => forgeCatalog.selectPursuit(item.id)}>
              <span>{item.title ?? "(untitled)"}</span>
              <span class="quiet">{item.close?.outcome}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}

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
{:else if forgeCatalog.pursuit.loading && work === null}
  <p class="quiet">Reading…</p>
{:else if work === null}
  <p class="quiet">{forgeCatalog.pursuit.error ?? "This work is not there."}</p>
{:else}
  <header class="work-head">
    <button class="back" onclick={() => forgeCatalog.clearWork()}>
      ← all work
    </button>
    <strong>{work.title ?? "(untitled)"}</strong>
    <span class="quiet">
      {work.close === null ? "open" : work.close.outcome}
    </span>
    <!-- Talking about work that has ended is not offered less: what was
         said about a piece of work is often said after it. -->
    <button
      class="talk-about"
      onclick={() =>
        forgeCatalog.talkAbout({
          kind: "pursuit",
          about: `this work`,
          pursuitId: work.id,
        })}
    >say something</button>
  </header>

  {#if work.note !== null}
    <p class="quiet note">{work.note}</p>
  {/if}

  {#if !ended}
    <!-- Both answers are about work that can still move. Ended work
         collides with nothing anybody can act on, and a satisfied close
         leaves it behind its own landing — a true number about a
         question nobody is asking any more. -->
    {#if forgeCatalog.behind.data.length > 0}
      <p class="quiet">
        {forgeCatalog.behind.data.length}
        {forgeCatalog.behind.data.length === 1 ? "landing" : "landings"} on the line
        since this was cut.
      </p>
    {/if}

    {#if forgeCatalog.collisions.data.length > 0}
      <div class="collisions">
        <h4>Collides with the line</h4>
        <ul>
          {#each forgeCatalog.collisions.data as hit (hit.entry_id + hit.axis)}
            <li>
              <span>
                {projected.find((row) => row.entryId === hit.entry_id)?.name ??
                  hit.entry_id}
              </span>
              <span class="quiet">{hit.axis}</span>
            </li>
          {/each}
        </ul>
        <button type="button" onclick={settle}>let the rule settle these</button>
      </div>
    {/if}
  {/if}

  {#if clashing.length > 0}
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
    <!-- What closing would leave, and where the three verbs that name
         an entry are pressed. Only for work that can still move: an
         abandoned close lands nothing, so a fold of what it asks for
         would be a picture of a line that will never exist, and the
         contents tab is the answer for either ending.

         Rows this work takes off the line stay listed and dimmed, with
         the name and content the line has for them rather than
         anything the work said on the way off — that is what a removal
         leaves, and it is what putting one back has to put back. -->
    <h4>The line, as this would leave it</h4>
    <ul class="projected">
      {#each projected as row (row.entryId)}
        <li class:gone={!row.alive}>
          {#if row.assetId !== null}
            <img
              src={thumbCatalog.thumbById(row.assetId)}
              alt={row.name ?? "an entry with no name"}
              loading="lazy"
            />
          {:else}
            <span class="no-content" aria-hidden="true">—</span>
          {/if}
          <span class="op-name">{row.name ?? "(unnamed)"}</span>
          <span class="row-verbs">
            <!-- A row this work is taking off is offered neither a
                 rename nor a refill, because the fold gives a departing
                 entry a row that states existence and nothing else:
                 either would be written, accepted, and then discarded
                 by the close. A row the *line* is not holding is a
                 different case and keeps both — that rename lands. -->
            {#if !row.leaving}
              <button type="button" onclick={() => rename(row)}>rename</button>
              {#if gridSelection.selectedIds.size === 1}
                <button type="button" onclick={() => replace(row)}>
                  replace with the selected
                </button>
              {/if}
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
  {/if}

  <!-- The log. Oldest first, which is the order the chain holds and the
       order somebody reads a piece of work in — unlike the line's
       history, where the question is what happened last. -->
  <h4>What was asked for</h4>
  <ol class="rounds">
    {#each work.rounds as round (round.id)}
      <li>
        <p class="round-head">
          <span>{summarise(round)}</span>
          <span class="quiet">{when(round.at_ms)}</span>
          <button
            class="talk-about"
            onclick={() =>
              forgeCatalog.talkAbout({
                kind: "round",
                about: "this round",
                pursuitId: work.id,
                nodeId: round.id,
              })}
          >say something</button>
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
              <!-- An entry as *that round* had it, which is why the
                   round is named beside it: the same entry in two
                   rounds is two things to talk about. -->
              <button
                class="talk-about"
                onclick={() =>
                  forgeCatalog.talkAbout({
                    kind: "entry",
                    about: opName(op),
                    pursuitId: work.id,
                    nodeId: round.id,
                    entryId: op.entry_id,
                  })}
              >say something</button>
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
    <div class="compose">
      <!-- With nothing picked, this is not a disabled add: it is the
           way out. The drawer is an overlay, so a person told to go
           and select something in the grid is being told that by the
           thing in front of it — and a button carrying that
           instruction, disabled, is one that does nothing when
           pressed. So it steps aside instead, keeping the line and the
           work, and the sidebar brings the drawer back to them. -->
      {#if gridSelection.selectedIds.size === 0}
        <button type="button" onclick={() => forgeCatalog.stepAside()}>
          pick in the grid — this steps aside
        </button>
      {:else}
        <button type="button" onclick={addSelection} disabled={adding}>
          {adding ? "Adding…" : `add ${gridSelection.selectedIds.size} selected`}
        </button>
      {/if}
    </div>

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
  .talk-about {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    margin-left: auto;
    opacity: 0.7;
    padding: 0;
    text-decoration: underline;
  }
  .note {
    font-style: italic;
  }
  .collisions {
    border-left: 2px solid rgba(220, 90, 90, 0.6);
    padding-left: 0.5rem;
    margin: 0.5rem 0;
  }
  .collisions li {
    display: flex;
    gap: 0.5rem;
    font-size: 0.78rem;
  }
  .collisions button,
  .compose button,
  .close button,
  .row-verbs button {
    background: none;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    padding: 0.1rem 0.4rem;
  }
  .collisions button,
  .compose button,
  .close button {
    font-size: 0.78rem;
    margin-top: 0.3rem;
    padding: 0.15rem 0.5rem;
  }
  .said {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.78rem;
    border-left: 2px solid rgba(128, 128, 128, 0.6);
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
  .projected li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.78rem;
    padding: 0.15rem 0;
  }
  /* Off the line after this lands: dimmed and dashed, the same way the
     contents tab draws what a line let go. */
  .projected li.gone {
    opacity: 0.55;
  }
  .projected li.gone img,
  .projected li.gone .no-content {
    outline: 2px dashed currentColor;
    outline-offset: -2px;
  }
  .projected img,
  .no-content {
    width: 1.8rem;
    height: 1.8rem;
    flex: 0 0 auto;
    object-fit: cover;
    border-radius: 0.15rem;
    background: rgba(128, 128, 128, 0.15);
  }
  .no-content {
    display: grid;
    place-items: center;
    border: 1px dashed rgba(128, 128, 128, 0.5);
    opacity: 0.6;
  }
  .row-verbs {
    display: flex;
    gap: 0.3rem;
    margin-left: auto;
  }
  .rounds > li {
    border-top: 1px solid rgba(128, 128, 128, 0.25);
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
  .compose,
  .close {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-top: 0.6rem;
  }
  .new-work {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 1rem;
    padding-top: 0.7rem;
    border-top: 1px solid rgba(128, 128, 128, 0.3);
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
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    padding: 0.15rem 0.6rem;
  }
</style>
