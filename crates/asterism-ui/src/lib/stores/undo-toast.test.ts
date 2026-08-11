// undoToastCatalog unit tests. The store is a singleton, so each test
// dismisses first; the timers are faked, because the two properties
// worth asserting here are both about *when* the offer goes away —
// and a real 6-second wait per assertion would price them out.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { undoToastCatalog } from "./undo-toast.svelte";

describe("undoToastCatalog", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    undoToastCatalog.dismiss();
  });

  afterEach(() => {
    undoToastCatalog.dismiss();
    vi.useRealTimers();
  });

  it("shows the message with Undo as the default action label", () => {
    undoToastCatalog.show({ message: "Moved to Trash", onAction: () => {} });
    expect({
      message: undoToastCatalog.toast?.message,
      actionLabel: undoToastCatalog.toast?.actionLabel,
    }).toEqual({ message: "Moved to Trash", actionLabel: "Undo" });
  });

  it("takes itself down once the window closes", () => {
    undoToastCatalog.show({ message: "one", onAction: () => {} });
    // The default window, not one passed in: a caller that stopped
    // arming the timer would still pass a test that supplied `ms`.
    vi.advanceTimersByTime(7999);
    expect(undoToastCatalog.toast).not.toBeNull();
    vi.advanceTimersByTime(1);
    expect(undoToastCatalog.toast).toBeNull();
  });

  // The regression this store exists to not have: an outgoing toast's
  // timer firing on its successor. Both carry the same text, so the
  // guard has to be identity rather than equality.
  it("does not let a replaced toast's timer dismiss the one that replaced it", () => {
    undoToastCatalog.show({ message: "same text", onAction: () => {}, ms: 6000 });
    vi.advanceTimersByTime(5000);
    undoToastCatalog.show({ message: "same text", onAction: () => {}, ms: 6000 });
    // Past the first toast's deadline, well inside the second's.
    vi.advanceTimersByTime(2000);
    expect(undoToastCatalog.toast).not.toBeNull();
  });

  it("runs the action once and clears before awaiting it", async () => {
    let runs = 0;
    undoToastCatalog.show({
      message: "one",
      onAction: () => {
        runs += 1;
      },
    });
    const first = undoToastCatalog.act();
    // Cleared synchronously, which is what makes the second click a
    // no-op instead of a second restore.
    expect(undoToastCatalog.toast).toBeNull();
    await first;
    await undoToastCatalog.act();
    expect(runs).toBe(1);
  });

  it("drops the action when the toast is dismissed instead of taken", async () => {
    let runs = 0;
    undoToastCatalog.show({
      message: "one",
      onAction: () => {
        runs += 1;
      },
    });
    undoToastCatalog.dismiss();
    await undoToastCatalog.act();
    expect(runs).toBe(0);
  });
});
