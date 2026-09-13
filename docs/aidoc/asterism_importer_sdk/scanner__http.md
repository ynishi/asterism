# asterism-importer-sdk::scanner::http

`HttpScanner` — paginated HTTP source scanner.

Asks a URL for a page of records, hands each one over, follows the
cursor the response carried, and repeats. The caller supplies the
request and says how to read the answer; this scanner knows nothing
about any particular service.

**The inbound twin of [`SqliteScanner`](super::sqlite::SqliteScanner),
and deliberately not an API client.** That one takes a `SELECT` and a
column map and has no opinion about anybody's schema; this takes a
request and a few field names and has no opinion about anybody's API.
A named service is configuration rather than code, which is the only
version of "thirty or forty adapters" that stays maintainable.

# Why this one exists

Not for the sources it reaches. The port's classification was written
for remote sources, and no *scanner* had ever been one: a directory
cannot be rate limited and a SQLite file is never briefly
unreachable. [`RateLimited`](crate::SourceError::RateLimited) was
constructed nowhere in this workspace but a fixture in the runner's
tests, so `retry_after` — the field that whole class exists for —
had never been filled in by anything that read a header. An
interface is only as good as the adapter shaped least like the ones
it was written against, and this is that adapter.

Three things it does differently, each of which the port had ruled
on and nothing had exercised:

- **Its offset cannot be compared.** `FsScanner` compares paths and
  `SqliteScanner` compares ids, so both can decide whether a record
  falls before a resumption point. A cursor can only be handed back
  to the source that issued it. The port's rule that an offset is
  opaque and only its writer reads it is what makes that work.
- **Its checkpoints are coarse.** `FsScanner` emits one behind every
  file, and `SqliteScanner` behind every row it can order. A page is
  the smallest thing this can take up after, so fifty records share
  one checkpoint.
- **Resuming costs nothing.** The others read from the beginning and
  skip what is behind the point. This asks the source to start
  there, so the records before it are never fetched, parsed or paid
  for.

# What it does not know how to do

One pagination dialect: a cursor at a path in the response body, sent
back as a query parameter. `Link: rel="next"` headers and page
numbers are the other two common ones and are not here — the shape to
add them is an enum in place of [`Cursor`], and adding it before a
second dialect is actually needed would be guessing at what it needs
to hold.

It also leaves
[`payload_is_whole_artefact`](super::SourceScanner::payload_is_whole_artefact)
at its `false` default, for the same reason `SqliteScanner` does: a
record lifted out of a JSON array and addressed `<url>#<id>` has no
bytes of its own at that address, so a digest declared from here
could never be checked.

## Types

- `Cursor` — How the next page is asked for.
- `HttpScanner` — Paginated HTTP scanner.
- `RecordMap` — Where the records are in the response, and what identifies one.

