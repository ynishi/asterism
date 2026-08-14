# asterism-core::domain::probe

`probe` — the port a format's identity measurement is written
against.

Two of the three fingerprint axes are format-specific.
[`content_hash`](crate::domain::content_hash) streams a whole file and
works on anything; the other two have to know what the container is,
because the question they answer — which bytes decide what this
decodes to, and which are notes written *about* it — has no answer
that is true of every file. So the vocabulary of the answers lives in
[`content_region`](crate::domain::content_region) and
[`material_meta`](crate::domain::material_meta), and the reading of
any particular container lives behind this trait.

# Why a port and not a `match`

The walkers used to be functions in the domain layer with the format
written into them, and the gate that routed a file to them was a
`matches!` on the declared mime — one copy per axis. Adding a second
format meant widening two `matches!` arms and adding a second parser,
several hundred lines of untrusted-input handling, to a layer whose
whole claim is that it has no I/O and no format knowledge. The parser
is the part that grows per format, and it is the part that belongs
furthest from here.

What is left in the domain is the vocabulary the columns carry and
this trait. An implementation is an adapter: it holds a format's
judgement about its own container and produces domain values. Adding
a format adds an implementation and a line in whatever registry the
adapter layer keeps — the probe, not a second list of the formats it
answers for, which the probe states itself
([`ArtefactProbe::declares`]); nothing in this crate is edited.

That is what it costs to read the *next* file, and it is not the
whole cost of the format arriving. Every artefact already in the
library carries `unsupported:<mime>` on both walking axes, and a
marker is a final answer to "has anybody looked"
([`is_axis_answer`](crate::domain::content_hash::is_axis_answer)), so
[`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint)
passes those rows over for good. Files imported after the probe lands
would take digests while the ones that were there first keep the
marker, and the column would hold two meanings with nothing to tell
them apart. So a format also arrives owing a way back to the rows it
was refused on, **and it owes one per axis**: a probe may claim the
second axis a slice after the first, and the column it was refused on
stays refused until something says otherwise.

The cheap way to say it is a numbered `UPDATE` writing NULL over that
one marker, which hands the rows back to the ordinary walk — what
both of JPEG's axes took, one step each.
[`needs_content_walk`](crate::domain::content_hash::needs_content_walk)
is the other shape — a second predicate, selecting one marker, driven
by a migration-chain read — and it stays reserved for the case it was
written for. It reads the content column only, so there is no
equivalent predicate on the meta axis and none has been needed.

## Types

- `FormatClaim` — One format a probe answers for, and the axes it answers on.
- `GateOpen` — That the gate for one axis was asked, and answered `true`.

## Traits

- `ArtefactProbe` — One format's reading of an artefact's identity.
- `ProbeGates` — Everything a caller may ask a probe: the two gates, read off its

