# asterism-core::domain::sort_eval

Sort evaluation — the backend port of the UI grid comparator.

The UI sorts the asset grid entirely client-side in
`crates/asterism-ui/src/lib/sort/card-cmp.ts`: `buildCardCmp`
produces a JS compare-fn from the `(target, order, reverse)`
[`SortSpec`], `sortCards` applies it, and `computeBucketRecency`
feeds the "most-recently-touched" orders. Query Groups must freeze
this exact
ordering into `asset_bucket.position`, so the comparator is
re-implemented here on the backend and the members are materialised
in the resulting order.

Entry point: [`sort_asset_ids`] — takes the resolved id set (as
[`SortableAsset`] rows) plus the ancillary lookups the UI reads from
its catalogs ([`SortContext`]) and returns the ordered ids that W1b's
materialize pipeline writes as `position 0, 1, 2, …`.

# Faithfulness to the UI (drift watch)

Each branch cites the `card-cmp.ts` symbol it mirrors — by name, not
by line, because the comparator previously lived inside `App.svelte`
and every citation here rotted as that component grew (the extraction
into its own module is what made the references stable, and made the
parity test below possible at all).

The paired test file is `card-cmp.test.ts`: its cases carry the same
names as the `tests` module here, over the same fixture values, so a
one-sided edit surfaces as a missing or renamed case on the other
side. `collation_parity` in both files pins the string ordering.

One deliberate, documented divergence exists because the backend
cannot rely on the UI's ambient state:

0. **The `Rating` and `UpdatedAt` axes have no UI half at all.** Both
   were wired server-side first, so `card-cmp.ts` has no `rating` or
   `updated_at` branch and no paired case exists for their tests
   below. `UpdatedAt` in particular exists for a caller the grid is
   not — an API consumer paging a differential sync — so its parity
   gap may well be permanent. Documented (see [`SortTarget::Rating`],
   [`SortTarget::UpdatedAt`]), not an omission the parity discipline
   missed.

   `Duration` and `FileSize` landed backend-first the same way and no
   longer share that gap: `card-cmp.ts` carries both branches and
   `card-cmp.test.ts` holds the paired cases (same names, same
   fixture values as the `duration / file size` block below), so the
   two comparators are pinned to each other. The layer below the
   comparator has since caught up as well — the index row carries
   both columns (`AssetIndex::duration_ms` /
   `file_size_bytes`), `indexToLightCard` forwards them, and the
   grid's Sort dropdown offers the two axes. Before that the picker
   withheld them: the rows it sorts carried neither column, so
   picking one client-side compared absent values throughout and
   answered in tie-break order.

1. **A final `id` tie-break is appended.** The UI leans on
   `Array.prototype.sort` stability + server order to break ties. The
   materialized `position` must be deterministic (it feeds
   `content_hash`), so after the axis key and the shared
   `occurred_at DESC` tie-break, ids break any remaining tie
   ascending.

# String collation

The alphabetical axes compare through ICU collation on both sides:
`Intl.Collator` in the UI, ICU4X [`Collator`] here. Which collation
is a **search parameter** — [`SortSpec::collation`], a BCP-47 tag
that rides in the query group's `query_json` — not a constant in
this module and not global settings state. `None` selects CLDR root.

Root is the default because it is the only value the two ICU builds
reproduce. `Intl.Collator` cannot name root: `"root"` is not a valid
language tag (`RangeError`) and `"und"` is not a supported locale, so
ECMA-402 resolves it to the *host* default — on a Japanese Mac,
`ja-JP`. The UI therefore asks for `"en"`, which CLDR leaves
untailored and which is consequently root. See
`card-cmp.ts` `ROOT_COLLATION_TAG` for that mapping.

Measured parity over a 76-entry corpus (Latin / accents / kana /
half-width / Han / CJK extensions / astral / emoji / symbols / the
sentinel), comparing this evaluator's root order against
`Intl.Collator("en")` on both JavaScriptCore (the WKWebView engine
the app actually runs in) and Node (the engine vitest runs in):

* JavaScriptCore and Node agree on **all 76** — which is what makes
  the vitest-side parity test meaningful evidence about the shipped
  runtime rather than about Node.
* ICU4X root agrees on **73 of 76**.

That is not a one-off measurement: the corpus and both orders live in
`fixtures/collation/` and are re-checked by three consumers — the
`collation_parity` tests below (ICU4X), `collation-parity.test.ts`
(Node), and `just collation-jsc` (JavaScriptCore). The last one is
what keeps "Node stands in for the shipped engine" an assertion
rather than an assumption; `fixtures/collation/README.md` has the
arrangement.

The four classes that used to disagree under [`str::cmp`] — case
(`apple` < `Zebra`), accents (`édition` < `future`), kana (`アイ` <
`んご`) and astral-vs-[`TAIL_SENTINEL`] — all agree now. The last of
those was never merely an ordering difference: under code-point order
an emoji label (U+1F642) sorted *after* the U+FFFF sentinel, so a
frozen group materialised a tail the grid would never render. ICU
weights U+FFFF above every assigned code point, restoring the
sentinel's contract.

## Known limitation — CJK extension ideographs

The residual 3 of 76 are Han **extension** ideographs (Ext A
U+3400–, Ext B U+20000–: `㐀` `𠀀` `𠮷`). ICU interleaves the
extension blocks among the URO; ICU4X places the URO first and the
extensions after it. Everyday Japanese lives in the URO
(U+4E00–9FFF) and agrees; the divergence is reachable only by a
label or cover whose first differing character is a rare-kanji form
such as `𠮷野` or `𩸽`. Pinned rather than fixed — closing it would
need a hand-written tailoring, and it is cheaper to let a rare-kanji
group's frozen tail sit one slot away from where the grid draws it.
`collation_parity::cjk_extension_is_the_residual_divergence` holds
the case, paired with the same name in `card-cmp.test.ts`.

`cover` is lower-cased before comparison to match the UI
(`buildCardCmp`'s `cover` branch); the other alpha axes compare raw,
also matching the UI (persona / modality / tag / group do **not**
lower-case).

## Functions

- `collator_for` — Builds the collator the alphabetical axes compare under, from the
- `sort_asset_ids` — Orders the resolved id set exactly as the grid would render it under

## Types

- `SortContext` — Ancillary lookups the UI comparator reads from its catalogs, supplied
- `SortableAsset` — The per-asset attributes the comparator reads.

