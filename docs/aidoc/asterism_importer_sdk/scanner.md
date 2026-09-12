# asterism-importer-sdk::scanner

`SourceScanner` trait and shared item type.

Enumerates or watches an external source and produces
[`ScanEvent`]s: the [`RawItem`]s themselves, and the points a later
scan could take up from.
Bundled implementations live in the sibling modules
([`fs`] and [`sqlite`]); importer authors typically reuse one
instead of writing their own.

## Types

- `ItemStream` — Async stream of scan events, or failures.
- `RawItem` — A raw scanned item — a payload plus the metadata needed to attribute
- `ScanEvent` — What a scanner puts on its stream: a record, or a point it could be
- `ScanFuture` — Future returned by [`SourceScanner::scan`] — resolves to the item
- `ScanMode` — Scan mode passed to [`SourceScanner::scan`].

## Traits

- `SourceScanner` — Trait every source scanner implements.

