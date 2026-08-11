// settingsCatalog unit tests.
//
// The catalog is a singleton, so `beforeEach` resets the underlying
// Resource (the H1 hardening that makes these testable — same shape as
// `catalog-derived.test.ts`) and every test loads its own row set
// through the mocked api choke point. Without the reset the "before
// the first load" case would only succeed by running first.
//
// What is worth pinning here is the typed-read contract, because the
// values arrive as JSON *text* and a silent mis-parse would show the
// wrong toggle position with no error anywhere:
//
//   - before the first fetch, and for keys outside the registry, the
//     caller's fallback wins (the frame must not flip for a user who
//     changed nothing);
//   - a value of the wrong shape falls back rather than coercing
//     (`"true"` is not `true`, `1` is not `true`);
//   - `isPinned` reports the env layer, which is what makes a control
//     read-only.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { SETTING_KEYS, settingsCatalog } from "./settings.svelte";

vi.mock("../api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

// Builds a row the way the backend would: the chain always starts with
// the default, and the winning layer is last, so `value_json` and
// `source` are derived rather than free-floating. Passing them
// separately would let a test assert a shape the backend cannot
// produce.
function row(
  key: string,
  value_json: string,
  opts: {
    kind?: string;
    source?: string;
    default_json?: string;
    env?: { value_json: string; origin: string; rejected?: string };
  } = {},
) {
  const source = opts.source ?? "stored";
  const default_json = opts.default_json ?? "false";
  const layers: {
    source: string;
    value_json: string;
    origin: string | null;
    rejected: string | null;
  }[] = [
    { source: "default", value_json: default_json, origin: null, rejected: null },
  ];
  if (opts.env) {
    layers.push({
      source: "env",
      value_json: opts.env.value_json,
      origin: opts.env.origin,
      rejected: opts.env.rejected ?? null,
    });
  }
  if (source !== "default") {
    // The winning layer carries the effective value; anything below it
    // stays as seeded above.
    const existing = layers.find((l) => l.source === source);
    if (existing) existing.value_json = value_json;
    else layers.push({ source, value_json, origin: null, rejected: null });
  } else {
    layers[0].value_json = value_json;
  }
  return {
    key,
    kind: opts.kind ?? "bool",
    value_json,
    source,
    layers,
    env_var: opts.env?.origin ?? null,
    min: null,
    max: null,
    summary: "",
  };
}

describe("settingsCatalog", () => {
  beforeEach(() => {
    apiMock.mockReset();
    settingsCatalog.list.reset();
  });

  it("returns the caller's fallback before the first load", () => {
    // Nothing loaded — the catalog cannot know the backend default, so
    // the literal the caller passes is what renders. This is the frame
    // the grid paints on startup, hence the same key the app reads.
    expect(settingsCatalog.bool(SETTING_KEYS.cleanMode, true)).toBe(true);
    expect(settingsCatalog.bool(SETTING_KEYS.importAutoOrganize, true)).toBe(
      true,
    );
    expect(settingsCatalog.text("dispatch.comfy.endpoint", "x")).toBe("x");
    expect(settingsCatalog.int("jobs.concurrency", 7)).toBe(7);
  });

  it("returns the caller's fallback for a key outside the registry", async () => {
    apiMock.mockResolvedValueOnce([row(SETTING_KEYS.cleanMode, "true")]);
    await settingsCatalog.load();
    expect(settingsCatalog.bool("ui.no_such_key", true)).toBe(true);
  });

  it("parses resolved values by declared shape", async () => {
    apiMock.mockResolvedValueOnce([
      row(SETTING_KEYS.cleanMode, "true"),
      row("jobs.concurrency", "8", { kind: "int", default_json: "0" }),
      row("dispatch.comfy.endpoint", '"http://h:1"', {
        kind: "text",
        default_json: '""',
      }),
    ]);
    await settingsCatalog.load();

    expect(settingsCatalog.bool(SETTING_KEYS.cleanMode, false)).toBe(true);
    expect(settingsCatalog.int("jobs.concurrency", 0)).toBe(8);
    expect(settingsCatalog.text("dispatch.comfy.endpoint", "")).toBe(
      "http://h:1",
    );
  });

  it("falls back instead of coercing a mismatched shape", async () => {
    apiMock.mockResolvedValueOnce([
      row(SETTING_KEYS.cleanMode, '"true"'),
      row("jobs.concurrency", "1.5", { kind: "int" }),
      row("dispatch.comfy.endpoint", "42", { kind: "text" }),
      row(SETTING_KEYS.importAutoOrganize, "not json"),
    ]);
    await settingsCatalog.load();

    expect(settingsCatalog.bool(SETTING_KEYS.cleanMode, false)).toBe(false);
    expect(settingsCatalog.int("jobs.concurrency", 0)).toBe(0);
    expect(settingsCatalog.text("dispatch.comfy.endpoint", "d")).toBe("d");
    expect(settingsCatalog.bool(SETTING_KEYS.importAutoOrganize, true)).toBe(
      true,
    );
  });

  it("reports whether a stored row exists, so Reset is offered only when it can clear one", async () => {
    apiMock.mockResolvedValueOnce([
      // env-supplied: differs from the default, but nothing to reset.
      row("jobs.concurrency", "8", {
        kind: "int",
        source: "env",
        default_json: "0",
        env: { value_json: "8", origin: "ASTERISM_JOB_CONCURRENCY" },
      }),
      row(SETTING_KEYS.cleanMode, "true"),
    ]);
    await settingsCatalog.load();

    expect(settingsCatalog.isOverridden("jobs.concurrency")).toBe(false);
    expect(settingsCatalog.isOverridden(SETTING_KEYS.cleanMode)).toBe(true);
    expect(settingsCatalog.isOverridden("ui.unknown")).toBe(false);
  });

  it("exposes what a layer contributes, so a caller can name the fallback", async () => {
    apiMock.mockResolvedValueOnce([
      row("jobs.concurrency", "2", {
        kind: "int",
        source: "stored",
        default_json: "0",
        env: { value_json: "8", origin: "ASTERISM_JOB_CONCURRENCY" },
      }),
    ]);
    await settingsCatalog.load();

    // The user's choice is in force, and the chain still records what
    // it is shadowing.
    expect(settingsCatalog.int("jobs.concurrency", 0)).toBe(2);
    expect(settingsCatalog.layerValue("jobs.concurrency", "env")).toBe("8");
    expect(settingsCatalog.layerValue("jobs.concurrency", "default")).toBe("0");
    expect(settingsCatalog.layerValue("jobs.concurrency", "stored")).toBe("2");
    expect(settingsCatalog.layerValue(SETTING_KEYS.cleanMode, "env")).toBe(null);
  });

  it("keeps a rejected layer visible without letting it win", async () => {
    // An exported-but-unusable variable is listed with its reason, and
    // the value in force comes from the layer below it.
    apiMock.mockResolvedValueOnce([
      row("jobs.concurrency", "0", {
        kind: "int",
        source: "default",
        default_json: "0",
        env: {
          value_json: "many",
          origin: "ASTERISM_JOB_CONCURRENCY",
          rejected: "setting value many is not valid JSON",
        },
      }),
    ]);
    await settingsCatalog.load();

    expect(settingsCatalog.int("jobs.concurrency", 7)).toBe(0);
    // The layer is present — a caller can render it — but it is not the
    // fallback anyone should be promised.
    expect(settingsCatalog.layerValue("jobs.concurrency", "env")).toBe("many");
    expect(settingsCatalog.isOverridden("jobs.concurrency")).toBe(false);
  });

  it("serialises the value and re-reads after a write", async () => {
    apiMock
      .mockResolvedValueOnce(row(SETTING_KEYS.cleanMode, "true")) // set_setting
      .mockResolvedValueOnce([row(SETTING_KEYS.cleanMode, "true")]); // list
    await settingsCatalog.set(SETTING_KEYS.cleanMode, true);

    expect(apiMock).toHaveBeenNthCalledWith(1, "set_setting", {
      command: { key: SETTING_KEYS.cleanMode, value_json: "true" },
    });
    expect(apiMock).toHaveBeenNthCalledWith(2, "list_settings");
    expect(settingsCatalog.bool(SETTING_KEYS.cleanMode, false)).toBe(true);
  });

  it("propagates a write rejection to the caller", async () => {
    apiMock.mockRejectedValueOnce(new Error("backend said no"));
    await expect(
      settingsCatalog.set(SETTING_KEYS.cleanMode, true),
    ).rejects.toThrow("backend said no");
    // The failed write must not have been followed by a re-read.
    expect(apiMock).toHaveBeenCalledTimes(1);
  });

  it("falls back to the caller's default when the load itself fails", async () => {
    apiMock.mockRejectedValueOnce(new Error("offline"));
    await settingsCatalog.load();
    expect(settingsCatalog.list.error).toContain("offline");
    expect(settingsCatalog.bool(SETTING_KEYS.importAutoOrganize, true)).toBe(
      true,
    );
  });

  it("re-reads after a reset and lands on the layer beneath", async () => {
    apiMock
      .mockResolvedValueOnce(row("jobs.concurrency", "8", { kind: "int" }))
      .mockResolvedValueOnce([
        // Clearing the user's choice hands the key back to the
        // environment, not straight to the built-in default.
        row("jobs.concurrency", "8", {
          kind: "int",
          source: "env",
          default_json: "0",
          env: { value_json: "8", origin: "ASTERISM_JOB_CONCURRENCY" },
        }),
      ]);
    await settingsCatalog.reset("jobs.concurrency");

    expect(apiMock).toHaveBeenNthCalledWith(1, "reset_setting", {
      command: { key: "jobs.concurrency" },
    });
    expect(settingsCatalog.int("jobs.concurrency", 0)).toBe(8);
    expect(settingsCatalog.isOverridden("jobs.concurrency")).toBe(false);
  });

  // vitest runs these in the node environment, where `localStorage`
  // does not exist — which is also why the catalog guards on
  // `typeof localStorage === "undefined"` before touching it. A minimal
  // in-memory stub is enough to exercise the carry.
  describe("legacy localStorage carry", () => {
    let store: Map<string, string>;

    beforeEach(() => {
      store = new Map();
      vi.stubGlobal("localStorage", {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => void store.set(k, v),
        removeItem: (k: string) => void store.delete(k),
      });
    });

    afterEach(() => {
      vi.unstubAllGlobals();
    });

    it("is a no-op when localStorage is unavailable", async () => {
      vi.unstubAllGlobals();
      apiMock.mockResolvedValueOnce([
        row(SETTING_KEYS.cleanMode, "false", { source: "default" }),
      ]);
      await settingsCatalog.load();
      await settingsCatalog.migrateLegacyLocalStorage();
      expect(apiMock).toHaveBeenCalledTimes(1);
    });

    it("carries a legacy value into a key still at its default", async () => {
      localStorage.setItem("asterism.import.auto_organize.v1", "0");
      apiMock.mockResolvedValueOnce([
        row(SETTING_KEYS.importAutoOrganize, "true", {
          source: "default",
          default_json: "true",
        }),
      ]);
      await settingsCatalog.load();

      apiMock
        .mockResolvedValueOnce(row(SETTING_KEYS.importAutoOrganize, "false"))
        .mockResolvedValueOnce([
          row(SETTING_KEYS.importAutoOrganize, "false"),
        ]);
      await settingsCatalog.migrateLegacyLocalStorage();

      expect(apiMock).toHaveBeenNthCalledWith(2, "set_setting", {
        command: {
          key: SETTING_KEYS.importAutoOrganize,
          value_json: "false",
        },
      });
      // Handled entries are dropped, so a re-run is a no-op.
      expect(localStorage.getItem("asterism.import.auto_organize.v1")).toBe(
        null,
      );
    });

    it("does not overwrite a key the backend already reports as set", async () => {
      localStorage.setItem("asterism.clean_mode.v1", "0");
      apiMock.mockResolvedValueOnce([
        row(SETTING_KEYS.cleanMode, "true", { source: "stored" }),
      ]);
      await settingsCatalog.load();
      await settingsCatalog.migrateLegacyLocalStorage();

      // Only the initial list call — no write.
      expect(apiMock).toHaveBeenCalledTimes(1);
      expect(settingsCatalog.bool(SETTING_KEYS.cleanMode, false)).toBe(true);
      expect(localStorage.getItem("asterism.clean_mode.v1")).toBe(null);
    });

    it("keeps the legacy entry when the carry write fails", async () => {
      localStorage.setItem("asterism.clean_mode.v1", "1");
      apiMock.mockResolvedValueOnce([
        row(SETTING_KEYS.cleanMode, "false", { source: "default" }),
      ]);
      await settingsCatalog.load();

      apiMock.mockRejectedValueOnce(new Error("offline"));
      await settingsCatalog.migrateLegacyLocalStorage();

      // Left in place so the next launch retries.
      expect(localStorage.getItem("asterism.clean_mode.v1")).toBe("1");
    });
  });
});
