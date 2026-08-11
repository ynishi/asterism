// Threads catalog — the App-level Threads primitive. Unified surface for
// both UI-authored notes (`author_kind = "human"`) and HTTP-authored
// writes (`author_kind = "claude_code"` / `"agent"` / `"persona"`,
// arriving through /asterism/threads/{id}/messages on the loopback
// server). They land on the same rows; the drawer renders them
// interleaved so "Context → Action" stays visually contiguous.
//
// Scope of this file (P1):
//   - list Threads under a fixed anchor (Home tab uses
//     AppGlobal). `activeAnchor` is a reactive input; App orchestrates
//     `load(anchor)` from a `$effect`.
//   - append / delete Messages with optimistic reflection.
//   - archive / delete Threads.
//   - polling for HTTP-authored writes (`refreshMessages(activeId)`).
//     App drives the cadence (3 s default) from a `$effect` — the
//     store stays lifecycle-agnostic, mirroring dispatch.svelte.ts.
//     The SSE upgrade (P2) will replace the poll but
//     keeps the same catalog contract.
//
// Deliberately NOT owned here:
//   - `activeAnchor` reload orchestration (App-side `$effect`).
//   - SSE subscription (P2; will replace the poll without changing
//     `messages` semantics).
//   - Composer / drawer open state (component-local).
//
// SvelteMap is used for `threads` and `messagesByThread` because both
// are partially mutated (a single archive-toggle or a fresh message
// append should not rebuild the whole collection). Derived reads that
// need a whole-collection view build a plain Map inside `$derived.by`
// on the consumer side.

import { SvelteMap } from "svelte/reactivity";

import type {
  AppendMessageCommand,
  ArchiveThreadCommand,
  CreateThreadCommand,
  DeleteMessageCommand,
  DeleteThreadCommand,
  MessageDto,
  ThreadDto,
} from "../../bindings";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

/// Anchor argument shape for `list_threads`. `kind` mirrors
/// `ThreadAnchorDto.kind`; `id` is required for every kind except
/// `"app_global"`. Callers pass `null` for AppGlobal so the
/// discriminant is obvious at the call site.
export interface ThreadAnchorArgs {
  kind: string;
  id: string | null;
  includeArchived?: boolean;
}

/// Wire arg shape of `list_threads`. Kept internal — the Resource
/// wraps it so consumers only see `catalog.load(anchor)`.
interface ListThreadsArgs {
  anchor_kind: string;
  anchor_id: string | null;
  include_archived: boolean;
}

class ThreadsCatalog {
  /// Threads under the current anchor, keyed by id. Rebuilt whole on
  /// `load()`; individual entries are patched by mutations so the UI
  /// does not have to reload after an archive-toggle.
  threads = new SvelteMap<string, ThreadDto>();

  /// Message rows keyed by thread id. Populated lazily on
  /// `loadMessages(threadId)` and grown by `appendMessage` /
  /// `refreshMessages`.
  messagesByThread = new SvelteMap<string, MessageDto[]>();

  /// Thread id the drawer is focused on (`null` = drawer showing the
  /// thread list). App / component set / clear this.
  activeThreadId = $state<string | null>(null);

  /// Anchor used by the current `load()` — surfaced so `refresh()`
  /// can re-run without the caller re-plumbing arguments.
  #currentAnchor: ThreadAnchorArgs | null = null;

  /// Underlying Resource for the Thread list. Data is returned as an
  /// array; `threads` (SvelteMap) is rebuilt from it on every accepted
  /// write so downstream reactives see one atomic swap.
  #listResource = new Resource<ListThreadsArgs, ThreadDto[]>(
    (args) =>
      api<ThreadDto[]>("list_threads", {
        anchorKind: args.anchor_kind,
        anchorId: args.anchor_id,
        includeArchived: args.include_archived,
      }),
    [] as ThreadDto[],
    "threadsCatalog.list",
  );

  get loading(): boolean {
    return this.#listResource.loading;
  }

  get error(): string | null {
    return this.#listResource.error;
  }

  /// Refetches the Thread list for the given anchor. App calls this
  /// from a `$effect` keyed on `activeAnchor`.
  async load(anchor: ThreadAnchorArgs): Promise<void> {
    this.#currentAnchor = anchor;
    const written = await this.#listResource.load({
      anchor_kind: anchor.kind,
      anchor_id: anchor.id,
      include_archived: anchor.includeArchived ?? false,
    });
    if (!written) return;
    // Rebuild the SvelteMap from the fresh array. Clearing before
    // inserting means archived / deleted rows disappear cleanly.
    this.threads.clear();
    for (const row of this.#listResource.data) {
      this.threads.set(row.id, row);
    }
    // Drop cached message rows for threads that are gone.
    for (const id of Array.from(this.messagesByThread.keys())) {
      if (!this.threads.has(id)) this.messagesByThread.delete(id);
    }
    // If the active thread disappeared, unfocus.
    if (this.activeThreadId && !this.threads.has(this.activeThreadId)) {
      this.activeThreadId = null;
    }
  }

  /// Re-runs `load()` for the last-used anchor (helper for post-mutation
  /// refreshes). No-op before the first `load()`.
  async refresh(): Promise<void> {
    if (this.#currentAnchor) await this.load(this.#currentAnchor);
  }

  reset(): void {
    this.#listResource.reset();
    this.threads.clear();
    this.messagesByThread.clear();
    this.activeThreadId = null;
    this.#currentAnchor = null;
  }

  /// Fetches Messages for one Thread and replaces the cached list.
  async loadMessages(threadId: string): Promise<void> {
    const rows = await api<MessageDto[]>("list_thread_messages", {
      threadId,
      sinceMs: null,
    });
    this.messagesByThread.set(threadId, rows);
  }

  /// Polls for Messages added since the newest row currently cached
  /// (the P2 SSE upgrade replaces this — the surface stays the same).
  /// No-op when the thread has no cached messages yet; call
  /// `loadMessages` first.
  async refreshMessages(threadId: string): Promise<void> {
    const existing = this.messagesByThread.get(threadId);
    if (!existing) return;
    const lastMs = existing.length
      ? existing[existing.length - 1].created_at_ms
      : null;
    const fresh = await api<MessageDto[]>("list_thread_messages", {
      threadId,
      sinceMs: lastMs,
    });
    if (fresh.length === 0) return;
    // Fresh rows always come after `lastMs`; simple concat preserves
    // chronological order.
    this.messagesByThread.set(threadId, existing.concat(fresh));
    const thread = this.threads.get(threadId);
    if (thread) {
      thread.last_message_at_ms = fresh[fresh.length - 1].created_at_ms;
      thread.message_count = existing.length + fresh.length;
    }
  }

  /// Creates a Thread. Returns the created row; the caller usually
  /// sets `activeThreadId` on the response.
  async createThread(command: CreateThreadCommand): Promise<ThreadDto> {
    const created = await api<ThreadDto>("create_thread", { command });
    this.threads.set(created.id, created);
    return created;
  }

  /// Appends a Message to a Thread. Author identity (`author_kind`,
  /// `author_name`, `author_persona_id`) is caller-supplied — this
  /// catalog does not default it, since HTTP callers pass different
  /// values through the same service.
  async appendMessage(command: AppendMessageCommand): Promise<MessageDto> {
    const stored = await api<MessageDto>("append_thread_message", {
      command,
    });
    const existing = this.messagesByThread.get(command.thread_id) ?? [];
    // Guard against idempotent-echo: the server may return an
    // already-appended row (same id — append is idempotent). Both
    // the message-cache insert AND the projection-column bump must
    // sit inside this branch, otherwise a retry inflates
    // `message_count` on the sidebar despite the trigger-maintained
    // server-side count staying correct.
    if (!existing.some((m) => m.id === stored.id)) {
      this.messagesByThread.set(command.thread_id, existing.concat(stored));
      const thread = this.threads.get(command.thread_id);
      if (thread) {
        thread.last_message_at_ms = stored.created_at_ms;
        thread.updated_at_ms = stored.created_at_ms;
        thread.message_count = (thread.message_count ?? 0) + 1;
      }
    }
    return stored;
  }

  /// Deletes a Message (misfire correction). Idempotent.
  async deleteMessage(command: DeleteMessageCommand, threadId: string): Promise<void> {
    await api<void>("delete_thread_message", { command });
    const existing = this.messagesByThread.get(threadId);
    if (existing) {
      this.messagesByThread.set(
        threadId,
        existing.filter((m) => m.id !== command.message_id),
      );
      const thread = this.threads.get(threadId);
      if (thread && thread.message_count > 0) {
        thread.message_count -= 1;
      }
    }
  }

  /// Toggles the archived flag. The row stays in the map so a
  /// subsequent `include_archived` list still finds it; the flag
  /// itself drives the default-list filter server-side.
  async archiveThread(command: ArchiveThreadCommand): Promise<void> {
    const updated = await api<ThreadDto>("archive_thread", { command });
    this.threads.set(updated.id, updated);
    // Default listing excludes archived rows; drop it from the visible
    // map when the archive toggle just fired unless the caller opted
    // into `include_archived`.
    if (
      updated.archived
      && !(this.#currentAnchor?.includeArchived ?? false)
    ) {
      this.threads.delete(updated.id);
      if (this.activeThreadId === updated.id) this.activeThreadId = null;
    }
  }

  /// Deletes a Thread (cascades to messages). Idempotent.
  async deleteThread(command: DeleteThreadCommand): Promise<void> {
    await api<void>("delete_thread", { command });
    this.threads.delete(command.thread_id);
    this.messagesByThread.delete(command.thread_id);
    if (this.activeThreadId === command.thread_id) {
      this.activeThreadId = null;
    }
  }
}

/// Singleton — components import the instance directly (0-prop
/// default).
export const threadsCatalog = new ThreadsCatalog();
