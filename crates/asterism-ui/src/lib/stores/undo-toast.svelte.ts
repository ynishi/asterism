// Undo-toast catalog — one transient message with one action on it.
//
// Exists for the actions that are reversible but not *visibly* so. The
// status line already says "moved to trash", and the trash view is the
// long way back; what neither offers is the way back at the moment the
// user notices, which is the second after the gesture. That second is
// what this store owns.
//
// Sibling of `dispatchCatalog.flash`, deliberately not folded into it:
// a flash is a message, this is a message plus a commitment to run
// something later, and only one of the two can be dropped on the floor
// when a newer one arrives.
//
// Scope:
//   - `toast: { message, actionLabel, run } | null` — what is on
//     screen. `null` = nothing.
//   - `show({ message, actionLabel, onAction, ms })` — replaces
//     whatever is showing and arms the auto-dismiss.
//   - `act()` — runs the action and takes the toast down first, so a
//     double click cannot run it twice.
//   - `dismiss()` — takes it down without running anything.
//
// Replacing an in-flight toast drops the older action **unrun**, and
// that is the intended reading: the offer was "undo the thing you just
// did", and once a newer thing has been done the older offer no longer
// describes the state the user is looking at. The timer is cleared on
// every transition so an outgoing toast cannot dismiss its successor.
//
// Deliberately NOT owned here:
//   - What "undo" means. The caller passes a closure over the ids it
//     just acted on; this store never talks to the backend.
//   - The interaction-mode stack. A toast is not modal — it steals no
//     focus, blocks nothing, and Escape belongs to whatever overlay is
//     actually on screen.

interface UndoToast {
  message: string;
  actionLabel: string;
  /** What the action button runs. Awaited by `act()` so a caller can
   *  report failure, but errors are the caller's to handle. */
  run: () => void | Promise<void>;
}

interface UndoToastOptions {
  message: string;
  onAction: () => void | Promise<void>;
  /** Defaults to `"Undo"` — the only label used so far, kept as an
   *  option so a second kind of offer does not need a second store. */
  actionLabel?: string;
  /** How long the offer stands. */
  ms?: number;
}

/**
 * How long an Undo stays on offer.
 *
 * Longer than a status flash (5 s) because this one asks for a
 * decision rather than reporting a fact: the user has to notice the
 * mistake, find the button and reach it. It is also what the driven
 * suite has to work inside — `card-trash.spec.ts` needs ~2-3 s to
 * observe the toast and click it, and a window sized to the gesture
 * alone would make that assertion a race against the timer.
 */
const DEFAULT_MS = 8000;

class UndoToastCatalog {
  toast = $state<UndoToast | null>(null);

  #timer: ReturnType<typeof setTimeout> | undefined;

  show(options: UndoToastOptions): void {
    this.#clearTimer();
    this.toast = {
      message: options.message,
      actionLabel: options.actionLabel ?? "Undo",
      run: options.onAction,
    };
    const mine = this.toast;
    this.#timer = setTimeout(() => {
      // Identity, not equality: two trashes in a row produce two
      // objects with the same text, and only this one's timer may
      // take this one down.
      if (this.toast === mine) this.toast = null;
    }, options.ms ?? DEFAULT_MS);
  }

  async act(): Promise<void> {
    const current = this.toast;
    if (current === null) return;
    // Down first, then run: the action is a backend round trip, and a
    // toast that stays up while it runs invites the second click that
    // would restore everything twice.
    this.#clearTimer();
    this.toast = null;
    await current.run();
  }

  dismiss(): void {
    this.#clearTimer();
    this.toast = null;
  }

  #clearTimer(): void {
    if (this.#timer !== undefined) {
      clearTimeout(this.#timer);
      this.#timer = undefined;
    }
  }
}

export const undoToastCatalog = new UndoToastCatalog();
