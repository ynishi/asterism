# asterism-core::domain::album_meta

AlbumMeta — what a person or an agent *says about* an asset, in
Album's own words.

# The gap this fills

An asset's `extra` bag already has two populations. Its top level is
what an importer read out of the source — EXIF fields, a camera
model, the raw text of a PNG chunk — facts about the artefact,
reported by whatever was holding it. Underscore-prefixed keys are
Album's own bookkeeping, and `_trace` in particular is where this
library keeps assertions rather than facts (see
[`content_hash::DECLARED_HASH_NOTE_KEY`](crate::domain::content_hash::DECLARED_HASH_NOTE_KEY)
on why a claim does not get a column of its own).

What neither zone had is a place for a statement **Album's user
makes**, about anything, under a name they chose. `_trace` holds
four kinds of statement and Album knows what all four mean; there
was no way to add a fifth without teaching the application about it
first.

# Why not reuse what is already there

- **Tags / labels** are facets: words that put a row in a bucket.
  They carry no record of who said them or through what, so a tag
  cannot say "an agent asserted this at import" as distinct from "a
  person typed it".
- **`extra` top level** is the importer's, and putting a person's
  assertion beside a camera's readings is how a later reader comes
  to treat one as the other.
- **First-class columns** (`title`, `rating`, `register_note`) are
  single slots the application understands. Adding one per thing
  somebody might want to say is not a design.

# Why external identifiers land here rather than becoming keys

The case this was designed against: an artefact arrives carrying
something that *looks* like an identifier — a workflow id, a
generator's own reference, an id minted by hand. It is tempting to
use it as a natural key, and that is the mistake. An identifier is
only an identity if its issuer maintains it as one; most do not.
ComfyUI is the measured example — its embedded graph carries no
identifier for the artefact at all, its node ids are unique only
within one file, and the reference to an input image is a bare
filename. Borrowing uniqueness from a system that is not keeping any
produces a key that silently stops being unique.

So an external identifier is recorded here as **what it is**: a
statement somebody made, with a name they gave it. Asset identity
stays [`AssetId`](crate::domain::value::AssetId), internal and
minted by Album, exactly as it was. Looking rows up by a recorded
value is a *filter* over a secondary index — a different layer, and
deliberately not this one.

# Why there is no verdict

[`declared_hash`](crate::domain::content_hash::DECLARED_HASH_NOTE_KEY)
records a claim and later a verdict, because a job reads the bytes
and can say whether the claim held. Nothing can do that here. A
statement under a name its author invented has no checker, so a
`verified` field would either stay absent forever or be filled in by
something inventing an answer.

## Functions

- `entry` — The recorded statement.
- `parse_key` — Checks a key.
- `parse_value` — Checks a value.

## Constants

- `META_KEY` — Field inside `_trace` that holds every AlbumMeta entry.

