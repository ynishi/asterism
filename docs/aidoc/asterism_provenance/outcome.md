# asterism-provenance::outcome

What applying a record to a file actually achieved.

Here rather than beside the writer that produces it, because two
layers need to name it and neither may depend on the other: the port
is declared in `asterism-core` (ports live in the core; adapters
never define traits) and the implementation lives in
`asterism-infra`. This crate is the one both can already see.

# Why a failure lives in the value and not only in the error

Applying a disclosure is two operations against one file, and either
can fail without the other being affected. An error return can say
only "the whole thing failed", so a writer that reports a failed
manifest that way discards the packet it had already produced —
which is how an expired certificate came to withhold the half that
needs no certificate at all.

So the two halves report their own outcome, including their own
failure, and the error channel is left for the case where nothing
could be attempted: the file could not be read, or its container is
one this build does not write into. What a caller does with a
half that failed is the caller's decision — a manifest that did not
land while the packet did is still a disclosed file, and calling
that an export failure is the judgement this type exists to let
somebody else make.

## Types

- `Half` — What became of one half of a disclosure.
- `Skipped` — Why a half was not attempted.
- `Stamped` — The result of writing a [`DisclosureRecord`](crate::DisclosureRecord)

