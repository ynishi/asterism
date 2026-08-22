# asterism-core::domain::forge::model::line

The line — the forge's top entity.

A line is a repository: one canonical history, and everything that
is on it derives from that history.

```text
  Line ── history ──▶ what the line carries   (a chain, folded)
   │
   └──── meta ──────▶ what the line is        (created / renamed)
```

# The top of the forge

Every rule the forge states is stated per line: what a name is
unique among, where a pursuit files, how collisions settle, what
the canonical set is. A line is therefore the largest thing the
forge has an opinion about.

Grouping and ownership are outside it, with the model of teams and
members that holds them, and the forge answers nothing about whose
line this is.

# The name, and what it does not claim

A line keeps an identifier, because a line has to be callable by
something a person chose, and reaching outside for a string on
every read would put the display of a line somewhere other than the
line.

What it does not keep is any claim about that name. "Unique among
what?" needs an owner to answer — a person's lines, a team's lines
— and the owner is outside, so uniqueness is enforced where the
namespace lives. [`Name`] promises exactly one thing: it is not
blank. Where there is only ever one line, it is [`Line::ROOT`].

# Two records, and neither moves the other

[`Line::record`] moves the history. [`Line::rename`] and
[`Line::set_strategy`] move the description. A rename is not a
change point — the history says what happened to what the line
carries, and a rename did not — and recording one does not touch
[`Meta`], because "the line moved" and "the line is described
differently" would otherwise collapse into one value that answers
neither question.

# Standing, and why it is not in the history

```text
  Open ──archive──▶ Archived ──(drop)──▶ gone
    ▲                   │                  │
    └──── reopen ───────┘        what it held is released
  takes change points   readable, holds
                        everything still
```

[`Standing`] sits beside the name and the strategy rather than in
the chain, for the same reason a rename is not a change point: the
history says what happened to what the line *carries*, and "this
line is finished with" is a statement about the line. It moves
[`Meta`] and nothing else.

**Dropping is reachable only through the archive**, as purging is
reachable only through the trash everywhere else here. Two steps,
because the second one is irreversible and takes the history with
it.

# What a line holds, and why anything cares

[`Line::holds`] is every [`Content`] any change point on the line
has ever named. It is not a cache and not a second record — it is a
fold, like everything else about a line — but it is the one fold
something outside the forge has to act on: **while a line holds a
content, the layer that keeps the bytes may not let it go.**

An entry taken off the line does not release what it held. The
change point that put it there is still in the chain and still
names it, and the chain is not rewritten. So the set only ever
grows, and the only thing that shrinks it is dropping the line.

That is deliberate rather than unfortunate. A line says what is on
it *now*: `alive`, under this name, at this content. A line saying
that about bytes somebody deleted is a line telling a lie about the
present, which is a different thing from a log of past events —
those stay true whatever happens to what they name, which is why
the ledger this model replaced could name an asset without holding
it.

# Rewriting is not a verb here

There is no filter, no rebase, no editing a change point after the
fact. Wanting one is usually wanting to release a content without
dropping everything, and the answer is that the same result is
reachable with the verbs that already exist: open a new line, and
put on it what should have been there. That is a new history rather
than an edited one, which is what it honestly is — the change
points of a filtered line could not name the work they came out
of, because that work asked for something else.

What the old line then needs is to be archived and dropped, which
is the pair above.

## Types

- `Line` — One repository: an identifier, a history, and how it settles
- `Standing` — Whether a line is still being worked on.

