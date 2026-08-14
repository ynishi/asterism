# asterism-core::application_support::chapter_intake

What a fresh reading of a material's chapter list means for the
rows — the machine half of the layer model.

A container declares how its content is divided, and that
declaration can change: the file is replaced, a better parser lands,
a codec's chapters were unreadable last time. So "read the chapters
again" has to be an operation that can run any number of times and
leave the same state, without touching anything a person wrote.
[`replace_imported_chapters`] is that operation.

Functions rather than a service, for the reason
[`duplicate_detection`](crate::application_support::duplicate_detection)
gives beside it: the job handler that will drive this already holds
both ports it names, so a struct would add a handle without adding a
decision. It lives here rather than in
[`MaterialLayerService`](crate::application::MaterialLayerService)
because only a job drives it — the split this module doc states.

# Why replacement, not merge

A merge needs an identity for "the same chapter across two
readings", and containers do not offer one: MP4's `chpl` numbers its
entries by position, Matroska's `ChapterAtom` carries a UID that is
only unique within the file it came from, and a re-encode changes
both. Matching on `(start_ms, label)` would treat a shifted timestamp
as a new chapter and a re-titled one as a deletion — so a merge
would be a guess presented as a reconciliation. Replacement makes
the imported band exactly what the file says, which is the only
claim it was ever making.

The person's own band is untouched by construction: it is a
different row of `material_layer`, and this function names the
imported one.

## Functions

- `replace_imported_chapters` — Makes the imported chapter band of `(asset_id, material_ord)` say

## Types

- `ScannedChapter` — One section as a probe read it out of the material.

