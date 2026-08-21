# asterism-core::domain::forge::model::act

When something happened, and who did it.

One type rather than two loose fields, because the two always
arrive together and a record with only half of them answers
neither question: a time with nobody attached says an event
occurred and refuses to say whose it was, which is the shape a
history exists to avoid. Every node of a history carries an
[`Act`], and so does anything whose own description can move.

# Two records, not one

[`Meta`] is the second of those. A line's change points say what
happened to what the line carries; they say nothing about the line
being renamed, or its strategy being changed, and those are not
change points — putting them on the same chain would make "the
line moved" and "the line is described differently" the same
event, and neither question could be answered afterwards.

So a description keeps its own two stamps: when it was made, and
the last time it moved. They are equal until something moves it,
which is a fact rather than a placeholder — nothing has happened to
it yet.

# Whether it was allowed is not asked here

Recording who did a thing is what a history owes. Deciding whether
they were permitted to needs to know what a person is, and that
question belongs to the layer that does.

# An actor is a handle and a kind, and nothing else

[`Actor`] holds an [`ActorId`] and says whether it was a person or
the server itself. Everything else about them is outside:

```text
  node        ActorId + kind                    ← the forge keeps this much
                 │
  outside        ├─ User(id)   ──► an authenticated user   (teams, and its auth)
                 └─ System(id) ──► an instance or server   (above teams again)
```

**The link is not burned into the node.** Which user an id stands
for is a row somebody else keeps, and it has to be, because the
binding has not happened yet — the owner of an instance is an
unbound reference until authentication binds it. Recording the
answer now would mean rewriting every node the day it arrives, and
nodes do not move.

**The kind stays here**, because a history has to be able to say
"the server diverged this" rather than "somebody did", and that is
a fact about its own record. An answer it had to go outside for
would be an answer it could fail to get.

That is not a hypothetical: a record of a selection has to say who
made it, and "a rule wrote this one" is a different answer from "a
person chose this one". Reading it off the node is what keeps the
two from being told apart by guesswork later.

**Only what can be identified gets to be an actor.** A persona is a
voice, and an agent is a thing operating on somebody's behalf;
neither can be held to a choice. If it turns out one of them should
be recorded, it arrives as a variant here or as something hanging
off a user — and either way it is somebody the forge could name.

# What this stopped recording

An act used to carry the write-side triple the rest of the codebase
uses — who, which agent carried it out, and through which entry
point. It now carries an actor, and **the agent and the entry point
are no longer on a forge node at all**. That is a loss, so it is
worth saying plainly rather than leaving somebody to discover it:
from here on, this layer cannot answer "which tool wrote this
round", and the answer is not recoverable later for anything
written in between.

It is deliberate. What a history of choices has to answer is *whose
choice this was* — the record of a selection is only worth keeping
if the answer is somebody who can be asked about it — and which
surface the request arrived through is a fact about the request. If
that turns out to be wanted here, it comes back as a second axis on
an actor, and it starts being true from the day it lands.

## Types

- `Act` — One thing somebody did, at a time.
- `Actor` — Whoever did something, as far as the forge is concerned.
- `Meta` — A thing's own history, for the part of it that no other history

