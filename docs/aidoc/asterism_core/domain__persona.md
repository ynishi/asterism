# asterism-core::domain::persona

`Persona` — the primary aggregate root.

"Primary aggregate root" is realised through a `persona_id` bucket
relationship rather than object containment (a persona may accumulate
tens of thousands of assets, so nested containment does not scale). The
`Asset` aggregate holds a `persona_id` reference and the application
service enforces the cascade-delete invariant.

## Types

- `Persona` — A persona registered inside Asterism (aggregate root).

