<script lang="ts">
  // SessionCommentsHover — read-only floating panel that aggregates
  // every asset_comment attached to any Asset inside the given
  // Session and renders them as one chronological time-line.
  //
  // Rationale:
  //   * There is no first-class SessionComment model yet; the UI
  //     reuses the existing `asset_comment` API so the comment
  //     surface can already answer "what did I / persona note about
  //     this session as a whole" without a backend schema change.
  //   * Posting is deliberately not exposed here — a per-message
  //     comment still routes through DetailPane; a session-scope
  //     comment will land once SessionComment exists.
  //
  // Comment fetch shape:
  //   1. `list_assets` with `session_id: sessionId` to enumerate the
  //      Session's assets (single round-trip, backend-guarded page
  //      size).
  //   2. Parallel `list_asset_comments` per asset via `Promise.all`.
  //      This is N+1 in round-trip count but N is bounded by the
  //      session's message_count (dogfood: <= a few hundred; the
  //      Tauri IPC layer coalesces the queue). A dedicated
  //      `list_session_comments` endpoint is left for the backend
  //      change that introduces SessionComment.
  //   3. Flatten and sort ascending by `created_at_ms` (the DTO does
  //      not carry `occurred_at_ms`; created is the closest analogue
  //      of "when the note was written").
  //
  // Close paths: Escape key, backdrop click, and the ✕ button all
  // fire `onClose()` — the parent (`App.svelte`) owns the mount
  // gate and clears its `sessionCommentsHover` state on close.
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import type {
    AssetCardDto,
    AssetCommentDto,
    AssetPageDto,
    ListAssetsQuery,
  } from "./bindings";
  import { noteAuthorLabel } from "./lib/formatters";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { profileCatalog } from "./lib/stores/profile.svelte";

  interface Props {
    sessionId: string;
    x: number;
    y: number;
    onClose: () => void;
  }

  let { sessionId, x, y, onClose }: Props = $props();

  // Per-comment view row — pairs the comment with a small amount of
  // asset context so the time-line line can label "which message did
  // this note attach to" without another lookup.
  type Row = {
    comment: AssetCommentDto;
    asset: AssetCardDto;
  };

  let loading = $state(true);
  let error = $state<string | null>(null);
  let rows = $state<Row[]>([]);
  // Compose state — appending to a Session unit is realised by
  // attaching an asset_comment to the Session's oldest message (the
  // representative asset). Introducing SessionComment as its own
  // model would require a backend schema change, so this reuses the
  // existing API to secure the compose path first.
  let representative = $state<AssetCardDto | null>(null);
  let draft = $state("");
  let authorKind = $state<"user" | "persona">("user");
  let posting = $state(false);
  let postError = $state<string | null>(null);

  function fmtTime(ms: number): string {
    const d = new Date(ms);
    const y = d.getFullYear();
    const mo = String(d.getMonth() + 1).padStart(2, "0");
    const da = String(d.getDate()).padStart(2, "0");
    const hh = String(d.getHours()).padStart(2, "0");
    const mm = String(d.getMinutes()).padStart(2, "0");
    return `${y}-${mo}-${da} ${hh}:${mm}`;
  }

  async function loadAll(): Promise<void> {
    loading = true;
    error = null;
    try {
      // Session sizes are bounded (dogfood tops out in the low
      // hundreds); one big page beats paging round-trips here.
      const query: ListAssetsQuery = {
        viewer_subject: null,
        persona_id: null,
        modality: null,
        occurred_from_ms: null,
        occurred_until_ms: null,
        // Ingest / modification windows are differential-sync axes for
        // API consumers; this panel wants the Session's whole comment
        // history, so it asks for no window on either.
        created_from_ms: null,
        created_until_ms: null,
        updated_from_ms: null,
        updated_until_ms: null,
        // Comment aggregation is a live-set view; a trashed message
        // should not contribute to a Session's comment panel.
        trash: "live",
        tag_ids: [],
        // No tags are asked for, so the composition is inert — spelled
        // out only because the wire type requires it.
        tag_match: "any",
        group_ids: [],
        session_id: sessionId,
        label: null,
        text_match: null,
        // `session_id` alone is the drill — it asks for the rows filed
        // inside this container, which is a filter, not a visibility
        // rule.
        format: null,
        color: null,
        // The comment panel aggregates every message in the Session
        // regardless of star rating, so no band is asked for.
        rating_min: null,
        rating_max: null,
        // AlbumMeta is what somebody said about a row; narrowing a
        // Session's comment history by one would answer about the
        // messages that happen to carry a statement.
        album_meta_key: null,
        album_meta_value: null,
        // Nor by length or size. Both ends of both bands stay open on
        // purpose: naming either end would drop the rows whose column is
        // NULL, and a text message has no length, no measured dimensions
        // and — on the card projection — no recorded size, so the panel
        // would go empty.
        duration_min_ms: null,
        duration_max_ms: null,
        size_min_bytes: null,
        size_max_bytes: null,
        pixels_min: null,
        pixels_max: null,
        // The panel groups by comment, not by a display order, so it takes
        // whatever order the container's rows arrive in.
        sort: null,
        offset: 0,
        limit: 500,
      };
      const page = await invoke<AssetPageDto>("list_assets", { query });
      const assets = page.items;
      // Representative asset for compose target — the oldest message
      // (MIN occurred_at). A session-wide meta comment is treated with
      // the "attach to the Session's oldest message" semantic.
      if (assets.length > 0) {
        representative = [...assets].sort(
          (a, b) => a.occurred_at_ms - b.occurred_at_ms,
        )[0];
      } else {
        representative = null;
      }
      if (assets.length === 0) {
        rows = [];
        return;
      }
      // Parallel fan-out — Promise.all so a slow one does not
      // gate the fast ones. Each rejection is caught locally so
      // one failed asset does not empty the whole panel.
      const perAsset = await Promise.all(
        assets.map(async (a) => {
          try {
            const cs = await invoke<AssetCommentDto[]>("list_asset_comments", {
              assetId: a.id,
            });
            return cs.map((c) => ({ comment: c, asset: a }) as Row);
          } catch {
            return [] as Row[];
          }
        }),
      );
      const flat = perAsset.flat();
      flat.sort((a, b) => a.comment.created_at_ms - b.comment.created_at_ms);
      rows = flat;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      rows = [];
    } finally {
      loading = false;
    }
  }

  function handleKey(ev: KeyboardEvent) {
    if (ev.key === "Escape") {
      ev.preventDefault();
      onClose();
    }
  }

  onMount(() => {
    void loadAll();
    window.addEventListener("keydown", handleKey);
  });
  onDestroy(() => {
    window.removeEventListener("keydown", handleKey);
  });

  // Author label — `noteAuthorLabel` is the shared reading, so "You" /
  // persona name / "(deleted persona)" come out identically here, on
  // the card thread, and on the material marks list.
  function authorLabel(c: AssetCommentDto): string {
    return noteAuthorLabel(c.author_kind, c.author_persona_id);
  }

  // Short excerpt of the message the comment attaches to — helps the
  // reader identify which message the note is about when the same
  // Session has dozens of messages.
  function assetExcerpt(a: AssetCardDto): string {
    const cover = (a.cover ?? "").trim();
    if (cover.length === 0) return "(no cover)";
    return cover.length > 60 ? `${cover.slice(0, 60)}…` : cover;
  }

  async function submit(): Promise<void> {
    const body = draft.trim();
    if (body.length === 0 || representative === null) return;
    let author_persona_id: string | null = null;
    if (authorKind === "persona") {
      // Persona post — prefer the sidebar-active persona, falling
      // back to the representative asset's owning persona (same
      // pattern as DetailPane).
      author_persona_id = activeFilter.activePersona ?? representative.persona_id;
    }
    posting = true;
    postError = null;
    try {
      await invoke<AssetCommentDto>("post_asset_comment", {
        command: {
          asset_id: representative.id,
          author_kind: authorKind,
          author_persona_id,
          body,
        },
      });
      draft = "";
      await loadAll();
    } catch (err) {
      postError = err instanceof Error ? err.message : String(err);
    } finally {
      posting = false;
    }
  }
</script>

<div
  class="session-comments-backdrop"
  onclick={onClose}
  role="button"
  tabindex="-1"
  aria-label="Close session comments"
></div>

<div
  class="session-comments-panel card-thread-overlay"
  style="left: {x}px; top: {y}px;"
  role="dialog"
  aria-label="Session comments"
  onclick={(e) => e.stopPropagation()}
>
  <div class="head">
    <span class="head-title">Session Comments</span>
    <button
      type="button"
      class="close-btn"
      aria-label="Close"
      onclick={onClose}
    >✕</button>
  </div>
  {#if loading}
    <p class="empty">loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if rows.length === 0}
    <p class="empty">no comments in this session</p>
  {:else}
    <ul class="rows">
      {#each rows as r (r.comment.id)}
        <li class="row" class:persona={r.comment.author_kind === "persona"}>
          <header class="row-head">
            {#if r.comment.author_kind === "persona"}
              {@const av = profileCatalog.personaAvatarUrl(
                r.comment.author_persona_id,
              )}
              {#if av}
                <img class="avatar" src={av} alt="" />
              {:else}
                <span class="avatar-placeholder">○</span>
              {/if}
            {:else}
              <span class="avatar-placeholder user">You</span>
            {/if}
            <span class="author">{authorLabel(r.comment)}</span>
            <span class="time">{fmtTime(r.comment.created_at_ms)}</span>
          </header>
          <p class="body">{r.comment.body}</p>
          <p class="asset-hint" title={r.asset.cover ?? ""}>
            on: {assetExcerpt(r.asset)}
          </p>
        </li>
      {/each}
    </ul>
  {/if}
  {#if representative !== null}
    <div class="compose">
      <div class="compose-head">
        <label class="kind-toggle">
          <input type="radio" bind:group={authorKind} value="user" /> You
        </label>
        <label class="kind-toggle">
          <input type="radio" bind:group={authorKind} value="persona" /> Persona
        </label>
      </div>
      <textarea
        class="compose-input"
        placeholder="Add a comment to this session…"
        bind:value={draft}
        disabled={posting}
        onkeydown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void submit();
          }
        }}
      ></textarea>
      {#if postError}
        <p class="error compose-error">{postError}</p>
      {/if}
      <div class="compose-foot">
        <span class="compose-hint">
          Cmd/Ctrl+Enter · attaches to first message
        </span>
        <button
          type="button"
          class="post-btn"
          onclick={submit}
          disabled={posting || draft.trim().length === 0}
        >{posting ? "posting…" : "Post"}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  /* Backdrop — a translucent capture layer so a click outside the
     panel closes it. Kept low-opacity so the Sessions grid behind
     stays visible (the panel is a peek, not a full modal). */
  .session-comments-backdrop {
    position: fixed;
    inset: 0;
    background: transparent;
    z-index: 54;
  }
  /* Panel visual language mirrors `.card-thread-overlay` in
     App.svelte (Note / Thread hover) so the three overlays read as
     siblings. Width is a touch wider because a session's comments
     usually have longer text. */
  .session-comments-panel {
    position: fixed;
    width: 340px;
    max-height: 380px;
    background: var(--surface-raised);
    border: 1px solid var(--accent-line);
    border-radius: 8px;
    box-shadow: 0 12px 30px var(--shadow-color);
    z-index: 55;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--ink);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.72rem;
    padding: 0.35rem 0.5rem 0.35rem 0.7rem;
    background: var(--accent-surface);
    color: var(--accent-ink);
    font-weight: 600;
    border-bottom: 1px solid var(--accent-line);
  }
  .head-title {
    letter-spacing: 0.02em;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: var(--accent-ink);
    cursor: pointer;
    font-size: 0.85rem;
    line-height: 1;
    padding: 0.1rem 0.35rem;
  }
  .close-btn:hover {
    color: var(--ink);
  }
  .rows {
    list-style: none;
    padding: 0.4rem 0.5rem;
    margin: 0;
    overflow-y: auto;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .row {
    border-left: 2px solid var(--accent-line-strong);
    padding: 0.3rem 0.5rem;
    background: var(--accent-surface);
    border-radius: 3px;
    font-size: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .row.persona {
    border-left-color: var(--accent-line-strong);
    background: var(--accent-surface);
  }
  .row-head {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .avatar {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    object-fit: cover;
  }
  .avatar-placeholder {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--accent-surface);
    color: var(--accent-ink);
    font-size: 0.55rem;
    font-weight: 600;
    flex-shrink: 0;
  }
  .avatar-placeholder.user {
    background: var(--accent-surface-strong);
    color: var(--ink);
  }
  .author {
    font-weight: 600;
    color: var(--ink);
    flex-shrink: 0;
  }
  .time {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
    color: var(--accent-ink);
    font-size: 0.65rem;
  }
  .body {
    margin: 0;
    color: var(--ink);
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .asset-hint {
    margin: 0;
    color: var(--accent-ink);
    font-size: 0.65rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    margin: 0;
    padding: 1rem 0.75rem;
    text-align: center;
    color: var(--accent-ink);
    font-size: 0.75rem;
  }
  .error {
    margin: 0;
    padding: 0.6rem 0.75rem;
    color: var(--danger-ink);
    background: var(--danger-surface);
    border-top: 1px solid var(--danger-line);
    font-size: 0.75rem;
  }
  /* Compose section — mirrors DetailPane's comment compose tone
     (accent-tinted frame, monotone textarea, primary Post button)
     so the Session and Message comment compose UIs stay visually
     unified. */
  .compose {
    border-top: 1px solid var(--accent-line);
    padding: 0.5rem 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    background: var(--surface-raised);
  }
  .compose-head {
    display: flex;
    gap: 0.6rem;
    font-size: 0.68rem;
    color: var(--accent-ink);
  }
  .kind-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    cursor: pointer;
  }
  .compose-input {
    width: 100%;
    box-sizing: border-box;
    min-height: 3rem;
    padding: 0.3rem 0.4rem;
    border: 1px solid var(--accent-line);
    border-radius: 4px;
    background: var(--surface-raised);
    color: var(--ink);
    font-size: 0.75rem;
    font-family: inherit;
    resize: vertical;
  }
  .compose-input:focus {
    outline: none;
    border-color: var(--accent-line-strong);
  }
  .compose-error {
    padding: 0.3rem 0.4rem;
    border-top: none;
    border-radius: 3px;
  }
  .compose-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .compose-hint {
    font-size: 0.62rem;
    color: var(--accent-ink);
  }
  .post-btn {
    background: var(--accent-fill);
    color: var(--accent-on-fill);
    border: none;
    border-radius: 4px;
    padding: 0.25rem 0.7rem;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
  }
  .post-btn:disabled {
    background: var(--accent-surface-strong);
    cursor: not-allowed;
  }
  .post-btn:hover:not(:disabled) {
    background: var(--accent-fill-hover);
  }
</style>
