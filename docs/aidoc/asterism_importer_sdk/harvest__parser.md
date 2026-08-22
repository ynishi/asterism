# asterism-importer-sdk::harvest::parser

[`SourceParser`] impl that decodes `asterism_agent_harvest` JSON
dumps and emits [`Footprint::ChatMessage`] × N.

A message is addressed `<dump>#conversation=<id>/message=<id>`,
both ids as the dump states them. A message with no `id` of its
own is dropped, not numbered by its place in the array — see
[`crate::parser`] for the rule and
[`crate::parser::RecordAddresses`] for the count that goes with it.

## Types

- `HarvestSourceParser` — SourceParser for the `asterism_agent_harvest` canonical schema.

