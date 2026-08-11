import { describe, expect, it } from "vitest";
import { albumMetaKeyProblem, readAlbumMeta } from "./album-meta";

describe("readAlbumMeta", () => {
  it("reads the statements out of the bag they ride in", () => {
    const out = readAlbumMeta({
      camera: "X100",
      _trace: {
        declared_hash: { value: "sha256:aa" },
        meta: {
          "workflow-id": {
            value: "wf-1",
            source: "pushed",
            declared_at_ms: 42,
          },
          plate: {
            value: "offwhite",
            source: "manual",
            operator: "claude-code",
            declared_at_ms: 43,
          },
        },
      },
    });

    // Sorted by name: the object arrives in whatever order it was
    // serialised, so insertion order would let two renders of one row
    // disagree.
    expect(out.map((s) => s.key)).toEqual(["plate", "workflow-id"]);
    expect(out[1]).toEqual({
      key: "workflow-id",
      value: "wf-1",
      source: "pushed",
      operator: null,
      declaredAtMs: 42,
    });
    expect(out[0].operator).toBe("claude-code");
  });

  it("returns nothing rather than throwing on the shapes the bag can hold", () => {
    // Four writers share `_trace`, and this runs on every detail
    // render: an entry in an unexpected shape has to cost one row, not
    // the panel.
    expect(readAlbumMeta({})).toEqual([]);
    expect(readAlbumMeta({ _trace: "not an object" })).toEqual([]);
    expect(readAlbumMeta({ _trace: { meta: [] } })).toEqual([]);
    expect(readAlbumMeta({ _trace: { meta: { a: "not an entry" } } })).toEqual(
      [],
    );
  });

  it("drops an entry that says nothing under its name", () => {
    // The server never writes one — a statement *is* the value — so
    // this is a hand-edited bag, and a row with a name and no value
    // would put a name on screen with nothing being said under it.
    const out = readAlbumMeta({
      _trace: {
        meta: {
          "half-written": { source: "manual" },
          real: { value: "v" },
        },
      },
    });
    expect(out.map((s) => s.key)).toEqual(["real"]);
  });

  it("tolerates an entry whose stamp is not a number", () => {
    const out = readAlbumMeta({
      _trace: { meta: { a: { value: "v", declared_at_ms: "yesterday" } } },
    });
    expect(out).toEqual([
      { key: "a", value: "v", source: null, operator: null, declaredAtMs: null },
    ]);
  });
});

describe("albumMetaKeyProblem", () => {
  it("accepts the shape the server accepts", () => {
    expect(albumMetaKeyProblem("workflow-id")).toBeNull();
    expect(albumMetaKeyProblem("plate_no_2")).toBeNull();
    expect(albumMetaKeyProblem("  trimmed  ")).toBeNull();
  });

  it("names what is wrong instead of spending a round trip on it", () => {
    // The same three rules `album_meta::parse_key` enforces. This is an
    // earlier answer to that question, not a second policy — if they
    // ever disagree, the server is the one that decides.
    expect(albumMetaKeyProblem("Workflow")).toMatch(/lowercase/);
    expect(albumMetaKeyProblem("a.b")).toMatch(/lowercase/);
    expect(albumMetaKeyProblem("")).toMatch(/required/);
    expect(albumMetaKeyProblem("k".repeat(65))).toMatch(/64/);
    expect(albumMetaKeyProblem("k".repeat(64))).toBeNull();
  });
});
