// String collation for the grid sort — the UI half of a two-sided
// contract with `asterism-core::domain::sort_eval`.
//
// The grid's order is frozen into `asset_bucket.position` when a Query
// Group materialises, so the two comparators have to agree on how
// strings compare. Both sides run ICU collation; this module is where
// the UI picks which collation, from the same search parameter the
// backend reads (`SortSpec.collation` / `activeFilter.sortCollation`).
//
// # Why `"en"` means root
//
// The neutral, language-independent collation is CLDR root, and
// `Intl.Collator` has no way to name it:
//
//   - `"root"` is not a well-formed language tag — `RangeError`.
//   - `"und"` is well-formed but unsupported:
//     `Intl.Collator.supportedLocalesOf(["und"])` is `[]` on both
//     JavaScriptCore and V8. ECMA-402 then falls back to DefaultLocale,
//     i.e. the *host's* locale. On a Japanese Mac `new
//     Intl.Collator("und")` resolves to `ja-JP` — exactly the
//     machine-dependence the fixed collation is meant to remove.
//
// `"en"` is supported everywhere and CLDR gives English no collation
// tailoring, so it *is* root. Measured against ICU4X root over a
// 76-entry corpus: JavaScriptCore and Node agree on all 76, and the
// Rust side agrees on 73 (the residual is CJK extension ideographs —
// see the `sort_eval` module docs).
export const ROOT_COLLATION_TAG = "en";

// One `Intl.Collator` per tag. Construction is the expensive part
// (`compare` afterwards is cheap), and the comparator is rebuilt on
// every sort-input change, so building one per keystroke would be
// wasteful. Keyed by the requested tag rather than the resolved one so
// a caller always gets back what it asked for.
const cache = new Map<string, (a: string, b: string) => number>();

// Compare-fn for the alphabetical sort axes.
//
// `collation` is the search parameter: `null` (the default) selects
// root, anything else is a BCP-47 tag naming a tailoring. An
// unusable tag throws here rather than silently degrading — the
// backend fails loud on the same input (`sort_eval::collator_for`),
// and a UI that quietly sorted under a different collation than the
// `position` it was about to freeze would be worse than a visible
// error.
export function textComparator(
  collation: string | null,
): (a: string, b: string) => number {
  const tag = collation ?? ROOT_COLLATION_TAG;
  const hit = cache.get(tag);
  if (hit) return hit;
  const cmp = new Intl.Collator(tag).compare;
  cache.set(tag, cmp);
  return cmp;
}
