# asterism-core::domain::forge::model::react

Letting a line's rule answer a collision.

```text
  collisions(L, W)
       ├─ none ─────────────────────────────────► nothing to do
       └─ some ──► Strategy::resolve ──► ops ──► a pass, written by the server
                         │                 │
                         │                 └─ checked: do these actually settle it?
                         └─ none ──────────────► the collision stands, for a person
```

**A rule does nothing a person could not have done.** It writes the
operations somebody resolving by hand would have written, in the
same four verbs, into an ordinary pass. There is no vocabulary for
resolving, no transformation applied on the way onto a line, and no
record of resolution separate from the operations themselves. What
it does is save somebody the typing.

That is the whole of what automatic resolution is here, and it is
why the complexity lives in the rule rather than in the model: the
sequences differ — fork the entry and move your value onto it, keep
yours and move the line's, put both on new entries and take the old
one off, record what you tried and then drop it — and every one of
them is expressible already.

# What comes back is checked

A rule is written outside the model, so it can return operations
that do not settle what it was asked about. Folding them in and
looking is cheap, and the alternative is finding out at the far end
of the work, when a close refuses for a collision somebody was told
had been handled.

# A rule that writes nothing is not a failure

Some lines are meant to be sorted out by hand. Their rule returns
nothing, no pass is written, and the collision stays exactly where
anybody can see it — which is the state a person then acts on.

# Nothing here touches the line

The pass goes on the work log. A line moves when work ends, and
that is somewhere else.

## Functions

- `react` — Runs the line's rule over whatever this work collides with.

