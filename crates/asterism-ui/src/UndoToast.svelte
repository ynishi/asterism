<script lang="ts">
  // UndoToast — the message-with-a-way-back, mounted once at the top
  // level of App. Leaf component with no props: it reads and writes
  // only through its store, the same shape as ConfirmModal.
  //
  // Consumes:
  //   - undoToastCatalog.toast / .act / .dismiss             (store)
  //
  // Sits above DispatchToast rather than replacing it: the two can be
  // on screen together (a dispatch running while a card is trashed),
  // and a toast that covered the other would hide progress the user
  // did not ask to trade away.
  //
  // `role="status"` and not `alertdialog`: this steals no focus and
  // blocks nothing. The action is a plain button, so keyboard users
  // reach it by Tab; there is no Escape handling here, because a
  // non-modal toast that swallowed Escape would take it from whatever
  // overlay is actually on screen.
  import { undoToastCatalog } from "./lib/stores/undo-toast.svelte";
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
</script>

{#if undoToastCatalog.toast !== null}
  {@const toast = undoToastCatalog.toast}
  <div
    class="undo-toast"
    class:stacked={dispatchCatalog.status !== null}
    role="status"
    aria-live="polite"
  >
    <span class="undo-toast-message">{toast.message}</span>
    <button
      type="button"
      class="undo-toast-action"
      onclick={() => void undoToastCatalog.act()}
    >{toast.actionLabel}</button>
    <button
      type="button"
      class="undo-toast-dismiss"
      aria-label="Dismiss"
      onclick={() => undoToastCatalog.dismiss()}
    >✕</button>
  </div>
{/if}

<style>
  /* Same visual family as `.dispatch-toast` (DispatchToast.svelte) and
     the same shelf when alone: an undo snackbar's home is the bottom
     edge (Material snackbar spec; Gmail / Google Photos put Undo
     there), and a fixed slot one row up read as "floating mid-screen"
     the moment the dispatch slot under it was empty (2026-08-01
     feedback). It climbs to the higher row only while a dispatch
     message actually occupies the bottom one — both visible, neither
     covered. z-index one above DispatchToast: when they overlap during
     a transition, the one carrying an action has to be the clickable
     one. */
  .undo-toast {
    position: fixed;
    bottom: 5rem;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.6rem 0.5rem 1rem;
    background: #1f1e33;
    color: #e9e7ff;
    border-radius: 6px;
    font-size: 0.85rem;
    box-shadow: 0 6px 18px rgba(23, 22, 42, 0.3);
    z-index: 46;
    max-width: 60ch;
  }
  .undo-toast.stacked {
    bottom: 8.5rem;
  }
  .undo-toast-message {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .undo-toast-action {
    flex: none;
    padding: 0.25rem 0.7rem;
    border-radius: 5px;
    border: 1px solid #6f68ff;
    background: transparent;
    color: #b9b4ff;
    font-family: inherit;
    font-size: 0.82rem;
    font-weight: 600;
    cursor: pointer;
  }
  .undo-toast-action:hover {
    background: #2c2a4d;
    color: #e9e7ff;
  }
  .undo-toast-dismiss {
    flex: none;
    padding: 0.15rem 0.35rem;
    border: none;
    background: transparent;
    color: #8f8bb5;
    font-family: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .undo-toast-dismiss:hover {
    color: #e9e7ff;
  }
</style>
