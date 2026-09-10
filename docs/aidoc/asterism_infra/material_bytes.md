# asterism-infra::material_bytes

The bytes a locator addresses, when they are on this machine.

Every job that decodes, measures or hashes an original asks the same
two questions in the same order, and the answers are not the same
kind of thing: *are there bytes here at all* decides whether the row
is retired, and *could they be read just now* decides whether it is
left for a later pass. Recording a temporary failure permanently is
the dims-walk mistake those jobs each carry a comment about.

[`read`] answers both in one value, so the two stay in the order
that keeps them apart:

- `None` — nothing here will ever have bytes. A remote locator, a
  caller-minted name, or a record inside a container nothing opens.
- `Some(Err(_))` — bytes that should be here and were not readable
  this time.
- `Some(Ok(bytes))` — the bytes.

## Which containers open, and which of their records

A [`Record`](SourceLocator::Record) is a container plus an address
inside it, and most such records have no bytes of their own.
`ContainerRecord::holds_its_own_bytes` is the question, and its
docstring says which shapes answer yes and why the container is not
opened on the others' behalf.

A shape answering `true` there is not the whole test. A card
archive addresses two kinds of thing with one spelling: the entries
it packs (`#assets/icon/images/main.png`) and the slots the card
states (`#field=name`), and only the first names bytes. Which is
which is a question only the archive can answer, so this module
asks it — an address the archive does not hold reads as `None`,
the permanent answer, rather than as a read that failed. Retrying a
slot suffix on every backfill pass, forever, is the walk that never
shrinks.

## The ceiling

An entry states its own uncompressed length and the file it sits in
was written by somebody else, so the length is a claim rather than a
fact. [`MAX_ENTRY_BYTES`] is the ceiling the read is held to, and an
entry over it is refused rather than allocated for — the same
judgement `fingerprint::hash_artefact` makes about a file it is
asked to hold whole.

## Functions

- `read` — The bytes this locator addresses.

## Constants

- `MAX_ENTRY_BYTES` — The most an entry is read into memory, at 64 MiB.

