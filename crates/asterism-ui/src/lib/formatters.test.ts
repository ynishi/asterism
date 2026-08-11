// The byline reading for the note vocabulary that `AssetCommentDto`
// and `MaterialMarkDto` share.
//
// The case worth a test is the pair of nulls, because both arrive as
// "no name to show" and only one of them means the author is gone:
//
//   - `author_persona_id === null` on a persona note is schema V68's
//     headstone. `ON DELETE SET NULL` cleared the id when the Persona
//     was purged, and there is no id to look up — ever.
//   - a non-null id the catalog cannot name is a catalog miss. It is
//     the ordinary state while `personaCatalog` is still loading, and
//     rendering "(deleted persona)" there would report a purge that
//     never happened.
//
// The fixture therefore has to *disagree* with the naive reading:
// a persona id that resolves to nothing is included precisely because
// a one-branch implementation ("no name → deleted") passes every other
// case in this file.
//
// `personaCatalog` is left unseeded, so `personaName` misses for every
// id — which is the harsher arrangement for the claim under test.
import { describe, expect, it } from "vitest";
import { DELETED_PERSONA_LABEL, fmtDimensions, noteAuthorLabel } from "./formatters";

describe("noteAuthorLabel", () => {
  it("names the User side without consulting the catalog", () => {
    expect(noteAuthorLabel("user", null)).toBe("You");
  });

  it("reads a persona note with no id as the purged author", () => {
    expect(noteAuthorLabel("persona", null)).toBe(DELETED_PERSONA_LABEL);
  });

  it("does not call a catalog miss a deletion", () => {
    // Present id, absent from the catalog: unknown, not purged.
    expect(noteAuthorLabel("persona", "1f0b46c1-0000-7000-8000-000000000000")).toBe("?");
  });

  it("treats a missing author_kind as a persona note rather than as the User", () => {
    // `author_kind` is `string | null` on the wire. Only the literal
    // "user" is the User; anything else keeps the persona reading, so
    // a null never renders somebody else's note as "You".
    expect(noteAuthorLabel(null, null)).toBe(DELETED_PERSONA_LABEL);
  });
});

// The detail panel's Dimensions row. The caller gates the whole `<dt>` /
// `<dd>` pair on the return being truthy, so what "no measurement" maps
// to decides whether an unmeasured asset grows a row saying nothing.
describe("fmtDimensions", () => {
  it("renders a measured pair", () => {
    expect(fmtDimensions(4000, 1000)).toBe("4000 × 1000");
  });

  it("keeps the pair in the order it was stored", () => {
    // The stored dimensions are coded — the byte stream's own, before
    // orientation — so a portrait photo arrives here as a landscape
    // pair. Normalising it (swapping to put the larger side first, say)
    // would invent a display size the row has no basis for, and would
    // make the two calls below indistinguishable.
    expect(fmtDimensions(1000, 4000)).toBe("1000 × 4000");
    expect(fmtDimensions(4000, 1000)).not.toBe(fmtDimensions(1000, 4000));
  });

  it("reports an unmeasured asset as absent rather than as a placeholder", () => {
    // `null`, so the caller drops the row. A `—` here would put a
    // Dimensions row on every text note in the library, reading as "this
    // was measured and has no size".
    expect(fmtDimensions(null, null)).toBeNull();
    // Rows predating V69 arrive as `undefined` through the wire type's
    // optional field rather than as an explicit null; same statement.
    expect(fmtDimensions(undefined, undefined)).toBeNull();
  });

  it("refuses half a pair", () => {
    // `AssetService::add` rejects a half-written pair, so this is a
    // write that got past the gate rather than an ordinary state. Show
    // nothing rather than `4000 × ?`, which would read as a measurement.
    expect(fmtDimensions(4000, null)).toBeNull();
    expect(fmtDimensions(null, 1000)).toBeNull();
  });

  it("shows a measured zero rather than hiding it", () => {
    // Zero is a value the column can hold and a statement a parser can
    // make; it is not the absent state. A falsy-check implementation
    // (`if (!width)`) would drop it, which is the same conflation the
    // sort axis refuses.
    expect(fmtDimensions(0, 0)).toBe("0 × 0");
  });
});
