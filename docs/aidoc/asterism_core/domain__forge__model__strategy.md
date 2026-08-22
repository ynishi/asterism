# asterism-core::domain::forge::model::strategy

How a line settles a collision — the contract, and not the rule.

```text
  collision ──► Strategy::resolve ──► Ok(complete ops)  ──► a round carries them
                      │                                     and the collision is gone
                      └──────────────► Err              ──► no op is written at all
```

When work moves an axis the line moved first, the answer is not to
pick a winner. It is to put a second entry on the line beside the
first, so both candidates survive and the history stays one chain —
and then which one lives is an ordinary later choice rather than a
decision somebody is cornered into now.

# Nothing here says how

There is more than one sensible way to do that: keep the line's
side and mint a new entry for the work's, keep the work's side and
move the line's, mint new entries for both. Which one a line uses
is a setting, and what the divergent entry ends up being called
follows from it. **None of that is the model's to decide**, so this
module holds the contract and no rule that satisfies it.

What the model does require is one sentence: a strategy decides an
id and a name, and returns them as operations complete enough to go
straight into a round. There is no half-written entry, no entry that
gets a name later, and no path that writes some operations and then
fails — a refusal writes nothing, and the work stays open carrying
its collision.

# Naming belongs to the rule

A divergent entry is born named, because an entry with no name is a
shape only this one path would have, and every rule about names
would need an exception for it. What it is called depends on how
the split was made, so the rule that split it is what names it.

A rule that cannot decide a name on its own — because the answer
depends on something outside the forge — holds whatever it needs to
ask, and whoever assembles it supplies that. The model does not
see it either way.

# Refusing is a real outcome

[`StrategyError`] has two cases: no name is available, and the rule
cannot decide. Both leave the work exactly
as it was, open and colliding, which is a state somebody can act on
by hand. What they must not leave is half a divergence.

# Doing nothing is a rule, not a missing one

A line where nobody wants entries minted automatically points at a
rule that returns no operations. That is a rule like any other, and
it keeps "this line does not settle collisions by itself" from
being a flag every caller has to branch on.

# A rule is picked, so it says what it is

Choosing how a line settles is somebody's decision, and a list of
slugs is not something anybody can choose from. Every rule carries
an [`About`] — what it is called, and what it does to which side —
and carries it itself, so the list a person reads is built out of
the rules that exist rather than out of a table of labels that has
to be kept in step with them.

What a line records is the [`id`](Strategy::id), never the label.
A rule can be renamed, translated or reworded without moving a
single line off it.

## Types

- `About` — What a rule says about itself, for whoever is choosing one.
- `Divergence` — What a rule is given to work from.
- `StrategyError` — Why a rule wrote nothing.

## Traits

- `Strategy` — One rule for turning a collision into a divergence.

