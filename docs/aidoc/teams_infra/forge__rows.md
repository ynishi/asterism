# teams-infra::forge::rows

The shapes this plane's forge store keeps, and the two translations
either side of them.

Every field here is a scalar or an id. Nothing holds a [`Line`], a
[`Pursuit`], a [`ChangePoint`] or an [`Op`] — that is the whole
point of the module, and the reason it is written out rather than
replaced by keeping the domain values in a map.

The names match what the tables are called, because the adapter
that writes those is meant to be able to read this one as a
specification of what it owes.

# This is the local plane's module again, and that is deliberate

`asterism-infra` holds one of these, shared by the adapters that
live there. This plane cannot reach it: `asterism-infra` is on #83
§4's never-list, and the edge #148 decision 20 opens is to
`asterism-core` alone. So the team's adapter carries its own copy,
which is the cost the decision names rather than one it overlooked.

What keeps the copy honest is the same thing that keeps the schemas
honest: it stays *literally* parallel, so a divergence is a diff
somebody can read rather than behaviour somebody meets. Both halves
are thin — taking a domain value apart is field access, and putting
one back is [`restore`], which lives in `asterism-core` and is
shared. The rules neither copy holds are the model's.

Two things are deliberately not parallel, and they are the two
worth knowing before diffing this against its source. **The doc
comments are shortened**: the arguments they make are the same, and
the long form is the one over in `asterism-infra`, so prose that
differs there is prose, not drift — the field lists, the function
signatures and the bodies are what a diff is for. **There is no
`team_id`**: scope is the adapter's, the column is the adapter's,
and a row shape that carried it would put the team into every
signature the port deliberately leaves it out of.

The tests below come across with the module, because they ask
things no store can reach — a message that already carries a
correction is not something any port hands over — and a copy that
kept the translation and dropped them would leave the same cases
unasked here and answered there.

## Functions

- `anchor_columns` — Flattens an anchor into the columns a store keeps it in.
- `read_line` — Rebuilds a line from its row, its change points and their rows.
- `read_pursuit` — Rebuilds work from its row, its nodes and their operations.
- `read_thread` — Rebuilds a conversation from its row, what was said, and every
- `take_anchor_apart` — The head row of a thread, anchor flattened into its five columns.
- `take_change_point_apart` — One change point's rows, for the single-node write a close makes.
- `take_close_apart` — One ending, for the append a commit makes.
- `take_message_apart` — One thing said, for the append `say` makes.
- `take_new_line_apart` — The row for a line that has just been opened.
- `take_pursuit_apart` — The work's own row, and one `pursuit_node` row plus its
- `take_revision_apart` — One correction, for the append `amend` makes.
- `take_revisions_apart` — Every correction to one message, oldest first.
- `take_round_apart` — One round, for the append a push makes.
- `take_thread_apart` — A whole conversation, for the write that opens one.

## Types

- `ActRow` — An act, flattened the way a row carries one: a stamp, a handle, and
- `AnchorColumns` — What a thread hangs off, as the five columns that carry it.
- `ChangePointRow` — `change_point` — one move of a line, without the table it carries.
- `ChangeRowRow` — `change_row` — one axis-triple of one entry, under one change
- `LineRow` — `line` — the repository, and the three things about it that are not
- `PursuitNodeRow` — `pursuit_node` — a round or an ending.
- `PursuitOpRow` — `pursuit_op` — one operation of one round, in the order it was
- `PursuitRow` — `pursuit` — one line of work.
- `ThreadMessageRow` — `forge_thread_message` — one thing said.
- `ThreadRevisionRow` — `forge_thread_revision` — a correction to something said.
- `ThreadRow` — `forge_thread` — one conversation, and what it hangs off.

