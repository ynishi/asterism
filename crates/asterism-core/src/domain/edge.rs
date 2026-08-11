//! `ConstellationEdge` — the backbone of the hover-burst experience.
//!
//! One edge represents an asset-to-asset relationship that surfaces when the
//! user hovers a card. The `edge_rebuild` job persists edges incrementally,
//! scoped to a window around each asset (same session id or ±48h) so we
//! avoid an O(n²) full scan. Are.na-style "same channel" connections are
//! not stored here — they are derived from the `asset_tag` table on demand.

use crate::domain::value::{AssetId, EdgeId};
use crate::error::DomainError;

/// Axis along which an edge is created.
///
/// The slug form is the shared vocabulary between the DB layer and DTOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// Same session id, or two assets that occurred within a short time
    /// window of each other.
    TimeProximity,
    /// Shared keyword or topic.
    KeywordOverlap,
    /// Co-occurrence across personas around the same time.
    CoPresence,
    /// Recurring cadence (weekly meetings, monthly reviews, and so on).
    Cadence,
    /// Explicit reference — the Heptabase-style backlink, and the
    /// weaker half of the provenance vocabulary.
    ///
    /// Written by a claim whose
    /// [`ClaimRelation`](crate::domain::provenance::ClaimRelation) is
    /// `reference`: *this was made with that in view*. The difference
    /// from [`DerivedFrom`](Self::DerivedFrom) is the strength of the
    /// sentence, not the confidence in it — `DerivedFrom` asserts that
    /// one artefact came out of another, which is a claim about a
    /// mechanism; this one says only that somebody worked with the
    /// other in front of them, which stays true whether or not any of
    /// its bytes were ever read by anything.
    ///
    /// Until 2026-08-06 nothing in the application wrote this kind —
    /// [`is_synth`](Self::is_synth) below has described it as "a user
    /// declaring a backlink" the whole time, and the declaration verb
    /// could only produce `DerivedFrom`. So a person recording the
    /// weaker fact had to overstate it or drop it.
    Reference,
    /// The `from` asset was derived from `to` by an outbound
    /// [`Exporter`][exporter-note] — img2img output pointing back at
    /// the source photo, a baked LoRA pointing back at its training
    /// set, a Gemini multimodal response pointing back at the prompt
    /// images. Written by the dispatch runner as it reifies each
    /// [`Derived`][derived-note] into a new `Asset`; a Selection with
    /// N inputs produces N `DerivedFrom` edges per output.
    ///
    /// [exporter-note]: `asterism-dispatch-sdk::Exporter`
    /// [derived-note]: `asterism-dispatch-sdk::Derived`
    DerivedFrom,
    /// The two assets were observed to hold **the same bytes** — a
    /// fact about their content, not a verdict about their identity.
    ///
    /// This is the difference from [`DerivedFrom`](Self::DerivedFrom),
    /// and it is the whole reason the kind exists. `DerivedFrom` says
    /// one thing came out of another; nothing about the corpus can
    /// contradict it later. `IdenticalTo` says only "these hashed to
    /// the same value". Whether that means *the same asset* is decided
    /// per event, by a person or by the lane's `on_duplicate` Strategy
    /// declaration: Asterism is a collection / production
    /// library where the same bytes legitimately arrive twice as two
    /// different things (a Re-In round trip, a deliberate variant, a
    /// dispatch product), so no global "same hash = same asset"
    /// invariant is imposed.
    ///
    /// The consequence to hold on to: **a pair ruled apart keeps this
    /// edge.** `fold_policy = keep` means a person looked and said
    /// these are different things — and the byte-level coincidence is
    /// still true, still worth being able to trace, and is exactly
    /// what stops the conflict from being re-discovered as news. An
    /// edge here is therefore *not* evidence that anything was folded;
    /// the headstone (`asset.folded_into`) is where a fold is
    /// recorded.
    ///
    /// # Direction
    ///
    /// The relation is symmetric, the storage is not: `edge` is keyed
    /// `UNIQUE(from_asset, to_asset, kind)`
    /// (`migrations.rs:103`), so a direction has to be chosen. The
    /// rule is **younger → older** (`occurred_at`, then id as the
    /// tie-break): `from` is the newer of the two rows. On the ingest
    /// path that is the arrival which raised the conflict, so it reads
    /// as the sentence detection makes ("this new import is identical
    /// to that"), and it matches `DerivedFrom`, whose `from` is
    /// likewise the younger row.
    ///
    /// Age rather than "whichever row was being hashed" because the
    /// two part company on the backfill walk, which reaches rows in
    /// storage order and can hash the *older* half second. Orienting by
    /// arrival there would write `(older, newer)` for a pair the ingest
    /// path writes as `(newer, older)` — the same symmetric fact in two
    /// rows, which is exactly what the next bullet says nothing but
    /// this rule prevents.
    ///
    /// Two things follow, and both are load-bearing:
    ///
    /// - **Readers must query both sides.** The incumbent — the row a
    ///   user is far more likely to be looking at — is on the `to`
    ///   side, so [`EdgeRepository::edges_of`][of] (outgoing only)
    ///   cannot see the link from there. The read path is
    ///   [`edges_incident`][incident], which returns an
    ///   [`IncidentEdge`] carrying which side matched; symmetric pairs
    ///   collapse through [`dedupe_incident_pairs`].
    /// - **Writers must not orient it any other way.** The UNIQUE key
    ///   is over the ordered pair, so `(A,B)` and `(B,A)` are two rows
    ///   the database is happy to hold at once — one symmetric fact
    ///   stored twice, and no constraint can catch it. Nothing but this
    ///   rule keeps the pair single.
    ///
    /// # Label
    ///
    /// [`ConstellationEdge::label`] carries the **axis** on which the
    /// two agreed, written from
    /// [`DuplicateAxis::as_str`](crate::domain::duplicate_conflict::DuplicateAxis::as_str)
    /// so the label and the queue row it accompanies say one word:
    /// `"artefact"` (every byte), `"content"` (only the bytes that
    /// decide the decoded result) or `"meta"` (the metadata that
    /// definition drops). The artefact axis was labelled `"file"` until
    /// V64 rewrote the stored labels; that spelling is gone, and
    /// `DuplicateAxis::parse` no longer answers to it.
    ///
    /// One row per pair means one slot, so the axes are ordered rather
    /// than accumulated: `"artefact"` is the stronger claim and implies
    /// the other two, so a later content-axis match on a pair already
    /// labelled `"artefact"` leaves the label alone, while an
    /// artefact-axis match on a pair labelled `"content"` upgrades it.
    /// The label is never a list.
    ///
    /// [of]: crate::domain::repository::EdgeRepository::edges_of
    /// [incident]: crate::domain::repository::EdgeRepository::edges_incident
    IdenticalTo,
}

impl EdgeKind {
    /// Slug representation shared by the DB schema and DTOs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TimeProximity => "time_proximity",
            Self::KeywordOverlap => "keyword_overlap",
            Self::CoPresence => "co_presence",
            Self::Cadence => "cadence",
            Self::Reference => "reference",
            Self::DerivedFrom => "derived_from",
            Self::IdenticalTo => "identical_to",
        }
    }

    /// Whether the `edge_rebuild` job owns this kind.
    ///
    /// Two populations share the `edge` table and they have opposite
    /// lifecycles:
    ///
    /// - **Synth** (this returns `true`) — recomputed from observable
    ///   signals (timestamps, keywords, personas). They are disposable
    ///   by design: the job throws the old set away and derives a fresh
    ///   one whenever an input changes.
    /// - **Provenance** ([`Reference`](Self::Reference),
    ///   [`DerivedFrom`](Self::DerivedFrom),
    ///   [`IdenticalTo`](Self::IdenticalTo)) — *asserted* once by
    ///   someone who knew something the data does not contain (an
    ///   exporter reifying its output, a user declaring a backlink, a
    ///   re-ingest quoting its parent). Nothing can recompute them, so
    ///   a rebuild that deletes them destroys the only copy.
    ///
    /// `IdenticalTo` sits on the asserted side even though a hash
    /// comparison *looks* recomputable: the comparison is only run
    /// when a material is fingerprinted, over a window the rebuild
    /// knows nothing about (same persona, any age), so a rebuild that
    /// dropped it would leave a conflict a user has already ruled on
    /// with no record that it was ever raised.
    ///
    /// The write path uses this to scope its delete
    /// ([`EdgeRepository::replace_synth_edges_of`][port]); before that
    /// scoping existed the rebuild wiped `derived_from` edges as
    /// collateral.
    ///
    /// [port]: crate::domain::repository::EdgeRepository::replace_synth_edges_of
    pub fn is_synth(&self) -> bool {
        match self {
            Self::TimeProximity | Self::KeywordOverlap | Self::CoPresence | Self::Cadence => true,
            Self::Reference | Self::DerivedFrom | Self::IdenticalTo => false,
        }
    }

    /// Every kind the rebuild owns, for adapters that need the set as
    /// data (a SQL `IN` list, for instance) rather than as a predicate.
    pub fn synth_kinds() -> &'static [EdgeKind] {
        &[
            Self::TimeProximity,
            Self::KeywordOverlap,
            Self::CoPresence,
            Self::Cadence,
        ]
    }

    /// Parses a slug (unknown values yield a validation error).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "time_proximity" => Ok(Self::TimeProximity),
            "keyword_overlap" => Ok(Self::KeywordOverlap),
            "co_presence" => Ok(Self::CoPresence),
            "cadence" => Ok(Self::Cadence),
            "reference" => Ok(Self::Reference),
            "derived_from" => Ok(Self::DerivedFrom),
            "identical_to" => Ok(Self::IdenticalTo),
            other => Err(DomainError::Validation(format!(
                "unknown edge kind: {other:?}"
            ))),
        }
    }
}

/// An edge connecting two assets.
///
/// Invariants: `from != to` (enforced at construction) and `(from, to,
/// kind)` is unique (enforced by the repository and by a DB constraint).
#[derive(Debug, Clone, PartialEq)]
pub struct ConstellationEdge {
    /// Surrogate id (UUID v7).
    pub id: EdgeId,
    /// Asset the burst starts from.
    pub from: AssetId,
    /// Asset the burst lands on.
    pub to: AssetId,
    /// Axis along which the edge was created.
    pub kind: EdgeKind,
    /// Optional human-readable label shown alongside the burst target
    /// (for example "same session" or "shared keyword: X").
    pub label: Option<String>,
    /// Optional weight; the top-N burst uses this as its sort key.
    pub weight: Option<f32>,
}

impl ConstellationEdge {
    /// Builds an edge, rejecting `from == to`.
    pub fn new(from: AssetId, to: AssetId, kind: EdgeKind) -> Result<Self, DomainError> {
        if from == to {
            return Err(DomainError::Validation(
                "ConstellationEdge must connect two distinct assets".into(),
            ));
        }
        Ok(Self {
            id: EdgeId::new(),
            from,
            to,
            kind,
            label: None,
            weight: None,
        })
    }
}

/// Which side of a [`ConstellationEdge`] a given asset sits on.
///
/// Edges are written unidirectionally by the `edge_rebuild` job (only
/// the newer asset's rebuild sees the older sibling), so the hover
/// burst has to query both directions and remember which side was
/// looked up. The dispatch layer uses this hint to pick the correct
/// "burst target" (the *other* side) and to expose a directional
/// signal to the UI (Outgoing / Incoming / Both after dedupe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    /// The queried asset is the `from` side of the edge; the burst
    /// target lives at `edge.to`.
    Outgoing,
    /// The queried asset is the `to` side of the edge; the burst
    /// target lives at `edge.from`.
    Incoming,
    /// Two symmetric edges — one `Outgoing`, one `Incoming` — were
    /// collapsed on the same `(other_asset, kind)` pair during
    /// dedupe. Signals "confirmed link from both sides" to the UI.
    Both,
}

impl EdgeDirection {
    /// Slug used in the wire DTO.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Both => "both",
        }
    }
}

/// An edge as seen from one endpoint's perspective — the pair
/// [`ConstellationEdge`] returned by a bidirectional query plus the
/// [`EdgeDirection`] telling the caller which side of the edge the
/// queried asset is on.
///
/// The dispatch layer collapses two symmetric [`IncidentEdge`]s that
/// share `(other_asset, kind)` into a single [`Both`](EdgeDirection::Both)
/// entry (weight aggregated by max) — this is what makes the hover
/// burst symmetric even though the underlying storage stays
/// one-directional.
#[derive(Debug, Clone, PartialEq)]
pub struct IncidentEdge {
    /// The stored edge row.
    pub edge: ConstellationEdge,
    /// Which endpoint of `edge` the queried asset was matched on.
    pub direction: EdgeDirection,
}

impl IncidentEdge {
    /// Returns the asset id on the *other* side of [`edge`](Self::edge)
    /// relative to the queried asset — this is the id the hover
    /// burst wants to render as a linked card.
    pub fn other_side(&self) -> AssetId {
        match self.direction {
            EdgeDirection::Outgoing | EdgeDirection::Both => self.edge.to,
            EdgeDirection::Incoming => self.edge.from,
        }
    }

    /// Symmetric pair key: `(min(from,to), max(from,to), kind)`. Two
    /// `IncidentEdge`s that share this key are the same conceptual
    /// link seen from both sides and get collapsed by
    /// [`dedupe_incident_pairs`].
    pub fn pair_key(&self) -> (AssetId, AssetId, EdgeKind) {
        let (a, b) = if self.edge.from <= self.edge.to {
            (self.edge.from, self.edge.to)
        } else {
            (self.edge.to, self.edge.from)
        };
        (a, b, self.edge.kind)
    }
}

/// Collapses symmetric `Outgoing` + `Incoming` pairs sharing the same
/// `(other_asset, kind)` into a single `Both` entry, keeping the
/// higher-weight side's `label` and the max of the two weights.
///
/// Preserves the input ordering of the first-seen edge per pair — the
/// caller keeps its weight-descending ordering intact.
pub fn dedupe_incident_pairs(edges: Vec<IncidentEdge>) -> Vec<IncidentEdge> {
    use std::collections::HashMap;
    let mut by_key: HashMap<(AssetId, AssetId, EdgeKind), usize> = HashMap::new();
    let mut out: Vec<IncidentEdge> = Vec::with_capacity(edges.len());
    for inc in edges {
        let key = inc.pair_key();
        if let Some(idx) = by_key.get(&key).copied() {
            // Two symmetric sides — collapse into Both, take max
            // weight (NULL loses), prefer the heavier side's label.
            let existing = &mut out[idx];
            let (w_hi, label_hi) = pick_heavier(existing, &inc);
            existing.edge.weight = w_hi;
            existing.edge.label = label_hi;
            existing.direction = EdgeDirection::Both;
        } else {
            by_key.insert(key, out.len());
            out.push(inc);
        }
    }
    out
}

fn pick_heavier(a: &IncidentEdge, b: &IncidentEdge) -> (Option<f32>, Option<String>) {
    match (a.edge.weight, b.edge.weight) {
        (Some(wa), Some(wb)) if wa >= wb => (Some(wa), a.edge.label.clone()),
        (Some(_), Some(wb)) => (Some(wb), b.edge.label.clone()),
        (Some(wa), None) => (Some(wa), a.edge.label.clone()),
        (None, Some(wb)) => (Some(wb), b.edge.label.clone()),
        (None, None) => (None, a.edge.label.clone().or_else(|| b.edge.label.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(
        from: AssetId,
        to: AssetId,
        kind: EdgeKind,
        weight: Option<f32>,
        label: &str,
    ) -> ConstellationEdge {
        let mut e = ConstellationEdge::new(from, to, kind).unwrap();
        e.weight = weight;
        e.label = Some(label.into());
        e
    }

    #[test]
    fn edge_rejects_self_loop() {
        let a = AssetId::new();
        assert!(ConstellationEdge::new(a, a, EdgeKind::TimeProximity).is_err());
        assert!(ConstellationEdge::new(a, AssetId::new(), EdgeKind::Cadence).is_ok());
    }

    #[test]
    fn incident_edge_other_side_flips_by_direction() {
        let a = AssetId::new();
        let b = AssetId::new();
        let e = edge(a, b, EdgeKind::TimeProximity, Some(1.0), "same-session");
        let out = IncidentEdge {
            edge: e.clone(),
            direction: EdgeDirection::Outgoing,
        };
        assert_eq!(out.other_side(), b);
        let inc = IncidentEdge {
            edge: e,
            direction: EdgeDirection::Incoming,
        };
        assert_eq!(inc.other_side(), a);
    }

    #[test]
    fn dedupe_collapses_symmetric_pair_into_both() {
        let a = AssetId::new();
        let b = AssetId::new();
        let outgoing = IncidentEdge {
            edge: edge(a, b, EdgeKind::TimeProximity, Some(0.7), "outgoing-side"),
            direction: EdgeDirection::Outgoing,
        };
        let incoming = IncidentEdge {
            edge: edge(b, a, EdgeKind::TimeProximity, Some(1.0), "incoming-side"),
            direction: EdgeDirection::Incoming,
        };
        let collapsed = dedupe_incident_pairs(vec![outgoing, incoming]);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].direction, EdgeDirection::Both);
        // Heavier side wins on both weight and label.
        assert_eq!(collapsed[0].edge.weight, Some(1.0));
        assert_eq!(collapsed[0].edge.label.as_deref(), Some("incoming-side"));
    }

    #[test]
    fn dedupe_leaves_asymmetric_edges_alone() {
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();
        let only_outgoing = IncidentEdge {
            edge: edge(a, b, EdgeKind::KeywordOverlap, Some(0.5), "shared-kw"),
            direction: EdgeDirection::Outgoing,
        };
        let only_incoming = IncidentEdge {
            edge: edge(c, a, EdgeKind::TimeProximity, Some(0.9), "same-day"),
            direction: EdgeDirection::Incoming,
        };
        let out = dedupe_incident_pairs(vec![only_outgoing.clone(), only_incoming.clone()]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].direction, EdgeDirection::Outgoing);
        assert_eq!(out[1].direction, EdgeDirection::Incoming);
    }

    #[test]
    fn dedupe_treats_different_kinds_as_distinct() {
        // Same asset pair on two different kinds must not collapse.
        let a = AssetId::new();
        let b = AssetId::new();
        let e_time = IncidentEdge {
            edge: edge(a, b, EdgeKind::TimeProximity, Some(1.0), "same-session"),
            direction: EdgeDirection::Outgoing,
        };
        let e_kw = IncidentEdge {
            edge: edge(a, b, EdgeKind::KeywordOverlap, Some(0.4), "shared-kw"),
            direction: EdgeDirection::Outgoing,
        };
        let out = dedupe_incident_pairs(vec![e_time, e_kw]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedupe_preserves_first_seen_ordering() {
        // Heavier edge appears first in input; after collapse it keeps
        // the caller's position (weight-desc ordering upstream).
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();
        let heavy = IncidentEdge {
            edge: edge(a, b, EdgeKind::TimeProximity, Some(1.0), "same-session"),
            direction: EdgeDirection::Outgoing,
        };
        let light = IncidentEdge {
            edge: edge(a, c, EdgeKind::TimeProximity, Some(0.5), "same-day"),
            direction: EdgeDirection::Outgoing,
        };
        let sym_of_heavy = IncidentEdge {
            edge: edge(b, a, EdgeKind::TimeProximity, Some(0.9), "outgoing-side"),
            direction: EdgeDirection::Incoming,
        };
        let out = dedupe_incident_pairs(vec![heavy, light, sym_of_heavy]);
        assert_eq!(out.len(), 2);
        // heavy stays at position 0, symmetric pair merged.
        assert_eq!(out[0].direction, EdgeDirection::Both);
        assert_eq!(out[0].edge.weight, Some(1.0));
        assert_eq!(out[1].direction, EdgeDirection::Outgoing);
    }

    #[test]
    fn provenance_kinds_are_not_owned_by_the_rebuild() {
        // The rebuild deletes what it owns. Anything asserted rather
        // than derived has to answer `false` here or the assertion is
        // lost the next time a keyword changes.
        assert!(EdgeKind::TimeProximity.is_synth());
        assert!(EdgeKind::KeywordOverlap.is_synth());
        assert!(EdgeKind::CoPresence.is_synth());
        assert!(EdgeKind::Cadence.is_synth());
        assert!(!EdgeKind::DerivedFrom.is_synth());
        assert!(!EdgeKind::Reference.is_synth());
        // A hash agreement is observed once, over a window the rebuild
        // does not scan; recomputing it is not something the rebuild
        // could do, so deleting it loses the only record that a
        // conflict was ever raised.
        assert!(!EdgeKind::IdenticalTo.is_synth());
    }

    #[test]
    fn the_synth_kind_list_and_the_predicate_agree() {
        // The adapter builds its `DELETE ... IN (…)` from the list
        // while the domain reasons with the predicate; a kind added to
        // one and not the other would silently change what a rebuild
        // destroys.
        for kind in EdgeKind::synth_kinds() {
            assert!(kind.is_synth(), "{kind:?} listed but not synth");
        }
        for kind in [
            EdgeKind::TimeProximity,
            EdgeKind::KeywordOverlap,
            EdgeKind::CoPresence,
            EdgeKind::Cadence,
            EdgeKind::Reference,
            EdgeKind::DerivedFrom,
            EdgeKind::IdenticalTo,
        ] {
            assert_eq!(
                kind.is_synth(),
                EdgeKind::synth_kinds().contains(&kind),
                "{kind:?} disagrees between list and predicate"
            );
        }
    }

    /// The slug is the vocabulary the DB column and the DTOs share, so
    /// it round-trips both ways or a stored row stops parsing.
    #[test]
    fn identical_to_round_trips_through_its_slug() {
        assert_eq!(EdgeKind::IdenticalTo.as_str(), "identical_to");
        assert_eq!(
            EdgeKind::parse("identical_to").unwrap(),
            EdgeKind::IdenticalTo
        );
        // Near-misses are refused rather than folded into the new kind:
        // a slug nobody writes must not start meaning something.
        assert!(EdgeKind::parse("identical").is_err());
        assert!(EdgeKind::parse("identicalTo").is_err());
    }

    /// Every kind's slug survives `as_str` → `parse`, and no two share
    /// one. Written over the full list because the failure it guards is
    /// a copy-paste arm (`Self::IdenticalTo => "derived_from"`), which
    /// no single-kind test can see.
    #[test]
    fn every_kind_has_its_own_slug_and_parses_back() {
        let kinds = [
            EdgeKind::TimeProximity,
            EdgeKind::KeywordOverlap,
            EdgeKind::CoPresence,
            EdgeKind::Cadence,
            EdgeKind::Reference,
            EdgeKind::DerivedFrom,
            EdgeKind::IdenticalTo,
        ];
        let slugs: std::collections::HashSet<&str> = kinds.iter().map(|k| k.as_str()).collect();
        assert_eq!(slugs.len(), kinds.len(), "two kinds share one slug");
        for kind in kinds {
            assert_eq!(EdgeKind::parse(kind.as_str()).unwrap(), kind);
        }
    }

    #[test]
    fn edge_direction_slug_is_stable() {
        assert_eq!(EdgeDirection::Outgoing.as_str(), "outgoing");
        assert_eq!(EdgeDirection::Incoming.as_str(), "incoming");
        assert_eq!(EdgeDirection::Both.as_str(), "both");
    }
}
