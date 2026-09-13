# asterism-core::domain::import_definition

An import somebody can run without typing it, and the record of
having run one.

#295 gave an import a memory and #297 gave it a source that can
refuse; neither made anything *start* one. A definition is what a
person types once so that nothing has to type it again, and a run is
what happened the times it went.

## A definition is a stored command line

Which is exactly where a credential wants to go, and the one thing
this must be built not to hold. [`ImportDefinition::secret_ref`]
carries the **name** of an environment variable and never its value,
following `auth.secret_ref` on the outbound side for the reason
stated there: a value resolved when the importer starts is in
neither this row nor anything written down afterwards.

The arguments beside it are stored verbatim and are readable by
anything that can read the definition. That is a real trade and it
is the operator's to make knowingly, so it is said on the field
rather than left to be discovered.

## A run is written whatever happened

Including for one whose importer never started. A definition that
looks as though it was never run, when somebody asked for it and
something went wrong, is the failure mode a record of runs exists to
prevent — and "the binary is not there" is the most likely first
thing to go wrong on a machine nobody has configured yet.

## Types

- `ImportDefinition` — An import that can be run on demand.
- `ImportRun` — What happened one time an import was run.
- `RunOutcome` — How a run ended.

