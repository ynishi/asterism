//! The perceptual walk over real storage: which rows it offers, and
//! what makes one leave.
//!
//! How a decode becomes a status is a unit test beside the handler,
//! over values. What that cannot tell anybody is whether an answered
//! row stops coming back. The walk's predicate is the status column
//! alone — no mime filter, because what can be read is the reader's
//! question — so every material is offered, including the ones that
//! will never hold a fingerprint, and each has to leave on its first
//! pass. A walk that re-offered an answered row would spin on the same
//! page forever while reporting progress, which is the failure this
//! binary exists to catch.

use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::material::Material;
use asterism_core::domain::measurement::{Measurement, MeasurementStatus};
use asterism_core::domain::repository::AssetRepository;
use asterism_core::domain::value::{MimeType, Modality, PersonaId, SourceKind, SourceRef};
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::SqliteAssetRepository;
use chrono::Utc;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

/// These fixtures are about which rows the walk hands back, not about
/// who registered them.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
    let pid = Uuid::now_v7();
    let pack = format!("pack-{pid}");
    isle.call(move |conn| {
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'P', 0, 0)",
            rusqlite::params![pid, pack],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    PersonaId::from_uuid(pid)
}

/// One asset whose primary material declares `mime`, saved the way
/// ingest saves it: nothing has looked at the bytes yet.
async fn row_declaring(
    repo: &SqliteAssetRepository,
    persona: PersonaId,
    locator: &str,
    mime: Option<&str>,
) -> Asset {
    let now = Utc::now();
    let mut asset = Asset::new(
        persona,
        SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
        Some(Modality::new("tape").unwrap()),
        now,
        &unattributed(),
    );
    let mut material = Material::primary(asset.source.locator.clone(), Some(1), now);
    material.mime = mime.map(MimeType::parse);
    asset.materials = vec![material];
    repo.save(&asset).await.unwrap();
    asset
}

/// Every material arrives pending and is offered once — the picture,
/// the recording, and the row whose format nobody named.
///
/// The last of those is the one worth seeding. It is also the row the
/// migration's `mime IS NULL` arm exists for, and a predicate that
/// filtered on format in SQL would drop it here instead of answering
/// it.
#[tokio::test]
async fn the_walk_offers_every_pending_material_once() {
    let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
    let assets = SqliteAssetRepository::new(isle.clone());
    let persona = seed_persona(&isle).await;

    let picture = row_declaring(&assets, persona, "/pics/a.png", Some("image/png")).await;
    let recording = row_declaring(&assets, persona, "/clips/b.mp4", Some("video/mp4")).await;
    let unnamed = row_declaring(&assets, persona, "/blobs/c", None).await;

    let page = assets
        .scan_materials_without_perceptual_hash(None, 16)
        .await
        .unwrap();
    assert_eq!(
        page.len(),
        3,
        "the walk filters on the status column alone, so every material is offered"
    );

    // Each row is answered the way its pass would answer it: one
    // fingerprint, and two rows that will never hold one.
    let value = "p1-dhash:0123456789abcdef0123456789abcdef";
    assets
        .set_material_perceptual_hash(&picture.id, 0, &Measurement::computed(value.into()))
        .await
        .unwrap();
    assets
        .set_material_perceptual_hash(
            &recording.id,
            0,
            &Measurement::unsupported("video/mp4".into()),
        )
        .await
        .unwrap();
    assets
        .set_material_perceptual_hash(&unnamed.id, 0, &Measurement::unsupported("unknown".into()))
        .await
        .unwrap();

    let page = assets
        .scan_materials_without_perceptual_hash(None, 16)
        .await
        .unwrap();
    assert!(
        page.is_empty(),
        "an answered row leaves the walk, whatever the answer was: {page:?}"
    );
}

/// What was written is what reads back, on the entity the rest of the
/// system holds — including the answers that carry no value.
#[tokio::test]
async fn the_answer_reads_back_off_the_material() {
    let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
    let assets = SqliteAssetRepository::new(isle.clone());
    let persona = seed_persona(&isle).await;

    let picture = row_declaring(&assets, persona, "/pics/a.png", Some("image/png")).await;
    let recording = row_declaring(&assets, persona, "/clips/b.mp4", Some("video/mp4")).await;

    let fresh = assets.find(&picture.id).await.unwrap().unwrap();
    assert_eq!(
        fresh.materials[0].perceptual_hash_status,
        MeasurementStatus::Pending,
        "insert leaves the column pending, the way it leaves the digests"
    );
    assert_eq!(fresh.materials[0].perceptual_hash, None);

    let value = "p1-dhash:0123456789abcdef0123456789abcdef";
    assets
        .set_material_perceptual_hash(&picture.id, 0, &Measurement::computed(value.into()))
        .await
        .unwrap();
    assets
        .set_material_perceptual_hash(
            &recording.id,
            0,
            &Measurement::unsupported("video/mp4".into()),
        )
        .await
        .unwrap();

    let fresh = assets.find(&picture.id).await.unwrap().unwrap();
    assert_eq!(
        fresh.materials[0].perceptual_hash,
        Some(value.to_string()),
        "the value is stored verbatim, tag and all"
    );
    assert_eq!(
        fresh.materials[0].perceptual_hash_status,
        MeasurementStatus::Computed
    );
    assert_eq!(fresh.materials[0].perceptual_hash_reason, None);

    let fresh = assets.find(&recording.id).await.unwrap().unwrap();
    assert_eq!(fresh.materials[0].perceptual_hash, None);
    assert_eq!(
        fresh.materials[0].perceptual_hash_status,
        MeasurementStatus::Unsupported
    );
    assert_eq!(
        fresh.materials[0].perceptual_hash_reason,
        Some("video/mp4".to_string()),
        "the format that answered the row is what the reason carries"
    );
}

/// What the rebuild's input leaves out, and why each exclusion is
/// there.
///
/// An edge is drawn between things a person can open. A trashed asset
/// is not one, a folded asset has already been answered by the fold,
/// and another persona's library is a different library. A row with no
/// stored value has nothing to compare. The `ord = 0` filter is the
/// boundary duplicate detection draws for the same reason: an edge is
/// a claim about two assets, not about two files inside them.
#[tokio::test]
async fn the_print_scan_reads_only_what_an_edge_may_point_at() {
    let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
    let assets = SqliteAssetRepository::new(isle.clone());
    let persona = seed_persona(&isle).await;
    let stranger = seed_persona(&isle).await;

    let value = "p1-dhash:0123456789abcdef0123456789abcdef";
    let printed = |repo: &SqliteAssetRepository, id| {
        let repo = repo.clone();
        async move {
            repo.set_material_perceptual_hash(&id, 0, &Measurement::computed(value.into()))
                .await
                .unwrap();
        }
    };

    let kept = row_declaring(&assets, persona, "/pics/kept.png", Some("image/png")).await;
    let unmeasured = row_declaring(&assets, persona, "/pics/blank.png", Some("image/png")).await;
    let trashed = row_declaring(&assets, persona, "/pics/gone.png", Some("image/png")).await;
    let folded = row_declaring(&assets, persona, "/pics/folded.png", Some("image/png")).await;
    let elsewhere = row_declaring(&assets, stranger, "/pics/other.png", Some("image/png")).await;

    printed(&assets, kept.id).await;
    printed(&assets, trashed.id).await;
    printed(&assets, folded.id).await;
    printed(&assets, elsewhere.id).await;
    // `unmeasured` deliberately gets none.

    assets.trash(&trashed.id, Utc::now()).await.unwrap();
    let (folded_id, keeper) = (*folded.id.as_uuid(), *kept.id.as_uuid());
    isle.call(move |conn| {
        conn.execute(
            "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
            rusqlite::params![folded_id, keeper],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let prints = assets.scan_perceptual_prints(&persona).await.unwrap();
    assert_eq!(
        prints.iter().map(|p| p.asset_id).collect::<Vec<_>>(),
        vec![kept.id],
        "the trashed, the folded, the unmeasured and another persona's are all out"
    );
    assert_eq!(prints[0].value, value, "the value arrives as it is stored");

    // The unmeasured row is in the library and simply has no answer
    // yet — it is still the walk's to offer.
    let pending = assets
        .scan_materials_without_perceptual_hash(None, 16)
        .await
        .unwrap();
    assert_eq!(
        pending.iter().map(|m| m.asset_id).collect::<Vec<_>>(),
        vec![unmeasured.id]
    );
}

/// The three exact axes are untouched by a perceptual write. They are
/// what duplicate detection reads, and a fingerprint that reached them
/// would put an approximate claim in front of a fold.
#[tokio::test]
async fn a_perceptual_write_leaves_the_exact_axes_alone() {
    let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
    let assets = SqliteAssetRepository::new(isle.clone());
    let persona = seed_persona(&isle).await;

    let picture = row_declaring(&assets, persona, "/pics/a.png", Some("image/png")).await;
    let digest = format!("sha256:{}", "a".repeat(64));
    assets
        .set_material_fingerprint(
            &picture.id,
            0,
            &asterism_core::domain::repository::MaterialFingerprint {
                file: Measurement::computed(digest.clone()),
                content: Measurement::bare(MeasurementStatus::EmptySpan),
                meta: Measurement::bare(MeasurementStatus::EmptySpan),
                meta_kv: None,
                meta_raw: None,
                meta_text: None,
            },
        )
        .await
        .unwrap();

    assets
        .set_material_perceptual_hash(
            &picture.id,
            0,
            &Measurement::computed("p1-dhash:0123456789abcdef0123456789abcdef".into()),
        )
        .await
        .unwrap();

    let fresh = assets.find(&picture.id).await.unwrap().unwrap();
    assert_eq!(
        fresh.materials[0].content_hash,
        Some(digest),
        "the artefact axis survives a perceptual write"
    );
    assert_eq!(
        fresh.materials[0].content_hash_status,
        MeasurementStatus::Computed
    );
}
