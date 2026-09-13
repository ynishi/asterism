# asterism-core::application::import_state_service

Reading and writing where an importer got to.

Thin on purpose, and the thinness is the design. Every judgement
about a resumption point — whether a scan has one at all, whether a
run earned the right to move it, what the offset means — is made in
the importer, which is the only place that can make it. This service
is the other end of the wire: it stores what it is given and hands
back what it stored.

Two things it deliberately does not do. It does not validate the
offset, because the offset belongs to the adapter that wrote it and
a validator here would be this side inventing an opinion about a
shape it cannot know — see [`import_state`](crate::domain::import_state)
for the rule and why this is where it would break first. And it does
not check whether the persona exists: an import that reaches this
point has been posting assets to that persona all along, and a
second existence check here would answer a question the write path
already answered, in a place with nothing to do about the answer.

## Types

- `ImportStateService` — Where an importer got to, read and written.

