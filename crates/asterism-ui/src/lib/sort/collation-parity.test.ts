// Engine-level collation check for the UI half of the sort contract.
//
// `card-cmp.test.ts` covers the comparator's *branches* with small
// fixtures. This file covers the collation itself: the whole shared
// corpus, sorted, against a frozen expected order that the backend test
// and the `jsc` recipe read from the same files.
//
// What this can and cannot prove: vitest runs on Node (V8 + full ICU),
// the app runs in WKWebView (JavaScriptCore). Different ICU builds. A
// green run here is evidence about the shipped runtime only while the
// two engines agree — `just collation-jsc` is what checks that, and
// `fixtures/collation/README.md` explains the arrangement.
import { describe, expect, it } from "vitest";

// `?raw` rather than `node:fs` so the file stays inside the frontend's
// type world — this package has no `@types/node`, and pulling it in for
// one test would be a dependency added to work around a path.
// `../../../../../` is `src/lib/sort/` → repo root.
import corpusRaw from "../../../../../fixtures/collation/corpus.txt?raw";
import goldenIcuRaw from "../../../../../fixtures/collation/golden-icu.txt?raw";
import goldenIcu4xRaw from "../../../../../fixtures/collation/golden-icu4x.txt?raw";

import { ROOT_COLLATION_TAG, textComparator } from "./collation";

function lines(raw: string): string[] {
  return raw.split("\n").filter((l: string) => l.length > 0);
}

const corpus = lines(corpusRaw);
const goldenIcu = lines(goldenIcuRaw);
const goldenIcu4x = lines(goldenIcu4xRaw);

// The documented residual: Han extension ideographs, where ICU
// interleaves the extension blocks among the URO and ICU4X does not.
const CJK_EXTENSION = ["㐀", "𠀀", "𠮷"];

describe("collation corpus", () => {
  it("fixtures are consistent with each other", () => {
    // Guards the fixtures themselves: a hand-edit that drops or
    // duplicates an entry would otherwise surface as a confusing
    // ordering failure below.
    expect(new Set(corpus).size).toBe(corpus.length);
    expect([...goldenIcu].sort()).toEqual([...corpus].sort());
    expect([...goldenIcu4x].sort()).toEqual([...corpus].sort());
  });

  it("root collation reproduces the ICU golden order", () => {
    const sorted = [...corpus].sort(textComparator(null));
    expect(sorted).toEqual(goldenIcu);
  });

  it("the two goldens differ only in CJK extension ideographs", () => {
    // The parity statement, from this side. `sort_eval.rs` asserts the
    // same thing over the same files, so neither language can quietly
    // widen the gap.
    const drop = (xs: string[]) => xs.filter((s) => !CJK_EXTENSION.includes(s));
    expect(drop(goldenIcu4x)).toEqual(drop(goldenIcu));
    // ...and they really do differ, so this test cannot pass by the
    // divergence having silently disappeared without the fixtures being
    // updated.
    expect(goldenIcu4x).not.toEqual(goldenIcu);
  });

  it("the tail sentinel is last and no label can escape it", () => {
    // The property the sentinel exists for, stated over the whole
    // corpus rather than one emoji pair: nothing sorts past U+FFFF.
    expect(goldenIcu.at(-1)).toBe("\u{FFFF}");
    expect(goldenIcu4x.at(-1)).toBe("\u{FFFF}");
  });

  it("und would not have worked", () => {
    // Why `ROOT_COLLATION_TAG` is `"en"`. If a future engine starts
    // supporting `und`, this fails and the mapping should be revisited.
    expect(Intl.Collator.supportedLocalesOf(["und"])).toEqual([]);
    expect(Intl.Collator.supportedLocalesOf([ROOT_COLLATION_TAG])).not.toEqual(
      [],
    );
    expect(() => new Intl.Collator("root")).toThrow(RangeError);
  });
});
