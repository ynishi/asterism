# asterism-core::application::series_strategy_service

`SeriesStrategyService` — registering, editing and removing the rules
the series axis derives keys under.

The axis has worked since S3 and had no door: the seeded `VDSL recipe`
was the only rule any library could hold. This is the door, and it is
an HTTP-shaped one for a structural reason — an importer runs in its
own process and talks to the server over the wire, so a rule it (or
the agent driving it) wants to state has to cross as **data**. What
comes in is a caller's declaration, on the same terms `album_meta`
arrives: believed, labelled as somebody's statement, and never a
reading the server made of anybody's bytes.

Every write here takes an [`AttributionContext`] it does not persist:
`series_strategy` carries no attribution column and is not getting
one, since a rule states how some *generator* writes metadata rather
than anything a person authored. Taking the argument is still the
point — see the [`application`](crate::application) module doc for
why, and `ModalityService`, which stands in the same position over
the same kind of master table.

# Refusing a rule is the point of this file

One `series_strategy` row this build cannot read makes **every** page
of the derivation walk fail — the walk promotes every rule on the page
and one `Err` is the page — so no material gets a key under *any*
rule (`SqliteSeriesRepository::scan_underived` states the blast
radius). The column can only hold what a writer put there, and this is
the writer. So the checks below are not input hygiene; they are the
thing standing between a typo and a library with no series keys at
all, and each of them refuses something the schema would happily
store:

- an unknown `decode` token — the `CHECK` would refuse it, so the
  write fails loudly, but only for tokens the schema knows to name;
- an `applies_to` that is not a `type/subtype` pair — [`MimeType::parse`]
  is total, so `""`, `"png"` and `"image/*"` all store fine and claim
  nothing forever;
- an **empty path** — a rule that means nothing and says nothing about
  it: an empty `include` path selects nothing and an empty `exclude`
  path drops nothing (see [`Path`]), so the author who wrote `[[]]`
  gets a rule that silently is not the one they wrote.

What the type system already refuses arrives as a deserialisation
failure at the transport, before any of this runs.

# An edit invalidates, and invalidation is a delete

Four of the five fields are inputs to a key. Changing one makes every
key derived under the id a key nothing would derive again, and the
whole of the repair is: delete that rule's rows, then let the walk —
whose population is "a pair with no row" — answer them under the new
rule. `name` is not one of the four, so a rename costs nothing.

# KNOWN LIMITATION — nothing here bounds how big a rule is

`name`, the path count and the segment lengths are unbounded; the only
ceiling is axum's default 2 MB request body. That is not merely a
large row, because of where the rule travels:
`underived_page_sql` selects `s.include, s.exclude` **per pair**, and a
page is `SERIES_DERIVE_PAGE = 200` pairs, so one rule carrying a 2 MB
`include` makes every page of the walk materialise on the order of
400 MB. `RegisteredRow`'s doc declines to carry *three `i64` columns*
down that same path, which is the measure of how much this one is not
covered by that care.

No number is picked here because there is no measurement to pick it
from, and this codebase's other ceilings are all measured (the PNG
probe's metadata cap is 1 MiB against a weighed 40 KB card).
What is owed is a reading of what a real rule weighs — the seeded one
is 21 bytes of `include` — and a limit an order or two above it,
refused here beside the other three. Recorded rather than guessed
(see `drafting-discipline`'s rule against writing a bug into a spec:
this is a bug carried, not a design).

## Types

- `SeriesStrategyService` — Series Strategy lifecycle: list / create / update / delete, plus the

