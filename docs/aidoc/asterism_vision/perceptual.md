# asterism-vision::perceptual

A fingerprint that survives a transform — the cheap half of
duplicate detection.

The three exact axes ask whether two files hold the same bytes, and
they are right to: a digest that widened its equivalence would fold
two different pictures into one, which destroys, where a narrow one
only fails to notice. The reasoning is written out once, beside the
digests it constrains, in
`asterism_core::domain::content_hash`. Nothing here weakens it.

What this module adds is the question those axes cannot ask. A
resized or recompressed copy shares no bytes with its original, so
every exact axis reads it as an unrelated file. A perceptual
fingerprint reads the picture instead — coarsely, deliberately — and
two values that differ in a few bits mean two images that look the
same at a glance.

## This value never enters the duplicate axes

It is not a fourth duplicate axis. Detection walks the axes
strongest-first, stops at the first agreement, writes an
`identical_to` edge, and may enqueue a fold; a claim this
approximate has no business anywhere in that sequence, and
`asterism_core::domain::content_hash::is_duplicate_key` refuses a
value carrying this tag by construction — it tests for the axis's
own prefix, and no axis spells this one.

So the tag exists to say which question a stored value answers, the
way every other digest in this workspace does, and to stay
unmistakable for the ones that answer sameness. It is not a
sub-namespace of any of them: `p1-dhash:` begins with no other tag,
which is what keeps a reader from mistaking it for one.

## Functions

- `distance` — How many of the 128 comparisons two fingerprints disagree on.
- `fingerprint` — The fingerprint as bits — [`None`] for an image with no pixels,
- `of_image` — The fingerprint as a stored value, tag and all — [`None`] for an
- `parse` — Read a stored value back to bits — [`None`] when it carries another

## Constants

- `NEAR_DUPLICATE_DISTANCE` — How far apart two fingerprints may sit and still be proposed as
- `PERCEPTUAL_DIGEST_PREFIX` — Algorithm tag on a stored perceptual value: perceptual definition

