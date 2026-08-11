//! Constellation edge planning — pure domain logic that decides how a
//! `target` asset should connect to its `candidates`.
//!
//! The `edge_rebuild` job (in `asterism-infra`) fetches candidates, then
//! calls [`plan_edges`] here to compute the edges. Keeping this step pure
//! (no I/O) lets us pin the weight rules with unit tests.
//!
//! ## Weight rules (v1)
//!
//! | Condition                                | Kind             | Weight                | Label                    |
//! |------------------------------------------|------------------|-----------------------|--------------------------|
//! | Same `bundle_id`                         | `TimeProximity`  | 1.0                   | `same-bundle`            |
//! | `|Δt| < 24h`                             | `TimeProximity`  | 0.7                   | `same-day`               |
//! | `|Δt| < 48h`                             | `TimeProximity`  | 0.5                   | `near`                   |
//! | Shared keywords (n)                      | `KeywordOverlap` | `min(0.4 + 0.1n, 0.9)`| `shared-keyword: <top>`  |
//! | Different persona + `|Δt| < 24h`         | `CoPresence`     | 0.5                   | `co-presence`            |
//!
//! `bundle_id` replaced the old `session_id` grouping key after the
//! Session-model refactor:
//! Session is now the Dialog-modality 1st-class entity and its id
//! is scoped to a single conversation, so grouping edges by it would
//! collapse the "tape bundle / journal kind bundle / PNG note pair"
//! constellations the edge fabric was designed for. `bundle_id` is
//! modality-agnostic and carries that role verbatim.
//!
//! For each `(to, kind)` pair only the highest-weighted edge is kept. The
//! window restriction on candidates (bundle id / ±48h) is the fetcher's
//! responsibility — this function assumes they are already trimmed.

use std::collections::HashMap;

use crate::domain::asset::Asset;
use crate::domain::edge::{ConstellationEdge, EdgeKind};

const HOUR_MS: i64 = 3_600_000;

/// Plans constellation edges from `target` to each `candidate`. `target` is
/// always the `from` side; the target itself is filtered out even if it
/// appears in `candidates` (self-loops are rejected).
pub fn plan_edges(target: &Asset, candidates: &[Asset]) -> Vec<ConstellationEdge> {
    let mut best: HashMap<(uuid::Uuid, EdgeKind), ConstellationEdge> = HashMap::new();

    for candidate in candidates {
        if candidate.id == target.id {
            continue;
        }
        for planned in pair_edges(target, candidate) {
            let key = (*planned.to.as_uuid(), planned.kind);
            let keep = match best.get(&key) {
                Some(existing) => planned.weight.unwrap_or(0.0) > existing.weight.unwrap_or(0.0),
                None => true,
            };
            if keep {
                best.insert(key, planned);
            }
        }
    }

    let mut edges: Vec<_> = best.into_values().collect();
    // Deterministic output order: weight desc, then target id asc. Keeps
    // both tests and the rendered UI stable.
    edges.sort_by(|a, b| {
        b.weight
            .unwrap_or(0.0)
            .partial_cmp(&a.weight.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.to.as_uuid().cmp(b.to.as_uuid()))
    });
    edges
}

/// Enumerates the edges that hold between `target` and a single
/// `candidate` (applies the weight rule table).
fn pair_edges(target: &Asset, candidate: &Asset) -> Vec<ConstellationEdge> {
    let mut edges = Vec::new();
    let diff_ms =
        (target.occurred_at.timestamp_millis() - candidate.occurred_at.timestamp_millis()).abs();

    // TimeProximity: same-bundle > same-day > near; keep only the strongest
    // classification as a single edge. Grouping key is `bundle_id` after
    // the Session-model refactor — `session_id` is Dialog-only and now
    // scoped to a single conversation, unusable as an edge fabric key.
    let time_edge = if let (Some(a), Some(b)) = (&target.bundle_id, &candidate.bundle_id) {
        (a == b).then_some((1.0f32, "same-bundle"))
    } else {
        None
    }
    .or(if diff_ms < 24 * HOUR_MS {
        Some((0.7, "same-day"))
    } else if diff_ms < 48 * HOUR_MS {
        Some((0.5, "near"))
    } else {
        None
    });
    if let Some((weight, label)) = time_edge
        && let Ok(mut edge) =
            ConstellationEdge::new(target.id, candidate.id, EdgeKind::TimeProximity)
    {
        edge.weight = Some(weight);
        edge.label = Some(label.to_string());
        edges.push(edge);
    }

    // KeywordOverlap axis.
    let shared: Vec<&str> = target
        .keywords
        .iter()
        .filter(|k| candidate.keywords.contains(k))
        .map(|k| k.as_str())
        .collect();
    if !shared.is_empty()
        && let Ok(mut edge) =
            ConstellationEdge::new(target.id, candidate.id, EdgeKind::KeywordOverlap)
    {
        edge.weight = Some((0.4 + 0.1 * shared.len() as f32).min(0.9));
        edge.label = Some(format!("shared-keyword: {}", shared[0]));
        edges.push(edge);
    }

    // CoPresence: two personas that co-occur within a day.
    if target.persona_id != candidate.persona_id
        && diff_ms < 24 * HOUR_MS
        && let Ok(mut edge) = ConstellationEdge::new(target.id, candidate.id, EdgeKind::CoPresence)
    {
        edge.weight = Some(0.5);
        edge.label = Some("co-presence".to_string());
        edges.push(edge);
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::{BundleId, Keyword, Modality, PersonaId, SourceKind, SourceRef};
    use chrono::{TimeZone, Utc};

    fn asset(persona: PersonaId, at_ms: i64) -> Asset {
        Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "x.md").unwrap(),
            Some(Modality::new(Modality::STATE).unwrap()),
            Utc.timestamp_millis_opt(at_ms).unwrap(),
            // Edge planning reads time / persona / keywords only; the
            // fixture records nobody so the axis under test is the only
            // thing varying.
            &crate::domain::attribution::AttributionContext::unrecorded(),
        )
    }

    #[test]
    fn same_bundle_beats_same_day() {
        let persona = PersonaId::new();
        let mut target = asset(persona, 0);
        target.bundle_id = Some(BundleId::new("bundle-1").unwrap());
        let mut near = asset(persona, HOUR_MS);
        near.bundle_id = Some(BundleId::new("bundle-1").unwrap());

        let edges = plan_edges(&target, &[near]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::TimeProximity);
        assert_eq!(edges[0].weight, Some(1.0));
        assert_eq!(edges[0].label.as_deref(), Some("same-bundle"));
    }

    #[test]
    fn session_id_no_longer_fires_time_proximity() {
        // Session membership (now `container_id`, session-model v2) is not
        // an edge grouping key — only `bundle_id` is. Two assets with no
        // shared bundle_id fall back to same-day TimeProximity.
        let persona = PersonaId::new();
        let target = asset(persona, 0);
        let near = asset(persona, HOUR_MS);
        let edges = plan_edges(&target, &[near]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::TimeProximity);
        assert_eq!(edges[0].weight, Some(0.7));
        assert_eq!(edges[0].label.as_deref(), Some("same-day"));
    }

    #[test]
    fn keyword_overlap_and_co_presence_stack_as_separate_kinds() {
        let target_persona = PersonaId::new();
        let other_persona = PersonaId::new();
        let mut target = asset(target_persona, 0);
        target.keywords = vec![Keyword::new("release").unwrap()];
        let mut other = asset(other_persona, 2 * HOUR_MS);
        other.keywords = vec![Keyword::new("release").unwrap()];

        let edges = plan_edges(&target, &[other]);
        let kinds: Vec<_> = edges.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&EdgeKind::TimeProximity), "same day");
        assert!(kinds.contains(&EdgeKind::KeywordOverlap));
        assert!(
            kinds.contains(&EdgeKind::CoPresence),
            "cross-persona co-presence"
        );
    }

    #[test]
    fn far_candidates_and_self_are_excluded() {
        let persona = PersonaId::new();
        let target = asset(persona, 0);
        let far = asset(persona, 100 * HOUR_MS);
        let edges = plan_edges(&target, &[target.clone(), far]);
        assert!(edges.is_empty());
    }
}
