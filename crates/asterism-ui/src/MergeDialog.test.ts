/**
 * @vitest-environment happy-dom
 *
 * MergeDialog wiring tests — the layer between the state machine and a
 * person's finger.
 *
 * `merge-dialog.test.ts` next door pins the rules about the order the
 * two backend calls may happen in. It cannot say whether any of those
 * rules are reachable: a dialog whose confirm button called
 * `runPreview`, or whose keeper radio was wired to the wrong row, would
 * pass every one of those tests and still fold the wrong thing. That
 * gap is what this file is for, and it is why the suite grew a jsdom
 * environment (see `vite.config.ts`).
 *
 * Scope is the wiring and nothing else. Layout, colour and copy are not
 * asserted — the queries below go through roles and visible text, so
 * this stays a test of what the dialog *offers* rather than of how it
 * is drawn.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import type { AssetCardDto, MergeAssetsDto } from "./bindings";
import { api } from "./lib/api";
import MergeDialog from "./MergeDialog.svelte";
import { mergeDialog } from "./lib/stores/merge-dialog.svelte";

vi.mock("./lib/api", () => ({ api: vi.fn() }));
// Thumbnails reach for the Tauri asset protocol and a fetch loop, and
// this file is about which row a click names, not about pictures.
vi.mock("./lib/stores/thumb.svelte", () => ({
  thumbCatalog: { thumbSrc: () => "" },
}));

const apiMock = vi.mocked(api);

function card(id: string, locator: string): AssetCardDto {
  return {
    id,
    persona_id: "p1",
    modality: "image",
    occurred_at_ms: 1,
    cover: null,
    labels: [],
    file_size_bytes: 10,
    duration_ms: null,
    pixel_count: null,
    mime: "image/png",
    media: "image",
    source_locator: locator,
    group_ids: [],
    primary_group_position: null,
    created_at_ms: 1,
    updated_at_ms: 1,
    rating: null,
    palette: null,
    has_note: false,
    has_thread: false,
    role: "asset",
    title: null,
    member_count: 0,
    score: null,
    snippet: null,
    author_kind: null,
    author_subject: null,
    operator_ai: null,
  } as AssetCardDto;
}

const CARDS = [card("a", "/library/one.png"), card("b", "/inbox/one.png")];

function dto(overrides: Partial<MergeAssetsDto> = {}): MergeAssetsDto {
  return {
    keeper_id: "a",
    folded_ids: ["b"],
    already_folded_ids: [],
    refusals: [],
    warnings: [],
    totals: {
      edges_repointed: 0,
      edges_dropped: 0,
      buckets_moved: 0,
      children_repointed: 0,
      tags_moved: 0,
      comments_moved: 0,
      threads_reanchored: 0,
      columns_merged: 0,
      values_discarded: 0,
    },
    committed: true,
    ...overrides,
  };
}

/** Mounts the dialog over `members` with a fresh `onCommitted` spy. */
function open(members = ["a", "b"], cards = CARDS) {
  const onCommitted = vi.fn();
  mergeDialog.start(members);
  render(MergeDialog, { props: { cards, onCommitted } });
  return { onCommitted, user: userEvent.setup() };
}

const previewButton = () => screen.getByRole("button", { name: /would do|Pick the one/ });
const confirmButton = () => screen.getByRole("button", { name: /does not come back/ });

beforeEach(() => {
  apiMock.mockReset();
  mergeDialog.close();
  cleanup();
});

describe("MergeDialog — what it draws", () => {
  it("draws one row per member, named from the cards it was handed", () => {
    open();
    expect(screen.getByText("library/one.png")).toBeTruthy();
    expect(screen.getByText("inbox/one.png")).toBeTruthy();
    expect(screen.getAllByRole("radio")).toHaveLength(2);
  });

  it("draws a member the cards do not cover, by id", () => {
    // The plan is the store's ids. A dialog that drew only the rows it
    // could name would show two rows while folding three, at the one
    // moment it must not be lying about scope.
    open(["a", "b", "ghost"]);
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    expect(screen.getByText("ghost")).toBeTruthy();
  });

  it("preselects no survivor", () => {
    open();
    // Same rule the questions section states for its fold buttons: a
    // default would turn an irreversible fold into a one-click accept
    // of a guess.
    for (const radio of screen.getAllByRole<HTMLInputElement>("radio")) {
      expect(radio.checked).toBe(false);
    }
  });
});

describe("MergeDialog — the way through", () => {
  it("will not preview until a survivor is named", async () => {
    open();
    expect(previewButton().hasAttribute("disabled")).toBe(true);
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("previews the plan the radios describe", async () => {
    const { user } = open();
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await user.click(screen.getAllByRole("radio")[0]);
    await user.click(previewButton());
    expect(apiMock).toHaveBeenCalledWith("merge_assets", {
      command: { keeper_id: "a", discard_ids: ["b"], member_ids: ["a", "b"], dry_run: true },
    });
  });

  // The one above clicks the first radio, which is also the first
  // member — so it would pass just as well against a dialog that
  // ignored the click and always kept row one. This is the same test
  // with the axis disagreeing with the default.
  it("keeps the row whose radio was clicked, not the first one", async () => {
    const { user } = open();
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await user.click(screen.getAllByRole("radio")[1]);
    await user.click(previewButton());
    expect(apiMock).toHaveBeenCalledWith("merge_assets", {
      command: { keeper_id: "b", discard_ids: ["a"], member_ids: ["a", "b"], dry_run: true },
    });
  });

  it("shows the counts and the warning the preview came back with", async () => {
    const { user } = open();
    apiMock.mockResolvedValueOnce(
      dto({
        committed: false,
        warnings: [{ keeper_id: "a", headstone_id: "b", kind: "lineage" }],
        totals: { ...dto().totals, tags_moved: 4 },
      }),
    );
    await user.click(screen.getAllByRole("radio")[0]);
    await user.click(previewButton());
    expect(screen.getByText(/1 row folds into the one you kept/)).toBeTruthy();
    expect(screen.getByText(/4 tags moved/)).toBeTruthy();
    // The dry run is the only moment warnings exist, so this is the
    // only screen that can carry them — and it carries them above the
    // button that acts.
    expect(screen.getByText(/connected by lineage/)).toBeTruthy();
  });

  it("offers no confirm button before a preview, and one after", async () => {
    const { user } = open();
    expect(screen.queryByRole("button", { name: /does not come back/ })).toBeNull();
    // Naming a survivor is not enough. The state after this click is
    // the one a "confirm is enabled once the form is filled in" dialog
    // would already be offering to fold from, and the whole point of
    // the preview gate is that this is not that dialog.
    await user.click(screen.getAllByRole("radio")[0]);
    expect(screen.queryByRole("button", { name: /does not come back/ })).toBeNull();
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await user.click(previewButton());
    expect(confirmButton()).toBeTruthy();
  });

  it("takes the confirm button away again when the survivor changes", async () => {
    const { user } = open();
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await user.click(screen.getAllByRole("radio")[0]);
    await user.click(previewButton());
    await user.click(screen.getAllByRole("radio")[1]);
    // The preview on screen described keeping `a`. Confirming it now
    // would run a fold nothing had shown.
    expect(screen.queryByRole("button", { name: /does not come back/ })).toBeNull();
    expect(previewButton()).toBeTruthy();
  });
});

describe("MergeDialog — outcomes", () => {
  it("hands the folded ids to the caller on a committed run", async () => {
    const { user, onCommitted } = open();
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await user.click(screen.getAllByRole("radio")[0]);
    await user.click(previewButton());
    apiMock.mockResolvedValueOnce(dto({ folded_ids: ["b"], committed: true }));
    await user.click(confirmButton());
    // This is the callback the panel reloads and drops grid selection
    // on: rows that left the live set, by id.
    expect(onCommitted).toHaveBeenCalledWith(["b"]);
  });

  it("lists a refusal and tells the caller nothing happened", async () => {
    const { user, onCommitted } = open();
    apiMock.mockResolvedValueOnce(dto({ committed: false }));
    await user.click(screen.getAllByRole("radio")[0]);
    await user.click(previewButton());
    apiMock.mockResolvedValueOnce(
      dto({
        folded_ids: [],
        refusals: [{ asset_id: "b", reason: "the keeper is in the trash" }],
        committed: false,
      }),
    );
    await user.click(confirmButton());
    expect(screen.getByText(/Nothing was merged/)).toBeTruthy();
    expect(screen.getByText(/the keeper is in the trash/)).toBeTruthy();
    // Nothing was written, so the panel must not reload as if it had.
    expect(onCommitted).not.toHaveBeenCalled();
  });
});
