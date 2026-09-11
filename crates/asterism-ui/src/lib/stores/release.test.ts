// releaseCatalog tests. The `api` / `mutate` choke points are mocked;
// the catalog's own machinery — its Resources, what each write
// invalidates, and when it stops watching a run — runs for real.
//
// Same shape `forge.test.ts` uses and for its reason: what these pin is
// not visible in the markup, which is DOM and does not run here.
//
// Three rules are worth the test.
//
// **A write invalidates the reads it moved.** Writing a change point out
// makes the point's release count wrong and opens a drawer on a record
// that has none of its files yet; sending makes the send list wrong.
// Both re-read rather than splicing the answer in, so `Resource`'s
// generation guard stays the only thing deciding what is on screen.
//
// **A terminal dispatch is read once and not polled.** The runner parks
// a row and nothing moves it afterwards, so a poll over one is a timer
// that will tick sixty times to learn nothing. This is the half a
// reader of the store cannot check by eye.
//
// **A count belongs to the line it was read on.** A change point id
// belongs to one chain, so nothing remembered about one line may answer
// for another — and the catalog notices the move itself rather than
// relying on somebody to clear it.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DispatchDto, ForgeReleaseDto, ForgeSendDto } from "../../bindings";
import { api } from "../api";
import { mutate } from "../mutate";
import { dispatchCatalog } from "./dispatch.svelte";
import { releaseCatalog } from "./release.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));
vi.mock("../mutate", () => ({ mutate: vi.fn() }));

const apiMock = vi.mocked(api);
const mutateMock = vi.mocked(mutate);

function release(id: string, files: ForgeReleaseDto["files"] = []): ForgeReleaseDto {
  return {
    id,
    line_id: "L1",
    change_point_id: "c1",
    snapshot_id: "s1",
    dispatch_id: `${id}-run`,
    at_ms: 1,
    actor_kind: "user",
    actor_id: "u1",
    files,
  };
}

function stampedFile(path: string): ForgeReleaseDto["files"][number] {
  return {
    asset_id: "a1",
    path,
    xmp: { state: "written", detail: null },
    manifest: { state: "skipped", detail: "no_signing_identity" },
    prompt_dropped: false,
    system_dropped: false,
  };
}

function send(id: string): ForgeSendDto {
  return {
    id,
    release_id: "r1",
    destination: "an agency",
    dispatch_id: `${id}-run`,
    at_ms: 2,
    actor_kind: "user",
    actor_id: "u1",
  };
}

function dispatch(id: string, state: string): DispatchDto {
  return {
    id,
    snapshot_id: "s1",
    persona_id: "p1",
    exporter_slug: "transfer",
    action: "put",
    params_json: "{}",
    handle_json: null,
    attempt_json: null,
    state,
    state_message: null,
    progress_current: null,
    progress_total: null,
    output_asset_ids: [],
    created_at_ms: 1,
    updated_at_ms: 1,
    completed_at_ms: null,
    source_group_id: null,
    source_query_json: null,
    operator_ai: null,
  };
}

/// Answers each read by name, the way `forge.test.ts` does: several
/// reads fan out at once and which command asked is the only thing that
/// separates them, so a queue of `mockResolvedValueOnce` would pin an
/// order `Promise.all` does not promise.
function answering(table: Record<string, unknown>): void {
  apiMock.mockImplementation((async (cmd: string) => {
    if (!(cmd in table)) throw new Error(`unexpected read: ${cmd}`);
    return table[cmd];
  }) as unknown as typeof api);
}

beforeEach(() => {
  vi.resetAllMocks();
  releaseCatalog.close();
  releaseCatalog.byPoint = {};
  releaseCatalog.dispatches = {};
  releaseCatalog.outputDir.reset();
});

describe("opening a release", () => {
  it("reads the record and its sends, and reads the run once when it has parked", async () => {
    const poll = vi.spyOn(dispatchCatalog, "pollDispatch");
    answering({
      get_forge_release: release("r1", [stampedFile("/out/a.png")]),
      list_forge_release_sends: [],
      get_dispatch: dispatch("r1-run", "done"),
    });

    await releaseCatalog.open("r1");

    expect(releaseCatalog.openId).toBe("r1");
    expect(releaseCatalog.release.data?.files).toHaveLength(1);
    // Parked, so it is read and left alone. A poll here is sixty ticks
    // spent learning what the first read already said.
    expect(poll).not.toHaveBeenCalled();
    expect(releaseCatalog.releaseSettled).toBe(true);
  });

  /// The other half of the same rule, and the one that matters on the
  /// first frame: a release is recorded before its files exist, so the
  /// run is still going and the drawer has to be told when to look
  /// again.
  it("follows a run that has not parked", async () => {
    const poll = vi
      .spyOn(dispatchCatalog, "pollDispatch")
      .mockResolvedValue(undefined);
    answering({
      get_forge_release: release("r1"),
      list_forge_release_sends: [],
      get_dispatch: dispatch("r1-run", "running"),
    });

    await releaseCatalog.open("r1");

    expect(poll).toHaveBeenCalledWith("r1-run", expect.any(Function));
    expect(releaseCatalog.releaseSettled).toBe(false);
  });

  /// An empty file list is what a finished release that wrote nothing
  /// has *and* what one whose copies are on the way has. The dispatch
  /// is what tells them apart, which is why the drawer asks it rather
  /// than counting rows.
  it("does not read an empty file list as a finished release", async () => {
    vi.spyOn(dispatchCatalog, "pollDispatch").mockResolvedValue(undefined);
    answering({
      get_forge_release: release("r1"),
      list_forge_release_sends: [],
      get_dispatch: dispatch("r1-run", "running"),
    });
    await releaseCatalog.open("r1");
    expect(releaseCatalog.release.data?.files).toEqual([]);
    expect(releaseCatalog.releaseSettled).toBe(false);
  });
});

describe("writing a change point out", () => {
  it("re-reads the point's releases and opens the new one", async () => {
    vi.spyOn(dispatchCatalog, "pollDispatch").mockResolvedValue(undefined);
    mutateMock.mockResolvedValue(release("r2") as never);
    answering({
      release_output_dir: "/home/releases",
      list_forge_releases_of_change_point: [release("r2"), release("r1")],
      get_forge_release: release("r2"),
      list_forge_release_sends: [],
      get_dispatch: dispatch("r2-run", "running"),
    });
    // A stale count, which the write has just made wrong.
    releaseCatalog.byPoint = { c1: [release("r1")] };
    await releaseCatalog.ensureReleasesOf("L1", []);

    await releaseCatalog.writeOut("L1", "c1", "p1");

    // The directory is the backend's answer, read at the moment the
    // verb is pressed rather than derived here.
    expect(mutateMock).toHaveBeenCalledWith(
      "release_forge_change_point",
      {
        command: {
          line_id: "L1",
          change_point_id: "c1",
          persona_id: "p1",
          output_dir: "/home/releases",
        },
      },
      "write this change point out",
    );
    // Re-read rather than incremented: the count is the record's.
    expect(releaseCatalog.byPoint.c1).toHaveLength(2);
    expect(releaseCatalog.openId).toBe("r2");
  });
});

describe("sending a release", () => {
  it("re-reads the sends rather than splicing the new one in", async () => {
    vi.spyOn(dispatchCatalog, "pollDispatch").mockResolvedValue(undefined);
    mutateMock.mockResolvedValue(send("snd-2") as never);
    answering({
      get_forge_release: release("r1", [stampedFile("/out/a.png")]),
      list_forge_release_sends: [send("snd-2"), send("snd-1")],
      get_dispatch: dispatch("x", "done"),
    });
    await releaseCatalog.open("r1");

    await releaseCatalog.send("r1", "an agency", '{"endpoint":"file:///tmp"}');

    expect(mutateMock).toHaveBeenCalledWith(
      "send_forge_release",
      {
        command: {
          release_id: "r1",
          destination: "an agency",
          profile_json: '{"endpoint":"file:///tmp"}',
        },
      },
      "send this release",
    );
    expect(releaseCatalog.sends.data).toHaveLength(2);
  });

  /// **The screen holds no second opinion about when a send is
  /// allowed.** The backend refuses a release with no file rows and one
  /// whose copies have moved; `mutate` carries the refusal to the
  /// screen and this re-throws so the form can keep what was typed.
  it("lets a refusal through rather than deciding for the backend", async () => {
    vi.spyOn(dispatchCatalog, "pollDispatch").mockResolvedValue(undefined);
    mutateMock.mockRejectedValue(new Error("this release has written no files"));
    answering({ list_forge_release_sends: [] });

    await expect(
      releaseCatalog.send("r1", "an agency", "{}"),
    ).rejects.toThrow("written no files");
  });
});

describe("what a line's counts are about", () => {
  it("forgets another line's points without being told to", async () => {
    answering({ list_forge_releases_of_change_point: [release("r1")] });
    await releaseCatalog.ensureReleasesOf("L1", ["c1"]);
    expect(releaseCatalog.byPoint.c1).toHaveLength(1);

    answering({ list_forge_releases_of_change_point: [] });
    await releaseCatalog.ensureReleasesOf("L2", ["c9"]);

    // `c1` is on L1's chain and answers for nothing here.
    expect(releaseCatalog.byPoint.c1).toBeUndefined();
    expect(releaseCatalog.byPoint.c9).toEqual([]);
  });

  it("asks only about the points it is missing", async () => {
    answering({ list_forge_releases_of_change_point: [] });
    await releaseCatalog.ensureReleasesOf("L1", ["c1", "c2"]);
    expect(apiMock).toHaveBeenCalledTimes(2);

    apiMock.mockClear();
    await releaseCatalog.ensureReleasesOf("L1", ["c1", "c2"]);
    expect(apiMock).not.toHaveBeenCalled();
  });

  /// A read that failed is not evidence that a point has no releases.
  /// Recording it as none would put "no releases" on a row that may
  /// have several, and nothing would ever ask again.
  it("leaves a point it could not read unknown rather than empty", async () => {
    apiMock.mockImplementation((async () => {
      throw new Error("the backend said no");
    }) as unknown as typeof api);

    await releaseCatalog.ensureReleasesOf("L1", ["c1"]);

    expect(releaseCatalog.byPoint.c1).toBeUndefined();
  });
});
