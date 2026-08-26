// forgeCatalog tests. The api / mutate choke points are mocked; the
// catalog's own machinery — its Resources, its deriveds, and what each
// write invalidates — runs for real.
//
// What these pin is the panel's answer to a question #180 had to settle
// once: **closing the drawer ends the question rather than pausing
// it**. Everything a selection produced goes with it, because the next
// open would otherwise show a fold of a chain that moved in between,
// and a satisfied close moves one. Separate clears written at each call
// site is how one of them gets missed, which is what happened to
// `released` before this file existed.
//
// The other rule worth a test is that an entry off the line is not
// offered as something on it. The wire carries one boolean for two
// different answers — "was taken off" and "was never here" — and a
// derived that let either through as contents would be a panel
// claiming a line holds what it let go. Neither rule is visible in the
// markup, which is DOM and does not run here.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ForgeCloseDto,
  ForgeDiscardedDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
  ForgeOpDto,
  ForgePursuitDto,
  ForgeRoundDto,
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

function pursuit(id: string, close: ForgeCloseDto | null = null): ForgePursuitDto {
  return {
    id,
    line_id: "L1",
    parent_id: null,
    base_id: "c1",
    head_id: id,
    title: id,
    note: null,
    opened_at_ms: 1,
    opened_by_kind: "user",
    opened_by_id: "u1",
    rounds: [],
    close,
  };
}

function ended(id: string, outcome: string): ForgePursuitDto {
  return pursuit(id, {
    id: `${id}-close`,
    parent_id: id,
    outcome,
    note: null,
    at_ms: 2,
    actor_kind: "user",
    actor_id: "u1",
  });
}

/** A pursuit whose single round asks for these operations. */
function asking(id: string, ops: ForgeOpDto[]): ForgePursuitDto {
  return { ...pursuit(id), rounds: [{ ...round("r1"), ops }] };
}

function round(id: string): ForgeRoundDto {
  return {
    id,
    parent_id: "p0",
    at_ms: 3,
    actor_kind: "system",
    actor_id: "u1",
    note: null,
    ops: [],
  };
}

/// Answers each read by name. The work half fans several reads out at
/// once, so which command asked is the only thing that separates them —
/// a queue of `mockResolvedValueOnce` would pin an order that
/// `Promise.all` does not promise.
///
/// The throw is a diagnostic and not an assertion: it happens inside a
/// `Resource` fetcher, which catches, warns and carries on. A test that
/// cares whether a read was made asserts on the calls.
function answering(table: Record<string, unknown>): void {
  apiMock.mockImplementation((async (cmd: string) => {
    if (!(cmd in table)) throw new Error(`unexpected read: ${cmd}`);
    return table[cmd];
  }) as unknown as typeof api);
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
  forgeCatalog.pursuits.reset();
  forgeCatalog.clearWork();
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

describe("work against a line", () => {
  it("keeps ended work out of what a round can be written to", async () => {
    answering({
      list_forge_pursuits_of_line: [
        pursuit("open-one"),
        ended("done", "satisfied"),
        ended("dropped", "abandoned"),
      ],
      get_forge_line_states: [],
    });
    await forgeCatalog.selectLine("L1");

    expect(forgeCatalog.openWork.map((w) => w.id)).toEqual(["open-one"]);
    // Both endings stay listed, for the reason the `pursuits` read
    // gives.
    expect(forgeCatalog.endedWork.map((w) => w.id)).toEqual([
      "done",
      "dropped",
    ]);
  });

  it("asks nothing about a pursuit it has just cut from the head", async () => {
    mutateMock.mockResolvedValue(pursuit("P1"));
    answering({
      get_forge_pursuit: pursuit("P1"),
      list_forge_pursuits_of_line: [pursuit("P1")],
    });

    await forgeCatalog.openPursuit("L1", "a title", null);

    expect(forgeCatalog.working).toBe("P1");
    // Level with the line by construction: `open` cuts from where the
    // line is now, so both answers are empty and the two reads that
    // would say so are not made.
    const asked = apiMock.mock.calls.map((c) => c[0]);
    expect(asked).not.toContain("get_forge_pursuit_collisions");
    expect(asked).not.toContain("get_forge_pursuit_behind");
  });

  it("re-reads collisions after a round, and not how far behind it is", async () => {
    mutateMock.mockResolvedValue(undefined);
    answering({
      get_forge_pursuit: pursuit("P1"),
      get_forge_pursuit_collisions: [],
    });

    await forgeCatalog.pushRound(
      "P1",
      [{ entry_id: "e1", kind: "add", content_asset_id: "a1", name: "one" }],
      null,
    );

    // What a round asks for is half of what a collision is made of, so
    // writing one can make or clear one. How far behind the work is
    // counts landings on the line, which no write on this side moves.
    expect(apiMock.mock.calls.map((c) => c[0])).not.toContain(
      "get_forge_pursuit_behind",
    );
  });

  it("reports a rule that declined rather than raising it", async () => {
    mutateMock.mockResolvedValue({ round: null, collisions: [] });
    answering({
      get_forge_pursuit: pursuit("P1"),
      get_forge_pursuit_collisions: [],
    });

    await expect(forgeCatalog.resolve("P1")).resolves.toBe(false);

    mutateMock.mockResolvedValue({ round: round("r1"), collisions: [] });
    await expect(forgeCatalog.resolve("P1")).resolves.toBe(true);
  });

  it("lets go of the line's contents and chain when work closes", async () => {
    apiMock.mockResolvedValue(history("L1"));
    await forgeCatalog.history.load({ lineId: "L1" });

    mutateMock.mockResolvedValue(undefined);
    answering({
      get_forge_pursuit: ended("P1", "satisfied"),
      get_forge_pursuit_collisions: [],
      list_forge_pursuits_of_line: [ended("P1", "satisfied")],
      get_forge_line_states: [entry("landed", true)],
    });

    await forgeCatalog.closePursuit("P1", "L1", "satisfied", null);

    // The one verb on this side that moves a line moves both answers
    // about it: what it holds is re-read, and the chain is dropped so
    // the history tab reads again rather than showing a fold from
    // before the landing.
    expect(forgeCatalog.states.data.map((e) => e.entry_id)).toEqual(["landed"]);
    expect(forgeCatalog.history.data).toBeNull();
  });

  it("lets go of the work when the line under it changes", async () => {
    mutateMock.mockResolvedValue(pursuit("P1"));
    answering({
      get_forge_pursuit: pursuit("P1"),
      list_forge_pursuits_of_line: [],
      get_forge_line_states: [],
    });
    await forgeCatalog.openPursuit("L1", null, null);
    expect(forgeCatalog.working).toBe("P1");

    await forgeCatalog.selectLine("L2");

    // A pursuit id belongs to the line it is against. Left standing, it
    // would name work under a line it is not against.
    expect(forgeCatalog.working).toBeNull();
    expect(forgeCatalog.pursuit.data).toBeNull();
  });

  it("folds what the work asks for over what the line holds", async () => {
    answering({
      get_forge_line_states: [entry("held", true), entry("let-go", false)],
      list_forge_pursuits_of_line: [],
    });
    await forgeCatalog.selectLine("L1");

    apiMock.mockResolvedValue(
      asking("P1", [
        { entry_id: "fresh", kind: "add", content_asset_id: "a9", name: "new" },
        { entry_id: "held", kind: "rename", content_asset_id: null, name: "renamed" },
        { entry_id: "held", kind: "remove", content_asset_id: null, name: null },
      ]),
    );
    await forgeCatalog.pursuit.load({ pursuitId: "P1" });

    const rows = new Map(forgeCatalog.projection.map((r) => [r.entryId, r]));
    // Existence arrives with the content and name it was added under.
    expect(rows.get("fresh")).toMatchObject({ name: "new", alive: true });
    // Existence standing alone: renaming something on its way off says
    // nothing anybody can read back, so what the line ends up calling
    // it is what it called it before.
    expect(rows.get("held")).toMatchObject({ name: "held", alive: false });
    // An entry the line already let go is in the fold and stays off it.
    expect(rows.get("let-go")?.alive).toBe(false);
  });

  it("says nothing about a removal of what the line is not holding", async () => {
    answering({
      get_forge_line_states: [entry("let-go", false)],
      list_forge_pursuits_of_line: [],
    });
    await forgeCatalog.selectLine("L1");

    apiMock.mockResolvedValue(
      asking("P1", [
        { entry_id: "fresh", kind: "add", content_asset_id: "a9", name: "new" },
        { entry_id: "fresh", kind: "remove", content_asset_id: null, name: null },
        { entry_id: "let-go", kind: "remove", content_asset_id: null, name: null },
      ]),
    );
    await forgeCatalog.pursuit.load({ pursuitId: "P1" });

    const rows = new Map(forgeCatalog.projection.map((r) => [r.entryId, r]));
    // Added and taken off inside one piece of work: the fold leaves a
    // removal, the line was not holding it, and the close records
    // nothing — so it is not an entry the line has, in either state.
    // Two presses away, which is why it is pinned.
    expect(rows.has("fresh")).toBe(false);
    // The line's own entry stays exactly as the line has it. A
    // redundant removal is not a second letting-go, and it is not a
    // reason for the entry to leave the list either.
    expect(rows.get("let-go")).toMatchObject({ name: "let-go", alive: false });
  });

  it("folds the work before it meets the line", async () => {
    answering({
      get_forge_line_states: [entry("held", true)],
      list_forge_pursuits_of_line: [],
    });
    await forgeCatalog.selectLine("L1");

    apiMock.mockResolvedValue(
      asking("P1", [
        { entry_id: "held", kind: "remove", content_asset_id: null, name: null },
        { entry_id: "held", kind: "rename", content_asset_id: null, name: "late" },
        { entry_id: "held", kind: "replace", content_asset_id: "a9", name: null },
      ]),
    );
    await forgeCatalog.pursuit.load({ pursuitId: "P1" });

    // The winning existence decides what the row says, and it wins over
    // the whole work rather than over what came before it: an entry on
    // its way off states existence and nothing else, so neither the
    // rename nor the replace after the removal reaches the line. Both
    // verbs are on screen for a row this work has taken off, so both
    // are one press from being believed.
    expect(forgeCatalog.projection).toEqual([
      { entryId: "held", name: "held", assetId: "asset-held", alive: false },
    ]);
  });

  it("counts a clash against the line, not against the work's own ops", async () => {
    answering({
      get_forge_line_states: [entry("key-visual", true)],
      list_forge_pursuits_of_line: [],
    });
    await forgeCatalog.selectLine("L1");

    // The case that will actually happen: a name defaulted from a
    // filename meets one the line is already holding. Nothing in the
    // work says it twice, so counting the work's own ops would be
    // silent here — and the close would be refused.
    apiMock.mockResolvedValue(
      asking("P1", [
        {
          entry_id: "fresh",
          kind: "add",
          content_asset_id: "a9",
          name: "key-visual",
        },
      ]),
    );
    await forgeCatalog.pursuit.load({ pursuitId: "P1" });
    expect(forgeCatalog.wouldClash).toEqual(["key-visual"]);

    // And the case the other way: a name written twice by the work but
    // ending on one entry is not a clash, because the fold keeps the
    // last operation per entry and axis.
    apiMock.mockResolvedValue(
      asking("P1", [
        { entry_id: "fresh", kind: "add", content_asset_id: "a9", name: "x" },
        { entry_id: "fresh", kind: "rename", content_asset_id: null, name: "y" },
        { entry_id: "fresh", kind: "rename", content_asset_id: null, name: "x" },
      ]),
    );
    await forgeCatalog.pursuit.load({ pursuitId: "P1" });
    expect(forgeCatalog.wouldClash).toEqual([]);
  });

  it("names the anchor by ids and reads back what hangs off it", async () => {
    answering({ list_forge_threads_about: [] });
    await forgeCatalog.talkAbout({
      kind: "entry",
      about: "cut 04",
      pursuitId: "P1",
      nodeId: "r1",
      entryId: "e1",
    });

    // Every id the command takes, including the ones this kind has no
    // use for. Both directions are refused on the other side — a
    // `"round"` carrying an entry id would answer about the round for a
    // caller that asked about the entry — so the absent ones are said
    // rather than left out.
    expect(apiMock).toHaveBeenCalledWith("list_forge_threads_about", {
      anchorKind: "entry",
      pursuitId: "P1",
      lineId: null,
      nodeId: "r1",
      entryId: "e1",
      changePointId: null,
    });
  });

  it("lets go of a conversation when the work under it goes", async () => {
    answering({ list_forge_threads_about: [] });
    await forgeCatalog.talkAbout({
      kind: "pursuit",
      about: "this work",
      pursuitId: "P1",
    });

    forgeCatalog.clearWork();

    expect(forgeCatalog.talkingAbout).toBeNull();
    expect(forgeCatalog.threads.data).toEqual([]);
  });

  it("lets go of the work when the panel closes", async () => {
    mutateMock.mockResolvedValue(pursuit("P1"));
    answering({
      get_forge_pursuit: pursuit("P1"),
      list_forge_pursuits_of_line: [pursuit("P1")],
    });
    await forgeCatalog.openPursuit("L1", null, null);

    forgeCatalog.closePanel();

    expect(forgeCatalog.working).toBeNull();
    expect(forgeCatalog.pursuit.data).toBeNull();
    expect(forgeCatalog.pursuits.data).toEqual([]);
  });
});
