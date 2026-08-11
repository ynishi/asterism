//! `ProvenanceRef` — how a re-ingested artefact names where it came from.
//!
//! An img2img output is a *new file*. Nothing inside it points back at
//! the picture it was made from: the generator wrote its own bytes, and
//! whatever metadata the input carried did not survive the round trip
//! (a different tool, a different container, a re-encode). So the link
//! cannot be recovered from the artefact — it has to be *declared* by
//! whoever ran the chain.
//!
//! That declaration is this module. The ingest side accepts one string,
//! and its scheme says how to resolve it:
//!
//! | form | meaning |
//! |---|---|
//! | `asset:<uuid>` | the parent asset, named directly |
//! | `dispatch:<uuid>` | every asset that dispatch produced (an export is a dispatch, so this names "what I sent out") |
//! | `sidecar` | read the `<file>.meta.json` sitting next to the input |
//!
//! # Why there is no receipt id
//!
//! An earlier draft minted an "export receipt" to hand out. It turned
//! out to be redundant: an export *is* a dispatch, its outputs are
//! reified as assets, and both already have ids. A receipt would only
//! have added a shorter token and an expiry rule — not worth a second
//! id space that can disagree with the first.
//!
//! # Why a string and not a typed field per form
//!
//! The value is carried *outside* Asterism — through a shell pipeline,
//! a note in a chat, an n8n variable, a person's clipboard. Whatever
//! survives that trip has to be one opaque token that can be copied,
//! and a scheme prefix is how the receiver still knows what it holds.
//! It is the same reasoning as the `sha256:` prefix on
//! [`content_hash`](crate::domain::content_hash): the value declares
//! how to read it, so a second form can land later without a second
//! column.
//!
//! # Why parsing is separate from resolving
//!
//! Parsing is total and pure — it can say "this is a receipt id" without
//! a database. Resolving needs repositories and can legitimately fail
//! (the receipt expired, the parent was purged). Keeping them apart
//! means the ingest path can record *what was claimed* even when it
//! cannot confirm it, which is the behaviour that matters: a broken
//! link is not a reason to refuse the file.

use crate::domain::value::{AssetId, DispatchId};
use crate::error::DomainError;

/// Key under which the ingest path records the provenance claim on
/// [`Asset::extra`](crate::domain::asset::Asset::extra).
///
/// Underscore-prefixed to sit beside `_dispatch` (written by the
/// dispatch runner) without colliding with an importer's own keys,
/// which live at the top level of the same bag.
pub const TRACE_KEY: &str = "_trace";

/// `_trace.source` vocabulary — which channel a provenance claim
/// arrived through. A *bookkeeping of origin*, not a trust ranking:
/// caller-trust is the ingest regime, and hardening belongs to the
/// transport layer if it ever comes.
///
/// The value is derived structurally from where the claim entered, so
/// no caller asserts it:
///
/// | value | channel |
/// |---|---|
/// | `embedded` | dug out of the artefact's own surroundings — an ingest-time `sidecar` claim the importer detected next to the file |
/// | `pushed` | reported with the payload at ingest time — an `asset:` / `dispatch:` claim carried on `AddAssetCommand.derived_from` by whoever ran the chain |
/// | `manual` | declared after the fact through `DeclareProvenanceCommand` (`POST /assets/{id}/provenance`), regardless of form |
pub mod source {
    /// Claim detected in the artefact's own surroundings at ingest.
    pub const EMBEDDED: &str = "embedded";
    /// Claim pushed with the ingest payload by the caller.
    pub const PUSHED: &str = "pushed";
    /// Claim declared after the fact on an existing asset.
    pub const MANUAL: &str = "manual";
}

/// Every field inside `_trace` that a provenance claim owns.
///
/// `_trace` is shared. A provenance claim writes some of it; the
/// declared content hash writes `declared_hash`; a fold writes `fold`
/// and `absorbed`. Recording a claim has to replace **its own** fields
/// — a re-declaration that left the previous claim's `dispatch_id`
/// behind would name a hop the current claim never went through — and
/// leave everything else exactly as it was.
///
/// The list exists because the claim writer cannot ask a note which
/// fields it *might* have written: `Unresolved` produces `reason` and
/// no `form`, `Resolved` the other way round, and a claim that once
/// resolved through a dispatch and now does not has to lose the
/// `dispatch_id` it is no longer entitled to. So the set is named here
/// rather than derived from whatever this particular note happens to
/// carry.
pub const CLAIM_FIELDS: &[&str] = &[
    "derived_from",
    "form",
    "resolved",
    "claim",
    "dispatch_id",
    "reason",
    "source",
    "operator",
    "relation",
];

/// Sidecar vocabulary, re-exported from the contract crate.
///
/// The exporter writes the block and this module reads it, and the two
/// crates cannot see each other — a divergence between them would fail
/// silently as "no identity in this sidecar", so both take the words
/// from the same place.
pub use asterism_contract::sidecar::{SIDECAR_IDENTITY_KEY, SIDECAR_SUFFIX};

/// What a claim asserts about the artefact and the thing it names
/// (a claim is
/// `{relation, parent_ref, channel, state}` — this is the first field).
///
/// The other three were built first and this one was not, so every
/// claim so far has meant [`DerivedFrom`](Self::DerivedFrom) by
/// construction. That is a stronger sentence than a person often means.
/// "I made this with those two in front of me" is a real, useful piece
/// of provenance — and it is *not* the assertion that this came out of
/// those, which is what a `derived_from` edge says and what nothing in
/// the corpus can contradict afterwards
/// ([`EdgeKind::DerivedFrom`](crate::domain::edge::EdgeKind::DerivedFrom)).
/// Without a weaker word the only way to record the weaker fact was to
/// overstate it, or to not record it at all.
///
/// # Why this is a closed set
///
/// Unlike `Modality` / `SourceKind`, which are open slugs so a new
/// consumer is a data change, each value here has to map onto an
/// [`EdgeKind`](crate::domain::edge::EdgeKind) — a closed enum the
/// storage and the rebuild scoping both switch on. An unrecognised
/// relation has no edge to become, so [`parse`](Self::parse) refuses it
/// rather than guessing, for the reason
/// [`EdgeKind::parse`](crate::domain::edge::EdgeKind::parse) refuses
/// its own unknowns.
///
/// # Why the platform is irrelevant here
///
/// The lineage Asterism keeps is its own: "this was made from / with
/// these", declared by whoever knew. It is deliberately not a reading
/// of any external identity system — xmpMM, C2PA and the rest are
/// channels a claim can *arrive* on ([`source`]), never the substrate
/// it is stored in. So this vocabulary carries no trace of them, and a
/// claim declared by hand and a claim dug out of an embedded packet
/// produce the same edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClaimRelation {
    /// The artefact **came out of** what it names — an img2img output
    /// and its input, a render and its source. The default, because it
    /// is what every claim written before this type existed meant.
    #[default]
    DerivedFrom,
    /// The artefact was **made with** what it names in view, without
    /// the claim that any of it passed through a machine. A person
    /// working from two references is the case; whether a generator
    /// ever read those bytes is a separate question this word does not
    /// answer.
    Reference,
}

impl ClaimRelation {
    /// Slug, spelled the same as the edge kind it becomes.
    ///
    /// Deliberately the same strings rather than a second vocabulary
    /// that has to be kept in step: a claim's relation and the edge it
    /// writes are the same statement at two layers.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DerivedFrom => "derived_from",
            Self::Reference => "reference",
        }
    }

    /// The edge this relation becomes.
    ///
    /// The mapping lives here, in one place, so the two vocabularies
    /// cannot drift into meaning different things — the alternative is
    /// a `match` at each call site, and the second one added is where
    /// they stop agreeing.
    pub fn edge_kind(&self) -> crate::domain::edge::EdgeKind {
        match self {
            Self::DerivedFrom => crate::domain::edge::EdgeKind::DerivedFrom,
            Self::Reference => crate::domain::edge::EdgeKind::Reference,
        }
    }

    /// Reads a relation slug; unknown values are refused (see the type
    /// doc on why this set is closed).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug.trim() {
            "derived_from" => Ok(Self::DerivedFrom),
            "reference" => Ok(Self::Reference),
            other => Err(DomainError::Validation(format!(
                "unknown provenance relation: {other:?} \
                 (expected \"derived_from\" or \"reference\")"
            ))),
        }
    }
}

/// A declared origin for an artefact being (re-)ingested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceRef {
    /// The parent asset, named by its own id. The shortest form —
    /// available to any caller that still holds the id it exported.
    Asset(AssetId),
    /// Every asset a dispatch produced. One token covers an N-output
    /// export, which is what "I sent this batch out and this came
    /// back" looks like from the caller's side.
    Dispatch(DispatchId),
    /// "The answer is next to the file" — resolve by reading the
    /// `<filename>.meta.json` sidecar the exporter wrote alongside the
    /// payload.
    Sidecar,
}

/// Scheme for [`ProvenanceRef::Asset`].
const ASSET_SCHEME: &str = "asset:";
/// Scheme for [`ProvenanceRef::Dispatch`].
const DISPATCH_SCHEME: &str = "dispatch:";
/// Whole-token form for [`ProvenanceRef::Sidecar`] (it addresses a
/// location relative to the artefact, so it carries no id of its own).
const SIDECAR_TOKEN: &str = "sidecar";

/// Reads a declaration.
///
/// Errors are `Validation` and describe the accepted forms — the caller
/// is expected to turn them into a recorded "unresolved" note rather
/// than a failed ingest, so the message is what the user will read when
/// they go looking for why a link is missing.
pub fn parse(spec: &str) -> Result<ProvenanceRef, DomainError> {
    let spec = spec.trim();
    if spec == SIDECAR_TOKEN {
        return Ok(ProvenanceRef::Sidecar);
    }
    if let Some(rest) = spec.strip_prefix(ASSET_SCHEME) {
        let id = uuid::Uuid::parse_str(rest.trim())
            .map(AssetId::from_uuid)
            .map_err(|e| {
                DomainError::Validation(format!(
                    "derived_from {spec:?} is not a usable asset id: {e}"
                ))
            })?;
        return Ok(ProvenanceRef::Asset(id));
    }
    if let Some(rest) = spec.strip_prefix(DISPATCH_SCHEME) {
        let id = uuid::Uuid::parse_str(rest.trim())
            .map(DispatchId::from_uuid)
            .map_err(|e| {
                DomainError::Validation(format!(
                    "derived_from {spec:?} is not a usable dispatch id: {e}"
                ))
            })?;
        return Ok(ProvenanceRef::Dispatch(id));
    }
    Err(DomainError::Validation(format!(
        "derived_from {spec:?} has no known scheme \
         (expected \"asset:<uuid>\", \"dispatch:<uuid>\" or \"sidecar\")"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_form_yields_the_named_parent() {
        let parent = AssetId::new();
        let spec = format!("asset:{parent}");
        assert_eq!(parse(&spec).unwrap(), ProvenanceRef::Asset(parent));
    }

    #[test]
    fn dispatch_form_names_a_whole_export() {
        let dispatch = DispatchId::new();
        assert_eq!(
            parse(&format!("dispatch:{dispatch}")).unwrap(),
            ProvenanceRef::Dispatch(dispatch)
        );
    }

    #[test]
    fn sidecar_form_is_the_bare_word() {
        assert_eq!(parse("sidecar").unwrap(), ProvenanceRef::Sidecar);
    }

    #[test]
    fn surrounding_whitespace_survives_a_copy_paste() {
        // The token travels through shells and chat windows; a stray
        // space is the most ordinary damage it takes.
        let parent = AssetId::new();
        assert_eq!(
            parse(&format!("  asset:{parent} ")).unwrap(),
            ProvenanceRef::Asset(parent)
        );
        assert_eq!(parse(" sidecar\n").unwrap(), ProvenanceRef::Sidecar);
    }

    #[test]
    fn a_bare_uuid_is_rejected_rather_than_guessed() {
        // Guessing the scheme would make `receipt:<uuid>` and a bare
        // uuid mean the same thing right up until receipts start using
        // uuids too, at which point old callers would silently resolve
        // against the wrong table.
        let bare = AssetId::new().to_string();
        let err = parse(&bare).unwrap_err().to_string();
        assert!(err.contains("no known scheme"), "{err}");
    }

    #[test]
    fn a_malformed_asset_id_says_so_instead_of_falling_through() {
        let err = parse("asset:not-a-uuid").unwrap_err().to_string();
        assert!(err.contains("not a usable asset id"), "{err}");
    }

    #[test]
    fn a_malformed_dispatch_id_says_so_instead_of_falling_through() {
        let err = parse("dispatch:").unwrap_err().to_string();
        assert!(err.contains("not a usable dispatch id"), "{err}");
        let err = parse("dispatch:nope").unwrap_err().to_string();
        assert!(err.contains("not a usable dispatch id"), "{err}");
    }

    #[test]
    fn a_relation_round_trips_through_its_slug() {
        for relation in [ClaimRelation::DerivedFrom, ClaimRelation::Reference] {
            assert_eq!(ClaimRelation::parse(relation.as_str()).unwrap(), relation);
        }
    }

    #[test]
    fn a_relation_is_spelled_the_same_as_the_edge_it_becomes() {
        // The two vocabularies are one statement at two layers. If they
        // drift, a claim recorded as `reference` writes an edge stored
        // under some other word and the `_trace` note stops describing
        // the graph it produced.
        for relation in [ClaimRelation::DerivedFrom, ClaimRelation::Reference] {
            assert_eq!(relation.as_str(), relation.edge_kind().as_str());
        }
    }

    #[test]
    fn the_default_relation_is_what_earlier_claims_meant() {
        // Every claim written before this type existed produced a
        // `derived_from` edge. A different default would silently
        // re-interpret them.
        assert_eq!(ClaimRelation::default(), ClaimRelation::DerivedFrom);
    }

    #[test]
    fn an_unknown_relation_is_refused_rather_than_defaulted() {
        // Defaulting would turn a typo into the *stronger* of the two
        // claims, which is the direction that cannot be walked back.
        let err = ClaimRelation::parse("derived-from")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown provenance relation"), "{err}");
        assert!(
            err.contains("derived_from"),
            "the message names the accepted forms: {err}"
        );
    }

    #[test]
    fn the_retired_receipt_scheme_is_not_silently_accepted() {
        // An earlier draft had `receipt:<id>`. If a stale token turns
        // up, it must fail loudly rather than parse as something else.
        let err = parse("receipt:01KYGVYWAQ").unwrap_err().to_string();
        assert!(err.contains("no known scheme"), "{err}");
    }
}
