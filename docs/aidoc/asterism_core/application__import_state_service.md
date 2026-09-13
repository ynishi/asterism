# asterism-core::application::import_state_service

Reading and writing where an importer got to.

Thin on purpose, and the thinness is the design. Every judgement
about a resumption point is made in the importer, which is the only
place that can make one. This service is the other end of the wire:
it stores what it is given and hands back what it stored.

Two things it deliberately does not do. It does not validate the
offset — see [`import_state`](crate::domain::import_state) for that
rule. And it does not check whether the persona exists: a write
arrives from an import that has been posting assets to that persona
all along, so the check would repeat one the asset path has already
made; and a read arrives before anything has been posted, where the
honest answer to an unknown persona is the same as to a known one
with nothing stored — nothing.

## Types

- `ImportStateService` — Where an importer got to, read and written.

