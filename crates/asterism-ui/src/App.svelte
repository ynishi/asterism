<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { mutate } from "./lib/mutate";
  import { summariseBulk } from "./lib/bulk-status";
  import { untrack } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onDestroy } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { VList } from "virtua/svelte";
  import ActiveFilters from "./ActiveFilters.svelte";
  import CardActionIcons from "./CardActionIcons.svelte";
  import ConstellationBurst from "./ConstellationBurst.svelte";
  import DetailPane from "./DetailPane.svelte";
  import DiscoverSection from "./DiscoverSection.svelte";
  import DispatchHistoryPanel from "./DispatchHistoryPanel.svelte";
  import DispatchToast from "./DispatchToast.svelte";
  import SnapshotView from "./SnapshotView.svelte";
  import GroupsSection from "./GroupsSection.svelte";
  import JobsTickerBanner from "./JobsTickerBanner.svelte";
  import ModalityList from "./ModalityList.svelte";
  import FormatList from "./FormatList.svelte";
  import ColorList from "./ColorList.svelte";
  import MetricBands from "./MetricBands.svelte";
  import ConfirmModal from "./ConfirmModal.svelte";
  import UndoToast from "./UndoToast.svelte";
  import DuplicatesPanel from "./DuplicatesPanel.svelte";
  import PersonaStrip from "./PersonaStrip.svelte";
  import ProfileCard from "./ProfileCard.svelte";
  import PromptModal from "./PromptModal.svelte";
  import QuickLook from "./QuickLook.svelte";
  import SessionCommentsHover from "./SessionCommentsHover.svelte";
  import SessionTile from "./SessionTile.svelte";
  import SettingsMaintenance from "./SettingsMaintenance.svelte";
  import SettingsModalities from "./SettingsModalities.svelte";
  import SettingsModel from "./SettingsModel.svelte";
  import SettingsPreferences from "./SettingsPreferences.svelte";
  import SharedLinesPanel from "./SharedLinesPanel.svelte";
  import ForgePanel from "./ForgePanel.svelte";
  import SidebarSearch from "./SidebarSearch.svelte";
  import TagList from "./TagList.svelte";
  import { baseName } from "./lib/basename";
  import { perfBaseline } from "./lib/dev/perf-baseline";
  import { recordEvent } from "./lib/telemetry";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { assetPageCatalog } from "./lib/stores/asset-page.svelte";
  import { gridSelection } from "./lib/stores/grid-selection.svelte";
  import { interaction } from "./lib/interaction/mode.svelte";
  import { dispatchCatalog } from "./lib/stores/dispatch.svelte";
  import { sharedCatalog } from "./lib/stores/shared.svelte";
  import { forgeCatalog } from "./lib/stores/forge.svelte";
  import { groupCatalog } from "./lib/stores/group.svelte";
  import { modalityCatalog } from "./lib/stores/modality.svelte";
  import { formatCatalog } from "./lib/stores/format.svelte";
  import { beginDrag, cardDrag, type DragSource, type DropTarget } from "./lib/interaction/drag.svelte";
  import { colorCatalog } from "./lib/stores/color.svelte";
  import { confirmCatalog } from "./lib/stores/confirm.svelte";
  import { undoToastCatalog } from "./lib/stores/undo-toast.svelte";
  import { personaCatalog } from "./lib/stores/personas.svelte";
  import { profileCatalog } from "./lib/stores/profile.svelte";
  import { promptCatalog } from "./lib/stores/prompt.svelte";
  import { sessionCatalog } from "./lib/stores/session.svelte";
  import {
    SETTING_KEYS,
    settingsCatalog,
  } from "./lib/stores/settings.svelte";
  import { tagCatalog } from "./lib/stores/tag.svelte";
  import { themeCatalog } from "./lib/stores/theme.svelte";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";
  import { threadsCatalog } from "./lib/stores/threads.svelte";
  import ThreadDrawer from "./ThreadDrawer.svelte";
  import { hydrateFromURL, syncToURL } from "./lib/stores/url-adapter";
  import {
    TAIL_SENTINEL,
    buildCardCmp,
    computeBucketRecency,
    firstUserLabel,
    type CardSortLookups,
  } from "./lib/sort/card-cmp";
  import { textComparator } from "./lib/sort/collation";
  import {
    fmtDateTime,
    noteAuthorLabel,
    personaName,
    renderMarkdown,
  } from "./lib/formatters";
  import type {
    AssetCardDto,
    AssetCommentDto,
    AssetDetailDto,
    AssetDto,
    AssetPageDto,
    AssetTextDto,
    AttachTagBatchResult,
    ConstellationItemDto,
    DetachTagBatchResult,
    DirDto,
    EmptyTrashResult,
    GroupSummaryDto,
    PersonaProfileDto,
    PersonaThemeDto,
    CreateQueryGroupCommand,
    PromoteSnapshotToGroupResult,
    SessionDto,
    UpdateAssetMetaBatchResult,
  } from "./bindings";

  // Seed `activeFilter` from `window.location.search` before any state
  // reads it. Runs at script-body time (Tauri v2 has no SSR — `window`
  // is always available), so the initial data fetches downstream see
  // the URL-derived selection instead of the class defaults. Tag /
  // group display names arrive later; a follow-up `$effect` reconciles
  // them once `tagCounts` / `groupCounts` resolve.
  hydrateFromURL();

  // v1 grid: persona sidebar + modality tabs + a dense grid whose cards
  // trigger a small constellation-burst panel on hover. Virtualisation
  // (virtua / svelte-virtual-list) is deferred until scrolling gets slow
  // — the lightweight card projection keeps rendering costs down for now.

  // The Modality sidebar axis is backend-authoritative: the ordered
  // (slug, label, terminal, sort_order, hidden) rows come from the
  // `modality` master via `modalityCatalog.list` (loaded once on
  // mount, below). The sidebar reads `.visible` (master rows plus any
  // slug present in the counts), while the bulk "Move to Modality…"
  // menu reads registered rows only — you can browse a bucket you
  // cannot file into. The old frontend-declared enum +
  // `localStorage` order (`asterism.modality_order.v1`) is retired —
  // drag-reorder persists through `modalityCatalog.reorder` onto the
  // master `sort_order`.

  // Free-form role slugs that a chat message can carry. Anything a
  // parser puts here (via `ChatRole::Other`) shows up as a label; the
  // sidebar only surfaces the four well-known ones.
  const ROLES = [
    ["user", "You"],
    ["assistant", "Assistant"],
    ["system", "System"],
    ["tool", "Tool"],
  ] as const;

  // `personas` array moved to `personaCatalog.list` (personas.svelte.ts).
  // activeFilter.activePersona moved to `activeFilter.activeFilter.activePersona`.
  // Persona-scoped wallpaper (theme row + resolved blob URL) moved
  // to `themeCatalog` (theme.svelte.ts, Phase C wave 8a). Reads
  // stay reactive through `themeCatalog.theme` / `.wallpaperUrl`;
  // the `$effect` that drives `loadFor(activePersona)` and the
  // `setAsWallpaper` / `clearWallpaper` mutations remain in App
  // because the status-line update is UI chrome the store has no
  // business owning.
  // Sidebar Profile card. The `profiles` map + avatar-thumb cache
  // moved to `profileCatalog` (profile.svelte.ts) in wave 8b so
  // PersonaStrip / the ProfileCard modal / the card-thread head
  // share a single decode budget. `profileCard` (which persona
  // the hover-anchored modal is currently open next to) stays in
  // App because the sidebar-side hover timers own its lifetime;
  // the edit-form buffer (`bio_short` / `role_tag` inputs) lives
  // inside the ProfileCard component now, so App no longer tracks
  // it.
  let profileCard = $state<{ personaId: string; x: number; y: number } | null>(
    null,
  );
  let profileCloseTimer: number | null = null;

  // Card action-icon menu — the strip of Eagle-style icons that
  // floats inside the card while the pointer is over it. Hovering
  // an icon opens the corresponding panel; the card itself no
  // longer auto-opens the Thread overlay, so a bare pass-through
  // does not disturb the grid.
  let cardThreadHover = $state<{
    assetId: string;
    x: number;
    y: number;
    comments: AssetCommentDto[];
    draft: string;
    authorKind: "user" | "persona";
    posting: boolean;
  } | null>(null);
  // Note overlay — shared between grid Card (asset register_note) and
  // SessionsView tile (session note metadata). Kind discriminates the
  // save path (asset UpdateAssetMetaCommand vs session patch_metadata)
  // so the overlay UI itself stays uniform: Messages and Sessions are
  // treated as the same kind of Card — a note looks and behaves the
  // same on both.
  let cardNoteHover = $state<{
    kind: "asset" | "session";
    targetId: string;
    x: number;
    y: number;
    draft: string;
    saving: boolean;
  } | null>(null);
  let cardActionCloseTimer: number | null = null;
  const CARD_ACTION_CLOSE_GRACE_MS = 250;
  // Session comment aggregation panel — spawned from a SessionsView
  // tile's 💬 icon. Coord fields mirror the Note / Thread overlays so
  // it lands relative to the tile it opened from. It currently reuses
  // the existing `asset_comment` fetch via `SessionCommentsHover`; the
  // data source swaps once a SessionComment model lands.
  let sessionCommentsHover = $state<{
    sessionId: string;
    x: number;
    y: number;
  } | null>(null);
  // Grace period after the pointer leaves a persona row before the
  // card dismisses. Long enough to travel the ~6 px gap from row
  // to card without the card blinking out, short enough that
  // wandering into the grid clears it promptly.
  const PROFILE_CLOSE_GRACE_MS = 180;
  // Avatar thumb cache moved to `profileCatalog` (see above).
  // activeFilter.activeModality / activeFilter.activeLabel moved to `activeFilter.*`.
  // Multi-tag OR filter (any-of): an asset needs to carry at least
  // one of these tags to pass. `SvelteSet` / `SvelteMap` are the
  // reactive Set/Map primitives from `svelte/reactivity` — every
  // .add() / .delete() / .set() / .clear() call trips reactivity
  // natively, so the sidebar / grid header / detail chip states
  // stay in sync without a manual tick counter.
  // activeFilter.activeTagIds / activeFilter.activeTagNames moved to `activeFilter.*` (SvelteSet /
  // SvelteMap instances on the class).
  // Tags catalog (counts + name lookup) moved to `tagCatalog`
  // (tag.svelte.ts). The sidebar-section UX state (expand / free-text
  // filter / render cap / visibleTagCounts) lives on `TagList.svelte`
  // — it is ephemeral to the section and does not need to reach App.

  // Groups catalog (counts + dirs + group-in-group links + tree
  // derivations) moved to `groupCatalog` (group.svelte.ts).
  // Groups sidebar section (create / delete / rename forms +
  // expanded-disclosure sets + sidebar drag state + Groups + Dirs
  // template) moved to `GroupsSection.svelte` (wave 5b-2).
  // activeFilter.activeGroupIds / activeFilter.activeGroupNames moved to `activeFilter.*`.
  // Selector (grid multi-select `gridSelection.selectedIds` / `gridSelection.lastAnchorId` +
  // `restore`) moved to `gridSelection`
  // (grid-selection.svelte.ts). Click-gesture
  // interpretation (`onCardClick`) stays here, template-adjacent.
  // Registered exporter slugs the current build ships (fetched once
  // on mount; the action bar renders one option per entry).
  let exporterSlugs = $state<string[]>([]);
  // Recent Selections for the active persona moved to
  // `selectionCatalog` (selection.svelte.ts, Phase C wave 9).
  // `SelectionsList` consumes the store directly; App keeps
  // `restoreSelection` because the mutation fans into
  // `gridSelection.selectedIds` / `gridSelection.lastAnchorId` (grid-adjacent state).
  // Sidebar count aggregations — total asset count per persona
  // (never scoped, matches Group / Tag "all assets" semantic) and
  // per modality (scoped to `activeFilter.activePersona`, so switching persona
  // narrows the modality tallies to that persona's slice).
  // `personaCounts` moved to `personaCatalog.counts` (personas.svelte.ts).
  // `modalityCounts` moved to `modalityCatalog.counts` (modality.svelte.ts).
  // Pinned SavedQueries + detail modal state moved to
  // `savedQueryCatalog` (saved-query.svelte.ts, Phase C wave 7).
  // `SavedQueriesList` / `SavedQueryDetailModal` consume the
  // store directly; App keeps the mutation wrappers
  // (`saveCurrentQuery` / `restoreSavedQuery` / `deleteSavedQuery`)
  // because they need `customPrompt` / `currentFilter()` /
  // `dispatchStatus` / the activeFilter fanout — surfaces the
  // store has no business owning.

  // In-flight dispatch monitoring + jobs-pipeline observability
  // moved to `dispatchCatalog` (dispatch.svelte.ts, Phase C wave
  // C). `DispatchToast` + `JobsTickerBanner` consume the store;
  // App keeps the 3-s `$effect` cadence that drives
  // `refreshJobsSnapshot()` (component lifecycle surface).

  // Inline prompt modal — Tauri v2 macOS WKWebView renders
  // `window.prompt()` as a silent `null` return, so the Selector
  // action bar has to bring its own text-input surface. The state
  // is `null` while no prompt is active; setting it to a request
  // renders the overlay and captures the user's answer through the
  // `resolve` callback.
  // Prompt modal state moved to `promptCatalog` (prompt.svelte.ts,
  // Phase C wave A). Callsites now `await promptCatalog.open(...)`.
  // `PromptModal.svelte` handles render + focus timing.

  // Drag-reorder is only meaningful when exactly one group is the
  // active filter — the "browsing one collection" mode where a
  // user-chosen order exists. The carried card and the row under the
  // pointer both live in `cardDrag` now.
  // View mode: "messages" = one tile per asset (current), "groups" =
  // dir/group drill-down. The retired "sessions" mode is gone
  // altogether; Session tiles now surface inside the Messages grid
  // whenever the active modality is
  // "dialogue", with a "Show messages" toggle that interleaves the
  // per-message tiles alongside the Session tiles.
  // activeFilter.viewMode moved to `activeFilter.activeFilter.viewMode`.

  // The "Show messages" toggle and the `dialogueSessionMode`
  // interleave are gone, and so is the `top_level` filter that replaced
  // them: one Asset is one Card. A message is a card, its container is
  // a card, and MODALITY (Session / Message) is what separates them —
  // a browsing choice rather than a visibility rule baked into the
  // query.

  // Card-level "is this an image?" — read off `media`, the slug the
  // backend's single render policy already decided
  // (`domain::render::render_policy`).
  //
  // This used to re-derive the answer here with
  // `mime.startsWith("image/")`, which meant the rule lived in two
  // places and only one of them could learn anything: an `image/*`
  // subtype the backend names but this file does not, or a container
  // (which owns no bytes and gets no player whatever its mime says),
  // were both invisible from here.
  function cardIsImage(card: { media?: string }): boolean {
    return card.media === "image";
  }

  // Card-level "does this show a picture?" — true for images and for
  // videos, which carry an extracted frame in the same thumb cache.
  // Distinct from `cardIsImage`, which asks whether the *original
  // file* is an image: wallpaper / avatar take the source path
  // directly, and a `.mp4` is no use to them.
  function cardIsVisual(card: { media?: string }): boolean {
    return card.media === "image" || card.media === "video";
  }

  // F4 grid sort axis (Phase 1). Client-side reordering of the
  // already-fetched page — server API stays untouched. The default
  // "recency" preserves the historical `occurred_at` DESC order the
  // server delivers; the other axes bucket by the field first and
  // then fall back to `occurred_at` DESC so items inside a bucket
  // still land newest-first (Lightroom-style grid segment order).
  // - `tag` = first user-facing `labels[0]`, skipping the internal
  //   `persona:` / `journal_kind:` prefixes so buckets read as tag
  //   names, not routing metadata.
  // - `group` uses `card.group_ids[0]` resolved through
  //   `groupNameById` — the server enriches every card via a
  //   single bulk `SELECT` on `asset_bucket`, so no extra round
  //   trip is needed. Cards not filed into any group land in the
  //   `(no group)` bucket, sorted after the named ones.
  // Two-axis sorter: `activeFilter.sortTarget` picks the dimension (occurred /
  // added / persona / modality / tag / group / cover),
  // `activeFilter.sortOrder` picks the direction inside that dimension (alpha /
  // ordered / updated), and `activeFilter.sortReverse` flips whichever order
  // came out. The old single-`sortMode` union is kept as a legacy
  // alias for the persisted state (see `sortMode` derived below).
  // SortTarget / SortOrder types moved to
  // `./lib/stores/filter.svelte`; re-import so local references keep
  // working. activeFilter.sortTarget / activeFilter.sortOrder / activeFilter.sortReverse moved to
  // `activeFilter.*`.
  type SortTarget = import("./lib/stores/filter.svelte").SortTarget;
  type SortOrder = import("./lib/stores/filter.svelte").SortOrder;

  // Which wire-level orders each target can express. The bucketing
  // targets (persona / modality / tag / group) offer a real choice of
  // bucket sequence; `tag` has no domain-native order of its own, so it
  // gets two of the three rather than all.
  //
  // On the time axes and `cover` the target *is* the order, so the lone
  // token is a wire-format placeholder (`SortSpec.order` is not
  // nullable) that the comparator ignores — those axes still offer both
  // directions, which is what the user actually picks between there.
  // The dropdown the user sees is built from this by `orderChoicesFor`,
  // which pairs each allowed order with its two directions.
  const ORDER_OPTIONS: Record<SortTarget, SortOrder[]> = {
    occurred_at: ["updated"],
    created_at: ["updated"],
    persona: ["ordered", "alpha", "updated"],
    modality: ["ordered", "alpha", "updated"],
    tag: ["alpha", "updated"],
    group: ["ordered", "alpha", "updated"],
    cover: ["alpha"],
    // Length and size: the target is the ordering, so the lone token is
    // the same wire placeholder the time axes carry. Both directions
    // stay available — that is the choice the user makes here (longest
    // vs shortest, largest vs smallest), and `DURATION_CHOICES` /
    // `SIZE_CHOICES` below name them in those words rather than the time
    // axis's "Newest / Oldest".
    //
    // Both are offered in the Sort dropdown, which they were not until
    // the index row started carrying the two columns: the grid sorts
    // index rows, so while those rows had no size and no length every
    // pick compared absent values and answered in `occurred_at DESC` —
    // an axis that claims to sort and does not. `indexToLightCard`
    // forwards them now (`AssetIndexEntryDto.duration_ms` /
    // `file_size_bytes`), so what the user picks is what the grid does.
    duration: ["updated"],
    file_size: ["updated"],
    // Resolution, on the same terms — a continuous key whose target is
    // its own ordering, so the lone token is the wire placeholder again
    // and `PIXEL_CHOICES` names the two directions in its own words.
    pixels: ["updated"],
    // The rank is the whole ordering, so there is one reading of it.
    // `ordered` is the token that says "a sequence someone else
    // determined" — here the retriever rather than the user's hand — and
    // it never reaches the wire (`relevance` is frontend-only), so no
    // stored rule can carry the pair.
    relevance: ["ordered"],
  };
  // What the dropdown shows for each order. The wire tokens say how the
  // comparator is implemented, not what the user gets: `alpha` names an
  // algorithm, and `updated` reads as "by updated_at" — a column no
  // comparator touches.
  //
  // The Order dropdown carries direction; there is no separate reverse
  // button. Two controls that each own part of the ordering is what
  // produced the contradiction this replaced: a static `A→Z` is a lie
  // the moment a reverse toggle flips it, and a label that flips with
  // the toggle breaks Nielsen #4 (an option's text must not change
  // because another control moved). Folding direction into the option
  // removes the second owner, so the text can state the whole ordering
  // and stay true.
  //
  // The full Sort × Order cross product would be dozens of entries, so
  // the sort *key* stays in its own dropdown and only the ordering
  // folds — that keeps every list to 6 or fewer, inside the 10-item
  // ceiling design systems put on sort dropdowns.
  //
  // This sequence is canonical: `orderChoicesFor` filters it, never
  // reorders it, so an option sits in the same place on every axis that
  // offers it. Axes that cannot express an ordering omit it rather than
  // showing it greyed — only the sort types that apply are worth
  // listing.
  type OrderChoice = {
    value: string;
    label: string;
    order: SortOrder;
    reverse: boolean;
  };
  const ORDER_CHOICES: readonly OrderChoice[] = [
    { value: "alpha:asc", label: "A→Z", order: "alpha", reverse: false },
    { value: "alpha:desc", label: "Z→A", order: "alpha", reverse: true },
    { value: "updated:asc", label: "Newest first", order: "updated", reverse: false },
    { value: "updated:desc", label: "Oldest first", order: "updated", reverse: true },
    // The hand arrangement: sidebar order for persona / modality, card
    // positions for group. "Manual" named the mechanism; "as arranged"
    // names what the user did, which is the thing they are choosing
    // between. No `1→N` gloss — position numbers are an internal fact
    // (`asset_bucket.position`) that never appears on screen, so the
    // notation explains one unknown with another.
    { value: "ordered:asc", label: "As arranged", order: "ordered", reverse: false },
    {
      value: "ordered:desc",
      label: "As arranged, reversed",
      order: "ordered",
      reverse: true,
    },
  ];
  // Relevance borrows the `ordered` token (see `ORDER_OPTIONS`) but not
  // its labels: "As arranged" names the *user's* arrangement, and this
  // sequence is the retriever's. Same `<order>:<dir>` values as the
  // canonical list so `currentOrderValue` and the `\` flip keep working
  // unchanged.
  const RELEVANCE_CHOICES: readonly OrderChoice[] = [
    { value: "ordered:asc", label: "Best match first", order: "ordered", reverse: false },
    { value: "ordered:desc", label: "Best match last", order: "ordered", reverse: true },
  ];
  // The metric axes borrow the `updated` token (see `ORDER_OPTIONS`) and
  // none of its labels: "Newest first" would name a time ordering on an
  // axis that reads a length or a byte count. Same `<order>:<dir>`
  // values as the canonical list, so `currentOrderValue` and the `\`
  // flip keep working unchanged.
  const DURATION_CHOICES: readonly OrderChoice[] = [
    { value: "updated:asc", label: "Longest first", order: "updated", reverse: false },
    { value: "updated:desc", label: "Shortest first", order: "updated", reverse: true },
  ];
  const SIZE_CHOICES: readonly OrderChoice[] = [
    { value: "updated:asc", label: "Largest first", order: "updated", reverse: false },
    { value: "updated:desc", label: "Smallest first", order: "updated", reverse: true },
  ];
  // Resolution reads in the same words as size — "largest" is the
  // honest gloss for a pixel count, where "highest" would suggest a
  // vertical measurement and "widest" would name one coded side, which
  // is precisely the reading this axis exists to avoid.
  const PIXEL_CHOICES: readonly OrderChoice[] = [
    { value: "updated:asc", label: "Largest first", order: "updated", reverse: false },
    { value: "updated:desc", label: "Smallest first", order: "updated", reverse: true },
  ];
  function orderChoicesFor(target: SortTarget): OrderChoice[] {
    if (target === "relevance") return [...RELEVANCE_CHOICES];
    if (target === "duration") return [...DURATION_CHOICES];
    if (target === "file_size") return [...SIZE_CHOICES];
    if (target === "pixels") return [...PIXEL_CHOICES];
    const allowed = ORDER_OPTIONS[target];
    return ORDER_CHOICES.filter((c) => allowed.includes(c.order));
  }
  // The `(order, reverse)` pair the wire carries, as the select's value.
  // `SortSpec` keeps its two fields — this is a presentation join, not a
  // model change.
  let currentOrderValue = $derived(
    `${activeFilter.sortOrder}:${activeFilter.sortReverse ? "desc" : "asc"}`,
  );
  // Auto-adjust the ordering when the target changes so an impossible
  // combination (e.g. `occurred_at + alpha`) never stays selected.
  // Snaps to the target's first choice, direction included — the pair is
  // what the dropdown selects now, so leaving `sortReverse` where it was
  // would land on an option the user did not pick.
  $effect(() => {
    const opts = ORDER_OPTIONS[activeFilter.sortTarget];
    if (!opts.includes(activeFilter.sortOrder)) {
      const first = orderChoicesFor(activeFilter.sortTarget)[0];
      untrack(() => {
        activeFilter.sortOrder = first.order;
        activeFilter.sortReverse = first.reverse;
      });
    }
  });

  // Canonical ordering the sidebar shows the modalities in — follows
  // the master `sort_order` (via `modalityCatalog.rank`) rather than
  // pure alphabet so the domain-native reading stays legible. Slugs
  // missing from the master drop to the end and fall back to
  // alphabetical among themselves. Kept as a thin wrapper so it can
  // still be handed to `SessionsView` as a `(slug) => number` prop.
  function modalityRank(slug: string): number {
    return modalityCatalog.rank(slug);
  }

  // "Focused Dir" — when a sidebar Dir row is clicked, its
  // sub-dirs and immediate Groups surface as a horizontal lane
  // above the Messages grid so the user can drill sideways
  // without losing the current asset view. `null` = no lane.
  // Root ("Root") is a virtual key that mirrors the sidebar's
  // top level. Set to a real dir id to zoom in one level.
  let focusedDirId = $state<string | null>(null);

  // Sessions view renders the server order (`started_at_ms` DESC)
  // as-is; the old `sortedSessions` derived (grid-era client
  // re-sort) is gone, and `SessionsView.svelte` carries no
  // comparator.

  // Occurred-at range across the filtered page, shown next to the
  // item count so the user can see at a glance which slice of
  // history the grid currently covers.
  //
  // The date format on individual cards is `MM-DD` (compact enough
  // to sit inside a tile), but for the range we always show the
  // year — otherwise `11-15 → 09-20` looks like it goes backwards
  // in time when the newest item just happens to be earlier in the
  // calendar than the oldest one.
  function fmtDateYear(ms: number): string {
    const d = new Date(ms);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  }
  let pageDateRange = $derived.by(() => {
    const src = filteredItems as AssetCardDto[];
    if (src.length === 0) return "";
    let min = src[0].occurred_at_ms;
    let max = src[0].occurred_at_ms;
    for (let i = 1; i < src.length; i++) {
      const t = src[i].occurred_at_ms;
      if (t < min) min = t;
      if (t > max) max = t;
    }
    const from = fmtDateYear(min);
    const to = fmtDateYear(max);
    if (from === to) return `range: ${from}`;
    // Present the range oldest→newest so the arrow reads
    // chronologically regardless of the grid's current sort axis.
    return `range: ${from} → ${to}`;
  });

  // `firstUserLabel` / `INTERNAL_LABEL_PREFIXES` moved to
  // `./lib/sort/card-cmp` (imported above) — the tag-axis sort key and
  // the grid's tag section headers now read the same extracted helper
  // the backend mirrors.
  // Optional session drill-in — when a Sessions tile is clicked the
  // UI switches back to Messages view with this filter applied so
  // the user sees just that session's assets.
  // activeFilter.activeSessionId / activeFilter.activeSessionLabel moved to `activeFilter.*`.
  // Free-text search input. Empty = list mode (fast); non-empty flips
  // the query to `search_assets` (Tantivy BM25 over the indexed body,
  // intersected server-side with the active filter chips).
  // activeFilter.searchText moved to `activeFilter.activeFilter.searchText`.
  // Messages-view page + viewport hydration cache moved to
  // `assetPageCatalog` (asset-page.svelte.ts, wave ①).
  // Sessions view page moved to `sessionCatalog.page` (wave 6).
  // Global indicator for the `SessionRebuild` job. Backend emits
  // `sessions:progress` at start and end so this reflects any
  // in-flight rebuild whether the caller was startup drift, an
  // Import batch auto-enqueue, or an explicit user request.
  let sessionRebuildActive = $state(false);

  // Live job-pipeline observability moved to `dispatchCatalog`
  // (jobsSnapshot + activeKindGauges + refreshJobsSnapshot).
  // The `jobs:tick` per-event accumulator (`noteJobTick` /
  // `jobTicks` / `jobTickerBanner`) is dropped as dead code in
  // wave C — the derived was never consumed by any template,
  // and the DB-poll `activeKindGauges` is the only live source
  // the header ticker ever read. If a per-event burn-down is
  // wanted later, rebuild against the store's snapshot cadence.
  $effect(() => {
    // Kick a first fetch on mount, then keep polling. 3 s cadence
    // is loose enough that the query doesn't compete with real
    // work but tight enough that the gauge tracks the burn-down.
    let cancelled = false;
    void dispatchCatalog.refreshJobsSnapshot();
    const t = setInterval(() => {
      if (cancelled) return;
      void dispatchCatalog.refreshJobsSnapshot();
    }, 3000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  });

  // Dispatch-history drawer reload (W6). Triggers on drawer
  // open / persona flip / state-filter change; while the drawer is
  // open we also poll at 3 s so a running job's row lifecycle chip
  // updates without a manual refresh. Skip entirely on "all
  // personas" because the panel refuses to render a cross-persona
  // history (the sidebar corner is inherently persona-scoped).
  $effect(() => {
    if (!dispatchCatalog.historyOpen) return;
    if (activeFilter.activePersona === null) return;
    void activeFilter.activePersona; // reload on flip
    void dispatchCatalog.historyStateFilter;
    void dispatchCatalog.history.load({
      personaId: activeFilter.activePersona,
      snapshotId: null,
      stateSlug: dispatchCatalog.historyStateFilter,
      limit: 100,
    });
    let cancelled = false;
    const t = setInterval(() => {
      if (cancelled) return;
      void dispatchCatalog.history.load({
        personaId: activeFilter.activePersona,
        snapshotId: null,
        stateSlug: dispatchCatalog.historyStateFilter,
        limit: 100,
      });
    }, 3000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  });
  // `anchor` pins the burst next to the card it was fired from —
  // viewport coords computed by `openConstellationAt`, the sole
  // burst open path since the W1 hover regrammar retired the
  // corner-placement hover path.
  // `pinned` (W4): ✦ click / ⇧Space set it — the panel survives
  // pointer leave for unhurried reading, released by Esc or a
  // second ✦ click / ⇧Space on the same card.
  let burst = $state<{
    assetId: string;
    items: ConstellationItemDto[];
    anchor: { x: number; y: number };
    pinned: boolean;
  } | null>(null);
  // Single close point so the "pinnedBurst" stack entry can never
  // outlive the panel.
  function closeBurst() {
    interaction.remove("pinnedBurst");
    burst = null;
  }

  // ---- DetailPane bridge (extracted 2026-07-20 L3 pilot) ----
  // Reader stage still uses this type on its own render-mode chip.
  type DetailMode = "md" | "raw" | "html" | "term";

  // Signal to DetailPane: which asset to show. Grid card click,
  // provenance chip, session drill-in, constellation burst — all
  // funnel through `openDetail(id)` which just sets this state.
  let openAssetId = $state<string | null>(null);

  // The out-of-band asset-change signal for DetailPane's LRU cache
  // moved to `assetPageCatalog.invalidations` / `.invalidateDetail`
  // (wave ①) — DetailPane / GroupsSection consume it via the
  // catalog import instead of props.

  // Handle exposed by DetailPane via bind:this — App reaches back
  // to trigger arrow-key navigation while the overlay / fullscreen
  // stage is up (DetailPane owns the fullscreen flag so the
  // image-only filter stays inside the component).
  let detailPaneRef = $state<{
    navigate?: (delta: number) => void;
    isFullscreen?: () => boolean;
    exitFullscreen?: () => void;
    isOpen?: () => boolean;
    getModality?: () => string | null;
    handleImageShortcut?: (key: string) => boolean;
  } | null>(null);

  // ---- Quick Look (Space peek tier, W3) ----
  // The read-only glance between the grid card and DetailPane.
  // Space toggles it for the hovered / selected card, ←/→ retarget
  // (moving the selection with it, Finder semantics), Enter
  // escalates to detail, ⇧Space opens the constellation instead.
  let quickLook = $state<{ assetId: string } | null>(null);
  let quickLookText = $state<string | null>(null);
  let quickLookTextLoading = $state(false);

  // Quick Look text LRU (Detail cache pattern, DetailPane.svelte
  // "Detail LRU cache"). Space toggle + ←/→ retarget are frequent
  // enough that a plain re-fetch on every open feels laggy — the
  // IPC + SQLite round-trip is the dominant cost, not the marked +
  // DOMPurify render. Insertion-order `Map` doubles as LRU: hit
  // deletes + re-sets so the touched entry sits at the tail. Out-of-
  // band asset changes (drag-drop into a group, meta edit) purge
  // via `assetPageCatalog.invalidations` — same signal Detail uses.
  const QL_TEXT_CACHE_MAX = 50;
  const quickLookTextCache = new Map<string, string | null>();
  function qlTextCachePut(id: string, text: string | null) {
    if (quickLookTextCache.has(id)) quickLookTextCache.delete(id);
    quickLookTextCache.set(id, text);
    while (quickLookTextCache.size > QL_TEXT_CACHE_MAX) {
      const first = quickLookTextCache.keys().next().value;
      if (first === undefined) break;
      quickLookTextCache.delete(first);
    }
  }
  function qlTextCacheGet(id: string): string | null | undefined {
    if (!quickLookTextCache.has(id)) return undefined;
    const text = quickLookTextCache.get(id) ?? null;
    quickLookTextCache.delete(id);
    quickLookTextCache.set(id, text);
    return text;
  }
  let lastQlInvalidationTick = 0;
  $effect(() => {
    const { id, tick } = assetPageCatalog.invalidations;
    if (tick > lastQlInvalidationTick && id) {
      lastQlInvalidationTick = tick;
      untrack(() => quickLookTextCache.delete(id));
    }
  });

  // Target resolution reuses the bulk-ops `cardById` (hydration
  // cache first, then the light page item — declared next to the
  // bulk handlers below).
  let quickLookCard = $derived.by(() =>
    quickLook ? (cardById(quickLook.assetId) ?? null) : null,
  );
  // Zombie guard (review M2): a background reload / query-group
  // refresh can drop the target card from the page while the peek
  // is up. The panel unmounts on `card == null`, so the state and
  // the "preview" stack entry must follow — otherwise aim-hover
  // stays suppressed with nothing visible on screen.
  $effect(() => {
    if (quickLook !== null && quickLookCard === null) closeQuickLook();
  });

  // The Space target: the card under the pointer wins (peek what
  // you look at), else the selection anchor, else the first card in
  // the current order — Space should always answer with *something*
  // visible instead of silently no-oping (2026-07-24 dogfood).
  function quickLookTargetId(): string | null {
    return (
      hoveredCardId ?? gridSelection.lastAnchorId ?? flatCardIds()[0] ?? null
    );
  }

  function openQuickLook(id: string) {
    quickLook = { assetId: id };
    interaction.push("preview");
    // Quick Look drives the selection (Finder semantics): the peeked
    // card is the selected card, so rating keys / context menu keep
    // one consistent target.
    gridSelection.selectedIds.clear();
    gridSelection.selectedIds.add(id);
    gridSelection.lastAnchorId = id;
    // Cache hit paints the body immediately; the background refetch
    // still runs to reconcile against server-side mutations (Detail
    // cache pattern). Miss = clear + spinner, fetch fills both.
    const cached = qlTextCacheGet(id);
    if (cached !== undefined) {
      quickLookText = cached;
      quickLookTextLoading = false;
      void loadQuickLookText(id, true);
    } else {
      void loadQuickLookText(id, false);
    }
  }
  function closeQuickLook() {
    quickLook = null;
    interaction.remove("preview");
  }
  function toggleQuickLook() {
    if (quickLook) {
      closeQuickLook();
      return;
    }
    const id = quickLookTargetId();
    if (id !== null) openQuickLook(id);
  }
  function navigateQuickLook(delta: number) {
    if (!quickLook) return;
    const order = flatCardIds();
    const at = order.indexOf(quickLook.assetId);
    const next = at >= 0 ? order[at + delta] : undefined;
    if (next) openQuickLook(next);
  }
  async function loadQuickLookText(id: string, silent = false) {
    // `silent` mode is the cache-hit background refresh: the panel
    // is already painted from cache, so we skip the clear + spinner
    // and only reconcile the body once the fetch resolves.
    if (!silent) {
      quickLookText = null;
      quickLookTextLoading = true;
    }
    const card = cardById(id);
    // Nothing textual behind a picture — that holds for a video as
    // much as for a still, so neither pays for the text fetch.
    if (!card || cardIsVisual(card)) {
      if (!silent) quickLookTextLoading = false;
      return;
    }
    try {
      const texts = await invoke<AssetTextDto[]>("asset_texts", {
        assetIds: [id],
      });
      const body = texts[0]?.text ?? null;
      qlTextCachePut(id, body);
      // Drop a stale resolve when the target moved on (←/→ mid-fetch).
      if (quickLook?.assetId === id) {
        quickLookText = body;
      }
    } catch (error) {
      console.warn("asset_texts failed", error);
    } finally {
      // A stale resolve must not clear the flag under a newer
      // in-flight fetch (review L1) — only the current target's
      // fetch owns the loading state.
      if (!silent && quickLook?.assetId === id) quickLookTextLoading = false;
    }
  }

  function openDetail(id: string) {
    // Escalating from (or clicking through) a Quick Look closes it —
    // detail supersedes the peek tier.
    if (quickLook) closeQuickLook();
    openAssetId = id;
    interaction.push("detail");
    recordEvent("asset_open", {
      personaId: activeFilter.activePersona,
      payload: { asset_id: id },
    });
  }
  function closeDetail() {
    openAssetId = null;
    interaction.remove("detail");
  }
  async function navigateDetail(delta: number) {
    detailPaneRef?.navigate?.(delta);
  }
  // Session Reader — the drilled-in session as one continuous
  // transcript (chronological, full message bodies resolved from the
  // original source files via `asset_texts`; covers are only a
  // 200-char snippet so they serve as the fallback, not the body).
  let readerOpen = $state(false);
  let readerLoading = $state(false);
  // Duplicates panel — the byte-identical-originals work list. An
  // overlay rather than a view mode: it is a maintenance pass over the
  // library, not another way to look at it.
  let duplicatesOpen = $state(false);
  // asset id → full text (null = source unreadable, fall back to cover).
  let readerTexts = $state<Map<string, string | null>>(new Map());
  // Markdown rendering toggle for the Reader (chat bodies are mostly
  // markdown; raw mode is one click away when the rendering lies).
  // Reader-side render mode. Same chip strip as the detail overlay.
  let readerMode = $state<DetailMode>("md");

  // `renderMarkdown` moved to `lib/formatters.ts` (Phase C wave B).

  // Content-type auto-detection — mirrors render-session's HasTable /
  // HasMermaid / HasBox family so a message rich in code, tables, or
  // diagrams stands out at a glance in the grid. Detection runs off
  // the same `readerTexts` that the Reader already loads, so no extra
  // round-trip is needed once a session is opened.
  const CONTENT_FLAGS = [
    { id: "code", icon: "⌨", label: "code" },
    { id: "table", icon: "📊", label: "table" },
    { id: "mermaid", icon: "🎨", label: "mermaid" },
    { id: "link", icon: "🔗", label: "link" },
  ] as const;
  type ContentFlag = (typeof CONTENT_FLAGS)[number]["id"];

  const RE_FENCE = /^```/m;
  const RE_MERMAID = /^```mermaid\b/mi;
  // A markdown table row surfaces via at least two contiguous lines
  // starting and ending with `|` — one for the header, one for the
  // separator. A single stray `|` in prose is common enough that we
  // require the pair.
  const RE_TABLE = /^\|.*\|\s*$\n^\|[\s\-:|]+\|\s*$/m;
  const RE_LINK = /\[[^\]]+\]\([^\s)]+\)/;

  function detectFlags(text: string | null | undefined): Set<ContentFlag> {
    const out = new Set<ContentFlag>();
    if (!text) return out;
    if (RE_MERMAID.test(text)) out.add("mermaid");
    if (RE_FENCE.test(text)) out.add("code");
    if (RE_TABLE.test(text)) out.add("table");
    if (RE_LINK.test(text)) out.add("link");
    return out;
  }

  // Filter chips over the auto-detected flags. Empty set = show all.
  let activeContentFlags = new SvelteSet<ContentFlag>();

  function toggleContentFlag(f: ContentFlag) {
    if (activeContentFlags.has(f)) activeContentFlags.delete(f);
    else activeContentFlags.add(f);
  }

  let status = $state("connecting...");
  let dataProfile = $state(import.meta.env.DEV ? "dev" : "dogfood");
  let searchDebounce: ReturnType<typeof setTimeout> | undefined;
  // Filter-change reload debounce. A burst of
  // sidebar toggles (multi-tag select, persona→modality in quick
  // succession) collapses into one grid fetch instead of one per
  // mutation. Kept short so a single deliberate click still feels
  // instant. Sibling of `searchDebounce`.
  let filterReloadDebounce: ReturnType<typeof setTimeout> | undefined;
  const FILTER_RELOAD_DEBOUNCE_MS = 200;
  // Loading-pill reveal delay (Item 2). The in-flight pill only paints
  // once a fetch has been running longer than this, so a warm-cache
  // reload (typically <30 ms) never flashes a spinner. Set by the
  // effect below off the catalog loading flags.
  let showLoadingPill = $state(false);
  const LOADING_PILL_DELAY_MS = 250;

  async function loadActiveProfile() {
    try {
      dataProfile = await invoke<string>("active_profile");
    } catch (error) {
      console.warn("active_profile failed", error);
    }
  }

  async function loadPersonas() {
    await personaCatalog.load();
    if (personaCatalog.list.error === null) {
      status = `backend OK — ${personaCatalog.list.data.length} persona(s)`;
    } else {
      status = `backend error: ${personaCatalog.list.error}`;
    }
  }

  // Selecting a group implicitly selects the collections nested
  // inside it: the filter expands each active group id into its
  // descendant closure over `groupLinks`. Client-side walk on a
  // small acyclic graph (the backend rejects cycles; the visited
  // set is a second guard) — the SQL layer keeps its flat OR shape.
  function expandGroupIds(ids: Iterable<string>): string[] {
    const out = new Set<string>();
    const stack = Array.from(ids);
    while (stack.length > 0) {
      const id = stack.pop()!;
      if (out.has(id)) continue;
      out.add(id);
      for (const link of groupCatalog.links.data) {
        if (link.parent_group_id === id) stack.push(link.child_group_id);
      }
    }
    return Array.from(out);
  }

  /**
   * Flips the grid between the live set and the trash.
   *
   * Clears the search box on the way in: search is served by the
   * full-text index, which by construction holds only live assets, so
   * the backend refuses a trashed-side search outright. Without this
   * the search branch would keep the previous *live* result page on
   * screen (the catalog deliberately keeps the last good page on error)
   * and only report the failure in the status line.
   *
   * The 🎲 draw is deliberately left alone on the way in: it is a `WHERE`
   * clause over the `asset` table, so it answers for the trashed side as
   * readily as the live one. "Something at random out
   * of the trash" is a question worth being able to ask, and clearing
   * the draw here would answer it by silently changing the subject.
   */
  function toggleTrashView() {
    activeFilter.trashView = !activeFilter.trashView;
    if (activeFilter.trashView && activeFilter.searchText.trim().length > 0) {
      activeFilter.searchText = "";
      status = "search is live-only — cleared it to show the trash";
    }
  }

  /**
   * Brings one asset back from the trash and drops it from the current
   * (trash) page. A reload would be simpler but would repaint the whole
   * grid on every single restore; the row is gone from this side either
   * way, so removing it locally is both faster and truthful.
   */
  /** @returns whether the asset was actually restored. */
  async function restoreAsset(assetId: string): Promise<boolean> {
    try {
      await mutate(
        "restore_asset",
        { command: { asset_id: assetId } },
        "restore this from the trash",
      );
      assetPageCatalog.dropItem(assetId);
      // The row is gone from this side, so it must not keep counting
      // toward the selection bar — a following bulk action would send a
      // dead id.
      gridSelection.selectedIds.delete(assetId);
      status = "restored";
      return true;
    } catch (err) {
      // The reason is already on screen — `mutate` put it there — so the
      // status line does not repeat it. What a caller needs from here is
      // whether it happened: a bulk loop that counted a refusal as a
      // success would write "restored 5" over the refusal saying
      // otherwise.
      console.warn("restore_asset failed", err);
      return false;
    }
  }

  /**
   * Deletes one trashed asset forever. Callers go through
   * `purgeFromCard`, which owns the confirm and the selection
   * expansion; this is the per-id half.
   */
  /** @returns whether the asset was actually purged. */
  async function purgeOne(assetId: string): Promise<boolean> {
    try {
      await mutate(
        "purge_asset",
        { command: { asset_id: assetId } },
        "delete this permanently",
      );
      assetPageCatalog.dropItem(assetId);
      // The row is gone from this side, so it must not keep counting
      // toward the selection bar — a following bulk action would send a
      // dead id.
      gridSelection.selectedIds.delete(assetId);
      status = "deleted forever";
      return true;
    } catch (err) {
      // Same as `restoreAsset`: the refusal is on screen, and the
      // caller is told what happened rather than being left to assume.
      console.warn("purge_asset failed", err);
      return false;
    }
  }

  function currentFilter() {
    return {
      viewer_subject: null,
      persona_id: activeFilter.activePersona,
      modality: activeFilter.activeModality,
      occurred_from_ms: null,
      occurred_until_ms: null,
      tag_ids: Array.from(activeFilter.activeTagIds),
      // How the tag chips compose. `group_ids` stays OR regardless —
      // nesting expansion already produces a set the user means as
      // "anything under these".
      tag_match: activeFilter.tagMatchAll ? "all" : "any",
      group_ids: expandGroupIds(activeFilter.activeGroupIds),
      session_id: activeFilter.activeSessionId,
      label: activeFilter.activeLabel,
      // 🔍 exact search: the text rides here as a set predicate, so it
      // composes with the chips and the count / sort stay honest. In ✦
      // fuzzy mode this is null and the text goes to Retrieval instead
      // (`loadAssets`) — the two never carry it at once, which is what
      // keeps "which domain answered" unambiguous. The split itself
      // lives on the store so both halves stay one expression.
      text_match: activeFilter.textMatch(),
      // No visibility flags here on purpose. One Asset is one Card, so
      // the grid asks for the whole set and the facets narrow it. The
      // grid used to send `top_level: true`, which made members exist
      // in the data and nowhere in the UI; if a Card ever stops being
      // 1:1 with an Asset, that belongs wherever Cards get built, not
      // in a flag every caller has to remember.
      // FORMAT facet (material mime top-level type).
      format: activeFilter.activeFormat,
      // COLOR facet (palette swatch).
      color: activeFilter.activeColor,
      // Playback-length, stored-size and resolution bands, in the wire's
      // ms / bytes / raw pixel count. The sidebar holds them in seconds /
      // MB / MP; the store owns the conversion so neither this builder
      // nor the section does it. Naming either end of a band excludes
      // rows whose column is NULL (`ListAssetsQuery::duration_min_ms`).
      ...activeFilter.metricBands(),
      // Which side of the trash to read. Everything downstream — grid,
      // index, counts, reader — goes through this builder, so the trash
      // view cannot half-apply.
      trash: activeFilter.trashView ? "trashed" : "live",
      offset: 0,
      // Grid render is virtualised with `virtua`'s `VList` so a
      // 6-figure page paints only the visible rows regardless
      // of total. Higher than the server-side `MAX_LIMIT`
      // (200_000) is pointless — it clamps back.
      limit: 200_000,
    };
  }

  // Thin wrappers around the catalog stores — kept so App-side
  // `$effect` blocks + DetailPane callback props (`onLoadTagCounts` /
  // `onLoadGroupCounts`) can invoke a zero-arg function without
  // pulling `activeFilter.activePersona` into every callsite. The
  // stores themselves handle error logging + empty-array reset.
  async function loadTagCounts() {
    await tagCatalog.loadCounts(activeFilter.activePersona);
  }

  async function loadGroupCounts() {
    await groupCatalog.loadCounts(activeFilter.activePersona);
  }

  // Group selection methods live on the shared filter store
  // (see `./lib/stores/filter.svelte`).

  // createGroup / deleteGroup / createDir / deleteDir moved to
  // `GroupsSection.svelte` (wave 5b-2). The sidebar mutation → catalog
  // reload chain lives with the handlers that trigger it; App no
  // longer needs to know these commands exist.

  async function loadDirs() {
    await groupCatalog.loadDirs(activeFilter.activePersona);
  }

  async function loadGroupLinks() {
    await groupCatalog.loadLinks(activeFilter.activePersona);
  }

  // Sidebar tree assembly (flat lists → parent-keyed maps) and the
  // group-name lookup moved to `groupCatalog` (group.svelte.ts): the
  // maps cross-cut counts + dirs + links, so co-locating them with
  // the raw data avoids a three-way join at every consumer. The
  // top-level parent key is `""` (see `groupCatalog.ROOT` in the
  // store); templates read via `.get(dir.id) ?? []`.
  const ROOT = "";

  // Cross-cutting derived: reads `activeFilter` selection state on
  // top of the catalog, so it lives here rather than on the catalog
  // store. `activeFilter.activeGroupIds.size === 1` marks the single
  // active collection — used for reorder mode and the child-group
  // band above the grid.
  let soloGroupId = $derived(
    activeFilter.activeGroupIds.size === 1 && activeFilter.viewMode === "messages"
      ? Array.from(activeFilter.activeGroupIds)[0]
      : null,
  );

  // toggleDirExpand / toggleGroupExpand moved to GroupsSection
  // alongside their `expandedDirs` / `expandedGroups` state.

  // Group ids beneath a dir (its whole subtree). Used by the
  // dir-name click: a dir is not a filter axis of its own, so
  // selecting one just toggles the underlying groups.
  function dirGroupIds(dirId: string): string[] {
    const out: string[] = [];
    const stack = [dirId];
    while (stack.length > 0) {
      const id = stack.pop()!;
      for (const gc of groupCatalog.groupsByDir.get(id) ?? []) out.push(gc.group.id);
      for (const child of groupCatalog.dirChildren.get(id) ?? []) stack.push(child.id);
    }
    return out;
  }

  function toggleDirFilter(dirId: string) {
    // Sidebar Dir row was clicked. Focus the lane on this dir so
    // the user sees its children next to the grid; a second click
    // on the same dir clears the focus and hides the lane.
    focusedDirId = focusedDirId === dirId ? null : dirId;
    // Filter application stays independent of the lane focus: the
    // dir's full group closure only turns on when the user
    // explicitly opts in via the lane's "Filter to this dir" chip.
  }

  function clearDirFocus() {
    focusedDirId = null;
  }

  // Derived material for the dir-focused lane above the grid.
  let dirNameById = $derived.by(() => {
    const m = new Map<string, string>();
    for (const d of groupCatalog.dirs.data) m.set(d.id, d.name);
    return m;
  });
  let dirParentById = $derived.by(() => {
    const m = new Map<string, string | null>();
    for (const d of groupCatalog.dirs.data) m.set(d.id, d.parent_id);
    return m;
  });
  // Root → focused ancestor chain, oldest first. `focusedDirId ===
  // ROOT` yields `[ROOT]`; a leaf dir yields
  // `[ROOT, grandparent, parent, focus]`. Used to paint the
  // breadcrumb above the lane so the user can jump up any level.
  let dirBreadcrumb = $derived.by(() => {
    if (focusedDirId === null) return [] as string[];
    if (focusedDirId === ROOT) return [ROOT];
    const chain: string[] = [];
    let cursor: string | null = focusedDirId;
    // Bound the walk in case the tree ever loops (would be a bug,
    // but guarding is cheap and keeps the UI stable).
    for (let i = 0; i < 64 && cursor !== null; i++) {
      chain.push(cursor);
      cursor = dirParentById.get(cursor) ?? null;
    }
    chain.push(ROOT);
    chain.reverse();
    return chain;
  });
  function crumbLabel(id: string): string {
    return id === ROOT ? "Root" : dirNameById.get(id) ?? "?";
  }
  function goUpDir() {
    if (focusedDirId === null || focusedDirId === ROOT) return;
    const parent = dirParentById.get(focusedDirId) ?? null;
    focusedDirId = parent ?? ROOT;
  }
  // Group / dir cover picks + the thumb blob-URL cache moved to
  // `thumbCatalog` (thumb.svelte.ts, wave ②).

  let laneChildDirs = $derived<DirDto[]>(
    focusedDirId === null ? [] : groupCatalog.dirChildren.get(focusedDirId) ?? [],
  );
  let laneChildGroups = $derived<GroupSummaryDto[]>(
    focusedDirId === null ? [] : groupCatalog.groupsByDir.get(focusedDirId) ?? [],
  );

  function applyDirFilter(dirId: string) {
    // Explicit "narrow the grid to every asset under this dir"
    // action — separate from the lane focus so the two axes are
    // controllable independently.
    const ids = dirGroupIds(dirId);
    if (ids.length === 0) return;
    const allActive = ids.every((id) => activeFilter.activeGroupIds.has(id));
    const byName = groupCatalog.nameById;
    for (const id of ids) {
      if (allActive) {
        activeFilter.activeGroupIds.delete(id);
        activeFilter.activeGroupNames.delete(id);
      } else if (!activeFilter.activeGroupIds.has(id)) {
        activeFilter.activeGroupIds.add(id);
        activeFilter.activeGroupNames.set(id, byName.get(id) ?? "?");
      }
    }
  }

  // createDir / deleteDir / all rename helpers / sidebar drag state
  // (dragGroupId / dragDirId / dragOverDirId) / sidebar drag handlers
  // (onGroupRowDragStart / onDirRowDragStart / onSidebarDragEnd /
  // dirAccepts / onDirDrag* / onDirDrop / groupAccepts /
  // onGroupDrag* / onGroupDrop) / linkGroups moved to
  // `GroupsSection.svelte` (wave 5b-2). `unlinkGroups` stays here
  // because the child-group band above the grid (soloGroupId case)
  // still calls it.

  async function unlinkGroups(parentId: string, childId: string) {
    try {
      await mutate(
        "unlink_group",
        { command: { parent_group_id: parentId, child_group_id: childId } },
        "unlink these groups",
      );
      await loadGroupLinks();
      await loadAssets();
    } catch (error) {
      console.warn("unlink_group failed", error);
    }
  }

  // Drill from the child band into a nested collection: the child
  // becomes the sole active group.
  function drillIntoGroup(gc: GroupSummaryDto) {
    activeFilter.activeGroupIds.clear();
    activeFilter.activeGroupNames.clear();
    activeFilter.activeGroupIds.add(gc.group.id);
    activeFilter.activeGroupNames.set(gc.group.id, gc.group.name);
  }

  // Drag-reorder is only offered when a single group is the active
  // filter (union filters have no meaningful per-bucket position) and
  // the messages view is up (the sessions view is a rollup, not the
  // per-asset grid the reorder targets). `$derived` so the template
  // and handler branches stay in sync as the underlying SvelteSet /
  // $state flips.
  // Grid drag-reorder is a manual-Group affordance: the position of a
  // Query Group's members is burnt in by the evaluation Job, and
  // the backend rejects reorder / add / remove with kind='query'.
  // Restricting `reorderActive` to `kind='manual'` keeps the
  // reorder chrome off the grid so the gesture never starts on a
  // group whose members it cannot rewrite.
  // Searching replaces the order wholesale: a non-empty query leaves the
  // list endpoint entirely (`assetPageCatalog.loadPage` → the Tantivy
  // path) and the page arrives ranked by relevance, which is not a value
  // any card field carries and so not something the Sort picker can
  // express. The grid keeps that ranking and says who owns it.
  //
  // Group *count* deliberately does not appear here. The repository does
  // switch to `asset_bucket.position` when the filter names exactly one
  // group (`SqliteAssetRepository::page`), but that is an implementation
  // detail of how the page arrives, not a mode: the comparator re-sorts
  // it onto the selected axis exactly as it does a 3-group page, so
  // `Sort: Tag` means tag order whether one group is checked or five.
  // An earlier pass gated the whole picker on `activeGroupIds.size === 1`
  // and the toolbar collapsed to a fixed "Manual" the moment a single
  // group was checked — a sorter that stops sorting on a filter change.
  // The manual arrangement is a *choice* (`Sort: Group` + `Order:
  // ordered`, the single-group case of that pair), not a state the
  // selection count forces.
  // True only while **Retrieval** owns the sequence — ✦ fuzzy with text
  // in the box, where the page arrives already ranked and re-sorting it
  // would discard the only ordering the query produced.
  //
  // 🔍 exact deliberately does not count, though its box is just as
  // non-empty: there the text rides on `filter.text_match` down the list
  // path, so the answer is a set in the server's order and the sorter is
  // the thing that gives it one (the exact side counts exactly, sorts,
  // and saves). Keyed off text alone, this flag hid the
  // sort picker behind a static "Order: ⌕ Relevance" the moment anything
  // was typed, which in exact mode named an ordering nothing produced.
  let searchOrderActive = $derived(
    activeFilter.searchFuzzy && activeFilter.searchText.trim().length > 0,
  );
  // Twin of `searchOrderActive` for the 🎲 draw: there the sequence is
  // the shuffle, and re-sorting it would replace the one property the
  // draw has (order *is* the randomness). Everything
  // keyed off the search flag is keyed off this one for the same
  // reason: the picker would offer an ordering the grid does not apply,
  // bucket headers would caption a clustering that is not there, and a
  // drag-reorder would index a sequence the user cannot see.
  let randomOrderActive = $derived(activeFilter.discoverRandom);
  let reorderActive = $derived.by(() => {
    if (activeFilter.viewMode !== "messages") return false;
    if (activeFilter.activeGroupIds.size !== 1) return false;
    // Rearranging means moving a card relative to the cards around it,
    // so the grid has to be *showing* the arrangement — otherwise the
    // drop lands where the card sits in a sequence the user cannot see.
    // `Group` + `ordered` is that view (search re-ranks on top of any
    // axis, so it disqualifies the gesture too).
    if (searchOrderActive || randomOrderActive) return false;
    if (activeFilter.sortTarget !== "group" || activeFilter.sortOrder !== "ordered") {
      return false;
    }
    // Reversed, the grid reads back-to-front while `reorderOnto` still
    // indexes the page front-to-back, so every drop would land mirrored.
    if (activeFilter.sortReverse) return false;
    const soleId = activeFilter.activeGroupIds.values().next().value as string | undefined;
    if (soleId === undefined) return false;
    return !groupCatalog.isQueryGroup(soleId);
  });
  // Cards are always draggable in Messages view: two independent
  // targets accept them — grid cards (reorder inside a group) and
  // sidebar Group entries (add-to-group, the Are.na "drop into a
  // channel" gesture).
  // Filing is a live-set act. Dragging a trashed card into a Group
  // would file something on its way to deletion — the membership row
  // even survives the trash, so it would look filed right up until the
  // retention sweep removed it. Keyed on the page's provenance, not the
  // toggle, for the same reason the icon strip is.
  let draggableActive = $derived(
    activeFilter.viewMode === "messages" && !assetPageCatalog.pageIsTrash,
  );
  // Drag state lives in `cardDrag` (lib/interaction/drag.svelte.ts):
  // which card is in flight, where the pointer is, and what sits under
  // it. Every target — grid card, Group row, Modality row — reads the
  // same store for its highlight and marks itself with
  // `data-drop-kind` / `data-drop-id` instead of registering handlers.

  // Drag auto-scroll: a card picked up at the top of a long list has to
  // be able to reach a Group sitting below the fold, and the wheel is
  // not usable mid-drag. A rAF loop reads the live pointer Y off
  // `cardDrag` and nudges window scroll while the pointer sits in the
  // top / bottom dead-zone. rAF rather than reacting to moves alone,
  // because a cursor held still at the edge stops producing events and
  // the scroll would stall.
  let scrollRafId: number | null = null;
  const EDGE_ZONE_PX = 60;
  const EDGE_MAX_SPEED_PX = 18;

  $effect(() => {
    if (cardDrag.active) startEdgeAutoScroll();
    else stopEdgeAutoScroll();
  });

  function edgeScrollTick() {
    const h = window.innerHeight;
    const topGap = cardDrag.y;
    const bottomGap = h - cardDrag.y;
    let delta = 0;
    if (topGap < EDGE_ZONE_PX) {
      // Linear ramp: full speed at the edge, zero at the boundary.
      delta = -Math.round(EDGE_MAX_SPEED_PX * (1 - topGap / EDGE_ZONE_PX));
    } else if (bottomGap < EDGE_ZONE_PX) {
      delta = Math.round(EDGE_MAX_SPEED_PX * (1 - bottomGap / EDGE_ZONE_PX));
    }
    if (delta !== 0) window.scrollBy(0, delta);
    scrollRafId = requestAnimationFrame(edgeScrollTick);
  }

  function startEdgeAutoScroll() {
    if (scrollRafId !== null) return;
    scrollRafId = requestAnimationFrame(edgeScrollTick);
  }

  function stopEdgeAutoScroll() {
    if (scrollRafId !== null) {
      cancelAnimationFrame(scrollRafId);
      scrollRafId = null;
    }
  }

  /**
   * Where a dropped card lands. One router for every target kind — the
   * drag helper reports what was under the pointer and this decides
   * what that means, so a new target is a `data-drop-kind` on the
   * element plus a branch here.
   */
  function onCardDropTarget(target: DropTarget, source: DragSource) {
    if (source.kind !== "card") return;
    switch (target.kind) {
      case "card":
        void reorderOnto(source.id, target.id);
        return;
      case "group":
        void addDraggedToGroup(source.id, target.id);
        return;
      case "modality":
        void moveDraggedToModality(source.id, target.id);
        return;
      case "trash":
        void trashFromCard(source.id);
        return;
    }
  }

  async function reorderOnto(assetId: string, ontoAssetId: string) {
    if (!reorderActive || assetPageCatalog.page === null) return;
    // Indices come from the *page*, not the displayed list, because the
    // write replaces the group's whole membership order — a card the
    // content-flag chips filtered out of view is still a member and must
    // keep its slot. `reorderActive` guarantees the two run in the same
    // direction (single group, `Group` + `ordered`, unreversed), so a
    // page index and what the user sees agree on who comes before whom.
    const src = assetPageCatalog.page.items.findIndex((c) => c.id === assetId);
    const index = assetPageCatalog.page.items.findIndex((c) => c.id === ontoAssetId);
    if (src < 0 || index < 0 || src === index) return;
    // Optimistic local reorder — splice out src, splice in at target
    // slot. Then flush to the backend and reload to confirm.
    const groupId = Array.from(activeFilter.activeGroupIds)[0];
    const items = [...assetPageCatalog.page.items];
    const [moved] = items.splice(src, 1);
    items.splice(index, 0, moved);
    // Renumbering the slots is not bookkeeping — it *is* the optimistic
    // update. The grid sorts by `primary_group_position` (`Group` +
    // `ordered`), not by array order, so a spliced-but-unnumbered page
    // gets sorted straight back into the sequence the user just changed
    // and the drop reads as "nothing happened". These numbers are
    // exactly what the write below stores, so the optimistic page and
    // the canonical one agree.
    const reordered = items.map((card, slot) => ({
      ...card,
      primary_group_position: slot,
    }));
    assetPageCatalog.page = {
      ...assetPageCatalog.page,
      items: reordered,
    } as AssetPageDto;
    try {
      await invoke("reorder_group_assets", {
        command: {
          group_id: groupId,
          ordered_asset_ids: reordered.map((c) => c.id),
        },
      });
      // Reload to pick up the canonical order; also refreshes counts
      // in case the write raced against a concurrent add/remove. The
      // filter did not change, so the catalog's fetch-key cache would
      // hand back this very page — carrying the positions this write
      // just superseded — unless the key is dropped first.
      assetPageCatalog.invalidateKey();
      await loadAssets();
    } catch (error) {
      console.warn("reorder_group_assets failed", error);
      // Rollback: the reload wipes the optimistic swap, and it can only
      // do that if it is allowed to leave the fetch-key cache.
      assetPageCatalog.invalidateKey();
      await loadAssets();
    }
  }

  // Sidebar Group entry as a drop target. Two payloads land here:
  // a grid card (the Are.na "drop into a channel" gesture → add the
  // asset) or another sidebar group row (→ connect it as a nested
  // collection). HTML5 requires preventDefault() on BOTH dragenter
  // and dragover for the element to be a valid drop target; skipping
  // dragenter is why the browser rejects the drop (and never fires
  // groupAccepts / onGroupDrag* / onGroupDrop moved to
  // `GroupsSection.svelte` (wave 5b-2). App keeps `dragOverGroupId`
  // above as a bindable prop source so `onCardDragEnd` can still
  // defensively clear it (edge case where the browser skips the row
  // dragleave on drag cancel).

  // Sidebar click toggle moved to `activeFilter.toggleTag` (the
  // TagList sidebar section calls it directly). Retain the comment
  // here for the design breadcrumb: OR semantic is enforced
  // downstream by the domain query (an asset needs at least one of
  // the active tags).

  // Detail-view chip click: idempotent-add (not toggle) so clicking
  // a chip that is already filtering does not accidentally remove
  // the filter the user is trying to reinforce.
  // Tag selection methods live on the shared filter store
  // (see `./lib/stores/filter.svelte`).

  // Stale-response guards live inside the stores now: the sessions
  // page on `sessionCatalog.page` (Resource generation) and the
  // messages page on `assetPageCatalog` (own generation counter).
  // The two views keep separate storage, so a slow fetch for one
  // view can never null out the other's page.

  // Per-view cache keys so `loadAssets` can skip the round-trip
  // when the effect fires purely because `activeFilter.viewMode` flipped and
  // the filter itself hasn't changed since the last successful
  // fetch. Refetching 110 k assets on every Sessions ↔ Messages
  // toggle pinned the main thread for seconds; keeping both
  // `page` and `sessionPage` populated across a swap and only
  // refetching when the filter key actually differs makes the
  // toggle instant.
  function fetchKey(): string {
    return JSON.stringify({
      f: currentFilter(),
      q: activeFilter.searchText.trim(),
      // The draw is not part of `currentFilter()` — it decides which
      // wire answers, not what the filter says — so without it here,
      // flipping 🎲 on and off would hit the key cache and leave the
      // previous branch's page on screen. The nonce rides inside it so
      // "draw again" (identical arguments, deliberately new answer)
      // reads as a different request; `null` while the draw is off, so
      // an unused counter cannot invalidate a listing.
      r: activeFilter.discoverRandom ? activeFilter.randomNonce : null,
    });
  }
  // `indexToLightCard` + the messages fetch machinery (fetch-key
  // skip, index/search branches, stale guard) + the viewport
  // hydration cache moved to `assetPageCatalog` (wave ①).
  // App keeps the view dispatch below because the sessions branch
  // + `status` chrome + query composition are App-owned.

  async function loadAssets() {
    const key = fetchKey();
    const fresh = await assetPageCatalog.loadPage({
      filter: currentFilter(),
      // Only ✦ fuzzy sends text down the Retrieval branch. In 🔍 exact
      // mode the catalog must take the listing branch — the text is
      // already on `filter.text_match`, and letting it through here too
      // would answer the query twice, from the domain that does not
      // claim to cover the library. The draw takes precedence over
      // both branches, and sends no text at all: a ✦ query is cleared
      // when the draw is turned on, and a 🔍 one is already inside
      // `filter.text_match`, narrowing the pool.
      searchText: activeFilter.discoverRandom ? "" : activeFilter.retrievalText(),
      random: activeFilter.discoverRandom,
      key,
    });
    if (!fresh && assetPageCatalog.error !== null) {
      status = `list error: ${assetPageCatalog.error}`;
    }
    // The rank rides every reload the page takes, so it can never
    // describe a different query than the one on screen.
    syncRankOrder();
  }

  // When the `✦ Relevance` axis can actually mean something: the axis is
  // picked, the box is 🔍 exact, and it has text.
  //
  // ✦ fuzzy is excluded because there the page *is* the ranked answer —
  // `searchOrderActive` skips the client sort outright — so fetching a
  // rank would be asking the same question a second time. The trash is
  // excluded because Retrieval has no index for it and the request would
  // come back `400`; a request known to fail is not worth sending.
  let relevanceOrderActive = $derived(
    activeFilter.sortTarget === "relevance" &&
      !activeFilter.searchFuzzy &&
      !activeFilter.trashView &&
      activeFilter.searchText.trim().length > 0,
  );

  // Keeps the rank hint in step with the query. Called from
  // `loadAssets` — so it inherits the search debounce and every filter
  // reload — and from the axis effect below, which is the one trigger
  // that does not pass through a reload.
  function syncRankOrder() {
    if (!relevanceOrderActive) {
      assetPageCatalog.clearRankOrder();
      return;
    }
    void assetPageCatalog.loadRankOrder({
      filter: currentFilter(),
      text: activeFilter.searchText,
      key: fetchKey(),
    });
  }

  async function openSession(s: SessionDto) {
    // Since the Show-messages toggle landed, the filter-chip drill-in
    // narrow is no longer needed here — this is a single-step entry
    // that opens the Reader on its own. No side effects on
    // activeSessionId / activeSessionLabel (no chip is left behind).
    // The legacy drill-in path (📖 read button) still serves other
    // views and is untouched.
    await openReaderForSession(s.id);
  }

  // Session tile 💬 click opener — twin of `openSessionNoteFromIcon`
  // for the comments panel. Coord derivation matches
  // the Note path so both overlays land in the same place.
  function openSessionCommentsFromIcon(
    session: SessionDto,
    ev: MouseEvent,
  ) {
    if (cardActionCloseTimer !== null) {
      window.clearTimeout(cardActionCloseTimer);
      cardActionCloseTimer = null;
    }
    cardThreadHover = null;
    cardNoteHover = null;
    const tileEl = (ev.currentTarget as HTMLElement).closest(
      ".session-tile",
    ) as HTMLElement | null;
    const rect = (tileEl ?? (ev.currentTarget as HTMLElement)).getBoundingClientRect();
    sessionCommentsHover = {
      sessionId: session.id,
      x: Math.min(rect.right + 6, window.innerWidth - 360),
      y: Math.min(rect.top, window.innerHeight - 400),
    };
  }

  function scrollToTop() {
    if (typeof window === "undefined") return;
    window.scrollTo({ top: 0, behavior: "auto" });
    // The `<main class="content">` region also scrolls independently
    // in some viewport widths — reset both so the fix is width-proof.
    const main = document.querySelector("main.content");
    if (main) main.scrollTop = 0;
  }

  function clearSession() {
    if (activeFilter.activeSessionId !== null) {
      activeFilter.activeSessionId = null;
      activeFilter.activeSessionLabel = null;
    }
  }

  // Debounce keystrokes so typing does not hammer the search endpoint.
  // The component owns the input event — it mutates
  // `activeFilter.searchText` synchronously and calls this back to
  // reschedule the reload.
  function reloadWithSearchDebounce() {
    clearTimeout(searchDebounce);
    searchDebounce = setTimeout(() => {
      const q = activeFilter.searchText.trim();
      if (q.length > 0) {
        // Post-debounce = one settled query, not one per keystroke.
        recordEvent("search", {
          personaId: activeFilter.activePersona,
          payload: { q },
        });
      }
      loadAssets();
    }, 250);
  }

  // Immediate reload path — used by the clear (✕) button and the
  // Escape key inside `SidebarSearch`. Bypasses the debounce so the
  // grid snaps back to unfiltered the same tick.
  function reloadSearchImmediate() {
    clearTimeout(searchDebounce);
    loadAssets();
  }

  // The escape hatch offered next to the "more beyond the shortlist"
  // hint. Retrieval caps at K and says so; the exact
  // side answers the same text as a set predicate, where "every match"
  // is a claim it can actually make. One-way on purpose — going back to
  // ✦ is the toggle in `SidebarSearch`, which is where mode lives.
  function switchToExactSearch() {
    activeFilter.searchFuzzy = false;
    reloadSearchImmediate();
  }

  // Explicit clear — used by `resetFilters` below to wipe search
  // together with the other axes. The `SidebarSearch` component
  // handles user-visible clears (button + Escape) on its own.
  function clearSearch() {
    activeFilter.searchText = "";
    reloadSearchImmediate();
  }

  function resetFilters() {
    activeFilter.activePersona = null;
    activeFilter.activeModality = null;
    activeFilter.activeLabel = null;
    activeFilter.durationMinSec = null;
    activeFilter.durationMaxSec = null;
    activeFilter.sizeMinMb = null;
    activeFilter.sizeMaxMb = null;
    activeFilter.pixelsMinMp = null;
    activeFilter.pixelsMaxMp = null;
    activeFilter.clearTags();
    activeFilter.clearGroups();
    clearSession();
    clearSearch();
  }

  // `anyFilterActive` moved to SidebarSearch.svelte as a local
  // `$derived` — the Active-filters header is the sole consumer.

  // Telemetry: one `app_open` per App instantiation (= per launch;
  // the component is never remounted). Runs in the plain script body
  // so it fires exactly once, before any effect.
  recordEvent("app_open", { personaId: activeFilter.activePersona });

  // Telemetry: previous persona id, so the reload effect below can
  // tell a persona switch apart from the other filter axes.
  // `undefined` = effect has not fired yet (skip the initial load).
  let telemetryPersona: string | null | undefined = undefined;

  // Reload whenever a filter or the view mode flips. SvelteSet
  // mutations register as reactive reads via `.size` so we do not
  // need a manual dependency tick.
  $effect(() => {
    void activeFilter.activePersona;
    void activeFilter.activeModality;
    void activeFilter.activeFormat;
    void activeFilter.activeColor;
    void activeFilter.activeLabel;
    void activeFilter.activeTagIds.size;
    // How those tags compose (OR / AND). Same reasoning as `trashView`
    // below: `currentFilter()` reads it, but that call happens inside
    // the debounce callback, outside this effect's dependency window.
    // `searchFuzzy` is deliberately *not* tracked here — the search box
    // owns its own reload path (SidebarSearch's immediate callback), and
    // a second trigger would fire two loads for one click.
    void activeFilter.tagMatchAll;
    // The three metric bands, for the same reason: `currentFilter()`
    // reads them inside the debounce callback, outside this effect's
    // dependency window, so without these lines a typed band would
    // change the sidebar and never the grid. Every end needs its own
    // line — the spread in `currentFilter()` is not a dependency.
    void activeFilter.durationMinSec;
    void activeFilter.durationMaxSec;
    void activeFilter.sizeMinMb;
    void activeFilter.sizeMaxMb;
    void activeFilter.pixelsMinMp;
    void activeFilter.pixelsMaxMp;
    void activeFilter.activeGroupIds.size;
    void activeFilter.activeSessionId;
    void activeFilter.viewMode;
    // The nesting graph feeds the descendant expansion inside
    // `currentFilter`, so a link/unlink must also reload the grid.
    void groupCatalog.links.data;
    // Which side of the trash the grid reads. Tracked here explicitly:
    // `currentFilter()` reads it too, but that call happens inside the
    // debounce callback, outside this effect's dependency-collection
    // window — so without this line the toggle would change the icons
    // and never the data.
    void activeFilter.trashView;
    // Whether the grid draws at random, and the counter that asks for a
    // fresh draw. Both are read by `fetchKey` / `loadAssets`, which run
    // inside the debounce callback and so outside this effect's
    // dependency window — the same reason `trashView` is listed above.
    // Turning the draw on also goes through the component's immediate
    // reload; that one lands first and this effect's later call finds
    // the key already served.
    void activeFilter.discoverRandom;
    void activeFilter.randomNonce;
    // Baseline: from the filter-change effect firing to the next
    // painted frame. Combined with the invoke stamps inside
    // `loadAssets`, this pins down where the sidebar-click lag lives
    // (backend vs frontend derived+render).
    perfBaseline.measureToPaint("filter->paint", {
      viewMode: activeFilter.viewMode,
      tagCount: activeFilter.activeTagIds.size,
      groupCount: activeFilter.activeGroupIds.size,
    });
    const persona = activeFilter.activePersona;
    const personaChanged =
      telemetryPersona !== undefined && telemetryPersona !== persona;
    telemetryPersona = persona;
    // Debounce the actual fetch (Item 2): the latest scheduled reload
    // wins, so a rapid axis burst issues one `loadAssets` call. The
    // persona_switch telemetry is measured inside the debounced body
    // so the duration reflects the real fetch, not the debounce wait.
    clearTimeout(filterReloadDebounce);
    filterReloadDebounce = setTimeout(() => {
      const reloadT0 = performance.now();
      const reload = loadAssets();
      if (personaChanged) {
        // Duration = the user-perceived wait from the sidebar click to
        // the page data being in place (render lag is out of scope —
        // the paint proxy above covers it in DEV).
        void reload.then(() =>
          recordEvent("persona_switch", {
            personaId: persona,
            durationMs: performance.now() - reloadT0,
          }),
        );
      }
    }, FILTER_RELOAD_DEBOUNCE_MS);
  });
  // Picking the sort axis changes no filter and touches no search box,
  // so it reaches neither reload path — the rank needs its own trigger.
  // `untrack` keeps the effect's dependency to the axis alone: reading
  // the text / filter here too would re-fire it on every keystroke,
  // ahead of (and bypassing) the search debounce.
  $effect(() => {
    void activeFilter.sortTarget;
    untrack(() => syncRankOrder());
  });
  // Reveal the grid loading pill only after a fetch has been in flight
  // for `LOADING_PILL_DELAY_MS` (Item 2) — a warm reload resolves
  // first and the spinner never flashes. Tracks both the messages and
  // sessions loading flags; the effect cleanup cancels a pending
  // reveal the instant loading settles.
  $effect(() => {
    const loading = assetPageCatalog.loading || sessionCatalog.page.loading;
    if (!loading) {
      showLoadingPill = false;
      return;
    }
    const t = setTimeout(() => {
      showLoadingPill = true;
    }, LOADING_PILL_DELAY_MS);
    return () => clearTimeout(t);
  });
  $effect(() => {
    loadPersonas();
  });
  // One-shot load of the Modality master (backend-authoritative axis).
  // The rows rarely change at runtime; the persona-scoped counts that
  // ride alongside are refreshed by the `activePersona` effect below
  // via `loadSidebarCounts`. Reload orchestration stays App-side.
  $effect(() => {
    void modalityCatalog.load();
  });
  // Application preferences (`ui.clean_mode` / `import.auto_organize`),
  // followed by the one-shot carry of the retired `localStorage`
  // entries. Loaded once on mount; every write goes through
  // `setPreference`, which re-reads. Until this resolves the toggles
  // render their registry defaults.
  $effect(() => {
    void settingsCatalog
      .load()
      .then(() => settingsCatalog.migrateLegacyLocalStorage());
  });
  $effect(() => {
    void loadActiveProfile();
  });
  // Threads drawer: reload the AppGlobal Thread list the first time
  // the drawer opens (and on every subsequent open — cheap query,
  // catches HTTP-authored writes between opens). Reload orchestration
  // stays here. The P2 SSE upgrade replaces the second
  // effect below (the message-poll cadence).
  let threadPollTimer: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (!threadDrawerOpen) return;
    // Load, then ensure the AppGlobal "Inbox" default thread exists
    // if it is missing. Only fires when the list came back
    // empty — subsequent opens skip the create because the row is
    // already present.
    void (async () => {
      await threadsCatalog.load({ kind: "app_global", id: null });
      if (threadsCatalog.threads.size === 0 && !threadsCatalog.error) {
        try {
          await threadsCatalog.createThread({
            anchor_kind: "app_global",
            anchor_id: null,
            title: "Inbox",
          });
        } catch (e) {
          console.warn("[App] ensure Inbox thread failed", e);
        }
      }
    })();
  });
  $effect(() => {
    // Poll cadence when the drawer is open and a Thread is focused
    // (3 s — matches the dispatch stats cadence). P2 replaces this
    // with an SSE subscription and the effect goes away.
    if (!threadDrawerOpen) {
      if (threadPollTimer !== null) {
        clearInterval(threadPollTimer);
        threadPollTimer = null;
      }
      return;
    }
    const id = threadsCatalog.activeThreadId;
    if (!id) return;
    threadPollTimer = setInterval(() => {
      void threadsCatalog.refreshMessages(id);
    }, 3000);
    return () => {
      if (threadPollTimer !== null) {
        clearInterval(threadPollTimer);
        threadPollTimer = null;
      }
    };
  });
  // PromptModal is store-driven (promptCatalog.request) — mirror it
  // onto the interaction stack so Escape routes to cancel instead of
  // falling through to the selection-clear sink (review M1).
  // Both mirrors write the stack from inside an effect, so the write
  // must be untracked: `push()` reads the stack it replaces, and an
  // effect that reads what it writes re-runs itself until Svelte kills
  // the scheduler (effect_update_depth_exceeded) — after which nothing
  // in the app re-renders. That is not a theoretical loop: opening the
  // first confirm froze the whole UI until the process was restarted
  // (2026-08-01, caught by the Empty Trash e2e's stuck-Cancel trail).
  // Untracked, the effects depend on their request signal alone.
  $effect(() => {
    const open = promptCatalog.request !== null;
    untrack(() => {
      if (open) interaction.push("prompt");
      else interaction.remove("prompt");
    });
  });
  // ConfirmModal is store-driven the same way. Pushed after "prompt"
  // in file order only; the stack is ordered by push time, and this
  // modal is opened from inside another layer (context menu / thread
  // drawer), so it lands on top where Escape reaches it first.
  $effect(() => {
    const open = confirmCatalog.request !== null;
    untrack(() => {
      if (open) interaction.push("confirm");
      else interaction.remove("confirm");
    });
  });
  // One-shot load of the registered exporter slugs so the Selector
  // action bar can render one option per entry.
  $effect(() => {
    void loadExporterSlugs();
    // Also warm the AppGlobal thread list once so the ⤴ unread badge
    // (W6) has data before the drawer is ever opened. HTTP-authored
    // messages that land later are picked up by the drawer's own
    // poll; the badge is a startup snapshot by design.
    void threadsCatalog.load({ kind: "app_global", id: null });
  });
  // Refresh tag / group counts + the organisation tree whenever the
  // persona filter flips.
  $effect(() => {
    void activeFilter.activePersona;
    // The sidebar counts follow the grid's trash side, so the toggle
    // has to refetch them too — otherwise the chips keep describing the
    // side the user just left.
    void activeFilter.trashView;
    loadTagCounts();
    loadGroupCounts();
    loadDirs();
    loadGroupLinks();
    loadSidebarCounts();
    // Grouping (Session container) sidebar list — follows the persona
    // scope; `list_sessions` honours persona_id / offset / limit only.
    void sessionCatalog.loadPage({
      persona_id: activeFilter.activePersona,
      offset: 0,
      limit: 200,
    });
  });

  // Mirror `activeFilter` selection back to `window.location.search`.
  // `history.replaceState` avoids growing the
  // shell's history stack — Back / Forward should never navigate away
  // from the app. `URLSearchParams` iteration is deterministic on the
  // key list the adapter uses, so the effect is stable under identical
  // state.
  $effect(() => {
    void activeFilter.activePersona;
    void activeFilter.activeModality;
    void activeFilter.activeFormat;
    void activeFilter.activeColor;
    void activeFilter.activeTagIds.size;
    void activeFilter.tagMatchAll;
    void activeFilter.activeGroupIds.size;
    void activeFilter.searchText;
    void activeFilter.searchFuzzy;
    void activeFilter.viewMode;
    void activeFilter.sortTarget;
    void activeFilter.sortOrder;
    void activeFilter.sortReverse;
    syncToURL();
  });

  // Reconcile display names for URL-hydrated ids. On cold refresh only
  // ids come out of the URL; the names populate once
  // `loadTagCounts` / `loadGroupCounts` resolve. This effect fills
  // `activeFilter.active*Names` entries that are still missing, so the
  // sidebar chip labels paint correctly on the first render pass after
  // catalog fetch.
  $effect(() => {
    if (tagCatalog.counts.data.length === 0 || activeFilter.activeTagIds.size === 0) return;
    const byId = tagCatalog.nameById;
    untrack(() => {
      for (const id of activeFilter.activeTagIds) {
        if (activeFilter.activeTagNames.has(id)) continue;
        const name = byId.get(id);
        if (name) activeFilter.activeTagNames.set(id, name);
      }
    });
  });
  $effect(() => {
    if (groupCatalog.counts.data.length === 0 || activeFilter.activeGroupIds.size === 0) return;
    const byId = groupCatalog.nameById;
    untrack(() => {
      for (const id of activeFilter.activeGroupIds) {
        if (activeFilter.activeGroupNames.has(id)) continue;
        const name = byId.get(id);
        if (name) activeFilter.activeGroupNames.set(id, name);
      }
    });
  });

  // Listen for SessionRebuild progress. The backend handler
  // broadcasts `{phase: "start"|"done", ...}` at both ends of the
  // rebuild — auto-enqueued rebuilds (startup drift, Import batches)
  // have no caller-visible task id, so a global event is how the UI
  // learns about them.
  $effect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    listen<{ phase: string }>("sessions:progress", (evt) => {
      const phase = evt.payload?.phase;
      if (phase === "start") {
        sessionRebuildActive = true;
      } else if (phase === "done") {
        sessionRebuildActive = false;
        // Refresh the Grouping sidebar list so the just-built
        // containers show up (asset-model v4: sessions live in the
        // sidebar, not the grid).
        void sessionCatalog.loadPage({
          persona_id: activeFilter.activePersona,
          offset: 0,
          limit: 200,
        });
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  });

  // Listen for `jobs:tick` — fired by the job worker after EVERY job
  // completes (jobs/mod.rs, payload `{kind, ok}`). The kinds that
  // actually mutate the asset / session surface trigger a grid or
  // sessions reload; the rest (e.g. thumb_gen, per-size) are ignored
  // to avoid burst reloads on Import. This is the existing broadcast
  // that had no UI listener until now — the earlier NotifyAssetsChanged
  // detour was a misdiagnosis. Debounced so a wave of pipeline jobs
  // for one Import collapses into one refresh.
  const RELOAD_KINDS = new Set([
    "cover_gen",
    "index_rebuild",
    "session_rebuild",
    "auto_tag",
  ]);
  let jobsTickReloadTimer: number | null = null;
  const scheduleJobsTickReload = () => {
    if (jobsTickReloadTimer !== null) {
      window.clearTimeout(jobsTickReloadTimer);
    }
    jobsTickReloadTimer = window.setTimeout(() => {
      jobsTickReloadTimer = null;
      loadAssets();
    }, 250);
  };
  $effect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    listen<{ kind?: string; ok?: boolean }>("jobs:tick", (evt) => {
      const kind = evt.payload?.kind ?? "";
      if (!RELOAD_KINDS.has(kind)) return;
      scheduleJobsTickReload();
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
      if (jobsTickReloadTimer !== null) {
        window.clearTimeout(jobsTickReloadTimer);
        jobsTickReloadTimer = null;
      }
    };
  });

  // File drag-and-drop from the OS (Finder / a Screenshot preview /
  // a browser download). Tauri surfaces the OS event as
  // `tauri://drag-drop` with `{ paths: string[], position }`; we
  // dispatch each path through `add_asset` with the currently
  // active persona so a screenshot lands in the grid without a
  // detour through the CLI importer.
  //
  // A dropped file arrives unclassified.
  //
  // This used to map the extension onto a `ContentKind` and then onto
  // whatever modality carried that kind. With the format kinds gone
  // (they are the material's mime now) the lookup had nowhere to land
  // and fell through to `work_product` — every dropped photo filed as a
  // work product, which is worse than saying nothing. Classification is
  // a judgement about meaning, and the extension does not carry one;
  // the row lands in Unclassified and the user names it.
  // Compute the deepest directory that contains every dropped path,
  // for use as `auto_organize_base_dir`. Returns null when
  // the shared prefix would degenerate into "/" or a home-directory
  // root — the Dir tree the backend would build off those has no
  // meaningful structure and would just clutter Groups with system
  // paths. `~` prefix support is best-effort; if the path is still
  // shell-form it is skipped for common-prefix purposes.
  function commonParentDir(paths: string[]): string | null {
    if (paths.length === 0) return null;
    // Take the parent dir of each absolute path (drop the basename).
    const parents: string[] = [];
    for (const p of paths) {
      if (!p.startsWith("/")) return null;
      const cut = p.lastIndexOf("/");
      parents.push(cut <= 0 ? "/" : p.slice(0, cut));
    }
    // Walk segment-by-segment while all parents share the same head.
    const split = parents.map((p) => p.split("/").filter(Boolean));
    const first = split[0];
    let sharedSegments = 0;
    outer: for (let i = 0; i < first.length; i++) {
      for (const other of split) {
        if (other[i] !== first[i]) break outer;
      }
      sharedSegments++;
    }
    if (sharedSegments === 0) return null;
    const base = "/" + first.slice(0, sharedSegments).join("/");
    // Skip degenerate roots — the backend uses the base_dir as the
    // Dir-tree anchor, so anchoring at "/" or "/Users/<name>" would
    // pull every drop into a giant flat tree.
    if (base === "/") return null;
    if (/^\/Users\/[^/]+$/.test(base)) return null;
    if (/^\/home\/[^/]+$/.test(base)) return null;
    return base;
  }

  // Shared drop-import runner. Called both by the drop-event
  // handler (persona already active) and by the persona-picker
  // modal (persona chosen after the drop). `filterRecentDrops` is
  // NOT flipped on — turning it on made most of the grid vanish
  // after a small drop, which felt broken. The sidebar chip is
  // where the user opts in.
  async function runDropImport(paths: string[], personaId: string) {
    if (paths.length === 0) return;
    status = `dropping ${paths.length} file${paths.length === 1 ? "" : "s"}…`;
    const savedActive = activeFilter.activePersona;
    activeFilter.activePersona = personaId;
    // Hand the shared parent to the backend so it can build
    // a matching Dir tree + leaf Group. Only computed once per drop
    // so every file in the batch anchors to the same root even if the
    // paths individually resolve to deeper folders.
    const baseDir = autoOrganizeDrop ? commonParentDir(paths) : null;
    let ok = 0;
    const freshIds = new Set<string>();
    for (const p of paths) {
      const id = await importDroppedFile(p, baseDir);
      if (id) {
        ok++;
        freshIds.add(id);
      }
    }
    // Restore the active persona if the caller had `all` selected
    // and only chose one for the import — the buffered-drop flow
    // does not want to silently switch the sidebar filter.
    if (savedActive === null && pendingDropPaths.length > 0) {
      activeFilter.activePersona = savedActive;
    }
    status = `dropped ${ok}/${paths.length} into ${personaName(personaId)}`;
    if (freshIds.size > 0) {
      const merged = new Set(recentDropIds);
      for (const id of freshIds) merged.add(id);
      recentDropIds = merged;
    }
    pendingDropPaths = [];
    await loadTagCounts();
    await loadAssets();
    // Follow-up reload for cover_gen / thumb_gen — the initial
    // reload right after add_asset lands the row but the pipeline
    // hasn't produced cover / thumb yet, so the card renders as
    // a placeholder. A short delayed refetch swaps in the real
    // artefacts without asking the user to hit a manual refresh.
    scheduleImportRefreshes();
  }

  // Two follow-up reloads after an import, spaced to catch the
  // usual cover_gen (~0.5s) and thumb_gen (~1.5s) turnarounds. Runs
  // debounced so a batch of quick drops does not stack timers.
  let importRefreshTimer: number | null = null;
  function scheduleImportRefreshes() {
    if (importRefreshTimer !== null) {
      window.clearTimeout(importRefreshTimer);
    }
    importRefreshTimer = window.setTimeout(async () => {
      importRefreshTimer = null;
      try {
        await loadTagCounts();
        await loadAssets();
      } catch (error) {
        console.warn("post-import refresh failed", error);
      }
      // Second sweep for slower thumb sizes.
      window.setTimeout(() => {
        void loadAssets();
      }, 2000);
    }, 800);
  }

  // Clipboard paste: grab an image from `navigator.clipboard.read()`
  // and hand the bytes to the Tauri side (`paste_image_import`),
  // which writes them under `~/Pictures/Asterism/pasted/` and
  // dispatches `add_asset` in one call. The clipboard read API
  // needs the app window focused; WKWebView honours it. Falls
  // through with a status message when the clipboard has no image
  // or no persona is active (same fallback as the drop path).
  async function pasteFromClipboard() {
    if (activeFilter.activePersona === null) {
      status = "paste: pick a persona first";
      return;
    }
    let items: ClipboardItems;
    try {
      items = await navigator.clipboard.read();
    } catch (error) {
      console.warn("clipboard.read failed", error);
      status = "paste: clipboard access blocked";
      return;
    }
    for (const item of items) {
      for (const type of item.types) {
        if (!type.startsWith("image/")) continue;
        try {
          const blob = await item.getType(type);
          const buf = new Uint8Array(await blob.arrayBuffer());
          const dto = await invoke<AssetDto>("paste_image_import", {
            command: {
              persona_id: activeFilter.activePersona,
              bytes: Array.from(buf),
              mime_type: type,
            },
          });
          status = `pasted ${dto.locator.split("/").pop() ?? "image"} into ${personaName(activeFilter.activePersona)}`;
          const merged = new Set(recentDropIds);
          merged.add(dto.id);
          recentDropIds = merged;
          await loadTagCounts();
          await loadAssets();
          scheduleImportRefreshes();
          return;
        } catch (error) {
          console.warn("paste_image_import failed", error);
          status = `paste error: ${JSON.stringify(error)}`;
          return;
        }
      }
    }
    status = "paste: no image on the clipboard";
  }

  async function importDroppedFile(
    path: string,
    baseDir: string | null = null,
  ): Promise<string | null> {
    if (activeFilter.activePersona === null) return null;
    // macOS screenshots dragged out of the preview thumbnail live
    // in `/private/var/folders/…/TemporaryItems/` and get GC'd
    // when the preview closes; a locator into that path breaks
    // fullscreen playback and every follow-up read. The Rust side
    // copies TEMP paths into `~/Pictures/Asterism/dropped/` so
    // both the file and its assetProtocol scope survive.
    let durablePath = path;
    try {
      durablePath = await invoke<string>("rehome_dropped_path", { source: path });
    } catch (error) {
      console.warn("rehome_dropped_path failed", path, error);
    }
    try {
      const dto = await invoke<AssetDto>("add_asset", {
        command: {
          persona_id: activeFilter.activePersona,
          source_kind: "fs",
          locator: durablePath,
          modality: null,
          occurred_at_ms: Date.now(),
          session_id: null,
          labels: ["dropped"],
          register_note: null,
          platform: null,
          file_size_bytes: null,
          duration_ms: null,
          extra_json: null,
          cover_hint: null,
          auto_organize_base_dir: baseDir,
        },
      });
      return dto.id;
    } catch (error) {
      console.warn("add_asset (drop) failed", durablePath, error);
      return null;
    }
  }
  // Dragging-over highlight + "just imported" filter. The three
  // events fire in the order Tauri emits them: enter → over (many)
  // → drop / leave. The last two clear the highlight; drop also
  // dispatches the actual import.
  let dropOverlay = $state(false);
  let recentDropIds = $state<Set<string>>(new Set());
  let filterRecentDrops = $state(false);
  // Internal drag flag — set while an HTML5 draggable row (persona
  // / modality reorder) is being dragged. The Tauri file-drop
  // listener sometimes echoes on those internal drags depending on
  // the WKWebView build; the flag lets us swallow the overlay + the
  // import dispatch so a persona reorder does not accidentally
  // dispatch add_asset on an empty payload.
  let internalDragging = $state(false);
  // Buffered drop paths waiting on a persona choice. When a drop
  // lands while `activeFilter.activePersona === null`, the paths pile up here and
  // a modal asks the user to pick a persona. On pick, the flush
  // runs against the chosen persona and the buffer clears.
  let pendingDropPaths = $state<string[]>([]);
  $effect(() => {
    let unlistenDrop: UnlistenFn | undefined;
    let unlistenEnter: UnlistenFn | undefined;
    let unlistenLeave: UnlistenFn | undefined;
    let cancelled = false;
    listen<{ paths?: string[] }>("tauri://drag-enter", (evt) => {
      // Guard against internal HTML5 drags (persona / modality
      // reorder). If we somehow get echoed here without any file
      // paths, treat it as an internal drag and skip.
      if (internalDragging) return;
      if ((evt.payload?.paths ?? []).length === 0) return;
      dropOverlay = true;
    }).then((fn) => {
      if (cancelled) fn(); else unlistenEnter = fn;
    });
    listen<unknown>("tauri://drag-leave", () => {
      dropOverlay = false;
    }).then((fn) => {
      if (cancelled) fn(); else unlistenLeave = fn;
    });
    listen<{ paths?: string[]; position?: { x: number; y: number } }>(
      "tauri://drag-drop",
      async (evt) => {
        dropOverlay = false;
        const paths = evt.payload?.paths ?? [];
        if (paths.length === 0) return;
        if (internalDragging) return;
        // Sidebar-persona targeting: if the drop lands on top of a
        // persona row in the sidebar, that persona wins over the
        // currently-active filter — matches the "drop on the folder
        // to file it there" affordance from Finder / Eagle. We hit-
        // test via `elementFromPoint` at the drop coordinates (the
        // Tauri payload reports physical pixels, so scale by
        // `devicePixelRatio` back to CSS space).
        let targetPersona = activeFilter.activePersona;
        const pos = evt.payload?.position;
        if (pos) {
          const dpr = window.devicePixelRatio || 1;
          const el = document.elementFromPoint(pos.x / dpr, pos.y / dpr);
          const dropTarget = el?.closest("[data-persona-id]") as HTMLElement | null;
          const hitId = dropTarget?.dataset?.personaId;
          if (hitId) targetPersona = hitId;
        }
        // Still no persona → buffer the paths and surface a modal
        // that asks the user to pick one.
        if (targetPersona === null) {
          pendingDropPaths = paths;
          return;
        }
        await runDropImport(paths, targetPersona);
      },
    ).then((fn) => {
      if (cancelled) fn(); else unlistenDrop = fn;
    });
    return () => {
      cancelled = true;
      unlistenEnter?.();
      unlistenLeave?.();
      unlistenDrop?.();
    };
  });

  // `jobs:tick` listener + `noteJobTick` dropped in wave C. The
  // per-event accumulator (`jobTickerBanner` derived) was never
  // consumed, so the whole listener is dead. `activeKindGauges`
  // via the 3-s `jobs_stats` poll is the only live source of
  // the ticker banner.

  // Auto-dismiss timer for the constellation-burst panel. The panel
  // needs to survive the pointer jumping from a card to the panel
  // itself (users click same-day links inside it), so we can't
  // dismiss on card leave immediately. A short grace period lets
  // the pointer settle onto the panel; entering the panel cancels
  // the timer, leaving the panel restarts it.
  let burstCloseTimer: ReturnType<typeof setTimeout> | undefined;
  const BURST_CLOSE_MS = 400;
  function scheduleBurstClose() {
    // A pinned burst never auto-closes (W4) — release is explicit
    // (Esc / ✦ re-click). Checked again inside the timeout so a pin
    // landing after the schedule still wins.
    if (burst?.pinned) return;
    clearTimeout(burstCloseTimer);
    burstCloseTimer = setTimeout(() => {
      if (burst?.pinned) return;
      burst = null;
    }, BURST_CLOSE_MS);
  }
  function cancelBurstClose() {
    clearTimeout(burstCloseTimer);
  }

  // Open the constellation burst anchored beside the card. The
  // card-relative placement makes "which card is this the
  // constellation of" obvious. Two entries, one open path
  // (W1 hover regrammar + W3 keymap): aim
  // or click the ✦ icon, or press ⇧Space on the hovered / selected
  // card — the card body itself never opens the burst.
  async function openConstellationAt(assetId: string, rect: DOMRect, pinned = false) {
    // Cancel the action-overlay close grace.
    cancelBurstClose();
    if (cardActionCloseTimer !== null) {
      window.clearTimeout(cardActionCloseTimer);
      cardActionCloseTimer = null;
    }
    // One panel at a time — close the text overlays.
    cardThreadHover = null;
    cardNoteHover = null;
    // Anchor to the card's right edge; flip left when the panel would
    // overflow the viewport, and clamp the top so it stays on screen
    // (mirrors the note / thread overlay placement math). Panel width
    // matches `.burst { width: 260px }` in ConstellationBurst.svelte.
    const PANEL_W = 260;
    let x = rect.right + 6;
    if (x + PANEL_W > window.innerWidth) {
      x = Math.max(8, rect.left - PANEL_W - 6);
    }
    const y = Math.max(8, Math.min(rect.top, window.innerHeight - 340));
    const anchor = { x, y };
    // Reuse an already-loaded burst for the same card — re-anchor,
    // and a pin request upgrades it in place (never downgrades: an
    // explicit unpinned re-open of a pinned card keeps the pin).
    if (burst && burst.assetId === assetId && burst.items.length > 0) {
      const keep = burst.pinned || pinned;
      burst = { ...burst, anchor, pinned: keep };
      if (keep) interaction.push("pinnedBurst");
      return;
    }
    try {
      const items = await invoke<ConstellationItemDto[]>("asset_constellation", {
        assetId,
        viewerSubject: null,
        limit: 3,
      });
      // Stack bookkeeping happens together with the burst swap —
      // never before the await, or a still-visible pinned panel
      // would lose its stack entry for the IPC round-trip (Escape
      // would clear the selection instead of closing it, review M2).
      burst = { assetId, items, anchor, pinned };
      if (pinned) interaction.push("pinnedBurst");
      else interaction.remove("pinnedBurst");
      recordEvent("burst_open", {
        personaId: activeFilter.activePersona,
        payload: { asset_id: assetId, items: items.length, anchored: true },
      });
    } catch {
      closeBurst();
    }
  }

  async function openCardConstellationFromIcon(
    card: AssetCardDto,
    ev: MouseEvent,
    pinned = false,
  ) {
    const cardEl = (ev.currentTarget as HTMLElement).closest(".card") as HTMLElement | null;
    const rect = (cardEl ?? (ev.currentTarget as HTMLElement)).getBoundingClientRect();
    await openConstellationAt(card.id, rect, pinned);
  }

  // ✦ click = pin toggle (W4): first click pins the panel, a second
  // click on the same card releases it. A click on another card's ✦
  // moves the pin there.
  function onConstellationIconClick(card: AssetCardDto, ev: MouseEvent) {
    if (burst?.assetId === card.id && burst.pinned) {
      closeBurst();
      return;
    }
    void openCardConstellationFromIcon(card, ev, true);
  }

  // ⇧Space entry: anchor beside the target card's DOM node, pinned —
  // a keyboard open has no hovering pointer to keep it alive, so it
  // persists until Esc / a second ⇧Space. A target scrolled out of
  // the virtual window has no node — no-op then (the shortcut is
  // aimed at what you can see).
  async function openConstellationFromKeyboard(id: string) {
    if (burst?.assetId === id && burst.pinned) {
      closeBurst();
      return;
    }
    const el = gridWrapperEl?.querySelector<HTMLElement>(
      `.card[data-asset-id="${id}"]`,
    );
    if (!el) return;
    await openConstellationAt(id, el.getBoundingClientRect(), true);
  }

  // Which card the pointer is currently over. Feeds the keyboard
  // `0`-`5` rating shortcut so a rate is applied to the card the user
  // is looking at without requiring a preceding click.
  let hoveredCardId = $state<string | null>(null);

  // True while the User is mid-selection or has a blocking overlay
  // open (marquee sweep / context menu, tracked on the interaction
  // mode stack) — aim-hover opens (✦ burst, sidebar ⓘ)
  // are suppressed then so a rapid ⌘/Shift-click run isn't fought
  // by a panel popping under the pointer. Explicit clicks bypass
  // this guard.
  function overlaysSuppressed(): boolean {
    // Threshold is >1, not >0: a single-card selection (Quick Look
    // sync, a lone ⌘-click) is not bulk intent — killing ✦ / ⓘ
    // aim-hover there would disable them needlessly. Suppression is
    // for multi-select runs.
    return gridSelection.selectedIds.size > 1 || interaction.overlayActive;
  }

  function onCardEnter(card: AssetCardDto) {
    hoveredCardId = card.id;
    // W1 hover regrammar:
    // hovering the card body opens nothing. It only tracks the card
    // for the keyboard rating shortcut, reveals the action-icon
    // strip (CSS), and pre-warms the 512 px detail thumb. The burst
    // opens from the ✦ icon (aim / click) alone.
    if (cardIsVisual(card)) {
      void thumbCatalog.ensureThumb(card.id, 512);
    }
  }

  function onCardLeave() {
    scheduleBurstClose();
    hoveredCardId = null;
    // Give the action overlays a grace window so a hop from the
    // card onto one of them (Thread / Note) does not blink out.
    scheduleCardActionClose();
  }

  /**
   * Opens the Thread overlay from the card's action-icon hover.
   * Anchor point mirrors the previous "right edge of card" pattern.
   */
  async function openCardThreadFromIcon(card: AssetCardDto, ev: MouseEvent) {
    // Icon-hover cancels a pending close from a sibling icon so a
    // quick sweep across the strip does not blink through overlays.
    if (cardActionCloseTimer !== null) {
      window.clearTimeout(cardActionCloseTimer);
      cardActionCloseTimer = null;
    }
    // Close a Note overlay if one is up — one action panel at a
    // time so the two never fight for space.
    cardNoteHover = null;
    // The icon strip sits inside the card, so the card's rect is
    // still the correct anchor.
    const cardEl = (ev.currentTarget as HTMLElement).closest(".card") as HTMLElement | null;
    const rect = (cardEl ?? (ev.currentTarget as HTMLElement)).getBoundingClientRect();
    let comments: AssetCommentDto[] = [];
    try {
      comments = await invoke<AssetCommentDto[]>("list_asset_comments", {
        assetId: card.id,
      });
    } catch {
      comments = [];
    }
    cardThreadHover = {
      assetId: card.id,
      x: Math.min(rect.right + 6, window.innerWidth - 320),
      y: Math.min(rect.top, window.innerHeight - 340),
      comments,
      draft: "",
      authorKind: "user",
      posting: false,
    };
  }

  /**
   * Opens the Note overlay (register_note edit) from the icon strip.
   */
  async function openCardNoteFromIcon(card: AssetCardDto, ev: MouseEvent) {
    if (cardActionCloseTimer !== null) {
      window.clearTimeout(cardActionCloseTimer);
      cardActionCloseTimer = null;
    }
    cardThreadHover = null;
    const cardEl = (ev.currentTarget as HTMLElement).closest(".card") as HTMLElement | null;
    const rect = (cardEl ?? (ev.currentTarget as HTMLElement)).getBoundingClientRect();
    // Fetch the current register_note off the AssetDto so the
    // textarea is prefilled with what is already persisted.
    let note = "";
    try {
      const dto = await invoke<AssetDetailDto>("asset_detail", {
        query: { asset_id: card.id, viewer_subject: null },
      }).then((d) => d.asset);
      note = dto.register_note ?? "";
    } catch {}
    cardNoteHover = {
      kind: "asset",
      targetId: card.id,
      x: Math.min(rect.right + 6, window.innerWidth - 320),
      y: Math.min(rect.top, window.innerHeight - 200),
      draft: note,
      saving: false,
    };
  }

  /**
   * Session tile note opener — same overlay UI as the grid Card path,
   * different save target (session patch_metadata). Called by
   * SessionsView via a shared prop so both surfaces route into
   * `cardNoteHover` without duplicating the overlay markup.
   */
  function openSessionNoteFromIcon(
    session: import("./bindings").SessionDto,
    ev: MouseEvent,
  ) {
    if (cardActionCloseTimer !== null) {
      window.clearTimeout(cardActionCloseTimer);
      cardActionCloseTimer = null;
    }
    cardThreadHover = null;
    const tileEl = (ev.currentTarget as HTMLElement).closest(
      ".session-tile",
    ) as HTMLElement | null;
    const rect = (tileEl ?? (ev.currentTarget as HTMLElement)).getBoundingClientRect();
    cardNoteHover = {
      kind: "session",
      targetId: session.id,
      x: Math.min(rect.right + 6, window.innerWidth - 320),
      y: Math.min(rect.top, window.innerHeight - 200),
      draft: session.note ?? "",
      saving: false,
    };
  }

  function scheduleCardActionClose() {
    if (cardActionCloseTimer !== null) window.clearTimeout(cardActionCloseTimer);
    cardActionCloseTimer = window.setTimeout(() => {
      cardActionCloseTimer = null;
      cardThreadHover = null;
      cardNoteHover = null;
    }, CARD_ACTION_CLOSE_GRACE_MS);
  }

  function onCardActionOverlayEnter() {
    if (cardActionCloseTimer !== null) {
      window.clearTimeout(cardActionCloseTimer);
      cardActionCloseTimer = null;
    }
  }

  async function saveCardNote() {
    if (!cardNoteHover) return;
    const body = cardNoteHover.draft.trim();
    const snapshot = cardNoteHover;
    cardNoteHover = { ...cardNoteHover, saving: true };
    try {
      if (snapshot.kind === "asset") {
        const dto = await invoke<AssetDto>("update_asset_meta", {
          command: {
            asset_id: snapshot.targetId,
            labels: null,
            register_note: body,
            cover: null,
            rating: null,
          },
        });
        assetPageCatalog.patchCard(snapshot.targetId, {
          has_note: !!dto.register_note,
        });
      } else {
        // Session note: patch_metadata is COALESCE per field (None =
        // leave unchanged), so an empty string is the practical clear
        // — the tile hides the icon accent when the string is empty.
        // A dedicated NULL-write endpoint is P4+ territory.
        await sessionCatalog.patchMetadata(snapshot.targetId, {
          note: body,
        });
      }
      cardNoteHover = cardNoteHover ? { ...cardNoteHover, saving: false } : null;
    } catch (err) {
      console.warn("note save failed", err);
      cardNoteHover = cardNoteHover ? { ...cardNoteHover, saving: false } : null;
    }
  }

  async function postCardThreadDraft() {
    if (!cardThreadHover) return;
    const body = cardThreadHover.draft.trim();
    if (!body) return;
    let author_persona_id: string | null = null;
    if (cardThreadHover.authorKind === "persona") {
      author_persona_id = activeFilter.activePersona;
      if (!author_persona_id) return; // toggle guarded, defence
    }
    cardThreadHover = { ...cardThreadHover, posting: true };
    try {
      const created = await invoke<AssetCommentDto>("post_asset_comment", {
        command: {
          asset_id: cardThreadHover.assetId,
          author_kind: cardThreadHover.authorKind,
          author_persona_id,
          body,
        },
      });
      cardThreadHover = {
        ...cardThreadHover,
        comments: [...cardThreadHover.comments, created],
        draft: "",
        posting: false,
      };
      // Reflect the has_thread badge locally so the card lights up
      // immediately, without waiting for a full page reload.
      assetPageCatalog.patchCard(cardThreadHover.assetId, { has_thread: true });
    } catch (err) {
      console.warn("card thread post failed", err);
      cardThreadHover = { ...cardThreadHover, posting: false };
    }
  }

  /**
   * Persists a star rating on `assetId` (0 = unrated / clear). Round-
   * trips the DTO from the backend and mutates any locally-cached
   * card projection so the star fill updates without a full grid
   * reload.
   */
  /**
   * Persists a new label list on the detail asset via
   * `update_asset_meta`. Refreshes `detail` + hydration cache so the
   * chip strip on the grid updates without a full page reload.
   */
  async function saveLabels(assetId: string, next: string[]) {
    try {
      const dto = await invoke<AssetDto>("update_asset_meta", {
        command: {
          asset_id: assetId,
          labels: next,
          register_note: null,
          cover: null,
          rating: null,
        },
      });
      assetPageCatalog.patchCard(assetId, { labels: dto.labels });
    } catch (err) {
      console.warn("saveLabels failed", err);
    }
  }

  async function setRating(assetId: string, rating: number) {
    // Rating is a live-set act — curating something on its way out is
    // not a coherent thing to offer, and the keyboard shortcut at
    // :4165 reaches this without going through a card affordance, so
    // the guard belongs here rather than on the widget.
    if (assetPageCatalog.pageIsTrash) {
      status = "restore it first — trashed items cannot be rated";
      return;
    }
    const clamped = Math.max(0, Math.min(5, Math.round(rating)));
    try {
      const dto = await invoke<AssetDto>("update_asset_meta", {
        command: {
          asset_id: assetId,
          labels: null,
          register_note: null,
          cover: null,
          rating: clamped,
        },
      });
      // Reflect the new value on the in-memory page + hydration cache
      // so the star widget re-renders without an extra grid fetch.
      assetPageCatalog.patchCard(assetId, { rating: dto.rating ?? null });
    } catch (err) {
      console.warn("setRating failed", err);
    }
  }

  // Constellation-burst panel (positioning / drag / presentation
  // helpers / select-all pivot) moved to `ConstellationBurst.svelte`
  // (wave ④). App keeps the `burst` payload + the hover
  // fetch / close-grace timers (grid-card-adjacent lifecycle).

  // -------------------------------------------------------------------
  // Selector: grid multi-select gesture + action-bar plumbing.
  // -------------------------------------------------------------------
  //
  // Click semantics on a grid card:
  //   - No modifier        → openDetail(id), dropping any selection
  //                          (commit-to-one). The W2 experiment with
  //                          Finder-style "bare click = exclusive
  //                          select" was rejected in dogfood: in a
  //                          browse-first grid a single click is not
  //                          a select gesture — the click IS the open
  //                          gesture.
  //   - Ctrl / ⌘ (metaKey) → toggle {id} in the selection
  //   - Shift              → extend from gridSelection.lastAnchorId
  //                          to id along the `filteredRows` order
  //   - Right-click        → retargets the selection when outside it
  //                          (openCardMenu), so single-card menu ops
  //                          need no prior select
  // Exits: background click / Escape clear the selection.
  function onCardClick(event: MouseEvent, id: string) {
    // A sweep that started + ended on a card fires a trailing click on
    // mouseup — swallow it once so the rubber-band select does not also
    // open the detail pane (W0 fix). Consumed exactly once; onMarqueeUp
    // clears the flag on the next tick regardless.
    if (marqueeJustSwept) {
      marqueeJustSwept = false;
      return;
    }
    // Same one-shot swallow for a drag: `pointerup` is followed by a
    // `click` on the source card, which would open the very card the
    // user just dropped somewhere else.
    if (cardDrag.justDropped) {
      cardDrag.justDropped = false;
      return;
    }
    if (event.metaKey || event.ctrlKey) {
      event.preventDefault();
      if (gridSelection.selectedIds.has(id)) {
        gridSelection.selectedIds.delete(id);
        if (gridSelection.lastAnchorId === id) {
          gridSelection.lastAnchorId = gridSelection.selectedIds.size ? Array.from(gridSelection.selectedIds).at(-1) ?? null : null;
        }
      } else {
        gridSelection.selectedIds.add(id);
        gridSelection.lastAnchorId = id;
      }
      return;
    }
    if (event.shiftKey && gridSelection.lastAnchorId !== null) {
      event.preventDefault();
      const order = flatCardIds();
      const a = order.indexOf(gridSelection.lastAnchorId);
      const b = order.indexOf(id);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a <= b ? [a, b] : [b, a];
        for (let i = lo; i <= hi; i++) gridSelection.selectedIds.add(order[i]);
      } else {
        gridSelection.selectedIds.add(id);
      }
      gridSelection.lastAnchorId = id;
      return;
    }
    // Bare click = commit-to-one: drop the multi-selection (if any)
    // and open the card's content. The clicked card becomes the focused
    // card, so there is no "0 selected, nothing focused" limbo, and
    // background click / Escape stay the selection exits.
    if (gridSelection.selectedIds.size > 0) {
      gridSelection.selectedIds.clear();
      gridSelection.lastAnchorId = null;
    }
    // Both roles go to the detail pane, which is where the metadata
    // side lives (title / modality / labels / note / thread). What
    // differs is the *left* half: an item shows its body, a container
    // shows its members as a transcript. Routing containers to the
    // Reader overlay instead gave them the transcript but took the
    // metadata pane away, so a session could be read and not named.
    void openDetail(id);
  }

  function flatCardIds(): string[] {
    const out: string[] = [];
    for (const row of filteredRows) {
      if (row.kind === "cards") {
        // Session tiles do not participate in grid selection /
        // keyboard nav — the Session id is not an asset id and the
        // click gesture opens a Reader instead of selecting. Only
        // Message items feed the id list.
        for (const item of row.items) {
          if (item.kind === "message") out.push(item.card.id);
        }
      }
    }
    return out;
  }

  function clearSelection() {
    gridSelection.selectedIds.clear();
    gridSelection.lastAnchorId = null;
    bulkModalityOpen = false;
    bulkTagOpen = false;
  }

  // ---- Rubber-band (marquee) selection ----
  // Drag on the grid *background* (never on a card — cards own their
  // click / HTML5 drag-to-group / reorder gestures) to sweep a
  // rectangle over the rendered cards. No modifier replaces the
  // selection; ⌘/Ctrl adds to it (Shift+drag is left undefined so it
  // never competes with Shift range-click). A sub-threshold,
  // no-modifier background click clears the selection (rework 6.3b —
  // one of the three documented exits from a live selection).
  // Intersection is viewport-space (getBoundingClientRect vs the rect)
  // over the *currently rendered* cards only; off-screen virtualised
  // rows are intentionally not swept (auto-scroll is out of scope).
  const MARQUEE_THRESHOLD_PX = 4;
  let marquee = $state<{
    x0: number;
    y0: number;
    x: number;
    y: number;
    additive: boolean;
    // Whether the mousedown landed on a card (vs the grid
    // background). A sub-threshold, no-modifier click only clears the
    // selection when it started on the background — a card-origin
    // click stays a normal card click (open / toggle / range).
    onCard: boolean;
  } | null>(null);
  // `true` once the drag passes the threshold — until then it might
  // still be a plain click, so nothing is drawn or selected.
  let marqueeActive = $state(false);
  // Selection snapshot captured at drag start. Doubles as the ⌘/Ctrl
  // additive base (read inside the apply loop) and the restore target
  // when the sweep is cancelled with Escape. Plain / non-reactive.
  let marqueeBase = new Set<string>();
  // Set for one tick after a threshold-crossing sweep so the trailing
  // `click` (fired on mouseup, after `onMarqueeUp`) does not re-open
  // the card the sweep started on. Consumed once in `onCardClick`.
  let marqueeJustSwept = false;
  let marqueeRaf = 0;

  function onGridMouseDown(e: MouseEvent) {
    // Left button only; never start on an interactive control inside
    // the grid chrome (its own click / focus gestures stay intact).
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (!target || target.closest("button, a, input, select, textarea")) {
      return;
    }
    const onCard = target.closest(".card") !== null;
    // In Messages view cards own the HTML5 drag-to-group / reorder
    // gesture, so a card-origin drag must not become a marquee. In
    // every other view cards are `draggable="false"` and tile the
    // grid — without this branch there is essentially nowhere left to
    // begin a sweep, which is the "marquee never even appears" bug
    // (W0). Card-origin
    // marquees are therefore allowed exactly when dragging is off.
    if (draggableActive && onCard) return;
    // Suppress the browser's native image drag (the thumb `<img>`) and
    // text selection so the sweep owns the pointer stream. `click`
    // still fires on mouseup, so a plain card click keeps opening the
    // detail pane — only keyboard focus on the card is given up, which
    // is acceptable for a pointer gesture.
    e.preventDefault();
    const additive = e.metaKey || e.ctrlKey;
    marquee = {
      x0: e.clientX,
      y0: e.clientY,
      x: e.clientX,
      y: e.clientY,
      additive,
      onCard,
    };
    marqueeActive = false;
    // Always snapshot the live selection: the additive apply loop
    // reads it as its base, and Escape restores to it.
    marqueeBase = new Set(gridSelection.selectedIds);
    window.addEventListener("mousemove", onMarqueeMove);
    window.addEventListener("mouseup", onMarqueeUp);
    // Capture-phase so the cancel pre-empts the global Escape chain
    // (`onWindowKeydown`); added / removed with the sweep listeners.
    window.addEventListener("keydown", onMarqueeKeydown, true);
    // If the button is released outside the webview (⌘-tab mid-
    // sweep), `mouseup` never arrives — tear the sweep down on
    // window blur so `marqueeActive` / the "marquee" stack entry
    // can't strand `overlaysSuppressed()` permanently true.
    window.addEventListener("blur", onMarqueeBlur);
  }

  // Blur teardown: drop the sweep without the mouseup click
  // semantics (no selection clear — the user is mid-app-switch,
  // not clicking the background).
  function onMarqueeBlur() {
    window.removeEventListener("mousemove", onMarqueeMove);
    window.removeEventListener("mouseup", onMarqueeUp);
    window.removeEventListener("keydown", onMarqueeKeydown, true);
    window.removeEventListener("blur", onMarqueeBlur);
    if (marqueeRaf !== 0) {
      cancelAnimationFrame(marqueeRaf);
      marqueeRaf = 0;
    }
    marquee = null;
    marqueeActive = false;
    interaction.remove("marquee");
    marqueeBase = new Set();
  }

  function onMarqueeMove(e: MouseEvent) {
    if (!marquee) return;
    if (!marqueeActive) {
      const dx = e.clientX - marquee.x0;
      const dy = e.clientY - marquee.y0;
      if (Math.hypot(dx, dy) < MARQUEE_THRESHOLD_PX) return;
      // Crossed the threshold — this is a marquee, not a click.
      marqueeActive = true;
      interaction.push("marquee");
    }
    // Suppress the browser's native text selection during the sweep.
    e.preventDefault();
    marquee = { ...marquee, x: e.clientX, y: e.clientY };
    if (marqueeRaf === 0) {
      marqueeRaf = requestAnimationFrame(() => {
        marqueeRaf = 0;
        applyMarqueeSelection();
      });
    }
  }

  function applyMarqueeSelection() {
    if (!marquee || !gridWrapperEl) return;
    const left = Math.min(marquee.x0, marquee.x);
    const right = Math.max(marquee.x0, marquee.x);
    const top = Math.min(marquee.y0, marquee.y);
    const bottom = Math.max(marquee.y0, marquee.y);
    const next = new Set<string>(marquee.additive ? marqueeBase : []);
    const cards = gridWrapperEl.querySelectorAll<HTMLElement>(
      ".card[data-asset-id]",
    );
    for (const el of cards) {
      const r = el.getBoundingClientRect();
      // AABB overlap test in viewport space.
      if (r.left < right && r.right > left && r.top < bottom && r.bottom > top) {
        const id = el.dataset.assetId;
        if (id) next.add(id);
      }
    }
    // Minimal diff against the reactive SvelteSet so only changed cards
    // re-render (avoids clearing + re-adding a large selection each
    // frame).
    const cur = gridSelection.selectedIds;
    for (const id of [...cur]) if (!next.has(id)) cur.delete(id);
    for (const id of next) if (!cur.has(id)) cur.add(id);
  }

  function onMarqueeUp() {
    window.removeEventListener("mousemove", onMarqueeMove);
    window.removeEventListener("mouseup", onMarqueeUp);
    window.removeEventListener("keydown", onMarqueeKeydown, true);
    window.removeEventListener("blur", onMarqueeBlur);
    if (marqueeRaf !== 0) {
      cancelAnimationFrame(marqueeRaf);
      marqueeRaf = 0;
    }
    const wasActive = marqueeActive;
    const additive = marquee?.additive ?? false;
    const startedOnCard = marquee?.onCard ?? false;
    marquee = null;
    marqueeActive = false;
    interaction.remove("marquee");
    marqueeBase = new Set();
    if (wasActive) {
      // A threshold-crossing sweep that started on a card ends on one
      // too, so a trailing `click` follows this mouseup — swallow the
      // next `onCardClick` so the sweep does not also open the detail
      // pane. The timeout clears the flag when the sweep ended on the
      // background and no click follows.
      marqueeJustSwept = true;
      setTimeout(() => (marqueeJustSwept = false), 0);
      return;
    }
    // Sub-threshold (a plain click). A no-modifier click on the grid
    // *background* clears a live selection (rework 6.3b). A card-origin
    // click is left to `onCardClick` (open / ⌘-toggle / Shift-range),
    // and ⌘/Ctrl clicks are additive gestures — neither clears here.
    if (!additive && !startedOnCard && gridSelection.selectedIds.size > 0) {
      clearSelection();
    }
  }

  // Escape cancels an in-flight sweep: tear down the listeners, drop
  // the overlay, and restore the selection to the drag-start snapshot
  // (`marqueeBase`). Registered capture-phase in `onGridMouseDown` so
  // it pre-empts the global Escape chain; only acts while a sweep is
  // live.
  function onMarqueeKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape" || !marquee) return;
    e.preventDefault();
    // Stop the global `onWindowKeydown` Escape chain from also firing —
    // it would clear the selection instead of restoring it.
    e.stopImmediatePropagation();
    window.removeEventListener("mousemove", onMarqueeMove);
    window.removeEventListener("mouseup", onMarqueeUp);
    window.removeEventListener("keydown", onMarqueeKeydown, true);
    window.removeEventListener("blur", onMarqueeBlur);
    if (marqueeRaf !== 0) {
      cancelAnimationFrame(marqueeRaf);
      marqueeRaf = 0;
    }
    // Restore the selection to the start-of-drag snapshot with a
    // minimal diff so only the cards that changed re-render.
    const cur = gridSelection.selectedIds;
    for (const id of [...cur]) if (!marqueeBase.has(id)) cur.delete(id);
    for (const id of marqueeBase) if (!cur.has(id)) cur.add(id);
    marquee = null;
    marqueeActive = false;
    interaction.remove("marquee");
    marqueeBase = new Set();
  }

  // Overlay geometry (viewport-fixed). Only rendered once the drag is
  // active so a stationary mousedown draws nothing.
  let marqueeRect = $derived(
    marquee
      ? {
          left: Math.min(marquee.x0, marquee.x),
          top: Math.min(marquee.y0, marquee.y),
          width: Math.abs(marquee.x - marquee.x0),
          height: Math.abs(marquee.y - marquee.y0),
        }
      : null,
  );

  // ---- Bulk actions on the grid multi-select ----
  // In-flight guard so a double-click doesn't fire two batch commands.
  let bulkBusy = $state(false);
  // Popover toggles for the menu-style bulk actions.
  let bulkModalityOpen = $state(false);
  let bulkTagOpen = $state(false);
  let bulkGroupOpen = $state(false);
  // Draft for the "add tag to selection" input.
  let bulkTagInput = $state("");

  // Resolve a selected id to its card. Prefers the hydration cache
  // (full card) over the light index item; both carry `labels` +
  // `modality`, which is all the bulk ops read.
  function cardById(id: string): AssetCardDto | undefined {
    const hit = assetPageCatalog.hydration.get(id);
    if (hit) return hit;
    return assetPageCatalog.page?.items.find((c) => c.id === id);
  }

  function modalityLabelOf(slug: string): string {
    return modalityCatalog.labelOf(slug);
  }

  // Remove the `inbox` label from every selected card. `labels` is a
  // full-replace field, so each card keeps its other labels — only
  // "inbox" is filtered out. Cards not carrying the label are skipped.
  async function bulkRemoveFromInbox() {
    const ids = Array.from(gridSelection.selectedIds);
    if (ids.length === 0 || bulkBusy) return;
    const items = [];
    for (const id of ids) {
      const card = cardById(id);
      if (!card || !card.labels.includes("inbox")) continue;
      items.push({
        asset_id: id,
        labels: card.labels.filter((l) => l !== "inbox"),
        register_note: null,
        cover: null,
        rating: null,
        modality: null,
      });
    }
    if (items.length === 0) {
      status = "selection has no Inbox items";
      return;
    }
    bulkBusy = true;
    try {
      const res = await invoke<UpdateAssetMetaBatchResult>(
        "update_asset_meta_batch",
        { command: { items } },
      );
      for (const it of items) {
        assetPageCatalog.patchCard(it.asset_id, { labels: it.labels });
      }
      status =
        res.failure_count > 0
          ? `Inbox: ${res.success_count} cleared, ${res.failure_count} failed`
          : `cleared ${res.success_count} from Inbox`;
      // While the Inbox filter is engaged the graduated cards must
      // leave the grid — a membership change the key-skip cache would
      // otherwise hide (see assetPageCatalog.invalidateKey).
      if (activeFilter.activeLabel === "inbox") {
        assetPageCatalog.invalidateKey();
        await loadAssets();
      }
      clearSelection();
    } catch (err) {
      console.warn("bulk remove from inbox failed", err);
      status = `Inbox bulk error: ${JSON.stringify(err)}`;
    } finally {
      bulkBusy = false;
    }
  }

  // Move every selected card to one modality. `modality` is the only
  // Some field; labels / note / cover / rating stay untouched.
  async function bulkMoveModality(slug: string) {
    await moveToModality(Array.from(gridSelection.selectedIds), slug);
  }

  /**
   * Drop-onto-a-modality-row handler. Moves the dragged card, or the
   * whole selection when the dragged card is part of it — the rule
   * Finder uses, and the one that makes "select a few, drag them over"
   * work without a separate gesture.
   */
  async function moveDraggedToModality(assetId: string, slug: string) {
    const ids = gridSelection.selectedIds.has(assetId)
      ? Array.from(gridSelection.selectedIds)
      : [assetId];
    await moveToModality(ids, slug);
  }

  /** Sidebar Group as a drop target — the Are.na "drop into a channel". */
  async function addDraggedToGroup(assetId: string, groupId: string) {
    if (groupCatalog.isQueryGroup(groupId)) return;
    const ids = gridSelection.selectedIds.has(assetId)
      ? Array.from(gridSelection.selectedIds)
      : [assetId];
    const added: string[] = [];
    try {
      for (const id of ids) {
        await mutate(
          "add_asset_to_group",
          { command: { asset_id: id, group_id: groupId } },
          "add this to the group",
        );
        added.push(id);
        assetPageCatalog.invalidateDetail(id);
      }
    } catch (err) {
      // Was `JSON.stringify(err)` in the status line — a raw object
      // where a sentence belongs. `mutate` carries the reason now; this
      // side owes only the count, since the loop stops at the first
      // refusal and the rest of the selection never left.
      console.warn("add to group failed", err);
    }
    // Outside the `try`, for the same reason as `trashFromCard`: when
    // the loop stops half way, the ids before the refusal really did
    // join the group, and a status line claiming so beside a sidebar
    // count and a grid that still show the old state would be the
    // interface disagreeing with itself. Safe on the same terms, over
    // two functions rather than one: `loadCounts` is Resource-backed
    // and `loadAssets` goes through `assetPageCatalog.loadPage`, which
    // reports through a boolean and `.error` rather than throwing.
    if (added.length > 0) {
      await groupCatalog.loadCounts(activeFilter.activePersona);
      // If the destination is the active filter the new members have to
      // appear, which the key-skip cache would otherwise hide.
      if (activeFilter.activeGroupIds.has(groupId)) {
        assetPageCatalog.invalidateKey();
        await loadAssets();
      }
    }
    status = summariseBulk(added.length, ids.length, {
      verb: "added",
      into: "to the group",
    });
  }

  /**
   * The "one card, or the whole selection it belongs to" rule, in one
   * place. A gesture aimed at a card that is part of the current
   * selection acts on all of it; a gesture aimed at a card outside the
   * selection acts on that card alone. Shared by trash / restore /
   * purge so the three cannot drift apart.
   */
  function selectionOrCard(assetId: string): string[] {
    return gridSelection.selectedIds.has(assetId)
      ? Array.from(gridSelection.selectedIds)
      : [assetId];
  }

  /**
   * Moves a card out of the live set — the one implementation behind
   * every way of asking for it: the drag onto the sidebar Trash row
   * (`onCardDropTarget`), the "Move to Trash" entry at the bottom of
   * the card context menu, and ⌘⌫ on a grid selection. The gestures
   * differ only in how the id arrives; everything after that —
   * expanding to the whole selection when the card is part of it (the
   * same rule the Group / Modality drops follow), dropping the rows
   * locally instead of repainting the grid, refreshing the sidebar
   * tallies — is one behaviour and lives here once.
   *
   * No confirm on any path: trashing is reversible (restore lives on
   * the card icons in trash view), so a confirm would only tax the
   * common case. The irreversible sibling — purge — does confirm.
   *
   * What it does offer is an Undo toast, and the offer is made here
   * for the same reason everything else is: all three gestures land on
   * this function, so wiring it at any one of them would leave the
   * other two without a way back. The toast is the near-term undo —
   * the trash view is still the long one — and it is armed only for
   * the rows that actually made it out, so an Undo cannot restore
   * something that was never trashed.
   */
  async function trashFromCard(assetId: string) {
    const ids = selectionOrCard(assetId);
    const trashed: string[] = [];
    try {
      for (const id of ids) {
        await mutate("trash_asset", { command: { asset_id: id } }, "move this to the trash");
        trashed.push(id);
        // The row left the live side: remove it locally rather than
        // repainting the whole grid (same reasoning as restoreAsset),
        // and drop it from the selection so a following bulk action
        // does not send a dead id.
        assetPageCatalog.dropItem(id);
        gridSelection.selectedIds.delete(id);
      }
    } catch (err) {
      // The reason is on screen already (`mutate`); repeating it in the
      // status line would put the same sentence in two places, in two
      // wordings. What the status line owes the user here is the part
      // the refusal cannot say: how much of what they asked for
      // actually happened. The loop stops at the first refusal, so
      // `trashed` is exactly that.
      console.warn("trash_asset failed", err);
    }
    if (trashed.length > 0) {
      // Live-side tallies shifted — refresh the sidebar counts. Outside
      // the `try` on purpose, so a partial trash still updates them;
      // safe because count loads go through `Resource.load`, which
      // catches and returns false rather than throwing
      // (`lib/stores/_resource.svelte.ts:60-66`). A count path that
      // bypassed `Resource` would skip the status line and the Undo
      // offer below.
      await loadSidebarCounts();
    }
    status = summariseBulk(trashed.length, ids.length, {
      verb: "moved",
      into: "to trash",
    });
    if (trashed.length > 0) {
      undoToastCatalog.show({
        message:
          trashed.length === 1 ? "Moved to Trash" : `Moved ${trashed.length} to Trash`,
        onAction: () => undoTrash(trashed),
      });
    }
  }

  /**
   * Puts back exactly what the last trash gesture took, from the
   * toast.
   *
   * Repaints instead of patching the page: the cards were dropped from
   * the grid on the way out, and the restored rows belong wherever the
   * current filter and sort put them — which is a question only a
   * reload can answer. `invalidateKey` is what makes the reload
   * happen; without it the key-skip cache would decide the page it
   * already has is the page being asked for.
   *
   * Works from either side of the trash toggle: the user has six
   * seconds to flip views, and the reload is correct for both (the
   * rows come back on the live side, and leave the trash one).
   */
  async function undoTrash(ids: string[]) {
    let restored = 0;
    for (const id of ids) {
      if (await restoreAsset(id)) restored += 1;
    }
    assetPageCatalog.invalidateKey();
    await loadAssets();
    await loadSidebarCounts();
    status = summariseBulk(restored, ids.length, { verb: "restored" });
  }


  /**
   * Restores a set of assets and refreshes the sidebar afterwards —
   * the whole-flow half that `restoreAsset` deliberately is not.
   *
   * The count refresh lives here rather than inside `restoreAsset`
   * because a set of ten restores is one change to the tallies, not
   * ten: a per-id refresh would fire four count queries per row and
   * paint intermediate numbers on the way to the same answer.
   */
  async function restoreMany(ids: string[]) {
    if (ids.length === 0) return;
    let restored = 0;
    for (const id of ids) {
      if (await restoreAsset(id)) restored += 1;
    }
    // Both sides of the trash shifted — the counts follow whichever
    // one the grid is showing.
    await loadSidebarCounts();
    // A partial result is worth saying even for one id, where
    // `restoreAsset` has already written "restored" optimistically.
    if (ids.length > 1 || restored !== ids.length) {
      status = summariseBulk(restored, ids.length, { verb: "restored" });
    }
  }

  /**
   * Trash-view counterpart of `trashFromCard`: brings the card — or
   * the whole selection it belongs to — back to the live side. Used by
   * the "Restore" entry in the trash-view context menu; the ↩︎ icon on
   * the card goes through `restoreMany([id])`, because a click on one
   * card's own icon is not a statement about the selection.
   */
  async function restoreFromCard(assetId: string) {
    await restoreMany(selectionOrCard(assetId));
  }

  /**
   * The one irreversible action in the app. Confirmed through the
   * in-app modal rather than `window.confirm()`: on Tauri v2 macOS
   * WKWebView the native dialog is not reliably shown, and a guard
   * that silently answers "no" reads as a guard while being a wall.
   *
   * The question is asked once for the whole set, not once per row —
   * a per-row loop of confirms is how a user learns to click through
   * them.
   */
  async function purgeMany(ids: string[]) {
    if (ids.length === 0) return;
    const ok = await confirmCatalog.open({
      title: ids.length === 1 ? "Delete forever?" : `Delete ${ids.length} forever?`,
      body: "Ratings, comments and group filing go with it. This cannot be undone.",
      confirmLabel: "Delete Forever",
      danger: true,
    });
    if (!ok) return;
    let purged = 0;
    for (const id of ids) {
      if (await purgeOne(id)) purged += 1;
    }
    // Same one-refresh-per-flow rule as `restoreMany`.
    await loadSidebarCounts();
    if (ids.length > 1 || purged !== ids.length) {
      // "forever" survives the partial branch: a half-finished purge is
      // exactly where the user needs to know that what did go is gone.
      status = summariseBulk(purged, ids.length, {
        verb: "deleted",
        qualifier: "forever",
      });
    }
  }

  async function purgeFromCard(assetId: string) {
    await purgeMany(selectionOrCard(assetId));
  }

  /**
   * Empties the trash outright — the bulk sibling of `purgeMany`, and
   * the only destructive action in the app that is not addressed to a
   * selection.
   *
   * Ignores every active filter, because the trash is one place rather
   * than a view of the library: the confirmation says "everything in
   * the trash", and honouring a lit persona / tag chip here would
   * leave rows behind that the prompt just promised were going. This
   * is what Finder and Photos do with the same words, and the backend
   * command carries no filter to make the other reading possible.
   *
   * Unlike the per-id paths this repaints rather than dropping rows
   * locally: after the sweep the page is not "these rows minus a few",
   * it is empty, and the key-skip cache would otherwise keep showing
   * the old one.
   */
  async function emptyTrash() {
    const ok = await confirmCatalog.open({
      title: "Empty Trash?",
      body:
        "Every asset in the trash is deleted permanently, including any " +
        "the current filter is hiding. Ratings, comments and group filing " +
        "go with them. This cannot be undone.",
      confirmLabel: "Empty Trash",
      danger: true,
    });
    if (!ok) return;
    try {
      const result = await mutate<EmptyTrashResult>(
        "empty_trash",
        { command: {} },
        "empty the trash",
      );
      // Nothing that was on screen still exists, so nothing may stay
      // in the selection either.
      gridSelection.selectedIds.clear();
      assetPageCatalog.invalidateKey();
      await loadAssets();
      await loadSidebarCounts();
      status =
        result.skipped > 0
          ? `emptied trash (${result.purged}) — ${result.skipped} could not be deleted`
          : `emptied trash (${result.purged})`;
    } catch (err) {
      // No status line for the reason: `mutate` has it. The sweep may
      // have taken part of the trash before it stopped, so this says
      // what is certain — it did not finish — rather than a count it
      // cannot know until the reload below lands.
      console.warn("empty_trash failed", err);
      status = "the trash was not fully emptied";
      // The sweep may have taken some of them before it stopped; the
      // grid on screen no longer describes the trash either way.
      assetPageCatalog.invalidateKey();
      await loadAssets();
      await loadSidebarCounts();
    }
  }

  async function moveToModality(ids: string[], slug: string) {
    if (ids.length === 0 || bulkBusy) return;
    const items = ids.map((id) => ({
      asset_id: id,
      labels: null,
      register_note: null,
      cover: null,
      rating: null,
      modality: slug,
    }));
    bulkBusy = true;
    try {
      const res = await invoke<UpdateAssetMetaBatchResult>(
        "update_asset_meta_batch",
        { command: { items } },
      );
      for (const id of ids) assetPageCatalog.patchCard(id, { modality: slug });
      // Modality tallies shifted — refresh the sidebar counts.
      await loadSidebarCounts();
      status =
        res.failure_count > 0
          ? `moved: ${res.success_count} ok, ${res.failure_count} failed`
          : `moved ${res.success_count} to ${modalityLabelOf(slug)}`;
      // If a modality filter is active the moved-away cards must leave
      // the grid (same membership-change reasoning as Inbox above).
      if (activeFilter.activeModality !== null) {
        assetPageCatalog.invalidateKey();
        await loadAssets();
      }
      bulkModalityOpen = false;
      clearSelection();
    } catch (err) {
      console.warn("bulk move modality failed", err);
      status = `modality bulk error: ${JSON.stringify(err)}`;
    } finally {
      bulkBusy = false;
    }
  }

  // Attach one tag (by name) to every selected card. The backend
  // creates the tag row if missing and de-dupes existing links.
  async function bulkAttachTag() {
    const name = bulkTagInput.trim();
    const ids = Array.from(gridSelection.selectedIds);
    if (name.length === 0 || ids.length === 0 || bulkBusy) return;
    const items = ids.map((id) => ({ asset_id: id, name }));
    bulkBusy = true;
    try {
      const res = await invoke<AttachTagBatchResult>("attach_tag_batch", {
        command: { items },
      });
      await loadTagCounts();
      status =
        res.failure_count > 0
          ? `#${name}: ${res.success_count} tagged, ${res.failure_count} failed`
          : `tagged ${res.success_count} with #${name}`;
      bulkTagInput = "";
    } catch (err) {
      console.warn("bulk attach tag failed", err);
      status = `tag bulk error: ${JSON.stringify(err)}`;
    } finally {
      bulkBusy = false;
    }
  }

  // Detach one tag (by id) from every selected card. Idempotent — a
  // card without the link is a no-op — so the picker can offer the
  // whole persona tag list without first computing the intersection.
  async function bulkDetachTag(tagId: string, tagName: string) {
    const ids = Array.from(gridSelection.selectedIds);
    if (ids.length === 0 || bulkBusy) return;
    const items = ids.map((id) => ({ asset_id: id, tag_id: tagId }));
    bulkBusy = true;
    try {
      const res = await invoke<DetachTagBatchResult>("detach_tag_batch", {
        command: { items },
      });
      await loadTagCounts();
      status =
        res.failure_count > 0
          ? `#${tagName}: ${res.success_count} removed, ${res.failure_count} failed`
          : `removed #${tagName} from ${res.success_count}`;
    } catch (err) {
      console.warn("bulk detach tag failed", err);
      status = `tag bulk error: ${JSON.stringify(err)}`;
    } finally {
      bulkBusy = false;
    }
  }

  // ---- Bulk group membership (UI side) ----
  // The context menu's "Group ▸" fold-out: attach / detach the whole
  // selection to an existing manual group in one `batch_group_
  // membership` call (the W-batch primitive, commit 114e199).
  // Query groups are excluded — their membership is rule-driven.
  let bulkManualGroups = $derived(
    groupCatalog.counts.data.filter((g) => g.group.kind === "manual"),
  );
  // Union of the selected cards' group ids — feeds the fold-out's
  // "Remove from group" list.
  function selectionGroupIds(): Set<string> {
    const out = new Set<string>();
    for (const id of gridSelection.selectedIds) {
      for (const gid of cardById(id)?.group_ids ?? []) out.add(gid);
    }
    return out;
  }
  async function bulkGroupMembershipOp(
    groupId: string,
    groupName: string,
    mode: "attach" | "detach",
  ) {
    const ids = Array.from(gridSelection.selectedIds);
    if (ids.length === 0 || bulkBusy) return;
    bulkBusy = true;
    try {
      const pairs = ids.map((asset_id) => ({ asset_id, group_id: groupId }));
      const [attached, detached] = await invoke<[number, number]>(
        "batch_group_membership",
        {
          command: {
            attach: mode === "attach" ? pairs : [],
            detach: mode === "detach" ? pairs : [],
          },
        },
      );
      // Patch the touched cards locally (mirror of the Inbox bulk
      // path) so the Remove list + group-axis sort stay fresh
      // without a page reload.
      for (const id of ids) {
        const card = cardById(id);
        if (!card) continue;
        const gids = new Set(card.group_ids);
        if (mode === "attach") gids.add(groupId);
        else gids.delete(groupId);
        assetPageCatalog.patchCard(id, { group_ids: Array.from(gids) });
      }
      await loadSidebarCounts();
      status =
        mode === "attach"
          ? `added ${attached} to ▤ ${groupName}`
          : `removed ${detached} from ▤ ${groupName}`;
    } catch (err) {
      console.warn("bulk group membership failed", err);
      status = `group bulk error: ${JSON.stringify(err)}`;
    } finally {
      bulkBusy = false;
    }
  }

  // ---- App-level Threads drawer.
  // Replaces the legacy MEMO dialog: instead of writing a Markdown
  // file that gets ingested as an Asset, a click opens the Threads
  // drawer where UI notes and Claude-Code / agent messages interleave
  // on the same rows — one unified message model.
  let threadDrawerOpen = $state(false);

  function openThreadDrawer(): void {
    threadDrawerOpen = true;
    interaction.push("drawer");
    // W6 unread badge: opening the drawer marks everything seen.
    threadsSeenAt = Date.now();
    localStorage.setItem(THREADS_SEEN_KEY, String(threadsSeenAt));
  }

  function closeThreadDrawer(): void {
    threadDrawerOpen = false;
    interaction.remove("drawer");
    // Closing counts as "caught up" too — messages posted while the
    // drawer was open were visible.
    threadsSeenAt = Date.now();
    localStorage.setItem(THREADS_SEEN_KEY, String(threadsSeenAt));
  }

  // ---- Threads unread badge (W6) ----
  // "Unread" = non-archived threads whose last message postdates the
  // last drawer open/close (persisted locally — there is no server
  // read-state; a local watermark is enough for a solo-user vault).
  const THREADS_SEEN_KEY = "asterism.threadsSeenAt";
  // NaN guard: a corrupt stored value must degrade to "everything
  // unread" (0), not to a permanently-dark badge (x > NaN is always
  // false). First launch = 0 = every thread counts, by design — the
  // badge doubles as "there are threads you have never opened".
  let threadsSeenAt = $state(
    (() => {
      const raw = Number(localStorage.getItem(THREADS_SEEN_KEY) ?? "0");
      return Number.isFinite(raw) ? raw : 0;
    })(),
  );
  let threadsUnread = $derived.by(() => {
    let n = 0;
    for (const t of threadsCatalog.threads.values()) {
      if (!t.archived && (t.last_message_at_ms ?? 0) > threadsSeenAt) n += 1;
    }
    return n;
  });

  async function loadExporterSlugs() {
    try {
      exporterSlugs = await invoke<string[]>("list_exporters");
    } catch {
      exporterSlugs = [];
    }
  }

  /**
   * Persona / Modality sidebar count refresh. Persona counts are
   * total (persona-agnostic; the number never changes when the
   * active persona flips — same rule as Group / Tag "all-assets"
   * counting). Modality counts are scoped to `activeFilter.activePersona` so
   * switching persona narrows the modality tallies to that
   * persona's slice; the "all" mode returns cross-persona totals.
   */
  async function loadSidebarCounts() {
    // All four count fetches live on their respective catalog stores
    // and are independent server-side aggregations (each resolves its
    // own scope from the personaId / trash arguments — none reads
    // another catalog's fetched state), so they run concurrently. The
    // old sequential chain was a 4× cold-boot latency tax with no
    // ordering dependency behind it (07-27 P0 follow-up, closed
    // 2026-08-03).
    // Counts follow the grid's side of the trash — a live number beside
    // a trash grid describes the other half of the app.
    const side = activeFilter.trashView ? "trashed" : "live";
    await Promise.all([
      personaCatalog.loadCounts(side),
      modalityCatalog.loadCounts(activeFilter.activePersona, side),
      // FORMAT facet (asset-model v4) — same persona / trash scope.
      formatCatalog.loadCounts(activeFilter.activePersona, side),
      // COLOR facet — same persona / trash scope again. Counts come
      // from the extracted palettes, so an asset whose thumbnail has
      // not been generated yet is simply not in them.
      colorCatalog.loadCounts(activeFilter.activePersona, side),
    ]);
  }

  /**
   * Reactive lookup maps for the sidebar count spans. Built once
   * per fetch and consumed via `.get()` in the templates so a
   * persona / modality list with dozens of rows resolves each
   * count in O(1) — matches the `groupNameById` / `tagNameById`
   * pattern already used for name resolution.
   */
  // `personaCountById` moved to `personaCatalog.countById` (personas.svelte.ts).
  // `modalityCountBySlug` moved to `modalityCatalog.countBySlug` (modality.svelte.ts).

  // `totalAssetCount` moved to `personaCatalog.totalCount` (personas.svelte.ts).

  // `modalityTotalCount` moved to `modalityCatalog.totalCount` (modality.svelte.ts).

  // Reloads the "Recent Selections" sidebar section. Delegates
  // the persona-scoped invoke selection to `selectionCatalog.loadFor`
  // (null persona → cross-persona `list_recent_selections`,
  // scoped persona → `list_selections`).
  // "Save as Group" (W5): the
  // successor of the old `saveCurrentQuery`. The current filter chips
  // + Sorter state become a Query Group — a `kind='query'` Group whose
  // membership is the materialised result of the stored rule. The
  // sidebar renders it in the Groups section (no separate SavedQuery
  // catalog), and W3b's synchronous first evaluation guarantees the
  // group is never visible with empty members.
  async function saveAsQueryGroup() {
    if (activeFilter.activePersona === null) {
      dispatchCatalog.flash("Query Groups need a single active persona");
      return;
    }
    const name = await customPrompt(
      "Save as Group",
      "e.g. inbox this week",
    );
    if (name === null || name.trim() === "") return;
    const searchText = activeFilter.searchText.trim();
    // `query_json` v1:
    // (a) search_text is a first-class field, not piggybacked in the
    //     filter blob any more,
    // (b) `filter.group_ids` are stored RAW (un-expanded) — the
    //     evaluation job walks the `bucket_link` closure via its
    //     recursive CTE at freeze time, so the rule tracks
    //     later nesting edits instead of freezing a stale closure,
    // (c) UI-derived paginate fields (`viewer_subject` / `offset` /
    //     `limit`) are dropped — the eval Job evaluates without a
    //     `LIMIT` and burns positions from its own sort, so nothing
    //     downstream consumes them.
    const rule = {
      v: 1,
      filter: {
        persona_id: activeFilter.activePersona,
        modality: activeFilter.activeModality,
        // FORMAT / COLOR facets ride the same `ListAssetsQuery` fields
        // the grid already filters by, so the evaluation job applies
        // them without any schema change (v4 P3 carry, closed
        // 2026-08-03).
        format: activeFilter.activeFormat,
        color: activeFilter.activeColor,
        occurred_from_ms: null,
        occurred_until_ms: null,
        tag_ids: Array.from(activeFilter.activeTagIds),
        tag_match: activeFilter.tagMatchAll ? "all" : "any",
        group_ids: Array.from(activeFilter.activeGroupIds),
        session_id: activeFilter.activeSessionId,
        label: activeFilter.activeLabel,
        // The metric bands are deterministic predicates like every other
        // field here, so they belong in the rule. Leaving them out
        // would freeze a *wider* set than the one on screen while the
        // sidebar still showed the band — the group would hold material
        // the user had just excluded. Wire units; the store converts.
        ...activeFilter.metricBands(),
      },
      // `✦ Relevance` is dropped to the default axis on the way in
      // (the same reason a ✦ query's text is kept out): the
      // order comes from a retriever that does not promise the same
      // answer twice, so freezing it into `asset_bucket.position` would
      // record a sequence nobody can reproduce. The wire enum has no
      // token for it either, so a rule naming it would be refused —
      // this writes what the group can actually be evaluated with.
      sort: {
        ...activeFilter.persistableSort(),
        // Persisted so the group's frozen `position` records the
        // collation it was materialised under. Omitted when root, to
        // match the backend's `skip_serializing_if`.
        ...(activeFilter.sortCollation
          ? { collation: activeFilter.sortCollation }
          : {}),
      },
      // Only 🔍 exact text goes into a rule: a Query Group is a
      // persistent set definition, and Retrieval does not promise the
      // same answer twice. Saving a ✦ query would freeze a membership
      // nobody can reproduce, so the text is dropped instead — the chips
      // and sort still save.
      search_text:
        !activeFilter.searchFuzzy && searchText.length > 0 ? searchText : null,
    };
    const command: CreateQueryGroupCommand = {
      persona_id: activeFilter.activePersona,
      name: name.trim(),
      query_json: JSON.stringify(rule),
    };
    try {
      await invoke("create_query_group", { command });
      dispatchCatalog.flash(`Saved as Query Group · ${name.trim()}`);
      void loadGroupCounts();
    } catch (e) {
      dispatchCatalog.flash(`Save failed: ${e}`);
    }
  }

  /**
   * Resolves the persona bucket to write the current Selection into.
   * If the sidebar is showing a single-persona filter, use it.
   * Otherwise walk `filteredRows` and pick the persona shared by every
   * selected card — a mixed-persona selection returns `mixed: true`
   * so the caller can surface a clear error.
   */
  function personaIdOfSelection(): { personaId: string | null; mixed: boolean } {
    if (activeFilter.activePersona !== null) {
      return { personaId: activeFilter.activePersona, mixed: false };
    }
    let seen: string | null = null;
    let mixed = false;
    for (const row of filteredRows) {
      if (row.kind !== "cards") continue;
      for (const item of row.items) {
        if (item.kind !== "message") continue;
        const card = item.card;
        if (!gridSelection.selectedIds.has(card.id)) continue;
        if (seen === null) seen = card.persona_id;
        else if (seen !== card.persona_id) mixed = true;
      }
    }
    return { personaId: seen, mixed };
  }

  /**
   * Promise-shaped inline prompt. Replaces `window.prompt`, which is
   * silently no-op on Tauri v2 macOS WKWebView. `title` labels the
   * modal; `placeholder` is the ghost text; `defaultValue` prefills
   * the input. Resolves to the trimmed string on OK, to `null` on
   * Cancel / Escape / backdrop click.
   */
  // `customPrompt` / `promptOk` / `promptCancel` moved to
  // `promptCatalog.open` / `.commit` / `.cancel`. Callsites read
  // through the store; App keeps a thin adapter so the four
  // `await customPrompt(...)` invocations don't have to rename.
  const customPrompt = (title: string, placeholder = "", defaultValue = "") =>
    promptCatalog.open(title, placeholder, defaultValue);

  /**
   * Dispatches the current volatile selection through the `file`
   * exporter in copy mode (W5: `dispatch_run` replaces the old
   * Selection → create_dispatch two-step). The user is prompted for
   * `output_dir`; the backend freezes the picked ids into a Snapshot
   * (content-hash deduped) at dispatch time.
   */
  async function copySelectionTo() {
    if (gridSelection.selectedIds.size === 0) return;
    const resolved = personaIdOfSelection();
    if (resolved.mixed) {
      dispatchCatalog.flash("Selection spans multiple personas — pick one axis first");
      return;
    }
    if (resolved.personaId === null) {
      dispatchCatalog.flash("Could not resolve a persona for the dispatch");
      return;
    }
    const outputDir = await customPrompt(
      "Copy selection to which directory?",
      "absolute path, e.g. /Users/you/desktop/alice",
      "",
    );
    if (!outputDir || !outputDir.trim()) return;
    // Reject relative paths and `~`-prefixed shell shorthand up front —
    // the backend enforces absolute-only anyway (2026-07-20 fallout: a
    // `~/selection1` input created a literal `~` directory).
    const trimmed = outputDir.trim();
    if (!trimmed.startsWith("/")) {
      dispatchCatalog.flash(
        `Dispatch aborted: output_dir must be an absolute path (starts with "/"). Got: ${trimmed}`,
        6000,
      );
      return;
    }
    try {
      const params = {
        output_dir: trimmed,
        mode: "copy",
        filename_template: "{{dispatch_id}}__{{index}}__{{basename}}",
      };
      const dto = await invoke<{ id: string; state: string }>("dispatch_run", {
        command: {
          persona_id: resolved.personaId,
          group_id: null,
          asset_ids: Array.from(gridSelection.selectedIds),
          exporter_slug: "file",
          action: "write",
          params_json: JSON.stringify(params),
        },
      });
      dispatchCatalog.beginDispatch(dto.id, `Dispatching · ${dto.id.slice(0, 8)}`);
      void dispatchCatalog.pollDispatch(dto.id);
    } catch (err) {
      dispatchCatalog.flash(`Dispatch failed: ${String(err)}`, 6000);
    }
  }

  // `promoteSelectionToGroup` was removed in W5a. The action-bar
  // "Promote to Group…" button is retired (the fixed Select
  // mode is gone). `promote_snapshot_to_group` is the snapshot →
  // group operation behind the Snapshot view's promote-to-Group
  // action (W6); the direct "volatile pick → Group" path lives on
  // the card context menu as `promote_volatile_selection` (W5-d),
  // which fuses the freeze + promote in one command.

  // `pollDispatch` moved to `dispatchCatalog.pollDispatch`.

  // --- Session Reader ---------------------------------------------
  // Chronological (oldest-first) ordering for the transcript; the
  // grid itself lists freshest-first.
  // Two Reader sources coexist:
  //   * `sessionReaderItems` — Session-scoped assets fetched via a
  //     Session tile click. Opens the Reader independently with zero
  //     activeSessionId side effects (since the Show-messages toggle
  //     landed, the filter-chip drill-in narrow is no longer needed)
  //   * `assetPageCatalog.page.items` — the legacy drill-in path
  //     (via the 📖 read button, opened with activeSessionId set)
  let sessionReaderItems = $state<AssetCardDto[] | null>(null);
  let readerItems = $derived(
    sessionReaderItems !== null
      ? [...sessionReaderItems].sort((a, b) => a.occurred_at_ms - b.occurred_at_ms)
      : assetPageCatalog.page === null
      ? []
      : [...assetPageCatalog.page.items].sort(
          (a, b) => a.occurred_at_ms - b.occurred_at_ms,
        ),
  );

  // Well-known chat role carried in the labels array (parsers store
  // the message role there); fall back to the modality slug, "" when
  // unclassified (asset-model v4).
  function cardRole(card: AssetCardDto): string {
    const known = ["user", "assistant", "system", "tool"];
    return card.labels.find((l) => known.includes(l)) ?? card.modality ?? "";
  }

  async function loadReaderTexts(ids: string[]) {
    if (ids.length === 0) {
      readerTexts = new Map();
      return;
    }
    readerLoading = true;
    try {
      const texts = await invoke<AssetTextDto[]>("asset_texts", {
        assetIds: ids,
      });
      readerTexts = new Map(texts.map((t) => [t.asset_id, t.text]));
    } catch (error) {
      console.warn("asset_texts failed", error);
      readerTexts = new Map();
    } finally {
      readerLoading = false;
    }
  }

  async function openReader() {
    const p = assetPageCatalog.page;
    if (p === null || p.items.length === 0) return;
    readerOpen = true;
    interaction.push("reader");
    // Texts are usually already cached by the session-open effect; the
    // conditional refetch covers the case where the reader is opened
    // outside a session drill-in (all-assets view).
    if (readerTexts.size === 0) {
      await loadReaderTexts(p.items.map((c) => c.id));
    }
  }

  /**
   * Reader path for Session tile clicks — fetches the Session's
   * assets directly and opens the Reader without touching
   * activeSessionId / filter chips. Since the Show-messages toggle
   * landed, a Session tile is a plain "open the Reader on its own"
   * entry rather than a filter-chip drill-in.
   */
  async function openReaderForSession(sessionId: string) {
    // Opening a container shows *its members* — not "its members that
    // also match whatever the grid happens to be filtered by".
    //
    // This used to spread `currentFilter()`, which made the reader
    // disagree with the count the sidebar prints next to each Grouping
    // row (that count is unfiltered). With MODALITY lit the reader was
    // not merely narrowed but always empty: members carry no modality
    // since v4 P2 moved conversation rows to `modality = NULL`, so
    // `modality = 'tape' AND container_id = …` matches nothing by
    // construction. Only the persona scope and the trash side survive
    // — both are "which album am I in", not "what am I looking for".
    const query = {
      viewer_subject: null,
      persona_id: activeFilter.activePersona,
      modality: null,
      occurred_from_ms: null,
      occurred_until_ms: null,
      tag_ids: [] as string[],
      group_ids: [] as string[],
      session_id: sessionId,
      label: null,
      format: null,
      color: null,
      trash: activeFilter.trashView ? "trashed" : "live",
      offset: 0,
      limit: 5000,
    };
    try {
      const page = await invoke<AssetPageDto>("list_assets", { query });
      sessionReaderItems = page.items;
      readerOpen = true;
      interaction.push("reader");
      await loadReaderTexts(page.items.map((c) => c.id));
    } catch (err) {
      console.warn("openReaderForSession failed", err);
    }
  }

  function closeReader() {
    readerOpen = false;
    interaction.remove("reader");
    // Clear the Session-scoped Reader source on close so the next
    // open falls back to the legacy grid-page path (📖 read button).
    sessionReaderItems = null;
    // Keep readerTexts populated — the grid badges depend on it, and
    // dropping it here would blank the badges the moment the reader
    // closes.
  }

  // Auto-load message bodies whenever a session becomes the current
  // drill-in. This powers content-type badges on the grid cards (📊
  // / ⌨ / 🎨 / 🔗) and also warms up the Reader so opening it feels
  // instant. Skipped for non-session views (all-assets browse) —
  // batch-loading every asset would be wasteful and the badges add
  // less value across an unrelated corpus.
  $effect(() => {
    const sid = activeFilter.activeSessionId;
    // Track page identity so drilling into a different session (or
    // paging inside one) triggers a refetch.
    const items = assetPageCatalog.page?.items ?? [];
    if (sid !== null && items.length > 0) {
      const ids = items.map((c) => c.id);
      void loadReaderTexts(ids);
    } else if (sid === null && sessionReaderItems === null) {
      // Clear stale badges when leaving the session view — but not
      // while the container reader owns the texts. Opening a container
      // deliberately leaves `activeSessionId` untouched (no filter
      // chip), so this branch would fire right after
      // `openReaderForSession` loaded the bodies and wipe every one of
      // them, leaving the reader with headers and no messages.
      readerTexts = new Map();
      activeContentFlags.clear();
    }
  });

  // asset id → detected content flags, recomputed whenever the text
  // cache turns over. A message with no detectable structure gets an
  // empty set (still a Map entry) so the card renders no badges.
  let flagsByCard = $derived.by(() => {
    const out = new Map<string, Set<ContentFlag>>();
    for (const [id, text] of readerTexts) {
      out.set(id, detectFlags(text));
    }
    return out;
  });

  // How many cards in the current page carry each flag — powers the
  // filter chip row counts.
  let flagCounts = $derived.by(() => {
    const counts: Record<ContentFlag, number> = { code: 0, table: 0, mermaid: 0, link: 0 };
    if (!assetPageCatalog.page) return counts;
    for (const card of assetPageCatalog.page.items) {
      const fs = flagsByCard.get(card.id);
      if (!fs) continue;
      for (const f of fs) counts[f] += 1;
    }
    return counts;
  });

  // Cards after the active-content-flag filter. Empty active set =
  // pass-through; otherwise a card must carry every active flag
  // (all-of semantics — matches how the existing tag filter narrows).
  // Base cards after active-content-flag / recent-drop narrows,
  // pre-sort. Exposed so downstream derivations (bucket recency
  // maps for `updated` order) do not need to re-run the filter.
  let filteredBase = $derived.by(() => {
    if (!assetPageCatalog.page) return [] as AssetCardDto[];
    let source = assetPageCatalog.page.items as AssetCardDto[];
    if (filterRecentDrops && recentDropIds.size > 0) {
      source = source.filter((c) => recentDropIds.has(c.id));
    }
    if (activeContentFlags.size === 0) return source;
    return source.filter((card) => {
      const fs = flagsByCard.get(card.id);
      if (!fs) return false;
      for (const need of activeContentFlags) {
        if (!fs.has(need)) return false;
      }
      return true;
    });
  });

  // Catalog-backed lookups the extracted comparator reads. Bundled
  // here (rather than imported by `card-cmp.ts`) so that module stays
  // pure and testable; this object is the App-side binding of its
  // `CardSortLookups` contract.
  // Derived rather than a plain const because `compareText` depends on
  // `activeFilter.sortCollation`; the four catalog lookups are stable
  // function references, so the object only really rebuilds when the
  // collation knob moves.
  let cardSortLookups: CardSortLookups = $derived.by(() => ({
    personaName,
    personaDisplayOrder,
    modalityRank,
    primaryGroupName,
    compareText: textComparator(activeFilter.sortCollation),
  }));

  // Per-bucket max(occurred_at_ms) for enum sort targets. Feeds the
  // `updated` order so persona / modality / group buckets rank by
  // their most-recent asset within the currently-filtered slice.
  // Derived over `filteredBase` (the pre-sort input) so `updated`
  // reflects what the user actually sees.
  let bucketRecency = $derived.by(() =>
    computeBucketRecency(filteredBase, cardSortLookups),
  );

  // Cards after the active-content-flag filter. Empty active set =
  // pass-through; otherwise a card must carry every active flag
  // (all-of semantics — matches how the existing tag filter narrows).
  let filteredItems = $derived.by(() => {
    const base = filteredBase;
    // Relevance ranking is the reason to search — re-sorting it would
    // discard the only ordering the query produced. See
    // `searchOrderActive`. The 🎲 draw skips for the mirror reason —
    // its sequence is the shuffle, and sorting it would answer "show me
    // something" with the same first screen every time.
    if (searchOrderActive || randomOrderActive) return base;
    // 2-axis sort: pick a compare from `activeFilter.sortTarget` × `activeFilter.sortOrder`,
    // then multiply the delta by ±1 depending on `activeFilter.sortReverse`.
    // The tiebreak stays `occurred_at DESC` throughout so
    // clustering by axis reads newest-first inside each bucket.
    const dir = activeFilter.sortReverse ? -1 : 1;
    const cmp = buildCardCmp<AssetCardDto>(
      activeFilter.sortTarget,
      activeFilter.sortOrder,
      dir,
      bucketRecency,
      cardSortLookups,
      // Only the `relevance` branch reads it. `null` while the axis is
      // anything else, or while the rank fetch has not landed — the
      // comparator then answers in the default order rather than
      // freezing the page in whatever sequence it arrived in.
      assetPageCatalog.rankOrder,
    );
    if (!cmp) return base;
    const sorted = base.slice();
    sorted.sort(cmp);
    return sorted;
  });

  // How much of what is on screen the rank actually reached. Says the
  // quiet part of the `✦ Relevance` axis out loud: the shortlist has a
  // ceiling, so the rows past it are ordered by the default axis, and a
  // grid that silently mixed the two would read as one ordering.
  let rankedOnPage = $derived.by(() => {
    const rank = assetPageCatalog.rankOrder;
    if (!relevanceOrderActive || rank === null) return 0;
    let n = 0;
    for (const c of filteredItems) if (rank.has(c.id)) n += 1;
    return n;
  });

  // `buildCardCmp` moved to `./lib/sort/card-cmp` (imported above).
  // It is the UI half of a two-sided contract — the backend mirrors it
  // in `asterism-core::domain::sort_eval` so Query Groups can freeze
  // the same order into `asset_bucket.position` — and a comparator that
  // lives inside this component script cannot be diffed against its
  // backend twin by any test. See `card-cmp.test.ts` for the parity
  // fixture, including the collation cases where the two sides
  // deliberately disagree.

  function personaDisplayOrder(id: string): number {
    const idx = personaCatalog.list.data.findIndex((p) => p.id === id);
    return idx < 0 ? personaCatalog.list.data.length : idx;
  }

  // Sidebar drag reorder — DEFERRED. Tauri v2 WKWebView with
  // `dragDropEnabled: true` intercepts the HTML5 `drop` event so
  // the JS side never sees it; the `dragend`-based settle also
  // fires without a reliable drop index. Backend command
  // (`reorder_personas`), `MODALITIES` reactive state, and the
  // localStorage persistence hook (`persistModalityOrder`) stay
  // wired so a follow-up (svelte-dnd-action or manual
  // pointerdown+pointerup tracking) can drop into place without a
  // schema churn. See the journal carry note for context.

  // Grid virtualisation state. The grid card size (`.grid` uses
  // `minmax(180px, 1fr)`) drives how many columns fit into the
  // available viewport width; a ResizeObserver on the wrapper keeps
  // `gridCols` in sync. `filteredItems` is then reshaped into rows
  // of `gridCols` cards so `virtua`'s `VList` can virtualise the
  // vertical scroll without knowing about individual cards.
  const CARD_MIN_PX = 180;
  const GRID_GAP_PX = 10; // 0.6rem @ 16px root, close enough for width math.
  let gridWrapperEl = $state<HTMLDivElement | null>(null);
  let gridWrapperWidth = $state(0);
  let gridCols = $derived(
    Math.max(
      1,
      Math.floor(
        (gridWrapperWidth + GRID_GAP_PX) / (CARD_MIN_PX + GRID_GAP_PX),
      ),
    ),
  );
  // Grouping key derivation. When `sortMode` isn't Recency, the
  // grid inserts a lightweight header row every time the bucket
  // key changes so the user can read the axis boundaries at a
  // glance (Lightroom's "grid segmentation" pattern). Recency
  // still renders flat because a per-day header would fire on
  // every timestamp.
  // Time-axis header bucketing. Same divider mechanism as
  // persona / modality / tag / group, but the raw timestamp is rounded
  // into a Finder/Photos-style hybrid: today, yesterday, weekday for
  // the last 7 days, then YYYY-MM further back. Rounding is what makes
  // per-row header explosion (the reason time axes were originally
  // skipped) a non-issue — clustered assets in the same day / month
  // collapse under one header.
  function cardTimeMs(card: AssetCardDto): number {
    return activeFilter.sortTarget === "created_at"
      ? card.created_at_ms
      : card.occurred_at_ms;
  }
  function startOfLocalDayMs(ts: number): number {
    const d = new Date(ts);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }
  const WEEKDAY_NAMES = [
    "Sunday", "Monday", "Tuesday", "Wednesday",
    "Thursday", "Friday", "Saturday",
  ];
  function timeBucketKey(ts: number, nowMs: number): string {
    const cardDay = startOfLocalDayMs(ts);
    const todayDay = startOfLocalDayMs(nowMs);
    const daysAgo = Math.round((todayDay - cardDay) / 86_400_000);
    if (daysAgo <= 0) return "d:0";
    if (daysAgo === 1) return "d:1";
    if (daysAgo < 7) return `d:${daysAgo}`;
    const d = new Date(cardDay);
    return `m:${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
  }
  function timeBucketLabel(ts: number, nowMs: number): string {
    const cardDay = startOfLocalDayMs(ts);
    const todayDay = startOfLocalDayMs(nowMs);
    const daysAgo = Math.round((todayDay - cardDay) / 86_400_000);
    if (daysAgo <= 0) return "🕒 Today";
    if (daysAgo === 1) return "🕒 Yesterday";
    if (daysAgo < 7) return `🕒 ${WEEKDAY_NAMES[new Date(cardDay).getDay()]}`;
    const d = new Date(cardDay);
    return `🕒 ${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
  }

  function bucketKey(card: AssetCardDto, nowMs: number): string {
    if (activeFilter.sortTarget === "persona") return `persona:${personaName(card.persona_id)}`;
    if (activeFilter.sortTarget === "modality") return `modality:${card.modality}`;
    if (activeFilter.sortTarget === "tag") return `tag:${firstUserLabel(card.labels)}`;
    if (activeFilter.sortTarget === "group") return `group:${primaryGroupName(card.group_ids)}`;
    if (activeFilter.sortTarget === "occurred_at" ||
        activeFilter.sortTarget === "created_at") {
      return `time:${timeBucketKey(cardTimeMs(card), nowMs)}`;
    }
    return "";
  }
  // When the card's bucket was last touched, on whichever axis is
  // active — the same `max(occurred_at_ms)` the `Recent` comparator
  // ranks buckets by, so the major header boundaries land exactly where
  // the ordering changes time band. `null` on axes that do not bucket.
  function bucketRecencyMs(card: AssetCardDto): number | null {
    const t = activeFilter.sortTarget;
    if (t === "persona") return bucketRecency.persona.get(card.persona_id) ?? null;
    if (t === "modality") return bucketRecency.modality.get(card.modality ?? "") ?? null;
    if (t === "tag") return bucketRecency.tag.get(firstUserLabel(card.labels)) ?? null;
    if (t === "group") {
      return bucketRecency.group.get(primaryGroupName(card.group_ids)) ?? null;
    }
    return null;
  }
  function bucketLabel(card: AssetCardDto, nowMs: number): string {
    if (activeFilter.sortTarget === "persona") return `● ${personaName(card.persona_id)}`;
    if (activeFilter.sortTarget === "modality") return `◆ ${card.modality}`;
    if (activeFilter.sortTarget === "tag") {
      const label = firstUserLabel(card.labels);
      // No space after the `#`: the tag axis buckets on whatever the
      // card's first user label happens to be, and some of those are
      // date-shaped (`2026-06-29`). Detached, `# 2026-06-29` reads as a
      // time bucket next to `🕒 Yesterday`; attached, the sigil belongs
      // to the value and the row reads as a tag.
      return `#${label === TAIL_SENTINEL ? "(no tag)" : label}`;
    }
    if (activeFilter.sortTarget === "group") {
      const name = primaryGroupName(card.group_ids);
      return `▤ ${name === TAIL_SENTINEL ? "(no group)" : name}`;
    }
    if (activeFilter.sortTarget === "occurred_at" ||
        activeFilter.sortTarget === "created_at") {
      return timeBucketLabel(cardTimeMs(card), nowMs);
    }
    return "";
  }

  // Grid item union — a card cell renders either a Message tile
  // (existing per-asset DOM) or a Session tile (SessionTile.svelte).
  // Sessions are 1st-class entries in the dialogue grid now, and
  // the mixed toggle interleaves the two kinds by
  // timestamp inside a single VList pass.
  type CardItem =
    | { kind: "message"; card: AssetCardDto }
    | { kind: "session"; session: SessionDto };
  // Headers come in two weights. `minor` is the bucket the sort target
  // clusters into (a tag, a persona, a time band). `major` appears only
  // under the `Recent` order, where the bucket sequence *is* a time
  // sequence — the tags are ordered by when each was last touched, so
  // the run of tags between two time boundaries is a real grouping and
  // the grid says so instead of leaving the reader to infer it from
  // card dates. Not sticky: these are ordinary VList rows, which is
  // what keeps a second level cheap (two-level *sticky* headers need
  // per-level pin offsets and are a known source of layout bugs).
  type GridRow =
    | { kind: "header"; level: "major" | "minor"; label: string; key: string }
    | { kind: "cards"; items: CardItem[]; key: string };

  // Time key used for interleaved-row ordering + time buckets in
  // mixed dialogue mode. Session uses `started_at_ms` so a fresh
  // Session lands next to messages authored around its start; the
  // per-message `occurred_at_ms` keeps the existing bucket cadence.
  function itemTimeMs(it: CardItem): number {
    return it.kind === "session"
      ? it.session.started_at_ms
      : it.card.occurred_at_ms;
  }
  function itemPersonaId(it: CardItem): string {
    return it.kind === "session" ? it.session.persona_id : it.card.persona_id;
  }
  function itemBucketKey(it: CardItem, nowMs: number): string {
    if (activeFilter.sortTarget === "persona") {
      return `persona:${personaName(itemPersonaId(it))}`;
    }
    if (
      activeFilter.sortTarget === "occurred_at" ||
      activeFilter.sortTarget === "created_at"
    ) {
      return `time:${timeBucketKey(itemTimeMs(it), nowMs)}`;
    }
    // modality / tag / group / cover do not apply uniformly across
    // Session + Message — collapse the bucket so mixed rows stream
    // headerless when those axes are picked.
    return "";
  }
  function itemBucketLabel(it: CardItem, nowMs: number): string {
    if (activeFilter.sortTarget === "persona") {
      return `● ${personaName(itemPersonaId(it))}`;
    }
    if (
      activeFilter.sortTarget === "occurred_at" ||
      activeFilter.sortTarget === "created_at"
    ) {
      return timeBucketLabel(itemTimeMs(it), nowMs);
    }
    return "";
  }

  let filteredRows = $derived.by(() => {
    const cols = gridCols;
    // Pin "now" once per rebuild so a mid-scroll day rollover cannot
    // shift the Today boundary while VList is iterating this pass.
    const nowMs = Date.now();

    // asset-model v4 P3: the grid renders top-level cards only —
    // Session tiles moved to the sidebar Grouping section, so the
    // per-(mode × toggle) interleave is gone and the item source is a
    // single homogeneous path. The `CardItem` union survives for the
    // row/render plumbing.
    const items: CardItem[] = (filteredItems as AssetCardDto[]).map(
      (c) => ({ kind: "message", card: c }),
    );

    if (items.length === 0) return [] as GridRow[];

    // Header rows fire whenever the sort target clusters into named
    // buckets. Cover still slides through headerless — no natural
    // bucket — and so do the two metric axes, which reach the same
    // outcome through the empty-key path below (`bucketKey` returns ""
    // for them: a length or a byte count is a continuum, and bucketing
    // it would mean inventing bands nobody picked). In mixed dialogue
    // mode, non-shared axes
    // (modality / tag / group) also skip because Session tiles
    // cannot supply the key (`itemBucketKey` returns "").
    // Relevance order skips too: the rows are ranked by match quality,
    // so a caption drawn from any card field would describe a clustering
    // that is not there (a "Yesterday" header opening a run that
    // continues into last month).
    const showHeaders =
      !searchOrderActive && !randomOrderActive && activeFilter.sortTarget !== "cover";

    // Under `Recent` the buckets are ranked by when each was last
    // touched, so consecutive buckets fall into time bands and the band
    // is a grouping the grid can state outright. Reuses the time-axis
    // buckets (`🕒 Yesterday` / `🕒 2026-07`) rather than inventing a
    // second date vocabulary. Off for every other order: under `A→Z`
    // the sequence has nothing to do with time, and a time caption over
    // it would be the same false clustering the search path avoids.
    const showMajor =
      showHeaders &&
      activeFilter.sortOrder === "updated" &&
      (activeFilter.sortTarget === "persona" ||
        activeFilter.sortTarget === "modality" ||
        activeFilter.sortTarget === "tag" ||
        activeFilter.sortTarget === "group");

    const out: GridRow[] = [];
    let currentMajor: string | null = null;
    let currentBucket: string | null = null;
    let buffer: CardItem[] = [];
    let rowCounter = 0;
    const flush = () => {
      if (buffer.length === 0) return;
      out.push({ kind: "cards", items: buffer, key: `r${rowCounter++}` });
      buffer = [];
    };
    for (const it of items) {
      if (showHeaders) {
        const bucket = it.kind === "message"
          ? bucketKey(it.card, nowMs)
          : itemBucketKey(it, nowMs);
        // Only emit a header when the bucket key is non-empty and
        // differs from the current one — empty keys (mixed mode
        // with a Session-incompatible axis) roll into whatever
        // bucket the last message opened, keeping the stream flat.
        if (bucket !== "" && bucket !== currentBucket) {
          if (showMajor && it.kind === "message") {
            const rec = bucketRecencyMs(it.card);
            // A bucket with no recency (nothing to take a max over)
            // cannot be placed on the timeline; it stays under whatever
            // band is open rather than opening a bogus one.
            if (rec !== null) {
              const major = timeBucketKey(rec, nowMs);
              if (major !== currentMajor) {
                flush();
                out.push({
                  kind: "header",
                  level: "major",
                  label: timeBucketLabel(rec, nowMs),
                  key: `H${out.length}`,
                });
                currentMajor = major;
              }
            }
          }
          flush();
          const label = it.kind === "message"
            ? bucketLabel(it.card, nowMs)
            : itemBucketLabel(it, nowMs);
          out.push({
            kind: "header",
            level: "minor",
            label,
            key: `h${out.length}`,
          });
          currentBucket = bucket;
        }
      }
      buffer.push(it);
      if (buffer.length === cols) flush();
    }
    flush();
    return out;
  });
  $effect(() => {
    if (!gridWrapperEl) return;
    const el = gridWrapperEl;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        // `contentRect.width` excludes padding, which is what we
        // actually want for column math (the CSS grid's own padding
        // has already been applied by that point).
        gridWrapperWidth = entry.contentRect.width;
      }
    });
    ro.observe(el);
    gridWrapperWidth = el.clientWidth;
    return () => ro.disconnect();
  });

  // Escape unwinds one zoom stage at a time: full-window image →
  // detail overlay → grid; ←/→ page through the grid while either
  // stage is up. The search input already handles Escape locally
  // (see onSearchKeydown).
  // Global shortcut registry — the source of truth for the
  // Settings modal (which just renders this list) and the keydown
  // handler (which matches keys against it). Keeping the two in
  // sync manually would drift; the list drives both.
  type Shortcut = {
    keys: string;
    label: string;
    scope: "global" | "grid" | "detail" | "fullscreen";
  };
  const SHORTCUTS: Shortcut[] = [
    { keys: "Esc", label: "Close overlay / step out", scope: "global" },
    { keys: "/", label: "Focus search", scope: "global" },
    { keys: "⌘,", label: "Open settings", scope: "global" },
    { keys: "⌘V", label: "Paste clipboard image", scope: "global" },
    { keys: "⌘⌫", label: "Move selection to Trash (live view only)", scope: "grid" },
    { keys: "1 – 7", label: "Sort target: Occurred / Added / Persona / Modality / Tag / Group / Cover", scope: "grid" },
    { keys: "\\", label: "Reverse sort direction", scope: "grid" },
    { keys: "← →", label: "Prev / next in detail overlay", scope: "detail" },
    { keys: "F", label: "Toggle fullscreen (image detail)", scope: "detail" },
    { keys: "Wheel", label: "Zoom at cursor (fullscreen)", scope: "fullscreen" },
    { keys: "+ / − / = / _", label: "Zoom in / out step", scope: "fullscreen" },
    { keys: "0 / R", label: "Reset zoom", scope: "fullscreen" },
    { keys: "1", label: "Snap to 200% (fullscreen)", scope: "fullscreen" },
    { keys: "Dbl-click", label: "Reset zoom", scope: "fullscreen" },
  ];
  let settingsOpen = $state(false);
  // Single toggle point so the "settings" mode-stack entry (W5, Esc
  // SoT) tracks every open/close path.
  function setSettingsOpen(open: boolean) {
    settingsOpen = open;
    if (open) interaction.push("settings");
    else interaction.remove("settings");
  }
  // Grid card presentation toggle — a Theme-style switch in the
  // sidebar header. Clean mode hides Rating stars / date / palette /
  // labels / hover-icon strip / flag & score badges so the grid
  // reduces to Modality + thumb + Persona + basename. Persisted in the
  // backend settings store (`ui.clean_mode`).
  let cleanMode = $derived(settingsCatalog.bool(SETTING_KEYS.cleanMode, false));

  // Import > Auto-organize on drop.
  // When ON, a folder (or set of files sharing a parent) dropped into
  // the grid is passed to the backend's existing auto-organize path
  // (AddAssetCommand.auto_organize_base_dir → asset_service builds a
  // Dir tree from the parent hierarchy and creates a Group at each
  // leaf). When OFF, files land flat under the persona with just the
  // "dropped" label (previous behaviour). Default ON — the feature
  // would otherwise sleep despite the wire being complete on the
  // backend.
  let autoOrganizeDrop = $derived(
    settingsCatalog.bool(SETTING_KEYS.importAutoOrganize, true),
  );

  // Single write path for every preference control. The catalog
  // re-reads after the write, so the deriveds settle on the backend's
  // resolved value rather than on optimistic local state.
  //
  // `savingPreference` guards the controls while a write is in flight:
  // two concurrent `set_setting` calls have no defined ordering at the
  // backend, and `Resource`'s generation counter only orders the list
  // *reads*, not the writes.
  //
  // Callers must re-assert the DOM afterwards (see the checkbox
  // handlers). A one-way `checked={derived}` binding only repaints when
  // the derived value *changes*, and a write can legitimately resolve
  // to the value that was already showing — a rejected write, or a key
  // an environment variable pins — which would otherwise strand the
  // checkbox in the position the click put it in.
  let savingPreference = $state(false);
  async function setPreference(key: string, value: boolean): Promise<void> {
    if (savingPreference) return;
    savingPreference = true;
    try {
      await settingsCatalog.set(key, value);
    } catch (e) {
      console.warn(`[App] setting ${key} failed:`, e);
    } finally {
      savingPreference = false;
    }
  }

  // Grid card context menu: position + target card. Opened by
  // right-click; closed by Esc / outside click / action pick.
  // W2 regrammar (Finder standard): right-clicking a card that is
  // NOT part of the current selection first retargets the selection
  // to that card (exclusive select) — the menu then always operates
  // on the selection, never on a "card under cursor vs highlighted
  // elsewhere" split. Mode by resulting size: >1 = bulk actions with
  // the count in the header, 1 = per-card reflex actions.
  let cardMenu = $state<{ x: number; y: number; card: AssetCardDto } | null>(null);
  function openCardMenu(e: MouseEvent, card: AssetCardDto) {
    e.preventDefault();
    if (!gridSelection.selectedIds.has(card.id)) {
      gridSelection.selectedIds.clear();
      gridSelection.selectedIds.add(card.id);
      gridSelection.lastAnchorId = card.id;
    }
    // Clamp to viewport so the menu never spills off the right /
    // bottom edge. Three shapes to cover: the trash-side menu is two
    // entries and a rule; the live menus carry the selection-action
    // block (tag input included) plus the removal tier, and the
    // single-card one appends the reflex actions on top of that.
    //
    // Getting this wrong is not cosmetic here. The destructive entry
    // is the last one, so a menu that overflows the bottom edge puts
    // it off screen — the clamp is what keeps "last" from meaning
    // "unreachable".
    const menuW = 240;
    const menuH = assetPageCatalog.pageIsTrash
      ? 120
      : gridSelection.selectedIds.size > 1
        ? 470
        : 530;
    const x = Math.min(e.clientX, window.innerWidth - menuW - 4);
    const y = Math.min(e.clientY, window.innerHeight - menuH - 4);
    cardMenu = { x, y, card };
    interaction.push("cardMenu");
  }
  function closeCardMenu() {
    cardMenu = null;
    interaction.remove("cardMenu");
    // Collapse the bulk submenus so a re-open starts folded.
    bulkModalityOpen = false;
    bulkTagOpen = false;
    bulkGroupOpen = false;
  }
  async function contextSetWallpaper(card: AssetCardDto) {
    if (card.persona_id === "") return;
    try {
      await invoke<PersonaThemeDto>("set_persona_theme", {
        command: {
          persona_id: card.persona_id,
          wallpaper_asset_id: card.id,
        },
      });
      status = `wallpaper set for ${personaName(card.persona_id)}`;
      if (activeFilter.activePersona === card.persona_id) {
        await themeCatalog.loadFor(card.persona_id);
      }
    } catch (error) {
      status = `wallpaper error: ${JSON.stringify(error)}`;
    } finally {
      closeCardMenu();
    }
  }
  async function contextCopyLocator(card: AssetCardDto) {
    try {
      await navigator.clipboard.writeText(card.source_locator);
      status = `copied path: ${card.source_locator.split("/").pop() ?? ""}`;
    } catch (error) {
      status = `copy error: ${JSON.stringify(error)}`;
    } finally {
      closeCardMenu();
    }
  }
  /**
   * Right-click "Group-ify selection" (W5-d): freezes the whole
   * volatile pick into a Snapshot and promotes it into a new manual
   * Group in one backend call (`promote_volatile_selection`). The
   * menu entry only renders while a selection exists; it always acts
   * on the full selection (the count in the label says so), not the
   * card under the cursor.
   */
  async function contextPromoteSelection() {
    closeCardMenu();
    if (gridSelection.selectedIds.size === 0) return;
    const resolved = personaIdOfSelection();
    if (resolved.mixed) {
      dispatchCatalog.flash(
        "Selection spans multiple personas — pick one axis first",
      );
      return;
    }
    if (resolved.personaId === null) {
      dispatchCatalog.flash("Could not resolve a persona for the selection");
      return;
    }
    const name = await customPrompt(
      "Group-ify selection",
      "unique per persona",
      "",
    );
    if (!name || !name.trim()) return;
    try {
      const result = await invoke<PromoteSnapshotToGroupResult>(
        "promote_volatile_selection",
        {
          command: {
            persona_id: resolved.personaId,
            asset_ids: Array.from(gridSelection.selectedIds),
            name: name.trim(),
            description: null,
            dir_id: null,
          },
        },
      );
      dispatchCatalog.flash(
        `Promoted · Group “${result.name}” · ${result.asset_count} asset(s)`,
      );
      // The pick is consumed by the promote (an operation ends
      // the volatile selection).
      clearSelection();
      void loadGroupCounts();
    } catch (e) {
      dispatchCatalog.flash(`Group-ify failed: ${String(e)}`, 6000);
    }
  }
  // Removal now lives at the bottom of this menu, in two tiers that
  // match what each side of the trash can actually do: the live menu
  // ends in "Move to Trash" (reversible, no confirm), the trash menu
  // ends in "Delete Forever" (irreversible, confirmed through
  // `confirmCatalog` — the old `window.confirm` guard was a no-op
  // inside WKWebView, which is why the hard delete stayed unwired for
  // so long). Neither is in the hover strip; see the note at the top
  // of `CardActionIcons.svelte`.
  // Number keys jump `activeFilter.sortTarget`; `activeFilter.sortOrder` sticks to whatever
  // the last selection allowed (see the ORDER_OPTIONS effect above
  // that clamps to a valid choice when the target changes).
  const SORT_KEYS: Record<string, SortTarget> = {
    "1": "occurred_at",
    "2": "created_at",
    "3": "persona",
    "4": "modality",
    "5": "tag",
    "6": "group",
    "7": "cover",
    "8": "relevance",
  };

  function onWindowKeydown(event: KeyboardEvent) {
    // Anything typed into a text-entry surface is off-limits so
    // the sort / search / fullscreen shortcuts do not clobber the
    // user's input. The svelte-window listener runs at the window
    // level so this guard is needed even for the default case.
    const inField =
      (event.target instanceof HTMLElement) &&
      (event.target.tagName === "INPUT" ||
        event.target.tagName === "TEXTAREA" ||
        (event.target as HTMLElement).isContentEditable);
    if (event.key === "Escape") {
      // Mid-IME Escape cancels the composition candidate — it must
      // never close a layer (review H1; the guard the old
      // ThreadDrawer listener carried, now owned here).
      if (event.isComposing) return;
      // One Escape = one layer, most-recent first — the interaction
      // mode stack is the SoT (W5), replacing the old
      // hand-ordered if/else chain. The marquee sweep sits outside:
      // its capture-phase Escape pre-empts this handler entirely.
      switch (interaction.top) {
        // Escape on a confirm is the safe answer, so it resolves the
        // awaiting caller with `false` rather than just unmounting the
        // modal (which would leave the promise dangling forever).
        case "confirm":     confirmCatalog.cancel(); break;
        case "cardMenu":    closeCardMenu(); break;
        case "preview":     closeQuickLook(); break;
        case "pinnedBurst": closeBurst(); break;
        case "settings":    setSettingsOpen(false); break;
        case "drawer":      closeThreadDrawer(); break;
        case "prompt":      promptCatalog.cancel(); break;
        case "detail":
          // Fullscreen is DetailPane-owned state nested INSIDE the
          // "detail" layer — peel it first so a Settings modal
          // opened over fullscreen still closes in LIFO order
          // (review M3).
          if (detailPaneRef?.isFullscreen?.()) {
            detailPaneRef.exitFullscreen?.();
          } else {
            closeDetail();
          }
          break;
        case "reader":      closeReader(); break;
        case "marquee":     break; // capture-phase handler owns it
        case "queryMenu":   break; // GroupsSection's own listener closes it
        case null:
          // Stack empty: bare Escape drops the grid multi-select —
          // unless focus sits in a text field (its own Escape
          // semantics, e.g. SidebarSearch clear, must not also wipe
          // the selection — review M1).
          if (!inField && gridSelection.selectedIds.size > 0) clearSelection();
          break;
        default:
          // Future modes land here — popping is the safe default.
          interaction.pop();
          break;
      }
      return;
    }
    // A confirm question owns the keyboard while it is up, and Escape
    // (handled above, via the mode stack) is the only key it shares.
    // The `inField` guard every shortcut below relies on does not cover
    // this case: focus sits on the modal's Cancel button, not in a text
    // field, so without this line a stray `3` would rate the cards
    // behind the dialog and Space would open a Quick Look under it.
    if (confirmCatalog.request !== null) return;
    // Space = Quick Look toggle; ⇧Space = constellation for the same
    // target (W3). Both aim at the hovered card
    // first, then the selection anchor. Guarded off inside fields
    // and while ANY other overlay owns the screen — the guard must
    // enumerate the non-stack overlays too (drawer / profile card /
    // context menu / prompt modal / dispatch panels), because a
    // Space that leaks through toggleQuickLook() rewrites the grid
    // selection under an in-flight bulk operation (review H1).
    if (
      !inField &&
      (event.key === " " || event.code === "Space") &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey
    ) {
      const overlayOwnsScreen =
        openAssetId !== null ||
        readerOpen ||
        settingsOpen ||
        threadDrawerOpen ||
        profileCard !== null ||
        cardMenu !== null ||
        promptCatalog.request !== null ||
        dispatchCatalog.historyOpen ||
        dispatchCatalog.snapshotOpenId !== null;
      if (overlayOwnsScreen) return;
      if (event.shiftKey) {
        const id = quickLook?.assetId ?? quickLookTargetId();
        if (id !== null) {
          event.preventDefault();
          // The burst anchors beside the grid card — leave the peek
          // tier first so the panel isn't painted behind the Quick
          // Look backdrop (review M1).
          if (quickLook) closeQuickLook();
          void openConstellationFromKeyboard(id);
        }
        return;
      }
      event.preventDefault();
      toggleQuickLook();
      return;
    }
    // Enter escalates an open Quick Look to the full detail overlay.
    if (!inField && event.key === "Enter" && quickLook !== null && openAssetId === null) {
      event.preventDefault();
      openDetail(quickLook.assetId);
      return;
    }
    // ⌘, opens the Settings modal (mirrors the OS convention).
    if ((event.metaKey || event.ctrlKey) && event.key === ",") {
      event.preventDefault();
      setSettingsOpen(!settingsOpen);
      return;
    }
    // ⌘V pastes the clipboard image via `pasteFromClipboard`.
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "v" && !inField) {
      event.preventDefault();
      void pasteFromClipboard();
      return;
    }
    // ⌘⌫ moves the grid selection to the trash (Finder's chord for the
    // same thing). Three deliberate limits:
    //
    //   * The modifier is required. A bare Delete would put a
    //     destructive action one stray keystroke from a grid that is
    //     otherwise driven by bare keys (0-5 rate, 1-7 sort), which is
    //     the same mistake as putting delete under hover.
    //   * Not in the trash view. The key would have to mean "purge"
    //     there, and purge is the one action that never gets a
    //     shortcut — it is deliberate or it is nothing.
    //   * Not while any overlay owns the screen (detail, reader,
    //     settings, drawer, card menu, prompt, confirm — all register
    //     on the interaction stack). The selection this would act on is
    //     not what the user is looking at.
    if (
      (event.metaKey || event.ctrlKey) &&
      (event.key === "Delete" || event.key === "Backspace") &&
      !inField &&
      !interaction.overlayActive &&
      !assetPageCatalog.pageIsTrash &&
      gridSelection.selectedIds.size > 0
    ) {
      event.preventDefault();
      // Any member id expands back to the whole selection inside
      // `trashFromCard` (`selectionOrCard`), so the anchor is only a
      // way in — the action is on the selection either way.
      const anchor =
        gridSelection.lastAnchorId !== null &&
        gridSelection.selectedIds.has(gridSelection.lastAnchorId)
          ? gridSelection.lastAnchorId
          : Array.from(gridSelection.selectedIds)[0];
      void trashFromCard(anchor);
      return;
    }
    // `0`-`5` rate the hovered card. Falls back to every card in the
    // grid multi-select when nothing is hovered but the Selector is
    // active — one keystroke rates the whole batch. Field-focus guard
    // above keeps typing intact.
    if (!inField && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
        && event.key.length === 1 && event.key >= "0" && event.key <= "5") {
      event.preventDefault();
      const n = Number(event.key);
      if (hoveredCardId !== null) {
        void setRating(hoveredCardId, n);
      } else if (gridSelection.selectedIds.size > 0) {
        for (const id of gridSelection.selectedIds) void setRating(id, n);
      }
      return;
    }
    // `/` focuses the sidebar search input from anywhere on the
    // grid. Skipped when the user is already inside a field so
    // typing a literal `/` still lands as a character. The input id
    // is set inside `SidebarSearch.svelte` — DOM getElementById is
    // resilient across the scoped-CSS namespace change from
    // extracting the search widget into its own component.
    if (event.key === "/" && !inField && quickLook === null) {
      const el = document.getElementById("sidebar-search-input") as HTMLInputElement | null;
      if (el) {
        event.preventDefault();
        el.focus();
        el.select();
      }
      return;
    }
    // Number keys 1-7 jump the sort target. Only when the detail
    // overlay is not up (its `1` shortcut is 200% zoom); reverse
    // via `\` (see below).
    if (!inField && openAssetId === null && !readerOpen && !settingsOpen && quickLook === null) {
      const mapped = SORT_KEYS[event.key];
      if (mapped) {
        event.preventDefault();
        activeFilter.sortTarget = mapped;
        return;
      }
      // Backslash flips the sort direction — it moves the Order
      // selection to the other direction of the same ordering (A→Z ⇄
      // Z→A, Newest ⇄ Oldest). Every axis offers both directions of
      // whatever it offers, so the flip always lands on a real option.
      // Chosen because it sits next to the number row on a US keyboard
      // and rarely gets typed intentionally.
      if (event.key === "\\") {
        event.preventDefault();
        activeFilter.sortReverse = !activeFilter.sortReverse;
        return;
      }
    }
    // `F` toggles fullscreen from the detail overlay for image
    // assets — muscle-memory for the ⛶ button that already lives
    // in the meta panel. Fullscreen zoom keys (+/- / 0 / 1 / r)
    // are handled by DetailPane, which owns the zoom state.
    if (!inField && detailPaneRef && openAssetId !== null) {
      if (detailPaneRef.handleImageShortcut?.(event.key)) {
        event.preventDefault();
        return;
      }
    }
    // ←/→ retarget an open Quick Look (moves the selection with it).
    if (
      (event.key === "ArrowRight" || event.key === "ArrowLeft") &&
      quickLook !== null &&
      openAssetId === null
    ) {
      event.preventDefault();
      navigateQuickLook(event.key === "ArrowRight" ? 1 : -1);
      return;
    }
    if ((event.key === "ArrowRight" || event.key === "ArrowLeft") && openAssetId !== null) {
      event.preventDefault();
      void navigateDetail(event.key === "ArrowRight" ? 1 : -1);
      return;
    }
  }

  // `parseExtra` / `fmtDurationMs` / `fmtBytes` / `fmtDateTime` /
  // `personaName` moved to `lib/formatters.ts` (Phase C wave B).
  // The persona name-lookup map moved with them onto
  // `personaCatalog.nameById` (`SvelteMap`, $derived on
  // `.list`), so the same O(1) sort-key path survives the
  // extraction.

  // Group-axis sort resolves `card.group_ids[0]` to a group name at
  // every compare (110k cards over ~hundreds of groups). The lookup
  // map is `groupCatalog.nameById` — built once per counts fetch and
  // shared with the SavedQuery detail modal + URL-hydrate effect.
  // Empty `group_ids` (assets not filed anywhere) collapse to a
  // single trailing bucket via the `TAIL_SENTINEL` code point so
  // `localeCompare` puts them last.
  //
  // Takes the id list rather than the card so it satisfies the
  // `CardSortLookups.primaryGroupName` contract in
  // `./lib/sort/card-cmp` without dragging `AssetCardDto` into that
  // module's pure surface.
  function primaryGroupName(groupIds: readonly string[]): string {
    const gid = groupIds[0];
    if (!gid) return TAIL_SENTINEL;
    return groupCatalog.nameById.get(gid) ?? TAIL_SENTINEL;
  }

  // Thumb blob-URL cache moved to `thumbCatalog` (wave ②);
  // App only owns the teardown hook.
  onDestroy(() => {
    thumbCatalog.revokeAll();
  });

  // Sidebar profile card — W1 hover regrammar: the persona row itself
  // no longer opens the card on hover (scanning the sidebar must not
  // pop panels). The
  // row's ⓘ affordance is the aim target: pointing at (or clicking)
  // it opens the card immediately, anchored beside the row. The
  // profile fetch lives on `profileCatalog.ensureProfile` (wave 8b)
  // so the ⓘ open, the strip's avatar-mini, and the card's own
  // render share a single fetch.
  // Monotonic token guarding the async open: bumped by every open
  // attempt AND by row leave, so a fetch that resolves after the
  // pointer already left drops its assignment instead of orphaning
  // an open card at a vacated row.
  let profileOpenSeq = 0;
  async function onPersonaHoverEnter(personaId: string, ev: MouseEvent, explicit = false) {
    // Aim-hover respects the suppression guard; an explicit ⓘ click
    // bypasses it (same split as the card action icons).
    if (!explicit && overlaysSuppressed()) return;
    if (profileCloseTimer !== null) {
      window.clearTimeout(profileCloseTimer);
      profileCloseTimer = null;
    }
    const row = (ev.currentTarget as HTMLElement).closest("li");
    const targetRect = (row ?? (ev.currentTarget as HTMLElement)).getBoundingClientRect();
    const seq = ++profileOpenSeq;
    await profileCatalog.ensureProfile(personaId);
    if (seq !== profileOpenSeq) return;
    profileCard = {
      personaId,
      x: targetRect.right + 6,
      y: targetRect.top,
    };
  }
  // Leaving the row: invalidate any in-flight open (see
  // `profileOpenSeq`), then schedule a short grace-period close on
  // an open card so the user can hop the ~6 px gap onto the card
  // without it blinking out. The card's own `onmouseenter` cancels
  // this timer.
  function onPersonaHoverLeave() {
    profileOpenSeq++;
    if (profileCard === null) return;
    if (profileCloseTimer !== null) window.clearTimeout(profileCloseTimer);
    profileCloseTimer = window.setTimeout(() => {
      profileCloseTimer = null;
      closeProfileCard();
    }, PROFILE_CLOSE_GRACE_MS);
  }
  // Mouse landed on the card itself — cancel the grace-period close
  // (this is the hop-onto-card path that must not dismiss).
  function onProfileCardEnter() {
    if (profileCloseTimer !== null) {
      window.clearTimeout(profileCloseTimer);
      profileCloseTimer = null;
    }
  }
  function closeProfileCard() {
    profileCard = null;
    if (profileCloseTimer !== null) {
      window.clearTimeout(profileCloseTimer);
      profileCloseTimer = null;
    }
  }
  async function contextSetAvatar(card: AssetCardDto) {
    if (card.persona_id === "") return;
    const pid = card.persona_id;
    const current = profileCatalog.profiles.get(pid) ?? null;
    try {
      const next = await invoke<PersonaProfileDto>("set_persona_profile", {
        command: {
          persona_id: pid,
          avatar_asset_id: card.id,
          bio_short: current?.bio_short ?? null,
          role_tag: current?.role_tag ?? null,
        },
      });
      profileCatalog.updateProfile(pid, next);
      status = `avatar set for ${personaName(pid)}`;
    } catch (error) {
      status = `avatar error: ${JSON.stringify(error)}`;
    } finally {
      closeCardMenu();
    }
  }

  // Persona wallpaper lifecycle moved to `themeCatalog.loadFor`
  // (theme.svelte.ts, Phase C wave 8a). `untrack` prevents the state
  // writes inside `loadFor` (wallpaperUrl / theme) from feeding back
  // into this effect — without it the sync `if (this.wallpaperUrl)
  // { … }` check registers a read on the same signal `loadFor` later
  // writes to, and the effect loops (a flashing wallpaper chip and a
  // background that never sticks). The `untrack` boundary stays here
  // because the reactive read of `activeFilter.activePersona` is
  // what drives dispatch — the store has no business knowing that.
  $effect(() => {
    const target = activeFilter.activePersona;
    untrack(() => {
      void themeCatalog.loadFor(target);
    });
  });

  // Detail-panel action: pin the currently-opened asset as the
  // active persona's wallpaper. Requires an active persona (the "all"
  // view has no owner to attach the theme to) and an image asset.
  //
  // The `wallpaperSaving` guard blocks re-entry while the save is in
  // flight — earlier iterations without it double-fired on rapid
  // clicks and made the persisted row race with the reload.
  let wallpaperSaving = $state(false);
  async function setAsWallpaper(assetId?: string) {
    if (assetId === undefined || activeFilter.activePersona === null || wallpaperSaving) return;
    wallpaperSaving = true;
    const targetPersona = activeFilter.activePersona;
    const targetAssetId = assetId;
    try {
      await invoke<PersonaThemeDto>("set_persona_theme", {
        command: {
          persona_id: targetPersona,
          wallpaper_asset_id: targetAssetId,
        },
      });
      status = `wallpaper set for ${personaName(targetPersona)}`;
      // Re-run the full theme + wallpaper load path so the pseudo
      // element picks up the new background even when
      // `activeFilter.activePersona` has not changed since the last load.
      await themeCatalog.loadFor(targetPersona);
    } catch (error) {
      console.warn("set_persona_theme failed", error);
      status = `wallpaper error: ${JSON.stringify(error)}`;
    } finally {
      wallpaperSaving = false;
    }
  }

  async function clearWallpaper() {
    if (activeFilter.activePersona === null || wallpaperSaving) return;
    wallpaperSaving = true;
    const targetPersona = activeFilter.activePersona;
    try {
      await invoke("delete_persona_theme", {
        command: { persona_id: targetPersona },
      });
      status = `wallpaper cleared`;
      await themeCatalog.loadFor(targetPersona);
    } catch (error) {
      console.warn("delete_persona_theme failed", error);
    } finally {
      wallpaperSaving = false;
    }
  }

  // `ensureThumb` / `thumbSrc` / `detailSrc` moved to
  // `thumbCatalog` (wave ②).

  // Direction / edge-kind presentation helpers + `burstSelectAllInGrid`
  // moved to `ConstellationBurst.svelte` (wave ④).

  function fmtDate(ms: number): string {
    const d = new Date(ms);
    return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }
</script>

<div
  class="layout"
  class:has-wallpaper={themeCatalog.wallpaperUrl !== null}
  class:drop-active={dropOverlay}
  style={themeCatalog.wallpaperUrl ? `--persona-wallpaper: url("${themeCatalog.wallpaperUrl}")` : ""}
>
  {#if sessionRebuildActive}
    <div class="rebuild-banner" role="status" aria-live="polite">
      <span class="rebuild-spinner" aria-hidden="true"></span>
      Building session index…
    </div>
  {/if}
  <aside class="sidebar">
    <h1>
      ⬒ Asterism
      {#if dataProfile !== "dogfood"}
        <span class="profile-badge" class:bench={dataProfile === "bench"}>{dataProfile}</span>
      {/if}
      <button
        class="clean-toggle"
        class:on={cleanMode}
        onclick={() => setPreference(SETTING_KEYS.cleanMode, !cleanMode)}
        title={cleanMode ? "Clean mode ON — click to show full detail" : "Clean mode OFF — click to reduce card chrome"}
        aria-label="Toggle clean grid mode"
      >
        <span class="clean-toggle-track">
          <span class="clean-toggle-knob"></span>
        </span>
        <span class="clean-toggle-label">Clean</span>
      </button>
      <button
        class="settings-gear"
        onclick={() => setSettingsOpen(true)}
        title="Settings (⌘,)"
        aria-label="Open settings"
      >⚙</button>
    </h1>
    <p class="status">{status}</p>

    <SidebarSearch
      onSearchDebounce={reloadWithSearchDebounce}
      onSearchImmediate={reloadSearchImmediate}
    />

    <!-- Discover — the ways in that are not "I know what I am looking
         for". Directly under the box because the two
         are the same question asked from opposite ends, and because
         this is the slot the old sidebar chip band left when it moved
         to the grid top. -->
    <DiscoverSection onReloadNow={reloadSearchImmediate} />

    <!-- Inbox / triage — persistent bucket for freshly-ingested
         assets. Every asset the ingest path lands in the DB gets
         the "inbox" label (see AssetService::add); clicking the
         chip toggles a label filter so the grid narrows to
         "unreviewed". Removing the label from an asset (detail
         pane) graduates it out of the Inbox. Persona-scoped by
         design — switch persona to "● all" for a cross-persona
         triage view. -->
    <h2>Inbox</h2>
    <ul>
      <li>
        <button
          class:active={activeFilter.activeLabel === "inbox"}
          onclick={() => (activeFilter.activeLabel = activeFilter.activeLabel === "inbox" ? null : "inbox")}
          title="Toggle: show only assets still carrying the `inbox` triage label"
        >
          📥 needs review
        </button>
      </li>
    </ul>

    {#if recentDropIds.size > 0}
      <h2>Recently added</h2>
      <ul>
        <li>
          <button
            class:active={filterRecentDrops}
            onclick={() => (filterRecentDrops = !filterRecentDrops)}
            title="Toggle: show only what you dropped in this session"
          >
            ▨ dropped this session · {recentDropIds.size}
          </button>
        </li>
        <li>
          <button
            onclick={() => {
              recentDropIds = new Set();
              filterRecentDrops = false;
            }}
            title="Forget the recently dropped list"
          >
            × clear
          </button>
        </li>
      </ul>
    {/if}

    <PersonaStrip
      {onPersonaHoverEnter}
      {onPersonaHoverLeave}
      onClearWallpaper={clearWallpaper}
    />

    <ModalityList />
    <!-- asset-model v4 P3: the two structural axes get their own
         sections — FORMAT (material mime facet) and GROUPING (Session
         containers; click opens the container's reader, the only
         place members surface). -->
    <FormatList />
    <!-- KIND (a facet over `role`) was removed once containers carried
         a modality of their own: it listed exactly the rows MODALITY →
         Session already lists. `role` still decides how a card draws
         itself, but it is not a browsing axis. -->
    <!-- COLOR is the third derived facet: the dominant-colour palette
         quantised into a closed swatch set. Composes with FORMAT and
         Modality rather than replacing either. -->
    <ColorList />
    <!-- Length / Size: two numeric bands over facts of the material,
         beside the other derived facets for that reason. Unlike them
         these are ranges rather than a closed set of values, so the
         section takes inputs instead of rows — and naming either end
         drops the material that has no such value (a still image has no
         length), which the section says on screen. Units are seconds /
         MB here and ms / bytes on the wire; `activeFilter.metricBands()`
         owns that conversion. -->
    <MetricBands />
    <!-- GROUPING (a list of individual containers, click to open the
         reader) was removed once containers became first-class cards.
         It was a third way to reach the same rows — KIND → Sessions
         narrows the grid to them, MODALITY → Session does the same on
         the semantic axis, and clicking a card opens it. A sidebar
         section that lists rows instead of filtering them was also the
         odd one out among the facets. -->

    <!--
      Trash is a view toggle, not a filter chip: it flips which side of
      `trashed_at` every query reads, and composes with the persona /
      modality / tag filters already set (so "the trash, for this
      persona" is askable). Restore / delete-forever live on the card
      icons in this mode.
    -->
    <h2>Trash</h2>
    <ul>
      <!-- Also a drop target: dragging a card here trashes it. Withheld
           while the grid already shows the trash — a trashed card is
           not re-trashable, same withholding shape the other rows use
           for a payload they refuse. -->
      <li
        class:drop-target={cardDrag.isOver("trash", "trash")}
        data-drop-kind={assetPageCatalog.pageIsTrash ? undefined : "trash"}
        data-drop-id={assetPageCatalog.pageIsTrash ? undefined : "trash"}
      >
        <button
          class:active={activeFilter.trashView}
          onclick={toggleTrashView}
          title="Show trashed items. Nothing here is deleted until it ages out or you delete it forever."
        >
          {activeFilter.trashView ? "◉" : "○"} trash
        </button>
      </li>
      <!-- Sits under Trash rather than beside the facets: it is not a
           way to look at the library, it is a maintenance job whose
           only action ends in the trash. -->
      <li>
        <button
          onclick={() => (duplicatesOpen = true)}
          title="Find assets whose original files are byte-identical."
        >
          ○ duplicates
        </button>
      </li>
    </ul>

    <!--
      Its own heading, because neither row under it is a facet and the
      one they sat under was Trash.

      A line is not a property an asset has. The grid narrows by
      persona, modality and tag, which assets carry; a line refers to
      assets and names them its own way, so one asset sits on two lines
      under two names. Narrowing the grid by a line would replace what
      it lists rather than filter it (#170). The team's lines are not a
      facet for a stronger reason on top of that one: what that row
      opens is not this library at all — they are read from that team's
      server. It sat under Trash until #170 gave these two a heading.

      Two rows under one heading is not the mixing #148 decision 16
      refuses. That decision is about where lines are *listed*: shared
      ones get their own panel so a surface never claims one library
      where there are two. These are the doors to those two panels, and
      each says which it opens.
    -->
    <h2>Forge</h2>
    <ul>
      <li>
        <button
          onclick={() => void forgeCatalog.openPanel()}
          title="Lines on this machine: what each holds, and how it got there."
        >
          ○ lines
        </button>
      </li>
      <li>
        <button
          onclick={() => void sharedCatalog.openPanel()}
          title="Lines a team hosts. Read from the team's server, not from here."
        >
          ○ shared lines
        </button>
      </li>
    </ul>

    <h2>Voice</h2>
    <ul>
      <li>
        <button class:active={activeFilter.activeLabel === null} onclick={() => (activeFilter.activeLabel = null)}>
          ● all
        </button>
      </li>
      {#each ROLES as [slug, label] (slug)}
        <li>
          <button class:active={activeFilter.activeLabel === slug} onclick={() => (activeFilter.activeLabel = slug)}>
            ○ {label}
          </button>
        </li>
      {/each}
    </ul>

    <!--
      W5a: SelectionsList / SavedQueriesList are gone. Persistent
      "selections" are absorbed by Groups and saved
      filters are Query Groups that live inside GroupsSection with a
      kind='query' icon. The Groups section below is the whole
      persistent-set surface now (tags + groups = 2 concepts, per the
      sidebar shape).
    -->

    <TagList />

    <GroupsSection onToggleDirFocus={toggleDirFilter} />
  </aside>

  <main class="content" class:clean-mode={cleanMode}>
    <!-- View mode toggle: Messages (1 tile / asset) vs Sessions
         (1 tile / session_id). Search box is Messages-only so we
         disable it in Sessions view. -->
    <div class="mode-row">
      <div class="view-mode">
        <button
          type="button"
          class:active={activeFilter.viewMode === "messages"}
          onclick={() => {
            activeFilter.viewMode = "messages";
            // The dir-focused lane is a Groups-view construct;
            // going back to Messages should not leave the lane
            // hanging over the grid.
            focusedDirId = null;
            scrollToTop();
          }}
        >
          Messages
        </button>
        <button
          type="button"
          class:active={activeFilter.viewMode === "groups"}
          onclick={() => {
            clearSession();
            activeFilter.viewMode = "groups";
            // Opening the Groups view lands on Root by default so
            // the drill-down UX is immediately visible; the lane
            // stays closed in Messages view unless the user opts
            // in via the toolbar toggle.
            if (focusedDirId === null) focusedDirId = ROOT;
            scrollToTop();
          }}
        >
          Groups
        </button>
      </div>
      <span class="sort-picker">
        {#if randomOrderActive}
          <!-- The draw owns the sequence, and it is the shuffle itself.
               Same treatment as the ✦ branch below: state what orders
               the grid instead of offering a control that would not.
               Checked first because a 🔍 query can be lit at the same
               time, and then it is the draw that decided the order. -->
          <span
            class="sort-manual"
            title="Random picks come back shuffled. Turn off 🎲 Random to sort by a field again."
          >Order: 🎲 Shuffle</span>
        {:else if searchOrderActive}
          <!-- Relevance, not this picker, owns the order here. Showing
               the selects would offer a choice with no effect on the
               grid. See `searchOrderActive`. -->
          <span
            class="sort-manual"
            title="Search results are ranked by relevance. Clear the search to sort by a field again."
          >Order: ⌕ Relevance</span>
        {:else}
        <label title="Grid sort dimension">
          Sort:
          <select bind:value={activeFilter.sortTarget}>
            <option value="occurred_at">Occurred</option>
            <option value="created_at">Added</option>
            <option value="persona">Persona</option>
            <option value="modality">Modality</option>
            <option value="tag">Tag</option>
            <option value="group">Group</option>
            <option value="cover">Cover</option>
            <!-- Orders the exact page by the Retrieval rank.
                 Only does anything with 🔍 exact text
                 in the box — the count line says how much of the page
                 is ranked, and with no rank the grid falls back to
                 Occurred rather than pretending. -->
            <option value="relevance">✦ Relevance</option>
            <!-- Length and size. These two were withheld from the picker
                 while the index rows the grid sorts carried neither
                 column: offering an axis is a claim that picking it
                 changes the order, and both would have handed back
                 `occurred_at DESC` — the shape that got msg_count
                 retired. The row earns them now
                 (`AssetIndexEntryDto.duration_ms` / `file_size_bytes`
                 → `indexToLightCard`), and the comparator was already
                 the backend's (`card-cmp.ts` ↔ `sort_eval.rs`).
                 Rows with no value tail in both directions rather than
                 reading as zero, so "Shortest first" does not open on a
                 still image. -->
            <option value="duration">Length</option>
            <option value="file_size">Size</option>
            <!-- Resolution as a total pixel count, and labelled
                 "Pixels" rather than "Resolution" because that is what
                 it orders on. The stored dimensions are the coded ones,
                 taken before orientation is applied, so an upright phone
                 capture sits in the row as a landscape pair; their
                 product is unchanged by that rotation and a per-side
                 axis would not be. "Resolution" invites the 1920×1080
                 reading, which is the pair this axis deliberately does
                 not offer. -->
            <option value="pixels">Pixels</option>
            <!-- msg_count retired from the picker with the grid
                 Session tiles (asset-model v4 P3). The wire token
                 outlived the option for a while, ordering by nothing
                 wherever a saved query still named it; it is gone from
                 the union and from `SortSpec` now, so such a spec is
                 refused rather than answered in arrival order. -->
          </select>
        </label>
        <!-- Direction lives in here, not in a separate toggle: the
             option text states the whole ordering, so it cannot be
             contradicted by a control sitting next to it. `\` still
             flips direction, which moves this selection. -->
        <label
          title="How the grid orders within that dimension. 'As arranged' is the order you put things in yourself — sidebar order for Persona and Modality, card positions for Group. \\ flips direction."
        >
          Order:
          <select
            value={currentOrderValue}
            onchange={(e) => {
              const picked = ORDER_CHOICES.find(
                (c) => c.value === e.currentTarget.value,
              );
              if (!picked) return;
              activeFilter.sortOrder = picked.order;
              activeFilter.sortReverse = picked.reverse;
            }}
          >
            {#each orderChoicesFor(activeFilter.sortTarget) as choice (choice.value)}
              <option value={choice.value}>{choice.label}</option>
            {/each}
          </select>
        </label>
        {/if}
      </span>
      <button
        type="button"
        class="thread-open-btn"
        onclick={openThreadDrawer}
        title="Open the Threads drawer (app-level notes + Claude Code messages)"
      >
        ⤴ Threads{#if threadsUnread > 0}<span
            class="thread-unread-badge"
            title="{threadsUnread} thread(s) with new messages"
          >{threadsUnread}</span>{/if}
      </button>
      <!-- The 📁 Root shortcut retired: the Groups tab already
           opens the dir/group drill-down, and duplicating the
           trigger inside Messages felt like clutter. -->
    </div>

    <ActiveFilters
      onClearSearch={clearSearch}
      onReset={resetFilters}
      onSaveAsGroup={saveAsQueryGroup}
    />

    <JobsTickerBanner />

    {#if (activeFilter.viewMode === "messages" || activeFilter.viewMode === "groups") && focusedDirId !== null}
      <!-- Dir-focused lane. The header names the focused dir; the
           first strip is sub-dirs (click to drill down), the
           second is groups filed directly under this dir (click
           to filter). Sub-dirs use folder icons, groups use tag
           icons so the difference reads at a glance. Empty rows
           collapse so a leaf dir with only groups (or only
           sub-dirs) doesn't paint an empty strip. -->
      <div class="dir-lane">
        <div class="dir-lane-head">
          <button
            type="button"
            class="dir-lane-up"
            disabled={focusedDirId === ROOT}
            onclick={goUpDir}
            title="Go up one level"
            aria-label="Go up"
          >
            ▲
          </button>
          <span class="dir-lane-crumb">
            {#each dirBreadcrumb as id, i (id)}
              {#if i > 0}
                <span class="dir-lane-crumb-sep">▸</span>
              {/if}
              {#if id === focusedDirId}
                <span class="dir-lane-crumb-current">
                  📁 {crumbLabel(id)}
                </span>
              {:else}
                <button
                  type="button"
                  class="dir-lane-crumb-link"
                  onclick={() => (focusedDirId = id)}
                  title="Jump to {crumbLabel(id)}"
                >
                  {crumbLabel(id)}
                </button>
              {/if}
            {/each}
          </span>
          <span class="dir-lane-counts">
            {laneChildDirs.length} sub-dir{laneChildDirs.length === 1 ? "" : "s"}
            · {laneChildGroups.length} group{laneChildGroups.length === 1 ? "" : "s"}
          </span>
          <button
            type="button"
            class="dir-lane-close"
            onclick={clearDirFocus}
            aria-label="Close dirs lane"
            title="Close lane"
          >
            ✕
          </button>
        </div>
        {#if laneChildDirs.length > 0}
          <div class="dir-lane-row lane-tiles">
            <span class="dir-lane-label">▶ dirs</span>
            {#each laneChildDirs as child (child.id)}
              {@const thumbUrl = thumbCatalog.dirCoverThumb(child.id)}
              <button
                type="button"
                class="lane-tile"
                onclick={() => (focusedDirId = child.id)}
                title="Focus lane on {child.name}"
              >
                <div class="lane-tile-thumb lane-tile-thumb-dir">
                  {#if thumbUrl}
                    <img src={thumbUrl} alt="" loading="lazy" />
                  {:else}
                    <span class="lane-tile-icon">📁</span>
                  {/if}
                </div>
                <div class="lane-tile-name">{child.name}</div>
              </button>
            {/each}
          </div>
        {/if}
        <!-- Immediate-groups strip removed: entering a dir means
             "narrow to the assets inside this dir", so the leaf
             group tiles duplicated the drill target. The dir
             filter still needs an explicit apply, though — see
             the button below. -->
        {#if laneChildGroups.length > 0}
          <div class="dir-lane-row dir-lane-apply">
            <button
              type="button"
              class="dir-lane-apply-btn"
              class:active={laneChildGroups.every((gc) =>
                activeFilter.activeGroupIds.has(gc.group.id),
              )}
              onclick={() => {
                if (focusedDirId !== null) applyDirFilter(focusedDirId);
              }}
              title="Toggle narrowing the grid to every asset under this dir"
            >
              📁 Filter grid to this dir
              <span class="dir-lane-apply-count">
                ({laneChildGroups.reduce((s, gc) => s + gc.asset_count, 0)} items)
              </span>
            </button>
          </div>
        {/if}
        {#if laneChildDirs.length === 0 && laneChildGroups.length === 0}
          <p class="dir-lane-empty">(Nothing directly under this dir. Pick an entry in the sidebar.)</p>
        {/if}
      </div>
    {/if}

    <!-- Fetch-in-flight pill: a 6-figure persona switch still costs
         ~1-2 s in the IPC/JSON layer, so surface the wait instead
         of looking frozen. Reads the store loading flags directly;
         the last-good page stays on screen underneath. -->
    {#if showLoadingPill}
      <div class="grid-loading-pill" role="status">
        <span class="grid-loading-spinner"></span> loading…
      </div>
    {/if}

    {#if assetPageCatalog.page}
      <p class="count">
        {#if assetPageCatalog.random}
          <!--
            The draw's own two numbers, and no third one pretending to
            be a page: `picked` is what is on screen, `setTotal` is the
            set it came out of (exact — the pool is a SQL predicate).
            "from" rather than "of" on purpose; "N of M" would read as a
            page into M, and there is no second page here.
          -->
          {assetPageCatalog.random.picked} picks from
          {assetPageCatalog.random.setTotal}
          <button
            type="button"
            class="count-escape"
            onclick={() => activeFilter.reshuffle()}
            title="Draw another handful out of the same filter"
          >
            ↻ reshuffle
          </button>
        {:else if assetPageCatalog.retrieval}
          <!--
            Retrieval has no library-wide count to show. Saying "N of the
            top M" is the whole claim it can make, and it is worth making
            even when the shortlist did not fill up — the number is about
            the candidates looked at either way, and a phrasing that
            switches shape would read as two different measurements.
          -->
          {assetPageCatalog.retrieval.matched} of the top
          {assetPageCatalog.retrieval.candidatesConsidered}
        {:else}
          {assetPageCatalog.page.total ?? assetPageCatalog.page.items.length} item(s)
        {/if}
        {#if pageDateRange}
          <span class="count-hint">{pageDateRange}</span>
        {/if}
        {#if activeFilter.searchText.trim().length > 0 && !(assetPageCatalog.random && activeFilter.searchFuzzy)}
          <!--
            The two modes measure different things, so they may not
            borrow each other's words: ✦ ranks candidates by nearness
            ("closest to"), 🔍 tests a predicate over the whole set
            ("containing"). Keyed off `searchFuzzy` rather than the
            presence of `retrieval`, because in exact mode `retrieval`
            is null while the text is very much in effect — it rides on
            `filter.text_match` down the list path.

            Suppressed for ✦ text under a random draw, and only there:
            that text reaches no query at all (the draw sends none, and
            `text_match` is null in fuzzy mode), so "closest to X" would
            describe a narrowing that did not happen. 🔍 text keeps its
            hint — it *is* in the predicate, narrowing the pool the
            picks came from.
          -->
          <span class="count-hint">
            {activeFilter.searchFuzzy ? "closest to" : "containing"}
            “{activeFilter.searchText.trim()}”
          </span>
        {/if}
        {#if rankedOnPage > 0}
          <!--
            The `✦ Relevance` axis, stated as the partial ordering it is:
            the page is the exact set (its count above is still the
            count), and this many of its rows carry a rank that pulled
            them to the front. The rest sit behind in the default order.
          -->
          <span class="count-hint">✦ {rankedOnPage} ranked first</span>
          {#if assetPageCatalog.rankInfo?.truncated}
            <!--
              The shortlist hit its ceiling, so the ranking describes the
              top of the library rather than all of it. Same tone as the
              ✦ path's own truncation note — a number with the width of
              the net it was measured over.
            -->
            <span class="count-hint"
              >top {assetPageCatalog.rankInfo.candidatesConsidered} scanned</span
            >
          {/if}
        {/if}
        {#if assetPageCatalog.retrieval?.truncated}
          <span class="count-hint"
            >more beyond the shortlist — narrow with filters</span
          >
          <!--
            The way out of a capped shortlist, next to the sentence that
            reports it: the exact side answers the same
            text as a set, with no K to fall off the end of. Narrowing
            with filters stays the other option — this does not replace
            the hint, it gives it an action.
          -->
          <button
            type="button"
            class="count-escape"
            onclick={switchToExactSearch}
            title="Re-ask the same text as an exact predicate — every match, counted"
          >
            see every match — 🔍 exact
          </button>
        {/if}
        {#if activeFilter.activeSessionId && activeFilter.activeSessionLabel}
          <button
            type="button"
            class="active-tag-chip reader-chip"
            onclick={openReader}
            title="Read this session as one continuous transcript"
          >
            📖 read
          </button>
        {/if}
      </p>
      <!-- Trash toolbar. Only on the trash side, and keyed off
           `pageIsTrash` (the page that is actually on screen) rather
           than `activeFilter.trashView` (the toggle's intent, which
           flips before the grid does) — every destructive affordance
           in this file follows that rule, because a Delete Forever
           painted over a live grid for one frame is the failure the
           rule exists to prevent.

           Restore and Delete Forever act on the selection, and read as
           disabled with nothing selected rather than disappearing: a
           control that comes and goes teaches nothing about what it
           needs. Empty Trash is the odd one out on purpose — it takes
           no selection and no filter, so it sits apart on the right,
           the way Finder and Photos place the same button. -->
      {#if assetPageCatalog.pageIsTrash}
        {@const chosen = gridSelection.selectedIds.size}
        <div class="trash-toolbar" role="toolbar" aria-label="Trash actions">
          <button
            type="button"
            class="trash-toolbar-btn trash-toolbar-restore"
            disabled={chosen === 0}
            onclick={() => void restoreMany(Array.from(gridSelection.selectedIds))}
            title="Bring the selected assets back to the live set"
          >
            ↩︎ Restore{chosen > 0 ? ` (${chosen})` : ""}
          </button>
          <button
            type="button"
            class="trash-toolbar-btn trash-toolbar-danger trash-toolbar-purge"
            disabled={chosen === 0}
            onclick={() => void purgeMany(Array.from(gridSelection.selectedIds))}
            title="Delete the selected assets permanently"
          >
            Delete Forever{chosen > 0 ? ` (${chosen})` : ""}
          </button>
          <span class="trash-toolbar-gap"></span>
          <button
            type="button"
            class="trash-toolbar-btn trash-toolbar-danger trash-toolbar-empty"
            disabled={(assetPageCatalog.page?.total ??
              assetPageCatalog.page?.items.length ??
              0) === 0}
            onclick={() => void emptyTrash()}
            title="Delete everything in the trash permanently, filter or no filter"
          >
            Empty Trash
          </button>
        </div>
      {/if}
      <!-- Nested-collection band: while browsing exactly one group,
           its connected child groups surface as chips above the
           asset grid (click = drill in, ✕ = disconnect). Assets of
           the children are already included in the grid through the
           descendant expansion in currentFilter(). -->
      {#if soloGroupId !== null && (groupCatalog.childGroupsByParent.get(soloGroupId) ?? []).length > 0}
        <div class="child-band">
          <span class="child-band-label">⊂ nested</span>
          {#each groupCatalog.childGroupsByParent.get(soloGroupId) ?? [] as cgc (cgc.group.id)}
            <span class="child-chip">
              <button
                class="child-chip-name"
                onclick={() => drillIntoGroup(cgc)}
                title="Open ~{cgc.group.name}"
              >
                ~ {cgc.group.name}
                <span class="child-chip-count">{cgc.asset_count}</span>
              </button>
              <button
                class="child-chip-x"
                onclick={() => soloGroupId !== null && unlinkGroups(soloGroupId, cgc.group.id)}
                title="Disconnect ~{cgc.group.name} from this group"
              >
                ✕
              </button>
            </span>
          {/each}
        </div>
      {/if}
      <!-- Content-type filter row — appears once a session is open and
           at least one message carries a detectable structure (code,
           table, mermaid, link). Click a chip to narrow the grid to
           cards with that flag; multiple chips combine as AND. -->
      {#if activeFilter.activeSessionId !== null && (flagCounts.code + flagCounts.table + flagCounts.mermaid + flagCounts.link) > 0}
        <div class="content-flag-band">
          <span class="flag-band-label">◇ content</span>
          {#each CONTENT_FLAGS as f (f.id)}
            {#if flagCounts[f.id] > 0}
              <button
                type="button"
                class="flag-chip"
                class:active={activeContentFlags.has(f.id)}
                onclick={() => toggleContentFlag(f.id)}
                title="Toggle #{f.label} filter"
              >
                <span class="flag-icon">{f.icon}</span>
                <span class="flag-name">{f.label}</span>
                <span class="flag-count">{flagCounts[f.id]}</span>
              </button>
            {/if}
          {/each}
          {#if activeContentFlags.size > 0}
            <button
              type="button"
              class="flag-clear"
              onclick={() => activeContentFlags.clear()}
              title="Clear content filter"
            >
              ✕
            </button>
          {/if}
        </div>
      {/if}
      <div
        class="grid-wrapper"
        class:reorder-mode={reorderActive}
        class:drag-mode={draggableActive}
        class:clean-mode={cleanMode}
        class:marquee-active={marqueeActive}
        bind:this={gridWrapperEl}
        onmousedown={onGridMouseDown}
        role="presentation"
      >
        <VList data={filteredRows} getKey={(row, _i) => row.key} style="height: 100%;">
          {#snippet children(row, _rowIndex)}
            {#if row.kind === "header"}
              <div
                class="grid-group-header"
                class:major={row.level === "major"}
              >{row.label}</div>
            {:else}
            <div
              class="grid-row"
              style="grid-template-columns: repeat({gridCols}, minmax(0, 1fr));"
            >
              {#each row.items as item (item.kind === "session" ? `s:${item.session.id}` : `m:${item.card.id}`)}
                {#if item.kind === "session"}
                  <SessionTile
                    session={item.session}
                    onOpen={openSession}
                    onOpenNote={openSessionNoteFromIcon}
                    onOpenComment={openSessionCommentsFromIcon}
                  />
                {:else}
                {@const card = item.card}
                {@const rc = assetPageCatalog.hydratedCard(card)}
                <!-- A card registers as a drop target only while the
                     arrangement is what the grid is showing
                     (`reorderActive`). Withholding the `data-drop-*`
                     attributes is the same shape `ModalityList` /
                     `GroupsSection` use for a row that does not accept the
                     payload, and it is what keeps the insertion line off:
                     `reorderOnto` already refuses the write, so an
                     affordance that lights up anyway promises a move the
                     drop will not make. -->
                <div
                  class="card"
                  data-asset-id={card.id}
                  class:hovered={burst?.assetId === card.id}
                  class:drop-target={cardDrag.isOver("card", card.id)}
                  class:dragging={cardDrag.sourceOf("card") === card.id}
                  class:selected={gridSelection.selectedIds.has(card.id)}
                  data-drop-kind={reorderActive ? "card" : undefined}
                  data-drop-id={reorderActive ? card.id : undefined}
                  onpointerdown={(e) => {
                    if (draggableActive) {
                      beginDrag(e, { kind: "card", id: card.id }, onCardDropTarget);
                    }
                  }}
                  onmouseenter={() => onCardEnter(rc)}
                  onmouseleave={onCardLeave}
                  onclick={(e) => onCardClick(e, rc.id)}
                  onkeydown={(event) => event.key === "Enter" && quickLook === null &&
                    (rc.role === "collection" ? openReaderForSession(rc.id) : openDetail(rc.id))}
                  oncontextmenu={(e) => openCardMenu(e, rc)}
                  role="button"
                  tabindex="0"
                >
                  <div class="card-head">
                    {#if rc.role === "collection"}
                      <!-- A container has no modality by design (v4
                           moved structure off that axis), so the slot
                           carries what it *is* instead of an empty
                           badge. -->
                      <span class="badge badge-collection" title="Container — its content is its members">◌ session</span>
                    {:else}
                      <span class="badge">{rc.modality}</span>
                    {/if}
                    {#if rc.labels.includes("inbox")}
                      <span
                        class="inbox-badge"
                        title="Still in the Inbox — clear the `inbox` label to graduate this out of triage"
                      >📥</span>
                    {/if}
                    {#if flagsByCard.get(rc.id)}
                      {#each CONTENT_FLAGS as f (f.id)}
                        {#if flagsByCard.get(rc.id)?.has(f.id)}
                          <span class="flag-badge" title={f.label}>{f.icon}</span>
                        {/if}
                      {/each}
                    {/if}
                    {#if rc.score !== null && rc.score !== undefined}
                      <!--
                        Search rank badge — the BM25 score assigned
                        by the Tantivy full-text index. Present only
                        when the current view came from
                        `search_assets`; list mode leaves it out so
                        the badge is a visible signal that ranking
                        is active. Two decimals keeps the range
                        readable (typical scores 5-40) without
                        overflowing the head bar.
                      -->
                      <span class="score-badge" title="BM25 rank score">
                        {rc.score.toFixed(2)}
                      </span>
                    {/if}
                    <span
                      class="rating"
                      class:rated={rc.rating !== null && rc.rating !== undefined && rc.rating > 0}
                      title="Rating (0-5) · click a star to set · 0-5 keys apply to the hovered card"
                    >
                      {#each [1, 2, 3, 4, 5] as n (n)}
                        <button
                          type="button"
                          class="rating-star"
                          class:filled={(rc.rating ?? 0) >= n}
                          onclick={(e) => { e.stopPropagation(); void setRating(rc.id, (rc.rating ?? 0) === n ? 0 : n); }}
                          aria-label={`Rate ${n}`}
                        >★</button>
                      {/each}
                    </span>
                    <span class="date">{fmtDate(rc.occurred_at_ms)}</span>
                  </div>
                  {#if rc.role === "collection"}
                    <!-- Container body: the name is the content. Falls
                         back to the generated cover, then to a plain
                         "(untitled)" — never to the item path, which
                         would look for material the container does not
                         own. The member count is the container's
                         headline number. -->
                    <p class="cover cover-collection">
                      {rc.title ?? rc.cover ?? "(untitled session)"}
                    </p>
                    <p class="collection-meta">{rc.member_count} item{rc.member_count === 1 ? "" : "s"}</p>
                  {:else if cardIsVisual(rc)}
                    <div class="thumb">
                      {#if rc.source_locator}
                        <img
                          src={thumbCatalog.thumbSrc(rc)}
                          alt={rc.cover ?? ""}
                          loading="lazy"
                          onerror={(e) => thumbCatalog.noteOriginalError(rc.id, e)}
                        />
                      {:else}
                        <!--
                          Placeholder while the viewport hydration
                          fetch is in flight. Empty `source_locator`
                          is the "light card, not yet hydrated"
                          marker — feeding it to `convertFileSrc`
                          would ask the asset protocol to serve an
                          empty path.
                        -->
                        <div class="thumb-placeholder"></div>
                      {/if}
                    </div>
                    {#if rc.palette && rc.palette.length > 0}
                      <div class="palette-strip" title="Dominant colours">
                        {#each rc.palette as hex, i (i)}
                          <span class="palette-swatch" style="background: {hex}"></span>
                        {/each}
                      </div>
                    {/if}
                    {#if rc.snippet}
                      <!--
                        Search snippet takes over the cover slot when
                        present. `{@html}` is safe here because the
                        server's Tantivy `SnippetGenerator.to_html()`
                        HTML-escapes every source character and only
                        injects the `<b>` wrapping around matched
                        terms — the loop is closed and there is no
                        untrusted markup path.
                      -->
                      <p class="cover cover-image cover-snippet">{@html rc.snippet}</p>
                    {:else if rc.cover}
                      <p class="cover cover-image">{rc.cover}</p>
                    {/if}
                  {:else}
                    {#if rc.snippet}
                      <p class="cover cover-snippet">{@html rc.snippet}</p>
                    {:else}
                      <!-- One phrase per state (v4 P3 wording unification):
                           "(loading…)" = the light index row is still
                           hydrating; "(no cover yet)" = hydrated but
                           cover_gen has not produced text — the same
                           wording SessionTile uses. -->
                      <p class="cover">{rc.cover ??
                        (assetPageCatalog.hydration.has(rc.id)
                          ? "(no cover yet)"
                          : "(loading…)")}</p>
                    {/if}
                  {/if}
                  {#if cleanMode && cardIsVisual(rc) && !rc.cover}
                    <!-- Clean mode's Content Name — a picture with no
                         cover text (image or video) uses the source
                         basename as its label so a name is still
                         visible on the card. Sidebar-only, invisible
                         in full mode. -->
                    <p class="cover clean-basename">{baseName(rc.source_locator)}</p>
                  {/if}
                  {#if rc.labels.length > 0}
                    <div class="labels">
                      <!-- Key is label+index, not the label alone: two
                           equal labels are two equal keys, and Svelte
                           answers that with each_key_duplicate — a
                           thrown error that takes the whole virtual
                           list down, not a doubled chip. The backend
                           drops repeats on the write and the read side,
                           so nothing should arrive here carrying one.
                           This stays anyway: the cost is an index, and
                           what it buys is that the grid does not hinge
                           on every future read path remembering. -->
                      {#each rc.labels as label, i (`${label}:${i}`)}
                        <span class="label">{label}</span>
                      {/each}
                    </div>
                  {/if}
                  <p class="persona-name">{personaName(rc.persona_id)}</p>
                  <!-- Card action-icon strip (Eagle-style): floats
                       inside the card on hover. W1 hover regrammar:
                       hover only reveals the strip; ✦ is the sole
                       aim-hover target
                       (view-only panel, opens at 0 ms beside the
                       card). Note / Thread fire on click only;
                       Detail opens via card click / Enter, Space
                       peeks. Filled tone signals "already has
                       content". -->
                  <CardActionIcons
                    hasNote={rc.has_note}
                    hasThread={rc.has_thread}
                    hasConstellation={burst?.assetId === rc.id}
                    trashMode={assetPageCatalog.pageIsTrash}
                    onNoteClick={(e) => { e.stopPropagation(); void openCardNoteFromIcon(rc, e); }}
                    onThreadClick={(e) => { e.stopPropagation(); void openCardThreadFromIcon(rc, e); }}
                    onConstellationClick={(e) => { e.stopPropagation(); onConstellationIconClick(rc, e); }}
                    onConstellationHoverEnter={(e) => { if (!overlaysSuppressed() && !burst?.pinned) void openCardConstellationFromIcon(rc, e); }}
                    onRestoreClick={(e) => { e.stopPropagation(); void restoreMany([rc.id]); }}
                    onOverlayEnter={onCardActionOverlayEnter}
                    onOverlayLeave={scheduleCardActionClose}
                  />
                </div>
                {/if}
              {/each}
            </div>
            {/if}
          {/snippet}
        </VList>
      </div>
      {#if assetPageCatalog.page}
        {#if assetPageCatalog.page.items.length === 0}
          <p class="empty">No items yet — add some through asterism-server or the add_asset command.</p>
        {:else if filteredItems.length === 0}
          <p class="empty">No cards match the current content filter — clear a chip above to widen the view.</p>
        {/if}
      {/if}
    {/if}

    <ConstellationBurst
      {burst}
      onClose={closeBurst}
      onOpenAsset={(id) => void openDetail(id)}
      onPanelEnter={cancelBurstClose}
      onPanelLeave={scheduleBurstClose}
    />
  </main>

  {#if readerOpen}
    <!-- Session Reader — the drilled-in session rendered as one
         chronological transcript with full message bodies. -->
    <div
      class="detail-backdrop"
      onclick={closeReader}
      role="button"
      tabindex="-1"
      aria-label="Close reader"
    >
      <div class="reader-panel" onclick={(e) => e.stopPropagation()} role="dialog">
        <button class="detail-close" onclick={closeReader} aria-label="Close">✕</button>
        <h3 class="reader-title">
          {activeFilter.activeSessionLabel ?? "session"}
          <span class="reader-count">· {readerItems.length} message(s)</span>
          <span class="reader-mode-strip">
            {#each ["md", "raw", "html", "term"] as mode (mode)}
              <button
                class="reader-mode-chip"
                class:active={readerMode === mode}
                onclick={() => (readerMode = mode as DetailMode)}
              >
                {mode}
              </button>
            {/each}
          </span>
        </h3>
        {#if readerLoading}
          <p class="detail-loading">loading…</p>
        {:else}
          <div class="reader-scroll">
            {#each readerItems as card (card.id)}
              <article class="reader-msg reader-msg-{cardRole(card)}">
                <header class="reader-meta">
                  <span class="reader-role">{cardRole(card)}</span>
                  <span class="reader-time">{fmtDateTime(card.occurred_at_ms)}</span>
                </header>
                {#if cardIsVisual(card)}
                  <img
                    class="reader-image"
                    src={thumbCatalog.thumbSrc(card)}
                    alt={card.cover ?? ""}
                    onerror={(e) => thumbCatalog.noteOriginalError(card.id, e)}
                  />
                {:else if readerMode === "md"}
                  <div class="reader-md">
                    <!-- eslint-disable-next-line svelte/no-at-html-tags — sanitized above -->
                    {@html renderMarkdown(readerTexts.get(card.id) ?? card.cover ?? "(no text)")}
                  </div>
                {:else if readerMode === "html"}
                  <!-- svelte-ignore a11y_missing_attribute -->
                  <iframe
                    class="reader-html"
                    sandbox="allow-same-origin"
                    srcdoc={readerTexts.get(card.id) ?? card.cover ?? ""}
                  ></iframe>
                {:else if readerMode === "term"}
                  <pre class="reader-term">{readerTexts.get(card.id) ?? card.cover ?? "(no text)"}</pre>
                {:else}
                  <pre>{readerTexts.get(card.id) ?? card.cover ?? "(no text)"}</pre>
                {/if}
              </article>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}

  <DetailPane
    bind:this={detailPaneRef}
    {openAssetId}
    onClose={closeDetail}
    onOpenAsset={(id) => (openAssetId = id)}
    onSetStatus={(msg) => (status = msg)}
    onSaveLabels={saveLabels}
    onSetAsWallpaper={setAsWallpaper}
    onRefreshCounts={() => { void loadSidebarCounts(); void loadTagCounts(); }}
  />
</div>

<!--
  The floating "N selected · Right-click for actions · Clear" pill
  was retired. A bare card click commits to that card (drops the
  selection and opens detail); background click / Escape clear to
  zero (see `onCardClick` and the marquee mouse-up handler). The
  right-click menu's header still surfaces the "N selected" count
  when a multi-select reaches for bulk actions.
-->

{#if marqueeActive && marqueeRect}
  <!-- Rubber-band selection overlay. Fixed to
       the viewport so its coords match the getBoundingClientRect
       intersection math; pointer-events off so it never eats the
       mousemove/up stream. -->
  <div
    class="marquee-rect"
    style="left: {marqueeRect.left}px; top: {marqueeRect.top}px; width: {marqueeRect.width}px; height: {marqueeRect.height}px;"
  ></div>
{/if}

{#if cardDrag.active}
  <!-- Drag ghost. HTML5 DnD drew this for us; on the pointer path the
       cursor is unchanged, so without a follower there is no sign that
       anything is being carried. `pointer-events: none` keeps it out of
       `elementFromPoint`, which would otherwise only ever find the
       ghost and never the row underneath it. -->
  {@const carried = gridSelection.selectedIds.has(cardDrag.sourceOf("card") ?? "")
    ? gridSelection.selectedIds.size
    : 1}
  <div
    class="drag-ghost"
    style="left: {cardDrag.x}px; top: {cardDrag.y}px;"
  >
    {carried} card{carried === 1 ? "" : "s"}
  </div>
{/if}

<DispatchToast />

<!-- The way back from the gesture that just happened, as opposed to
     the status line's record that it did. Mounted beside the other
     leaf overlays; `trashFromCard` is its only arming site. -->
<UndoToast />

<DispatchHistoryPanel />

<!-- The lines a team hosts, kept apart from the local ones because
     they come from somewhere else (#148 decision 16). Store-gated and
     0-prop, like the drawer above it. -->
<SharedLinesPanel />

<ForgePanel />

<SnapshotView
  onPromptName={(title, placeholder) => customPrompt(title, placeholder, "")}
  onFlash={(msg, ms) => dispatchCatalog.flash(msg, ms)}
  onLoadGroupCounts={() => void loadGroupCounts()}
/>

<PromptModal />

<!-- The yes/no sibling of PromptModal, and for the same reason: the
     native `window.confirm` is not reliably shown inside Tauri v2's
     macOS WKWebView, so a guard built on it answers "no" in silence.
     Mounted once here; callers reach it through `confirmCatalog.open`
     from anywhere in the tree (App's purge path, ThreadDrawer's
     delete). -->
<ConfirmModal />

<!--
  W5a: SavedQueryDetailModal was retired (the SavedQuery concept was
  absorbed into Query Groups in W3b). The Query Group entry in the
  sidebar carries kind='query'; the future rule-edit affordance lands
  through the Group context menu in W5c.
-->


{#if sessionCommentsHover !== null}
  <SessionCommentsHover
    sessionId={sessionCommentsHover.sessionId}
    x={sessionCommentsHover.x}
    y={sessionCommentsHover.y}
    onClose={() => (sessionCommentsHover = null)}
  />
{/if}

{#if cardNoteHover !== null}
  <div
    class="card-thread-overlay card-note-overlay"
    style="left: {cardNoteHover.x}px; top: {cardNoteHover.y}px;"
    onmouseenter={onCardActionOverlayEnter}
    onmouseleave={scheduleCardActionClose}
    role="dialog"
    aria-label="Note"
  >
    <div class="card-thread-head">Note</div>
    <div class="card-thread-compose" style="border-top: none;">
      <textarea
        class="card-thread-input"
        placeholder="Short annotation (register-note)…"
        value={cardNoteHover.draft}
        oninput={(e) => cardNoteHover && (cardNoteHover = { ...cardNoteHover, draft: (e.currentTarget as HTMLTextAreaElement).value })}
        onblur={saveCardNote}
        onkeydown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void saveCardNote();
          }
        }}
      ></textarea>
      <button
        type="button"
        class="card-thread-post-btn"
        onclick={saveCardNote}
        disabled={cardNoteHover.saving}
      >
        {cardNoteHover.saving ? "saving…" : "Save"}
      </button>
    </div>
  </div>
{/if}

<!-- App-level Threads drawer. Reload orchestration:
     opening the drawer triggers a first-time load; the poll cadence
     picks up HTTP-authored writes (Claude Code / agents) between
     opens. P2 replaces the poll with an SSE subscription; this
     wiring stays. -->
<ThreadDrawer open={threadDrawerOpen} onClose={closeThreadDrawer} />

{#if cardThreadHover !== null}
  <!-- Card thread hover overlay — a compact floating panel that
       shows the target Asset's thread and lets the User quick-post
       without opening the full detail. Positioned to the right of
       the hovered card; hopping onto the panel keeps it open. -->
  <div
    class="card-thread-overlay"
    style="left: {cardThreadHover.x}px; top: {cardThreadHover.y}px;"
    onmouseenter={onCardActionOverlayEnter}
    onmouseleave={scheduleCardActionClose}
    role="dialog"
    aria-label="Thread"
  >
    <div class="card-thread-head">Thread</div>
    <ul class="card-thread-list">
      {#each cardThreadHover.comments as c (c.id)}
        <li class="card-thread-post" class:persona={c.author_kind === "persona"}>
          {#if c.author_kind === "persona"}
            {@const av = profileCatalog.personaAvatarUrl(c.author_persona_id)}
            {#if av}
              <img class="card-thread-avatar" src={av} alt="" />
            {:else}
              <span class="card-thread-avatar-placeholder">○</span>
            {/if}
          {:else}
            <span class="card-thread-avatar-placeholder user">You</span>
          {/if}
          <span class="card-thread-author">
            {noteAuthorLabel(c.author_kind, c.author_persona_id)}
          </span>
          <span class="card-thread-body">{c.body}</span>
        </li>
      {/each}
      {#if cardThreadHover.comments.length === 0}
        <li class="card-thread-empty">No comments yet.</li>
      {/if}
    </ul>
    <div class="card-thread-compose">
      <div class="card-thread-toggle">
        <button
          type="button"
          class:active={cardThreadHover.authorKind === "user"}
          onclick={() => cardThreadHover && (cardThreadHover = { ...cardThreadHover, authorKind: "user" })}
        >
          <span class="card-thread-toggle-avatar user">You</span>
        </button>
        <button
          type="button"
          class:active={cardThreadHover.authorKind === "persona"}
          disabled={activeFilter.activePersona === null}
          onclick={() => cardThreadHover && (cardThreadHover = { ...cardThreadHover, authorKind: "persona" })}
          title={activeFilter.activePersona ? `as ${personaName(activeFilter.activePersona)}` : "Pick a persona in the sidebar first"}
        >
          {#if profileCatalog.personaAvatarUrl(activeFilter.activePersona)}
            <img class="card-thread-toggle-avatar-img" src={profileCatalog.personaAvatarUrl(activeFilter.activePersona) ?? ""} alt="" />
          {:else}
            <span class="card-thread-toggle-avatar">○</span>
          {/if}
          <span>{activeFilter.activePersona ? personaName(activeFilter.activePersona) : "Persona"}</span>
        </button>
      </div>
      <textarea
        class="card-thread-input"
        placeholder="Quick note…"
        value={cardThreadHover.draft}
        oninput={(e) => cardThreadHover && (cardThreadHover = { ...cardThreadHover, draft: (e.currentTarget as HTMLTextAreaElement).value })}
        onkeydown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void postCardThreadDraft();
          }
        }}
      ></textarea>
      <button
        type="button"
        class="card-thread-post-btn"
        onclick={postCardThreadDraft}
        disabled={cardThreadHover.posting || !cardThreadHover.draft.trim()}
      >
        {cardThreadHover.posting ? "posting…" : "Post"}
      </button>
    </div>
  </div>
{/if}

<svelte:window onkeydown={onWindowKeydown} onclick={() => cardMenu && closeCardMenu()} />

{#if settingsOpen}
  <div class="settings-backdrop"
       onclick={() => setSettingsOpen(false)}
       role="button" tabindex="-1" aria-label="Close settings">
    <div class="settings-panel" onclick={(e) => e.stopPropagation()} role="dialog">
      <header class="settings-header">
        <h3>Settings</h3>
        <button class="settings-close"
                onclick={() => setSettingsOpen(false)}
                aria-label="Close">✕</button>
      </header>
      <div class="settings-body">
        <h4>Keyboard shortcuts</h4>
        <table class="shortcut-table">
          <thead>
            <tr><th>Scope</th><th>Keys</th><th>Action</th></tr>
          </thead>
          <tbody>
            {#each SHORTCUTS as sh (sh.keys + sh.scope)}
              <tr>
                <td class="scope">{sh.scope}</td>
                <td class="keys">{sh.keys}</td>
                <td>{sh.label}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <SettingsModalities />
        <!-- Every registry key, including `import.auto_organize` which
             used to have a bespoke checkbox here. One list beats a
             hand-maintained control per key: a new setting gets its
             control from the backend alone. (A key that only takes
             effect at startup still needs adding to `STARTUP_ONLY` in
             the component for its badge.) -->
        <SettingsPreferences />
        <SettingsModel />
        <SettingsMaintenance />
      </div>
    </div>
  </div>
{/if}

<QuickLook
  card={quickLookCard}
  text={quickLookText}
  textLoading={quickLookTextLoading}
  onClose={closeQuickLook}
  onOpenDetail={(id) => openDetail(id)}
/>

{#if duplicatesOpen}
  <DuplicatesPanel
    onClose={() => (duplicatesOpen = false)}
    onResolved={(trashedIds) => {
      // Rows just left the live set: the grid and the sidebar tallies
      // both describe a corpus that changed underneath them, and a
      // selected row that is now in the trash would send a dead id on
      // the next bulk action (same rule as restore / purge above).
      for (const id of trashedIds) gridSelection.selectedIds.delete(id);
      void loadAssets();
      void loadSidebarCounts();
    }}
  />
{/if}

{#if profileCard !== null}
  <ProfileCard
    card={profileCard}
    onClose={closeProfileCard}
    onEnter={onProfileCardEnter}
    onSetStatus={(msg) => (status = msg)}
  />
{/if}

{#if cardMenu}
  <div class="card-menu"
       style={`left: ${cardMenu.x}px; top: ${cardMenu.y}px;`}
       onclick={(e) => e.stopPropagation()}
       role="menu">
    <!-- Selection actions (W5-e): every entry acts on the
         selection, which openCardMenu guarantees contains the
         clicked card (W2 retarget). INVARIANT (load-bearing): the
         selection stays N≥1 while this menu is open — nothing
         clears it programmatically (Escape closes the menu first
         in the ladder, bulk handlers close before clearing). If a
         future $effect ever clears selection on e.g. filter change,
         the size===0 guards inside each bulk handler are the only
         net. The count header only shows for a true multi-select;
         single-select menus add the per-card reflex actions below
         instead. -->
    {#if gridSelection.selectedIds.size > 1}
      <div class="card-menu-head">
        {gridSelection.selectedIds.size} selected
      </div>
    {/if}
    {#if assetPageCatalog.pageIsTrash}
      <!-- Trash-side menu. Only two things can be done to a row that
           is already in the trash, and every live-side entry (Copy to…
           / Inbox / Modality / Tag / Group / wallpaper / avatar) would
           be filing something the user has just thrown out. Offering
           them here was an oversight, not a feature: a menu whose
           entries do not apply to the surface it opened on teaches the
           user to stop reading it.

           Restore first, Delete Forever last, separated — the
           irreversible entry is the one furthest from where the menu
           opens (HIG). -->
      <!-- The id is read *before* `closeCardMenu()`, here and in every
           entry below: closing nulls `cardMenu`, and an argument
           evaluated after the close would dereference null. -->
      <button class="card-menu-item"
              onclick={() => { const id = cardMenu!.card.id; closeCardMenu(); void restoreFromCard(id); }}>
        <!-- Material `restore_from_trash` (a trash can with an up
             arrow), same standard-glyph treatment as Delete Forever
             below. -->
        <svg class="card-menu-glyph" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M19 4h-3.5l-1-1h-5l-1 1H5v2h14V4zM6 7v12c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6zm8 7v4h-4v-4H8l4-4 4 4h-2z"/></svg>
        {gridSelection.selectedIds.size > 1
          ? `Restore ${gridSelection.selectedIds.size}`
          : "Restore"}
      </button>
      <hr class="card-menu-sep" />
      <button class="card-menu-item card-menu-item-danger"
              onclick={() => { const id = cardMenu!.card.id; closeCardMenu(); void purgeFromCard(id); }}>
        <!-- Material `delete_forever` — the established glyph for
             permanent deletion (a trash can with an X). No emoji says
             this, so it is the one menu glyph drawn as an inline SVG;
             `currentColor` keeps it in the destructive tone. -->
        <svg class="card-menu-glyph" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zm2.46-7.12l1.41-1.41L12 12.59l2.12-2.12 1.41 1.41L13.41 14l2.12 2.12-1.41 1.41L12 15.41l-2.12 2.12-1.41-1.41L10.59 14l-2.13-2.12zM15.5 4l-1-1h-5l-1 1H5v2h14V4z"/></svg>
        {gridSelection.selectedIds.size > 1
          ? `Delete ${gridSelection.selectedIds.size} Forever`
          : "Delete Forever"}
      </button>
    {:else}
    {#if exporterSlugs.includes("file")}
        <button class="card-menu-item"
                disabled={dispatchCatalog.pendingId !== null}
                onclick={() => { closeCardMenu(); void copySelectionTo(); }}>
          ⇩ Copy to…
        </button>
      {/if}
      <button class="card-menu-item"
              disabled={bulkBusy}
              onclick={() => { closeCardMenu(); void bulkRemoveFromInbox(); }}>
        ▽ Remove from Inbox
      </button>
      <button class="card-menu-item"
              disabled={bulkBusy}
              onclick={() => { bulkModalityOpen = !bulkModalityOpen; bulkTagOpen = false; bulkGroupOpen = false; }}>
        {bulkModalityOpen ? "▾" : "▸"} Move to Modality…
      </button>
      {#if bulkModalityOpen}
        <div class="card-menu-sub" role="menu">
          <!-- Registered rows only. `visible` also carries the
               Unclassified bucket and any unregistered slug an importer
               wrote, and neither is a destination: the bucket's key is
               a sentinel the `Modality` newtype rejects, so picking it
               failed validation instead of moving anything. -->
          {#each modalityCatalog.all.filter((m) => !m.hidden) as row (row.slug)}
            <button type="button"
                    class="card-menu-item"
                    role="menuitem"
                    disabled={bulkBusy}
                    onclick={() => { closeCardMenu(); void bulkMoveModality(row.slug); }}>
              {row.label}
            </button>
          {/each}
        </div>
      {/if}
      <button class="card-menu-item"
              disabled={bulkBusy}
              onclick={() => { bulkTagOpen = !bulkTagOpen; bulkModalityOpen = false; bulkGroupOpen = false; }}>
        {bulkTagOpen ? "▾" : "▸"} Tag…
      </button>
      {#if bulkTagOpen}
        <div class="card-menu-sub" role="menu">
          <div class="card-menu-tag-add">
            <input class="card-menu-tag-input"
                   type="text"
                   placeholder="new / existing tag"
                   bind:value={bulkTagInput}
                   onkeydown={(e) => {
                     if (e.key === "Enter") {
                       e.preventDefault();
                       void bulkAttachTag();
                     }
                   }} />
            <button type="button"
                    class="card-menu-item card-menu-tag-addbtn"
                    onclick={() => void bulkAttachTag()}
                    disabled={bulkBusy || bulkTagInput.trim().length === 0}>
              Add
            </button>
          </div>
          {#if tagCatalog.counts.data.length > 0}
            <div class="card-menu-sub-head">Remove tag</div>
            <div class="card-menu-tag-list">
              {#each tagCatalog.counts.data as tc (tc.tag.id)}
                <button type="button"
                        class="card-menu-item"
                        role="menuitem"
                        disabled={bulkBusy}
                        title="Remove #{tc.tag.name} from the selected assets"
                        onclick={() => void bulkDetachTag(tc.tag.id, tc.tag.name)}>
                  # {tc.tag.name} ✕
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      <button class="card-menu-item"
              disabled={bulkBusy}
              onclick={() => { bulkGroupOpen = !bulkGroupOpen; bulkModalityOpen = false; bulkTagOpen = false; }}>
        {bulkGroupOpen ? "▾" : "▸"} Group…
      </button>
      {#if bulkGroupOpen}
        {@const inGroups = selectionGroupIds()}
        <div class="card-menu-sub" role="menu">
          <div class="card-menu-sub-head">Add to group</div>
          {#each bulkManualGroups as g (g.group.id)}
            <button type="button"
                    class="card-menu-item"
                    role="menuitem"
                    disabled={bulkBusy}
                    title="Add the selected assets to ▤ {g.group.name}"
                    onclick={() => void bulkGroupMembershipOp(g.group.id, g.group.name, "attach")}>
              ▤ {g.group.name}
            </button>
          {/each}
          {#if bulkManualGroups.length === 0}
            <div class="card-menu-sub-head">no manual groups yet</div>
          {/if}
          {#if bulkManualGroups.some((g) => inGroups.has(g.group.id))}
            <div class="card-menu-sub-head">Remove from group</div>
            {#each bulkManualGroups.filter((g) => inGroups.has(g.group.id)) as g (g.group.id)}
              <button type="button"
                      class="card-menu-item"
                      role="menuitem"
                      disabled={bulkBusy}
                      title="Remove the selected assets from ▤ {g.group.name}"
                      onclick={() => void bulkGroupMembershipOp(g.group.id, g.group.name, "detach")}>
                ▤ {g.group.name} ✕
              </button>
            {/each}
          {/if}
        </div>
      {/if}
      <button class="card-menu-item"
              onclick={() => contextPromoteSelection()}>
        ▤ Group-ify selection ({gridSelection.selectedIds.size})
      </button>
    {#if gridSelection.selectedIds.size === 1}
      <!-- Per-card reflex actions — appended only when the menu
           targets a single card, so multi-select menus stay
           single-audience (one menu, one target). -->
      {#if cardIsImage(cardMenu.card)}
        <button class="card-menu-item"
                onclick={() => contextSetWallpaper(cardMenu!.card)}>
          ▨ Set as {personaName(cardMenu.card.persona_id)} wallpaper
        </button>
        <button class="card-menu-item"
                onclick={() => contextSetAvatar(cardMenu!.card)}>
          ◉ Set as {personaName(cardMenu.card.persona_id)} avatar
        </button>
      {/if}
      <button class="card-menu-item"
              onclick={() => contextCopyLocator(cardMenu!.card)}>
        ⌘ Copy source path
      </button>
    {/if}
    <!-- Removal, last and separated. This is where a destructive
         action belongs: a menu is opened deliberately, the entry is
         read before it is clicked, the separator and the tone say what
         kind of entry it is, and the distance from the menu's top edge
         means a mis-aimed click lands on something harmless. Every one
         of those properties is missing from a hover strip, which is
         where this action used to be; none of the eight library apps
         surveyed puts removal there either.

         Reversible, so no confirm — the trash view is the undo, and
         `Move to Trash` says where the card is going rather than that
         it is gone. The irreversible sibling is on the other side of
         the trash toggle, above, and does confirm. -->
    <hr class="card-menu-sep" />
    <button class="card-menu-item card-menu-item-danger"
            onclick={() => { const id = cardMenu!.card.id; closeCardMenu(); void trashFromCard(id); }}>
      🗑 {gridSelection.selectedIds.size > 1
        ? `Move ${gridSelection.selectedIds.size} to Trash`
        : "Move to Trash"}
    </button>
    {/if}
  </div>
{/if}

{#if dropOverlay}
  <div class="drop-overlay" aria-hidden="true">
    <div class="drop-overlay-inner">
      <div class="drop-overlay-icon">▨</div>
      <div class="drop-overlay-title">
        {activeFilter.activePersona === null
          ? "Pick a persona first, then drop"
          : `Drop to import into ${personaName(activeFilter.activePersona)}`}
      </div>
      <div class="drop-overlay-sub">
        image / video / audio / files land here
      </div>
    </div>
  </div>
{/if}

{#if pendingDropPaths.length > 0}
  <!-- Persona picker modal that appears when a drop lands without
       an active persona. Buffers the paths and flushes on choice
       so the drop is not lost. Cancel discards the paths. -->
  <div class="drop-picker-backdrop" onclick={() => (pendingDropPaths = [])}
       role="button" tabindex="-1" aria-label="Cancel import">
    <div class="drop-picker-panel" onclick={(e) => e.stopPropagation()} role="dialog">
      <h3 class="drop-picker-title">
        Which persona should {pendingDropPaths.length} file{pendingDropPaths.length === 1 ? "" : "s"} land in?
      </h3>
      <p class="drop-picker-sub">
        A persona is needed to file the drop. Pick one below or cancel to discard.
      </p>
      <ul class="drop-picker-personas">
        {#each personaCatalog.list.data as p (p.id)}
          <li>
            <button
              type="button"
              class="drop-picker-persona-btn"
              onclick={() => {
                const paths = pendingDropPaths;
                void runDropImport(paths, p.id);
              }}
            >
              ○ {p.name}
            </button>
          </li>
        {/each}
      </ul>
      <div class="drop-picker-actions">
        <button
          type="button"
          class="drop-picker-cancel"
          onclick={() => (pendingDropPaths = [])}
        >Cancel</button>
      </div>
    </div>
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    background: #f7f7f5;
    color: #1a1a1a;
  }

  .layout {
    display: grid;
    grid-template-columns: 180px 1fr;
    /* Pin the outer frame to the viewport so the sidebar + mode
       toggle stay in place while the grid scrolls internally
       (Eagle-style: fixed chrome, scroll only where the data
       lives). Individual scroll regions manage their own overflow. */
    height: 100vh;
    overflow: hidden;
  }

  .sidebar {
    border-right: 1px solid #e2e2de;
    padding: 1rem;
    background: #fbfbf9;
    /* Sidebar owns its scroll — some persona lists get long, and
       we still want the persona picker reachable without dragging
       the whole page. */
    overflow-y: auto;
    height: 100vh;
  }

  /* Thin banner across the top when the SessionRebuild job is
     running. Absolute-positioned so it doesn't push the layout down,
     with an indeterminate spinner instead of a numeric bar (the job
     is a one-shot full rebuild, not incremental). */
  .rebuild-banner {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.75rem;
    font-size: 0.75rem;
    color: #6a5c3a;
    background: linear-gradient(90deg, #fef7d6 0%, #fdf0b4 50%, #fef7d6 100%);
    background-size: 200% 100%;
    animation: rebuild-shimmer 1.6s ease-in-out infinite;
    border-bottom: 1px solid #efe0a0;
    pointer-events: none;
  }
  .rebuild-spinner {
    display: inline-block;
    width: 0.7rem;
    height: 0.7rem;
    border: 2px solid #d4c383;
    border-top-color: transparent;
    border-radius: 50%;
    animation: rebuild-spin 0.9s linear infinite;
  }
  @keyframes rebuild-shimmer {
    0%,
    100% {
      background-position: 0% 50%;
    }
    50% {
      background-position: 100% 50%;
    }
  }
  @keyframes rebuild-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .sidebar h1 {
    font-size: 1.2rem;
    font-weight: 500;
    margin: 0 0 0.25rem;
  }

  .profile-badge {
    display: inline-block;
    margin-left: 0.35rem;
    padding: 0.08rem 0.32rem;
    border-radius: 4px;
    color: #fff;
    background: #c04a52;
    font-size: 0.58rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    line-height: 1.2;
    text-transform: uppercase;
    vertical-align: middle;
  }

  .profile-badge.bench {
    background: #7560b4;
  }

  .status {
    color: #999;
    font-size: 0.7rem;
    margin: 0 0 1rem;
  }

  .search-wrap {
    position: relative;
    margin-bottom: 0.5rem;
  }

  .search {
    width: 100%;
    padding: 0.35rem 1.5rem 0.35rem 0.5rem;
    font-size: 0.8rem;
    border: 1px solid #d8d8d0;
    border-radius: 5px;
    background: #fff;
    box-sizing: border-box;
  }

  .search:focus {
    outline: none;
    border-color: #8a86ff;
  }

  .search-clear {
    position: absolute;
    right: 0.25rem;
    top: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.4rem;
    padding: 0;
    background: none;
    border: none;
    font-size: 0.75rem;
    color: #999;
    cursor: pointer;
    line-height: 1;
  }

  .search-clear:hover {
    color: #444;
  }

  /* Active-filter chips band in the sidebar. Wraps flexibly so a
     handful of tags stack cleanly without pushing the Persona
     list off-screen. */
  .active-filters-band {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin: 0 0 0.5rem;
  }

  /* Section header for the active-filter band — its `↺` reset
     control lives on the right so a single click clears every
     axis, replacing the old standalone "reset all filters" line. */
  .active-filters-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.4rem;
  }

  .reset-icon {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 0 0.25rem;
    font-size: 0.85rem;
    line-height: 1.1;
    color: #7a76c9;
    cursor: pointer;
    /* Undo the sidebar `<h2>` uppercase transform so the icon
       glyph paints as itself instead of a hollow arrow. */
    text-transform: none;
    letter-spacing: 0;
  }

  .reset-icon:hover {
    background: #ecebfa;
    border-color: #d9d5f2;
  }

  .reset-all {
    display: block;
    width: 100%;
    background: none;
    border: 1px solid transparent;
    padding: 0.2rem 0.35rem;
    font-size: 0.7rem;
    color: #9c9a89;
    cursor: pointer;
    text-align: left;
    margin-bottom: 0.5rem;
    border-radius: 4px;
  }

  .reset-all:hover {
    color: #444;
    background: #f2f0ea;
  }

  .count-hint {
    color: #8a86ff;
    margin-left: 0.4rem;
    font-style: italic;
  }

  /* Sits in the count line with the hints, so it borrows their tone
     (same ink, same italic) and reads as a link rather than a control
     competing with the grid. */
  .count-escape {
    margin-left: 0.35rem;
    padding: 0;
    background: none;
    border: none;
    font: inherit;
    font-style: italic;
    color: #7a76c9;
    text-decoration: underline;
    text-underline-offset: 0.15em;
    cursor: pointer;
  }

  .count-escape:hover {
    color: #5a55b2;
  }

  /*
   * Card thumbnail placeholder while viewport hydration is in
   * flight — matches `.thumb` geometry (aspect-ratio 4/3 via the
   * inherited container) with a subdued fill so the grid does
   * not jump when the real image lands.
   */
  .thumb-placeholder {
    width: 100%;
    height: 100%;
    background: repeating-linear-gradient(
      45deg,
      #ecebe4,
      #ecebe4 4px,
      #e5e3da 4px,
      #e5e3da 8px
    );
    border-radius: 4px;
  }

  .sidebar h2 {
    font-size: 0.75rem;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 1rem 0 0.25rem;
  }

  .sidebar ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  /* Drop target while a card is over the row — same affordance as the
     Modality / Groups sections, so every sidebar drop reads alike. */
  .sidebar li.drop-target {
    outline: 2px dashed #b5b1e2;
    outline-offset: -2px;
    border-radius: 4px;
    background: #f2f1fb;
  }

  .sidebar button {
    background: none;
    border: none;
    padding: 0.2rem 0.3rem;
    font-size: 0.85rem;
    color: #555;
    cursor: pointer;
    width: 100%;
    text-align: left;
    border-radius: 4px;
  }

  .sidebar button:hover {
    background: #efefe9;
  }

  .sidebar button.active {
    color: #111;
    font-weight: 600;
    background: #eceae2;
  }

  /* `.sidebar-count` + `.sidebar button.active .sidebar-count` moved
     into SidebarSearch / PersonaStrip / ModalityList — no App-side
     template uses left after Phase C wave 4. Rules will resurrect if
     the remaining sidebar sections (Tags / Groups / Sessions / Saved
     Queries) start carrying count badges again. */

  .content {
    /* Flex column: mode toggle + count hint hold their natural
       height at the top, `.grid-wrapper` flexes to fill the rest,
       and VList handles the internal scroll. `min-height: 0` is
       required so the flex child can shrink below its content. */
    display: flex;
    flex-direction: column;
    height: 100vh;
    padding: 1rem 1.25rem 0;
    min-height: 0;
    overflow: hidden;
  }

  .count {
    color: #999;
    font-size: 0.75rem;
  }

  /* Trash toolbar — a band between the count line and the grid, on the
     trash side only. Bordered rather than floating: it is part of the
     page the user is reading, not an overlay over it, and the two
     destructive entries in it should never read as something that
     drifted on top of the grid. */
  .trash-toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.35rem 0 0.6rem;
    padding: 0.4rem 0.5rem;
    border: 1px solid #e3e1f2;
    border-radius: 6px;
    background: #faf9ff;
  }
  /* Pushes `Empty Trash` to the far edge: it acts on the whole trash
     rather than on the selection the other two read, and putting a
     gap between them is the cheapest way to say so. */
  .trash-toolbar-gap {
    flex: 1;
  }
  .trash-toolbar-btn {
    padding: 0.3rem 0.75rem;
    border: 1px solid #ccc;
    border-radius: 5px;
    background: #ffffff;
    color: #3a3856;
    font-family: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .trash-toolbar-btn:hover:not(:disabled) {
    background: #f2f2f6;
  }
  /* Disabled, not hidden: the buttons say what the toolbar can do, and
     a control that vanishes when it cannot run teaches nothing about
     what it needs (here: a selection). */
  .trash-toolbar-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  /* Same warning tone the confirm modal and the menu's removal tier
     use — a destructive action is marked before it is taken. */
  .trash-toolbar-danger {
    color: #c0392b;
    border-color: #e0b4ae;
  }
  .trash-toolbar-danger:hover:not(:disabled) {
    background: #fdf1ef;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 0.6rem;
    content-visibility: auto;
  }

  /* Virtualised grid: the wrapper fills the vertical space so the
     `VList` inside can measure its own viewport, and each row is an
     inner CSS grid so cards still land on aligned columns. The
     column count itself is driven from the wrapper's measured
     width in the script (see `gridCols`). Height is pinned to
     roughly the viewport minus the mode toggle + content padding
     so VList has a bounded scroll area to virtualise inside; a
     precise fit is unnecessary — anything within a few dozen
     pixels shows the same first-paint row set. */
  .grid-wrapper {
    flex: 1;
    min-height: 0;
  }

  /* While a rubber-band drag is live, kill native text selection +
     hint the crosshair so the sweep reads as a selection gesture. */
  .grid-wrapper.marquee-active {
    user-select: none;
    cursor: crosshair;
  }

  /* Rubber-band selection rectangle — subtle translucent fill + accent
     border in the app's indigo. Viewport-fixed; never intercepts
     pointer events. */
  .marquee-rect {
    position: fixed;
    z-index: 35;
    pointer-events: none;
    background: rgba(88, 80, 255, 0.12);
    border: 1px solid rgba(88, 80, 255, 0.55);
    border-radius: 2px;
  }

  /* Follows the cursor while a card is in flight. Offset a little so
     it never sits directly under the hotspot — `elementFromPoint` has
     to find the row below, and a label centred on the pointer reads as
     if it were the thing being pointed at. */
  .drag-ghost {
    position: fixed;
    z-index: 40;
    pointer-events: none;
    transform: translate(12px, 12px);
    padding: 0.15rem 0.45rem;
    background: #5850ff;
    color: #fff;
    border-radius: 4px;
    font-size: 0.7rem;
    font-variant-numeric: tabular-nums;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.2);
  }

  .grid-row {
    display: grid;
    gap: 0.6rem;
    padding-bottom: 0.6rem;
  }

  /* Sessions view scroll container. `.content` is a fixed-height
     flex column, so anything past the visible area disappears
     unless we mount an explicit vertical-scroll wrapper here.
     Padding-bottom mirrors the old `.content` bottom padding so
     the last row isn't glued to the window edge. */
  .sessions-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 4rem;
  }

  /* Group-boundary header inside the virtualised grid. Kept
     lightweight (1 line, muted purple) so it separates buckets
     without competing with the cards visually — Lightroom's
     segment header treatment adapted for the Asterism palette. */
  .grid-group-header {
    font-size: 0.78rem;
    color: #7a76c9;
    font-weight: 600;
    margin: 0.3rem 0 0.35rem;
    padding: 0.15rem 0.05rem;
    border-bottom: 1px solid #ecebfa;
  }
  /* The time band a run of buckets falls into, under the `Recent`
     order. It has to outrank the bucket header it introduces, or the
     two read as siblings and the nesting is lost — hence the larger
     type, the darker ink, and the solid rule against the bucket
     header's hairline. The extra top margin is the gap that makes the
     grouping visible without an indent (indenting the minor header
     would misalign it with the card grid it captions). */
  .grid-group-header.major {
    font-size: 0.92rem;
    color: #4b47a8;
    font-weight: 700;
    letter-spacing: 0.01em;
    margin: 1.1rem 0 0.15rem;
    padding-bottom: 0.28rem;
    border-bottom: 2px solid #cfcbee;
  }
  /* No leading gap when a major header opens the list — the grid's own
     top padding already provides it. */
  .grid-group-header.major:first-child {
    margin-top: 0.2rem;
  }

  .card {
    background: #fff;
    border: 1px solid #e6e6e2;
    border-radius: 8px;
    padding: 0.6rem;
    min-height: 90px;
    transition: border-color 0.1s, transform 0.1s;
  }

  .card.hovered {
    border-color: #8a86ff;
    transform: translateY(-1px);
  }

  /* Selector: cards in the current grid multi-select carry a strong
     colored ring so the pick reads at a glance regardless of how far
     the user has scrolled from the anchor. The ring stacks with
     `.hovered` — `box-shadow` composes cleanly. */
  .card.selected {
    border-color: #5850ff;
    box-shadow:
      0 0 0 2px #5850ff,
      0 2px 6px rgba(88, 80, 255, 0.25);
    background: #f5f4ff;
  }
  .card.selected.hovered {
    box-shadow:
      0 0 0 2px #5850ff,
      0 4px 10px rgba(88, 80, 255, 0.35);
  }

  /* Drag cues: any Messages-view card can be picked up (drag-mode),
     the grab cursor surfaces that affordance. Reorder-mode adds the
     drop-target edge for in-grid reorder. Dragging fades the source. */
  .grid.drag-mode .card {
    cursor: grab;
  }
  .grid.drag-mode .card:active {
    cursor: grabbing;
  }
  .card.dragging {
    opacity: 0.4;
  }
  /* WebKit starts an OS-level drag when an image (or a link) is picked
     up, and paints a translucent snapshot of it that floats over the
     window. `selectstart` covers text but not this — it is a separate
     path, and on macOS the snapshot ends up drifting across the sidebar
     mid-drag. Cards are dragged by our own pointer handler, so the
     native gesture has nothing to add. */
  .card,
  .card * {
    -webkit-user-drag: none;
  }
  .card.drop-target {
    border-left: 3px solid #7a76c9;
    padding-left: calc(0.6rem - 2px);
  }

  .card-head {
    display: flex;
    justify-content: space-between;
    margin-bottom: 0.35rem;
  }

  .badge {
    font-size: 0.65rem;
    background: #eceae2;
    border-radius: 4px;
    padding: 0.05rem 0.35rem;
    color: #666;
  }

  /* A container reads as a container at a glance — same badge shape,
     the sidebar's Grouping accent so the two surfaces agree. */
  .badge-collection {
    background: #e4e2f5;
    color: #5b56a8;
  }

  .cover-collection {
    font-weight: 600;
  }

  .collection-meta {
    margin: 0.15rem 0 0;
    font-size: 0.7rem;
    color: #b5b1e2;
    font-variant-numeric: tabular-nums;
  }

  .date {
    font-size: 0.65rem;
    color: #aaa;
  }

  .cover {
    font-size: 0.8rem;
    line-height: 1.45;
    margin: 0 0 0.4rem;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .cover-image {
    font-size: 0.7rem;
    color: #888;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    margin-top: 0.35rem;
  }

  /*
   * Search snippet variant of `.cover`. Extra line quota (5 instead
   * of 3) because the highlighted body window carries more signal
   * than the fixed cover string; a slightly smaller font keeps the
   * card footprint unchanged so mixed grid pages (search + list)
   * still tile evenly.
   */
  .cover-snippet {
    -webkit-line-clamp: 5;
    line-clamp: 5;
    font-size: 0.75rem;
    line-height: 1.4;
  }

  /* Match highlight injected by tantivy `SnippetGenerator.to_html()` — <b> tags around matched terms. */
  .cover-snippet :global(b) {
    background: #fff2a8;
    color: #333;
    font-weight: 600;
    padding: 0 1px;
    border-radius: 2px;
  }

  /*
   * BM25 rank badge — visible only in search mode. Distinct hue
   * from the `.badge` (modality slug) and `.flag-badge` (content
   * flag icons) so a quick glance separates ranking from category.
   */
  .score-badge {
    font-size: 0.6rem;
    background: #4d6cc9;
    color: #fff;
    border-radius: 3px;
    padding: 0.05rem 0.35rem;
    margin-left: 0.3rem;
    font-variant-numeric: tabular-nums;
  }
  .annot-badge {
    font-size: 0.7rem;
    margin-left: 0.2rem;
    opacity: 0.7;
    line-height: 1;
  }

  /* Clean-mode grid — reduces the card chrome to Modality + thumb +
     PersonaName + optional content name so the grid reads as a
     photo wall instead of a metadata dashboard. Toggled via the
     sidebar-header switch (persisted per user in localStorage).
     The class lives on `.content` so both the Messages grid (uses
     `.grid-wrapper`) and the Sessions grid (uses `.grid`) pick up
     the same reduction. */
  .content.clean-mode .card-head .flag-badge,
  .content.clean-mode .card-head .score-badge,
  .content.clean-mode .card-head .rating,
  .content.clean-mode .card-head .date,
  .content.clean-mode .card .palette-strip,
  .content.clean-mode .card .labels {
    /* `.session-meta` used to sit alongside these but the sessions
       grid now renders inside SessionsView.svelte (wave 6), so
       clean-mode hiding for that pill row has moved to the
       component's own scoped style block.
       The `.card-action-icons` hide-in-clean-mode rule moved to
       `CardActionIcons.svelte` with the strip's own styles when the
       icon strip was extracted. */
    display: none !important;
  }
  .content.clean-mode .card {
    padding: 0.4rem;
    background: transparent;
    border-color: transparent;
    transition: background 0.1s, border-color 0.1s;
  }
  /* Hovered card in clean mode brings the frame back so the User
     still gets the "which card am I on" affordance. */
  .content.clean-mode .card:hover {
    background: #ffffff;
    border-color: #e6e6e2;
  }
  .grid-wrapper.clean-mode .card-head {
    margin-bottom: 0.2rem;
  }
  .clean-basename {
    font-size: 0.72rem;
    color: #6a67a4;
    margin: 0.1rem 0 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Clean-mode toggle switch — sits between the app title and the
     settings gear. Compact pill so it doesn't crowd the sidebar
     header. */
  .clean-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: transparent;
    border: none;
    padding: 0 0.25rem;
    cursor: pointer;
    color: inherit;
    font-size: 0.68rem;
    margin-left: auto;
  }
  .clean-toggle-track {
    width: 24px;
    height: 12px;
    border-radius: 999px;
    background: #d6d3ec;
    position: relative;
    transition: background 0.12s;
  }
  .clean-toggle-knob {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #ffffff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
    transition: transform 0.12s;
  }
  .clean-toggle.on .clean-toggle-track {
    background: #5850ff;
  }
  .clean-toggle.on .clean-toggle-knob {
    transform: translateX(12px);
  }
  .clean-toggle-label {
    color: #6a67a4;
    letter-spacing: 0.02em;
  }
  .clean-toggle.on .clean-toggle-label {
    color: #5850ff;
    font-weight: 600;
  }

  /* Card action-icon strip — Eagle-style floating menu inside the
     card. The strip itself now lives in `CardActionIcons.svelte`,
     which owns the `.card-action-icons` /
     `.card-action-icon` / `.filled` selectors as `:global` so the
     hover cascade fires from either the Messages grid `.card` or
     the SessionsView `.card.session-tile`. The only rule that
     stays here is `.card { position: relative }`, because
     grid-card layout depends on it and this file still owns the
     grid `.card` shell.

     The `.card-note-overlay` size tweak stays because the Note
     overlay markup (positioned relative to the hovered card) still
     lives inline in this file — extracting it is a later carry. */
  .card {
    position: relative;
  }
  .card-note-overlay {
    max-height: 220px;
  }

  /* Card thread hover overlay — floating panel anchored to the
     right of the hovered card. Compact so it does not swallow the
     screen; scrolls internally when the thread is long. */
  .card-thread-overlay {
    position: fixed;
    width: 300px;
    max-height: 320px;
    background: #ffffff;
    border: 1px solid #d6d3ec;
    border-radius: 8px;
    box-shadow: 0 12px 30px rgba(23, 22, 42, 0.25);
    z-index: 55;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: #1f1e33;
  }
  .card-thread-head {
    font-size: 0.72rem;
    padding: 0.35rem 0.7rem;
    background: #f5f4ff;
    color: #6a67a4;
    font-weight: 600;
    border-bottom: 1px solid #eae7f8;
  }
  .card-thread-list {
    list-style: none;
    padding: 0.4rem 0.5rem;
    margin: 0;
    overflow-y: auto;
    flex: 1;
    min-height: 2.4rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .card-thread-post {
    border-left: 2px solid #5850ff;
    padding: 0.2rem 0.45rem;
    background: #f5f4ff;
    border-radius: 3px;
    font-size: 0.75rem;
    display: flex;
    gap: 0.4rem;
  }
  .card-thread-post.persona {
    border-left-color: #b47bff;
    background: #f8f3ff;
  }
  .card-thread-author {
    font-weight: 600;
    color: #2f2c5c;
    flex-shrink: 0;
  }
  .card-thread-avatar {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
    align-self: flex-start;
  }
  .card-thread-avatar-placeholder {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #eae7f8;
    color: #6a67a4;
    font-size: 0.55rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    flex-shrink: 0;
    align-self: flex-start;
  }
  .card-thread-avatar-placeholder.user {
    background: #5850ff;
    color: #ffffff;
  }
  .card-thread-toggle button {
    display: inline-flex;
    align-items: center;
    gap: 0.28rem;
  }
  .card-thread-toggle-avatar {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #eae7f8;
    color: #6a67a4;
    font-size: 0.5rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
  }
  .card-thread-toggle-avatar.user {
    background: #5850ff;
    color: #ffffff;
  }
  .card-thread-toggle-avatar-img {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    object-fit: cover;
  }
  .card-thread-toggle button.active .card-thread-toggle-avatar {
    background: #ffffff;
    color: #5850ff;
  }
  .card-thread-body {
    color: #1f1e33;
    white-space: pre-wrap;
    line-height: 1.3;
  }
  .card-thread-empty {
    color: #9c98c9;
    font-size: 0.72rem;
    padding: 0.3rem 0.5rem;
    text-align: center;
  }
  .card-thread-compose {
    border-top: 1px solid #eae7f8;
    padding: 0.4rem 0.55rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    background: #fafafd;
  }
  .card-thread-toggle {
    display: flex;
    gap: 0.25rem;
  }
  .card-thread-toggle button {
    padding: 0.15rem 0.55rem;
    font-size: 0.68rem;
    background: #ffffff;
    border: 1px solid #d6d3ec;
    border-radius: 3px;
    cursor: pointer;
    color: #6a67a4;
  }
  .card-thread-toggle button:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .card-thread-toggle button.active {
    background: #5850ff;
    color: #ffffff;
    border-color: #5850ff;
  }
  .card-thread-input {
    width: 100%;
    box-sizing: border-box;
    min-height: 2.2rem;
    padding: 0.35rem 0.5rem;
    font-size: 0.78rem;
    font-family: inherit;
    line-height: 1.35;
    background: #ffffff;
    border: 1px solid #d6d3ec;
    border-radius: 4px;
    outline: none;
    color: inherit;
    resize: vertical;
  }
  .card-thread-input:focus {
    border-color: #8a86ff;
  }
  .card-thread-post-btn {
    align-self: flex-end;
    padding: 0.3rem 0.9rem;
    background: #5850ff;
    color: #ffffff;
    border: none;
    border-radius: 4px;
    font-size: 0.75rem;
    cursor: pointer;
  }
  .card-thread-post-btn:hover:not(:disabled) {
    background: #4a42e0;
  }
  .card-thread-post-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Star rating widget on the card head. Buttons are compact (no
     padding) so 5 stars + badges still fit on one row on 200 px
     cards. Empty stars stay very faint until the card is hovered,
     so the widget disappears from the visual noise floor. */
  .rating {
    display: inline-flex;
    margin-left: auto;
    margin-right: 0.3rem;
    gap: 1px;
  }
  .rating-star {
    background: none;
    border: none;
    padding: 0;
    font-size: 0.9rem;
    line-height: 1;
    color: rgba(0, 0, 0, 0.08);
    cursor: pointer;
    transition: color 0.08s;
  }
  .card:hover .rating-star {
    color: rgba(0, 0, 0, 0.25);
  }
  .rating-star.filled {
    color: #f5a623;
  }
  .card:hover .rating-star.filled {
    color: #f5a623;
  }
  .rating-star:hover {
    color: #f5a623;
    transform: scale(1.15);
  }

  /* Palette strip — 5 swatches under an image thumb. Kept small so
     it does not compete with the thumb visually; grows on hover to
     surface the hex value on a tooltip title. */
  .palette-strip {
    display: flex;
    gap: 2px;
    margin-top: 0.35rem;
    height: 8px;
    border-radius: 3px;
    overflow: hidden;
  }
  .palette-swatch {
    flex: 1;
    display: block;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.04);
    transition: transform 0.08s;
  }
  .card:hover .palette-strip {
    height: 12px;
  }

  .thumb {
    width: 100%;
    aspect-ratio: 4 / 3;
    overflow: hidden;
    border-radius: 5px;
    background: #eceae2;
    margin-bottom: 0.35rem;
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .labels {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin-bottom: 0.3rem;
  }

  .label {
    font-size: 0.6rem;
    color: #7a76c9;
    background: #f0effc;
    border-radius: 3px;
    padding: 0.05rem 0.3rem;
  }
  .labels-edit {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    align-items: center;
  }

  /* Comment thread on the detail panel. Flat list, User and Persona
     posts distinguished by a colored left border + author name. */

  /* Detail-view tag chip: same look as .label but interactive. */
  .label-tag {
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .label-tag:hover {
    background: #e2ddf9;
    color: #5a55b2;
  }
  /* Active state = this chip's tag is already in the multi-tag
     filter set. Signals "clicking me is a no-op" so the user does
     not expect the chip to toggle off. */
  .label-tag-active {
    background: #7a76c9;
    color: #fff;
  }
  .label-tag-active:hover {
    background: #7a76c9;
    color: #fff;
    cursor: default;
  }

  /* Grid header active-tag chip: shows one currently applied tag
     filter with a small clear affordance. Multiple chips render
     side-by-side (AND semantic) so the user sees the whole active
     combo without a summary line. */
  .active-tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin-left: 0.3rem;
    padding: 0.1rem 0.5rem;
    font-size: 0.7rem;
    color: #7a76c9;
    background: #f0effc;
    border: 1px solid #d9d5f2;
    border-radius: 999px;
    cursor: pointer;
    font-family: inherit;
  }
  .active-tag-chip:hover {
    background: #e2ddf9;
    color: #5a55b2;
  }
  .chip-x {
    font-size: 0.6rem;
    opacity: 0.6;
  }

  /* Sidebar shared row cascade (.tags-list / .tag-name / .tag-count
     / .tags-filter) plus the standalone `.tags-toggle` /
     `.tags-empty` / `.tags-active-count` chip styles moved into
     TagList.svelte + GroupsSection.svelte along with the templates
     that use them (waves 5a / 5b-2). No App-side template still
     renders these classes, so keeping the rules here just triggered
     "unused CSS" warnings. */
  /* `.selections-list` / `.selection-row` / `.selection-name` /
     `.selection-count` moved to SelectionsList.svelte in wave 9
     (Selections graduated). `.saved-query-*` cascade left with
     SavedQueriesList / SavedQueryDetailModal in wave 7. No App-
     side template still renders these classes. */

  /* Provenance section — mini-graph rendering of 1-hop
     derived_from lineage. Two lanes ("↑ derived from" /
     "↓ derived into"), each with a horizontal strip of
     clickable chips that jump the detail-pane to the picked
     ancestor / descendant. Kept intentionally lightweight so
     the section reads as "genealogy" rather than a full graph
     canvas. */
  /* Group-specific chip in the grid header — same shape as
     .active-tag-chip but a distinct palette so users can tell
     tag-axis and group-axis filters apart at a glance. */
  .group-chip {
    background: #eef7f4;
    border-color: #c8e4d6;
    color: #4a8f78;
  }
  .group-chip:hover {
    background: #d9ede4;
    color: #2f6e5a;
  }

  /* Sidebar Groups/Dirs cascade (.group-row + variants / .group-edit /
     .group-delete / .dir-toggle / .dir-row / .dir-name / .dir-empty
     / .nest-badge / .nest-mark / .rename-input / .drop-target-root
     / .tags-toggle / .tags-empty / .tags-active-count / draggable
     override) moved into GroupsSection.svelte (wave 5b-2) along with
     the templates that use them. */

  /* Nested-collection band above the grid: one chip per child group
     of the solo-browsed collection. */
  .child-band {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0 0.6rem;
  }
  .child-band-label {
    font-size: 0.65rem;
    color: #9a96d9;
  }
  .child-chip {
    display: inline-flex;
    align-items: stretch;
    border: 1px solid #c8e4d6;
    border-radius: 999px;
    background: #eef7f4;
    overflow: hidden;
  }
  .child-chip-name {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.1rem 0.2rem 0.1rem 0.55rem;
    font-size: 0.7rem;
    color: #4a8f78;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .child-chip-name:hover {
    background: #d9ede4;
    color: #2f6e5a;
  }
  .child-chip-count {
    font-size: 0.6rem;
    color: #7ab89a;
    font-variant-numeric: tabular-nums;
  }
  .child-chip-x {
    padding: 0 0.45rem 0 0.25rem;
    font-size: 0.55rem;
    color: #9cc4b1;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .child-chip-x:hover {
    color: #d47272;
  }

  /* Content-type filter band — appears above the grid inside a
     session view, mirrors the render-session badge idea (T/M/B/…)
     with emoji glyphs so the meaning reads at a glance. */
  .content-flag-band {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    align-items: center;
    padding: 0.25rem 0.5rem 0.4rem;
    margin: -0.1rem 0 0.5rem;
    font-size: 0.7rem;
  }
  .flag-band-label {
    font-size: 0.62rem;
    color: #9a96d9;
    letter-spacing: 0.08em;
    margin-right: 0.2rem;
  }
  .flag-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.15rem 0.55rem;
    border: 1px solid #d9d5f2;
    border-radius: 999px;
    background: #fff;
    color: #4a4a4a;
    cursor: pointer;
    font-family: inherit;
    font-size: 0.7rem;
    line-height: 1;
    transition: background 0.1s ease, border-color 0.1s ease, color 0.1s ease;
  }
  .flag-chip:hover {
    border-color: #7a76c9;
    color: #2a2a2a;
  }
  .flag-chip.active {
    background: #7a76c9;
    border-color: #7a76c9;
    color: #fff;
  }
  .flag-icon {
    font-size: 0.85rem;
    line-height: 1;
  }
  .flag-name {
    font-size: 0.7rem;
  }
  .flag-count {
    font-size: 0.6rem;
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
  }
  .flag-chip.active .flag-count {
    opacity: 0.85;
  }
  .flag-clear {
    padding: 0.15rem 0.45rem;
    border: none;
    background: transparent;
    color: #9a96d9;
    cursor: pointer;
    font-family: inherit;
    font-size: 0.7rem;
  }
  .flag-clear:hover {
    color: #d47272;
  }

  /* Per-card content-flag badges — small emoji chips lined up next
     to the modality badge in the card head. Kept subtle so a card
     with 3 flags doesn't overpower its cover text. */
  .flag-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.1rem;
    height: 1.1rem;
    padding: 0 0.15rem;
    margin-left: 0.15rem;
    border-radius: 4px;
    background: #f0efe9;
    font-size: 0.7rem;
    line-height: 1;
  }

  /* Inbox triage marker on the card head — always visible (not
     hover-gated) so the User can scan the grid and spot which
     rows still need a look. Uses a warmer tint than the neutral
     flag-badge so it does not blend into content-type flags. */
  .inbox-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.1rem;
    height: 1.1rem;
    padding: 0 0.2rem;
    margin-left: 0.15rem;
    border-radius: 4px;
    background: #fff2d6;
    color: #7a5a10;
    font-size: 0.68rem;
    line-height: 1;
  }

  /* Sidebar "＋ new group / dir" inline creators (.group-create /
     .group-create input / .group-create button / .group-error) moved
     into GroupsSection.svelte (wave 5b-2). */

  /* Detail-overlay group chip (mirrors .label-tag styling but uses
     ~ prefix + slightly warmer edge — the same axis differentiator
     as the grid-header pill). */

  /* View mode toggle (Messages / Sessions) sits above the grid so
     the affordance is spatially adjacent to what it controls. */
  /* Row that holds the Messages / Sessions toggle on the left and
     the sort-axis picker on the right. Kept as `flex` (not `grid`)
     so the sort picker collapses to the right without perturbing
     the toggle when it's hidden in Sessions view. */
  .mode-row {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 0.8rem;
    margin-bottom: 0.6rem;
  }

  .mode-row .sort-picker {
    margin-left: auto;
  }

  /* Dialog-scope "Show messages" toggle. Sits between the view-mode
     buttons and the sort picker; the checkbox reads compact so the
     mode row stays a single line. */
  .show-messages-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-family: inherit;
    font-size: 0.72rem;
    color: #666;
    cursor: pointer;
    user-select: none;
  }
  .show-messages-toggle input[type="checkbox"] {
    margin: 0;
    cursor: pointer;
  }

  /* Groups-lane toggle sits next to the sort picker but uses a
     distinct pill treatment so the "on/off strip above the grid"
     nature is discoverable at a glance. */
  .lane-toggle {
    font-family: inherit;
    font-size: 0.72rem;
    color: #7a76c9;
    background: #f0effc;
    border: 1px solid transparent;
    border-radius: 8px;
    padding: 0.2rem 0.65rem;
    cursor: pointer;
  }

  .lane-toggle:hover {
    border-color: #d9d5f2;
  }

  .lane-toggle.active {
    background: #7a76c9;
    color: #fff;
  }

  /* `.jobs-ticker-*` cascade moved to JobsTickerBanner.svelte in
     wave C. */

  /* Dir-focused lane above the grid. Header shows the focused
     dir name + counts + a close button; the two rows underneath
     are sub-dirs and immediate groups respectively.
     When many tiles land in the lane (e.g. a 89-sub-dir root
     view), the strip could otherwise fill the entire viewport
     and hide the grid entirely — cap it at ~40 % of the window
     and scroll internally so both surfaces stay visible. */
  .dir-lane {
    padding: 0.5rem 0.15rem 0.7rem;
    border-bottom: 1px solid #e6e6e2;
    margin-bottom: 0.5rem;
    max-height: 40vh;
    overflow-y: auto;
    flex-shrink: 0;
  }

  .dir-lane-head {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    margin-bottom: 0.35rem;
  }

  .dir-lane-crumb {
    display: inline-flex;
    align-items: baseline;
    gap: 0.3rem;
    font-size: 0.85rem;
    color: #4d488a;
    font-weight: 600;
    flex-wrap: wrap;
  }

  .dir-lane-crumb-link {
    background: transparent;
    border: none;
    padding: 0;
    color: #7a76c9;
    font: inherit;
    cursor: pointer;
    text-transform: none;
  }

  .dir-lane-crumb-link:hover {
    text-decoration: underline;
  }

  .dir-lane-crumb-current {
    color: #4d488a;
  }

  .dir-lane-crumb-sep {
    color: #b8b3e8;
    font-weight: 400;
  }

  .dir-lane-up {
    background: transparent;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    color: #7a76c9;
    padding: 0 0.4rem;
    font-size: 0.75rem;
    cursor: pointer;
    line-height: 1.4;
  }

  .dir-lane-up:hover:not(:disabled) {
    background: #ecebfa;
  }

  .dir-lane-up:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .dir-lane-counts {
    color: #999;
    font-size: 0.7rem;
  }

  .dir-lane-close {
    margin-left: auto;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    color: #999;
    padding: 0 0.35rem;
    cursor: pointer;
    font-size: 0.75rem;
  }

  .dir-lane-close:hover {
    background: #ecebfa;
    color: #4d488a;
    border-color: #d9d5f2;
  }

  .dir-lane-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin: 0.2rem 0;
  }

  .dir-lane-label {
    color: #7a76c9;
    font-size: 0.7rem;
    padding-right: 0.35rem;
  }

  .dir-lane-empty {
    color: #999;
    font-size: 0.75rem;
    margin: 0.3rem 0;
  }

  /* Larger tile treatment for the lane's dir + group rows so the
     cover thumbnail carries the visual weight. Falls back to a
     folder / hash glyph while the fetch is still in flight (or
     when the group has no image asset at all). */
  .lane-tiles {
    align-items: flex-start;
  }

  .lane-tile {
    background: #fbfbf9;
    border: 1px solid #e6e6e2;
    border-radius: 10px;
    padding: 0.35rem 0.35rem 0.4rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-family: inherit;
    color: #444;
    cursor: pointer;
    width: 96px;
  }

  .lane-tile:hover {
    border-color: #b8b3e8;
    color: #333;
  }

  .lane-tile.active {
    background: #ecebfa;
    border-color: #7a76c9;
    color: #4d488a;
  }

  .lane-tile-thumb {
    position: relative;
    width: 100%;
    aspect-ratio: 1 / 1;
    border-radius: 6px;
    background: #eee;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .lane-tile-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .lane-tile-thumb-dir {
    background: #f4f0ff;
  }

  .lane-tile-icon {
    font-size: 1.6rem;
    color: #7a76c9;
    opacity: 0.7;
  }

  .lane-tile-count {
    position: absolute;
    right: 4px;
    bottom: 4px;
    background: rgba(255, 255, 255, 0.9);
    border-radius: 4px;
    padding: 0 4px;
    font-size: 0.65rem;
    color: #7a76c9;
    line-height: 1.4;
  }

  .lane-tile-name {
    font-size: 0.72rem;
    text-align: center;
    line-height: 1.2;
    color: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    word-break: break-all;
  }

  .dir-lane-apply {
    justify-content: flex-start;
  }

  .dir-lane-apply-btn {
    font-family: inherit;
    font-size: 0.75rem;
    color: #7a76c9;
    background: #f0effc;
    border: 1px solid transparent;
    border-radius: 8px;
    padding: 0.3rem 0.7rem;
    cursor: pointer;
  }

  .dir-lane-apply-btn:hover {
    border-color: #d9d5f2;
  }

  .dir-lane-apply-btn.active {
    background: #7a76c9;
    color: #fff;
  }

  .dir-lane-apply-count {
    font-size: 0.68rem;
    opacity: 0.85;
    margin-left: 0.35rem;
  }

  .group-chip-large {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.25rem 0.55rem;
    background: #fbfbf9;
    border: 1px solid #e6e6e2;
    border-radius: 8px;
    font-family: inherit;
    font-size: 0.75rem;
    color: #444;
    cursor: pointer;
  }

  .group-chip-large:hover {
    border-color: #b8b3e8;
    color: #333;
  }

  .group-chip-large.active {
    background: #ecebfa;
    border-color: #7a76c9;
    color: #4d488a;
  }

  .group-chip-icon {
    font-size: 0.85rem;
  }

  .group-chip-count {
    color: #999;
    font-size: 0.68rem;
    background: #fff;
    padding: 0 0.25rem;
    border-radius: 4px;
    border: 1px solid #ececec;
  }

  .group-chip-large.active .group-chip-count {
    background: #fff;
    border-color: #d9d5f2;
    color: #7a76c9;
  }

  .view-mode {
    display: inline-flex;
    gap: 0.15rem;
    background: #f0effc;
    border-radius: 8px;
    padding: 0.15rem;
  }

  .sort-picker {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.72rem;
    color: #7a76c9;
  }

  .sort-picker select {
    font: inherit;
    color: inherit;
    background: #f0effc;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 0.15rem 0.4rem;
    cursor: pointer;
  }

  .sort-picker select:hover {
    border-color: #d9d5f2;
  }
  .sort-picker label {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
  }
  /* Stands in for the two selects while relevance owns the order. Same
     chip shape as `select` so the toolbar keeps its rhythm, minus the
     affordances — nothing here is clickable. */
  .sort-manual {
    background: #f0effc;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    padding: 0.15rem 0.4rem;
    cursor: default;
  }
  /* Sidebar reorder feedback — a subtle top border marks the drop
     target so the user sees where the row will land, and the drag
     origin fades so it stays distinct from the shifted rows. */
  /* Same reasoning as `.card` above — sidebar rows are drag sources
     (group → group, group → dir) through the pointer handler, so the
     native image/link drag has no part to play. */
  aside.sidebar,
  aside.sidebar * {
    -webkit-user-drag: none;
  }
  aside.sidebar li.dragging {
    opacity: 0.4;
  }
  aside.sidebar li.reorder-target {
    border-top: 2px solid #6c58c3;
  }
  .view-mode button {
    border: none;
    background: transparent;
    padding: 0.2rem 0.8rem;
    font-size: 0.72rem;
    color: #7a76c9;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
  }
  .view-mode button.active {
    background: #7a76c9;
    color: #fff;
  }

  /* Sessions grid tile: same footprint as a message card but with a
     session-specific meta row (message count + occurrence range). */
  .session-card {
    text-align: left;
    border: 1px solid #eeecf8;
    cursor: pointer;
    font-family: inherit;
  }
  .session-card:hover {
    background: #f8f7fd;
  }
  .session-meta {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: 0.6rem;
    color: #9a96d9;
    margin-top: 0.3rem;
  }
  .session-count {
    background: #f0effc;
    color: #7a76c9;
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
    font-variant-numeric: tabular-nums;
  }
  .session-range {
    font-variant-numeric: tabular-nums;
    opacity: 0.7;
  }

  /* Active session chip in the grid header uses the same pill shape
     as the tag chips but a different glyph (⇱) so the two axes are
     visually distinct. */
  .session-chip {
    background: #fdf6f0;
    border-color: #f0d5c0;
    color: #b28860;
  }
  .session-chip:hover {
    background: #f7e5d3;
    color: #8a6540;
  }

  .persona-name {
    font-size: 0.65rem;
    color: #bbb;
    margin: 0;
  }

  .empty {
    color: #999;
    font-size: 0.85rem;
  }

  /* Fetch-in-flight pill (persona switch on 6-figure grids). Fixed
     top-center of the content pane so it reads over both the stale
     grid and the sessions view. */
  .grid-loading-pill {
    position: fixed;
    top: 0.9rem;
    left: 55%;
    transform: translateX(-50%);
    z-index: 60;
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.3rem 0.8rem;
    background: rgba(255, 254, 249, 0.95);
    border: 1px solid #d9d5f2;
    border-radius: 999px;
    box-shadow: 0 4px 14px rgba(80, 70, 160, 0.14);
    font-size: 0.75rem;
    color: #5a55b8;
  }
  .grid-loading-spinner {
    width: 12px;
    height: 12px;
    border: 2px solid #d9d5f2;
    border-top-color: #5a55b8;
    border-radius: 50%;
    animation: grid-loading-spin 0.8s linear infinite;
  }
  @keyframes grid-loading-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* .burst* cascade moved to ConstellationBurst.svelte (wave ④). */

  .detail-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(20, 18, 30, 0.72);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2vh 2vw;
    z-index: 100;
    border: none;
    cursor: default;
  }


  .detail-close {
    position: absolute;
    top: 0.4rem;
    right: 0.6rem;
    z-index: 2;
    background: rgba(255, 255, 255, 0.85);
    border: 1px solid #d5d3ca;
    border-radius: 999px;
    width: 1.9rem;
    height: 1.9rem;
    cursor: pointer;
    color: #555;
    font-size: 0.85rem;
  }

  .detail-close:hover {
    color: #111;
    background: #fff;
  }

  .detail-loading {
    padding: 3rem;
    text-align: center;
    color: #999;
    margin: 0;
  }




  .detail-media {
    position: relative;
  }



  /* Full-window image stage — sits above the detail overlay.
     The backdrop hosts a zoomable image via CSS transform; cursor
     hints at the current interaction (zoom-out at rest, grab when
     the image is enlarged, grabbing while dragging). */

  /* Session Reader chip in the grid header (sits beside the ⇱
     session chip). */
  .reader-chip {
    background: #f3f0fc;
    border-color: #d9d5f2;
    color: #6f6c9c;
  }
  .reader-chip:hover {
    background: #e2ddf9;
    color: #4a4780;
  }

  /* Session Reader panel — a reading column, not a grid. */
  .reader-panel {
    background: #fbfbf9;
    border-radius: 10px;
    width: min(94vw, 860px);
    max-height: 96vh;
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
  }
  .reader-title {
    margin: 0;
    padding: 1rem 3rem 0.8rem 1.4rem;
    font-size: 0.95rem;
    color: #111;
    border-bottom: 1px solid #e2e2de;
    word-break: break-word;
  }
  .reader-count {
    font-size: 0.7rem;
    color: #999;
    font-weight: normal;
  }
  .reader-scroll {
    overflow-y: auto;
    padding: 1rem 1.4rem 2rem;
  }
  .reader-msg {
    margin-bottom: 1.1rem;
    padding: 0.6rem 0.8rem;
    border-radius: 8px;
    border-left: 3px solid #e2e2de;
    background: #fff;
  }
  .reader-msg-user {
    border-left-color: #7a76c9;
    background: #f7f6fd;
  }
  .reader-msg-assistant {
    border-left-color: #7ab89a;
  }
  .reader-msg-system,
  .reader-msg-tool {
    border-left-color: #d8d8d0;
    background: #fafaf7;
  }
  .reader-meta {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 0.35rem;
  }
  .reader-role {
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #9a96d9;
  }
  .reader-time {
    font-size: 0.62rem;
    color: #bbb;
    font-variant-numeric: tabular-nums;
  }
  .reader-msg pre {
    margin: 0;
    font-size: 0.82rem;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
    color: #2a2a2a;
    font-family: inherit;
  }
  .reader-image {
    max-width: 320px;
    max-height: 240px;
    border-radius: 6px;
    display: block;
  }

  /* Reader mode chip strip — parallels the detail overlay's chips
     (`.detail-mode-strip` above). Sits inline in the reader title. */
  .reader-mode-strip {
    margin-left: 0.6rem;
    display: inline-flex;
    gap: 0.2rem;
    vertical-align: middle;
  }
  .reader-mode-chip {
    padding: 0.1rem 0.5rem;
    border: 1px solid #d0d0d0;
    border-radius: 3px;
    background: #fafafa;
    cursor: pointer;
    font-size: 0.7rem;
    color: #555;
    font-family: ui-monospace, "SF Mono", monospace;
  }
  .reader-mode-chip:hover {
    background: #eee;
  }
  .reader-mode-chip.active {
    background: #6c58c3;
    border-color: #6c58c3;
    color: #fff;
  }
  .reader-html {
    width: 100%;
    min-height: 40vh;
    border: 1px solid #e2e2de;
    border-radius: 4px;
    background: #fff;
  }
  .reader-term {
    font-family: ui-monospace, "SF Mono", "Menlo", monospace !important;
    font-size: 0.8rem !important;
    background: #14161c;
    color: #f4f4f8 !important;
    padding: 0.8rem 1rem;
    border-radius: 4px;
    line-height: 1.55 !important;
  }

  /* MD / raw toggle in the reader header (legacy — kept while the
     new `.reader-mode-strip` beds in; safe to remove once no other
     surface references it). */
  .reader-md-toggle {
    margin-left: 0.5rem;
    padding: 0.1rem 0.5rem;
    font-size: 0.62rem;
    border: 1px solid #d9d5f2;
    border-radius: 999px;
    background: #fff;
    color: #9a96d9;
    cursor: pointer;
    font-family: inherit;
    vertical-align: middle;
  }
  .reader-md-toggle.active {
    background: #7a76c9;
    border-color: #7a76c9;
    color: #fff;
  }

  /* Rendered-markdown body. `:global` because the nodes come from
     `{@html}` and carry no Svelte scoping class. */
  .reader-md {
    font-size: 0.82rem;
    line-height: 1.55;
    color: #2a2a2a;
    word-break: break-word;
  }
  .reader-md :global(p),
  .reader-md :global(ul),
  .reader-md :global(ol),
  .reader-md :global(blockquote),
  .reader-md :global(pre),
  .reader-md :global(table) {
    margin: 0 0 0.6rem;
  }
  .reader-md :global(h1),
  .reader-md :global(h2),
  .reader-md :global(h3),
  .reader-md :global(h4) {
    margin: 0.8rem 0 0.4rem;
    font-size: 0.9rem;
    line-height: 1.3;
  }
  .reader-md :global(h1) {
    font-size: 1rem;
  }
  .reader-md :global(code) {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 0.76rem;
    background: #f0efe9;
    border-radius: 3px;
    padding: 0.05rem 0.25rem;
  }
  .reader-md :global(pre) {
    background: #f4f3ee;
    border: 1px solid #e6e4dc;
    border-radius: 6px;
    padding: 0.55rem 0.7rem;
    overflow-x: auto;
    white-space: pre;
  }
  .reader-md :global(pre code) {
    background: transparent;
    padding: 0;
  }
  .reader-md :global(blockquote) {
    border-left: 3px solid #d9d5f2;
    padding-left: 0.7rem;
    color: #666;
  }
  .reader-md :global(ul),
  .reader-md :global(ol) {
    padding-left: 1.3rem;
  }
  .reader-md :global(a) {
    color: #5a55b2;
  }
  .reader-md :global(img) {
    max-width: 100%;
  }
  .reader-md :global(table) {
    border-collapse: collapse;
    font-size: 0.76rem;
  }
  .reader-md :global(th),
  .reader-md :global(td) {
    border: 1px solid #e2e2de;
    padding: 0.2rem 0.5rem;
  }
  .reader-md :global(hr) {
    border: none;
    border-top: 1px solid #e2e2de;
    margin: 0.8rem 0;
  }

  /* Full-window stage prev/next — quiet edge affordances that light
     up on hover (keyboard arrows are the primary path). */














  /* Tag chip group: the tag button plus its promote / detach
     satellites. Rendered inline so the trio stays visually
     bound and wraps as one unit. */
  .tag-chip-group {
    display: inline-flex;
    align-items: stretch;
    border-radius: 3px;
    overflow: hidden;
    border: 1px solid transparent;
  }
  .tag-chip-group:hover {
    border-color: #d0d0d0;
  }
  .tag-chip-action {
    background: #f6f6f6;
    border: none;
    border-left: 1px solid #e4e4e4;
    padding: 0 0.35rem;
    font-size: 0.8rem;
    line-height: 1;
    cursor: pointer;
    color: #666;
  }
  .tag-chip-action:hover {
    background: #eaeaea;
    color: #111;
  }
  .tag-chip-promote:hover {
    background: #ffe9c9;
    color: #7a4a00;
  }
  /* "Already promoted" state — the ✓ mark plus a muted olive tint
     signals that a Group ~<tag> already exists for this persona,
     so a second click will error on the `(persona, name)` unique
     constraint. Kept clickable so the toast can echo the actual
     error and the user learns the tag is already snapshot'd. */
  .tag-chip-promote.tag-chip-promoted {
    background: #eef3e7;
    color: #4a6a1a;
  }
  .tag-chip-promote.tag-chip-promoted:hover {
    background: #dfe8d3;
    color: #2c4a00;
  }
  .tag-chip-detach:hover {
    background: #fde5e5;
    color: #a00;
  }

  /* Inline "add tag" form under the tag list. Kept compact so it
     lives comfortably in the detail meta panel. */
  .tag-add-row {
    display: flex;
    gap: 0.25rem;
    margin-top: 0.35rem;
  }
  .tag-add-input {
    flex: 1;
    padding: 0.2rem 0.4rem;
    border: 1px solid #d0d0d0;
    border-radius: 3px;
    font-size: 0.85rem;
  }
  .tag-add-btn {
    padding: 0 0.55rem;
    border: 1px solid #d0d0d0;
    border-radius: 3px;
    background: #fafafa;
    cursor: pointer;
    font-weight: 600;
    color: #333;
  }
  .tag-add-btn:hover {
    background: #eee;
  }

  /* Persona wallpaper — rendered as a background layer under the
     main grid area only. The sidebar keeps its solid background so
     tag / group / dir controls stay legible; the wallpaper is a
     persona-identity signal, not full-window chrome. `--persona-
     wallpaper` is set on `.layout` inline; the ::before pseudo
     paints the image itself, and a white gradient co-mixed via the
     `background` shorthand fades it toward the background so cards
     always contrast without the ::before needing an `opacity`
     that would also fade the wallpaper's colour into a flat grey.
     `isolation: isolate` starts a new stacking context on
     `.content`, so the ::before paints under every child without
     any `.content > *` z-index / position override (which would
     otherwise clobber `.burst`'s `position: fixed`). */
  .layout.has-wallpaper .content {
    position: relative;
    isolation: isolate;
  }
  .layout.has-wallpaper .content::before {
    content: "";
    position: absolute;
    inset: 0;
    background:
      linear-gradient(rgba(255, 255, 255, 0.78), rgba(255, 255, 255, 0.78)),
      var(--persona-wallpaper);
    background-size: cover;
    background-position: center;
    background-repeat: no-repeat;
    pointer-events: none;
    z-index: -1;
  }


  .persona-wallpaper-clear {
    margin-left: 0.35rem;
    padding: 0 0.35rem;
    border: 1px solid #d0d0d0;
    border-radius: 3px;
    background: #fafafa;
    cursor: pointer;
    font-size: 0.75rem;
    color: #666;
  }
  .persona-wallpaper-clear:hover {
    background: #fde5e5;
    color: #a00;
  }

  /* Full-window drop hint. Layers above every other UI so it is
     always the first signal the user gets when Finder / a browser
     starts dragging a file over the app; a subtle blue tint plus
     the icon and label give an unambiguous "drop here to import"
     surface. */
  .drop-overlay {
    position: fixed;
    inset: 0;
    z-index: 900;
    background: rgba(30, 80, 200, 0.18);
    backdrop-filter: blur(1.5px);
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    animation: dropOverlayIn 120ms ease-out;
  }
  .drop-overlay-inner {
    background: rgba(255, 255, 255, 0.94);
    border: 2px dashed #4d68d5;
    border-radius: 14px;
    padding: 1.6rem 2.6rem;
    text-align: center;
    box-shadow: 0 10px 40px rgba(30, 80, 200, 0.15);
  }
  .drop-overlay-icon {
    font-size: 2.4rem;
    color: #4d68d5;
    line-height: 1;
    margin-bottom: 0.4rem;
  }
  .drop-overlay-title {
    font-size: 1rem;
    font-weight: 600;
    color: #223;
  }
  .drop-overlay-sub {
    font-size: 0.8rem;
    color: #778;
    margin-top: 0.3rem;
  }
  @keyframes dropOverlayIn {
    from { opacity: 0; transform: scale(0.985); }
    to { opacity: 1; transform: scale(1); }
  }

  /* Sidebar gear button. Small so it does not steal focus from the
     app title, but tinted on hover so it reads as clickable. */
  .settings-gear {
    float: right;
    background: transparent;
    border: none;
    font-size: 1rem;
    color: #999;
    cursor: pointer;
    padding: 0 0.2rem;
    line-height: 1;
  }
  .settings-gear:hover {
    color: #6c58c3;
  }

  /* Settings modal — chrome + shortcut table. */
  .settings-backdrop {
    position: fixed;
    inset: 0;
    z-index: 950;
    background: rgba(20, 22, 32, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .settings-panel {
    background: #fff;
    border-radius: 10px;
    min-width: 520px;
    max-width: 720px;
    max-height: 80vh;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
    cursor: default;
    display: flex;
    flex-direction: column;
  }
  .settings-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.9rem 1.2rem;
    border-bottom: 1px solid #eee;
  }
  .settings-header h3 {
    margin: 0;
    font-size: 1rem;
    color: #223;
  }
  .settings-close {
    background: none;
    border: none;
    font-size: 1rem;
    cursor: pointer;
    color: #888;
  }
  .settings-close:hover { color: #223; }
  .settings-body {
    padding: 1rem 1.2rem;
    overflow-y: auto;
  }
  .settings-body h4 {
    margin: 0 0 0.6rem;
    font-size: 0.85rem;
    color: #556;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .shortcut-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  .shortcut-table th {
    text-align: left;
    padding: 0.35rem 0.6rem;
    background: #f7f5ee;
    color: #556;
    font-weight: 600;
  }
  .shortcut-table td {
    padding: 0.35rem 0.6rem;
    border-top: 1px solid #eee;
    color: #333;
  }
  .shortcut-table td.scope {
    color: #888;
    font-size: 0.75rem;
    text-transform: uppercase;
  }
  .shortcut-table td.keys {
    font-family: ui-monospace, "SF Mono", monospace;
    background: #fafafa;
    color: #4a3a90;
    font-weight: 600;
    white-space: nowrap;
  }
  /* `.settings-hint` / `.settings-toggle` moved to
     SettingsPreferences.svelte along with the last markup that used
     them — Svelte scopes styles per component, so leaving them here
     would have styled nothing. */

  /* Sidebar persona row: mini avatar chip next to the name so the
     Profile card's identity signal has a lightweight preview in
     the always-visible list. Falls through to the plain "○" bullet
     when no avatar is set. */
  .persona-avatar-mini {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    object-fit: cover;
    vertical-align: middle;
    margin-right: 0.35em;
  }

  /* .profile-card cascade moved to ProfileCard.svelte in wave 8b
     (scoped-CSS namespace switch — same duplication policy as the
     other extracted components). */

  /* Card right-click context menu. Positioned near the cursor and
     kept small so the action set stays reflexive. */
  .card-menu {
    position: fixed;
    z-index: 960;
    background: #fff;
    border: 1px solid #d0d0d0;
    border-radius: 6px;
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.15);
    padding: 0.3rem 0;
    min-width: 200px;
  }
  .card-menu-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.45rem 0.8rem;
    border: none;
    background: transparent;
    cursor: pointer;
    font-size: 0.85rem;
    color: #223;
  }
  .card-menu-item:hover {
    background: #eee9ff;
  }
  /* Destructive tone (HIG): the entries that remove a card say so
     before they are clicked. Both tiers carry it — "Move to Trash" is
     reversible but still takes the card out of where the user filed
     it, and a tone that only appears on the unrecoverable one trains
     the eye to expect nothing from the other. */
  .card-menu-item-danger {
    color: #a00;
  }
  .card-menu-item-danger:hover {
    background: #fde5e5;
  }
  /* Rule above the destructive tier. The gap is doing the work of the
     separator as much as the line is: it is the thing that stops a
     click aimed at the entry above from landing here.

     An `<hr>` rather than a `div role="separator"`: the role is the
     element's own, so the markup carries no redundant ARIA. */
  .card-menu-sep {
    height: 1px;
    border: none;
    background: #e8e8ee;
    margin: 0.3rem 0;
  }
  /* Inline SVG glyph sized and baseline-tuned to sit like the emoji
     prefixes the other entries use. */
  .card-menu-glyph {
    width: 1em;
    height: 1em;
    vertical-align: -0.15em;
  }
  .card-menu-item:disabled {
    opacity: 0.45;
    cursor: default;
  }
  /* Bulk mode (W5-e): header count + fold-out submenus rendered in
     place (no floating popover — the menu itself is the anchor). */
  .card-menu-head {
    padding: 0.3rem 0.8rem 0.4rem;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #778;
    border-bottom: 1px solid #e8e8ee;
    margin-bottom: 0.2rem;
  }
  .card-menu-sub {
    border-top: 1px solid #eee;
    border-bottom: 1px solid #eee;
    background: #fafaff;
    max-height: 11rem;
    overflow-y: auto;
    padding: 0.2rem 0;
  }
  .card-menu-sub .card-menu-item {
    padding-left: 1.4rem;
    font-size: 0.82rem;
  }
  .card-menu-sub-head {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #99a;
    padding: 0.25rem 1.4rem 0.1rem;
  }
  .card-menu-tag-add {
    display: flex;
    gap: 0.35rem;
    padding: 0.3rem 0.8rem;
  }
  .card-menu-tag-input {
    flex: 1;
    min-width: 0;
    padding: 0.28rem 0.5rem;
    font-size: 0.82rem;
    font-family: inherit;
    color: #223;
    background: #fff;
    border: 1px solid #ccd;
    border-radius: 6px;
  }
  .card-menu-tag-input:focus {
    outline: none;
    border-color: #8a86ff;
  }
  .card-menu-tag-addbtn {
    width: auto;
    flex: 0 0 auto;
    border: 1px solid #ccd;
    border-radius: 6px;
    padding: 0.28rem 0.7rem;
  }
  .card-menu-tag-list {
    display: flex;
    flex-direction: column;
    max-height: 8rem;
    overflow-y: auto;
  }

  /* Persona picker modal for buffered drops. Overlays the app
     when files landed without an active persona; the click on the
     backdrop cancels, the click inside picks one. */
  .drop-picker-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: rgba(20, 22, 32, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .drop-picker-panel {
    background: #fff;
    border-radius: 10px;
    padding: 1.4rem 1.6rem;
    min-width: 340px;
    max-width: 440px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
    cursor: default;
  }
  .drop-picker-title {
    margin: 0 0 0.35rem;
    font-size: 1rem;
    color: #223;
  }
  .drop-picker-sub {
    margin: 0 0 0.9rem;
    font-size: 0.8rem;
    color: #778;
  }
  .drop-picker-personas {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 40vh;
    overflow-y: auto;
  }
  .drop-picker-persona-btn {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.4rem 0.7rem;
    margin: 0.15rem 0;
    border: 1px solid #d9d5f2;
    border-radius: 5px;
    background: #fbfaff;
    cursor: pointer;
    font-size: 0.9rem;
    color: #333;
  }
  .drop-picker-persona-btn:hover {
    background: #eee9ff;
    border-color: #b8afef;
  }
  .drop-picker-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.9rem;
  }
  .drop-picker-cancel {
    padding: 0.3rem 0.9rem;
    border: 1px solid #ccc;
    border-radius: 5px;
    background: #fafafa;
    cursor: pointer;
    font-size: 0.85rem;
    color: #555;
  }
  .drop-picker-cancel:hover {
    background: #eee;
  }

  /* --- Dispatch toast ------------------------ */

  /* The selector-bar cascade (.selector-bar / .selector-count /
     .selector-btn / .selector-clear / .selector-hint) was retired
     together with the floating "N selected · Clear" pill it painted.
     Multi-select exits: bare card click (commit-to-one), background
     click, or Escape. The right-click card-menu (`.card-menu` /
     `.card-menu-sub`) surfaces bulk actions and the "N selected"
     count. */

  /* App-level Threads drawer trigger. The drawer itself is scoped
     inside ThreadDrawer.svelte. */
  .thread-open-btn {
    padding: 0.25rem 0.7rem;
    font-size: 0.8rem;
    font-family: inherit;
    color: #5a55b2;
    background: #f0effc;
    border: 1px solid #d9d5f2;
    border-radius: 6px;
    cursor: pointer;
  }
  .thread-unread-badge {
    margin-left: 0.35rem;
    padding: 0 0.4rem;
    border-radius: 999px;
    background: #6c58c3;
    color: #fff;
    font-size: 0.65rem;
    font-variant-numeric: tabular-nums;
  }

  .thread-open-btn:hover {
    background: #e2ddf9;
    color: #47429a;
  }

  /* `.dispatch-toast` moved to DispatchToast.svelte in wave C. */

  /* `.prompt-*` cascade moved to PromptModal.svelte in wave A. */
</style>
