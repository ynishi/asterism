# asterism-core::domain::forge::model::table

What a change point carries, and what folding a sequence of them
answers.

```text
  Table : Entry ──▶ Row
                     ├ existence?   on the line / off it
                     ├ content?     what it holds
                     └ name?        what it answers to
```

# Three axes rather than a verb

A person writes verbs — add this, rename that — but by the time a
table reaches a line, the question is per axis, and a verb set cannot
spell "says nothing about the name" except as another verb. So a
row states only the axes it moves, and [`Row::added`],
[`Row::replaced`], [`Row::renamed`] and [`Row::removed`] are the
four verbs written as the rows they mean.

The gain is that disagreement becomes visible without comparison:
two change points that touched different axes of one entry did not
disagree, and nothing has to work that out after the fact.

# Two shapes a row must not take

[`Row::new`] refuses both, because neither can be read back:

- **A row that states no axis** puts an entry in a table that never
  moved it, which is exactly the claim a reader takes from finding
  it there.
- **A row that takes an entry off while naming or filling it** gives
  one state two spellings. Removing and renaming across two change
  points reaches the same place, and the fold cannot tell the two
  apart afterwards — so allowing both would stop a table being a
  description of what its change point did.

A row that states existence alone is legal, and is what a revival
looks like once the axes already matching the head fall away.
Which rows make sense *against a particular head* is a different
question, and it belongs to the step that judges a table before it
is recorded rather than to the row.

An empty [`Table`] is refused for the same kind of reason: a change
point carrying nothing is a line advancing to say nothing, and
there is no reading of the history under which that means anything.

# The fold

[`states`] takes tables **in the chain's order** and lets later
ones win per axis, on the axes they state. The three axes derive
independently, which is why taking an entry off does not erase what
it was called: a name that is off the line is still readable, and
merely available again.

An entry appears in the result as soon as any table names it, on
the line or off it — "was taken off" and "was never here" are
different answers, and a caller can tell them apart.

## Functions

- `states` — Folds tables into what is on the line.

## Types

- `EntryState` — One entry's position: whether it is on the line, and what it
- `EntryStates` — Every entry a line has heard of, and where each one stands.
- `Row` — One entry's line in a table: what is said about it, on the axes
- `Table` — What one change point carries: one row per entry it moves.

