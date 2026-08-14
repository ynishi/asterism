# asterism-server::attribution

Turning what a remote caller said into the attribution a write
records.

Both remote surfaces this crate owns — the HTTP routes and the MCP
tools — are the [`Asserted`](AttributionChannel::Asserted) channel:
whatever `author_kind` / `author_subject` / `operator_ai` a command
carries is the caller's own statement about itself, believed and
labelled as such. Translating those fields into an
[`AttributionContext`] is the adapter's job; the
services below this layer never read them.

It is one function rather than an inline `AttributionContext::asserted`
per route so that every remote write agrees on what the fields mean,
including the two answers the pair form makes possible: a corrupt
pair is refused here, and an owner claim is refused by the
constructor (a caller cannot state owner-ness — that follows from the
surface or from authentication, never from the claim).

## Functions

- `asserted` — Builds the context for a request that arrived over HTTP or MCP.

