//! `MaterialMarkService` over real storage: which marks a post is
//! allowed to place, and what the four verbs leave behind.
//!
//! The service lives in `asterism-core`, and its tests live here for the
//! reason `duplicate_detection.rs` gives beside it: what it does depends
//! on rows in *other* aggregates — `asset.duration_ms` decides whether a
//! timeline exists to mark at all, and `persona.id` decides whether a
//! named author does. Standing those up as stubs would mean hand-writing
//! `AssetRepository`, a port of some forty verbs, of which this service
//! calls one; and the stub's answer to that one would then be the thing
//! under test.
//!
//! Every fixture that asserts a refusal **also places the mark the
//! refusal is contrasted against**, in the same test. A guard test whose
//! only asset is the one it expects to be refused would pass with the
//! guard replaced by `return Err(...)`.

use std::sync::Arc;

use asterism_contract::command::{
    DeleteMaterialMarkCommand, EditMaterialMarkCommand, PostMaterialMarkCommand,
};
use asterism_core::application::MaterialMarkService;
use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::material_layer::{LayerOrigin, LayerRole};
use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::{
    AssetRepository, MaterialLayerRepository, PersonaRepository,
};
use asterism_core::domain::value::{AssetId, Modality, PersonaId, SourceKind, SourceRef};
use asterism_core::error::DomainError;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::{
    SqliteAssetRepository, SqliteMaterialLayerRepository, SqliteMaterialMarkRepository,
    SqlitePersonaRepository,
};
use chrono::Utc;
use rusqlite_isle::AsyncIsleDriver;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. A mark records a `CommentAuthor` —
/// the voice — and this argument is not it.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// The service wired to real adapters, plus the handles a fixture needs
/// to seed rows the service only reads.
struct Fixture {
    service: MaterialMarkService,
    assets: Arc<SqliteAssetRepository>,
    personas: Arc<SqlitePersonaRepository>,
    /// Held so the layer assertions can read the bands the service
    /// created behind the post — the whole point of the lazy default
    /// is that no caller names one, so nothing else in these fixtures
    /// would ever see it.
    layers: Arc<SqliteMaterialLayerRepository>,
    driver: AsyncIsleDriver,
}

impl Fixture {
    async fn open() -> Self {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let assets = Arc::new(SqliteAssetRepository::new(isle.clone()));
        let personas = Arc::new(SqlitePersonaRepository::new(isle.clone()));
        let marks = Arc::new(SqliteMaterialMarkRepository::new(isle.clone()));
        let layers = Arc::new(SqliteMaterialLayerRepository::new(isle.clone()));
        let service =
            MaterialMarkService::new(marks, layers.clone(), assets.clone(), personas.clone());
        Self {
            service,
            assets,
            personas,
            layers,
            driver,
        }
    }

    async fn seed_persona(&self) -> PersonaId {
        let persona = Persona::new("P", None).unwrap();
        self.personas.save(&persona).await.unwrap();
        persona.id
    }

    /// One asset owned by `persona`. `duration_ms` is the axis under
    /// test: `Some` is a material with a timeline, `None` one without.
    async fn seed_asset(&self, persona: PersonaId, duration_ms: Option<u64>) -> AssetId {
        let locator = format!("m-{}.bin", uuid::Uuid::now_v7());
        let mut asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            Some(Modality::new("video").unwrap()),
            Utc::now(),
            &unattributed(),
        );
        asset.duration_ms = duration_ms;
        self.assets.save(&asset).await.unwrap();
        // The precondition is read back off the row, not off the entity
        // this fixture happens to hold — a `save` that dropped the
        // column would otherwise go unnoticed here and take the guard
        // test with it.
        assert_eq!(
            self.assets
                .find(&asset.id)
                .await
                .unwrap()
                .unwrap()
                .duration_ms,
            duration_ms,
            "the duration the guard reads has to survive the write"
        );
        asset.id
    }

    async fn close(self) {
        self.driver.shutdown().await.unwrap();
    }
}

/// A user-authored temporal post, filled in field by field so each test
/// can vary exactly one of them.
fn post_at(asset: AssetId, start_ms: Option<i64>, end_ms: Option<i64>) -> PostMaterialMarkCommand {
    PostMaterialMarkCommand {
        asset_id: asset.to_string(),
        anchor_kind: "temporal".into(),
        start_ms,
        end_ms,
        author_kind: "user".into(),
        author_persona_id: None,
        body: "here".into(),
    }
}

/// A timeline anchor needs a timeline: the same post is accepted on an
/// asset with a duration and refused on one without.
///
/// The pair is the test. `duration_ms` is `None` on most rows in a real
/// library (every image), so a guard asserted only against the refusal
/// would say nothing about which assets stay markable.
///
/// Checked by mutation on 2026-08-06: with the `duration_ms.is_none()`
/// branch in `material_mark_service::build_anchor` disabled, this test
/// failed — the untimed post came back as a placed
/// `MaterialMarkDto { anchor_kind: "temporal", start_ms: Some(1500), … }`.
/// Restored, it passes.
#[tokio::test]
async fn post_refuses_a_material_with_no_timeline() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let timed = fx.seed_asset(persona, Some(30_000)).await;
    let untimed = fx.seed_asset(persona, None).await;

    let placed = fx
        .service
        .post(post_at(timed, Some(1_500), None), &unattributed())
        .await
        .expect("an asset with a duration has a timeline to mark");
    assert_eq!(placed.start_ms, Some(1_500));
    assert_eq!(placed.anchor_kind, "temporal");

    let err = fx
        .service
        .post(post_at(untimed, Some(1_500), None), &unattributed())
        .await
        .expect_err("an asset with no duration has no timeline to mark");
    assert!(
        matches!(err, DomainError::Validation(_)),
        "a caller marking a material that has no timeline is a caller error, got {err:?}"
    );
    assert!(
        fx.service
            .list_by_asset(&untimed.to_string())
            .await
            .unwrap()
            .is_empty(),
        "the refused post must not have reached the row"
    );

    fx.close().await;
}

/// A post against an id no asset carries is `AssetNotFound`, not a
/// silently orphaned row.
#[tokio::test]
async fn post_refuses_an_unknown_asset() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let known = fx.seed_asset(persona, Some(30_000)).await;
    fx.service
        .post(post_at(known, Some(0), None), &unattributed())
        .await
        .expect("the id that does exist is markable");

    let err = fx
        .service
        .post(post_at(AssetId::new(), Some(0), None), &unattributed())
        .await
        .expect_err("an unknown asset has no material");
    assert!(
        matches!(err, DomainError::AssetNotFound(_)),
        "expected AssetNotFound, got {err:?}"
    );

    fx.close().await;
}

/// A persona author has to exist, and a persona post has to name one.
#[tokio::test]
async fn post_checks_the_named_persona_exists() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some(30_000)).await;

    let by_persona = PostMaterialMarkCommand {
        author_kind: "persona".into(),
        author_persona_id: Some(persona.to_string()),
        ..post_at(asset, Some(500), None)
    };
    let placed = fx
        .service
        .post(by_persona.clone(), &unattributed())
        .await
        .expect("a persona that exists may speak");
    assert_eq!(placed.author_kind, "persona");
    assert_eq!(placed.author_persona_id, Some(persona.to_string()));

    let stranger = PostMaterialMarkCommand {
        author_persona_id: Some(PersonaId::new().to_string()),
        ..by_persona.clone()
    };
    let err = fx
        .service
        .post(stranger, &unattributed())
        .await
        .expect_err("a persona nobody registered cannot author a mark");
    assert!(
        matches!(err, DomainError::PersonaNotFound(_)),
        "expected PersonaNotFound, got {err:?}"
    );

    let anonymous = PostMaterialMarkCommand {
        author_persona_id: None,
        ..by_persona
    };
    let err = fx
        .service
        .post(anonymous, &unattributed())
        .await
        .expect_err("author_kind = persona without an id names nobody");
    assert!(
        matches!(err, DomainError::Validation(_)),
        "expected Validation, got {err:?}"
    );

    fx.close().await;
}

/// A temporal post needs a position, and the position runs forward from
/// the presentation origin.
#[tokio::test]
async fn post_refuses_a_temporal_anchor_it_cannot_place() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some(30_000)).await;

    fx.service
        .post(post_at(asset, Some(0), Some(1)), &unattributed())
        .await
        .expect("the origin, one millisecond wide, is a position");

    for (label, command) in [
        ("no start", post_at(asset, None, None)),
        ("start before the origin", post_at(asset, Some(-1), None)),
        (
            "an interval covering nothing",
            post_at(asset, Some(5), Some(5)),
        ),
        ("an inverted interval", post_at(asset, Some(5), Some(4))),
        (
            "an anchor kind this build cannot place",
            PostMaterialMarkCommand {
                anchor_kind: "spatial".into(),
                ..post_at(asset, Some(5), None)
            },
        ),
    ] {
        let err = fx
            .service
            .post(command, &unattributed())
            .await
            .err()
            .unwrap_or_else(|| panic!("{label} must not place a mark"));
        assert!(
            matches!(err, DomainError::Validation(_)),
            "{label}: expected Validation, got {err:?}"
        );
    }

    fx.close().await;
}

/// What is placed comes back, in the material's order rather than the
/// order it was placed in, with the anchor intact.
///
/// The two marks go in at descending positions so that arrival order and
/// timeline order disagree; with them agreeing, the assertion would hold
/// with the ordering removed.
#[tokio::test]
async fn post_then_list_reads_the_material_in_order() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some(30_000)).await;
    let other = fx.seed_asset(persona, Some(30_000)).await;

    fx.service
        .post(post_at(asset, Some(9_000), Some(9_500)), &unattributed())
        .await
        .unwrap();
    fx.service
        .post(post_at(asset, Some(1_000), None), &unattributed())
        .await
        .unwrap();
    fx.service
        .post(post_at(other, Some(2_000), None), &unattributed())
        .await
        .unwrap();

    let listed = fx.service.list_by_asset(&asset.to_string()).await.unwrap();
    assert_eq!(
        listed.iter().map(|m| m.start_ms).collect::<Vec<_>>(),
        vec![Some(1_000), Some(9_000)],
        "the later-placed, earlier-positioned mark reads first"
    );
    assert_eq!(listed[0].end_ms, None, "an instant stays an instant");
    assert_eq!(listed[1].end_ms, Some(9_500));
    assert_eq!(listed[0].author_kind, "user");
    assert_eq!(listed[0].author_persona_id, None);
    assert_eq!(listed[0].edited_at_ms, None, "a fresh mark is unedited");
    assert_eq!(listed[0].asset_id, asset.to_string());

    let elsewhere = fx.service.list_by_asset(&other.to_string()).await.unwrap();
    assert_eq!(
        elsewhere.len(),
        1,
        "a listing is one asset's material, not the table"
    );

    fx.close().await;
}

/// `edit` rewrites the body, stamps `edited_at`, leaves the anchor where
/// it was, and refuses a body the domain refuses.
#[tokio::test]
async fn edit_rewrites_the_body_and_leaves_the_anchor() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some(30_000)).await;
    let placed = fx
        .service
        .post(post_at(asset, Some(1_000), Some(2_000)), &unattributed())
        .await
        .unwrap();

    let edit = EditMaterialMarkCommand {
        asset_id: asset.to_string(),
        mark_id: placed.id.clone(),
        body: "reworded".into(),
    };
    let edited = fx
        .service
        .edit(edit.clone(), &unattributed())
        .await
        .unwrap();
    assert_eq!(edited.body, "reworded");
    assert!(edited.edited_at_ms.is_some(), "an edit is stamped");
    assert_eq!(edited.start_ms, Some(1_000), "rewording does not move it");
    assert_eq!(edited.end_ms, Some(2_000));
    assert_eq!(
        fx.service
            .list_by_asset(&asset.to_string())
            .await
            .unwrap()
            .len(),
        1,
        "an edit replaces the mark rather than adding one"
    );

    // A tab is the case the schema cannot refuse (SQL `trim` strips only
    // U+0020) and the domain does.
    let blanked = EditMaterialMarkCommand {
        body: "\t".into(),
        ..edit.clone()
    };
    let err = fx
        .service
        .edit(blanked, &unattributed())
        .await
        .expect_err("a body that trims to nothing is not a mark");
    assert!(
        matches!(err, DomainError::Validation(_)),
        "expected Validation, got {err:?}"
    );

    let missing = EditMaterialMarkCommand {
        mark_id: uuid::Uuid::now_v7().to_string(),
        ..edit
    };
    let err = fx
        .service
        .edit(missing, &unattributed())
        .await
        .expect_err("an id no mark carries has no body to rewrite");
    assert!(
        matches!(err, DomainError::NotFound { .. }),
        "expected NotFound, got {err:?}"
    );

    assert_eq!(
        fx.service.list_by_asset(&asset.to_string()).await.unwrap()[0].body,
        "reworded",
        "neither refusal touched the stored body"
    );

    fx.close().await;
}

/// `delete` removes the one mark named and nothing else, and repeating
/// it is a no-op.
#[tokio::test]
async fn delete_removes_one_mark_and_is_idempotent() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some(30_000)).await;
    let first = fx
        .service
        .post(post_at(asset, Some(1_000), None), &unattributed())
        .await
        .unwrap();
    let second = fx
        .service
        .post(post_at(asset, Some(2_000), None), &unattributed())
        .await
        .unwrap();

    let command = DeleteMaterialMarkCommand {
        mark_id: first.id.clone(),
    };
    fx.service
        .delete(command.clone(), &unattributed())
        .await
        .unwrap();
    let left = fx.service.list_by_asset(&asset.to_string()).await.unwrap();
    assert_eq!(
        left.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
        vec![second.id],
        "the mark named went, the other stayed"
    );

    fx.service
        .delete(command, &unattributed())
        .await
        .expect("deleting a mark that is already gone is a no-op");

    fx.close().await;
}

/// A post names no layer, and lands in one anyway: the asset's default
/// annotation band, created on the first mark it ever receives.
///
/// Three claims, and each is one a plausible implementation gets wrong.
/// That the band is created lazily — an asset nobody has marked carries
/// no row, which is what keeps a hundred-thousand-image library from
/// carrying a hundred thousand empty bands. That the *second* post
/// reuses it rather than opening another, which is the difference
/// between a default and a per-mark band. And that the band is the
/// user's: `layer_id` is `NOT NULL`, so something has to be chosen, and
/// an imported or machine-owned choice would put every note in a band
/// the immutability guard forbids writing to.
#[tokio::test]
async fn post_creates_the_default_annotation_band_once_and_reuses_it() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let asset = fx.seed_asset(persona, Some(30_000)).await;

    assert!(
        fx.layers.list_by_asset(&asset).await.unwrap().is_empty(),
        "an asset nobody has marked carries no bands"
    );

    fx.service
        .post(post_at(asset, Some(1_000), None), &unattributed())
        .await
        .unwrap();
    let bands = fx.layers.list_by_asset(&asset).await.unwrap();
    assert_eq!(bands.len(), 1, "the first mark opens exactly one band");
    let band = &bands[0];
    assert_eq!(
        band.origin,
        LayerOrigin::User,
        "notes land in the user's own"
    );
    assert_eq!(band.role, LayerRole::Annotation);
    assert!(
        band.is_default,
        "and it is the one an unnamed post resolves"
    );
    assert_eq!(band.material_ord, 0, "the primary original");

    fx.service
        .post(post_at(asset, Some(2_000), None), &unattributed())
        .await
        .unwrap();
    let after = fx.layers.list_by_asset(&asset).await.unwrap();
    assert_eq!(
        after, bands,
        "the second mark joins the band rather than opening another"
    );
    assert_eq!(
        fx.service
            .list_by_asset(&asset.to_string())
            .await
            .unwrap()
            .len(),
        2,
        "and both marks are there"
    );

    fx.close().await;
}

/// A post that is going to be refused leaves no band behind.
///
/// The band is resolved after the anchor and the author are checked,
/// and this is what that ordering is for: an asset with no timeline can
/// never carry a mark, so a band opened on the way to refusing one
/// would be a row that nothing will ever write into and nothing will
/// ever clean up. The accepted post beside it is what keeps the
/// assertion from passing with the whole resolution step deleted.
#[tokio::test]
async fn a_refused_post_opens_no_band() {
    let fx = Fixture::open().await;
    let persona = fx.seed_persona().await;
    let timed = fx.seed_asset(persona, Some(30_000)).await;
    let untimed = fx.seed_asset(persona, None).await;

    fx.service
        .post(post_at(timed, Some(1_500), None), &unattributed())
        .await
        .expect("an asset with a duration has a timeline to mark");
    assert_eq!(fx.layers.list_by_asset(&timed).await.unwrap().len(), 1);

    fx.service
        .post(post_at(untimed, Some(1_500), None), &unattributed())
        .await
        .expect_err("an asset with no duration has no timeline to mark");
    assert!(
        fx.layers.list_by_asset(&untimed).await.unwrap().is_empty(),
        "the refused post must not have left a band on the way out"
    );

    fx.close().await;
}
