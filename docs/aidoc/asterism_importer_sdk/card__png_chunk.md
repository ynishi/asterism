# asterism-importer-sdk::card::png_chunk

Character-card PNG `tEXt` chunk decoders.

Two chunk keywords are canonical:
[`CHARA_KEYWORD`] (V2, base64 UTF-8 JSON) and [`CCV3_KEYWORD`] (V3,
same encoding). On read prefer `ccv3` when both are present; the
PNG importer or a character-card CLI uses
[`envelope_from_chunk`] to lift each chunk value into a
[`CardEnvelope`].

Chunk framing comes from `pngmeta`: where one chunk ends and the
next begins has a single right answer, and a card reader is the
wrong place to re-derive it. What stays here is the card-specific
half — which keyword wins, and how a chunk value becomes an
envelope.

A card is the one thing a PNG's `tEXt` chunks are read for here.
**Not** metadata in general: an ordinary image's chunks are that
image's own metadata and are hashed off its bytes server-side on the
`Meta` axis (`asterism-core::domain::material_meta`), with no reader
on this side at all. A character card is a different claim — the
chunk carries an envelope whose slots are separately addressable
records — and [`envelope_from_chunk`] is where that claim is made.

## Functions

- `envelope_from_chunk` — Decode a `chara` or `ccv3` chunk value into a [`CardEnvelope`].
- `envelope_from_png` — Walk a PNG and lift the character card out of it.

## Constants

- `CCV3_KEYWORD` — PNG `tEXt` chunk keyword carrying a V3 character card.
- `CHARA_KEYWORD` — PNG `tEXt` chunk keyword carrying a V2 character card.

