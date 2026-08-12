// Toast catalog — what the bottom of the window says about the
// operation the user just asked for. Two slots, because there are two
// things worth saying and they do not share a lifetime:
//
//   - `toast`: a transient message with one action on it (Undo).
//   - `refusal`: the operation did not happen, and why. Sticky.
//
// Exists for the actions that are reversible but not *visibly* so. The
// status line already says "moved to trash", and the trash view is the
// long way back; what neither offers is the way back at the moment the
// user notices, which is the second after the gesture. That second is
// what this store owns.
//
// The refusal slot exists for the opposite case, and for the same
// reason the read path has `Resource.error`: a write that the backend
// refused used to reach the browser console and nothing else, so the
// interface carried on as though the operation had happened. It is a
// separate slot rather than a second `toast` because the two answer
// different questions — an Undo offer expires (the state it describes
// has moved on), while a refusal has no deadline and is worth reading
// late. It goes when dismissed or when a newer refusal replaces it, and
// deliberately not when a later write succeeds: the bulk loops continue
// past a failure, so clearing on success would erase the reason for the
// refused item and leave the rest of the loop reporting progress
// against nothing. `lib/mutate.ts` carries that reasoning.
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
//   - Which failures count as refusals. `lib/mutate.ts` decides that by
//     being the wrapper a call site opted into; this store shows what it
//     is handed.

interface Refusal {
  /** What the user asked for, in their terms: "Could not trash …". */
  message: string;
  /** The backend's own words, or null when it gave none. Kept separate
   *  so the sentence above stays readable when this one is not. */
  detail: string | null;
}

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
  refusal = $state<Refusal | null>(null);

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

  /**
   * Show a refused operation. No timer: unlike an Undo offer, this one
   * does not expire, and a user who looked away is exactly the person
   * it is for.
   */
  refuse(message: string, detail: string | null = null): void {
    this.refusal = { message, detail };
  }

  dismissRefusal(): void {
    this.refusal = null;
  }

  #clearTimer(): void {
    if (this.#timer !== undefined) {
      clearTimeout(this.#timer);
      this.#timer = undefined;
    }
  }
}

export const undoToastCatalog = new UndoToastCatalog();
