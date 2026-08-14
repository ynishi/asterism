# asterism-importer-sdk::runner

Shared importer execution pipeline.

CLI/config parsing stays in the outer binary. This module receives
resolved values plus a scanner/parser pair and owns the mechanical
scan → parse → batch → HTTP → progress loop.

# Where a declared digest comes from

[`AssetSpec::declared_content_hash`](crate::AssetSpec::declared_content_hash)
is filled in here rather than in any parser, because this is the one
place that can see both halves of the question at once: the scanner
says whether its payload is a whole artefact
([`SourceScanner::payload_is_whole_artefact`]), and the spec says
whether the record still lives at the address the scanner read. Only
when both hold do the bytes in hand belong to the locator being
registered, and only then is a digest a true statement about the
file the server will later open.

A parser could not decide this on its own: it is handed the payload
and hands back footprints, and whether those footprints kept the
item's address or split it into records inside the item is visible
only after the mapping — one Claude Code session file yields
messages addressed `<file>#<uuid>`, and one PNG yields itself.

## Functions

- `run_import` — (no documentation)

## Types

- `ImportOptions` — (no documentation)
- `ImportSummary` — (no documentation)

