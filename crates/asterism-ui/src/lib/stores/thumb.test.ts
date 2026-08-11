// thumbCatalog unit tests.
// `convertFileSrc` needs a Tauri window bridge, so the core module
// is mocked with a deterministic `asset://` transform; the api
// choke point is mocked per command. Node ≥16.7 implements
// `URL.createObjectURL(Blob)`, so the blob-URL path runs for real.
// The catalog is a singleton without a test reset — every test
// uses unique asset / group ids to stay independent.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { activeFilter } from "./filter.svelte";
import { BLOB_CACHE_CAP, thumbCatalog } from "./thumb.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => `asset://${p}`,
  invoke: vi.fn(),
}));

const apiMock = vi.mocked(api);

function card(id: string, locator = `/orig/${id}.png`) {
  return {
    id,
    persona_id: "p1",
    modality: "image",
    occurred_at_ms: 0,
    cover: null,
    labels: [],
    file_size_bytes: null,
    // A still image has no playback length; the card carries the field
    // regardless, because absence is the answer the length axis tails on.
    duration_ms: null,
    // Absent for the same shape of reason: these fixtures are about
    // thumbnails, and nothing here measured a resolution.
    pixel_count: null,
    mime: "image/png",
    // The slug the backend projects from `render_policy`. The UI reads
    // this, not `mime` — a fixture that sets only `mime` describes a
    // payload no backend produces.
    media: "image",
    source_locator: locator,
    group_ids: [],
    primary_group_position: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    rating: null,
    palette: null,
    has_note: false,
    has_thread: false,
    role: "item",
    title: null,
    member_count: 0,
    score: null,
    snippet: null,
    author_kind: null,
    author_subject: null,
    operator_ai: null,
  };
}

const BYTES = [1, 2, 3];

/**
 * Answers `get_asset_thumbs` slot-for-slot.
 *
 * The catalog batches, so a mock has to honour the contract the real
 * command has: one slot per requested id, in order. `bytes = null`
 * makes every slot a miss.
 */
function mockThumbs(bytes: number[] | null = BYTES) {
  apiMock.mockImplementation((command: string, args?: unknown) => {
    if (command === "get_asset_thumbs") {
      const ids = (args as { assetIds?: string[] } | undefined)?.assetIds ?? [];
      return Promise.resolve(ids.map(() => bytes));
    }
    return Promise.resolve(null);
  });
}

/** Ids of every `get_asset_thumbs` call made so far, oldest first. */
function batchedIds(): string[][] {
  return apiMock.mock.calls
    .filter(([command]) => command === "get_asset_thumbs")
    .map(([, args]) => (args as { assetIds: string[] }).assetIds);
}

describe("thumbCatalog", () => {
  beforeEach(() => {
    apiMock.mockReset();
    activeFilter.reset();
  });

  it("thumbSrc falls back to the original file and swaps in the blob", async () => {
    mockThumbs();
    const c = card("t-swap");
    expect(thumbCatalog.thumbSrc(c)).toBe(`asset:///orig/t-swap.png`);
    await thumbCatalog.ensureThumb(c.id, 256); // await the batch
    const src = thumbCatalog.thumbSrc(c);
    expect(src.startsWith("blob:")).toBe(true);
    expect(apiMock).toHaveBeenCalledWith("get_asset_thumbs", {
      assetIds: ["t-swap"],
      sizePx: 256,
    });
  });

  it("sends one request for a screenful, not one per card", async () => {
    // Teeth for the whole batching change. `thumbSrc` runs once per
    // card during template evaluation, so a painted screenful used to
    // be a screenful of IPC round trips — 8,263 of them across 1,000
    // jumps, p95 up to 10.4 s [measured 2026-08-05, bench-scroll-v3].
    // Asserting the *count* is the only way to catch a regression that
    // still fills the cache correctly, one slow request at a time.
    mockThumbs();
    const screenful = ["b-1", "b-2", "b-3", "b-4", "b-5"];
    for (const id of screenful) thumbCatalog.thumbSrc(card(id));
    await thumbCatalog.ensureThumb("b-1", 256);

    const batches = batchedIds();
    expect(batches).toHaveLength(1);
    expect(batches[0]).toEqual(screenful);
    for (const id of screenful) {
      expect(thumbCatalog.thumbSrc(card(id)).startsWith("blob:")).toBe(true);
    }
  });

  it("keeps sizes in separate batches (the command takes one size)", async () => {
    mockThumbs();
    thumbCatalog.thumbSrc(card("s-mix"));
    thumbCatalog.detailSrc("/orig/s-mix.png", "s-mix");
    await thumbCatalog.ensureThumb("s-mix", 256);

    const sizes = apiMock.mock.calls
      .filter(([command]) => command === "get_asset_thumbs")
      .map(([, args]) => (args as { sizePx: number }).sizePx);
    expect(sizes.sort()).toEqual([256, 512]);
  });

  it("never falls back to the original for a video — an <img> cannot play one", async () => {
    mockThumbs(null); // frame not extracted yet
    const c = { ...card("t-video", "/orig/clip.mp4"), mime: "video/mp4", media: "video" };
    // The still path would hand the raw file to the <img>; for a video
    // that is a guaranteed load failure, so the placeholder holds.
    expect(thumbCatalog.thumbSrc(c)).toBe(
      "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
    );
    expect(thumbCatalog.detailSrc(c.source_locator, c.id, c.media)).toBe(
      "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
    );

    // Once the extracted frame lands it paints like any other thumb.
    // A separate id because the catalog is a singleton and the fetch
    // above is still in flight for `t-video` — its `#pending` guard
    // would swallow the second request.
    mockThumbs();
    const landed = {
      ...card("t-video-landed", "/orig/clip2.mp4"),
      mime: "video/mp4",
      media: "video",
    };
    await thumbCatalog.ensureThumb(landed.id, 256);
    expect(thumbCatalog.thumbSrc(landed).startsWith("blob:")).toBe(true);
  });

  it("decides on media, not mime — the slug alone holds the placeholder", async () => {
    // Teeth for moving the rule to the backend. Every other fixture
    // here carries both `mime` and `media`, so either implementation
    // passes them; this one carries **only the slug**, which is what
    // the store is supposed to be reading. A version that went back to
    // `mime.startsWith("video/")` sees nothing here and hands the raw
    // `.mp4` to an `<img>` — a guaranteed load failure that also marks
    // the original dead through `onerror`.
    mockThumbs(null); // frame not extracted yet
    const c = {
      ...card("t-media-only", "/orig/clip3.mp4"),
      mime: null,
      media: "video",
    };
    expect(thumbCatalog.thumbSrc(c)).toBe(
      "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
    );
  });

  it("caches per (assetId, sizePx) — 256 and 512 stay separate", async () => {
    mockThumbs();
    const c = card("t-sizes");
    await thumbCatalog.ensureThumb(c.id, 256);
    expect(thumbCatalog.thumbSrc(c).startsWith("blob:")).toBe(true);
    // 512 not fetched yet → detailSrc still falls back to the raw file.
    expect(thumbCatalog.detailSrc(c.source_locator, c.id)).toBe(
      `asset:///orig/t-sizes.png`,
    );
    await thumbCatalog.ensureThumb(c.id, 512);
    expect(
      thumbCatalog.detailSrc(c.source_locator, c.id).startsWith("blob:"),
    ).toBe(true);
  });

  it("leaves the cache empty on a miss with no retries left", async () => {
    mockThumbs(null); // thumb not generated yet
    const c = card("t-miss");
    await thumbCatalog.ensureThumb(c.id, 256, 0);
    expect(thumbCatalog.thumbSrc(c)).toBe(`asset:///orig/t-miss.png`);
  });

  it("groupCoverThumb picks the group's first image and renders its thumb path", async () => {
    // 1st call: cover asset unknown → kicks list_assets, returns null.
    let resolveList!: (v: unknown) => void;
    apiMock.mockImplementationOnce(
      () => new Promise((res) => (resolveList = res)),
    );
    expect(thumbCatalog.groupCoverThumb("g-img")).toBeNull();
    expect(apiMock).toHaveBeenCalledWith(
      "list_assets",
      expect.objectContaining({
        query: expect.objectContaining({ group_ids: ["g-img"], limit: 1 }),
      }),
    );
    // Cover resolves → 2nd call returns the original-file fallback
    // while the 128 px thumb fetch is in flight.
    mockThumbs(); // subsequent get_asset_thumb
    resolveList({ items: [card("cover-1")], offset: 0, limit: 1, total: 1 });
    await Promise.resolve(); // let the cover continuation land
    expect(thumbCatalog.groupCoverThumb("g-img")).toBe(
      `asset:///orig/cover-1.png`,
    );
    await thumbCatalog.ensureThumb("cover-1", 128);
    expect(thumbCatalog.groupCoverThumb("g-img")!.startsWith("blob:")).toBe(
      true,
    );
  });

  it("caches 'no image asset' as null without refetching", async () => {
    apiMock.mockResolvedValue({ items: [], offset: 0, limit: 1, total: 0 });
    expect(thumbCatalog.groupCoverThumb("g-empty")).toBeNull();
    await Promise.resolve();
    await Promise.resolve();
    apiMock.mockClear();
    expect(thumbCatalog.groupCoverThumb("g-empty")).toBeNull();
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("stops re-kicking the fetch after the retry budget is exhausted", async () => {
    mockThumbs(null); // thumb permanently missing
    const c = card("t-nokick");
    await thumbCatalog.ensureThumb(c.id, 256, 0); // exhausts immediately
    apiMock.mockClear();
    thumbCatalog.thumbSrc(c); // would previously re-kick ensureThumb
    await Promise.resolve();
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("serves a placeholder for dead originals instead of the asset URL", () => {
    const c = card("t-dead");
    // Simulate the <img onerror> report for a non-blob src.
    thumbCatalog.noteOriginalError(c.id, {
      currentTarget: { src: `asset:///orig/t-dead.png` },
    } as unknown as Event);
    expect(thumbCatalog.thumbSrc(c).startsWith("data:image/gif")).toBe(true);
    expect(
      thumbCatalog.detailSrc(c.source_locator, c.id).startsWith("data:image/gif"),
    ).toBe(true);
  });

  it("ignores onerror reports for blob/data srcs (decode failures)", () => {
    const c = card("t-blobfail");
    thumbCatalog.noteOriginalError(c.id, {
      currentTarget: { src: "blob:nodedata:xyz" },
    } as unknown as Event);
    expect(thumbCatalog.thumbSrc(c)).toBe(`asset:///orig/t-blobfail.png`);
  });

  it("revokeAll drops cached blob URLs (next read falls back again)", async () => {
    mockThumbs();
    const c = card("t-revoke");
    await thumbCatalog.ensureThumb(c.id, 256);
    expect(thumbCatalog.thumbSrc(c).startsWith("blob:")).toBe(true);
    thumbCatalog.revokeAll();
    expect(thumbCatalog.thumbSrc(c)).toBe(`asset:///orig/t-revoke.png`);
  });

  it("holds the cache at the cap and revokes what it drops", async () => {
    // Teeth: a blob URL is not garbage-collected, so an unbounded
    // cache grows until the app closes — measured at 3,361 entries and
    // still climbing after 200 jumps across a 110k library
    // [measured 2026-08-05, bench-scroll-v3]. Both halves matter: the size
    // has to stop growing, and the dropped URL has to be *revoked*,
    // because forgetting the key while the browser keeps the bytes
    // leaks the same memory with no way left to reach it.
    mockThumbs();
    thumbCatalog.revokeAll();
    const revoke = vi.spyOn(URL, "revokeObjectURL");

    for (let i = 0; i < BLOB_CACHE_CAP; i += 1) {
      await thumbCatalog.ensureThumb(`cap-${i}`, 256);
    }
    expect(thumbCatalog.blobUrlCount).toBe(BLOB_CACHE_CAP);
    expect(revoke).not.toHaveBeenCalled();

    await thumbCatalog.ensureThumb("cap-overflow", 256);
    expect(thumbCatalog.blobUrlCount).toBe(BLOB_CACHE_CAP);
    expect(revoke).toHaveBeenCalledTimes(1);

    revoke.mockRestore();
  });

  it("evicts by least-recently-used, not by insertion order", async () => {
    // The fixture disagrees with FIFO on purpose: `cap2-0` is the
    // oldest *insertion* but the newest *use*, so a FIFO cache would
    // drop it and an LRU keeps it. Without this the eviction could be
    // plain insertion-order and still pass the size assertion above —
    // while dropping the tile the user is looking at, which is the one
    // eviction that must never happen.
    mockThumbs();
    thumbCatalog.revokeAll();

    for (let i = 0; i < BLOB_CACHE_CAP; i += 1) {
      await thumbCatalog.ensureThumb(`cap2-${i}`, 256);
    }
    // Paint the oldest entry: that is what "in use" means here.
    expect(thumbCatalog.thumbSrc(card("cap2-0")).startsWith("blob:")).toBe(true);

    await thumbCatalog.ensureThumb("cap2-overflow", 256);

    // The touched one survived; the untouched next-oldest is the one
    // that went (its read falls back to the original file).
    expect(thumbCatalog.thumbSrc(card("cap2-0")).startsWith("blob:")).toBe(true);
    expect(thumbCatalog.thumbSrc(card("cap2-1"))).toBe(
      "asset:///orig/cap2-1.png",
    );
  });
});
