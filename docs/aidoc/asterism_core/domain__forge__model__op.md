# asterism-core::domain::forge::model::op

What work writes, and what it folds into.

```text
  Op : Entry ──▶ Add | Replace | Rename | Remove

  [Op] ──fold──▶ Entry ──▶ Row
```

Four verbs, and every one of them names an entry. That is the whole
vocabulary of work: there is nothing to say about a line that is
not about one of the things on it.

# An add mints its own entry

[`Op::add`] mints the [`EntryId`] on the spot, before any change
has been recorded. Work refers to what it proposed by that id, so a
later round can rename or replace something no history has heard of
— and when it does reach the line, it reaches it as the thing that
was being talked about all along rather than as a new arrival.

Nothing has to agree to a mint. An id is a surrogate, so there is
no shared counter to contend for, and proposing costs nothing that
has to be taken back if the work is abandoned.

# Nothing here asks whether two contents are the same

An add is taken at its word. Somebody meant to put something on the
line, so an entry arrives — and if what it holds is byte-identical
to what another entry already holds, that is still two entries and
both are on the line. No add is refused, folded into an existing
entry, or turned into a change to one, on the grounds of what it
points at.

This is a boundary rather than an omission. Whether two things are
*the same thing* is a question about bytes, and the layer that
holds them answers it — with a fingerprint over the original, an
`identical_to` edge recording the fact, and a queue of the
questions that fact raises. The forge sees the outcome and nothing
else: when that layer decides two things are one, the same
[`Content`] comes back, and sameness shows up here as two rows
agreeing rather than as a judgement the forge made.

Running the question this way round is what keeps the two kinds of
statement apart. A forge that folded adds by content would be
deciding what somebody's selection meant on the strength of a
digest, and the record of what was chosen out of what would quietly
become a record of what survived deduplication.

Where the forge could eventually use that answer — showing whoever
is adding that the line already holds these bytes, or letting a
collision settle onto one entry instead of two — it uses it. It
does not compute it.

# The fold reads work and nothing else

[`fold`] takes operations and returns rows. **It does not take the
line.** Given the head it would produce a different answer at
different moments, and "what this work asks for" would stop being a
property of the work — the same operations would mean one thing
before somebody else changed the line and another thing after.

Comparing that answer against a line is a later step, and a
separate one. Here the rule is only: per axis, the last operation
to write it wins.

# Existence absorbs, or stands alone

An entry being put on the line takes the winning content and name
with it, because that is one arrival rather than three statements.
An entry being taken off keeps nothing else: renaming something on
its way off says nothing anybody can read back.

An entry no operation puts anywhere keeps whatever axes were
written — replacing the content of something already on the line
says nothing about its existence, and should not.

## Functions

- `fold` — Folds operations into rows: per axis, the last one to write it

## Types

- `Op` — One thing work asks for about one entry.
- `OpKind` — The four verbs.
- `Rows` — What the winning operations said, per entry.

