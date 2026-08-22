//! `AssetService` — asset lifecycle, grid reads, and detail views.
//!
//! - Write path: [`add`](AssetService::add) validates and persists the
//!   asset, then enqueues the follow-up pipeline jobs (`cover_gen`,
//!   `auto_tag`, `edge_rebuild`).
//! - Read hot path: [`list`](AssetService::list) and
//!   [`search`](AssetService::search) pass the `AssetCard` projection
//!   straight through without materialising the full entity.
//! - Detail path: visibility is enforced here (through
//!   [`Visibility::visible_to`]) — the list path enforces it via SQL.
//!
//! There is no separate `SearchService` in v1: [`search`](AssetService::search)
//! only orchestrates the `AssetRetriever` port (Tantivy) against the
//! repository's filter surface, which is small enough to sit here. When
//! semantic search lands, this decision can be revisited.

use std::sync::Arc;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AttachTagBatchCommand,
    AttachTagBatchResult, DeclareProvenanceCommand, DetachTagBatchCommand, DetachTagBatchResult,
    EmptyTrashCommand, EmptyTrashResult, OnDuplicate as WireOnDuplicate, OrganizeByLocationCommand,
    OrganizeByLocationResult, PurgeAssetCommand, RestoreAssetCommand, TrashAssetCommand,
    UpdateAssetMetaBatchCommand, UpdateAssetMetaBatchResult, UpdateAssetMetaCommand,
};
use asterism_contract::dto::{
    AssetDetailDto, AssetDto, AssetPageDto, RetrievedIdsDto, RetrievedPageDto, SampledPageDto,
    VideoPreviewDto,
};
use asterism_contract::query::{
    GetAssetDetailQuery, ListAssetsQuery, RandomAssetsQuery, SearchAssetsQuery,
};
use chrono::Utc;

use crate::application::attribution_intake::refuse_assertion_from_owner_surface;
use crate::application::mapping::{
    asset_to_dto, detail_to_dto, page_to_dto, parse_asset_id, parse_ms, parse_persona_id,
    to_asset_query,
};
use crate::domain::asset::Asset;
use crate::domain::asset_comment::{AssetComment, CommentAuthor, SelectionGesture};
use crate::domain::attribution::{AttributionContext, OperatorRef};
use crate::domain::content_hash;
use crate::domain::edge::{ConstellationEdge, EdgeKind};
use crate::domain::job::JobKind;
use crate::domain::material::Material;
use crate::domain::provenance::{self, ProvenanceRef, SIDECAR_SUFFIX};
use crate::domain::repository::{
    AssetCommentRepository, AssetRepository, DimsScope, DirRepository, EdgeRepository,
    GroupRepository, JobQueue, PersonaRepository, SourceLookupScope, SourceTextReader,
    TagRepository, TextLocator,
};
use crate::domain::source_locator::{LocalPath, SourceLocator};
use crate::domain::value::{
    AssetId, CoverText, DirId, GroupId, Label, MimeType, Modality, OnDuplicate, PersonaId,
    RegisterNote, SourceKind, SourceRef, Viewer, dedup_labels,
};
use crate::error::DomainError;

/// Cap on the number of edges shown on the detail view; hover uses a
/// smaller limit (typically 3).
const DETAIL_EDGE_LIMIT: u32 = 12;

/// How many picks a random draw returns when the caller does not say.
/// A screenful is a few dozen cards, so a hundred gives the grid
/// something to scroll into without turning a "show me something" into a
/// corpus dump.
const RANDOM_PICKS_DEFAULT: u32 = 100;

/// Widest draw served. Above this the answer stops being a handful and
/// starts being a listing that cannot be paged — which is the list
/// path's job, with an order that means something.
const RANDOM_PICKS_MAX: u32 = 500;

/// How many duplicate groups one report returns when the caller does
/// not say. A duplicate report is a work list, not a corpus dump — a
/// screenful of groups to resolve is more useful than every group in
/// the library, and the next call picks up whatever is left once the
/// first batch has been dealt with.
const DUPLICATE_GROUP_DEFAULT_LIMIT: u32 = 50;

/// The wire spelling of a duplicate axis.
///
/// An exhaustive match rather than `as_str()` into a `String`: the two
/// sets are declared in two crates (the leaf contract crate cannot
/// depend on this one), and a match is what stops compiling when a third
/// axis is added to either. Going through the slug would compile fine
/// and produce a token the other side has never heard of.
fn axis_to_dto(
    axis: crate::domain::duplicate_conflict::DuplicateAxis,
) -> asterism_contract::dto::DuplicateAxis {
    use crate::domain::duplicate_conflict::DuplicateAxis as Domain;
    use asterism_contract::dto::DuplicateAxis as Wire;
    // Every arm names the same word on both sides. It did not always:
    // the wire said `File` where the domain said `Artefact`, and this
    // match was the seam between the two spellings. V64 rewrote the
    // stored values, so the seam is gone and this function is a pure
    // crate crossing.
    match axis {
        Domain::Artefact => Wire::Artefact,
        Domain::Content => Wire::Content,
        Domain::Meta => Wire::Meta,
    }
}

/// Well-known label slug the Inbox / Review flow keys off. Assets
/// wearing this label surface in the sidebar's 📥 Inbox chip;
/// clearing it in the detail pane / bulk action bar "graduates"
/// the asset out of the triage bucket. Kept as a `const` so the
/// UI and the ingest path stay locked to the same string without
/// a shared crate detour.
pub const INBOX_LABEL: &str = "inbox";

/// Edge label on a `derived_from` written because the ingesting caller
/// declared its parent, as opposed to one the dispatch runner wrote
/// while reifying its own output (`dispatch:<exporter>`).
///
/// The two are worth telling apart when reading a chain: a dispatch
/// edge is a fact Asterism observed end to end, a correlated one is a
/// claim it accepted from whoever ran the outside hop.
const CORRELATED_INGEST_LABEL: &str = "correlated-ingest";

/// Hard ceiling on `lineage_of`'s walk depth.
///
/// A chain is something a person reads. Eight hops is already a long
/// story about one artefact; past that the value is in a query, not a
/// picture, and an unbounded walk on a hand-declarable graph is an
/// invitation to pay for someone's mistake.
const LINEAGE_MAX_DEPTH: u32 = 8;

/// Node budget for one `lineage_of` walk. Reached when a chain fans
/// out (an export of many assets, each with its own children) rather
/// than running deep.
const LINEAGE_MAX_NODES: u32 = 200;

/// Per-node edge fanout the walk asks for. Above the per-hop parent
/// count of any realistic export, and the node budget is the real
/// bound anyway.
const LINEAGE_EDGE_FANOUT: u32 = 64;

/// How many trashed assets one `empty_trash` page takes. The call
/// loops until the trash is drained, so this bounds the memory a
/// single scan holds (and the size of one index commit), not how much
/// the command can delete.
const EMPTY_TRASH_SCAN_PAGE: u32 = 2_000;

/// How many pending provenance claims one re-resolve sweep looks at.
/// Bounded like the lineage walk; the sweep re-runs on every reify,
/// so a page left over is the next sweep's work, not a loss.
const UNRESOLVED_SWEEP_LIMIT: u32 = 500;

/// Where one asset's original artefact is, and what shape its bytes
/// have — the answer [`AssetService::original_file`] gives a caller that
/// is about to read them.
///
/// Deliberately not a DTO: it never crosses the wire. The transport that
/// asked opens `path` and streams it, so the bytes exist in one place
/// only (the socket), never in a field on this struct.
#[derive(Debug, Clone, PartialEq)]
pub struct OriginalFileRef {
    /// Filesystem path to open — the path
    /// [`SourceLocator::local_path`] gives, which is the whole question
    /// this type answers.
    pub path: std::path::PathBuf,
    /// The asset's locator, rendered for display: worth quoting back in
    /// an error, and the identity a person recognises. Not the storage
    /// form — a `file://` spelling reaches this field as the path it
    /// names, because that is the locator, and the spelling was never
    /// a fact about the artefact.
    pub locator: String,
    /// Data format of the original. `None` is "unknown", never "not
    /// applicable" — see [`Material::mime`]. The HTTP layer sends
    /// [`MimeType::as_str`] as the `Content-Type`, so an unrecognised
    /// value round-trips rather than being flattened.
    pub mime: Option<MimeType>,
}

/// Asset use-case service. Shared as an `Arc` through Tauri state and
/// server contexts.
pub struct AssetService {
    assets: Arc<dyn AssetRepository>,
    personas: Arc<dyn PersonaRepository>,
    tags: Arc<dyn TagRepository>,
    groups: Arc<dyn GroupRepository>,
    /// Comment port — held for one write path: the optional remark a
    /// selection gesture carries (#65) lands as an [`AssetComment`]
    /// pinned to the verb. Thread lifecycle (post / edit / delete)
    /// stays with `AssetCommentService`; this service only ever
    /// appends the gesture's footnote.
    comments: Arc<dyn AssetCommentRepository>,
    dirs: Arc<dyn DirRepository>,
    edges: Arc<dyn EdgeRepository>,
    /// Snapshot port — read-only use here to synthesise the
    /// `same_selection` axis on the constellation burst without
    /// storing edge rows for the m:n snapshot ↔ asset link.
    snapshots: Arc<dyn crate::domain::repository::SnapshotRepository>,
    /// Dispatch port — read-only use here, to resolve a
    /// `derived_from: dispatch:<id>` claim into the assets that
    /// dispatch produced. An export *is* a dispatch, so this is how
    /// "the batch I sent out" becomes a set of parents without a
    /// second id space (see `domain::provenance`).
    dispatches: Arc<dyn crate::domain::repository::DispatchRepository>,
    source_texts: Arc<dyn SourceTextReader>,
    jobs: Arc<dyn JobQueue>,
    /// Retrieval port, read side — [`search`](Self::search) asks it
    /// for a ranked shortlist and then narrows that with the SQL
    /// filter.
    retriever: Arc<dyn crate::domain::repository::AssetRetriever>,
    /// Retrieval port, write side — trashing an asset has to drop its
    /// document, or it keeps coming back as a candidate. Separate from
    /// `retriever` because these are different halves used by
    /// different paths, not one dependency used twice.
    indexer: Arc<dyn crate::domain::repository::AssetIndexer>,
    /// Body cache, write side — held for exactly one purpose: taking
    /// the composition stamp off a row whose re-index could not be
    /// queued, so the backfill walk finds it. Nothing here composes a
    /// body; that is the `IndexRebuild` handler's work.
    asset_bodies: Arc<dyn crate::domain::repository::AssetBodyRepository>,
    /// Sort port — read-only use here. [`list`](Self::list) needs the whole
    /// filtered set carrying the columns the comparator reads
    /// (`fetch_sortable_assets`, no `LIMIT`) whenever the caller names an
    /// axis, because ordering before pagination is the only way the page
    /// can be the page that axis asks for. The port's own doc says it
    /// shares no caller with the hot read path; that stopped being true
    /// here, deliberately — the alternative was a second sort
    /// implementation on the read side.
    query_groups: Arc<dyn crate::domain::repository::QueryGroupRepository>,
    /// Query Group invalidator (W4). Every user-facing write
    /// that could change a rule input notifies the persona it
    /// touched; the invalidator debounces and enqueues
    /// [`JobKind::QueryGroupRefresh`]. Handler-driven writes never
    /// re-enter this service, so no per-call opt-out is needed.
    query_group_invalidator: crate::application::query_group_invalidation::QueryGroupInvalidator,
    /// Session 1st-class entity resolver (P3). `add` routes
    /// every `AddAssetCommand::external_session_key` through
    /// [`SessionService::find_or_create_by_external_key`] to obtain a
    /// stable `Session.id`, which then lands on `asset.session_id`.
    /// Sharing the service (rather than the raw repository port) keeps
    /// the idempotence + seed-window semantics in one place — the
    /// importer never has to know Session invariants.
    sessions: Arc<crate::application::SessionService>,
    /// Where video preview renditions live (`<profile>/previews/`).
    /// [`video_preview`](Self::video_preview) reads the files the
    /// `preview_gen` job writes; the naming contract between the two
    /// is `domain::render::video_preview_path` and siblings.
    previews_dir: std::path::PathBuf,
    /// Scored tag suggestions (#112) — the person-facing half: listing
    /// what the model proposed and recording rulings. The suggestion
    /// job writes through its own handle; this one never inserts.
    tag_evidence: Arc<dyn crate::domain::repository::TagEvidenceRepository>,
    /// The bound encoder cell (#112) — read here only for its
    /// identity: which model's suggestions to show and rule on. An
    /// unset cell means no suggestions exist to list.
    visual_encoder: Arc<std::sync::OnceLock<Arc<dyn crate::domain::visual::VisualEncoder>>>,
}

impl AssetService {
    /// Constructs the service around a bundle of repository / queue ports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        assets: Arc<dyn AssetRepository>,
        personas: Arc<dyn PersonaRepository>,
        tags: Arc<dyn TagRepository>,
        groups: Arc<dyn GroupRepository>,
        comments: Arc<dyn AssetCommentRepository>,
        dirs: Arc<dyn DirRepository>,
        edges: Arc<dyn EdgeRepository>,
        snapshots: Arc<dyn crate::domain::repository::SnapshotRepository>,
        dispatches: Arc<dyn crate::domain::repository::DispatchRepository>,
        source_texts: Arc<dyn SourceTextReader>,
        jobs: Arc<dyn JobQueue>,
        retriever: Arc<dyn crate::domain::repository::AssetRetriever>,
        indexer: Arc<dyn crate::domain::repository::AssetIndexer>,
        asset_bodies: Arc<dyn crate::domain::repository::AssetBodyRepository>,
        query_groups: Arc<dyn crate::domain::repository::QueryGroupRepository>,
        query_group_invalidator: crate::application::query_group_invalidation::QueryGroupInvalidator,
        sessions: Arc<crate::application::SessionService>,
        previews_dir: std::path::PathBuf,
        tag_evidence: Arc<dyn crate::domain::repository::TagEvidenceRepository>,
        visual_encoder: Arc<std::sync::OnceLock<Arc<dyn crate::domain::visual::VisualEncoder>>>,
    ) -> Self {
        Self {
            assets,
            personas,
            tags,
            groups,
            comments,
            dirs,
            edges,
            snapshots,
            dispatches,
            source_texts,
            jobs,
            retriever,
            indexer,
            asset_bodies,
            query_groups,
            query_group_invalidator,
            sessions,
            previews_dir,
            tag_evidence,
            visual_encoder,
        }
    }

    /// Shorthand: fire the Query-Group invalidation hook for
    /// `persona_id`. Cheap (single mutex + timer rearm); every
    /// user-facing write that changes a rule input calls this after
    /// the write succeeds.
    fn notify_persona_touched(&self, persona_id: crate::domain::value::PersonaId) {
        self.query_group_invalidator.notify_persona(persona_id);
    }

    /// Kind gate: hand edits (add / remove / reorder / link-as-parent)
    /// are `kind='manual'` only — a query group's membership is owned by
    /// the evaluation job.
    async fn ensure_manual_group(
        &self,
        group_id: &crate::domain::value::GroupId,
        op: &str,
    ) -> Result<(), DomainError> {
        let group = self
            .groups
            .find(group_id)
            .await?
            .ok_or_else(|| DomainError::not_found("group", group_id))?;
        if group.kind == crate::domain::group::GroupKind::Query {
            return Err(DomainError::Validation(format!(
                "{op} is not available on a query group — its membership \
                 is defined by the stored query"
            )));
        }
        Ok(())
    }

    /// Turns a caller's `derived_from` claim into either a parent set
    /// or a recorded reason why it could not be honoured.
    ///
    /// Never returns an error: a provenance claim is metadata about an
    /// artefact, and losing the artefact because its label is wrong is
    /// the wrong trade. Everything that fails becomes an
    /// [`ResolvedOrigin::Unresolved`] note the user can act on later.
    async fn resolve_origin(
        &self,
        declared: Option<&str>,
        locator: &SourceLocator,
    ) -> ResolvedOrigin {
        let Some(claim) = declared else {
            return ResolvedOrigin::None;
        };
        let parsed = match provenance::parse(claim) {
            Ok(parsed) => parsed,
            Err(e) => return ResolvedOrigin::unresolved(claim, e.to_string()),
        };
        match parsed {
            ProvenanceRef::Asset(parent) => self.resolve_asset_parent(claim, parent, "asset").await,
            ProvenanceRef::Dispatch(dispatch) => {
                self.resolve_dispatch_parents(claim, dispatch, "dispatch")
                    .await
            }
            ProvenanceRef::Sidecar => self.resolve_sidecar(claim, locator).await,
        }
    }

    /// Resolves a directly-named parent asset.
    async fn resolve_asset_parent(
        &self,
        claim: &str,
        parent: AssetId,
        form: &'static str,
    ) -> ResolvedOrigin {
        match self.assets.find(&parent).await {
            // A trashed parent still counts. Trash is reversible, and
            // a chain that forgets its middle hop while the user is
            // deciding whether to restore it is worse than one that
            // points at something currently hidden.
            Ok(Some(_)) => ResolvedOrigin::Resolved {
                parents: vec![parent],
                form,
                claim: claim.trim().to_string(),
                dispatch: None,
            },
            Ok(None) => {
                ResolvedOrigin::unresolved(claim, format!("asset {parent} is not in this library"))
            }
            Err(e) => ResolvedOrigin::unresolved(claim, format!("parent lookup failed: {e}")),
        }
    }

    /// Resolves "the export I ran" into the assets it produced.
    ///
    /// An export is a dispatch and its outputs are already assets, so
    /// N parents is the same shape `reify` itself writes for an
    /// N-member snapshot.
    async fn resolve_dispatch_parents(
        &self,
        claim: &str,
        dispatch: crate::domain::value::DispatchId,
        form: &'static str,
    ) -> ResolvedOrigin {
        match self.dispatches.find(&dispatch).await {
            Ok(Some(job)) if !job.output_asset_ids.is_empty() => ResolvedOrigin::Resolved {
                parents: job.output_asset_ids.clone(),
                form,
                claim: claim.trim().to_string(),
                dispatch: Some(dispatch.to_string()),
            },
            // A dispatch that has not produced anything yet is not the
            // same as one with no parents: the answer can change, so
            // it is recorded as unresolved rather than as an empty
            // (and therefore silent) success.
            Ok(Some(job)) => ResolvedOrigin::unresolved(
                claim,
                format!(
                    "dispatch {dispatch} has produced no assets yet (state {:?})",
                    job.state
                ),
            ),
            Ok(None) => ResolvedOrigin::unresolved(
                claim,
                format!("dispatch {dispatch} is not in this library"),
            ),
            Err(e) => ResolvedOrigin::unresolved(claim, format!("dispatch lookup failed: {e}")),
        }
    }

    /// Reads the `<locator>.meta.json` sitting next to the artefact
    /// and resolves whatever it names.
    ///
    /// Preference order is deliberate. `_asterism.dispatch_id` names
    /// the *export* the file travelled through, which is the hop that
    /// actually happened; the card's `id` names the original, one hop
    /// further up. Falling back to the original keeps sidecars written
    /// before the identity block (and hand-written ones) usable — at
    /// the cost of a shorter chain, which `_trace.form` records so the
    /// difference stays visible.
    async fn resolve_sidecar(&self, claim: &str, locator: &SourceLocator) -> ResolvedOrigin {
        // A record addresses something *inside* a container file; there
        // is no "file next to it" to read. The variant is the whole
        // test now — the `contains('#')` this replaced also refused a
        // file whose own name carried a `#`, which has a sidecar like
        // any other file.
        if let SourceLocator::Record(record) = locator {
            return ResolvedOrigin::unresolved(
                claim,
                format!(
                    "{} addresses a record inside a container, so it has no sidecar",
                    record.container().as_str()
                ),
            );
        }
        // The sidecar is a JSON file this codebase's exporters write,
        // at a path composed here — no `material` row describes it, so
        // its text-ness is declared rather than looked up.
        //
        // Composed from the *display* rendering and read back through
        // the wire reader, because what is being built is a filesystem
        // neighbour rather than a column value: appending `.meta.json`
        // to the tagged storage form would produce a string that is
        // neither a path nor a locator. For the three variants that
        // reach this line (a `Record` returned above) the two renderings
        // are the same text.
        let sidecar_path = format!("{}{SIDECAR_SUFFIX}", locator.to_display());
        let sidecar = match SourceLocator::from_wire(sidecar_path.as_str()) {
            Ok(parsed) => TextLocator::of_known_text(parsed),
            // Unreachable in practice — the suffix alone makes the
            // string non-empty, which is the only refusal — but a
            // composed path is still a parse, and inventing a locator
            // for one that failed would be the fabrication this type
            // exists to stop.
            Err(e) => {
                return ResolvedOrigin::unresolved(
                    claim,
                    format!("sidecar path {sidecar_path} is not a locator: {e}"),
                );
            }
        };
        let text = match self
            .source_texts
            .read_batch(std::slice::from_ref(&sidecar))
            .await
        {
            Ok(mut texts) => texts.pop().flatten(),
            Err(e) => {
                return ResolvedOrigin::unresolved(claim, format!("sidecar read failed: {e}"));
            }
        };
        let Some(text) = text else {
            return ResolvedOrigin::unresolved(claim, format!("no sidecar at {sidecar_path}"));
        };
        let body: serde_json::Value = match serde_json::from_str(&text) {
            Ok(body) => body,
            Err(e) => {
                return ResolvedOrigin::unresolved(
                    claim,
                    format!("sidecar {sidecar_path} is not readable JSON: {e}"),
                );
            }
        };
        if let Some(raw) = body
            .get(provenance::SIDECAR_IDENTITY_KEY)
            .and_then(|id| id.get(asterism_contract::sidecar::SIDECAR_DISPATCH_ID_FIELD))
            .and_then(|v| v.as_str())
        {
            let origin = match provenance::parse(&format!("dispatch:{raw}")) {
                Ok(ProvenanceRef::Dispatch(dispatch)) => {
                    self.resolve_dispatch_parents(claim, dispatch, "sidecar-dispatch")
                        .await
                }
                _ => ResolvedOrigin::unresolved(
                    claim,
                    format!("sidecar {sidecar_path} names dispatch {raw:?}, which is not an id"),
                ),
            };
            return origin;
        }

        match body.get("id").and_then(|v| v.as_str()) {
            Some(raw) => match provenance::parse(&format!("asset:{raw}")) {
                Ok(ProvenanceRef::Asset(parent)) => {
                    self.resolve_asset_parent(claim, parent, "sidecar-asset")
                        .await
                }
                _ => ResolvedOrigin::unresolved(
                    claim,
                    format!("sidecar {sidecar_path} names asset {raw:?}, which is not an id"),
                ),
            },
            None => ResolvedOrigin::unresolved(
                claim,
                format!("sidecar {sidecar_path} names neither a dispatch nor an asset"),
            ),
        }
    }

    /// Writes the `DerivedFrom` edges a resolved claim asserts,
    /// child → parent, labelled as declared provenance.
    ///
    /// `INSERT OR IGNORE` underneath, so re-declaring accumulates
    /// rather than duplicates. Self-references are dropped — `asset:`
    /// is hand-writable, and an asset derived from itself is a cycle
    /// with no information in it.
    async fn write_derived_edges(
        &self,
        child: AssetId,
        parents: &[AssetId],
        relation: crate::domain::provenance::ClaimRelation,
    ) -> Result<(), DomainError> {
        let mut edges: Vec<ConstellationEdge> = Vec::with_capacity(parents.len());
        for parent in parents {
            if *parent == child {
                continue;
            }
            let mut edge = ConstellationEdge::new(child, *parent, relation.edge_kind())?;
            edge.label = Some(CORRELATED_INGEST_LABEL.to_string());
            edge.weight = Some(1.0);
            edges.push(edge);
        }
        self.edges.add_edges(edges).await
    }

    /// Runs duplicate detection over a digest the registering caller
    /// **stated**, so a match can be proposed at ingest without the
    /// server reading the file.
    ///
    /// # It proposes and never folds
    ///
    /// [`DetectionOrigin::Declared`] is what says so, and the reason is
    /// the shape of the claim rather than the shape of the pair: the
    /// value is an unverified assertion, the hashing job verifies it
    /// later, and there is no unfold verb — so a fold driven by a claim
    /// nobody has checked is not reversible, and a proposal is. A lane
    /// that declared `fold` still gets its fold, from the pass that
    /// measured the bytes.
    ///
    /// # What it does not buy
    ///
    /// `Artefact` is the strict axis, so an importer declaring a digest
    /// gets no hit for a re-export that differs by a metadata chunk.
    /// That case is `Content`, and neither `Content` nor `Meta` can be
    /// declared from outside — both need the container walker. This
    /// saves a read on the exact-copy case and nothing else.
    ///
    /// # Why the axis comes off the value
    ///
    /// The claim carries its own tag ([`content_hash::axis_of`]), which
    /// is what the tag is for, and the detector takes the axis as a
    /// parameter. Hardcoding `Artefact` here would be a second place
    /// that decides what a `cr1-sha256:` claim means, and the two would
    /// eventually disagree — the same reason `check_declaration` reads
    /// the tag rather than assuming one.
    ///
    /// # Every failure is swallowed
    ///
    /// The row is saved and the caller is about to be handed it. A
    /// proposal that could not be raised is raised again the moment the
    /// hash job fingerprints either side of the pair, so failing the
    /// ingest over it would destroy the durable half for the sake of a
    /// derivation that repairs itself.
    async fn propose_from_declaration(&self, asset_id: &AssetId, declared: &str) {
        let Some(axis) = content_hash::axis_of(declared) else {
            // Unreachable through `parse_declaration`, which refuses an
            // untagged value. Not an error either: a claim naming no
            // axis is a claim this cannot look up, and inventing one is
            // the single thing the tag exists to prevent.
            return;
        };
        let outcome = crate::application_support::duplicate_detection::detect_duplicate_on_axis(
            crate::application_support::duplicate_detection::DetectionPorts {
                assets: self.assets.as_ref(),
                edges: self.edges.as_ref(),
                queue: self.jobs.as_ref(),
            },
            asset_id,
            0,
            axis,
            declared,
            crate::application_support::duplicate_detection::DetectionOrigin::Declared,
            Utc::now(),
        )
        .await;
        match outcome {
            Ok(found) => tracing::info!(
                event = "action.duplicate.declared",
                asset_id = %asset_id,
                axis = %axis.as_str(),
                outcome = %found.describe(),
                "duplicate detection over a declared digest"
            ),
            Err(err) => tracing::warn!(
                event = "diag.duplicate.declared_failed",
                asset_id = %asset_id,
                axis = %axis.as_str(),
                error = %err,
                "could not propose against a declared digest"
            ),
        }
    }

    /// Declares (or repairs) the origin of an asset already in the
    /// library — the after-the-fact twin of
    /// [`AddAssetCommand::derived_from`].
    ///
    /// Same resolution, same honesty rules: a claim that cannot be
    /// resolved is recorded on `extra._trace` rather than rejected.
    /// Re-declaring replaces the note (latest claim wins) and adds
    /// edges on top of whatever earlier claims wrote — provenance is
    /// append-only, so a corrected claim widens the graph rather than
    /// rewriting it.
    ///
    /// Takes an `AttributionContext` like every other mutation and does
    /// not use it (`_attribution`): this verb writes a provenance
    /// *claim*, and `DeclareProvenanceCommand::operator_ai` is part of
    /// that claim (`_trace.operator`) rather than an attribution column
    /// — a different subject, read from the command on purpose.
    /// The asset's own attribution is left exactly as it was.
    pub async fn declare_provenance(
        &self,
        command: DeclareProvenanceCommand,
        _attribution: &AttributionContext,
    ) -> Result<AssetDto, DomainError> {
        let id = parse_asset_id(&command.asset_id)?;
        // Validated before the lookup: a blank operator is a rejected
        // assertion, not an unrecorded one, and finding that out after
        // the read costs a round trip for nothing.
        let operator = command.operator_ai.map(OperatorRef::new).transpose()?;
        // Refused before the lookup for the same reason the operator is:
        // an unknown relation is a rejected claim, not a claim to be
        // filed under the stronger word.
        let relation = command
            .relation
            .as_deref()
            .map(crate::domain::provenance::ClaimRelation::parse)
            .transpose()?
            .unwrap_or_default();
        let mut asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;
        let origin = self
            .resolve_origin(Some(&command.derived_from), &asset.source.locator)
            .await;
        // Channel bookkeeping (`_trace.source`): this endpoint *is* the
        // after-the-fact declaration channel, so every claim through it
        // is `manual` — even a `sidecar` form, which here is a person
        // (or agent) pointing at the sidecar rather than the importer
        // finding it on its own. The operator rides alongside it: the
        // channel says the claim was declared by hand, the operator says
        // through which agent. `asset.author` / `asset.operator_ai` are
        // deliberately left alone — repairing a link is not authoring
        // the asset.
        if let Some(note) = origin.trace_note(
            Some(provenance::source::MANUAL),
            operator.as_ref().map(OperatorRef::as_str),
            relation,
        ) {
            merge_claim_note(&mut asset.extra, note);
        }
        asset.updated_at = Utc::now();
        self.assets.save(&asset).await?;
        if let ResolvedOrigin::Resolved { parents, .. } = &origin {
            self.write_derived_edges(asset.id, parents, relation)
                .await?;
        }
        Ok(crate::application::mapping::asset_to_dto(&asset))
    }

    /// Records — or removes — one AlbumMeta statement on an asset.
    ///
    /// The sibling of [`declare_provenance`](Self::declare_provenance),
    /// and deliberately shaped like it: a statement somebody made, put
    /// in the bag this library keeps statements in, with the channel it
    /// arrived on and the agent it came through. What it is *not* is a
    /// provenance claim — nothing is resolved, no edge is drawn, and the
    /// application never acts on the value. See
    /// [`album_meta`](crate::domain::album_meta) for why that
    /// separation is the point rather than an omission.
    ///
    /// # Single slot, not append-only
    ///
    /// Declaring the same key twice leaves the later statement. A
    /// provenance claim is append-only because each one draws an edge
    /// and two parents are two true facts; two statements under one
    /// name are a correction and its subject, and keeping both would
    /// leave every reader to work out which is current.
    ///
    /// # Attribution
    ///
    /// Takes an `AttributionContext` and does not use it, for the reason
    /// [`declare_provenance`](Self::declare_provenance) does not: the
    /// operator recorded here is part of the *statement*
    /// (`_trace.meta.<key>.operator`), a different subject from who the
    /// asset is by. Saying something about a row is not authoring it.
    pub async fn declare_asset_meta(
        &self,
        command: asterism_contract::command::DeclareAssetMetaCommand,
        _attribution: &AttributionContext,
    ) -> Result<AssetDto, DomainError> {
        use crate::domain::album_meta;

        let id = parse_asset_id(&command.asset_id)?;
        // Everything checkable without the database is checked before
        // the read, so a malformed request costs no round trip — the
        // rule `declare_provenance` follows for its own operator.
        let key = album_meta::parse_key(&command.key)?;
        let value = command
            .value
            .as_deref()
            .map(album_meta::parse_value)
            .transpose()?;
        let operator = command.operator_ai.map(OperatorRef::new).transpose()?;

        let mut asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;

        let now = Utc::now();
        match value {
            Some(value) => {
                let note = album_meta::entry(
                    &value,
                    // This verb *is* the after-the-fact channel, the
                    // same reading `declare_provenance` gives its own
                    // claims. An ingest-time statement will carry
                    // `pushed`, and one dug out of an artefact
                    // `embedded`; neither arrives here.
                    provenance::source::MANUAL,
                    operator.as_ref().map(OperatorRef::as_str),
                    now.timestamp_millis(),
                );
                merge_meta_entry(&mut asset.extra, &key, Some(note));
            }
            None => merge_meta_entry(&mut asset.extra, &key, None),
        }
        asset.updated_at = now;
        self.assets.save(&asset).await?;
        // A declared statement is text somebody wrote about this asset,
        // and the derived document is composed from exactly that
        // population — so the index is stale the moment this returns.
        self.reindex(&id).await;
        Ok(crate::application::mapping::asset_to_dto(&asset))
    }

    /// Declares — or retracts — the asset's digital source type by hand.
    ///
    /// The third sibling of
    /// [`declare_provenance`](Self::declare_provenance) and
    /// [`declare_asset_meta`](Self::declare_asset_meta), and the one
    /// the disclosure module acts on: the recorded term outranks the
    /// container's evidence when the next disclosure is derived
    /// ([`record_for`](crate::domain::disclosure::record_for)), and a
    /// parent carrying one reads as declared rather than unknown. The
    /// term is validated at the door — an unknown value is refused, not
    /// recorded — because everything downstream signs this verbatim.
    ///
    /// Single-slot: a second declaration replaces the first, and `None`
    /// removes it, returning the asset to what its container evidence
    /// says on its own. No reindex: the term is not part of the derived
    /// search document.
    ///
    /// Takes an `AttributionContext` and does not use it, for the
    /// reason its siblings do not: the operator recorded here is part
    /// of the statement, not an attribution column.
    pub async fn declare_source_type(
        &self,
        command: asterism_contract::command::DeclareSourceTypeCommand,
        _attribution: &AttributionContext,
    ) -> Result<AssetDto, DomainError> {
        use crate::domain::disclosure;

        let id = parse_asset_id(&command.asset_id)?;
        // Checked before the read, the rule the siblings follow: an
        // unknown term is a rejected assertion, and a blank operator a
        // rejected statement, before either costs a round trip.
        let source_type = command
            .source_type
            .as_deref()
            .map(disclosure::DigitalSourceType::parse)
            .transpose()
            .map_err(|e| DomainError::Validation(e.to_string()))?;
        let operator = command.operator_ai.map(OperatorRef::new).transpose()?;

        let mut asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;
        let now = Utc::now();
        let entry = source_type.map(|ty| {
            album_meta_entry_for_source_type(
                ty,
                operator.as_ref().map(OperatorRef::as_str),
                now.timestamp_millis(),
            )
        });
        merge_source_type_assertion(&mut asset.extra, entry);
        asset.updated_at = now;
        self.assets.save(&asset).await?;
        Ok(crate::application::mapping::asset_to_dto(&asset))
    }

    /// Re-composes one asset's search document after a write to a field
    /// the document is derived from.
    ///
    /// The population is
    /// [`derive_text`](crate::domain::derived_text::derive_text)'s: a
    /// title, a cover, labels, keywords, a register note, declared meta.
    /// Every verb that writes one of those leaves the index describing a
    /// row that no longer exists, and for a picture those fields may be
    /// the only text there is — which makes this the difference between
    /// findable and not.
    ///
    /// Enqueue failure does not fail the write — the caller asked to
    /// edit an asset, not to index one — but it is **not** swallowed
    /// either, because nothing else would have noticed. The recovery
    /// this used to claim ("the backfill walk will get it") was untrue
    /// in exactly this case: the row's cached body carries the current
    /// composition stamp, so the walk, which selects bodies composed by
    /// an *older* reading, passes straight over it. The stale document
    /// would have survived until somebody edited that asset again.
    ///
    /// So a failure clears the stamp instead, which is what puts the row
    /// in front of the walk, and says so in the log. If that write fails
    /// too there is nothing further to try from here: both the queue and
    /// the database are refusing writes, and the caller's own write is
    /// the thing that matters.
    async fn reindex(&self, id: &AssetId) {
        let Err(err) = self
            .jobs
            .enqueue(
                JobKind::IndexRebuild,
                serde_json::json!({ "asset_id": id.to_string() }),
            )
            .await
        else {
            return;
        };
        tracing::warn!(
            event = "diag.index.enqueue_failed",
            asset_id = %id,
            error = %err,
            "could not queue a re-index; falling back to the backfill walk"
        );
        if let Err(err) = self.asset_bodies.unstamp(id).await {
            tracing::warn!(
                event = "diag.index.unstamp_failed",
                asset_id = %id,
                error = %err,
                "the row keeps a document composed from text that has changed"
            );
        }
    }

    /// Retries every provenance claim that is recorded but not yet
    /// resolved.
    ///
    /// The unresolved note exists *because* the answer can change — a
    /// dispatch that had produced nothing produces something, a
    /// sidecar that was missing gets written. This is the moment the
    /// change is cashed in: the dispatch runner calls it after `reify`
    /// lands new outputs. A claim that still does not resolve keeps
    /// its original note; churning the reason on every sweep would
    /// stamp `updated_at` on rows nothing happened to.
    ///
    /// Returns how many notes were repaired.
    pub async fn reresolve_unresolved(&self) -> Result<u32, DomainError> {
        let ids = self
            .assets
            .unresolved_provenance_ids(UNRESOLVED_SWEEP_LIMIT)
            .await?;
        let mut repaired = 0u32;
        for id in ids {
            let Some(mut asset) = self.assets.find(&id).await? else {
                continue;
            };
            let Some(claim) = asset
                .extra
                .get(provenance::TRACE_KEY)
                .and_then(|t| t.get("derived_from"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
            else {
                continue;
            };
            // The claim's arrival channel does not change when the
            // claim later resolves — carry the recorded `source`
            // forward verbatim. A pre-source note stays without one:
            // the channel is unknowable after the fact, and guessing
            // would forge exactly the bookkeeping the field exists for.
            let prior_source = asset
                .extra
                .get(provenance::TRACE_KEY)
                .and_then(|t| t.get("source"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            // Same rule for the operator: the agent that declared the
            // claim is a fact about the declaration, and this sweep is
            // not one. Carry whatever the note already said (a
            // hand-declared claim keeps naming its declarer) and add
            // nothing — the sweep runs unattended, so there is no
            // operator here to record.
            let prior_operator = asset
                .extra
                .get(provenance::TRACE_KEY)
                .and_then(|t| t.get("operator"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            // And the relation, which is a fact about what was claimed
            // rather than about this sweep. Unlike the two above it has
            // a correct answer when the note has none: a claim recorded
            // before the field existed came from a verb that could only
            // mean `derived_from`, so absent falls to the default here
            // instead of staying absent. An unparseable value would
            // have been refused at declaration time, so it falls the
            // same way rather than aborting a repair sweep over it.
            let prior_relation = asset
                .extra
                .get(provenance::TRACE_KEY)
                .and_then(|t| t.get("relation"))
                .and_then(|v| v.as_str())
                .and_then(|s| crate::domain::provenance::ClaimRelation::parse(s).ok())
                .unwrap_or_default();
            // What the note already answered, read before the re-resolve
            // so "did anything change" has a before to compare against —
            // the sweep would otherwise rewrite rows nothing happened to
            // every time it ran.
            let prior_resolved = asset
                .extra
                .get(provenance::TRACE_KEY)
                .and_then(|t| t.get("resolved"))
                .and_then(|v| v.as_bool())
                == Some(true);
            let origin = self
                .resolve_origin(Some(&claim), &asset.source.locator)
                .await;
            // A note that changed is the only one rewritten: doing it
            // unconditionally would stamp `updated_at` on rows nothing
            // happened to, every sweep.
            let ResolvedOrigin::Resolved { parents, .. } = &origin else {
                continue;
            };
            if prior_resolved {
                continue;
            }
            let parents = parents.clone();
            if let Some(note) = origin.trace_note(
                prior_source.as_deref(),
                prior_operator.as_deref(),
                prior_relation,
            ) {
                merge_claim_note(&mut asset.extra, note);
            }
            asset.updated_at = Utc::now();
            self.assets.save(&asset).await?;
            if !parents.is_empty() {
                self.write_derived_edges(asset.id, &parents, prior_relation)
                    .await?;
            }
            repaired += 1;
        }
        Ok(repaired)
    }

    /// Ingests an asset and enqueues the follow-up pipeline jobs.
    ///
    /// **A duplicate is not a failure here.** The Source value is looked
    /// up before anything is minted, so a record
    /// arriving again is answered with the row that already holds it —
    /// same `AssetId`, no second row, no error. A caller that means to
    /// make a second row at one location says so with
    /// `on_duplicate = Separate`, which is the one declaration that
    /// passes that branch. Rows that coincide on *bytes* rather than on
    /// location are still a question for a person: the match axes raise
    /// a proposal on the conflict queue, and this method never folds.
    ///
    /// The attribution comes from `attribution`, never from the
    /// command: `AddAssetCommand`'s three attribution fields are the
    /// carrier a *remote* caller uses, and the adapter that received the
    /// request is what turns them into a context. The only
    /// thing this method does with those fields is refuse the one
    /// combination translation cannot express — an assertion arriving
    /// through the owner's own surface.
    pub async fn add(
        &self,
        command: AddAssetCommand,
        attribution: &AttributionContext,
    ) -> Result<AssetDto, DomainError> {
        // Checked before anything is written — including before the
        // session resolution below, which *creates* a composite: a
        // contradictory command must not leave a freshly minted Session
        // container behind for an ingest that never landed.
        refuse_assertion_from_owner_surface(
            attribution,
            &[
                ("author_kind", command.author_kind.is_some()),
                ("author_subject", command.author_subject.is_some()),
                ("operator_ai", command.operator_ai.is_some()),
            ],
        )?;
        // Checked here for the reason stated above it: before the session
        // resolution, which writes. `add` is the single funnel every
        // `AddAssetCommand` passes through, so an importer that copied
        // only one of the two dimensions fails loud here rather than
        // landing half a resolution nothing downstream can tell from a
        // measurement.
        refuse_half_written_dims(command.width_px, command.height_px)?;
        // Snapshot the auto-organize hint so we can hand it to
        // `organize_by_location` after the save without cloning the
        // rest of the command through consume.
        let auto_organize_base_dir = command.auto_organize_base_dir.clone();
        let declared_origin = command.derived_from.clone();
        // The wire boundary for the locator, and the only place in this
        // method where it is a string. Everything below — the declared
        // hash gate, the `SourceRef`, the material, the provenance
        // resolve — is handed the value, so the questions they ask about
        // it are answered by the type rather than by three separate
        // sniffs of the same text.
        let locator = SourceLocator::from_wire(command.locator.as_str())?;
        let persona_id = parse_persona_id(&command.persona_id)?;
        let persona = self
            .personas
            .find(&persona_id)
            .await?
            .ok_or(DomainError::PersonaNotFound(persona_id))?;
        // A trashed persona is invisible only because every asset that
        // was live under it carries its trash stamp. A fresh asset would
        // carry none, so it would appear in a grid the user believes is
        // gone — and would be silently destroyed by the retention sweep's
        // persona cascade. Refuse instead: restore the persona first.
        if persona.trashed_at.is_some() {
            return Err(DomainError::Conflict(format!(
                "persona {persona_id} is in the trash; restore it before adding assets"
            )));
        }

        let modality = command.modality.map(Modality::new).transpose()?;
        let occurred_at = parse_ms(command.occurred_at_ms, "occurred_at_ms")?;

        // The duplicate strategy is read off the command here, above the
        // session resolution, because everything the command has to be
        // understood as belongs before the first write:
        // `find_or_create_by_external_key` *writes*, so a command this
        // service cannot accept must be refused before a freshly minted
        // Session container is left behind for an ingest that never
        // landed. This particular field cannot fail — the wire type is a
        // closed enum, so an unknown token was already refused by
        // deserialisation, further above still and before this service
        // was reached at all — which is why there is no "leaves a
        // Session behind" hazard on this field to guard against.
        //
        // `None` is carried through as `None`. The detector resolves an
        // undeclared registration against its own default; writing one
        // in here would erase the fact that nobody declared anything.
        let on_duplicate = command.on_duplicate.map(|strategy| match strategy {
            WireOnDuplicate::Ask => OnDuplicate::Ask,
            WireOnDuplicate::Fold => OnDuplicate::Fold,
            WireOnDuplicate::Separate => OnDuplicate::Separate,
        });

        // The pre-hash declaration, read here for the reason the
        // strategy above is: a notation this service cannot accept is
        // refused before `find_or_create_by_external_key` can mint a
        // Session container for an ingest that will not land. Unlike
        // `on_duplicate` the value is an open notation rather than a
        // closed enum, so deserialisation cannot police it and the
        // ordering has to.
        //
        // What comes back is **the claim**, and it is deliberately not
        // assigned to anything on the material. The file is hashed by
        // the background job exactly as it would have been without a
        // declaration; this only decides whether there is an assertion
        // to check the result against.
        let declared_hash = command
            .declared_content_hash
            .as_deref()
            .map(content_hash::parse_declaration)
            .transpose()?;
        // A locator with no bytes of its own — a record inside a
        // container file, a URL, a caller-minted name — is never
        // hashed: the job records the `no-bytes` status and moves on.
        // A declaration about it would therefore sit forever in the
        // "not checked yet" state, which is the one state a reader must
        // be able to trust. Refused on the same grounds as the content
        // axis: an assertion whose verification cannot arrive is worse
        // than no assertion, because it looks like one that is pending.
        if declared_hash.is_some() && locator.local_path().is_none() {
            return Err(DomainError::Validation(format!(
                "declared_content_hash was supplied for {:?}, which has no bytes to \
                 read (a record inside a container file, a remote address, or a name \
                 that is not a path) — nothing would ever check it",
                command.locator
            )));
        }

        // The AlbumMeta statements, checked here on the same grounds as
        // the two declarations above: `find_or_create_by_external_key`
        // writes, so a command this service cannot accept has to be
        // refused before a Session container is minted for an ingest
        // that will never land.
        //
        // A refusal fails the whole ingest rather than dropping the bad
        // entry and keeping the rest. The reason to state something at
        // registration is that the artefact is findable by it
        // afterwards; landing the row without the statement would leave
        // something that looks imported and answers to nothing, and the
        // caller would learn that from a search returning empty rather
        // than from the call that made the mistake.
        let album_meta = parse_album_meta(&command.album_meta)?;

        let source_kind = SourceKind::new(command.source_kind.as_str())?;
        // **Step 2 of the ingest lifecycle: look the Source value up,
        // before anything is minted**.
        //
        // A hit means this record is arriving again, so there is nothing
        // to mint and the caller is handed the row that was already
        // there. This is a lookup and not a constraint — several rows
        // may carry one Source value (`N : 1`), and the branch below is
        // the default answer rather than an enforced one.
        //
        // The order is what makes an answered duplicate stay answered. A
        // closed conflict row is keyed on the pair of `AssetId`s; mint
        // first and every arrival is a new id, so every pair is new and
        // the answer a person already gave never matches again. Under
        // `ScanMode::Enumerate` that is one copy of the library per
        // sweep.
        //
        // Placed after the validations above and before the session
        // resolution below, which *writes*: a re-arrival must not leave
        // a freshly minted Session container behind either.
        let held = self
            .assets
            .find_by_source(&persona_id, &source_kind, &locator, SourceLookupScope::Live)
            .await?;
        if let Some(existing) = held {
            // `Separate` is the caller saying it means to make a second
            // row: this lane produces identical material deliberately.
            // The Source value being held is then recorded rather than
            // obeyed — mint, and let the match axes raise whatever they
            // find. Nothing else can express that intent, because two
            // arrivals at one location are indistinguishable at the port
            // (one record re-scanned, or a lane writing every run's
            // output to the same path); only the caller knows which.
            //
            // `Ask` and `Fold` answer from the row. `Ask` deliberately
            // does *not* raise a Source-axis proposal — an ordinary
            // re-scan is the overwhelming majority of arrivals, and one
            // queued question per re-scanned file is the panel made
            // useless.
            if !matches!(on_duplicate, Some(OnDuplicate::Separate)) {
                return Ok(asset_to_dto(&existing));
            }
        }

        // Session routing. Membership is modality-agnostic (asset-model
        // v4) — the only guard left is the matrix-ambiguity check,
        // factored into `classify_session_binding` so it can be
        // unit-tested without spinning up the full repository graph.
        let binding = classify_session_binding(
            command.session_id.as_deref(),
            command.external_session_key.as_deref(),
        )?;
        // session-model v2: the resolved id is the composite Asset's id,
        // written onto the member via `container_id` (not `session_id`,
        // which is dead and removed in the contract phase). The
        // find_or_create now mints/returns a composite Asset id.
        let resolved_container_id: Option<crate::domain::value::AssetId> = match binding {
            SessionBinding::None => None,
            SessionBinding::DirectId(raw) => Some(parse_container_id(&raw)?),
            SessionBinding::ExternalKey(raw) => {
                let seed_ms = occurred_at.timestamp_millis();
                let dto = self
                    .sessions
                    // The Session this ingest may mint is part of the same
                    // request, so it is attributed to the same channel —
                    // the context travels with the call rather than being
                    // re-chosen here.
                    .find_or_create_by_external_key(
                        &persona_id,
                        &raw,
                        seed_ms,
                        seed_ms,
                        attribution,
                    )
                    .await?;
                Some(parse_container_id(&dto.id)?)
            }
        };

        let mut source = SourceRef::of_locator(source_kind, locator);
        source.file_size_bytes = command.file_size_bytes;
        source.platform = command.platform;

        let mut asset = Asset::new(persona_id, source, modality, occurred_at, attribution);
        // Material layer (asset-model v4): capture the physical-original
        // facts alongside the row. `add` only ever mints items (the
        // collection shape is minted by `SessionRepository::create`), so
        // the primary material always attaches.
        asset.attach_material(Material::primary(
            asset.source.locator.clone(),
            asset.source.file_size_bytes,
            asset.created_at,
        ))?;
        asset.container_id = resolved_container_id;
        // What the source calls this record, stored as stated. It is not
        // validated against anything and not compared with anything: a
        // key the library already holds is not a collision, because two
        // arrivals of one external record state one key and two
        // platforms number their records alike. The only thing this
        // assignment must not become is a lookup.
        asset.external_key = command.external_key;
        asset.bundle_id = command
            .bundle_id
            .map(crate::domain::value::BundleId::new)
            .transpose()?;
        // A caller is free to state the same label twice — a re-run
        // carrying its previous label set, an importer that adds a tag
        // the spec already listed. Stated twice, stored once: repeats
        // are dropped here so no row is written in a shape the label
        // chips cannot render (`dedup_labels`).
        asset.labels = dedup_labels(
            command
                .labels
                .into_iter()
                .map(Label::new)
                .collect::<Result<Vec<_>, _>>()?,
        );
        // Inbox-triage default: every freshly ingested asset gets
        // the `inbox` label so the sidebar "📥 Inbox" chip surfaces
        // it as "still needs a look" until the user clears the
        // label from the detail pane / bulk action bar. Idempotent
        // — an importer that already added the label (or a re-run
        // caller carrying the previous label set) does not get a
        // second copy.
        if !asset.labels.iter().any(|l| l.as_str() == INBOX_LABEL) {
            asset.labels.push(Label::new(INBOX_LABEL)?);
        }
        asset.register_note = command.register_note.map(RegisterNote::new).transpose()?;
        // The attribution pair is not assigned here: it came in with the
        // context `Asset::new` was handed. What the command still owns
        // is the duplicate strategy — a declaration about this ingest,
        // not a claim about who made it.
        asset.on_duplicate = on_duplicate;
        // Importer-supplied cover text — takes priority over the generic
        // `cover_gen` heuristic. The job still runs but bails out early
        // because `asset.cover.is_some()`.
        asset.cover = command.cover_hint.map(CoverText::new).transpose()?;
        asset.duration_ms = command.duration_ms;
        // Coded pixel dimensions, carried through as stated. The pair was
        // already checked at the top of this method, so by here the two
        // are either both present or both absent.
        asset.width_px = command.width_px;
        asset.height_px = command.height_px;
        if let Some(extra_json) = command.extra_json {
            asset.extra = serde_json::from_str(&extra_json)
                .map_err(|e| DomainError::Validation(format!("invalid extra_json: {e}")))?;
        }

        // Provenance claim (correlation ingest). Resolved before the
        // save so the outcome — link or broken link — is part of the
        // row from its first version, rather than a second write that
        // could be interrupted between the two.
        //
        // Channel bookkeeping (`_trace.source`): at ingest, a `sidecar`
        // claim is the importer reporting what it found next to the
        // file (`embedded`); an `asset:` / `dispatch:` claim is the
        // caller pushing what it knows with the payload (`pushed`).
        // Derived from the claim's parsed form, not caller-asserted —
        // a malformed claim defaults to `pushed`, since `sidecar` is
        // the one spelling that parses to the embedded form.
        let claim_source = declared_origin
            .as_deref()
            .map(|claim| match provenance::parse(claim) {
                Ok(ProvenanceRef::Sidecar) => provenance::source::EMBEDDED,
                _ => provenance::source::PUSHED,
            });
        let origin = self
            .resolve_origin(declared_origin.as_deref(), &asset.source.locator)
            .await;
        // No `operator` on an ingest-time note: this asset carries its
        // own `operator_ai` column, and the ingest is the operation that
        // note describes. The field exists for the paths where the
        // operation and the row it touches are not the same event (the
        // repair verb, a dispatch run).
        // The relation is `derived_from` and `AddAssetCommand` carries
        // no way to say otherwise. That is deliberate rather than
        // pending: an ingest-time claim is somebody handing over a file
        // and naming what it came out of, and the weaker "made with
        // this in view" is a statement about how a person worked, which
        // arrives — if it arrives — after the fact through the
        // declaration verb. Widening this would mean every importer had
        // to have an opinion about a distinction none of them can see.
        if let Some(note) = origin.trace_note(
            claim_source,
            None,
            crate::domain::provenance::ClaimRelation::DerivedFrom,
        ) {
            merge_claim_note(&mut asset.extra, note);
        }
        // The declared digest, kept as what it is: a statement made at
        // registration, in the bag this library keeps statements in.
        // The order no longer matters — `merge_claim_note` clears only
        // the claim's own fields — but it is left as it was because the
        // two notes are written in the order they are made.
        //
        // Nothing here touches the material. The hash job reads the file
        // and writes the column; this is only what that result will be
        // compared against, and it survives the gap between the two
        // because the row does. What the claim *does* reach is the
        // detector, below the save — see `propose_from_declaration`.
        if let Some(declared) = &declared_hash {
            merge_trace_field(
                &mut asset.extra,
                content_hash::DECLARED_HASH_NOTE_KEY,
                content_hash::declaration_claim(declared),
            );
        }
        // The AlbumMeta statements, into the same bag and beside the two
        // notes above — through `merge_meta_entry` rather than assembled
        // into `_trace` here, because by this point `extra_json` has
        // replaced the whole bag and the claims have been written back
        // into it. Building the object locally would be the third writer
        // on `_trace` to carry off the other two.
        //
        // Every entry is stamped `pushed`. Everything on this command
        // arrived with the payload, and the reading a provenance claim
        // gets from its own form has no counterpart here: a value dug
        // back out of an artefact reaches the row through a reader,
        // which is a different path and stamps `embedded` itself.
        //
        // No operator, for the reason the claim note above has none —
        // the row carries its own `operator_ai` column, and the ingest
        // *is* the operation this entry describes. The field is for the
        // paths where the operation and the row it touches are separate
        // events, which the declaration verb is and this is not.
        for (key, value) in &album_meta {
            merge_meta_entry(
                &mut asset.extra,
                key,
                Some(crate::domain::album_meta::entry(
                    value,
                    provenance::source::PUSHED,
                    None,
                    asset.created_at.timestamp_millis(),
                )),
            );
        }

        // A save, and nothing to inspect afterwards. What used to sit
        // here was the lookup above written as a post-mortem: save,
        // read the driver's error text, re-query, and translate the
        // collision into "already imported" or "in the trash — restore
        // it". Both readings were the constraint speaking. The first is
        // no longer a failure (the row comes back and the caller has
        // it), and the second stopped meaning anything once the lookup
        // passes over the trash — a trashed row is in the way of
        // nothing, so a re-import mints, which is what the person
        // importing asked for.
        self.assets.save(&asset).await?;
        // A fresh asset extends the persona's corpus — every Query
        // Group scoped to it may pick this row up (invalidation).
        self.notify_persona_touched(persona_id);

        // The `derived_from` edges go in after the save because they
        // reference the row that save just created. A failure here is
        // reported rather than swallowed: the artefact is already
        // stored, so a caller that re-runs the ingest is handed the row
        // it already has (the lookup above) and the link is still
        // missing — the report is the only way it can know to retry.
        if let ResolvedOrigin::Resolved { parents, .. } = &origin {
            self.write_derived_edges(
                asset.id,
                parents,
                crate::domain::provenance::ClaimRelation::DerivedFrom,
            )
            .await?;
        }

        // **Step 4 of the ingest lifecycle, run on a claim rather than
        // on a measurement**. The importer was holding the
        // bytes at scan time, so a digest costs it a CPU pass and no
        // extra I/O; reading it here is what lets a match be proposed
        // without the server opening the file. The hash job still runs,
        // still writes the columns, and still checks the claim — this
        // does not replace any of that, it only arrives first.
        //
        // Below the save because the detector re-reads the row.
        if let Some(declared) = &declared_hash {
            self.propose_from_declaration(&asset.id, declared).await;
        }

        // Enqueue failures are intentionally not rolled back — jobs can be
        // rederived from the persisted state (for example, a null cover
        // signals that `cover_gen` still needs to run). `edge_rebuild` is
        // not enqueued here: the KeywordOverlap axis needs the auto-tag
        // results, so `auto_tag`'s handler chain-enqueues it once its
        // keywords are committed.
        let payload = serde_json::json!({ "asset_id": asset.id.to_string() });
        for kind in [JobKind::CoverGen, JobKind::AutoTag] {
            let _ = self.jobs.enqueue(kind, payload.clone()).await;
        }
        // `IndexRebuild` reads the full body from disk via the
        // `SourceTextReader` port and pushes a Tantivy document, so it
        // is enqueued only for assets whose bytes are text — the same
        // shape as the thumbnail decision below, which asks the format
        // before spending a job on it.
        //
        // Unconditional here once, and it cost the full-text index: a
        // 5,000-file PNG corpus was read as lossy UTF-8 into the body
        // cache and tokenised into Tantivy [measured 2026-08-05]. The
        // handler refuses these too (`index_rebuild` builds a
        // `TextLocator`), so this is the cheap half of the answer —
        // not enqueuing a job that would decline itself.
        let primary_mime = asset.materials.first().and_then(|m| m.mime.as_ref());
        if primary_mime.is_some_and(MimeType::body_text) {
            let _ = self
                .jobs
                .enqueue(JobKind::IndexRebuild, payload.clone())
                .await;
        }
        // A container takes its cover from its earliest member, so it
        // cannot have one until members exist — and the member that
        // *becomes* the earliest is not known until it lands. Re-enqueue
        // the container's `cover_gen` on every member ingest; the
        // handler no-ops once a cover is set, so the steady-state cost
        // is one job that reads a single column.
        if let Some(container_id) = asset.container_id {
            let _ = self
                .jobs
                .enqueue(
                    JobKind::CoverGen,
                    serde_json::json!({ "asset_id": container_id.to_string() }),
                )
                .await;
        }
        // Fingerprinting for duplicate detection reads every byte of
        // the original, so it goes in below the default priority: a
        // worker slot held by a 4 GB video hash is a slot not painting
        // thumbnails, and during an import wave the grid filling in is
        // what the user is watching. Nothing waits on the hash — the
        // duplicate report is a maintenance pass, not part of ingest.
        let _ = self
            .jobs
            .enqueue_with_priority(JobKind::MaterialHash, payload.clone(), -10)
            .await;
        // The visual feature (#112) reads and decodes the original, so
        // it sits below the default priority for the reason the
        // fingerprint does. Enqueued only for image-bearing assets —
        // the one family the encoder can answer for — and the handler
        // settles cheaply when no model is configured, so an ingest
        // that predates a model install costs one skipped row.
        if asset
            .materials
            .iter()
            .any(|m| matches!(m.mime, Some(MimeType::Image(_))))
        {
            let _ = self
                .jobs
                .enqueue_with_priority(JobKind::VisualFeature, payload.clone(), -10)
                .await;
        }
        // Chapters are a container's own statement about how its content
        // is divided, so the job is enqueued only for the families that
        // have a playback timeline to divide — the same shape as the
        // index and thumbnail decisions above, asking the format before
        // spending a job on it. The handler asks again through the same
        // predicate, so the two cannot disagree about what is eligible.
        //
        // Below the default priority for the reason the fingerprint is:
        // reading chapters spawns an external process per material, and
        // during an import wave the grid filling in is what the person
        // is watching. Nothing waits on the answer.
        if asset
            .materials
            .iter()
            .any(|m| m.mime.as_ref().is_some_and(MimeType::carries_chapters))
        {
            let _ = self
                .jobs
                .enqueue_with_priority(JobKind::ChapterScan, payload.clone(), -10)
                .await;
        }
        // Image thumbs are cached ahead of time for the sizes that the
        // grid actually paints. Larger sizes (detail overlay 512 px,
        // fullscreen 1024 px) are generated on demand at open time so
        // an Import wave does not have to burn CPU on preview sizes
        // most cards never open. Priority is set so smaller sizes get
        // popped by the worker first (grid paint speed comes before
        // hover preview).
        // "Can this be shown as a tile?" goes through the one render
        // policy, the same call `thumb_gen` makes when it picks the job
        // up — enqueue and handler agreeing by construction is the
        // point (they used to carry separate copies of the rule).
        let policy = crate::domain::render::render_policy(
            asset.materials.first().and_then(|m| m.mime.as_ref()),
            asset.role,
            false,
        );
        if policy.thumbnail {
            for (size_px, priority) in [(128u32, 10i32), (256u32, 5i32)] {
                let _ = self
                    .jobs
                    .enqueue_with_priority(
                        JobKind::ThumbGen,
                        serde_json::json!({
                            "asset_id": asset.id.to_string(),
                            "size_px": size_px,
                        }),
                        priority,
                    )
                    .await;
            }
        }

        // Auto-organize the just-saved asset through the same
        // idempotent `organize_by_location` path the manual endpoint
        // uses, so importers can "add + file" without a follow-up
        // sweep. Failures here are logged but do not fail the ingest
        // — a locator that doesn't sit under `base_dir` simply lands
        // unorganised (the existing `skipped` counter accounts for
        // it), which is the same outcome as running the manual
        // endpoint later.
        if let Some(base_dir) = auto_organize_base_dir {
            let _ = self
                .organize_by_location(
                    OrganizeByLocationCommand {
                        persona_id: Some(persona_id.to_string()),
                        base_dir: Some(base_dir),
                    },
                    attribution,
                )
                .await;
        }

        // UI reconciliation is driven by the existing `jobs:tick`
        // broadcast (jobs/mod.rs emits `{kind, ok}` on every job
        // completion — the follow-up cover_gen / index_rebuild /
        // thumb_gen etc. enqueued above will each fire one). The UI
        // listens to `jobs:tick` and invalidates on kinds that touch
        // the asset surface, so no dedicated notify job is needed here.

        Ok(asset_to_dto(&asset))
    }

    /// Ingests a batch of assets in one call — the bulk form of
    /// [`add`](Self::add). Each item is processed independently: an
    /// individual failure is captured in the result rather than
    /// aborting the whole batch. Follow-up pipeline jobs are enqueued
    /// exactly as they are for single-asset ingest.
    pub async fn add_batch(
        &self,
        command: AddAssetBatchCommand,
        attribution: &AttributionContext,
    ) -> Result<AddAssetBatchResult, DomainError> {
        // A batch-level auto-organize sweep amortises the Dir /
        // Group cache across the whole run, so we suppress the
        // per-item flag while it is set — otherwise a 89 k batch
        // would fire 89 k full sweeps back-to-back.
        let batch_auto_organize_base_dir = command.auto_organize_base_dir.clone();
        let suppress_per_item = batch_auto_organize_base_dir.is_some();
        let mut succeeded = Vec::with_capacity(command.items.len());
        let mut failed = Vec::with_capacity(command.items.len());
        let mut success_count = 0u64;
        let mut failure_count = 0u64;
        for mut item in command.items {
            if suppress_per_item {
                item.auto_organize_base_dir = None;
            }
            match self.add(item, attribution).await {
                Ok(dto) => {
                    succeeded.push(dto.id);
                    failed.push(String::new());
                    success_count += 1;
                }
                Err(err) => {
                    succeeded.push(String::new());
                    failed.push(err.to_string());
                    failure_count += 1;
                }
            }
        }
        // Session aggregates are derived at query time, so this
        // enqueue no longer rebuilds a precomputed store — it drives
        // the reconciliation handler whose visible effect is the
        // `sessions:progress` broadcast that refreshes the UI after an
        // Import. Only fired when at least one asset landed — a
        // wholly-failed batch has nothing to fold in. Best effort: a
        // queue failure here does not fail the Import.
        if success_count > 0 {
            let _ = self.rebuild_sessions().await;
        }
        // Batch-level auto-organize: single sweep over every asset
        // that lives under `base_dir`. Persona is left `None` so
        // mixed-persona batches (rare but legal) still land in the
        // right buckets. Idempotent, so it is safe to run even when
        // the batch mostly contained pre-existing assets that landed
        // under a `duplicate` error path.
        if let Some(base_dir) = batch_auto_organize_base_dir
            && success_count > 0
        {
            let _ = self
                .organize_by_location(
                    OrganizeByLocationCommand {
                        persona_id: None,
                        base_dir: Some(base_dir),
                    },
                    attribution,
                )
                .await;
        }
        Ok(AddAssetBatchResult {
            succeeded,
            failed,
            success_count,
            failure_count,
        })
    }

    /// Partially updates asset metadata; fields left `None` are unchanged.
    pub async fn update_meta(
        &self,
        command: UpdateAssetMetaCommand,
        _attribution: &AttributionContext,
    ) -> Result<AssetDto, DomainError> {
        let id = parse_asset_id(&command.asset_id)?;
        let mut asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;
        if let Some(labels) = command.labels {
            // Same rule as the ingest path: the replacement list is
            // taken as stated apart from repeats, which are dropped
            // first-wins (`dedup_labels`).
            asset.labels = dedup_labels(
                labels
                    .into_iter()
                    .map(Label::new)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        if let Some(register_note) = command.register_note {
            asset.register_note = Some(RegisterNote::new(register_note)?);
        }
        if let Some(cover) = command.cover {
            asset.cover = Some(CoverText::new(cover)?);
        }
        // Rating semantics: `Some(0)` clears the rating; `Some(1..=5)`
        // stores it; `None` leaves the field untouched (partial
        // update convention). Values > 5 are clamped so a
        // programmer-error caller cannot poison the DB.
        if let Some(rating) = command.rating {
            asset.rating = if rating == 0 {
                None
            } else {
                Some(rating.min(5))
            };
        }
        if let Some(modality) = command.modality {
            // `Some("")` could serve as an explicit "unclassify" verb
            // later; for now the partial-update convention only sets.
            asset.modality = Some(Modality::new(modality)?);
        }
        if let Some(title) = command.title {
            // Empty means "unname it" rather than "store an empty
            // string": a blank title would render as a card with no
            // text at all, which reads as a bug. Trimmed first so a
            // stray space cannot masquerade as a name.
            let trimmed = title.trim();
            asset.title = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        asset.updated_at = Utc::now();
        let persona_id = asset.persona_id;
        self.assets.save(&asset).await?;
        // Metadata edits (modality flip, label change, memo, etc.)
        // change fields Query Group rules can filter on.
        self.notify_persona_touched(persona_id);
        // Four of the fields this verb writes — title, cover, labels,
        // register note — are sections of the derived document, so
        // renaming an asset has to make it findable by the new name.
        // Unconditional rather than gated on which fields the command
        // carried: a rating-only edit costs one job that re-composes to
        // the same bytes, and a gate here would be a second copy of
        // `derive_text`'s field list, kept in step by hand.
        self.reindex(&id).await;
        Ok(asset_to_dto(&asset))
    }

    /// Partially updates metadata for multiple assets in one call.
    pub async fn update_meta_batch(
        &self,
        command: UpdateAssetMetaBatchCommand,
        attribution: &AttributionContext,
    ) -> Result<UpdateAssetMetaBatchResult, DomainError> {
        let mut succeeded = Vec::with_capacity(command.items.len());
        let mut failed = Vec::with_capacity(command.items.len());
        let mut success_count = 0u64;
        let mut failure_count = 0u64;

        for item in command.items {
            match self.update_meta(item, attribution).await {
                Ok(dto) => {
                    succeeded.push(Some(dto));
                    failed.push(String::new());
                    success_count += 1;
                }
                Err(err) => {
                    succeeded.push(None);
                    failed.push(err.to_string());
                    failure_count += 1;
                }
            }
        }

        Ok(UpdateAssetMetaBatchResult {
            succeeded,
            failed,
            success_count,
            failure_count,
        })
    }

    /// Normalises the optional remark a selection gesture carries
    /// (#65): trimmed, and whitespace-only reads as absent — the same
    /// silent discard the comment UI applies to an empty submit, so a
    /// caller wiring a text field straight through never turns a blank
    /// into an error.
    fn gesture_remark(comment: Option<&str>) -> Option<&str> {
        comment.map(str::trim).filter(|c| !c.is_empty())
    }

    /// Appends the footnote a selection gesture carried (#65): one
    /// [`AssetComment`] pinned to the verb. `at` is the verb's own
    /// clock read — the instant `trash` stamped `trashed_at` with, or
    /// the one read `restore` / `trash_group` took for the gesture —
    /// so the remark and the gesture genuinely share a moment instead
    /// of each reading the clock and landing milliseconds apart.
    ///
    /// The author is [`CommentAuthor::User`] — the comment-side alias
    /// of the attribution `Owner`. The gesture commands carry no
    /// author fields, and a comment records who is *speaking*, not who
    /// is accountable (`AssetCommentService`'s stance); in a
    /// single-user vault the voice stating a culling reason is "me".
    /// The attribution handed to the verb stays unpersisted here, the
    /// same as on every comment write — closing that gap is #65's
    /// second open question, not this write.
    ///
    /// Called after the gesture has landed, so a failed gesture never
    /// leaves a footnote claiming it happened; a failure *here*
    /// surfaces to the caller with the gesture already in place, which
    /// is the recoverable side of that trade (the verbs are idempotent
    /// or reversible, prose handed in and dropped is neither).
    async fn post_gesture_comment(
        &self,
        asset_id: AssetId,
        body: &str,
        gesture: SelectionGesture,
        at: chrono::DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let comment = AssetComment::for_gesture(asset_id, CommentAuthor::User, body, gesture, at)?;
        self.comments.save(&comment).await
    }

    /// Moves an asset to the trash. Reversible via
    /// [`restore`](Self::restore) — every dependent row (tags, group
    /// filing and its order, comments, body, thumbnails, snapshot
    /// membership) is left untouched, so nothing has to be replayed.
    ///
    /// The search document *is* dropped, because a trashed asset must
    /// not come back as a search hit; [`restore`](Self::restore)
    /// re-indexes it.
    ///
    /// An optional remark (#65) lands as a gesture-pinned comment
    /// after the trash succeeds. No re-index follows it: the document
    /// is being dropped anyway, and the restore-side rebuild
    /// re-composes from the thread, remark included.
    pub async fn trash(
        &self,
        command: TrashAssetCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_asset_id(&command.asset_id)?;
        let asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;
        let persona_id = asset.persona_id;
        let now = Utc::now();
        self.assets.trash(&id, now).await?;
        if let Some(body) = Self::gesture_remark(command.comment.as_deref()) {
            self.post_gesture_comment(id, body, SelectionGesture::Trash, now)
                .await?;
        }
        self.unindex_removed_asset(&id).await;
        self.notify_persona_touched(persona_id);
        Ok(())
    }

    /// Returns a trashed asset to the live set and re-indexes it for
    /// search. Idempotent — restoring a live asset is a no-op.
    ///
    /// Re-indexing goes through [`JobKind::IndexRebuild`] rather than a
    /// direct indexer call: the body has to be re-read from the
    /// source locator, and that logic (plus its "source unreadable"
    /// handling) already lives in the job handler.
    ///
    /// An enqueue failure is logged and the restore still succeeds —
    /// refusing a restore because a queue write failed would be the
    /// worse trade. Be aware of what that costs, though: **there is no
    /// automatic backfill for this case.** The index backfill scan finds
    /// assets with no `asset_body` row, and trash deliberately preserves
    /// `asset_body`, so a restored asset is invisible to it. If the
    /// enqueue is lost the asset is restored but unsearchable until
    /// something re-indexes it explicitly.
    pub async fn restore(
        &self,
        command: RestoreAssetCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_asset_id(&command.asset_id)?;
        let asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;
        // Restoring one asset out from under a trashed persona would put
        // it back in a grid the user believes is empty, and the persona's
        // own restore would not be able to reclaim it (the stamps no
        // longer match). Restoring the persona is the operation that
        // brings this asset back with everything else.
        if let Some(persona) = self.personas.find(&asset.persona_id).await?
            && persona.trashed_at.is_some()
        {
            return Err(DomainError::Conflict(format!(
                "persona {} is in the trash; restore the persona to bring its assets back",
                asset.persona_id
            )));
        }
        self.assets.restore(&id).await?;
        // The salvage remark (#65) goes in before the re-index is
        // queued, so the rebuild composes a document that already
        // contains it — one job answers for both. `restore` stamps no
        // timestamp of its own (it clears one), so the clock read here
        // *is* the gesture's moment.
        if let Some(body) = Self::gesture_remark(command.comment.as_deref()) {
            self.post_gesture_comment(id, body, SelectionGesture::Restore, Utc::now())
                .await?;
        }
        if let Err(err) = self
            .jobs
            .enqueue(
                JobKind::IndexRebuild,
                serde_json::json!({ "asset_id": id.to_string() }),
            )
            .await
        {
            tracing::warn!(
                event = "diag.reindex.enqueue_failed",
                asset_id = %id,
                error = %err,
                "could not enqueue reindex for restored asset"
            );
        }
        self.notify_persona_touched(asset.persona_id);
        Ok(())
    }

    /// Permanently deletes an **already-trashed** asset; the FK cascade
    /// takes its dependent rows. Returns `Conflict` when the asset is
    /// still live — purge is reachable only through the trash, so a
    /// runaway bulk caller always leaves a recoverable state behind.
    ///
    /// A missing asset is a no-op rather than an error: the caller's
    /// intent ("this must be gone") already holds, and purge is the one
    /// verb where retrying after a partial failure has to be safe.
    pub async fn purge(
        &self,
        command: PurgeAssetCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_asset_id(&command.asset_id)?;
        let persona_id = self.assets.find(&id).await?.map(|asset| asset.persona_id);
        // Row first, index second — deliberately. Dropping the document
        // up front looks tidier but destroys it even when `purge`
        // rejects the call, so a bulk purge aimed at live assets would
        // 409 on every one *and* strip them all from search, with no
        // repair path (the index backfill only finds assets that have no
        // `asset_body` row, and a live asset has one). Deleting after
        // the row is gone risks at most a stale document, which is the
        // failure this code already tolerates and logs.
        self.assets.purge(&id).await?;
        self.unindex_removed_asset(&id).await;
        if let Some(persona_id) = persona_id {
            self.notify_persona_touched(persona_id);
        }
        Ok(())
    }

    /// Permanently deletes every asset in the trash. Irreversible.
    ///
    /// Shaped like
    /// [`RetentionService::purge_expired`](crate::application_support::RetentionService::purge_expired)
    /// rather than a loop over [`purge`](Self::purge): one row's
    /// failure must not abort the rest (the realistic cause is a
    /// restore landing between the scan and the purge, which comes
    /// back as `Conflict`), and the index deletions go behind one
    /// commit per page instead of one per asset.
    ///
    /// Runs page by page so a trash larger than the scan window still
    /// empties in a single call. Two exits, and both are about the
    /// same hazard — a row that keeps failing is a row the next scan
    /// hands straight back: a short page means the whole trash has
    /// been seen, and a full page that purged nothing means every row
    /// on it failed. Without them the loop would not terminate.
    ///
    /// Ignores every grid filter by construction: the command carries
    /// none. See [`EmptyTrashCommand`] for why.
    pub async fn empty_trash(
        &self,
        _command: EmptyTrashCommand,
        _attribution: &AttributionContext,
    ) -> Result<EmptyTrashResult, DomainError> {
        let mut result = EmptyTrashResult::default();
        let mut touched_personas: HashSet<crate::domain::value::PersonaId> = HashSet::new();

        loop {
            let ids = self.assets.list_trashed_ids(EMPTY_TRASH_SCAN_PAGE).await?;
            if ids.is_empty() {
                break;
            }
            let scanned = ids.len();
            let mut purged_ids = Vec::with_capacity(scanned);
            for id in ids {
                // Read the owner before the row is gone; after the
                // purge there is no way back to it, and the Query
                // Group invalidator keys off the persona.
                let persona_id = self.assets.find(&id).await?.map(|asset| asset.persona_id);
                match self.assets.purge(&id).await {
                    Ok(()) => {
                        if let Some(persona_id) = persona_id {
                            touched_personas.insert(persona_id);
                        }
                        purged_ids.push(id);
                        result.purged += 1;
                    }
                    Err(err) => {
                        tracing::warn!(
                            event = "diag.trash.empty_skipped",
                            asset_id = %id,
                            error = %err,
                            "empty trash skipped an asset"
                        );
                        result.skipped += 1;
                    }
                }
            }
            self.unindex_removed_assets(&purged_ids).await;
            // A short page is the whole trash, so there is nothing
            // left to scan for — including the rows that just failed,
            // which the next scan would only hand back to fail again
            // and be counted a second time.
            if scanned < EMPTY_TRASH_SCAN_PAGE as usize {
                break;
            }
            // A full page that moved nothing: every row on it failed,
            // and the next scan would return the same page.
            if purged_ids.is_empty() {
                break;
            }
        }

        for persona_id in touched_personas {
            self.notify_persona_touched(persona_id);
        }
        Ok(result)
    }

    /// Takes one asset the user just removed out of search.
    async fn unindex_removed_asset(&self, id: &crate::domain::value::AssetId) {
        self.unindex_removed_assets(std::slice::from_ref(id)).await;
    }

    /// Takes the assets a single user action removed out of search,
    /// behind one commit.
    ///
    /// Called after a trash / purge the user asked for — including the
    /// cascade when they purge a whole persona. The retention sweep has
    /// its own twin
    /// ([`RetentionService`](crate::application_support::RetentionService)):
    /// same effect on the index, different reason to be doing it, and a
    /// service a transport can reach has no business carrying the
    /// clock-driven one.
    ///
    /// Failures are logged rather than propagated. The row write this
    /// follows has already happened, so returning an error here would
    /// tell the caller nothing happened while the removal stands. A
    /// stale document is the recoverable direction — the asset lingers
    /// as a retrieval candidate until the next reindex.
    async fn unindex_removed_assets(&self, ids: &[crate::domain::value::AssetId]) {
        if ids.is_empty() {
            return;
        }
        let mut dropped = false;
        for id in ids {
            match self.indexer.remove(id).await {
                Ok(()) => dropped = true,
                Err(err) => tracing::warn!(
                    event = "diag.retrieval.remove_failed",
                    asset_id = %id,
                    error = %err,
                    "retrieval index remove failed"
                ),
            }
        }
        if dropped && let Err(err) = self.indexer.flush().await {
            tracing::warn!(
                event = "diag.retrieval.flush_failed",
                dropped = ids.len(),
                error = %err,
                "retrieval index flush failed after dropping documents"
            );
        }
    }

    /// Grid listing (read hot path; returns `AssetCard` projections).
    ///
    /// Two orders are reachable. Without `query.sort` the repository's own
    /// arrival order stands (the manual arrangement for a single-Group
    /// filter, `occurred_at DESC` otherwise) and the page comes straight
    /// out of SQL. Naming an axis routes through
    /// [`list_sorted`](Self::list_sorted) instead — the axis has to be
    /// applied to the whole filtered set before the page is cut, so the
    /// two cannot share the SQL `LIMIT`.
    pub async fn list(&self, query: ListAssetsQuery) -> Result<AssetPageDto, DomainError> {
        let domain_query = to_asset_query(&query)?;
        match &query.sort {
            None => Ok(page_to_dto(&self.assets.list(&domain_query).await?)),
            Some(spec) => self.list_sorted(&domain_query, spec).await,
        }
    }

    /// [`list`](Self::list) with a caller-named axis: order the whole
    /// filtered set through [`sort_asset_ids`], cut the page from the
    /// result, then hydrate.
    ///
    /// Same three steps the Query Group evaluator runs before it freezes
    /// `position` (`query_group_service`), minus the Tantivy intersection —
    /// a full-text term goes through
    /// [`search`](Self::search), which keeps BM25 rank on purpose.
    ///
    /// The group closure is deliberately **not** expanded here. `list`
    /// answers the filter it was handed (the desktop client expands
    /// client-side before it asks), and an axis must not change which rows
    /// come back — only their order.
    ///
    /// Cost: one un-`LIMIT`ed scan of the filter per call (measured at
    /// 68 ms / 100k rows for `fetch_sortable_assets`) plus the
    /// hydration of one page. Paying it is opt-in per request.
    async fn list_sorted(
        &self,
        query: &crate::domain::asset::AssetQuery,
        spec: &asterism_contract::sort::SortSpec,
    ) -> Result<AssetPageDto, DomainError> {
        let mut whole = query.clone();
        whole.offset = 0;
        whole.limit = u64::MAX; // ignored by fetch_sortable_assets (no LIMIT)
        let assets = self.query_groups.fetch_sortable_assets(&whole).await?;

        let ctx = crate::application::sort_context::build_sort_context(
            &*self.personas,
            &*self.assets,
            &*self.groups,
            query.persona_id.as_ref(),
        )
        .await?;
        let ordered = crate::domain::sort_eval::sort_asset_ids(spec, &assets, &ctx)?;
        let total = ordered.len() as u64;

        let page_ids = ordered
            .iter()
            .skip(query.offset as usize)
            .take(query.limit as usize)
            .map(|id| parse_asset_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let cards = self.assets.cards_by_ids(&page_ids, &query.viewer).await?;

        // `cards_by_ids` answers as a set — it is free to reorder, and it
        // drops ids the viewer cannot see. Re-project onto the sorted ids
        // so the response carries the order that was just computed.
        let mut by_id: HashMap<String, _> =
            cards.into_iter().map(|c| (c.id.to_string(), c)).collect();
        let items = page_ids
            .iter()
            .filter_map(|id| by_id.remove(&id.to_string()))
            .map(|c| crate::application::mapping::card_to_dto(&c))
            .collect();

        Ok(AssetPageDto {
            items,
            offset: query.offset,
            limit: query.limit,
            // Count of the whole filtered set, matching what the unsorted
            // path reports — pagination is over the sorted sequence, so the
            // total is the sequence length, not the page's.
            total: Some(total),
        })
    }

    /// Index-only listing for 6-figure grids. Returns everything the
    /// client needs to build the full-page virtualised scroll
    /// (order + per-row sort / filter fields) without paying the
    /// cover-text / source-locator serialisation cost. The
    /// frontend hydrates the ~40-row visible viewport separately
    /// via [`hydrate_cards`](Self::hydrate_cards).
    /// Honours `query.sort` on the same terms [`list`](Self::list) does.
    /// Without that branch the axis was accepted and dropped: the index
    /// path is what the grid reads for a non-search list, so an HTTP
    /// caller naming `Sort` / `Order` got the repository's arrival order
    /// back and no indication that its request had been ignored. The
    /// desktop client sends no axis (it sorts the index rows itself),
    /// which is why nothing noticed.
    pub async fn list_index(
        &self,
        query: ListAssetsQuery,
    ) -> Result<asterism_contract::dto::AssetIndexPageDto, DomainError> {
        let domain_query = to_asset_query(&query)?;
        match &query.sort {
            None => Ok(crate::application::mapping::index_page_to_dto(
                &self.assets.list_index(&domain_query).await?,
            )),
            Some(spec) => self.index_sorted(&domain_query, spec).await,
        }
    }

    /// [`list_index`](Self::list_index) with a caller-named axis — the
    /// index twin of [`list_sorted`](Self::list_sorted), step for step,
    /// so the two transports cannot answer a given `Sort` / `Order` pick
    /// differently.
    ///
    /// The ordering is computed over [`SortableAsset`] rows rather than
    /// over the index projection because the `Cover` axis needs cover
    /// text, which the index deliberately drops. Sorting the light rows
    /// directly would silently degrade that one axis to a no-op —
    /// the defect this method exists to fix, one axis smaller.
    ///
    /// [`SortableAsset`]: crate::domain::sort_eval::SortableAsset
    async fn index_sorted(
        &self,
        query: &crate::domain::asset::AssetQuery,
        spec: &asterism_contract::sort::SortSpec,
    ) -> Result<asterism_contract::dto::AssetIndexPageDto, DomainError> {
        let mut whole = query.clone();
        whole.offset = 0;
        whole.limit = u64::MAX; // ignored by fetch_sortable_assets (no LIMIT)
        let assets = self.query_groups.fetch_sortable_assets(&whole).await?;

        let ctx = crate::application::sort_context::build_sort_context(
            &*self.personas,
            &*self.assets,
            &*self.groups,
            query.persona_id.as_ref(),
        )
        .await?;
        let ordered = crate::domain::sort_eval::sort_asset_ids(spec, &assets, &ctx)?;
        let total = ordered.len() as u64;

        let page_ids = ordered
            .iter()
            .skip(query.offset as usize)
            .take(query.limit as usize)
            .map(|id| parse_asset_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let rows = self.assets.index_by_ids(&page_ids, &query.viewer).await?;

        // `index_by_ids` answers as a set. Re-project onto the sorted ids
        // so the response carries the order just computed.
        let mut by_id: HashMap<String, _> =
            rows.into_iter().map(|r| (r.id.to_string(), r)).collect();
        let items = page_ids
            .iter()
            .filter_map(|id| by_id.remove(&id.to_string()))
            .map(|r| crate::application::mapping::index_to_dto(&r))
            .collect();

        Ok(asterism_contract::dto::AssetIndexPageDto {
            items,
            offset: query.offset,
            limit: query.limit,
            // Length of the sorted sequence, matching what the unsorted
            // path reports: pagination is over that sequence.
            total: Some(total),
        })
    }

    /// Hydrates a batch of card projections by id. Companion to
    /// [`list_index`](Self::list_index) — the frontend keeps a
    /// full-page index and calls this for the ~40 rows the VList
    /// is about to paint (plus a small prefetch window). Ids
    /// hidden from the viewer are dropped from the response.
    ///
    /// # The caller passes ids a filtered read already vetted
    ///
    /// That is this verb's contract, and it is why folds are **not**
    /// redirected here while every other id-set hydration redirects them
    /// ([`fold_redirect`](crate::application::fold_redirect)). The ids
    /// come from a `list_index` page, which applied the enumerating half
    /// of the fold read rule in SQL, so a headstone cannot be among
    /// them; redirecting would be a second resolve per viewport paint on
    /// the hottest read in the app, answering a question already
    /// answered.
    ///
    /// A caller that holds ids from somewhere else — a freeze, a stored
    /// export, a note written months ago — wants `fold_redirect` first,
    /// or `snapshot_members` / the constellation, which do it for it.
    pub async fn hydrate_cards(
        &self,
        ids: Vec<String>,
        viewer_subject: Option<String>,
    ) -> Result<Vec<asterism_contract::dto::AssetCardDto>, DomainError> {
        let parsed: Vec<_> = ids
            .into_iter()
            .map(|s| parse_asset_id(&s))
            .collect::<Result<_, _>>()?;
        let viewer = match viewer_subject {
            None => crate::domain::value::Viewer::Owner,
            Some(s) => crate::domain::value::Viewer::Subject(s),
        };
        let cards = self.assets.cards_by_ids(&parsed, &viewer).await?;
        Ok(cards
            .iter()
            .map(crate::application::mapping::card_to_dto)
            .collect())
    }

    /// Retrieval under the grid's active filter — shortlist first,
    /// then narrow.
    ///
    /// Two steps, **in this order**:
    ///
    /// 1. [`AssetRetriever`] returns a ranked shortlist of candidate
    ///    ids. Retrieval has no `WHERE`-expressible form, so it cannot
    ///    be folded into the SQL predicate.
    /// 2. [`AssetRepository::filter_ids`] narrows those candidates by
    ///    the *same* filter surface the list path uses (modality / tag /
    ///    group / occurred range / label / container / visibility),
    ///    through the shared `QueryParts` builder.
    ///
    /// Survivors are put back into rank order, paged, then hydrated
    /// into `AssetCardDto` with `score` + `snippet` populated.
    ///
    /// The order matters and cannot be reversed: narrowing first would
    /// mean handing an arbitrarily large id set to the retriever, which
    /// is not a shape it can take.
    ///
    /// # What this answers, and what it does not
    ///
    /// The answer is **a shortlist narrowed by a filter**, not "every
    /// asset matching text + filter". Retrieval looks at
    /// [`RETRIEVAL_K_CEILING`](crate::domain::repository::RETRIEVAL_K_CEILING)
    /// candidates; assets past that are never seen by the filter half,
    /// so a common word under a narrow filter can come back thin. That
    /// is the contract, not a defect — an exhaustive, countable answer
    /// is what the Query side is for, and the text predicate that makes
    /// it reachable from here lands in a later wave (W2).
    ///
    /// The response says so in its own shape: [`RetrievedPageDto`] has
    /// no `total`. It carries `matched` (how many candidates passed the
    /// filter), `candidates_considered`, and `truncated` — enough to
    /// phrase the answer as "the best M of the N we looked at" and not
    /// enough to phrase it as a count of the library.
    ///
    /// # Order is relevance, and `sort` is refused
    ///
    /// The filter half is [`ListAssetsQuery`] verbatim, so it carries the
    /// `sort` axis the list path honours. This path does not: the answer
    /// order *is* the ranking. A named axis is rejected
    /// ([`DomainError::Validation`], `400`) rather than dropped — see the
    /// body for why, and use [`list`](Self::list) for a sorted listing.
    pub async fn search(&self, query: SearchAssetsQuery) -> Result<RetrievedPageDto, DomainError> {
        let text = query.text.trim();
        // Search reads the Tantivy index, which by construction holds
        // only live assets (trash drops the document, restore re-adds
        // it). So the trash selector cannot be honoured here — and
        // silently ignoring it would be worse than refusing: a caller
        // asking for the trash side would get the live side back and
        // believe the trash was empty. Validate explicitly, including
        // the typo case the wire contract promises to reject.
        match crate::application::mapping::parse_trash_filter(query.filter.trash.as_deref())? {
            crate::domain::asset::TrashFilter::LiveOnly => {}
            _ => {
                return Err(DomainError::Validation(
                    "search covers live assets only; the trash has no search index".into(),
                ));
            }
        }
        // `filter` is the list query verbatim, so it carries a `sort`
        // field this path cannot honour: the result sequence *is* the BM25
        // ranking, and an axis would have to discard it. Refuse rather
        // than accept-and-drop — the wire already rejects a misspelled
        // axis (`deserialize_sort`), so accepting a well-spelled one and
        // answering in relevance order would make the correctness of the
        // spelling decide whether the caller is told anything at all.
        //
        // Checked before the empty-text early return below, like the trash
        // selector above: whether the request is answerable must not depend
        // on whether it happens to match nothing.
        //
        // Lifting this is a real feature, not a shim: re-sorting the hits
        // means fetching `SortableAsset` rows for the survivors and
        // deciding what relevance still means once another axis owns the
        // order (a `relevance` target, or a documented two-key order).
        // The unblock point is this branch — delete it and apply the spec
        // to the `matched` vector below, where `total` is taken.
        if query.filter.sort.is_some() {
            return Err(DomainError::Validation(
                "search results are ranked by relevance; sort is not supported on the search \
                 path — use asset_list with filters for sorted listings"
                    .into(),
            ));
        }
        // The full filter, parsed exactly as the list path parses it —
        // `limit` / `offset` on the domain query are unused here because
        // paging happens after the intersection.
        //
        // Parsed before the empty-text early return below, like the trash
        // selector and the sort axis above and for the reason already given
        // there: whether the request is answerable must not depend on
        // whether it happens to match nothing. An inverted band is a fault
        // in the request, and this parse is where that is said, so leaving
        // it under the short-circuit made the same unanswerable band a
        // `400` with a search term and a `200` with an empty page without
        // one. The bands were simply the case that was missed when the
        // other two were lifted: the rating band has sat below the
        // short-circuit since it was wired, and this one move closes it
        // there too.
        let filter = to_asset_query(&query.filter)?;
        // Default paging when the caller left the fields at 0
        // (matches the SQL-side default previously baked into
        // `to_asset_query`).
        let raw_limit = if query.filter.limit == 0 {
            50
        } else {
            query.filter.limit
        };
        let limit = raw_limit.clamp(1, 500);
        let offset = query.filter.offset;
        let empty_page = |considered: u64| RetrievedPageDto {
            items: Vec::new(),
            offset,
            limit,
            matched: 0,
            candidates_considered: considered,
            truncated: false,
        };
        if text.is_empty() {
            return Ok(empty_page(0));
        }
        // The shortlist is asked for at its full width rather than
        // `limit + offset`: the SQL half prunes it afterwards, so a
        // page-sized shortlist would starve a filtered page.
        let found = self
            .retriever
            .retrieve(&crate::domain::repository::RetrievalQuery {
                intent: crate::domain::repository::RetrievalIntent::Text(text.to_string()),
                scope: filter.persona_id,
                k: crate::domain::repository::RETRIEVAL_K_CEILING,
            })
            .await?;
        // How wide a net was cast, reported verbatim so the answer can
        // be phrased as "the best M of the N we looked at" rather than
        // as a count of the library.
        let candidates_considered = found.candidates.len() as u64;
        let truncated = found.truncated;
        if found.candidates.is_empty() {
            return Ok(empty_page(0));
        }
        let candidate_ids: Vec<crate::domain::value::AssetId> =
            found.candidates.iter().map(|c| c.asset_id).collect();
        let kept: std::collections::HashSet<_> = self
            .assets
            .filter_ids(&candidate_ids, &filter)
            .await?
            .into_iter()
            .collect();
        // Back into rank order (`filter_ids` return order is
        // unspecified), then page.
        let matched: Vec<_> = found
            .candidates
            .into_iter()
            .filter(|c| kept.contains(&c.asset_id))
            .collect();
        let matched_count = matched.len() as u64;
        let page: Vec<_> = matched
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        if page.is_empty() {
            return Ok(RetrievedPageDto {
                items: Vec::new(),
                offset,
                limit,
                matched: matched_count,
                candidates_considered,
                truncated,
            });
        }
        // Hydrate the page slice in one round-trip. `cards_by_ids`
        // re-applies the visibility filter for the same viewer.
        let ids: Vec<crate::domain::value::AssetId> = page.iter().map(|c| c.asset_id).collect();
        let cards = self.assets.cards_by_ids(&ids, &filter.viewer).await?;
        // Index by id so rank order is preserved.
        let mut by_id: std::collections::HashMap<_, _> =
            cards.into_iter().map(|c| (c.id, c)).collect();
        let mut items = Vec::with_capacity(page.len());
        for cand in &page {
            if let Some(card) = by_id.remove(&cand.asset_id) {
                // The wire carries a snippet field, so only that shape
                // of evidence survives the mapping today. Other routes
                // (tags / rationale) reach the wire in a later wave.
                let snippet = match &cand.evidence {
                    crate::domain::repository::Evidence::Snippet(s) => Some(s.clone()),
                    _ => None,
                };
                items.push(crate::application::mapping::card_to_dto_with_hit(
                    &card, cand.score, snippet,
                ));
            }
        }
        Ok(RetrievedPageDto {
            items,
            offset,
            limit,
            matched: matched_count,
            candidates_considered,
            truncated,
        })
    }

    /// The same retrieval as [`search`](Self::search), reduced to the
    /// rank order — ids only, no cards, no paging.
    ///
    /// Serves the second composition form: the caller
    /// already holds an exact page from the Query side and wants
    /// Retrieval to decide only *the sequence* (`✦ Relevance` in the
    /// grid's sorter). Membership stays with the page it holds; this
    /// answers "which of these is the better match", so hydrating cards
    /// here would fetch rows the caller has already got.
    ///
    /// Everything before the hydration step is the same code path as
    /// `search`: the same validation (a `sort` axis, any trash selector
    /// but live, and an inverted band are `400`s — and all three are
    /// answered before the empty-query short-circuit, for the reasons
    /// written there),
    /// the same shortlist width ([`RETRIEVAL_K_CEILING`]), and the same
    /// narrowing through
    /// [`filter_ids`](crate::domain::repository::AssetRepository::filter_ids).
    /// Filtered candidates come back in rank order and all of them are
    /// returned — a rank hint that stopped at a page boundary would
    /// leave the rest of the grid unranked while the user scrolled.
    ///
    /// # Visibility
    ///
    /// The viewer clause is applied by `filter_ids` (it is part of the
    /// shared `QueryParts` predicate), so hidden assets never reach the
    /// response. The second check `search` gets from `cards_by_ids` is
    /// absent here because there is nothing to hydrate — which is
    /// harmless for this shape: the ids are used to *order* items the
    /// caller fetched through its own visibility-checked read, and an
    /// id the caller cannot see simply matches nothing on its side.
    /// This return value must never be used to decide what exists.
    pub async fn search_ids(
        &self,
        query: SearchAssetsQuery,
    ) -> Result<RetrievedIdsDto, DomainError> {
        let text = query.text.trim();
        // Same three refusals as `search`, in the same order and for the
        // same reasons (see that body): the trash has no index, the
        // answer's order *is* the ranking so a named axis cannot be
        // honoured — and must not be silently dropped — and an inverted
        // band is a fault in the request. All three are checked before
        // the empty-text early return below, which is where their whole
        // point lies.
        match crate::application::mapping::parse_trash_filter(query.filter.trash.as_deref())? {
            crate::domain::asset::TrashFilter::LiveOnly => {}
            _ => {
                return Err(DomainError::Validation(
                    "search covers live assets only; the trash has no search index".into(),
                ));
            }
        }
        if query.filter.sort.is_some() {
            return Err(DomainError::Validation(
                "search results are ranked by relevance; sort is not supported on the search \
                 path — use asset_list with filters for sorted listings"
                    .into(),
            ));
        }
        // The third refusal travels inside the filter parse rather than
        // in a branch of its own, which is the only thing that differs
        // from the two above; the reasoning for its position is the
        // paragraph in `search` and is not repeated here.
        let filter = to_asset_query(&query.filter)?;
        let empty = || RetrievedIdsDto {
            ids: Vec::new(),
            candidates_considered: 0,
            truncated: false,
        };
        if text.is_empty() {
            return Ok(empty());
        }
        let found = self
            .retriever
            .retrieve(&crate::domain::repository::RetrievalQuery {
                intent: crate::domain::repository::RetrievalIntent::Text(text.to_string()),
                scope: filter.persona_id,
                k: crate::domain::repository::RETRIEVAL_K_CEILING,
            })
            .await?;
        // Bounded by `RETRIEVAL_K_CEILING`, which is a `u32`.
        let candidates_considered = found.candidates.len() as u32;
        let truncated = found.truncated;
        if found.candidates.is_empty() {
            return Ok(empty());
        }
        let candidate_ids: Vec<crate::domain::value::AssetId> =
            found.candidates.iter().map(|c| c.asset_id).collect();
        let kept: std::collections::HashSet<_> = self
            .assets
            .filter_ids(&candidate_ids, &filter)
            .await?
            .into_iter()
            .collect();
        // Back into rank order — `filter_ids` return order is
        // unspecified, and the order is the entire payload here.
        let ids: Vec<String> = found
            .candidates
            .into_iter()
            .filter(|c| kept.contains(&c.asset_id))
            .map(|c| c.asset_id.to_string())
            .collect();
        Ok(RetrievedIdsDto {
            ids,
            candidates_considered,
            truncated,
        })
    }

    /// A random handful out of the filtered set — the sidebar's
    /// "🎲 Random".
    ///
    /// Retrieval-shaped, SQL-implemented. The behaviour is the
    /// Retrieval side's (no determinism promised, the
    /// same request may answer differently every time), but there is no
    /// retriever in the path: `RetrievalQuery`'s scope is the persona
    /// alone, and the picks have to come out of the *whole* filter — every
    /// chip plus `text_match`. Widening the port for one intent would put
    /// the grid's filter surface into a contract that does not want it, so
    /// this reads the repository directly and says so. When `Similar`
    /// lands and a composite retriever exists, this is a candidate to fold
    /// into it.
    ///
    /// # Two differences from [`search`](Self::search)
    ///
    /// * **Trash is honoured**, not refused. `search` cannot answer for
    ///   the trash because its index holds live rows only; this is a
    ///   `WHERE` clause over the `asset` table, where the trashed half is
    ///   as reachable as the live one.
    /// * **`set_total` is exact.** The pool is a SQL predicate, so its
    ///   size is a real count with no shortlist ceiling behind it — the
    ///   number `RetrievedPageDto` withholds precisely because retrieval
    ///   cannot produce it honestly.
    ///
    /// `filter.sort` is refused (`400`) for the reason `search` refuses
    /// it: the order *is* the shuffle, so an axis would have to discard
    /// it, and accepting-and-dropping would leave the caller believing a
    /// sort happened. `filter.limit` / `filter.offset` are ignored — a
    /// sample has no pages, and `k` is the only size knob.
    pub async fn sample(&self, query: RandomAssetsQuery) -> Result<SampledPageDto, DomainError> {
        if query.filter.sort.is_some() {
            return Err(DomainError::Validation(
                "random picks are shuffled; sort is not supported on the random path — use \
                 asset_list with filters for sorted listings"
                    .into(),
            ));
        }
        let k = query
            .k
            .unwrap_or(RANDOM_PICKS_DEFAULT)
            .clamp(1, RANDOM_PICKS_MAX);
        let filter = to_asset_query(&query.filter)?;
        let ids = self.assets.sample(&filter, k).await?;
        if ids.is_empty() {
            // An empty draw can only mean an empty pool: `k` is at least
            // 1, so `LIMIT k` over a non-empty set always returns
            // something. Saying `set_total: 0` here therefore states a
            // fact rather than skipping the count.
            return Ok(SampledPageDto {
                items: Vec::new(),
                picked: 0,
                set_total: 0,
            });
        }
        // The pool's size, from the list path's own count over the same
        // predicate — one row is fetched and thrown away, which is the
        // cost of not growing a second counting verb for one caller.
        let mut count_probe = filter.clone();
        count_probe.limit = 1;
        count_probe.offset = 0;
        let set_total = self.assets.list(&count_probe).await?.total.unwrap_or(0);
        // Hydrate in one round-trip, then put the cards back into the
        // order the draw produced. `cards_by_ids` may reorder freely and
        // drops rows the viewer cannot see, so the sequence has to be
        // rebuilt from the ids rather than taken from the reply.
        let cards = self.assets.cards_by_ids(&ids, &filter.viewer).await?;
        let mut by_id: std::collections::HashMap<_, _> =
            cards.into_iter().map(|c| (c.id, c)).collect();
        let mut items = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(card) = by_id.remove(id) {
                items.push(crate::application::mapping::card_to_dto(&card));
            }
        }
        Ok(SampledPageDto {
            picked: items.len() as u32,
            items,
            set_total,
        })
    }

    /// Sessions view — one row per `session_id` present in the query
    /// scope. The Messages view drills into a specific session by
    /// setting `session_id` on the query and calling `list`.
    pub async fn list_sessions(
        &self,
        query: ListAssetsQuery,
    ) -> Result<asterism_contract::dto::SessionPageDto, DomainError> {
        let domain_query = to_asset_query(&query)?;
        Ok(crate::application::mapping::session_page_to_dto(
            &self.assets.list_sessions(&domain_query).await?,
        ))
    }

    /// Sidebar Tags section — every tag paired with the number of
    /// distinct assets attached to it. Tags with a zero count in the
    /// requested scope are dropped so the sidebar does not list dead
    /// channels.
    ///
    /// `persona_id`:
    /// - `None` — count across every persona.
    /// - `Some(id)` — restrict to assets owned by that persona
    ///   (matches the sidebar's active persona filter).
    pub async fn list_tag_counts(
        &self,
        persona_id: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::TagCountDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let counts = self.tags.tag_counts(parsed.as_ref()).await?;
        Ok(counts
            .iter()
            .map(crate::application::mapping::tag_count_to_dto)
            .collect())
    }

    /// Sidebar Persona section — one `(persona_id, count)` entry
    /// per persona that owns at least one asset. Ordered by count
    /// descending then persona uuid ascending. The UI resolves
    /// `key` against its own `personaNameById` map.
    ///
    /// `trash` follows the grid so the chips describe the side the user
    /// is looking at (`None` = live, the pre-trash default).
    pub async fn list_persona_asset_counts(
        &self,
        trash: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::AssetCountEntryDto>, DomainError> {
        let side = crate::application::mapping::parse_trash_filter(trash)?;
        let rows = self.assets.counts_by_persona(side).await?;
        Ok(rows
            .into_iter()
            .map(|(pid, c)| asterism_contract::dto::AssetCountEntryDto {
                key: pid.to_string(),
                count: c,
            })
            .collect())
    }

    /// Sidebar Modality section — one `(modality, count)` entry
    /// per modality slug present in the corpus, optionally scoped
    /// to one persona (`None` = cross-persona total). Ordered by
    /// count descending then modality slug ascending.
    /// `trash` follows the grid, as on
    /// [`list_persona_asset_counts`](Self::list_persona_asset_counts).
    pub async fn list_modality_asset_counts(
        &self,
        persona_id: Option<&str>,
        trash: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::AssetCountEntryDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let side = crate::application::mapping::parse_trash_filter(trash)?;
        let rows = self
            .assets
            .counts_by_modality(parsed.as_ref(), side)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(m, c)| asterism_contract::dto::AssetCountEntryDto { key: m, count: c })
            .collect())
    }

    /// Sidebar FORMAT facet counts (asset-model v4) — one entry per
    /// mime top-level type present on top-level assets' primary
    /// materials. Same persona / trash scoping as
    /// [`list_modality_asset_counts`](Self::list_modality_asset_counts).
    pub async fn list_format_asset_counts(
        &self,
        persona_id: Option<&str>,
        trash: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::AssetCountEntryDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let side = crate::application::mapping::parse_trash_filter(trash)?;
        let rows = self.assets.counts_by_format(parsed.as_ref(), side).await?;
        Ok(rows
            .into_iter()
            .map(|(f, c)| asterism_contract::dto::AssetCountEntryDto { key: f, count: c })
            .collect())
    }

    /// Sidebar COLOR facet counts — one entry per swatch carried by at
    /// least one top-level asset's palette, in swatch order (the
    /// repository's ordering; see
    /// [`AssetRepository::counts_by_color`](crate::domain::repository::AssetRepository::counts_by_color)).
    /// Same persona / trash scoping as
    /// [`list_format_asset_counts`](Self::list_format_asset_counts).
    ///
    /// Swatches absent from the corpus are absent from the result
    /// rather than reported as zero: a sidebar full of empty colours
    /// would say the palette exists where it does not.
    pub async fn list_color_asset_counts(
        &self,
        persona_id: Option<&str>,
        trash: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::AssetCountEntryDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let side = crate::application::mapping::parse_trash_filter(trash)?;
        let rows = self.assets.counts_by_color(parsed.as_ref(), side).await?;
        Ok(rows
            .into_iter()
            .map(
                |(bucket, count)| asterism_contract::dto::AssetCountEntryDto {
                    key: bucket.as_str().to_string(),
                    count,
                },
            )
            .collect())
    }

    /// The duplicate report — sets of live assets that share a
    /// fingerprint on one axis, optionally scoped to one persona.
    ///
    /// `axis` is the slug `"artefact"` (every byte of the artefact) or
    /// `"content"` (only the bytes that decide the decoded result);
    /// `None` means the artefact axis, which is what the report has
    /// always answered and so what a caller that names none still gets.
    /// An unknown slug is refused rather than read as either — and
    /// `"file"`, this axis's slug before V64, is refused with the rest —
    /// [`DuplicateAxis::parse`](crate::domain::duplicate_conflict::DuplicateAxis::parse)
    /// records why a closed vocabulary does not get a fallback.
    ///
    /// Three counts ride with the groups, and an empty list cannot be
    /// read without them:
    ///
    /// - `unhashed_count` — open fingerprint work. Converges to zero
    ///   as the walk runs, which it can do now that the unreadable rows
    ///   are counted apart (issue #17).
    /// - `unreadable_count` — originals the fingerprint pass could not
    ///   read: moved, deleted, on a disk that was not plugged in. The
    ///   walk keeps retrying them, but the number does not move on its
    ///   own — the files have to come back.
    /// - `unwalked_count` — the content axis has no reading of these
    ///   rows. The column's migration marks every pre-existing row and
    ///   its next step reads the files, both before the app serves
    ///   anything, so what is left is the originals that could not be
    ///   opened then. Same posture as `unreadable_count`: not a
    ///   progress bar.
    ///
    /// All are reported on every axis: a caller standing on the
    /// artefact axis is entitled to know that switching would ask a
    /// question about a fraction of the library before it switches.
    ///
    /// Detection is what this returns — resolving a group (keeping one,
    /// trashing the rest) goes through the ordinary trash verb, so the
    /// two-step delete safety applies to duplicate cleanup like
    /// everything else.
    pub async fn list_duplicate_groups(
        &self,
        persona_id: Option<&str>,
        axis: Option<&str>,
        limit: Option<u32>,
    ) -> Result<asterism_contract::dto::DuplicateReportDto, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let axis = axis
            .map(crate::domain::duplicate_conflict::DuplicateAxis::parse)
            .transpose()?
            .unwrap_or(crate::domain::duplicate_conflict::DuplicateAxis::Artefact);
        let limit = limit.unwrap_or(DUPLICATE_GROUP_DEFAULT_LIMIT).clamp(1, 500);
        let groups = self
            .assets
            .list_duplicate_groups(parsed.as_ref(), axis, limit)
            .await?;
        let unhashed_count = self.assets.unhashed_material_count().await?;
        let unreadable_count = self.assets.unreadable_material_count().await?;
        let unwalked_count = self.assets.unwalked_material_count().await?;
        Ok(asterism_contract::dto::DuplicateReportDto {
            groups: groups
                .into_iter()
                .map(|g| asterism_contract::dto::DuplicateGroupDto {
                    axis: axis_to_dto(g.axis),
                    content_hash: g.content_hash,
                    members: g
                        .members
                        .iter()
                        .map(crate::application::mapping::card_to_dto)
                        .collect(),
                })
                .collect(),
            unhashed_count,
            unreadable_count,
            unwalked_count,
        })
    }

    /// The duplicate questions still worth asking — what a resolution
    /// panel lists, newest first.
    ///
    /// Distinct from [`list_duplicate_groups`](Self::list_duplicate_groups),
    /// which is a query: the report groups on the digest and would keep
    /// finding a pair that has already been ruled apart, because a
    /// `kept` ruling deliberately leaves both rows in the library. This
    /// returns what somebody still has to decide, and a ruled pair is
    /// gone from it.
    ///
    /// An empty list is not a statement about how far the library has
    /// been fingerprinted — that is `unhashed_count` on the duplicate
    /// report, and this call deliberately does not repeat it. A panel
    /// showing both reads both.
    ///
    /// Both sides are hydrated as cards in one round-trip each. The
    /// owner's view is used: this is the local library's own panel, and
    /// a subject-scoped read that dropped one side of a pair would show
    /// a question with one row in it.
    pub async fn list_duplicate_conflicts(
        &self,
        persona_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<asterism_contract::dto::DuplicateConflictDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let limit = limit.unwrap_or(DUPLICATE_GROUP_DEFAULT_LIMIT).clamp(1, 500);
        let conflicts = self
            .assets
            .list_open_duplicate_conflicts(parsed.as_ref(), limit)
            .await?;
        if conflicts.is_empty() {
            return Ok(Vec::new());
        }

        // One card read for every side of every pair, de-duplicated:
        // the same asset can be one half of several questions once a
        // third copy of the bytes arrives.
        let mut ids: Vec<crate::domain::value::AssetId> = Vec::with_capacity(conflicts.len() * 2);
        for conflict in &conflicts {
            for side in [conflict.newcomer, conflict.incumbent] {
                if !ids.contains(&side) {
                    ids.push(side);
                }
            }
        }
        let cards = self
            .assets
            .cards_by_ids(&ids, &crate::domain::value::Viewer::Owner)
            .await?;
        let by_id: HashMap<_, _> = cards.into_iter().map(|card| (card.id, card)).collect();

        let mut out = Vec::with_capacity(conflicts.len());
        for conflict in conflicts {
            // A pair whose sides the repository joined against `asset`
            // cannot lose one here — but rather than assume that
            // across two statements, a row that came back half
            // hydrated is skipped. Showing a question with one side is
            // worse than showing one question fewer.
            let (Some(newcomer), Some(incumbent)) = (
                by_id.get(&conflict.newcomer),
                by_id.get(&conflict.incumbent),
            ) else {
                continue;
            };
            out.push(asterism_contract::dto::DuplicateConflictDto {
                id: conflict.id.to_string(),
                axis: axis_to_dto(conflict.axis),
                content_hash: conflict.content_hash,
                newcomer: crate::application::mapping::card_to_dto(newcomer),
                incumbent: crate::application::mapping::card_to_dto(incumbent),
                fold_exclusion: conflict
                    .fold_exclusion
                    .map(|reason| reason.as_str().to_string()),
                detected_at_ms: conflict.detected_at.timestamp_millis(),
            });
        }
        Ok(out)
    }

    /// Answers one duplicate question: closes the row and, for
    /// `folded`, enqueues the fold.
    ///
    /// # What each refusal protects
    ///
    /// - **Already answered** — refused, whichever answer is being
    ///   offered. The two rulings are not interchangeable (`folded`
    ///   over a `kept` row would fold two assets somebody ruled apart),
    ///   and an idempotent success on a *differing* answer would report
    ///   that the caller's answer was recorded when the first one
    ///   stands. The refusal names the answer on record so the caller
    ///   can say what happened instead.
    /// - **A side that is no longer live** — refused. This is the same
    ///   rule the listing applies, from the other end: a headstone is
    ///   not a thing to compare, and a trashed row is on its way out.
    ///   Restoring it from the trash makes the question answerable
    ///   again, which is why nothing is stamped on the row here (the
    ///   verb that restores knows nothing about this queue).
    /// - **A keeper from outside the pair** — refused by the domain
    ///   ([`DuplicateConflict::headstone_for`]).
    /// - **A keeper on a `kept` ruling** — refused. "They are two
    ///   things" has no keeper, and ignoring the field would let a
    ///   caller believe it had folded something.
    ///
    /// # Why nothing is written to `fold_policy`
    ///
    /// A `kept` ruling writes the queue row and stops. `fold_policy =
    /// keep` is a statement about a *row* — "this is not a copy of
    /// anything" — and the detector honours it from either side, so
    /// setting it would answer every pair that row will ever take part
    /// in, including ones nobody has seen. What was actually said here
    /// is about one pair, and the closed row already carries it: the
    /// queue's key is `(pair_lo, pair_hi, axis)` with no `resolved_at`
    /// in it, so re-detecting this pair inserts nothing and the listing
    /// skips the answered row. The wider claim stays available as a
    /// separate act for whoever wants to make it.
    ///
    /// # The fold is enqueued, not performed
    ///
    /// `folded` closes the row and puts an `AssetFold` on the queue —
    /// the same job an automatic fold uses, re-reading the pair on its
    /// way in. So the answer is recorded even if the fold is refused
    /// later (the keeper was trashed in the meantime), which is the
    /// right direction: the ruling was made, and a fold that found the
    /// world changed says so in its own log line rather than erasing
    /// the ruling.
    pub async fn resolve_duplicate_conflict(
        &self,
        command: asterism_contract::command::ResolveDuplicateConflictCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::DuplicateResolutionDto, DomainError> {
        use crate::domain::duplicate_conflict::ConflictResolution;
        use crate::domain::value::DuplicateConflictId;

        let id = DuplicateConflictId::from_uuid(crate::application::mapping::parse_uuid(
            &command.conflict_id,
            "conflict_id",
        )?);
        let resolution = match command.resolution {
            asterism_contract::command::ConflictResolution::Folded => ConflictResolution::Folded,
            asterism_contract::command::ConflictResolution::Kept => ConflictResolution::Kept,
        };

        let conflict = self
            .assets
            .find_duplicate_conflict(&id)
            .await?
            .ok_or_else(|| DomainError::not_found("duplicate conflict", &command.conflict_id))?;

        if let Some(answer) = conflict.resolution {
            return Err(DomainError::Conflict(format!(
                "duplicate conflict {} was already resolved as {}",
                command.conflict_id,
                answer.as_str()
            )));
        }

        // Both sides, read as they are now. The listing filtered on
        // this same predicate when the caller saw the row; between
        // then and now either side can have been folded or trashed.
        for (side, which) in [
            (conflict.newcomer, "newcomer"),
            (conflict.incumbent, "incumbent"),
        ] {
            let asset = self
                .assets
                .find(&side)
                .await?
                .ok_or(DomainError::AssetNotFound(side))?;
            if asset.folded_into.is_some() {
                return Err(DomainError::Conflict(format!(
                    "the {which} of this conflict ({side}) has been folded away — \
                     there is nothing left to compare"
                )));
            }
            if asset.trashed_at.is_some() {
                return Err(DomainError::Conflict(format!(
                    "the {which} of this conflict ({side}) is in the trash — \
                     restore it to answer the question"
                )));
            }
        }

        let folded = match resolution {
            ConflictResolution::Folded => {
                let keeper_id = command.keeper_id.as_deref().ok_or_else(|| {
                    DomainError::Validation(
                        "folding needs keeper_id — which of the two rows stays".into(),
                    )
                })?;
                let keeper = parse_asset_id(keeper_id)?;
                let headstone = conflict.headstone_for(&keeper)?;
                Some((keeper, headstone))
            }
            ConflictResolution::Kept => {
                if command.keeper_id.is_some() {
                    return Err(DomainError::Validation(
                        "keeper_id has no meaning when the two are ruled separate things — \
                         both rows stay"
                            .into(),
                    ));
                }
                None
            }
        };

        let now = Utc::now();
        if !self
            .assets
            .close_duplicate_conflict(&id, resolution, now)
            .await?
        {
            // The row was open a moment ago, so this is the other
            // panel winning the race rather than a bad id.
            return Err(DomainError::Conflict(format!(
                "duplicate conflict {} was answered by somebody else while this answer \
                 was being made",
                command.conflict_id
            )));
        }

        // After the close, and only on the branch that folds. Enqueuing
        // first would leave a fold running against a question still
        // open if the close then failed — the shape the automatic path
        // avoids for the same reason.
        if let Some((keeper, headstone)) = folded {
            self.jobs
                .enqueue(
                    crate::domain::job::JobKind::AssetFold,
                    serde_json::json!({
                        "asset_id": headstone.to_string(),
                        "keeper_id": keeper.to_string(),
                    }),
                )
                .await?;
        }

        Ok(asterism_contract::dto::DuplicateResolutionDto {
            conflict_id: command.conflict_id,
            resolution: resolution.as_str().to_string(),
            resolved_at_ms: now.timestamp_millis(),
            keeper_id: folded.map(|(keeper, _)| keeper.to_string()),
            headstone_id: folded.map(|(_, headstone)| headstone.to_string()),
        })
    }

    /// Carries out a person's ruling that a set of rows is one thing —
    /// the manual merge verb.
    ///
    /// The other entry point to a fold is
    /// [`resolve_duplicate_conflict`](Self::resolve_duplicate_conflict):
    /// an answer to a **detected pair** that a fingerprint match raised.
    /// This one reaches the fold from somewhere the queue does not —
    /// a person looking at several rows in a panel and declaring them
    /// one thing — and every part of it reflects that difference. The
    /// set can be any size, the pair need not have been fingerprinted,
    /// and the rules that stop an *automatic* fold (lineage, dispatch
    /// output) are deliberately not binding: a person looking at the
    /// rows can see what those rules were protecting.
    ///
    /// # What this method does
    ///
    /// Four things, in order, and nothing else:
    ///
    /// 1. Parses the ids the caller sent and hands them to
    ///    [`MergePlan::declare`], which decides whether the declaration
    ///    is a declaration at all (the whole `member_ids` check lives
    ///    there — this method does not restate any of it).
    /// 2. Delegates to
    ///    [`AssetRepository::merge_into`] with the plan and the
    ///    caller's `dry_run` flag. That call is where the fold happens
    ///    or, on a preview, the transaction is dropped instead of
    ///    committed. Refusals, re-reads, per-fold structure moves —
    ///    the port doc records the whole shape and this method does
    ///    not go around it.
    /// 3. On a kept transaction only, enqueues one
    ///    [`JobKind::AssetFold`](crate::domain::job::JobKind::AssetFold)
    ///    per row that folded — see below.
    /// 4. On a preview only, collects the *warnings* — the exclusions
    ///    that would have declined an automatic fold of any pair in
    ///    the plan — so the panel can put "these two share a lineage —
    ///    going through with this loses the record that A was derived
    ///    from B" in front of the person about to click confirm.
    ///
    /// # Why a fold job runs after a merge that already folded
    ///
    /// A fold has effects outside the transaction, and
    /// [`resolve_duplicate_conflict`](Self::resolve_duplicate_conflict)
    /// has always had them: the headstone leaves the retrieval index and
    /// the persona's Query Groups are told to re-evaluate. This verb
    /// landed two days after the automatic path grew them and did
    /// neither, so a hand-merged row kept a Tantivy document — never
    /// drawn (the search path intersects its shortlist with the SQL
    /// population, which excludes folded rows) but occupying a place in
    /// a shortlist that is capped, and inflating the "considered N" the
    /// caller is told about. Frozen Query Group memberships kept it too,
    /// until whatever refreshed them next.
    ///
    /// Rather than doing those two things a second time here, the merge
    /// enqueues the **same job** the automatic path enqueues. The job is
    /// idempotent about the fold itself — it finds the row already
    /// standing as a headstone and does the outside-the-transaction half
    /// anyway ([`asset_fold`](../../../asterism_infra/jobs/handlers/fn.asset_fold.html)),
    /// which is what lets one implementation serve an entry point that
    /// folds and an entry point that has already folded.
    ///
    /// **After the transaction, and only on a kept one.** Enqueuing
    /// first would leave a fold running against a merge that then
    /// refused — the shape the automatic path avoids for the same
    /// reason, one line above its own enqueue. And the rows enqueued are
    /// [`MergeOutcome::folded`], not the plan's discards: a refused
    /// merge writes nothing (the all-or-nothing rule), so enqueueing the
    /// plan would fold asynchronously exactly the rows the transaction
    /// declined to fold.
    ///
    /// # Why `merge_into` is called exactly once
    ///
    /// The obvious shape — preview with `dry_run: true`, and if the
    /// caller then confirmed, run again with `dry_run: false` — reads
    /// the pair twice on the commit path (once to preview, once to
    /// commit) for a preview the caller has already seen, which is one
    /// call to the transaction more than the plan needs. The single
    /// call here passes the caller's flag straight through: the preview
    /// is a preview and the commit is a commit, both go through the
    /// same statements, and the port doc's guarantee that they report
    /// the same numbers from the same source is not something this
    /// verb has to arrange a second time. `MergeOutcome::committed`
    /// tells the two branches apart on the answer side (see the port
    /// doc on
    /// [`MergeOutcome::committed`](crate::domain::repository::MergeOutcome::committed)).
    ///
    /// # Why the warnings are gathered here and not in `merge_into`
    ///
    /// [`merge_into`] is the transaction, and lineage / dispatch are
    /// **not conditions for it to run** — they are annotations for a
    /// person about to run it. Pushing them into the port would make
    /// the preview do a graph walk on every fold, on every merge, for
    /// a value the port itself does not consult. The application verb
    /// is the only layer that both knows the plan and answers a
    /// caller who is reading the panel; the walk belongs here.
    ///
    /// # Why the commit branch returns no warnings
    ///
    /// The commit branch's
    /// [`MergeAssetsDto::warnings`](asterism_contract::dto::MergeAssetsDto::warnings)
    /// is **always empty**. Two reasons, both structural: the caller
    /// has already seen the warnings on the preview and is not being
    /// handed a second decision here, and recomputing them on the
    /// commit would be another graph walk for a report nobody is
    /// waiting on. A panel that wants the warnings beside the confirmed
    /// run keeps the ones the preview returned. The dry-run branch
    /// remains the one moment they are computed.
    pub async fn merge_assets(
        &self,
        command: asterism_contract::command::MergeAssetsCommand,
        attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::MergeAssetsDto, DomainError> {
        use crate::domain::merge_plan::MergePlan;

        // Attribution reaches this verb the same way it reaches every
        // other write on this service: the loopback port authenticates
        // nobody, so what the caller stated is what gets recorded. The
        // merge itself does not stamp new rows (a fold moves and
        // annotates existing ones inside one transaction); the value
        // is here because the same shape carries it on every write
        // command, and the day this verb starts writing a note keyed to
        // the caller — a `_trace.merged_by` line, say — this is the
        // field it reads.
        let _ = attribution;

        let keeper = parse_asset_id(&command.keeper_id)?;
        let discard = command
            .discard_ids
            .iter()
            .map(|id| parse_asset_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let members = command
            .member_ids
            .iter()
            .map(|id| parse_asset_id(id))
            .collect::<Result<Vec<_>, _>>()?;

        // Every other check is `MergePlan::declare`'s: the port doc on
        // the command names it as the authoritative account of what
        // the merge refuses and why, and restating any of it here would
        // be a second implementation the day one of the rules moves.
        let plan = MergePlan::declare(keeper, discard, &members)?;

        let outcome = self.assets.merge_into(&plan, command.dry_run).await?;

        // The after-effects of a fold, on the one branch that wrote
        // anything, through the job the automatic path uses. See the doc
        // above for why this is here, why it is after the transaction,
        // and why it reads `folded` rather than the plan.
        if outcome.committed {
            for headstone in &outcome.folded {
                self.jobs
                    .enqueue(
                        crate::domain::job::JobKind::AssetFold,
                        serde_json::json!({
                            "asset_id": headstone.to_string(),
                            "keeper_id": plan.keeper().to_string(),
                        }),
                    )
                    .await?;
            }
            // The by-hand half, for the path a person just confirmed.
            // The job above finishes the durable cleanup, but it is
            // asynchronous, and a headstone answering a search between
            // the commit and the job's turn is a hit the grid cannot
            // open — so the retrieval documents leave now, inline, the
            // same way `trash` takes its rows out.
            self.unindex_removed_assets(&outcome.folded).await;
            // And the keeper absorbed text it did not have — keywords,
            // labels, the discards' comment threads — so its document
            // predates its own row. Recomposition goes through the job
            // for the reason `reindex` gives: the body is re-derived,
            // not patched.
            self.reindex(&plan.keeper()).await;
        }

        // The graph walk is the only thing the preview does beyond the
        // transaction, so the same flag decides both: warnings are for
        // the caller about to click confirm.
        let warnings = if command.dry_run {
            self.collect_warnings(&plan).await?
        } else {
            Vec::new()
        };

        Ok(outcome_to_dto(&plan, outcome, warnings))
    }

    /// For each row in `plan.discard()`, asks whether an *automatic*
    /// fold of the (row, keeper) pair would have been declined —
    /// [`fold_excluded_by`] against the live edges — and returns the
    /// answers as one list, in `discard()` order.
    ///
    /// # Reads for hydration, not for validation
    ///
    /// Both sides are re-read here to hand [`Asset`] references to
    /// [`fold_excluded_by`], which is the port the automatic path
    /// reads for the same question. A row that is gone (either the
    /// keeper or a discard) is **not** a warning — it is a refusal,
    /// and the transaction inside [`merge_into`] catches it under
    /// [`FoldRefusal::Missing`] / [`FoldRefusal::KeeperMissing`]. So a
    /// pair that cannot be hydrated is skipped without producing a
    /// warning: the answer the caller wants for that row is on the
    /// refusals list beside this one, not synthesised here as a second
    /// version of the same refusal.
    ///
    /// # Cheapest first, first answer is the answer
    ///
    /// [`fold_excluded_by`] returns at most one exclusion per pair —
    /// the row records why the fold did not happen, not an inventory
    /// of everything true about the two rows. That is inherited here
    /// unchanged: one entry per discarded row that trips a rule, and
    /// the entries come out in the order the plan folds in, so a
    /// panel drawing the warning list beside the panel that ordered
    /// the discard reads the two side by side.
    ///
    /// [`Asset`]: crate::domain::asset::Asset
    /// [`fold_excluded_by`]: crate::application_support::duplicate_detection::fold_excluded_by
    /// [`merge_into`]: crate::domain::repository::AssetRepository::merge_into
    /// [`FoldRefusal::Missing`]: crate::domain::repository::FoldRefusal::Missing
    /// [`FoldRefusal::KeeperMissing`]: crate::domain::repository::FoldRefusal::KeeperMissing
    async fn collect_warnings(
        &self,
        plan: &crate::domain::merge_plan::MergePlan,
    ) -> Result<Vec<asterism_contract::dto::MergeWarningDto>, DomainError> {
        use crate::application_support::duplicate_detection::fold_excluded_by;

        let Some(keeper_asset) = self.assets.find(&plan.keeper()).await? else {
            // Missing keeper reaches the caller as `KeeperMissing` in
            // the refusals list — one description of the failure, not
            // two.
            return Ok(Vec::new());
        };
        let mut warnings = Vec::new();
        for head_id in plan.discard() {
            let Some(head_asset) = self.assets.find(head_id).await? else {
                continue;
            };
            if let Some(exclusion) =
                fold_excluded_by(&*self.edges, &head_asset, &keeper_asset).await?
            {
                warnings.push(asterism_contract::dto::MergeWarningDto {
                    keeper_id: plan.keeper().to_string(),
                    headstone_id: head_id.to_string(),
                    kind: exclusion.as_str().to_string(),
                });
            }
        }
        Ok(warnings)
    }

    /// Sidebar Groups section — every user-curated Group with its
    /// asset count, ordered by count descending then name ascending.
    /// Unlike tags, empty groups are kept so a freshly created
    /// bucket surfaces immediately.
    pub async fn list_groups(
        &self,
        persona_id: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::GroupSummaryDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let summaries = self.groups.list(parsed.as_ref()).await?;
        Ok(summaries
            .iter()
            .map(crate::application::mapping::group_summary_to_dto)
            .collect())
    }

    /// Creates a Group under the given persona.
    pub async fn create_group(
        &self,
        command: asterism_contract::command::CreateGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::GroupDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        if self.personas.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        let group = self
            .groups
            .create(persona_id, command.name, command.description, Utc::now())
            .await?;
        Ok(crate::application::mapping::group_to_dto(&group))
    }

    /// Moves a Group to the trash. The `asset_bucket` rows stay, so the
    /// membership and its drag-arranged order survive
    /// [`restore_group`](Self::restore_group). Member assets are never
    /// touched — a Group is a filing, not a container.
    ///
    /// The optional remark (#65) is the one exception to "never
    /// touched", and it touches threads, not assets: a comment is
    /// per-asset and a Group has no thread of its own, so the sentence
    /// said over the batch fans out to every member as a
    /// gesture-pinned [`AssetComment`]. Members stay live and
    /// searchable, so each fan-out write queues the same re-index a
    /// thread post does; the rebuild handler skips any member that is
    /// itself trashed or folded.
    pub async fn trash_group(
        &self,
        command: asterism_contract::command::TrashGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = crate::domain::value::GroupId::from_uuid(crate::application::mapping::parse_uuid(
            &command.group_id,
            "group_id",
        )?);
        let now = Utc::now();
        self.groups.trash(&id, now).await?;
        if let Some(body) = Self::gesture_remark(command.comment.as_deref()) {
            // One clock read for the whole fan-out: every member's
            // footnote carries the moment the *group* was thrown,
            // which is the one gesture that occasioned them all. A
            // failure part-way through surfaces to the caller with the
            // group already trashed and the earlier members' footnotes
            // in place — the recoverable side of the gesture-first
            // ordering `post_gesture_comment` documents.
            for asset_id in self.groups.member_asset_ids(&id).await? {
                self.post_gesture_comment(asset_id, body, SelectionGesture::TrashGroup, now)
                    .await?;
                self.reindex_after_thread_write(&asset_id).await;
            }
        }
        Ok(())
    }

    /// Re-composes a live asset's search document after this service
    /// appended a gesture comment to its thread — the fan-out half of
    /// [`trash_group`](Self::trash_group). Same contract as
    /// `AssetCommentService`'s thread-write re-index: an enqueue
    /// failure does not fail the write (the comment is saved, and the
    /// caller asked to trash a filing, not to index prose), but it is
    /// reported, and the composition stamp comes off so the backfill
    /// walk finds the row.
    async fn reindex_after_thread_write(&self, asset_id: &AssetId) {
        let Err(err) = self
            .jobs
            .enqueue(
                JobKind::IndexRebuild,
                serde_json::json!({ "asset_id": asset_id.to_string() }),
            )
            .await
        else {
            return;
        };
        tracing::warn!(
            event = "diag.index.enqueue_failed",
            asset_id = %asset_id,
            error = %err,
            "could not queue a re-index after a gesture comment write"
        );
        if let Err(err) = self.asset_bodies.unstamp(asset_id).await {
            tracing::warn!(
                event = "diag.index.unstamp_failed",
                asset_id = %asset_id,
                error = %err,
                "the asset keeps a document composed from a thread that has changed"
            );
        }
    }

    /// Returns a trashed Group to the sidebar, membership and order
    /// intact.
    pub async fn restore_group(
        &self,
        command: asterism_contract::command::RestoreGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = crate::domain::value::GroupId::from_uuid(crate::application::mapping::parse_uuid(
            &command.group_id,
            "group_id",
        )?);
        self.groups.restore(&id).await
    }

    /// Permanently deletes an **already-trashed** Group; the m:n rows
    /// drop via FK cascade. `Conflict` when the Group is still live.
    pub async fn purge_group(
        &self,
        command: asterism_contract::command::PurgeGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = crate::domain::value::GroupId::from_uuid(crate::application::mapping::parse_uuid(
            &command.group_id,
            "group_id",
        )?);
        self.groups.purge(&id).await
    }

    /// Idempotent add of an asset to a Group.
    pub async fn add_asset_to_group(
        &self,
        command: asterism_contract::command::AddAssetToGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let asset_id = parse_asset_id(&command.asset_id)?;
        let group_id = crate::domain::value::GroupId::from_uuid(
            crate::application::mapping::parse_uuid(&command.group_id, "group_id")?,
        );
        self.ensure_manual_group(&group_id, "add").await?;
        let persona = self.assets.find(&asset_id).await?.map(|a| a.persona_id);
        self.groups.add(&asset_id, &group_id, Utc::now()).await?;
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(())
    }

    /// Idempotent remove of an asset from a Group.
    pub async fn remove_asset_from_group(
        &self,
        command: asterism_contract::command::RemoveAssetFromGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let asset_id = parse_asset_id(&command.asset_id)?;
        let group_id = crate::domain::value::GroupId::from_uuid(
            crate::application::mapping::parse_uuid(&command.group_id, "group_id")?,
        );
        self.ensure_manual_group(&group_id, "remove").await?;
        let persona = self.assets.find(&asset_id).await?.map(|a| a.persona_id);
        self.groups.remove(&asset_id, &group_id).await?;
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(())
    }

    /// Applies a batch of membership changes: attach pairs
    /// first, then detach pairs, grouped per target group so each
    /// group's writes run as one bulk transaction on the writer isle.
    /// Idempotent per pair in both directions (duplicate attach and
    /// missing detach are no-ops). Returns `(attached, detached)`
    /// actually-written row counts — the AI/script cleanup caller
    /// uses them to verify its plan applied.
    pub async fn batch_group_membership(
        &self,
        command: asterism_contract::command::BatchGroupMembershipCommand,
        _attribution: &AttributionContext,
    ) -> Result<(u64, u64), DomainError> {
        use std::collections::{BTreeMap, HashSet};
        fn bucketize(
            entries: &[asterism_contract::command::GroupMembershipEntry],
        ) -> Result<BTreeMap<uuid::Uuid, Vec<crate::domain::value::AssetId>>, DomainError> {
            let mut map: BTreeMap<uuid::Uuid, Vec<crate::domain::value::AssetId>> = BTreeMap::new();
            for e in entries {
                let gid = crate::application::mapping::parse_uuid(&e.group_id, "group_id")?;
                let aid = parse_asset_id(&e.asset_id)?;
                map.entry(gid).or_default().push(aid);
            }
            Ok(map)
        }
        let attach = bucketize(&command.attach)?;
        let detach = bucketize(&command.detach)?;
        let mut attached = 0u64;
        let mut detached = 0u64;
        for (gid, assets) in &attach {
            let group_id = crate::domain::value::GroupId::from_uuid(*gid);
            self.ensure_manual_group(&group_id, "batch attach").await?;
            attached += self.groups.add_bulk(&group_id, assets, Utc::now()).await?;
        }
        for (gid, assets) in &detach {
            let group_id = crate::domain::value::GroupId::from_uuid(*gid);
            self.ensure_manual_group(&group_id, "batch detach").await?;
            detached += self.groups.remove_bulk(&group_id, assets).await?;
        }
        // Refresh hooks: one notify per distinct persona of the touched
        // assets (mirrors the single-pair paths; deduped so the
        // invalidator debounce isn't hammered by large batches).
        let mut asset_seen: HashSet<uuid::Uuid> = HashSet::new();
        let mut personas: HashSet<uuid::Uuid> = HashSet::new();
        for e in command.attach.iter().chain(command.detach.iter()) {
            let aid = parse_asset_id(&e.asset_id)?;
            if !asset_seen.insert(*aid.as_uuid()) {
                continue;
            }
            if let Some(a) = self.assets.find(&aid).await? {
                personas.insert(*a.persona_id.as_uuid());
            }
        }
        for p in personas {
            self.notify_persona_touched(crate::domain::value::PersonaId::from_uuid(p));
        }
        Ok((attached, detached))
    }

    /// Merges one manual Group into another (duplicate-group
    /// consolidation): members the target lacks move over
    /// (appended after its tail in source position order), then the
    /// source group is deleted. Same-persona manual groups only.
    /// Returns the number of members moved.
    pub async fn merge_groups(
        &self,
        command: asterism_contract::command::MergeGroupsCommand,
        _attribution: &AttributionContext,
    ) -> Result<u64, DomainError> {
        let from = crate::domain::value::GroupId::from_uuid(
            crate::application::mapping::parse_uuid(&command.from_group_id, "from_group_id")?,
        );
        let into = crate::domain::value::GroupId::from_uuid(
            crate::application::mapping::parse_uuid(&command.into_group_id, "into_group_id")?,
        );
        if from == into {
            return Err(DomainError::Validation(
                "cannot merge a group into itself".into(),
            ));
        }
        let from_g = self
            .groups
            .find(&from)
            .await?
            .ok_or_else(|| DomainError::not_found("group", from))?;
        let into_g = self
            .groups
            .find(&into)
            .await?
            .ok_or_else(|| DomainError::not_found("group", into))?;
        for g in [&from_g, &into_g] {
            if g.kind == crate::domain::group::GroupKind::Query {
                return Err(DomainError::Validation(
                    "merge is not available on a query group — its membership \
                     is defined by the stored query"
                        .into(),
                ));
            }
        }
        if from_g.persona_id != into_g.persona_id {
            return Err(DomainError::Validation(
                "cannot merge groups across personas".into(),
            ));
        }
        let moved = self.groups.merge(&from, &into, Utc::now()).await?;
        self.notify_persona_touched(into_g.persona_id);
        Ok(moved)
    }

    /// Rewrites the front-to-back order of a Group's members.
    pub async fn reorder_group_assets(
        &self,
        command: asterism_contract::command::ReorderGroupAssetsCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let group_id = crate::domain::value::GroupId::from_uuid(
            crate::application::mapping::parse_uuid(&command.group_id, "group_id")?,
        );
        let asset_ids: Vec<crate::domain::value::AssetId> = command
            .ordered_asset_ids
            .iter()
            .map(|s| parse_asset_id(s))
            .collect::<Result<Vec<_>, _>>()?;
        self.ensure_manual_group(&group_id, "reorder").await?;
        // Reorder only changes position within one group — the row
        // set is unchanged, so no Query Group rule that filters by
        // `group_ids` can shift its members. The `group` sort axis
        // reads `bucket.position` transitively via
        // `primaryGroupName(card).name`, not the group's own
        // position, so a reorder inside one group cannot change any
        // other group's sort output either. Skip the invalidation
        // hook here — the coarse per-persona refresh would be a
        // pure no-op that only burns cycles.
        self.groups.reorder(&group_id, &asset_ids).await
    }

    /// Renames a Group.
    pub async fn rename_group(
        &self,
        command: asterism_contract::command::RenameGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::GroupDto, DomainError> {
        let id = crate::application::mapping::parse_group_id(&command.group_id)?;
        let group = self.groups.rename(&id, command.name, Utc::now()).await?;
        Ok(crate::application::mapping::group_to_dto(&group))
    }

    /// Files a Group under a Dir (`None` = back to the root).
    pub async fn move_group_to_dir(
        &self,
        command: asterism_contract::command::MoveGroupToDirCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let group_id = crate::application::mapping::parse_group_id(&command.group_id)?;
        let dir_id = command
            .dir_id
            .as_deref()
            .map(crate::application::mapping::parse_dir_id)
            .transpose()?;
        self.groups
            .set_dir(&group_id, dir_id.as_ref(), Utc::now())
            .await
    }

    /// Connects a Group into another Group (idempotent; cycle- and
    /// persona-guarded by the repository).
    pub async fn link_group(
        &self,
        command: asterism_contract::command::LinkGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let parent = crate::application::mapping::parse_group_id(&command.parent_group_id)?;
        let child = crate::application::mapping::parse_group_id(&command.child_group_id)?;
        // Curating children into a group is a hand edit — query groups
        // define their inputs via the rule, not bucket_link.
        // A query group *as the child* is fine (a manual parent's
        // nesting closure pulls the query's materialised members in).
        // The composite cycle guard (write-site (b): bucket_link ∪
        // query references) runs inside the repository's link call,
        // atomic with the insert.
        self.ensure_manual_group(&parent, "link").await?;
        let persona = self.groups.find(&parent).await?.map(|g| g.persona_id);
        self.groups.link(&parent, &child, Utc::now()).await?;
        // A new bucket_link changes the nesting closure any Query
        // Group's raw `group_ids` walk expands to, so the
        // whole persona needs a coarse refresh.
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(())
    }

    /// Removes a Group-in-Group connection (no-op if absent).
    pub async fn unlink_group(
        &self,
        command: asterism_contract::command::UnlinkGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let parent = crate::application::mapping::parse_group_id(&command.parent_group_id)?;
        let child = crate::application::mapping::parse_group_id(&command.child_group_id)?;
        let persona = self.groups.find(&parent).await?.map(|g| g.persona_id);
        self.groups.unlink(&parent, &child).await?;
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(())
    }

    /// Every Group-in-Group connection in scope — the client builds
    /// the nesting graph (child bands, descendant expansion for the
    /// filter) from this flat list.
    pub async fn list_group_links(
        &self,
        persona_id: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::GroupLinkDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let links = self.groups.links(parsed.as_ref()).await?;
        Ok(links
            .iter()
            .map(crate::application::mapping::group_link_to_dto)
            .collect())
    }

    /// Rewrites the order of a Group's child groups.
    pub async fn reorder_group_children(
        &self,
        command: asterism_contract::command::ReorderGroupChildrenCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let parent = crate::application::mapping::parse_group_id(&command.parent_group_id)?;
        let children: Vec<crate::domain::value::GroupId> = command
            .ordered_child_ids
            .iter()
            .map(|s| crate::application::mapping::parse_group_id(s))
            .collect::<Result<Vec<_>, _>>()?;
        self.ensure_manual_group(&parent, "reorder children")
            .await?;
        self.groups.reorder_children(&parent, &children).await
    }

    /// Sidebar Dir tree — every dir in scope as a flat `parent_id`
    /// list, ordered by `(position, name)`.
    pub async fn list_dirs(
        &self,
        persona_id: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::DirDto>, DomainError> {
        let parsed = persona_id.map(parse_persona_id).transpose()?;
        let dirs = self.dirs.list(parsed.as_ref()).await?;
        Ok(dirs
            .iter()
            .map(crate::application::mapping::dir_to_dto)
            .collect())
    }

    /// Creates a Dir under the given persona.
    pub async fn create_dir(
        &self,
        command: asterism_contract::command::CreateDirCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::DirDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        if self.personas.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        let parent_id = command
            .parent_id
            .as_deref()
            .map(crate::application::mapping::parse_dir_id)
            .transpose()?;
        let dir = self
            .dirs
            .create(persona_id, parent_id, command.name, Utc::now())
            .await?;
        Ok(crate::application::mapping::dir_to_dto(&dir))
    }

    /// Renames a Dir.
    pub async fn rename_dir(
        &self,
        command: asterism_contract::command::RenameDirCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::DirDto, DomainError> {
        let id = crate::application::mapping::parse_dir_id(&command.dir_id)?;
        let dir = self.dirs.rename(&id, command.name, Utc::now()).await?;
        Ok(crate::application::mapping::dir_to_dto(&dir))
    }

    /// Re-parents a Dir (`None` = to the root).
    pub async fn move_dir(
        &self,
        command: asterism_contract::command::MoveDirCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = crate::application::mapping::parse_dir_id(&command.dir_id)?;
        let new_parent = command
            .new_parent_id
            .as_deref()
            .map(crate::application::mapping::parse_dir_id)
            .transpose()?;
        self.dirs
            .move_to(&id, new_parent.as_ref(), Utc::now())
            .await
    }

    /// Deletes an **empty** Dir.
    pub async fn delete_dir(
        &self,
        command: asterism_contract::command::DeleteDirCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = crate::application::mapping::parse_dir_id(&command.dir_id)?;
        self.dirs.delete(&id).await
    }

    /// Returns the Groups the asset currently belongs to — used by
    /// the detail overlay to render the "already in" state on each
    /// group toggle.
    pub async fn groups_of_asset(
        &self,
        asset_id: &str,
    ) -> Result<Vec<asterism_contract::dto::GroupDto>, DomainError> {
        let id = parse_asset_id(asset_id)?;
        let groups = self.groups.groups_of(&id).await?;
        Ok(groups
            .iter()
            .map(crate::application::mapping::group_to_dto)
            .collect())
    }

    /// Resolves the full source text of each asset (Reader view).
    /// Restricted assets are dropped for viewers outside their
    /// sharing list; unreadable sources come back as `text = None`
    /// so the caller can fall back to the stored cover.
    pub async fn asset_texts(
        &self,
        asset_ids: &[String],
        viewer_subject: Option<&str>,
    ) -> Result<Vec<asterism_contract::dto::AssetTextDto>, DomainError> {
        let viewer = match viewer_subject {
            None => Viewer::Owner,
            Some(subject) => Viewer::Subject(subject.to_string()),
        };
        // Resolve locators (and enforce visibility) first. An asset
        // whose bytes are not text carries `None` rather than a
        // locator: the reader would otherwise be asked for the body of
        // a picture, and the honest answer to "what does this JPEG say"
        // is nothing, not its bytes spelled as characters.
        let mut visible: Vec<(String, Option<TextLocator>)> = Vec::with_capacity(asset_ids.len());
        for raw_id in asset_ids {
            let id = parse_asset_id(raw_id)?;
            let Some(asset) = self.assets.find(&id).await? else {
                continue;
            };
            if !asset.visibility.visible_to(&viewer) {
                continue;
            }
            let mime = asset.materials.first().and_then(|m| m.mime.as_ref());
            visible.push((
                raw_id.clone(),
                TextLocator::new(asset.source.locator.clone(), mime),
            ));
        }
        let locators: Vec<TextLocator> = visible.iter().filter_map(|(_, l)| l.clone()).collect();
        let texts = self.source_texts.read_batch(&locators).await?;
        // Zip back by position: only the rows that produced a locator
        // consumed a slot in `texts`.
        let mut resolved = texts.into_iter();
        Ok(visible
            .into_iter()
            .map(|(asset_id, locator)| asterism_contract::dto::AssetTextDto {
                asset_id,
                text: locator.and_then(|_| resolved.next().flatten()),
            })
            .collect())
    }

    /// The visibility gate every single-asset read shares.
    ///
    /// Resolves `asset_id` and answers [`DomainError::AssetNotFound`]
    /// both when nothing is filed under the id and when the asset is
    /// restricted away from the viewer — deliberately the same error,
    /// so a caller outside the sharing list cannot infer the asset's
    /// existence from the answer. `None` viewer reads as the owner
    /// (caller-asserted; a filtering contract, not an authorization
    /// boundary — see the module doc).
    async fn find_visible(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
    ) -> Result<Asset, DomainError> {
        let id = parse_asset_id(asset_id)?;
        let asset = self
            .assets
            .find(&id)
            .await?
            .ok_or(DomainError::AssetNotFound(id))?;
        let viewer = match viewer_subject {
            None => Viewer::Owner,
            Some(subject) => Viewer::Subject(subject.to_string()),
        };
        if !asset.visibility.visible_to(&viewer) {
            return Err(DomainError::AssetNotFound(id));
        }
        Ok(asset)
    }

    /// [`Self::find_visible`] with no payload.
    ///
    /// For a transport route that serves bytes owned by *another*
    /// service (`GET /asterism/assets/{id}/thumbs/{size_px}` reads the
    /// thumb cache) but must apply the same visibility semantics as
    /// `detail` / `original_file`. Returning `()` keeps the domain
    /// entity from crossing the crate boundary just to be discarded.
    pub async fn assert_visible(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
    ) -> Result<(), DomainError> {
        self.find_visible(asset_id, viewer_subject)
            .await
            .map(|_| ())
    }

    /// Detail view (asset + tags + constellation edges).
    ///
    /// Restricted assets are hidden *entirely* from viewers outside their
    /// sharing list — the method returns `AssetNotFound` so callers cannot
    /// even infer the asset's existence.
    pub async fn detail(&self, query: GetAssetDetailQuery) -> Result<AssetDetailDto, DomainError> {
        let asset = self
            .find_visible(&query.asset_id, query.viewer_subject.as_deref())
            .await?;
        let tags = self.tags.tags_of(&asset.id).await?;
        let edges = self
            .edges
            .edges_of(&asset.id, None, DETAIL_EDGE_LIMIT)
            .await?;
        Ok(detail_to_dto(&asset, &tags, &edges))
    }

    /// Resolves where an asset's original bytes live, for a caller that
    /// intends to read them.
    ///
    /// Returns the resolution, never the bytes: an original can be a
    /// multi-gigabyte video, so whoever serves it streams from the path
    /// rather than holding it. The transport layer owns that half
    /// (`GET /asterism/assets/{id}/file`).
    ///
    /// Same visibility semantics as [`Self::detail`]: a restricted asset
    /// is [`DomainError::AssetNotFound`] for a viewer outside its
    /// sharing list, so a caller cannot infer the asset's existence from
    /// the answer.
    ///
    /// A locator with no bytes of its own — a record inside a container
    /// file (`session.jsonl#<uuid>`), a remote URL, a caller-minted
    /// logical name — is [`DomainError::Conflict`], not a not-found:
    /// the asset exists and is visible, its original simply is not a
    /// file on this disk.
    ///
    /// One question decides it, and it is the same one the hashing job
    /// asks: [`SourceLocator::local_path`]. This method used to ask a
    /// predicate and then re-derive the path itself — a `file://` strip
    /// and an absoluteness test — because the predicate's answer was a
    /// `bool`, and it accepted `file://` spellings that were not paths.
    /// Both passes are gone: the type already took the locator apart,
    /// so there is nothing left to derive.
    pub async fn original_file(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
    ) -> Result<OriginalFileRef, DomainError> {
        let asset = self.find_visible(asset_id, viewer_subject).await?;
        let locator = asset.source.locator.clone();
        let path = locator
            .local_path()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| {
                DomainError::Conflict(format!(
                    "asset original is not a local file (locator kind): {}",
                    locator.to_display()
                ))
            })?;
        // The primary material carries the format fact captured at
        // ingest; `guess_mime` covers an asset whose materials were
        // never filled in (a pre-material row, an importer that skipped
        // the field). Both can answer "unknown" — that stays `None`
        // here, because "we have not read the bytes" is not the same
        // claim as "these bytes are opaque", and the fallback to
        // `application/octet-stream` is an HTTP-level default.
        let mime = asset
            .materials
            .iter()
            .find(|m| m.ord == 0)
            .and_then(|m| m.mime.clone())
            .or_else(|| crate::domain::material::guess_mime(&locator));
        Ok(OriginalFileRef {
            path,
            locator: locator.to_display(),
            mime,
        })
    }

    /// Enqueues an incremental constellation-edge rebuild for the asset.
    pub async fn rebuild_edges(&self, asset_id: &str) -> Result<String, DomainError> {
        let id = parse_asset_id(asset_id)?;
        if self.assets.find(&id).await?.is_none() {
            return Err(DomainError::AssetNotFound(id));
        }
        self.jobs
            .enqueue(
                JobKind::EdgeRebuild,
                serde_json::json!({ "asset_id": id.to_string() }),
            )
            .await
    }

    /// Enqueues the Session reconciliation pass. Called after Import
    /// batches and by explicit user request. Returns the
    /// engine-assigned task id.
    ///
    /// The handler lives in `asterism_infra::jobs::handlers::session_rebuild`.
    /// It no longer rebuilds a precomputed store (the rkyv snapshot was
    /// retired when Session became a 1st-class entity and its
    /// aggregates became query-time derivations); its visible effect is
    /// the `sessions:progress` broadcast the UI refreshes on.
    pub async fn rebuild_sessions(&self) -> Result<String, DomainError> {
        self.jobs
            .enqueue(JobKind::SessionRebuild, serde_json::json!({}))
            .await
    }

    /// Enqueues a dimension re-measure for the named assets — one job
    /// each, and each one **overwrites** whatever is stored.
    ///
    /// The ordinary flow, not a repair: somebody put the right file
    /// behind a card and wants the row to catch up, or an agent knows
    /// these particular rows need redoing. Their asking is newer
    /// information than the stored value, which is why this path does
    /// not consult `dims_probed_at` and does not preserve an existing
    /// pair — both of those exist to keep the *automatic* pass from
    /// stepping on things, and nothing here is automatic.
    ///
    /// One job per asset rather than one job naming many: the queue's
    /// unit of retry is the job, so a batch that failed on its third
    /// artefact would re-read the first two on every attempt.
    ///
    /// Ids are not checked for existence here. The handler answers a
    /// missing row with a message rather than an error — between this
    /// call and the worker, an asset can be purged, and that is not a
    /// fault in the request.
    pub async fn remeasure_dims(&self, asset_ids: &[AssetId]) -> Result<Vec<String>, DomainError> {
        let mut task_ids = Vec::with_capacity(asset_ids.len());
        for asset_id in asset_ids {
            task_ids.push(
                self.jobs
                    .enqueue(
                        JobKind::AssetDims,
                        serde_json::json!({ "asset_id": asset_id.to_string() }),
                    )
                    .await?,
            );
        }
        Ok(task_ids)
    }

    /// Enqueues a pass over the whole table under `scope`.
    ///
    /// The library-scale sibling of [`remeasure_dims`](Self::remeasure_dims).
    ///
    /// **The slug is parsed here, before anything is enqueued.** The job
    /// parses it again when it runs, but that is too late to tell the
    /// caller: a mistyped scope would come back as `enqueued` and then
    /// fail on a worker where nobody is looking. Refusing at the verb is
    /// what makes the answer to "did you take my request" true.
    pub async fn remeasure_dims_batch(&self, scope: &str) -> Result<String, DomainError> {
        let scope = DimsScope::parse(scope)?;
        self.jobs
            .enqueue(
                JobKind::AssetDims,
                serde_json::json!({
                    "batch": true,
                    "scope": scope.as_str(),
                    "cursor": null,
                }),
            )
            .await
    }

    /// Enqueues a re-derivation of duplicate conflicts over fingerprints
    /// that are already written.
    ///
    /// Not a repair verb with a caveat attached — it is safe to call at
    /// any time, because the conflict insert is idempotent over
    /// `UNIQUE (pair_lo, pair_hi, axis)` and the pass never folds
    /// (`DetectionOrigin::Backfill`). The worst it can do is report the
    /// same pairs it reported last time.
    ///
    /// Exists because the inline derivation runs exactly once, when a
    /// digest lands, and the walk that would revisit a row selects on
    /// the fingerprint being absent — so a pair whose moment passed is
    /// otherwise invisible for good.
    pub async fn rescan_duplicates(&self) -> Result<String, DomainError> {
        self.jobs
            .enqueue(
                JobKind::DuplicateScan,
                serde_json::json!({ "batch": true, "cursor": null }),
            )
            .await
    }

    /// Enqueues a full backfill of the Tantivy full-text index —
    /// the handler walks `asset LEFT JOIN asset_body IS NULL` in
    /// pages of ~200 rows and chain-enqueues itself until the
    /// backlog drains. Idempotent (an already-indexed DB terminates
    /// on the first empty page). Returns the engine's first task id.
    pub async fn rebuild_index(&self) -> Result<String, DomainError> {
        self.jobs
            .enqueue(JobKind::IndexRebuild, serde_json::json!({ "batch": true }))
            .await
    }

    /// Returns the top-N edges (by weight) for hover-burst rendering.
    pub async fn edges_of(
        &self,
        asset_id: &str,
        kind: Option<&str>,
        limit: u32,
    ) -> Result<Vec<asterism_contract::dto::EdgeDto>, DomainError> {
        let id = parse_asset_id(asset_id)?;
        let kind = kind.map(EdgeKind::parse).transpose()?;
        let edges = self.edges.edges_of(&id, kind, limit).await?;
        Ok(edges
            .iter()
            .map(crate::application::mapping::edge_to_dto)
            .collect())
    }

    /// Builds the fully-resolved hover-burst payload (each edge together
    /// with its target card).
    ///
    /// **Bidirectional**: consults [`EdgeRepository::edges_incident`]
    /// so the queried asset surfaces every edge it participates in,
    /// not just the outgoing ones.
    ///
    /// The read has to do this because the write does not. `edge` is
    /// `UNIQUE(from_asset, to_asset, kind)` — one directed row for a
    /// relation that is symmetric — so a writer has to pick a side,
    /// and every writer picks the same one: `edge_rebuild` records
    /// from the asset it just ingested, and `identical_to` detection
    /// records from the newcomer towards the existing row. The mirror
    /// row is never written, so an outgoing-only read makes the older
    /// asset in each pair look unconnected.
    ///
    /// Writing both directions instead was considered and rejected.
    /// It would widen `replace_edges_of`'s atomic scope to two assets,
    /// and the job handlers are confined to partial per-column updates
    /// exactly because concurrent apalis workers already lost a write
    /// that way — `auto_tag` overwrote a computed `cover` with NULL
    /// under a worker buffer of 10. Correcting the asymmetry on the
    /// read side leaves that discipline intact.
    ///
    /// Two symmetric edges sharing the same `(other_asset, kind)`
    /// pair collapse into a single `Both` entry (weight = max) via
    /// [`crate::domain::edge::dedupe_incident_pairs`]. The DTO carries
    /// a `direction` slug (`"outgoing"` / `"incoming"` / `"both"`) so
    /// the UI can style the burst edge differently when it wants to.
    ///
    /// If a burst target is hidden from `viewer_subject` (restricted
    /// visibility), the card lookup drops it; the edge simply falls
    /// out of the response.
    pub async fn constellation_of(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
        limit: u32,
    ) -> Result<Vec<asterism_contract::dto::ConstellationItemDto>, DomainError> {
        let id = parse_asset_id(asset_id)?;
        let viewer = match viewer_subject {
            None => Viewer::Owner,
            Some(subject) => Viewer::Subject(subject.to_string()),
        };
        // Fetch both directions, then collapse symmetric pairs before
        // the card round-trip so we do not fetch the same target
        // twice.
        let incidents = self.edges.edges_incident(&id, None, limit).await?;
        let incidents = crate::domain::edge::dedupe_incident_pairs(incidents);
        let targets: Vec<_> = incidents.iter().map(|inc| inc.other_side()).collect();
        let cards = self.assets.cards_by_ids(&targets, &viewer).await?;
        let mut items: Vec<asterism_contract::dto::ConstellationItemDto> = incidents
            .iter()
            .filter_map(|inc| {
                let target_id = inc.other_side();
                cards.iter().find(|card| card.id == target_id).map(|card| {
                    asterism_contract::dto::ConstellationItemDto {
                        edge: crate::application::mapping::edge_to_dto(&inc.edge),
                        card: crate::application::mapping::card_to_dto(card),
                        direction: inc.direction.as_str().to_string(),
                    }
                })
            })
            .collect();

        // Phase-2 synthesis axis: fold "same_session" /
        // "same_selection" / "same_group" siblings into the burst so
        // the "why is this here" answer covers *user curation* (I
        // put these together, they came out of the same
        // conversation, they were dispatched together), not just
        // the automatic time / keyword axes. Kept out of the edge
        // table so group / selection membership churn does not
        // require an edge rebuild — the burst is fresh every hover.
        const SYNTH_PER_KIND: u64 = 3;
        let mut seen: std::collections::HashSet<AssetId> = std::collections::HashSet::new();
        for inc in &incidents {
            seen.insert(inc.other_side());
        }
        let mut synth: Vec<asterism_contract::dto::ConstellationItemDto> = Vec::new();
        let full_asset = self.assets.find(&id).await?;

        // same_session — dialogue / conversation cadence sibling
        // (members of the same composite Asset, keyed on container_id).
        if let Some(asset) = &full_asset
            && let Some(container_id) = &asset.container_id
        {
            let q = crate::domain::asset::AssetQuery {
                viewer: viewer.clone(),
                container_id: Some(*container_id),
                limit: SYNTH_PER_KIND + 1,
                ..Default::default()
            };
            let page = self.assets.list(&q).await?;
            for card in page.items {
                if card.id == id || !seen.insert(card.id) {
                    continue;
                }
                synth.push(synth_item(
                    &id,
                    &card,
                    "same_session",
                    Some("session".into()),
                ));
                if synth
                    .iter()
                    .filter(|i| i.edge.kind == "same_session")
                    .count()
                    >= SYNTH_PER_KIND as usize
                {
                    break;
                }
            }
        }

        // same_selection — user manually gathered these into the
        // same Snapshot at some point (may be many; take the most
        // recent handful). The synth edge kind slug stays
        // `same_selection` for wire stability (the frontend renders it);
        // only the freeze vocabulary underneath changed.
        let snaps = self
            .snapshots
            .list_containing_asset(&id, SYNTH_PER_KIND as u32)
            .await
            .unwrap_or_default();
        let selection_targets: Vec<AssetId> = snaps
            .iter()
            .flat_map(|s| s.asset_ids.iter().copied())
            .collect();
        if !selection_targets.is_empty() {
            // Frozen ids, so a member folded since the freeze is drawn as
            // its keeper rather than as a card the grid refuses to show
            // (`fold_redirect`). The label lookup below then has to ask
            // the freeze about the *headstone* as well: the freeze still
            // holds the id it was minted with.
            let named = crate::application::fold_redirect::hydrate_named(
                self.assets.as_ref(),
                &selection_targets,
                &viewer,
            )
            .await
            .unwrap_or_default();
            for card in &named.cards {
                if card.id == id || !seen.insert(card.id) {
                    continue;
                }
                let label = snaps
                    .iter()
                    .find(|s| {
                        s.asset_ids.iter().any(|stored| {
                            *stored == card.id || named.redirected.get(stored) == Some(&card.id)
                        })
                    })
                    .map(|s| format!("snapshot · {}", &s.id.to_string()[..8]));
                synth.push(synth_item(&id, card, "same_selection", label));
                if synth
                    .iter()
                    .filter(|i| i.edge.kind == "same_selection")
                    .count()
                    >= SYNTH_PER_KIND as usize
                {
                    break;
                }
            }
        }

        // same_group — user filed these into the same curated Group.
        let groups = self.groups.groups_of(&id).await.unwrap_or_default();
        for group in &groups {
            if synth.iter().filter(|i| i.edge.kind == "same_group").count()
                >= SYNTH_PER_KIND as usize
            {
                break;
            }
            let q = crate::domain::asset::AssetQuery {
                viewer: viewer.clone(),
                group_ids: vec![group.id],
                limit: SYNTH_PER_KIND + 1,
                ..Default::default()
            };
            let page = self.assets.list(&q).await?;
            for card in page.items {
                if card.id == id || !seen.insert(card.id) {
                    continue;
                }
                synth.push(synth_item(
                    &id,
                    &card,
                    "same_group",
                    Some(group.name.as_str().to_string()),
                ));
                if synth.iter().filter(|i| i.edge.kind == "same_group").count()
                    >= SYNTH_PER_KIND as usize
                {
                    break;
                }
            }
        }

        items.extend(synth);
        Ok(items)
    }

    /// Fetches the 1-hop `derived_from` lineage around `asset_id`
    /// and returns the split ancestors / descendants view the
    /// detail-pane Provenance section consumes.
    ///
    /// Edge semantics — the write path (`reify_derived`) stamps
    /// `ConstellationEdge { from = derived_asset, to = parent,
    /// kind = DerivedFrom }`, so from the queried asset's
    /// perspective an *outgoing* edge points at a parent
    /// (ancestor) and an *incoming* edge originates from a child
    /// (descendant). `limit` caps each side independently.
    pub async fn provenance_of(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
        limit: u32,
    ) -> Result<asterism_contract::dto::ProvenanceViewDto, DomainError> {
        let id = parse_asset_id(asset_id)?;
        let viewer = match viewer_subject {
            None => Viewer::Owner,
            Some(subject) => Viewer::Subject(subject.to_string()),
        };
        let incidents = self
            .edges
            .edges_incident(&id, Some(EdgeKind::DerivedFrom), limit)
            .await?;
        // Partition by which side of the derived_from edge our
        // asset sits on. `Both` (symmetric duplicate) should not
        // occur for DerivedFrom because the write path only ever
        // stamps a single direction, but treat it as ancestor for
        // safety (matches the "we produced this" reading).
        let mut ancestor_ids: Vec<_> = Vec::new();
        let mut descendant_ids: Vec<_> = Vec::new();
        for inc in &incidents {
            match inc.direction {
                crate::domain::edge::EdgeDirection::Outgoing
                | crate::domain::edge::EdgeDirection::Both => ancestor_ids.push(inc.edge.to),
                crate::domain::edge::EdgeDirection::Incoming => descendant_ids.push(inc.edge.from),
            }
        }
        let ancestor_cards = self.assets.cards_by_ids(&ancestor_ids, &viewer).await?;
        let descendant_cards = self.assets.cards_by_ids(&descendant_ids, &viewer).await?;
        Ok(asterism_contract::dto::ProvenanceViewDto {
            asset_id: id.to_string(),
            ancestors: ancestor_cards
                .iter()
                .map(crate::application::mapping::card_to_dto)
                .collect(),
            descendants: descendant_cards
                .iter()
                .map(crate::application::mapping::card_to_dto)
                .collect(),
        })
    }

    /// Walks the `derived_from` graph around `asset_id` and returns
    /// the whole chain, not just its immediate neighbours.
    ///
    /// This is the read side of correlation ingest: an artefact that
    /// went out to a generator, came back, went out again and came
    /// back once more is four assets and three hops, and the question
    /// worth asking of it is "what route did this take". The
    /// `dispatch_ids` list is that route — one entry per export the
    /// chain passed through, ancestor-ward, in order.
    ///
    /// Bounded on purpose. `depth` is clamped to
    /// [`LINEAGE_MAX_DEPTH`] and the walk stops at
    /// [`LINEAGE_MAX_NODES`]; either way `truncated` says so, because
    /// a lineage picture that quietly omits half the chain reads as
    /// "this is all there is". A `visited` set closes the other hole:
    /// `derived_from` can be declared by hand at ingest, so a cycle
    /// (A → B → A) is reachable, and it is cheaper to stop on the read
    /// side than to walk every ancestor on every ingest.
    pub async fn lineage_of(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
        depth: u32,
    ) -> Result<asterism_contract::dto::LineageViewDto, DomainError> {
        use asterism_contract::dto::{LineageEdgeDto, LineageNodeDto, LineageViewDto};
        use std::collections::{HashMap, HashSet, VecDeque};

        let start = parse_asset_id(asset_id)?;
        if self.assets.find(&start).await?.is_none() {
            return Err(DomainError::AssetNotFound(start));
        }
        let viewer = match viewer_subject {
            None => Viewer::Owner,
            Some(subject) => Viewer::Subject(subject.to_string()),
        };
        let max_depth = depth.clamp(1, LINEAGE_MAX_DEPTH);

        let mut depth_of: HashMap<AssetId, i32> = HashMap::from([(start, 0)]);
        let mut visited: HashSet<AssetId> = HashSet::from([start]);
        let mut edges: Vec<LineageEdgeDto> = Vec::new();
        let mut seen_edges: HashSet<(AssetId, AssetId)> = HashSet::new();
        // Ids that produced no further ancestors — collected as the
        // walk runs so a truncated walk still reports the roots it
        // did reach.
        let mut roots: Vec<AssetId> = Vec::new();
        let mut truncated = false;

        // One queue, two directions: ancestors carry +1 per hop and
        // descendants -1, so a node's sign says which side of the
        // queried asset it sits on and `abs` is its distance.
        let mut queue: VecDeque<(AssetId, i32)> = VecDeque::from([(start, 0)]);
        while let Some((current, current_depth)) = queue.pop_front() {
            if current_depth.unsigned_abs() >= max_depth {
                truncated = true;
                continue;
            }
            let incidents = self
                .edges
                .edges_incident(&current, Some(EdgeKind::DerivedFrom), LINEAGE_EDGE_FANOUT)
                .await?;
            let mut had_ancestor = false;
            for inc in &incidents {
                // Written child → parent, so an outgoing edge points
                // at a parent and an incoming one at a child.
                let (next, next_depth) = match inc.direction {
                    crate::domain::edge::EdgeDirection::Outgoing
                    | crate::domain::edge::EdgeDirection::Both => {
                        had_ancestor = true;
                        (inc.edge.to, current_depth + 1)
                    }
                    crate::domain::edge::EdgeDirection::Incoming => {
                        (inc.edge.from, current_depth - 1)
                    }
                };
                // Only follow a hop that keeps going the way we came:
                // stepping from an ancestor down to its other children
                // would wander into siblings (another export of the
                // same original), which is a different question than
                // "what route did this take". Going the same way means
                // the distance strictly grows — a sign comparison only
                // catches the zero crossing, and let a sibling two
                // hops up slip back in at distance one (dogfood,
                // 2026-08-01).
                if current_depth != 0 && next_depth.unsigned_abs() <= current_depth.unsigned_abs() {
                    continue;
                }
                if seen_edges.insert((inc.edge.from, inc.edge.to)) {
                    edges.push(LineageEdgeDto {
                        from_asset_id: inc.edge.from.to_string(),
                        to_asset_id: inc.edge.to.to_string(),
                        label: inc.edge.label.clone(),
                    });
                }
                if visited.contains(&next) {
                    continue;
                }
                if visited.len() >= LINEAGE_MAX_NODES as usize {
                    truncated = true;
                    break;
                }
                visited.insert(next);
                depth_of.insert(next, next_depth);
                queue.push_back((next, next_depth));
            }
            // A node on the ancestor side (or the start itself) with
            // nothing above it is where this chain began.
            if !had_ancestor && current_depth >= 0 {
                roots.push(current);
            }
        }

        // Cards in one batch — the walk collected ids, not rows. The
        // card projection carries what the grid paints and not the
        // `extra` bag, so the dispatch stamp costs one point lookup
        // per node. Bounded by `LINEAGE_MAX_NODES`, and a real chain
        // is a handful of hops rather than the ceiling.
        let ids: Vec<AssetId> = depth_of.keys().copied().collect();
        let cards = self.assets.cards_by_ids(&ids, &viewer).await?;
        let mut nodes: Vec<LineageNodeDto> = Vec::with_capacity(cards.len());
        // The hop a node *came through* (its resolved claim) stays off
        // the node — `LineageNodeDto.dispatch_id` means "the dispatch
        // that produced this asset", and the claim names a dispatch one
        // step upstream — but it feeds the backbone below.
        let mut claimed_of: HashMap<String, String> = HashMap::new();
        for card in &cards {
            let dispatch_id = match self.assets.find(&card.id).await? {
                Some(asset) => {
                    if let Some(claimed) = claimed_dispatch_of(&asset) {
                        claimed_of.insert(card.id.to_string(), claimed);
                    }
                    dispatch_id_of(&asset)
                }
                None => None,
            };
            nodes.push(LineageNodeDto {
                depth: depth_of.get(&card.id).copied().unwrap_or_default(),
                dispatch_id,
                card: crate::application::mapping::card_to_dto(card),
            });
        }
        // Ancestors first, deepest last: the order the chain happened.
        nodes.sort_by_key(|n| n.depth);

        // The backbone: every export the ancestor side passed through,
        // nearest hop first, each named once. Two sources feed it: the
        // `_dispatch` stamp reify writes on an export copy, and the
        // resolved claim the ingest recorded on the artefact that came
        // back. They usually name the same dispatch — the stamp sits
        // one node above the claim — hence the dedup; what the claim's
        // copy buys is survival, because export copies are disposable
        // and a purged copy takes its stamp with it.
        let mut dispatch_ids: Vec<String> = Vec::new();
        for node in nodes.iter().filter(|n| n.depth >= 0) {
            for candidate in node.dispatch_id.iter().chain(claimed_of.get(&node.card.id)) {
                if !dispatch_ids.iter().any(|d| d == candidate) {
                    dispatch_ids.push(candidate.clone());
                }
            }
        }
        // A card the viewer cannot see drops out of `cards_by_ids`, so
        // the reported roots are the visible ones.
        let visible: HashSet<AssetId> = cards.iter().map(|c| c.id).collect();

        Ok(LineageViewDto {
            asset_id: start.to_string(),
            nodes,
            edges,
            roots: roots
                .into_iter()
                .filter(|id| visible.contains(id))
                .map(|id| id.to_string())
                .collect(),
            dispatch_ids,
            truncated,
        })
    }

    /// Where the preview rendition for a video stands — and the nudge
    /// that makes one exist.
    ///
    /// Formats the webview cannot display (VP9 WebM, Matroska —
    /// measured) play through a transcoded H.264 MP4
    /// rendition cached beside the profile database. The pane calls
    /// this on open and polls while `pending`; the first call for a
    /// missing rendition enqueues the `PreviewGen` job, and a `.part`
    /// staging file suppresses re-enqueueing while a transcode runs.
    ///
    /// Same visibility gate as [`Self::detail`]: even the *status* is an
    /// existence oracle (and `pending` enqueues work on the asker's
    /// behalf), so a viewer outside a restricted asset's sharing list
    /// gets [`DomainError::AssetNotFound`] before any of that.
    pub async fn video_preview(
        &self,
        asset_id: &str,
        viewer_subject: Option<&str>,
    ) -> Result<VideoPreviewDto, DomainError> {
        use crate::domain::render;
        let asset = self.find_visible(asset_id, viewer_subject).await?;
        let id = asset.id;
        let mime = asset
            .materials
            .iter()
            .find(|m| m.ord == 0)
            .and_then(|m| m.mime.as_ref());
        if !render::needs_video_preview(mime) {
            return Ok(VideoPreviewDto {
                status: "not_needed".into(),
                path: None,
                detail: None,
            });
        }
        let id_str = id.to_string();
        let dest = render::video_preview_path(&self.previews_dir, &id_str);
        if dest.is_file() {
            return Ok(VideoPreviewDto {
                status: "ready".into(),
                path: Some(dest.to_string_lossy().into_owned()),
                detail: None,
            });
        }
        let failed = render::video_preview_failed_path(&self.previews_dir, &id_str);
        if let Ok(reason) = std::fs::read_to_string(&failed) {
            return Ok(VideoPreviewDto {
                status: "failed".into(),
                path: None,
                detail: Some(reason.chars().take(500).collect()),
            });
        }
        // No rendition, no failure, and no transcode in flight —
        // enqueue one. The `.part` check keeps a polling pane from
        // stacking duplicate jobs behind a long transcode; a
        // duplicate that slips through anyway finds the cached file
        // and skips.
        if !render::video_preview_part_path(&self.previews_dir, &id_str).exists() {
            let payload = serde_json::json!({ "asset_id": id_str });
            // Interactive: a human is looking at the pane right now.
            let _ = self
                .jobs
                .enqueue_with_priority(JobKind::PreviewGen, payload, 100)
                .await;
        }
        Ok(VideoPreviewDto {
            status: "pending".into(),
            path: None,
            detail: None,
        })
    }

    /// Requests a thumbnail size for an asset by enqueueing a
    /// `ThumbGen` job. Used as the on-demand fallback from
    /// `GET /asterism/assets/{id}/thumbs/{size_px}` when the pair
    /// is not cached — the caller polls the endpoint until the job
    /// materialises the blob (idempotent via
    /// `thumb_cache.INSERT OR REPLACE`).
    pub async fn enqueue_thumb_gen(&self, asset_id: &str, size_px: u32) -> Result<(), DomainError> {
        let _ = parse_asset_id(asset_id)?;
        let payload = serde_json::json!({
            "asset_id": asset_id,
            "size_px": size_px,
        });
        // Highest priority: an interactive open is waiting on this
        // thumb, so it should jump ahead of the background waves.
        self.jobs
            .enqueue_with_priority(JobKind::ThumbGen, payload, 100)
            .await
            .map(|_| ())
    }

    /// Auto-organises existing assets under a Dir tree derived from
    /// each asset's `source_locator` parent path. See
    /// [`OrganizeByLocationCommand`] for the shape.
    ///
    /// Runs entirely against the write isle — a full 100 k backfill
    /// hits O(N) writer round-trips, so the caller should expect a
    /// multi-minute wait on a busy DB. Import-time integration
    /// (auto-organising *new* assets on `add`) is a follow-up
    /// carry.
    pub async fn organize_by_location(
        &self,
        command: OrganizeByLocationCommand,
        _attribution: &AttributionContext,
    ) -> Result<OrganizeByLocationResult, DomainError> {
        let persona_filter = command
            .persona_id
            .as_deref()
            .map(parse_persona_id)
            .transpose()?;

        // Prime the caches from existing structure so a re-run is
        // near-idempotent — Dir siblings unique by
        // `(persona, parent, name)`, Groups by `(persona, name)`.
        let mut dir_cache: HashMap<(PersonaId, Option<DirId>, String), DirId> = HashMap::new();
        for dir in self.dirs.list(persona_filter.as_ref()).await? {
            dir_cache.insert((dir.persona_id, dir.parent_id, dir.name), dir.id);
        }
        let mut group_cache: HashMap<(PersonaId, String), GroupId> = HashMap::new();
        for gs in self.groups.list(persona_filter.as_ref()).await? {
            group_cache.insert((gs.group.persona_id, gs.group.name), gs.group.id);
        }

        // Fetch the asset set to organise in one shot. The current
        // `MAX_LIMIT` cap (200 k) already covers the intended scale;
        // extending to chunked pagination is a follow-up if the
        // Album ever grows past that.
        let query = crate::domain::asset::AssetQuery {
            viewer: Viewer::Owner,
            persona_id: persona_filter,
            modality: None,
            modality_unset: false,
            occurred_from: None,
            occurred_until: None,
            created_from: None,
            created_until: None,
            updated_from: None,
            updated_until: None,
            tag_ids: Vec::new(),
            // Inert: no tag is named, so nothing combines.
            tag_match: asterism_contract::query::TagMatch::Any,
            group_ids: Vec::new(),
            container_id: None,
            label: None,
            text_match: None,
            format: None,
            color: None,
            rating_min: None,
            rating_max: None,
            album_meta_key: None,
            album_meta_value: None,
            // Auto-organize files whatever it is handed; a length, size
            // or resolution band would silently exclude the stills, the
            // containers nothing could probe and everything ingested
            // before the dimension columns existed — exactly the rows
            // most in need of filing.
            duration_min_ms: None,
            duration_max_ms: None,
            size_min_bytes: None,
            size_max_bytes: None,
            pixels_min: None,
            pixels_max: None,
            // Trashed assets are not filed: auto-organize must not
            // resurrect them into Dirs / Groups.
            trash: crate::domain::asset::TrashFilter::LiveOnly,
            offset: 0,
            limit: 200_000,
        };
        let page = self.assets.list(&query).await?;

        let mut dirs_created: u64 = 0;
        let mut groups_created: u64 = 0;
        let mut assets_organized: u64 = 0;
        let mut skipped: u64 = 0;
        // Collect the personas we touched — organize_by_location can
        // span multiple personas when called without a persona filter,
        // and every persona whose asset-to-group membership changed
        // needs a Query Group refresh.
        let mut touched_personas: std::collections::HashSet<PersonaId> =
            std::collections::HashSet::new();

        for card in page.items {
            // Only a file has a directory to be filed under. The other
            // three shapes were already skipped — a logical name has no
            // parent, a remote's parent is not a folder — but by
            // `Path::parent` returning something unusable rather than by
            // the question being declined.
            let SourceLocator::File(path) = &card.source_locator else {
                skipped += 1;
                continue;
            };
            let components = extract_dir_components(path, command.base_dir.as_deref());
            if components.is_empty() {
                skipped += 1;
                continue;
            }

            // Walk the Dir tree, creating missing rungs as we go.
            let persona = card.persona_id;
            let mut parent: Option<DirId> = None;
            let now = Utc::now();
            for comp in &components {
                let key = (persona, parent, comp.clone());
                let dir_id = if let Some(id) = dir_cache.get(&key) {
                    *id
                } else {
                    let dir = self.dirs.create(persona, parent, comp.clone(), now).await?;
                    dirs_created += 1;
                    dir_cache.insert(key, dir.id);
                    dir.id
                };
                parent = Some(dir_id);
            }

            // Leaf Group holds the assets. Group names are unique
            // per persona globally, so use the joined path — the
            // Dir tree still nests them visually in the sidebar via
            // `set_dir`.
            let leaf_dir_id = parent.expect("components non-empty");
            let group_name = components.join("/");
            let group_id = if let Some(id) = group_cache.get(&(persona, group_name.clone())) {
                *id
            } else {
                let group = self
                    .groups
                    .create(persona, group_name.clone(), None, now)
                    .await?;
                // File the group under the leaf Dir so the sidebar
                // shows it nested — the domain call is idempotent.
                self.groups
                    .set_dir(&group.id, Some(&leaf_dir_id), now)
                    .await?;
                groups_created += 1;
                group_cache.insert((persona, group_name), group.id);
                group.id
            };

            self.groups.add(&card.id, &group_id, now).await?;
            assets_organized += 1;
            touched_personas.insert(persona);
        }

        // Debounce collapses the per-persona notifications back down
        // to one refresh per persona touched.
        for persona in touched_personas {
            self.notify_persona_touched(persona);
        }

        Ok(OrganizeByLocationResult {
            dirs_created,
            groups_created,
            assets_organized,
            skipped,
        })
    }

    /// Attaches a tag to an asset by name — creates the tag row on
    /// the first use and then links it. Idempotent: repeat calls with
    /// the same `(asset_id, name)` collapse to a no-op because both
    /// `find_or_create` and `link` are.
    pub async fn attach_tag(
        &self,
        command: asterism_contract::command::AttachTagCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::TagDto, DomainError> {
        let asset_id = parse_asset_id(&command.asset_id)?;
        let trimmed = normalize_tag_name(&command.name)?;
        let tag = self.tags.find_or_create(trimmed).await?;
        let persona = self.assets.find(&asset_id).await?.map(|a| a.persona_id);
        self.tags.link(&asset_id, &tag.id).await?;
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(crate::application::mapping::tag_to_dto(&tag))
    }

    /// The visual model this process has bound, if any (#112) — the
    /// status the UI's model panel reads. All-`None` covers every
    /// no-model shape (feature off, no package, failed bind) because
    /// they are indistinguishable to a caller and act identically.
    pub async fn visual_model_status(&self) -> asterism_contract::dto::VisualModelStatusDto {
        match self.visual_encoder.get() {
            Some(encoder) => {
                let identity = encoder.identity();
                asterism_contract::dto::VisualModelStatusDto {
                    model_id: Some(identity.model_id.clone()),
                    dim: Some(identity.dim),
                    preprocess_ver: Some(identity.preprocess_ver),
                }
            }
            None => asterism_contract::dto::VisualModelStatusDto {
                model_id: None,
                dim: None,
                preprocess_ver: None,
            },
        }
    }

    /// Lists what the bound model proposed for one asset (#112),
    /// score-descending, rulings included. With no model bound there
    /// is nothing to list — an empty answer, not an error, because a
    /// pane that shows suggestions must render the same on a build
    /// without the feature.
    pub async fn tag_suggestions_of(
        &self,
        asset_id: &str,
    ) -> Result<Vec<asterism_contract::dto::TagSuggestionDto>, DomainError> {
        let asset_id = parse_asset_id(asset_id)?;
        let Some(encoder) = self.visual_encoder.get() else {
            return Ok(Vec::new());
        };
        let model_id = &encoder.identity().model_id;
        let evidence = self.tag_evidence.of_asset(&asset_id, model_id).await?;
        // One name lookup for the batch; a suggestion whose tag was
        // deleted has already left through the FK cascade.
        let tags = self.tags.list().await?;
        Ok(evidence
            .into_iter()
            .filter_map(|row| {
                let tag = tags.iter().find(|t| t.id == row.tag_id)?;
                Some(asterism_contract::dto::TagSuggestionDto {
                    tag_id: row.tag_id.to_string(),
                    name: tag.name.clone(),
                    model_id: row.model_id,
                    score: row.score,
                    disposition: row.disposition.as_str().to_string(),
                    suggested_at_ms: row.suggested_at_ms,
                    resolved_at_ms: row.resolved_at_ms,
                })
            })
            .collect())
    }

    /// Accepts one suggestion (#112): the ruling lands on the evidence
    /// row and the tag is linked in `asset_tag` — from this moment it
    /// is a tag the person put there, indistinguishable from any
    /// other, which is the design. Refuses when no suggestion is open
    /// or no model is bound.
    pub async fn accept_tag_suggestion(
        &self,
        asset_id: &str,
        tag_id: &str,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let asset_id = parse_asset_id(asset_id)?;
        let tag_id = crate::application::mapping::parse_tag_id(tag_id)?;
        let Some(encoder) = self.visual_encoder.get() else {
            return Err(DomainError::Validation(
                "no model is bound; there are no suggestions to rule on".into(),
            ));
        };
        let model_id = encoder.identity().model_id.clone();
        self.tag_evidence
            .resolve(
                &asset_id,
                &tag_id,
                &model_id,
                crate::domain::visual::TagSuggestionDisposition::Accepted,
                chrono::Utc::now().timestamp_millis(),
            )
            .await?;
        let persona = self.assets.find(&asset_id).await?.map(|a| a.persona_id);
        self.tags.link(&asset_id, &tag_id).await?;
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(())
    }

    /// Rejects one suggestion (#112): the ruling lands on the evidence
    /// row and this model never proposes the pair again. Nothing else
    /// changes — `asset_tag` was never written.
    pub async fn reject_tag_suggestion(
        &self,
        asset_id: &str,
        tag_id: &str,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let asset_id = parse_asset_id(asset_id)?;
        let tag_id = crate::application::mapping::parse_tag_id(tag_id)?;
        let Some(encoder) = self.visual_encoder.get() else {
            return Err(DomainError::Validation(
                "no model is bound; there are no suggestions to rule on".into(),
            ));
        };
        let model_id = encoder.identity().model_id.clone();
        self.tag_evidence
            .resolve(
                &asset_id,
                &tag_id,
                &model_id,
                crate::domain::visual::TagSuggestionDisposition::Rejected,
                chrono::Utc::now().timestamp_millis(),
            )
            .await
    }

    /// Removes a tag from an asset. Idempotent — missing links are a
    /// no-op. The tag row is left in place; assets on other cards
    /// keep referencing it, and orphan tags drop out of the sidebar
    /// naturally via `tag_counts`.
    pub async fn detach_tag(
        &self,
        command: asterism_contract::command::DetachTagCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let asset_id = parse_asset_id(&command.asset_id)?;
        let tag_id = crate::application::mapping::parse_tag_id(&command.tag_id)?;
        let persona = self.assets.find(&asset_id).await?.map(|a| a.persona_id);
        self.tags.unlink(&asset_id, &tag_id).await?;
        if let Some(p) = persona {
            self.notify_persona_touched(p);
        }
        Ok(())
    }

    /// Attaches a tag to many assets in one call — the bulk form of
    /// [`attach_tag`](Self::attach_tag). Each item is processed
    /// independently: an individual failure is captured in the result
    /// rather than aborting the whole batch (same "loop + tally"
    /// contract as [`update_meta_batch`](Self::update_meta_batch)).
    pub async fn attach_tag_batch(
        &self,
        command: AttachTagBatchCommand,
        attribution: &AttributionContext,
    ) -> Result<AttachTagBatchResult, DomainError> {
        let mut succeeded = Vec::with_capacity(command.items.len());
        let mut failed = Vec::with_capacity(command.items.len());
        let mut success_count = 0u64;
        let mut failure_count = 0u64;

        for item in command.items {
            match self.attach_tag(item, attribution).await {
                Ok(dto) => {
                    succeeded.push(Some(dto));
                    failed.push(String::new());
                    success_count += 1;
                }
                Err(err) => {
                    succeeded.push(None);
                    failed.push(err.to_string());
                    failure_count += 1;
                }
            }
        }

        Ok(AttachTagBatchResult {
            succeeded,
            failed,
            success_count,
            failure_count,
        })
    }

    /// Detaches a tag from many assets in one call — the bulk form of
    /// [`detach_tag`](Self::detach_tag). `detach` carries no payload so
    /// the per-item slot is a success flag.
    pub async fn detach_tag_batch(
        &self,
        command: DetachTagBatchCommand,
        attribution: &AttributionContext,
    ) -> Result<DetachTagBatchResult, DomainError> {
        let mut succeeded = Vec::with_capacity(command.items.len());
        let mut failed = Vec::with_capacity(command.items.len());
        let mut success_count = 0u64;
        let mut failure_count = 0u64;

        for item in command.items {
            match self.detach_tag(item, attribution).await {
                Ok(()) => {
                    succeeded.push(true);
                    failed.push(String::new());
                    success_count += 1;
                }
                Err(err) => {
                    succeeded.push(false);
                    failed.push(err.to_string());
                    failure_count += 1;
                }
            }
        }

        Ok(DetachTagBatchResult {
            succeeded,
            failed,
            success_count,
            failure_count,
        })
    }

    /// Renames a tag in place.
    ///
    /// The name goes through [`normalize_tag_name`] — the same rule
    /// [`attach_tag`](Self::attach_tag) applies — so the two mints
    /// cannot disagree about what a name is.
    ///
    /// A name already held by a *different* tag is a `Conflict`, not
    /// an implicit merge: folding two channels together deletes one
    /// of them, and that has to be asked for
    /// ([`merge_tags`](Self::merge_tags)). Renaming a tag to the name
    /// it already carries succeeds and changes nothing.
    ///
    /// No `updated_at` moves and no invalidation fires: the tag's id
    /// is what membership and Query-Group rules are written against,
    /// and that is exactly what a rename leaves alone.
    pub async fn rename_tag(
        &self,
        command: asterism_contract::command::RenameTagCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::dto::TagDto, DomainError> {
        let tag_id = crate::application::mapping::parse_tag_id(&command.tag_id)?;
        let name = normalize_tag_name(&command.name)?;
        // The port reports the storage fact ("that name is taken");
        // the way out belongs here, where the invoked command is
        // known. A caller that hits this is one route away from what
        // it actually wanted, and the refusal is the only place it
        // will find out.
        let tag = self
            .tags
            .rename(&tag_id, name)
            .await
            .map_err(|err| match err {
                DomainError::Conflict(fact) => DomainError::Conflict(format!(
                    "{fact} — rename does not merge; fold the two channels together \
                     with the tag merge command (POST /asterism/tags/merge)"
                )),
                other => other,
            })?;
        Ok(crate::application::mapping::tag_to_dto(&tag))
    }

    /// Deletes a tag and every link to it, in one transaction.
    ///
    /// The sibling of [`detach_tag`](Self::detach_tag) one level up:
    /// detach unlinks one asset and leaves the channel standing, this
    /// removes the channel. Assets are otherwise untouched — their
    /// `updated_at` does not move, because a tag lives on `asset_tag`
    /// rather than on the asset (see `ListAssetsQuery::updated_from_ms`).
    pub async fn delete_tag(
        &self,
        command: asterism_contract::command::DeleteTagCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::command::DeleteTagResult, DomainError> {
        let tag_id = crate::application::mapping::parse_tag_id(&command.tag_id)?;
        // Collected before the delete: afterwards the links that name
        // the affected personas are gone.
        let personas = self.tags.personas_with_tag(&tag_id).await?;
        let detached_assets = self.tags.delete(&tag_id).await?;
        for persona in personas {
            self.notify_persona_touched(persona);
        }
        Ok(asterism_contract::command::DeleteTagResult {
            deleted: true,
            detached_assets,
        })
    }

    /// Folds one tag into another and deletes the source — the repair
    /// verb for the synonym / spelling-variant sprawl an automatic
    /// tagger produces at scale.
    ///
    /// `dry_run` reports the same numbers without writing anything, so
    /// a caller can size a merge it cannot undo before committing to
    /// it. The target's `axis` survives either way: merge folds the
    /// source *into* the target, so the surviving classification is
    /// the one the caller chose to keep.
    ///
    /// Asset `updated_at` does not move (tags are an `asset_tag`
    /// fact), but Query Groups filtering on either tag do get an
    /// invalidation — their membership genuinely changed.
    pub async fn merge_tags(
        &self,
        command: asterism_contract::command::MergeTagsCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::command::MergeTagsResult, DomainError> {
        let source = crate::application::mapping::parse_tag_id(&command.source_tag_id)?;
        let target = crate::application::mapping::parse_tag_id(&command.target_tag_id)?;
        if source == target {
            return Err(DomainError::Validation(
                "cannot merge a tag into itself".into(),
            ));
        }
        // Both ends: the source's assets change tag, and the target's
        // membership grows. A rule naming either one can flip. A dry
        // run writes nothing, so it skips the collection too — the two
        // link-set walks are exactly the cost a caller probing a
        // large-channel merge is trying to avoid.
        let mut personas: HashSet<uuid::Uuid> = HashSet::new();
        if !command.dry_run {
            for tag in [&source, &target] {
                for persona in self.tags.personas_with_tag(tag).await? {
                    personas.insert(*persona.as_uuid());
                }
            }
        }
        let outcome = self.tags.merge(&source, &target, command.dry_run).await?;
        if !command.dry_run {
            for persona in personas {
                self.notify_persona_touched(crate::domain::value::PersonaId::from_uuid(persona));
            }
        }
        Ok(asterism_contract::command::MergeTagsResult {
            affected_assets: outcome.affected_assets,
            already_tagged: outcome.already_tagged,
            source_removed: outcome.source_removed,
        })
    }

    /// Promotes a Tag into a hand-curated Group under the given
    /// persona: creates the group, then attaches every asset that
    /// currently carries the tag.
    ///
    /// The tag itself is left alone — the promotion is a snapshot,
    /// not a move. Assets tagged after the promotion do not auto-flow
    /// into the group; that would blur the "Tag = organic / Group =
    /// hand-picked" split (see `sets/base/domain` split rationale).
    pub async fn promote_tag_to_group(
        &self,
        command: asterism_contract::command::PromoteTagToGroupCommand,
        _attribution: &AttributionContext,
    ) -> Result<asterism_contract::command::PromoteTagToGroupResult, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        if self.personas.find(&persona_id).await?.is_none() {
            return Err(DomainError::PersonaNotFound(persona_id));
        }
        let tag_id = crate::application::mapping::parse_tag_id(&command.tag_id)?;
        let dir_id = command
            .dir_id
            .as_deref()
            .map(crate::application::mapping::parse_dir_id)
            .transpose()?;

        // Pull every asset carrying this tag in one shot. The persona
        // filter matches the group's owner so we do not accidentally
        // pull cross-persona assets into a persona-scoped bucket. The
        // 200 k `MAX_LIMIT` mirrors `organize_by_location` and covers
        // the intended scale — chunked pagination is a follow-up.
        let query = crate::domain::asset::AssetQuery {
            viewer: Viewer::Owner,
            persona_id: Some(persona_id),
            modality: None,
            modality_unset: false,
            occurred_from: None,
            occurred_until: None,
            created_from: None,
            created_until: None,
            updated_from: None,
            updated_until: None,
            tag_ids: vec![tag_id],
            // One tag: the two combinators select the same set.
            tag_match: asterism_contract::query::TagMatch::Any,
            group_ids: Vec::new(),
            container_id: None,
            label: None,
            text_match: None,
            format: None,
            color: None,
            rating_min: None,
            rating_max: None,
            album_meta_key: None,
            album_meta_value: None,
            // Every asset carrying the tag, on the same reasoning as
            // `organize_by_location`: a metric band here would quietly
            // drop the members that have no length / no recorded size /
            // no measured dimensions.
            duration_min_ms: None,
            duration_max_ms: None,
            size_min_bytes: None,
            size_max_bytes: None,
            pixels_min: None,
            pixels_max: None,
            // Trashed assets are not filed: auto-organize must not
            // resurrect them into Dirs / Groups.
            trash: crate::domain::asset::TrashFilter::LiveOnly,
            offset: 0,
            limit: 200_000,
        };
        let page = self.assets.list(&query).await?;

        let now = Utc::now();
        let group = self
            .groups
            .create(persona_id, command.name.clone(), command.description, now)
            .await?;

        if let Some(ref dir) = dir_id {
            self.groups.set_dir(&group.id, Some(dir), now).await?;
        }

        let mut attached: u64 = 0;
        for card in page.items {
            self.groups.add(&card.id, &group.id, now).await?;
            attached += 1;
        }
        // A new manual group under `persona_id` changes what every
        // Query Group's `group_ids` filter can resolve to.
        self.notify_persona_touched(persona_id);

        Ok(asterism_contract::command::PromoteTagToGroupResult {
            group_id: group.id.to_string(),
            persona_id: group.persona_id.to_string(),
            name: group.name,
            asset_count: attached,
        })
    }
}

/// The one normalisation a tag name goes through, shared by every
/// path that mints or rewrites one
/// ([`AssetService::attach_tag`] / [`AssetService::rename_tag`]).
///
/// Surrounding whitespace is dropped and a name that is nothing but
/// whitespace is rejected — otherwise a stray Enter materialises a
/// useless row, and `" topic"` and `"topic"` become two channels that
/// look identical in the sidebar.
///
/// Case is **not** folded: `tag.name` is `TEXT UNIQUE` with SQLite's
/// default binary collation, so `Topic` and `topic` are two tags on
/// the storage side. Folding here would make the application layer
/// disagree with the constraint it depends on. Collapsing such a pair
/// is what `merge_tags` is for.
fn normalize_tag_name(raw: &str) -> Result<&str, DomainError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation("tag name must be non-empty".into()));
    }
    Ok(trimmed)
}

/// Extracts the Dir-tree components from a local path.
///
/// - With `base_dir` set, the prefix is stripped and only paths
///   that start with it survive; the components are every path
///   segment of the leftover parent path.
/// - Without `base_dir`, the components are just the single
///   immediate parent folder name — a safer default when the
///   caller doesn't know where the tree should start.
///
/// Takes a [`LocalPath`] rather than a locator because the Dir facet is
/// a fact about local paths and about nothing else: a record has its
/// container's parent, not its own; a remote's `//` is not a directory
/// separator; a logical name has no parent at all. The caller matches
/// the variant, so those three are skipped by the same `match` that
/// produces this argument instead of by `Path::parent` happening to
/// answer something.
fn extract_dir_components(locator: &LocalPath, base_dir: Option<&str>) -> Vec<String> {
    let path = locator.as_path();
    let parent = match path.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    match base_dir {
        Some(base) => {
            let base_path = Path::new(base);
            let rel = match parent.strip_prefix(base_path) {
                Ok(r) => r,
                Err(_) => return Vec::new(),
            };
            rel.components()
                .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
                .filter(|s| !s.is_empty())
                .collect()
        }
        None => parent
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
    }
}

/// Outcome of classifying an [`AddAssetCommand`]'s Session-binding
/// axes (P3). One-of-three verdict that
/// [`AssetService::add`] uses to decide whether to resolve the
/// key through the Session service, take the caller-supplied
/// Session id verbatim, or skip the field entirely.
/// Parses a composite Asset id (hyphenated UUID text) into an
/// [`AssetId`](crate::domain::value::AssetId) for the `container_id`
/// membership write. session-model v2: the ingest resolve produces a
/// composite id (either supplied directly or minted by
/// `find_or_create_by_external_key`) that must round-trip as the BLOB
/// `asset.id`.
fn parse_container_id(raw: &str) -> Result<crate::domain::value::AssetId, DomainError> {
    uuid::Uuid::parse_str(raw)
        .map(crate::domain::value::AssetId::from_uuid)
        .map_err(|e| DomainError::Validation(format!("invalid composite id {raw:?}: {e}")))
}

/// Outcome of a `derived_from` claim on an [`AddAssetCommand`].
///
/// Three states rather than `Option<Vec<AssetId>>` because "nobody
/// claimed anything" and "someone claimed something that did not
/// resolve" call for different behaviour: the first writes nothing at
/// all, the second writes a note saying what was claimed and why it
/// did not stick.
#[derive(Debug, PartialEq, Eq)]
enum ResolvedOrigin {
    /// The caller made no claim.
    None,
    /// The claim resolved to one or more parents.
    Resolved {
        parents: Vec<AssetId>,
        /// Which form was used (`asset` / `dispatch` /
        /// `sidecar-dispatch` / `sidecar-asset`) — recorded so a later
        /// reader can tell a directly-named parent from a resolved one.
        form: &'static str,
        /// The claim verbatim, as the caller wrote it. The parents
        /// above are what it resolved *to*; without the claim itself
        /// the resolution is one-way — once the parents are purged
        /// (export copies are disposable), nothing on this asset says
        /// which export it came through.
        claim: String,
        /// The dispatch the resolution went through, when it went
        /// through one (`dispatch` and `sidecar-dispatch` forms). Kept
        /// so the lineage backbone can name the hop even after the
        /// dispatch's reified outputs have left the ledger.
        dispatch: Option<String>,
    },
    /// The claim was kept but could not be turned into a parent.
    Unresolved { claim: String, reason: String },
}

impl ResolvedOrigin {
    fn unresolved(claim: &str, reason: String) -> Self {
        Self::Unresolved {
            claim: claim.trim().to_string(),
            reason,
        }
    }

    /// The `extra._trace` payload for this outcome, if it warrants one.
    ///
    /// An unresolved claim is written *because* it failed: a link that
    /// is simply missing looks identical to one that was never asked
    /// for, and the difference is exactly what someone debugging a
    /// chain needs.
    ///
    /// `source` is the channel the claim arrived through
    /// ([`provenance::source`] vocabulary — `embedded` / `pushed` /
    /// `manual`), derived by the call site from where it sits, never
    /// asserted by the caller of the API. `None` — a legacy note being
    /// re-resolved that never carried one — stays absent rather than
    /// being guessed: the channel is unknowable after the fact.
    ///
    /// `operator` is the agent that performed *this* operation
    /// ([`OperatorRef`](crate::domain::attribution::OperatorRef)
    /// vocabulary) — caller-asserted, unlike `source`, and a different
    /// question: `source` says which channel the claim came in on,
    /// `operator` says through what it was driven. Absent when nobody
    /// asserted one, on the same terms as everywhere else in
    /// [`attribution`](crate::domain::attribution): an unrecorded
    /// operator is not the person at the keyboard.
    ///
    /// `relation` is written **always**, and that is the difference
    /// from `source`. A missing channel is unknowable after the fact,
    /// so it stays missing rather than being guessed; a missing
    /// relation is not ambiguous at all — every note written before the
    /// field existed came from a verb that could only mean
    /// `derived_from`. So absent reads as
    /// [`ClaimRelation::DerivedFrom`](crate::domain::provenance::ClaimRelation::DerivedFrom)
    /// and new notes say so out loud.
    fn trace_note(
        &self,
        source: Option<&str>,
        operator: Option<&str>,
        relation: crate::domain::provenance::ClaimRelation,
    ) -> Option<serde_json::Value> {
        let mut note = match self {
            Self::None => return None,
            Self::Resolved {
                parents,
                form,
                claim,
                dispatch,
            } => {
                let mut note = serde_json::json!({
                    "derived_from": parents.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                    "form": form,
                    "resolved": true,
                    "claim": claim,
                });
                if let Some(dispatch) = dispatch {
                    note["dispatch_id"] = serde_json::json!(dispatch);
                }
                note
            }
            Self::Unresolved { claim, reason } => serde_json::json!({
                "derived_from": claim,
                "resolved": false,
                "reason": reason,
            }),
        };
        note["relation"] = serde_json::json!(relation.as_str());
        if let Some(source) = source {
            note["source"] = serde_json::json!(source);
        }
        if let Some(operator) = operator {
            note["operator"] = serde_json::json!(operator);
        }
        Some(note)
    }
}

/// Reads the dispatch that produced an asset out of its `extra` bag.
///
/// `reify` stamps `extra._dispatch.dispatch_id` on everything an
/// exporter hands back — that stamp is the hop's identity, and the
/// sequence of them along a chain is the route the artefact took.
///
/// **Only that stamp counts.** An earlier version fell back to
/// `bundle_id` on the grounds that `reify` sets it to the same value,
/// which is true but not exclusive: `bundle_id` is a
/// grouping slot any producer may fill, and the image importer fills
/// it with a synthetic key so a PNG clusters with the notes extracted
/// from its tEXt chunks. Reading that as a dispatch put a hop in the
/// chain that never happened — a ledger is better empty than wrong
/// (found by dogfooding, 2026-07-29).
fn dispatch_id_of(asset: &Asset) -> Option<String> {
    asset
        .extra
        .get("_dispatch")
        .and_then(|d| d.get("dispatch_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Reads the dispatch a resolved `derived_from` claim went through,
/// out of the `_trace` note the ingest wrote.
///
/// The lineage backbone's second source. The first — the `_dispatch`
/// stamp read by [`dispatch_id_of`] — lives on the export copies, and
/// export copies are disposable: purge them and a chain that relied
/// only on their stamps forgets which export it travelled through.
/// The claim note lives on the artefact that came back, which is the
/// asset the user actually keeps. Only a *resolved* claim counts — an
/// unresolved one names a dispatch nobody has confirmed, and the
/// ledger is better empty than wrong.
fn claimed_dispatch_of(asset: &Asset) -> Option<String> {
    let trace = asset.extra.get(provenance::TRACE_KEY)?;
    if trace.get("resolved").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    trace
        .get("dispatch_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Writes `value` under `key` in an asset's `extra` bag, preserving
/// whatever the importer already put there.
///
/// `extra` is free-form and usually an object, but an importer is
/// allowed to store an array or a scalar. Rather than overwrite that
/// (the ingest side does not own it), a non-object payload is moved
/// under `_extra` so both survive — the same shape the dispatch runner
/// uses when it stamps `_dispatch`.
/// Writes one field *inside* the `_trace` note, keeping the fields
/// already there.
///
/// The sibling below replaces a whole top-level key, which is what the
/// provenance note wants (a claim and its resolution are one statement,
/// rewritten together). A declared digest is a second, independent
/// statement about the same registration, so it has to land beside
/// `derived_from` / `source` / `operator` rather than instead of them.
///
/// A `_trace` that is not an object is started fresh. The bag under
/// this key is the server's own bookkeeping — an importer that put a
/// string there through `extra_json` has written in a reserved place,
/// and the provenance note already treats that value the same way.
/// Records a provenance claim into `_trace` without disturbing the rest
/// of the bag.
///
/// The claim's own fields ([`provenance::CLAIM_FIELDS`]) are cleared
/// first and then written from `note`, so a re-declaration cannot leave
/// a previous claim's `dispatch_id` or `reason` standing. Everything
/// else in `_trace` — the declared hash, a fold record, what a merge
/// absorbed — is left alone.
///
/// This used to be a whole-object replace, and the ingest path worked
/// around it by writing the claim *before* the declared hash. The
/// after-the-fact verbs have no such ordering to exploit, so declaring
/// provenance on a row that carried a declared hash silently took the
/// hash with it [measured 2026-08-06: `_trace` came back holding only the
/// claim's own six fields].
fn merge_claim_note(extra: &mut serde_json::Value, note: serde_json::Value) {
    let mut trace = extra
        .get(provenance::TRACE_KEY)
        .filter(|existing| existing.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(map) = trace.as_object_mut() {
        for field in provenance::CLAIM_FIELDS {
            map.remove(*field);
        }
        if let Some(fresh) = note.as_object() {
            for (key, value) in fresh {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    merge_extra_key(extra, provenance::TRACE_KEY, trace);
}

/// Checks every AlbumMeta statement carried by an ingest, and hands
/// back the normalised pairs in the order they will be written.
///
/// The collision check is the part the wire shape cannot do for the
/// caller. `" workflow"` and `"workflow"` are two entries in a JSON
/// object and one key once [`album_meta::parse_key`] has trimmed them,
/// so accepting both would let the second eat the first while the
/// caller's own records still say two things were stated. That is the
/// silent merge `parse_key` refuses uppercase to avoid, one layer up —
/// and the only place it can be caught, since by the time the entries
/// reach `_trace.meta` the loser is already gone.
///
/// [`album_meta::parse_key`]: crate::domain::album_meta::parse_key
fn parse_album_meta(
    raw: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, DomainError> {
    use crate::domain::album_meta;

    let mut parsed: Vec<(String, String)> = Vec::with_capacity(raw.len());
    for (key, value) in raw {
        let key = album_meta::parse_key(key)?;
        let value = album_meta::parse_value(value)?;
        if parsed.iter().any(|(seen, _)| seen == &key) {
            return Err(DomainError::Validation(format!(
                "album meta key {key:?} was given twice in one ingest — two \
                 spellings of one name, and taking the later one would drop a \
                 statement the caller still believes it made"
            )));
        }
        parsed.push((key, value));
    }
    Ok(parsed)
}

/// Writes or removes one entry in `_trace.meta`, leaving the rest of
/// the bag — including every other statement under `meta` — untouched.
///
/// `None` removes. When the removal empties `meta`, the object goes
/// with it rather than being left behind as `{}`: an empty container is
/// a reader's second thing to check for the same "nobody has said
/// anything" it already learns from the key being absent.
fn merge_meta_entry(extra: &mut serde_json::Value, key: &str, entry: Option<serde_json::Value>) {
    use crate::domain::album_meta::META_KEY;

    let mut trace = extra
        .get(provenance::TRACE_KEY)
        .filter(|existing| existing.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let Some(trace_map) = trace.as_object_mut() else {
        return;
    };
    let meta = trace_map
        .entry(META_KEY.to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !meta.is_object() {
        *meta = serde_json::json!({});
    }
    if let Some(meta_map) = meta.as_object_mut() {
        match entry {
            Some(note) => {
                meta_map.insert(key.to_string(), note);
            }
            None => {
                meta_map.remove(key);
            }
        }
    }
    if meta.as_object().is_some_and(serde_json::Map::is_empty) {
        trace_map.remove(META_KEY);
    }
    merge_extra_key(extra, provenance::TRACE_KEY, trace);
}

/// The statement [`AssetService::declare_source_type`] files under
/// `_trace.source_type`.
///
/// The same shape as an AlbumMeta entry — value, channel, operator,
/// moment — because it is the same kind of thing: a statement somebody
/// made, with enough beside it to answer "who said this, and when". The
/// value is the term's URI, the spelling every emitter writes.
fn album_meta_entry_for_source_type(
    ty: crate::domain::disclosure::DigitalSourceType,
    operator: Option<&str>,
    at_ms: i64,
) -> serde_json::Value {
    crate::domain::album_meta::entry(ty.uri(), provenance::source::MANUAL, operator, at_ms)
}

/// Writes — or removes — the source-type assertion under `_trace`.
///
/// The single-field sibling of [`merge_meta_entry`], with the same
/// care: everything else in `_trace` is left exactly as it was.
fn merge_source_type_assertion(extra: &mut serde_json::Value, entry: Option<serde_json::Value>) {
    use crate::domain::disclosure::SOURCE_TYPE_KEY;

    let mut trace = extra
        .get(provenance::TRACE_KEY)
        .filter(|existing| existing.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(map) = trace.as_object_mut() {
        match entry {
            Some(note) => {
                map.insert(SOURCE_TYPE_KEY.to_string(), note);
            }
            None => {
                map.remove(SOURCE_TYPE_KEY);
            }
        }
    }
    merge_extra_key(extra, provenance::TRACE_KEY, trace);
}

fn merge_trace_field(extra: &mut serde_json::Value, field: &str, value: serde_json::Value) {
    let mut trace = extra
        .get(provenance::TRACE_KEY)
        .filter(|existing| existing.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    trace[field] = value;
    merge_extra_key(extra, provenance::TRACE_KEY, trace);
}

fn merge_extra_key(extra: &mut serde_json::Value, key: &str, value: serde_json::Value) {
    match extra {
        serde_json::Value::Object(map) => {
            map.insert(key.to_string(), value);
        }
        serde_json::Value::Null => {
            *extra = serde_json::json!({ key: value });
        }
        other => {
            let carried = other.take();
            *other = serde_json::json!({ "_extra": carried, key: value });
        }
    }
}

/// Field-by-field port of a
/// [`MergeOutcome`](crate::domain::repository::MergeOutcome) into the
/// wire [`MergeAssetsDto`](asterism_contract::dto::MergeAssetsDto).
///
/// Kept outside the impl block for the reason
/// [`classify_session_binding`] is: this is a pure translation between
/// two shapes and takes nothing from the service's own state, and
/// isolating it here keeps it inspectable without spinning up the
/// service. `keeper_id` is echoed from the plan the caller sent
/// rather than derived from the outcome — the outcome carries the
/// discards and their answers, not the keeper, since the fold does
/// not touch it.
///
/// The two id-carrying lists ([`folded`] and [`already_folded`]) are
/// mapped one-to-one, and the refusals are dropped through
/// [`FoldRefusal::as_str`](crate::domain::repository::FoldRefusal::as_str)
/// — the same slug the port doc names as the vocabulary the wire
/// carries.
///
/// [`folded`]: crate::domain::repository::MergeOutcome::folded
/// [`already_folded`]: crate::domain::repository::MergeOutcome::already_folded
fn outcome_to_dto(
    plan: &crate::domain::merge_plan::MergePlan,
    outcome: crate::domain::repository::MergeOutcome,
    warnings: Vec<asterism_contract::dto::MergeWarningDto>,
) -> asterism_contract::dto::MergeAssetsDto {
    let totals = &outcome.totals;
    asterism_contract::dto::MergeAssetsDto {
        keeper_id: plan.keeper().to_string(),
        folded_ids: outcome.folded.iter().map(AssetId::to_string).collect(),
        already_folded_ids: outcome
            .already_folded
            .iter()
            .map(AssetId::to_string)
            .collect(),
        refusals: outcome
            .refusals
            .into_iter()
            .map(|(id, reason)| asterism_contract::dto::MergeRefusalDto {
                asset_id: id.to_string(),
                reason: reason.as_str().to_string(),
            })
            .collect(),
        warnings,
        totals: asterism_contract::dto::MergeTotalsDto {
            edges_repointed: totals.edges_repointed,
            edges_dropped: totals.edges_dropped,
            buckets_moved: totals.buckets_moved,
            children_repointed: totals.children_repointed,
            tags_moved: totals.tags_moved,
            comments_moved: totals.comments_moved,
            threads_reanchored: totals.threads_reanchored,
            columns_merged: totals.columns_merged,
            values_discarded: totals.values_discarded,
        },
        committed: outcome.committed,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SessionBinding {
    /// Neither `session_id` nor `external_session_key` was
    /// supplied — the asset lands without a Session reference.
    None,
    /// Caller passed `session_id` directly. The application layer
    /// treats it as an opaque `Session.id` and does not resolve.
    DirectId(String),
    /// Caller passed `external_session_key`. The application layer
    /// resolves it via
    /// [`crate::application::SessionService::find_or_create_by_external_key`].
    ExternalKey(String),
}

/// Classifies the Session-binding inputs of `AddAssetCommand`.
///
/// The only guard left is mutual exclusion: `session_id` and
/// `external_session_key` supplied together is a matrix-ambiguity
/// error. The former Dialog-only gate is gone (asset-model v4):
/// membership in a Session container is modality-agnostic — its DB
/// backing, the V27 CHECK, was already dropped in V31, and
/// `ContentKind::Composition` always defined membership as
/// modality-agnostic.
///
/// The function is deliberately kept pure so the guard behaviour can
/// be unit-tested without spinning up the full repository graph.
fn classify_session_binding(
    session_id: Option<&str>,
    external_session_key: Option<&str>,
) -> Result<SessionBinding, DomainError> {
    match (session_id, external_session_key) {
        (Some(_), Some(_)) => Err(DomainError::Validation(
            "session_id and external_session_key are mutually exclusive on \
             AddAssetCommand — pass exactly one"
                .into(),
        )),
        (None, None) => Ok(SessionBinding::None),
        (Some(id), None) => Ok(SessionBinding::DirectId(id.to_string())),
        (None, Some(key)) => Ok(SessionBinding::ExternalKey(key.to_string())),
    }
}

/// Refuses a resolution stated as one number — `width_px` without
/// `height_px`, or the reverse.
///
/// The columns are two independent nullable integers, and the invariant
/// "either both are measured or neither is" has no DB constraint behind
/// it: SQLite cannot add a `CHECK` that reads another column through
/// `ALTER TABLE ADD COLUMN`, and rebuilding the whole `asset` table for
/// two columns does not pay. So the pair is asserted on the way in, at
/// the two places a write can start one:
///
/// - the importer road cannot express a half — [`Footprint`]'s `dims` is
///   a single `Option<(u32, u32)>`, so `*_to_spec` writes both or
///   neither;
/// - **this** road can, because both wire fields carry `#[serde(default)]`
///   and a sender may simply omit one. `{"width_px": 1920}` arrives as
///   `(Some, None)`, and that is what this refuses.
///
/// Same polarity and same wording shape as the `author_kind` /
/// `author_subject` pair, which is a co-requirement rather than the
/// mutual exclusion `classify_session_binding` above enforces.
///
/// Pure, for the reason its neighbour is: the rule is worth a test that
/// does not need a repository graph.
fn refuse_half_written_dims(
    width_px: Option<u32>,
    height_px: Option<u32>,
) -> Result<(), DomainError> {
    let missing = match (width_px, height_px) {
        (Some(_), None) => "height_px",
        (None, Some(_)) => "width_px",
        _ => return Ok(()),
    };
    Err(DomainError::Validation(format!(
        "width_px and height_px are one measurement — {missing} is missing, \
         and half a resolution is not a smaller answer than none"
    )))
}

/// Builds a `ConstellationItemDto` for a synthesised burst edge —
/// `same_session` / `same_selection` / `same_group`. The edge id is
/// namespaced with a `synth:<kind>:` prefix so downstream callers
/// can tell it apart from persisted rows (frontend does not care;
/// the kind slug already differentiates it visually).
fn synth_item(
    from: &AssetId,
    card: &crate::domain::asset::AssetCard,
    kind: &str,
    label: Option<String>,
) -> asterism_contract::dto::ConstellationItemDto {
    asterism_contract::dto::ConstellationItemDto {
        edge: asterism_contract::dto::EdgeDto {
            id: format!("synth:{kind}:{}", card.id),
            from_asset_id: from.to_string(),
            to_asset_id: card.id.to_string(),
            kind: kind.to_string(),
            label,
            weight: None,
        },
        card: crate::application::mapping::card_to_dto(card),
        // Synth edges are symmetric by construction ("X shares
        // session with Y" is inherent to both sides); the UI
        // treats "both" as the confirmed-link direction hint.
        direction: "both".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The assertion writes into `_trace` beside what is already there,
    /// and its removal takes only its own key — the care every `_trace`
    /// writer here owes the others.
    #[test]
    fn a_source_type_assertion_leaves_the_rest_of_the_trace_alone() {
        use crate::domain::disclosure;

        let mut extra = serde_json::json!({
            "_trace": { "source": "manual" },
            "unrelated": true,
        });
        let entry = album_meta_entry_for_source_type(
            disclosure::DigitalSourceType::DigitalCapture,
            Some("asterism-ui"),
            1_000,
        );
        merge_source_type_assertion(&mut extra, Some(entry));
        assert_eq!(
            disclosure::asserted_source_type(&extra),
            Some(disclosure::DigitalSourceType::DigitalCapture),
            "the entry must read back through the disclosure module's own reader"
        );
        assert_eq!(extra["_trace"]["source"], "manual");
        assert_eq!(extra["unrelated"], true);

        merge_source_type_assertion(&mut extra, None);
        assert_eq!(disclosure::asserted_source_type(&extra), None);
        assert_eq!(
            extra["_trace"]["source"], "manual",
            "retracting the assertion is not license to touch the claim"
        );
    }

    #[test]
    fn classify_none_when_both_axes_absent() {
        let out = classify_session_binding(None, None).unwrap();
        assert_eq!(out, SessionBinding::None);
    }

    #[test]
    fn classify_direct_id_passes_through_verbatim() {
        let out = classify_session_binding(Some("session-uuid-123"), None).unwrap();
        assert_eq!(out, SessionBinding::DirectId("session-uuid-123".into()));
    }

    #[test]
    fn classify_external_key_marks_for_find_or_create() {
        let out = classify_session_binding(None, Some("cc.session.42")).unwrap();
        assert_eq!(out, SessionBinding::ExternalKey("cc.session.42".into()));
    }

    /// Membership is modality-agnostic (asset-model v4): the former
    /// Dialog-only rejection is gone, so a tape / journal / image
    /// asset binding to a Session container is a valid input now.
    #[test]
    fn classify_accepts_any_modality_binding() {
        let out = classify_session_binding(None, Some("some.stem")).unwrap();
        assert_eq!(out, SessionBinding::ExternalKey("some.stem".into()));
    }

    /// A measurement is two numbers. One of them is refused, from either
    /// side, and the message names the one that is missing.
    #[test]
    fn a_resolution_stated_as_one_number_is_refused_from_either_side() {
        let width_only = refuse_half_written_dims(Some(1920), None)
            .expect_err("half a resolution is not a measurement");
        assert!(
            matches!(&width_only, DomainError::Validation(msg) if msg.contains("height_px")),
            "the message names the half that is missing: {width_only}"
        );
        let height_only = refuse_half_written_dims(None, Some(1080))
            .expect_err("and the other way round is the same mistake");
        assert!(
            matches!(&height_only, DomainError::Validation(msg) if msg.contains("width_px")),
            "{height_only}"
        );
    }

    /// Both halves and neither half are the two answers that pass — and
    /// `0` is a measurement like any other, not a stand-in for absence.
    #[test]
    fn both_halves_and_neither_half_are_both_accepted() {
        refuse_half_written_dims(Some(1920), Some(1080)).unwrap();
        refuse_half_written_dims(None, None).unwrap();
        refuse_half_written_dims(Some(0), Some(0))
            .expect("a zero pair is a stated pair, whatever it means");
    }

    #[test]
    fn classify_rejects_both_axes_supplied_together() {
        let err = classify_session_binding(Some("sid"), Some("cc.42")).unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(msg) if msg.contains("mutually exclusive")),
            "matrix ambiguity between session_id and external_session_key \
             must fail with Validation before touching any repository"
        );
    }

    fn meta_map(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn album_meta_comes_back_normalised() {
        let parsed = parse_album_meta(&meta_map(&[(" workflow-id ", " wf-1 ")])).unwrap();
        assert_eq!(parsed, vec![("workflow-id".into(), "wf-1".into())]);
    }

    #[test]
    fn album_meta_refuses_two_spellings_of_one_name() {
        // Two entries on the wire, one key after trimming. Taking the
        // later one would discard a statement the caller still believes
        // it made, and nothing downstream could tell that happened.
        let err =
            parse_album_meta(&meta_map(&[("workflow", "a"), (" workflow", "b")])).unwrap_err();
        assert!(
            matches!(&err, DomainError::Validation(msg) if msg.contains("twice in one ingest")),
            "{err}"
        );
    }

    #[test]
    fn album_meta_refuses_the_whole_set_over_one_bad_entry() {
        // Partial acceptance would land the asset without the statement
        // that made it findable, and report success either way.
        assert!(parse_album_meta(&meta_map(&[("ok", "v"), ("a.b", "v")])).is_err());
        assert!(parse_album_meta(&meta_map(&[("ok", "v"), ("blank", "  ")])).is_err());
    }
}
