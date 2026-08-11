// Profile catalog — per-persona profile row cache. Sibling of the other
// catalog stores; owns the data that backs the sidebar Profile hover
// card + the persona avatar-mini rendered in the persona strip /
// card thread heads.
//
// This store is deliberately NOT on the Resource primitive: it is a
// lazy per-key cache (one row per persona, fetched on demand), not a
// single-value fetch machine. Its cross-cutting concern is
// fine-grained map reactivity, which SvelteMap covers —
// both caches below moved off the old "fresh Map per write" /
// "manual tick counter" workarounds in wave H2.
//
// Scope:
//   - `profiles: SvelteMap<personaId, PersonaProfileDto | null>` —
//     the lazy cache. `null` = row exists in the DB but has no row
//     set yet (get_persona_profile returned `null`). Missing key =
//     not fetched yet. SvelteMap so `.set()` fires reactivity on
//     exactly the touched key.
//   - `ensureProfile(personaId)` — returns the cached row if
//     present, otherwise fires `get_persona_profile` and populates
//     the map. Never throws (invoke failures are logged + return
//     `null` so the sidebar keeps rendering).
//   - `updateProfile(personaId, next)` — writes a fresh row into
//     the cache. Used by the "Set as avatar" reflex-menu action
//     and by the ProfileCard save-form once `set_persona_profile`
//     returns success.
//   - Avatar thumb cache: `avatarUrl(assetId)` returns a blob URL
//     for the 128 px thumb of the profile's avatar asset, kicking
//     a background fetch on cache-miss and returning `null` while
//     pending. `personaAvatarUrl(personaId)` is the two-step
//     helper the strip / thread heads use (ensure profile, then
//     resolve the avatar). The blob URL cache lives on the store
//     so PersonaStrip / DetailPane / the card thread head share a
//     single decode budget; SvelteMap reads inside templates /
//     deriveds auto-track, so no tick counter. The in-flight
//     guard is a plain (non-reactive) Set because `avatarUrl` is
//     called during template evaluation, where writing reactive
//     state is illegal — the SvelteMap is only written from async
//     continuations.
//
// Deliberately NOT owned here:
//   - `profileCard` (which persona is currently hovered), the
//     hover-open + close-grace timers, and the edit-form buffer.
//     Those are modal / UI state that lives with the ProfileCard
//     component + App (App holds the modal open/close state; the
//     card owns its own edit buffer). The store only tracks the
//     data the modal renders.
//   - `contextSetAvatar` / the save form's invoke of
//     `set_persona_profile` — App / ProfileCard invoke the Tauri
//     command themselves (they need `status = ...`) and then
//     `profileCatalog.updateProfile(pid, next)` to write the
//     freshly-returned row into the cache.

import type { PersonaProfileDto } from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";

// Avatar-mini render size. The persona strip / thread head both
// paint a 16-96 px circle so the 128 px thumb covers the largest
// call site without paying video-frame decode cost.
const AVATAR_THUMB_SIZE_PX = 128;

class ProfileCatalog {
  profiles = new SvelteMap<string, PersonaProfileDto | null>();
  // Blob URL cache keyed by asset id — landed URLs only.
  #avatarThumbUrls = new SvelteMap<string, string>();
  // Synchronous in-flight guard (see docstring: must stay
  // non-reactive because it is written during template evaluation).
  #avatarThumbPending = new Set<string>();

  async ensureProfile(
    personaId: string,
  ): Promise<PersonaProfileDto | null> {
    if (this.profiles.has(personaId)) {
      return this.profiles.get(personaId) ?? null;
    }
    try {
      const p = await api<PersonaProfileDto | null>(
        "get_persona_profile",
        { personaId },
      );
      this.profiles.set(personaId, p);
      return p;
    } catch (error) {
      console.warn("get_persona_profile failed", error);
      return null;
    }
  }

  updateProfile(personaId: string, next: PersonaProfileDto): void {
    this.profiles.set(personaId, next);
  }

  avatarUrl(assetId: string | null | undefined): string | null {
    if (!assetId) return null;
    // SvelteMap read — callers re-render once `#ensureAvatarThumb`
    // lands the blob (reading inside a $derived / template
    // auto-tracks the touched key).
    const cached = this.#avatarThumbUrls.get(assetId);
    if (cached) return cached;
    void this.#ensureAvatarThumb(assetId);
    return null;
  }

  personaAvatarUrl(
    personaId: string | null | undefined,
  ): string | null {
    if (!personaId) return null;
    void this.ensureProfile(personaId);
    const profile = this.profiles.get(personaId) ?? null;
    return this.avatarUrl(profile?.avatar_asset_id);
  }

  async #ensureAvatarThumb(assetId: string): Promise<void> {
    if (
      this.#avatarThumbPending.has(assetId) ||
      this.#avatarThumbUrls.has(assetId)
    ) {
      return;
    }
    this.#avatarThumbPending.add(assetId);
    try {
      const bytes = await api<number[] | Uint8Array | null>(
        "get_asset_thumb",
        { assetId, sizePx: AVATAR_THUMB_SIZE_PX },
      );
      if (bytes && (bytes as ArrayLike<number>).length > 0) {
        const buf = bytes instanceof Uint8Array
          ? bytes
          : new Uint8Array(bytes);
        const url = URL.createObjectURL(
          new Blob([buf], { type: "image/jpeg" }),
        );
        this.#avatarThumbUrls.set(assetId, url);
      }
    } catch (error) {
      console.warn("avatar thumb fetch failed", assetId, error);
    } finally {
      this.#avatarThumbPending.delete(assetId);
    }
  }
}

export const profileCatalog = new ProfileCatalog();
