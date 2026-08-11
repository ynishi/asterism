// Confirm catalog — inline yes/no modal, replaces `window.confirm()`.
//
// Same reason `promptCatalog` exists for `window.prompt()`: on Tauri v2
// macOS WKWebView the native dialog is not reliably shown, and a
// `confirm()` that returns a silent `false` turns a guard into a wall
// (the action never runs and nothing says why). A guard that can fail
// closed on its own is worse than no guard, because the callsite reads
// as protected.
//
// Scope:
//   - `request: { title, body, confirmLabel, danger, resolve } | null`
//     — the currently-open question. `null` = idle. `resolve` is the
//     `Promise` continuation the caller is awaiting; `confirm()` /
//     `cancel()` invoke it exactly once and null the request back out
//     so the modal unmounts.
//   - `open({ title, body, confirmLabel, danger })` — returns a
//     `Promise<boolean>` that resolves `true` only when the user picks
//     the confirm button. Escape / Cancel / backdrop click all resolve
//     `false`, so a caller that forgets a branch fails safe.
//   - `confirm()` / `cancel()` — resolve and clear.
//
// Overwriting an in-flight question is not defended against, matching
// `promptCatalog`: the callsite pattern is
// `if (!(await confirmCatalog.open(...))) return;` inside a handler
// that is not re-entered while it awaits.
//
// Deliberately NOT owned here:
//   - Focus. The component that renders the modal owns the DOM refs and
//     decides what takes focus (the Cancel button — the safe answer is
//     the one a stray Return should give).
//   - The interaction-mode stack entry. App mirrors `request` onto
//     `interaction` with a `$effect`, the same wiring `prompt` uses, so
//     Escape routes here instead of falling through to the
//     selection-clear sink.

interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel: string;
  /** Tones the confirm button as destructive (HIG: the irreversible
   *  choice says so before it is clicked). */
  danger: boolean;
  resolve: (value: boolean) => void;
}

interface ConfirmOptions {
  title: string;
  body?: string;
  confirmLabel?: string;
  danger?: boolean;
}

class ConfirmCatalog {
  request = $state<ConfirmRequest | null>(null);

  open(options: ConfirmOptions): Promise<boolean> {
    return new Promise((resolve) => {
      this.request = {
        title: options.title,
        body: options.body ?? "",
        confirmLabel: options.confirmLabel ?? "OK",
        danger: options.danger ?? false,
        resolve,
      };
    });
  }

  confirm(): void {
    const r = this.request;
    if (r === null) return;
    this.request = null;
    r.resolve(true);
  }

  cancel(): void {
    const r = this.request;
    if (r === null) return;
    this.request = null;
    r.resolve(false);
  }
}

export const confirmCatalog = new ConfirmCatalog();
