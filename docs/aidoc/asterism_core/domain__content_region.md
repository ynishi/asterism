# asterism-core::domain::content_region

`content_region` — what a reading of "the bytes that decide what
this artefact decodes to" can conclude, and what each conclusion is
stored as.

[`content_hash`](crate::domain::content_hash) fingerprints a whole
file, which answers "are these the same file". Two exports of one
picture that differ only in a `tEXt` chunk — the ComfyUI workflow
blob, an exporter's timestamp, a caption written on re-save — are
not the same file and get different digests, while being the same
picture pixel for pixel. The content axis answers that second
question, under the separate, versioned tag
[`CONTENT_DIGEST_PREFIX`](crate::domain::content_hash::CONTENT_DIGEST_PREFIX).

# Vocabulary here, container reading elsewhere

**This module holds no format knowledge and reads no bytes.** The
reading is a container parser over input an importer collected from
outside, with the failure modes a parser has, and it is one
implementation per format — so it lives behind
[`probe::ArtefactProbe`](crate::domain::probe::ArtefactProbe), with
the byte-level walking in `asterism-media-probe` and each format's
judgement about its own container in the adapters beside it. What is
left here is the part every format's answer has to be phrased in:
three outcomes, four reserved markers, and the rule for labelling a
format nothing walked.

The split is what keeps a second format from widening the domain
layer. Adding one adds a probe and a registry line; the words a
column can carry do not move, which matters because they are already
written into a live database.

[`content_hash`](crate::domain::content_hash) is the neighbouring
vocabulary — which prefixes exist, which values are reserved, what a
caller's declaration is allowed to say — and the prefix itself is
defined once, over there, and imported.

# "No digest" is not one state

[`ContentRegion`] has three, and the caller cannot collapse them by
accident because there is no `Option` to unwrap. A file no probe
handles is [`ContentRegion::Unsupported`] and falls back to the file
axis, which still works on it. A file some probe does handle but
could not read to a region is [`ContentRegion::EmptySpan`], and the
distinction matters more than it looks: hashing a region of zero
bytes produces a perfectly real digest — the well-known SHA-256 of
nothing — and writing it would put every truncated PNG in one
duplicate group, each unrelated to the next. Measured on the mp4
side, where fragmented files walk to zero samples and produced
exactly that collision.

## Functions

- `unsupported_format` — The outcome for an artefact that is **not** going to be read — the

## Types

- `ContentRegion` — What a reading of an artefact's bytes concluded.

## Constants

- `EMPTY_SPAN` — Value stored when a probe claimed the format and its reading yielded
- `NOT_WALKED` — Value written to the content column of every material that existed
- `TOO_LARGE` — Value stored when the format *is* one a probe walks and the job
- `UNKNOWN_FORMAT` — Format label used when a probe refused the bytes and nothing named
- `UNSUPPORTED_PREFIX` — Tag on every value that says "no digest on this axis, use the file

