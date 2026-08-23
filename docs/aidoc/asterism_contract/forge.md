# asterism-contract::forge

The forge's wire shapes — a line, what is on it, and what a caller
asks it to do.

A topic module rather than rows in [`command`](crate::command) and
[`dto`](crate::dto): the forge's vocabulary answers to a model of
its own, and a reader of [`ForgeLineDto`] needs the types beside it
more than it needs the asset DTO two hundred lines up.

# The model these are a projection of

A line is a repository with one canonical history: a genesis and a
chain of change points, each carrying a table keyed by entry and
axis over three axes — existence, content, name. Nothing here holds
what that history answers. [`ForgeLineDto`] carries the line's own
fields and the id of its head; what is *on* the line is
[`ForgeEntryStateDto`], folded on read, and the chain that produced
it is [`ForgeLineHistoryDto`].

Two reads exist because both are real questions, and a screen wants
the fold. The history grows with the line and is for something
showing how a line got where it is.

## Types

- `CloseForgePursuitCommand` — Ends a piece of work.
- `ForgeChangePointDto` — One node of a line's history.
- `ForgeChangeRowDto` — What one landing said about one entry.
- `ForgeCloseDto` — How a piece of work ended.
- `ForgeCollisionDto` — One axis of one entry that this work asks to move and the line has
- `ForgeDiscardedDto` — What dropping a line released.
- `ForgeEntryStateDto` — Where one entry stands on a line, folded from the whole chain.
- `ForgeLineActCommand` — Archives a line, reopens one, or drops one — the three verbs whose
- `ForgeLineDto` — A line, without what is on it.
- `ForgeLineHistoryDto` — A line's whole history: where it began and every landing since.
- `ForgeOpDto` — One operation of a round.
- `ForgePursuitActCommand` — Asks the line's rule to answer whatever this work collides with.
- `ForgePursuitDto` — A piece of work against a line, whole: how it opened, every round
- `ForgeResolvedDto` — What `resolve` did.
- `ForgeRoundDto` — One round of work — what it asks the line to say, and who asked.
- `ForgeStrategyDto` — A rule a line can be pointed at.
- `OpenForgeLineCommand` — Opens a line.
- `OpenForgePursuitCommand` — Opens work against a line.
- `PushForgeRoundCommand` — Writes a round.
- `RenameForgeLineCommand` — Renames a line. The name is the line's own description, so this is
- `SetForgeLineStrategyCommand` — Points a line at a different rule.

