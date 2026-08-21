# asterism-core::application::forge::pursuit_service

Work use cases — opening a line of work, writing passes, looking at
what the line did, and ending it.

```text
  open      reads the line's head, writes the work log
  push      writes the work log. does not read the line
  resolve   reads the line, writes the work log
  close     reads both, writes both — the only one

  collisions / behind    read both, write nothing
```

Four verbs and two questions, and only one of them touches a line's
history. That asymmetry is the point of the design rather than an
accident of it: the operation that happens most often, writing a
pass, never reads the line at all, so two people working against
one line do not contend until one of them finishes.

# What this service is allowed to decide

Nothing. It loads what the model needs, calls it, writes back what
came out, and — for the one operation that can lose a race — reads
again and asks again. Every refusal in here comes from the model or
from a port.

# Losing the race is not an error to report

Two pieces of work can finish against one line at the same moment,
and only one of them lands on the head. The other is told, and this
service reads the line again and decides again rather than handing
that back to a caller.

It matters that this is a fresh decision and not a retry of the old
one. Reading again means normalising against a line that has moved,
so what the work still changes may be less than it was, and what it
now collides with may be more. Handing back the same answer would
be writing a decision made against a line that no longer exists.

## Types

- `PursuitService` — Work use-case service.

