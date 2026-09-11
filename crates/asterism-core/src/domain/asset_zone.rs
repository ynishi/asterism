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
//! from that day's local midnight**, and where the midnight falls is
//! the zone's rule for that year — a daylight-saving transition moves
//! it without a line of code here noticing. What is deliberately not
//! handled, and stated once so nobody looks for it: a midnight a zone
//! skips, a day a zone repeats or shortens, and the date line are all
//! read as whatever `earliest()` and `+ 24h` give, and nothing here
//! corrects for them.

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

/// The 24 hours from `(year, month, day)`'s local midnight in `zone`,
/// as UTC milliseconds.
///
/// `None` when `earliest()` has no midnight to start from: a date the
/// calendar lacks (29 February in a non-leap year, 31 April), and — the
/// case the module doc says is not handled — a midnight the zone's
/// rule skipped that year. The window is `+ 24h` from the start rather
/// than "to the next local midnight", so a day a transition shortens
/// or lengthens is still answered as 24 hours; that is the rule, and
/// the test on a transition day states what it measures.
pub fn day_window(zone: Tz, year: i32, month: u32, day: u32) -> Option<DayWindow> {
    let midnight = zone
        .with_ymd_and_hms(year, month, day, 0, 0, 0)
        .earliest()?;
    let from_ms = midnight.timestamp_millis();
    Some(DayWindow {
        from_ms,
        until_ms: from_ms + DAY_MS,
    })
}

/// The calendar day `instant` falls on in `zone` — the inverse of
/// [`day_window`], and the one function that writes the derived
/// `occurred_local_date` column.
pub fn local_date(zone: Tz, instant: DateTime<Utc>) -> NaiveDate {
    instant.with_timezone(&zone).date_naive()
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
}
