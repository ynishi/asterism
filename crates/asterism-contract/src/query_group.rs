//! Query Group `query_json` — the persisted rule of a query-backed Group.
//!
//! A Query Group stores its *rule* (not its members) as a versioned
//! JSON blob on
//! the `bucket.query_json` column. The members are materialised into
//! `asset_bucket` rows by the evaluation job. This module is the typed
//! form of that blob:
//!
//! ```json
//! {
//!   "v": 1,
//!   "filter": { ...ListAssetsQuery... },
//!   "sort":   { "target": "...", "order": "...", "reverse": false },
//!   "search_text": "optional free text or null"
//! }
//! ```
//!
//! # Why a version tag
//!
//! The v1 shape replaces the legacy `saved_query.filter_json` hack — where
//! `search_text` piggybacked *inside* the filter object
//! (`App.svelte:2603-2607`) and `group_ids` were pre-expanded to a flat
//! list (`App.svelte:618/2379`). The v1 shape splits `search_text` out as
//! a first-class field and keeps `group_ids` **raw** (nesting expansion is
//! the evaluation job's re-entrant CTE, not something frozen here). The
//! explicit `v` lets the migration and every future read reject an
//! unknown shape loudly instead of silently mis-parsing.
//!
//! # `filter` reuse
//!
//! `filter` is the existing [`ListAssetsQuery`](crate::query::ListAssetsQuery)
//! verbatim — no re-definition. `group_ids` inside it are raw (un-expanded)
//! group ids; the evaluator walks the nesting graph itself.

use serde::{Deserialize, Serialize};

use crate::query::ListAssetsQuery;
use crate::sort::SortSpec;

/// The only `query_json` schema version this build understands.
pub const QUERY_JSON_VERSION: u32 = 1;

/// Typed `query_json` v1 payload.
///
/// Construct via [`QueryGroupQuery::new`] (stamps the current version) and
/// read back via [`QueryGroupQuery::parse`] (validates the version).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryGroupQuery {
    /// Schema version. Always [`QUERY_JSON_VERSION`] for this shape; any
    /// other value is rejected by [`QueryGroupQuery::parse`]. Required on
    /// the wire (a missing `v` is a parse error, not a defaulted 1) so an
    /// un-versioned legacy blob can never masquerade as v1.
    pub v: u32,
    /// Filter predicates — the existing grid list query, reused verbatim.
    /// `group_ids` here are **raw** (un-expanded); the evaluation job
    /// expands nesting via its recursive CTE.
    pub filter: ListAssetsQuery,
    /// Sort axis evaluated by the materialize job to freeze `position`.
    pub sort: SortSpec,
    /// Free-text search delegated to Tantivy. `None` (or omitted) = no
    /// full-text constraint, so the query is pure SQL filter. Split out of
    /// the filter blob (no more piggyback).
    #[serde(default)]
    pub search_text: Option<String>,
}

/// Failure modes of [`QueryGroupQuery::parse`].
#[derive(Debug)]
pub enum QueryJsonError {
    /// The JSON did not deserialise into the v1 shape.
    Parse(serde_json::Error),
    /// The blob deserialised but carried an unsupported `v`.
    UnsupportedVersion {
        /// The `v` value found on the wire.
        found: u32,
        /// The version this build supports ([`QUERY_JSON_VERSION`]).
        expected: u32,
    },
}

impl std::fmt::Display for QueryJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryJsonError::Parse(e) => write!(f, "query_json parse error: {e}"),
            QueryJsonError::UnsupportedVersion { found, expected } => write!(
                f,
                "unsupported query_json version: found {found}, expected {expected}"
            ),
        }
    }
}

impl std::error::Error for QueryJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            QueryJsonError::Parse(e) => Some(e),
            QueryJsonError::UnsupportedVersion { .. } => None,
        }
    }
}

impl QueryGroupQuery {
    /// Builds a v1 payload, stamping [`QUERY_JSON_VERSION`].
    pub fn new(filter: ListAssetsQuery, sort: SortSpec, search_text: Option<String>) -> Self {
        Self {
            v: QUERY_JSON_VERSION,
            filter,
            sort,
            search_text,
        }
    }

    /// Parses and validates a stored `query_json` string.
    ///
    /// Returns [`QueryJsonError::UnsupportedVersion`] for any `v` other
    /// than [`QUERY_JSON_VERSION`] (v1), and [`QueryJsonError::Parse`] for
    /// malformed JSON / a missing required field.
    pub fn parse(json: &str) -> Result<Self, QueryJsonError> {
        let q: QueryGroupQuery = serde_json::from_str(json).map_err(QueryJsonError::Parse)?;
        if q.v != QUERY_JSON_VERSION {
            return Err(QueryJsonError::UnsupportedVersion {
                found: q.v,
                expected: QUERY_JSON_VERSION,
            });
        }
        Ok(q)
    }

    /// Serialises back to the `query_json` string form.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sort::{SortOrder, SortTarget};

    #[test]
    fn parses_minimal_v1() {
        let json = r#"{
            "v": 1,
            "filter": { "persona_id": "p1", "group_ids": ["g1", "g2"] },
            "sort": { "target": "occurred_at", "order": "updated", "reverse": false }
        }"#;
        let q = QueryGroupQuery::parse(json).unwrap();
        assert_eq!(q.v, 1);
        assert_eq!(q.filter.persona_id.as_deref(), Some("p1"));
        // group_ids stay raw (un-expanded).
        assert_eq!(q.filter.group_ids, vec!["g1", "g2"]);
        assert_eq!(q.sort.target, SortTarget::OccurredAt);
        assert_eq!(q.sort.order, SortOrder::Updated);
        assert!(!q.sort.reverse);
        assert_eq!(q.search_text, None);
    }

    #[test]
    fn search_text_is_first_class() {
        let json = r#"{
            "v": 1,
            "filter": {},
            "sort": { "target": "tag", "order": "alpha", "reverse": true },
            "search_text": "sunset"
        }"#;
        let q = QueryGroupQuery::parse(json).unwrap();
        assert_eq!(q.search_text.as_deref(), Some("sunset"));
        assert!(q.sort.reverse);
    }

    #[test]
    fn rejects_wrong_version() {
        let json = r#"{
            "v": 2,
            "filter": {},
            "sort": { "target": "occurred_at", "order": "updated", "reverse": false }
        }"#;
        match QueryGroupQuery::parse(json) {
            Err(QueryJsonError::UnsupportedVersion { found, expected }) => {
                assert_eq!(found, 2);
                assert_eq!(expected, 1);
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_version() {
        // No `v` at all must be a parse error, never a silent v1.
        let json = r#"{
            "filter": {},
            "sort": { "target": "occurred_at", "order": "updated", "reverse": false }
        }"#;
        assert!(matches!(
            QueryGroupQuery::parse(json),
            Err(QueryJsonError::Parse(_))
        ));
    }

    #[test]
    fn round_trips_through_json() {
        let q = QueryGroupQuery::new(
            ListAssetsQuery {
                persona_id: Some("p1".into()),
                ..Default::default()
            },
            SortSpec {
                target: SortTarget::Group,
                order: SortOrder::Updated,
                reverse: false,
                collation: None,
            },
            Some("beach".into()),
        );
        let json = q.to_json().unwrap();
        let back = QueryGroupQuery::parse(&json).unwrap();
        assert_eq!(q, back);
    }
}
