/**
 * @vitest-environment happy-dom
 *
 * DetailPane label-chip tests — what the pane does with a row whose
 * `labels` array holds the same value twice.
 *
 * The backend drops repeats on the way in and on the way out
 * (`asterism_core::domain::value::dedup_labels`), so in the running app
 * this pane should never be handed one. That is the reason to pin the
 * property here rather than to skip it: what these tests state is that
 * the component does not depend on its input having been cleaned, and
 * the price of being wrong about that is the pane not rendering at all.
 *
 * It has to be asked here rather than in a Rust test because the
 * failure is a Svelte one. A keyed `{#each}` over the label strings
 * turns two equal labels into two equal keys, and Svelte answers that
 * with `each_key_duplicate` — a thrown error that takes down the whole
 * virtual list the pane sits in, not a doubled chip. Reported
 * 2026-07-20 from the running app, naming `assistant`.
 *
 * Scope is the chip strip. Layout and copy are not asserted.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import type { AssetDetailDto } from "./bindings";
import { invoke } from "@tauri-apps/api/core";
import DetailPane from "./DetailPane.svelte";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => path,
}));
// MaterialMarks (video / audio bodies only) reads through this; the
// fixture below is an image, so nothing should call it.
vi.mock("./lib/api", () => ({ api: vi.fn(async () => null) }));
// Thumbnails reach for the Tauri asset protocol and a fetch loop, and
// this file is about the chips beside the picture, not the picture.
vi.mock("./lib/stores/thumb.svelte", () => ({
  thumbCatalog: {
    thumbSrc: () => "",
    detailSrc: () => "",
    noteOriginalError: () => {},
  },
}));

const invokeMock = vi.mocked(invoke);

const ASSET_ID = "a1";

function detail(labels: string[]): AssetDetailDto {
  return {
    asset: {
      id: ASSET_ID,
      persona_id: "p1",
      source_kind: "file",
      locator: "/library/one.png",
      file_size_bytes: 10,
      platform: null,
      mime: "image/png",
      media: "image",
      content_hash: null,
      content_hash_status: null,
      modality: "image",
      labels,
      occurred_at_ms: 1,
      container_id: null,
      title: null,
      bundle_id: null,
      role: "asset",
      cover: null,
      keywords: [],
      register_note: null,
      visibility_restricted: false,
      visibility_sharing: [],
      duration_ms: null,
      width_px: null,
      height_px: null,
      rating: null,
      palette: null,
      extra_json: null,
      created_at_ms: 1,
      updated_at_ms: 1,
      author_kind: null,
      author_subject: null,
      operator_ai: null,
      attributed_via: null,
      on_duplicate: null,
      folded_into: null,
      fold_policy: "auto",
    },
    tags: [],
    edges: [],
  } as AssetDetailDto;
}

/** Answers every read the pane fires while opening one image asset. */
function serve(labels: string[]) {
  invokeMock.mockImplementation((verb: string) => {
    switch (verb) {
      case "asset_detail":
        return Promise.resolve(detail(labels));
      case "asset_lineage":
        return Promise.resolve({
          asset_id: ASSET_ID,
          nodes: [],
          edges: [],
          roots: [],
          dispatch_ids: [],
          truncated: false,
        });
      default:
        // groups_of_asset / asset_texts / list_snapshots_containing /
        // list_asset_comments — all list-shaped, all empty here.
        return Promise.resolve([]);
    }
  });
}

function mount() {
  render(DetailPane, {
    props: {
      openAssetId: ASSET_ID,
      onClose: vi.fn(),
      onOpenAsset: vi.fn(),
      onSetStatus: vi.fn(),
      onSaveLabels: vi.fn(async () => {}),
      onSetAsWallpaper: vi.fn(async () => {}),
      onRefreshCounts: vi.fn(),
      onRevealInGrid: vi.fn(),
    },
  });
}

beforeEach(() => {
  cleanup();
  invokeMock.mockReset();
});

describe("DetailPane — label chips", () => {
  it("draws every label of a row that carries the same one twice", async () => {
    // The fixture disagrees with the default a passing test could be
    // read against: a list of *distinct* labels renders identically
    // whether the key is the label or the label plus its index, so a
    // repeat is the only shape that tells the two apart.
    serve(["assistant", "cc", "assistant"]);
    mount();

    // Both copies are on screen. Under a label-only key this line does
    // not merely find one chip — Svelte throws `each_key_duplicate`
    // while reconciling, the chip strip never renders, and the query
    // times out.
    const chips = await screen.findAllByRole("button", { name: "Remove assistant" });
    expect(chips).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: "Remove cc" })).toHaveLength(1);
  });

  it("draws a repeat-free row once per label", async () => {
    serve(["assistant", "cc"]);
    mount();

    expect(await screen.findAllByRole("button", { name: "Remove assistant" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Remove cc" })).toHaveLength(1);
  });
});
