# asterism-importer-sdk::port

The inbound port's shared vocabulary: what a failure is, and what a
resumption point holds.

These two types are what the scanner traits are written against, and
they are deliberately the part of the port that does not depend on
how an adapter is run — whether Asterism starts it and reads it, or
it runs itself and pushes. A rate limit is a rate limit either way,
and a cursor holds the same thing either way.

## Why a classification at all

[`SourceError`] replaced a three-variant enum whose variants said
where the failure happened rather than what to do about it, and the
runner treated all three alike: record the message, carry on to the
next item. A source that had gone away was handled as one unreadable
file, so an import against a moved directory or an expired
credential spun through its whole stream reporting errors instead of
stopping and saying so. With seven importers that is a bad hour;
with thirty it is thirty separate answers to the same question.

The five classes here are the ones inbound frameworks converged on —
Airbyte's `config_error` / `transient_error` / `system_error` with
`RATE_LIMITED` broken out as its own action, and Kafka Connect's
coarser `RetriableException` — plus the per-item class this port
already had and which none of those needs, because they stream rows
rather than read files.

## Types

- `Disposition` — What a caller should do about a [`SourceError`].
- `SourceError` — Why a source could not be read.
- `SyncState` — Where a scan left off, so the next one can start there.

