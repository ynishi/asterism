/**
 * @vitest-environment happy-dom
 *
 * DuplicatesPanel selection tests — the half of the merge path that
 * decides *which rows* a ruling is over.
 *
 * `merge-dialog.test.ts` pins the order the backend calls may happen
 * in and `MergeDialog.test.ts` pins that the dialog offers them. Both
 * start from a plan somebody already handed to the store. This file
 * covers the step before: ticking rows in the report and turning them
 * into that plan. What can go wrong here is not a crash — it is a
 * ruling over a different set than the one on screen, which is the
 * failure `MergePlan::declare` takes a `member_ids` argument to catch
 * and which nothing else in this suite would see.
 *
 * The existing "Keep this, trash the rest" button is left alone by
 * every test here; that path is covered by the e2e suite.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import type { AssetCardDto, DuplicateGroupDto, DuplicateReportDto } from "./bindings";
import { api } from "./lib/api";
import DuplicatesPanel from "./DuplicatesPanel.svelte";
import { duplicatesCatalog } from "./lib/stores/duplicates.svelte";
import { mergeDialog } from "./lib/stores/merge-dialog.svelte";

vi.mock("./lib/api", () => ({ api: vi.fn() }));
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

function group(hash: string, ids: string[]): DuplicateGroupDto {
  return {
    axis: "artefact",
    content_hash: hash,
    members: ids.map((id) => card(id, `/library/${id}.png`)),
  };
}

const REPORT: DuplicateReportDto = {
  groups: [group("h1", ["a", "b", "c"]), group("h2", ["x", "y"])],
  unhashed_count: 0,
  unreadable_count: 0,
  unwalked_count: 0,
};

/** Answers the panel's two opening reads, then nothing else. */
function serveReport(report: DuplicateReportDto = REPORT) {
  apiMock.mockImplementation((verb: string) => {
    if (verb === "list_duplicate_groups") return Promise.resolve(report);
    if (verb === "list_duplicate_conflicts") return Promise.resolve([]);
    return Promise.reject(new Error(`unexpected verb ${verb}`));
  });
}

async function mount() {
  serveReport();
  render(DuplicatesPanel, { props: { onClose: vi.fn(), onResolved: vi.fn() } });
  const user = userEvent.setup();
  // The panel reads on mount; the checkboxes exist once it has.
  await vi.waitFor(() => expect(screen.getAllByRole("checkbox").length).toBe(5));
  return user;
}

const checkboxes = () => screen.getAllByRole<HTMLInputElement>("checkbox");
const mergeButton = () => screen.queryByRole("button", { name: /Merge \d+ into one/ });

beforeEach(() => {
  apiMock.mockReset();
  duplicatesCatalog.reset();
  mergeDialog.close();
  cleanup();
});

describe("DuplicatesPanel — turning ticks into a ruling", () => {
  it("offers no merge until two rows in one group are ticked", async () => {
    const user = await mount();
    expect(mergeButton()).toBeNull();
    await user.click(checkboxes()[0]);
    // One row is not a merge, and the command that says so is refused
    // a layer down. Offering the button here would be an invitation to
    // a call that cannot succeed.
    expect(mergeButton()).toBeNull();
    await user.click(checkboxes()[1]);
    expect(mergeButton()?.textContent).toContain("Merge 2 into one");
  });

  it("rules over exactly the ticked rows, in the order the group is drawn", async () => {
    const user = await mount();
    // Ticked back-to-front on purpose: the fold runs in the order it is
    // given and the keeper's note is concatenated in it, so the order
    // has to be the list's, not the order somebody happened to click.
    await user.click(checkboxes()[2]);
    await user.click(checkboxes()[0]);
    await user.click(mergeButton() as HTMLElement);
    expect(mergeDialog.isOpen).toBe(true);
    expect(mergeDialog.members).toEqual(["a", "c"]);
  });

  it("does not carry ticks across groups", async () => {
    const user = await mount();
    await user.click(checkboxes()[0]);
    await user.click(checkboxes()[1]);
    expect(mergeButton()?.textContent).toContain("Merge 2 into one");
    // Group two. A selection spanning both would be a ruling whose
    // members are not all on screen together — and the count on the
    // button is what would have said so.
    await user.click(checkboxes()[3]);
    expect(mergeButton()).toBeNull();
    expect(checkboxes()[0].checked).toBe(false);
    await user.click(checkboxes()[4]);
    expect(mergeButton()?.textContent).toContain("Merge 2 into one");
  });

  it("opens the dialog over the second group's rows when that is what was ticked", async () => {
    const user = await mount();
    await user.click(checkboxes()[3]);
    await user.click(checkboxes()[4]);
    await user.click(mergeButton() as HTMLElement);
    // The axis that disagrees with the default: a panel that always
    // read group one would pass the test above and fold the wrong pair
    // here.
    expect(mergeDialog.members).toEqual(["x", "y"]);
  });
});
