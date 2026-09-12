# asterism-importer-sdk::port

The inbound port's shared vocabulary: what a failure is, and what a
resumption point holds.

[`SourceError`] is what the scanner traits are written against.
[`SyncState`] is not yet in any signature: the type and its
serialised form are settled here first, ahead of the transport that
will carry it. Both are deliberately the part of the port that does
not depend on how an adapter is run — whether Asterism starts it and
reads it, or it runs itself and pushes. A rate limit is a rate limit
either way, and a cursor holds the same thing either way.

## Why a classification at all

[`SourceError`] replaced a three-variant enum whose variants said
where a failure happened rather than what to do about it. Nothing
downstream could act on that, so the decision moved into the
scanners: `SqliteScanner` returned from its own reader thread after
sending a source-level failure, ending the stream, and `FsScanner`
let its watch task fall out of its loop. Each author decided, in
their own way, that this failure meant stop — and an adapter that
decided otherwise would keep a dead scan alive with nothing able to
tell. One adapter deciding that is a preference. Thirty or forty
deciding it separately is thirty or forty answers to one question,
and an upstream change moves every one of them.

The five classes here are the ones inbound frameworks converged on:
Airbyte's `config_error` / `transient_error` / `system_error`
failure types, `RATE_LIMITED` broken out as an action of its own,
and Kafka Connect's coarser `RetriableException`. Carrying on past a
single bad record is in both of those too — Connect's
`errors.tolerance` with a dead-letter queue, Airbyte's `IGNORE`
action — but as an operator setting rather than a thing the source
says. Here it is a variant, because the scanner is what knows that
one file would not open while the directory is fine.

## Types

- `Disposition` — What a caller should do about a [`SourceError`].
- `SourceError` — Why a source could not be read.
- `SyncState` — Where a scan left off, so the next one can start there.

