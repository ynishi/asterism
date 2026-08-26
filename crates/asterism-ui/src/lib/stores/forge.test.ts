// forgeCatalog tests. The api / mutate choke points are mocked; the
// catalog's own machinery — three Resources, two deriveds, and what
// each write invalidates — runs for real.
//
// What these pin is the panel's answer to a question #180 had to settle
// once: **closing the drawer ends the question rather than pausing
// it**. Everything a selection produced goes with it, because the next
// open would otherwise show a fold of a chain that moved in between —
// and it will move, once #170's second child lands rounds on a line.
// Three separate clears written at three call sites is how one of them
// gets missed, which is what happened to `released` before this file
// existed.
//
// The other rule worth a test is that an entry off the line is not
// offered as something on it. The wire carries one boolean for two
// different answers — "was taken off" and "was never here" — and a
// derived that let either through as contents would be a panel
// claiming a line holds what it let go. Neither rule is visible in the
// markup, which is DOM and does not run here.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ForgeDiscardedDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
} from "../../bindings";
import { api } from "../api";
import { mutate } from "../mutate";
import { forgeCatalog } from "./forge.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));
vi.mock("../mutate", () => ({ mutate: vi.fn() }));

const apiMock = vi.mocked(api);
const mutateMock = vi.mocked(mutate);

function line(id: string, name: string, standing = "open"): ForgeLineDto {
  return {
    id,
    name,
    strategy_id: "mainline-first",
    standing,
    head_id: "h1",
    created_at_ms: 1,
    updated_at_ms: 1,
  };
}

function entry(id: string, alive: boolean): ForgeEntryStateDto {
  return { entry_id: id, alive, name: id, content_asset_id: `asset-${id}` };
}

function history(lineId: string): ForgeLineHistoryDto {
  return {
    line: line(lineId, lineId),
    genesis_id: "g1",
    genesis_at_ms: 1,
    changes: [],
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  forgeCatalog.open = false;
  forgeCatalog.selected = null;
  forgeCatalog.released = null;
  forgeCatalog.lines.reset();
  forgeCatalog.states.reset();
  forgeCatalog.history.reset();
  forgeCatalog.strategies.reset();
});

describe("what the panel holds", () => {
  it("offers only living entries as contents, and the rest apart", async () => {
    apiMock.mockResolvedValueOnce([entry("kept", true), entry("gone", false)]);
    await forgeCatalog.states.load({ lineId: "L1" });

    expect(forgeCatalog.onTheLine.map((e) => e.entry_id)).toEqual(["kept"]);
    expect(forgeCatalog.offTheLine.map((e) => e.entry_id)).toEqual(["gone"]);
  });

  it("reads both the list and the rules when the panel opens", async () => {
    apiMock.mockResolvedValue([]);
    await forgeCatalog.openPanel();

    expect(forgeCatalog.open).toBe(true);
    const called = apiMock.mock.calls.map((c) => c[0]);
    expect(called).toContain("list_forge_lines");
    // Opening a line needs a rule and there is no default to offer, so
    // the two are read together rather than the rules on first use.
    expect(called).toContain("list_forge_strategies");
  });
});

describe("closing ends the question", () => {
  it("drops the selection and everything it produced", async () => {
    apiMock.mockResolvedValue([entry("kept", true)]);
    await forgeCatalog.states.load({ lineId: "L1" });
    apiMock.mockResolvedValue(history("L1"));
    await forgeCatalog.history.load({ lineId: "L1" });
    forgeCatalog.selected = "L1";
    forgeCatalog.released = ["asset-1"];

    forgeCatalog.closePanel();

    expect(forgeCatalog.open).toBe(false);
    expect(forgeCatalog.selected).toBeNull();
    // A fold of a chain that may have moved is worse than no answer:
    // the next open reads again rather than correcting itself.
    expect(forgeCatalog.states.data).toEqual([]);
    expect(forgeCatalog.history.data).toBeNull();
    expect(forgeCatalog.released).toBeNull();
  });
});

describe("what each write invalidates", () => {
  it("re-reads the list after opening a line", async () => {
    mutateMock.mockResolvedValue(undefined);
    apiMock.mockResolvedValue([line("L1", "ROOT")]);

    await forgeCatalog.openLine("ROOT", "mainline-first");

    expect(mutateMock).toHaveBeenCalledWith(
      "open_forge_line",
      { command: { name: "ROOT", strategy_id: "mainline-first" } },
      expect.any(String),
    );
    expect(apiMock).toHaveBeenCalledWith("list_forge_lines", {});
    expect(forgeCatalog.lines.data.map((l) => l.name)).toEqual(["ROOT"]);
  });

  it("re-reads the list after a rename, since the name is on it", async () => {
    mutateMock.mockResolvedValue(undefined);
    apiMock.mockResolvedValue([line("L1", "the only line")]);

    await forgeCatalog.rename("L1", "the only line");

    expect(mutateMock).toHaveBeenCalledWith(
      "rename_forge_line",
      { lineId: "L1", command: { line_id: "L1", name: "the only line" } },
      expect.any(String),
    );
    expect(forgeCatalog.lines.data[0].name).toBe("the only line");
  });

  it("re-reads the list after a standing moves", async () => {
    mutateMock.mockResolvedValue(undefined);
    apiMock.mockResolvedValue([line("L1", "ROOT", "archived")]);

    await forgeCatalog.archive("L1");

    expect(mutateMock).toHaveBeenCalledWith(
      "archive_forge_line",
      { lineId: "L1" },
      expect.any(String),
    );
    expect(forgeCatalog.lines.data[0].standing).toBe("archived");
  });

  it("keeps what a discard released, and lets go of the line", async () => {
    const dropped: ForgeDiscardedDto = {
      line_id: "L1",
      released_asset_ids: ["a1", "a2"],
    };
    apiMock.mockResolvedValue([entry("kept", true)]);
    await forgeCatalog.states.load({ lineId: "L1" });
    forgeCatalog.selected = "L1";

    mutateMock.mockResolvedValue(dropped);
    apiMock.mockResolvedValue([]);
    await forgeCatalog.discard("L1");

    // The only place these ids are ever named: after the write there is
    // no record left to derive them from.
    expect(forgeCatalog.released).toEqual(["a1", "a2"]);
    expect(forgeCatalog.selected).toBeNull();
    expect(forgeCatalog.states.data).toEqual([]);
  });

  it("leaves another line's selection alone when discarding", async () => {
    apiMock.mockResolvedValue([entry("kept", true)]);
    await forgeCatalog.states.load({ lineId: "L2" });
    forgeCatalog.selected = "L2";

    mutateMock.mockResolvedValue({ line_id: "L1", released_asset_ids: [] });
    apiMock.mockResolvedValue([]);
    await forgeCatalog.discard("L1");

    expect(forgeCatalog.selected).toBe("L2");
    expect(forgeCatalog.states.data.map((e) => e.entry_id)).toEqual(["kept"]);
  });
});
