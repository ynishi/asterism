//! A JSONPath subset — enough to steer a state machine and pluck out
//! items, and deliberately no more.
//!
//! ```text
//! $.foo          object field
//! $.foo.bar      dot chain
//! $.arr[0]       array index
//! $.arr[*]       array wildcard
//! ```
//!
//! # Why a subset
//!
//! A schema-driven exporter reads a path out of caller-supplied JSON, so
//! the grammar is a public surface that has to be explainable in the
//! params example a backend author works from. Filters, slices and
//! recursive descent would each need a sentence there and none of them
//! has appeared in a real backend's response shape: the documented cases
//! are "the status field", "the error message", and "the array of
//! outputs".
//!
//! A wildcard may appear anywhere the walk can widen, not only last.
//! Nothing enforces a position because nothing needs to: the walk is a
//! breadth-first frontier, so `$.a[*].b[*]` falls out of the same loop
//! that handles one level.
//!
//! # Missing is not an error
//!
//! Every function here answers with what it found. A path that matches
//! nothing yields an empty vector or `None`, and the caller decides
//! whether that is a failure — a poll predicate reads it as "not yet",
//! while a handle extraction reads it as a backend that did not answer
//! the way its profile said it would. Returning a `Result` here would
//! push that judgement into a layer that cannot make it.
//!
//! # Where this lives and why
//!
//! It moved into the SDK when a second exporter needed it. One grammar
//! with two spellings is worse than either spelling: a profile author
//! reads one paragraph of documentation and cannot tell which adapter it
//! describes, and a fix to the wildcard in one copy leaves the other
//! wrong in a way no test in either crate can see.

use serde_json::Value;

/// One step of a parsed path.
enum PathSeg {
    /// `.name`
    Field(String),
    /// `[3]`
    Index(usize),
    /// `[*]`
    Wildcard,
}

/// Splits an expression into steps.
///
/// A leading `$` and the dot after it are optional, so `$.a.b`, `.a.b`
/// and `a.b` parse the same. An index form that is neither `*` nor a
/// number falls back to a field name rather than erroring, which keeps
/// a typo visible as "matched nothing" instead of turning into a
/// dispatch-time rejection two phases later.
fn parse(expr: &str) -> Vec<PathSeg> {
    let mut segs: Vec<PathSeg> = Vec::new();
    let expr = expr.trim();
    let expr = expr.strip_prefix('$').unwrap_or(expr);
    let expr = expr.strip_prefix('.').unwrap_or(expr);
    for raw in expr.split('.') {
        if raw.is_empty() {
            continue;
        }
        // Possible forms: `foo`, `foo[0]`, `foo[*]`, `[0]`, `[*]`
        let (field, index_part) = match raw.find('[') {
            None => (raw, ""),
            Some(open) => (&raw[..open], &raw[open..]),
        };
        if !field.is_empty() {
            segs.push(PathSeg::Field(field.to_string()));
        }
        if !index_part.is_empty() {
            let inner = index_part.trim_start_matches('[').trim_end_matches(']');
            if inner == "*" {
                segs.push(PathSeg::Wildcard);
            } else if let Ok(idx) = inner.parse::<usize>() {
                segs.push(PathSeg::Index(idx));
            } else {
                // Unsupported index form — leave as a literal field
                // fallback so grammar errors are visible.
                segs.push(PathSeg::Field(inner.to_string()));
            }
        }
    }
    segs
}

/// Every value the expression selects, in document order.
pub fn many(root: &Value, expr: &str) -> Vec<Value> {
    let segs = parse(expr);
    let mut frontier: Vec<&Value> = vec![root];
    for seg in &segs {
        let mut next: Vec<&Value> = Vec::new();
        for v in &frontier {
            match seg {
                PathSeg::Field(name) => {
                    if let Some(child) = v.get(name) {
                        next.push(child);
                    }
                }
                PathSeg::Index(idx) => {
                    if let Some(child) = v.get(*idx) {
                        next.push(child);
                    }
                }
                PathSeg::Wildcard => match v {
                    Value::Array(arr) => next.extend(arr.iter()),
                    Value::Object(obj) => next.extend(obj.values()),
                    _ => {}
                },
            }
        }
        frontier = next;
    }
    frontier.into_iter().cloned().collect()
}

/// The first value the expression selects, or `None`.
pub fn first(root: &Value, expr: &str) -> Option<Value> {
    many(root, expr).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "status": "done",
            "nested": { "deep": { "leaf": 7 } },
            "outputs": [
                { "url": "a.png", "tags": ["x", "y"] },
                { "url": "b.png", "tags": [] }
            ]
        })
    }

    #[test]
    fn a_field_and_a_dot_chain_resolve() {
        assert_eq!(first(&doc(), "$.status"), Some(json!("done")));
        assert_eq!(first(&doc(), "$.nested.deep.leaf"), Some(json!(7)));
    }

    #[test]
    fn the_dollar_and_its_dot_are_both_optional() {
        // Three spellings of one path. Profiles in the wild carry all
        // three, and a reader should not have to know which one an
        // adapter's documentation happened to use.
        for expr in ["$.status", ".status", "status"] {
            assert_eq!(first(&doc(), expr), Some(json!("done")), "{expr}");
        }
    }

    #[test]
    fn an_index_selects_one_and_a_wildcard_selects_all() {
        assert_eq!(first(&doc(), "$.outputs[0].url"), Some(json!("a.png")));
        assert_eq!(
            many(&doc(), "$.outputs[*].url"),
            vec![json!("a.png"), json!("b.png")]
        );
    }

    /// The wildcard is not restricted to the last segment: the walk is a
    /// frontier, so nesting them is the same loop running twice. Pinned
    /// because the documentation used to claim the restriction existed.
    #[test]
    fn wildcards_nest() {
        assert_eq!(
            many(&doc(), "$.outputs[*].tags[*]"),
            vec![json!("x"), json!("y")]
        );
    }

    #[test]
    fn a_path_that_matches_nothing_is_empty_rather_than_an_error() {
        assert_eq!(first(&doc(), "$.absent"), None);
        assert!(many(&doc(), "$.outputs[9].url").is_empty());
        assert!(
            many(&doc(), "$.status[*]").is_empty(),
            "scalar has no children"
        );
    }

    #[test]
    fn the_root_selects_the_whole_document() {
        // `handle_from` defaults to `$`, which has to mean "keep the
        // whole response body" rather than "no segments, so nothing".
        assert_eq!(first(&doc(), "$"), Some(doc()));
    }
}
