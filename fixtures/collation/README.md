# Collation fixtures

Shared fixtures for the two-sided sort contract: the grid comparator in
`crates/asterism-ui/src/lib/sort/card-cmp.ts` and its backend port in
`crates/asterism-core/src/domain/sort_eval.rs`. Query Groups freeze the
backend's order into `asset_bucket.position`, so the two implementations
have to agree on how strings compare — these files are what "agree" is
checked against, by three different engines.

| file | what it is |
|---|---|
| `corpus.txt` | 76 strings, one per line. Latin / accents / kana / half-width / Han / CJK extensions / astral / emoji / symbols / the `TAIL_SENTINEL` (U+FFFF). |
| `golden-icu.txt` | `corpus.txt` in `Intl.Collator("en")` order — what the grid shows. |
| `golden-icu4x.txt` | `corpus.txt` in ICU4X root order — what gets frozen into `position`. |

## Who reads them

| consumer | asserts |
|---|---|
| `crates/asterism-ui/src/lib/sort/collation-parity.test.ts` (vitest / Node) | Node's `Intl.Collator("en")` reproduces `golden-icu.txt` |
| `crates/asterism-core/src/domain/sort_eval.rs` `collation_parity` | ICU4X root reproduces `golden-icu4x.txt`, **and** the two goldens differ in exactly the three documented CJK-extension entries |
| `just collation-jsc` (macOS) | JavaScriptCore reproduces `golden-icu.txt` |

The third one is the point of the whole arrangement. vitest runs on Node
(V8 + full ICU); the app runs in WKWebView (JavaScriptCore). Those are
different ICU builds, so a green vitest run is only evidence about the
shipped runtime *while* the two engines agree. `just collation-jsc` is
what turns that from an assumption into a check.

## Why "en" and not "und"

`Intl.Collator` cannot name CLDR root. `"root"` is not a well-formed
language tag (`RangeError`), and `"und"` is well-formed but unsupported —
`Intl.Collator.supportedLocalesOf(["und"])` is `[]` on both engines — so
ECMA-402 falls back to the host's default locale. CLDR gives English no
collation tailoring, so `"en"` *is* root and is supported everywhere.

## Known divergence

`golden-icu.txt` and `golden-icu4x.txt` differ in three entries: `㐀`
(Ext A, U+3400), `𠀀` and `𠮷` (Ext B, U+20000+). ICU interleaves the Han
extension blocks among the URO; ICU4X places the URO first. Everyday
Japanese is URO and agrees. Pinned, not fixed — see the `sort_eval`
module docs.

## Regenerating

The goldens are frozen expectations, not build output — regenerate only
when a deliberate change to the collation makes a test fail, and read the
diff before accepting it. A failure here means one of: an ICU/ICU4X
upgrade moved an ordering, the collation parameter's default changed, or
someone edited one comparator without the other.
