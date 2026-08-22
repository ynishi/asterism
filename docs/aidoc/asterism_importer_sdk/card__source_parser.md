# asterism-importer-sdk::card::source_parser

[`SourceParser`] adapter that turns any character-card [`RawItem`]
(PNG tEXt or standalone JSON) into per-slot [`Footprint`]s.

Composes the pipeline:
[`RawItem`] → [`envelope_from_png`] / [`CardEnvelope::from_json`] →
[`CardParserRegistry::dispatch`] → `Vec<Footprint>`.

Importer binaries plug this straight into an [`FsScanner`](crate::FsScanner)
so they only need to configure the source_kind slug and the batch
posting loop.

## Types

- `CharaSourceParser` — [`SourceParser`] that decodes character cards from PNG tEXt chunks

