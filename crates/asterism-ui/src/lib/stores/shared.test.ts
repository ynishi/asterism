// sharedCatalog tests. The api / mutate choke points are mocked; the
// catalog's own machinery — its Resources, the alive filter and the
// command shapes — runs for real.
//
// What these pin is the panel's honesty about where its contents come
// from (#148 decision 16). A shared line is served through rather than
// mirrored, and every rule that follows from it is invisible in the
// markup, which is DOM and does not run here: that opening the panel
// reads rather than showing whatever it last had, that disconnecting
// empties the view rather than leaving the last answer on screen, and
// that an entry the line took off is not offered as something on the
// line.
//
// Which read follows which write is the same question asked of the
// verbs. A round is a request and a satisfied close is the one moment
// the line moves, so re-reading the contents after the first — or not
// after the second — is a screen telling somebody the line is
// somewhere it is not.
//
// And one word. Decision 11 says the UI has to say "re-enacted" when
// the chain was replayed, and a message that quietly stopped saying it
// would still be a passing publish.
//
// The sections below are where each of those lives, and this paragraph
// does not index them: it said which section held what twice, and both
// times a section landed that made the sentence false.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AssetDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeOpDto,
  ForgePursuitDto,
  ForgeRoundDto,
  PromotedAssetDto,
  TeamLedgerEventDto,
} from "../../bindings";
import { api } from "../api";
import { mutate } from "../mutate";
import { sharedCatalog } from "./shared.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));
vi.mock("../mutate", () => ({ mutate: vi.fn() }));

const apiMock = vi.mocked(api);
const mutateMock = vi.mocked(mutate);

function line(id: string, name: string): ForgeLineDto {
  return {
    id,
    name,
    strategy_id: "mainline-first",
    standing: "open",
    head_id: "h1",
    created_at_ms: 1,
    updated_at_ms: 1,
  };
}

function entry(id: string, name: string, alive: boolean): ForgeEntryStateDto {
  return { entry_id: id, alive, name, content_asset_id: "a1" };
}

function pursuit(
  id: string,
  rounds: ForgeRoundDto[] = [],
  close: ForgePursuitDto["close"] = null,
): ForgePursuitDto {
  return {
    id,
    line_id: "l1",
    parent_id: null,
    base_id: "b1",
    head_id: rounds.at(-1)?.id ?? "b1",
    title: "some work",
    note: null,
    opened_at_ms: 10,
    opened_by_kind: "member",
    opened_by_id: "u1",
    rounds,
    close,
  };
}

function round(id: string, ops: ForgeOpDto[]): ForgeRoundDto {
  return {
    id,
    parent_id: "b1",
    at_ms: 20,
    actor_kind: "member",
    actor_id: "u1",
    note: null,
    ops,
  };
}

beforeEach(() => {
  apiMock.mockReset();
  mutateMock.mockReset();
  sharedCatalog.session = null;
  sharedCatalog.teamId = "t1";
  sharedCatalog.selected = null;
  sharedCatalog.working = null;
  sharedCatalog.said = null;
  sharedCatalog.lines.reset();
  sharedCatalog.states.reset();
  sharedCatalog.history.reset();
  sharedCatalog.pursuits.reset();
  sharedCatalog.stored = null;
  sharedCatalog.storedRejected = false;
  sharedCatalog.deviceTokens.reset();
});

describe("what the panel shows", () => {
  it("offers only what is on the line", async () => {
    // An entry taken off the line is in the server's answer — the fold
    // reports every entry it has heard of — and it is not on the line.
    // Cloning it is refused by the client, so offering it would be a
    // button that cannot work.
    apiMock.mockResolvedValueOnce([
      entry("e1", "a.png", true),
      entry("e2", "gone.png", false),
    ]);
    await sharedCatalog.show("l1");

    expect(sharedCatalog.states.data).toHaveLength(2);
    expect(sharedCatalog.onTheLine.map((s) => s.entry_id)).toEqual(["e1"]);
  });

  it("reads a line through the server rather than from anything kept", async () => {
    apiMock.mockResolvedValue([entry("e1", "a.png", true)]);
    await sharedCatalog.show("l1");

    expect(apiMock).toHaveBeenCalledWith("shared_line_states", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
    expect(apiMock).toHaveBeenCalledWith("shared_line_history", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
  });

  it("reads when the panel opens rather than showing what it last had", async () => {
    // Decision 16 again: a served-through view that rendered its last
    // answer on open would be a mirror, and the staleness the decision
    // says does not exist would be back.
    sharedCatalog.session = "u1";
    apiMock.mockResolvedValueOnce("u1"); // refreshSession
    apiMock.mockResolvedValueOnce([line("l1", "shared")]);

    await sharedCatalog.openPanel();

    expect(sharedCatalog.open).toBe(true);
    expect(apiMock).toHaveBeenCalledWith("list_shared_lines", { teamIdRaw: "t1" });
  });

  it("does not ask a server it is not connected to", async () => {
    apiMock.mockResolvedValueOnce(null); // refreshSession answers nobody

    await sharedCatalog.openPanel();

    expect(sharedCatalog.session).toBeNull();
    expect(apiMock).not.toHaveBeenCalledWith("list_shared_lines", expect.anything());
  });

  it("counts a line's change points out of its history", async () => {
    // The visible difference between the two seedings.
    apiMock.mockResolvedValueOnce([entry("e1", "a.png", true)]);
    apiMock.mockResolvedValueOnce({
      line: line("l1", "shared"),
      genesis_id: "g1",
      genesis_at_ms: 1,
      changes: [
        { id: "c1", parent_id: "g1", from_pursuit_id: "p1", by_node_id: "n1", at_ms: 2, actor_kind: "user", actor_id: "a1", table: [] },
        { id: "c2", parent_id: "c1", from_pursuit_id: "p2", by_node_id: "n2", at_ms: 3, actor_kind: "user", actor_id: "a1", table: [] },
      ],
    });

    await sharedCatalog.show("l1");

    expect(sharedCatalog.changePoints).toBe(2);
  });

  it("empties when the connection goes, rather than going stale", async () => {
    apiMock.mockResolvedValueOnce([line("l1", "shared")]);
    await sharedCatalog.lines.load({ teamId: "t1" });
    apiMock.mockResolvedValue([entry("e1", "a.png", true)]);
    await sharedCatalog.show("l1");
    expect(sharedCatalog.lines.data).toHaveLength(1);

    apiMock.mockResolvedValueOnce(undefined);
    await sharedCatalog.disconnect();

    expect(sharedCatalog.session).toBeNull();
    expect(sharedCatalog.selected).toBeNull();
    expect(sharedCatalog.lines.data).toEqual([]);
    expect(sharedCatalog.states.data).toEqual([]);
  });
});

describe("the frame's three states", () => {
  // Worth pinning here rather than left to the markup for the reason
  // the catalog's header gives: the difference between "nobody has been
  // asked" and "there is nobody to ask" is not visible in a resource,
  // and a screen that merged them would report on a team it is not
  // talking to. Each of the three is what some surface renders instead
  // of a list.

  it("has nobody to ask before a connection", () => {
    expect(sharedCatalog.phase).toBe("disconnected");
  });

  it("has no team chosen once there is somebody to ask", () => {
    sharedCatalog.session = "u1";
    sharedCatalog.teamId = "";

    expect(sharedCatalog.phase).toBe("no-team");
  });

  it("is ready only once a team is named", () => {
    sharedCatalog.session = "u1";

    expect(sharedCatalog.phase).toBe("ready");
  });

  it("does not read a team's lines before one is chosen", async () => {
    // Connected is not the same as ready, and opening the panel in
    // between must not ask for the lines of the empty string.
    sharedCatalog.teamId = "";
    apiMock.mockResolvedValueOnce("u1"); // refreshSession

    await sharedCatalog.openPanel();

    expect(sharedCatalog.phase).toBe("no-team");
    expect(apiMock).not.toHaveBeenCalledWith(
      "list_shared_lines",
      expect.anything(),
    );
  });

  it("goes back to having nobody to ask when the connection drops", async () => {
    sharedCatalog.session = "u1";
    apiMock.mockResolvedValueOnce(undefined);

    await sharedCatalog.disconnect();

    expect(sharedCatalog.phase).toBe("disconnected");
  });

  it("keeps the team across a disconnect, so connecting again skips no-team", async () => {
    // Which is why the header calls this where a *window* begins
    // rather than where every session does. Dropping the id because a
    // connection dropped would make somebody type it twice.
    sharedCatalog.session = "u1";
    apiMock.mockResolvedValueOnce(undefined);
    await sharedCatalog.disconnect();

    sharedCatalog.session = "u1";

    expect(sharedCatalog.teamId).toBe("t1");
    expect(sharedCatalog.phase).toBe("ready");
  });
});

describe("the two writes", () => {
  it("clones the entry of the line that is open", async () => {
    apiMock.mockResolvedValue([entry("e1", "a.png", true)]);
    await sharedCatalog.show("l1");
    mutateMock.mockResolvedValueOnce({ id: "asset-1" } as AssetDto);

    await sharedCatalog.clone("e1", "persona-1");

    expect(mutateMock).toHaveBeenCalledWith(
      "clone_shared_entry",
      { teamIdRaw: "t1", lineId: "l1", entryId: "e1", personaId: "persona-1" },
      "clone that entry",
    );
  });

  it("refuses to clone when no line is open", async () => {
    await expect(sharedCatalog.clone("e1", "persona-1")).rejects.toThrow();
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("says the word when the chain was re-enacted", async () => {
    // #148 decision 11: the acts are restamped to the publisher, so
    // the team's line does not record who did the work upstream — and
    // the UI has to say that word rather than call it a history.
    mutateMock.mockResolvedValueOnce(line("l9", "the whole story"));
    apiMock.mockResolvedValueOnce([]);

    await sharedCatalog.publish("local-1", "the whole story", "mainline-first", true);

    expect(mutateMock).toHaveBeenCalledWith(
      "publish_line_to_team",
      {
        teamIdRaw: "t1",
        lineId: "local-1",
        name: "the whole story",
        strategyId: "mainline-first",
        reenact: true,
      },
      "publish that line to the team",
    );
    expect(sharedCatalog.said).toContain("re-enacted");
  });

  it("does not call it a re-enactment when it was not one", async () => {
    mutateMock.mockResolvedValueOnce(line("l9", "as it stands"));
    apiMock.mockResolvedValueOnce([]);

    await sharedCatalog.publish("local-1", "as it stands", "mainline-first", false);

    expect(sharedCatalog.said).not.toContain("re-enacted");
    expect(sharedCatalog.said).toContain("as it stands");
  });

  it("re-reads the team's lines after seeding one", async () => {
    mutateMock.mockResolvedValueOnce(line("l9", "seeded"));
    apiMock.mockResolvedValueOnce([line("l9", "seeded")]);

    await sharedCatalog.publish("local-1", "seeded", "mainline-first", false);

    expect(apiMock).toHaveBeenCalledWith("list_shared_lines", { teamIdRaw: "t1" });
    expect(sharedCatalog.lines.data.map((l) => l.id)).toEqual(["l9"]);
  });
});

describe("promoting an asset onto the open work", () => {
  // The act #66 exists for, and the one entry point content has (#148
  // decision 5). What these pin is that the verb names the three ids
  // the catalog holds rather than any a screen passed, and that what
  // follows a promotion is a re-read of the work and of nothing else:
  // a round is a request, so the line has not moved.

  function promoted(over: Partial<PromotedAssetDto> = {}): PromotedAssetDto {
    return {
      entry_id: "e9",
      team_asset_id: "ta9",
      digest: "sha256:abc",
      bytes_already_held: false,
      already_promoted: false,
      ...over,
    };
  }

  /// A line showing, with one open pursuit selected on it.
  ///
  /// Selected out of the list rather than assigned, because that is
  /// how a screen gets there and because the verb asks the list what
  /// the selected work is: a pursuit id set beside an empty list is a
  /// state the catalog reads as "not in this line's list".
  async function withOpenWork(): Promise<void> {
    apiMock.mockResolvedValueOnce([]); // states
    apiMock.mockResolvedValueOnce(null); // history
    apiMock.mockResolvedValueOnce([pursuit("p1")]); // pursuits
    await sharedCatalog.show("l1");
    sharedCatalog.selectPursuit("p1");
  }

  it("names the team, the line and the work the catalog has open", async () => {
    await withOpenWork();
    mutateMock.mockResolvedValueOnce(promoted());
    apiMock.mockReset();
    apiMock.mockResolvedValueOnce([]);

    await sharedCatalog.promote("asset-1", "cut-01");

    expect(mutateMock).toHaveBeenCalledWith(
      "promote_asset_to_team",
      {
        teamIdRaw: "t1",
        lineId: "l1",
        pursuitId: "p1",
        assetId: "asset-1",
        named: "cut-01",
      },
      "promote that asset to the team",
    );
  });

  it("re-reads the work and not what the line holds", async () => {
    // Nothing has landed. Re-reading the contents would be asking a
    // question whose answer cannot have changed, and drawing the
    // answer as if it had is how a screen says a line moved when it
    // did not.
    await withOpenWork();
    mutateMock.mockResolvedValueOnce(promoted());
    apiMock.mockReset();
    apiMock.mockResolvedValueOnce([]);

    await sharedCatalog.promote("asset-1", "cut-01");

    expect(apiMock).toHaveBeenCalledWith("shared_line_pursuits", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
    expect(apiMock).not.toHaveBeenCalledWith(
      "shared_line_states",
      expect.anything(),
    );
  });

  it("refuses when no work is open", async () => {
    // Decision 5 again: content enters against open work, so there is
    // nothing to promote to.
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");

    await expect(sharedCatalog.promote("asset-1", "cut-01")).rejects.toThrow(
      /no work is open/,
    );
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("refuses when no line is open", async () => {
    sharedCatalog.working = "p1";

    await expect(sharedCatalog.promote("asset-1", "cut-01")).rejects.toThrow(
      /no line is open/,
    );
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("says a repeat sent nothing, and does not call it a landing", async () => {
    // A repeat is answered from this machine, before anything is
    // uploaded. The sentence has to say that rather than read like a
    // second promotion that worked.
    await withOpenWork();
    mutateMock.mockResolvedValueOnce(
      promoted({
        team_asset_id: null,
        bytes_already_held: null,
        already_promoted: true,
      }),
    );

    await sharedCatalog.promote("asset-1", "cut-01");

    expect(sharedCatalog.said).toContain("nothing was sent");
    // The clause only the sent path has. Asserting the absence of "on
    // the line" passed on a technicality — the repeat sentence says
    // "onto this line", which is a different string and the same
    // subject.
    expect(sharedCatalog.said).not.toContain("closing it as satisfied");
  });

  it("refuses when the work being read has ended", async () => {
    // Reading ended work is ordinary, so `working` is set for one as
    // readily as for an open one. Nothing may enter against it — and
    // the refusal belongs here rather than at the server, because
    // `enter_content` streams the body before it asks.
    apiMock.mockResolvedValueOnce([]); // states
    apiMock.mockResolvedValueOnce(null); // history
    apiMock.mockResolvedValueOnce([
      pursuit("p1", [], {
        id: "c1",
        parent_id: "b1",
        outcome: "satisfied",
        note: null,
        at_ms: 30,
        actor_kind: "member",
        actor_id: "u1",
      }),
    ]);
    await sharedCatalog.show("l1");
    sharedCatalog.selectPursuit("p1");

    // Which refusal, not merely that one happened: the guard has two,
    // and the other one ("not in this line's list") would pass a bare
    // `toThrow` while saying something else entirely.
    await expect(sharedCatalog.promote("asset-1", "cut-01")).rejects.toThrow(
      /has ended/,
    );
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("refuses when the selected work is not in the line's list", async () => {
    // What a failed re-read leaves: `Resource` puts its data back to
    // the initial, so the pursuit a screen selected is no longer
    // there. Not the same fact as ended work, and not the same
    // sentence.
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";

    await expect(sharedCatalog.promote("asset-1", "cut-01")).rejects.toThrow(
      /not in this line's list/,
    );
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("says what a promotion is and is not, when one was sent", async () => {
    // On the work, not on the line: the entry reaches the line when
    // the work closes satisfied.
    await withOpenWork();
    mutateMock.mockResolvedValueOnce(promoted());

    await sharedCatalog.promote("asset-1", "cut-01");

    expect(sharedCatalog.said).toContain("on the work");
    expect(sharedCatalog.said).toContain("satisfied");
  });
});

describe("working a shared line", () => {
  // The four verbs #198 wires, and what each of them is allowed to
  // assume. Two rules run through all of them: a pursuit here belongs
  // to a line somebody else may also be working, so the list is
  // re-read rather than patched from what a write answered; and
  // nothing reaches the line until a satisfied close, so the contents
  // are re-read after that one and after no other.

  it("reads the work against a line beside its contents and its chain", async () => {
    // All three arrive with the line, which is what lets the inner
    // tabs be a choice of what to draw rather than three more reads.
    apiMock.mockResolvedValue([]);

    await sharedCatalog.show("l1");

    expect(apiMock).toHaveBeenCalledWith("shared_line_pursuits", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
  });

  it("opens work against the line that is showing, and leaves it open", async () => {
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    mutateMock.mockResolvedValueOnce(pursuit("p1"));
    apiMock.mockResolvedValueOnce([pursuit("p1")]);

    await sharedCatalog.openPursuit("a name", "");

    expect(mutateMock).toHaveBeenCalledWith(
      "open_shared_pursuit",
      {
        teamIdRaw: "t1",
        lineId: "l1",
        title: "a name",
        // Empty is nothing said, not an empty note.
        note: null,
      },
      "open work against that line",
    );
    expect(sharedCatalog.working).toBe("p1");
    expect(sharedCatalog.work?.id).toBe("p1");
  });

  it("refuses to open work when no line is open", async () => {
    await expect(sharedCatalog.openPursuit("t", "")).rejects.toThrow();
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("pushes a round into the open work and re-reads the work list", async () => {
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    mutateMock.mockResolvedValueOnce(pursuit("p1"));
    apiMock.mockResolvedValueOnce([pursuit("p1")]);
    await sharedCatalog.openPursuit("", "");
    apiMock.mockReset();
    apiMock.mockResolvedValueOnce([pursuit("p1")]);

    const op: ForgeOpDto = {
      entry_id: "e1",
      kind: "rename",
      content_asset_id: null,
      name: "cut-02",
    };
    await sharedCatalog.pushRound([op], "");

    expect(mutateMock).toHaveBeenLastCalledWith(
      "push_shared_round",
      { teamIdRaw: "t1", pursuitId: "p1", ops: [op], note: null },
      "push that round",
    );
    // A round is a request. Nothing has reached the line, so re-reading
    // what the line holds would be asking a question whose answer
    // cannot have changed.
    expect(apiMock).not.toHaveBeenCalledWith(
      "shared_line_states",
      expect.anything(),
    );
    expect(apiMock).toHaveBeenCalledWith("shared_line_pursuits", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
  });

  it("refuses to push a round when no work is open", async () => {
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");

    await expect(sharedCatalog.pushRound([], "")).rejects.toThrow();
    expect(mutateMock).not.toHaveBeenCalled();
  });

  it("re-reads what the line holds when work closes satisfied", async () => {
    // The one moment the line moves.
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";
    mutateMock.mockResolvedValueOnce(pursuit("p1"));
    apiMock.mockReset();
    apiMock.mockResolvedValue([]);

    await sharedCatalog.closePursuit("satisfied", "done");

    expect(mutateMock).toHaveBeenCalledWith(
      "close_shared_pursuit",
      { teamIdRaw: "t1", pursuitId: "p1", outcome: "satisfied", note: "done" },
      "close that work",
    );
    expect(apiMock).toHaveBeenCalledWith("shared_line_states", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
    expect(apiMock).toHaveBeenCalledWith("shared_line_history", {
      teamIdRaw: "t1",
      lineId: "l1",
    });
    expect(sharedCatalog.working).toBeNull();
    expect(sharedCatalog.said).toContain("on the line");
  });

  it("says the line did not move when work is abandoned", async () => {
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";
    mutateMock.mockResolvedValueOnce(pursuit("p1"));

    await sharedCatalog.closePursuit("abandoned", "");

    expect(sharedCatalog.said).toContain("did not move");
    expect(sharedCatalog.said).not.toContain("on the line");
  });

  it("ends the work under a line when another line is opened", async () => {
    // A piece of work belongs to the line it is against, so it cannot
    // survive the line going.
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";

    await sharedCatalog.show("l2");

    expect(sharedCatalog.working).toBeNull();
  });

  it("ends the work under a line when the line is let go", async () => {
    // The pairing is the catalog's rather than the panel's, on the
    // rule `lookAt` follows: a screen writing both fields is a second
    // place to remember that one belongs to the other.
    apiMock.mockResolvedValue([]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";

    sharedCatalog.closeLine();

    expect(sharedCatalog.selected).toBeNull();
    expect(sharedCatalog.working).toBeNull();
  });

  it("drops the work when the connection goes, rather than going stale", async () => {
    apiMock.mockResolvedValue([pursuit("p1")]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";
    apiMock.mockResolvedValueOnce(undefined);

    await sharedCatalog.disconnect();

    expect(sharedCatalog.working).toBeNull();
    expect(sharedCatalog.pursuits.data).toEqual([]);
    expect(sharedCatalog.work).toBeNull();
  });

  it("drops the work when another team is named", async () => {
    apiMock.mockResolvedValue([pursuit("p1")]);
    await sharedCatalog.show("l1");
    sharedCatalog.working = "p1";
    apiMock.mockResolvedValueOnce([]); // lines for the new team

    await sharedCatalog.lookAt("t2");

    expect(sharedCatalog.working).toBeNull();
    expect(sharedCatalog.pursuits.data).toEqual([]);
  });

  it("keeps open and ended work apart", async () => {
    apiMock.mockResolvedValueOnce([]); // states
    apiMock.mockResolvedValueOnce(null); // history
    apiMock.mockResolvedValueOnce([
      pursuit("p1"),
      pursuit("p2", [], {
        id: "c1",
        parent_id: "b1",
        outcome: "satisfied",
        note: null,
        at_ms: 30,
        actor_kind: "member",
        actor_id: "u1",
      }),
    ]);
    await sharedCatalog.show("l1");

    expect(sharedCatalog.openWork.map((w) => w.id)).toEqual(["p1"]);
    expect(sharedCatalog.endedWork.map((w) => w.id)).toEqual(["p2"]);
  });

  it("folds the open work over the line, so a rename shows before it lands", async () => {
    // The same fold the local plane uses (`lib/forge-projection`), over
    // this plane's two reads. What it buys is the reason the fold
    // exists at all: the states still say the old name, because nothing
    // has landed, and a person deciding whether to close has to see
    // what closing would leave.
    apiMock.mockResolvedValueOnce([entry("e1", "cut-01", true)]); // states
    apiMock.mockResolvedValueOnce(null); // history
    apiMock.mockResolvedValueOnce([
      pursuit("p1", [
        round("r1", [
          {
            entry_id: "e1",
            kind: "rename",
            content_asset_id: null,
            name: "cut-02",
          },
        ]),
      ]),
    ]);
    await sharedCatalog.show("l1");
    sharedCatalog.selectPursuit("p1");

    expect(sharedCatalog.states.data[0].name).toBe("cut-01");
    expect(sharedCatalog.projection).toEqual([
      {
        entryId: "e1",
        name: "cut-02",
        assetId: "a1",
        alive: true,
        stated: null,
      },
    ]);
  });

  it("says nothing of a fold when no work is open", async () => {
    // `working` is its own state rather than derived from the list, so
    // an unopened pursuit's rounds are not folded onto the line behind
    // somebody's back.
    apiMock.mockResolvedValueOnce([entry("e1", "cut-01", true)]);
    apiMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce([
      pursuit("p1", [
        round("r1", [
          {
            entry_id: "e1",
            kind: "remove",
            content_asset_id: null,
            name: null,
          },
        ]),
      ]),
    ]);
    await sharedCatalog.show("l1");

    expect(sharedCatalog.work).toBeNull();
    expect(sharedCatalog.projection.map((row) => row.alive)).toEqual([true]);
  });
});

describe("walking the ledger", () => {
  // What these pin is that the walk is a walk. Every one of them is a
  // way the same read stops being one: a page that replaces instead of
  // extends, a resume that starts over, a walk that survives the team
  // it belongs to.

  function event(seq: number): TeamLedgerEventDto {
    return {
      seq,
      event_id: `e${seq}`,
      team_id: "t1",
      actor_kind: "member",
      actor_user_id: "u1",
      actor_display_name: "someone",
      occurred_at_ms: 1000 + seq,
      kind: "teams.team.created/1",
      subjects: [],
      payload_json: "{}",
    };
  }

  beforeEach(() => {
    sharedCatalog.forgetLedger();
  });

  it("asks for the first page with no cursor", async () => {
    apiMock.mockResolvedValueOnce({ events: [event(1)], next_after: null });

    await sharedCatalog.readLedgerPage();

    expect(apiMock).toHaveBeenCalledWith("team_ledger_page", {
      teamIdRaw: "t1",
      after: null,
      limit: 50,
    });
    expect(sharedCatalog.ledger.map((e) => e.seq)).toEqual([1]);
  });

  it("appends the next page rather than replacing what it has", async () => {
    apiMock.mockResolvedValueOnce({ events: [event(1), event(2)], next_after: 2 });
    await sharedCatalog.readLedgerPage();
    apiMock.mockResolvedValueOnce({ events: [event(3)], next_after: null });

    await sharedCatalog.readLedgerPage();

    expect(apiMock).toHaveBeenLastCalledWith("team_ledger_page", {
      teamIdRaw: "t1",
      after: 2,
      limit: 50,
    });
    expect(sharedCatalog.ledger.map((e) => e.seq)).toEqual([1, 2, 3]);
  });

  it("resumes from the last seq it saw when the cursor is null", async () => {
    // A null cursor says nothing lay past there *when the page was
    // taken*, not that the walk is over — so asking again has to ask
    // about what comes after the last event, not about the beginning.
    // Passing nothing would append a second copy of the whole ledger.
    apiMock.mockResolvedValueOnce({ events: [event(1), event(2)], next_after: null });
    await sharedCatalog.readLedgerPage();
    apiMock.mockResolvedValueOnce({ events: [], next_after: null });

    await sharedCatalog.readLedgerPage();

    expect(apiMock).toHaveBeenLastCalledWith("team_ledger_page", {
      teamIdRaw: "t1",
      after: 2,
      limit: 50,
    });
    expect(sharedCatalog.ledger.map((e) => e.seq)).toEqual([1, 2]);
  });

  it("drops the walk when another team is named", async () => {
    apiMock.mockResolvedValueOnce({ events: [event(1)], next_after: 1 });
    await sharedCatalog.readLedgerPage();
    apiMock.mockResolvedValueOnce([]); // lines for the new team

    await sharedCatalog.lookAt("t2");

    expect(sharedCatalog.ledger).toEqual([]);
    expect(sharedCatalog.ledgerCursor).toBeNull();
    expect(sharedCatalog.ledgerRead).toBe(false);
  });

  it("drops the walk when the panel is opened again", async () => {
    // The same decision the lines are held to: opening reads rather
    // than showing what it last had. A ledger is the one read here
    // that grows while nobody is looking, so a kept walk would be the
    // staleness decision 16 says does not exist.
    apiMock.mockResolvedValueOnce({ events: [event(1)], next_after: 1 });
    await sharedCatalog.readLedgerPage();
    sharedCatalog.session = "u1";
    apiMock.mockResolvedValueOnce("u1"); // refreshSession
    apiMock.mockResolvedValueOnce([]); // lines

    await sharedCatalog.openPanel();

    expect(sharedCatalog.ledger).toEqual([]);
    expect(sharedCatalog.ledgerRead).toBe(false);
  });

  it("drops the walk when the connection goes", async () => {
    apiMock.mockResolvedValueOnce({ events: [event(1)], next_after: 1 });
    await sharedCatalog.readLedgerPage();
    apiMock.mockResolvedValueOnce(undefined);

    await sharedCatalog.disconnect();

    expect(sharedCatalog.ledger).toEqual([]);
    expect(sharedCatalog.ledgerRead).toBe(false);
  });

  it("keeps what it has when a page fails", async () => {
    apiMock.mockResolvedValueOnce({ events: [event(1)], next_after: 1 });
    await sharedCatalog.readLedgerPage();
    apiMock.mockRejectedValueOnce(new Error("the server said no"));

    await sharedCatalog.readLedgerPage();

    expect(sharedCatalog.ledgerError).toContain("the server said no");
    expect(sharedCatalog.ledger.map((e) => e.seq)).toEqual([1]);
  });
});

describe("the teams to pick from", () => {
  // The read the typed team id was waiting for. What these pin is that
  // it belongs to the connection rather than to a team — it is read
  // when there is somebody to ask and dropped when there is not — and
  // that an empty answer is not the same statement as "no way in".

  beforeEach(() => {
    sharedCatalog.teams.reset();
  });

  it("reads them when the panel opens on a connection", async () => {
    sharedCatalog.session = "u1";
    apiMock.mockResolvedValueOnce("u1"); // refreshSession
    apiMock.mockResolvedValueOnce({
      teams: [
        { team_id: "t1", role: "owner", created_at_ms: 1 },
        { team_id: "t2", role: "member", created_at_ms: 2 },
      ],
    });
    apiMock.mockResolvedValueOnce([]); // lines, since a team is named

    await sharedCatalog.openPanel();

    expect(apiMock).toHaveBeenCalledWith("my_teams");
    expect(sharedCatalog.teams.data.map((one) => one.team_id)).toEqual([
      "t1",
      "t2",
    ]);
  });

  it("does not ask a server it is not connected to", async () => {
    // The list is about the account, so with nobody to ask there is
    // nothing to answer — and asking would be the panel reporting on a
    // connection it does not have.
    apiMock.mockResolvedValueOnce(null); // refreshSession answers nobody

    await sharedCatalog.openPanel();

    expect(apiMock).not.toHaveBeenCalledWith("my_teams");
  });

  it("reads them on connecting, which is the phase they are for", async () => {
    mutateMock.mockResolvedValueOnce("u1");
    // What the connection stored, read back before the teams are:
    // a login that did not ask to be remembered stores nothing, and
    // this is the read that says which (#204).
    apiMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce({
      teams: [{ team_id: "t1", role: "owner", created_at_ms: 1 }],
    });

    await sharedCatalog.connect("http://x", "who", "secret", false);

    expect(apiMock).toHaveBeenCalledWith("my_teams");
    expect(sharedCatalog.teams.data).toHaveLength(1);
  });

  it("reads them again after founding one", async () => {
    // The list is one shorter than the truth until it does, and the
    // person who just founded a team is the likeliest to pick it.
    mutateMock.mockResolvedValueOnce({ team_id: "t9" });
    apiMock.mockResolvedValueOnce({
      teams: [{ team_id: "t9", role: "owner", created_at_ms: 3 }],
    });

    await sharedCatalog.createTeam();

    expect(sharedCatalog.teams.data.map((one) => one.team_id)).toEqual(["t9"]);
  });

  it("drops them when the connection goes", async () => {
    apiMock.mockResolvedValueOnce({
      teams: [{ team_id: "t1", role: "owner", created_at_ms: 1 }],
    });
    await sharedCatalog.teams.load({});
    apiMock.mockResolvedValueOnce(undefined);

    await sharedCatalog.disconnect();

    expect(sharedCatalog.teams.data).toEqual([]);
  });

  it("keeps them when another team is named", async () => {
    // Naming a team says nothing about which teams the account is in.
    // `lookAt` drops what belonged to the team named before; this
    // belongs to the connection.
    apiMock.mockResolvedValueOnce({
      teams: [{ team_id: "t1", role: "owner", created_at_ms: 1 }],
    });
    await sharedCatalog.teams.load({});
    apiMock.mockResolvedValueOnce([]); // lines for the new team

    await sharedCatalog.lookAt("t2");

    expect(sharedCatalog.teams.data).toHaveLength(1);
  });
});

describe("the roster", () => {
  beforeEach(() => {
    sharedCatalog.roster.reset();
  });

  it("reads the members of the team now named", async () => {
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [
        { user_id: "u1", role: "owner" },
        { user_id: "u2", role: "member" },
      ],
      viewer: { role: "owner", admin: false },
    });

    await sharedCatalog.roster.load({ teamId: "t1" });

    expect(apiMock).toHaveBeenCalledWith("team_roster", { teamIdRaw: "t1" });
    expect(sharedCatalog.roster.data?.members.map((m) => m.role)).toEqual([
      "owner",
      "member",
    ]);
  });

  it("drops it when another team is named", async () => {
    // A roster belongs to one team, on the same rule as the walk: what
    // naming a team ends is `lookAt`'s, written once.
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [{ user_id: "u1", role: "owner" }],
      viewer: { role: "owner", admin: false },
    });
    await sharedCatalog.roster.load({ teamId: "t1" });
    apiMock.mockResolvedValueOnce([]); // lines for the new team

    await sharedCatalog.lookAt("t2");

    expect(sharedCatalog.roster.data).toBeNull();
  });

  it("drops it when the connection goes", async () => {
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [{ user_id: "u1", role: "owner" }],
      viewer: { role: "owner", admin: false },
    });
    await sharedCatalog.roster.load({ teamId: "t1" });
    apiMock.mockResolvedValueOnce(undefined);

    await sharedCatalog.disconnect();

    expect(sharedCatalog.roster.data).toBeNull();
  });

  it("answers a new team with its id, so a caller can name it", async () => {
    mutateMock.mockResolvedValueOnce({ team_id: "t9" });

    const made = await sharedCatalog.createTeam();

    expect(mutateMock).toHaveBeenCalledWith("create_team", {}, "create a team");
    expect(made).toBe("t9");
    expect(sharedCatalog.said).toContain("t9");
  });

  it("takes the reader's own standing from what the read said", async () => {
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [
        { user_id: "u1", role: "owner" },
        { user_id: "u2", role: "member" },
      ],
      viewer: { role: "owner", admin: false },
    });

    await sharedCatalog.roster.load({ teamId: "t1" });

    expect(sharedCatalog.myRole).toBe("owner");
    expect(sharedCatalog.iAmAdmin).toBe(false);
  });

  it("says an admin holds no role and is an admin anyway", async () => {
    // The case the rows cannot answer. An instance admin reaches a
    // team by standing outside it, so no membership row describes
    // them; derived from the rows this would read as "nothing you may
    // do", and what they may do is delete the team.
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [{ user_id: "u1", role: "owner" }],
      viewer: { role: null, admin: true },
    });

    await sharedCatalog.roster.load({ teamId: "t1" });

    expect(sharedCatalog.myRole).toBeNull();
    expect(sharedCatalog.iAmAdmin).toBe(true);
  });

  it("re-reads the roster after letting somebody in", async () => {
    sharedCatalog.teamId = "t1";
    mutateMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [
        { user_id: "u1", role: "owner" },
        { user_id: "u2", role: "member" },
      ],
      viewer: { role: "owner", admin: false },
    });

    await sharedCatalog.inviteMember("u2", "member");

    expect(mutateMock).toHaveBeenCalledWith(
      "invite_team_member",
      { teamIdRaw: "t1", userId: "u2", role: "member" },
      "invite that account",
    );
    expect(sharedCatalog.roster.data?.members).toHaveLength(2);
  });

  it("names each of the three member-shaped writes to its own command", async () => {
    sharedCatalog.teamId = "t1";
    const rows = {
      team_id: "t1",
      members: [{ user_id: "u1", role: "owner" }],
      viewer: { role: "owner", admin: false },
    };

    mutateMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce(rows);
    await sharedCatalog.removeMember("u2");
    expect(mutateMock).toHaveBeenLastCalledWith(
      "remove_team_member",
      { teamIdRaw: "t1", userId: "u2" },
      "remove that member",
    );

    mutateMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce(rows);
    await sharedCatalog.grantOwner("u2");
    expect(mutateMock).toHaveBeenLastCalledWith(
      "grant_team_owner",
      { teamIdRaw: "t1", userId: "u2" },
      "make that member an owner",
    );

    mutateMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce(rows);
    await sharedCatalog.revokeOwner("u2");
    expect(mutateMock).toHaveBeenLastCalledWith(
      "revoke_team_owner",
      { teamIdRaw: "t1", userId: "u2" },
      "take back the owner role",
    );
  });

  it("stops looking at a team it deleted", async () => {
    sharedCatalog.teamId = "t1";
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [{ user_id: "u1", role: "owner" }],
      viewer: { role: "owner", admin: false },
    });
    await sharedCatalog.roster.load({ teamId: "t1" });
    mutateMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce([]); // the teams list, one shorter now

    await sharedCatalog.deleteTeam();

    expect(mutateMock).toHaveBeenCalledWith(
      "delete_team",
      { teamIdRaw: "t1" },
      "delete the team",
    );
    expect(sharedCatalog.teamId).toBe("");
    expect(sharedCatalog.roster.data).toBeNull();
  });

  it("stops looking at a team the reader took themself out of", async () => {
    // Removing yourself ends the relationship the panel was drawing.
    // The re-read every other removal performs would be refused by the
    // gate this reader no longer passes, and `Resource.load` turns
    // that into an error message under a panel still pointed at the
    // team — so this takes the path deleting one takes.
    sharedCatalog.teamId = "t1";
    sharedCatalog.session = "u1";
    apiMock.mockResolvedValueOnce({
      team_id: "t1",
      members: [
        { user_id: "u1", role: "owner" },
        { user_id: "u2", role: "owner" },
      ],
      viewer: { role: "owner", admin: false },
    });
    await sharedCatalog.roster.load({ teamId: "t1" });
    mutateMock.mockResolvedValueOnce(null);
    apiMock.mockResolvedValueOnce([]); // the teams list, one shorter now

    await sharedCatalog.removeMember("u1");

    expect(sharedCatalog.teamId).toBe("");
    expect(sharedCatalog.roster.data).toBeNull();
    expect(sharedCatalog.said).toContain("no longer in");
  });
});

describe("the connection this machine remembers", () => {
  // #204. What these pin is the one thing the DOM cannot say: a
  // reconnect nobody asked for is attempted, and none of the ways it
  // can end is reported as a failure — because a window whose stored
  // credential was refused is a window showing the form it would have
  // shown anyway.
  //
  // The credential itself never appears here, and that is the point:
  // it lives in the OS keychain, the commands are the only thing that
  // touches it, and every value in these tests is one a file may hold.

  const stored = {
    base_url: "http://x",
    login: "who",
    token_id: "dt1",
    label: "Asterism on macos",
  };

  it("tries what it remembers when the panel opens on no session", async () => {
    apiMock.mockResolvedValueOnce(null); // refreshSession answers nobody
    apiMock.mockResolvedValueOnce(stored);
    apiMock.mockResolvedValueOnce({ outcome: "connected", user: "u1" });
    apiMock.mockResolvedValueOnce({ teams: [] });

    await sharedCatalog.openPanel();

    expect(apiMock).toHaveBeenCalledWith("connect_team_server_stored");
    expect(sharedCatalog.session).toBe("u1");
    expect(sharedCatalog.storedRejected).toBe(false);
  });

  it("does not try one on a window already connected", async () => {
    // The session a window has is the one it keeps: a second login
    // would replace it with an identical one for no reason.
    apiMock.mockResolvedValueOnce("u1"); // refreshSession
    apiMock.mockResolvedValueOnce({ teams: [] });

    await sharedCatalog.openPanel();

    expect(apiMock).not.toHaveBeenCalledWith("connect_team_server_stored");
  });

  it("says so when what it remembered was refused", async () => {
    // The one outcome worth a sentence. `stored` survives it, because
    // the login it carries is what the person is about to type a
    // password beside.
    apiMock.mockResolvedValueOnce(null); // refreshSession
    apiMock.mockResolvedValueOnce(stored);
    apiMock.mockResolvedValueOnce({ outcome: "rejected", user: null });

    await sharedCatalog.openPanel();

    expect(sharedCatalog.session).toBeNull();
    expect(sharedCatalog.storedRejected).toBe(true);
    expect(sharedCatalog.stored).toEqual(stored);
  });

  it("says nothing when there was nothing to try", async () => {
    apiMock.mockResolvedValueOnce(null); // refreshSession
    apiMock.mockResolvedValueOnce(null); // nothing stored

    await sharedCatalog.openPanel();

    expect(apiMock).not.toHaveBeenCalledWith("connect_team_server_stored");
    expect(sharedCatalog.storedRejected).toBe(false);
    expect(sharedCatalog.stored).toBeNull();
  });

  it("keeps what it remembers when the attempt kept the file", async () => {
    // `nothing` is two ends wearing one name: the file was dropped
    // because its keychain entry was gone, or it was kept because the
    // keychain would not answer — a locked store, a prompt somebody
    // dismissed. The command says which by what it left on disk and
    // nothing else, so this reads it back. Assuming it was dropped is
    // how one dismissed prompt empties a form of a connection this
    // machine still has.
    apiMock.mockResolvedValueOnce(null); // refreshSession
    apiMock.mockResolvedValueOnce(stored);
    apiMock.mockResolvedValueOnce({ outcome: "nothing", user: null });
    apiMock.mockResolvedValueOnce(stored); // the file survived

    await sharedCatalog.openPanel();

    expect(sharedCatalog.session).toBeNull();
    expect(sharedCatalog.storedRejected).toBe(false);
    expect(sharedCatalog.stored).toEqual(stored);
  });

  it("lets go of what the attempt dropped", async () => {
    // The other end of `nothing`: the keychain answered that the entry
    // is gone, so the command dropped the file it was the sole index
    // of, and the form has nothing left to pre-fill from.
    apiMock.mockResolvedValueOnce(null); // refreshSession
    apiMock.mockResolvedValueOnce(stored);
    apiMock.mockResolvedValueOnce({ outcome: "nothing", user: null });
    apiMock.mockResolvedValueOnce(null); // the file went with the entry

    await sharedCatalog.openPanel();

    expect(sharedCatalog.session).toBeNull();
    expect(sharedCatalog.storedRejected).toBe(false);
    expect(sharedCatalog.stored).toBeNull();
  });

  it("keeps quiet when the server could not be reached", async () => {
    // Unreachable is a fact about the attempt rather than about the
    // credential, so nothing is forgotten and nothing is reported: the
    // window lands on the form, which is where it started.
    apiMock.mockResolvedValueOnce(null); // refreshSession
    apiMock.mockResolvedValueOnce(stored);
    apiMock.mockRejectedValueOnce(new Error("connection refused"));

    await expect(sharedCatalog.openPanel()).resolves.toBeUndefined();

    expect(sharedCatalog.session).toBeNull();
    expect(sharedCatalog.storedRejected).toBe(false);
  });

  it("carries the choice to be remembered into the login", async () => {
    mutateMock.mockResolvedValueOnce("u1");
    apiMock.mockResolvedValueOnce(stored);
    apiMock.mockResolvedValueOnce({ teams: [] });

    await sharedCatalog.connect("http://x", "who", "secret", true);

    expect(mutateMock).toHaveBeenCalledWith(
      "connect_team_server",
      { baseUrl: "http://x", login: "who", password: "secret", remember: true },
      "connect to that team server",
    );
    expect(sharedCatalog.stored).toEqual(stored);
  });

  it("gives up being remembered when the connection is ended", async () => {
    sharedCatalog.stored = stored;
    sharedCatalog.storedRejected = true;
    apiMock.mockResolvedValueOnce(undefined);

    await sharedCatalog.disconnect();

    expect(apiMock).toHaveBeenCalledWith("disconnect_team_server");
    expect(sharedCatalog.stored).toBeNull();
    expect(sharedCatalog.storedRejected).toBe(false);
  });

  it("re-reads both sides after a revoke", async () => {
    // Revoking may have been this machine's own row, and only the
    // command knows: the listing is asked again rather than patched,
    // and so is what this machine still holds.
    sharedCatalog.stored = stored;
    mutateMock.mockResolvedValueOnce(undefined);
    apiMock.mockResolvedValueOnce(null); // nothing stored any more
    apiMock.mockResolvedValueOnce({ tokens: [] });

    await sharedCatalog.revokeDevice("dt1");

    expect(mutateMock).toHaveBeenCalledWith(
      "revoke_team_device_token",
      { tokenId: "dt1" },
      "revoke that device",
    );
    expect(sharedCatalog.stored).toBeNull();
    expect(sharedCatalog.deviceTokens.data).toEqual([]);
  });
});
