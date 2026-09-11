<script lang="ts">
  // DateJump — the "Occurred on" sidebar section: hold the grid to a
  // day, a week or a month, or to one day of every year.
  //
  // A filter, not a destination. It narrows by the asset's resolved
  // time the way the chips narrow by persona and modality, so it
  // composes with all of them, the count line stays honest, and a
  // Query Group can carry it. What it is *not* is a scroll position or
  // a view of its own — the grid stays the grid, holding fewer cards.
  //
  // State (0-prop, catalog-driven):
  //   - activeFilter.dayFrom / .dayUntil / .dayOfYear
  //
  // This component never does date arithmetic: `activeFilter.jumpTo()`
  // turns a date and a span into the range, `jumpToDayOfYear()` turns
  // a date into the month-and-day, and `jumpDate()` / `jumpSpan()`
  // read the controls' state back out. A second conversion here would
  // be a second definition of which day a week starts on.
  //
  // The span picker can show nothing selected, and that is a real
  // state rather than a bug: a restored Query Group, or a rule written
  // through the MCP tool, may hold a range that is not a whole day,
  // week or month. The filter is still applied — the range is what
  // the grid is under — and the row says so instead of rounding the
  // rule into the nearest span it can draw.
  import { activeFilter, viewerTimeZone, type JumpSpan } from "./lib/stores/filter.svelte";

  const SPANS: { value: JumpSpan; label: string; title: string }[] = [
    { value: "day", label: "Day", title: "The calendar day containing the date" },
    { value: "week", label: "Week", title: "The Monday-to-Sunday week containing the date (ISO 8601)" },
    { value: "month", label: "Month", title: "The calendar month containing the date" },
  ];

  let date = $derived(activeFilter.jumpDate());
  let span = $derived(activeFilter.jumpSpan());
  let everyYear = $derived(activeFilter.dayOfYear !== null);

  // Committed on `change` (blur / Enter / picker dismiss) rather than
  // on input, matching the metric bands: a half-typed year sends the
  // grid to the first century and back.
  function onDate(e: Event) {
    const el = e.currentTarget as HTMLInputElement;
    if (el.value.trim() === "") {
      activeFilter.clearJump();
      return;
    }
    if (everyYear) {
      // The toggle is the cut in force; a new date moves which day of
      // the year, not which kind of question.
      activeFilter.jumpToDayOfYear(el.value);
      return;
    }
    // Keep the span the user already picked; a date change should move
    // the range, not resize it. With no span recognised — a restored
    // rule the picker cannot draw — a day is the honest default,
    // because the date box is what was just used.
    activeFilter.jumpTo(el.value, span ?? "day");
  }

  function pickSpan(next: JumpSpan) {
    // Nothing to resize until a date is set; the buttons are disabled
    // then, and this is the guard behind that.
    if (date === null) return;
    activeFilter.jumpTo(date, next);
  }

  // The range as a sentence, for the case the span buttons cannot
  // describe. The dates are shown as themselves: they are calendar
  // dates in the filter's zone, and formatting them through the
  // machine's own zone could move one by a day.
  function rangeText(): string {
    return `${activeFilter.dayFrom ?? "…"} → ${activeFilter.dayUntil ?? "…"}`;
  }

  // The zone the days are read in, shown only when it is not where the
  // viewer is — a restored rule carries its own, and a day under a zone
  // the viewer does not live in is worth a line on screen.
  let foreignZone = $derived(
    activeFilter.hasDayFilter() && activeFilter.dayTimeZone !== viewerTimeZone()
      ? activeFilter.dayTimeZone
      : null,
  );
</script>

<h2>Occurred on</h2>
<div class="jump">
  <input
    type="date"
    aria-label="Hold the grid to a date"
    value={date ?? ""}
    onchange={onDate}
  />
  <div class="jump-spans" role="group" aria-label="How far the range reaches">
    {#each SPANS as s (s.value)}
      <button
        type="button"
        class:active={span === s.value}
        disabled={date === null || everyYear}
        title={s.title}
        onclick={() => pickSpan(s.value)}
      >
        {s.label}
      </button>
    {/each}
  </div>
  <label class="jump-every-year" title="This month and day, in every year">
    <input
      type="checkbox"
      checked={everyYear}
      disabled={date === null}
      onchange={() => activeFilter.toggleEveryYear()}
    />
    same day, every year
  </label>
  {#if activeFilter.hasDayFilter()}
    {#if span === null && !everyYear}
      <!-- A range this picker did not draw and cannot round off. It
           is applied all the same, so it is shown as itself. -->
      <p class="jump-note">{rangeText()}</p>
    {/if}
    {#if foreignZone !== null}
      <p class="jump-note">read in {foreignZone}</p>
    {/if}
    <button type="button" class="jump-clear" onclick={() => activeFilter.clearJump()}>× clear</button>
  {/if}
</div>

<style>
  /* Mirrors the `.sidebar h2` cascade every sidebar section duplicates,
     because scoped styles do not reach across components. Kept in sync
     until the whole sidebar graduates out of App (wave 9). */
  h2 {
    font-size: 0.75rem;
    color: var(--ink-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 1rem 0 0.25rem;
  }

  .jump {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.1rem 0.3rem;
  }

  .jump input[type="date"] {
    width: 100%;
    min-width: 0;
    font-family: inherit;
    font-size: 0.8rem;
    color: var(--ink);
    padding: 0.1rem 0.25rem;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--surface-raised);
    font-variant-numeric: tabular-nums;
  }
  .jump input[type="date"]:focus {
    outline: none;
    border-color: var(--accent-line-strong);
  }

  /* Three segments across the sidebar's width. The metric bands wrap
     their label onto its own line for the same reason: 180px minus
     padding does not hold a label and three controls side by side. */
  .jump-spans {
    display: flex;
    gap: 0.2rem;
  }

  .jump-spans button {
    flex: 1 1 0;
    min-width: 0;
    padding: 0.1rem 0.2rem;
    font-family: inherit;
    font-size: 0.72rem;
    color: var(--ink-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--line);
    border-radius: 4px;
    cursor: pointer;
  }
  .jump-spans button:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  .jump-spans button.active {
    background: var(--accent-surface);
    border-color: var(--accent-line-strong);
    color: var(--accent-ink);
  }
  .jump-spans button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .jump-every-year {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.72rem;
    color: var(--ink-secondary);
    cursor: pointer;
  }
  .jump-every-year input {
    margin: 0;
  }
  .jump-every-year:has(input:disabled) {
    opacity: 0.45;
    cursor: default;
  }

  .jump-note {
    margin: 0;
    font-size: 0.68rem;
    line-height: 1.3;
    color: var(--ink-faint);
    font-variant-numeric: tabular-nums;
  }

  .jump-clear {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0.1rem 0.3rem;
    font-family: inherit;
    font-size: 0.8rem;
    color: var(--ink-secondary);
    cursor: pointer;
    border-radius: 4px;
  }
  .jump-clear:hover {
    background: var(--surface-hover);
  }
</style>
