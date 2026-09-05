<script lang="ts">
  // ProfileCard — extracted from App.svelte (2026-07-21 Phase C
  // wave 8b). The hover-anchored modal that pops next to a persona
  // row in the sidebar. Shows the avatar + role tag + bio, and
  // hosts the inline edit form (bio / role text fields + Save /
  // Cancel).
  //
  // App owns the "when does the card open" wiring: hover timers +
  // the `card` position object. ProfileCard owns "what does it
  // look like when open" — the render, the edit-form buffer, the
  // Save invoke path (which writes back through
  // `profileCatalog.updateProfile`). The `onClose` / `onEnter`
  // callbacks let App keep control of the close-grace timer so
  // hopping the ~6 px gap between the row and the card does not
  // dismiss it.
  //
  // Consumes:
  //   - profileCatalog                                     (store; the
  //     profile map + avatar thumb cache both live here so the
  //     card, the persona strip, and the thread head share a
  //     single fetch budget)
  //   - personaCatalog.list                                (store; the
  //     persona-name resolution mirrors what the strip does)
  //
  // Props:
  //   - card: { personaId, x, y }  — position + which row was
  //     hovered. Owned by App because the hover open-timer sets
  //     it.
  //   - onClose  — dismiss the card (App clears its own
  //     close-grace timer as part of the same operation).
  //   - onEnter  — cursor landed on the card itself; App cancels
  //     the pending close-grace timer so the card sticks.
  //   - onSetStatus  — flash a message in the App-owned status
  //     line (invoke failures during save).
  import { invoke } from "@tauri-apps/api/core";
  import type { PersonaProfileDto } from "./bindings";
  import { personaCatalog } from "./lib/stores/personas.svelte";
  import { profileCatalog } from "./lib/stores/profile.svelte";

  interface Props {
    card: { personaId: string; x: number; y: number };
    onClose: () => void;
    onEnter: () => void;
    onSetStatus: (msg: string) => void;
  }

  let { card, onClose, onEnter, onSetStatus }: Props = $props();

  // Edit-form buffer. Reset every time the card opens for a fresh
  // persona (guarded by the `card.personaId` derived below) or the
  // Cancel button fires; Save writes the buffer through
  // `set_persona_profile` and calls `profileCatalog.updateProfile`
  // with the returned row.
  let editing = $state(false);
  let form = $state<{ bio_short: string; role_tag: string }>({
    bio_short: "",
    role_tag: "",
  });

  // Reset the edit state whenever the hovered persona changes so
  // switching rows never carries a stale draft across.
  let lastPersonaId = $state<string | null>(null);
  $effect(() => {
    if (card.personaId !== lastPersonaId) {
      lastPersonaId = card.personaId;
      editing = false;
    }
  });

  function startEdit() {
    const p = profileCatalog.profiles.get(card.personaId) ?? null;
    form = {
      bio_short: p?.bio_short ?? "",
      role_tag: p?.role_tag ?? "",
    };
    editing = true;
  }

  async function save() {
    const pid = card.personaId;
    const current = profileCatalog.profiles.get(pid) ?? null;
    try {
      const next = await invoke<PersonaProfileDto>("set_persona_profile", {
        command: {
          persona_id: pid,
          // Avatar edit lives in a separate flow (the asset
          // detail's "Set as ... avatar" action); the inline
          // form only touches text fields, so echo the current
          // avatar back.
          avatar_asset_id: current?.avatar_asset_id ?? null,
          bio_short: form.bio_short.trim() || null,
          role_tag: form.role_tag.trim() || null,
        },
      });
      profileCatalog.updateProfile(pid, next);
      editing = false;
    } catch (error) {
      console.warn("set_persona_profile failed", error);
      onSetStatus(`profile error: ${JSON.stringify(error)}`);
    }
  }

  // Reactive reads through the store. Kept as `$derived` so any
  // mutation to `profileCatalog.profiles` / the avatar cache
  // repaints without a manual tick — mirrors how the old inline
  // `{@const}` block re-fired inside App.svelte.
  let profile = $derived(profileCatalog.profiles.get(card.personaId) ?? null);
  let persona = $derived(personaCatalog.list.data.find((p) => p.id === card.personaId));
  let avatarSrc = $derived(profileCatalog.avatarUrl(profile?.avatar_asset_id));
</script>

<div
  class="profile-card"
  style={`left: ${card.x}px; top: ${card.y}px;`}
  onmouseenter={onEnter}
  onmouseleave={onClose}
  role="tooltip"
>
  <div class="profile-card-head">
    <div class="profile-card-avatar">
      {#if avatarSrc}
        <img src={avatarSrc} alt="" />
      {:else}
        <span class="profile-card-avatar-fallback">○</span>
      {/if}
    </div>
    <div class="profile-card-name-block">
      <div class="profile-card-name">{persona?.name ?? card.personaId}</div>
      {#if profile?.role_tag}
        <span class="profile-card-role">{profile.role_tag}</span>
      {/if}
    </div>
  </div>
  {#if editing}
    <div class="profile-card-form">
      <label>
        <span>bio</span>
        <input
          type="text"
          bind:value={form.bio_short}
          placeholder="one-line note"
          maxlength="140"
        />
      </label>
      <label>
        <span>role</span>
        <input
          type="text"
          bind:value={form.role_tag}
          placeholder="companion / agent / friend / …"
          maxlength="40"
        />
      </label>
      <div class="profile-card-actions">
        <button class="profile-card-save" onclick={save}>Save</button>
        <button class="profile-card-cancel" onclick={() => (editing = false)}>Cancel</button>
      </div>
    </div>
  {:else}
    {#if profile?.bio_short}
      <p class="profile-card-bio">{profile.bio_short}</p>
    {:else}
      <p class="profile-card-bio dim">no bio — click ✎ to add</p>
    {/if}
    <div class="profile-card-actions">
      <button class="profile-card-edit" onclick={startEdit} title="Edit profile">✎ edit</button>
    </div>
  {/if}
</div>

<style>
  /* Copied verbatim from App.svelte (Phase C wave 8b extraction).
     Scoped-CSS namespace switch when this section moved into its
     own component severed the ancestor selector match, so we keep
     the whole cascade here — same duplication pattern as
     ModalityList (wave 4) / TagList (5a) / GroupsSection (5b-2) /
     SessionsView (6). */

  /* Floating Profile card — appears 300 ms after the pointer
     settles on a persona row. Positioned to the right of the row
     via inline coords in the template. */
  .profile-card {
    position: fixed;
    z-index: 800;
    background: var(--surface-raised);
    border: 1px solid var(--accent-line);
    border-radius: 8px;
    box-shadow: 0 12px 32px var(--shadow-color);
    padding: 0.7rem 0.85rem;
    min-width: 240px;
    max-width: 300px;
    font-size: 0.85rem;
  }
  .profile-card-head {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    margin-bottom: 0.55rem;
  }
  .profile-card-avatar {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: var(--accent-surface);
    overflow: hidden;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .profile-card-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .profile-card-avatar-fallback {
    color: var(--accent-ink);
    font-size: 1.1rem;
  }
  .profile-card-name-block {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }
  .profile-card-name {
    font-weight: 600;
    color: var(--ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .profile-card-role {
    font-size: 0.7rem;
    color: var(--accent-ink);
    background: var(--accent-surface);
    padding: 0.05rem 0.4rem;
    border-radius: 3px;
    align-self: flex-start;
  }
  .profile-card-bio {
    margin: 0 0 0.55rem;
    color: var(--ink-secondary);
    font-size: 0.8rem;
    line-height: 1.35;
  }
  .profile-card-bio.dim {
    color: var(--ink-faint);
    font-style: italic;
  }
  .profile-card-form {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }
  .profile-card-form label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .profile-card-form span {
    font-size: 0.7rem;
    color: var(--ink-secondary);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .profile-card-form input {
    padding: 0.25rem 0.4rem;
    border: 1px solid var(--line);
    border-radius: 4px;
    font-size: 0.85rem;
  }
  .profile-card-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.35rem;
  }
  .profile-card-save,
  .profile-card-cancel,
  .profile-card-edit {
    padding: 0.2rem 0.6rem;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--surface-raised);
    cursor: pointer;
    font-size: 0.75rem;
    color: var(--ink);
  }
  .profile-card-save {
    background: var(--accent-fill);
    color: var(--accent-on-fill);
    border-color: var(--accent-fill);
  }
  .profile-card-save:hover {
    background: var(--accent-fill-hover);
  }
  .profile-card-cancel:hover,
  .profile-card-edit:hover {
    background: var(--surface-hover);
  }
</style>
