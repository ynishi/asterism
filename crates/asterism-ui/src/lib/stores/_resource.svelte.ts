// Resource — async fetch state machine shared by all catalogs
// (State Hardening). Owns the four cross-cutting
// concerns that used to be hand-rolled (and drifted) per catalog:
//
//   - stale-response guard: every `load()` bumps `#generation`;
//     responses from a superseded call are dropped, so rapid
//     persona switching can never leave last-write-wins data.
//   - loading flag: true while the *latest* load is in flight.
//   - error normalization: one policy — `console.warn` with the
//     catalog-provided label, `data` falls back to the initial
//     value, and the message is exposed on `error` for any UI
//     that wants to surface it. No silent `catch {}` swallowing.
//   - reset: back to the initial value, invalidating in-flight
//     responses (used on persona clear / teardown).
//
// Catalogs place a Resource on a public field
// (`counts = new Resource(...)`) and connect domain deriveds via
// `$derived.by(() => ... this.counts.data ...)`. Consumers read
// `catalog.field.data` / `.loading` / `.error`.
//
// `load()` resolves to `true` iff this call's result was written
// (success + still the newest generation). Callers that keep
// sibling cache state (e.g. App's `sessionsFetchKey`) use the
// boolean to update it atomically with the accepted write.
//
// Multi-step fetchers (thumb cascades etc.) receive an `isStale`
// callback as their second argument: check it after each await to
// skip wasted follow-up requests and to avoid allocating leakable
// resources (blob URLs) for a response that will be dropped.
//
// Deliberately NOT owned here:
//   - domain deriveds (`nameById` etc.) — catalog-side.
//   - mutation calls (create / remove / rename) — those keep
//     throwing to their caller from catalog methods.
//   - reload orchestration — App-side `$effect`.

export class Resource<TArgs, T> {
  data = $state() as T;
  loading = $state(false);
  error = $state<string | null>(null);
  #generation = 0;

  constructor(
    private fetcher: (args: TArgs, isStale: () => boolean) => Promise<T>,
    private initial: T,
    private label: string, // console.warn prefix, e.g. "tagCatalog.counts"
  ) {
    this.data = initial;
  }

  async load(args: TArgs): Promise<boolean> {
    const gen = ++this.#generation;
    this.loading = true;
    try {
      const result = await this.fetcher(args, () => gen !== this.#generation);
      if (gen !== this.#generation) return false; // stale response, drop
      this.data = result;
      this.error = null;
      return true;
    } catch (e) {
      if (gen !== this.#generation) return false;
      this.error = String(e);
      this.data = this.initial;
      console.warn(`[${this.label}] load failed:`, e);
      return false;
    } finally {
      if (gen === this.#generation) this.loading = false;
    }
  }

  reset(): void {
    this.#generation++; // invalidate in-flight responses
    this.data = this.initial;
    this.loading = false;
    this.error = null;
  }
}
