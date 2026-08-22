# asterism-importer-sdk::parser

`SourceParser` — turn a scanned [`RawItem`] into one or more
[`Footprint`]s.

Parsers are the only piece an importer author must write; everything
else in the pipeline (scanning, mapping to `AddAssetCommand`, HTTP,
progress) is provided by the SDK.

# Contract

Return one `Footprint` per **thing the user collected** (one chat
message, one image, one doc, one note). Do **not** return one giant
`Footprint` per raw item when the item contains many collectibles —
for example, one `.jsonl` file typically becomes many
`Footprint::ChatMessage`s, not one summary. See
`crate::footprint::Footprint` for how each variant maps to the
server-side asset.

# `occurred_at` fallback ladder

Every footprint needs a `DateTime<Utc>`. Parsers resolve it in this
order, top wins:

1. **A timestamp inside the payload** — a `timestamp` field on the
   JSON record, a `created_at` column on the SQLite row, an EXIF
   `DateTimeOriginal`. This is per-*record* and is by far the most
   accurate.
2. **[`crate::scanner::RawItem::occurred_at`]** — what the scanner
   derived from the container (file `mtime`, HTTP `Date` header,
   row timestamp column when the scanner already lifted it). This
   is per-*container*, so it is a good fallback for single-record
   items (one image = one file, one doc = one file) and a
   **coarse** fallback for multi-record containers (all messages
   in one `.jsonl` share the same `mtime`, which loses ordering).
3. **`Utc::now()`** — last resort when neither of the above is
   available. Signals to the caller that ordering downstream will
   be unreliable.

Never invert the order. Using the file `mtime` for a chat message
inside a session log makes every message look like it arrived at
the moment the file was last flushed, which erases the
within-session ordering the domain relies on for edge / grid
placement.

# Partial success on multi-footprint items

One `RawItem` may yield many footprints (JSONL: one file → many
messages; SQLite: one table → many rows). If some records inside
the item are malformed, prefer to **skip them and return the good
ones** rather than returning `ParseError::Malformed` for the whole
batch — a single bad line should not drop the rest of a session.
Reserve [`ParseError::Malformed`] for cases where the whole
`RawItem` is unusable (wrong file type, unreadable header).

Skipping is not silent. Count what was dropped and say so once per
container at the end of `parse`; [`RecordAddresses`] owns both the
count and the wording. An importer that drops records without a
word leaves the operator reading a run that looks complete.

# A record's address is the source's to give

A record inside a container is addressed
`<container>#<the id the source declared>`. When the source
declares no id there is no address, and the record does not become
an asset. Do **not** substitute the record's position — a line
number, an array index, an ordinal.

A position describes the container's contents at one moment, not
the record. Insert one line ahead and every address behind the
insert lands on its neighbour, where the server's
`(source_kind, source_locator)` lookup finds the neighbour's row
and discards the arriving payload. Nothing errors; the import
reports success and the record is gone. The address is also
unreadable in the other direction — the readers in
`asterism-infra` match a fragment against the record's own id, and
no record has the id `L3`, so the body never resolves.

[`RecordAddresses`] is the shared implementation of this rule.

## Types

- `ParseError` — Errors returned by parsers.
- `RecordAddresses` — One container's worth of the addressing rule above: hands back the

## Traits

- `SourceParser` — Trait every source parser implements.

