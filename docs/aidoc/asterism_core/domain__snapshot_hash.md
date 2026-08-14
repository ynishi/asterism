# asterism-core::domain::snapshot_hash

Snapshot `content_hash` — the canonical member-set fingerprint.

A Snapshot is a pure content object (a git-tree
analogue): `UNIQUE(persona_id, content_hash)` is the
whole dedupe story, so every producer (the V19 migration backfill,
the dispatch freeze, the promote path, the re-dispatch reuse lookup)
must derive the hash from the member list the same way. This module
is that single definition.

# Canonical form

SHA-256 over the **ordered** member asset ids in their wire string
form (lowercase hyphenated UUID), each followed by a `\n` separator,
hex-encoded lowercase. Order is part of the identity — the same set
in a different order is a different snapshot, because `position` is
frozen content.

## Functions

- `content_hash` — Hashes the ordered member ids into the snapshot `content_hash`.

