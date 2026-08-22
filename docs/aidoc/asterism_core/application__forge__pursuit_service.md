# asterism-core::application::forge::pursuit_service

Work use cases — opening a line of work, writing rounds, looking at
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
round, never reads the line at all, so two people working against
one line do not contend until one of them finishes.

# What this service is allowed to decide

Nothing. It loads what the model needs, calls it, and writes back
what came out. Every refusal in here comes from the model or from a
port.

## Types

- `PursuitService` — Work use-case service.

