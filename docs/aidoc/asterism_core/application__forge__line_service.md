# asterism-core::application::forge::line_service

Line use cases — opening one, reading what is on it, and moving its
own description.

```text
  open                              writes the whole line, genesis
                                      and history included
  rename / set_strategy             writes the line's description
  archive / reopen                  writes the line's standing
  get / states / strategies         reads
  discard                           reads both logs, writes neither
                                      — it takes them away
```

Nothing here writes to a line's history. A line moves when work
ends, and that is
[`PursuitService::close`](super::PursuitService).

# The one verb that reads the other log

[`LineService::discard`]. What a drop releases is the union of what
the line holds and what the work against it holds, so the call that
drops has to ask the pursuits — and it is the only one here that
does. Every other verb on this service can answer from a line
alone, which is why the dependency is worth naming rather than
assuming.

# What this service is allowed to decide

Nothing. It loads, calls the model, and writes back what came out.
The two checks it does make are not judgements: that a line exists
before it is written to, and that the rule a caller names is one
this deployment carries. Both are lookups with one answer.

# Choosing a rule is a real choice

[`LineService::strategies`] exists so that it can be made. A line
settles collisions by a rule, and the rules differ in what happens
to somebody's work — so the list a person picks from is built from
the rules themselves, and every one of them says what it does.

## Types

- `LineService` — Line use-case service.

