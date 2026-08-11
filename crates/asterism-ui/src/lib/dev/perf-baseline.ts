// Baseline instrumentation used before the state-architecture
// migration. The helpers split each user-perceived reload into (a)
// backend invoke time and (b) frontend derived+render time so the
// before/after comparison rests on numbers instead of a subjective
// "feels faster" judgement.
//
// Everything no-ops outside `import.meta.env.DEV` so a production build
// is not affected. Records live in `window.__ASTERISM_PERF__` for easy
// copy-paste out of the console.

type Extra = Record<string, number | string | boolean | null | undefined>;

interface Sample {
  scope: string;
  startedAt: number;
  endedAt: number | null;
  totalMs: number | null;
  stamps: Array<{ label: string; ms: number; extra?: Extra }>;
  extra?: Extra;
}

interface Baseline {
  now(): number;
  begin(scope: string, extra?: Extra): Sample | null;
  stamp(sample: Sample | null, label: string, sinceT0: number, extra?: Extra): void;
  end(sample: Sample | null, extra?: Extra): void;
  measureToPaint(label: string, extra?: Extra): void;
  samples(): Sample[];
  clear(): void;
}

const DEV = import.meta.env.DEV;

const now = (): number =>
  typeof performance !== "undefined" ? performance.now() : Date.now();

function makeBaseline(): Baseline {
  if (!DEV) {
    return {
      now,
      begin: () => null,
      stamp: () => {},
      end: () => {},
      measureToPaint: () => {},
      samples: () => [],
      clear: () => {},
    };
  }

  const store: Sample[] = [];
  const w = window as unknown as { __ASTERISM_PERF__?: Sample[] };
  if (!w.__ASTERISM_PERF__) w.__ASTERISM_PERF__ = store;

  const begin: Baseline["begin"] = (scope, extra) => {
    const sample: Sample = {
      scope,
      startedAt: now(),
      endedAt: null,
      totalMs: null,
      stamps: [],
      extra,
    };
    store.push(sample);
    // Trim to a rolling window so long dev sessions do not leak.
    if (store.length > 200) store.splice(0, store.length - 200);
    return sample;
  };

  const stamp: Baseline["stamp"] = (sample, label, sinceT0, extra) => {
    if (!sample) return;
    sample.stamps.push({ label, ms: now() - sinceT0, extra });
  };

  const end: Baseline["end"] = (sample, extra) => {
    if (!sample) return;
    sample.endedAt = now();
    sample.totalMs = sample.endedAt - sample.startedAt;
    if (extra) sample.extra = { ...(sample.extra ?? {}), ...extra };
    const invokeMs = sample.stamps
      .filter((s) => s.label.startsWith("invoke:"))
      .reduce((acc, s) => acc + s.ms, 0);
    const frontendMs = sample.totalMs - invokeMs;
    // Single console line per reload so a session log can be
    // eyeballed and grep'd. Extra objects deliberately render inline.
    const line =
      `[perf] ${sample.scope} total=${sample.totalMs.toFixed(1)}ms invoke=${invokeMs.toFixed(1)}ms frontend=${frontendMs.toFixed(1)}ms ` +
      sample.stamps.map((s) => `${s.label}=${s.ms.toFixed(1)}ms`).join(" ");
    // eslint-disable-next-line no-console
    console.log(line, sample.extra ?? "");
    // Mirror into the Rust-side log (terminal) via tauri-plugin-log so
    // perf numbers are readable without opening webview devtools. Lazy
    // import + fire-and-forget: never blocks or throws into the
    // measured path.
    void import("@tauri-apps/plugin-log")
      .then((l) => l.info(line))
      .catch(() => {});
  };

  const measureToPaint: Baseline["measureToPaint"] = (label, extra) => {
    const t0 = now();
    // Two rAF ticks: the first fires before layout/paint, the second
    // fires after the browser has painted the frame produced by the
    // work between t0 and rAF #1. Good enough as a "when did the
    // pixels change" proxy for a keypress → reload → render loop.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const ms = now() - t0;
        // eslint-disable-next-line no-console
        console.log(`[perf] paint ${label}=${ms.toFixed(1)}ms`, extra ?? "");
      });
    });
  };

  return {
    now,
    begin,
    stamp,
    end,
    measureToPaint,
    samples: () => store.slice(),
    clear: () => {
      store.length = 0;
    },
  };
}

export const perfBaseline: Baseline = makeBaseline();
