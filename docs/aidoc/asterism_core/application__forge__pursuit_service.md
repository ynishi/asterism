# asterism-core::application::forge::pursuit_service

Work use cases — opening a line of work, writing passes, looking at
what the line did, and ending it.

```text
  open      reads the line's head, writes the pursuit
  push      writes the pursuit. does not read the line
  resolve   reads the line, writes the pursuit
  close     reads both, writes both — the only one

  collisions / behind    read both, write nothing
```

Four verbs and two questions, and only one of them touches a line's
history. That asymmetry is the point of the design rather than an
accident of it: the operation that happens most often, writing a
pass, never reads the line at all, so two people working against
one line do not contend until one of them finishes.

# What this service is allowed to decide

Nothing. It loads what the model needs, calls it, and writes back
what came out. Every refusal in here comes from the model or from a
port.

# Losing the race is not an error to report, and not answered here

Two pieces of work can finish against one line at the same moment,
and only one of them lands on the head. What the loser needs is a
fresh decision against the line that won — not the same answer
written again, because normalising against a line that has moved
may leave less to record than there was, or more to collide with.

Deciding that again is the model's, and the moment to do it belongs
to whoever is holding the write. So this service hands the store
[`Deciding`] along with what it decided, and the store asks for a
second answer under its own lock if the first is refused. Reading
again from here and trying again would be deciding against a line
that can move between the read and the write, once per attempt, for
as many attempts as anybody has patience for.

## Types

- `PursuitService` — Work use-case service.

