# asterism-importer-sdk::card::registry

Registry that dispatches a [`CardEnvelope`] to the right
[`CharacterCardParser`] by its `spec` string.

Pre-loaded defaults, first-registration-wins on collision, an
optional look-up hook, and a `dispatch` that returns `None` when no
parser claims the spec (so the caller can decide between skipping
and falling back to raw V2 defaults).

## Types

- `CardParserRegistry` — Routes envelopes to their parser by exact `spec()` match.

