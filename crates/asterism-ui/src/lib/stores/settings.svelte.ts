// Settings catalog — application preferences, backend-authoritative.
//
// Preferences used to live in `localStorage` (`asterism.clean_mode.v1`
// and friends). That put them out of reach of every non-webview
// consumer — a `--headless` core, the loopback HTTP server, and the job
// engine could not read a value the user had set — and left them
// outside the profile isolation that the database and index already
// get. They now live in the `app_setting` table, so one profile backup
// carries data *and* preferences, and `dev` / `dogfood` / `bench` stay
// separated for free.
//
// Resolution is `default → env → stored`, uniformly for every key: what
// the user picks wins, and an environment variable only supplies the
// value while nothing is stored. Each row carries the whole chain
// (`layers`), not just the winner, so a screen can show what a value is
// shadowing rather than presenting a number with no provenance.
//
// Scope:
//   - `list`: Resource over `list_settings` — every key in the closed
//     backend registry, already resolved. Rows carry the registry
//     metadata (`kind` / `min` / `max` / `env_var` / `summary`) plus
//     `layers`, so a settings screen needs no second round trip.
//   - `bool()` / `int()` / `text()`: typed reads of the resolved value.
//     Each takes the fallback the caller wants *before the first fetch
//     resolves* — the catalog cannot invent one, because the default
//     is the backend's to declare.
//   - `layerValue()`: the value a named layer contributes, or `null`
//     when that layer is not in the chain. Lets a caller answer "what
//     will this fall back to?" without re-deriving precedence.
//   - `isOverridden()`: whether a stored row exists, i.e. whether Reset
//     has anything to clear.
//   - `set()` / `reset()`: authoritative-re-read writes, same policy as
//     `modalityCatalog`'s settings-UI CRUD. Rejections propagate to the
//     caller; message display is the caller's.
//
// Reload wiring: App-side. `load()` is called once on mount; nothing
// here decides when to reload.
//
// Note on `set` / `reset`: both return the *resolved* row, so a caller
// repaints from what the application will actually use. A write can
// still resolve to the value that was already showing (setting a key to
// what it already was), so a control must not assume its own repaint is
// driven by the value changing.

import type { SettingDto } from "../../bindings";
import { SvelteMap } from "svelte/reactivity";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

class SettingsCatalog {
  list = new Resource(
    () => api<SettingDto[]>("list_settings"),
    [] as SettingDto[],
    "settingsCatalog.list",
  );

  // key → resolved row. Rebuilt whenever `list.data` reassigns.
  byKey = $derived.by(() => {
    const m = new SvelteMap<string, SettingDto>();
    for (const row of this.list.data) m.set(row.key, row);
    return m;
  });

  // Registry order, for a settings screen that wants to render all of
  // them. The backend orders the listing; this store never re-sorts.
  all = $derived(this.list.data);

  // Parsed value for `key`, or `fallback` when the key is not loaded
  // yet / not in the registry / stored as another shape. Callers supply
  // the same literal the backend registry declares, so the pre-fetch
  // frame matches the post-fetch one and the UI does not flip.
  #parse<T>(key: string, fallback: T, guard: (v: unknown) => v is T): T {
    const row = this.byKey.get(key);
    if (!row) return fallback;
    try {
      const parsed: unknown = JSON.parse(row.value_json);
      return guard(parsed) ? parsed : fallback;
    } catch {
      return fallback;
    }
  }

  bool(key: string, fallback: boolean): boolean {
    return this.#parse(key, fallback, (v): v is boolean => typeof v === "boolean");
  }

  int(key: string, fallback: number): number {
    return this.#parse(
      key,
      fallback,
      (v): v is number => typeof v === "number" && Number.isInteger(v),
    );
  }

  text(key: string, fallback: string): string {
    return this.#parse(key, fallback, (v): v is string => typeof v === "string");
  }

  // The value a named layer contributes, or `null` when that layer is
  // not part of the chain. `layerValue(k, "env")` answers "what does
  // Reset hand this key back to?" without the caller re-deriving the
  // precedence rule.
  layerValue(key: string, source: string): string | null {
    const layer = this.byKey.get(key)?.layers.find((l) => l.source === source);
    return layer?.value_json ?? null;
  }

  // Whether a stored row exists — i.e. whether Reset has anything to
  // clear. Distinct from "differs from the default": an env-supplied
  // value also differs, but there is nothing for the user to reset.
  isOverridden(key: string): boolean {
    return this.byKey.get(key)?.source === "stored";
  }

  async load(): Promise<void> {
    await this.list.load(undefined);
  }

  async set(key: string, value: boolean | number | string): Promise<void> {
    await api<SettingDto>("set_setting", {
      command: { key, value_json: JSON.stringify(value) },
    });
    await this.load();
  }

  async reset(key: string): Promise<void> {
    await api<SettingDto>("reset_setting", { command: { key } });
    await this.load();
  }

  // One-shot carry of the three preferences that used to live in
  // `localStorage`, so an existing profile does not silently revert to
  // the registry defaults on the first launch after the migration.
  //
  // Only a key still sitting at its default is carried: anything else
  // in force was set after the migration and must win over a stale
  // browser entry. (None of the three declares an `env_var`, so in
  // practice the alternative to `default` is always `stored`.) Each
  // legacy entry is removed once handled, so this is idempotent and
  // costs one `localStorage.getItem` per key per launch afterwards.
  //
  // Removable once every dogfood profile has launched at least one
  // build containing it — there is no other consumer to coordinate.
  async migrateLegacyLocalStorage(): Promise<void> {
    if (typeof localStorage === "undefined") return;
    const legacy: [string, string][] = [
      ["asterism.clean_mode.v1", SETTING_KEYS.cleanMode],
      // asterism.dialogue.show_messages.v1 died with the Dialogue slug
      // (asset-model v4 P3) — no carry target, the key is gone.
      ["asterism.import.auto_organize.v1", SETTING_KEYS.importAutoOrganize],
    ];
    for (const [oldKey, newKey] of legacy) {
      let raw: string | null = null;
      try {
        raw = localStorage.getItem(oldKey);
      } catch {
        return; // storage unavailable — nothing to carry
      }
      if (raw === null) continue;
      if (this.byKey.get(newKey)?.source === "default") {
        try {
          await this.set(newKey, raw === "1");
        } catch (e) {
          console.warn(`[settingsCatalog] carrying ${oldKey} failed:`, e);
          continue; // leave the entry for the next launch to retry
        }
      }
      try {
        localStorage.removeItem(oldKey);
      } catch {
        // Non-fatal: the guard above makes a re-run a no-op.
      }
    }
  }
}

// Exported as `settingsCatalog` for parallelism with `modalityCatalog`.
export const settingsCatalog = new SettingsCatalog();

// Registry keys this frontend reads. Mirrors the backend
// `SETTING_REGISTRY`; a typo here would silently fall back to the
// literal default forever, so the keys are named once and imported.
export const SETTING_KEYS = {
  cleanMode: "ui.clean_mode",
  importAutoOrganize: "import.auto_organize",
} as const;
