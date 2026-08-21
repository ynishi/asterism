# asterism-core::domain::measurement

`measurement` — what a fingerprint column's *status* can say, now
that the digest columns hold only digests.

The name is the issue's own vocabulary: the whole defect was that a
reader could not tell a **measurement** from a note about why there
is no measurement, and these types are the two halves of that
sentence made storable — [`Measurement`] is one axis's stored
triple, [`MeasurementStatus`] the word that says which of the two
it is holding.

The vocabulary the three hash columns used to carry inline — a
digest, or a marker string explaining why there is none
(`unsupported:<mime>`, `unsupported:empty-span`,
`unhashable:no-bytes`, …) — is split in two. The digest column holds
a digest or NULL; a status column beside it holds one of the words
below, non-nullable; and a reason column holds the free-text part
(the media type, an I/O error) where a status has one.

# Why the marker vocabulary moved out of the value slot

Every reader of the old columns had to know the marker grammar
before it could tell a measurement from a note about why there is no
measurement — `is_duplicate_key`, `is_axis_answer`,
`needs_fingerprint` and `needs_content_walk` all existed to make
that distinction, two of them restated in SQL. The established shape
for the same distinction elsewhere (`getxattr(2)`'s `ENOTSUP` /
`ENODATA` / value) is a nullable payload beside a non-nullable
status, and the marker-in-the-value design was safe only for as long
as the column never crossed an application boundary. Issue #17 is
the record of that decision.

# One vocabulary, three columns

The three axes share one status set for the reason they shared one
marker set: most of these words say something about the artefact
rather than about which measurement was attempted. Which subset a
writer can produce differs — the file axis streams and never walks,
so it only ever says `pending` / `computed` / `no-bytes` / `failed`
— but a reader faces one closed set wherever it looks.

## Types

- `Measurement` — One axis's stored triple: the status, the digest when there is one,
- `MeasurementStatus` — What one fingerprint axis's status column says about the digest

