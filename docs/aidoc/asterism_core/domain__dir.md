# asterism-core::domain::dir

`Dir` — a persona-scoped folder tree for organising the sidebar.

Dir and Group are two deliberately orthogonal mechanisms:

- **Dir (organisation axis)** — a strict tree (`parent_id`, at most
  one parent) whose only job is to lay out the sidebar. A dir
  contains dirs and groups; it never contains assets and it never
  filters the grid on the SQL side. Selecting a dir in the UI just
  expands to the group ids beneath it and feeds the existing
  `group_ids` OR filter.
- **Group nesting (curation axis)** — the m:n `bucket_link`
  connection (see [`GroupLink`](crate::domain::group::GroupLink)),
  which is about what a collection *contains*, Are.na style.

Keeping the two apart means the query layer never has to answer
"does selecting a parent group include its descendants?" — the tree
semantics live entirely in the Dir axis and the client.

# Naming

`dir` is not a reserved word in SQLite, so unlike `Group`/`bucket`
the domain name and the table name coincide.

## Types

- `Dir` — A sidebar folder, scoped to one persona.

