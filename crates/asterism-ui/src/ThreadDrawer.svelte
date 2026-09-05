<script lang="ts">
  // ThreadDrawer — App-level Threads primitive drawer (replaces the
  // legacy MEMO dialog).
  //
  // Layout: two-pane inside a right-side drawer.
  //   - Left mini-panel: Thread list (newest first) + "new thread"
  //     button. Selecting a row focuses that Thread.
  //   - Right main pane: Message list (chronological) + a composer
  //     footer. Author bubble differs by `author_kind` so a Claude
  //     Code write and a human note interleave visually — the "Context
  //     → Action" flow.
  //
  // 0-prop by design. Reads
  // `threadsCatalog` directly. The App-side `$effect` orchestrates:
  //   - drawer open + anchor change → `threadsCatalog.load(anchor)`
  //   - active thread change → `threadsCatalog.loadMessages(id)`
  //   - poll cadence (P1) → `threadsCatalog.refreshMessages(id)`.
  //     P2 replaces the poll with an SSE subscription; this component
  //     stays unchanged (catalog contract is stable).

  import { confirmCatalog } from "./lib/stores/confirm.svelte";
  import { threadsCatalog } from "./lib/stores/threads.svelte";
  import type { ThreadDto, MessageDto } from "./bindings";

  interface Props {
    open: boolean;
    onClose: () => void;
  }
  let { open, onClose }: Props = $props();

  // Local composer state — component-local, because the composer is
  // not shared state.
  let composerBody = $state("");
  let composerBusy = $state(false);
  let composerError = $state<string | null>(null);

  // New-thread inline form.
  let showNewThreadForm = $state(false);
  let newThreadTitle = $state("");
  let newThreadBusy = $state(false);

  const threads = $derived.by<ThreadDto[]>(() => {
    const rows = Array.from(threadsCatalog.threads.values());
    // freshest first — matches server-side sort but sorts again in
    // case optimistic writes shifted the ordering.
    rows.sort((a, b) => {
      const av = a.last_message_at_ms ?? a.created_at_ms;
      const bv = b.last_message_at_ms ?? b.created_at_ms;
      return bv - av;
    });
    return rows;
  });

  const activeThread = $derived.by<ThreadDto | null>(() => {
    const id = threadsCatalog.activeThreadId;
    return id ? (threadsCatalog.threads.get(id) ?? null) : null;
  });

  const activeMessages = $derived.by<MessageDto[]>(() => {
    const id = threadsCatalog.activeThreadId;
    return id ? (threadsCatalog.messagesByThread.get(id) ?? []) : [];
  });

  async function pickThread(id: string): Promise<void> {
    threadsCatalog.activeThreadId = id;
    if (!threadsCatalog.messagesByThread.has(id)) {
      try {
        await threadsCatalog.loadMessages(id);
      } catch (e) {
        console.warn("[ThreadDrawer] loadMessages failed", e);
      }
    }
  }

  async function submitNewThread(): Promise<void> {
    const title = newThreadTitle.trim();
    if (!title || newThreadBusy) return;
    newThreadBusy = true;
    try {
      const created = await threadsCatalog.createThread({
        anchor_kind: "app_global",
        anchor_id: null,
        title,
      });
      newThreadTitle = "";
      showNewThreadForm = false;
      await pickThread(created.id);
    } catch (e) {
      console.warn("[ThreadDrawer] createThread failed", e);
    } finally {
      newThreadBusy = false;
    }
  }

  async function submitMessage(): Promise<void> {
    const body = composerBody.trim();
    const id = threadsCatalog.activeThreadId;
    if (!body || !id || composerBusy) return;
    composerBusy = true;
    composerError = null;
    try {
      await threadsCatalog.appendMessage({
        thread_id: id,
        author_kind: "human",
        author_name: null,
        author_persona_id: null,
        role: "note",
        body,
        refs: [],
        idempotency_key: null,
      });
      composerBody = "";
    } catch (e) {
      composerError = String(e);
      console.warn("[ThreadDrawer] appendMessage failed", e);
    } finally {
      composerBusy = false;
    }
  }

  function onComposerKey(evt: KeyboardEvent): void {
    // Enter submits, Shift+Enter inserts newline.
    if (evt.key === "Enter" && !evt.shiftKey && !evt.isComposing) {
      evt.preventDefault();
      void submitMessage();
    }
  }

  async function archiveActive(): Promise<void> {
    const id = threadsCatalog.activeThreadId;
    if (!id) return;
    const t = threadsCatalog.threads.get(id);
    try {
      await threadsCatalog.archiveThread({
        thread_id: id,
        archived: !(t?.archived ?? false),
      });
    } catch (e) {
      console.warn("[ThreadDrawer] archiveThread failed", e);
    }
  }

  async function deleteActive(): Promise<void> {
    const id = threadsCatalog.activeThreadId;
    if (!id) return;
    // Hard delete cascades to messages, so it is confirmed — through
    // the app's own modal rather than `window.confirm`, which returns
    // a silent `false` inside Tauri v2's macOS WKWebView. The old
    // inline call kept the drawer self-contained at the cost of the
    // delete never firing at all; `confirmCatalog` is store-driven, so
    // the drawer still owns no modal markup.
    const ok = await confirmCatalog.open({
      title: "Delete this thread?",
      body: "Every message in it goes with it. This cannot be undone.",
      confirmLabel: "Delete Forever",
      danger: true,
    });
    if (!ok) return;
    try {
      await threadsCatalog.deleteThread({ thread_id: id });
    } catch (e) {
      console.warn("[ThreadDrawer] deleteThread failed", e);
    }
  }

  function fmtTime(ms: number): string {
    const d = new Date(ms);
    if (!Number.isFinite(d.getTime())) return "";
    return d.toISOString().slice(5, 16).replace("T", " ");
  }

  // Escape handling moved to App's interaction mode stack (W5, Esc
  // SoT) — the drawer registers as "drawer" on open and App's single
  // Escape switch closes the top-most layer. A component-local
  // window listener here would double-handle the same keypress (the
  // second handler would see an already-popped stack and clear the
  // grid selection instead).

  // Backdrop click-close is accessible via keyboard when the backdrop
  // (role="button") receives Enter / Space. The main Escape path is
  // App's interaction-stack switch (the drawer registers as
  // "drawer"); this covers the case where focus lands on the
  // backdrop itself.
  function onBackdropKeydown(evt: KeyboardEvent): void {
    if (evt.key === "Enter" || evt.key === " ") {
      evt.preventDefault();
      onClose();
    }
  }

  // Only close when the click hits the backdrop itself, not an
  // inner element bubbling up. Replaces the old
  // `onclick={(e) => e.stopPropagation()}` inside the drawer body —
  // that sibling pattern trips the svelte-a11y lint since the drawer
  // container has no independent interactive semantics.
  function onBackdropClick(evt: MouseEvent): void {
    if (evt.target === evt.currentTarget) onClose();
  }

  function authorLabel(m: MessageDto): string {
    switch (m.author_kind) {
      case "human":
        return "you";
      case "claude_code":
        return "Claude Code";
      case "agent":
        return m.author_name ?? "agent";
      case "persona":
        return "persona";
      default:
        return m.author_kind;
    }
  }
</script>


{#if open}
  <div
    class="drawer-backdrop"
    onclick={onBackdropClick}
    onkeydown={onBackdropKeydown}
    role="button"
    tabindex="-1"
    aria-label="Close threads drawer"
  >
    <div
      class="drawer"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-label="Threads"
    >
      <header class="drawer-head">
        <h3>Threads</h3>
        <button class="drawer-close" onclick={onClose} aria-label="Close">✕</button>
      </header>

      <div class="drawer-body">
        <!-- Left mini-panel: thread list. -->
        <nav class="thread-list" aria-label="Thread list">
          <button
            type="button"
            class="thread-new-btn"
            onclick={() => (showNewThreadForm = !showNewThreadForm)}
          >
            + New thread
          </button>
          {#if showNewThreadForm}
            <form
              class="thread-new-form"
              onsubmit={(e) => {
                e.preventDefault();
                void submitNewThread();
              }}
            >
              <input
                type="text"
                placeholder="thread title"
                bind:value={newThreadTitle}
                disabled={newThreadBusy}
                aria-label="New thread title"
              />
              <button type="submit" disabled={newThreadBusy || !newThreadTitle.trim()}
                >Create</button
              >
            </form>
          {/if}

          {#if threadsCatalog.loading && threads.length === 0}
            <p class="muted">Loading threads…</p>
          {:else if threadsCatalog.error}
            <p class="error">{threadsCatalog.error}</p>
          {:else if threads.length === 0}
            <p class="muted">No threads yet — create one to begin.</p>
          {:else}
            <ul>
              {#each threads as t (t.id)}
                <li>
                  <button
                    type="button"
                    class="thread-row"
                    class:active={threadsCatalog.activeThreadId === t.id}
                    onclick={() => void pickThread(t.id)}
                  >
                    <span class="thread-title">{t.title}</span>
                    <span class="thread-meta">
                      {t.message_count} · {fmtTime(t.last_message_at_ms ?? t.updated_at_ms)}
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </nav>

        <!-- Right pane: messages + composer. -->
        <section class="thread-main" aria-label="Thread contents">
          {#if !activeThread}
            <p class="muted centered">Select a thread on the left.</p>
          {:else}
            <div class="thread-toolbar">
              <span class="thread-heading">{activeThread.title}</span>
              <button
                type="button"
                class="tool-btn"
                onclick={() => void archiveActive()}
                title={activeThread.archived ? "Unarchive" : "Archive"}
              >
                {activeThread.archived ? "Unarchive" : "Archive"}
              </button>
              <button
                type="button"
                class="tool-btn danger"
                onclick={() => void deleteActive()}
                title="Delete thread"
              >
                Delete
              </button>
            </div>

            <div class="message-list" role="log" aria-live="polite">
              {#if activeMessages.length === 0}
                <p class="muted centered">No messages yet.</p>
              {:else}
                {#each activeMessages as m (m.id)}
                  <article class="message message-{m.author_kind}">
                    <header class="message-head">
                      <span class="author">{authorLabel(m)}</span>
                      <span class="stamp">{fmtTime(m.created_at_ms)}</span>
                    </header>
                    <p class="message-body">{m.body}</p>
                  </article>
                {/each}
              {/if}
            </div>

            <footer class="composer">
              {#if composerError}
                <p class="error">{composerError}</p>
              {/if}
              <textarea
                class="composer-input"
                placeholder="Write a note. Enter to send, Shift+Enter for newline."
                bind:value={composerBody}
                onkeydown={onComposerKey}
                disabled={composerBusy}
                rows="3"
              ></textarea>
              <div class="composer-actions">
                <button
                  type="button"
                  onclick={() => void submitMessage()}
                  disabled={composerBusy || !composerBody.trim()}
                >
                  Send
                </button>
              </div>
            </footer>
          {/if}
        </section>
      </div>
    </div>
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: var(--wash-down);
    z-index: 40;
    display: flex;
    justify-content: flex-end;
  }
  .drawer {
    width: min(760px, 90vw);
    height: 100vh;
    background: var(--asterism-surface, var(--surface-stage));
    color: var(--asterism-text, var(--ink-secondary));
    display: flex;
    flex-direction: column;
    box-shadow: -4px 0 24px var(--shadow-color-strong);
  }
  .drawer-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--line);
  }
  .drawer-head h3 {
    margin: 0;
    font-size: 0.95rem;
    letter-spacing: 0.02em;
  }
  .drawer-close {
    background: transparent;
    color: inherit;
    border: none;
    font-size: 1rem;
    cursor: pointer;
    opacity: 0.7;
  }
  .drawer-close:hover {
    opacity: 1;
  }
  .drawer-body {
    display: grid;
    grid-template-columns: 220px 1fr;
    flex: 1;
    min-height: 0;
  }
  .thread-list {
    border-right: 1px solid var(--line);
    padding: 0.5rem;
    overflow-y: auto;
  }
  .thread-new-btn {
    width: 100%;
    padding: 0.4rem;
    background: transparent;
    color: inherit;
    border: 1px dashed var(--line);
    border-radius: 4px;
    cursor: pointer;
    margin-bottom: 0.5rem;
    font-size: 0.85rem;
  }
  .thread-new-form {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin-bottom: 0.5rem;
  }
  .thread-new-form input {
    padding: 0.3rem;
    background: var(--wash-up);
    color: inherit;
    border: 1px solid var(--line);
    border-radius: 3px;
    font-size: 0.85rem;
  }
  .thread-new-form button {
    padding: 0.3rem;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .thread-list ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .thread-row {
    display: flex;
    flex-direction: column;
    width: 100%;
    text-align: left;
    padding: 0.4rem 0.5rem;
    background: transparent;
    color: inherit;
    border: none;
    border-radius: 3px;
    cursor: pointer;
    margin-bottom: 0.15rem;
  }
  .thread-row:hover {
    background: var(--wash-up);
  }
  .thread-row.active {
    background: var(--wash-up-strong);
  }
  .thread-title {
    font-size: 0.9rem;
    font-weight: 500;
  }
  .thread-meta {
    font-size: 0.72rem;
    opacity: 0.6;
    margin-top: 0.1rem;
  }
  .thread-main {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 0.5rem 0.75rem;
  }
  .thread-toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--line);
  }
  .thread-heading {
    flex: 1;
    font-size: 0.95rem;
    font-weight: 500;
  }
  .tool-btn {
    background: transparent;
    color: inherit;
    border: 1px solid var(--line);
    padding: 0.25rem 0.6rem;
    font-size: 0.75rem;
    border-radius: 3px;
    cursor: pointer;
  }
  .tool-btn.danger {
    color: var(--danger-ink);
    border-color: var(--danger-line);
  }
  .message-list {
    flex: 1;
    overflow-y: auto;
    padding: 0.5rem 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .message {
    padding: 0.5rem 0.6rem;
    border-radius: 4px;
    background: var(--wash-up);
    border-left: 3px solid var(--line);
  }
  .message-claude_code {
    border-left-color: var(--info-fill);
  }
  .message-agent {
    border-left-color: var(--warning-fill);
  }
  .message-persona {
    border-left-color: var(--cat-orchid);
  }
  .message-system {
    opacity: 0.7;
    font-style: italic;
  }
  .message-head {
    display: flex;
    justify-content: space-between;
    font-size: 0.72rem;
    opacity: 0.7;
    margin-bottom: 0.25rem;
  }
  .message-body {
    white-space: pre-wrap;
    margin: 0;
    font-size: 0.9rem;
    line-height: 1.4;
  }
  .composer {
    border-top: 1px solid var(--line);
    padding-top: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .composer-input {
    width: 100%;
    resize: vertical;
    background: var(--wash-up);
    color: inherit;
    border: 1px solid var(--line);
    border-radius: 3px;
    padding: 0.4rem;
    font-family: inherit;
    font-size: 0.85rem;
  }
  .composer-actions {
    display: flex;
    justify-content: flex-end;
  }
  .composer-actions button {
    padding: 0.3rem 0.9rem;
    background: var(--info-surface);
    color: inherit;
    border: 1px solid var(--info-line);
    border-radius: 3px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .composer-actions button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .muted {
    opacity: 0.55;
    font-size: 0.85rem;
  }
  .centered {
    text-align: center;
    margin: 1rem 0;
  }
  .error {
    color: var(--danger-ink);
    font-size: 0.8rem;
    margin: 0.25rem 0;
  }
</style>
