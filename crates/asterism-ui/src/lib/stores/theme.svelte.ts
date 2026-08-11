// Theme catalog — persona-scoped wallpaper. Sibling of `personaCatalog` /
// `modalityCatalog` / `tagCatalog` / `groupCatalog` /
// `sessionCatalog`. Owns the theme row for the currently-active
// persona plus the resolved blob URL that renders as the grid
// background.
//
// Scope:
//   - `current`: Resource whose data bundles `{ theme,
//     wallpaperUrl }` — the two land atomically because they are
//     one fetch cascade's product. The old hand-rolled
//     `#generation` counter is replaced by the Resource guard; the
//     multi-step fetcher uses the `isStale` callback to bail out
//     of the thumb cascade early and to avoid creating a blob URL
//     for a response that would be dropped.
//   - `theme` / `wallpaperUrl` getters: consumer-facing aliases
//     over `current.data` so templates keep reading
//     `themeCatalog.theme?.wallpaper_asset_id` etc.
//   - `loadFor(personaId)`: revokes the previous blob URL, resets,
//     and (for a non-null persona) fetches theme + wallpaper
//     through the 1024 → 512 → 256 thumb cascade, falling back to
//     the original file via `asset_detail` + `convertFileSrc` if
//     every thumb size misses.
//
// Deliberately NOT owned here:
//   - `setAsWallpaper` / `clearWallpaper` / `contextSetWallpaper`:
//     the invoke + status-line update stays in App because the
//     status string is UI chrome the store has no business owning.
//     Callers invoke `set_persona_theme` / `delete_persona_theme`
//     themselves and then `await themeCatalog.loadFor(personaId)` to
//     refresh the cached row + blob URL.
//   - The `$effect(() => themeCatalog.loadFor(activeFilter.activePersona))`
//     dispatcher: lives in App so the `untrack` boundary sits next
//     to the read of `activeFilter.activePersona` (the reactive
//     signal that drives it). Wrapping the call site in `untrack`
//     is what keeps the internal wallpaper writes from
//     re-triggering the same effect.

import type { AssetDetailDto, PersonaThemeDto } from "../../bindings";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

// Wallpaper resolution cascade — see the App-side notes on why the
// 1024 px thumb is enqueued at high priority the first time we ask
// for it, and why we fall through to smaller cached sizes plus the
// original-file `convertFileSrc` path if the 1024 thumb has not
// landed yet.
const WALLPAPER_SIZE_PX = 1024;
const FALLBACK_SIZES = [WALLPAPER_SIZE_PX, 512, 256] as const;

interface ThemeState {
  theme: PersonaThemeDto | null;
  wallpaperUrl: string | null;
}

const EMPTY: ThemeState = { theme: null, wallpaperUrl: null };

async function fetchThemeState(
  personaId: string,
  isStale: () => boolean,
): Promise<ThemeState> {
  const theme = await api<PersonaThemeDto | null>("get_persona_theme", {
    personaId,
  });
  if (!theme || isStale()) return EMPTY;
  const assetId = theme.wallpaper_asset_id;
  if (!assetId) return { theme, wallpaperUrl: null };

  for (const size of FALLBACK_SIZES) {
    const bytes = await api<number[] | Uint8Array | null>("get_asset_thumb", {
      assetId,
      sizePx: size,
    });
    if (isStale()) return { theme, wallpaperUrl: null };
    if (bytes && (bytes as ArrayLike<number>).length > 0) {
      const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      const url = URL.createObjectURL(new Blob([buf], { type: "image/jpeg" }));
      return { theme, wallpaperUrl: url };
    }
  }

  // Every thumb size missed — fall back to the original file
  // through the Tauri asset protocol. We need the locator, so
  // fetch the asset detail (small round trip; only fires when
  // no thumb size is cached yet).
  try {
    const detail = await api<AssetDetailDto>("asset_detail", {
      query: { asset_id: assetId, viewer_subject: null },
    });
    if (isStale()) return { theme, wallpaperUrl: null };
    return { theme, wallpaperUrl: convertFileSrc(detail.asset.locator) };
  } catch (fallbackError) {
    console.warn(
      "wallpaper original-file fallback failed",
      assetId,
      fallbackError,
    );
    return { theme, wallpaperUrl: null };
  }
}

class ThemeCatalog {
  current = new Resource(fetchThemeState, EMPTY, "themeCatalog.current");

  get theme(): PersonaThemeDto | null {
    return this.current.data.theme;
  }

  get wallpaperUrl(): string | null {
    return this.current.data.wallpaperUrl;
  }

  async loadFor(personaId: string | null): Promise<void> {
    // Revoke the outgoing blob URL before dropping the reference —
    // the browser holds the decoded image alive until revoke.
    const prev = this.current.data.wallpaperUrl;
    if (prev) URL.revokeObjectURL(prev);
    // reset() also invalidates any in-flight cascade (users flick
    // between persona rows fast).
    this.current.reset();
    if (personaId === null) return;
    await this.current.load(personaId);
  }
}

export const themeCatalog = new ThemeCatalog();
