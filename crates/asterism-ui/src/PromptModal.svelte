<script lang="ts">
  // PromptModal — extracted from App.svelte (2026-07-21 Phase C
  // wave A / SavedQuery-modal sibling). Owns the inline text-
  // input overlay that replaces `window.prompt()` (silent `null`
  // on Tauri v2 macOS WKWebView). Consumes `promptCatalog`
  // directly; every callsite in App becomes
  // `const answer = await promptCatalog.open(...)`.
  //
  // Focus handling stays on the component (not on the store)
  // because the DOM ref lives here. A `$effect` fires when
  // `promptCatalog.request` transitions from `null` to a
  // request and focuses the input on the next microtask.
  //
  // Consumes:
  //   - promptCatalog.request / .commit / .cancel          (store)
  //
  // No props — this is a leaf modal that reads and writes only
  // through the store. Mount it once at the top level of App.
  import { promptCatalog } from "./lib/stores/prompt.svelte";

  let inputEl: HTMLInputElement | null = null;

  // Focus the input the moment a fresh request opens. Reading
  // `request` inside the effect auto-tracks the transition; the
  // microtask defer lets Svelte mount the `<input>` before the
  // focus call fires.
  $effect(() => {
    if (promptCatalog.request !== null) {
      queueMicrotask(() => inputEl?.focus());
    }
  });
</script>

{#if promptCatalog.request !== null}
  {@const req = promptCatalog.request}
  <div
    class="prompt-backdrop"
    onclick={() => promptCatalog.cancel()}
    role="button"
    tabindex="-1"
    aria-label="Cancel prompt"
  ></div>
  <div class="prompt-panel" role="dialog" aria-modal="true" aria-labelledby="prompt-title">
    <h3 id="prompt-title" class="prompt-title">{req.title}</h3>
    <input
      class="prompt-input"
      type="text"
      placeholder={req.placeholder}
      bind:value={req.draft}
      bind:this={inputEl}
      onkeydown={(e) => {
        // Escape is owned by App's interaction-stack switch (the
        // "prompt" mode mirrors promptCatalog.request) — handling it
        // here too would double-fire on the same keypress.
        if (e.key === "Enter") { e.preventDefault(); promptCatalog.commit(); }
      }}
    />
    <div class="prompt-actions">
      <button class="prompt-btn ghost" onclick={() => promptCatalog.cancel()}>Cancel</button>
      <button class="prompt-btn primary" onclick={() => promptCatalog.commit()}>OK</button>
    </div>
  </div>
{/if}

<style>
  /* Copied verbatim from App.svelte (Phase C wave A extraction).
     Same duplication policy as the other extracted modals —
     SavedQueryDetailModal ships its own `.prompt-*` cascade
     with slightly different values (min-width / z-index /
     backdrop colour) because wave 7 tuned the modal to sit
     above the customPrompt one; not consolidated to avoid a
     visual regression. */

  .prompt-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(23, 22, 42, 0.35);
    z-index: 60;
    cursor: default;
  }
  .prompt-panel {
    position: fixed;
    top: 40%;
    left: 50%;
    transform: translate(-50%, -50%);
    min-width: 30rem;
    max-width: 90vw;
    background: #ffffff;
    padding: 1.1rem 1.3rem;
    border-radius: 10px;
    box-shadow: 0 20px 60px rgba(23, 22, 42, 0.35);
    z-index: 61;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .prompt-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: #1f1e33;
  }
  .prompt-input {
    padding: 0.55rem 0.7rem;
    font-size: 0.95rem;
    border: 1px solid #d3d1e8;
    border-radius: 6px;
    outline: none;
    color: #1f1e33;
    background: #fafafd;
    font-family: inherit;
  }
  .prompt-input:focus {
    border-color: #5850ff;
    box-shadow: 0 0 0 3px rgba(88, 80, 255, 0.18);
    background: #ffffff;
  }
  .prompt-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }
  .prompt-btn {
    padding: 0.4rem 1rem;
    border-radius: 6px;
    font-size: 0.9rem;
    cursor: pointer;
    border: 1px solid transparent;
    font-family: inherit;
  }
  .prompt-btn.primary {
    background: #5850ff;
    color: #ffffff;
    border-color: #5850ff;
  }
  .prompt-btn.primary:hover {
    background: #4a42e0;
    border-color: #4a42e0;
  }
  .prompt-btn.ghost {
    background: transparent;
    color: #555;
    border-color: #ccc;
  }
  .prompt-btn.ghost:hover {
    background: #f2f2f6;
  }
</style>
