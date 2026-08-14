# asterism-core::domain::material_meta

`material_meta` — the canonical form the metadata a container
carries *about* an artefact is rendered into, the digest taken over
that form, and what a reading of it can conclude.

[`content_region`](crate::domain::content_region) is defined as the
bytes that survive into the decoded result, so **the metadata it
drops is the exact complement**, and until this module nothing
hashed that complement. The three axes stand in one relation:

```text
   Artefact  =  Content  +  Meta
```

`Artefact` agreement implies both of the others; neither of the
others implies anything about the rest. That is what makes this axis
worth carrying rather than folding into one of them — a generator
emits both shapes routinely. One picture re-exported with a caption
written into it is `Content` and not `Artefact`; a batch off one
workflow whose frames differ only by seed is `Meta` and not
`Content`.

# The canonical form

A **JSON object, key → value**, keys sorted, no whitespace — the
same rendering discipline
[`SourceLocator::to_storage`](crate::domain::source_locator::SourceLocator::to_storage)
follows, and for the same reason: the digest is an equality test on
the rendered string, so two equal metadata sets must render
identically. `serde_json` over a [`BTreeMap`] emits keys in order and
compact by default, so both properties are facts about the
declaration rather than about a `format!` string. **Nothing here
hand-writes JSON.**

Two rules live inside the form, and both decide what the digest
*means*.

## Values stay as the container stated them — strings, unparsed

A ComfyUI `parameters` chunk happens to hold JSON. Parsing it in
order to re-render it would put number formatting and nested key
order into the digest's definition, so two files the container calls
identical could stop matching on a serialiser's habits. The digest
says one thing: *the container carried this text*. The type is what
enforces it — the map is `BTreeMap<String, String>` end to end, and
there is no `from_str` anywhere on this path.

If that proves too strict — the same workflow re-saved by a tool
that reformats — the answer is a **new prefix**, and it is written
down beside the prefix itself
([`META_DIGEST_PREFIX`](crate::domain::content_hash::META_DIGEST_PREFIX)).

## Album's own fields never enter it

Title, labels, `register_note`, ratings: those are what a person
wrote *here*, and a digest that moved when somebody renamed a
picture would be measuring the library rather than the artefact.
Structurally enforced by the input:
[`ArtefactProbe::meta_of`](crate::domain::probe::ArtefactProbe::meta_of)
takes the artefact's bytes and nothing else, so no library-side value
has a route in.

# A digest is the entrance, not the body

Exact equality is the wrong question for metadata on its own: a
batch off one workflow differs by a seed, and a digest over the
whole of it separates precisely the rows that belong together. The
hash answers "made identically" cheaply and indexably; the useful
question — "made the same way apart from *this*" — is a comparison
over the structured value. So both are stored: the digest is the
index, and [`MaterialMeta::canonical`] — the same bytes that were
hashed — is what a person reads and a field comparison walks.

# Which containers are read, and how, is not decided here

This module holds the form and the digest; the reading of any
particular container is one implementation per format behind
[`ArtefactProbe`](crate::domain::probe::ArtefactProbe). That includes
the questions a reader cannot avoid answering — which chunks or boxes
count as metadata at all, and how each decodes to a string — because
answering them carelessly redefines the axis rather than widening it,
and the argument for a given answer is an argument about a specific
container. A format no probe reads gets
[`ContentRegion`]-shaped markers rather than a digest, and falls back
to the artefact axis, which still works on it.

[`ContentRegion`]: crate::domain::content_region::ContentRegion
[`BTreeMap`]: std::collections::BTreeMap

## Functions

- `digest_of` — The digest of an already-rendered canonical form.
- `render` — Renders a metadata set into the canonical form — **the only place
- `unsupported_format` — The outcome for an artefact that is **not** going to be read — the

## Types

- `MaterialMeta` — What a reading of an artefact's metadata concluded.

