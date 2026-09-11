# asterism-core::domain::asset_zone

Which stamp is an asset's time, and which zone reads it — resolved
once, in one place, so nothing downstream branches on either.

An asset row carries two instants and neither is "the time" on its
own. `occurred_at` is when the thing it records happened, and it is
the right answer for a photograph or a message. It is the wrong
answer for a row whose importer had no occurrence to record and
wrote the import moment in its place — a generated image, a
transcript export, a database dump — where the moment the row
*arrived* (`created_at`) is the only time it has. Which of the two
a row means is not a guess made at read time: the importer that
wrote the row knew, and [`OccurredSource`] is where it says so.

The instant is then a point on the UTC line, and a calendar day is a
zone's reading of that line. Two layers supply the zone, on the
shape [`app_setting`](crate::domain::app_setting) uses for a
setting's value:

- **Local** — a zone the asset itself carries
  ([`AssetTime::time_zone`]), when a supplier recorded where the
  thing happened. The higher layer: a row that knows its own zone is
  not re-read in somebody else's.
- **Global** — the zone the viewer is asking from
  ([`GlobalZone`]), carried on the query the way a collation is
  (`SortSpec::collation`) rather than held as process state. The
  fallback, and the only zone most rows have.

[`resolve`] applies both rules and hands back one [`ResolvedTime`]:
the stamp, the instant it names, the zone, and which layer supplied
the zone. A consumer reads the result and never the inputs — the
service that maps a query, the repository that writes a derived
column, and the mapper that puts the answer on the wire all call
the same two lines instead of each carrying a copy of the rule.

[`day_window`] and [`local_date`] are the two directions between an
instant and a calendar day under a zone. A day is **the 24 hours
from that day's first instant** — its local midnight, or the end of
the gap where a zone's rule skipped that midnight — and where that
falls is the zone's rule for that year: a daylight-saving transition
moves it without a line of code here noticing. What is deliberately
not handled, and stated once so nobody looks for it: the window
[`day_window`] opens for a day a zone repeats or shortens is still
24 hours from its start (a range cut takes midnight to midnight
instead, and a zoned row is matched on its calendar day — neither
reads this window's length), and a date a zone skipped whole at the
date line is a date its calendar does not have; nothing here
corrects for either.

## Functions

- `day_window` — The 24 hours from the first instant of `(year, month, day)` in
- `local_date` — The calendar day `instant` falls on in `zone` — the inverse of
- `resolve` — Resolves an asset's time against the viewer's zone.

## Types

- `AssetTime` — The four facts on an asset row that its time is resolved from. Each
- `DayAsk` — The calendar cut a listing asks for, on the resolved time.
- `DayFilter` — The day filter as the repository receives it: the ask, and the
- `DayWindow` — One calendar day as a half-open UTC window, in the unit the
- `GlobalZone` — The viewer's zone — the global layer, carried on the query.
- `OccurredSource` — Where a row's `occurred_at` came from — written by the importer that
- `ResolvedTime` — One asset's time, resolved: what a consumer acts on.
- `TimeStamp` — Which of an asset's two instants is its time.
- `ZoneSource` — Which layer supplied the zone a [`ResolvedTime`] is read in.

