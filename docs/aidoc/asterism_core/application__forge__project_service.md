# asterism-core::application::forge::project_service

Project use cases — opening the context work files under, and
reading it back (#63 decisions 1–2).

**Deprecated.** The model this serves is replaced by
[`model`](crate::domain::forge::model), where a line is the top and
nothing groups lines inside the forge.

Thin next to [`legacy_pursuit_service`](super::legacy_pursuit_service), and it
stays that way while the merge is unwritten: a project has no
lifecycle of its own. It is opened, it is read, and everything that
happens *to* it happens through the pursuits filed under it.

Two rules live here rather than in the schema, for opposite
reasons. **Name uniqueness among one persona's projects** is
application-side and read-checked, so two callers racing can both
find the name free and both write it — the rule is advisory under
concurrency, and closing that is a schema decision (a partial
UNIQUE) rather than a service one. **The line minted with the
project** is the other way round: the repository writes both in one
transaction, and this layer only decides that there is exactly one
and what it is called.

## Types

- `ProjectService` — Project use-case service.

