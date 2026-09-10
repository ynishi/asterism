# asterism-core::domain::disclosure::carried

What a file currently carries, and whether it still stands up.

The read side of what [`outcome`](super::outcome) says for the write
side. `Stamped` says what an application put into a file; this says
what is in one now, which is a different question with a different
set of answers — a file nobody here ever stamped has an answer, and
a file stamped last week may have stopped matching its own bytes
since.

# Two axes, and folding them is the trap

*Does the mark still describe these bytes* and *is the signer
somebody we trust* are separate questions, and the second one is not
about the file at all. C2PA says so itself: a manifest whose only
failure is `signingCredential.untrusted` stays in the `Valid`
validation state, and the reference implementation filters that
status out of the summary a person reads.

The reason to keep them apart here is sharper than tidiness. In the
reference implementation trust checking is on by default while the
anchor set is empty outside its own test configuration, so a release
build reports **every** signer as untrusted — a genuine
conformance-program certificate included — until anchors are
configured. A verdict that folded trust into integrity would show a
correctly signed file as broken in production and fine under `cargo
test`, which is the shape of bug that survives a test suite.

So [`Mark`] answers only for the bytes, [`Signer`] answers only for
the certificate, and [`Carried`] holds both without mixing them.

# Why an absent mark is a value and not a `None`

"This file carries nothing" is an answer somebody acts on — it is
the row that says *re-apply from the database* — and it is not the
same as "nobody looked". The series key already records that
distinction for a different question, and the reasoning carries: an
`Option` would spend the one shape that means "unasked" on a state
the reader deliberately reached.

## Functions

- `integrity_of` — The integrity verdict for a manifest, from the failure codes a

## Types

- `Carried` — What one file carries, on both axes, for both kinds of mark.
- `Mark` — What one kind of mark says about the bytes it sits in.
- `Signer` — What the deployment can say about who signed.

## Constants

- `BINDING_FAILURES` — Validation status codes that say the bytes changed after signing.
- `SIGNATURE_MISMATCH` — The code two different things produce, and the reason this mapping
- `TRUST_FAILURES` — The validation status code that belongs to the trust axis rather

