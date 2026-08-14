# asterism-core::domain::content_hash

`content_hash` — the fingerprint of an original artefact's bytes.

Asterism's identity for an asset is its `locator`
(`UNIQUE(source_kind, source_locator)`), which answers "have I seen
this *path* before" and nothing else. The same photograph copied
into two folders is two assets, rated twice, tagged twice, and
shown twice in the grid — the duplicate problem every library tool
eventually grows a answer for.

This module is that answer's first stage: a digest of the file's
bytes, stored on the material, so "the same picture" becomes a
question SQL can group by.

# Not the snapshot hash

[`snapshot_hash`](crate::domain::snapshot_hash) also produces a
SHA-256 hex string and means something entirely different: it
fingerprints an *ordered list of asset ids*, never a byte on disk.
The two must not be confused when reading a schema — a
`content_hash` column on `snapshot` is a member set, the one on
`material` is a file.

# Why the algorithm is in the value

Stored values carry a `sha256:` prefix. Exact-byte matching is the
cheap half of duplicate detection; the useful half is perceptual
(a re-encoded or resized copy of the same photograph), and that
wants a different algorithm rather than a different column. A
prefixed value lets a later pHash / embedding land beside this one
and lets a reader tell at a glance which kind of "same" a row is
claiming.

## Functions

- `axis_of` — Which question a stored value answers, read off its tag — `None`
- `declaration_claim` — The note recorded at registration: the claim, and nothing else.
- `declaration_verdict` — The note the hash job writes once it has read the bytes: the same
- `digest_prefix` — The algorithm tag values on `axis` carry — the half of
- `is_axis_answer` — Whether a **versioned** column holds an answer — a digest of the
- `is_duplicate_key` — Whether a stored value may stand for "the same picture" — the rule
- `needs_content_walk` — Whether one material is still owed the **data migration** that
- `needs_fingerprint` — Whether one material still owes a fingerprint pass — **the** rule,
- `parse_declaration` — Reads a caller's **declaration** about the bytes it is registering
- `reserved_values` — The values on `axis` that carry its prefix and still do not stand

## Constants

- `CONTENT_DIGEST_PREFIX` — Algorithm tag of the *content* axis — the digest of only those bytes
- `CONTENT_REGION_EMPTY` — The content-axis digest of a region with no bytes in it — the
- `CONTENT_RESERVED_VALUES` — [`RESERVED_VALUES`] for the content axis.
- `DECLARED_HASH_NOTE_KEY` — Key under which a declaration and its verdict live on the asset's
- `EMPTY` — The digest of zero bytes — the one fingerprint every empty file
- `META_DIGEST_PREFIX` — Algorithm tag of the **meta** axis — the digest over the metadata a
- `META_EMPTY` — The meta-axis digest of the empty rendering (`{}`) — the [`EMPTY`]
- `META_RESERVED_VALUES` — [`RESERVED_VALUES`] for the meta axis.
- `RESERVED_VALUES` — The values a hash column holds that stand for something other than
- `UNHASHABLE` — Tag on the value stored for a material that can never have a

