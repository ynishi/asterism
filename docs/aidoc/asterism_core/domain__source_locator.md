# asterism-core::domain::source_locator

`source_locator` — where an artefact's bytes are, held as a typed
value instead of being sniffed out of a string at every call site.

A locator has four shapes and they are not variations of one string:
a file on this machine, one record inside a container file on this
machine, something a remote scheme addresses, and a caller-minted
name for something that never had bytes. Each shape is its own type
holding its information already taken apart, and
[`SourceLocator`] is the umbrella over them — a sum composed with
`From`, owning no recognition logic beyond the one boundary below.

# Why this is its own module

It is the only code that knows the storage encoding. Keeping that in
one file is what makes the claim checkable: a reader who wants to
know how a locator is spelled on disk reads
[`SourceLocator::to_storage`] and its inverse, and there is nowhere
else to look.

# The storage encoding is tagged JSON

[`to_storage`](SourceLocator::to_storage) writes, and
[`TryFrom<&str>`](SourceLocator::try_from) reads, one object per
shape:

```json
{"kind":"file",   "path":"/pics/a.png"}
{"kind":"record", "container":"/logs/s.jsonl","record":"0198c1c2-…"}
{"kind":"remote", "scheme":"hf","target":"org/model/f.safetensors"}
{"kind":"logical","name":"chat/0198c1c2/msg-1"}
```

Reading is `serde` over that form, and **there is no recognition to
do**: the tag says which type it is, and each type then validates its
own fields — a `file` whose path has no root is still refused, a
`remote` whose scheme is one character still is, and `file` is still
never a [`Scheme`].

Two properties of the rendering are load-bearing rather than
stylistic:

- **Canonical.** Fixed field order, no whitespace, no optional keys.
  The `(persona_id, source_kind, source_locator)` lookup is an
  equality test on this string, so two equal locators must render to
  byte-identical text. `serde` derives the whole rendering from the
  shape below; nothing here hand-writes JSON.
- **Opaque to SQL.** Nothing queries inside it, so it is a
  self-describing encoding in a TEXT column — not a reason to reach
  for SQLite's JSON functions.

What the tag **removes rather than manages**, all of it inherited
from the delimited form this replaced:

- **percent-escaping.** No character in a path or a record address is
  special any more, so nothing has to be escaped on the way in or
  unescaped on the way out.
- **the `/pics/a#b.png` ambiguity.** A legal POSIX filename with a
  `#` in it was indistinguishable from a container plus a record.
  Here it is a `file` whose `path` contains a `#`, and nothing looks
  at that character. (The rows *already stored* under the old form
  are settled by the rewrite migration, which can ask the filesystem
  which of the two a given string was — a test no parser can run.)
- **the split-direction question.** `split` or `rsplit`, first `#` or
  last: there is no split.

# Two readers, because there are two boundaries

[`TryFrom<&str>`](SourceLocator::try_from) is the **storage** reader
and speaks only the form above. A locator arriving from an *importer*
is a different contract — `FootprintSource::locator` is documented as
a path, or `<container>#<record>` for a per-record source, and the
parsers emit exactly that — so it has its own entry point,
[`from_wire`](SourceLocator::from_wire), which is where the ordered
guess still lives. Keeping them apart is what lets the column form
change without renegotiating the SDK contract, and it is why nothing
outside `from_wire` reads a `#`.

A third reader exists and is not here: the **frozen** copy inside the
rewrite migration, which reads what the columns held *before* the
tag. It is deliberately a snapshot rather than a call into this
module, for the reason the V56 snapshot beside it already gives — a
landed migration has to keep meaning what it meant when it ran.

## Types

- `ContainerRecord` — One record inside a container file on this machine.
- `LocalPath` — An absolute path on this machine.
- `LogicalName` — A caller-minted name. No bytes anywhere.
- `RecordAddress` — How a container's reader finds one record again.
- `RemoteRef` — An artefact addressed by a scheme this machine does not resolve to a
- `RemoteTarget` — Everything after `<scheme>://`.
- `Scheme` — A URI scheme, lowercased.
- `SourceLocator` — Where an artefact's bytes are. Says nothing about what it is, and

