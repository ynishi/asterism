# asterism-importer-sdk::scanner

`SourceScanner` trait and shared item type.

Enumerates or watches an external source and produces [`RawItem`]s.
Bundled implementations live in the sibling modules
([`fs`] and [`sqlite`]); importer authors typically reuse one
instead of writing their own.

## Types

- `ItemStream` — Async stream of scanned items, or failures.
- `RawItem` — A raw scanned item — a payload plus the metadata needed to attribute
- `ScanFuture` — Future returned by [`SourceScanner::scan`] — resolves to the item
- `ScanMode` — Scan mode passed to [`SourceScanner::scan`].

## Traits

- `SourceScanner` — Trait every source scanner implements.

