# asterism-core::domain::provenance::source

`_trace.source` vocabulary — which channel a provenance claim
arrived through. A *bookkeeping of origin*, not a trust ranking:
caller-trust is the ingest regime, and hardening belongs to the
transport layer if it ever comes.

The value is derived structurally from where the claim entered, so
no caller asserts it:

| value | channel |
|---|---|
| `embedded` | dug out of the artefact's own surroundings — an ingest-time `sidecar` claim the importer detected next to the file |
| `pushed` | reported with the payload at ingest time — an `asset:` / `dispatch:` claim carried on `AddAssetCommand.derived_from` by whoever ran the chain |
| `manual` | declared after the fact through `DeclareProvenanceCommand` (`POST /assets/{id}/provenance`), regardless of form |

## Constants

- `EMBEDDED` — Claim detected in the artefact's own surroundings at ingest.
- `MANUAL` — Claim declared after the fact on an existing asset.
- `PUSHED` — Claim pushed with the ingest payload by the caller.

