<script lang="ts">
  // ForgePanel — this machine's lines, and how each got there.
  //
  // The frame #170's four surfaces attach to. Why it is shaped this way
  // — the ratio it follows, what the model forbids, where each of those
  // surfaces lands on it — is in `lib/stores/forge.svelte.ts`, beside
  // the reads it makes. This header says only what this component adds.
  //
  // 0-prop: it reads `forgeCatalog` directly and takes nothing from
  // whoever mounts it. The App mounts it once and the sidebar opens it
  // — the arrangement `SharedLinesPanel` has, and the list of reads is
  // the same shape, so a reader who knows one knows this.
  //
  // A drawer, two columns wide. The forge answers several questions
  // about a line at once, and the list of lines has to stay in view
  // while any of them is read or selecting becomes a round trip. A
  // single-column drawer has no room for both — which is why the
  // team's drawer took this width and this shape too (#217).
  //
  // Tabs rather than panels. Contents, work and history are answers
  // about one line from one place, and a person moving between them is
  // changing the question rather than the subject — so the line stays
  // named in the header while the body below it swaps.
  //
  // Work among them rather than a surface beside this one, which is
  // what #170 lists it as. It is a third answer about the same line
  // rather than a different subject: what the line says, what somebody
  // is asking it to say, and how it got here. The button in the header
  // stays, and now goes there.
  //
  // It sits between the other two, because working a line is the common
  // path and the chain is the occasional one — the ratio the store's
  // header argues for, applied to an order it did not have to state
  // while there were only two.
  //
  // The writes here are the ones about a line itself: opening,
  // renaming, re-pointing, archiving, reopening, discarding. #170 gives
  // the line verbs to a later child and does not name opening among
  // them — an omission that shows the moment the panel runs on a
  // machine with no line, which is every machine until somebody calls
  // the command by hand.
  //
  // Work against a line is `ForgeWork`'s, and only its close touches
  // the line at all. What this component does for it is the frame: it
  // says which line, and hands that line down.
  import ForgeWork from "./ForgeWork.svelte";
  import ForgeTalk from "./ForgeTalk.svelte";
  import TabStrip from "./TabStrip.svelte";
  import { forgeCatalog } from "./lib/stores/forge.svelte";
  import { detailRequest } from "./lib/stores/detail-request.svelte";
  import { gridSelection } from "./lib/stores/grid-selection.svelte";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";
  import { confirmCatalog } from "./lib/stores/confirm.svelte";
  import { promptCatalog } from "./lib/stores/prompt.svelte";
  import type { ForgeLineDto } from "./bindings";
  import { axes } from "./lib/forge-projection";

  let tab = $state<"contents" | "work" | "history">("contents");
  let showOffTheLine = $state(false);
  // One change point open at a time. The chain is read to answer "what
  // happened here", and a screen that let every point stand open would
  // be the wall of table the design warns about.
  let openPoint = $state<string | null>(null);

  const open = $derived(
    forgeCatalog.lines.data.filter((line) => line.standing === "open"),
  );
  const archived = $derived(
    forgeCatalog.lines.data.filter((line) => line.standing !== "open"),
  );
  const current = $derived(
    forgeCatalog.lines.data.find((line) => line.id === forgeCatalog.selected) ??
      null,
  );

  async function select(lineId: string) {
    showOffTheLine = false;
    // A point id belongs to the line it is on, so an open one from the
    // previous line matches nothing here — it would just leave the
    // first point of the new chain looking collapsed when the reader
    // had asked for one to be open.
    openPoint = null;
    // Everything the *store* has to let go of on this move is
    // `selectLine`'s, written once there.
    await forgeCatalog.selectLine(lineId);
    if (tab === "history") await forgeCatalog.history.load({ lineId });
  }

  // The chain loads on demand rather than beside the contents: it
  // answers a question asked after the work rather than during it.
  async function toHistory() {
    tab = "history";
    if (forgeCatalog.selected !== null && forgeCatalog.history.data === null) {
      await forgeCatalog.history.load({ lineId: forgeCatalog.selected });
    }
  }

  // The new-line form. Its own state rather than the store's: a
  // half-typed name is this component's business and nobody else's.
  let newName = $state("");
  let newStrategy = $state("");
  let opening = $state(false);

  async function openLine(event: Event) {
    event.preventDefault();
    opening = true;
    try {
      await forgeCatalog.openLine(newName, newStrategy);
      newName = "";
    } finally {
      opening = false;
    }
  }

  // The line verbs. Rename and re-point read a value first, so they go
  // through the prompt the App already mounts; archive and reopen are
  // one press each.
  async function rename(line: ForgeLineDto) {
    const name = await promptCatalog.open(
      "Rename this line",
      "a name",
      line.name,
    );
    if (name === null || name.trim() === "") return;
    await forgeCatalog.rename(line.id, name);
  }

  async function repoint(line: ForgeLineDto, strategyId: string) {
    if (strategyId === line.strategy_id) return;
    await forgeCatalog.setStrategy(line.id, strategyId);
  }

  // Discard is the verb the confirm modal exists for: it takes the
  // line, its whole history, and every piece of work against it. The
  // released assets come back in the answer and are named here,
  // because after this write nothing can derive them again.
  //
  // **Every piece of work that has ended.** The model refuses the drop
  // while any is still open, because dropping takes the history that
  // work was cut from and would leave a log against nothing. The body
  // said the pursuits went with it and stopped there, which is the half
  // that reads as a warning; the half that reads as an instruction was
  // missing until a refusal arrived on a screen and had to be explained
  // by the toast.
  async function discard(line: ForgeLineDto) {
    const open = forgeCatalog.openWork.length;
    const ok = await confirmCatalog.open({
      title: `Discard ${line.name}?`,
      body:
        open > 0
          ? `${open} ${open === 1 ? "pursuit is" : "pursuits are"} still open against this line, and the ` +
            "forge will refuse to drop it until they are closed. Close them " +
            "under work, then discard."
          : "The line, its history, and every pursuit against it go with it. " +
            "The assets it held stay in the library. This cannot be undone.",
      confirmLabel: "Discard Forever",
      danger: true,
    });
    if (!ok) return;
    await forgeCatalog.discard(line.id);
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  // What the library knows about what is on the line, read when the
  // list of ids changes rather than at each site that loads states.
  // A line's contents move under three different verbs, and an effect
  // over what is rendered answers for all of them.
  $effect(() => {
    void forgeCatalog.ensureCards(
      forgeCatalog.states.data.map((state) => state.content_asset_id),
    );
  });

  // What a tile shows instead of a picture, and empty when a picture
  // is what it should show.
  //
  // **A video has a frame.** `thumbById` names the forge as the surface
  // that waits for one, and the first build of this asked for a
  // thumbnail only when `media` was exactly `image` — so a video was a
  // word where its own frame was on its way. Both kinds that carry
  // pixels go to the thumbnail; the two that do not are what this
  // answers for.
  //
  // For those two, the card's cover text is what the grid shows and is
  // a great deal more use than a slug: `none` is `MediaKind`'s name for
  // "no inline player", not a thing to put in front of somebody. The
  // slug is the fallback to the fallback.
  //
  // An entry whose card has not arrived says nothing rather than
  // guessing — the id is not a fact about the thing.
  function kindOf(assetId: string | null): string {
    if (assetId === null) return "";
    const card = forgeCatalog.cards[assetId];
    if (card === undefined) return "";
    if (card.media === "image" || card.media === "video") return "";
    return card.cover ?? (card.media === "none" ? "no preview" : card.media);
  }

</script>

<!-- One tile, two lists. What is on the line and what it let go are
     drawn apart and drawn the same way, so the difference a reader sees
     is the dimming rather than two renderings that drifted.

     A thumbnail where there is one to show, and what the thing *is*
     where there is not: a line holds assets rather than pictures, and
     an entry carrying a recording used to be an empty grey box that
     never filled — indistinguishable from one still loading, because
     `thumbById` answers both with the same transparent pixel. -->
{#snippet tile(assetId: string | null, name: string | null)}
  {#if assetId === null}
    <!-- An entry can carry a name and no content: a table may name one
         before anything fills it. Nothing to open, so not a button. -->
    <span class="no-content" aria-hidden="true">—</span>
    <span class="entry-name">{name ?? "(unnamed)"}</span>
  {:else}
    <!-- A tile opens the asset properly.
         What a line shows of an entry is a thumbnail and the name the
         *line* gives it, which is deliberately not the asset's — so
         "what is this actually" is a question the forge raises and
         cannot answer. The detail pane answers it, and comes up over
         the drawer rather than instead of it, so closing it leaves the
         line exactly where it was.

         The picture and the name are one button rather than a picture
         that happens to be clickable. The first build made only the
         image one, with the affordance carried by the cursor and a
         tooltip, and the first person to meet it asked where to press.
         The whole cell is the target now, and it says so at rest. -->
    <button
      class="tile"
      onclick={() => detailRequest.open(assetId)}
      title={`Open ${name ?? "this entry"}`}
    >
      {#if kindOf(assetId) !== ""}
        <span class="no-content kind">{kindOf(assetId)}</span>
      {:else}
        <img
          src={thumbCatalog.thumbById(assetId)}
          alt={name ?? "an entry with no name"}
          loading="lazy"
        />
      {/if}
      <span class="entry-name">{name ?? "(unnamed)"}<span class="open-hint">↗</span></span>
    </button>
  {/if}
{/snippet}

{#if forgeCatalog.open}
  <!-- Backdrop absorbs outside-click and Escape; the drawer stops
       propagation so an interior click never closes it. Same guard
       SharedLinesPanel uses, for the same reason. -->
  <div
    class="drawer-backdrop"
    onclick={() => forgeCatalog.closePanel()}
    onkeydown={(e) => e.key === "Escape" && forgeCatalog.closePanel()}
    role="button"
    tabindex="-1"
    aria-label="Close the forge"
  >
    <div
      class="drawer"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      tabindex="-1"
      aria-label="Forge"
    >
      <header class="drawer-head">
        <h2>Forge</h2>
        <button
          class="drawer-close"
          onclick={() => forgeCatalog.closePanel()}
          aria-label="Close"
        >✕</button>
      </header>

      {#if forgeCatalog.released !== null}
        <!-- Above the two columns rather than inside one: the line this
             answers about is gone, so there is no line detail to sit
             in. The only place these ids are ever named — after the
             write nothing can derive them again — with a dismiss,
             because a notice that cannot be cleared is one a person
             stops reading. -->
        <p class="released">
          Discarded. {forgeCatalog.released.length}
          {forgeCatalog.released.length === 1 ? "asset" : "assets"} released back
          to the library.
          <button type="button" onclick={() => (forgeCatalog.released = null)}>
            dismiss
          </button>
        </p>
      {/if}

      <section class="forge" aria-label="Lines on this machine">
  <nav class="lines" aria-label="Lines">
    <h3>Lines</h3>
    {#if forgeCatalog.lines.loading}
      <p class="quiet">Reading…</p>
    {:else if forgeCatalog.lines.error !== null}
      <p class="quiet">{forgeCatalog.lines.error}</p>
    {:else if forgeCatalog.lines.data.length === 0}
      <p class="quiet">No line on this machine yet.</p>
    {/if}

    <ul>
      {#each open as line (line.id)}
        <li>
          <button
            class:selected={line.id === forgeCatalog.selected}
            onclick={() => select(line.id)}
          >{line.name}</button>
        </li>
      {/each}
    </ul>

    {#if archived.length > 0}
      <!-- A section rather than a filter: a discard is only reachable
           from an archived line, so hiding these would hide exactly the
           ones somebody is about to drop. -->
      <h4>Archived</h4>
      <ul>
        {#each archived as line (line.id)}
          <li>
            <button
              class:selected={line.id === forgeCatalog.selected}
              onclick={() => select(line.id)}
            >{line.name}</button>
          </li>
        {/each}
      </ul>
    {/if}

    <!-- The strategy is chosen, not defaulted. It decides how the line
         settles a collision, and picking one on somebody's behalf is
         picking how their work gets resolved. The list comes from the
         route that names what this deployment carries, so a rule that
         is not built in cannot be typed in by accident. -->
    <form class="new-line" onsubmit={openLine}>
      <h4>New line</h4>
      <label>
        Name
        <input type="text" bind:value={newName} required />
      </label>
      <label>
        Rule
        <select bind:value={newStrategy} required>
          <option value="" disabled>choose…</option>
          {#each forgeCatalog.strategies.data as rule (rule.id)}
            <option value={rule.id} title={rule.summary}>{rule.name}</option>
          {/each}
        </select>
      </label>
      {#if forgeCatalog.strategies.data.length === 0}
        <!-- Without a rule there is nothing to open a line with, and the
             submit below is disabled. Said rather than left as a button
             that does not respond: `Resource` falls back to an empty
             list on failure and puts the reason on `.error`, which
             looks the same from here as a deployment carrying no rule
             at all. -->
        <p class="quiet">
          {forgeCatalog.strategies.error ?? "This deployment carries no rule."}
        </p>
      {/if}
      <button type="submit" disabled={opening || newStrategy === ""}>
        {opening ? "Opening…" : "Open"}
      </button>
    </form>
  </nav>

  <div class="line">
    {#if current === null}
      <p class="quiet">Select a line.</p>
    {:else}
      <header>
        <h3>{current.name}</h3>
        <span class="quiet">{current.standing}</span>
        <!-- It says open work, so it lands where work is opened.
             Switching to the tab is not enough: a piece of work being
             read stays showing, and the form to start another is behind
             a "← all work" nobody was told to press. Letting go of what
             is showing is what makes the button mean what it says. -->
        <button
          type="button"
          onclick={() => {
            forgeCatalog.clearWork();
            tab = "work";
          }}
        >
          open work
        </button>
      </header>

      <!-- The line's own description, and the two verbs that move it.
           Neither is a landing: the history says what happened to what
           the line carries, and a rename did not. -->
      <div class="verbs">
        <button type="button" onclick={() => rename(current)}>rename</button>

        <label class="rule">
          rule
          <select
            value={current.strategy_id}
            onchange={(e) => repoint(current, e.currentTarget.value)}
          >
            {#each forgeCatalog.strategies.data as rule (rule.id)}
              <option value={rule.id} title={rule.summary}>{rule.name}</option>
            {/each}
          </select>
        </label>

        {#if current.standing === "open"}
          <!-- Archiving is the step before dropping as well as a state
               of its own: a discard is only reachable from here. -->
          <button type="button" onclick={() => forgeCatalog.archive(current.id)}>
            archive
          </button>
        {:else}
          <button type="button" onclick={() => forgeCatalog.reopen(current.id)}>
            reopen
          </button>
          <button type="button" class="danger" onclick={() => discard(current)}>
            discard
          </button>
        {/if}
      </div>

      <!-- Row shared with `SharedLinesPanel` as `TabStrip` (#217); this
           file's own CSS values are the ones the component kept. The
           `ariaLabel` below is not this file's own — it came from
           `SharedLinesPanel`'s line row, which had one where this row
           never did. -->
      <div class="tabs">
        <TabStrip
          ariaLabel="What to read about this line"
          tabs={[
            { key: "contents", label: "on the line", onSelect: () => (tab = "contents") },
            { key: "work", label: "work", onSelect: () => (tab = "work") },
            { key: "history", label: "history", onSelect: toHistory },
          ]}
          active={tab}
        />
      </div>

      {#if tab === "contents"}
        {#if forgeCatalog.states.loading}
          <p class="quiet">Reading…</p>
        {:else}
          <!-- Tiles, because looking at what is on a line is the reason
               to open this. The name goes under the picture rather than
               in place of it: it is the line's name for the entry, not
               the asset's, and the two can differ — which is also what
               the tile opens the asset to settle. -->
          <ul class="entries">
            {#each forgeCatalog.onTheLine as entry (entry.entry_id)}
              <li>
                {@render tile(entry.content_asset_id, entry.name)}
              </li>
            {/each}
          </ul>
          <p class="quiet">{forgeCatalog.onTheLine.length} on the line</p>

          {#if forgeCatalog.offTheLine.length > 0}
            <!-- Findable, and drawn so it cannot be read as contents.
                 "Off the line" rather than "taken off": the wire says
                 only that the line does not hold these now, and an
                 entry named without ever being added reads the same as
                 one a change point removed. -->
            <button
              class="disclose"
              onclick={() => (showOffTheLine = !showOffTheLine)}
            >
              {forgeCatalog.offTheLine.length} off the line
              {showOffTheLine ? "▾" : "▸"}
            </button>
            {#if showOffTheLine}
              <ul class="entries gone">
                {#each forgeCatalog.offTheLine as entry (entry.entry_id)}
                  <li>
                    {@render tile(entry.content_asset_id, entry.name)}
                  </li>
                {/each}
              </ul>
            {/if}
          {/if}
        {/if}
      {:else if tab === "work"}
        <!-- The line is handed down rather than read from the store
             again: this component has already decided which line is
             showing, and a second read of the same selection is a
             second place for the two to disagree. -->
        <ForgeWork line={current} />
      {:else if forgeCatalog.history.loading}
        <p class="quiet">Reading…</p>
      {:else if forgeCatalog.history.data === null}
        <p class="quiet">No history read yet.</p>
      {:else}
        <ol class="chain">
          {#each [...forgeCatalog.history.data.changes].reverse() as point (point.id)}
            <li>
              <button
                class="point"
                aria-expanded={openPoint === point.id}
                onclick={() =>
                  (openPoint = openPoint === point.id ? null : point.id)}
              >
                <span class="rows">
                  {openPoint === point.id ? "▾" : "▸"}
                  {point.table.length}
                  {point.table.length === 1 ? "row" : "rows"}
                </span>
                <span>{point.actor_id}</span>
                <span class="quiet">{when(point.at_ms)}</span>
              </button>
              <!-- A conversation about what landed rather than about
                   the work it came out of. The model keeps those as
                   separate anchors, and a change point outlives the
                   pursuit that produced it. -->
              <button
                class="talk-about"
                onclick={() =>
                  forgeCatalog.talkAbout({
                    kind: "change",
                    about: "what landed here",
                    lineId: current.id,
                    changePointId: point.id,
                  })}
              >say something</button>

              {#if openPoint === point.id}
                <!-- One line per entry the point moved, phrased from
                     the axes rather than read off a verb: the model
                     stores which axes a row states, and "added" or
                     "renamed" is a reading of that. A row states only
                     what it moved, so a blank axis is silence and not
                     a null value. -->
                <ul class="rows-open">
                  {#each point.table as row (row.entry_id)}
                    <li>
                      <span class="axes">{axes(row)}</span>
                      <span class="quiet">{row.name ?? row.entry_id}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ol>
        <!-- The genesis is not a change point, and the model keeps the
             two apart. Folding it into the chain would claim something
             the record does not. -->
        <p class="quiet genesis">
          genesis · {when(forgeCatalog.history.data.genesis_at_ms)}
        </p>
      {/if}

      <!-- Mounted once, under whichever tab is showing, because a
           conversation is about something on one of them and moving the
           reader to a fourth tab to read it would be moving them away
           from what it is about. Renders nothing until somebody opens
           one. -->
      <ForgeTalk />
    {/if}
  </div>
      </section>
    </div>
  </div>
{:else if forgeCatalog.steppedAside}
  <!-- The way back.
       Stepping aside is half a gesture without it: the drawer goes, and
       somebody is left in the grid with no sign that the forge is
       waiting and no way back that is not a guess. This says which work
       is waiting, counts what has been picked so far, and returns to
       it — and carries the one control that does the opposite, because
       somewhere has to: the ✕ ends the question rather than the
       stepping aside, and takes the line, the work and everything
       loaded about both with it. Fixed to the corner rather than laid
       out in the page, because what it interrupts is a person looking
       at the grid. -->
  <aside class="waiting" aria-label="The forge is waiting">
    <span>
      Picking for <strong>{forgeCatalog.pursuit.data?.title ?? "this work"}</strong>
      · {gridSelection.selectedIds.size} selected
    </span>
    <button type="button" onclick={() => forgeCatalog.openPanel()}>
      back to the forge
    </button>
    <button
      type="button"
      class="quiet-btn"
      onclick={() => forgeCatalog.closePanel()}
      aria-label="Stop picking for the forge"
    >✕</button>
  </aside>
{/if}

<style>
  .waiting {
    position: fixed;
    right: 1rem;
    bottom: 1rem;
    z-index: 60;
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.78rem;
    background: var(--panel-bg, #1b1b1e);
    color: var(--panel-fg, #e8e8ea);
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.3rem;
    box-shadow: 0 0.3rem 1rem rgba(0, 0, 0, 0.4);
    padding: 0.5rem 0.7rem;
  }
  .waiting button {
    background: none;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.78rem;
    padding: 0.15rem 0.5rem;
  }
  .waiting .quiet-btn {
    border: 0;
    opacity: 0.7;
    padding: 0 0.1rem;
  }
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
    /* Two columns, and the list of lines stays in view while a line is
       read. The team's drawer draws the same width for the same reason
       (#217). */
    width: min(52rem, 96vw);
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
    margin-bottom: 0.8rem;
  }
  .drawer-head h2 {
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
  .forge {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
  }
  .lines {
    flex: 0 0 12rem;
  }
  .line {
    flex: 1 1 auto;
    min-width: 0;
  }
  h3,
  h4 {
    margin: 0 0 0.4rem;
    font-size: 0.9rem;
  }
  h4 {
    margin-top: 0.9rem;
    font-weight: 500;
  }
  ul,
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .lines button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    padding: 0.2rem 0;
    text-align: left;
    width: 100%;
  }
  .lines button.selected {
    font-weight: 600;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
  }
  header button {
    margin-left: auto;
  }
  .tabs {
    margin: 0.6rem 0;
  }
  .entries {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(6.5rem, 1fr));
    gap: 0.6rem;
    margin: 0.5rem 0;
  }
  .entries li {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    min-width: 0;
  }
  .entries img {
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    border-radius: 0.2rem;
    background: rgba(128, 128, 128, 0.15);
  }
  /* The button is the whole cell — picture and name — and it says at
     rest that it is one: an arrow beside the name, and a frame that
     answers to hover and to the keyboard. A cursor and a tooltip were
     what it had first, and neither is visible until somebody has
     already guessed. */
  .tile {
    background: none;
    border: 1px solid transparent;
    border-radius: 0.25rem;
    color: inherit;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.15rem;
    text-align: left;
    width: 100%;
  }
  .tile:hover,
  .tile:focus-visible {
    border-color: rgba(128, 128, 128, 0.55);
    background: rgba(128, 128, 128, 0.12);
  }
  .open-hint {
    opacity: 0.55;
    margin-left: 0.25rem;
  }
  .tile:hover .open-hint,
  .tile:focus-visible .open-hint {
    opacity: 1;
  }
  .no-content {
    display: grid;
    place-items: center;
    aspect-ratio: 1;
    border: 1px dashed rgba(128, 128, 128, 0.5);
    border-radius: 0.2rem;
    opacity: 0.6;
  }
  /* Solid rather than dashed: this entry holds something, and the
     dashes next door mean it does not. */
  .no-content.kind {
    border-style: solid;
    font-size: 0.72rem;
    opacity: 0.8;
  }
  .entry-name {
    font-size: 0.72rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Off the line: dimmed and dashed, so it cannot be read as
     contents while staying findable. */
  .entries.gone li {
    opacity: 0.55;
  }
  .entries.gone img,
  .entries.gone .no-content {
    outline: 2px dashed currentColor;
    outline-offset: -2px;
  }
  .point {
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
  .rows-open {
    margin: 0 0 0.4rem 1.2rem;
    border-left: 1px solid rgba(128, 128, 128, 0.3);
    padding-left: 0.6rem;
  }
  .rows-open li {
    display: flex;
    gap: 0.5rem;
    padding: 0.15rem 0;
    font-size: 0.78rem;
  }
  .axes {
    min-width: 9rem;
  }
  /* Block, not flex: the row of facts is `.point`'s own layout, and
     the expanded table sits under it rather than beside it. */
  .chain li {
    display: block;
  }
  .rows {
    min-width: 4.5rem;
  }
  .disclose {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    padding: 0.3rem 0;
  }
  .talk-about {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    opacity: 0.7;
    padding: 0 0 0.2rem 1.2rem;
    text-decoration: underline;
  }
  .verbs {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin: 0.4rem 0;
    font-size: 0.78rem;
  }
  .verbs button {
    background: none;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    padding: 0.15rem 0.5rem;
  }
  .verbs button.danger {
    border-color: rgba(220, 90, 90, 0.6);
  }
  .rule {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    opacity: 0.85;
  }
  .released {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.78rem;
    border-left: 2px solid rgba(220, 90, 90, 0.6);
    padding-left: 0.5rem;
    margin: 0 0 0.8rem;
  }
  .released button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    opacity: 0.7;
    padding: 0;
    text-decoration: underline;
  }
  .new-line {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 1.1rem;
    padding-top: 0.8rem;
    border-top: 1px solid rgba(128, 128, 128, 0.3);
  }
  .new-line label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    opacity: 0.85;
  }
  .new-line input,
  .new-line select {
    width: 100%;
    box-sizing: border-box;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.78rem;
    margin: 0.3rem 0;
  }
  .genesis {
    border-top: 1px solid rgba(128, 128, 128, 0.3);
    padding-top: 0.4rem;
  }
</style>
