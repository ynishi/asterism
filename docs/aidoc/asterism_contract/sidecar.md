# asterism-contract::sidecar

The shape of an exported artefact's `<file>.meta.json` sidecar.

Two crates that must not depend on each other need this vocabulary:
an exporter writes the sidecar (adapter side), and the ingest path
reads it back when a returning file declares `derived_from:
sidecar` (core side). Adapters do not depend on the core and the
core does not depend on adapters, so the words live here, in the
leaf DTO crate both already use — the same reasoning that puts
[`DerivedDto`](crate::dto::DerivedDto) here.

Constants rather than a struct: the body is an
[`AssetCardDto`](crate::dto::AssetCardDto) projection (possibly
field-filtered by the caller) with one extra key, and the reader
walks it as JSON. A struct would claim a rigidity the file does not
have — it can be hand-written, truncated by an allowlist, or come
from a version that knew fewer fields.

## Constants

- `SIDECAR_DISPATCH_ID_FIELD` — Field inside the identity block naming the dispatch (the hop).
- `SIDECAR_IDENTITY_KEY` — Key under which a sidecar carries the export's own identity.
- `SIDECAR_SCHEMA` — Version tag written into the identity block.
- `SIDECAR_SUFFIX` — Suffix appended to an artefact's locator to find its sidecar.

