// Grid selection store — the persistent grid multi-select snapshot
// that seeds an outbound dispatch.
// Multi-select is orthogonal to the openDetail gesture — a bare
// card click still opens the detail panel *when nothing is
// selected*, and Ctrl/⌘-click or Shift-click toggles / extends the
// selection instead (see App's `onCardClick`).
//
// Scope:
//   - `selectedIds`: SvelteSet of selected asset ids — the persistent
//     multi-select snapshot. Anything that acts on "what is picked"
//     reads it, which by now is more than the grid: dispatch, the
//     snapshot freeze, and the forge's rounds all start here.
//   - `lastAnchorId`: last card whose id was toggled into the
//     selection — the Shift-extend anchor. `null` means "no anchor
//     yet, next Shift-click is treated as a plain toggle".
//   - `restore(snapshot)`: one-tap rehydrate of a frozen Snapshot
//     ("Re-select" and the detail-pane freeze chips) — flips
//     `activeFilter.activePersona` to the freeze's owner, then
//     repopulates the set + anchor. Lives here because the mutation
//     is grid-selection semantics.
//
// Deliberately NOT owned here:
//   - dispatch / promote composition — App-side: they compose
//     `personaIdOfSelection()` (walks App's filtered rows),
//     `customPrompt`, and the dispatch flow.
//   - click-gesture interpretation (`onCardClick`) — App-side,
//     grid-template-adjacent.

import type { SnapshotDto } from "../../bindings";
import { SvelteSet } from "svelte/reactivity";
import { activeFilter } from "./filter.svelte";

class GridSelection {
  selectedIds = new SvelteSet<string>();
  lastAnchorId = $state<string | null>(null);

  /// Ends the pick. An operation that consumes the selection calls
  /// this, which is the app's convention rather than this store's
  /// invention — `contextPromoteSelection` in `App.svelte` states it
  /// where it promotes.
  ///
  /// Here rather than at each caller because the anchor has to go with
  /// the set: a Shift-extend from a card nothing is selected on reads
  /// as a plain toggle, and a stale anchor makes it read as a range
  /// from wherever the last consumed selection ended.
  clear(): void {
    this.selectedIds.clear();
    this.lastAnchorId = null;
  }

  restore(snapshot: SnapshotDto): void {
    const assetIds = snapshot.asset_ids;
    if (activeFilter.activePersona !== snapshot.persona_id) {
      activeFilter.activePersona = snapshot.persona_id;
    }
    this.selectedIds.clear();
    for (const id of assetIds) this.selectedIds.add(id);
    this.lastAnchorId = assetIds.at(-1) ?? null;
  }
}

export const gridSelection = new GridSelection();
