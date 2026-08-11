<script lang="ts">
  // SettingsModalities — Modality master management section, rendered
  // inside the App settings panel. The modality master is editable from
  // the screen rather than from a config file: add / rename / reclass /
  // hide / reorder / count=0 delete of the backend-authoritative master,
  // plus a one-click "Register" for unregistered slugs (importer escape
  // hatch → formal registration).
  //
  // State (0-prop, catalog-driven):
  //   - modalityCatalog.all / .unregistered  (source rows)
  //   - CRUD via modalityCatalog.create / update / remove / reorder
  // Only `busy` (connect-flooding guard) + local error strings live in
  // the component; every persistent field is catalog-owned. Writes
  // authoritative-re-read (catalog owns that); this file just surfaces
  // the thrown error message.
  import { modalityCatalog } from "./lib/stores/modality.svelte";

  const SLUG_PATTERN = "[a-z0-9_-]{1,64}";
  const SLUG_RE = new RegExp(`^${SLUG_PATTERN}$`);

  let busy = $state(false);
  let error = $state<string | null>(null);

  let newSlug = $state("");
  let newLabel = $state("");
  let newTerminal = $state(false);
  let addError = $state<string | null>(null);

  function errMsg(e: unknown): string {
    if (typeof e === "string") return e;
    if (e && typeof e === "object" && "message" in e) {
      return String((e as { message: unknown }).message);
    }
    return String(e);
  }

  // Serialize writes: a single in-flight guard both prevents double
  // submits and keeps the authoritative re-read from racing.
  async function run(
    fn: () => Promise<void>,
    onErr: (msg: string) => void,
  ): Promise<void> {
    if (busy) return;
    busy = true;
    onErr("");
    try {
      await fn();
    } catch (e) {
      onErr(errMsg(e));
    } finally {
      busy = false;
    }
  }

  function move(slug: string, dir: -1 | 1): void {
    const slugs = modalityCatalog.all.map((r) => r.slug);
    const i = slugs.indexOf(slug);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= slugs.length) return;
    [slugs[i], slugs[j]] = [slugs[j], slugs[i]];
    run(() => modalityCatalog.reorder(slugs), (m) => (error = m || null));
  }

  function commitLabel(slug: string, current: string, next: string): void {
    const v = next.trim();
    if (v === "" || v === current) return;
    run(() => modalityCatalog.update(slug, { label: v }), (m) => (error = m || null));
  }

  function commitTerminal(slug: string, terminal: boolean): void {
    run(() => modalityCatalog.update(slug, { terminal }), (m) => (error = m || null));
  }

  function commitHidden(slug: string, hidden: boolean): void {
    run(() => modalityCatalog.update(slug, { hidden }), (m) => (error = m || null));
  }

  function del(slug: string): void {
    run(() => modalityCatalog.remove(slug), (m) => (error = m || null));
  }

  function add(): void {
    const slug = newSlug.trim();
    const label = newLabel.trim() || slug;
    if (!SLUG_RE.test(slug)) {
      addError = "slug must match [a-z0-9_-] (1–64 chars)";
      return;
    }
    run(
      async () => {
        await modalityCatalog.create({
          slug,
          label,
          terminal: newTerminal,
          sort_order: modalityCatalog.all.length,
          hidden: false,
          cover_template: null,
        });
        newSlug = "";
        newLabel = "";
        newTerminal = false;
      },
      (m) => (addError = m || null),
    );
  }

  function register(slug: string): void {
    run(
      () =>
        modalityCatalog.create({
          slug,
          label: slug,
          terminal: false,
          sort_order: modalityCatalog.all.length,
          hidden: false,
          cover_template: null,
        }),
      (m) => (error = m || null),
    );
  }
</script>

<h4>Modalities</h4>

{#if error}
  <p class="modality-error">{error}</p>
{/if}

<table class="modality-table">
  <thead>
    <tr>
      <th class="order">Order</th>
      <th>Slug</th>
      <th>Label</th>
      <th>Terminal</th>
      <th class="num">Count</th>
      <th class="center">Hidden</th>
      <th></th>
    </tr>
  </thead>
  <tbody>
    {#each modalityCatalog.all as row, i (row.slug)}
      <tr>
        <td class="order">
          <button
            type="button"
            title="Move up"
            disabled={busy || i === 0}
            onclick={() => move(row.slug, -1)}>▲</button
          >
          <button
            type="button"
            title="Move down"
            disabled={busy || i === modalityCatalog.all.length - 1}
            onclick={() => move(row.slug, 1)}>▼</button
          >
        </td>
        <td class="slug">{row.slug}</td>
        <td>
          <input
            class="label-input"
            type="text"
            value={row.label}
            disabled={busy}
            onblur={(e) => commitLabel(row.slug, row.label, e.currentTarget.value)}
            onkeydown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
          />
        </td>
        <td class="center">
          <input
            type="checkbox"
            checked={row.terminal}
            disabled={busy}
            onchange={(e) => commitTerminal(row.slug, e.currentTarget.checked)}
            title="Read as a terminal transcript"
          />
        </td>
        <td class="num">{row.asset_count}</td>
        <td class="center">
          <input
            type="checkbox"
            checked={row.hidden}
            disabled={busy}
            onchange={(e) => commitHidden(row.slug, e.currentTarget.checked)}
          />
        </td>
        <td class="center">
          <button
            type="button"
            class="del"
            disabled={busy || row.asset_count > 0}
            title={row.asset_count > 0
              ? "In use — hide it instead of deleting"
              : "Delete"}
            onclick={() => del(row.slug)}>✕</button
          >
        </td>
      </tr>
    {/each}
  </tbody>
</table>

<div class="add-row">
  <input
    class="add-slug"
    type="text"
    placeholder="slug"
    pattern={SLUG_PATTERN}
    bind:value={newSlug}
    disabled={busy}
  />
  <input
    class="add-label"
    type="text"
    placeholder="label"
    bind:value={newLabel}
    disabled={busy}
  />
  <label class="terminal-toggle">
    <input type="checkbox" bind:checked={newTerminal} disabled={busy} />
    terminal
  </label>
  <button type="button" class="add-btn" disabled={busy} onclick={add}>Add</button>
</div>
{#if addError}
  <p class="modality-error">{addError}</p>
{/if}

{#if modalityCatalog.unregistered.length > 0}
  <p class="unregistered-head">Unregistered slugs (carried by assets):</p>
  <ul class="unregistered">
    {#each modalityCatalog.unregistered as u (u.slug)}
      <li>
        <span class="slug">{u.slug}</span>
        <span class="num">({u.count})</span>
        <button
          type="button"
          class="register-btn"
          disabled={busy}
          onclick={() => register(u.slug)}>Register</button
        >
      </li>
    {/each}
  </ul>
{/if}

<style>
  /* Mirror the settings-panel `.shortcut-table` tone (App.svelte):
     #f7f5ee header, #eee row borders, mono slug cells. Kept scoped
     here rather than promoted to a shared stylesheet. */
  .modality-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  .modality-table th {
    text-align: left;
    padding: 0.35rem 0.6rem;
    background: #f7f5ee;
    color: #556;
    font-weight: 600;
  }
  .modality-table td {
    padding: 0.3rem 0.6rem;
    border-top: 1px solid #eee;
    color: #333;
    vertical-align: middle;
  }
  .modality-table th.num,
  .modality-table td.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .modality-table th.center,
  .modality-table td.center {
    text-align: center;
  }
  .modality-table td.slug,
  .unregistered .slug {
    font-family: ui-monospace, "SF Mono", monospace;
    color: #4a3a90;
    white-space: nowrap;
  }
  td.order {
    white-space: nowrap;
    width: 1%;
  }
  td.order button {
    background: none;
    border: none;
    cursor: pointer;
    color: #888;
    font-size: 0.75rem;
    padding: 0 0.1rem;
    line-height: 1;
  }
  td.order button:hover:not(:disabled) {
    color: #223;
  }
  td.order button:disabled {
    color: #ddd;
    cursor: default;
  }
  .label-input {
    width: 100%;
    box-sizing: border-box;
    padding: 0.2rem 0.35rem;
    border: 1px solid #ddd;
    border-radius: 4px;
    font-size: 0.8rem;
    font-family: inherit;
    color: #333;
  }
  .terminal-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: #555;
  }
  .del {
    background: none;
    border: none;
    cursor: pointer;
    color: #b46;
    font-size: 0.8rem;
  }
  .del:hover:not(:disabled) {
    color: #922;
  }
  .del:disabled {
    color: #ddd;
    cursor: default;
  }
  .add-row {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    margin-top: 0.7rem;
  }
  .add-slug,
  .add-label {
    padding: 0.25rem 0.4rem;
    border: 1px solid #ddd;
    border-radius: 4px;
    font-size: 0.8rem;
    font-family: inherit;
    color: #333;
  }
  .add-slug {
    width: 8rem;
    font-family: ui-monospace, "SF Mono", monospace;
  }
  .add-label {
    flex: 1;
  }
  .add-btn,
  .register-btn {
    padding: 0.25rem 0.7rem;
    border: 1px solid #c9c4e0;
    border-radius: 4px;
    background: #f2f0fb;
    color: #4a3a90;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }
  .add-btn:hover:not(:disabled),
  .register-btn:hover:not(:disabled) {
    background: #e7e3f7;
  }
  .add-btn:disabled,
  .register-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .unregistered-head {
    margin: 1rem 0 0.3rem;
    font-size: 0.78rem;
    color: #888;
  }
  .unregistered {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .unregistered li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0;
    font-size: 0.82rem;
  }
  .unregistered .num {
    color: #999;
    font-variant-numeric: tabular-nums;
  }
  .unregistered .register-btn {
    margin-left: auto;
  }
  .modality-error {
    margin: 0.5rem 0;
    padding: 0.4rem 0.6rem;
    background: #fdecec;
    border: 1px solid #f3c6c6;
    border-radius: 4px;
    color: #922;
    font-size: 0.8rem;
  }
</style>
