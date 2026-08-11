// Grid sort comparator — the UI half of the two-sided sort contract.
//
// Extracted from `App.svelte` (was `buildCardCmp` + `bucketRecency` +
// `firstUserLabel`, all instance-script locals). The backend re-implements
// this exact comparator in `crates/asterism-core/src/domain/sort_eval.rs`
// so Query Groups can freeze the grid order into `asset_bucket.position`.
// Two SoTs that must agree cannot be diffed while one of them is a local
// function inside a component script: the extraction is what makes the
// parity test (`card-cmp.test.ts`) possible, and the Rust docstring cites
// this module rather than `App.svelte` line numbers (which drift every
// time the component grows).
//
// Purity contract: no catalog imports, no runes, no DOM. Everything the
// comparator needs from App-owned catalogs arrives through
// [`CardSortLookups`], mirroring the backend's `SortContext`. That keeps
// the module importable from vitest with hand-built fixtures.
//
// String keys compare through [`CardSortLookups.compareText`], an
// injected ICU collator (`./collation`), because the backend freezes
// this order into `asset_bucket.position` and has to reach the same
// answer. It is injected rather than built here for the same reason the
// catalog lookups are: which collation to use is a *search parameter*
// (`activeFilter.sortCollation`, mirroring `SortSpec.collation`), not a
// constant this module gets to pick.
//
// Known divergence from the backend (documented, not accidental):
//
//   - CJK **extension** ideographs (Ext A U+3400–, Ext B U+20000–).
//     ICU interleaves them among the URO; ICU4X puts the URO first.
//     Everyday Japanese is URO and agrees. See the `collation parity`
//     block in `card-cmp.test.ts` and the `sort_eval` module docs.
// One axis has no backend twin, by design: `relevance` orders the page
// by a Retrieval rank (`search_asset_ids`) rather than by a card field,
// and a retriever does not promise the same answer twice — so it cannot
// be frozen into `asset_bucket.position` and the wire enum does not
// carry the token. The
// rank arrives as a `Map<id, position>` parameter for the same reason
// the catalog lookups do: it is a *search result*, not a card field, and
// this module stays pure.
//
//
// Every member of `SortTarget` has a branch below. `msg_count` used to be
// the exception — declared on the union, implemented by neither side, so
// picking it ordered by nothing — and it was removed from the union and
// from the wire (`SortSpec`'s Rust enum) rather than given a comparator,
// because the grid retired it from the picker when the Session tiles left
// (asset-model v4 P3).

import type { SortOrder, SortTarget } from "../stores/filter.svelte";

// Sort key for "no user-visible label" / "not filed in any group". A
// high private-use code point so these cluster at the tail of an
// ascending string sort instead of the head. The backend mirrors the
// literal (`sort_eval.rs` `TAIL_SENTINEL`).
export const TAIL_SENTINEL = "\u{FFFF}";

// Label namespaces the grid never surfaces as a tag. Mirrored by
// `sort_eval.rs` `INTERNAL_LABEL_PREFIXES`.
export const INTERNAL_LABEL_PREFIXES = ["persona:", "journal_kind:"];

// First label that is not internal; [`TAIL_SENTINEL`] when the card
// carries none. Also used by the grid's section-header labels, so it
// stays exported rather than folded into the comparator.
export function firstUserLabel(labels: readonly string[]): string {
  for (const l of labels) {
    let hidden = false;
    for (const p of INTERNAL_LABEL_PREFIXES) {
      if (l.startsWith(p)) {
        hidden = true;
        break;
      }
    }
    if (!hidden) return l;
  }
  return TAIL_SENTINEL;
}

// The card fields the comparator reads. Structural subset of
// `AssetCardDto` so the real DTO satisfies it, and so tests can build
// fixtures without inventing the ~10 fields the sort never touches.
// Backend counterpart: `sort_eval::SortableAsset`.
export type SortableCard = {
  // Read by the `relevance` axis, which looks the card up in a rank map
  // keyed by asset id. Present on the backend twin too
  // (`sort_eval::SortableAsset::id`, the value its sort returns).
  readonly id: string;
  readonly persona_id: string;
  // `null` = unclassified (asset-model v4). Normalised to "" for
  // ranking — an unknown slug already ranks last, and the backend
  // (`sort_eval`) reads NULL as "" through the same convention, so
  // the parity contract holds without a dedicated branch.
  readonly modality: string | null;
  readonly occurred_at_ms: number;
  readonly created_at_ms: number;
  readonly labels: readonly string[];
  readonly group_ids: readonly string[];
  // Slot inside the primary group (`group_ids[0]`), `null` when unfiled.
  // Read only by `group` + `ordered`, which is the hand arrangement.
  readonly primary_group_position: number | null;
  readonly cover: string | null;
  // Playback length in ms, `null` for material that does not play — a
  // still image, or a container the importer could not probe. Kept
  // nullable rather than folded into a number: a stand-in `0` would
  // place it at one end of the band and flip it to the other under
  // reverse. Backend twin: `sort_eval::SortableAsset::duration_ms`.
  //
  // Optional, unlike its backend twin, and that is now only about
  // fixtures: every row the grid sorts carries the field
  // (`AssetCardDto.duration_ms`, forwarded onto light rows by
  // `asset-page.svelte.ts` `indexToLightCard`), so the axis has real
  // keys to compare. The optionality stays because an absent property
  // and an explicit `null` are the same statement here — unmeasured,
  // tail in both directions — which lets a test fixture omit what it is
  // not about and still be answered honestly.
  readonly duration_ms?: number | null;
  // Stored size in bytes, `null` when the row carries no recorded size.
  // Same three-valued treatment, and the same optionality, as
  // `duration_ms`.
  readonly file_size_bytes?: number | null;
  // Total pixel count, `null` when nothing measured the row's
  // dimensions. Same three-valued treatment and same optionality again.
  //
  // Already multiplied out by the backend: the axis orders on the
  // product because the two columns behind it are coded dimensions taken
  // before orientation, so neither side alone says how large the picture
  // is. The card never receives the pair, which is what stops a
  // comparator here from quietly sorting by a coded width.
  readonly pixel_count?: number | null;
};

// Per-bucket `max(occurred_at_ms)` over the slice being sorted, feeding
// the `updated` orders so buckets rank by last touch. Backend
// counterpart: `sort_eval::BucketRecency`.
export type BucketRecency = {
  persona: Map<string, number>;
  modality: Map<string, number>;
  tag: Map<string, number>;
  group: Map<string, number>;
};

// Ambient lookups the comparator reads from App-owned catalogs. Passed
// in rather than imported so this module stays pure. Backend
// counterpart: `sort_eval::SortContext`.
export type CardSortLookups = {
  // Persona id → display name; `"?"` when unknown (`formatters.ts`).
  personaName: (id: string) => string;
  // Persona id → sidebar display index; list length when unknown (tail).
  personaDisplayOrder: (id: string) => number;
  // Modality slug → canonical sidebar rank; tail rank when unknown.
  modalityRank: (slug: string) => number;
  // `group_ids[0]` resolved to a name; [`TAIL_SENTINEL`] when unfiled.
  primaryGroupName: (groupIds: readonly string[]) => string;
  // Collation for every alphabetical axis. Build it with
  // `textComparator(activeFilter.sortCollation)` from `./collation`;
  // the backend builds its half from the same `SortSpec.collation`
  // value (`sort_eval::collator_for`).
  compareText: (a: string, b: string) => number;
};

// Builds the recency maps over exactly the slice under sort (the
// pre-sort input), so `updated` reflects what the user actually sees.
export function computeBucketRecency(
  cards: Iterable<SortableCard>,
  lookups: CardSortLookups,
): BucketRecency {
  const persona = new Map<string, number>();
  const modality = new Map<string, number>();
  const tag = new Map<string, number>();
  const group = new Map<string, number>();
  for (const c of cards) {
    const t = c.occurred_at_ms;
    const pt = persona.get(c.persona_id);
    if (pt === undefined || t > pt) persona.set(c.persona_id, t);
    const mkey = c.modality ?? "";
    const mt = modality.get(mkey);
    if (mt === undefined || t > mt) modality.set(mkey, t);
    const tk = firstUserLabel(c.labels);
    const tt = tag.get(tk);
    if (tt === undefined || t > tt) tag.set(tk, t);
    const g = lookups.primaryGroupName(c.group_ids);
    const gt = group.get(g);
    if (gt === undefined || t > gt) group.set(g, t);
  }
  return { persona, modality, tag, group };
}

// Compare-fn factory. `dir` is `+1` / `-1` from `sortReverse`, applied to
// the primary key only — the `occurred_at DESC` tie-break is never
// reversed, so reversing an axis flips the buckets but keeps each bucket
// reading newest-first.
//
// `null` means "leave the incoming order alone". No target reaches it
// today — every member of `SortTarget` is handled below — so it is the
// landing spot for a union member added without a branch here, which is
// the state `msg_count` sat in before it was retired. `sortCards` and the
// App-side caller both treat it as "do not sort", so such an axis would
// degrade quietly; the guard against that is on the wire side, where the
// Rust enum refuses a token it cannot order.
//
// `occurred_at` used to return `null` too, on the grounds that the server
// already emits DESC. It does not always: a page filtered to exactly one
// Group comes back in `asset_bucket.position` order
// (`SqliteAssetRepository::page`), so the shortcut silently handed the
// grid a hand-arranged sequence while the section headers went on
// labelling time buckets — picking `Occurred` produced a list that was
// not in occurred order. The comparator now owns the axis outright, which
// is also what the backend twin has always done
// (`sort_eval::primary_cmp`, `SortTarget::OccurredAt`). Callers that
// *want* the server's position order ask for it explicitly rather than
// inferring it from a comparator that declined to sort.
// `rank` is the `✦ Relevance` axis's key: asset id → position in the
// retriever's answer (0 = best match). Ignored by every other target.
export function buildCardCmp<C extends SortableCard>(
  target: SortTarget,
  order: SortOrder,
  dir: number,
  recency: BucketRecency,
  lookups: CardSortLookups,
  rank?: ReadonlyMap<string, number> | null,
): ((a: C, b: C) => number) | null {
  const tie = (a: C, b: C) => b.occurred_at_ms - a.occurred_at_ms;
  const byOccurred = (a: C, b: C) => dir * (b.occurred_at_ms - a.occurred_at_ms);
  if (target === "occurred_at") {
    return byOccurred;
  }
  if (target === "relevance") {
    // No rank to sort by — the page was fetched without one, or the
    // fetch has not landed yet. Fall back to the default axis rather
    // than to "leave the incoming order alone": the incoming order is
    // the server's, which for a single-Group page is the hand
    // arrangement, and calling that "relevance" would label a sequence
    // the retriever never produced.
    if (!rank || rank.size === 0) return byOccurred;
    return (a, b) => {
      const ra = rank.get(a.id);
      const rb = rank.get(b.id);
      // Out of the shortlist is not "no match": the retriever stopped
      // at its ceiling, and these rows are here because the *exact*
      // predicate selected them. They keep their default order behind
      // the ranked ones instead of being dropped or interleaved.
      if (ra === undefined && rb === undefined) return tie(a, b);
      if (ra === undefined) return dir;
      if (rb === undefined) return -dir;
      if (ra !== rb) return dir * (ra - rb);
      return tie(a, b);
    };
  }
  if (target === "created_at") {
    return (a, b) => dir * (b.created_at_ms - a.created_at_ms);
  }
  if (target === "persona") {
    // "ordered" = sidebar display_order (asc). "updated" = per-persona
    // max(occurred_at_ms) across the filtered slice, so buckets rank by
    // last-touch. Alpha is the remaining option.
    return (a, b) => {
      if (order === "ordered") {
        const ra = lookups.personaDisplayOrder(a.persona_id);
        const rb = lookups.personaDisplayOrder(b.persona_id);
        if (ra !== rb) return dir * (ra - rb);
      } else if (order === "updated") {
        const ra = recency.persona.get(a.persona_id) ?? 0;
        const rb = recency.persona.get(b.persona_id) ?? 0;
        if (ra !== rb) return dir * (rb - ra);
      } else {
        const ka = lookups.personaName(a.persona_id);
        const kb = lookups.personaName(b.persona_id);
        if (ka !== kb) return dir * lookups.compareText(ka, kb);
      }
      return tie(a, b);
    };
  }
  if (target === "modality") {
    return (a, b) => {
      if (order === "ordered") {
        const ra = lookups.modalityRank(a.modality ?? "");
        const rb = lookups.modalityRank(b.modality ?? "");
        if (ra !== rb) return dir * (ra - rb);
      } else if (order === "updated") {
        const ra = recency.modality.get(a.modality ?? "") ?? 0;
        const rb = recency.modality.get(b.modality ?? "") ?? 0;
        if (ra !== rb) return dir * (rb - ra);
      } else {
        if (a.modality !== b.modality) {
          return dir * lookups.compareText(a.modality ?? "", b.modality ?? "");
        }
      }
      return tie(a, b);
    };
  }
  if (target === "tag") {
    // Same two readings the other bucketing axes offer: `alpha` puts the
    // tags in name order, `updated` puts the most recently touched tag
    // first. `updated` used to share the `alpha` comparison, so the two
    // options produced identical grids — switching between them did
    // nothing, which is how a duplicate reads to anyone using it.
    return (a, b) => {
      const ka = firstUserLabel(a.labels);
      const kb = firstUserLabel(b.labels);
      if (order === "updated") {
        const ra = recency.tag.get(ka) ?? 0;
        const rb = recency.tag.get(kb) ?? 0;
        if (ra !== rb) return dir * (rb - ra);
      } else if (ka !== kb) {
        return dir * lookups.compareText(ka, kb);
      }
      return tie(a, b);
    };
  }
  if (target === "group") {
    return (a, b) => {
      const ka = lookups.primaryGroupName(a.group_ids);
      const kb = lookups.primaryGroupName(b.group_ids);
      if (order === "updated") {
        const ra = recency.group.get(ka) ?? 0;
        const rb = recency.group.get(kb) ?? 0;
        if (ra !== rb) return dir * (rb - ra);
      } else if (order === "ordered") {
        // The hand arrangement. Buckets stay in name order — Groups
        // carry no cross-group sequence on the card — and inside each
        // one the cards take their `asset_bucket.position`, which is
        // what the drag gesture writes. Unfiled cards have no slot and
        // already sort into the tail bucket by name, so the sentinel
        // only orders them among themselves before the tie-break.
        if (ka !== kb) return dir * lookups.compareText(ka, kb);
        const pa = a.primary_group_position ?? Number.MAX_SAFE_INTEGER;
        const pb = b.primary_group_position ?? Number.MAX_SAFE_INTEGER;
        if (pa !== pb) return dir * (pa - pb);
      } else {
        if (ka !== kb) return dir * lookups.compareText(ka, kb);
      }
      return tie(a, b);
    };
  }
  if (target === "cover") {
    return (a, b) => {
      const ka = (a.cover ?? "").toLowerCase();
      const kb = (b.cover ?? "").toLowerCase();
      if (ka !== kb) return dir * lookups.compareText(ka, kb);
      return tie(a, b);
    };
  }
  // Playback length, stored size and resolution — one branch, because
  // the ordering is one rule: largest first in the natural direction,
  // rows carrying no value at the tail in *both* directions. The backend
  // folds the same rule into `sort_eval::absent_last_desc`, which the
  // `Rating` axis shares.
  //
  // All three axes are in the grid's Sort dropdown. The first two were
  // not when these branches landed, and the obstacle was upstream of
  // this module: the list path sorts index-derived light rows, which
  // carried no size and no length, so every row compared as absent and
  // the answer came back in tie-break order — an axis that claims to
  // sort and does not, which is what got `msg_count` retired. That was
  // settled by putting the columns on the index row
  // (`AssetIndexEntryDto`) and forwarding them in `indexToLightCard`;
  // `pixel_count` was added to both from the start for the same reason.
  if (target === "duration" || target === "file_size" || target === "pixels") {
    const keyOf =
      target === "duration"
        ? (c: C) => c.duration_ms ?? null
        : target === "file_size"
          ? (c: C) => c.file_size_bytes ?? null
          : (c: C) => c.pixel_count ?? null;
    return (a, b) => {
      const ka = keyOf(a);
      const kb = keyOf(b);
      // `dir` multiplies the value comparison alone. Applying it to the
      // absent cases would park them at one end and flip them to the
      // other on reverse, so "shortest first" would open on a still
      // image — see `sort_eval::absent_last_desc`.
      if (ka === null && kb === null) return tie(a, b);
      if (ka === null) return 1;
      if (kb === null) return -1;
      if (ka !== kb) return dir * (kb - ka);
      return tie(a, b);
    };
  }
  return null;
}

// Convenience wrapper: recency + comparator + sort in one call, matching
// the shape of the backend's `sort_asset_ids`. `App.svelte` keeps the
// pieces separate (it caches `bucketRecency` as its own derived), so
// this exists mainly for tests and for any future caller that wants the
// whole operation.
export function sortCards<C extends SortableCard>(
  cards: readonly C[],
  target: SortTarget,
  order: SortOrder,
  reverse: boolean,
  lookups: CardSortLookups,
  rank?: ReadonlyMap<string, number> | null,
): C[] {
  const recency = computeBucketRecency(cards, lookups);
  const cmp = buildCardCmp<C>(
    target,
    order,
    reverse ? -1 : 1,
    recency,
    lookups,
    rank,
  );
  const out = cards.slice();
  if (cmp) out.sort(cmp);
  return out;
}
