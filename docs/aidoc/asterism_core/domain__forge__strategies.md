# asterism-core::domain::forge::strategies

The rules a line can settle collisions by, and the lookup that
finds one.

```text
  Line ── strategy : StrategyId ──► Strategies::get ──► &dyn Strategy
                                         │
                                         ├ mainline-first   the line stays, ours moves aside
                                         ├ mine-first       ours stays, the line's moves aside
                                         ├ both-diverge     both move aside, the old entry goes
                                         ├ discard-mine     ours is written down and dropped
                                         └ by-hand          nothing is written
```

[`Strategy`] is stated in the
model, deliberately without an implementation: how a collision is
split, and what the new entry is called, are not things the model
decides. This module is where the answers live.

# Resolving is ordinary work

A rule writes what a person would have written by hand, in the four
verbs everybody uses. Nothing here is a special mechanism: there is
no vocabulary for resolving, no transformation applied on the way
onto a line, and no record of a resolution apart from the
operations themselves. What a rule saves is the typing.

That is also why the rules are behind a trait rather than spelled
in the model. The sequences are involved, they differ from each
other, and more of them are sensible than anybody should have to
pick between in advance.

# What every rule is up against

An axis stops colliding when the work stops requesting it — see
[`collisions`](crate::domain::forge::model::change::collisions).
Asking for the same value again changes nothing, so **no rule can
make this work's version win in place**: it wins by taking the
contested entry off the line and putting the value somewhere
nobody is arguing about. A person resolving by hand hits the same
wall for the same reason.

```text
  line  E "cut-01" = theirs          work  E = mine

  mainline-first   E stays theirs           mine → new entry "cut-01 (2)"
  mine-first       E is taken off           mine → new entry "cut-01"
                                            theirs → new entry "cut-01 (2)"
  both-diverge     E is taken off           both → new entries, both numbered
  discard-mine     E stays theirs           mine → new entry, then removed
  by-hand          nothing is written       the collision stands
```

`mainline-first` and `discard-mine` differ by one operation, the
removal of the entry that was forked. `mine-first` and
`both-diverge` write the same three and differ in who gets the
name — which is the whole of the difference between "this work's
version is what `cut-01` means now" and "`cut-01` means neither of
them any more".

# Beside the model, not inside it

A rule is domain logic — it produces the forge's own operations, in
the forge's own vocabulary, and no part of it is coordination or
I/O. So it belongs to this layer rather than to the services above.
What keeps it out of `model` is narrower: `model` holds what is
true of every line, and which rule a line uses is true of that
line. A rule sitting in there would read as one of the model's
statements when it is one of its options.

# Looking one up cannot fail into silence

[`Strategies::get`] returns an `Option`, and the caller is expected
to refuse rather than fall back. A line that points at a rule this
deployment does not carry has to say so: settling it by whatever
rule happens to be first would settle it by a rule nobody chose,
and the record would not say that had happened.

## Types

- `Builtin` — The rules that ship with the forge.

## Traits

- `Strategies` — Finds the rule a line points at.

