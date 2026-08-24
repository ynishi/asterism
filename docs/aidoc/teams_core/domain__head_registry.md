# teams-core::domain::head_registry

`head_registry` — the instance's carriage of a trained tag head
(#132 phase 3).

The artifact (`asterism-tag-head-v1`) is what a member's training
run writes locally: the per-tag rows, the encoder identity they
were trained against, and the held-out eval. It is kilobytes of
JSON, which is why it rides this registry row whole — no blob
store involved. The instance is a carrier, not an authority, the
same stance the route took when it carried the model entry
(#127): it stores and re-serves the publisher's bytes **verbatim**
and deliberately types nothing of the body — no rows, no eval, no
floors. Verification belongs to the member app, which re-runs the
same checks its startup bind runs before a pulled head may score.

What *is* validated is the envelope — the part the instance
answers for as a carrier: the bytes are one JSON object, the
`schema` field names the one artifact version this instance
carries, `head` is the non-empty label supersession history is
keyed by, and the encoder identity fields are present — a pull
must be able to refuse a head trained against another encoder
before parsing anything deeper.

The schema string is the app-side artifact's
(`asterism-infra`'s head store writes it); it is re-spelled here
rather than imported because `teams-*` depends on `asterism-core`
only (#83 §4) — the same one-notation-two-spellings shape as the
digest grammar note in [`crate::domain::store`].

## Types

- `TagHeadEntry` — A validated head entry: the publisher's bytes, verbatim, plus the

## Constants

- `HEAD_ENTRY_SCHEMA_V1` — The one artifact schema this plane carries. A future `-v2` is a

