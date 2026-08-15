# asterism-exporter-common::jsonpath

A JSONPath subset — enough to steer a state machine and pluck out
items, and deliberately no more.

```text
$.foo          object field
$.foo.bar      dot chain
$.arr[0]       array index
$.arr[*]       array wildcard
```

# Why a subset

A schema-driven exporter reads a path out of caller-supplied JSON, so
the grammar is a public surface that has to be explainable in the
params example a backend author works from. Filters, slices and
recursive descent would each need a sentence there and none of them
has appeared in a real backend's response shape: the documented cases
are "the status field", "the error message", and "the array of
outputs".

A wildcard may appear anywhere the walk can widen, not only last.
Nothing enforces a position because nothing needs to: the walk is a
breadth-first frontier, so `$.a[*].b[*]` falls out of the same loop
that handles one level.

# Missing is not an error

Every function here answers with what it found. A path that matches
nothing yields an empty vector or `None`, and the caller decides
whether that is a failure — a poll predicate reads it as "not yet",
while a handle extraction reads it as a backend that did not answer
the way its profile said it would. Returning a `Result` here would
push that judgement into a layer that cannot make it.

# Where this lives and why

It left `asterism-exporter-http` when a second exporter needed it.
One grammar with two spellings is worse than either spelling: a
profile author reads one paragraph of documentation and cannot tell
which adapter it describes, and a fix to the wildcard in one copy
leaves the other wrong in a way no test in either crate can see.

Adapters reach it through [`crate::ResponsePath`] rather than calling
[`many`] directly, so the selection grammar is substitutable for the
same reason the substitution grammar is.

## Functions

- `first` — The first value the expression selects, or `None`.
- `many` — Every value the expression selects, in document order.

