<script lang="ts">
  // MaterialChapters — how a playable asset is divided, and by whom.
  //
  // The sibling of `MaterialMarks.svelte`: that one shows the notes
  // written *at* positions inside the material, this one shows the
  // claims about how the material is *split* into sections. They share a
  // timeline and nothing else — a mark says "look at this", a chapter
  // says "this section starts here" — which is why they are two
  // aggregates behind two layer roles rather than one list with a flag.
  //
  // What this component adds over the chip strip it replaces: the chips
  // read `extra.chapters`, a blob the importer happened to leave on the
  // asset. There was no way to tell the file's own declaration from
  // anything else, nowhere to put a correction, and a re-scan overwrote
  // whatever was there. Chapters now hang off a *band*
  // (`MaterialLayer`), and the band says who produced it — so the file's
  // list and a person's can both exist, and re-reading the file replaces
  // only its own.
  //
  // Ownership follows `MaterialMarks` exactly: this component owns the
  // bands of one asset and fetches them itself (a component may own
  // state that nothing else reads; a catalog would only add a reload
  // orchestration question that no second reader is asking). Same three
  // props, for the same reasons:
  //   * `assetId` / `durationMs` — the asset in the pane and whether its
  //     material has a timeline at all.
  //   * `media` — the live `<video>` / `<audio>` element, a `bind:this`
  //     handle passed down. Positions are read *from* it (the playhead a
  //     new section is stamped at) and written *to* it (a click on a
  //     section seeks).
  //
  // The ruler is drawn in the DOM rather than onto the waveform canvas,
  // which is where the old chapter ticks went. Two reasons, both from
  // the pane: the canvas exists only in the audio branch (decoding a
  // whole video for peaks OOMs the webview) and chapters belong to video
  // just as much, and pushing rows into a canvas the pane owns would
  // hand this component's state back up to its parent. `MaterialMarks`
  // settled the same question the same way.
  //
  // Renders nothing when the asset has no timeline — a still image has
  // no divisions to declare.
  import { api } from "./lib/api";
  import { hasTimeline, markRatio, positionMsFromMedia } from "./lib/material-mark";
  import {
    bandEditable,
    bandLabel,
    buildCreateBandCommand,
    buildMoveCommand,
    buildPostChapterCommand,
    buildRenameCommand,
    chapterListNote,
    chapterRangeLabel,
    chapterRowKeys,
    pickBandId,
    structureBands,
  } from "./lib/material-layer";
  import { fmtDurationMs } from "./lib/formatters";
  import type {
    ChapterMarkDto,
    MaterialLayerDto,
    MaterialLayerViewDto,
  } from "./bindings";

  interface Props {
    assetId: string;
    durationMs: number | null;
    media: HTMLMediaElement | null;
  }

  let { assetId, durationMs, media }: Props = $props();

  // Every band over the material, each with its chapters, in the order
  // the backend handed them over. One call answers both halves of the
  // panel — which bands there are, and what is in the open one — so
  // there is no moment where the switcher and the list disagree.
  let views = $state<MaterialLayerViewDto[]>([]);
  let activeBandId = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  // Writes report their own failures. Kept apart from `loadError`: "the
  // bands could not be read" and "this edit was refused" are different
  // situations, and the second one leaves a readable list on screen.
  let writeError = $state<string | null>(null);
  let draft = $state("");
  let busy = $state(false);
  // Playhead, mirrored from the media element so the ruler and the "add
  // at …" label follow playback.
  let positionMs = $state(0);

  const timeline = $derived(hasTimeline(durationMs));
  const bands = $derived(structureBands(views));
  const active = $derived(bands.find((b) => b.layer.id === activeBandId) ?? null);
  const chapters = $derived<ChapterMarkDto[]>(active ? active.chapters : []);
  // Positional, so the two `{#each}` blocks below read
  // `rowKeys[i]` rather than deriving a key per row: `(layer_id, ord)`
  // repeats are resolved against the whole list, not against one entry.
  const rowKeys = $derived(chapterRowKeys(chapters));
  const editable = $derived(active !== null && bandEditable(active.layer.origin));
  const listNote = $derived(chapterListNote(bands, active));
  // Derived rather than an `{@const}` in the ruler: `{@const}` is only
  // legal as the immediate child of a block, and the playhead sits
  // beside the `{#each}` rather than inside it.
  const playheadRatio = $derived(markRatio(positionMs, durationMs));

  async function load(id: string): Promise<void> {
    try {
      const rows = await api<MaterialLayerViewDto[]>("list_material_layers", {
        assetId: id,
      });
      // The pane may have moved to another asset while this was in
      // flight; dropping the stale answer keeps one asset's bands from
      // appearing under another.
      if (assetId !== id) return;
      views = rows;
      activeBandId = pickBandId(structureBands(rows), activeBandId);
      loadError = null;
    } catch (err) {
      if (assetId !== id) return;
      console.warn("list_material_layers failed", err);
      views = [];
      activeBandId = null;
      loadError = err instanceof Error ? err.message : String(err);
    }
  }

  $effect(() => {
    const id = assetId;
    // Reading `durationMs` through the derived registers it, so an asset
    // whose duration arrives late still gets its bands fetched.
    if (!timeline) {
      views = [];
      activeBandId = null;
      loadError = null;
      return;
    }
    draft = "";
    writeError = null;
    void load(id);
  });

  // Follow the element's playhead, exactly as the marks panel does:
  // `timeupdate` fires a few times a second during playback, `seeked`
  // and `loadedmetadata` cover the transitions it does not report.
  $effect(() => {
    const el = media;
    if (!el) return;
    const sync = () => {
      positionMs = positionMsFromMedia(el.currentTime);
    };
    sync();
    el.addEventListener("timeupdate", sync);
    el.addEventListener("seeked", sync);
    el.addEventListener("loadedmetadata", sync);
    return () => {
      el.removeEventListener("timeupdate", sync);
      el.removeEventListener("seeked", sync);
      el.removeEventListener("loadedmetadata", sync);
    };
  });

  /// Every write goes through here, and every write ends in a re-read.
  ///
  /// Re-reading rather than patching the one row that changed is the
  /// rule the whole panel follows, and it is not only tidiness: making a
  /// band the default moves the flag *off* whichever band held it, so
  /// there is no single entry to patch (`set_default_material_layer`
  /// returns nothing for exactly that reason), and a chapter's place in
  /// its band is decided by `ord` on the same side that assigned it.
  async function write(op: () => Promise<void>): Promise<void> {
    busy = true;
    writeError = null;
    try {
      await op();
      await load(assetId);
    } catch (err) {
      console.warn("material layer write failed", err);
      writeError = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }

  /// Jump. Same move the marks make, and the same one the chapter chips
  /// made before them.
  function seekTo(startMs: number): void {
    if (media === null) return;
    media.currentTime = startMs / 1000;
  }

  function openBand(id: string): void {
    activeBandId = id;
    writeError = null;
    draft = "";
  }

  async function makeDefault(layerId: string): Promise<void> {
    await write(async () => {
      await api<void>("set_default_material_layer", {
        command: { layer_id: layerId },
      });
    });
  }

  async function createBand(): Promise<void> {
    await write(async () => {
      const created = await api<MaterialLayerDto>("create_material_layer", {
        command: buildCreateBandCommand(assetId, bands),
      });
      // Open what was just made. Set before the re-read so `pickBandId`
      // finds it and keeps it, rather than falling back to the default
      // band and leaving the person looking at the file's list after
      // asking for one of their own.
      activeBandId = created.id;
    });
  }

  async function removeBand(layerId: string): Promise<void> {
    await write(async () => {
      await api<void>("delete_material_layer", {
        command: { layer_id: layerId },
      });
      // The id is gone; `pickBandId` falls back to the default band.
      activeBandId = null;
    });
  }

  async function addChapter(): Promise<void> {
    const layer = active;
    if (layer === null) return;
    const command = buildPostChapterCommand(
      layer.layer.id,
      positionMs,
      draft,
      chapters,
    );
    if (command === null) return;
    await write(async () => {
      await api<ChapterMarkDto>("post_chapter_mark", { command });
      draft = "";
    });
  }

  /// Retitle. Fired on `change` rather than on input: a write per
  /// keystroke would be a write per keystroke.
  async function rename(chapter: ChapterMarkDto, label: string): Promise<void> {
    const command = buildRenameCommand(chapter, label);
    if (command === null) return;
    await write(async () => {
      await api<ChapterMarkDto>("edit_chapter_mark", { command });
    });
  }

  /// Move a section to where playback stands.
  ///
  /// The position is read at the moment of the click, not typed: the
  /// person watches, hears the section begin, and says "here" — the same
  /// gesture the mark composer uses. A typed timecode would be a second
  /// way to say the same thing, and the one that can be off by a digit.
  async function moveHere(chapter: ChapterMarkDto): Promise<void> {
    const command = buildMoveCommand(chapter, positionMs);
    if (command === null) return;
    await write(async () => {
      await api<ChapterMarkDto>("edit_chapter_mark", { command });
    });
  }

  async function removeChapter(chapter: ChapterMarkDto): Promise<void> {
    await write(async () => {
      await api<void>("delete_chapter_mark", {
        command: { layer_id: chapter.layer_id, chapter_id: chapter.id },
      });
    });
  }
</script>

{#if timeline}
  <section class="chapter-panel" aria-label="Chapters">
    <!-- Band switcher. A band has no name of its own, so each chip says
         what the band *is* — who produced it — and which one is the
         default. Shown even when there is one band: "From the file" is
         the fact that made this panel necessary.

         `role="group"` for the same reason the duplicates axis switcher
         carries one: the chips are a set to choose from, and a reader
         arriving at the second one otherwise hears a lone button with
         nothing saying what the choice is over. The verbs sit inside
         the group rather than beside it because each acts on whichever
         chip is currently pressed. -->
    <div class="chapter-bands" role="group" aria-label="Which band of chapters to show">
      {#each bands as b (b.layer.id)}
        <button
          type="button"
          class="chapter-band"
          class:active={b.layer.id === activeBandId}
          class:mine={bandEditable(b.layer.origin)}
          aria-pressed={b.layer.id === activeBandId}
          onclick={() => openBand(b.layer.id)}
        >
          {bandLabel(b.layer.origin, b.layer.role)}
          {#if b.layer.is_default}<span class="chapter-band-default">default</span>{/if}
        </button>
      {/each}
      <button
        type="button"
        class="chapter-band-add"
        onclick={createBand}
        disabled={busy}
        title="Start a band of your own"
      >+ band</button>
      {#if active && !active.layer.is_default}
        <button
          type="button"
          class="chapter-band-action"
          onclick={() => makeDefault(active.layer.id)}
          disabled={busy}
        >Make default</button>
      {/if}
      {#if editable && active}
        <button
          type="button"
          class="chapter-band-action danger"
          onclick={() => removeBand(active.layer.id)}
          disabled={busy}
        >Delete band</button>
      {/if}
    </div>

    <!-- Ruler — the sections laid out where they start, so the shape of
         the division is readable before any of it is read. -->
    <div class="chapter-ruler">
      {#each chapters as c, i (rowKeys[i])}
        {@const ratio = markRatio(c.start_ms, durationMs)}
        {#if ratio !== null}
          <button
            type="button"
            class="chapter-tick"
            style="left: {ratio * 100}%"
            onclick={() => seekTo(c.start_ms)}
            title={`${chapterRangeLabel(c.start_ms, c.end_ms)} — ${c.label}`}
            aria-label={`Jump to ${fmtDurationMs(c.start_ms)}`}
          ></button>
        {/if}
      {/each}
      {#if playheadRatio !== null}
        <div class="chapter-playhead" style="left: {playheadRatio * 100}%"></div>
      {/if}
    </div>

    <ul class="chapter-list">
      {#each chapters as c, i (rowKeys[i])}
        <li class="chapter-row" class:mine={editable}>
          <button
            type="button"
            class="chapter-jump"
            onclick={() => seekTo(c.start_ms)}
            title={`Jump to ${chapterRangeLabel(c.start_ms, c.end_ms)}`}
          >
            <span class="chapter-time">{chapterRangeLabel(c.start_ms, c.end_ms)}</span>
          </button>
          {#if editable}
            <input
              class="chapter-title-input"
              type="text"
              value={c.label}
              placeholder="Untitled section"
              aria-label={`Title of the section at ${fmtDurationMs(c.start_ms)}`}
              onchange={(e) => void rename(c, e.currentTarget.value)}
            />
            <button
              type="button"
              class="chapter-row-action"
              onclick={() => moveHere(c)}
              disabled={busy}
              title={`Move this section to ${fmtDurationMs(positionMs)}`}
              aria-label={`Move the section at ${fmtDurationMs(c.start_ms)} to ${fmtDurationMs(positionMs)}`}
            >⇥</button>
            <button
              type="button"
              class="chapter-row-action danger"
              onclick={() => removeChapter(c)}
              disabled={busy}
              title="Delete"
              aria-label={`Delete the section at ${fmtDurationMs(c.start_ms)}`}
            >✕</button>
          {:else}
            <span class="chapter-title">{c.label}</span>
          {/if}
        </li>
      {/each}
      {#if listNote !== null}
        <li class="chapter-note">
          {loadError ? `chapters unavailable (${loadError})` : listNote}
        </li>
      {/if}
    </ul>

    {#if editable}
      <!-- Compose. The position is shown, not chosen: it is wherever
           playback stands when the section is added. A title is
           optional — an untitled section is a legal one, here and in the
           containers this reads back. -->
      <div class="chapter-compose">
        <span class="chapter-at">at {fmtDurationMs(positionMs)}</span>
        <input
          class="chapter-input"
          type="text"
          placeholder="Section starts here…"
          aria-label="Title of the new section"
          bind:value={draft}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void addChapter();
            }
          }}
        />
        <button
          type="button"
          class="chapter-add-btn"
          onclick={addChapter}
          disabled={busy}
        >{busy ? "saving…" : "Add section"}</button>
      </div>
    {/if}
    {#if writeError}
      <p class="chapter-error">{writeError}</p>
    {/if}
  </section>
{/if}

<style>
  /* Sits under the player beside the marks panel, at the player's
     width. Both hosts (`.detail-media-video` / `.detail-media-audio`)
     are dark-on-light and light-on-dark respectively, so the panel
     carries its own surface rather than inheriting either — the same
     reason `.mark-panel` does. */
  .chapter-panel {
    width: 100%;
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex-shrink: 0;
    box-sizing: border-box;
    padding: 0.4rem 0.5rem;
    background: #ffffff;
    color: #1f1e33;
    border: 1px solid #e4e1f4;
    border-radius: 4px;
  }

  /* Band switcher — chips, in the amber the chapter chips used, so the
     structure surface stays visually apart from the marks' indigo. */
  .chapter-bands {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
  }
  .chapter-band {
    display: inline-flex;
    align-items: baseline;
    gap: 0.3rem;
    background: rgba(250, 204, 21, 0.12);
    border: 1px solid rgba(250, 204, 21, 0.4);
    border-radius: 999px;
    padding: 0.15rem 0.55rem;
    font-size: 0.75rem;
    color: #4a3908;
    cursor: pointer;
  }
  .chapter-band:hover {
    background: rgba(250, 204, 21, 0.24);
  }
  .chapter-band.active {
    background: rgba(250, 204, 21, 0.34);
    border-color: #d9a706;
  }
  /* A band one owns reads as one's own before the buttons say so. */
  .chapter-band.mine {
    border-style: dashed;
  }
  .chapter-band-default {
    font-size: 0.62rem;
    color: #6b571a;
  }
  .chapter-band-add,
  .chapter-band-action {
    background: none;
    border: 1px solid #d6d3ec;
    border-radius: 999px;
    padding: 0.15rem 0.5rem;
    font-size: 0.68rem;
    color: #6a67a4;
    cursor: pointer;
  }
  .chapter-band-add:hover:not(:disabled),
  .chapter-band-action:hover:not(:disabled) {
    background: #f4f2ff;
  }
  .chapter-band-action.danger:hover:not(:disabled) {
    color: #d0393b;
    border-color: #e8b4b5;
    background: #fdf3f3;
  }
  .chapter-band-add:disabled,
  .chapter-band-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Ruler — full timeline width, ticks placed by percentage. */
  .chapter-ruler {
    position: relative;
    width: 100%;
    height: 14px;
    background: rgba(250, 204, 21, 0.18);
    border-radius: 3px;
  }
  .chapter-tick {
    position: absolute;
    top: 0;
    width: 3px;
    height: 100%;
    margin-left: -1px;
    padding: 0;
    border: none;
    border-radius: 1px;
    background: #d9a706;
    cursor: pointer;
  }
  .chapter-tick:hover {
    background: #a37a04;
    width: 5px;
    margin-left: -2px;
  }
  .chapter-playhead {
    position: absolute;
    top: -2px;
    width: 1px;
    height: calc(100% + 4px);
    background: rgba(31, 30, 51, 0.8);
    pointer-events: none;
  }

  /* List — the sections in the band's own reading order (`ord`), which
     need not be the timeline's. */
  .chapter-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    max-height: 180px;
    overflow-y: auto;
  }
  .chapter-row {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    padding: 0.15rem 0.3rem;
    border-left: 3px solid #d9a706;
    background: rgba(250, 204, 21, 0.08);
    border-radius: 3px;
  }
  .chapter-row.mine {
    border-left-style: dashed;
  }
  .chapter-jump {
    background: none;
    border: none;
    padding: 0;
    text-align: left;
    cursor: pointer;
    color: inherit;
    flex-shrink: 0;
  }
  .chapter-time {
    font-family: "SF Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    color: #6b571a;
  }
  .chapter-title {
    flex: 1;
    min-width: 0;
    font-size: 0.78rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chapter-title-input {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    padding: 0.1rem 0.35rem;
    font-size: 0.78rem;
    font-family: inherit;
    background: #fffdf5;
    border: 1px solid #e6dcb4;
    border-radius: 3px;
    outline: none;
    color: #1f1e33;
  }
  .chapter-title-input:focus {
    border-color: #d9a706;
    background: #ffffff;
  }
  .chapter-row-action {
    background: none;
    border: none;
    color: #b7b1e5;
    cursor: pointer;
    font-size: 0.72rem;
    line-height: 1;
    padding: 0;
    flex-shrink: 0;
  }
  .chapter-row-action:hover:not(:disabled) {
    color: #4a3908;
  }
  .chapter-row-action.danger:hover:not(:disabled) {
    color: #d0393b;
  }
  .chapter-row-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .chapter-note {
    font-size: 0.72rem;
    color: #9c98c9;
    padding: 0.15rem 0.3rem;
  }

  /* Compose — mirrors `.mark-compose`, so adding a section and writing a
     mark are the same gesture at the same place on screen. */
  .chapter-compose {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .chapter-at {
    font-family: "SF Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    color: #6b571a;
    flex-shrink: 0;
  }
  .chapter-input {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    padding: 0.25rem 0.45rem;
    font-size: 0.78rem;
    font-family: inherit;
    background: #fafafd;
    border: 1px solid #d6d3ec;
    border-radius: 4px;
    outline: none;
    color: #1f1e33;
  }
  .chapter-input:focus {
    border-color: #d9a706;
    background: #ffffff;
  }
  .chapter-add-btn {
    padding: 0.25rem 0.7rem;
    background: #d9a706;
    color: #ffffff;
    border: none;
    border-radius: 4px;
    font-size: 0.75rem;
    cursor: pointer;
    flex-shrink: 0;
  }
  .chapter-add-btn:hover:not(:disabled) {
    background: #b98d05;
  }
  .chapter-add-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .chapter-error {
    margin: 0;
    font-size: 0.7rem;
    color: #d0393b;
  }
</style>
