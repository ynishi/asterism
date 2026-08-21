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

# Select or re-render: a new digest has to say which

Every digest in this workspace has to be a function of content
rather than of the way somebody happened to write that content
down, and there are only two routes to it.

A digest that **selects** feeds the artefact's own bytes to the
hash — a byte range, a chunk, a string exactly as the container
stated it — and inherits its stability from the file. A digest that
**re-renders** parses the artefact and serialises the result. That
buys insensitivity to formatting, and pays for it by taking the
serialiser's habits into the definition: key order, number spelling,
string escaping, and what to do about a duplicate key all become
part of what the value means.

Neither route is wrong, and neither is free. What is wrong is
leaving the choice unsaid, because the two fail in opposite
directions and the directions do not cost the same. A re-rendering
digest that widens an equivalence too far reports two different
artefacts as one, and duplicate resolution acts on that by folding
them — a wrong answer that destroys. A selecting digest that is too
narrow only fails to notice a match, which costs a row.

So a digest that lands here owes three things: which of the two it
is; the canonical form written out in full if it re-renders — naming
a published scheme is not enough on its own, because the rule for
numbers and the rule for duplicate keys are the parts that decide
the answers; and a versioned tag, because a definition that has been
stored cannot be edited afterwards without changing what every value
written under it meant. [`META_DIGEST_PREFIX`] is that trade being
made deliberately for the meta axis: it selects, because
re-rendering a ComfyUI `prompt` graph would put a serialiser's
number formatting between two files the container itself calls
identical.

## Functions

- `awaits_fingerprint` — Whether one material still counts toward the "still fingerprinting"
- `axis_of` — Which question a stored value answers, read off its tag — `None`
- `axis_open_work` — Whether one axis is **open work** — the progress side of the split
- `declaration_claim` — The note recorded at registration: the claim, and nothing else.
- `declaration_verdict` — The note the hash job writes once it has read the bytes: the same
- `digest_prefix` — The algorithm tag values on `axis` carry — the half of
- `fingerprint_unreadable` — Whether one material is stuck on an unreadable original: the walk
- `is_axis_answer` — Whether one axis holds a **final answer** — the rule that decides
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
- `UNHASHABLE` — The **legacy stored spelling** of "this material can never have a

