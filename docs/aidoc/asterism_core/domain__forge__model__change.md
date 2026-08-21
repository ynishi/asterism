# asterism-core::domain::forge::model::change

Putting work on a line — the one place both logs are read at once.

```text
  what work asks for  ──normalise──▶  what would change
        (rows)              ▲              (write set)
                            │                   │
                   the line's head              │
                                                ▼
                                          collisions?
```

Everywhere else in the model, one log is read at a time. Here they
meet, and the order matters: what a proposal *means* is only
decided against the line it would change.

# Normalising is what makes work survivable

[`normalise`] drops what the line already says. An axis whose value
matches the head is not a change; an arrival for something already
on the line is a content-and-name change rather than an arrival; a
removal of something already off the line has nothing left to do.

Without it, two people doing the same thing would leave the second
permanently unable to change anything — and worse, the second one's
unchanged rows would write themselves over a line that had moved
on, undoing whatever happened in between.

# An empty write set is an outcome, not a failure

When everything falls away, the work has nothing left to say —
usually because somebody else said it first. There is nothing to
record, because a change point carrying nothing is a line advancing
to say nothing. What was attempted stays readable either way.

# Collision is derived, never stored

A collision is an axis this work would write that the line has
already moved since the work was cut. That is the whole of the
definition — there is no second clause, and nothing about whether
anybody looked.

**What clears one is the work saying something different.** Not a
record of having read, which is a claim about the reader and can be
written without changing anything; not a flag, which is a second
thing to keep true. When the work's value for an axis becomes what
the line already says, normalising drops it, and there is nothing
left to collide.

So resolving is ordinary work: the operations somebody would have
written by hand. If the line moves that axis again afterwards, it
collides again, and is resolved again — one at a time, which is
what resolving against a moving line means.

Nothing here stores a collision. It is computed from the two logs
whenever it is asked, so it cannot go stale and there is no flag
for anybody to forget to clear.

# This module reports; it does not settle

What to do about a collision is a decision — take the line's side,
take the work's, or put both on the line under different names —
and it belongs to whoever set the line up rather than to the code
that noticed.

## Functions

- `collisions` — The axes this work would move that the line moved after the work
- `normalise` — Drops everything the line already says.
- `since` — Every change recorded after `base`, in the chain's order.
- `states_at` — What the line carried at `at`, folded from the chain up to and
- `write_set` — What a normalised set of rows would change, axis by axis.

## Types

- `Axis` — One thing a row can move.
- `Collision` — An axis the line moved first.
- `WriteSet` — What a change would actually move.

