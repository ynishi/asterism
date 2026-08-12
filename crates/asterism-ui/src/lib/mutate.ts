// mutate — the write half of the `invoke` layer.
//
// `api()` next door is deliberately transparent: it hands a failure
// back to its caller and says nothing. That is right for a read, where
// `Resource` normalises the failure and a catalog decides what to
// render. It is wrong for a write, because there is no `Resource` on
// the write path and the caller's `catch` has historically ended at
// `console.warn` — the operation did not happen, and the interface
// carried on as though it had.
//
// So: same call, plus one guarantee — **a failure reaches the screen
// whether or not the call site remembers to render anything.** Scope
// that honestly: the guarantee covers the calls routed through here,
// and moving a call site to `mutate` is what buys it. The grid, group
// and trash paths are routed; tag detach, persona themes, material
// marks, threads, modalities, sessions and settings are not, and the
// console is still where their failures end.
//
// The error is re-thrown afterwards, which is what keeps this additive.
// Every existing `catch` that rolls back optimistic state, restores a
// selection or logs for a developer keeps working untouched; this only
// adds the half that was missing.
//
// Which failures are surfaced is decided by which wrapper a call site
// opted into, not by inspecting the error. A background refresh that
// fails is not a refused operation, and the way to say so is to leave
// it on `api()`. Two call sites in `App.svelte` make the point:
// `trash_asset` belongs here, `post-import refresh` does not.
//
// `action` is a verb phrase in the user's terms — "trash this asset",
// not "trash_asset". It reads as "Could not <action>."
import { api } from "./api";
import { undoToastCatalog } from "./stores/undo-toast.svelte";

/**
 * The reason to show under the message, or null when there is none.
 *
 * Every Tauri command here returns `{ kind, message }` on failure
 * (`src-tauri/src/error.rs`), and a plain `Error` carries `message` in
 * the same place — so one property read covers both, and covers a
 * serialization failure or a panic crossing the boundary as well. Not
 * imported from `bindings.ts`: that file carries the DTOs the UI
 * consumes, and `UiError` is not among them — it arrives as a rejection
 * value rather than as a return type.
 *
 * `kind` is read for exactly one variant, and it has to be. `UiError`
 * is `#[serde(tag = "kind", content = "message")]`, so what crosses the
 * wire is the *inner* string — the `#[error("not found: {0}")]` prefix
 * that makes it a sentence stays on the Rust side. For `NotFound` that
 * inner string is `format!("asset {id}")`, which would otherwise reach
 * the user as a bare id under "Could not delete this comment." The
 * other three carry their own prose (`Conflict` is "dir is not empty —
 * move or delete its contents first"), so prefixing them would only
 * add a word the sentence already implies.
 *
 * "could not be found", not "no longer exists". `error.rs:14-16` is
 * explicit that a restricted asset is hidden behind this same variant —
 * *"they surface as 'not found' for viewers outside their sharing
 * list"* — so the row may be perfectly intact and simply not this
 * viewer's to see. Saying it was destroyed would be a confident answer
 * to a question this side cannot tell apart.
 */
function detailOf(error: unknown): string | null {
  if (typeof error === "object" && error !== null && "message" in error) {
    const { kind, message } = error as { kind?: unknown; message: unknown };
    if (typeof message !== "string" || message === "") return null;
    return kind === "NotFound" ? `${message} could not be found` : message;
  }
  if (typeof error === "string" && error !== "") return error;
  return null;
}

/**
 * Invoke a command that changes something, and tell the user when it is
 * refused.
 *
 * @param action What the user asked for, as a verb phrase:
 *               `"trash this asset"`, `"rename the group"`.
 */
export async function mutate<T>(
  cmd: string,
  args: Record<string, unknown> | undefined,
  action: string,
): Promise<T> {
  try {
    // Deliberately does *not* clear a standing refusal on success. That
    // was tried and reverted: three of the bulk loops here continue past
    // a failure (`undoTrash`, `restoreMany`, `purgeMany` in
    // `App.svelte`), so with ids `[refused, ok, ok]` the second id's
    // success would wipe the first id's reason and leave a status line
    // saying two of three were done with nothing on screen to say why
    // the third was not — this issue's own defect, rebuilt inside a
    // loop, and visible or not depending on where in the selection the
    // refused id happened to sit.
    //
    // The cost of not clearing is that a refusal outlives the gesture
    // that raised it: trash A (refused), then trash B (fine), and A's
    // message is still there, worded so that it could be about B. That
    // is the lesser of the two, and it is what the dismiss button is
    // for. Recorded in the pull request rather than hidden here.
    return await api<T>(cmd, args);
  } catch (error) {
    undoToastCatalog.refuse(`Could not ${action}.`, detailOf(error));
    throw error;
  }
}
