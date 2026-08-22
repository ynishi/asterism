# asterism-core::domain::forge::clock

What time it is.

A port for something every other service in this codebase reads off
the system clock directly, and the difference is what a timestamp
means once it is written.

```text
  elsewhere      a row's updated_at      convenient, overwritten by the next write
  here           an act's `at`           evidence, kept forever, ordered by nothing
```

A forge node does not move once recorded, so a wrong time is wrong
for good. And the chain — not the clock — is what orders a history,
so a wrong time breaks nothing: no fold reads it and nothing fails.
One thing is ordered by it and it is the exception that shows the
rule — a conversation, which has no chain to read an order out of
(see [`thread`](crate::domain::forge::model::thread)). A backwards
clock swaps two remarks there and refuses nothing; the one order it
cannot produce, an answer before its question, is put right when the
conversation is read. What a wrong time does instead is answer a
question incorrectly, quietly, for as long as the record exists.

That question is not incidental. A record of a selection has to say
what was chosen, out of what, by whom, **and when** — those are the
terms on which this layer is worth building at all. A value that
nothing verifies and nothing depends on is exactly the value that
has to be pinned by a test, and pinning it means the service asking
something rather than reading a global.

# Not for the model

Nothing under [`model`](crate::domain::forge::model) takes a clock.
A decision is handed an [`Act`](crate::domain::forge::model::act::Act)
that already carries its time, which is what keeps deciding
reproducible: the same line, the same work and the same act give the
same answer, today and in a test. This port belongs to the services
that assemble an act, and to nothing below them.

# Not for the rest of the codebase

The other services keep calling the system clock, and should. Their
timestamps say when a row was last touched, a row that will be
touched again — turning those into evidence would be paying this
cost everywhere for a claim only this layer makes.

## Types

- `SystemClock` — The clock a running system uses.

## Traits

- `Clock` — Says what time it is.

