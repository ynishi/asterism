# asterism-core::domain::forge::model::value

The values the model is made of.

Nothing here knows how it is stored. A value that cannot be written
down without a column is not a value, and every rule these carry is
one the model states.

Two of them are worth reading before the rest of the module:

**[`Content`] is the only reference the forge holds downward.** It
wraps an asset id behind a private field, which is what lets every
other type in the model carry a reference into the layer below
without naming what it refers to. The forge compares content and
moves it around; reaching the thing itself is the boundary's
business, and keeping the vocabulary out of the model is what stops
the two sides from growing into each other.

**[`Name`] promises one thing and refuses to promise more.** It is
trimmed and never blank, and two names match when their trimmed
forms match exactly — nothing else is normalised, since names are
chosen by people who can see the ones already there. Where a name
has to be unique is deliberately absent: that needs an owner to
answer, and the owner is outside the forge.

The ids are surrogate and minted, never derived from content.
[`EntryId`] in particular is minted by work that has changed nothing
yet, which is what lets a later round point at what an earlier one
proposed.

## Types

- `ActorId` — Surrogate id for whoever did something — the forge's handle on
- `ChangePointId` — Surrogate id for a node of a line's history — its genesis, or
- `Content` — The one reference the forge holds into the layer below.
- `EntryId` — Surrogate id for an `Entry` — the thing a line names, minted
- `Existence` — Whether a row puts its entry on the line or takes it off.
- `LineId` — Surrogate id for a `Line` — one repository.
- `MessageId` — Surrogate id for a `Message`.
- `Name` — A name something can answer to: trimmed, and never blank.
- `NodeId` — Surrogate id for a node of a pursuit — where it opened, one
- `PursuitId` — Surrogate id for a `Pursuit` — one line of work.
- `StrategyId` — Which rule a line settles collisions by.
- `ThreadId` — Surrogate id for a `Thread` — one run of messages about one

