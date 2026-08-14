# asterism-provenance::xmp

The XMP packet — the half of the disclosure that needs no
certificate.

IPTC Extension properties can only be carried in XMP; there is no
EXIF or IIM spelling of them. So this packet is the entire IPTC side
of the acceptance criteria, and it is the side that works today,
unsigned, on a machine with no signing identity configured.

# Why this is written by hand rather than by an RDF library

An XMP packet is RDF/XML, and a general RDF serialiser would be
entitled to emit any of several equivalent forms — properties as
attributes instead of elements, a different namespace prefix, a
different node ordering. Equivalent to a parser, not equivalent here:
the bytes go inside a C2PA hard binding (see the module docs on
ordering), so *which* equivalent form is produced decides whether a
signature made over one rendering still verifies against another. A
packet this crate writes has to be a function of the record and
nothing else, which is what a hand-written template is and a
serialiser is not obliged to be.

The scope also does not justify a dependency: five properties, all
simple text or a URI, all in one namespace, none of them a container
(no `rdf:Alt` / `rdf:Bag` / `rdf:Seq`), no language alternatives.

# Ordering against the C2PA manifest

**The packet is written before the manifest is signed, never after.**
The manifest's hard binding covers the XMP, so editing the packet
afterwards invalidates the signature. This is not a deduction: IPTC's
own 2025.1 announcement carries a worked example whose caption
records that adding the new AI properties invalidated the C2PA
metadata that was already in the file. The apply path enforces the
order, and its tests are what hold it.

# No padding

The XMP specification suggests trailing whitespace so a packet can be
rewritten in place without moving the bytes after it. This writes
none, and marks the packet read-only (`<?xpacket end="r"?>`)
accordingly. In-place rewriting is not something that can happen to a
signed file anyway — any edit breaks the binding — and JPEG's APP1
segment leaves the packet 65,504 bytes, which is a budget a generator
prompt can exhaust on its own. Spending part of it on padding for an
update path that cannot exist would be paying twice.

(65,533 is the segment's payload, not the packet's: the 29-byte
`http://ns.adobe.com/xap/1.0/\0` identifier is inside it and has to
be paid first. The figure is
[`embed::JPEG_MAX_PACKET`](crate::embed::JPEG_MAX_PACKET), which is
what the writer enforces and what
[`EmbedError::PacketTooLarge`](crate::embed::EmbedError::PacketTooLarge)
reports.)

## Functions

- `render` — Renders the record's IPTC properties as an XMP packet.

## Constants

- `IPTC_EXT_NS` — The IPTC Extension namespace, unchanged since 2008 and still the

