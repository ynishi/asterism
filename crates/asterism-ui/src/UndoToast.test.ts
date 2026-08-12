/**
 * @vitest-environment happy-dom
 *
 * UndoToast rendering tests — specifically, that a refusal put into the
 * store reaches the document.
 *
 * `lib/stores/undo-toast.test.ts` next door pins the store's two slots,
 * and `lib/mutate.test.ts` pins that a refused write fills one of them.
 * Neither can answer the question this file exists for, and it is the
 * question the whole change is about: **a message that exists in state
 * and never renders is the defect, not the fix.** There is precedent —
 * a cycle rejection in nested drag-and-drop stopped appearing because
 * its error element sat under an unrelated conditional, and it was
 * found by hand during device testing rather than by a test.
 *
 * A document is enough to answer it. The refusal is plain markup gated
 * on one store field; nothing about whether it renders depends on a
 * WebView, a real backend, or a window. What a driven suite would add
 * here is minutes and a second place for the same assertion to drift.
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import UndoToast from "./UndoToast.svelte";
import { undoToastCatalog } from "./lib/stores/undo-toast.svelte";

describe("UndoToast — refusals", () => {
  beforeEach(() => {
    undoToastCatalog.dismiss();
    undoToastCatalog.dismissRefusal();
  });

  afterEach(() => {
    cleanup();
    undoToastCatalog.dismiss();
    undoToastCatalog.dismissRefusal();
  });

  it("renders nothing when nothing was refused", () => {
    render(UndoToast);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("puts the refusal and its reason on screen", async () => {
    render(UndoToast);
    undoToastCatalog.refuse(
      "Could not delete this folder.",
      "dir is not empty — move or delete its contents first",
    );
    // `findBy`, not `getBy`: the store write and the render are
    // separate ticks.
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not delete this folder.");
    expect(alert.textContent).toContain("dir is not empty");
  });

  it("announces assertively, unlike the two toasts below it", async () => {
    render(UndoToast);
    undoToastCatalog.refuse("Could not trash this.");
    const alert = await screen.findByRole("alert");
    // An Undo offer is `polite` — it reports something that happened.
    // This reports something that did not, over whatever the reader was
    // being told.
    expect(alert.getAttribute("aria-live")).toBe("assertive");
  });

  it("shows the message alone when the backend gave no reason", async () => {
    render(UndoToast);
    undoToastCatalog.refuse("Could not empty the trash.");
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not empty the trash.");
  });

  it("goes away when dismissed", async () => {
    const user = userEvent.setup();
    render(UndoToast);
    undoToastCatalog.refuse("Could not trash this.");
    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: "Dismiss this message" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps its dismiss distinguishable from the Undo toast's", async () => {
    const user = userEvent.setup();
    render(UndoToast);
    undoToastCatalog.show({ message: "Moved to Trash", onAction: () => {} });
    undoToastCatalog.refuse("Could not trash the next one.");
    await screen.findByRole("alert");
    // Both dismisses are on screen. `getByRole` throws on more than one
    // match, so this fails outright if they ever share a name again —
    // which is the state a screen-reader user cannot navigate.
    await user.click(screen.getByRole("button", { name: "Dismiss this message" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(await screen.findByRole("status")).toBeTruthy();
  });

  it("shares the screen with an Undo offer rather than replacing it", async () => {
    render(UndoToast);
    undoToastCatalog.show({ message: "Moved to Trash", onAction: () => {} });
    undoToastCatalog.refuse("Could not trash the next one.");
    const alert = await screen.findByRole("alert");
    const status = await screen.findByRole("status");
    // Both, at once: a refusal is not a reason to withdraw an offer the
    // user may still want to take.
    expect(alert.textContent).toContain("Could not trash the next one.");
    expect(status.textContent).toContain("Moved to Trash");
  });
});
