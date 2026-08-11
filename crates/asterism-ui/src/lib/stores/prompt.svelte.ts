// Prompt catalog — inline text-input modal, replaces
// `window.prompt()` which returns a silent `null` on Tauri v2
// macOS WKWebView. Callers await
// `open(title, placeholder, default)` and receive either the
// typed string or `null` when the user cancels.
//
// Scope:
//   - `request: { title, placeholder, draft, resolve } | null` —
//     the currently-open prompt. `null` = idle. `resolve` is the
//     `Promise` continuation the caller is awaiting; `commit()` /
//     `cancel()` invoke it exactly once and null the request
//     back out so the modal unmounts.
//   - `open(title, placeholder, defaultValue)` — returns a
//     `Promise<string | null>` that resolves with the user's
//     answer (Enter / OK) or `null` (Escape / Cancel /
//     backdrop click). Overwriting an in-flight prompt is not
//     defended against — the callsite pattern is
//     `const name = await promptCatalog.open(...)` inside a
//     handler that gates re-entry via UI (button disable) or by
//     awaiting the previous promise first. This matches the
//     pre-extraction App-inline behaviour where `customPrompt`
//     had the same shape.
//   - `commit()` — resolves the current request with the current
//     `draft` and clears the state.
//   - `cancel()` — resolves the current request with `null` and
//     clears the state.
//
// Deliberately NOT owned here:
//   - Input focus. Whichever component renders the modal owns
//     the `<input>` element and drives the focus timing (a
//     `$effect` on `request` transitioning to non-null is the
//     idiomatic Svelte 5 hook). Keeping the DOM ref out of the
//     store lets multiple render surfaces coexist if we ever
//     need one (unlikely, but the boundary keeps the store DOM-
//     agnostic).

interface PromptRequest {
  title: string;
  placeholder: string;
  draft: string;
  resolve: (value: string | null) => void;
}

class PromptCatalog {
  request = $state<PromptRequest | null>(null);

  open(
    title: string,
    placeholder = "",
    defaultValue = "",
  ): Promise<string | null> {
    return new Promise((resolve) => {
      this.request = { title, placeholder, draft: defaultValue, resolve };
    });
  }

  commit(): void {
    const r = this.request;
    if (r === null) return;
    this.request = null;
    r.resolve(r.draft);
  }

  cancel(): void {
    const r = this.request;
    if (r === null) return;
    this.request = null;
    r.resolve(null);
  }
}

export const promptCatalog = new PromptCatalog();
