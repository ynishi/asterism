//! Building an artefact's disclosure out of the library, and putting it
//! into a file.
//!
//! Two verbs, and the second is the one the acceptance criteria are
//! really about:
//!
//! - [`record_for`](ProvenanceService::record_for) — assemble what an
//!   asset discloses from what is *stored*: the container metadata a
//!   probe read, the `derived_from` edges the library recorded, the
//!   asset's own title.
//! - [`apply_to`](ProvenanceService::apply_to) — write that record into
//!   a file through the [`ProvenanceWriter`] port.
//!
//! # Why this makes the database the source of truth
//!
//! Nothing here reads the target file's metadata. The record is derived
//! entirely from rows, so a file that came back from a downstream
//! conversion with its manifest stripped can be handed to
//! [`apply_to`](ProvenanceService::apply_to) and get the same disclosure
//! again — the answer never lived in the file. That is the property a
//! manifest cannot have on its own, since any re-encode removes it.
//!
//! # Why the port is here and not in `repository`
//!
//! [`ProvenanceWriter`] is an outbound port like the repositories, and
//! it lives in the core for the same reason they do — adapters
//! implement traits, they do not define them (`asterism-infra`'s crate
//! doc). It is declared beside its only caller rather than in
//! [`repository`](crate::domain::repository) because it is not one: a
//! repository owns the storage of an entity, and this one owns no
//! entity at all — it takes a value and a path and modifies a file
//! neither it nor this service owns.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use asterism_provenance::{DisclosureRecord, Stamped};

use crate::domain::disclosure::{self, ParentEvidence};
use crate::domain::edge::EdgeKind;
use crate::domain::repository::{AssetRepository, EdgeRepository};
use crate::domain::value::AssetId;
use crate::error::DomainError;

/// Ceiling on the `derived_from` edges one record names.
///
/// A record lists what an artefact came out of, and a manifest is a
/// document somebody reads: a lineage of a hundred parents is not more
/// informative than one of eight, and each parent costs a repository
/// read to establish whether it is itself synthetic. The number is a
/// budget rather than a semantic limit, and a truncated list is
/// truncated at the shallow end — the edges come back
/// weight-descending, so what survives is what the library considers
/// the strongest links.
const MAX_PARENTS: u32 = 8;

/// Writes a disclosure into a file that already exists.
///
/// The one outbound port of this service. Implemented by
/// `asterism-infra::provenance`, which owns the containers, the XMP
/// packet and the C2PA signer; nothing about any of those reaches this
/// side of the boundary.
#[async_trait]
pub trait ProvenanceWriter: Send + Sync {
    /// Applies `record` to the file at `path`, replacing it.
    ///
    /// Returns what was actually written. The two halves of a
    /// disclosure fail independently, so a caller must be able to see
    /// which of them landed rather than inferring success from the
    /// absence of an error ([`Stamped`]).
    async fn apply(&self, path: &Path, record: &DisclosureRecord) -> Result<Stamped, DomainError>;
}

/// Assembles and applies AI-disclosure provenance.
pub struct ProvenanceService {
    assets: Arc<dyn AssetRepository>,
    edges: Arc<dyn EdgeRepository>,
    writer: Arc<dyn ProvenanceWriter>,
}

impl ProvenanceService {
    /// Builds the service over its ports.
    pub fn new(
        assets: Arc<dyn AssetRepository>,
        edges: Arc<dyn EdgeRepository>,
        writer: Arc<dyn ProvenanceWriter>,
    ) -> Self {
        Self {
            assets,
            edges,
            writer,
        }
    }

    /// Assembles what one asset discloses.
    ///
    /// `dispatch_id` is context the caller has and the library does not:
    /// an asset's `session_id` carries a dispatch id only when the asset
    /// was reified by one, and reading it as a dispatch for every asset
    /// would put a session id into a signed claim under a field that
    /// says dispatch. The export path knows its own dispatch and passes
    /// it; a re-apply months later legitimately passes `None`.
    pub async fn record_for(
        &self,
        asset_id: &AssetId,
        dispatch_id: Option<&str>,
    ) -> Result<DisclosureRecord, DomainError> {
        let asset = self
            .assets
            .find(asset_id)
            .await?
            .ok_or_else(|| DomainError::not_found("asset", asset_id))?;

        // The first material's metadata. An asset can hold several, and
        // the disclosure is about the file being exported — which is
        // the primary one. A collection has no materials at all and
        // yields no container evidence, which is the correct answer for
        // something that has no bytes.
        let meta_kv = asset
            .materials
            .first()
            .and_then(|material| material.meta_kv.clone());

        let edges = self
            .edges
            .edges_of(asset_id, Some(EdgeKind::DerivedFrom), MAX_PARENTS)
            .await?;
        let mut parents = Vec::with_capacity(edges.len());
        for edge in edges {
            // `from` is the newer asset by the write path's convention,
            // so the parent is on the far side.
            let parent_id = edge.to;
            // Whether the parent is itself synthetic is what separates
            // "a model made this" from "a model altered a photograph".
            // A parent this cannot read reads as not synthetic — the
            // weaker of the two claims, which is the right direction to
            // fail in: it can only widen `trainedAlgorithmicMedia` into
            // `compositeWithTrainedAlgorithmicMedia`, never the reverse.
            // Storage will not let an edge dangle (a foreign key refuses
            // it, pinned by `an_edge_cannot_point_at_an_asset_that_is_
            // not_there`), so this covers a row disappearing between
            // these two reads rather than a shape a caller can write.
            let parent_meta = self
                .assets
                .find(&parent_id)
                .await?
                .and_then(|parent| parent.materials.first().and_then(|m| m.meta_kv.clone()));
            parents.push(ParentEvidence {
                asset_id: parent_id.to_string(),
                synthetic: disclosure::is_synthetic(parent_meta.as_deref()),
            });
        }

        Ok(disclosure::record_for(
            &asset_id.to_string(),
            asset.title.as_deref(),
            dispatch_id,
            meta_kv.as_deref(),
            &parents,
        ))
    }

    /// Writes an asset's disclosure into a file.
    ///
    /// The file is named by the caller rather than taken from the
    /// asset's own locator: the two are different files whenever this
    /// matters. An export writes a copy somewhere else and stamps the
    /// copy, and a re-apply is pointed at whatever came back from
    /// downstream. Stamping the library's own original would be a
    /// different operation, and not one any caller has asked for.
    pub async fn apply_to(
        &self,
        asset_id: &AssetId,
        path: &Path,
        dispatch_id: Option<&str>,
    ) -> Result<Stamped, DomainError> {
        let record = self.record_for(asset_id, dispatch_id).await?;
        self.writer.apply(path, &record).await
    }
}

// The behaviour of this service is tested in
// `crates/asterism-infra/tests/provenance_service.rs`, against the real
// SQLite repositories rather than fakes. That is not a preference: what
// there is to get wrong here — which material's metadata is read, which
// side of a `derived_from` edge is the parent, what a purged parent does
// to the term — is precisely the part a hand-written fake would encode
// its author's assumption of, and the assumption is the thing under
// test. The mapping those answers feed, which is pure, is tested beside
// itself in `domain::disclosure`.
