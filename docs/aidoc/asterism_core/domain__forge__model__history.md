# asterism-core::domain::forge::model::history

A line's history: what it carries, and how it got there.

```text
  Genesis ──▶ ChangePoint ──▶ ChangePoint ──▶ …
     │             │                        ▲ head
     │             └ parent / from / by / table / act
     └ act
```

The history *is* the line's record. There is no second place that
says what is on it — [`History::states`] folds the chain every time
it is asked, and a kept copy would be a second thing to hold true.

# One chain, never a fork

A line exists so that there is one answer to what it carries, and
two chains would be two answers. [`History::record`] therefore
refuses any change point that does not name the current head as its
parent. That refusal is the whole of the rule — not a check
somewhere else that callers are asked to remember, and not a
resolution step that quietly picks one of the two.

What a caller does with the refusal is rebuild against the new
head, which is where a collision with the work that got there first
becomes visible. That belongs to the act that produces change
points, and it is not in this module.

# The chain is the order

Which change point came first is which one took the other as its
parent. Nothing here reads a clock to decide that, and
[`ChangePoint::act`] is a record of when something happened rather
than an input to ordering — two nodes minted in the same
millisecond are still ordered, and a clock that steps backwards
changes nothing.

# A genesis is not a change point

[`Genesis`] carries no table, because there is nothing before it to
change, and comes from no work, because no work had a line to be on
yet. It is a separate type rather than a [`ChangePoint`] with empty
fields: modelled as one type, `parent`, `from` and `table` would
all be `Option`, all three would have to be empty together or
filled together, and a shape that has to be kept consistent by
agreement gets filled halfway.

Its purpose is that a line has a head from the moment it exists.
Without it, "the head of a line nothing has changed yet" would be
an absence every reader carries, and the first change would be a
shape of its own.

# It only grows

Taking an entry off a line is a change point that says so. The
record stays, the name and content it had stay readable, and
nothing here removes a node — there is no method to, which is the
only way to mean it.

# A change point cannot be minted from here

`ChangePoint::new` is visible to the model and to nobody else,
and outside tests exactly one function calls it: the one that
closes work. A change point exists because a pursuit was satisfied,
and the two are born together — a constructor anybody could reach
would be a way to write the second half of that pair on its own.

What this module still owns is everything about the chain: what a
change point *is*, and that recording one refuses anything but the
head. Who is allowed to make one is a different question, and the
answer is one caller.

## Types

- `ChangePoint` — One move of a line's history.
- `Genesis` — The node a line begins at.
- `History` — The chain itself.

