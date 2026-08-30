<script lang="ts">
  // DetailPane — extracted from App.svelte (2026-07-20 L3 pilot β').
  // Owns: detail overlay (backdrop / panel / meta column) + all four
  // modality bodies (image / video / audio / text) + waveform decode +
  // fullscreen zoom stage + label/note/tag/group/comment/provenance/
  // selection sub-panels + per-asset detail LRU cache.
  //
  // Not owned: Reader stage, Grid, Sidebar, SavedQuery modal — those
  // stay in App.svelte. Shared display formatters (`parseExtra` /
  // `fmtDateTime` / `fmtBytes` / `fmtDurationMs` / `renderMarkdown`
  // / `personaName`) moved to `lib/formatters.ts` in wave B and are
  // imported directly instead of threaded through as props.
  // `detailSrc` reads `thumbCatalog.detailSrc` directly since the
  // thumb cache graduated in wave ②. `selectionRowLabel` moved to
  // `selectionCatalog.rowLabel` in wave 9 and DetailPane reads it
  // through the store directly. Sidebar count refreshes
  // (`onLoadTagCounts` / `onLoadGroupCounts`) collapsed to
  // `tagCatalog.loadCounts(activeFilter.activePersona)` /
  // `groupCatalog.loadCounts(activeFilter.activePersona)`.
  //
  // Marks (notes at a position *inside* the material, as opposed to the
  // comment thread's notes *about* the asset) are `MaterialMarks.svelte`
  // and own themselves; this pane only hands down the asset id, its
  // `duration_ms`, and the live media element the marks are read from
  // and seek through. `MaterialChapters.svelte` is the other band over
  // that timeline — how the material is *divided*, and by whom — and
  // takes the same three props for the same reasons. It replaced a chip
  // strip this pane built out of `extra.chapters`, a blob that could not
  // say whether the file had declared those sections or somebody had
  // written them; nothing here reads `extra.chapters` any more.
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { mutate } from "./lib/mutate";
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import AlbumMetaSection from "./AlbumMetaSection.svelte";
  import SourceTypeRow from "./SourceTypeRow.svelte";
  import { readAlbumMeta } from "./lib/album-meta";
  import MaterialChapters from "./MaterialChapters.svelte";
  import MaterialMarks from "./MaterialMarks.svelte";
  import PromoteToTeam from "./PromoteToTeam.svelte";
  import { baseName } from "./lib/basename";
  import {
    fmtBytes,
    fmtDateTime,
    fmtDimensions,
    fmtDurationMs,
    parseExtra,
    personaName,
    pickDetailMode,
    renderMarkdown,
    type DetailMode,
  } from "./lib/formatters";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { assetPageCatalog } from "./lib/stores/asset-page.svelte";
  import { gridSelection } from "./lib/stores/grid-selection.svelte";
  import { groupCatalog } from "./lib/stores/group.svelte";
  import { modalityCatalog } from "./lib/stores/modality.svelte";
  import { tagCatalog } from "./lib/stores/tag.svelte";
  import { themeCatalog } from "./lib/stores/theme.svelte";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";
  import type {
    AssetCardDto,
    AssetCommentDto,
    AssetDetailDto,
    AssetDto,
    AssetPageDto,
    AssetTextDto,
    GroupDto,
    LineageViewDto,
    PromoteTagToGroupResult,
    SnapshotDto,
    TagDto,
    TagSuggestionDto,
    VideoPreviewDto,
  } from "./bindings";

  type DetailSnap = {
    detail: AssetDetailDto;
    groupIds: string[];
    text: string | null;
    mode: DetailMode;
  };

  interface Props {
    // Input: which asset id to show. null closes the overlay.
    openAssetId: string | null;

    // Callbacks (DetailPane → App).
    // `onAddTagToGridFilter` was replaced by a direct
    // `activeFilter.addTag` call — the App-side `$effect` that
    // tracks `activeFilter.activeTagIds.size` still picks up the
    // mutation and triggers `loadAssets` transparently.
    // `currentPageItems` / `invalidations` / `onLoadAssets` /
    // `onAssetChanged` props collapsed into direct
    // `assetPageCatalog` reads/calls (wave ①).
    onClose: () => void;
    onOpenAsset: (id: string) => void;
    onSetStatus: (msg: string) => void;
    onSaveLabels: (assetId: string, next: string[]) => Promise<void>;
    onSetAsWallpaper: (assetId: string) => Promise<void>;
    // Refresh the sidebar tallies (modality / persona counts) after a
    // modality edit shifts an asset between buckets. Was referenced by
    // `saveModality` before the prop declaration was lost in a refactor
    // (since restored).
    onRefreshCounts: () => void;
  }

  let {
    openAssetId,
    onClose,
    onOpenAsset,
    onSetStatus,
    onSaveLabels,
    onSetAsWallpaper,
    onRefreshCounts,
  }: Props = $props();

  // Sibling asset ids for arrow-key navigation — derived straight
  // from the catalog's messages page (was a prop before wave ①).
  let currentPageItems = $derived(
    assetPageCatalog.page?.items.map((c) => ({
      id: c.id,
      modality: c.modality,
    })) ?? [],
  );

  // -------------------------------------------------------------------
  // Core detail state
  // -------------------------------------------------------------------
  let detail = $state<AssetDetailDto | null>(null);

  // Video preview rendition (VP9 WebM / Matroska — formats the
  // webview cannot display, measured). The backend
  // answers ready / pending / not_needed / failed; while pending a
  // transcode is running and this pane polls. `null` = the first
  // answer has not arrived yet.
  let videoPreview = $state<VideoPreviewDto | null>(null);
  let videoPreviewTimer: ReturnType<typeof setTimeout> | null = null;

  function stopVideoPreviewPoll() {
    if (videoPreviewTimer !== null) {
      clearTimeout(videoPreviewTimer);
      videoPreviewTimer = null;
    }
  }

  async function pollVideoPreview(assetId: string) {
    try {
      const dto = await invoke<VideoPreviewDto>("asset_video_preview", {
        assetId,
      });
      // Guard against navigation while the request was in flight.
      if (detail?.asset.id !== assetId) return;
      videoPreview = dto;
      if (dto.status === "pending") {
        videoPreviewTimer = setTimeout(
          () => void pollVideoPreview(assetId),
          1500,
        );
      }
    } catch (err) {
      // A status call that fails is a failure — surfacing it beats
      // silently degrading to a player that may not play (the silent
      // crossed-out icon is the defect this whole flow exists to
      // remove).
      if (detail?.asset.id === assetId) {
        videoPreview = {
          status: "failed",
          path: null,
          detail: `preview status call failed: ${String(err)}`,
        };
      }
    }
  }

  $effect(() => {
    const d = detail;
    stopVideoPreviewPoll();
    videoPreview = null;
    if (d && mediaKind(d.asset.media) === "video") {
      void pollVideoPreview(d.asset.id);
    }
    return () => stopVideoPreviewPoll();
  });
  let detailLoading = $state(false);
  // Members of the open asset when it is a container. A container owns
  // no body of its own, so without this the pane shows a cover line and
  // nothing else — "the detail of a session" with the session's actual
  // content missing. Fetched here rather than folded into
  // `AssetDetailDto` because it is a list whose size is unbounded and
  // only one role ever needs it.
  let members = $state<AssetCardDto[]>([]);
  let memberTexts = $state<Map<string, string | null>>(new Map());
  let membersLoading = $state(false);
  const isContainer = $derived(detail?.asset.role === "collection");
  // 1-hop derived_from lineage — fetched independently so the header
  // paints without waiting for the graph query.
  // Multi-hop `derived_from` chain around the open asset. Ancestors
  // sit at positive depth (what it came from), descendants at
  // negative (what came out of it) — see `LineageViewDto`.
  let provenance = $state<LineageViewDto | null>(null);
  // Split once, ordered by distance, so each lane reads outward from
  // the asset in the pane.
  const lineageAncestors = $derived(
    (provenance?.nodes ?? []).filter((n) => n.depth > 0).sort((a, b) => a.depth - b.depth),
  );
  const lineageDescendants = $derived(
    (provenance?.nodes ?? []).filter((n) => n.depth < 0).sort((a, b) => b.depth - a.depth),
  );
  let provenanceLoading = $state(false);
  let detailGroupIds = new SvelteSet<string>();
  let detailSelections = $state<SnapshotDto[]>([]);

  // -------------------------------------------------------------------
  // Audio waveform
  // -------------------------------------------------------------------
  // Downsampled envelope (positive amplitudes, 0..1) rendered on the
  // canvas that overlays the native <audio> element. Peaks are cached
  // per assetId so revisiting the same track redraws instantly.
  // Video waveform is deliberately unwired — decoding the whole
  // stream into an AudioBuffer for a long recording OOMs the
  // WKWebView; only audio-modality assets get the treatment.
  const WAVEFORM_PEAK_COUNT = 240;
  const WAVEFORM_CACHE_MAX = 20;
  const waveformCache = new Map<string, Float32Array | "error">();
  function waveformCachePut(id: string, value: Float32Array | "error") {
    if (waveformCache.has(id)) waveformCache.delete(id);
    waveformCache.set(id, value);
    while (waveformCache.size > WAVEFORM_CACHE_MAX) {
      const first = waveformCache.keys().next().value;
      if (first === undefined) break;
      waveformCache.delete(first);
    }
  }
  let waveformPeaks = $state<Float32Array | null>(null);
  let waveformError = $state<string | null>(null);
  let waveformDecoding = $state(false);
  let audioEl = $state<HTMLAudioElement | null>(null);
  // The video branch has no waveform (see above), but it does have a
  // timeline, so the element is bound for the same reason `audioEl` is:
  // `MaterialMarks` reads the playhead off it and seeks it.
  let videoEl = $state<HTMLVideoElement | null>(null);
  let waveformCanvas = $state<HTMLCanvasElement | null>(null);
  let audioProgress = $state(0);
  let audioDurationSec = $state(0);

  // Mime top-level type → the player this pane should show, for the
  // three that have one. `null` = "this format says nothing about how
  // to display it", which hands the decision back to the modality
  // lookup (`text/plain` covers a journal entry and a transcript
  // alike, so it is not an answer).
  // Now a read of the slug the backend decided (`media`), not a
  // second implementation of the rule. `"none"` is the backend saying
  // "these bytes call for no player", which is the same `null` this
  // returned before — it hands the decision to the modality lookup.
  function mediaKind(media: string | null | undefined): string | null {
    return media && media !== "none" ? media : null;
  }

  /// The classification's only say in how a body reads: `"term"` when
  /// it is a terminal transcript, otherwise nothing (the preview layer
  /// sniffs the text's own shape).
  function terminalKind(modality: string | null | undefined): string | null {
    return modalityCatalog.isTerminal(modality) ? "term" : null;
  }

  async function decodeAudioPeaks(url: string): Promise<Float32Array> {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`fetch ${resp.status}`);
    const buf = await resp.arrayBuffer();
    const AC =
      window.OfflineAudioContext ??
      (window as unknown as { webkitOfflineAudioContext?: typeof OfflineAudioContext })
        .webkitOfflineAudioContext;
    if (!AC) throw new Error("OfflineAudioContext unavailable");
    // 1 ch × 1 s @ 44.1 kHz is a throwaway sink for decode-only.
    const ctx = new AC(1, 44100, 44100);
    const decoded = await ctx.decodeAudioData(buf);
    const raw = decoded.getChannelData(0);
    const target = WAVEFORM_PEAK_COUNT;
    const step = Math.max(1, Math.floor(raw.length / target));
    const peaks = new Float32Array(target);
    for (let i = 0; i < target; i++) {
      const start = i * step;
      const end = Math.min(raw.length, start + step);
      let max = 0;
      for (let j = start; j < end; j++) {
        const v = raw[j] < 0 ? -raw[j] : raw[j];
        if (v > max) max = v;
      }
      peaks[i] = max;
    }
    return peaks;
  }

  // Envelope + playhead only. The chapter ticks this used to draw came
  // from `extra.chapters` and are now `MaterialChapters`' own ruler:
  // chapters belong to a band that says who declared them, the band is
  // that component's state, and drawing it here would mean handing a
  // child's rows back up to its parent to paint. The DOM ruler also
  // reaches the video branch, which has no canvas at all — decoding a
  // whole video for peaks OOMs the webview. The material's duration went
  // with the ticks: an envelope is drawn per peak and a playhead per
  // fraction, and neither asks how long the material runs.
  function drawWaveform(
    canvas: HTMLCanvasElement,
    peaks: Float32Array,
    progress: number,
  ) {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0) return;
    canvas.width = Math.floor(rect.width * dpr);
    canvas.height = Math.floor(rect.height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const w = rect.width;
    const h = rect.height;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = "rgba(255,255,255,0.05)";
    ctx.fillRect(0, 0, w, h);
    const mid = h / 2;
    const barW = w / peaks.length;
    for (let i = 0; i < peaks.length; i++) {
      const p = peaks[i];
      const barH = Math.max(1, p * h * 0.9);
      const played = i / peaks.length < progress;
      ctx.fillStyle = played ? "#7dd3fc" : "rgba(255,255,255,0.32)";
      ctx.fillRect(i * barW, mid - barH / 2, Math.max(1, barW - 1), barH);
    }
    // playhead on top
    if (progress > 0 && progress <= 1) {
      ctx.fillStyle = "#f472b6";
      ctx.fillRect(progress * w - 1, 0, 2, h);
    }
  }

  function onAudioTimeUpdate() {
    if (!audioEl) return;
    audioDurationSec = Number.isFinite(audioEl.duration) ? audioEl.duration : 0;
    audioProgress =
      audioDurationSec > 0 ? audioEl.currentTime / audioDurationSec : 0;
  }

  function onWaveformClick(e: MouseEvent) {
    if (!audioEl || audioDurationSec <= 0) return;
    const canvas = e.currentTarget as HTMLCanvasElement;
    const rect = canvas.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    audioEl.currentTime = ratio * audioDurationSec;
  }


  // Fire the decode when detail flips to a fresh audio asset.
  $effect(() => {
    const d = detail;
    if (!d || d.asset.modality !== "audio") {
      waveformPeaks = null;
      waveformError = null;
      waveformDecoding = false;
      audioProgress = 0;
      audioDurationSec = 0;
      return;
    }
    const id = d.asset.id;
    const cached = waveformCache.get(id);
    if (cached === "error") {
      waveformPeaks = null;
      waveformError = "waveform unavailable";
      return;
    }
    if (cached instanceof Float32Array) {
      waveformPeaks = cached;
      waveformError = null;
      return;
    }
    const url = convertFileSrc(d.asset.locator);
    waveformDecoding = true;
    waveformError = null;
    waveformPeaks = null;
    void (async () => {
      try {
        const peaks = await decodeAudioPeaks(url);
        waveformCachePut(id, peaks);
        // Guard against navigation before decode returns.
        if (detail?.asset.id !== id) return;
        waveformPeaks = peaks;
      } catch (err) {
        waveformCachePut(id, "error");
        if (detail?.asset.id !== id) return;
        waveformError = err instanceof Error ? err.message : String(err);
        waveformPeaks = null;
      } finally {
        if (detail?.asset.id === id) waveformDecoding = false;
      }
    })();
  });

  // Redraw whenever peaks / progress / canvas mount changes.
  $effect(() => {
    if (!waveformCanvas || !waveformPeaks || !detail) return;
    drawWaveform(waveformCanvas, waveformPeaks, audioProgress);
  });

  // -------------------------------------------------------------------
  // Fullscreen zoom stage (image modality only)
  // -------------------------------------------------------------------
  // Third zoom stage for images: grid thumb (256) → detail (512 +
  // meta column) → full-window stage. Window-filling overlay, not OS
  // fullscreen; shows the original file, no chrome but a close hint.
  let fullscreen = $state(false);
  // `zoom` is a multiplier (1 = fit-to-view, 8 upper bound) and
  // (panX, panY) are pixel offsets applied to the image transform.
  // Wheel / ± keys drive zoom; drag drives pan whenever `zoom > 1`.
  // Navigating to another image or closing the stage resets everything
  // so each open starts at fit.
  let zoom = $state(1);
  let panX = $state(0);
  let panY = $state(0);
  let isPanning = $state(false);
  let panStartX = 0;
  let panStartY = 0;
  const ZOOM_MIN = 0.5;
  const ZOOM_MAX = 8;

  function resetZoom() {
    zoom = 1;
    panX = 0;
    panY = 0;
  }

  function applyZoom(next: number, cursorX?: number, cursorY?: number, rect?: DOMRect) {
    const clamped = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, next));
    if (clamped === zoom) return;
    // Anchor the zoom on the cursor so wheeling in toward a point
    // keeps that point visually stationary. Without the anchor the
    // image would just pump around its centre and the user has to
    // pan back every step.
    if (cursorX !== undefined && cursorY !== undefined && rect) {
      const cx = cursorX - rect.left - rect.width / 2;
      const cy = cursorY - rect.top - rect.height / 2;
      const ratio = clamped / zoom;
      panX = cx - (cx - panX) * ratio;
      panY = cy - (cy - panY) * ratio;
    }
    zoom = clamped;
    // Snap back to centre once we drop out of zoomed mode so a
    // fresh drag does not inherit an offscreen offset.
    if (zoom <= 1) {
      panX = 0;
      panY = 0;
    }
  }

  function onFullscreenWheel(event: WheelEvent) {
    event.preventDefault();
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    // Trackpad pinch reports small deltaY with ctrlKey; mouse wheel
    // reports larger absolute values. Both funnel through the same
    // multiplicative step so a pinch does not feel wildly different
    // from a scroll wheel.
    const step = Math.exp(-event.deltaY * 0.0035);
    applyZoom(zoom * step, event.clientX, event.clientY, rect);
  }

  function onFullscreenPointerDown(event: PointerEvent) {
    if (zoom <= 1) return;
    isPanning = true;
    panStartX = event.clientX - panX;
    panStartY = event.clientY - panY;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function onFullscreenPointerMove(event: PointerEvent) {
    if (!isPanning) return;
    panX = event.clientX - panStartX;
    panY = event.clientY - panStartY;
  }

  function onFullscreenPointerUp(event: PointerEvent) {
    if (!isPanning) return;
    isPanning = false;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  }

  $effect(() => {
    // Reset the zoom stage whenever we drop out of fullscreen or
    // switch to a different asset. Reading `fullscreen` and the
    // asset id registers both signals with the effect so either
    // transition retriggers it; the reset itself runs untracked so
    // its writes to `zoom` / `panX` / `panY` do not feed back in.
    void fullscreen;
    void detail?.asset.id;
    untrack(() => resetZoom());
  });

  // -------------------------------------------------------------------
  // Text mode + comments + labels + tags + notes state
  // -------------------------------------------------------------------
  // Detail-view text state. The detail overlay for non-image
  // modalities used to render only the cover snippet; the full body
  // is now fetched from `asset_texts` (the same endpoint the Reader
  // uses) so users can actually read / render the document without
  // dropping into the Reader stage.
  let detailText = $state<string | null>(null);
  let detailTextLoading = $state(false);
  let detailMode = $state<DetailMode>("md");
  let detailModeUserPicked = $state(false);

  // Inline edit drafts for the detail panel's Label chip strip and
  // Note textarea. `labelDraft` is the "+ add label" text input;
  // `noteDraft` shadows the persisted `register_note` while the User
  // types so blur can compare and save only on change.
  let labelDraft = $state("");
  let noteDraft = $state("");
  let noteSaving = $state(false);
  // `coverDraft` shadows the persisted `cover` (the tile's one-line
  // description) while the User edits it in the meta panel; blur
  // compares and saves only on change (mirrors `noteDraft`).
  // Description editing moved off the grid card to here.
  let coverDraft = $state("");
  let coverSaving = $state(false);

  // AssetComment thread — flat list rendered on the detail panel.
  let assetComments = $state<AssetCommentDto[]>([]);
  let commentDraft = $state("");
  let commentAuthorKind = $state<"user" | "persona">("user");
  let commentPosting = $state(false);

  // Tag input for the "attach tag" form.
  let newTagInput = $state("");
  // Promote-tag-to-group in-flight guard.
  let promotingTagId = $state<string | null>(null);

  // -------------------------------------------------------------------
  // Detail LRU cache
  // -------------------------------------------------------------------
  // First open pays the full three-fetch cost; second open populates
  // the overlay from cache instantly and re-fetches in the background
  // to catch server-side mutations. Insertion-order `Map` doubles as
  // an LRU: on hit we delete + re-set so the touched entry sits at
  // the tail.
  const DETAIL_CACHE_MAX = 30;
  const detailCache = new Map<string, DetailSnap>();
  function detailCachePut(id: string, snap: DetailSnap) {
    if (detailCache.has(id)) detailCache.delete(id);
    detailCache.set(id, snap);
    while (detailCache.size > DETAIL_CACHE_MAX) {
      const first = detailCache.keys().next().value;
      if (first === undefined) break;
      detailCache.delete(first);
    }
  }
  function detailCacheGet(id: string): DetailSnap | undefined {
    const snap = detailCache.get(id);
    if (snap === undefined) return undefined;
    detailCache.delete(id);
    detailCache.set(id, snap);
    return snap;
  }
  function detailCacheInvalidate(id: string) {
    detailCache.delete(id);
  }

  // Out-of-band asset changes (e.g. drag-drop into a group in the
  // sidebar) arrive via `assetPageCatalog.invalidations`. Purge the
  // cached snapshot so the next reopen re-fetches from the server.
  let lastInvalidationTick = 0;
  $effect(() => {
    const { id, tick } = assetPageCatalog.invalidations;
    if (tick > lastInvalidationTick && id) {
      lastInvalidationTick = tick;
      untrack(() => detailCacheInvalidate(id));
    }
  });

  // -------------------------------------------------------------------
  // openDetail / closeDetail — driven by the `openAssetId` prop
  // -------------------------------------------------------------------
  async function openDetail(assetId: string) {
    // Cache hit: populate state synchronously so the overlay paints
    // immediately, then re-fetch in the background to reconcile.
    const cached = detailCacheGet(assetId);
    // Kick off the Selector reverse-lookup independently of the cache
    // path — it always fires so the chip strip stays fresh even if
    // the detail body came from cache.
    detailSelections = [];
    void invoke<SnapshotDto[]>("list_snapshots_containing", {
      assetId,
      limit: 12,
    })
      .then((rows) => {
        if (detail && detail.asset.id === assetId) detailSelections = rows;
      })
      .catch(() => (detailSelections = []));
    // Reset the comment thread and (re)load it — cheap enough to
    // always refetch so a post from another surface / another
    // session shows up on reopen.
    assetComments = [];
    commentDraft = "";
    void loadAssetComments(assetId);
    // Fire the Provenance lineage fetch independently so the
    // detail-pane paints immediately and the Provenance section
    // fills in as soon as the 1-hop query returns.
    provenance = null;
    provenanceLoading = true;
    void invoke<LineageViewDto>("asset_lineage", {
      assetId,
      viewerSubject: null,
      depth: 4,
    })
      .then((view) => {
        if (detail?.asset.id !== assetId && provenance?.asset_id !== assetId) return;
        provenance = view;
      })
      .catch((e) => {
        console.warn("asset_lineage failed", e);
        provenance = null;
      })
      .finally(() => {
        provenanceLoading = false;
      });
    if (cached) {
      detail = cached.detail;
      noteDraft = cached.detail.asset.register_note ?? "";
      coverDraft = cached.detail.asset.cover ?? "";
      titleDraft = cached.detail.asset.title ?? "";
      labelDraft = "";
      // Members are not in the detail cache (they are their own query),
      // so the cached path has to fetch them too. Without this, any
      // reopen that hits the cache — including the one right after a
      // metadata edit invalidates and re-reads — showed a container
      // with "nothing filed in here yet".
      void loadMembers(cached.detail);
      detailGroupIds.clear();
      for (const id of cached.groupIds) detailGroupIds.add(id);
      detailText = cached.text;
      detailMode = cached.mode;
      detailModeUserPicked = false;
      detailLoading = false;
      detailTextLoading = false;
      // Background refresh — mutate cache + state if the fetch
      // reveals drift. Errors are swallowed (the cached view stays).
      void (async () => {
        try {
          const [primary, owning, texts] = await Promise.all([
            invoke<AssetDetailDto>("asset_detail", {
              query: { asset_id: assetId, viewer_subject: null },
            }),
            invoke<GroupDto[]>("groups_of_asset", { assetId }).catch(() => [] as GroupDto[]),
            invoke<AssetTextDto[]>("asset_texts", {
              assetIds: [assetId],
              viewerSubject: null,
            }).catch(() => [] as AssetTextDto[]),
          ]);
          if (!detail || detail.asset.id !== assetId) return;
          detail = primary;
          detailGroupIds.clear();
          for (const g of owning) detailGroupIds.add(g.id);
          // Same mime-first rule the render branch uses: an
          // unclassified media asset reads as `text` through the
          // modality lookup alone, and would have its body fetched
          // and cached for a pane that shows a player.
          const kind = mediaKind(primary.asset.media) ?? terminalKind(primary.asset.modality);
          let text: string | null = null;
          if (kind !== "image" && kind !== "video" && kind !== "audio") {
            text = texts.find((t) => t.asset_id === assetId)?.text ?? null;
            detailText = text;
            if (!detailModeUserPicked)
              detailMode = pickDetailMode(text, kind, primary.asset.labels);
          }
          detailCachePut(assetId, {
            detail: primary,
            groupIds: owning.map((g) => g.id),
            text,
            mode: detailMode,
          });
        } catch (error) {
          console.warn("detail refresh failed", error);
        }
      })();
      return;
    }
    detailLoading = true;
    detailText = null;
    detailTextLoading = true;
    detailModeUserPicked = false;
    // Fire all three fetches in parallel so the overlay paints as
    // soon as the fastest of them lands.
    const detailPromise = invoke<AssetDetailDto>("asset_detail", {
      query: { asset_id: assetId, viewer_subject: null },
    });
    const groupsPromise = invoke<GroupDto[]>("groups_of_asset", { assetId });
    // We can't tell whether the asset is text-shaped until
    // `detail` returns, so kick the text fetch off speculatively
    // and drop the result later for image / video / audio.
    const textsPromise = invoke<AssetTextDto[]>("asset_texts", {
      assetIds: [assetId],
      viewerSubject: null,
    }).catch((e) => {
      console.warn("asset_texts (detail) failed", e);
      return [] as AssetTextDto[];
    });
    try {
      const primary = await detailPromise;
      detail = primary;
      noteDraft = primary.asset.register_note ?? "";
      coverDraft = primary.asset.cover ?? "";
      titleDraft = primary.asset.title ?? "";
      labelDraft = "";
      void loadMembers(primary);
      const [owning, texts] = await Promise.all([
        groupsPromise.catch((e) => {
          console.warn("groups_of_asset failed", e);
          return [] as GroupDto[];
        }),
        textsPromise,
      ]);
      detailGroupIds.clear();
      for (const g of owning) {
        detailGroupIds.add(g.id);
      }
      let text: string | null = null;
      const primaryKind = primary
        ? (mediaKind(primary.asset.media) ?? terminalKind(primary.asset.modality))
        : null;
      if (
        primary &&
        primaryKind !== "image" &&
        primaryKind !== "video" &&
        primaryKind !== "audio"
      ) {
        text = texts.find((t) => t.asset_id === assetId)?.text ?? null;
        detailText = text;
        if (!detailModeUserPicked) {
          detailMode = pickDetailMode(text, primaryKind, primary.asset.labels);
        }
      }
      if (primary) {
        detailCachePut(assetId, {
          detail: primary,
          groupIds: owning.map((g) => g.id),
          text,
          mode: detailMode,
        });
      }
    } catch (error) {
      onSetStatus(`detail error: ${JSON.stringify(error)}`);
    } finally {
      detailLoading = false;
      detailTextLoading = false;
    }
  }

  /// Loads a container's members, oldest first — the order a session
  /// was lived in. Items get an empty list without a round trip.
  ///
  /// Only the persona scope is inherited; the grid's facets are
  /// deliberately not applied, for the same reason the reader drops
  /// them: "what is inside this container" is not "what is inside it
  /// that also matches my current search".
  async function loadMembers(primary: AssetDetailDto) {
    if (primary.asset.role !== "collection") {
      members = [];
      memberTexts = new Map();
      return;
    }
    membersLoading = true;
    try {
      const page = await invoke<AssetPageDto>("list_assets", {
        query: {
          viewer_subject: null,
          persona_id: primary.asset.persona_id,
          modality: null,
          occurred_from_ms: null,
          occurred_until_ms: null,
          tag_ids: [] as string[],
          group_ids: [] as string[],
          session_id: primary.asset.id,
          label: null,
          format: null,
          color: null,
          trash: "live",
          offset: 0,
          limit: 5000,
        },
      });
      members = [...page.items].sort(
        (a, b) => a.occurred_at_ms - b.occurred_at_ms,
      );
      // Bodies for the transcript. One batch call rather than per
      // message: a session is read whole, so the round trips would all
      // be paid anyway, just more slowly.
      const texts = await invoke<AssetTextDto[]>("asset_texts", {
        assetIds: members.map((m) => m.id),
        viewerSubject: null,
      }).catch((e) => {
        console.warn("member texts failed", e);
        return [] as AssetTextDto[];
      });
      memberTexts = new Map(texts.map((t) => [t.asset_id, t.text]));
    } catch (err) {
      console.warn("member list failed", err);
      members = [];
      memberTexts = new Map();
    } finally {
      membersLoading = false;
    }
  }

  /// Well-known chat role carried in the labels array (the parsers put
  /// it there); falls back to the modality slug, then to nothing.
  function memberRole(card: AssetCardDto): string {
    const known = ["user", "assistant", "system", "tool"];
    return card.labels.find((l) => known.includes(l)) ?? card.modality ?? "";
  }

  function closeDetailInternal() {
    detail = null;
    members = [];
    detailGroupIds.clear();
    fullscreen = false;
    detailText = null;
    detailTextLoading = false;
    detailModeUserPicked = false;
    provenance = null;
    provenanceLoading = false;
  }

  function requestClose() {
    onClose();
  }

  // Drive open/close from the `openAssetId` prop.
  $effect(() => {
    const target = openAssetId;
    const current = untrack(() => detail?.asset.id ?? null);
    const loading = untrack(() => detailLoading);
    if (target === null) {
      if (current !== null || loading) untrack(() => closeDetailInternal());
      return;
    }
    if (target === current) return;
    untrack(() => void openDetail(target));
  });

  // -------------------------------------------------------------------
  // Prev/next navigation while overlay (or fullscreen) is up
  // -------------------------------------------------------------------
  async function navigateDetail(delta: number) {
    if (currentPageItems.length === 0 || !detail || detailLoading) return;
    const currentId = detail.asset.id;
    const idx = currentPageItems.findIndex((it) => it.id === currentId);
    if (idx < 0) return;
    const n = currentPageItems.length;
    for (let step = 1; step <= n; step++) {
      const cand = currentPageItems[(((idx + delta * step) % n) + n) % n];
      if (cand.id === currentId) return;
      // In fullscreen, only image assets are eligible — skip non-image
      // candidates so an arrow press does not drop out of the stage.
      if (fullscreen && cand.modality !== "image") continue;
      onOpenAsset(cand.id);
      return;
    }
  }

  // External API — App's keyboard shortcut handler forwards arrow
  // key presses here so the fullscreen image-only filter above stays
  // authoritative (App does not know the fullscreen flag).
  export function navigate(delta: number) {
    void navigateDetail(delta);
  }

  // External API — App's Escape key handler needs to know whether
  // to close fullscreen (higher stage) or the detail overlay proper.
  export function isFullscreen(): boolean {
    return fullscreen;
  }
  export function exitFullscreen() {
    fullscreen = false;
  }
  export function isOpen(): boolean {
    return detail !== null || detailLoading;
  }
  export function getModality(): string | null {
    return detail?.asset.modality ?? null;
  }
  // Route the fullscreen-stage zoom shortcuts (F / +/- / 0/r / 1)
  // through DetailPane so `fullscreen` / `zoom` / `applyZoom` etc.
  // stay component-local. Returns true when the key was consumed.
  export function handleImageShortcut(key: string): boolean {
    // Toggle fullscreen for an image detail
    const isImage = detail?.asset.media === "image";
    if (key.toLowerCase() === "f" && isImage) {
      fullscreen = !fullscreen;
      return true;
    }
    if (!fullscreen || !isImage) return false;
    if (key === "+" || key === "=") {
      applyZoom(zoom * 1.25);
      return true;
    }
    if (key === "-" || key === "_") {
      applyZoom(zoom / 1.25);
      return true;
    }
    if (key === "0" || key.toLowerCase() === "r") {
      resetZoom();
      return true;
    }
    if (key === "1") {
      applyZoom(2);
      return true;
    }
    return false;
  }

  // -------------------------------------------------------------------
  // Model-proposed tag suggestions (#112). Loaded per asset; only the
  // open (`suggested`) rows render — a ruling removes the chip, and
  // the model never re-proposes a ruled pair.
  // -------------------------------------------------------------------
  let tagSuggestions: TagSuggestionDto[] = $state([]);
  const openTagSuggestions = $derived(
    tagSuggestions.filter((s) => s.disposition === "suggested"),
  );

  $effect(() => {
    const assetId = detail?.asset.id ?? null;
    untrack(() => {
      tagSuggestions = [];
      if (assetId !== null) void loadTagSuggestions(assetId);
    });
  });

  async function loadTagSuggestions(assetId: string) {
    try {
      const rows = await invoke<TagSuggestionDto[]>("list_tag_suggestions", {
        assetId,
      });
      if (detail?.asset.id === assetId) tagSuggestions = rows;
    } catch (error) {
      // A build without the feature answers empty, so an error here is
      // worth a console line but not a status banner.
      console.warn("list_tag_suggestions failed", error);
    }
  }

  async function acceptTagSuggestion(s: TagSuggestionDto) {
    if (!detail) return;
    try {
      await invoke("accept_tag_suggestion", {
        assetId: detail.asset.id,
        tagId: s.tag_id,
      });
      tagSuggestions = tagSuggestions.map((row) =>
        row.tag_id === s.tag_id ? { ...row, disposition: "accepted" } : row,
      );
      if (!detail.tags.some((t) => t.id === s.tag_id)) {
        detail.tags = [...detail.tags, { id: s.tag_id, name: s.name, axis: null }];
      }
      detailCacheInvalidate(detail.asset.id);
      await tagCatalog.loadCounts(activeFilter.activePersona);
      assetPageCatalog.invalidateDetail(detail.asset.id);
    } catch (error) {
      console.warn("accept_tag_suggestion failed", error);
      onSetStatus(`accept_tag_suggestion error: ${JSON.stringify(error)}`);
    }
  }

  async function rejectTagSuggestion(s: TagSuggestionDto) {
    if (!detail) return;
    try {
      await invoke("reject_tag_suggestion", {
        assetId: detail.asset.id,
        tagId: s.tag_id,
      });
      tagSuggestions = tagSuggestions.map((row) =>
        row.tag_id === s.tag_id ? { ...row, disposition: "rejected" } : row,
      );
    } catch (error) {
      console.warn("reject_tag_suggestion failed", error);
      onSetStatus(`reject_tag_suggestion error: ${JSON.stringify(error)}`);
    }
  }

  // -------------------------------------------------------------------
  // Tag actions
  // -------------------------------------------------------------------
  async function attachTagToDetail() {
    if (!detail) return;
    const name = newTagInput.trim();
    if (!name) return;
    try {
      const tag = await invoke<TagDto>("attach_tag", {
        command: { asset_id: detail.asset.id, name },
      });
      // Optimistic append; a full refetch runs behind the scenes in
      // case the server merged with an existing tag row.
      if (!detail.tags.some((t) => t.id === tag.id)) {
        detail.tags = [...detail.tags, tag];
      }
      detailCacheInvalidate(detail.asset.id);
      newTagInput = "";
      await tagCatalog.loadCounts(activeFilter.activePersona);
      assetPageCatalog.invalidateDetail(detail.asset.id);
    } catch (error) {
      console.warn("attach_tag failed", error);
      onSetStatus(`attach_tag error: ${JSON.stringify(error)}`);
    }
  }

  async function detachTagFromDetail(tagId: string) {
    if (!detail) return;
    try {
      await invoke("detach_tag", {
        command: { asset_id: detail.asset.id, tag_id: tagId },
      });
      detail.tags = detail.tags.filter((t) => t.id !== tagId);
      detailCacheInvalidate(detail.asset.id);
      await tagCatalog.loadCounts(activeFilter.activePersona);
      assetPageCatalog.invalidateDetail(detail.asset.id);
    } catch (error) {
      console.warn("detach_tag failed", error);
      onSetStatus(`detach_tag error: ${JSON.stringify(error)}`);
    }
  }

  function isTagPromoted(tagName: string): boolean {
    if (!detail) return false;
    const marker = `~${tagName}`;
    return groupCatalog.counts.data.some(
      (gc) =>
        gc.group.name === marker &&
        gc.group.persona_id === detail!.asset.persona_id,
    );
  }

  async function promoteTag(tagId: string, tagName: string) {
    if (!detail) return;
    if (promotingTagId) return;
    promotingTagId = tagId;
    const name = `~${tagName}`;
    try {
      const result = await invoke<PromoteTagToGroupResult>(
        "promote_tag_to_group",
        {
          command: {
            tag_id: tagId,
            persona_id: detail.asset.persona_id,
            name,
            description: null,
            dir_id: null,
          },
        },
      );
      onSetStatus(`▤ ${result.asset_count} assets → ${result.name}`);
      await groupCatalog.loadCounts(activeFilter.activePersona);
      await assetPageCatalog.reload();
    } catch (error) {
      console.warn("promote_tag_to_group failed", error);
      onSetStatus(`promote error: ${JSON.stringify(error)}`);
    } finally {
      promotingTagId = null;
    }
  }

  // -------------------------------------------------------------------
  // Group toggle
  // -------------------------------------------------------------------
  async function toggleAssetInGroup(assetId: string, groupId: string) {
    try {
      if (detailGroupIds.has(groupId)) {
        await mutate(
          "remove_asset_from_group",
          { command: { asset_id: assetId, group_id: groupId } },
          "remove this from the group",
        );
        detailGroupIds.delete(groupId);
      } else {
        // Both arms of the toggle go through `mutate`: one gesture, one
        // control, and a refusal that appeared on the way out but not on
        // the way in would be the harder half to explain.
        await mutate(
          "add_asset_to_group",
          { command: { asset_id: assetId, group_id: groupId } },
          "add this to the group",
        );
        detailGroupIds.add(groupId);
      }
      detailCacheInvalidate(assetId);
      await groupCatalog.loadCounts(activeFilter.activePersona);
      await assetPageCatalog.reload();
    } catch (error) {
      console.warn("toggleAssetInGroup failed", error);
    }
  }

  // -------------------------------------------------------------------
  // Labels
  // -------------------------------------------------------------------
  async function addLabel() {
    const name = labelDraft.trim();
    labelDraft = "";
    if (!name || !detail) return;
    const existing = detail.asset.labels;
    if (existing.includes(name)) return;
    await onSaveLabels(detail.asset.id, [...existing, name]);
    // App-side saveLabels updates detail.asset.labels through the
    // AssetDto response, but since detail is DetailPane-owned we
    // apply the mutation locally too.
    if (detail && detail.asset.id === detail.asset.id) {
      detail.asset = { ...detail.asset, labels: [...existing, name] };
    }
    detailCacheInvalidate(detail.asset.id);
    assetPageCatalog.invalidateDetail(detail.asset.id);
  }

  async function removeLabel(label: string) {
    if (!detail) return;
    const next = detail.asset.labels.filter((l) => l !== label);
    await onSaveLabels(detail.asset.id, next);
    if (detail) {
      detail.asset = { ...detail.asset, labels: next };
    }
    detailCacheInvalidate(detail.asset.id);
    assetPageCatalog.invalidateDetail(detail.asset.id);
  }

  // -------------------------------------------------------------------
  // AlbumMeta
  // -------------------------------------------------------------------

  // The declare verb answers with the whole row, so the new bag is
  // already in hand — the pane takes it rather than refetching what it
  // was just given. The two invalidations are the same pair every other
  // mutation here does: this pane's own LRU, and the card the grid holds.
  function applyDeclaredMeta(asset: AssetDto) {
    if (!detail || detail.asset.id !== asset.id) return;
    detail.asset = asset;
    detailCacheInvalidate(asset.id);
    assetPageCatalog.invalidateDetail(asset.id);
  }

  // -------------------------------------------------------------------
  // Note
  // -------------------------------------------------------------------
  async function saveNote() {
    if (!detail) return;
    const trimmed = noteDraft.trim();
    // `register_note: null` on the wire means "leave unchanged" per
    // the UpdateAssetMetaCommand semantics — the current UI has no
    // way to clear a note; a blank note gets rewritten to an empty
    // string via a follow-up commit if the User asks.
    if ((detail.asset.register_note ?? "") === trimmed) return;
    noteSaving = true;
    try {
      const dto = await invoke<AssetDto>("update_asset_meta", {
        command: {
          asset_id: detail.asset.id,
          labels: null,
          register_note: trimmed,
          cover: null,
          rating: null,
        },
      });
      if (detail && detail.asset.id === dto.id) {
        detail = {
          ...detail,
          asset: { ...detail.asset, register_note: dto.register_note },
        };
      }
      detailCacheInvalidate(dto.id);
      assetPageCatalog.invalidateDetail(dto.id);
    } catch (err) {
      console.warn("saveNote failed", err);
    } finally {
      noteSaving = false;
    }
  }

  // -------------------------------------------------------------------
  // Cover (Description) — the tile's one-line description. Edited here
  // (not on the grid card): the card is for browsing, the pane for
  // editing.
  // Mirrors `saveNote`: blur/⌘Enter compares against the persisted
  // value and only commits a change, then reflects it onto the grid
  // tile via `patchCard`.
  // -------------------------------------------------------------------
  async function saveCover() {
    if (!detail) return;
    const trimmed = coverDraft.trim();
    if ((detail.asset.cover ?? "") === trimmed) return;
    coverSaving = true;
    try {
      const dto = await invoke<AssetDto>("update_asset_meta", {
        command: {
          asset_id: detail.asset.id,
          labels: null,
          register_note: null,
          cover: trimmed,
          rating: null,
        },
      });
      if (detail && detail.asset.id === dto.id) {
        detail = {
          ...detail,
          asset: { ...detail.asset, cover: dto.cover },
        };
      }
      detailCacheInvalidate(dto.id);
      assetPageCatalog.invalidateDetail(dto.id);
      assetPageCatalog.patchCard(dto.id, { cover: dto.cover });
    } catch (err) {
      console.warn("saveCover failed", err);
    } finally {
      coverSaving = false;
    }
  }

  // -------------------------------------------------------------------
  // Title — what a person decided to call this, as opposed to `cover`
  // (derived text a job can regenerate). It carries the most weight on
  // a container: one owns no body to derive a cover from, so an unnamed
  // session borrows its first member's line and ends up called
  // something like "msg-1".
  // -------------------------------------------------------------------
  let titleDraft = $state("");
  let titleSaving = $state(false);
  async function saveTitle() {
    if (!detail) return;
    const trimmed = titleDraft.trim();
    if ((detail.asset.title ?? "") === trimmed) return;
    titleSaving = true;
    try {
      const dto = await invoke<AssetDto>("update_asset_meta", {
        command: {
          asset_id: detail.asset.id,
          labels: null,
          register_note: null,
          cover: null,
          rating: null,
          title: trimmed,
        },
      });
      if (detail && detail.asset.id === dto.id) {
        detail = { ...detail, asset: { ...detail.asset, title: dto.title } };
      }
      detailCacheInvalidate(dto.id);
      assetPageCatalog.invalidateDetail(dto.id);
      assetPageCatalog.patchCard(dto.id, { title: dto.title ?? null });
    } catch (err) {
      console.warn("saveTitle failed", err);
    } finally {
      titleSaving = false;
    }
  }

  // -------------------------------------------------------------------
  // Modality
  // -------------------------------------------------------------------
  let modalitySaving = $state(false);
  async function saveModality(newModality: string) {
    if (!detail) return;
    const trimmed = newModality.trim();
    if (!trimmed || detail.asset.modality === trimmed) return;
    modalitySaving = true;
    try {
      const dto = await invoke<AssetDto>("update_asset_meta", {
        command: {
          asset_id: detail.asset.id,
          labels: null,
          register_note: null,
          cover: null,
          rating: null,
          modality: trimmed,
        },
      });
      if (detail && detail.asset.id === dto.id) {
        detail = {
          ...detail,
          asset: { ...detail.asset, modality: dto.modality },
        };
      }
      detailCacheInvalidate(dto.id);
      assetPageCatalog.invalidateDetail(dto.id);
      assetPageCatalog.patchCard(dto.id, { modality: dto.modality });
      onRefreshCounts();
    } catch (err) {
      console.warn("saveModality failed", err);
    } finally {
      modalitySaving = false;
    }
  }

  // -------------------------------------------------------------------
  // Comments
  // -------------------------------------------------------------------
  async function loadAssetComments(assetId: string) {
    try {
      assetComments = await invoke<AssetCommentDto[]>("list_asset_comments", {
        assetId,
      });
    } catch {
      assetComments = [];
    }
  }

  async function postComment() {
    if (!detail) return;
    const body = commentDraft.trim();
    if (!body) return;
    let author_persona_id: string | null = null;
    if (commentAuthorKind === "persona") {
      // Persona post — pick the sidebar-active persona; fall back to
      // the Asset's owning persona so a post is still identifiable.
      author_persona_id = activeFilter.activePersona ?? detail.asset.persona_id;
    }
    commentPosting = true;
    try {
      const created = await invoke<AssetCommentDto>("post_asset_comment", {
        command: {
          asset_id: detail.asset.id,
          author_kind: commentAuthorKind,
          author_persona_id,
          body,
        },
      });
      assetComments = [...assetComments, created];
      commentDraft = "";
    } catch (err) {
      console.warn("post_asset_comment failed", err);
    } finally {
      commentPosting = false;
    }
  }

  async function deleteComment(commentId: string) {
    if (!detail) return;
    try {
      await mutate(
        "delete_asset_comment",
        { command: { comment_id: commentId } },
        "delete this comment",
      );
      assetComments = assetComments.filter((c) => c.id !== commentId);
    } catch (err) {
      console.warn("delete_asset_comment failed", err);
    }
  }

  // -------------------------------------------------------------------
  // Handlers wired directly to template
  // -------------------------------------------------------------------
  function handleAddTagChipClick(tag: { id: string; name: string }) {
    activeFilter.addTag(tag);
    requestClose();
  }

  function handleProvenanceChipClick(id: string) {
    requestClose();
    onOpenAsset(id);
  }

  function handleSelectionChipClick(sel: SnapshotDto) {
    requestClose();
    gridSelection.restore(sel);
  }

  // Chip label for a Snapshot-reverse-lookup row. A Snapshot is a
  // nameless content object (no rename surface), so the label
  // is a short id + creation time.
  function selectionRowLabel(sel: SnapshotDto): string {
    const short = sel.id.slice(0, 6);
    const d = new Date(sel.created_at_ms);
    const iso = Number.isFinite(d.getTime())
      ? d.toISOString().slice(5, 16).replace("T", " ")
      : "";
    return iso ? `${short} · ${iso}` : short;
  }

  async function handleSetAsWallpaper() {
    if (!detail || activeFilter.activePersona === null) return;
    await onSetAsWallpaper(detail.asset.id);
  }
</script>

{#if detail || detailLoading}
  <div
    class="detail-backdrop"
    onclick={requestClose}
    role="button"
    tabindex="-1"
    aria-label="Close detail"
  >
    <div class="detail-panel" onclick={(e) => e.stopPropagation()} role="dialog">
      <button class="detail-close" onclick={requestClose} aria-label="Close">✕</button>
      {#if detailLoading}
        <p class="detail-loading">loading…</p>
      {:else if detail}
        {@const extra = parseExtra(detail.asset)}
        <!-- The material's format fact decides which player to show;
             the modality-kind lookup is the fallback for legacy
             user slugs. Without the mime branch the `video` / `audio`
             cases below became unreachable in asset-model v4, which
             retired those slugs from the modality master (V38) —
             every unclassified video fell through to the text body. -->
        {@const detailKind = mediaKind(detail.asset.media) ?? terminalKind(detail.asset.modality)}
        <div class="detail-body">
          {#if isContainer}
            <!-- A container's body is its members, read in order. Same
                 transcript shape as the Reader overlay — the difference
                 is that here it sits beside the metadata pane, so a
                 session can be read and named in one place. -->
            {#if membersLoading}
              <p class="detail-loading">loading…</p>
            {:else if members.length === 0}
              <p class="detail-loading">nothing filed in here yet</p>
            {:else}
              <div class="transcript">
                {#each members as m (m.id)}
                  <article class="transcript-msg transcript-msg-{memberRole(m)}">
                    <header class="transcript-meta">
                      <span class="transcript-role">{memberRole(m)}</span>
                      <span class="transcript-time">{fmtDateTime(m.occurred_at_ms)}</span>
                    </header>
                    {#if memberTexts.get(m.id)}
                      <p class="transcript-text">{memberTexts.get(m.id)}</p>
                    {:else}
                      <p class="transcript-text transcript-fallback">
                        {m.cover ?? "(no body)"}
                      </p>
                    {/if}
                  </article>
                {/each}
              </div>
            {/if}
          {:else if detailKind === "image"}
            <div class="detail-media">
              <!-- Click-to-zoom into the full-window stage; the ⛶
                   button carries the same action for discoverability. -->
              <img
                class="detail-zoomable"
                src={thumbCatalog.detailSrc(
                  detail.asset.locator,
                  detail.asset.id,
                  detail.asset.media,
                )}
                alt={detail.asset.cover ?? ""}
                onclick={() => (fullscreen = true)}
                onerror={(e) => thumbCatalog.noteOriginalError(detail!.asset.id, e)}
              />
              <button
                class="detail-fullscreen-btn"
                onclick={() => (fullscreen = true)}
                title="View full window (Esc to close)"
                aria-label="View full window"
              >
                ⛶
              </button>
            </div>
          {:else if detailKind === "video"}
            <div class="detail-media detail-media-video">
              <!-- Native <video> through the Tauri asset protocol.
                   Formats the webview cannot display (VP9 WebM /
                   Matroska) play a transcoded H.264 rendition
                   instead; the backend owns that decision and this
                   pane just follows the status it reports. -->
              {#if videoPreview?.status === "ready" && videoPreview.path}
                <video
                  controls
                  preload="metadata"
                  bind:this={videoEl}
                  src={convertFileSrc(videoPreview.path)}
                >
                  <track kind="captions" />
                </video>
              {:else if videoPreview?.status === "not_needed"}
                <video
                  controls
                  preload="metadata"
                  bind:this={videoEl}
                  src={convertFileSrc(detail.asset.locator)}
                >
                  <track kind="captions" />
                </video>
              {:else if videoPreview?.status === "failed"}
                <p class="detail-video-note">
                  Preview transcode failed{videoPreview.detail
                    ? `: ${videoPreview.detail}`
                    : ""}
                </p>
              {:else if videoPreview?.status === "pending"}
                <p class="detail-video-note">
                  Preparing a playable preview… (transcoding)
                </p>
              {/if}
              <MaterialChapters
                assetId={detail.asset.id}
                durationMs={detail.asset.duration_ms}
                media={videoEl}
              />
              <MaterialMarks
                assetId={detail.asset.id}
                durationMs={detail.asset.duration_ms}
                media={videoEl}
              />
            </div>
          {:else if detailKind === "audio"}
            <div class="detail-media detail-media-audio">
              <!-- Envelope preview drawn from OfflineAudioContext-
                   decoded peaks. Click to seek. Native <audio controls>
                   stays underneath so scrub / volume / play still
                   work when the waveform is unavailable. Chapters are
                   `MaterialChapters` below, on their own ruler — they
                   are rows in a band that names who declared them, not
                   ticks this canvas can read off the asset. -->
              <div class="waveform-wrap">
                {#if waveformPeaks}
                  <canvas
                    class="waveform-canvas"
                    bind:this={waveformCanvas}
                    onclick={onWaveformClick}
                  ></canvas>
                {:else if waveformDecoding}
                  <div class="waveform-placeholder">decoding…</div>
                {:else if waveformError}
                  <div class="waveform-placeholder dim">
                    waveform unavailable ({waveformError})
                  </div>
                {:else}
                  <div class="waveform-placeholder dim">no waveform</div>
                {/if}
              </div>
              <audio
                controls
                preload="metadata"
                bind:this={audioEl}
                ontimeupdate={onAudioTimeUpdate}
                onloadedmetadata={onAudioTimeUpdate}
                src={convertFileSrc(detail.asset.locator)}
              ></audio>
              <MaterialChapters
                assetId={detail.asset.id}
                durationMs={detail.asset.duration_ms}
                media={audioEl}
              />
              <MaterialMarks
                assetId={detail.asset.id}
                durationMs={detail.asset.duration_ms}
                media={audioEl}
              />
              {#if detail.asset.cover}
                <p class="detail-audio-cover">{detail.asset.cover}</p>
              {/if}
            </div>
          {:else}
            <div class="detail-media detail-media-text">
              <!-- Render-mode toolbar. Chip strip mirrors the
                   Reader toggle so the interaction feels the same
                   across the two stages. -->
              <div class="detail-mode-strip">
                {#each ["md", "raw", "html", "term"] as mode (mode)}
                  <button
                    class="detail-mode-chip"
                    class:active={detailMode === mode}
                    onclick={() => {
                      detailMode = mode as DetailMode;
                      detailModeUserPicked = true;
                    }}
                  >
                    {mode}
                  </button>
                {/each}
              </div>
              <div class="detail-text-body">
                {#if detailTextLoading && detailText === null}
                  <p class="detail-loading">loading…</p>
                {:else if detailMode === "md"}
                  <div class="detail-md">
                    <!-- eslint-disable-next-line svelte/no-at-html-tags — sanitized via DOMPurify -->
                    {@html renderMarkdown(detailText ?? detail.asset.cover ?? "(no text)")}
                  </div>
                {:else if detailMode === "html"}
                  <!-- svelte-ignore a11y_missing_attribute -->
                  <iframe
                    class="detail-html"
                    sandbox="allow-same-origin"
                    srcdoc={detailText ?? detail.asset.cover ?? ""}
                  ></iframe>
                {:else if detailMode === "term"}
                  <pre class="detail-term">{detailText ?? detail.asset.cover ?? "(no text)"}</pre>
                {:else}
                  <pre class="detail-raw">{detailText ?? detail.asset.cover ?? "(no text)"}</pre>
                {/if}
              </div>
            </div>
          {/if}

          <aside class="detail-meta">
            <h3>{detail.asset.cover ?? detail.asset.modality}</h3>

            <dl>
              <dt>Description</dt>
              <dd class="cover-edit">
                <textarea
                  class="cover-input"
                  placeholder="One-line description (cover)…"
                  bind:value={coverDraft}
                  onblur={saveCover}
                  onkeydown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      e.preventDefault();
                      void saveCover();
                      (e.currentTarget as HTMLTextAreaElement).blur();
                    }
                  }}
                ></textarea>
                {#if coverSaving}
                  <span class="note-saving">saving…</span>
                {/if}
              </dd>
              <dt>Modality</dt>
              <dd>
                <select
                  class="detail-modality-select"
                  disabled={modalitySaving}
                  value={detail.asset.modality}
                  onchange={(e) => saveModality(e.currentTarget.value)}
                >
                  <!-- Options come from the Modality master (hidden
                       included) — no local slug duplication. When the
                       asset carries a slug the master doesn't know
                       (importer escape hatch), surface the current
                       value as a leading option so the <select> can
                       still round-trip it. -->
                  {#if !modalityCatalog.all.some((m) => m.slug === detail?.asset.modality)}
                    <option value={detail.asset.modality}>{detail.asset.modality}</option>
                  {/if}
                  {#each modalityCatalog.all as m (m.slug)}
                    <option value={m.slug}>{m.slug}</option>
                  {/each}
                </select>
              </dd>
              {#if isContainer}
                <!-- A container's content is its members, so the detail
                     of a session has to show them: without this the
                     pane is a cover line and nothing else. Chronological
                     — the order the session was lived in. -->
                <dt>Contents</dt>
                <dd class="member-list">
                  {#if membersLoading}
                    <span class="member-empty">loading…</span>
                  {:else if members.length === 0}
                    <span class="member-empty">nothing filed in here yet</span>
                  {:else}
                    {#each members as m, i (m.id)}
                      <button
                        type="button"
                        class="member-row"
                        onclick={() => onOpenAsset(m.id)}
                        title={m.cover ?? m.source_locator}
                      >
                        <span class="member-ord">{i + 1}</span>
                        <span class="member-cover">{m.cover ?? "(no cover yet)"}</span>
                      </button>
                    {/each}
                  {/if}
                </dd>
              {/if}
              <dt>Persona</dt><dd>{personaName(detail.asset.persona_id)}</dd>
              <dt>Occurred</dt><dd>{fmtDateTime(detail.asset.occurred_at_ms)}</dd>
              <dt>Ingested</dt><dd>{fmtDateTime(detail.asset.created_at_ms)}</dd>
              <dt>Source</dt><dd class="mono">{detail.asset.source_kind}</dd>
              <dt>Locator</dt><dd class="mono locator">{detail.asset.locator}</dd>
              {#if detail.asset.platform}
                <dt>Platform</dt><dd>{detail.asset.platform}</dd>
              {/if}
              {#if detail.asset.file_size_bytes != null}
                <dt>Size</dt><dd>{fmtBytes(detail.asset.file_size_bytes)}</dd>
              {/if}
              <!-- Attribution: who the row is by, and through which
                   agent (asterism-core `domain::attribution`). Each row
                   renders only when the value was asserted — an absent
                   author is *unrecorded*, and printing "owner" or
                   "unknown" in its place would show a claim nobody
                   made. `author_subject` is present exactly when
                   `author_kind` is "subject", so the fallback prints
                   the kind ("owner") rather than an empty cell. -->
              {#if detail.asset.author_kind}
                <dt>Author</dt>
                <dd>{detail.asset.author_subject ?? detail.asset.author_kind}</dd>
              {/if}
              {#if detail.asset.operator_ai}
                <dt>Operator</dt><dd class="mono">{detail.asset.operator_ai}</dd>
              {/if}
              <!-- Always editable, not just when already set: an
                   unnamed container is exactly the case that needs a
                   name, so hiding the field when `title` is null hid it
                   from everyone who wanted it. -->
              <dt>Title</dt>
              <dd class="note-edit">
                <input
                  class="title-input"
                  type="text"
                  placeholder={isContainer ? "Name this session…" : "Optional name…"}
                  bind:value={titleDraft}
                  disabled={titleSaving}
                  onblur={saveTitle}
                  onkeydown={(e) => {
                    if (e.key === "Enter") { e.preventDefault(); void saveTitle(); }
                  }}
                />
                {#if titleSaving}
                  <span class="note-saving">saving…</span>
                {/if}
              </dd>
              {#if detail.asset.container_id}
                <dt>Session</dt><dd class="mono">{detail.asset.container_id}</dd>
              {/if}
              <dt>Labels</dt>
              <dd class="labels-edit">
                <!-- label+index as the key, on the same grounds as the
                     card chips in App.svelte: a repeat would otherwise
                     be two equal keys, which Svelte throws on
                     (each_key_duplicate) rather than rendering twice.
                     The backend drops repeats on write and on read, so
                     this is the second line of defence, not the first.
                     Note the ✕ below removes by value, so a repeat that
                     did get through would lose both copies at once. -->
                {#each detail.asset.labels as label, i (`${label}:${i}`)}
                  <span class="label label-editable">
                    {label}
                    <button
                      type="button"
                      class="label-remove"
                      onclick={() => removeLabel(label)}
                      title={`Remove "${label}"`}
                      aria-label={`Remove ${label}`}
                    >✕</button>
                  </span>
                {/each}
                <input
                  class="label-add"
                  type="text"
                  placeholder="+ label"
                  bind:value={labelDraft}
                  onkeydown={(e) => {
                    if (e.key === "Enter") { e.preventDefault(); void addLabel(); }
                  }}
                  onblur={() => { if (labelDraft.trim()) void addLabel(); }}
                />
              </dd>

              <dt>Note</dt>
              <dd class="note-edit">
                <textarea
                  class="note-input"
                  placeholder="Short annotation (register-note)…"
                  bind:value={noteDraft}
                  onblur={saveNote}
                  onkeydown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      e.preventDefault();
                      void saveNote();
                      (e.currentTarget as HTMLTextAreaElement).blur();
                    }
                  }}
                ></textarea>
                {#if noteSaving}
                  <span class="note-saving">saving…</span>
                {/if}
              </dd>
              {#if detailSelections.length > 0}
                <dt>Snapshots</dt>
                <dd>
                  {#each detailSelections as sel (sel.id)}
                    <button
                      class="detail-selection-chip"
                      onclick={() => handleSelectionChipClick(sel)}
                      title="Restore this frozen pick into the grid multi-select"
                    >
                      {selectionRowLabel(sel)}
                      <span class="detail-selection-count">{sel.asset_ids.length}</span>
                    </button>
                  {/each}
                </dd>
              {/if}

              <!-- Provenance — multi-hop derived_from chain.
                   Ordered by distance so the lane reads as the route the
                   artefact took, not as an unordered neighbour set. -->
              {#if provenanceLoading || (provenance && (lineageAncestors.length > 0 || lineageDescendants.length > 0))}
                <dt>Provenance</dt>
                <dd class="provenance-container">
                  {#if provenanceLoading}
                    <div class="provenance-loading">loading lineage…</div>
                  {:else if provenance}
                    {#if lineageAncestors.length > 0}
                      <div class="provenance-lane">
                        <span class="provenance-lane-label">
                          ↑ derived from ({lineageAncestors.length})
                          {#if provenance.truncated}
                            <span class="provenance-truncated" title="the chain continues past the depth this view walked">
                              · more above
                            </span>
                          {/if}
                        </span>
                        <div class="provenance-lane-strip">
                          {#each lineageAncestors as node (node.card.id)}
                            <button
                              type="button"
                              class="provenance-chip"
                              onclick={() => handleProvenanceChipClick(node.card.id)}
                              title={`${node.depth} hop${node.depth === 1 ? "" : "s"} up — ${node.card.cover ?? node.card.source_locator}`}
                            >
                              <span class="provenance-chip-hop">{node.depth}</span>
                              <span class="provenance-chip-cover">
                                {node.card.cover ?? node.card.source_locator.split("/").pop() ?? node.card.id.slice(0, 8)}
                              </span>
                            </button>
                          {/each}
                        </div>
                      </div>
                    {/if}
                    {#if lineageDescendants.length > 0}
                      <div class="provenance-lane">
                        <span class="provenance-lane-label">
                          ↓ derived into ({lineageDescendants.length})
                        </span>
                        <div class="provenance-lane-strip">
                          {#each lineageDescendants as node (node.card.id)}
                            <button
                              type="button"
                              class="provenance-chip"
                              onclick={() => handleProvenanceChipClick(node.card.id)}
                              title={`${-node.depth} hop${node.depth === -1 ? "" : "s"} down — ${node.card.cover ?? node.card.source_locator}`}
                            >
                              <span class="provenance-chip-hop">{-node.depth}</span>
                              <span class="provenance-chip-cover">
                                {node.card.cover ?? node.card.source_locator.split("/").pop() ?? node.card.id.slice(0, 8)}
                              </span>
                            </button>
                          {/each}
                        </div>
                      </div>
                    {/if}
                  {/if}
                </dd>
              {/if}

              <!-- Source type — what the file's origin rests on: the
                   container's evidence, or the person's assertion over
                   it (#108). Beside the other statements because an
                   assertion is one; the disclosure signs whichever
                   voice wins. -->
              <SourceTypeRow
                assetId={detail.asset.id}
                onChanged={applyDeclaredMeta}
              />

              <!-- AlbumMeta — what somebody said about this row, under a
                   name they chose. In the statements neighbourhood —
                   Provenance, Source type — because all three are
                   statements rather than readings, and apart from
                   Provenance because a claim draws an edge and this
                   draws nothing. -->
              <AlbumMetaSection
                assetId={detail.asset.id}
                statements={readAlbumMeta(parseExtra(detail.asset))}
                onChanged={applyDeclaredMeta}
              />

              <dt>Thread</dt>
              <dd class="thread-container">
                <ul class="thread-list">
                  {#each assetComments as c (c.id)}
                    <li class="thread-post" class:persona={c.author_kind === "persona"}>
                      <div class="thread-post-head">
                        <span class="thread-author">
                          {#if c.author_kind === "user"}
                            You
                          {:else}
                            {personaName(c.author_persona_id ?? "")}
                          {/if}
                        </span>
                        <span class="thread-when">
                          {fmtDateTime(c.created_at_ms)}
                          {#if c.edited_at_ms !== null}
                            · edited {fmtDateTime(c.edited_at_ms)}
                          {/if}
                        </span>
                        <button
                          type="button"
                          class="thread-delete"
                          onclick={() => deleteComment(c.id)}
                          title="Delete"
                          aria-label="Delete comment"
                        >✕</button>
                      </div>
                      <p class="thread-body">{c.body}</p>
                    </li>
                  {/each}
                  {#if assetComments.length === 0}
                    <li class="thread-empty">No comments yet.</li>
                  {/if}
                </ul>
                <div class="thread-compose">
                  <div class="thread-author-toggle">
                    <button
                      type="button"
                      class:active={commentAuthorKind === "user"}
                      onclick={() => (commentAuthorKind = "user")}
                    >as You</button>
                    <button
                      type="button"
                      class:active={commentAuthorKind === "persona"}
                      disabled={activeFilter.activePersona === null}
                      onclick={() => (commentAuthorKind = "persona")}
                      title={activeFilter.activePersona
                        ? `as ${personaName(activeFilter.activePersona)}`
                        : "Pick a persona in the sidebar to post as one"}
                    >as Persona</button>
                  </div>
                  <textarea
                    class="thread-input"
                    placeholder={commentAuthorKind === "user"
                      ? "Add a note as You…"
                      : `Add a note as ${activeFilter.activePersona ? personaName(activeFilter.activePersona) : "Persona"}…`}
                    bind:value={commentDraft}
                    onkeydown={(e) => {
                      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                        e.preventDefault();
                        void postComment();
                      }
                    }}
                  ></textarea>
                  <div class="thread-actions">
                    <button
                      type="button"
                      class="thread-post-btn"
                      onclick={postComment}
                      disabled={commentPosting || !commentDraft.trim()}
                    >
                      {commentPosting ? "posting…" : "Post"}
                    </button>
                  </div>
                </div>
              </dd>

              {#if detailKind === "image"}
                <!-- The stored columns, not `extra.dims`. That key was
                     read here for two waves and written by nothing: the
                     image parser puts the pair on the Footprint field,
                     never in its bag, so the row never rendered. It also
                     printed `JSON.stringify`, which would have shown
                     `{"0":3024,"1":4032}` had anything ever populated it.
                     V69 put the pair on the asset, so this reads the
                     value rather than a would-be copy of it — the bag
                     stays for what has no column (see `Camera` below). -->
                {#if fmtDimensions(detail.asset.width_px, detail.asset.height_px)}
                  <dt>Dimensions</dt>
                  <dd>{fmtDimensions(detail.asset.width_px, detail.asset.height_px)}</dd>
                {/if}
                {#if extra.camera_make || extra.camera_model}
                  <dt>Camera</dt>
                  <dd>{[extra.camera_make, extra.camera_model].filter(Boolean).join(" ")}</dd>
                {/if}
                {#if extra.orientation != null}
                  <dt>Orientation</dt><dd>{extra.orientation}</dd>
                {/if}
                {#if extra.exif_seen === false}
                  <dt>EXIF</dt><dd class="dim">not present (used file mtime)</dd>
                {/if}
              {/if}

              {#if detailKind === "video"}
                <!-- Same move as the image arm one block up. The value
                     shown is the *coded* pair: no probe reads the mp4
                     display matrix or Matroska's DisplayWidth, so an
                     upright phone clip reads as `1920 × 1080` and there
                     is no Orientation row here to say otherwise. That is
                     a gap in what gets measured; showing the measurement
                     is still better than showing nothing, which is what
                     this row did before. -->
                {#if fmtDimensions(detail.asset.width_px, detail.asset.height_px)}
                  <dt>Dimensions</dt>
                  <dd>{fmtDimensions(detail.asset.width_px, detail.asset.height_px)}</dd>
                {/if}
                {#if detail.asset.duration_ms != null}
                  <dt>Duration</dt><dd>{fmtDurationMs(detail.asset.duration_ms)}</dd>
                {/if}
                {#if extra.codec}
                  <dt>Codec</dt><dd class="mono">{extra.codec}</dd>
                {/if}
                <!-- No `Frame rate` row. `extra.framerate` was read here
                     and set by nobody: the parser pins the field to
                     `None` because neither probe exposes it cheaply
                     (`importer-video/src/parser.rs`). A row that cannot
                     fire is not a neutral leftover — restored, it would
                     read as "this clip has no frame rate". It comes back
                     when something measures one. -->
              {/if}

              <!-- The three rows below were dead on the same terms as
                   the video ones, and for a longer reach: the audio
                   probe measures codec / sample rate / channels into
                   `Footprint::Audio`, and `audio_to_spec` was dropping
                   the last two outright while turning the first into a
                   `codec:<slug>` label. The values existed and reached
                   nothing. The parser records them in its bag now. -->
              {#if detailKind === "audio"}
                {#if detail.asset.duration_ms != null}
                  <dt>Duration</dt><dd>{fmtDurationMs(detail.asset.duration_ms)}</dd>
                {/if}
                {#if extra.codec}
                  <dt>Codec</dt><dd class="mono">{extra.codec}</dd>
                {/if}
                {#if extra.sample_rate != null}
                  <dt>Sample rate</dt><dd>{extra.sample_rate} Hz</dd>
                {/if}
                {#if extra.channels != null}
                  <dt>Channels</dt><dd>{extra.channels === 1 ? "mono" : extra.channels === 2 ? "stereo" : `${extra.channels}ch`}</dd>
                {/if}
              {/if}
            </dl>

            {#if detailKind === "image" && activeFilter.activePersona !== null}
              <!-- Persona theme action — only for image assets and
                   only when a persona is selected. -->
              <div class="wallpaper-action">
                <button
                  type="button"
                  class="wallpaper-btn"
                  onclick={handleSetAsWallpaper}
                  title={`Use this image as ${personaName(activeFilter.activePersona)}'s wallpaper`}
                >
                  ▨ Set as {personaName(activeFilter.activePersona)} wallpaper
                </button>
                {#if themeCatalog.theme?.wallpaper_asset_id === detail.asset.id}
                  <span class="wallpaper-current">· current wallpaper</span>
                {/if}
              </div>
            {/if}

            <h4>Tags</h4>
            <div class="tags">
              {#each detail.tags as t (t.id)}
                <span class="tag-chip-group">
                  <button
                    type="button"
                    class="label label-tag"
                    class:label-tag-active={activeFilter.activeTagIds.has(t.id)}
                    onclick={() => handleAddTagChipClick(t)}
                    title={activeFilter.activeTagIds.has(t.id)
                      ? `Already filtering by #${t.name}`
                      : `Add #${t.name} to grid filter`}
                  >
                    # {t.name}
                  </button>
                  <button
                    type="button"
                    class="tag-chip-action tag-chip-promote"
                    class:tag-chip-promoted={isTagPromoted(t.name)}
                    onclick={() => promoteTag(t.id, t.name)}
                    title={isTagPromoted(t.name)
                      ? `Already promoted to Group ~${t.name}`
                      : `Promote #${t.name} into a Group (▤)`}
                    aria-label={isTagPromoted(t.name)
                      ? `Tag ${t.name} is already promoted`
                      : `Promote tag ${t.name} into a Group`}
                  >{isTagPromoted(t.name) ? "✓" : "▤"}</button>
                  <button
                    type="button"
                    class="tag-chip-action tag-chip-detach"
                    onclick={() => detachTagFromDetail(t.id)}
                    title={`Remove #${t.name} from this asset`}
                    aria-label={`Detach tag ${t.name}`}
                  >×</button>
                </span>
              {/each}
            </div>
            <form
              class="tag-add-row"
              onsubmit={(e) => {
                e.preventDefault();
                void attachTagToDetail();
              }}
            >
              <input
                type="text"
                class="tag-add-input"
                placeholder="add tag…"
                bind:value={newTagInput}
              />
              <button type="submit" class="tag-add-btn" title="Attach tag" aria-label="Attach tag">+</button>
            </form>

            {#if openTagSuggestions.length > 0}
              <h4>Suggested</h4>
              <div class="tags">
                {#each openTagSuggestions as s (s.tag_id)}
                  <span class="tag-chip-group">
                    <span
                      class="label label-tag tag-suggestion-chip"
                      title={`Proposed by ${s.model_id} at ${s.score.toFixed(2)}`}
                    >
                      # {s.name}
                      <span class="tag-suggestion-score">{s.score.toFixed(2)}</span>
                    </span>
                    <button
                      type="button"
                      class="tag-chip-action tag-suggestion-accept"
                      onclick={() => acceptTagSuggestion(s)}
                      title={`Accept #${s.name}`}
                      aria-label={`Accept suggested tag ${s.name}`}
                    >✓</button>
                    <button
                      type="button"
                      class="tag-chip-action tag-chip-detach"
                      onclick={() => rejectTagSuggestion(s)}
                      title={`Reject #${s.name} — this model will not propose it again`}
                      aria-label={`Reject suggested tag ${s.name}`}
                    >×</button>
                  </span>
                {/each}
              </div>
            {/if}

            {#if groupCatalog.counts.data.length > 0}
              <h4>Groups</h4>
              <div class="tags">
                {#each groupCatalog.counts.data as gc (gc.group.id)}
                  <button
                    type="button"
                    class="label label-tag group-chip-detail"
                    class:label-tag-active={detailGroupIds.has(gc.group.id)}
                    onclick={() => toggleAssetInGroup(detail!.asset.id, gc.group.id)}
                    title={detailGroupIds.has(gc.group.id)
                      ? `Remove this asset from ~${gc.group.name}`
                      : `Add this asset to ~${gc.group.name}`}
                  >
                    ~ {gc.group.name}
                  </button>
                {/each}
              </div>
            {/if}

            <!-- Last of the column, because it is the one act here that
                 sends this asset somewhere else. Everything above says
                 what the asset is to this library; this hands it to a
                 team. It owns itself for the reason `MaterialMarks`
                 does — what it needs from this pane is the id and a
                 name to start from, and the three ids a promotion goes
                 to are the shared catalog's. -->
            <PromoteToTeam
              assetId={detail.asset.id}
              defaultName={detail.asset.title ?? baseName(detail.asset.locator)}
            />
          </aside>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if fullscreen && detail && detail.asset.media === "image"}
  <!-- Full-window image stage (zoom stage 3). Original file, no
       meta chrome; any click or Esc drops back to the detail
       overlay underneath. -->
  <div
    class="fullscreen-backdrop"
    class:panning={isPanning}
    class:zoomed={zoom > 1}
    onclick={() => {
      if (zoom > 1) return;
      fullscreen = false;
    }}
    ondblclick={() => resetZoom()}
    onwheel={onFullscreenWheel}
    onpointerdown={onFullscreenPointerDown}
    onpointermove={onFullscreenPointerMove}
    onpointerup={onFullscreenPointerUp}
    onpointercancel={onFullscreenPointerUp}
    role="button"
    tabindex="-1"
    aria-label="Close full-window view"
  >
    <img
      src={convertFileSrc(detail.asset.locator)}
      alt={detail.asset.cover ?? ""}
      style={`transform: translate(${panX}px, ${panY}px) scale(${zoom});`}
      draggable="false"
    />
    <button
      class="fullscreen-nav fullscreen-nav-prev"
      onclick={(e) => {
        e.stopPropagation();
        navigateDetail(-1);
      }}
      aria-label="Previous image"
    >
      ‹
    </button>
    <button
      class="fullscreen-nav fullscreen-nav-next"
      onclick={(e) => {
        e.stopPropagation();
        navigateDetail(1);
      }}
      aria-label="Next image"
    >
      ›
    </button>
    <span class="fullscreen-hint">
      {#if zoom > 1}
        {(zoom * 100).toFixed(0)}% · drag to pan · dbl-click / 0 to reset · Esc to close
      {:else}
        ← → next · wheel / + − to zoom · Esc / click to close
      {/if}
    </span>
  </div>
{/if}

<style>
  /* -----------------------------------------------------------------
   * Detail overlay backdrop / panel chrome
   * ----------------------------------------------------------------- */
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

  .detail-panel {
    background: #fbfbf9;
    border-radius: 10px;
    width: min(96vw, 1200px);
    max-height: 96vh;
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
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

  .detail-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    gap: 0;
    min-height: 0;
    height: 100%;
  }

  /* -----------------------------------------------------------------
   * Media containers per modality
   * ----------------------------------------------------------------- */
  .detail-media {
    background: #1a1a1a;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    min-height: 300px;
    max-height: 96vh;
    position: relative;
  }
  .detail-media img {
    max-width: 100%;
    max-height: 96vh;
    object-fit: contain;
    display: block;
  }
  .detail-zoomable {
    cursor: zoom-in;
  }
  .detail-fullscreen-btn {
    position: absolute;
    right: 0.6rem;
    bottom: 0.6rem;
    width: 1.9rem;
    height: 1.9rem;
    border: 1px solid rgba(255, 255, 255, 0.35);
    border-radius: 6px;
    background: rgba(20, 18, 30, 0.55);
    color: #eee;
    font-size: 0.9rem;
    cursor: pointer;
    line-height: 1;
  }
  .detail-fullscreen-btn:hover {
    background: rgba(20, 18, 30, 0.8);
  }

  .detail-media-video {
    background: #0c0b12;
    display: flex;
    /* Column since the marks panel joined the player: the two stack,
       the way the waveform and its chapter chips already do in the
       audio branch below. */
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 1rem;
    gap: 0.6rem;
  }
  .detail-media-video video {
    max-width: 100%;
    max-height: 100%;
    /* Without this the player refuses to shrink below its intrinsic
       height in a column flex container and pushes the marks panel out
       of the clipped `.detail-media` box. */
    min-height: 0;
    border-radius: 4px;
  }
  .detail-video-note {
    margin: 0;
    font-size: 0.85rem;
    color: #9a96b0;
    text-align: center;
    max-width: 480px;
  }
  .detail-media-audio {
    background: #f4f2ea;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 2rem 1.5rem;
    gap: 0.8rem;
  }
  .detail-media-audio audio {
    width: 100%;
    max-width: 480px;
  }
  .detail-audio-cover {
    margin: 0;
    font-size: 0.85rem;
    color: #555;
    text-align: center;
    max-width: 480px;
    white-space: pre-wrap;
  }

  /* -----------------------------------------------------------------
   * Waveform (audio modality)
   * ----------------------------------------------------------------- */
  .waveform-wrap {
    width: 100%;
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .waveform-canvas {
    width: 100%;
    height: 72px;
    background: #1e1a24;
    border-radius: 4px;
    cursor: pointer;
    display: block;
  }
  .waveform-placeholder {
    width: 100%;
    height: 72px;
    background: #1e1a24;
    color: rgba(255, 255, 255, 0.72);
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.8rem;
  }
  .waveform-placeholder.dim {
    color: rgba(255, 255, 255, 0.35);
  }
  /* -----------------------------------------------------------------
   * Text mode + render modes
   * ----------------------------------------------------------------- */
  .detail-media-text {
    background: #fbfbf9;
    padding: 0;
    align-items: stretch;
    justify-content: flex-start;
    overflow: hidden;
    flex-direction: column;
    display: flex;
  }
  .detail-mode-strip {
    display: flex;
    gap: 0.25rem;
    padding: 0.4rem 0.6rem;
    border-bottom: 1px solid #e2e2de;
    background: #f4f4f0;
    flex-shrink: 0;
  }
  .detail-mode-chip {
    padding: 0.15rem 0.55rem;
    border: 1px solid #d0d0d0;
    border-radius: 3px;
    background: #fafafa;
    cursor: pointer;
    font-size: 0.72rem;
    color: #555;
    text-transform: lowercase;
    font-family: ui-monospace, "SF Mono", monospace;
  }
  .detail-mode-chip:hover {
    background: #eee;
  }
  .detail-mode-chip.active {
    background: #6c58c3;
    border-color: #6c58c3;
    color: #fff;
  }
  .detail-text-body {
    padding: 1.2rem 1.5rem;
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  .detail-media-text pre {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    color: #333;
    font-family: inherit;
  }
  .detail-term {
    font-family: ui-monospace, "SF Mono", "Menlo", monospace !important;
    font-size: 0.82rem !important;
    background: #14161c;
    color: #f4f4f8 !important;
    padding: 0.9rem 1.1rem;
    border-radius: 4px;
    line-height: 1.55 !important;
    text-shadow: 0 0 1px rgba(0, 0, 0, 0.35);
  }
  .detail-raw {
    font-family: ui-monospace, "SF Mono", monospace !important;
    font-size: 0.8rem !important;
  }
  .detail-html {
    width: 100%;
    height: 100%;
    min-height: 60vh;
    border: 1px solid #e2e2de;
    border-radius: 4px;
    background: #fff;
  }
  .detail-md {
    font-size: 0.9rem;
    line-height: 1.55;
    color: #222;
  }
  .detail-md :global(h1),
  .detail-md :global(h2),
  .detail-md :global(h3) {
    margin: 1em 0 0.4em;
  }
  .detail-md :global(pre) {
    background: #f4f4f0;
    padding: 0.7rem 0.9rem;
    border-radius: 4px;
    overflow-x: auto;
    font-family: ui-monospace, "SF Mono", monospace;
    font-size: 0.8rem;
  }
  .detail-md :global(code) {
    font-family: ui-monospace, "SF Mono", monospace;
    background: #f4f4f0;
    padding: 0.1em 0.35em;
    border-radius: 3px;
    font-size: 0.85em;
  }
  .detail-md :global(pre code) {
    background: none;
    padding: 0;
  }
  .detail-md :global(table) {
    border-collapse: collapse;
    margin: 0.8em 0;
  }
  .detail-md :global(th),
  .detail-md :global(td) {
    border: 1px solid #d9d5c8;
    padding: 0.35em 0.6em;
    text-align: left;
  }
  .detail-md :global(blockquote) {
    border-left: 3px solid #b8afef;
    margin: 0.6em 0;
    padding: 0.15em 0 0.15em 0.9em;
    color: #555;
  }

  /* -----------------------------------------------------------------
   * Meta column (labels / tags / groups / note / provenance / thread)
   * ----------------------------------------------------------------- */
  .detail-meta {
    border-left: 1px solid #e2e2de;
    padding: 1.2rem 1rem;
    overflow-y: auto;
    background: #fbfbf9;
  }
  .detail-meta h3 {
    font-size: 0.95rem;
    margin: 0 0 0.8rem;
    color: #111;
    padding-right: 2rem;
    word-break: break-word;
  }
  .detail-meta h4 {
    font-size: 0.7rem;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 1rem 0 0.4rem;
  }
  .detail-meta dl {
    display: grid;
    grid-template-columns: 5rem 1fr;
    gap: 0.35rem 0.6rem;
    font-size: 0.75rem;
    margin: 0;
  }
  .detail-meta dt {
    color: #888;
    font-weight: 500;
  }
  .detail-meta dd {
    color: #222;
    margin: 0;
    word-break: break-word;
  }
  .detail-modality-select {
    font-size: 0.75rem;
    font-family: inherit;
    background: #f0f0ec;
    color: #333;
    border: 1px solid #d0d0c8;
    border-radius: 3px;
    padding: 1px 4px;
    cursor: pointer;
  }
  .detail-modality-select:hover:not(:disabled) {
    background: #e6e6e0;
    border-color: #b0b0a0;
  }
  .detail-meta dd.mono,
  .detail-meta dd .mono {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 0.7rem;
  }
  .detail-meta dd.locator {
    word-break: break-all;
    color: #555;
  }
  .detail-meta dd.dim {
    color: #999;
  }
  .detail-meta .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
  }

  /* A suggestion is not a tag the asset has: the dashed border keeps
     the distinction visible at a glance, and the score rides along so
     ruling on a weak match is an informed act. */
  .tag-suggestion-chip {
    border-style: dashed;
    opacity: 0.85;
    cursor: default;
  }

  .tag-suggestion-score {
    font-size: 0.75em;
    opacity: 0.65;
    margin-left: 0.25rem;
  }

  .tag-suggestion-accept {
    color: var(--accent, #7bd88f);
  }

  /* -----------------------------------------------------------------
   * Label inline edit
   * ----------------------------------------------------------------- */
  .label-editable {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.35rem;
    font-size: 0.65rem;
  }
  .label-remove {
    background: none;
    border: none;
    padding: 0;
    color: #b7b1e5;
    font-size: 0.7rem;
    cursor: pointer;
    line-height: 1;
  }
  .label-remove:hover {
    color: #d0393b;
  }
  .label-add {
    font-size: 0.7rem;
    padding: 0.15rem 0.45rem;
    min-width: 6rem;
    background: #fafafd;
    border: 1px dashed #c6c0e8;
    border-radius: 3px;
    outline: none;
    color: inherit;
  }
  .label-add:focus {
    border-style: solid;
    border-color: #8a86ff;
    background: #ffffff;
  }

  /* -----------------------------------------------------------------
   * Note textarea
   * ----------------------------------------------------------------- */
  /* Transcript — the container's body. Mirrors the Reader overlay's
     message shape so a session looks the same wherever it is read. */
  .transcript {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.2rem;
  }
  .transcript-msg {
    border-left: 3px solid #e2e0f0;
    border-radius: 0 6px 6px 0;
    padding: 0.5rem 0.7rem;
    background: #fbfbfd;
  }
  .transcript-msg-user {
    border-left-color: #b5b1e2;
    background: #f6f5fc;
  }
  .transcript-msg-assistant {
    border-left-color: #9ed0b8;
    background: #fbfdfc;
  }
  .transcript-meta {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    margin-bottom: 0.3rem;
  }
  .transcript-role {
    font-size: 0.68rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: #8a86ab;
  }
  .transcript-time {
    font-size: 0.68rem;
    color: #bbb;
    font-variant-numeric: tabular-nums;
  }
  .transcript-text {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
    color: #333;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .transcript-fallback {
    color: #888;
  }

  .title-input {
    width: 100%;
    box-sizing: border-box;
    padding: 0.35rem 0.5rem;
    font-size: 0.85rem;
    font-family: inherit;
    background: #fafafd;
    border: 1px solid #e2e0f0;
    border-radius: 5px;
    color: #333;
  }
  .title-input:focus {
    outline: none;
    border-color: #b5b1e2;
  }

  /* Container contents — a compact index, not a reader. Clicking a row
     opens that member's own detail; reading the session through goes
     through the Reader overlay. */
  .member-list {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    max-height: 14rem;
    overflow-y: auto;
  }
  .member-empty {
    font-size: 0.78rem;
    color: #aaa;
  }
  .member-row {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    width: 100%;
    padding: 0.2rem 0.3rem;
    background: none;
    border: none;
    border-radius: 4px;
    font-family: inherit;
    font-size: 0.8rem;
    color: #555;
    text-align: left;
    cursor: pointer;
  }
  .member-row:hover {
    background: #efefe9;
  }
  .member-ord {
    flex: 0 0 auto;
    min-width: 1.2rem;
    color: #b5b1e2;
    font-variant-numeric: tabular-nums;
  }
  .member-cover {
    flex: 1 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .note-edit {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .note-input {
    width: 100%;
    box-sizing: border-box;
    min-height: 4rem;
    padding: 0.4rem 0.55rem;
    font-size: 0.82rem;
    font-family: inherit;
    line-height: 1.4;
    background: #fafafd;
    border: 1px solid #d6d3ec;
    border-radius: 4px;
    outline: none;
    color: inherit;
    resize: vertical;
  }
  .note-input:focus {
    border-color: #8a86ff;
    background: #ffffff;
  }
  .note-saving {
    font-size: 0.65rem;
    color: #7a76c9;
  }

  /* Cover (Description) editor — same tone as the Note editor but a
     shorter default height (a cover is a one-liner). */
  .cover-edit {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .cover-input {
    width: 100%;
    box-sizing: border-box;
    min-height: 2.6rem;
    padding: 0.4rem 0.55rem;
    font-size: 0.82rem;
    font-family: inherit;
    line-height: 1.4;
    background: #fafafd;
    border: 1px solid #d6d3ec;
    border-radius: 4px;
    outline: none;
    color: inherit;
    resize: vertical;
  }
  .cover-input:focus {
    border-color: #8a86ff;
    background: #ffffff;
  }

  /* -----------------------------------------------------------------
   * Comment thread
   * ----------------------------------------------------------------- */
  .thread-container {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .thread-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-height: 280px;
    overflow-y: auto;
  }
  .thread-post {
    padding: 0.4rem 0.6rem;
    border-left: 3px solid #5850ff;
    background: #f5f4ff;
    border-radius: 3px;
  }
  .thread-post.persona {
    border-left-color: #b47bff;
    background: #f8f3ff;
  }
  .thread-post-head {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.68rem;
    color: #6a67a4;
    margin-bottom: 0.15rem;
  }
  .thread-author {
    font-weight: 600;
    color: #2f2c5c;
  }
  .thread-when {
    flex: 1;
    color: #9793c9;
  }
  .thread-delete {
    background: none;
    border: none;
    color: #b7b1e5;
    cursor: pointer;
    font-size: 0.75rem;
    padding: 0;
    line-height: 1;
  }
  .thread-delete:hover {
    color: #d0393b;
  }
  .thread-body {
    margin: 0;
    font-size: 0.82rem;
    line-height: 1.4;
    white-space: pre-wrap;
    color: #1f1e33;
  }
  .thread-empty {
    color: #9c98c9;
    font-size: 0.75rem;
    padding: 0.4rem 0.6rem;
  }
  .thread-compose {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .thread-author-toggle {
    display: flex;
    gap: 0.25rem;
  }
  .thread-author-toggle button {
    padding: 0.2rem 0.6rem;
    font-size: 0.72rem;
    background: #fafafd;
    border: 1px solid #d6d3ec;
    border-radius: 3px;
    cursor: pointer;
    color: #6a67a4;
  }
  .thread-author-toggle button:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .thread-author-toggle button.active {
    background: #5850ff;
    color: #ffffff;
    border-color: #5850ff;
  }
  .thread-input {
    width: 100%;
    box-sizing: border-box;
    min-height: 3rem;
    padding: 0.4rem 0.55rem;
    font-size: 0.82rem;
    font-family: inherit;
    line-height: 1.4;
    background: #fafafd;
    border: 1px solid #d6d3ec;
    border-radius: 4px;
    outline: none;
    color: inherit;
    resize: vertical;
  }
  .thread-input:focus {
    border-color: #8a86ff;
    background: #ffffff;
  }
  .thread-actions {
    display: flex;
    justify-content: flex-end;
  }
  .thread-post-btn {
    padding: 0.35rem 0.9rem;
    background: #5850ff;
    color: #ffffff;
    border: none;
    border-radius: 4px;
    font-size: 0.82rem;
    cursor: pointer;
  }
  .thread-post-btn:hover:not(:disabled) {
    background: #4a42e0;
  }
  .thread-post-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* -----------------------------------------------------------------
   * Selection chip strip
   * ----------------------------------------------------------------- */
  .detail-selection-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.18rem 0.55rem;
    margin: 0.15rem 0.2rem 0.15rem 0;
    background: #eeecff;
    color: #3d38a8;
    border: 1px solid #d3d0f0;
    border-radius: 999px;
    font-size: 0.78rem;
    cursor: pointer;
  }
  .detail-selection-chip:hover {
    background: #dcd7ff;
    border-color: #b9b2f0;
  }
  .detail-selection-count {
    font-size: 0.7rem;
    color: #5850ff;
    font-variant-numeric: tabular-nums;
  }

  /* -----------------------------------------------------------------
   * Provenance section
   * ----------------------------------------------------------------- */
  .provenance-container {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .provenance-loading {
    font-size: 0.72rem;
    color: #888;
    font-style: italic;
  }
  .provenance-lane {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .provenance-lane-label {
    font-size: 0.68rem;
    color: #6a67a4;
    letter-spacing: 0.02em;
  }
  .provenance-lane-strip {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
  }
  .provenance-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.55rem;
    background: #f4f1ff;
    color: #3d38a8;
    border: 1px solid #dcd7ff;
    border-radius: 999px;
    font-size: 0.75rem;
    cursor: pointer;
    max-width: 22ch;
  }
  .provenance-chip:hover {
    background: #e6dfff;
    border-color: #b9b2f0;
  }
  /* Hop distance from the open asset. Reads as a step number along
     the chain, so a 3-hop return trip is visibly further away than
     the export it came out of. */
  .provenance-chip-hop {
    flex: none;
    min-width: 1.1rem;
    padding: 0 0.25rem;
    background: #ded8fb;
    color: #4f4b8c;
    border-radius: 999px;
    font-size: 0.62rem;
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  .provenance-truncated {
    color: #8b87c4;
    font-weight: 400;
  }
  .provenance-chip-cover {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 16ch;
  }

  /* -----------------------------------------------------------------
   * Tag chip + promote / detach actions
   * ----------------------------------------------------------------- */
  .label {
    display: inline-block;
    padding: 0.1rem 0.5rem;
    background: #f0effc;
    color: #7a76c9;
    border-radius: 3px;
    font-size: 0.72rem;
    line-height: 1.4;
  }
  .label-tag {
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .label-tag:hover {
    background: #e2ddf9;
    color: #5a55b2;
  }
  .label-tag-active {
    background: #7a76c9;
    color: #fff;
  }
  .label-tag-active:hover {
    background: #7a76c9;
    color: #fff;
    cursor: default;
  }
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

  /* -----------------------------------------------------------------
   * Wallpaper action (image detail only)
   * ----------------------------------------------------------------- */
  .wallpaper-action {
    margin-top: 0.6rem;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .wallpaper-btn {
    padding: 0.3rem 0.6rem;
    border: 1px solid #d0d0d0;
    border-radius: 3px;
    background: #fafafa;
    cursor: pointer;
    font-size: 0.85rem;
    color: #333;
  }
  .wallpaper-btn:hover {
    background: #ffe9c9;
    color: #7a4a00;
  }
  .wallpaper-current {
    font-size: 0.75rem;
    color: #4a6a1a;
  }

  /* -----------------------------------------------------------------
   * Fullscreen zoom stage (image only, sits above detail overlay)
   * ----------------------------------------------------------------- */
  .fullscreen-backdrop {
    position: fixed;
    inset: 0;
    z-index: 200;
    background: #0c0b12;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: zoom-out;
    overflow: hidden;
    touch-action: none;
  }
  .fullscreen-backdrop.zoomed {
    cursor: grab;
  }
  .fullscreen-backdrop.panning {
    cursor: grabbing;
  }
  .fullscreen-backdrop img {
    max-width: 100vw;
    max-height: 100vh;
    object-fit: contain;
    display: block;
    transform-origin: 50% 50%;
    transition: transform 40ms linear;
    will-change: transform;
    user-select: none;
    -webkit-user-drag: none;
  }
  .fullscreen-backdrop.panning img {
    transition: none;
  }
  .fullscreen-hint {
    position: fixed;
    right: 0.8rem;
    bottom: 0.6rem;
    font-size: 0.65rem;
    color: rgba(255, 255, 255, 0.45);
    pointer-events: none;
  }
  .fullscreen-nav {
    position: fixed;
    top: 50%;
    transform: translateY(-50%);
    width: 2.4rem;
    height: 4rem;
    border: none;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.06);
    color: rgba(255, 255, 255, 0.55);
    font-size: 1.6rem;
    line-height: 1;
    cursor: pointer;
  }
  .fullscreen-nav:hover {
    background: rgba(255, 255, 255, 0.14);
    color: #fff;
  }
  .fullscreen-nav-prev {
    left: 0.6rem;
  }
  .fullscreen-nav-next {
    right: 0.6rem;
  }
</style>
