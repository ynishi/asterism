# asterism-core::application::send_service

`SendService` — putting a release's stamped set on a destination's
host.

One verb: [`send`](SendService::send) reads the release's file rows,
builds the transport's params from the caller's profile plus that
list, starts a dispatch over the release's own snapshot, and records
that it happened.

# The bytes are the release's copies, never the library's

Stamping runs after harvest, on the copies the `file` exporter
reported — [`outbound_stamp`](crate::application_support::outbound_stamp)
is where that ordering is decided. A transport sends its bytes inside
`dispatch`, before harvest, so a transport pointed at the frozen
members would send the library's own unstamped files. The stamped set
already exists by then and the release names every copy by path, so
the paths are what this hands over and the ordering falls out with
nothing in the runner to change.

The dispatch still runs over the release's snapshot, which is what
makes the exporter's inputs the same assets the copies were made
from. That is what the sidecar's columns are rendered against; it is
not where the bytes come from.

# Where the file list lives in the params

Under [`RESERVED_KEY`], written here and refused from a profile. The
alternative was a second argument to `Exporter::dispatch` beside
`params`, which would widen the port every adapter is written against
for one adapter's convenience. A reserved key costs the profile
author one word they may not use, and the refusal below is what tells
them so — a profile that set it would otherwise decide which bytes
left.

# A send with nothing to send is refused

Two states get the same answer, [`Blocked`](crate::error::ConflictKind::Blocked),
because both are the state being in the way rather than the request
being malformed: a release whose run has not written its files yet,
and a release one of whose copies is no longer where it was written.
Sending the first would put an empty directory on somebody's host;
sending the second would send a set that is not the set the release
stamped.

## Types

- `SendService` — Recording a send.

## Constants

- `RESERVED_KEY` — The params key this service writes the file list under, and which a

