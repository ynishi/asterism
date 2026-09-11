//! Which stamp is an asset's time, and which zone reads it — resolved
//! once, in one place, so nothing downstream branches on either.
//!
//! An asset row carries two instants and neither is "the time" on its
//! own. `occurred_at` is when the thing it records happened, and it is
//! the right answer for a photograph or a message. It is the wrong
//! answer for a row whose importer had no occurrence to record and
//! wrote the import moment in its place — a generated image, a
//! transcript export, a database dump — where the moment the row
//! *arrived* (`created_at`) is the only time it has. Which of the two
//! a row means is not a guess made at read time: the importer that
//! wrote the row knew, and [`OccurredSource`] is where it says so.
//!
//! The instant is then a point on the UTC line, and a calendar day is a
//! zone's reading of that line. Two layers supply the zone, on the
//! shape [`app_setting`](crate::domain::app_setting) uses for a
//! setting's value:
//!
//! - **Local** — a zone the asset itself carries
//!   ([`AssetTime::time_zone`]), when a supplier recorded where the
//!   thing happened. The higher layer: a row that knows its own zone is
//!   not re-read in somebody else's.
//! - **Global** — the zone the viewer is asking from
//!   ([`GlobalZone`]), carried on the query the way a collation is
//!   (`SortSpec::collation`) rather than held as process state. The
//!   fallback, and the only zone most rows have.
//!
//! [`resolve`] applies both rules and hands back one [`ResolvedTime`]:
//! the stamp, the instant it names, the zone, and which layer supplied
//! the zone. A consumer reads the result and never the inputs — the
//! service that maps a query, the repository that writes a derived
//! column, and the mapper that puts the answer on the wire all call
//! the same two lines instead of each carrying a copy of the rule.
//!
//! [`day_window`] and [`local_date`] are the two directions between an
//! instant and a calendar day under a zone. A day is **the 24 hours
//! from that day's first instant** — its local midnight, or the end of
//! the gap where a zone's rule skipped that midnight — and where that
//! falls is the zone's rule for that year: a daylight-saving transition
//! moves it without a line of code here noticing. What is deliberately
//! not handled, and stated once so nobody looks for it: a day a zone
//! repeats or shortens is still 24 hours from its start, and a date a
//! zone skipped whole at the date line is a date its calendar does not
//! have; nothing here corrects for either.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

use crate::error::DomainError;

/// Which of an asset's two instants is its time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeStamp {
    /// `occurred_at` — when the recorded thing happened.
    Occurred,
    /// `created_at` — when the row entered the library, which is the
    /// only time a row has when its importer had no occurrence to
    /// record.
    Added,
}

/// Where a row's `occurred_at` came from — written by the importer that
/// produced the row, and the fact [`resolve`] reads to choose a
/// [`TimeStamp`].
///
/// Closed, because the one decision it drives has two outcomes and a
/// slug outside the set would have to be guessed into one of them.
/// [`Unknown`](Self::Unknown) is the value a row holds when nothing
/// recorded a source — every row written before the column existed,
/// and any writer that has not been taught to say — and it resolves to
/// [`TimeStamp::Occurred`], which is what every reader assumed before
/// the source was recorded at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OccurredSource {
    /// A capture time read out of the artefact's own metadata.
    Exif,
    /// The file's modification time, taken because the artefact
    /// carried nothing better.
    Mtime,
    /// A timestamp the source record itself states — a message's
    /// `timestamp`, a row's own column.
    Record,
    /// The importer had no occurrence and wrote the import moment.
    /// The one value that turns the stamp to [`TimeStamp::Added`].
    Import,
    /// Nobody said.
    #[default]
    Unknown,
}

impl OccurredSource {
    /// Every value, in the order the slugs are listed to a caller.
    pub const ALL: [OccurredSource; 5] = [
        OccurredSource::Exif,
        OccurredSource::Mtime,
        OccurredSource::Record,
        OccurredSource::Import,
        OccurredSource::Unknown,
    ];

    /// Stable slug — the stored column value and the wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exif => "exif",
            Self::Mtime => "mtime",
            Self::Record => "record",
            Self::Import => "import",
            Self::Unknown => "unknown",
        }
    }

    /// Resolves a stored or caller-supplied slug. A slug outside the
    /// set is refused rather than read as [`Unknown`](Self::Unknown):
    /// the difference between "nobody said" and "somebody said
    /// something this code cannot read" is the difference a reader of
    /// the column needs.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == raw)
            .ok_or_else(|| {
                let accepted: Vec<&str> = Self::ALL.iter().map(|s| s.as_str()).collect();
                DomainError::Validation(format!(
                    "unknown occurred_source {raw:?}; expected one of {}",
                    accepted.join(", ")
                ))
            })
    }

    /// The stamp this source makes the row's time.
    pub const fn stamp(self) -> TimeStamp {
        match self {
            Self::Import => TimeStamp::Added,
            Self::Exif | Self::Mtime | Self::Record | Self::Unknown => TimeStamp::Occurred,
        }
    }
}

/// Which layer supplied the zone a [`ResolvedTime`] is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneSource {
    /// The asset's own zone.
    Local,
    /// The viewer's zone, because the asset carries none.
    Global,
}

impl ZoneSource {
    /// Stable slug used on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Global => "global",
        }
    }
}

/// The viewer's zone — the global layer, carried on the query.
///
/// A newtype rather than a bare `Tz` so the two zones [`resolve`]
/// takes cannot be swapped at the call site: one is a field of the
/// asset and the other is this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalZone(pub Tz);

/// The four facts on an asset row that its time is resolved from. Each
/// corresponds to one column, and nothing here is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetTime {
    /// When the recorded thing happened, as the importer wrote it.
    pub occurred_at: DateTime<Utc>,
    /// When the row entered the library.
    pub created_at: DateTime<Utc>,
    /// Where `occurred_at` came from.
    pub occurred_source: OccurredSource,
    /// The zone the thing happened in, when a supplier recorded one.
    pub time_zone: Option<Tz>,
}

impl AssetTime {
    /// Which instant is this asset's time — decided by the source
    /// alone, so the answer is the same in every zone.
    pub const fn stamp(&self) -> TimeStamp {
        self.occurred_source.stamp()
    }

    /// The instant [`stamp`](Self::stamp) names.
    pub const fn instant(&self) -> DateTime<Utc> {
        match self.stamp() {
            TimeStamp::Occurred => self.occurred_at,
            TimeStamp::Added => self.created_at,
        }
    }
}

/// One asset's time, resolved: what a consumer acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTime {
    /// Which instant was chosen.
    pub stamp: TimeStamp,
    /// The chosen instant.
    pub instant: DateTime<Utc>,
    /// The zone it is read in.
    pub zone: Tz,
    /// Which layer supplied [`zone`](Self::zone).
    pub zone_source: ZoneSource,
}

/// Resolves an asset's time against the viewer's zone.
///
/// Two rules and nothing else: the source picks the stamp, and the
/// asset's own zone outranks the viewer's. Every consumer reads the
/// result; none re-derives it.
pub fn resolve(asset: &AssetTime, global: GlobalZone) -> ResolvedTime {
    let (zone, zone_source) = match asset.time_zone {
        Some(local) => (local, ZoneSource::Local),
        None => (global.0, ZoneSource::Global),
    };
    ResolvedTime {
        stamp: asset.stamp(),
        instant: asset.instant(),
        zone,
        zone_source,
    }
}

/// One calendar day as a half-open UTC window, in the unit the
/// repository compares instants in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayWindow {
    /// The day's local midnight, inclusive (unix epoch ms).
    pub from_ms: i64,
    /// Twenty-four hours later, exclusive (unix epoch ms).
    pub until_ms: i64,
}

/// Milliseconds in the day [`day_window`] opens.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The 24 hours from the first instant of `(year, month, day)` in
/// `zone`, as UTC milliseconds.
///
/// The first instant is the day's local midnight, except where the
/// zone's rule skipped that midnight — Havana moves to daylight time
/// at 00:00, so its 9 March 2025 begins at 01:00 — in which case it is
/// the first wall-clock second the zone does have that day. Either
/// way the window is `+ 24h` from there rather than "to the next local
/// midnight", so a day a transition shortens or lengthens is still
/// answered as 24 hours; that is the rule, and the tests on the two
/// kinds of transition day state what they measure.
///
/// `None` means one thing: the calendar has no such date — 29 February
/// in a common year, 31 April, month 13. A date a zone skipped whole
/// (a date-line move) answers the same way, because in that zone the
/// calendar does not have it either; the module doc says that case is
/// not corrected for beyond this.
pub fn day_window(zone: Tz, year: i32, month: u32, day: u32) -> Option<DayWindow> {
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let first_instant = first_second_of(zone, date)?;
    let from_ms = first_instant.timestamp_millis();
    Some(DayWindow {
        from_ms,
        until_ms: from_ms + DAY_MS,
    })
}

/// The earliest wall-clock second of `date` that `zone` resolves.
///
/// Midnight, nearly always. When the rule skipped it, the seconds the
/// zone lacks form one run from 00:00 up to the gap's end (a
/// transition is a jump forward, so nothing before the end resolves
/// and everything from it on does), which is what lets a binary search
/// over the day's seconds find the end in seventeen probes rather than
/// a minute-by-minute walk. Seconds and not minutes because the tz
/// database records some old transitions at second precision.
///
/// `None` only when no second of the day resolves — the whole date is
/// absent from the zone's calendar.
fn first_second_of(zone: Tz, date: NaiveDate) -> Option<DateTime<Tz>> {
    let resolves = |second: u32| {
        zone.from_local_datetime(&date.and_hms_opt(second / 3600, second / 60 % 60, second % 60)?)
            .earliest()
    };
    if let Some(midnight) = resolves(0) {
        return Some(midnight);
    }
    // First resolving second in `1..SECONDS_IN_DAY`, if any.
    let (mut low, mut high) = (1u32, SECONDS_IN_DAY);
    let mut found = None;
    while low < high {
        let mid = low + (high - low) / 2;
        match resolves(mid) {
            Some(at) => {
                found = Some(at);
                high = mid;
            }
            None => low = mid + 1,
        }
    }
    found
}

/// Seconds in the day [`first_second_of`] searches.
const SECONDS_IN_DAY: u32 = 24 * 60 * 60;

/// The calendar day `instant` falls on in `zone` — the inverse of
/// [`day_window`], and the one function that writes the derived
/// `occurred_local_date` column.
pub fn local_date(zone: Tz, instant: DateTime<Utc>) -> NaiveDate {
    instant.with_timezone(&zone).date_naive()
}

/// The calendar cut a listing asks for, on the resolved time.
///
/// One of two shapes, never both: a range of days, or one month-and-
/// day across every year. The mapper refuses a request naming both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayAsk {
    /// Days from `from` (inclusive) to `until` (exclusive). Both are
    /// real calendar dates by construction — `NaiveDate` cannot hold
    /// 30 February — so [`day_window`] answers for both.
    Range {
        /// First day, inclusive.
        from: NaiveDate,
        /// Day after the last, exclusive.
        until: NaiveDate,
    },
    /// This month and day, in every year the corpus spans. `month` /
    /// `day` are in range (the mapper's check), but a given year may
    /// still lack the date — 29 February — and that year contributes
    /// no window.
    DayOfYear {
        /// Month, `1..=12`.
        month: u32,
        /// Day of the month, `1..=` the month's longest.
        day: u32,
    },
}

/// The day filter as the repository receives it: the ask, and the
/// viewer's zone the global layer reads it in.
///
/// Rows with a zone of their own are not read through this zone at all
/// — each such row has a stored local day, and the ask is compared to
/// that directly. [`global_windows`](Self::global_windows) is therefore
/// the half of the predicate that concerns unzoned rows only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayFilter {
    /// The viewer's zone.
    pub zone: GlobalZone,
    /// What is asked.
    pub ask: DayAsk,
}

impl DayFilter {
    /// The UTC windows an **unzoned** row's resolved instant must fall
    /// in one of.
    ///
    /// A range is one window from the first day's midnight to the
    /// exclusive day's midnight. A day-of-year is one window per year
    /// in `years`, OR-ed by the caller; `years` is the span the corpus
    /// occupies, which the repository knows and this function does
    /// not — handing it in is what keeps the arithmetic here and the
    /// corpus fact there. A year in which the zone has no such midnight
    /// (29 February in a common year) contributes nothing, and an empty
    /// result is a predicate that matches no unzoned row, which is the
    /// right answer for an empty corpus.
    pub fn global_windows(&self, years: std::ops::RangeInclusive<i32>) -> Vec<DayWindow> {
        use chrono::Datelike;
        let zone = self.zone.0;
        match self.ask {
            DayAsk::Range { from, until } => {
                let open = |d: NaiveDate| day_window(zone, d.year(), d.month(), d.day());
                match (open(from), open(until)) {
                    (Some(start), Some(end)) if start.from_ms < end.from_ms => vec![DayWindow {
                        from_ms: start.from_ms,
                        until_ms: end.from_ms,
                    }],
                    // Inverted or empty: nothing, on the terms the raw
                    // occurrence window sets (an empty page, not an
                    // error). Neither end can fail to open — both are
                    // real dates — so the `None` arms are unreachable
                    // and fold into the same answer.
                    _ => Vec::new(),
                }
            }
            DayAsk::DayOfYear { month, day } => years
                .filter_map(|year| day_window(zone, year, month, day))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::{America, Asia, UTC};

    fn at(ms: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(ms).expect("fixture instant")
    }

    fn asset(source: OccurredSource, time_zone: Option<Tz>) -> AssetTime {
        AssetTime {
            occurred_at: at(1_000),
            created_at: at(2_000),
            occurred_source: source,
            time_zone,
        }
    }

    #[test]
    fn the_source_picks_the_stamp_and_only_import_turns_it_to_added() {
        for source in OccurredSource::ALL {
            let resolved = resolve(&asset(source, None), GlobalZone(UTC));
            match source {
                OccurredSource::Import => {
                    assert_eq!(resolved.stamp, TimeStamp::Added, "{source:?}");
                    assert_eq!(resolved.instant, at(2_000), "{source:?}");
                }
                _ => {
                    assert_eq!(resolved.stamp, TimeStamp::Occurred, "{source:?}");
                    assert_eq!(resolved.instant, at(1_000), "{source:?}");
                }
            }
        }
    }

    #[test]
    fn the_assets_own_zone_outranks_the_viewers_and_absence_falls_back() {
        let local = resolve(
            &asset(OccurredSource::Exif, Some(Asia::Tokyo)),
            GlobalZone(America::New_York),
        );
        assert_eq!(local.zone, Asia::Tokyo);
        assert_eq!(local.zone_source, ZoneSource::Local);

        let global = resolve(
            &asset(OccurredSource::Exif, None),
            GlobalZone(America::New_York),
        );
        assert_eq!(global.zone, America::New_York);
        assert_eq!(global.zone_source, ZoneSource::Global);
    }

    #[test]
    fn slugs_round_trip_and_an_unknown_slug_is_refused() {
        for source in OccurredSource::ALL {
            assert_eq!(OccurredSource::parse(source.as_str()).unwrap(), source);
        }
        assert_eq!(OccurredSource::default(), OccurredSource::Unknown);
        let err = OccurredSource::parse("guess").unwrap_err();
        assert!(matches!(err, DomainError::Validation(ref m) if m.contains("guess")));
    }

    /// 8 March 2026 is the day America/New_York moves to daylight time
    /// (02:00 EST becomes 03:00 EDT), so the local day is 23 hours long.
    /// The window is not: it starts at local midnight — 05:00Z, EST —
    /// and runs 24 hours, to 05:00Z on the 9th, which is 01:00 EDT and
    /// one hour past the next local midnight (04:00Z). That overshoot
    /// is the rule the module doc states, measured.
    #[test]
    fn a_transition_day_is_still_twenty_four_hours_from_local_midnight() {
        let window = day_window(America::New_York, 2026, 3, 8).unwrap();
        assert_eq!(window.from_ms, 1_772_946_000_000, "2026-03-08T05:00:00Z");
        assert_eq!(window.until_ms, 1_773_032_400_000, "2026-03-09T05:00:00Z");
        assert_eq!(window.until_ms - window.from_ms, DAY_MS);
        let next_local_midnight = day_window(America::New_York, 2026, 3, 9).unwrap().from_ms;
        assert_eq!(
            next_local_midnight, 1_773_028_800_000,
            "2026-03-09T04:00:00Z"
        );
        assert_eq!(window.until_ms - next_local_midnight, 60 * 60 * 1000);
    }

    /// Cuba moves to daylight time at 00:00, so 9 March 2025 has no
    /// midnight in America/Havana: the clock goes from 23:59:59 CST
    /// (04:59:59Z) to 01:00:00 CDT (05:00:00Z). The day's first instant
    /// is the end of that gap — measured 1741496400 s, 2025-03-09T05:00Z
    /// — and the window runs 24 hours from it, to 01:00 CDT on the 10th.
    /// The same request without the gap handling answered `None`, which
    /// read as "no such date" for a date the calendar plainly has.
    #[test]
    fn a_skipped_midnight_starts_the_day_at_the_end_of_the_gap() {
        let window = day_window(America::Havana, 2025, 3, 9).unwrap();
        assert_eq!(window.from_ms, 1_741_496_400_000, "2025-03-09T05:00:00Z");
        assert_eq!(window.until_ms, 1_741_582_800_000, "2025-03-10T05:00:00Z");
        // Both ends of the gap, off the zone itself: the second before
        // the start resolves to the day before, and the start is 01:00
        // on the 9th.
        let start = at(window.from_ms).with_timezone(&America::Havana);
        assert_eq!(start.date_naive(), ymd(2025, 3, 9));
        assert_eq!(start.format("%H:%M:%S").to_string(), "01:00:00");
        assert_eq!(
            local_date(America::Havana, at(window.from_ms - 1)),
            ymd(2025, 3, 8)
        );
        // And the day after, which has a midnight, opens there.
        assert_eq!(
            day_window(America::Havana, 2025, 3, 10).unwrap().from_ms,
            1_741_579_200_000,
            "2025-03-10T04:00:00Z, midnight CDT"
        );
    }

    #[test]
    fn a_fixed_offset_zone_opens_the_day_at_its_own_midnight() {
        // Tokyo has no transitions: midnight JST is 15:00Z the day before.
        let window = day_window(Asia::Tokyo, 2026, 1, 1).unwrap();
        assert_eq!(
            window.from_ms,
            Utc.with_ymd_and_hms(2025, 12, 31, 15, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
    }

    #[test]
    fn a_date_the_calendar_lacks_is_none() {
        assert_eq!(day_window(UTC, 2025, 2, 29), None);
        assert_eq!(day_window(UTC, 2026, 4, 31), None);
        assert_eq!(day_window(UTC, 2026, 13, 1), None);
        assert!(day_window(UTC, 2024, 2, 29).is_some(), "a leap year has it");
    }

    #[test]
    fn local_date_round_trips_with_day_window() {
        for zone in [UTC, Asia::Tokyo, America::New_York] {
            for (y, m, d) in [(2026, 3, 8), (2026, 1, 1), (2024, 2, 29), (2026, 11, 1)] {
                let window = day_window(zone, y, m, d).unwrap();
                let expected = NaiveDate::from_ymd_opt(y, m, d).unwrap();
                assert_eq!(
                    local_date(zone, at(window.from_ms)),
                    expected,
                    "{zone} start"
                );
                assert_eq!(
                    local_date(zone, at(window.from_ms - 1)),
                    expected.pred_opt().unwrap(),
                    "{zone} before"
                );
            }
        }
        // The instant that makes the two zones disagree: 23:30 in Tokyo
        // is the previous day's afternoon in New York.
        let late_tokyo = Utc.with_ymd_and_hms(2026, 6, 1, 14, 30, 0).unwrap();
        assert_eq!(
            local_date(Asia::Tokyo, late_tokyo),
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
        assert_eq!(
            local_date(America::New_York, late_tokyo),
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
        let later = Utc.with_ymd_and_hms(2026, 6, 1, 15, 30, 0).unwrap();
        assert_eq!(
            local_date(Asia::Tokyo, later),
            NaiveDate::from_ymd_opt(2026, 6, 2).unwrap()
        );
        assert_eq!(
            local_date(America::New_York, later),
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_range_is_one_window_from_first_midnight_to_the_exclusive_days_midnight() {
        let filter = DayFilter {
            zone: GlobalZone(Asia::Tokyo),
            ask: DayAsk::Range {
                from: ymd(2026, 3, 8),
                until: ymd(2026, 3, 10),
            },
        };
        let windows = filter.global_windows(1970..=2100);
        assert_eq!(
            windows,
            vec![DayWindow {
                from_ms: day_window(Asia::Tokyo, 2026, 3, 8).unwrap().from_ms,
                until_ms: day_window(Asia::Tokyo, 2026, 3, 10).unwrap().from_ms,
            }]
        );
        // Inverted and empty both answer nothing, as the raw window does.
        for (from, until) in [
            (ymd(2026, 3, 10), ymd(2026, 3, 8)),
            (ymd(2026, 3, 8), ymd(2026, 3, 8)),
        ] {
            let filter = DayFilter {
                zone: GlobalZone(UTC),
                ask: DayAsk::Range { from, until },
            };
            assert!(
                filter.global_windows(1970..=2100).is_empty(),
                "{from}..{until}"
            );
        }
    }

    #[test]
    fn a_day_of_year_is_one_window_per_year_the_calendar_has_it() {
        let filter = DayFilter {
            zone: GlobalZone(UTC),
            ask: DayAsk::DayOfYear { month: 2, day: 29 },
        };
        // 2024 and 2028 are leap years; 2025-2027 are not.
        let windows = filter.global_windows(2024..=2028);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0], day_window(UTC, 2024, 2, 29).unwrap());
        assert_eq!(windows[1], day_window(UTC, 2028, 2, 29).unwrap());
        let ordinary = DayFilter {
            zone: GlobalZone(America::New_York),
            ask: DayAsk::DayOfYear { month: 3, day: 8 },
        };
        assert_eq!(ordinary.global_windows(2020..=2026).len(), 7);
    }
}
