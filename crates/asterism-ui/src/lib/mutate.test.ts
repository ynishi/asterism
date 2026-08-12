// `mutate` unit tests. The one property worth pinning is the pair:
// a refused write must both reach the toast store *and* keep throwing,
// because every existing `catch` on the write path rolls back local
// state and would silently stop doing so if the error were swallowed
// here.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { mutate } from "./mutate";
import { undoToastCatalog } from "./stores/undo-toast.svelte";

vi.mock("./api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

describe("mutate", () => {
  beforeEach(() => {
    apiMock.mockReset();
    undoToastCatalog.dismissRefusal();
  });

  afterEach(() => {
    undoToastCatalog.dismissRefusal();
  });

  it("says nothing when the command succeeds", async () => {
    apiMock.mockResolvedValue("ok");
    await expect(mutate("trash_asset", { id: "a" }, "trash this")).resolves.toBe(
      "ok",
    );
    expect(undoToastCatalog.refusal).toBeNull();
  });

  it("keeps a refusal standing when a later call succeeds", async () => {
    // The case this protects: a bulk loop that continues past a failure
    // (`restoreMany`, `purgeMany`, `undoTrash`) would otherwise have the
    // next id's success erase the refused id's reason, leaving a partial
    // status line with nothing on screen to explain it. Clearing on
    // success was tried and reverted for exactly this.
    apiMock.mockRejectedValue({ kind: "Conflict", message: "referenced" });
    await mutate("restore_asset", { id: "1" }, "restore the first").catch(() => {});
    apiMock.mockReset();
    apiMock.mockResolvedValue(undefined);
    await mutate("restore_asset", { id: "2" }, "restore the second");
    expect(undoToastCatalog.refusal?.detail).toBe("referenced");
  });

  it("surfaces the refusal in the user's terms, with the backend's reason", async () => {
    // The `{ kind, message }` shape every Tauri command returns on
    // failure (`src-tauri/src/error.rs`).
    apiMock.mockRejectedValue({
      kind: "Conflict",
      message: "asset is referenced by a snapshot",
    });
    await expect(
      mutate("trash_asset", { id: "a" }, "move this to the trash"),
    ).rejects.toBeDefined();
    expect(undoToastCatalog.refusal).toEqual({
      message: "Could not move this to the trash.",
      detail: "asset is referenced by a snapshot",
    });
  });

  it("makes a NotFound readable instead of a bare id", async () => {
    // The shape the backend actually emits: the tagged enum sends the
    // inner string, so `#[error("not found: {0}")]`'s prefix never
    // leaves Rust and `message` is `format!("asset {id}")`.
    apiMock.mockRejectedValue({
      kind: "NotFound",
      message: "asset 3f2a5c10-0000-4000-8000-000000000001",
    });
    await expect(
      mutate("delete_asset_comment", { id: "c" }, "delete this comment"),
    ).rejects.toBeDefined();
    expect(undoToastCatalog.refusal?.detail).toBe(
      "asset 3f2a5c10-0000-4000-8000-000000000001 could not be found",
    );
  });

  it("leaves the other variants' own prose alone", async () => {
    apiMock.mockRejectedValue({
      kind: "Conflict",
      message: "dir is not empty — move or delete its contents first",
    });
    await expect(
      mutate("delete_dir", { id: "d" }, "delete this folder"),
    ).rejects.toBeDefined();
    expect(undoToastCatalog.refusal?.detail).toBe(
      "dir is not empty — move or delete its contents first",
    );
  });

  it("re-throws, so a caller's rollback still runs", async () => {
    apiMock.mockRejectedValue({ kind: "Internal", message: "disk full" });
    let rolledBack = false;
    try {
      await mutate("purge_asset", { id: "a" }, "delete this permanently");
    } catch {
      rolledBack = true;
    }
    expect(rolledBack).toBe(true);
  });

  it("uses the text of a rejection that is not a UiError", async () => {
    // A serialization failure or a panic crossing the boundary arrives
    // as an `Error`, which carries `message` in the same place a
    // `UiError` does. Nothing special is needed to read it.
    apiMock.mockRejectedValue(new Error("boom"));
    await expect(mutate("empty_trash", {}, "empty the trash")).rejects.toThrow();
    expect(undoToastCatalog.refusal).toEqual({
      message: "Could not empty the trash.",
      detail: "boom",
    });
  });

  it("still names the operation when the rejection carries no text", async () => {
    // The remaining case: something with no readable message at all.
    // Saying only that it was refused beats the console-only silence
    // this exists to remove.
    apiMock.mockRejectedValue({ unexpected: true });
    await expect(mutate("empty_trash", {}, "empty the trash")).rejects.toBeDefined();
    expect(undoToastCatalog.refusal).toEqual({
      message: "Could not empty the trash.",
      detail: null,
    });
  });

  it("keeps the newest refusal when a second one arrives", async () => {
    apiMock.mockRejectedValue({ kind: "NotFound", message: "asset 1" });
    await mutate("trash_asset", { id: "1" }, "trash the first").catch(() => {});
    apiMock.mockRejectedValue({ kind: "NotFound", message: "asset 2" });
    await mutate("trash_asset", { id: "2" }, "trash the second").catch(() => {});
    expect(undoToastCatalog.refusal?.detail).toBe("asset 2 could not be found");
  });
});
