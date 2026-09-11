# asterism-core::domain::send

`ReleaseSend` — the record that a release was put on a destination's
host.

```text
  Release ── snapshot ── DispatchJob
     ▲                        ▲
     └──── ReleaseSend ───────┘
              act
              destination
```

# Why it hangs off a release and not off a change point

What travels is the stamped set, and the stamped set is what a
release left behind: [`Release::files`](crate::domain::release::Release::files)
names every copy by asset and by path. A send anchored one node
higher would have to freeze and stamp a second time to know what its
bytes were, and the two stamped sets would then differ by whenever
the second pass ran.

# Two sends of one release are two records

Nothing here keys on the release, and nothing refuses a second send:
a re-submission after a rejection is the case this record exists to
hold, and a store that collapsed the two would answer "when did this
go out" with one of the dates.

# What a send does not carry

What became of the put. The dispatch it names is where that lives —
its state, and the attempt record naming each file and what the
server answered — and nothing in this process would maintain a
second copy: the runner does not know what a send is, which is the
same reason
[`OutboundStamping`](crate::application_support::OutboundStamping)
is a port rather than a call.

# Why the type is not called `Send`

`Send` is a trait in the prelude, and traits and structs share the
type namespace. A `struct Send` here would shadow the bound in every
module that imported it, so `Send + Sync` on the ports beside it
would stop naming the auto trait. The record's own word is in the
module name, the id, the repository and the verb; only the struct
carries the qualifier.

## Types

- `ReleaseSend` — One release, put on a destination's host.

