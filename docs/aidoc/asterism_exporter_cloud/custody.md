# asterism-exporter-cloud::custody

Where a produced file lands once we hold it.

`<custody_root>/dispatch/<dispatch_id>/<nnn>-<name>`.

Dispatch-addressed rather than content-addressed. The question the
harvest asks is "which files did this dispatch produce", and this
layout answers it by listing a directory — no index, no lookup, and
it still answers after the platform's URL has expired and the
response that named it is gone. Content addressing answers a
different question (is this the same file as that one), the core's
digest axes already answer that one, and a digest can be computed
over these bytes later without moving them.

Writing is idempotent by path: the index within the harvest and the
dispatch id are both stable, so a re-collect after a failed fetch
overwrites the same file rather than producing a second asset beside
the first.

## Types

- `CustodyPaths` — Resolves custody paths under one root.

