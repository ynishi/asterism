// sharedCatalog tests. The api / mutate choke points are mocked; the
// catalog's own machinery — the two Resources, the alive filter and
// the command shapes — runs for real.
//
// What these pin is the panel's honesty about where its contents come
// from (#148 decision 16). A shared line is served through rather than
// mirrored, so three of the rules worth a test are that opening the
// panel reads rather than showing whatever it last had, that
// disconnecting empties the view rather than leaving the last answer on
// screen, and that an entry the line took off is not offered as
// something on the line. All three would be invisible in the markup,
// which is DOM and does not run here.
//
// The last is the word. Decision 11 says the UI has to say
// "re-enacted" when the chain was replayed, and a message that quietly
// stopped saying it would still be a passing publish.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssetDto, ForgeEntryStateDto, ForgeLineDto } from "../../bindings";
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

beforeEach(() => {
  apiMock.mockReset();
  mutateMock.mockReset();
  sharedCatalog.session = null;
  sharedCatalog.teamId = "t1";
  sharedCatalog.selected = null;
  sharedCatalog.said = null;
  sharedCatalog.lines.reset();
  sharedCatalog.states.reset();
  sharedCatalog.history.reset();
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
