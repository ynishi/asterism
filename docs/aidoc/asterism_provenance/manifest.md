# asterism-provenance::manifest

The C2PA manifest *definition* — what would be signed, built as a
value, with nothing here able to sign it.

`c2pa::Builder::with_definition` takes a JSON document describing the
manifest; producing that document is a mapping problem (what the
database holds → what the manifest asserts) and producing a
*signature* is a key-management problem. They are separated here so
the mapping can be tested exhaustively on a machine with no
certificate, which is every machine this repository has today.

# Two assertions, and why the second one exists

`c2pa.actions` carries the standard claim: this asset was created,
and its `digitalSourceType` is the same IPTC URI the XMP packet
states. That is the half a validator understands.

`io.github.ynishi.asterism.provenance` carries what the database
knows and the standard has no field for: the asset id, the dispatch
the file left through, and the ids it was derived from. A reader that
has this Asterism instance can resolve those; a reader that does not
at least learns that the lineage exists and is recorded somewhere.

The label is reverse-DNS under a domain that resolves to the author,
which is the convention the C2PA specification asks third-party
assertions to follow. A label invented outside a controlled namespace
is one that can collide with somebody else's meaning of the same
words.

# Why the parents are ids and not ingredients

C2PA has a first-class way to say "this was made from that": an
ingredient, which carries the parent's own hash and, where it has
one, its manifest. That is a stronger statement than an id, and it is
one this path cannot honestly make — signing happens over a file that
has been exported, and the parents' bytes are not in hand at that
moment (they may have been purged, and re-reading them would make an
export's cost depend on the depth of its lineage). An ingredient
constructed without the parent's bytes would be an assertion about a
hash nobody computed.

So the ids go in the custom assertion, where they are what they are:
a pointer into the library that recorded the edge. Promoting them to
ingredients is a later change that needs the parent files, not a
detail left out.

## Functions

- `definition` — Renders the manifest definition for one record.

## Constants

- `ACTIONS_LABEL` — Label of the standard actions assertion.
- `ASTERISM_ASSERTION_SCHEMA` — Version of the payload under [`ASTERISM_LABEL`].
- `ASTERISM_LABEL` — Label of Asterism's own assertion (module docs on the namespace).

