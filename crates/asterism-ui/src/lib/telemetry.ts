// telemetry — fire-and-forget local event recording (dogfooding
// metrics, backed by the `event_log` table via `record_event`).
//
// Contract: recording must never affect the interaction being
// measured. Every failure path is swallowed — a lost event is
// strictly better than a blocked or noisy UI. Works in DEV and
// release alike (unlike `dev/perf-baseline.ts`, which is the
// DEV-only console view of the same moments).
//
// Kinds are open slugs owned by the call sites: `app_open`,
// `persona_switch`, `search`, `burst_open`, `asset_open`. Payloads
// stay small (a few scalar facts), serialised here so call sites
// pass plain objects.
import { api } from "./api";

export interface EventOpts {
  /** Persona in scope when the event fired (null = all-personas). */
  personaId?: string | null;
  /** User-perceived duration of the measured interaction. */
  durationMs?: number;
  /** Small extension bag — serialised to `payload_json`. */
  payload?: Record<string, unknown>;
}

export function recordEvent(kind: string, opts: EventOpts = {}): void {
  try {
    void api("record_event", {
      command: {
        kind,
        persona_id: opts.personaId ?? null,
        duration_ms:
          opts.durationMs === undefined ? null : Math.round(opts.durationMs),
        payload_json:
          opts.payload === undefined ? null : JSON.stringify(opts.payload),
      },
    }).catch(() => {});
  } catch {
    // `api` itself throwing (no Tauri context, e.g. vitest) is a
    // no-op by design.
  }
}
