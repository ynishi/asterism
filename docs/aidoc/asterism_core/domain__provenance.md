# asterism-core::domain::provenance

`ProvenanceRef` — how a re-ingested artefact names where it came from.

An img2img output is a *new file*. Nothing inside it points back at
the picture it was made from: the generator wrote its own bytes, and
whatever metadata the input carried did not survive the round trip
(a different tool, a different container, a re-encode). So the link
cannot be recovered from the artefact — it has to be *declared* by
whoever ran the chain.

That declaration is this module. The ingest side accepts one string,
and its scheme says how to resolve it:

| form | meaning |
|---|---|
| `asset:<uuid>` | the parent asset, named directly |
| `dispatch:<uuid>` | every asset that dispatch produced (an export is a dispatch, so this names "what I sent out") |
| `sidecar` | read the `<file>.meta.json` sitting next to the input |

# Why there is no receipt id

An earlier draft minted an "export receipt" to hand out. It turned
out to be redundant: an export *is* a dispatch, its outputs are
reified as assets, and both already have ids. A receipt would only
have added a shorter token and an expiry rule — not worth a second
id space that can disagree with the first.

# Why a string and not a typed field per form

The value is carried *outside* Asterism — through a shell pipeline,
a note in a chat, an n8n variable, a person's clipboard. Whatever
survives that trip has to be one opaque token that can be copied,
and a scheme prefix is how the receiver still knows what it holds.
It is the same reasoning as the `sha256:` prefix on
[`content_hash`](crate::domain::content_hash): the value declares
how to read it, so a second form can land later without a second
column.

# Why parsing is separate from resolving

Parsing is total and pure — it can say "this is a receipt id" without
a database. Resolving needs repositories and can legitimately fail
(the receipt expired, the parent was purged). Keeping them apart
means the ingest path can record *what was claimed* even when it
cannot confirm it, which is the behaviour that matters: a broken
link is not a reason to refuse the file.

## Functions

- `parse` — Reads a declaration.

## Types

- `ClaimRelation` — What a claim asserts about the artefact and the thing it names
- `ProvenanceRef` — A declared origin for an artefact being (re-)ingested.

## Constants

- `CLAIM_FIELDS` — Every field inside `_trace` that a provenance claim owns.
- `TRACE_KEY` — Key under which the ingest path records the provenance claim on

