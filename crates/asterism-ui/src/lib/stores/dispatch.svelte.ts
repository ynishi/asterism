// Dispatch catalog — dispatch monitoring + jobs-pipeline
// observability. Two loosely
// related signals fuse into one store because they share the
// "background work is happening" UX corner (bottom toast +
// header ticker banner) and callers touch both from the same
// action-bar mutation flows.
//
// Scope:
//   - `pendingId: string | null` — dispatch id being polled.
//     Action-bar buttons disable while non-null so the user
//     can't fire another dispatch on top of the pending one.
//   - `status: string | null` — toast message string. `null` =
//     no toast rendering. `flash(msg, ms)` sets + auto-clears
//     after `ms`; `set(msg)` sets without an auto-clear (used
//     by `pollDispatch` which drives its own lifecycle).
//   - `jobsSnapshot: JobsSnapshotDto | null` — the DB-level `Jobs`
//     table snapshot from `jobs_stats` (wire type from
//     `bindings.ts` — no local re-definition). `null`
//     before the
//     first fetch. The poll loop lives on the caller side
//     (App owns the 3-s `$effect` cadence) and pipes results
//     in through `refreshJobsSnapshot()` so the store stays
//     lifecycle-agnostic.
//   - `activeKindGauges: KindGauge[]` — $derived: only kinds
//     with non-drained buckets (pending / running / failed >
//     0). Sorted most-active first (pending + running desc).
//   - `pollDispatch(id)` — the 60-iteration `get_dispatch`
//     poll (1.5 s interval, ~90 s ceiling). Updates `status`
//     as it goes, clears `pendingId` + fades `status` at the
//     end. Callers `beginDispatch(id, initial)` then `void
//     pollDispatch(id)` — the App-side `saveCurrent{Selection,
//     Query}` / `dispatchSelection` / `promoteSelectionToGroup`
//     wrappers follow this pattern.
//
// Deliberately NOT owned here:
//   - The 3-s `$effect` that drives `refreshJobsSnapshot()` on
//     mount + on cadence. Effects are component lifecycle
//     surface; the store stays DOM-agnostic. App mounts the
//     effect once, the store owns the state it writes into.
//   - The per-kind "jobs:tick" event listener (previously
//     `noteJobTick` + `jobTickerBanner` $derived at App scope):
//     the derived was never consumed by any template, so wave
//     C drops the listener + accumulator entirely as dead
//     code. The DB-poll (`activeKindGauges`) is the only live
//     source of the header ticker.

import type {
  DispatchDto,
  JobKindSnapshotDto,
  JobsSnapshotDto,
} from "../../bindings";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

export type KindGauge = JobKindSnapshotDto & { kind: string };

/// Wire arg shape of `list_dispatch` — the four optional filters the
/// backend accepts plus a hard `limit`. `state` is a lifecycle-slug
/// filter (`"pending" | "running" | "done" | "failed" | "cancelled"`);
/// `null` means "every state".
export interface DispatchListArgs {
  personaId: string | null;
  snapshotId: string | null;
  stateSlug: string | null;
  limit: number;
}

class DispatchCatalog {
  pendingId = $state<string | null>(null);
  status = $state<string | null>(null);
  jobsSnapshot = $state<JobsSnapshotDto | null>(null);

  // Dispatch-history drawer state (W6). Opened from the
  // bottom status chip, backed by the `list_dispatch` Resource. The
  // filter defaults to "every state" so the first paint shows the
  // newest jobs regardless of lifecycle.
  historyOpen = $state(false);
  historyStateFilter = $state<string | null>(null);
  history = new Resource<DispatchListArgs, DispatchDto[]>(
    async (args) =>
      api<DispatchDto[]>("list_dispatch", {
        personaId: args.personaId,
        snapshotId: args.snapshotId,
        stateSlug: args.stateSlug,
        limit: args.limit,
      }),
    [] as DispatchDto[],
    "dispatchCatalog.history",
  );

  // Snapshot-view open state (SnapshotView shared component).
  // A string id is the freeze under inspection; `null` = closed. Set
  // from a history-panel row click or from a Group detail's
  // "promoted from" chip.
  snapshotOpenId = $state<string | null>(null);

  openHistory(): void {
    this.historyOpen = true;
  }
  closeHistory(): void {
    this.historyOpen = false;
  }
  openSnapshot(id: string): void {
    this.snapshotOpenId = id;
  }
  closeSnapshot(): void {
    this.snapshotOpenId = null;
  }

  activeKindGauges = $derived.by<KindGauge[]>(() => {
    const snap = this.jobsSnapshot;
    if (!snap) return [];
    const out: KindGauge[] = [];
    for (const [kind, k] of Object.entries(snap.by_kind)) {
      // Skip a kind entirely once its queue is fully drained so
      // the banner doesn't linger on completed work.
      if (k.pending + k.running + k.failed === 0) continue;
      out.push({ kind, ...k });
    }
    // Most-active first (pending + running desc) so the eye
    // lands on the heaviest kind straight away.
    out.sort(
      (a, b) => b.pending + b.running - (a.pending + a.running),
    );
    return out;
  });

  // Set the status message and clear it after `ms`. The
  // caller-side pattern before wave C was `dispatchStatus = ...;
  // window.setTimeout(() => dispatchStatus = null, 5000)`
  // repeated at every mutation site; consolidating it here cuts
  // the boilerplate and centralises the fade duration.
  flash(message: string, ms = 5000): void {
    this.status = message;
    window.setTimeout(() => {
      // Guard against a later flash overwriting the timer: only
      // clear if we're still showing THIS message.
      if (this.status === message) this.status = null;
    }, ms);
  }

  // Set the status without an auto-clear. `pollDispatch` uses
  // this because the poll body updates `status` on every tick;
  // the terminal state arms its own fade via `flash`.
  set(message: string | null): void {
    this.status = message;
  }

  beginDispatch(id: string, message: string): void {
    this.pendingId = id;
    this.set(message);
  }

  async refreshJobsSnapshot(): Promise<void> {
    try {
      this.jobsSnapshot = await api<JobsSnapshotDto>("jobs_stats");
    } catch (err) {
      console.warn("jobs_stats failed", err);
    }
  }

  async pollDispatch(id: string): Promise<void> {
    for (let i = 0; i < 60; i++) {
      await new Promise((r) => window.setTimeout(r, 1500));
      try {
        const dto = await api<DispatchDto>("get_dispatch", { id });
        if (dto.state === "done") {
          this.set(`Done · produced ${dto.output_asset_ids.length} asset(s)`);
          break;
        }
        if (dto.state === "failed" || dto.state === "cancelled") {
          this.set(`${dto.state}: ${dto.state_message ?? "no detail"}`);
          break;
        }
        this.set(
          `${dto.state}${dto.state_message ? " · " + dto.state_message : ""}`,
        );
      } catch (err) {
        this.set(`Poll failed: ${String(err)}`);
        break;
      }
    }
    this.pendingId = null;
    // Fade the terminal status after a moment so the corner
    // doesn't stay pinned.
    const terminal = this.status;
    window.setTimeout(() => {
      if (this.status === terminal) this.status = null;
    }, 6000);
  }
}

export const dispatchCatalog = new DispatchCatalog();
