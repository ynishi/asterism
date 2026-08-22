# asterism-core::domain::forge::model::restore

Building the model back from what a store kept.

```text
  stored rows ──► restore::line / restore::pursuit / restore::thread
                       │
                       ├─ the ids come from outside, here and
                       │  nowhere else in the model
                       │
                       └─ the nodes go back through record / push /
                          end / say — the same refusals a fresh
                          write meets
```

# Why this is one module rather than a constructor per type

Every other constructor in [`model`](super) mints. A line mints its
id and its genesis, work mints its id and its opening node, a round
mints a node id — which means that until this module existed, no
value here could be built holding an id somebody else chose. That
was the property, and it is the reason the read half of the ports
had no implementation but a fake for as long as it did.

A rehydration constructor takes every field including the id, so it
is the one door through which a stored row can contradict a rule
this module holds. Spreading it across the types would put a piece
of that door on each of them and leave nothing that could be read
as the whole. Here it is one file, and what it may and may not do
is stated once.

# What it does not do

**It does not skip the model's questions.** The nodes are handed
back one at a time to [`History::record`], [`Pursuit::push`] and
[`Pursuit::end`] — the same calls a fresh write goes through, and
the same refusals. A chain whose parents do not line up, a table
that would leave two live entries under one name, a round on a log
that has ended: each of those is a stored row that cannot become a
value here, and the read fails rather than handing back something
the model would not have written.

That is the cost as well as the point. The check that reads a name
twice folds the chain, so putting back a history of *n* change
points costs what *n* reads cost, and a store that keeps something
it cannot read back is a store that has to be repaired rather than
opened. Both are consequences somebody may want to price
differently later; neither is a thing to discover by accident.

**It does not put the chain in order.** A change point carries its
parent, so [`line()`] takes them in whatever order a store hands them
over and walks the links from the genesis. Nothing has to keep a
sequence number beside the chain, and a store that got the order
wrong is caught by `record` rather than believed.

## Functions

- `change_point` — One change point, as it was kept.
- `close` — The node work ended at.
- `genesis` — The node a line began at.
- `line` — A whole line, from its genesis and the change points on it.
- `message` — One thing said, as it was kept, with every correction to it.
- `meta` — The two stamps a store kept for a thing's description.
- `open` — The node work opened at.
- `pursuit` — A whole pursuit, from the node it opened at and what followed.
- `round` — One round, as it was kept.
- `thread` — A whole conversation, from what it hangs off and what was said.

## Types

- `Node` — A node of a pursuit after the one it opened at.

