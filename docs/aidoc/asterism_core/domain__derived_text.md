# asterism-core::domain::derived_text

Derived text — the one string an asset offers a full-text index,
assembled from everything the row already says about itself.

# The gap this fills

Until this module existed, "what does search see" had exactly one
answer: the bytes of the original, when those bytes were text. A
conversation transcript was searchable and a picture was not, and
the picture was not searchable *even though the library already
held sentences about it* — a title somebody typed, the alt text an
importer lifted out of the page it came from, the keywords the
auto-tag pass wrote, the generation prompt sitting in a PNG `tEXt`
chunk, a note a person left in the comment thread. Every one of
those is text, on the row, already stored. None of them reached the
index, so the honest description of the search surface was "text
files only", and a library that is mostly pictures had mostly
nothing to search.

The fix is not a new store. It is to stop treating "the body" as a
synonym for "the file's bytes": the body an index wants is a
*projection* of the asset, and the file is one field of it.

# Why a pure function

Composition happens here, in the domain, and not in the job that
writes the index, for the ordinary reason — the rule for what is
searchable about an asset is a statement about assets, and a rule
that lives inside an infrastructure handler can only be tested by
standing up a queue. The handler's job is to *fetch* (the file, the
comment thread) and to *write* (the body cache, the two indexes);
deciding what the text is belongs to the entity that has the
fields.

The two inputs the asset cannot supply itself are arguments rather
than ports, for the same reason: `file_body` lives behind a reader
of the outside world and `comment_bodies` behind a second
aggregate's repository. Handing them in keeps this function total,
synchronous, and free of a trait object.

# What is deliberately left out

- **`_trace` apart from `meta`.** The trace bag holds Album's own
  bookkeeping — provenance claims, a declared hash and its verdict,
  resolution flags. Those are assertions *about the record*, not
  words about the subject, and indexing them would put an internal
  status word in the same haystack as a person's sentence. Only
  [`album_meta::META_KEY`] — the zone whose whole purpose is "a
  statement somebody made, under a name they chose" — is read.
- **The `source` / `operator` / `declared_at_ms` fields of a
  declared-meta entry.** They describe the statement, not its
  content; `manual` appearing in every document is a term that
  matches everything and distinguishes nothing.
- **Identifiers.** No `AssetId`, no locator, no persona id. A UUID
  is not a word, and a locator is already the address the row is
  found by.
- **Tags.** The one exclusion here that is a *judgement* rather than
  a type argument, so it is written down. A tag is already a precise
  instrument: the sidebar filters on it, `tag_counts` counts it, and
  a person who tagged something can get back exactly the set they
  tagged. Full text is the opposite instrument — it exists for the
  words somebody would guess, which is why a picture needs it and a
  tag does not [Furnas et al. 1987: two people choose the same term
  for one thing under 20% of the time, which is the case *for*
  indexing prose and *against* diluting it with a controlled
  vocabulary that already answers exactly]. Folding tags in would
  also make every tag rename a re-composition of every document that
  carried it.

  Reversing this is a decision somebody may make, and it is not a
  one-line change: it needs [`COMPOSITION_VERSION`] raised (so the
  walk re-composes the library) and a re-index wired into all seven
  tag verbs on `AssetService` — `attach_tag`, `detach_tag`, their two
  batch forms, `rename_tag`, `delete_tag` and `merge_tags` — since
  none of them touches the asset row today.

# Ordering

Sections come out in a fixed order — file body, title, cover,
labels, keywords, register note, material metadata (recovered text
then the digest's body), declared meta, comments — so the derived
string is a function of the asset's state
and nothing else. Two runs over an unchanged row produce the same
bytes, which is what makes "the body cache is stale" a decidable
question rather than a diff of two orderings.

Nothing here is a ranking signal: the joined string is one flat
field, so a title does not outweigh a comment. Field-weighted
scoring is a change to the index schema, not to this function.

## Functions

- `derive_text` — Builds the indexable text for one asset, or `None` when the row has

## Constants

- `COMPOSITION_VERSION` — Which reading of an asset a cached body was composed by.

