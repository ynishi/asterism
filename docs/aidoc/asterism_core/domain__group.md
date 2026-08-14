# asterism-core::domain::group

`Group` — a user-curated set of assets, persona-scoped.

Groups are the first primitive we add to the relationship
catalogue beyond `Tag` (auto-labelled channel) and
`ConstellationEdge` (auto-derived similarity). They exist because
Tag alone cannot express **"assets I hand-picked into a bucket"**
— a Tag is an organic label that any auto_tag or importer can
attach; a Group is a deliberate act of curation ("read later",
"mood board for project X", "favorites"). They are also different
from `Session` (auto-grouping key from an import run) — a Group
is user-authored, not derived.

# Naming

Domain type: `Group`.
Persistence table: `bucket` (SQL reserves `GROUP` as a keyword;
quoting the identifier everywhere is fragile so the storage
adapter renames the table). The wire DTO and every API surface
use the `Group` label — only the DB layer sees `bucket`.

## Types

- `Group` — A user-created bucket of assets, scoped to one persona.
- `GroupKind` — Whether a Group's membership is hand-curated or query-defined.
- `GroupLink` — One group-in-group connection — the Are.na "channel connected into
- `GroupSummary` — A group paired with the number of assets currently linked to it,

