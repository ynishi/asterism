# asterism-contract::digest

The notation a digest is written in — the `sha256:` tag, and the
hasher that produces a value carrying it.

# Why the grammar sits in the contract crate

[`AddAssetCommand::declared_content_hash`](crate::command::AddAssetCommand::declared_content_hash)
is a field of this crate, and the grammar of a field's value belongs
with the field: a caller that may state a digest has to be able to
spell one without reaching for anything else.

The caller that needs it most is an importer.
`asterism-importer-sdk` depends on exactly one Asterism crate — this
one — and that is the whole point of it: an importer states where
bytes are and what it found in them, and pointing it at
`asterism-core` would put the entire domain (repositories, services,
duplicate axes) behind a plugin whose job is to read files. The two
alternatives were both worse. A second `sha256:` and a second
`of_bytes` in the SDK is the two-crates-one-predicate shape the
`is_duplicate_error` family had before it was deleted — one rule,
two spellings, kept in step by whoever remembers. An SDK → core
dependency is a layering inversion that no later edit undoes.

There is a verbatim precedent one file away: `chrono` was moved into
this crate's dependencies "so the SDK can drop its self-defined
Derived and the core can consume the shared shape without pulling
the SDK" (`Cargo.toml`). Same shape, same reason, same direction of
travel.

# What deliberately did *not* come with it

Only the notation moved. What a stored value **means** is domain and
stayed in `asterism_core::domain::content_hash`: the markers
(`unhashable:no-bytes`, the `unsupported:` family), the reserved
values, `is_duplicate_key`, which axis a value belongs to, and the
two versioned tags (`cr1-sha256:`, `m1-sha256:`) that the container
walkers produce. Core re-exports the three names below, so nothing
on its side spells a different import than it did before.

An importer can therefore say "these bytes hash to this" and cannot
say "and that makes them a duplicate" — which is the correct
division: a declaration is an unverified assertion, and the rules
that read digests as sameness run where the bytes were actually
measured.

## Functions

- `of_bytes` — Hashes a whole slice — the convenience form for callers that already

## Types

- `ContentHasher` — Incremental hasher over an artefact's bytes.

## Constants

- `DIGEST_PREFIX` — Algorithm tag and separator on every digest this module produces —

