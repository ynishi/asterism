# asterism-core::domain::disclosure::record

`DisclosureRecord` — everything one exported file is going to say
about where it came from, decided before either emitter runs.

The two emitters write into different places for different readers.
The XMP packet carries the IPTC properties a platform reads to decide
whether a file is synthetic; the C2PA manifest carries a signed claim
plus the lineage this library holds. They must not disagree, and the
cheapest way to make disagreement impossible is for both to be
rendered from one value that was assembled once.

# What is here and what is deliberately not

This is a *statement*, not a projection of a row. The application
service builds it out of what the database holds — material metadata
for the generator and the prompt, derivation edges for the parents,
attribution for the operator — and every judgement (is this
synthetic, is the parent a photograph, may the prompt be disclosed)
is made there, on the way in. Nothing downstream of this type
re-decides anything: an emitter that had a policy would be a second
place the answer could differ from the first.

# The two audiences, and why the identifiers only reach one

Asterism's own identifiers — the asset id, the dispatch the file left
through, the parents it was made from — go into the C2PA manifest's
custom assertion and **not** into the XMP packet. Two reasons, and
the second is the load-bearing one:

1. IPTC has no property for them. Inventing an `Iptc4xmpExt` field
   would put a private vocabulary into a namespace that is not this
   repository's to extend.
2. The manifest is where lineage belongs by design, and it is signed.
   An id in an unsigned XMP packet is an id anybody can rewrite,
   which makes it useless for exactly the job it would be there for
   — finding the row again — while still being present in every
   published file.

The sidecar the file exporter already writes carries the same
identifiers in the clear for the receiver that has no C2PA reader
(`asterism-contract::sidecar`), so nothing is lost by keeping them
out of the packet.

# Who wrote the prompt is not disclosed

IPTC 2025.1 defines `AIPromptWriterName`, and this record does not
carry it. The property names a person, and IPTC is explicit that the
person who wrote the prompt is not thereby the image's creator —
which is why it has a field of its own rather than riding
`dc:creator`. Nothing in this application states who wrote a prompt:
the prompt reaching a record is read back out of the container the
file arrived in, and a dispatch may run against text written by
somebody else, generated, or rewritten across rounds. Filling the
property from the asset's author or from the operator would assert
something nobody stated, in a file that cannot be taken back once
published — the asymmetry [`PromptDisclosure`] already turns on, and
a name is a stronger claim than the text is.

So the field, its setter argument and the emitter branch are absent
rather than present-and-unreachable. If a surface for stating it ever
exists, this is where it returns, under the same withholding control
the prompt has.

[`PromptDisclosure`]: super::PromptDisclosure

# A human pass is asserted, never inferred

[`DigitalSourceType::HumanEdits`] is the one value a caller has to
state. Nothing observable distinguishes "a person worked on this" from
"no generator metadata was found", and the copyright question that
makes the distinction worth recording turns on the human layer being
evidenced rather than assumed. Deriving it from an absence would
manufacture exactly the evidence it is supposed to record.

## Types

- `DisclosureRecord` — What one exported artefact will disclose.

