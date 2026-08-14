# asterism-core::domain::observation

Observability domain — the four streams and their policies.

What the application records about itself is split by *writer,
volume, value and read pattern*, because those four properties
differ across the four kinds and every downstream decision
(retention, sampling, where a record may be read) follows from them.

This module owns the classification. It does not write anything —
`asterism-infra` holds the tables and the sink, and consults
[`STREAM_REGISTRY`] for policy rather than repeating it.

## Types

- `Env` — Which dataset a record was produced against.
- `Stream` — One of the four observation streams.
- `StreamPolicy` — Retention, sampling and persistence floor for one stream.

## Constants

- `STREAM_REGISTRY` — Closed registry of stream policies.

