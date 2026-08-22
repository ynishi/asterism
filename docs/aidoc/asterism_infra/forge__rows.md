# asterism-infra::forge::rows

The shapes a store keeps, and the two translations either side of
them.

Every field here is a scalar or an id. Nothing holds a [`Line`], a
[`Pursuit`], a [`ChangePoint`] or an [`Op`] — that is the whole
point of the module, and the reason it is written out rather than
replaced by keeping the domain values in a map.

The names match what the SQLite tables will be called, because the
adapter that writes those is meant to be able to read this one as a
specification of what it owes.

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
- `take_pursuit_apart` — The work's own row, and one `pursuit_node` row plus its `pursuit_op`s for
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
- `PursuitOpRow` — `pursuit_op` — one operation of one round, in the order it was written.
- `PursuitRow` — `pursuit` — one line of work.
- `ThreadMessageRow` — `forge_thread_message` — one thing said.
- `ThreadRevisionRow` — `forge_thread_revision` — a correction to something said.
- `ThreadRow` — `forge_thread` — one conversation, and what it hangs off.

