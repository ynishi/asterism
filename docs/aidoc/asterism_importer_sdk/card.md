# asterism-importer-sdk::card

# Character-card parser subsystem

Trait-based decomposition of character-card family formats
(SillyTavern V2 / V3, CharacterHub, RisuAI, AgnAI, KoboldAI) into
per-slot [`crate::Footprint`]s. This is the "one card → many
footprints" pipeline design axiom from [`crate::catalogue`] in
executable form.

## Layers

- [`envelope`] — the parsed wire shape: [`CardEnvelope`] holds
  `{spec, spec_version, data{…}}` and [`CardContext`] carries the
  caller-supplied ingest metadata (source_kind, locator,
  session_id, occurred_at, platform).
- [`png_chunk`] — base64 UTF-8 JSON decoder for the PNG text chunks
  `chara` (V2) and `ccv3` (V3). Feeds an envelope back to the
  parser; chunk framing is `pngmeta`'s.
- [`parser`] — the extension trait [`CharacterCardParser`] and the
  canonical V2 slot logic exposed as free functions
  ([`parser::v2_default`]) so derivatives can chain rather than
  re-implement.
- [`registry`] — [`CardParserRegistry`] dispatches an envelope to
  the right parser by its `spec` string; pre-loaded with [`V2Parser`]
  + [`V3Parser`].

## Extension pattern

```ignore
use asterism_importer_sdk::card::{
    CardContext, CardEnvelope, CardParserRegistry, CharacterCardParser,
};

struct ChubParser;
impl CharacterCardParser for ChubParser {
    fn spec(&self) -> &'static str { "chara_card_v2" /* chub piggybacks on V2 */ }
    fn parse_extras(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<_> {
        // Promote data.extensions.chub.alt_expressions[] → Image × N here
        Vec::new()
    }
}
```

## Session id

Every footprint decomposed from one card shares one `session_id`
(typically [`crate::bundle::session_id_for`] of the container
locator), so `edge_rebuild` clusters them via
`time_proximity = 1.0`.

