//! Query Group evaluation — the pure pieces of the materialize pipeline.
//!
//! The full pipeline (parse → nesting expand → SQL filter → sort → bulk
//! materialize) is orchestrated in the application layer
//! ([`QueryGroupService`](crate::application::query_group_service)). The
//! cycle guard below is the piece of it that is pure domain logic and
//! lives here so it can be unit tested with no I/O.
//!
//! # There is no intersection step any more
//!
//! A `search_text`-bearing rule used to be evaluated as "SQL filter ∩
//! retrieval shortlist", and this module held the pure half of that
//! intersection. It is gone: the rule's text is now
//! `AssetQuery::text_match`, a `WHERE` term resolved in SQL beside the
//! other predicates. A Query
//! Group is a persistent set definition, and a retrieval shortlist is
//! neither complete nor deterministic, so it could not define one.

use std::collections::{HashMap, HashSet};

use crate::domain::value::GroupId;

/// Dependency graph over groups for the query-cycle guard.
///
/// Edge `u → v` means "evaluating `u` depends on `v`":
///
/// - **containment** — a `bucket_link` parent depends on its children
///   (the nesting closure pulls the child's members in), and
/// - **query reference** — a query group depends on every raw group id
///   in its rule's `filter.group_ids`.
///
/// A cycle in the composite graph makes refresh a mutual-trigger loop
/// that no debounce can stop, so writes that would close a cycle are
/// rejected at both write sites (rule save / `bucket_link` link). The
/// graph is built by the service from the persona's links + query rules
/// and checked with [`reaches`]; both are pure so they unit-test with
/// no I/O. Check-then-write is race-free on the serialized writer (the
/// same condition the existing bucket_link cycle CTE relies on).
pub type DependencyGraph = HashMap<GroupId, Vec<GroupId>>;

/// Builds the composite dependency graph from containment edges
/// (`parent → child` pairs) and query rules (`(group, referenced raw
/// group ids)` pairs).
pub fn dependency_graph(
    containment: impl IntoIterator<Item = (GroupId, GroupId)>,
    query_refs: impl IntoIterator<Item = (GroupId, Vec<GroupId>)>,
) -> DependencyGraph {
    let mut graph: DependencyGraph = HashMap::new();
    for (parent, child) in containment {
        graph.entry(parent).or_default().push(child);
    }
    for (group, refs) in query_refs {
        graph.entry(group).or_default().extend(refs);
    }
    graph
}

/// Whether `target` is reachable from `start` by following dependency
/// edges (including `start == target` via a non-empty path — pass the
/// candidate edge's endpoints to ask "would adding `X → start` close a
/// cycle?" as `reaches(graph, start, X)`).
pub fn reaches(graph: &DependencyGraph, start: &GroupId, target: &GroupId) -> bool {
    let mut stack: Vec<GroupId> = vec![*start];
    let mut seen: HashSet<GroupId> = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == *target {
            return true;
        }
        if !seen.insert(node) {
            continue;
        }
        if let Some(next) = graph.get(&node) {
            stack.extend(next.iter().copied());
        }
    }
    false
}

#[cfg(test)]
mod cycle_tests {
    use super::*;

    fn g() -> GroupId {
        GroupId::new()
    }

    #[test]
    fn direct_query_reference_cycle_detected() {
        // a --query-ref--> b; adding b --query-ref--> a must be caught:
        // reaches(graph, a, b)? asking for the reverse direction.
        let (a, b) = (g(), g());
        let graph = dependency_graph([], [(a, vec![b])]);
        // b's new refs would include a → check a reachable from a's ref
        // target … the caller asks: does any of b's new refs reach b?
        // new ref = a; a → b exists ⇒ cycle.
        assert!(reaches(&graph, &a, &b));
    }

    #[test]
    fn indirect_cycle_via_containment_detected() {
        // q --query-ref--> parent --contains--> q  (q references a
        // manual group that contains q as a nested child).
        let (q, parent) = (g(), g());
        let graph = dependency_graph([(parent, q)], []);
        // q's candidate refs = [parent]; parent reaches q ⇒ cycle.
        assert!(reaches(&graph, &parent, &q));
    }

    #[test]
    fn acyclic_reference_passes() {
        let (a, b, c) = (g(), g(), g());
        let graph = dependency_graph([(b, c)], [(a, vec![b])]);
        // c depends on nothing; from c we reach neither a nor b.
        assert!(!reaches(&graph, &c, &a));
        assert!(!reaches(&graph, &c, &b));
    }

    #[test]
    fn self_is_reachable_only_through_edges() {
        let a = g();
        let graph = dependency_graph([], []);
        // No edges: a "reaches" a trivially by the start==target check —
        // callers must not ask reaches(g, x, x) for the self-reference
        // case; they check each candidate ref instead (a ref list
        // containing the group itself asks reaches(graph, self, self)
        // which is true, correctly rejecting self-reference).
        assert!(reaches(&graph, &a, &a));
    }
}
