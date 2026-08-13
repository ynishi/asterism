//! The wire face of `MaterialLayerService` over real storage: what the
//! three adapters (HTTP, MCP, Tauri IPC) actually call.
//!
//! Its sibling `material_layer_service.rs` covers the acts themselves —
//! who may write into which band, and what a re-reading does. What is
//! left, and what is asserted here, is everything the wire adds on top
//! of them: the assembly `list_views` performs, the two commands whose
//! meaning is carried by an *absent* field, and the fact that a refusal
//! survives the trip through a command rather than being lost in the
//! parse.
//!
//! It lives here rather than in `asterism-core` for the reason that
//! sibling gives: what these verbs do depends on rows in other
//! aggregates, and standing those up as stubs would make the stub the
//! thing under test.

use std::sync::Arc;

use asterism_contract::command::{
    CreateMaterialLayerCommand, DeleteChapterMarkCommand, DeleteMaterialLayerCommand,
    EditChapterMarkCommand, PostChapterMarkCommand, SetDefaultMaterialLayerCommand,
};
use asterism_core::application::MaterialLayerService;
use asterism_core::application_support::{ScannedChapter, replace_imported_chapters};
use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::material_mark::TimelineSpan;
use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::{AssetRepository, PersonaRepository};
use asterism_core::domain::value::{
    AssetId, MaterialLayerId, Modality, PersonaId, SourceKind, SourceRef,
};
use asterism_core::error::DomainError;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::{
    SqliteAssetRepository, SqliteChapterMarkRepository, SqliteMaterialLayerRepository,
    SqlitePersonaRepository,
};
use chrono::Utc;
use rusqlite_isle::AsyncIsleDriver;

/// A caller that states nothing, which records nothing — layers carry no
/// attribution columns, the same as marks.
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

    async fn seed_asset(&self) -> AssetId {
        let persona = self.seed_persona().await;
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

    async fn seed_persona(&self) -> PersonaId {
        let persona = Persona::new("P", None).unwrap();
        self.personas.save(&persona).await.unwrap();
        persona.id
    }

    /// The band a re-reading of the material owns, created through the
    /// intake path rather than by hand.
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

    /// Opens a band of the given role through the wire face, returning
    /// its id as the wire spells it.
    async fn create(&self, asset: AssetId, role: &str, ord: u32) -> String {
        self.service
            .create_layer(
                CreateMaterialLayerCommand {
                    asset_id: asset.to_string(),
                    material_ord: None,
                    role: role.to_string(),
                    ord,
                },
                &unattributed(),
            )
            .await
            .expect("a band the person owns")
            .id
    }

    async fn close(self) {
        self.driver.shutdown().await.unwrap();
    }
}

/// An asset-level read answers with every band and the sections in each,
/// and an annotation band's list is empty rather than absent.
///
/// The annotation half is the claim worth pinning: that band holds notes,
/// which are read through the mark route, so an empty `chapters` there is
/// a statement about where to look rather than a band that happens to
/// have nothing in it.
#[tokio::test]
async fn an_asset_level_read_carries_each_bands_sections_with_it() {
    let fx = Fixture::open().await;
    let asset = fx.seed_asset().await;

    let imported = fx
        .scan(asset, &[(0, Some(60_000), "Opening"), (60_000, None, "")])
        .await;
    let notes = fx.create(asset, "annotation", 0).await;

    let views = fx.service.list_views(&asset.to_string()).await.unwrap();
    assert_eq!(views.len(), 2, "both bands are answered for");

    let file = views
        .iter()
        .find(|v| v.layer.id == imported.to_string())
        .expect("the band the reading of the material opened");
    assert_eq!(file.layer.origin, "imported");
    assert_eq!(file.layer.role, "structure");
    assert!(
        file.layer.is_default,
        "the file's own reading is shown first"
    );
    assert_eq!(
        file.chapters
            .iter()
            .map(|c| (c.start_ms, c.end_ms, c.label.as_str(), c.ord))
            .collect::<Vec<_>>(),
        vec![(0, Some(60_000), "Opening", 0), (60_000, None, "", 1),],
        "an untitled section and one with no stated end survive the trip"
    );

    let mine = views
        .iter()
        .find(|v| v.layer.id == notes)
        .expect("the band the command opened");
    assert_eq!(mine.layer.origin, "user");
    assert_eq!(mine.layer.role, "annotation");
    assert!(!mine.layer.is_default, "a new band does not seize the flag");
    assert!(
        mine.chapters.is_empty(),
        "a band that holds notes states no sections"
    );

    // The per-band read answers with the same rows as the bundle, which
    // is what lets a surface refresh one band without re-reading the
    // asset.
    assert_eq!(
        fx.service
            .list_chapter_marks(&imported.to_string())
            .await
            .unwrap()
            .len(),
        file.chapters.len()
    );

    fx.close().await;
}

/// A create that names no `material_ord` means the primary original, and
/// a role this build has no variant for is refused as a caller error.
#[tokio::test]
async fn a_create_defaults_to_the_primary_original_and_refuses_an_unknown_role() {
    let fx = Fixture::open().await;
    let asset = fx.seed_asset().await;

    let sections = fx.create(asset, "structure", 1).await;
    let opened = fx
        .service
        .list_views(&asset.to_string())
        .await
        .unwrap()
        .into_iter()
        .find(|v| v.layer.id == sections)
        .expect("the band that was just opened");
    assert_eq!(
        opened.layer.material_ord, 0,
        "an omitted ordinal is the original every surface marks"
    );
    assert_eq!(opened.layer.ord, 1, "display order is the caller's");

    let err = fx
        .service
        .create_layer(
            CreateMaterialLayerCommand {
                asset_id: asset.to_string(),
                material_ord: None,
                role: "chapters".into(),
                ord: 0,
            },
            &unattributed(),
        )
        .await
        .expect_err("a role slug this build has no variant for");
    assert!(
        matches!(err, DomainError::Validation(_)),
        "a slug a caller sent is a caller error, got {err:?}"
    );

    let err = fx
        .service
        .list_views("not-a-uuid")
        .await
        .expect_err("an id that is not one");
    assert!(
        matches!(err, DomainError::Validation(_)),
        "expected Validation, got {err:?}"
    );

    fx.close().await;
}

/// On an edit, `start_ms` absent leaves the section where it is; present
/// with no `end_ms` moves it and gives it no stated end.
///
/// The two are one absent field apart, which is the whole reason `end_ms`
/// is read only when `start_ms` is there — asserted in both directions
/// because either alone passes under an implementation that ignores the
/// pair entirely.
#[tokio::test]
async fn an_edit_reads_the_end_only_when_the_start_is_there() {
    let fx = Fixture::open().await;
    let asset = fx.seed_asset().await;
    let band = fx.create(asset, "structure", 0).await;

    let posted = fx
        .service
        .post_chapter_mark(
            PostChapterMarkCommand {
                layer_id: band.clone(),
                start_ms: 1_000,
                end_ms: Some(2_000),
                label: "as posted".into(),
                ord: 0,
            },
            &unattributed(),
        )
        .await
        .expect("a person writes into their own band");
    assert_eq!((posted.start_ms, posted.end_ms), (1_000, Some(2_000)));

    let retitled = fx
        .service
        .edit_chapter_mark(
            EditChapterMarkCommand {
                layer_id: band.clone(),
                chapter_id: posted.id.clone(),
                label: "retitled".into(),
                start_ms: None,
                // Ignored: without a start there is no span to state an
                // end for, and honouring it here would make "leave it
                // alone" unsayable.
                end_ms: Some(9_999),
                ord: None,
            },
            &unattributed(),
        )
        .await
        .expect("a retitle that does not move the section");
    assert_eq!(retitled.label, "retitled");
    assert_eq!(
        (retitled.start_ms, retitled.end_ms),
        (1_000, Some(2_000)),
        "an absent start leaves the whole span alone, end included"
    );
    assert_eq!(retitled.ord, 0, "an absent ord leaves the order alone");

    let moved = fx
        .service
        .edit_chapter_mark(
            EditChapterMarkCommand {
                layer_id: band.clone(),
                chapter_id: posted.id.clone(),
                label: "moved".into(),
                start_ms: Some(5_000),
                end_ms: None,
                ord: Some(3),
            },
            &unattributed(),
        )
        .await
        .expect("a move to a section with no stated end");
    assert_eq!((moved.start_ms, moved.end_ms), (5_000, None));
    assert_eq!(moved.ord, 3);

    fx.service
        .delete_chapter_mark(
            DeleteChapterMarkCommand {
                layer_id: band.clone(),
                chapter_id: posted.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("and removes it again");
    assert!(
        fx.service
            .list_chapter_marks(&band)
            .await
            .unwrap()
            .is_empty()
    );

    fx.close().await;
}

/// The ownership guard is the same refusal through a command as through
/// the domain verb, and choosing which band to show still is not an edit
/// to either.
///
/// Contrasted against the accepted call in the same fixture, so the
/// refusals cannot be passing because the wire face is simply broken.
#[tokio::test]
async fn the_guard_survives_the_trip_through_a_command() {
    let fx = Fixture::open().await;
    let asset = fx.seed_asset().await;

    let imported = fx.scan(asset, &[(0, Some(60_000), "As shipped")]).await;
    let mine = fx.create(asset, "structure", 1).await;

    // Accepted: the person's own band takes the flag, and gives it back.
    for band in [&mine, &imported.to_string()] {
        fx.service
            .set_default_layer(
                SetDefaultMaterialLayerCommand {
                    layer_id: band.clone(),
                },
                &unattributed(),
            )
            .await
            .expect("choosing which reading to show is not an edit to it");
    }

    // Refused: writing into, and dropping, the band the file owns.
    let of_the_file = fx
        .service
        .list_chapter_marks(&imported.to_string())
        .await
        .unwrap()[0]
        .id
        .clone();
    let refusals: Vec<(&str, DomainError)> = vec![
        (
            "post",
            fx.service
                .post_chapter_mark(
                    PostChapterMarkCommand {
                        layer_id: imported.to_string(),
                        start_ms: 5_000,
                        end_ms: None,
                        label: "not mine to write".into(),
                        ord: 0,
                    },
                    &unattributed(),
                )
                .await
                .expect_err("an imported band is not a caller's to write into"),
        ),
        (
            "edit",
            fx.service
                .edit_chapter_mark(
                    EditChapterMarkCommand {
                        layer_id: imported.to_string(),
                        chapter_id: of_the_file.clone(),
                        label: "retitled".into(),
                        start_ms: None,
                        end_ms: None,
                        ord: None,
                    },
                    &unattributed(),
                )
                .await
                .expect_err("nor to edit"),
        ),
        (
            "delete chapter",
            fx.service
                .delete_chapter_mark(
                    DeleteChapterMarkCommand {
                        layer_id: imported.to_string(),
                        chapter_id: of_the_file.clone(),
                    },
                    &unattributed(),
                )
                .await
                .expect_err("nor to delete from"),
        ),
        (
            "delete layer",
            fx.service
                .delete_layer(
                    DeleteMaterialLayerCommand {
                        layer_id: imported.to_string(),
                    },
                    &unattributed(),
                )
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
        fx.service
            .list_chapter_marks(&imported.to_string())
            .await
            .unwrap()
            .len(),
        1,
        "and none of the refusals reached the stored rows"
    );

    fx.service
        .delete_layer(
            DeleteMaterialLayerCommand {
                layer_id: mine.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("a person's own band is theirs to drop");
    assert_eq!(
        fx.service
            .list_views(&asset.to_string())
            .await
            .unwrap()
            .len(),
        1
    );

    fx.close().await;
}
