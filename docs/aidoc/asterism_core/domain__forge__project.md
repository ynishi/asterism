# asterism-core::domain::forge::project

`Project` — the repo of the forge's git analogy (#63): the shared
context pursuits file under, and the owner of a mainline.

The pursuit answers "one line of work"; the project answers "one
body of work" — the unit a mainline is canonical *for*. Without it
the mainline would be a persona-wide singleton, and two unrelated
efforts would fight over one namespace of living names. With it,
scope has a referent (an `In(Existing)` of another project's living
asset is cross-project contamination, catchable at IN time) and
"the canonical set" means canonical for something in particular.

# Shape

- [`Project`] is a thin, immutable row: identity, persona, a
  required human name, an optional note. No status column, no
  members — what belongs to a project derives through its pursuits
  and its mainline, never from a membership table.
- [`Mainline`](crate::domain::forge::mainline::Mainline) rows are
  the project's lines. v1 mints exactly one, named
  [`MAIN`](crate::domain::forge::mainline::Mainline::MAIN), in the
  same transaction as the project (application-enforced); the
  schema admits more so a later multi-line model is an enum's
  worth of change, not a migration.

# Invariants (service-enforced, entity-checked where local)

- `name` is non-blank (checked here); uniqueness among one
  persona's projects is an application rule, checked where both
  rows are visible — like living-name uniqueness on a mainline,
  and unlike a schema UNIQUE, so a later archival verb can free
  names without a migration.
- A pursuit filing under a project shares its persona
  (cross-aggregate, application service — the persona cascade
  rule every forge pairing states).

## Types

- `Project` — The repo above the pursuits. Thin and immutable: identity plus a

