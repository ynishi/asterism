// Interaction mode stack — the W1 slice of the interaction model.
// Overlay-ish UI states register themselves here so cross-cutting
// guards ("is any blocking overlay open?", "should aim-hover be suppressed?") read
// one source instead of OR-ing scattered boolean flags. Later waves
// migrate the Escape chain (pop the top mode) and the command
// registry on top of this store; until then the hover suppression
// guard is the only consumer.
//
// Modes form a flat set with stack order retained (push order ≈
// z-order). `push` is idempotent per mode — re-pushing an already
// registered mode moves it to the top instead of duplicating it.

export type OverlayMode =
  | "marquee"
  | "cardMenu"
  | "detail"
  | "fullscreen"
  | "reader"
  | "settings"
  | "drawer"
  | "preview"
  | "prompt"
  | "confirm"
  | "queryMenu"
  | "pinnedBurst";

class InteractionModeStore {
  private stack = $state<OverlayMode[]>([]);

  get top(): OverlayMode | null {
    return this.stack.length > 0 ? this.stack[this.stack.length - 1] : null;
  }

  get overlayActive(): boolean {
    return this.stack.length > 0;
  }

  has(mode: OverlayMode): boolean {
    return this.stack.includes(mode);
  }

  push(mode: OverlayMode) {
    // Already on top = a real no-op, not a fresh array with the same
    // content. The distinction is load-bearing under $state: an
    // identity-changing write retriggers every effect that read the
    // stack, and an effect that mirrors a store onto this stack
    // (App's confirm/prompt wiring) then re-runs its own write
    // forever — effect_update_depth_exceeded, scheduler dead, no
    // DOM update anywhere in the app after the modal opens
    // (2026-08-01, caught by the Empty Trash e2e).
    if (this.stack[this.stack.length - 1] === mode) return;
    const next = this.stack.filter((m) => m !== mode);
    next.push(mode);
    this.stack = next;
  }

  remove(mode: OverlayMode) {
    if (!this.stack.includes(mode)) return;
    this.stack = this.stack.filter((m) => m !== mode);
  }

  pop(): OverlayMode | null {
    const top = this.top;
    if (top !== null) this.stack = this.stack.slice(0, -1);
    return top;
  }
}

export const interaction = new InteractionModeStore();
