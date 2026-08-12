// Every branch of `summariseBulk`, for every verb a call site passes,
// as the string it produces.
//
// Asserting the output rather than the shape is the point, and reading
// the asserted output as English is the other half of it. Three earlier
// versions were wrong at a boundary — "moved to trash of 5 — the rest
// was refused", "none of the 1 were moved to trash", "it was not
// deleted forever" — and the third one was *asserted* by a previous
// version of this file. A transcription of what the code does is not a
// test of whether it should do it.
import { describe, expect, it } from "vitest";
import { summariseBulk, type BulkVerb } from "./bulk-status";

const TRASH: BulkVerb = { verb: "moved", into: "to trash" };
const RESTORE: BulkVerb = { verb: "restored" };
const PURGE: BulkVerb = { verb: "deleted", qualifier: "forever" };
const GROUP: BulkVerb = { verb: "added", into: "to the group" };

describe("summariseBulk", () => {
  it("says the plain verb when one was asked for and one happened", () => {
    expect(summariseBulk(1, 1, TRASH)).toBe("moved to trash");
    expect(summariseBulk(1, 1, RESTORE)).toBe("restored");
    expect(summariseBulk(1, 1, PURGE)).toBe("deleted forever");
    expect(summariseBulk(1, 1, GROUP)).toBe("added to the group");
  });

  it("counts when several were asked for and all happened", () => {
    expect(summariseBulk(5, 5, TRASH)).toBe("moved 5 to trash");
    expect(summariseBulk(5, 5, RESTORE)).toBe("restored 5");
    expect(summariseBulk(5, 5, PURGE)).toBe("deleted 5 forever");
    expect(summariseBulk(5, 5, GROUP)).toBe("added 5 to the group");
  });

  it("names both numbers when some were refused", () => {
    expect(summariseBulk(3, 5, TRASH)).toBe(
      "moved 3 of 5 to trash — the rest was refused",
    );
    expect(summariseBulk(3, 5, RESTORE)).toBe(
      "restored 3 of 5 — the rest was refused",
    );
    expect(summariseBulk(3, 5, PURGE)).toBe(
      "deleted 3 of 5 forever — the rest was refused",
    );
    expect(summariseBulk(3, 5, GROUP)).toBe(
      "added 3 of 5 to the group — the rest was refused",
    );
  });

  it("keeps the count readable when exactly one of several happened", () => {
    // The boundary that broke when the caller decided whether to show
    // the number: `say(1)` dropping it gave "moved to trash of 5".
    expect(summariseBulk(1, 5, TRASH)).toBe(
      "moved 1 of 5 to trash — the rest was refused",
    );
    expect(summariseBulk(1, 5, RESTORE)).toBe(
      "restored 1 of 5 — the rest was refused",
    );
    expect(summariseBulk(1, 5, PURGE)).toBe(
      "deleted 1 of 5 forever — the rest was refused",
    );
    expect(summariseBulk(1, 5, GROUP)).toBe(
      "added 1 of 5 to the group — the rest was refused",
    );
  });

  it("stays singular when the one thing asked for was refused", () => {
    // The second boundary, and the commonest case in practice: one
    // card, refused, nothing selected. It read "none of the 1 were
    // moved to trash" when the caller owned this phrasing.
    expect(summariseBulk(0, 1, TRASH)).toBe("it was not moved to trash");
    expect(summariseBulk(0, 1, RESTORE)).toBe("it was not restored");
    expect(summariseBulk(0, 1, GROUP)).toBe("it was not added to the group");
  });

  it("carries how many were refused when none happened", () => {
    // Only the most recent refusal is on screen, so this number is the
    // only place five refusals differ from one.
    expect(summariseBulk(0, 5, TRASH)).toBe("none of the 5 were moved to trash");
    expect(summariseBulk(0, 5, RESTORE)).toBe("none of the 5 were restored");
    expect(summariseBulk(0, 5, GROUP)).toBe(
      "none of the 5 were added to the group",
    );
  });

  it("drops the qualifier where the sentence turns negative", () => {
    // "it was not deleted forever" reads as *deleted, but not
    // permanently* — about the only irreversible action here, in the
    // direction that says an asset is gone while it is still in the
    // trash. The previous version of this file asserted both of these
    // in their broken form.
    expect(summariseBulk(0, 1, PURGE)).toBe("it was not deleted");
    expect(summariseBulk(0, 5, PURGE)).toBe("none of the 5 were deleted");
  });
});
