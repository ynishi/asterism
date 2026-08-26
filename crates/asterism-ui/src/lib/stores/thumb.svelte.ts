// Thumb catalog — blob-URL caches for asset thumbnails + the
// group/dir cover picks. One shared
// decode budget for every surface that paints a thumb: grid cards
// (256 px), detail view (512 px), constellation pre-warm (512 px),
// dir-lane covers (128 px).
//
// NOT on the Resource primitive (escape hatch): these are
// per-key lazy caches (same shape as `profileCatalog`), not
// single-value fetch machines. The in-flight guards are plain
// (non-reactive) Sets because `thumbSrc` / `detailSrc` /
// `groupCoverThumb` run during template evaluation, where writing
// reactive state is illegal — cache writes happen only in async
// continuations, signalled through the tick counters.
//
// Scope:
//   - `thumbSrc(card)` — 256 px grid thumb; falls back to
//     `convertFileSrc(source_locator)` for the first paint.
//   - `detailSrc(locator, assetId)` — 512 px detail preview so a
//     multi-MB original is not streamed through the asset
//     protocol; falls back to the raw file on cache miss.
//   - `ensureThumb(assetId, sizePx)` — fetch + retry loop. A cache
//     miss means the Tauri command already enqueued a
//     high-priority `thumb_gen` job; exponential backoff retries
//     (~6 s total) swap the fresh blob in without caller polling.
//   - `groupCoverThumb(groupId)` / `dirCoverThumb(dirId)` — lane
//     covers: first image asset of the group (fetched lazily, one
//     `list_assets limit:1` per group on screen; `null` cached as
//     "tried, no image"), rendered through the 128 px thumb path.
//     Reads `activeFilter.activePersona` for the cover query and
//     `groupCatalog` for the dir→group tree.
//   - `revokeAll()` — teardown hook (App `onDestroy`) so the
//     browser drops the underlying Blobs.
//   - Negative caches: exhausted-thumb keys stop
//     re-kicking `ensureThumb`, and originals reported dead via
//     `noteOriginalError` (img onerror) render a transparent
//     placeholder instead of re-hitting the asset protocol with a
//     404 on every mount.
//
// Instrumented, not changed, by `lib/dev/thumb-perf.ts`: `ensureThumb`
// stamps each fetch and records how it ended, and the three residency
// counters (`blobUrlCount` / `missingCount` / `deadCount`) are exposed
// as getters for the bench dump. Every one of those calls no-ops
// outside `import.meta.env.DEV`, and none of them feeds back into a
// decision this catalog makes — the retry budget, the negative caches
// and the blob lifetime are exactly what they were before.
//
// Deliberately NOT owned here:
//   - the wallpaper cascade (themeCatalog) and avatar thumbs
//     (profileCatalog) — different lifecycles, already extracted.

import type { AssetCardDto, AssetPageDto } from "../../bindings";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api } from "../api";
import type { ThumbAttempt, ThumbOutcome } from "../dev/thumb-perf";
import { thumbPerf } from "../dev/thumb-perf";
import { activeFilter } from "./filter.svelte";
import { groupCatalog } from "./group.svelte";

// 1×1 transparent GIF — served instead of a dead `asset://` URL once
// an original is known to be gone, so the webview stops re-requesting
// the missing file on every card mount (stress rows
// whose source files lived in a wiped scratchpad flooded the asset
// protocol with 404s on every scroll).
const TRANSPARENT_PX =
  "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

// Whether the original bytes are a video, in which case the file
// itself is never a usable `<img>` source — only the frame the
// backend extracted for it is.
//
// Reads the `media` slug the backend decided rather than re-deriving
// it from the mime string: that rule lives in
// `domain::render::render_policy`, and a copy here could only ever
// fall behind it.
function isVideo(media: string | null | undefined): boolean {
  return media === "video";
}

/**
 * Upper bound on cached thumb blobs.
 *
 * Blob URLs are not garbage-collected. The browser holds the bytes
 * until `revokeObjectURL` is called, so a cache with no bound grows
 * for as long as the session keeps scrolling — measured at 3,361
 * entries and still climbing after 200 jumps across a 110k library
 * [measured 2026-08-05, bench-scroll-v3], with nothing to stop it short of
 * closing the app. At ~20 KB per 256 px entry that is tens of
 * megabytes of blob store no code can reach, and the failure mode at
 * the end of it is the machine swapping.
 *
 * The bound has to clear the working set by a wide margin: evicting
 * something still on screen blanks that tile. The grid mounts on the
 * order of tens of cards (12-15 at the bench window size, ~50 at a
 * large one), so this is one to two orders of magnitude of headroom
 * while capping the footprint at roughly 25 MB.
 */
export const BLOB_CACHE_CAP = 1_200;

/**
 * How many times a missing thumbnail is polled for before the key is
 * written off.
 *
 * A miss means the backend just enqueued a `thumb_gen`, so the retries
 * are a poll for a blob that is about to exist rather than repeated
 * requests for the same work.
 */
export const THUMB_RETRY_BUDGET = 6;

/**
 * Delay before the next poll, given the retries still left.
 *
 * Doubles from 250 ms and holds at 4 s, so a budget of six spends
 * **250 + 500 + 1000 + 2000 + 4000 + 4000 = 11.75 s** before giving
 * up. The former comment here claimed "~6 s (250, 500, 1000, 2000,
 * 4000)" — five delays, not six, and it was the code that was right:
 * the bench measured a p50 of 11,776 ms on assets whose thumbnails did
 * not exist yet [measured 2026-08-05], which is this sum.
 */
function retryDelayMs(retriesLeft: number): number {
  return Math.min(4000, 250 * Math.pow(2, THUMB_RETRY_BUDGET - retriesLeft - 1));
}

class ThumbCatalog {
  // Cache of thumb blob URLs keyed by "<assetId>@<sizePx>". The
  // grid uses 256 px and the detail view 512 px, so the same asset
  // can hold two entries.
  #urls = new Map<string, string>();
  // Requests already in flight; prevents a burst of duplicate
  // invokes while a fetch is still going.
  #pending = new Set<string>();
  // Ids waiting to go out, grouped by size — one batch per size per
  // flush. Filled by `ensureThumb`, drained by the microtask.
  #queued = new Map<number, Set<string>>();
  // The flush currently scheduled, so callers awaiting `ensureThumb`
  // settle when their batch has been applied. `null` between flushes.
  #flushing: Promise<void> | null = null;
  // Polls still owed per key. Lives here rather than in a recursive
  // argument because a key's retries now span batches.
  #retriesLeft = new Map<string, number>();
  // Reactive tick so consumers re-render when a thumb lands.
  tick = $state(0);

  // Cover-image cache per Group: {group_id → first image asset
  // seen inside that group}. Populated lazily by the lane so
  // switching dirs only triggers fetches for the groups currently
  // on screen. `null` means "we already tried and there is no
  // image asset in that group", so the UI can render a folder
  // placeholder instead of retrying.
  #coverAsset = new SvelteMap<string, AssetCardDto | null>();
  #coverPending = new Set<string>();
  coverTick = $state(0);

  // Negative caches. `#missingThumb`: keys whose
  // `ensureThumb` retry budget is exhausted — stop re-kicking the
  // fetch (each miss also enqueues a backend `thumb_gen` job, so
  // unbounded re-kicks pile dead jobs). Session-lifetime by design:
  // the retry loop already covers the "thumb is being generated
  // right now" window. `#deadOriginals`: asset ids whose original
  // file 404'd through the asset protocol (reported by the `<img>`
  // onerror via `noteOriginalError`) — SvelteSet so the cards
  // showing the dead fallback re-render onto the placeholder.
  #missingThumb = new Set<string>();
  #deadOriginals = new SvelteSet<string>();

  // `<img onerror>` hook: mark the asset's original as gone unless
  // the failing src was a blob (decode failure of a real thumb —
  // different problem, keep the fallback path alive for it).
  noteOriginalError(assetId: string, ev: Event): void {
    const src = (ev.currentTarget as HTMLImageElement | null)?.src ?? "";
    if (src.startsWith("blob:") || src.startsWith("data:")) return;
    this.#deadOriginals.add(assetId);
  }

  #key(assetId: string, sizePx: number): string {
    return `${assetId}@${sizePx}`;
  }

  /**
   * Marks a key as most-recently used.
   *
   * `Map` preserves insertion order, so deleting and re-inserting
   * moves the entry to the end and leaves the least-recently-touched
   * one at the front — which is what [`#evict`](#evict) takes. Safe to
   * call during template evaluation: `#urls` is a plain `Map`, not
   * reactive state, so reordering it signals nothing and re-renders
   * nothing.
   */
  #touch(key: string): void {
    const url = this.#urls.get(key);
    if (url === undefined) return;
    this.#urls.delete(key);
    this.#urls.set(key, url);
  }

  /**
   * Drops least-recently-used blobs past [`BLOB_CACHE_CAP`], revoking
   * each so the browser actually releases the bytes.
   *
   * Called only from `ensureThumb`'s async continuation — never during
   * template evaluation. Revoking a URL an `<img>` is currently
   * pointing at blanks that tile, and template evaluation is exactly
   * when those `<img>` elements are reading their `src`.
   */
  #evictOverflow(): void {
    while (this.#urls.size > BLOB_CACHE_CAP) {
      const oldest = this.#urls.keys().next().value;
      if (oldest === undefined) return;
      const url = this.#urls.get(oldest);
      this.#urls.delete(oldest);
      if (url !== undefined) URL.revokeObjectURL(url);
    }
  }

  // Residency counters for the bench driver (`thumb-perf.dump()`).
  // Exposed as getters rather than by handing the caches out: the
  // sizes are the measurement (what the
  // frontend is still holding after a long scroll), the caches
  // themselves are not something a reader should be able to mutate.
  get blobUrlCount(): number {
    return this.#urls.size;
  }

  get missingCount(): number {
    return this.#missingThumb.size;
  }

  get deadCount(): number {
    return this.#deadOriginals.size;
  }

  // Bench instrumentation only — DEV-gated, and the `enabled` check
  // keeps a production build from paying for the entry object or the
  // DOM probe. Records nothing the catalog acts on.
  #notePerf(
    assetId: string,
    sizePx: number,
    attempt: ThumbAttempt,
    outcome: ThumbOutcome,
  ): void {
    if (!thumbPerf.enabled) return;
    thumbPerf.record({
      assetId,
      sizePx,
      requestedAtMs: attempt.requestedAtMs,
      resolvedAtMs: thumbPerf.now(),
      outcome,
      retryCount: attempt.retryCount,
      visibleAtResolve: thumbPerf.visible(assetId),
    });
  }

  /**
   * Asks for one thumbnail, joining whatever batch is being assembled.
   *
   * The call does not reach the backend by itself. `thumbSrc` runs
   * once per card during template evaluation, so a painted screenful
   * calls this tens of times in a single tick; the ids collect in
   * `#queued` and a microtask sends them as **one** `get_asset_thumbs`.
   * Asking per card meant an IPC round trip per tile per scroll —
   * 8,263 of them across 1,000 jumps, with p95 reaching 10.4 s once
   * the blob cache stopped absorbing the repeats [measured 2026-08-05,
   * bench-scroll-v3].
   *
   * The returned promise settles when the batch this key joined has
   * been applied, so a caller that wants to await one thumbnail still
   * can.
   */
  ensureThumb(
    assetId: string,
    sizePx: number,
    retriesLeft = THUMB_RETRY_BUDGET,
  ): Promise<void> {
    const key = this.#key(assetId, sizePx);
    if (this.#urls.has(key) || this.#missingThumb.has(key)) {
      return Promise.resolve();
    }
    if (this.#pending.has(key)) {
      // Already joined a batch — almost always because `thumbSrc`
      // kicked it while painting. Hand back that batch's promise
      // rather than a resolved one: an awaiting caller means to wait
      // for the thumbnail, and returning early would report "done"
      // before the request had even gone out.
      return this.#flushing ?? Promise.resolve();
    }
    this.#pending.add(key);
    this.#retriesLeft.set(key, retriesLeft);
    return this.#enqueue(assetId, sizePx);
  }

  /** Adds a key to the pending batch and makes sure a flush is coming. */
  #enqueue(assetId: string, sizePx: number): Promise<void> {
    let ids = this.#queued.get(sizePx);
    if (ids === undefined) {
      ids = new Set();
      this.#queued.set(sizePx, ids);
    }
    ids.add(assetId);
    if (this.#flushing === null) {
      this.#flushing = new Promise((resolve) => {
        // A microtask, not a timer: everything a single paint asks for
        // lands in the same batch, and nothing waits a frame for it.
        queueMicrotask(() => {
          void this.#flush().finally(() => {
            this.#flushing = null;
            resolve();
          });
        });
      });
    }
    return this.#flushing;
  }

  async #flush(): Promise<void> {
    const work = this.#queued;
    this.#queued = new Map();
    // One request per size. The grid asks for 256 and the detail view
    // for 512, and the command takes a single size per call.
    for (const [sizePx, ids] of work) {
      await this.#fetchBatch([...ids], sizePx);
    }
  }

  /**
   * Sends one batch and applies every slot.
   *
   * JPEG bytes arrive as either `number[]` or `Uint8Array` depending
   * on the runtime — either way they go straight to `Blob`.
   */
  async #fetchBatch(assetIds: string[], sizePx: number): Promise<void> {
    // Bench instrumentation (DEV only): stamps the first attempt per
    // key and counts the retries that follow. No behaviour reads it.
    const attempts = new Map<string, ThumbAttempt>();
    for (const assetId of assetIds) {
      attempts.set(assetId, thumbPerf.begin(assetId, sizePx));
    }

    let slots: (number[] | Uint8Array | null)[];
    try {
      slots =
        (await api<(number[] | Uint8Array | null)[]>("get_asset_thumbs", {
          assetIds,
          sizePx,
        })) ?? [];
    } catch (error) {
      // Silent — every caller falls back to convertFileSrc(originals).
      console.warn("get_asset_thumbs failed", sizePx, assetIds.length, error);
      for (const assetId of assetIds) {
        const key = this.#key(assetId, sizePx);
        this.#pending.delete(key);
        this.#retriesLeft.delete(key);
        const attempt = attempts.get(assetId);
        if (attempt) this.#notePerf(assetId, sizePx, attempt, "dead");
      }
      return;
    }

    let landed = false;
    assetIds.forEach((assetId, index) => {
      const key = this.#key(assetId, sizePx);
      const bytes = slots[index] ?? null;
      this.#pending.delete(key);
      const attempt = attempts.get(assetId);

      if (bytes && (bytes as ArrayLike<number>).length > 0) {
        const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        const url = URL.createObjectURL(new Blob([buf], { type: "image/jpeg" }));
        this.#urls.set(key, url);
        this.#retriesLeft.delete(key);
        landed = true;
        if (attempt) {
          this.#notePerf(
            assetId,
            sizePx,
            attempt,
            attempt.retryCount > 0 ? "retried" : "hit",
          );
        }
        return;
      }

      // Cache miss — the command has already enqueued a high-priority
      // `thumb_gen` for us, so the retry is a poll for the blob it is
      // about to write, not a second request for the same work.
      const left = (this.#retriesLeft.get(key) ?? 0) - 1;
      if (left > 0) {
        this.#retriesLeft.set(key, left);
        setTimeout(() => {
          // The key may have been satisfied (a neighbouring batch) or
          // given up on while this timer waited.
          if (this.#urls.has(key) || this.#missingThumb.has(key)) return;
          this.#pending.add(key);
          void this.#enqueue(assetId, sizePx);
        }, retryDelayMs(left));
        return;
      }

      // Budget exhausted — remember the miss so viewport re-mounts
      // stop re-kicking the fetch and the backend job enqueue.
      this.#missingThumb.add(key);
      this.#retriesLeft.delete(key);
      if (attempt) this.#notePerf(assetId, sizePx, attempt, "missing");
    });

    if (landed) {
      // Entries are only added here, so this is the one place the
      // bound has to be enforced.
      this.#evictOverflow();
      this.tick += 1;
    }
  }

  thumbSrc(card: AssetCardDto): string {
    void this.tick; // subscribe to reactive updates
    const key = this.#key(card.id, 256);
    const cached = this.#urls.get(key);
    if (cached) {
      // A tile being painted is the definition of "in use" — this is
      // what keeps the visible screenful at the safe end of the LRU.
      this.#touch(key);
      return cached;
    }
    // Kick off the fetch; fall back to the original file for the
    // first paint (Tauri asset-protocol scope permitting). Known-dead
    // originals get the placeholder instead so the protocol is not
    // re-hit on every mount.
    void this.ensureThumb(card.id, 256);
    if (this.#deadOriginals.has(card.id)) return TRANSPARENT_PX;
    // A video original is not something an `<img>` can render — the
    // fallback that helps an image here would guarantee a broken load
    // and get the file marked dead via `onerror`. Wait for the
    // extracted frame instead; the placeholder holds the tile until
    // `ensureThumb` lands.
    if (isVideo(card.media)) return TRANSPARENT_PX;
    return convertFileSrc(card.source_locator);
  }

  // A 256 px thumb for a surface that holds an asset id and no card.
  //
  // The forge is that surface: a line's entry names the asset it
  // carries (`ForgeEntryStateDto.content_asset_id`) and nothing else
  // about it, so `thumbSrc`'s two fallbacks — the original file, and
  // the video placeholder — have nothing to read. A cache miss
  // therefore paints the placeholder and waits for `ensureThumb`'s job
  // rather than reaching for the original, which is the right trade
  // here: an entry's asset is one of hundreds on a line, and a panel
  // that streamed every original on open would be worse than one that
  // fills in.
  //
  // One consequence worth knowing: a key that exhausted its retries
  // stays on the placeholder for the rest of the session here, where
  // `thumbSrc` would still have the original to fall back to.
  thumbById(assetId: string): string {
    void this.tick; // subscribe to reactive updates
    const key = this.#key(assetId, 256);
    const cached = this.#urls.get(key);
    if (cached) {
      this.#touch(key);
      return cached;
    }
    void this.ensureThumb(assetId, 256);
    return TRANSPARENT_PX;
  }

  // Detail-view src: prefer the pre-generated 512 px preview so we
  // do not stream a multi-MB original through the asset protocol;
  // fall back to the raw file on cache miss / fetch failure. The
  // 512 px thumb is written on ingest by asterism-import-image
  // with `--thumb-sizes 256,512`.
  detailSrc(locator: string, assetId: string, media?: string | null): string {
    void this.tick;
    const key = this.#key(assetId, 512);
    const cached = this.#urls.get(key);
    if (cached) {
      this.#touch(key);
      return cached;
    }
    void this.ensureThumb(assetId, 512);
    if (this.#deadOriginals.has(assetId)) return TRANSPARENT_PX;
    // Same reason as `thumbSrc`: an `<img>` cannot render a video, so
    // the extracted frame is the only thing worth waiting for.
    if (isVideo(media)) return TRANSPARENT_PX;
    return convertFileSrc(locator);
  }

  async #ensureGroupCover(groupId: string): Promise<void> {
    if (this.#coverAsset.has(groupId) || this.#coverPending.has(groupId)) {
      return;
    }
    this.#coverPending.add(groupId);
    try {
      // "Is this an image?" is a material fact, so it is asked through
      // the FORMAT facet. This used to filter on `modality: "image"`,
      // a slug V38 deleted from the master — the query could only ever
      // return zero rows, so no Group has shown a cover since. (Listed
      // as an unverified suspicion in the journal's Not Done; the v4
      // role wave is what surfaced it.)
      const query = {
        viewer_subject: null,
        persona_id: activeFilter.activePersona,
        modality: null,
        format: "image",
        role: "item",
        occurred_from_ms: null,
        occurred_until_ms: null,
        tag_ids: [],
        group_ids: [groupId],
        session_id: null,
        label: null,
        offset: 0,
        limit: 1,
      };
      const result = await api<AssetPageDto>("list_assets", { query });
      const first = result.items[0] ?? null;
      this.#coverAsset.set(groupId, first);
      this.coverTick += 1;
    } catch (err) {
      console.warn("group cover fetch failed", groupId, err);
      this.#coverAsset.set(groupId, null);
    } finally {
      this.#coverPending.delete(groupId);
    }
  }

  groupCoverThumb(groupId: string): string | null {
    void this.coverTick;
    const cached = this.#coverAsset.get(groupId);
    if (cached === undefined) {
      void this.#ensureGroupCover(groupId);
      return null;
    }
    if (cached === null) return null;
    void this.tick;
    const key = this.#key(cached.id, 128);
    const cachedThumb = this.#urls.get(key);
    if (cachedThumb) {
      // Lane covers are painted alongside the grid and belong at the
      // safe end of the LRU for the same reason tiles do.
      this.#touch(key);
      return cachedThumb;
    }
    void this.ensureThumb(cached.id, 128);
    if (this.#deadOriginals.has(cached.id)) return null;
    return convertFileSrc(cached.source_locator);
  }

  // Sub-dir cover: recurse into descendant groups' first image
  // by picking any descendant group's cover (breadth-first) —
  // cheaper than aggregating all descendants, and dogfood-good
  // enough since a leaf dir usually has one distinctive folder
  // anyway. One level deep only to keep the fetch fan-out bounded.
  dirCoverThumb(dirId: string): string | null {
    const groupsHere = groupCatalog.groupsByDir.get(dirId) ?? [];
    for (const gc of groupsHere) {
      const src = this.groupCoverThumb(gc.group.id);
      if (src) return src;
    }
    for (const child of groupCatalog.dirChildren.get(dirId) ?? []) {
      const childGroups = groupCatalog.groupsByDir.get(child.id) ?? [];
      for (const gc of childGroups) {
        const src = this.groupCoverThumb(gc.group.id);
        if (src) return src;
      }
    }
    return null;
  }

  // Every URL is revoked on App destroy so the browser drops the
  // underlying Blobs.
  revokeAll(): void {
    for (const url of this.#urls.values()) {
      URL.revokeObjectURL(url);
    }
    this.#urls.clear();
  }
}

export const thumbCatalog = new ThumbCatalog();

// The bench dump carries the catalog's residency counters alongside
// the fetch entries; wiring it here (rather than importing the
// catalog from `dev/thumb-perf`) keeps the dependency one-way. No-op
// outside DEV.
thumbPerf.bindCounts(() => ({
  blobUrlCount: thumbCatalog.blobUrlCount,
  missingCount: thumbCatalog.missingCount,
  deadCount: thumbCatalog.deadCount,
}));
