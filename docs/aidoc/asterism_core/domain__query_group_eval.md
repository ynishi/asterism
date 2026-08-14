# asterism-core::domain::query_group_eval

Query Group evaluation — the pure pieces of the materialize pipeline.

The full pipeline (parse → nesting expand → SQL filter → sort → bulk
materialize) is orchestrated in the application layer
([`QueryGroupService`](crate::application::query_group_service)). The
cycle guard below is the piece of it that is pure domain logic and
lives here so it can be unit tested with no I/O.

# There is no intersection step any more

A `search_text`-bearing rule used to be evaluated as "SQL filter ∩
retrieval shortlist", and this module held the pure half of that
intersection. It is gone: the rule's text is now
`AssetQuery::text_match`, a `WHERE` term resolved in SQL beside the
other predicates. A Query
Group is a persistent set definition, and a retrieval shortlist is
neither complete nor deterministic, so it could not define one.

## Functions

- `dependency_graph` — Builds the composite dependency graph from containment edges
- `reaches` — Whether `target` is reachable from `start` by following dependency

## Types

- `DependencyGraph` — Dependency graph over groups for the query-cycle guard.

