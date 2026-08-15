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

Where the resolved value could leak back out is anything this
adapter writes down about the call: the recorded exchange, and the
note the harvest puts on each produced asset. The recording redacts
the auth header by name *and* scrubs any other occurrence of the
value (see [`Exchange`]); the note scrubs what it copies for the same
reason, in one place, because a platform that echoes the request is
how a query-string credential comes back (see [`call_note`]).

## The profile declares its own deadline

No shared default: the measured range across platforms is ten
minutes to thirty days, so a constant would be wrong nearly
everywhere. [`CloudDispatchParams::deadline_seconds`] is required,
and exceeding it fails the job with a message starting
[`EXPIRY_PREFIX`] — distinguishable from a backend failure, which is
reported in the backend's own words.

## The call is recorded, and it arrives with the artefact

The request as sent and the response as received are kept on the
dispatch row, in the exporter-owned handle payload the runner already
persists and hands back — and the harvest copies that record onto
every [`Derived`] it returns, under `extra.cloud.call`, together with
the finished job's response whole. The reified asset therefore
carries how it was made rather than only a dispatch id to go and ask,
and it carries the part that is easiest to lose: the seed the
platform ran with and the prompt as it rewrote them are siblings of
the artefacts array, so keeping the selected item alone would drop
exactly the two values a hosted call cannot reconstruct.

Both halves are unconditional, and that is a change from the first
cut of this adapter, where recording was a profile flag defaulting to
off. A hosted platform hands back a result URL and little else: the
model may be an ambient default the provider updates, the seed is an
input that is usually not echoed, and an enhanced prompt is not the
prompt that was sent. None of it is in the bytes, and none of it can
be recovered later by parsing them — the moment of the call is the
only moment it exists. A switch that turns off the one capture point
turns off the record entirely, and its default decided that for
nearly every profile. A profile that still carries the retired flag
keeps parsing, whichever way it is set — including `false`, which
this build no longer honours.

What it costs is honest: a submit body and a submit response ride
along in the handle payload the poll loop reads on every tick, and a
copy of the record lands on each produced asset. That second copy is
per artefact and carries the whole harvest response, so a job with
several outputs stores the envelope once per output — for a hosted
generation call, kilobytes each. A generation payload large enough to
make that the wrong shape would be the measurement that forces a
dedicated column, and this adapter has not met one; the ComfyUI-scale
workflow blob that would is carried by a different exporter, against
a backend that embeds it in the file anyway.

## Modules

- [`custody`](custody.md): Where a produced file lands once we hold it.
- [`grammar`](grammar.md): The profile grammar this adapter uses: the shared one, plus

