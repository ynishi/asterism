// Card dragging, built on Pointer Events instead of HTML5 DnD.
//
// ## Why not HTML5 drag-and-drop
//
// Tauri's window-level file-drop handler intercepts drags before the
// webview sees them, so `dragstart` / `dragover` / `drop` never fire in
// the page. The `dragDropEnabled` flag that controls it is either/or —
// its real meaning is "Tauri's drag-drop is on and the DOM's is off",
// and the file-drop handler covers the whole window in a way that
// cannot propagate into the webview [tauri#14373]. Turning it off would
// bring DOM events back and take OS file drops away.
//
// This project needs both: dropping a screenshot in from Finder, and
// dragging a card onto a sidebar row. Pointer Events sit below that
// mechanism entirely, so neither has to lose.
//
// (History: `dragDropEnabled: false` was set deliberately when Group
// drop landed (d32f2a8), flipped to `true` two commits later to add
// drop-import (57c5ae3) — which silently killed Group drop — and then
// dropped altogether when the window moved to the Rust builder
// (4f42827). Nothing in the code said the two were exclusive.)
//
// ## Shape
//
// A drag source calls `beginCardDrag` on `pointerdown`. Drop targets
// mark themselves declaratively with `data-drop-kind` / `data-drop-id`
// and never register a handler: pointer capture routes every move to
// the source element, so a target cannot hear the pointer itself — the
// position is resolved against the DOM with `elementFromPoint` instead.
// Adding a new kind of drop target is therefore two attributes and a
// branch in the caller's `onDrop`.

/** What sits under the pointer: a `data-drop-kind` / `-id` pair. */
export type DropTarget = { kind: string; id: string };

/** What is being carried. Same shape as a target — a group row is both. */
export type DragSource = { kind: string; id: string };

// Movement (px) before a press counts as a drag rather than a click.
// Without it every click would start — and immediately end — a drag,
// and the card's own click handler would fight the drop.
const DRAG_THRESHOLD_PX = 4;

class CardDrag {
  /** What is being carried, or `null` when nothing is in flight. */
  source = $state<DragSource | null>(null);
  /** Live pointer position, for the ghost that follows the cursor. */
  x = $state(0);
  y = $state(0);
  /** Drop target currently under the pointer. */
  over = $state<DropTarget | null>(null);
  /**
   * Set when a drag actually happened, and cleared by the next click.
   * `pointerup` is followed by a `click` on the same element, which
   * would otherwise open the card the user just dropped somewhere.
   * Same one-shot swallow the marquee sweep uses.
   */
  justDropped = $state(false);

  get active(): boolean {
    return this.source !== null;
  }

  /** Id of the dragged item when it is of `kind`, else `null`. */
  sourceOf(kind: string): string | null {
    return this.source?.kind === kind ? this.source.id : null;
  }

  /**
   * Is this specific target the one under the pointer? Never true for
   * the item being dragged — a row is not a destination for itself.
   */
  isOver(kind: string, id: string): boolean {
    if (this.source?.kind === kind && this.source.id === id) return false;
    return this.over?.kind === kind && this.over.id === id;
  }

  reset(): void {
    this.source = null;
    this.over = null;
  }
}

export const cardDrag = new CardDrag();

/**
 * Resolves what is under the pointer.
 *
 * Pointer capture sends every `pointermove` to the element the drag
 * started on, so drop targets receive nothing and cannot report
 * themselves. Asking the document what is at the coordinates is the
 * standard answer, and it keeps targets declarative — they only need
 * the two data attributes.
 */
function targetAt(x: number, y: number): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  const hit = el?.closest("[data-drop-kind]") as HTMLElement | null;
  const kind = hit?.dataset.dropKind;
  const id = hit?.dataset.dropId;
  return kind !== undefined && id !== undefined ? { kind, id } : null;
}

/**
 * Starts a drag from a `pointerdown`.
 *
 * `onDrop` fires once, on release over a target, and only if the
 * pointer travelled far enough to be a drag. A press that never moves
 * stays a click and the caller's click handler runs as usual.
 *
 * The source is handed back rather than read off the store: the store
 * is cleared before `onDrop` runs, so that a handler awaiting a
 * round-trip cannot leave the ghost on screen.
 */
export function beginDrag(
  event: PointerEvent,
  source: DragSource,
  onDrop: (target: DropTarget, source: DragSource) => void,
): void {
  // Left button or touch only — a right-click opens the context menu,
  // and a middle-click should not haul the card around.
  if (event.button !== 0) return;
  const origin = event.currentTarget as HTMLElement | null;
  if (origin === null) return;

  const pointerId = event.pointerId;
  const startX = event.clientX;
  const startY = event.clientY;
  let started = false;

  // Capture on `currentTarget`, not `target`: the pointer went down on
  // some child (a cover line, a thumbnail), and capture follows the
  // element it was set on — a child that re-renders mid-drag would
  // drop the capture with it.
  origin.setPointerCapture(pointerId);

  // To the browser a pointer drag over text *is* a selection gesture,
  // and it starts the moment the button goes down — before the 4px
  // threshold has decided this is a drag at all. Clearing the selection
  // afterwards does not help: the gesture is still live and the browser
  // rebuilds it on the next move. Refusing `selectstart` for the
  // duration is what stops it, and unlike `preventDefault()` on
  // `pointerdown` it leaves the compatibility mouse events — and so the
  // card's own click — intact.
  const blockSelection = (e: Event) => e.preventDefault();
  document.addEventListener("selectstart", blockSelection);

  const detach = () => {
    document.removeEventListener("selectstart", blockSelection);
    origin.removeEventListener("pointermove", onMove);
    origin.removeEventListener("pointerup", onUp);
    origin.removeEventListener("pointercancel", onCancel);
    origin.removeEventListener("lostpointercapture", onCancel);
  };

  const onMove = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    if (!started) {
      const travelled = Math.hypot(e.clientX - startX, e.clientY - startY);
      if (travelled < DRAG_THRESHOLD_PX) return;
      started = true;
      cardDrag.source = source;
    }
    cardDrag.x = e.clientX;
    cardDrag.y = e.clientY;
    cardDrag.over = targetAt(e.clientX, e.clientY);
  };

  const onUp = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    detach();
    const target = cardDrag.over;
    const dragged = started;
    cardDrag.reset();
    if (!dragged) return;
    // Swallow the click that follows this release, so dropping a card
    // does not also open it.
    cardDrag.justDropped = true;
    if (target === null) return;
    // A row is not a destination for itself.
    if (target.kind === source.kind && target.id === source.id) return;
    onDrop(target, source);
  };

  // Touch gestures and OS-level interruptions cancel pointers; without
  // this the state would stay "dragging" after the finger left.
  const onCancel = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    detach();
    cardDrag.reset();
  };

  origin.addEventListener("pointermove", onMove);
  origin.addEventListener("pointerup", onUp);
  origin.addEventListener("pointercancel", onCancel);
  // Safety net: capture can end without a pointerup (element removed,
  // another capture taking over), which would otherwise strand us.
  origin.addEventListener("lostpointercapture", onCancel);
}
