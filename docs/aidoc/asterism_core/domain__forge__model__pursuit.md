# asterism-core::domain::forge::model::pursuit

One line of work: what it is trying to do, and every pass at it.

```text
  Pursuit ── of : →Line ── parent? : →Pursuit
   └ WorkLog
       Open ──▶ Round ──▶ Round ──▶ Close?
         │        │  └ ops
         │        └ parent
         └ base : the change point this was cut from
```

The shape mirrors a line's history deliberately: a root that is not
like the others, a chain that only grows, and everything the thing
currently asks for derived by folding it. What differs is what the
nodes carry — a line's node carries a table that has changed it, a
work log's carries operations that have not changed anything yet.

# A pass is the unit, not the pursuit

[`Round`] is where work happens. A pursuit is the container that
says what the passes are for, and it holds nothing that a round
could hold instead.

A round that writes nothing is refused. Work is what a person does
to a request, and a node carrying no operations records that
nothing happened.

# The base does not move

[`Open`] names the change point the work was cut from, and that is
the only thing the base ever is. Nothing here moves it, and there
is no operation that could.

It is what "since" is measured from. Everything the line recorded
after it is something this work may not account for, and comparing
the two is how that is found out — so a base that crept forward
would shrink the window every time somebody looked at it, and a
change nobody ever reconciled would come out clean.

# A pass is a write, and nothing else

There is no node here that records having looked at something.
Work stops colliding with a line by *saying something different*,
not by noting that it read. A note of that kind would be a claim
about the reader — writable without changing anything — and
whatever depended on it could be had for nothing.

# Closing is terminal

[`Close`] carries which kind of ending it was, and nothing can be
pushed after it. Satisfied means the intent was met — the act that
turns that into a change point spans both logs and is not here.
Abandoned means it was not, and that is a record rather than a
deletion: the pass that was dropped stays readable, which is the
only way "we tried this and stopped" survives.

Reopening is not a verb. Picking work back up is a new pursuit
with the same parent, and that reads as what happened.

# Parent is where work belongs

[`Pursuit::parent`] says which larger piece of work this is part
of. It is fixed when the pursuit is opened, because a link that can
be corrected is a link that has to be checked for cycles; one that
can only point at something already open cannot form one.

Nothing else about the relationship is stored. Which pursuits are
under a parent, and what they changed, is a question answered by
looking — and a stored answer would be a second copy of it.

## Types

- `Close` — The node work ends at.
- `Intent` — Why a pursuit was opened, in the words of whoever opened it.
- `Open` — The node work begins at.
- `Outcome` — How a pursuit ended.
- `Pursuit` — One line of work against one line.
- `Round` — One pass at the work.
- `WorkLog` — The chain of passes.

