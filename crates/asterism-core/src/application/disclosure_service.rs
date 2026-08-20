//! Building an artefact's disclosure out of the library, and putting it
//! into a file.
//!
//! Two verbs, and the second is the one the acceptance criteria are
//! really about:
//!
//! - [`record_for`](DisclosureService::record_for) — assemble what an
//!   asset discloses from what is *stored*: the container metadata a
//!   probe read, the `derived_from` edges the library recorded, the
//!   asset's own title.
//! - [`apply_to`](DisclosureService::apply_to) — write that record into
//!   a file through the [`DisclosureWriter`] port.
//!
//! # Why this makes the database the source of truth
//!
//! Nothing here reads the target file's metadata. The record is derived
//! entirely from rows, so a file that came back from a downstream
//! conversion with its manifest stripped can be handed to
//! [`apply_to`](DisclosureService::apply_to) and get the same disclosure
//! again — the answer never lived in the file. That is the property a
//! manifest cannot have on its own, since any re-encode removes it.
//!
//! # Why the port is here and not in `repository`
//!
//! [`DisclosureWriter`] is an outbound port like the repositories, and
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

use crate::domain::axis_status::AxisStatus;
use crate::domain::disclosure::{DisclosureRecord, Stamped};

use crate::domain::disclosure::{self, ParentEvidence, PromptDisclosure};
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
/// `asterism-infra::disclosure`, which owns the containers, the XMP
/// packet and the C2PA signer; nothing about any of those reaches this
/// side of the boundary.
#[async_trait]
pub trait DisclosureWriter: Send + Sync {
    /// Applies `record` to the file at `path`, replacing it.
    ///
    /// Returns what was actually written. The two halves of a
    /// disclosure fail independently, so a caller must be able to see
    /// which of them landed rather than inferring success from the
    /// absence of an error ([`Stamped`]).
    ///
    /// # What `Err` means here
    ///
    /// Only that **nothing could be attempted** — the file could not be
    /// read, or its container is not one the implementation writes
    /// into. A half that was tried and failed comes back inside
    /// [`Stamped`] as [`Half::Failed`](crate::domain::disclosure::Half::Failed),
    /// alongside whatever the other half achieved.
    ///
    /// The split is not stylistic. An implementation that reported a
    /// failed manifest through `Err` would have to discard the packet
    /// it had already produced, because an error return has nowhere to
    /// carry it — which is how a certificate expiring came to withhold
    /// the disclosure half that needs no certificate. Collecting both
    /// outcomes and letting the caller decide which of them is a fault
    /// keeps that decision where the context is.
    async fn apply(&self, path: &Path, record: &DisclosureRecord) -> Result<Stamped, DomainError>;
}

/// Assembles and applies AI-disclosure provenance.
pub struct DisclosureService {
    assets: Arc<dyn AssetRepository>,
    edges: Arc<dyn EdgeRepository>,
    writer: Arc<dyn DisclosureWriter>,
    prompts: PromptDisclosure,
}

impl DisclosureService {
    /// Builds the service over its ports.
    ///
    /// `prompts` is configuration, not a port: it is the one thing this
    /// service decides rather than reads, and it decides it the same way
    /// for every asset it is asked about. It is taken here rather than
    /// per call so that an installation states it once, in the
    /// composition root, and no call site can quietly differ.
    pub fn new(
        assets: Arc<dyn AssetRepository>,
        edges: Arc<dyn EdgeRepository>,
        writer: Arc<dyn DisclosureWriter>,
        prompts: PromptDisclosure,
    ) -> Self {
        Self {
            assets,
            edges,
            writer,
            prompts,
        }
    }

    /// Assembles what one asset discloses.
    ///
    /// # Errors
    ///
    /// Beside the repository errors that propagate, this refuses
    /// ([`DomainError::Conflict`]) an asset whose primary material has
    /// not been fingerprinted: until the hash job runs there is no
    /// answer to build from, only a question nobody has asked yet, and
    /// the record it would produce is indistinguishable from one for a
    /// file with nothing to say.
    ///
    /// The refusal holds for an asserted asset too: an assertion
    /// answers what the file *is*, not whether its container has been
    /// read, and the gate is about the second. The fingerprint runs
    /// either way; the assertion outranks whatever it finds.
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
        //
        // `meta_kv` alone is not enough to read, because its `None`
        // merges states the storage keeps apart: the fingerprint job
        // has not run, a probe read the container and this is what it
        // holds, and no probe reads this format. The status column
        // beside the digest is where the distinction lives, and the
        // two statuses that are not answers — `pending` (the question
        // not yet asked) and `failed` (asked, and the bytes could not
        // be read) — are refused: a record built on either stamps an
        // unmarked file that cannot be told afterwards from one with
        // nothing to say. The automatic path already orders the stamp
        // after the hash, so what this catches is a caller racing the
        // fingerprint or re-applying before it ran.
        let meta_kv = match asset.materials.first() {
            None => None,
            Some(material)
                if matches!(
                    material.meta_hash_status,
                    AxisStatus::Pending | AxisStatus::Failed
                ) =>
            {
                return Err(DomainError::Conflict(format!(
                    "asset {asset_id} has not had its metadata fingerprinted yet: \
                     a disclosure built now would read \"nothing established\" \
                     out of a question that has not been asked"
                )));
            }
            Some(material) => material.meta_kv.clone(),
        };

        let edges = self
            .edges
            .edges_of(asset_id, Some(EdgeKind::DerivedFrom), MAX_PARENTS)
            .await?;
        let mut parents = Vec::with_capacity(edges.len());
        for edge in edges {
            // `from` is the newer asset by the write path's convention,
            // so the parent is on the far side.
            let parent_id = edge.to;
            // What the parent declares is what separates "a model made
            // this" from "a model altered a photograph" — and only a
            // declaration separates them. A parent this cannot read is
            // unknown, and unknown asserts nothing: it neither widens
            // the term into `compositeWithTrainedAlgorithmicMedia` nor
            // narrows it, because putting either movement on a parent
            // nobody read would write a claim no evidence made.
            // Storage will not let an edge dangle (a foreign key refuses
            // it, pinned by `an_edge_cannot_point_at_an_asset_that_is_
            // not_there`), so the missing-row case covers a row
            // disappearing between these two reads rather than a shape a
            // caller can write. A parent racing its own fingerprint
            // reads the same way — unknown until its rows land, and a
            // re-apply after they do re-derives with what they say.
            //
            // A person's assertion on the parent is read first: an
            // asserted term is a declaration on the same footing as the
            // container's own, and it is what the hand-assertion route
            // exists to supply for a parent whose container says
            // nothing.
            let origin = match self.assets.find(&parent_id).await? {
                None => disclosure::ParentOrigin::Unknown,
                Some(parent) => match disclosure::asserted_source_type(&parent.extra) {
                    Some(ty) => disclosure::ParentOrigin::declared(ty),
                    None => disclosure::declared_origin(
                        parent.materials.first().and_then(|m| m.meta_kv.as_deref()),
                    ),
                },
            };
            parents.push(ParentEvidence {
                asset_id: parent_id.to_string(),
                origin,
            });
        }

        Ok(disclosure::record_for(
            &asset_id.to_string(),
            asset.title.as_deref(),
            dispatch_id,
            meta_kv.as_deref(),
            &parents,
            disclosure::asserted_source_type(&asset.extra),
            self.prompts,
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
// `crates/asterism-infra/tests/disclosure_service.rs`, against the real
// SQLite repositories rather than fakes. That is not a preference: what
// there is to get wrong here — which material's metadata is read, which
// side of a `derived_from` edge is the parent, what a purged parent does
// to the term — is precisely the part a hand-written fake would encode
// its author's assumption of, and the assumption is the thing under
// test. The mapping those answers feed, which is pure, is tested beside
// itself in `domain::disclosure`.
