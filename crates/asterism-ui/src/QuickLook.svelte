<script lang="ts">
  // QuickLook — the Space-key peek overlay (W3).
  // The Finder Quick Look tier of the open grammar: a pure read-only
  // glance sitting between the grid card and the full DetailPane —
  // no metadata editing, no comments, no zoom stage. App's keymap
  // owns the lifecycle (Space toggle, ←/→ retarget via selection,
  // Enter escalates to detail, ⇧Space opens the constellation); this
  // component only renders the target and mirrors close / escalate
  // for the pointer path (✕ button, backdrop click, header button).
  //
  // Props (App-owned grid state — the target follows the selection):
  //   - card — hydrated target card (`null` renders nothing)
  //   - text — full body for text-shaped modalities (null = loading
  //     failed / unreadable → cover fallback)
  //   - textLoading — fetch in flight (body area shows a pulse)
  //   - onClose / onOpenDetail — pointer mirrors of Space / Enter
  import { untrack } from "svelte";
  import type { AssetCardDto } from "./bindings";
  import {
    fmtDateTime,
    personaName,
    pickDetailMode,
    renderMarkdown,
    type DetailMode,
  } from "./lib/formatters";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";
  import { modalityCatalog } from "./lib/stores/modality.svelte";

  interface Props {
    card: AssetCardDto | null;
    text: string | null;
    textLoading: boolean;
    onClose: () => void;
    onOpenDetail: (id: string) => void;
  }

  let { card, text, textLoading, onClose, onOpenDetail }: Props = $props();

  // Same 4-mode strip as DetailPane. Auto-pick when the target card
  // or the fetched body changes; the user's manual pick sticks until
  // the target shifts. `qlModeUserPicked` resets on card swap so the
  // sniffer runs again for the next preview.
  let qlMode = $state<DetailMode>("md");
  let qlModeUserPicked = $state(false);
  let lastCardId = $state<string | null>(null);

  $effect(() => {
    const id = card?.id ?? null;
    if (id !== lastCardId) {
      lastCardId = id;
      untrack(() => {
        qlModeUserPicked = false;
      });
    }
    if (!qlModeUserPicked && !textLoading) {
      // "term" is the one reading the classification decides; the rest
      // of the shape is sniffed from the text itself.
      const kind =
        card && modalityCatalog.isTerminal(card.modality) ? "term" : null;
      const picked = pickDetailMode(text, kind, card?.labels);
      untrack(() => {
        qlMode = picked;
      });
    }
  });
</script>

{#if card}
  <!-- Backdrop closes only on a direct hit (target === currentTarget,
       ThreadDrawer pattern) so the panel needs no stopPropagation
       handlers of its own — window-level keymap (Space / ←→ / Esc)
       keeps flowing. -->
  <div
    class="ql-backdrop"
    onclick={(e) => e.target === e.currentTarget && onClose()}
    onkeydown={(e) => e.key === "Enter" && e.target === e.currentTarget && onClose()}
    role="button"
    tabindex="-1"
    aria-label="Close preview"
  >
    <div class="ql-panel" role="dialog" tabindex="-1">
      <header class="ql-head">
        <span class="ql-badge">{card.modality}</span>
        <span class="ql-persona">{personaName(card.persona_id)}</span>
        <!-- Which agent produced this, when one was asserted. Rendered
             only then: absent means unrecorded, and a "—" in the strip
             would read as "made by hand". The card carries the slug, so
             the peek answers it without escalating to the detail pane. -->
        {#if card.operator_ai}
          <span class="ql-operator" title="Operator — the agent that performed the operation">
            {card.operator_ai}
          </span>
        {/if}
        <span class="ql-date">{fmtDateTime(card.occurred_at_ms)}</span>
        <button
          class="ql-open"
          onclick={() => onOpenDetail(card.id)}
          title="Open detail (Enter)"
        >⤢ detail</button>
        <button class="ql-close" onclick={onClose} aria-label="Close preview">✕</button>
      </header>
      {#if card.media === "image" || card.media === "video"}
        {#if card.source_locator}
          <!-- Video shows its extracted frame here rather than a
               player: Quick Look is the fast peek (space bar), and
               `detailSrc` already has the frame the grid painted.
               Playback lives in the detail pane. -->
          <img
            class="ql-image"
            src={thumbCatalog.detailSrc(card.source_locator, card.id, card.media)}
            alt={card.cover ?? ""}
          />
        {:else}
          <!-- Light card not yet hydrated (virtualised out of the
               viewport window) — no locator to serve yet. -->
          <p class="ql-loading">loading…</p>
        {/if}
      {:else if textLoading}
        <p class="ql-loading">loading…</p>
      {:else}
        <!-- Mode chip strip — mirrors DetailPane so the same muscle
             memory (md/raw/html/term) applies at the peek tier. -->
        <div class="ql-mode-strip">
          {#each ["md", "raw", "html", "term"] as mode (mode)}
            <button
              class="ql-mode-chip"
              class:active={qlMode === mode}
              onclick={() => {
                qlMode = mode as DetailMode;
                qlModeUserPicked = true;
              }}
            >
              {mode}
            </button>
          {/each}
        </div>
        {#if qlMode === "md"}
          <div class="ql-text">
            <!-- eslint-disable-next-line svelte/no-at-html-tags — renderMarkdown sanitizes -->
            {@html renderMarkdown(text ?? card.cover ?? "(no text)")}
          </div>
        {:else if qlMode === "html"}
          <!-- svelte-ignore a11y_missing_attribute -->
          <iframe
            class="ql-html"
            sandbox="allow-same-origin"
            srcdoc={text ?? card.cover ?? ""}
          ></iframe>
        {:else if qlMode === "term"}
          <pre class="ql-term">{text ?? card.cover ?? "(no text)"}</pre>
        {:else}
          <pre class="ql-raw">{text ?? card.cover ?? "(no text)"}</pre>
        {/if}
      {/if}
      <footer class="ql-hint">
        Space close · ← → move · Enter detail · ⇧Space constellation
      </footer>
    </div>
  </div>
{/if}

<style>
  /* Quick Look floats over the grid rather than replacing it — the
     point is a glance, not a modal context switch. `--wash-down` is
     what the drawers and dialogs take as well; the two that reach for
     `--scrim` are the ones that cover the whole shell, and this is
     deliberately not one of them. Enough to keep the panel edges
     readable on a busy grid, not enough to make the grid a memory. */
  .ql-backdrop {
    position: fixed;
    inset: 0;
    background: var(--wash-down);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 40;
  }

  .ql-panel {
    background: var(--surface-raised);
    border: 1px solid var(--accent-line);
    border-radius: 12px;
    box-shadow: 0 12px 36px var(--shadow-color-strong);
    width: min(720px, 76vw);
    max-height: 78vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .ql-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.55rem 0.8rem;
    border-bottom: 1px solid var(--accent-line);
    font-size: 0.75rem;
    color: var(--ink-secondary);
  }

  .ql-badge {
    padding: 0.05rem 0.5rem;
    border-radius: 999px;
    background: var(--accent-surface);
    color: var(--accent-ink);
    text-transform: uppercase;
    font-size: 0.6rem;
    letter-spacing: 0.04em;
  }

  .ql-persona {
    color: var(--accent-ink);
    font-weight: 600;
  }

  .ql-operator {
    padding: 0.05rem 0.45rem;
    border-radius: 999px;
    border: 1px solid var(--accent-line);
    color: var(--accent-ink);
    font-size: 0.62rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  .ql-date {
    color: var(--ink-faint);
    font-variant-numeric: tabular-nums;
  }

  .ql-open {
    margin-left: auto;
    padding: 0.1rem 0.55rem;
    background: var(--accent-surface);
    color: var(--accent-ink);
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    font-size: 0.65rem;
    cursor: pointer;
  }
  .ql-open:hover {
    background: var(--accent-surface-strong);
  }

  .ql-close {
    width: 22px;
    height: 22px;
    padding: 0;
    line-height: 20px;
    text-align: center;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: var(--ink-faint);
    font-size: 0.75rem;
    cursor: pointer;
  }
  .ql-close:hover {
    background: var(--accent-surface);
    color: var(--ink);
  }

  .ql-image {
    display: block;
    max-width: 100%;
    max-height: calc(78vh - 5.4rem);
    object-fit: contain;
    margin: 0 auto;
    background: var(--accent-surface);
  }

  .ql-mode-strip {
    display: flex;
    gap: 0.25rem;
    padding: 0.35rem 0.6rem;
    border-bottom: 1px solid var(--accent-line);
    background: var(--accent-surface);
    flex-shrink: 0;
  }
  .ql-mode-chip {
    padding: 0.1rem 0.5rem;
    border: 1px solid var(--accent-line);
    border-radius: 3px;
    background: var(--surface-raised);
    cursor: pointer;
    font-size: 0.68rem;
    color: var(--ink-secondary);
    text-transform: lowercase;
    font-family: ui-monospace, "SF Mono", monospace;
  }
  .ql-mode-chip:hover {
    background: var(--accent-surface);
  }
  .ql-mode-chip.active {
    background: var(--accent-fill);
    border-color: var(--accent-fill);
    color: var(--accent-on-fill);
  }

  .ql-text {
    overflow-y: auto;
    padding: 0.8rem 1rem;
    font-size: 0.85rem;
    line-height: 1.55;
    color: var(--ink);
  }
  .ql-raw,
  .ql-term {
    overflow: auto;
    margin: 0;
    padding: 0.8rem 1rem;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.8rem;
    line-height: 1.5;
    font-family: ui-monospace, "SF Mono", "Menlo", monospace;
  }
  .ql-raw {
    color: var(--ink);
    background: var(--surface-raised);
  }
  .ql-term {
    background: var(--surface-stage);
    color: var(--ink-secondary);
    font-size: 0.82rem;
    line-height: 1.55;
    text-shadow: 0 0 1px var(--shadow-color-strong);
  }
  .ql-html {
    display: block;
    width: 100%;
    height: calc(78vh - 6rem);
    border: 1px solid var(--accent-line);
    background: var(--surface-raised);
  }

  .ql-loading {
    padding: 2rem;
    text-align: center;
    color: var(--accent-ink);
    font-size: 0.8rem;
  }

  .ql-hint {
    padding: 0.35rem 0.8rem;
    border-top: 1px solid var(--accent-line);
    font-size: 0.62rem;
    color: var(--accent-ink-dim);
    text-align: center;
    user-select: none;
  }
</style>
