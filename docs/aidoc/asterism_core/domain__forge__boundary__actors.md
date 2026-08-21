# asterism-core::domain::forge::boundary::actors

The face that asks who somebody is.

```text
  the write path          this contract            the forge
  AttributionContext ───►  Actors::resolve  ───►   ActorId
  (author / operator /                             (a handle, and
   channel)                                         nothing else)
```

The forge records who did a thing as an
[`Actor`](crate::domain::forge::model::act::Actor), which is a
handle and a kind. What the handle stands for is not its business:
which authenticated user, on which instance, is answered by the
side that knows what a user is. This is where that question is
asked.

# Why the forge does not just record the triple

It used to. The triple says who was writing, which agent carried it
out, and through which entry point — and it is the right answer for
a row that records a write. A forge node is not that: it is a
statement about a choice, kept forever, in a record whose whole
purpose is to still answer "who chose this" years later.

Two things follow. The forge's actors are a **wider** set — a
line's own rule can act, and no person did that — and a wider set
cannot be written in a narrower vocabulary without pretending a
rule is somebody. And the identity on the other side is **not
settled yet**: the owner of an instance is an unbound reference
until authentication binds it, so a node that recorded the answer
today would be recording an absence, and would have to be rewritten
the day the answer arrives. Nodes do not get rewritten.

A handle solves both. It exists before the binding, it keeps
pointing at the same actor afterwards, and what it resolves to is a
row on the other side of this contract — one row to update, rather
than a history to rewrite.

# Resolving mints

[`Actors::resolve`] answers for anybody, including somebody it has
never seen. There is no "unknown actor" case for a caller to
handle, because there is nothing sensible to do with one: work is
being recorded, it was done by somebody, and a record that shrugs
about who is a record that fails at its one job.

# The server is an actor too

[`Actors::server`] answers for the instance itself, which is what a
line's rule writes as when it turns a collision into a divergence.
It is asked rather than assumed for the reason a user is: one
deployment is one server today, and several is exactly the setting
where "the system did it" stops being an answer.

## Traits

- `Actors` — What the side that knows about people answers.

