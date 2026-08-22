# asterism-core::domain::forge::closings

Ending work — the one call that writes to both logs.

```text
  Lines      open / get / rename / set_strategy      one log
  Pursuits   open / get / push                       one log
  Closings   commit                                  both, or neither
```

Every other call the forge makes writes to one log. This one writes
to two, because ending work as satisfied puts a change point on the
line and the two nodes are one act. It exists so that "one act" has
somewhere to be true outside the model: [`Closing`] already makes
the pair impossible to hold apart in memory, and this makes it
impossible to store apart.

# What the contract requires

**Both, or neither.** Every node in the closing is kept, or none of
them is. There is no partial outcome for a caller to detect, and
nothing to compensate for afterwards — an ending that half-happened
would leave the two logs disagreeing about whether work is over,
and no later read could tell which of them was right.

**On the parent nothing has taken.** A closing names the node it
sits on, and two nodes on one parent is a fork. The write refuses
one, which is what "the line moved since this was decided" looks
like from underneath — somebody else's change point is already
where this one would go.

**Decided again, once, by whoever is holding the write.** A caller
that loses that race does not hear about it. The store asks
[`Deciding`] for an ending against the two logs as the write finds
them, and that attempt is final: the caller decided outside the
write, where the line could still move, and this one is decided
inside it, where it cannot.

That is the whole of the concurrency story. Nothing is locked while
a caller decides, no order is imposed on who gets to close first,
and losing costs one re-decision rather than a round trip — where
the collision with whoever won becomes visible in the ordinary way,
because deciding again is deciding against the line that won.

# How it is achieved is not stated here

A transaction, an append that takes several streams, one row
holding both — the forge does not care and does not ask. What it
states is the outcome a caller can rely on, which is the only part
it can reason about. Naming a mechanism here would be the storage
deciding what the model means.

# Why abandoning comes through here too

An abandoned closing writes to one log, so it does not need this
call. It goes through it anyway, because "which endings need both
logs" is a question about the model rather than about the caller —
and a second path for the easy case is a path somebody reaches for
with the hard one.

[`Closing`]: crate::domain::forge::model::closing::Closing

## Traits

- `Closings` — Keeps what ending work produced.
- `Deciding` — Ending work, against the two logs as they are handed over.

