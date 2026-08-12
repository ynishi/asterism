//! `MaterialLayerService` over real storage: who may write into which
//! band, and what a re-reading of the material does to the ones it owns.
//!
//! The service lives in `asterism-core`, and its tests live here for the
//! reason its sibling `material_mark_service.rs` gives: what it does
//! depends on rows in other aggregates — `asset.id` decides whether a
//! band may be opened at all — and standing those up as stubs would mean
//! hand-writing `AssetRepository`, a port of some forty verbs, of which
//! this service calls one. The stub's answer to that one would then be
//! the thing under test.
//!
//! Every fixture that asserts a refusal **also performs the act the
//! refusal is contrasted against**, in the same test. The immutability
//! guard is the whole point of this unit, and a guard test whose only
//! band is the one it expects to be refused would pass with the guard
//! replaced by `return Err(...)`.

use std::sync::Arc;

use asterism_core::application::MaterialLayerService;
use asterism_core::application_support::{ScannedChapter, replace_imported_chapters};
use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::material_layer::{LayerOrigin, LayerRole};
use asterism_core::domain::material_mark::TimelineSpan;
use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::{
    AssetRepository, LayerScope, MaterialLayerRepository, PersonaRepository,
};
use asterism_core::domain::value::{
    AssetId, ChapterMarkId, MaterialLayerId, Modality, PersonaId, SourceKind, SourceRef,
};
use asterism_core::error::DomainError;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::{
    SqliteAssetRepository, SqliteChapterMarkRepository, SqliteMaterialLayerRepository,
    SqlitePersonaRepository,
};
use chrono::Utc;
use rusqlite_isle::AsyncIsleDriver;

/// A caller that states nothing, which records nothing — layers carry
/// no attribution columns, the same as marks.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

struct Fixture {
    service: MaterialLayerService,
    layers: Arc<SqliteMaterialLayerRepository>,
    chapters: Arc<SqliteChapterMarkRepository>,
    assets: Arc<SqliteAssetRepository>,
    personas: Arc<SqlitePersonaRepository>,
    driver: AsyncIsleDriver,
}

impl Fixture {
    async fn open() -> Self {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let assets = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let personas = Arc::new(SqlitePersonaRepository::new(isle.clone()));
        let layers = Arc::new(SqliteMaterialLayerRepository::new(isle.clone()));
        let chapters = Arc::new(SqliteChapterMarkRepository::new(isle.clone()));
        let service = MaterialLayerService::new(layers.clone(), chapters.clone(), assets.clone());
        Self {
            service,
            layers,
            chapters,
            assets,
            personas,
            driver,
        }
    }

    async fn seed_persona(&self) -> PersonaId {
        let persona = Persona::new("P", None).unwrap();
        self.personas.save(&persona).await.unwrap();
        persona.id
    }

    async fn seed_asset(&self, persona: PersonaId) -> AssetId {
        let locator = format!("m-{}.mkv", uuid::Uuid::now_v7());
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            Some(Modality::new("video").unwrap()),
            Utc::now(),
            &unattributed(),
        );
        asset.duration_ms = Some(600_000);
        self.assets.save(&asset).await.unwrap();
        asset.id
    }

    /// The band a re-reading of the material owns, created the way
    /// production creates it — through the intake path, not by hand.
    async fn scan(&self, asset: AssetId, sections: &[(u64, Option<u64>, &str)]) -> MaterialLayerId {
        let scanned: Vec<ScannedChapter> = sections
            .iter()
            .map(|(start, end, label)| ScannedChapter {
                span: TimelineSpan::new(*start, *end).unwrap(),
                label: (*label).to_string(),
            })
            .collect();
        replace_imported_chapters(
            self.layers.as_ref(),
            self.chapters.as_ref(),
            &asset,
            0,
            &scanned,
        )
        .await
        .unwrap()
        .id
    }

    fn structure(&self, asset: AssetId) -> LayerScope {
        LayerScope {
            asset_id: asset,
            material_ord: 0,
            role: LayerRole::Structure,
        }
    }

    async fn close(self) {
        self.driver.shutdown().await.unwrap();
    }
}

/// Reading a material's chapters writes them into an imported band,
/// which becomes the default because nothing else holds that flag.
///
/// The claim about the *default* is the one worth pinning: a file that
/// declares its own divisions is the best answer available until a
/// person says otherwise, and a reader that had to choose a band by
/// hand before seeing any chapters would show an empty list on every
/// freshly imported file.
#[tokio::test]
async fn a_reading_of_the_material_lands_in_an_imported_band() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona).await;

    let layer = fx
        .scan(
            asset,
            &[
                (0, Some(60_000), "Opening"),
                (60_000, Some(300_000), ""),
                (300_000, None, "Ending"),
            ],
        )
        .await;

    let bands = fx.layers.list_by_asset(&asset).await.unwrap();
    assert_eq!(bands.len(), 1);
    assert_eq!(bands[0].origin, LayerOrigin::Imported);
    assert_eq!(bands[0].role, LayerRole::Structure);
    assert!(bands[0].is_default, "the file's own reading is shown first");

    let listed = fx.service.list_chapters(&layer).await.unwrap();
    assert_eq!(
        listed.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        vec!["Opening", "", "Ending"],
        "an untitled section survives as an untitled section"
    );
    assert_eq!(
        listed.iter().map(|c| c.ord).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "reading order is the container's own"
    );
    assert!(
        listed[2].span.is_instant(),
        "a section the container gave no end for stays open"
    );

    fx.close().await;
}

/// Re-reading replaces the imported band wholesale and leaves the
/// person's own band exactly as it was.
///
/// This is the behaviour the whole layer model exists for: before it,
/// "read the file again" either destroyed a person's chapters or
/// duplicated the file's. Both halves are asserted, because either one
/// alone passes under an implementation that gets the other wrong — a
/// replacement scoped to the whole asset would clear the user band, and
/// one that appended rather than replaced would leave the stale imported
/// sections behind.
#[tokio::test]
async fn re_reading_replaces_the_imported_band_and_spares_the_users() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona).await;

    let imported = fx.scan(asset, &[(0, Some(60_000), "As shipped")]).await;
    let mine = fx
        .service
        .create_user_layer(fx.structure(asset), 1, &unattributed())
        .await
        .unwrap();
    let of_mine = fx
        .service
        .post_chapter(
            &mine.id,
            TimelineSpan::new(12_000, Some(48_000)).unwrap(),
            "where it really starts",
            0,
            &unattributed(),
        )
        .await
        .unwrap();

    let same_layer = fx.scan(asset, &[(0, Some(90_000), "Re-cut")]).await;
    assert_eq!(
        same_layer, imported,
        "a second reading writes into the band the first one opened"
    );

    let after = fx.service.list_chapters(&imported).await.unwrap();
    assert_eq!(after.len(), 1, "the stale section is gone, not appended to");
    assert_eq!(after[0].label, "Re-cut");
    assert_eq!(after[0].span.end_ms(), Some(90_000));

    assert_eq!(
        fx.service.list_chapters(&mine.id).await.unwrap(),
        vec![of_mine],
        "the person's own reading is untouched by the file being read again"
    );

    // A file that no longer declares chapters empties its band, and
    // still does not reach across to the person's.
    fx.scan(asset, &[]).await;
    assert!(
        fx.service
            .list_chapters(&imported)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(fx.service.list_chapters(&mine.id).await.unwrap().len(), 1);

    fx.close().await;
}

/// The four writing verbs accept the person's band and refuse the
/// file's.
///
/// The accepted half runs first and against the same fixture, so the
/// refusals cannot be passing because the verbs are broken: every one of
/// them is shown working on a user band immediately before it is shown
/// refusing an imported one.
#[tokio::test]
async fn a_person_edits_their_own_band_and_not_the_files() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona).await;

    let imported = fx.scan(asset, &[(0, Some(60_000), "As shipped")]).await;
    let mine = fx
        .service
        .create_user_layer(fx.structure(asset), 1, &unattributed())
        .await
        .unwrap();

    // Accepted: post, edit, delete, and dropping the band itself.
    let posted = fx
        .service
        .post_chapter(
            &mine.id,
            TimelineSpan::new(1_000, Some(2_000)).unwrap(),
            "mine",
            0,
            &unattributed(),
        )
        .await
        .expect("a person writes into their own band");
    let edited = fx
        .service
        .edit_chapter(
            &mine.id,
            &posted.id,
            "corrected",
            Some(TimelineSpan::new(1_500, Some(2_500)).unwrap()),
            Some(3),
            &unattributed(),
        )
        .await
        .expect("and corrects it, position included");
    assert_eq!(edited.label, "corrected");
    assert_eq!(edited.span.start_ms(), 1_500, "a chapter may be moved");
    assert_eq!(edited.ord, 3);
    fx.service
        .delete_chapter(&mine.id, &posted.id, &unattributed())
        .await
        .expect("and removes it again");

    // Refused: the same four verbs against the file's own band.
    let existing = fx.service.list_chapters(&imported).await.unwrap()[0].id;
    let refusals: Vec<(&str, DomainError)> = vec![
        (
            "post",
            fx.service
                .post_chapter(
                    &imported,
                    TimelineSpan::new(5_000, None).unwrap(),
                    "not mine to write",
                    0,
                    &unattributed(),
                )
                .await
                .expect_err("an imported band is not a caller's to write into"),
        ),
        (
            "edit",
            fx.service
                .edit_chapter(
                    &imported,
                    &existing,
                    "retitled",
                    None,
                    None,
                    &unattributed(),
                )
                .await
                .expect_err("nor to edit"),
        ),
        (
            "delete chapter",
            fx.service
                .delete_chapter(&imported, &existing, &unattributed())
                .await
                .expect_err("nor to delete from"),
        ),
        (
            "delete layer",
            fx.service
                .delete_user_layer(&imported, &unattributed())
                .await
                .expect_err("nor to drop — the next reading would recreate it"),
        ),
    ];
    for (verb, err) in refusals {
        assert!(
            matches!(err, DomainError::Validation(_)),
            "{verb}: a caller writing into a band it does not own is a caller error, got {err:?}"
        );
    }

    assert_eq!(
        fx.service.list_chapters(&imported).await.unwrap().len(),
        1,
        "and none of the refusals touched the stored rows"
    );
    assert!(
        fx.layers
            .find(&imported)
            .await
            .unwrap()
            .is_some_and(|l| l.origin == LayerOrigin::Imported),
        "including the band itself"
    );

    fx.service
        .delete_user_layer(&mine.id, &unattributed())
        .await
        .expect("a person's own band is theirs to drop");
    assert_eq!(fx.layers.list_by_asset(&asset).await.unwrap().len(), 1);

    fx.close().await;
}

/// `set_default` moves the flag, and is open to every origin.
///
/// Choosing to read the file's own chapter list rather than one's own is
/// not an edit to either, so this verb deliberately does not carry the
/// guard the four above do — which is why the fixture moves the flag in
/// both directions.
#[tokio::test]
async fn choosing_which_band_to_read_is_not_an_edit_to_it() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona).await;

    let imported = fx.scan(asset, &[(0, None, "As shipped")]).await;
    let mine = fx
        .service
        .create_user_layer(fx.structure(asset), 1, &unattributed())
        .await
        .unwrap();
    assert!(!mine.is_default, "a new band does not seize the flag");

    fx.service
        .set_default(&mine.id, &unattributed())
        .await
        .unwrap();
    let flags = |bands: Vec<asterism_core::domain::material_layer::MaterialLayer>| {
        bands
            .into_iter()
            .map(|l| (l.id, l.is_default))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        flags(fx.service.list_by_asset(&asset).await.unwrap()),
        vec![(imported, false), (mine.id, true)]
    );

    fx.service
        .set_default(&imported, &unattributed())
        .await
        .expect("and back again — reading the file's list is always allowed");
    assert_eq!(
        flags(fx.service.list_by_asset(&asset).await.unwrap()),
        vec![(imported, true), (mine.id, false)]
    );

    fx.close().await;
}

/// The ids a verb is given have to name things that exist, and a
/// chapter has to be in the band it is named under.
///
/// The last of these is the one a `find(id)`-shaped implementation gets
/// wrong: with the chapter reached by id alone, a caller could pass its
/// own band's id and another band's chapter, pass the guard on the
/// former, and edit the latter.
#[tokio::test]
async fn a_verb_refuses_ids_that_name_nothing_or_name_another_band() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona).await;

    let imported = fx.scan(asset, &[(0, None, "As shipped")]).await;
    let mine = fx
        .service
        .create_user_layer(fx.structure(asset), 1, &unattributed())
        .await
        .unwrap();
    let of_the_file = fx.service.list_chapters(&imported).await.unwrap()[0].id;

    let err = fx
        .service
        .create_user_layer(
            LayerScope {
                asset_id: AssetId::new(),
                material_ord: 0,
                role: LayerRole::Structure,
            },
            0,
            &unattributed(),
        )
        .await
        .expect_err("an unknown asset has no material to band");
    assert!(
        matches!(err, DomainError::AssetNotFound(_)),
        "expected AssetNotFound, got {err:?}"
    );

    let err = fx
        .service
        .list_chapters(&MaterialLayerId::new())
        .await
        .expect_err("an id no band carries has no chapters");
    assert!(
        matches!(err, DomainError::NotFound { .. }),
        "expected NotFound, got {err:?}"
    );

    let err = fx
        .service
        .edit_chapter(
            &mine.id,
            &of_the_file,
            "smuggled",
            None,
            None,
            &unattributed(),
        )
        .await
        .expect_err("naming one's own band does not reach into another's chapter");
    assert!(
        matches!(err, DomainError::NotFound { .. }),
        "expected NotFound, got {err:?}"
    );

    let err = fx
        .service
        .delete_chapter(&mine.id, &ChapterMarkId::new(), &unattributed())
        .await
        .expect_err("deleting a chapter that was never in this band is not a no-op");
    assert!(
        matches!(err, DomainError::NotFound { .. }),
        "expected NotFound, got {err:?}"
    );

    assert_eq!(
        fx.service.list_chapters(&imported).await.unwrap()[0].label,
        "As shipped",
        "no refusal reached the row"
    );

    fx.close().await;
}

/// A chapter verb aimed at an annotation band is refused.
///
/// The two roles hold different aggregates: a `chapter_mark` row in an
/// annotation band would be invisible to every reader of that band,
/// which reads `material_mark`. Contrasted against the same call on a
/// structure band, so the refusal cannot be the verb simply not working.
#[tokio::test]
async fn chapters_do_not_go_into_a_band_that_holds_notes() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona).await;

    let notes = fx
        .service
        .create_user_layer(
            LayerScope {
                asset_id: asset,
                material_ord: 0,
                role: LayerRole::Annotation,
            },
            0,
            &unattributed(),
        )
        .await
        .unwrap();
    let sections = fx
        .service
        .create_user_layer(fx.structure(asset), 0, &unattributed())
        .await
        .unwrap();

    fx.service
        .post_chapter(
            &sections.id,
            TimelineSpan::new(0, None).unwrap(),
            "a section",
            0,
            &unattributed(),
        )
        .await
        .expect("a structure band is where sections go");

    let err = fx
        .service
        .post_chapter(
            &notes.id,
            TimelineSpan::new(0, None).unwrap(),
            "a section",
            0,
            &unattributed(),
        )
        .await
        .expect_err("a band that holds notes has no sections in it");
    assert!(
        matches!(err, DomainError::Validation(_)),
        "expected Validation, got {err:?}"
    );

    fx.close().await;
}
