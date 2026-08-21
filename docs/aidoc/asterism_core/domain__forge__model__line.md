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

## Types

- `Line` — One repository: an identifier, a history, and how it settles

