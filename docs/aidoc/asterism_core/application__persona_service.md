# asterism-core::application::persona_service

`PersonaService` — use cases for the persona lifecycle.

Weak CQRS split: reads (`list`) go through a projection; writes
(`register` / `set_archived` / `trash` / `restore` / `purge`) enforce
the invariants.

Deleting a persona is a two-step verb like everywhere else in the
trash model: `trash` takes the persona and its live assets out of
sight (reversibly, keyed on a shared stamp), and only `purge` — which
refuses a live persona — lets the DB cascade do its irreversible
work. `archived` is a different thing entirely: a sidebar visibility
toggle over data that is still live.

Every write here takes an [`AttributionContext`] it does not persist:
no persona / theme / profile column carries attribution, and none is
being added (see the [`application`](crate::application) module doc
for why the argument is required anyway).

## Types

- `PersonaService` — Persona use-case service. Shared as an `Arc` through Tauri state and

