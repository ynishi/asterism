# asterism-disclosure-format 0.0.0

# asterism-disclosure-format

How an AI disclosure is written down: the two renderings of a
[`DisclosureRecord`](asterism_core::domain::disclosure::DisclosureRecord),
as values.

Nothing Asterism exported used to carry a machine-readable
disclosure. The PNG text chunk a generator writes is a reproduction
aid — it is unsigned, it is trivially editable, and it does not map
onto the fields IPTC defines. This crate turns what the core decided
into the bytes that carry it:

| module | what it produces |
|---|---|
| [`xmp`] | the XMP packet, which needs no certificate |
| [`embed`] | that packet inside a PNG or a JPEG, as a byte transform |
| [`manifest`] | the C2PA manifest *definition*, ready for a signer this crate does not have |

## Why the vocabulary is not here

It was, and the split is by reason to change. What may be asserted
about an artefact changes with IPTC and with what this library can
establish — that is domain, and it lives in
[`asterism_core::domain::disclosure`]. How a packet is written into a
PNG changes with the container, which is this crate. Keeping both
here put a chunk walker and a CRC into the core's dependency graph,
the parser that crate's own manifest records evicting, and it left
nowhere in the core to model reading a disclosure *back*.

## Why an exporter cannot do this on its own

Inside a generator only tensors move between nodes: running a signed
image through a save node re-encodes it from pixels and strips the
signature. Video is worse — the encode discards whatever frame-level
record existed, so a manifest can only be attached after it. Signing
is therefore only possible in a layer that holds the file *after* it
has been written, which is this application rather than a plug-in
inside the generator.

## Two vocabularies, not one

Both an IPTC XMP property and a C2PA manifest are emitted, and that
is not redundancy. Platforms read inconsistently — some parse
`Iptc4xmpExt:DigitalSourceType` and ignore C2PA, others do the
reverse — and IPTC's own recommendation is to emit both. They also
fail differently: XMP survives an ordinary metadata-preserving copy
and is editable by anyone, while a manifest is tamper-evident and is
removed by any re-encode.

## The ordering constraint

**XMP is written before the manifest is signed.** The manifest's hard
binding covers the XMP packet, so editing the packet afterwards
invalidates the signature — IPTC's own 2025.1 announcement carries a
worked example whose caption records exactly that happening. The two
emitters are one ordered operation, not two independent ones.

## What this crate refuses to do

- **Sign.** No key material, no cryptography backend, no `c2pa`
  dependency. Signing lives in `asterism-infra`, where a certificate
  is configuration rather than something this repository ships.
- **Decide.** Whether a file is synthetic, whether a prompt may be
  disclosed, which parent is which — those are read out of the
  database by the application service and arrive here already
  settled
  ([`DisclosureRecord`](asterism_core::domain::disclosure::DisclosureRecord)).
- **Decode pixels.** Every writer here is a byte transform over a
  container. An export hands back the image it was given.

## What it does not cover yet

- **A watermark.** The industry pairs an invisible watermark with the
  manifest precisely because the manifest does not survive a
  re-encode. A watermark changes pixels, which is a different
  contract from the one above, and it is tracked separately.
- **ExtendedXMP.** A packet too large for a JPEG segment falls back
  to a reduced record rather than being split ([`embed::stamp`]).
- **Video containers.** [`embed`] writes into PNG and JPEG only. MP4
  and MOV carry their manifest as a JUMBF box, which the signing
  adapter writes through the `c2pa` crate; there is no XMP half for
  them here.

## Modules

- [`embed`](embed.md): Putting an XMP packet into a container, and taking the old one out.
- [`manifest`](manifest.md): The C2PA manifest *definition* — what would be signed, built as a
- [`xmp`](xmp.md): The XMP packet — the half of the disclosure that needs no

