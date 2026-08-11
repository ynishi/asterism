<script lang="ts">
  // SessionTile — a single Session card, extracted from the retired
  // `SessionsView.svelte`.
  // The Sessions tab is gone: Sessions surface directly in the
  // Messages grid whenever the active modality is "dialogue". This
  // component wraps one Session tile with the same DOM / CSS the old
  // view rendered (inline title edit + delete + cover + meta +
  // CardActionIcons) so App.svelte can drop tiles straight into the
  // shared responsive grid alongside Message cards.
  //
  // Props are the App-owned side effects (drill open + note / comment
  // overlay openers). Data (`session`) is a plain prop rather than a
  // catalog read so App can interleave Sessions and Messages in one
  // `filteredRows` pass without teaching this component about the
  // union type.
  //
  // Write coordination (rename / delete) lives locally because it
  // fires directly on `sessionCatalog.rename` / `remove`. The tile is
  // the sole surface that can trigger these writes now, so keeping
  // the error banner scoped to this component matches the ownership.
  import type { SessionDto } from "./bindings";
  import CardActionIcons from "./CardActionIcons.svelte";
  import { personaName } from "./lib/formatters";
  import { sessionCatalog } from "./lib/stores/session.svelte";

  interface Props {
    session: SessionDto;
    onOpen: (s: SessionDto) => void;
    /**
     * Note affordance — same 📝 icon that lives on the Messages
     * grid Card, threaded through the shared `CardActionIcons`
     * component. App owns the overlay (shared
     * `cardNoteHover` state) so Session and Message share one
     * uniform UI.
     */
    onOpenNote?: (s: SessionDto, ev: MouseEvent) => void;
    /**
     * Comment affordance — 💬 icon opens the Session-scope comments
     * panel (`SessionCommentsHover`) which aggregates every
     * asset_comment attached to the Session's messages.
     */
    onOpenComment?: (s: SessionDto, ev: MouseEvent) => void;
  }

  let { session, onOpen, onOpenNote, onOpenComment }: Props = $props();

  // Compact `MM-DD` renderer. Same format the Message tile uses for
  // `.date` so mixed rows line up visually.
  function fmtDate(ms: number): string {
    const d = new Date(ms);
    return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  let writeError = $state<string | null>(null);

  function errMsg(e: unknown): string {
    if (typeof e === "string") return e;
    if (e && typeof e === "object" && "message" in e) {
      return String((e as { message: unknown }).message);
    }
    return String(e);
  }

  async function commitTitle(next: string): Promise<void> {
    const trimmed = next.trim();
    const current = (session.title ?? "").trim();
    if (trimmed === current) return;
    // Empty string means "clear back to untitled" — the rename
    // endpoint is the sole path that expresses NULL because
    // patchMetadata treats null as "leave unchanged".
    const payload = trimmed.length > 0 ? trimmed : null;
    try {
      writeError = null;
      await sessionCatalog.rename(session.id, payload);
    } catch (e) {
      writeError = errMsg(e);
    }
  }

  async function del(): Promise<void> {
    if (session.message_count > 0) return;
    try {
      writeError = null;
      await sessionCatalog.remove(session.id);
    } catch (e) {
      writeError = errMsg(e);
    }
  }
</script>

<!-- Outer wrapper is a plain <div> so the inline title <input> /
     ✕ button can nest as focusable children without the
     "interactive inside a button" a11y warning. Inner `.drill`
     button fires the drill-in action; nested controls
     stopPropagation so they never trigger it. -->
<div class="card session-card session-tile">
  <div class="card-head">
    <input
      class="title-input"
      type="text"
      value={session.title ?? ""}
      placeholder={session.external_key}
      onclick={(e) => e.stopPropagation()}
      onblur={(e) => commitTitle(e.currentTarget.value)}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          e.currentTarget.blur();
        }
      }}
    />
    <span class="date">{fmtDate(session.ended_at_ms)}</span>
    <button
      type="button"
      class="del"
      disabled={session.message_count > 0}
      title={session.message_count > 0
        ? "Cannot delete while messages exist"
        : "Delete session"}
      onclick={(e) => {
        e.stopPropagation();
        del();
      }}>✕</button
    >
  </div>
  <button
    type="button"
    class="drill"
    onclick={() => onOpen(session)}
    title="Open messages in this session"
  >
    <p class="cover">{session.cover_hint ?? "(no cover yet)"}</p>
    <div class="session-meta">
      <span class="session-count">{session.message_count} msg</span>
      <span class="session-range">
        {fmtDate(session.started_at_ms)} → {fmtDate(session.ended_at_ms)}
      </span>
    </div>
    <p class="persona-name">{personaName(session.persona_id)}</p>
  </button>
  {#if writeError}
    <p class="session-error">{writeError}</p>
  {/if}
  <!-- Floating action strip — same `CardActionIcons` component the
       Messages grid Card uses. `showConstellation={false}` drops ✦:
       Session is not an Asset so there is no per-tile constellation
       grouping to open. `hasThread` is hard-coded to `false`
       because the SessionDto does not carry an aggregated comment
       count yet — `has_comments` arrives with the SessionComment
       model. -->
  <CardActionIcons
    hasNote={!!session.note}
    hasThread={false}
    showConstellation={false}
    onNoteClick={(e) => {
      e.stopPropagation();
      onOpenNote?.(session, e);
    }}
    onThreadClick={(e) => {
      e.stopPropagation();
      onOpenComment?.(session, e);
    }}
  />
</div>

<style>
  /* Base card shell — mirrors App's `.card`. `position: relative` so
     the absolutely-positioned CardActionIcons strip lands inside
     this box. Same measurements as the message tile so a Session
     card and a Message card slot into the same responsive grid
     (`auto-fill minmax(180px, 1fr)`) without a size discontinuity. */
  .card {
    position: relative;
    background: #fff;
    border: 1px solid #e6e6e2;
    border-radius: 8px;
    padding: 0.6rem;
    min-height: 90px;
    transition:
      border-color 0.1s,
      transform 0.1s;
  }

  /* Session-tile variant — softer border tone to distinguish from a
     plain Message tile at a glance. */
  .session-card {
    text-align: left;
    border: 1px solid #eeecf8;
    cursor: pointer;
    font-family: inherit;
  }
  .session-card:hover {
    background: #f8f7fd;
  }

  /* Card header row (mirrors App's `.card-head`). */
  .card-head {
    display: flex;
    justify-content: space-between;
    margin-bottom: 0.35rem;
  }
  .date {
    font-size: 0.65rem;
    color: #aaa;
  }

  /* Cover preview — 3-line clamp (messages grid uses the same limit). */
  .cover {
    font-size: 0.8rem;
    line-height: 1.45;
    margin: 0 0 0.4rem;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  /* Session-specific meta row: message count pill + occurred_at
     range. */
  .session-meta {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: 0.6rem;
    color: #9a96d9;
    margin-top: 0.3rem;
  }
  .session-count {
    background: #f0effc;
    color: #7a76c9;
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
    font-variant-numeric: tabular-nums;
  }
  .session-range {
    font-variant-numeric: tabular-nums;
    opacity: 0.7;
  }

  /* Persona the session is filed under (membership, not authorship) —
     same treatment as the messages grid. */
  .persona-name {
    font-size: 0.65rem;
    color: #bbb;
    margin: 0;
  }

  /* Title <input> replaces the read-only badge — flush with the
     card head, no chrome by default so untouched tiles read as
     labels; a subtle border appears on focus so the user sees the
     click target. */
  .title-input {
    flex: 1;
    min-width: 0;
    padding: 0.05rem 0.35rem;
    font-size: 0.75rem;
    font-family: inherit;
    color: #333;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
  }
  .title-input:hover {
    border-color: #eee;
    background: #fafafa;
  }
  .title-input:focus {
    outline: none;
    border-color: #c9c4e0;
    background: #fff;
  }

  /* Delete button. */
  .del {
    background: none;
    border: none;
    cursor: pointer;
    color: #b46;
    font-size: 0.8rem;
    padding: 0 0.25rem;
    line-height: 1;
  }
  .del:hover:not(:disabled) {
    color: #922;
  }
  .del:disabled {
    color: #ddd;
    cursor: default;
  }

  /* Drill-in button wraps the "clickable body" (cover + meta +
     persona name). Bare-<button> reset so the region reads like a
     card body while staying keyboard-focusable. */
  .drill {
    display: block;
    width: 100%;
    padding: 0;
    background: none;
    border: none;
    text-align: left;
    cursor: pointer;
    color: inherit;
    font-family: inherit;
    font-size: inherit;
  }
  .drill:hover {
    background: transparent; /* card-level hover handles the tint */
  }

  /* Per-tile write-error banner. */
  .session-error {
    margin: 0.4rem 0 0;
    padding: 0.4rem 0.6rem;
    background: #fdecec;
    border: 1px solid #f3c6c6;
    border-radius: 4px;
    color: #922;
    font-size: 0.75rem;
  }
</style>
