/**
 * @vitest-environment happy-dom
 *
 * MaterialChapters — what the panel does with the bands it is handed.
 *
 * The arithmetic is pinned next door in `lib/material-layer.test.ts`,
 * which runs on Node. What is left here is what only a mounted
 * component can answer, and each case is here because getting it wrong
 * is invisible in a pure function:
 *
 *   * **Whose band is on screen decides what may be done to it.** The
 *     service refuses a hand edit of an imported band, so the panel must
 *     not offer one. A test of `bandEditable` says the predicate is
 *     right; only a render says the compose box is actually gone.
 *   * **A band that declares no chapters and a material nobody has read
 *     for chapters are different facts.** Both are empty lists, and the
 *     way to collapse them is to render the same sentence for both.
 *   * **A repeated `(layer_id, ord)` is a thrown error, not a doubled
 *     row.** `each_key_duplicate` takes down the whole panel while
 *     reconciling — the same failure `DetailPane.test.ts` was written
 *     for after a row carried one label twice, and unobservable without
 *     mounting the block.
 *
 * Scope is the panel's structure and its writes. Layout is not
 * asserted.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import { api } from "./lib/api";
import MaterialChapters from "./MaterialChapters.svelte";
import type { ChapterMarkDto, MaterialLayerViewDto } from "./bindings";

vi.mock("./lib/api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

const ASSET_ID = "asset-1";
const FILE_BAND = "layer-file";
const MY_BAND = "layer-mine";

function chapter(
  layerId: string,
  id: string,
  startMs: number,
  ord: number,
  label: string,
  endMs: number | null = null,
): ChapterMarkDto {
  return { id, layer_id: layerId, start_ms: startMs, end_ms: endMs, label, ord };
}

function band(
  id: string,
  origin: string,
  chapters: ChapterMarkDto[],
  isDefault = false,
): MaterialLayerViewDto {
  return {
    layer: {
      id,
      asset_id: ASSET_ID,
      material_ord: 0,
      origin,
      role: "structure",
      is_default: isDefault,
      ord: 0,
    },
    chapters,
  };
}

/** Answers the one read the panel fires, with the bands given. */
function serve(views: MaterialLayerViewDto[]) {
  apiMock.mockImplementation(async (cmd: string) => {
    if (cmd === "list_material_layers") return views as never;
    return undefined as never;
  });
}

function mount(durationMs: number | null = 600_000) {
  render(MaterialChapters, {
    props: { assetId: ASSET_ID, durationMs, media: null },
  });
}

beforeEach(() => {
  cleanup();
  apiMock.mockReset();
});

describe("MaterialChapters — what it shows", () => {
  it("draws the file's sections and says whose band they are", async () => {
    serve([
      band(
        FILE_BAND,
        "imported",
        [
          chapter(FILE_BAND, "c1", 0, 0, "Opening", 90_000),
          chapter(FILE_BAND, "c2", 90_000, 1, "Second movement"),
        ],
        true,
      ),
    ]);
    mount();

    expect(await screen.findByText("Second movement")).toBeTruthy();
    expect(screen.getByText("Opening")).toBeTruthy();
    // A stated interval reads as one; a section with no stated end reads
    // as its start alone, never as "runs to the next one".
    expect(screen.getByText("0:00 – 1:30")).toBeTruthy();
    expect(screen.getByText("1:30")).toBeTruthy();
    // The band has no name of its own — the chip says what it is.
    expect(screen.getByRole("button", { name: /From the file/ })).toBeTruthy();
  });

  it("offers nothing to edit on a band the file declared", async () => {
    serve([band(FILE_BAND, "imported", [chapter(FILE_BAND, "c1", 0, 0, "Opening")], true)]);
    mount();

    await screen.findByText("Opening");
    // `require_user_owned` would refuse every one of these. An absent
    // button is a better answer than a rejected write.
    expect(screen.queryByRole("button", { name: "Add section" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: /^Title of the section/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Delete the section/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete band" })).toBeNull();
  });

  it("tells 'nobody read this' apart from 'the file declares none'", async () => {
    serve([]);
    mount();
    expect(
      await screen.findByText("This material has not been read for chapters."),
    ).toBeTruthy();

    cleanup();
    apiMock.mockReset();
    serve([band(FILE_BAND, "imported", [], true)]);
    mount();
    expect(await screen.findByText("This file declares no chapters.")).toBeTruthy();
  });

  it("renders every section of a band that states no reading order", async () => {
    // `ord` carries `DEFAULT 0` and no UNIQUE, so a container that
    // declares its sections without an order lands as a band of zeroes.
    // Under a bare `(layer_id, ord)` key this line does not merely find
    // one row: Svelte throws `each_key_duplicate` while reconciling, the
    // panel never renders, and the query times out.
    serve([
      band(
        FILE_BAND,
        "imported",
        [
          chapter(FILE_BAND, "c1", 0, 0, "One"),
          chapter(FILE_BAND, "c2", 60_000, 0, "Two"),
          chapter(FILE_BAND, "c3", 120_000, 0, "Three"),
        ],
        true,
      ),
    ]);
    mount();

    expect(await screen.findByText("Three")).toBeTruthy();
    expect(screen.getByText("One")).toBeTruthy();
    expect(screen.getByText("Two")).toBeTruthy();
  });

  it("stays off an asset with no timeline", async () => {
    serve([band(FILE_BAND, "imported", [chapter(FILE_BAND, "c1", 0, 0, "Opening")], true)]);
    mount(null);

    // A still image has no divisions to declare, so the panel does not
    // appear — and does not ask the backend for bands either.
    expect(screen.queryByLabelText("Chapters")).toBeNull();
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("opens the default band, and switches to the one that is clicked", async () => {
    serve([
      band(FILE_BAND, "imported", [chapter(FILE_BAND, "c1", 0, 0, "Opening")], true),
      band(MY_BAND, "user", [chapter(MY_BAND, "m1", 30_000, 0, "Where it really starts")]),
    ]);
    mount();

    await screen.findByText("Opening");
    expect(screen.queryByRole("textbox", { name: /^Title of the section/ })).toBeNull();

    screen.getByRole("button", { name: /Yours/ }).click();

    // A band of one's own puts each title in an editable field, so the
    // switch is read off the field rather than off the text.
    const title = (await screen.findByRole("textbox", {
      name: "Title of the section at 0:30",
    })) as HTMLInputElement;
    expect(title.value).toBe("Where it really starts");
    expect(screen.queryByText("Opening")).toBeNull();
  });
});

describe("MaterialChapters — writing into a band of one's own", () => {
  it("adds a section at the playhead and re-reads the bands", async () => {
    serve([band(MY_BAND, "user", [chapter(MY_BAND, "m1", 0, 0, "First")])]);
    mount();

    const input = (await screen.findByRole("textbox", {
      name: "Title of the new section",
    })) as HTMLInputElement;
    input.value = "Second";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    screen.getByRole("button", { name: "Add section" }).click();

    await vi.waitFor(() => {
      expect(apiMock).toHaveBeenCalledWith("post_chapter_mark", {
        command: {
          layer_id: MY_BAND,
          // The media element is absent in this mount, so the playhead
          // stands at the origin.
          start_ms: 0,
          end_ms: null,
          label: "Second",
          ord: 1,
        },
      });
    });
    // Re-read rather than splice: where the new section sits in the band
    // is decided by `ord`, on the side that assigned it.
    await vi.waitFor(() => {
      expect(apiMock.mock.calls.filter((c) => c[0] === "list_material_layers").length).toBe(2);
    });
  });

  it("retitles a section without restating where it is", async () => {
    serve([band(MY_BAND, "user", [chapter(MY_BAND, "m1", 30_000, 0, "Old", 90_000)])]);
    mount();

    const title = (await screen.findByRole("textbox", {
      name: "Title of the section at 0:30",
    })) as HTMLInputElement;
    title.value = "New";
    title.dispatchEvent(new Event("change", { bubbles: true }));

    await vi.waitFor(() => {
      expect(apiMock).toHaveBeenCalledWith("edit_chapter_mark", {
        command: {
          layer_id: MY_BAND,
          chapter_id: "m1",
          label: "New",
          // `start_ms: null` leaves the section alone. Any number here
          // would travel the move arm and, since `end_ms` is only read
          // beside `start_ms`, would silently restate the 1:30 end.
          start_ms: null,
          end_ms: null,
          ord: null,
        },
      });
    });
  });

  it("opens the band it just made, rather than falling back to the default", async () => {
    const created = {
      id: MY_BAND,
      asset_id: ASSET_ID,
      material_ord: 0,
      origin: "user",
      role: "structure",
      is_default: false,
      ord: 1,
    };
    let bands = [band(FILE_BAND, "imported", [chapter(FILE_BAND, "c1", 0, 0, "Opening")], true)];
    apiMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_material_layers") return bands as never;
      if (cmd === "create_material_layer") {
        bands = [...bands, band(MY_BAND, "user", [])];
        return created as never;
      }
      return undefined as never;
    });
    mount();

    await screen.findByText("Opening");
    screen.getByRole("button", { name: "+ band" }).click();

    await vi.waitFor(() => {
      expect(apiMock).toHaveBeenCalledWith("create_material_layer", {
        command: { asset_id: ASSET_ID, material_ord: null, role: "structure", ord: 1 },
      });
    });
    // The new band is never the default, so a re-read that consulted
    // the flag would drop the reader back onto the file's list the
    // moment they asked for one of their own.
    expect(
      await screen.findByRole("textbox", { name: "Title of the new section" }),
    ).toBeTruthy();
    expect(await screen.findByText("No sections in this band yet.")).toBeTruthy();
  });

  it("re-reads every band after moving the default flag", async () => {
    serve([
      band(FILE_BAND, "imported", [chapter(FILE_BAND, "c1", 0, 0, "Opening")], true),
      band(MY_BAND, "user", []),
    ]);
    mount();

    await screen.findByText("Opening");
    screen.getByRole("button", { name: /Yours/ }).click();
    await screen.findByRole("textbox", { name: "Title of the new section" });
    screen.getByRole("button", { name: "Make default" }).click();

    await vi.waitFor(() => {
      expect(apiMock).toHaveBeenCalledWith("set_default_material_layer", {
        command: { layer_id: MY_BAND },
      });
    });
    // The flag moves *off* whichever band held it, so there is no one
    // entry to patch — which is why the call answers with nothing and
    // the panel re-reads the asset's bands instead.
    await vi.waitFor(() => {
      expect(apiMock.mock.calls.filter((c) => c[0] === "list_material_layers").length).toBe(2);
    });
  });

  it("shows a refused write and keeps the list on screen", async () => {
    serve([band(MY_BAND, "user", [chapter(MY_BAND, "m1", 30_000, 0, "First")])]);
    mount();

    const title = (await screen.findByRole("textbox", {
      name: "Title of the section at 0:30",
    })) as HTMLInputElement;

    apiMock.mockImplementation(async (cmd: string) => {
      if (cmd === "edit_chapter_mark") throw new Error("an imported band is read-only");
      return [] as never;
    });
    title.value = "New";
    title.dispatchEvent(new Event("change", { bubbles: true }));

    // "This edit was refused" is not "the bands could not be read", and
    // the second one is the only one that should empty the panel.
    expect(await screen.findByText("an imported band is read-only")).toBeTruthy();
    expect(screen.getByLabelText("Chapters")).toBeTruthy();
  });
});
