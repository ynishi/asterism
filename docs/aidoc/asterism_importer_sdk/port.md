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

## Why the classes are what they are

[`SourceError`] replaced a three-variant enum that named where a
failure happened — source unavailable, item read failed, other.
Nothing downstream could act on that, so a run against a rejected
credential and a run that skipped one unreadable file produced the
same report: a count, and a message only a person could read.

The classes here are cuts a caller acts on differently. A
configuration nobody has changed will be refused again, so the run
stops and the message is for whoever wrote it. A source that was
briefly unreachable will not, so the same run repeated is worth
something — and a rate limit is that case with the wait stated
rather than guessed, which is what lets a report say when the source
is expected back instead of only that it failed. A record that could
not be read costs that record, and a run missing one file out of ten
thousand is not a failed import.

What the classification does not do is tell a scanner how to behave.
That is [`SourceError`]'s own section below, and it is one rule.

## Types

- `Disposition` — What a [`SourceError`] means for the run it happened in.
- `SourceError` — Why a source could not be read.
- `SyncState` — Where a scan left off, so the next one can start there.

