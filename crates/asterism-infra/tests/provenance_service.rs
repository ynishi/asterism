//! `ProvenanceService` over real storage: what an asset discloses, and
//! whether the answer survives being written into a file.
//!
//! The service lives in `asterism-core` and its tests live here for the
//! reason the sibling suites give: what it does depends on rows in other
//! aggregates — which material carries the container metadata, which
//! side of a `derived_from` edge the parent sits on, what a purged
//! parent leaves behind — and a hand-written `AssetRepository` stub
//! would encode its author's answer to exactly those questions while
//! they are the questions under test.
//!
//! The pure half (container metadata → IPTC term) is tested beside
//! itself in `asterism_core::domain::disclosure`. What is here is the
//! wiring: the reads, their order, and the round trip through a file.

use std::sync::Arc;

use asterism_core::application::provenance_service::ProvenanceService;
use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::edge::{ConstellationEdge, EdgeKind};
use asterism_core::domain::material::Material;
use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::{
    AssetRepository, EdgeRepository, MaterialFingerprint, PersonaRepository,
};
use asterism_core::domain::value::{AssetId, Modality, PersonaId, SourceKind, SourceRef};
use asterism_infra::provenance::ProvenanceWriter;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::{
    SqliteAssetRepository, SqliteEdgeRepository, SqlitePersonaRepository,
};
use asterism_provenance::{DigitalSourceType, Half, Skipped, embed};
use chrono::Utc;
use rusqlite_isle::AsyncIsleDriver;

/// A caller that states nothing, which records nothing.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

struct Fixture {
    service: ProvenanceService,
    assets: Arc<SqliteAssetRepository>,
    personas: Arc<SqlitePersonaRepository>,
    edges: Arc<SqliteEdgeRepository>,
    driver: AsyncIsleDriver,
}

impl Fixture {
    async fn open() -> Self {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let assets = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let personas = Arc::new(SqlitePersonaRepository::new(isle.clone()));
        let edges = Arc::new(SqliteEdgeRepository::new(isle.clone()));
        // Unsigned: these fixtures are about which record comes out of
        // the library, and a signature would only add a certificate to
        // the setup without changing any answer here. The signing path
        // has its own tests in `src/provenance.rs`.
        let writer = Arc::new(ProvenanceWriter::unsigned());
        let service = ProvenanceService::new(assets.clone(), edges.clone(), writer);
        Self {
            service,
            assets,
            personas,
            edges,
            driver,
        }
    }

    async fn seed_persona(&self) -> PersonaId {
        let persona = Persona::new("P", None).unwrap();
        self.personas.save(&persona).await.unwrap();
        persona.id
    }

    /// One image asset, optionally carrying a material whose canonical
    /// container metadata is `meta_kv`.
    async fn seed_asset(
        &self,
        persona: PersonaId,
        title: Option<&str>,
        meta_kv: Option<&str>,
    ) -> AssetId {
        let locator = format!("m-{}.png", uuid::Uuid::now_v7());
        let source = SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap();
        let mut asset = Asset::new(
            persona,
            source.clone(),
            Some(Modality::new("image").unwrap()),
            Utc::now(),
            &unattributed(),
        );
        asset.title = title.map(str::to_string);
        if meta_kv.is_some() {
            asset
                .attach_material(Material::primary(
                    asset.source.locator.clone(),
                    None,
                    Utc::now(),
                ))
                .expect("an item may carry a material");
        }
        self.assets.save(&asset).await.unwrap();
        // `meta_kv` is not written by `save`: it lands in the same
        // statement as the digest it belongs to, because the two are one
        // measurement and a row holding a digest whose object had not
        // arrived yet would show a reader a body that says something
        // other than what the index was built from. So the fixture takes
        // the same route the `material_hash` job does rather than
        // reaching around it.
        if let Some(meta_kv) = meta_kv {
            self.assets
                .set_material_fingerprint(
                    &asset.id,
                    0,
                    &MaterialFingerprint {
                        file: "unhashable:no-bytes".into(),
                        content: "unhashable:no-bytes".into(),
                        meta: "m1-sha256:0".into(),
                        meta_kv: Some(meta_kv.to_string()),
                        meta_raw: None,
                    },
                )
                .await
                .unwrap();
        }
        // Read the precondition back off the row rather than off the
        // entity this fixture happens to hold: a `save` that dropped
        // `meta_kv` would otherwise take every assertion below with it
        // and still look green.
        let stored = self.assets.find(&asset.id).await.unwrap().unwrap();
        assert_eq!(
            stored.materials.first().and_then(|m| m.meta_kv.as_deref()),
            meta_kv,
            "the metadata the record is built from has to survive the write"
        );
        asset.id
    }

    /// `child` was derived from `parent`, in the direction the write
    /// path uses (`from` = the newer asset).
    async fn seed_derived_from(&self, child: AssetId, parent: AssetId) {
        self.edges
            .add_edges(vec![
                ConstellationEdge::new(child, parent, EdgeKind::DerivedFrom).unwrap(),
            ])
            .await
            .unwrap();
    }

    async fn close(self) {
        self.driver.shutdown().await.unwrap();
    }
}

/// A ComfyUI export's stored metadata.
fn comfy() -> String {
    serde_json::json!({ "Software": "ComfyUI", "workflow": "{}" }).to_string()
}

/// A camera file's stored metadata — `Make` at the address the JPEG
/// probe writes it under.
fn camera() -> String {
    serde_json::json!({ "exif:0x010f": "FUJIFILM" }).to_string()
}

/// The 1×1 PNG these fixtures stamp.
fn png() -> Vec<u8> {
    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(kind);
        hasher.update(payload);
        out.extend_from_slice(&hasher.finalize().to_be_bytes());
        out
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
    png.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x02]));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}

/// The record is assembled out of the row, and the container metadata it
/// is assembled from is the primary material's.
#[tokio::test]
async fn the_record_comes_out_of_what_the_library_stored() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some("a shot"), Some(&comfy())).await;

    let record = fx
        .service
        .record_for(&asset, Some("dispatch-1"))
        .await
        .unwrap();

    assert_eq!(record.asset_id, asset.to_string());
    assert_eq!(
        record.source_type,
        Some(DigitalSourceType::TrainedAlgorithmicMedia)
    );
    assert_eq!(record.ai_system.as_deref(), Some("ComfyUI"));
    assert_eq!(record.title.as_deref(), Some("a shot"));
    assert_eq!(
        record.dispatch_id.as_deref(),
        Some("dispatch-1"),
        "the dispatch is the caller's context, not the library's"
    );
    fx.close().await;
}

/// An asset nothing was measured on discloses nothing, and says so by
/// omission rather than by a term meaning "unknown".
#[tokio::test]
async fn an_asset_with_no_container_metadata_asserts_no_term() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, None, None).await;

    let record = fx.service.record_for(&asset, None).await.unwrap();
    assert_eq!(record.source_type, None);
    assert!(!record.discloses_anything());
    assert_eq!(
        record.asset_id,
        asset.to_string(),
        "the identifier is still worth carrying"
    );
    fx.close().await;
}

/// The parent is on the far side of the edge, and whether it is itself
/// synthetic decides which of the two generative terms is true.
#[tokio::test]
async fn a_generated_child_of_a_photograph_reads_as_a_composite() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let photograph = fx.seed_asset(persona, None, Some(&camera())).await;
    let child = fx.seed_asset(persona, None, Some(&comfy())).await;
    fx.seed_derived_from(child, photograph).await;

    let record = fx.service.record_for(&child, None).await.unwrap();
    assert_eq!(
        record.source_type,
        Some(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia),
        "a model altered material that did not come from one"
    );
    assert_eq!(record.parents, vec![photograph.to_string()]);

    // The direction matters: read from the parent's side, the same edge
    // must not make the photograph a child of anything.
    let parent_record = fx.service.record_for(&photograph, None).await.unwrap();
    assert!(
        parent_record.parents.is_empty(),
        "`from` is the newer asset, so the parent has no parents through this edge"
    );
    assert_eq!(
        parent_record.source_type,
        Some(DigitalSourceType::DigitalCapture)
    );
    fx.close().await;
}

/// Two generated assets in a chain stay `trainedAlgorithmicMedia`: no
/// material from outside a model is in either.
#[tokio::test]
async fn a_generated_child_of_a_generated_parent_is_not_a_composite() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let parent = fx.seed_asset(persona, None, Some(&comfy())).await;
    let child = fx.seed_asset(persona, None, Some(&comfy())).await;
    fx.seed_derived_from(child, parent).await;

    let record = fx.service.record_for(&child, None).await.unwrap();
    assert_eq!(
        record.source_type,
        Some(DigitalSourceType::TrainedAlgorithmicMedia)
    );
    assert_eq!(record.parents, vec![parent.to_string()]);
    fx.close().await;
}

/// The service treats a parent it cannot read as not synthetic, and
/// storage is why that branch is not reachable from here.
#[tokio::test]
async fn an_edge_cannot_point_at_an_asset_that_is_not_there() {
    // Worth pinning rather than assuming. The service reads each parent
    // to establish whether it is itself synthetic, and a parent it
    // cannot find falls back to "not synthetic" — the weaker of the two
    // claims, which is the direction a guess is allowed to fail in. That
    // fallback exists for a row that vanishes between the two reads;
    // it is not a state an edge can be *written* into, because the
    // foreign key refuses one. If this ever stops failing, the fallback
    // has become reachable through ordinary writes and wants a test of
    // its own rather than a comment.
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let child = fx.seed_asset(persona, None, Some(&comfy())).await;

    let dangling = fx
        .edges
        .add_edges(vec![
            ConstellationEdge::new(child, AssetId::new(), EdgeKind::DerivedFrom).unwrap(),
        ])
        .await;
    let err = dangling.expect_err("an edge to nothing is refused by storage");
    assert!(err.to_string().contains("FOREIGN KEY"), "{err}");

    // And the record is what it was before the refused write.
    let record = fx.service.record_for(&child, None).await.unwrap();
    assert!(record.parents.is_empty());
    assert_eq!(
        record.source_type,
        Some(DigitalSourceType::TrainedAlgorithmicMedia)
    );
    fx.close().await;
}

/// An asset that is not there is a `404`, not an empty record.
#[tokio::test]
async fn an_unknown_asset_is_refused_rather_than_disclosed_as_nothing() {
    let fx = Fixture::open().await;
    let err = fx
        .service
        .record_for(&AssetId::new(), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("asset not found"), "{err}");
    fx.close().await;
}

/// The whole point of building the record from rows: a file with no
/// metadata of its own gets the disclosure anyway.
#[tokio::test]
async fn a_file_that_carries_nothing_is_stamped_from_the_database() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, None, Some(&comfy())).await;

    // Deliberately not the asset's own file, and carrying no metadata —
    // the shape of something that came back from a downstream
    // conversion with its manifest and its packet stripped.
    let dir = tempfile::tempdir().unwrap();
    let returned = dir.path().join("stripped.png");
    std::fs::write(&returned, png()).unwrap();
    assert_eq!(
        embed::read_xmp(&std::fs::read(&returned).unwrap()).unwrap(),
        None,
        "the file starts with nothing to read"
    );

    let stamped = fx.service.apply_to(&asset, &returned, None).await.unwrap();
    assert_eq!(stamped.xmp, Half::Written);
    assert!(stamped.discloses());
    assert_eq!(
        stamped.manifest,
        Half::Skipped(Skipped::NoSigningIdentity),
        "no identity is configured, and the test certificates are refused"
    );

    let packet = embed::read_xmp(&std::fs::read(&returned).unwrap())
        .unwrap()
        .expect("the disclosure was re-applied from the row");
    assert!(packet.contains("trainedAlgorithmicMedia"));
    assert!(packet.contains("ComfyUI"));
    fx.close().await;
}

/// Applying twice leaves one packet, not two — the property that makes
/// re-applying safe to do more than once.
#[tokio::test]
async fn re_applying_is_idempotent() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, None, Some(&comfy())).await;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.png");
    std::fs::write(&path, png()).unwrap();

    fx.service.apply_to(&asset, &path, None).await.unwrap();
    let once = std::fs::read(&path).unwrap();
    fx.service.apply_to(&asset, &path, None).await.unwrap();
    let twice = std::fs::read(&path).unwrap();

    assert_eq!(
        once, twice,
        "the same record applied to the same file is the same bytes"
    );
    let marker = b"W5M0MpCehiHzreSzNTczkc9d";
    assert_eq!(
        twice.windows(marker.len()).filter(|w| *w == marker).count(),
        1,
        "one packet, not a second one appended beside the first"
    );
    fx.close().await;
}

/// The port is what the core sees; this asserts the adapter satisfies it
/// rather than only the inherent method of the same name.
#[tokio::test]
async fn the_adapter_is_reachable_through_the_port() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.png");
    std::fs::write(&path, png()).unwrap();

    let writer: Arc<dyn asterism_core::application::provenance_service::ProvenanceWriter> =
        Arc::new(ProvenanceWriter::unsigned());
    let record = asterism_provenance::DisclosureRecord::for_asset("asset-1")
        .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia);
    let stamped = writer.apply(&path, &record).await.unwrap();
    assert_eq!(stamped.xmp, Half::Written);
}
