// Chapter band arithmetic — and the shape pins for the three commands
// that write into a band.
//
// Three kinds of assertion live here and they are worth telling apart.
//
// The `chapterRowKeys` cases are about a crash. A Svelte key must be
// unique, `(layer_id, ord)` is not — the schema indexes the pair without
// a UNIQUE and defaults `ord` to 0 — and a repeat is answered with
// `each_key_duplicate`, thrown while reconciling, which takes the panel
// down rather than doubling a row. Same failure `DetailPane.test.ts`
// pins for a repeated label, one layer down.
//
// The `buildRenameCommand` / `buildMoveCommand` / `buildPostChapterCommand`
// cases are contract pins. Their counterpart is
// `material_layer_service::{post,edit}_chapter_mark` in Rust, where
// `end_ms` is read *only* when `start_ms` is present — the one line that
// makes "leave the section where it is" and "move it and give it no
// stated end" two different requests instead of one. Nothing here can
// execute that arm, so each test asserts the whole object: a field
// renamed or dropped on either side shows up as a failing equality
// rather than as a section that silently loses its end.
//
// The rest — `bandLabel`, `pickBandId`, `chapterListNote` — are about
// what a reader is told. A band has no name of its own, and "nobody has
// read this file for chapters" is not the same fact as "this file
// declares none"; both are answers a caption can quietly get wrong.
import { describe, expect, it } from "vitest";
import {
  bandEditable,
  bandLabel,
  buildCreateBandCommand,
  buildMoveCommand,
  buildPostChapterCommand,
  buildRenameCommand,
  chapterListNote,
  chapterRangeLabel,
  chapterRowKeys,
  nextChapterOrd,
  pickBandId,
  structureBands,
} from "./material-layer";
import type { ChapterMarkDto, MaterialLayerViewDto } from "../bindings";

const LAYER = "layer-1";

function chapter(
  id: string,
  startMs: number,
  ord: number,
  extra: Partial<ChapterMarkDto> = {},
): ChapterMarkDto {
  return {
    id,
    layer_id: LAYER,
    start_ms: startMs,
    end_ms: null,
    label: `section ${id}`,
    ord,
    ...extra,
  };
}

function band(
  id: string,
  origin: string,
  opts: { role?: string; isDefault?: boolean; ord?: number; chapters?: ChapterMarkDto[] } = {},
): MaterialLayerViewDto {
  return {
    layer: {
      id,
      asset_id: "asset-1",
      material_ord: 0,
      origin,
      role: opts.role ?? "structure",
      is_default: opts.isDefault ?? false,
      ord: opts.ord ?? 0,
    },
    chapters: opts.chapters ?? [],
  };
}

describe("structureBands", () => {
  it("keeps the chapter bands and drops the note bands", () => {
    const bands = structureBands([
      band("a", "imported"),
      band("b", "user", { role: "annotation" }),
      band("c", "user"),
    ]);
    expect(bands.map((b) => b.layer.id)).toEqual(["a", "c"]);
  });

  it("preserves the order the backend stated", () => {
    // Not re-sorted by `ord` here: display order is decided on the side
    // that assigned it, and a second opinion in the client is the thing
    // to avoid. A list whose `ord` disagrees with its position is
    // therefore passed through as-is.
    const bands = structureBands([
      band("late", "user", { ord: 9 }),
      band("early", "imported", { ord: 0 }),
    ]);
    expect(bands.map((b) => b.layer.id)).toEqual(["late", "early"]);
  });
});

describe("bandLabel", () => {
  it("names a chapter band by who produced it", () => {
    expect(bandLabel("imported", "structure")).toBe("From the file");
    expect(bandLabel("user", "structure")).toBe("Yours");
    expect(bandLabel("machine", "structure")).toBe("Detected");
  });

  it("says the same three things about a note band", () => {
    expect(bandLabel("imported", "annotation")).toBe("Notes from the file");
    expect(bandLabel("user", "annotation")).toBe("Your notes");
    expect(bandLabel("machine", "annotation")).toBe("Detected notes");
  });

  it("falls back to the pair itself for a slug this build has no word for", () => {
    // A row written by a newer build. Printing the pair is at least
    // true; picking one of the captions above would claim the band is
    // something it is not.
    expect(bandLabel("transcribed", "structure")).toBe("transcribed structure");
  });
});

describe("bandEditable", () => {
  it("admits only a band the person owns", () => {
    // Mirrors `require_user_owned`: the service refuses a hand edit of
    // the other two, so the surface does not offer one.
    expect(bandEditable("user")).toBe(true);
    expect(bandEditable("imported")).toBe(false);
    expect(bandEditable("machine")).toBe(false);
  });
});

describe("pickBandId", () => {
  it("keeps the band already open across a reload", () => {
    const bands = [band("file", "imported", { isDefault: true }), band("mine", "user")];
    // The reason this matters: adding a section re-reads every band, and
    // falling back to the default here would drop the reader back onto
    // the file's list the moment they wrote into their own.
    expect(pickBandId(bands, "mine")).toBe("mine");
  });

  it("opens the default band when nothing is open yet", () => {
    const bands = [band("mine", "user"), band("file", "imported", { isDefault: true })];
    expect(pickBandId(bands, null)).toBe("file");
  });

  it("falls back to the default when the open band is gone", () => {
    // What deleting a band of one's own leaves behind.
    const bands = [band("file", "imported", { isDefault: true })];
    expect(pickBandId(bands, "deleted")).toBe("file");
  });

  it("takes the first band when none is the default", () => {
    // `create_material_layer` never marks a band default, so an asset
    // whose only bands are hand-made has no flag to read.
    const bands = [band("first", "user"), band("second", "user")];
    expect(pickBandId(bands, null)).toBe("first");
  });

  it("answers nothing when there are no bands", () => {
    expect(pickBandId([], null)).toBe(null);
    expect(pickBandId([], "stale")).toBe(null);
  });
});

describe("chapterRowKeys", () => {
  it("keys a row on its band and its stated order, not on its id", () => {
    expect(chapterRowKeys([chapter("c1", 0, 0), chapter("c2", 5_000, 1)])).toEqual([
      `${LAYER}:0`,
      `${LAYER}:1`,
    ]);
  });

  it("gives a re-scan the same keys after every id changed", () => {
    // The point of the pair. Re-reading a material replaces an imported
    // band's rows wholesale, so the sections come back with new ids; a
    // key made of the id would tear down and rebuild a list whose
    // content did not change.
    const before = [chapter("old-1", 0, 0), chapter("old-2", 5_000, 1)];
    const after = [chapter("new-1", 0, 0), chapter("new-2", 5_000, 1)];
    expect(chapterRowKeys(after)).toEqual(chapterRowKeys(before));
  });

  it("keeps a band whose rows all state ord 0 unique", () => {
    // The crash case. `ord` carries `DEFAULT 0` and no UNIQUE, so a
    // container that declares no reading order lands as a band of
    // zeroes. Repeated keys are not a doubled row — Svelte throws
    // `each_key_duplicate` mid-reconcile and the panel never renders.
    const keys = chapterRowKeys([
      chapter("c1", 0, 0),
      chapter("c2", 5_000, 0),
      chapter("c3", 9_000, 0),
    ]);
    expect(keys).toEqual([`${LAYER}:0`, `${LAYER}:0#1`, `${LAYER}:0#2`]);
    expect(new Set(keys).size).toBe(3);
  });

  it("disambiguates by position among the repeats, so a re-scan still matches", () => {
    const before = [chapter("old-1", 0, 0), chapter("old-2", 5_000, 0)];
    const after = [chapter("new-1", 0, 0), chapter("new-2", 5_000, 0)];
    expect(chapterRowKeys(after)).toEqual(chapterRowKeys(before));
  });
});

describe("nextChapterOrd", () => {
  it("starts a band at 0", () => {
    expect(nextChapterOrd([])).toBe(0);
  });

  it("goes one past the highest, not one past the count", () => {
    // A band whose middle section was deleted has a gap. Counting rows
    // would hand back an `ord` that is still in the band, and two rows
    // sharing one would share a key.
    expect(nextChapterOrd([chapter("c1", 0, 0), chapter("c3", 9_000, 2)])).toBe(3);
  });

  it("ignores a row whose ord is not a number", () => {
    const broken = { ...chapter("c1", 0, 0), ord: Number.NaN };
    expect(nextChapterOrd([broken])).toBe(0);
  });
});

describe("buildCreateBandCommand", () => {
  it("opens a chapter band over the primary material, after the ones there", () => {
    const command = buildCreateBandCommand("asset-1", [
      band("file", "imported", { ord: 0 }),
      band("job", "machine", { ord: 1 }),
    ]);
    // `material_ord: null` is the primary original — the axis
    // `duration_ms` measures and the player reports.
    expect(command).toEqual({
      asset_id: "asset-1",
      material_ord: null,
      role: "structure",
      ord: 2,
    });
  });

  it("opens the first band at ord 0", () => {
    expect(buildCreateBandCommand("asset-1", []).ord).toBe(0);
  });
});

describe("buildPostChapterCommand", () => {
  it("states a start, no end, and the next reading position", () => {
    const command = buildPostChapterCommand(LAYER, 12_345.6, "  Second movement  ", [
      chapter("c1", 0, 0),
    ]);
    // `end_ms: null` is the honest answer: the person marked where a
    // section starts and said nothing about where it stops. An end taken
    // from the next chapter would be the panel inventing a claim.
    expect(command).toEqual({
      layer_id: LAYER,
      start_ms: 12_346,
      end_ms: null,
      label: "Second movement",
      ord: 1,
    });
  });

  it("accepts an untitled section", () => {
    // Unlike a mark's body. A container is free to declare a section
    // with no title (`chpl` with an empty string), so the domain accepts
    // one and a person may write one too.
    expect(buildPostChapterCommand(LAYER, 0, "   ", [])?.label).toBe("");
  });

  it("refuses a position that is not one", () => {
    expect(buildPostChapterCommand(LAYER, Number.NaN, "x", [])).toBe(null);
    expect(buildPostChapterCommand(LAYER, -1, "x", [])).toBe(null);
  });
});

describe("buildRenameCommand", () => {
  it("sends the new title and leaves the section where it is", () => {
    const command = buildRenameCommand(chapter("c1", 5_000, 0), "  Overture  ");
    // `start_ms: null` is the load-bearing field. Sending the stored
    // start back would take the move arm, and because `end_ms` is read
    // only alongside `start_ms`, would restate the end as well.
    expect(command).toEqual({
      layer_id: LAYER,
      chapter_id: "c1",
      label: "Overture",
      start_ms: null,
      end_ms: null,
      ord: null,
    });
  });

  it("says nothing when the title did not change", () => {
    const row = chapter("c1", 5_000, 0, { label: "Overture" });
    expect(buildRenameCommand(row, "Overture")).toBe(null);
    expect(buildRenameCommand(row, "  Overture  ")).toBe(null);
  });
});

describe("buildMoveCommand", () => {
  it("carries the title and the end the section already had", () => {
    const row = chapter("c1", 5_000, 0, { label: "Overture", end_ms: 30_000 });
    const command = buildMoveCommand(row, 7_500.4);
    // The end travels because it is only read alongside the start;
    // omitting it here would convert a section with a stated end into
    // one without, which is a different claim about the material.
    expect(command).toEqual({
      layer_id: LAYER,
      chapter_id: "c1",
      label: "Overture",
      start_ms: 7_500,
      end_ms: 30_000,
      ord: null,
    });
  });

  it("keeps an end the move has passed, rather than dropping it", () => {
    // `TimelineSpan::new` refuses this as an inverted interval and the
    // panel shows the refusal. Silently dropping the end to make the
    // write succeed would discard something the material states in order
    // to spare the caller a message.
    const row = chapter("c1", 5_000, 0, { end_ms: 6_000 });
    expect(buildMoveCommand(row, 9_000)?.end_ms).toBe(6_000);
  });

  it("says nothing when the section is already there", () => {
    expect(buildMoveCommand(chapter("c1", 5_000, 0), 5_000)).toBe(null);
    expect(buildMoveCommand(chapter("c1", 5_000, 0), 5_000.2)).toBe(null);
  });

  it("refuses a position that is not one", () => {
    expect(buildMoveCommand(chapter("c1", 5_000, 0), Number.NaN)).toBe(null);
    expect(buildMoveCommand(chapter("c1", 5_000, 0), -1)).toBe(null);
  });
});

describe("chapterRangeLabel", () => {
  it("prints a stated interval as one", () => {
    expect(chapterRangeLabel(0, 90_000)).toBe("0:00 – 1:30");
  });

  it("prints a section with no stated end as its start alone", () => {
    // `end_ms: null` says the section declares no end. Printing "0:00 –
    // 3:20" from the next chapter, or "0:00 – end", would both be claims
    // the data does not make.
    expect(chapterRangeLabel(0, null)).toBe("0:00");
  });
});

describe("chapterListNote", () => {
  it("says nothing when there are chapters to show", () => {
    const withRows = band("file", "imported", { chapters: [chapter("c1", 0, 0)] });
    expect(chapterListNote([withRows], withRows)).toBe(null);
  });

  it("separates 'nobody looked' from 'the file declares none'", () => {
    // The distinction this whole note exists for. No band means the
    // material has not been read for chapters and what it declares is
    // still unknown; an empty imported band means it was read and the
    // answer was none. One sentence for both would report an unasked
    // question as an answered one.
    const empty = band("file", "imported");
    expect(chapterListNote([], null)).toBe(
      "This material has not been read for chapters.",
    );
    expect(chapterListNote([empty], empty)).toBe("This file declares no chapters.");
  });

  it("reads an empty band of one's own as an empty list, not as a statement", () => {
    const mine = band("mine", "user");
    expect(chapterListNote([mine], mine)).toBe("No sections in this band yet.");
  });
});
