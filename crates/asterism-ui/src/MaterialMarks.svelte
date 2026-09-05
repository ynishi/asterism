<script lang="ts">
  // MaterialMarks — the marks written inside a playable asset, on the
  // timeline they were written on.
  //
  // The thread in DetailPane's meta column is the other half of the
  // pair: an `asset_comment` is a note *about* the asset and arrives in
  // the order it was written, a `material_mark` is a note *at a
  // position inside* what the asset holds and arrives in the material's
  // own order (`start_ms` ascending). The four verbs — write, read,
  // jump, order — are the thread's four; only the ordering differs.
  //
  // Ownership: this component owns the marks of exactly one asset and
  // fetches them itself, the same shape the comment thread uses inside
  // DetailPane (a component may own state that nothing else reads; a
  // catalog would only add a reload orchestration question that no
  // second reader is asking). Three props:
  //   * `assetId` / `durationMs` — the asset in the pane and whether
  //     its material has a timeline at all.
  //   * `media` — the live `<video>` / `<audio>` element, a `bind:this`
  //     handle passed down. Marks are read *from* it (the
  //     playhead a new mark is stamped at) and written *to* it (a click
  //     on a mark seeks). Duplicating the element here instead would
  //     mean a second decoder for the same file.
  //
  // Renders nothing at all when the asset has no timeline — a still
  // image has nowhere to put a temporal anchor and the service refuses
  // one (`material_mark_service.rs`, the `temporal` arm).
  import { api } from "./lib/api";
  import {
    buildPostCommand,
    currentMarkId,
    hasTimeline,
    markRatio,
    positionMsFromMedia,
  } from "./lib/material-mark";
  import { fmtDurationMs, noteAuthorLabel } from "./lib/formatters";
  import type { MaterialMarkDto } from "./bindings";

  interface Props {
    assetId: string;
    durationMs: number | null;
    media: HTMLMediaElement | null;
  }

  let { assetId, durationMs, media }: Props = $props();

  // The list is held in the order the backend handed it over. The
  // repository orders by `start_ms` with id as the tie-break, which is
  // the material's own order; re-sorting here would put a second
  // opinion about ordering in the client.
  let marks = $state<MaterialMarkDto[]>([]);
  let loadError = $state<string | null>(null);
  let draft = $state("");
  let posting = $state(false);
  let postError = $state<string | null>(null);
  // Playhead, mirrored from the media element so the ruler and the
  // "mark at …" label follow playback.
  let positionMs = $state(0);

  const timeline = $derived(hasTimeline(durationMs));
  const activeMarkId = $derived(currentMarkId(marks, positionMs));
  // Derived rather than an `{@const}` in the ruler: `{@const}` is only
  // legal as the immediate child of a block, and the playhead sits
  // beside the `{#each}` rather than inside it.
  const playheadRatio = $derived(markRatio(positionMs, durationMs));

  async function load(id: string): Promise<void> {
    try {
      const rows = await api<MaterialMarkDto[]>("list_material_marks", {
        assetId: id,
      });
      // The pane may have moved to another asset while this was in
      // flight; dropping the stale answer keeps one asset's marks from
      // appearing under another.
      if (assetId !== id) return;
      marks = rows;
      loadError = null;
    } catch (err) {
      if (assetId !== id) return;
      console.warn("list_material_marks failed", err);
      marks = [];
      loadError = err instanceof Error ? err.message : String(err);
    }
  }

  $effect(() => {
    const id = assetId;
    // Reading `durationMs` through the derived registers it, so an
    // asset whose duration arrives late still gets its marks fetched.
    if (!timeline) {
      marks = [];
      loadError = null;
      return;
    }
    draft = "";
    postError = null;
    void load(id);
  });

  // Follow the element's playhead. `timeupdate` fires a few times a
  // second during playback; `seeked` and `loadedmetadata` cover the
  // transitions it does not report.
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

  // --- the four verbs ---------------------------------------------------

  /// Write. The position is read at the moment of the click, not at the
  /// moment the draft was typed — the user watches, sees the thing,
  /// then writes what it was.
  async function post(): Promise<void> {
    const command = buildPostCommand(assetId, positionMs, draft);
    if (command === null) return;
    posting = true;
    postError = null;
    try {
      await api<MaterialMarkDto>("post_material_mark", { command });
      draft = "";
      // Re-read rather than splice the new row in: its place in the
      // list is decided by `start_ms`, and that decision belongs to
      // the same side that made it for every other row.
      await load(assetId);
    } catch (err) {
      console.warn("post_material_mark failed", err);
      postError = err instanceof Error ? err.message : String(err);
    } finally {
      posting = false;
    }
  }

  /// Jump. Same move the chapter ticks make in DetailPane.
  function seekTo(startMs: number | null): void {
    if (media === null || startMs === null) return;
    media.currentTime = startMs / 1000;
  }

  async function remove(markId: string): Promise<void> {
    try {
      await api<void>("delete_material_mark", {
        command: { mark_id: markId },
      });
      // Dropping the row keeps the order it already had; nothing about
      // the remaining marks' positions changed.
      marks = marks.filter((m) => m.id !== markId);
    } catch (err) {
      console.warn("delete_material_mark failed", err);
      postError = err instanceof Error ? err.message : String(err);
    }
  }

  function authorLabel(m: MaterialMarkDto): string {
    return noteAuthorLabel(m.author_kind, m.author_persona_id);
  }
</script>

{#if timeline}
  <section class="mark-panel" aria-label="Marks on this timeline">
    <!-- Ruler — the marks laid out where they sit, so the shape of the
         annotated material is readable before any of it is read. The
         waveform canvas next to it does the same for audio, but only
         audio has one (decoding a whole video for peaks OOMs the
         webview), so the ruler is drawn in the DOM and serves both. -->
    <div class="mark-ruler">
      {#each marks as m (m.id)}
        {@const ratio = markRatio(m.start_ms, durationMs)}
        {#if ratio !== null}
          <button
            type="button"
            class="mark-tick"
            class:current={m.id === activeMarkId}
            style="left: {ratio * 100}%"
            onclick={() => seekTo(m.start_ms)}
            title={`${fmtDurationMs(m.start_ms)} — ${m.body}`}
            aria-label={`Jump to ${fmtDurationMs(m.start_ms)}`}
          ></button>
        {/if}
      {/each}
      {#if playheadRatio !== null}
        <div class="mark-playhead" style="left: {playheadRatio * 100}%"></div>
      {/if}
    </div>

    <ul class="mark-list">
      {#each marks as m (m.id)}
        <li class="mark-row" class:current={m.id === activeMarkId} class:persona={m.author_kind === "persona"}>
          <button
            type="button"
            class="mark-jump"
            onclick={() => seekTo(m.start_ms)}
            title={`Jump to ${fmtDurationMs(m.start_ms)}`}
          >
            <span class="mark-time">{fmtDurationMs(m.start_ms)}</span>
            <span class="mark-body">{m.body}</span>
          </button>
          <span class="mark-author">{authorLabel(m)}</span>
          <button
            type="button"
            class="mark-delete"
            onclick={() => remove(m.id)}
            title="Delete"
            aria-label="Delete mark"
          >✕</button>
        </li>
      {/each}
      {#if marks.length === 0}
        <li class="mark-empty">
          {loadError ? `marks unavailable (${loadError})` : "No marks yet."}
        </li>
      {/if}
    </ul>

    <div class="mark-compose">
      <span class="mark-at">at {fmtDurationMs(positionMs)}</span>
      <input
        class="mark-input"
        type="text"
        placeholder="Mark this moment…"
        bind:value={draft}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void post();
          }
        }}
      />
      <button
        type="button"
        class="mark-post-btn"
        onclick={post}
        disabled={posting || draft.trim().length === 0}
      >{posting ? "marking…" : "Mark"}</button>
    </div>
    {#if postError}
      <p class="mark-error">{postError}</p>
    {/if}
  </section>
{/if}

<style>
  /* Sits under the player, at the player's width. Both hosts
     (`.detail-media-video` / `.detail-media-audio`) are dark-on-light
     and light-on-dark respectively, so the panel carries its own
     surface rather than inheriting either. */
  .mark-panel {
    width: 100%;
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex-shrink: 0;
    box-sizing: border-box;
    padding: 0.4rem 0.5rem;
    background: var(--surface-raised);
    color: var(--ink);
    border: 1px solid var(--accent-line);
    border-radius: 4px;
  }

  /* Ruler — full timeline width, ticks placed by percentage. */
  .mark-ruler {
    position: relative;
    width: 100%;
    height: 18px;
    background: var(--accent-surface);
    border-radius: 3px;
  }
  .mark-tick {
    position: absolute;
    top: 0;
    width: 3px;
    height: 100%;
    margin-left: -1px;
    padding: 0;
    border: none;
    border-radius: 1px;
    background: var(--accent-fill);
    cursor: pointer;
  }
  .mark-tick:hover {
    background: var(--accent-fill-hover);
    width: 5px;
    margin-left: -2px;
  }
  .mark-tick.current {
    background: var(--cat-rose);
  }
  .mark-playhead {
    position: absolute;
    top: -2px;
    width: 1px;
    height: calc(100% + 4px);
    background: var(--ink);
    pointer-events: none;
  }

  /* List — the marks in the material's order, one row each. */
  .mark-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    max-height: 180px;
    overflow-y: auto;
  }
  .mark-row {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    padding: 0.15rem 0.3rem;
    border-left: 3px solid var(--accent-line-strong);
    background: var(--accent-surface);
    border-radius: 3px;
  }
  .mark-row.persona {
    border-left-color: var(--cat-orchid);
    background: var(--accent-surface);
  }
  .mark-row.current {
    background: var(--surface-hover);
  }
  .mark-jump {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    background: none;
    border: none;
    padding: 0;
    text-align: left;
    cursor: pointer;
    color: inherit;
    font-size: 0.78rem;
  }
  .mark-time {
    font-family: "SF Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    color: var(--accent-ink);
    flex-shrink: 0;
  }
  .mark-body {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mark-author {
    font-size: 0.62rem;
    color: var(--accent-ink);
    flex-shrink: 0;
  }
  .mark-delete {
    background: none;
    border: none;
    color: var(--accent-ink-dim);
    cursor: pointer;
    font-size: 0.72rem;
    line-height: 1;
    padding: 0;
    flex-shrink: 0;
  }
  .mark-delete:hover {
    color: var(--danger-ink);
  }
  .mark-empty {
    font-size: 0.72rem;
    color: var(--accent-ink);
    padding: 0.15rem 0.3rem;
  }

  /* Compose — the position is shown, not chosen: it is wherever
     playback stands when Mark is pressed. */
  .mark-compose {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .mark-at {
    font-family: "SF Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    color: var(--accent-ink);
    flex-shrink: 0;
  }
  .mark-input {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    padding: 0.25rem 0.45rem;
    font-size: 0.78rem;
    font-family: inherit;
    background: var(--surface-raised);
    border: 1px solid var(--accent-line);
    border-radius: 4px;
    outline: none;
    color: var(--ink);
  }
  .mark-input:focus {
    border-color: var(--accent-line-strong);
    background: var(--surface-raised);
  }
  .mark-post-btn {
    padding: 0.25rem 0.7rem;
    background: var(--accent-fill);
    color: var(--accent-on-fill);
    border: none;
    border-radius: 4px;
    font-size: 0.75rem;
    cursor: pointer;
    flex-shrink: 0;
  }
  .mark-post-btn:hover:not(:disabled) {
    background: var(--accent-fill-hover);
  }
  .mark-post-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .mark-error {
    margin: 0;
    font-size: 0.7rem;
    color: var(--danger-ink);
  }
</style>
