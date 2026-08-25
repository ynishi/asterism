# asterism-teams-wire::projection

The captured projection — descriptive metadata a promoter said at
the time (#148 decisions 12, 13 and 14).

## What this crate's part in it is

Decision 14 keeps the body opaque, and the transport's share of
that is small and absolute: **the body is a `String` here and no
shape in this crate has a field that came from inside it.** Why the
rule exists, and why
[`EntryProjectionEnvelope::version`] is a fact about the envelope
rather than a breach of it, are argued once in
`teams_core::domain::projection`.

Decision 13's declaration is likewise settled before a body reaches
here — it lives at the member's mapper, the only place that knows
both the local model and the body. By the time this crate sees one,
the answer is a string. A filter expressed on the wire would be a
second place to forget something, and forgetting there fails in the
unsafe direction.

## Types

- `EntryProjectionDto` — One captured projection, read back
- `EntryProjectionEnvelope` — One entry's projection, as it rides onto a round push.
- `WithProjections` — A round push as a team takes it: whatever the forge's push command

## Constants

- `PROJECTION_VERSION` — The current envelope version — what a client writing today stamps,

