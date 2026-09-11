// URL <-> activeFilter adapter.
//
// Tauri v2 desktop app runs as a single-page bundle over `index.html`
// with no SvelteKit router — this module treats `window.location.search`
// directly. On mount `hydrateFromURL()` seeds `activeFilter` before the
// initial data loads; on subsequent mutations `syncToURL()` writes back
// via `history.replaceState` so a refresh / deep-link reproduces the
// current selection. `navigate` is never invoked because the shell
// window has no session history the user relies on.
//
// Serialised axes:
//   p    activePersona                     (string | null)
//   m    activeModality                    (string | null)
//   fmt  activeFormat                      (string | null)
//   col  activeColor                       (string | null)
//   t    activeTagIds                      (comma-separated ids)
//   g    activeGroupIds                    (comma-separated ids)
//   s    searchText                        (raw string, trimmed)
//   sm   searchFuzzy                       ("e" = exact; fuzzy dropped)
//   tm   tagMatchAll                       ("all" = AND; OR dropped)
//   v    viewMode                          (messages/sessions/groups)
//   day  dayFrom..dayUntil                 (YYYY-MM-DD; an open end is empty)
//   doy  dayOfYear                         (MM-DD)
//   tz   dayTimeZone                       (IANA name; only beside day / doy)
//   sort <target>:<order>[:r]              (default is dropped)
//
// `sm` / `tm` carry *how* a predicate reads, not what is selected, and a
// deep link that drops them would reproduce the same chips against a
// different domain. Both encode only their non-default value, so
// an untouched filter still serialises to the empty string.
//
// `day` carries the dates rather than the date-and-span the picker
// shows, because the dates are what the store holds and what the filter
// selects by (`Filter.dayFrom`). That is what lets a link written under
// a range the picker cannot draw reproduce it exactly. `tz` is written
// whenever a day is, so the link reproduces the set and not only the
// dates: the same days in another zone are a different set.
//
// Intentionally omitted:
// - activeSessionId / activeSessionLabel — drill-in ephemeral, resets on
//   refresh by design.
// - activeLabel — narrow secondary filter, out of scope for MVP.
// - activeTagNames / activeGroupNames — display labels rehydrated from
//   tagCounts / groupCounts once those catalog fetches resolve
//   (App.svelte `$effect` in the wiring section).
// - discoverRandom / randomNonce — the 🎲 draw is not reproducible, so a
//   link naming it would restore the state and not the answer: the same
//   URL would open on different assets each time, which is the one thing
//   a deep link is supposed not to do (same reasoning as
//   `persistableSort()` dropping the relevance axis).

import {
  activeFilter,
  isSortOrder,
  isSortTarget,
  viewerTimeZone,
  type SortTarget,
  type SortOrder,
  type ViewMode,
} from "./filter.svelte";

const KEY_PERSONA = "p";
const KEY_MODALITY = "m";
// FORMAT facet (asset-model v4 P3) — material mime top-level type.
const KEY_FORMAT = "fmt";
// COLOR facet — palette swatch slug.
const KEY_COLOR = "col";
const KEY_TAGS = "t";
const KEY_GROUPS = "g";
const KEY_SEARCH = "s";
// Search mode: `"e"` = 🔍 exact (`text_match`). ✦ fuzzy is the default
// and is never written.
const KEY_SEARCH_MODE = "sm";
const SEARCH_MODE_EXACT = "e";
// Tag composition: `"all"` = AND. OR is the default and is never written.
const KEY_TAG_MATCH = "tm";
const TAG_MATCH_ALL = "all";
const KEY_VIEW = "v";
// Calendar range, `<from>..<until>`; either side may be empty for an
// open end. `..` rather than `-`, which the dates themselves contain.
const KEY_DAY = "day";
const DAY_SEP = "..";
// Day of year, `MM-DD`.
const KEY_DAY_OF_YEAR = "doy";
// The zone the days are read in. Written only beside one of the two
// above; read only then.
const KEY_TIME_ZONE = "tz";
const KEY_SORT = "sort";

const DEFAULT_VIEW: ViewMode = "messages";
const DEFAULT_SORT_TARGET: SortTarget = "occurred_at";
const DEFAULT_SORT_ORDER: SortOrder = "updated";

const VIEW_MODES: readonly ViewMode[] = ["messages", "groups"];

function isViewMode(v: string): v is ViewMode {
  return (VIEW_MODES as readonly string[]).includes(v);
}

// `isSortTarget` / `isSortOrder` come from `filter.svelte` — the URL is
// one of two places a sort axis arrives as an unchecked string (the other
// is a stored Query Group rule), and a second copy of the vocabulary here
// is what has to be remembered when the union changes.

// A calendar date off the query string, in the one form the store
// takes. Shape only: the link path writes the field directly, so
// whether the calendar has the day (`2026-02-30`) is left to the
// backend, which refuses it and says so — a link with a typo should
// say so. A half that is not even the shape reads as an open end
// rather than failing the whole link.
function parseIsoDay(raw: string | undefined): string | null {
  if (raw === undefined) return null;
  const s = raw.trim();
  return /^\d{4}-\d{2}-\d{2}$/.test(s) ? s : null;
}

function parseDayOfYear(raw: string | null): { month: number; day: number } | null {
  if (!raw) return null;
  const m = /^(\d{2})-(\d{2})$/.exec(raw.trim());
  return m ? { month: Number(m[1]), day: Number(m[2]) } : null;
}

function splitCsv(raw: string | null): string[] {
  if (!raw) return [];
  return raw.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
}

// -------------------------------------------------------------------
// Hydrate — one-shot on mount, before the first data fetch.
//
// Names for tags / groups arrive later (via loadTagCounts /
// loadGroupCounts). App.svelte owns a follow-up $effect that reconciles
// activeTagNames / activeGroupNames from those catalog results.
//
// Split into a pure `decodeFromSearch(search)` and a `window`-reading
// wrapper for the same reason `encodeToSearch` is pure: the round trip
// (encode → decode) is the property worth pinning, and pinning it should
// not require a DOM environment.
// -------------------------------------------------------------------
export function hydrateFromURL(): void {
  decodeFromSearch(window.location.search);
}

export function decodeFromSearch(rawSearch: string): void {
  const params = new URLSearchParams(rawSearch);

  const persona = params.get(KEY_PERSONA);
  activeFilter.activePersona = persona && persona.length > 0 ? persona : null;

  const modality = params.get(KEY_MODALITY);
  activeFilter.activeModality = modality && modality.length > 0 ? modality : null;

  const format = params.get(KEY_FORMAT);
  activeFilter.activeFormat = format && format.length > 0 ? format : null;

  // An unknown swatch slug is left to the backend to reject (the list
  // query validates it and surfaces the accepted set) rather than
  // silently dropped here — a deep link with a typo should say so.
  const color = params.get(KEY_COLOR);
  activeFilter.activeColor = color && color.length > 0 ? color : null;

  const tagIds = splitCsv(params.get(KEY_TAGS));
  activeFilter.activeTagIds.clear();
  activeFilter.activeTagNames.clear();
  for (const id of tagIds) activeFilter.activeTagIds.add(id);

  // An unrecognised value reads as the default rather than being
  // rejected: the knob is binary, and a typo'd `tm` should compose the
  // chips the plain way rather than break the link.
  activeFilter.tagMatchAll = params.get(KEY_TAG_MATCH) === TAG_MATCH_ALL;

  const groupIds = splitCsv(params.get(KEY_GROUPS));
  activeFilter.activeGroupIds.clear();
  activeFilter.activeGroupNames.clear();
  for (const id of groupIds) activeFilter.activeGroupIds.add(id);

  const search = params.get(KEY_SEARCH);
  activeFilter.searchText = search ?? "";
  activeFilter.searchFuzzy = params.get(KEY_SEARCH_MODE) !== SEARCH_MODE_EXACT;

  const view = params.get(KEY_VIEW);
  if (view && isViewMode(view)) {
    activeFilter.viewMode = view;
  } else if (view === "sessions") {
    // Migration path: the retired Sessions tab persists
    // in older URLs / bookmarks — fall back to Messages so the app
    // still lands on a valid grid instead of the enum default.
    activeFilter.viewMode = "messages";
  }

  // The range wins over the day-of-year when a hand-edited link names
  // both: the wire refuses the pair, and one cut applied is a grid that
  // says what it is under, where a refused query is a grid that says
  // nothing. A link with neither clears the filter — hydrate runs once
  // over live singleton state, and a cold link must not inherit a day
  // the session had picked.
  const day = params.get(KEY_DAY);
  const dayOfYear = parseDayOfYear(params.get(KEY_DAY_OF_YEAR));
  if (day) {
    const [rawFrom, rawUntil] = day.split(DAY_SEP);
    activeFilter.dayFrom = parseIsoDay(rawFrom);
    activeFilter.dayUntil = parseIsoDay(rawUntil);
    activeFilter.dayOfYear = null;
  } else {
    activeFilter.dayFrom = null;
    activeFilter.dayUntil = null;
    activeFilter.dayOfYear = dayOfYear;
  }
  // The zone the link was written under, when it names one and a day
  // is in force; the viewer's own otherwise. An unknown name is left to
  // the backend to refuse (the query validates it and says so) rather
  // than silently replaced here — a link with a typo should say so.
  const zone = params.get(KEY_TIME_ZONE);
  activeFilter.dayTimeZone =
    activeFilter.hasDayFilter() && zone && zone.length > 0 ? zone : viewerTimeZone();

  const sort = params.get(KEY_SORT);
  if (sort) {
    const parts = sort.split(":");
    const [rawTarget, rawOrder, rawReverse] = parts;
    if (rawTarget && isSortTarget(rawTarget)) {
      activeFilter.sortTarget = rawTarget;
    }
    if (rawOrder && isSortOrder(rawOrder)) {
      activeFilter.sortOrder = rawOrder;
    }
    activeFilter.sortReverse = rawReverse === "r";
  }
}

// -------------------------------------------------------------------
// Encode — pure fn so tests can pin the format without mocking
// `window`. `syncToURL` calls it and pushes the string through
// `history.replaceState`.
// -------------------------------------------------------------------
export function encodeToSearch(): string {
  const params = new URLSearchParams();

  if (activeFilter.activePersona) params.set(KEY_PERSONA, activeFilter.activePersona);
  if (activeFilter.activeModality) params.set(KEY_MODALITY, activeFilter.activeModality);
  if (activeFilter.activeFormat) params.set(KEY_FORMAT, activeFilter.activeFormat);
  if (activeFilter.activeColor) params.set(KEY_COLOR, activeFilter.activeColor);

  if (activeFilter.activeTagIds.size > 0) {
    params.set(KEY_TAGS, Array.from(activeFilter.activeTagIds).join(","));
  }
  // Written independently of the tag list: the state survives dropping
  // back to one chip (see `ActiveFilters`), so a link taken at that
  // moment should reproduce it when a second chip returns.
  if (activeFilter.tagMatchAll) params.set(KEY_TAG_MATCH, TAG_MATCH_ALL);
  if (activeFilter.activeGroupIds.size > 0) {
    params.set(KEY_GROUPS, Array.from(activeFilter.activeGroupIds).join(","));
  }

  const search = activeFilter.searchText.trim();
  if (search.length > 0) params.set(KEY_SEARCH, search);
  if (!activeFilter.searchFuzzy) params.set(KEY_SEARCH_MODE, SEARCH_MODE_EXACT);

  if (activeFilter.viewMode !== DEFAULT_VIEW) {
    params.set(KEY_VIEW, activeFilter.viewMode);
  }

  if (activeFilter.dayOfYear !== null) {
    const pad = (n: number) => String(n).padStart(2, "0");
    const { month, day } = activeFilter.dayOfYear;
    params.set(KEY_DAY_OF_YEAR, `${pad(month)}-${pad(day)}`);
  } else if (activeFilter.dayFrom !== null || activeFilter.dayUntil !== null) {
    const from = activeFilter.dayFrom ?? "";
    const until = activeFilter.dayUntil ?? "";
    params.set(KEY_DAY, `${from}${DAY_SEP}${until}`);
  }
  if (activeFilter.hasDayFilter()) {
    params.set(KEY_TIME_ZONE, activeFilter.dayTimeZone);
  }

  const sortIsDefault =
    activeFilter.sortTarget === DEFAULT_SORT_TARGET &&
    activeFilter.sortOrder === DEFAULT_SORT_ORDER &&
    !activeFilter.sortReverse;
  if (!sortIsDefault) {
    const tail = activeFilter.sortReverse ? ":r" : "";
    params.set(
      KEY_SORT,
      `${activeFilter.sortTarget}:${activeFilter.sortOrder}${tail}`,
    );
  }

  const qs = params.toString();
  return qs.length > 0 ? `?${qs}` : "";
}

// -------------------------------------------------------------------
// Sync — call from an App.svelte `$effect` that reads every persisted
// axis. `replaceState` keeps the shell's history stack empty so the
// user's Back / Forward gesture never navigates away from the app.
// -------------------------------------------------------------------
export function syncToURL(): void {
  const nextSearch = encodeToSearch();
  const { pathname, hash } = window.location;
  const nextUrl = `${pathname}${nextSearch}${hash}`;
  if (window.location.search === nextSearch) return;
  history.replaceState(history.state, "", nextUrl);
}
