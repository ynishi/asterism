// The calendar filter — "what is from these days" and "what happened on
// this day, any year" as predicates over the asset's resolved time,
// sitting beside persona and modality rather than replacing the grid
// with a view of its own.
//
// Expectations are calendar dates, not instants: the store holds the
// wire's own `YYYY-MM-DD` form and the backend opens each day in the
// zone the query names, so nothing here depends on the machine's zone.
import { beforeEach, describe, expect, it } from "vitest";
import { activeFilter, viewerTimeZone } from "./filter.svelte";

describe("jumpTo", () => {
  beforeEach(() => activeFilter.reset());

  it("sends a day to that day alone, half-open", () => {
    activeFilter.jumpTo("2026-03-14", "day");
    expect(activeFilter.dayFrom).toBe("2026-03-14");
    expect(activeFilter.dayUntil).toBe("2026-03-15");
  });

  it("opens a week on the Monday containing the date", () => {
    // 2026-03-14 is a Saturday, so the week it belongs to opened on the
    // 9th. Monday rather than Sunday is ISO 8601, and the point of
    // fixing it is that a saved rule must select the same assets on
    // every machine — a locale-dependent first day would not.
    expect(new Date(Date.UTC(2026, 2, 14)).getUTCDay()).toBe(6);
    activeFilter.jumpTo("2026-03-14", "week");
    expect(activeFilter.dayFrom).toBe("2026-03-09");
    expect(activeFilter.dayUntil).toBe("2026-03-16");
  });

  it("opens a week that already starts on Monday on that same day", () => {
    // The boundary case the modulo is for: a Monday must not be pushed
    // back into the previous week.
    activeFilter.jumpTo("2026-03-09", "week");
    expect(activeFilter.dayFrom).toBe("2026-03-09");
  });

  it("opens a week containing a Sunday without rolling past it", () => {
    // `getUTCDay()` is 0 on Sunday, which is the value that would shift
    // a week forward rather than back without the `+6 % 7`.
    expect(new Date(Date.UTC(2026, 2, 15)).getUTCDay()).toBe(0);
    activeFilter.jumpTo("2026-03-15", "week");
    expect(activeFilter.dayFrom).toBe("2026-03-09");
    expect(activeFilter.dayUntil).toBe("2026-03-16");
  });

  it("opens a month on the first and closes on the next first", () => {
    // February, so a month of 28 days is not assumed to be 30 or 31.
    activeFilter.jumpTo("2026-02-14", "month");
    expect(activeFilter.dayFrom).toBe("2026-02-01");
    expect(activeFilter.dayUntil).toBe("2026-03-01");
  });

  it("carries a month across a year boundary", () => {
    activeFilter.jumpTo("2026-12-20", "month");
    expect(activeFilter.dayFrom).toBe("2026-12-01");
    expect(activeFilter.dayUntil).toBe("2027-01-01");
  });

  it("carries a week across a year boundary", () => {
    // 2026-12-31 is a Thursday; its week runs into 2027.
    activeFilter.jumpTo("2026-12-31", "week");
    expect(activeFilter.dayFrom).toBe("2026-12-28");
    expect(activeFilter.dayUntil).toBe("2027-01-04");
  });

  it("leaves the filter alone for a date the calendar does not have", () => {
    // `Date.UTC(2026, 1, 30)` is 2 March, silently. Guessing at the
    // nearby date would send the grid somewhere nobody asked for, so a
    // malformed date changes nothing.
    activeFilter.jumpTo("2026-03-14", "day");
    activeFilter.jumpTo("2026-02-30", "day");
    expect(activeFilter.dayFrom).toBe("2026-03-14");
    activeFilter.jumpTo("not-a-date", "day");
    expect(activeFilter.dayFrom).toBe("2026-03-14");
  });

  it("replaces a day-of-year cut rather than sitting beside it", () => {
    // The wire refuses both cuts at once; the store never produces
    // that state.
    activeFilter.jumpToDayOfYear("2026-03-14");
    activeFilter.jumpTo("2026-03-14", "day");
    expect(activeFilter.dayOfYear).toBeNull();
    expect(activeFilter.dayFrom).toBe("2026-03-14");
  });
});

describe("jumpToDayOfYear / toggleEveryYear", () => {
  beforeEach(() => activeFilter.reset());

  it("keeps the month and day and drops the year", () => {
    activeFilter.jumpToDayOfYear("2019-03-14");
    expect(activeFilter.dayOfYear).toEqual({ month: 3, day: 14 });
    expect(activeFilter.dayFrom).toBeNull();
    expect(activeFilter.dayUntil).toBeNull();
  });

  it("accepts 29 February, which the calendar has", () => {
    // Refused per year by the backend, not here: some years have it.
    activeFilter.jumpToDayOfYear("2024-02-29");
    expect(activeFilter.dayOfYear).toEqual({ month: 2, day: 29 });
  });

  it("refuses a date the calendar does not have", () => {
    activeFilter.jumpToDayOfYear("2026-02-30");
    expect(activeFilter.dayOfYear).toBeNull();
  });

  it("flips a range to its first day, every year, and back", () => {
    activeFilter.jumpTo("2026-03-14", "week");
    activeFilter.toggleEveryYear(2026);
    expect(activeFilter.dayOfYear).toEqual({ month: 3, day: 9 });
    expect(activeFilter.dayFrom).toBeNull();
    activeFilter.toggleEveryYear(2026);
    expect(activeFilter.dayOfYear).toBeNull();
    expect(activeFilter.dayFrom).toBe("2026-03-09");
    expect(activeFilter.dayUntil).toBe("2026-03-10");
  });

  it("clears rather than lands on 1 March when the year lacks the day", () => {
    activeFilter.jumpToDayOfYear("2024-02-29");
    activeFilter.toggleEveryYear(2026);
    expect(activeFilter.hasDayFilter()).toBe(false);
  });

  it("does nothing with no date to flip", () => {
    activeFilter.toggleEveryYear(2026);
    expect(activeFilter.hasDayFilter()).toBe(false);
  });
});

describe("jumpDate / jumpSpan — reading the controls back out", () => {
  beforeEach(() => activeFilter.reset());

  it("round-trips each span", () => {
    for (const span of ["day", "week", "month"] as const) {
      activeFilter.jumpTo("2026-03-14", span);
      expect(activeFilter.jumpSpan()).toBe(span);
    }
  });

  it("reports the date the range opens on, not the one that was typed", () => {
    // A week picked from Saturday the 14th opens on Monday the 9th, and
    // that is what the picker shows: the date box and the span together
    // have to describe the range actually in force.
    activeFilter.jumpTo("2026-03-14", "week");
    expect(activeFilter.jumpDate()).toBe("2026-03-09");
  });

  it("places a day-of-year in the year asked for", () => {
    activeFilter.jumpToDayOfYear("2019-03-14");
    expect(activeFilter.jumpDate(2026)).toBe("2026-03-14");
    expect(activeFilter.jumpSpan()).toBeNull();
  });

  it("shows no date for a day-of-year the year lacks", () => {
    activeFilter.jumpToDayOfYear("2024-02-29");
    expect(activeFilter.jumpDate(2026)).toBeNull();
    expect(activeFilter.jumpDate(2028)).toBe("2028-02-29");
  });

  it("says a range it did not draw is no span, and still applies it", () => {
    // A rule written through the MCP tool, or by hand, can name any two
    // dates. Rounding it to the nearest day on the way in would make a
    // restored Query Group select a different set than the group holds,
    // so the pair stands and the picker shows no span.
    activeFilter.dayFrom = "2026-03-01";
    activeFilter.dayUntil = "2026-03-10";
    expect(activeFilter.jumpSpan()).toBeNull();
    expect(activeFilter.hasDayFilter()).toBe(true);
  });

  it("says no span for a half-open range", () => {
    activeFilter.dayFrom = "2026-03-14";
    activeFilter.dayUntil = null;
    expect(activeFilter.jumpSpan()).toBeNull();
    expect(activeFilter.hasDayFilter()).toBe(true);
  });
});

describe("composition and clearing", () => {
  beforeEach(() => activeFilter.reset());

  it("is absent until asked for, so no client is silently truncated", () => {
    expect(activeFilter.hasDayFilter()).toBe(false);
    expect(activeFilter.dayFilter()).toEqual({
      day_from: null,
      day_until: null,
      day_of_year: null,
      time_zone: null,
    });
  });

  it("hands the query builders the wire fields with the viewer's zone", () => {
    // The zone travels only beside a day: the backend refuses a day
    // without one, and a spread that carried the days without it
    // would be a query builder that could forget.
    activeFilter.jumpTo("2026-03-14", "day");
    expect(activeFilter.dayFilter()).toEqual({
      day_from: "2026-03-14",
      day_until: "2026-03-15",
      day_of_year: null,
      time_zone: viewerTimeZone(),
    });
    activeFilter.jumpToDayOfYear("2026-03-14");
    expect(activeFilter.dayFilter()).toEqual({
      day_from: null,
      day_until: null,
      day_of_year: { month: 3, day: 14 },
      time_zone: viewerTimeZone(),
    });
  });

  it("names a real zone for the viewer", () => {
    // What the platform resolves, or UTC where it resolves nothing —
    // never an empty string, which the backend would refuse with
    // nothing on screen to say why.
    expect(viewerTimeZone().length).toBeGreaterThan(0);
  });

  it("leaves every other axis alone", () => {
    // The whole claim of the feature: a date narrows beside the chips
    // rather than instead of them.
    activeFilter.activePersona = "p1";
    activeFilter.activeModality = "dialogue";
    activeFilter.addTag({ id: "t1", name: "rooftop" });
    activeFilter.jumpTo("2026-03-14", "week");
    expect(activeFilter.activePersona).toBe("p1");
    expect(activeFilter.activeModality).toBe("dialogue");
    expect(activeFilter.activeTagIds.has("t1")).toBe(true);
    expect(activeFilter.hasDayFilter()).toBe(true);
  });

  it("is cleared by reset, like the chips it sits beside", () => {
    activeFilter.jumpTo("2026-03-14", "week");
    activeFilter.reset();
    expect(activeFilter.hasDayFilter()).toBe(false);
  });

  it("is cleared on its own by clearJump, zone included", () => {
    activeFilter.activePersona = "p1";
    activeFilter.jumpTo("2026-03-14", "week");
    activeFilter.dayTimeZone = "Asia/Tokyo";
    activeFilter.clearJump();
    expect(activeFilter.hasDayFilter()).toBe(false);
    expect(activeFilter.dayTimeZone).toBe(viewerTimeZone());
    expect(activeFilter.activePersona).toBe("p1");
  });
});

describe("restoreQueryGroup — the calendar filter is part of a saved rule", () => {
  beforeEach(() => {
    activeFilter.reset();
    activeFilter.searchText = "";
    activeFilter.searchFuzzy = true;
  });

  function rule(filter: Record<string, unknown>): string {
    return JSON.stringify({
      v: 1,
      filter: { persona_id: null, modality: null, tag_ids: [], group_ids: [], ...filter },
      sort: { target: "occurred_at", order: "updated", reverse: false },
      search_text: null,
    });
  }

  it("restores the range and the zone a rule carries", () => {
    expect(
      activeFilter.restoreQueryGroup(
        rule({ day_from: "2026-03-09", day_until: "2026-03-16", time_zone: "Asia/Tokyo" }),
      ),
    ).toBe(true);
    expect(activeFilter.dayFrom).toBe("2026-03-09");
    expect(activeFilter.jumpSpan()).toBe("week");
    // The rule's zone, not the viewer's: the group holds the set those
    // days select in Tokyo, and the restored filter shows that set.
    expect(activeFilter.dayTimeZone).toBe("Asia/Tokyo");
    expect(activeFilter.dayFilter().time_zone).toBe("Asia/Tokyo");
  });

  it("restores a day-of-year", () => {
    activeFilter.restoreQueryGroup(
      rule({ day_of_year: { month: 3, day: 14 }, time_zone: "UTC" }),
    );
    expect(activeFilter.dayOfYear).toEqual({ month: 3, day: 14 });
    expect(activeFilter.dayFrom).toBeNull();
  });

  it("restores a rule that predates the fields as no cut", () => {
    // Absent means the rule was frozen over the whole calendar, which
    // is the set it was saved as. Reading it as anything else would
    // change what an existing group holds.
    activeFilter.jumpTo("2026-03-14", "day");
    activeFilter.dayTimeZone = "Asia/Tokyo";
    expect(activeFilter.restoreQueryGroup(rule({}))).toBe(true);
    expect(activeFilter.hasDayFilter()).toBe(false);
    expect(activeFilter.dayTimeZone).toBe(viewerTimeZone());
  });

  it("restores a range the picker cannot draw without rounding it", () => {
    activeFilter.restoreQueryGroup(rule({ day_from: "2026-03-01", day_until: "2026-03-10" }));
    expect(activeFilter.dayFrom).toBe("2026-03-01");
    expect(activeFilter.dayUntil).toBe("2026-03-10");
    expect(activeFilter.jumpSpan()).toBeNull();
  });

  it("reads days with no zone in the viewer's zone", () => {
    // A hand-written rule can omit the zone; the viewer's is the honest
    // reading of a day nobody placed, and the backend gets a zone
    // either way.
    activeFilter.restoreQueryGroup(rule({ day_from: "2026-03-01", day_until: "2026-03-02" }));
    expect(activeFilter.dayFilter().time_zone).toBe(viewerTimeZone());
  });

  it("drops a day-of-year that is not a pair of numbers", () => {
    activeFilter.restoreQueryGroup(rule({ day_of_year: { month: "3", day: 14 } }));
    expect(activeFilter.dayOfYear).toBeNull();
  });
});
