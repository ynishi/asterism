# asterism-infra::sqlite::repo

Repository adapters — SQLite implementations of the ports declared in
`asterism-core`.

Conventions shared by every adapter:

- Each adapter holds a cloneable writer `AsyncIsle` handle and issues
  queries through `isle.call`. Activating a WAL reader pool is deferred
  until read contention is measurable.
- Only `rusqlite` primitives are handled inside an isle closure;
  promotion into domain types happens outside (see the convention in
  [`crate::sqlite::map`]).
- Visibility filtering (for restricted assets) is always applied
  inside SQL by the asset adapter.

