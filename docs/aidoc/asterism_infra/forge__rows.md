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

- `read_line` — Rebuilds a line from its row, its change points and their rows.
- `read_pursuit` — Rebuilds work from its row, its nodes and their operations.
- `take_change_point_apart` — One change point's rows, for the single-node write a close makes.
- `take_close_apart` — One ending, for the append a commit makes.
- `take_new_line_apart` — The row for a line that has just been opened.
- `take_pursuit_apart` — The work's own row, and one `pursuit_node` row plus its `pursuit_op`s for
- `take_round_apart` — One pass, for the append a push makes.

## Types

- `ActRow` — An act, flattened the way a row carries one: a stamp, a handle, and
- `ChangePointRow` — `change_point` — one move of a line, without the table it carries.
- `ChangeRowRow` — `change_row` — one axis-triple of one entry, under one change
- `LineRow` — `line` — the repository, and the two things about it that are not
- `PursuitNodeRow` — `pursuit_node` — a pass or an ending.
- `PursuitOpRow` — `pursuit_op` — one operation of one pass, in the order it was written.
- `PursuitRow` — `pursuit` — one line of work.

