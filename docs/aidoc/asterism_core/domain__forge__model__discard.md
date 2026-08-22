# asterism-core::domain::forge::model::discard

What goes when a line goes, and what that lets go of.

```text
  Line (Archived)          Pursuit*  (against it, all ended)
    └ holds ──┐              └ holds ──┐
              ▼                        ▼
           releases(line, work) ──▶ every Content that was held
                                    by either, and by nothing
                                    else afterwards
```

# Why this is one answer rather than two

A line holds what its chain named; a work log holds what its
operations named. Dropping a line takes the work against it —
a log cut from a history that no longer exists is a record of a
proposal against nothing, and its base names a node that is gone.
So the set that becomes releasable is the union, and asking for it
as a union is the point: a caller adding the two up itself is a
caller that can forget the second one, and forgetting it looks
exactly like success.

# This module reads both logs

It is the third that does, after [`change`](super::change) and
[`closing`](super::closing), and the reason is different from
theirs. Those two answer what work *means* against a line. This one
answers what is lost — a question about the two records as records,
which is why it takes the work as a slice rather than reaching for
it: a line does not keep a list of its pursuits, and one that did
would be a second answer to what the work logs already say.

# It does not drop anything

There is no delete here, as there is nowhere else in this module —
see the model's own note on that. What this returns is what a drop
*would* release, together with the refusals that say a drop may not
happen at all. Whatever does the deleting asks first.

## Functions

- `releases` — What dropping this line would let go of.

