<script lang="ts">
  // SettingsPreferences — the application-settings section of the
  // settings panel. Renders every key the backend registry declares,
  // rather than a hand-maintained list of controls, so adding a key is
  // a backend-only change.
  //
  // State (0-prop, catalog-driven):
  //   - settingsCatalog.all       (registry order, backend-authoritative)
  //   - settingsCatalog.set / reset
  // Only `busy` (write serialisation) and a local error string live
  // here. The catalog re-reads after every write, so the controls
  // repaint from the resolved value rather than from optimistic state.
  //
  // Every control is editable. Resolution is `default → env → stored`,
  // so a value chosen here always wins — there is no state in which the
  // screen accepts a write that something else then discards.
  //
  // What the row has to convey instead is *provenance*: which layer the
  // shown value came from, and what it is covering up. `row.layers`
  // carries the whole chain (lowest precedence first), so the row
  // renders a trail — `built-in 0 → environment 8 → your choice 2` —
  // and Reset can say concretely what the value will fall back to.
  // Showing only the winner is what previously left a stored row
  // invisible underneath an env var, with no way to see or clear it.
  //
  // A layer can also be present but *rejected* (an export that does not
  // parse, a stored row outside the key's range). Those render struck
  // through with the reason: the same argument applies one level down —
  // a value that was supplied and thrown away is exactly what a user
  // cannot otherwise find out about. The winner is therefore the
  // highest *non-rejected* layer, not simply the last one.
  //
  // Int keys carry `min` / `max` from the registry. They are rendered
  // as input constraints for feedback only: the backend rejects an
  // out-of-range write from any caller, including a raw HTTP `PUT`.
  import type { SettingDto } from "./bindings";
  import { settingsCatalog } from "./lib/stores/settings.svelte";

  let busy = $state(false);
  let error = $state<string | null>(null);

  // Keys whose effect is only picked up when the process starts. The
  // registry says so in prose; this list is what drives the badge.
  const STARTUP_ONLY = new Set(["jobs.concurrency"]);

  function errMsg(e: unknown): string {
    if (typeof e === "string") return e;
    if (e && typeof e === "object" && "message" in e) {
      return String((e as { message: unknown }).message);
    }
    return String(e);
  }

  // Serialises writes issued *from this component*. A control
  // elsewhere writing the same key (App.svelte's sidebar clean-mode
  // button and dialogue show-messages checkbox have their own guard)
  // is not ordered against these.
  //
  // That is tolerable rather than accidental: both paths recompute
  // from the catalog's resolved value and re-read the whole list
  // afterwards, and `Resource`'s generation guard drops superseded
  // responses — so the worst case is a transient stale render that the
  // winning read repaints, not a divergent stored value.
  async function run(fn: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    error = null;
    try {
      await fn();
    } catch (e) {
      error = errMsg(e);
    } finally {
      busy = false;
    }
  }

  function parsed(row: SettingDto): unknown {
    try {
      return JSON.parse(row.value_json);
    } catch {
      return null;
    }
  }

  function boolValue(row: SettingDto): boolean {
    return parsed(row) === true;
  }

  function intValue(row: SettingDto): number {
    const v = parsed(row);
    return typeof v === "number" ? v : 0;
  }

  function textValue(row: SettingDto): string {
    const v = parsed(row);
    return typeof v === "string" ? v : "";
  }

  // Reset is offered only when a stored row exists, because that is the
  // only thing it deletes. A value coming from the env layer also
  // differs from the default, but there is nothing there for the user
  // to clear.
  function isOverridden(row: SettingDto): boolean {
    return row.source === "stored";
  }

  function rangeHint(row: SettingDto): string {
    if (row.min === null || row.max === null) return "";
    return ` (${row.min}–${row.max})`;
  }

  // Human label per layer. `stored` reads as "your choice", because
  // that is the distinction the row is drawing. An unrecognised source
  // renders as itself rather than falling into one of the three labels
  // — a fourth layer must not silently claim to be the user's.
  function layerLabel(source: string): string {
    if (source === "default") return "built-in";
    if (source === "env") return "environment";
    if (source === "stored") return "your choice";
    return source;
  }

  // The chain as a single line: every layer that supplied a value, in
  // precedence order, with the one in force marked and any rejected one
  // struck through. Only worth rendering when something is stacked — a
  // key sitting on its default alone has no story to tell.
  function hasChain(row: SettingDto): boolean {
    return row.layers.length > 1;
  }

  // Whether this layer is the one actually in force. The winner is the
  // highest non-rejected layer, which is not always the last entry: a
  // rejected export sits above the default but loses to it.
  function isEffective(row: SettingDto, index: number): boolean {
    const layer = row.layers[index];
    if (layer.rejected !== null) return false;
    return row.layers.slice(index + 1).every((l) => l.rejected !== null);
  }

  // What Reset lands on: the highest layer below the stored row that is
  // actually usable. Skipping rejected layers matters — an exported but
  // invalid variable must not be advertised as the fallback.
  function resetTarget(row: SettingDto): string {
    const beneath = row.layers
      .slice(0, -1)
      .filter((l) => l.rejected === null)
      .pop();
    return beneath
      ? `${layerLabel(beneath.source)} ${beneath.value_json}`
      : "the built-in default";
  }
</script>

<h4>Preferences</h4>

{#if settingsCatalog.list.error}
  <p class="settings-error">
    Could not load settings: {settingsCatalog.list.error}
  </p>
{/if}
{#if error}
  <p class="settings-error">{error}</p>
{/if}

<ul class="pref-list">
  {#each settingsCatalog.all as row (row.key)}
    <li class="pref-row">
      <div class="pref-main">
        {#if row.kind === "bool"}
          <label class="settings-toggle">
            <input
              type="checkbox"
              checked={boolValue(row)}
              disabled={busy}
              onchange={async (e) => {
                const el = e.currentTarget;
                const next = el.checked;
                await run(() => settingsCatalog.set(row.key, next));
                // Re-assert from the settled value: a rejected write
                // leaves the resolved value unchanged, and an unchanged
                // value does not repaint a one-way `checked=`.
                el.checked = boolValue(
                  settingsCatalog.byKey.get(row.key) ?? row,
                );
              }}
            />
            <span class="pref-key">{row.key}</span>
          </label>
        {:else}
          <label class="pref-field">
            <span class="pref-key">{row.key}{rangeHint(row)}</span>
            {#if row.kind === "int"}
              <input
                type="number"
                class="pref-input"
                value={intValue(row)}
                min={row.min ?? undefined}
                max={row.max ?? undefined}
                disabled={busy}
                onchange={async (e) => {
                  const el = e.currentTarget;
                  // `<input type="number">` sanitises anything it
                  // cannot parse to the empty string, and `Number("")`
                  // is 0 — which for `jobs.concurrency` is a *valid*
                  // value, so clearing the field would silently store
                  // "follow the machine" instead of being rejected.
                  // The empty case has to be checked before the
                  // integer check, not by it.
                  const raw = el.value.trim();
                  const next = Number(raw);
                  if (raw === "" || !Number.isInteger(next)) {
                    error = `${row.key} must be a whole number`;
                    el.value = String(intValue(row));
                    return;
                  }
                  await run(() => settingsCatalog.set(row.key, next));
                  el.value = String(
                    intValue(settingsCatalog.byKey.get(row.key) ?? row),
                  );
                }}
              />
            {:else}
              <input
                type="text"
                class="pref-input"
                value={textValue(row)}
                disabled={busy}
                onchange={async (e) => {
                  const el = e.currentTarget;
                  const next = el.value;
                  await run(() => settingsCatalog.set(row.key, next));
                  el.value = textValue(
                    settingsCatalog.byKey.get(row.key) ?? row,
                  );
                }}
              />
            {/if}
          </label>
        {/if}
        <div class="pref-actions">
          {#if STARTUP_ONLY.has(row.key)}
            <span class="pref-badge" title="Takes effect on the next launch"
              >restart</span
            >
          {/if}
          {#if isOverridden(row)}
            <button
              type="button"
              class="pref-reset"
              disabled={busy}
              title="Clear your choice and fall back to {resetTarget(row)}"
              onclick={() => run(() => settingsCatalog.reset(row.key))}
            >Reset</button>
          {/if}
        </div>
      </div>
      {#if hasChain(row)}
        <!-- Provenance trail: every layer that has a value, in
             precedence order, with the one in force marked. This is the
             piece that makes a shadowed value visible instead of
             silently lost. -->
        <p class="pref-chain">
          {#each row.layers as layer, i (layer.source)}
            {#if i > 0}<span class="pref-chain-arrow">→</span>{/if}
            <span
              class="pref-chain-item"
              class:effective={isEffective(row, i)}
              class:rejected={layer.rejected !== null}
              title={layer.rejected ?? undefined}
            >
              {layerLabel(layer.source)}
              <code>{layer.value_json}</code>
              {#if layer.origin}<span class="pref-chain-origin">({layer.origin})</span>{/if}
            </span>
          {/each}
        </p>
        {#each row.layers.filter((l) => l.rejected !== null) as bad (bad.source)}
          <p class="pref-rejected-note">
            Ignored: {layerLabel(bad.source)}{bad.origin
              ? ` (${bad.origin})`
              : ""} — {bad.rejected}
          </p>
        {/each}
      {/if}
      <p class="settings-hint">
        {row.summary}
        {#if row.env_var && !row.layers.some((l) => l.source === "env")}
          <span class="pref-env-note">
            {row.env_var} applies when you have not set a value here.
          </span>
        {/if}
      </p>
    </li>
  {/each}
</ul>

<style>
  /* `.settings-hint` / `.settings-toggle` moved here from App.svelte
     with the markup that uses them (Svelte scopes styles per
     component). Tone matches the rest of the settings panel. */
  .settings-hint {
    margin-top: 0.5rem;
    font-size: 0.75rem;
    color: #999;
    font-style: italic;
  }
  .settings-toggle {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.85rem;
    color: #333;
    cursor: pointer;
  }
  .settings-toggle input[type="checkbox"] {
    width: 1rem;
    height: 1rem;
    cursor: pointer;
  }
  .pref-list {
    list-style: none;
    margin: 0 0 0.6rem;
    padding: 0;
  }
  .pref-row {
    padding: 0.35rem 0;
    border-bottom: 1px solid var(--hairline, rgba(255, 255, 255, 0.06));
  }
  .pref-row:last-child {
    border-bottom: none;
  }
  .pref-main {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .pref-field {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1;
    min-width: 0;
  }
  .pref-key {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.78rem;
    opacity: 0.9;
  }
  .pref-input {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.78rem;
  }
  .pref-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .pref-badge {
    font-size: 0.66rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.55;
    border: 1px solid currentColor;
    border-radius: 3px;
    padding: 0 0.25rem;
  }
  .pref-reset {
    font-size: 0.72rem;
    background: none;
    border: 1px solid var(--hairline, rgba(255, 255, 255, 0.18));
    border-radius: 3px;
    padding: 0.1rem 0.4rem;
    cursor: pointer;
    color: inherit;
    opacity: 0.75;
  }
  .pref-reset:hover:not(:disabled) {
    opacity: 1;
  }
  .pref-chain {
    margin: 0.25rem 0 0;
    font-size: 0.72rem;
    color: #888;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
  }
  .pref-chain-item {
    opacity: 0.7;
  }
  /* The layer in force reads at full strength; the ones it shadows
     stay legible but recede. */
  .pref-chain-item.effective {
    opacity: 1;
    font-weight: 600;
  }
  .pref-chain-item code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  .pref-chain-origin {
    opacity: 0.8;
  }
  .pref-chain-arrow {
    opacity: 0.4;
  }
  .pref-chain-item.rejected {
    text-decoration: line-through;
    opacity: 0.5;
  }
  .pref-rejected-note {
    margin: 0.15rem 0 0;
    font-size: 0.72rem;
    color: #b06b2c;
  }
  .pref-env-note {
    display: block;
    opacity: 0.75;
  }
  .settings-error {
    color: var(--danger, #e2665b);
    font-size: 0.78rem;
    margin: 0.2rem 0;
  }
</style>
