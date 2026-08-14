# asterism-core::domain::chapter_mark

`ChapterMark` — one entry in a chapter list: a named section of an
Asset's material.

A container declares how its content is divided (MP4's `chpl` and
`chap`, Matroska's `Chapters` segment, an MP3's ffmetadata
`[CHAPTER]` blocks). That declaration is data the file carries, and
before this type it had nowhere to land: an import either threw it
away or flattened it into free-text notes, where it was
indistinguishable from something a person had written.

A chapter belongs to a [`MaterialLayer`](crate::domain::material_layer)
with role `Structure`, not to the asset. The layer is what says
whether this list is the file's own, a person's, or a job's — which
is the whole reason re-reading a file can replace one list without
touching another.

Not a [`MaterialMark`](crate::domain::material_mark). A mark is a
note fastened to a position ("look at this"); a chapter is a claim
about how the material is *divided* ("this section starts here").
They share [`TimelineSpan`] and nothing else: the two carry
different fields, answer to different layer roles, and the rules
below differ from that aggregate's precisely where the difference in
meaning is.

## Types

- `ChapterMark` — One named section of a material.

