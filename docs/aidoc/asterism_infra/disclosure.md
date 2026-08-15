# asterism-infra::disclosure

Writing a [`DisclosureRecord`] into a file that already exists.

This is the adapter half of AI disclosure. What is asserted is
decided in [`asterism_core::domain::disclosure`];
`asterism-disclosure-format` renders that decision into the two
forms it can take, as values; this module puts them into a file on
disk, in the one order that works, and signs the manifest when — and
only when — a signing identity has been configured.

# Why it happens here rather than inside the generator

Inside a generation graph only tensors move between nodes, so a signed
image fed through a save node is re-encoded from pixels and comes out
with the signature gone. Video is worse: the combine step discards
whatever frame-level record existed, and the manifest can only be
attached after the encode. Signing is therefore possible only in a
layer that holds the finished file, which is this application.

# The order is not a preference

```text
read → XMP packet → C2PA manifest → one write
```

The manifest's hard binding is computed over the file's bytes, and on
a still image the XMP packet is part of them. Writing the packet after
signing therefore invalidates the signature — IPTC's own 2025.1
announcement carries a worked example whose caption records exactly
that outcome, so this is documented behaviour rather than a
deduction. [`DisclosureWriter::apply`] does the two in this order and
[`tests::the_packet_is_written_before_the_manifest_is_signed`] is what
keeps them there.

# What happens with no certificate

The IPTC/XMP half still lands. That is the half platforms read most
widely, it needs no key material, and it is a legitimate disclosure on
its own — IPTC's guidance is to emit the XMP property *or* a C2PA
manifest, with both being better than either.

The manifest half does not, and there is deliberately no fallback to
the test certificates the C2PA tooling ships with. A manifest signed
by them validates as untrusted, which is strictly worse than no
manifest: an absent manifest says nothing, and an untrusted one makes
a provenance claim that a validator actively rejects.

[`SigningIdentity::from_files`] refuses them by the name they carry.
That is a heuristic and a rename defeats it, which is why
[`inspect_certificate`] reads the certificate's own extensions
beside it — though not for the same question. It refuses what cannot
sign at all and reports the rest, because the extended key usage a
test certificate carries is one legitimate certificates carry too:
the two are not told apart by structure.

Neither check decides whether a certificate is *trusted*. That is a
validator's question, asked against a published trust list, and not
one a signer can answer about itself.

# The gap this leaves on video

A still gets its XMP packet either way; MP4 and MOV get nothing
without a certificate. MP4 can carry XMP in a `uuid` box, but writing
BMFF boxes is not something this module does — the manifest path goes
through `c2pa`, which knows the container, and the packet path goes
through `asterism-disclosure-format::embed`, which knows PNG and JPEG. Until
one of those two grows the other's format, an unsigned video export
carries no disclosure at all, and [`Stamped`] says so rather than
reporting a success it did not have.

# Renditions are not signed

The preview rendition path re-encodes through ffmpeg, which produces
a new file carrying none of the original's container tags, stream
tags, manifest or XMP. A rendition therefore cannot inherit a
signature and must not be given one of its own that would claim the
original's provenance for a derived file. Nothing here is wired to
that path; this note is why.

## Functions

- `inspect_certificate` — Reads a signing certificate's own extensions and reports what they

## Types

- `CertificateVerdict` — What reading a certificate's own extensions concluded.
- `Container` — A container this module can write a disclosure into.
- `DisclosureError` — What went wrong applying a record.
- `DisclosureWriter` — Applies disclosure records to files.
- `SigningIdentity` — The certificate and key a manifest is signed with.
- `Strictness` — Whether a certificate no trust list would carry may still sign.

