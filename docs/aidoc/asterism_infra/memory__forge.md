# asterism-infra::memory::forge

The forge's ports, over rows held in memory.

```text
  Lines / Pursuits / Closings
           │
           ├─ write ──► forge::rows::take_*_apart ──► Vec under a Mutex
           │
           └─ read  ──► forge::rows::read_* ──► restore ──► Line / Pursuit
```

One `Mutex` over the whole store rather than one per table: the
close writes a change point, its rows and an ending together, and a
reader must not see half of that. A real adapter gets the same
property from a transaction; here it comes from holding the lock
across the whole of `commit`, which is the same statement made the
only way this store can make it.

# What it is for

Proving the model and the services against something that keeps
rows, before any of it depends on SQLite being right. A store that
kept the domain objects would answer every call correctly by
construction and would never once ask whether a line can be built
back out of what was written down — which is the question the whole
read half exists to ask, and the one the first fake never reached.

# What it is not

Durable, concurrent beyond one process, or a thing to run anything
real on. There is no index: reads scan, which is fine at the sizes
a test builds and would not be at any other.

## Types

- `HoldsEverything` — What the layer below answers, for a store that has no layer below.
- `HoldsNothing` — The same face, answering no.
- `MemoryActors` — Handles for whoever is writing, minted once per subject and kept.
- `MemoryForge` — An in-memory forge store. Clone it to hand the same tables to every

