# asterism-importer-sdk::card::parser

[`CharacterCardParser`] trait + canonical V2 slot logic.

The trait has six per-slot hooks whose defaults cover the V2 spec.
Derivatives override individual hooks — see [`crate::card::v3`] for
the reference example.

The V2 slot logic is also exposed as free functions in
[`v2_default`] so derivatives can chain (e.g. V3
[`crate::card::v3::V3Parser`] calls [`v2_default::creator_notes`]
and appends multilingual variants).

## Traits

- `CharacterCardParser` — Extension trait for character-card decomposition.

