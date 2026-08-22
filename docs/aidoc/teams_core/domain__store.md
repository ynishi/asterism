# teams-core::domain::store

`store` — the team-side view of instance-owned blobs, and the
declared-digest contract (#83 §3).

The store proper is a global CAS the instance owns; what this
module types is the layer everything consults instead of bytes:

- [`TeamBlobLink`] — the visibility AND dedupe boundary. A digest
  "exists" for a caller iff a link row sits in a team they belong
  to; purge scope is a team's links, never the CAS directly.
- [`Locator`] — a *private* link to content the instance does not
  hold: a uri, and at most a digest **hint** from the last
  sighting. The instance guarantees nothing about it.
- The promotion verification rule — [`verify_declared_digest`],
  whose two outcomes are the whole point of this module.

# Digest notation is `asterism-core`'s, reused as-is

Every digest here is the `sha256:`-prefixed storage form defined by
`asterism_core::domain::content_hash` (and the contract crate under
it). [`parse_digest`] delegates to that parser rather than
re-spelling the grammar — one notation, one set of shape rules,
across the local app and the teams plane (#83 §3: "core digest
reused, `sha256:` prefix kept; strip only at the path-mapping
edge").

## Functions

- `parse_digest` — Validates a whole-file digest in the shared notation and hands back
- `verify_declared_digest` — The declared-digest verification rule (#83 §3), with exactly two

## Types

- `DeclaredDigest` — The digest a promotion **declares** — mandatory, computed by the
- `Locator` — A private-space link: where a user last saw some content, outside
- `TeamBlobLink` — One `(team, digest)` row — the boundary that makes a blob visible.
- `VerifiedCopy` — Proof that a promotion's bytes hashed to what was declared — the

