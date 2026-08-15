# asterism-exporter-cloud 0.0.0

# asterism-exporter-cloud

Profile-driven exporter for hosted generation APIs. Like
[`asterism_exporter_http`][http] it is configured rather than
written — a platform is a JSON profile, not a Rust crate — and it
keeps that crate's grammars verbatim by sharing them
([`asterism_exporter_common`]). What it adds is the four things a
cloud platform needs and the HTTP exporter does not provide.

[http]: https://github.com/ynishi/asterism

## The bytes end up in our custody

A hosted platform answers with a URL that expires — ten minutes at
one vendor, thirty days at another. The HTTP exporter maps that URL
straight into the Derived's locator, so the record names something
that will not be there when it is next read. Here a [`FetchSchema`]
step pulls the bytes to a path under the custody root before the
harvest returns, and *that path* is the locator. The database
indexes; the bytes are files.

The path is dispatch-addressed —
`<custody_root>/dispatch/<dispatch_id>/<nnn>-<name>` — because what
the harvest needs to answer is "which files did this dispatch
produce". Content addressing is a different question, already
answered by the core's digest axes, and can be layered on top
without moving anything. Writing is idempotent by path, so a
re-collect after a failed fetch overwrites rather than producing a
second asset.

## The profile names its secret and never carries it

[`AuthSchema::secret_ref`] holds an *environment variable name*. The
value is read from the process environment at each call and used to
render `{{secret}}` in the auth header — it is not in the params
blob, so it is not on the dispatch row, which matters because params
are persisted unedited and handed back on every read. Loading a
`.env` file is the binary's job, done once at startup: an adapter
that went looking for dotenv files itself would make "which file did
this credential come from" invisible to the profile that named it.

The one place the resolved value could leak back out is the recorded
exchange, so the recording redacts the auth header by name *and*
scrubs any other occurrence of the value (see [`Exchange`]).

## The profile declares its own deadline

No shared default: the measured range across platforms is ten
minutes to thirty days, so a constant would be wrong nearly
everywhere. [`CloudDispatchParams::deadline_seconds`] is required,
and exceeding it fails the job with a message starting
[`EXPIRY_PREFIX`] — distinguishable from a backend failure, which is
reported in the backend's own words.

## The raw exchange is kept when asked for

With `"record_exchange": true` the request as sent and the response
as received are kept on the dispatch row, in the exporter-owned
handle payload the runner already persists and hands back. That is
the cheapest thing that satisfies "on the dispatch row rather than
on a Derived", and it is an assumption: a dedicated column is the
obvious alternative, and the measurement that would force it is a
generation payload large enough that carrying it through every poll
is the wrong shape. Recorded here rather than assumed away.

## Modules

- [`custody`](custody.md): Where a produced file lands once we hold it.
- [`grammar`](grammar.md): The profile grammar this adapter uses: the shared one, plus

