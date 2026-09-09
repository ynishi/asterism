# asterism-infra::material_bytes

The bytes a locator addresses, when they are on this machine.

Every job that decodes, measures or hashes an original asks the same
two questions in the same order, and the answers are not the same
kind of thing: *are there bytes here at all* decides whether the row
is retired, and *could they be read just now* decides whether it is
left for a later pass. Recording a temporary failure permanently is
the dims-walk mistake those jobs each carry a comment about.

[`read`] answers both in one value, so the two stay in the order
that keeps them apart:

- `None` — nothing here will ever have bytes. A remote locator, a
  caller-minted name, or a record inside a container nothing opens.
- `Some(Err(_))` — bytes that should be here and were not readable
  this time.
- `Some(Ok(bytes))` — the bytes.

## Which containers open

A [`Record`](SourceLocator::Record) is a container plus an address
inside it, and `SourceLocator::local_path` refuses to hand over the
container on the record's behalf — a thousand-line log would answer
every line with the whole file, which is one fingerprint repeated a
thousand times. That reasoning holds for a record whose bytes *are*
the container's: a JSONL line is text the importer already carried,
and there is nothing else in the file that belongs to it alone.

A ZIP entry is the other case. It has bytes of its own, at a known
offset, and reading it yields those and nothing else. So the opening
is per container shape rather than blanket, and [`opens_for_records`]
is the whole list: `.charx`, the character-card archive. A plain
`.zip` is not on it and is never handed here — an archive is not
opened because it is an archive.

## Functions

- `opens_for_records` — Whether a container's shape is one whose records can be read out of
- `read` — The bytes this locator addresses.

