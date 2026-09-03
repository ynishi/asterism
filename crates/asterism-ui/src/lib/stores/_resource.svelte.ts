// Resource — async fetch state machine shared by all catalogs
// (State Hardening). Owns the cross-cutting concerns that used to be
// hand-rolled (and drifted) per catalog:
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
//   - answered flag: whether any load has landed since construction
//     or the last reset — success or failure. `data` alone cannot
//     say it: an initial value and an empty answer read the same,
//     and a screen that must not act until a read has answered
//     (a picker offering to *open* work because the list of work
//     is empty, #219) needs the difference.
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
  /// Whether a load has landed since construction or the last reset.
  /// Set on success and on failure alike — both are answers — and
  /// only by the newest generation, so a dropped stale response does
  /// not claim to have answered for the load that superseded it.
  ///
  /// A fact about this resource's lifetime, not about any subject:
  /// `load` keeps the last answer and this flag until the next lands,
  /// which is right for the same subject read again and wrong for a
  /// subject that changed. A catalog whose reads are about something
  /// that can change — a team's lines, a line's work — resets the
  /// resource wherever it changes the subject, before it reads, so
  /// that "answered" means answered for what is on now. `shared`'s
  /// `lookAt` and `show` are that rule; a consumer drawing a claim
  /// from an empty or a missing answer relies on it.
  answered = $state(false);
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
      this.answered = true;
      return true;
    } catch (e) {
      if (gen !== this.#generation) return false;
      this.error = String(e);
      this.data = this.initial;
      this.answered = true;
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
    this.answered = false;
  }
}
