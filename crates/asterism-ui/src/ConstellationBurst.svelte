<script lang="ts">
  // ConstellationBurst — extracted from App.svelte (wave ④).
  // The card-anchored peek panel listing a card's constellation
  // members (related assets + the edge that connects them), opened
  // from the ✦ action icon (aim-hover / click — W1 hover
  // regrammar). Owns the fixed-position panel, the
  // edge-kind / direction presentation helpers, and the "select all
  // in grid" pivot (writes `gridSelection` directly).
  //
  // Not owned: the card-hover lifecycle. App keeps the fetch debounce
  // + the close-grace timers because they are grid-card-adjacent —
  // the panel only reports pointer enter/leave via callbacks so the
  // pointer can travel card → panel without the panel blinking out.
  //
  // Props (App bridge):
  //   - burst — the active burst payload (`null` renders nothing).
  //     Stays App-owned state because the grid template also reads
  //     `burst?.assetId` for the hover highlight.
  //   - onClose — ✕ / item-click / select-all ask App to null it.
  //   - onOpenAsset(id) — item click jumps to that member's detail.
  //   - onPanelEnter / onPanelLeave — cancel / restart App's
  //     close-grace timer.
  //
  // CSS: the `.burst*` cascade moved here wholesale (scoped styles
  // do not reach across component boundaries).
  import type { ConstellationItemDto } from "./bindings";
  import { gridSelection } from "./lib/stores/grid-selection.svelte";

  interface Props {
    // `anchor`: the panel is placed beside the card it was fired
    // from (viewport coords computed by App) via `left` / `top`.
    // Always set — the corner-placement hover path was retired by
    // the W1 hover regrammar.
    burst:
      | {
          assetId: string;
          items: ConstellationItemDto[];
          anchor: { x: number; y: number };
          // W4: pinned bursts survive pointer leave (App skips the
          // close grace); released by Esc / ✦ re-click.
          pinned: boolean;
        }
      | null;
    onClose: () => void;
    onOpenAsset: (id: string) => void;
    onPanelEnter: () => void;
    onPanelLeave: () => void;
  }

  let { burst, onClose, onOpenAsset, onPanelEnter, onPanelLeave }: Props =
    $props();

  // Inline position — card-anchored via left/top on the fixed box.
  let positionStyle = $derived(
    burst ? `left: ${burst.anchor.x}px; top: ${burst.anchor.y}px;` : "",
  );

  // Constellation direction hint from ConstellationItemDto.direction.
  // `both` = the same conceptual link was written from both sides
  // (dedupe collapsed two symmetric edges), so it deserves visual
  // emphasis; `outgoing` / `incoming` say which side of the edge the
  // queried asset sits on.
  function directionArrow(direction: string): string {
    switch (direction) {
      case "both":     return "↔";
      case "incoming": return "←";
      case "outgoing": return "→";
      default:         return "·";
    }
  }

  function directionTitle(direction: string): string {
    switch (direction) {
      case "both":     return "confirmed link from both sides";
      case "incoming": return "linked from the other asset";
      case "outgoing": return "linked to the other asset";
      default:         return direction;
    }
  }

  /**
   * Maps a constellation edge kind slug to a compact symbol +
   * color pair. The burst chip renders the symbol before the kind
   * label so the "why is this here" axis pops without needing to
   * read the label. Kinds not in the table fall through to a
   * neutral bullet — new stored EdgeKinds land safely, we just
   * lose the axis-specific styling until the map catches up.
   */
  function burstKindSymbol(kind: string): string {
    switch (kind) {
      case "time_proximity":  return "⏱";
      case "keyword_overlap": return "🏷";
      case "co_presence":     return "👥";
      case "cadence":         return "🎼";
      case "reference":       return "🔖";
      case "derived_from":    return "🌱";
      case "same_session":    return "🧵";
      case "same_selection":  return "📎";
      case "same_group":      return "📁";
      default:                return "·";
    }
  }
  function burstKindClass(kind: string): string {
    // Coarse color families — six buckets so hover-burst legends
    // stay memorable. Stored auto-edges (time / keyword / co-
    // presence / cadence / reference) share the "auto" palette;
    // provenance is green; the three user-curation axes each
    // get their own hue.
    switch (kind) {
      case "time_proximity":
      case "keyword_overlap":
      case "co_presence":
      case "cadence":
      case "reference":       return "kind-auto";
      case "derived_from":    return "kind-derived";
      case "same_session":    return "kind-session";
      case "same_selection":  return "kind-selection";
      case "same_group":      return "kind-group";
      default:                return "kind-other";
    }
  }

  /**
   * Preview text for a constellation-burst chip: the card's cover
   * (truncated so the burst stays scannable), falling back to the
   * source_locator's basename, then a short id, so text-shaped
   * relatives (dialogue / note / message) do not collapse into an
   * unreadable "..." row.
   */
  function burstItemPreview(item: ConstellationItemDto): string {
    const MAX = 72;
    const cover = item.card.cover?.trim();
    if (cover && cover.length > 0) {
      return cover.length > MAX ? cover.slice(0, MAX) + "…" : cover;
    }
    const basename =
      item.card.source_locator.split("/").filter(Boolean).pop() ?? "";
    if (basename.length > 0) return basename;
    return item.card.id.slice(0, 8);
  }

  /** Full-text tooltip for a burst chip (cover + edge kind + id). */
  function burstItemTooltip(item: ConstellationItemDto): string {
    const parts: string[] = [];
    if (item.card.cover) parts.push(item.card.cover);
    parts.push(`${item.edge.label ?? item.edge.kind} · ${item.card.modality}`);
    parts.push(`id: ${item.card.id.slice(0, 8)}`);
    return parts.join("\n");
  }

  /**
   * Turns the whole active constellation burst — the root card
   * the burst was fired from plus every related member — into
   * the grid multi-select, then closes the burst so downstream
   * actions (promote to Group, batch dispatch, ...) can operate
   * on the full cluster in one gesture.
   */
  function burstSelectAllInGrid() {
    if (!burst) return;
    gridSelection.selectedIds.clear();
    gridSelection.selectedIds.add(burst.assetId);
    for (const item of burst.items) gridSelection.selectedIds.add(item.card.id);
    gridSelection.lastAnchorId = burst.assetId;
    onClose();
  }
</script>

{#if burst && burst.items.length > 0}
  <div
    class="burst"
    style={positionStyle}
    onmouseenter={onPanelEnter}
    onmouseleave={onPanelLeave}
    role="dialog"
  >
    <button class="burst-close" onclick={onClose} aria-label="Close constellation">✕</button>
    <h3>
      ✦ constellation{#if burst.pinned}<span
          class="burst-pin"
          title="Pinned — Esc or ✦ click releases">📌</span>{/if}
      <!-- Phase-1 burst→grid action: turns the whole burst
           (root + every related member) into a fresh grid
           multi-select. Lets the user pivot from "hover peek"
           to "operate on all of these" in one click — the
           entry point for downstream Group promotion, batch
           dispatch, etc. -->
      <button
        type="button"
        class="burst-select-all"
        onclick={() => burstSelectAllInGrid()}
        title="Clear the grid multi-select and pick every card in this burst"
      >
        select all in grid ({burst.items.length + 1})
      </button>
    </h3>
    {#each burst.items as item (item.edge.id)}
      <!-- Click = jump to that member's detail view.
           Content = modality badge + cover snippet or
           filename fallback so the user can tell text-shaped
           relatives apart without opening each one. -->
      <button
        type="button"
        class="burst-item burst-item-{item.direction} {burstKindClass(item.edge.kind)}"
        onclick={() => { onClose(); onOpenAsset(item.card.id); }}
        title={burstItemTooltip(item)}
      >
        <span class="burst-label">
          <span class="burst-arrow" title={directionTitle(item.direction)}>{directionArrow(item.direction)}</span>
          <span class="burst-kind-symbol" title={item.edge.kind}>{burstKindSymbol(item.edge.kind)}</span>
          <span class="burst-kind">{item.edge.label ?? item.edge.kind}</span>
          <span class="burst-modality">{item.card.modality}</span>
        </span>
        <p class="burst-cover">{burstItemPreview(item)}</p>
      </button>
    {/each}
  </div>
{/if}

<style>
  /* Burst cascade — moved wholesale from App.svelte (wave ④,
     scoped CSS does not reach across component boundaries). */

  .burst {
    position: fixed;
    width: 260px;
    background: #fffef9;
    border: 1px solid #d9d5f2;
    border-radius: 10px;
    padding: 0.7rem 0.8rem;
    box-shadow: 0 6px 18px rgba(80, 70, 160, 0.12);
  }

  .burst h3 {
    font-size: 0.75rem;
    color: #7a76c9;
    margin: 0 0 0.4rem;
  }

  .burst-pin {
    margin-left: 0.3rem;
    font-size: 0.7rem;
    opacity: 0.85;
    user-select: none;
  }

  .burst-close {
    position: absolute;
    top: 0.35rem;
    right: 0.4rem;
    width: 20px;
    height: 20px;
    padding: 0;
    line-height: 18px;
    text-align: center;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: #999;
    font-size: 0.7rem;
    cursor: pointer;
  }

  .burst-close:hover {
    background: #f0eefa;
    color: #333;
  }

  .burst-item {
    /* Phase-1: burst chips are now buttons so the whole row is
       click-to-open. Strip the default button chrome and lay
       them out block-style so the label + cover stack matches
       the previous <div> version. */
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    cursor: pointer;
    border: none;
    border-top: 1px solid #eeecf8;
    padding: 0.35rem 0 0.35rem 0.5rem;
    border-left: 2px solid transparent;
    color: inherit;
    font: inherit;
  }
  .burst-item:hover {
    background: #f6f5ff;
  }

  /* Direction hint from ConstellationItemDto.direction:
     both = confirmed on both sides (accent stroke, thicker);
     outgoing / incoming = one-directional edge (tinted pastel). */
  .burst-item-both {
    border-left-color: #7a76c9;
    border-left-width: 3px;
    padding-left: 0.45rem;
  }
  .burst-item-outgoing {
    border-left-color: #d9d5f2;
  }
  .burst-item-incoming {
    border-left-color: #f2d5e8;
  }

  .burst-label {
    font-size: 0.62rem;
    color: #9a96d9;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }

  .burst-arrow {
    font-family: ui-monospace, "SF Mono", "Menlo", monospace;
    opacity: 0.7;
  }

  .burst-kind {
    letter-spacing: 0.02em;
  }

  /* Small pill after the edge kind that tells the user what
     modality the target card is — cheap disambiguation for
     bursts that mix dialogue / image / audio siblings. */
  .burst-modality {
    padding: 0.02rem 0.35rem;
    border-radius: 999px;
    background: #eeecff;
    color: #6a67a4;
    text-transform: uppercase;
    font-size: 0.55rem;
    letter-spacing: 0.03em;
  }

  .burst-cover {
    font-size: 0.75rem;
    margin: 0.15rem 0 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: #333;
  }

  /* Edge-kind axis color. Applied via `kind-<axis>` class on the
     burst-item button so the left-border stripe conveys "why"
     (time / provenance / session / selection / group). The
     direction stripe (both / outgoing / incoming) also lives on
     the border-left — kind color wins because the axis is the
     more informative bit; direction stays as border-left-width
     via .burst-item-both. */
  .burst-item.kind-auto {
    border-left-color: #b8b4de;
  }
  .burst-item.kind-derived {
    border-left-color: #7ec49a;
  }
  .burst-item.kind-session {
    border-left-color: #5eb0c4;
  }
  .burst-item.kind-selection {
    border-left-color: #e0a06a;
  }
  .burst-item.kind-group {
    border-left-color: #b591d5;
  }
  .burst-item.kind-other {
    border-left-color: #cccccc;
  }

  .burst-kind-symbol {
    font-size: 0.85em;
    line-height: 1;
    opacity: 0.9;
  }

  /* "select all in grid" button in the burst drag-handle. */
  .burst-select-all {
    float: right;
    margin-left: 0.5rem;
    padding: 0.1rem 0.55rem;
    background: #eeecff;
    color: #3d38a8;
    border: 1px solid #d3d0f0;
    border-radius: 999px;
    font-size: 0.65rem;
    cursor: pointer;
  }
  .burst-select-all:hover {
    background: #dcd7ff;
    border-color: #b9b2f0;
  }
</style>
