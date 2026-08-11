<script lang="ts">
  // ConfirmModal — the in-app replacement for `window.confirm()`.
  // Sibling of `PromptModal.svelte`, same shape: a leaf modal with no
  // props that reads and writes only through its store. Mount it once
  // at the top level of App; every callsite becomes
  // `if (!(await confirmCatalog.open({ ... }))) return;`.
  //
  // Consumes:
  //   - confirmCatalog.request / .confirm / .cancel          (store)
  //
  // Focus lands on Cancel, not on the confirm button. This modal only
  // ever guards something irreversible, so the answer a stray Return
  // gives has to be the safe one. The destructive button is reachable
  // by Tab or by pointer — one deliberate move away, which is the
  // whole point of asking.
  //
  // Escape is NOT handled here. App mirrors `confirmCatalog.request`
  // onto the interaction-mode stack and its single Escape switch calls
  // `cancel()`, the same arrangement PromptModal uses; a listener here
  // would double-handle the keypress.
  import { confirmCatalog } from "./lib/stores/confirm.svelte";

  let cancelEl: HTMLButtonElement | null = $state(null);

  // Focus the safe choice the moment a question opens. The microtask
  // defer lets Svelte mount the button before the focus call fires.
  $effect(() => {
    if (confirmCatalog.request !== null) {
      queueMicrotask(() => cancelEl?.focus());
    }
  });
</script>

{#if confirmCatalog.request !== null}
  {@const req = confirmCatalog.request}
  <!-- A real `<button>`, not the `div role="button"` PromptModal uses.
       Same behaviour, but a native button carries the keyboard
       semantics the a11y rules ask a click handler to have, so this
       one ships without the warning its sibling still emits.
       `tabindex="-1"` keeps a full-screen control out of the tab
       order — Escape and the Cancel button are the keyboard paths. -->
  <button
    type="button"
    class="confirm-backdrop"
    onclick={() => confirmCatalog.cancel()}
    tabindex="-1"
    aria-label="Cancel"
  ></button>
  <div
    class="confirm-panel"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="confirm-title"
    aria-describedby="confirm-body"
  >
    <h3 id="confirm-title" class="confirm-title">{req.title}</h3>
    <p id="confirm-body" class="confirm-body">{req.body}</p>
    <div class="confirm-actions">
      <button
        class="confirm-btn ghost"
        bind:this={cancelEl}
        onclick={() => confirmCatalog.cancel()}
      >Cancel</button>
      <button
        class="confirm-btn"
        class:danger={req.danger}
        class:primary={!req.danger}
        onclick={() => confirmCatalog.confirm()}
      >{req.confirmLabel}</button>
    </div>
  </div>
{/if}

<style>
  /* Sits above every other layer on purpose: this modal is only ever
     opened from inside one (the card context menu, the thread drawer),
     and a confirm painted behind the thing that raised it is the same
     defect as no confirm at all. */
  .confirm-backdrop {
    position: fixed;
    inset: 0;
    /* Button resets — this is a `<button>` for its keyboard semantics,
       not for its looks. */
    appearance: none;
    border: none;
    padding: 0;
    margin: 0;
    display: block;
    background: rgba(23, 22, 42, 0.4);
    z-index: 1200;
    cursor: default;
  }
  .confirm-panel {
    position: fixed;
    top: 40%;
    left: 50%;
    transform: translate(-50%, -50%);
    min-width: 24rem;
    max-width: 90vw;
    background: #ffffff;
    padding: 1.1rem 1.3rem;
    border-radius: 10px;
    box-shadow: 0 20px 60px rgba(23, 22, 42, 0.35);
    z-index: 1201;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .confirm-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: #1f1e33;
  }
  .confirm-body {
    margin: 0;
    font-size: 0.88rem;
    line-height: 1.45;
    color: #4a4863;
  }
  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
  .confirm-btn {
    padding: 0.4rem 1rem;
    border-radius: 6px;
    font-size: 0.9rem;
    cursor: pointer;
    border: 1px solid transparent;
    font-family: inherit;
  }
  .confirm-btn.primary {
    background: #5850ff;
    color: #ffffff;
    border-color: #5850ff;
  }
  .confirm-btn.primary:hover {
    background: #4a42e0;
    border-color: #4a42e0;
  }
  /* The irreversible choice carries the warning tone (HIG: a
     destructive action is marked before it is taken, not after). */
  .confirm-btn.danger {
    background: #c0392b;
    color: #ffffff;
    border-color: #c0392b;
  }
  .confirm-btn.danger:hover {
    background: #a5301f;
    border-color: #a5301f;
  }
  .confirm-btn.ghost {
    background: transparent;
    color: #555;
    border-color: #ccc;
  }
  .confirm-btn.ghost:hover {
    background: #f2f2f6;
  }
</style>
