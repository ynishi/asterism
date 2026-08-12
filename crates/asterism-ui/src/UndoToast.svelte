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

{#if undoToastCatalog.refusal !== null}
  {@const refusal = undoToastCatalog.refusal}
  <!-- `role="alert"` and `aria-live="assertive"`, unlike the two below:
       this one reports that something the user asked for did not
       happen, which is worth interrupting a screen reader for. Still
       not `alertdialog` — it steals no focus and blocks nothing. -->
  <div
    class="refusal-toast"
    class:stacked-one={undoToastCatalog.toast !== null ||
      dispatchCatalog.status !== null}
    class:stacked-two={undoToastCatalog.toast !== null &&
      dispatchCatalog.status !== null}
    role="alert"
    aria-live="assertive"
  >
    <div class="refusal-toast-text">
      <span class="refusal-toast-message">{refusal.message}</span>
      {#if refusal.detail !== null}
        <span class="refusal-toast-detail">{refusal.detail}</span>
      {/if}
    </div>
    <!-- Named apart from the Undo toast's dismiss below: the two can be
         on screen together, and "Dismiss" twice gives a screen-reader
         user no way to tell which one they are on. -->
    <button
      type="button"
      class="undo-toast-dismiss"
      aria-label="Dismiss this message"
      onclick={() => undoToastCatalog.dismissRefusal()}
    >✕</button>
  </div>
{/if}

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

  /* Same family again, and the same shelf when alone. It climbs one row
     per occupied slot below it rather than claiming a fixed row: a
     refusal is the least frequent of the three, and a permanent gap
     reserved for it would read as the mid-air float the undo slot was
     moved off in the 2026-08-01 feedback.

     Same width as its siblings, and taller when it needs to be. Those
     two carry a phrase the user can finish reading in the second before
     it fades; this one carries a sentence the backend wrote, has no
     fade, and is the only one whose text the user may need to read
     twice. So the message wraps here instead of being clipped with an
     ellipsis, which is exactly what a truncated reason would deserve to
     be called. */
  .refusal-toast {
    position: fixed;
    bottom: 5rem;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.6rem 0.6rem 0.6rem 1rem;
    background: #33202a;
    color: #ffe9ef;
    border-left: 3px solid #ff6f8f;
    border-radius: 6px;
    font-size: 0.85rem;
    box-shadow: 0 6px 18px rgba(23, 22, 42, 0.3);
    z-index: 47;
    max-width: 60ch;
  }
  .refusal-toast.stacked-one {
    bottom: 8.5rem;
  }
  .refusal-toast.stacked-two {
    bottom: 12rem;
  }
  .refusal-toast-text {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .refusal-toast-message {
    font-weight: 600;
  }
  .refusal-toast-detail {
    color: #e0b8c4;
    font-size: 0.8rem;
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
