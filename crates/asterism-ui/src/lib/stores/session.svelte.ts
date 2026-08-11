// Session catalog — the rolled-up view over per-message assets. Sibling
// of `personaCatalog` / `modalityCatalog` / `tagCatalog` /
// `groupCatalog`: catalog-shape data lives here, selection state
// (`activeFilter.activeSessionId` / `activeSessionLabel`) stays on
// `activeFilter`.
//
// Scope:
//   - `page`: Resource over the backend `list_sessions` invoke —
//     read the current `SessionPageDto` via `page.data` (`null`
//     before the first load). The stale-response guard is the
//     Resource's internal generation counter; the old caller-
//     injected `seq / currentSeq` interface is gone.
//     Items are the new 1st-class `SessionDto` shape after the
//     Session-model migration; the server
//     returns them in `started_at_ms` DESC and the sessions view
//     renders that order as-is — no client-side re-sort exists
//     today (the old `sortedSessions` derived is gone).
//   - `loadPage(query)`: fires the load and resolves `true` iff
//     the store accepted the result (newest request + success).
//     Consumers use that boolean to update sibling cache state
//     (App's `sessionsFetchKey`) atomically with the write.
//   - `clear()`: drop cached page + invalidate in-flight loads.
//     Used by cache-invalidation paths that want the next view
//     flip to force a fetch.
//
// Cross-view note: App's `loadSeq` counter still guards the
// messages-view write (`page` in App); the sessions write is now
// self-guarded here. A sessions response landing after the user
// flipped to messages is harmless — the two views keep separate
// storage, and the accepted data is always consistent with the
// query captured at call time (so `sessionsFetchKey` stays truthful).
//
// Deliberately NOT owned here:
//   - `openSession(s)` / `clearSession()`: side-effect helpers
//     (mutate `activeFilter`, swap `viewMode`, scroll App to top).
//     App-side because scroll behaviour is grid-adjacent, not
//     catalog-internal.
//   - Fetch-key cache (`sessionsFetchKey`): App-side alongside its
//     `messagesFetchKey` sibling because both keys are compared
//     against the same `fetchKey()` composed from `currentFilter()`
//     + `searchText`. Splitting the key would just move the
//     comparison across a module boundary.

import type { SessionDto, SessionPageDto } from "../../bindings";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

class SessionCatalog {
  page = new Resource(
    (query: unknown) => api<SessionPageDto>("list_sessions", { query }),
    null as SessionPageDto | null,
    "sessionCatalog.page",
  );

  async loadPage(query: unknown): Promise<boolean> {
    return await this.page.load(query);
  }

  clear(): void {
    this.page.reset();
  }

  // P2 CRUD. Rename / patchMetadata write
  // through and splice the server-returned DTO back into
  // `page.data.items` in place (Svelte 5 `$state` proxifies the
  // Resource's data, so reassigning `page.data = { ...page, items:
  // [...] }` re-triggers the SessionsView derived rebuild without a
  // full `loadPage` round-trip). Delete drops the item locally.
  //
  // Rationale: `loadPage` needs the App-side filter query as an
  // argument, which this catalog does not track — mutating in place
  // keeps the write path self-contained and avoids introducing a
  // fetch-key-aware reload dance for a 1-tile edit. A follow-up
  // reload (persona flip, search change) will re-hydrate from the
  // server anyway.
  //
  // Errors bubble up so the caller component (SessionsView) can
  // display them; the local page is left untouched on failure.

  #replaceItem(updated: SessionDto): void {
    const page = this.page.data;
    if (!page) return;
    const idx = page.items.findIndex((s) => s.id === updated.id);
    if (idx < 0) return;
    const items = page.items.slice();
    items[idx] = updated;
    this.page.data = { ...page, items };
  }

  /// Renames the Session. `newTitle = null` clears the title back
  /// to untitled (the canonical clear path — patchMetadata cannot
  /// express NULL because per-field null there means "leave
  /// unchanged").
  async rename(id: string, newTitle: string | null): Promise<void> {
    const updated = await api<SessionDto>("rename_session", {
      command: { id, title: newTitle },
    });
    this.#replaceItem(updated);
  }

  /// Partial metadata update. Fields omitted from `patch` are left
  /// unchanged server-side.
  async patchMetadata(
    id: string,
    patch: { title?: string; note?: string; cover_hint?: string },
  ): Promise<void> {
    const updated = await api<SessionDto>("patch_session_metadata", {
      command: { id, ...patch },
    });
    this.#replaceItem(updated);
  }

  /// Delete-if-empty: rejected server-side when any asset still
  /// references the Session. The UI gates the ✕ button behind
  /// `message_count === 0` so this should only surface under a
  /// race; the thrown error propagates.
  async remove(id: string): Promise<void> {
    await api<void>("delete_session", { command: { id } });
    const page = this.page.data;
    if (!page) return;
    const items = page.items.filter((s) => s.id !== id);
    const total = typeof page.total === "number"
      ? Math.max(0, page.total - (page.items.length - items.length))
      : page.total;
    this.page.data = { ...page, items, total };
  }
}

export const sessionCatalog = new SessionCatalog();
