# asterism-importer-sdk::bundle

Deriving the grouping key that ties the footprints of one container
together.

A parser handed one `RawItem` that really is many records — a
character card's slots, a harvest envelope's conversations — needs a
key every one of them can carry so `edge_rebuild` draws a
`same-bundle` edge across the set. The key has to be **derived from
the container's locator**: a random one would move on every
re-import and split a set that arrived twice into two.

# Why this is not in `png_text` any more

It used to be. That module read a PNG's `tEXt` chunks and emitted one
`Footprint::Note` per chunk, and this function existed to bundle
those notes back to the image they came out of. The notes are gone —
the text inside an image is that image's metadata rather than a
record of its own, and it now travels on the image's row as the
`Meta` axis. What survived
is this derivation, and it never had anything to do with PNG text:
its callers are the card parser and the harvest parser, both of them
over real containers.

**The namespace bytes are unchanged**, so every id this has ever
produced still comes back the same — a card imported before the move
keeps its bundle.

## Functions

- `session_id_for` — Derives the shared grouping key for every footprint decomposed from

