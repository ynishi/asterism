# asterism-importer-sdk::store

Where a resumption point is kept, as the importer sees it.

#293 gave a scan a position and a run the right to hand one back, and
left it homeless: the point was printed and an operator carried it to
the next run by hand. This is the port that ends that, and the only
thing it says about *where* a point is kept is that somewhere is not
here.

## Why a port rather than a call

The obvious shape is for the runner to POST to the server directly,
and it is the wrong one for the same reason the rest of this crate is
written against traits. An importer under test would then need a
server; and the transport is the question that is still open — an
adapter that runs itself and pushes is the shape taken today, and one
Asterism starts and reads is the one that was not. A port is what
lets the second arrive without the runner learning about it.

[`HttpSyncStore`](crate::client::HttpSyncStore) is the implementation
that exists, and it is thirty lines over the same `ApiClient` every
record already travels through.

## Types

- `StateKey` — What a stored position is filed under.

## Traits

- `SyncStore` — Somewhere to keep a resumption point between runs.

