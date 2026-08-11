//! Pixel dimensions from the ingest command to the detail payload.
//!
//! The seam this file covers is `AddAssetCommand` → the row → `AssetDto`,
//! which is the half of the road no unit test spans: the importer half
//! (`Footprint` → `AssetSpec` → `AddAssetCommand`) is asserted in
//! `asterism-importer-sdk/src/footprint.rs`, and the column half
//! (`save` → `find`, the overwrite rule, an out-of-range read) in
//! `asterism-infra/src/sqlite/repo/asset.rs`.
//!
//! **Every fixture uses a width that differs from its height.** The two
//! fields are independent `Option<u32>`s by choice, so nothing in the
//! type system stops a transposed copy — a square fixture would pass one
//! at every hop below.
//!
//! Its own test binary because `init_core` opens the profile-global
//! Tantivy index (one core per test binary, as with the sibling e2e
//! files).

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_contract::query::GetAssetDetailQuery;
use asterism_core::error::DomainError;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the two columns, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str, occurred_at_ms: i64) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: None,
        occurred_at_ms,
        session_id: None,
        external_session_key: None,
        external_key: None,
        bundle_id: None,
        labels: Vec::new(),
        register_note: None,
        platform: None,
        file_size_bytes: None,
        duration_ms: None,
        width_px: None,
        height_px: None,
        extra_json: None,
        cover_hint: None,
        auto_organize_base_dir: None,
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

/// Boots a core over a fresh temp profile and registers one persona.
///
/// Returns the tempdir alongside the context: dropping it would take the
/// database out from under the core mid-test.
async fn one_persona(tag: &str) -> (tempfile::TempDir, CoreCtx, String, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(format!("e2e-{tag}")),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    (tmp, core, persona.id, corpus)
}

/// Writes a file and returns its locator.
fn a_file(corpus: &std::path::Path, name: &str) -> String {
    let path = corpus.join(name);
    std::fs::write(&path, "# body\n").expect("write file");
    path.to_str().expect("utf-8 path").to_string()
}

/// A measured pair reaches the detail payload — and comes back the way
/// round it went in.
///
/// Both the value `add` returns and a later `detail` read are checked.
/// The first is the mapping in front of the write; the second is the one
/// behind the read, and they are different code (`asset_to_dto` over a
/// freshly built entity versus one hydrated from the row).
#[tokio::test(flavor = "multi_thread")]
async fn a_measured_pair_reaches_the_detail_payload_unswapped() {
    let (_tmp, core, persona, corpus) = one_persona("dims-roundtrip").await;

    let mut command = add_command(
        &persona,
        &a_file(&corpus, "measured.png"),
        1_785_000_000_000,
    );
    command.width_px = Some(1920);
    command.height_px = Some(1080);
    assert_ne!(
        command.width_px, command.height_px,
        "a square fixture would pass a transposed copy at every hop"
    );

    let added = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("add asset");
    assert_eq!(
        (added.width_px, added.height_px),
        (Some(1920), Some(1080)),
        "the value `add` answers with is not what was stated"
    );

    let detail = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: added.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("detail");
    assert_eq!(
        (detail.asset.width_px, detail.asset.height_px),
        (Some(1920), Some(1080)),
        "the pair did not survive the round trip through the row"
    );
}

/// **A `(0, 0)` pair is a statement and stays one.**
///
/// Nothing on this road may read a zero as "nobody measured": that is the
/// reading `NULL` carries, and the two have to stay distinguishable in
/// both directions. Whether a parser can produce a zero is a separate
/// question this change does not answer — which is exactly why the road
/// must not decide it.
#[tokio::test(flavor = "multi_thread")]
async fn a_zero_pair_is_not_folded_into_absence() {
    let (_tmp, core, persona, corpus) = one_persona("dims-zero").await;

    let mut zeroed = add_command(&persona, &a_file(&corpus, "zeroed.png"), 1_785_000_000_000);
    zeroed.width_px = Some(0);
    zeroed.height_px = Some(0);
    let added = core
        .asset_service
        .add(zeroed, &unattributed())
        .await
        .expect("add asset");
    assert_eq!(
        (added.width_px, added.height_px),
        (Some(0), Some(0)),
        "a stated zero was read as an absence"
    );

    let detail = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: added.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("detail");
    assert_eq!(
        (detail.asset.width_px, detail.asset.height_px),
        (Some(0), Some(0)),
        "and it has to still be a zero after the row"
    );

    // The other direction, in the same fixture, so the pair of readings
    // is asserted against each other rather than in two files.
    let unmeasured = core
        .asset_service
        .add(
            add_command(
                &persona,
                &a_file(&corpus, "unmeasured.png"),
                1_785_000_001_000,
            ),
            &unattributed(),
        )
        .await
        .expect("add asset");
    assert_eq!(
        (unmeasured.width_px, unmeasured.height_px),
        (None, None),
        "an asset nobody measured must not report zero pixels"
    );
    assert_ne!(
        (added.width_px, added.height_px),
        (unmeasured.width_px, unmeasured.height_px),
        "a stated zero and an unmeasured asset are two different answers"
    );
}

/// An asset with no dimensions is still readable through `detail`.
///
/// This is the shape of **every row that predates the migration**, so it
/// is the case a library upgrading into this change consists of. A read
/// path that required the pair, or that refused a half, would make those
/// rows unopenable.
#[tokio::test(flavor = "multi_thread")]
async fn an_asset_without_dimensions_still_opens() {
    let (_tmp, core, persona, corpus) = one_persona("dims-absent").await;

    let added = core
        .asset_service
        .add(
            add_command(&persona, &a_file(&corpus, "legacy.md"), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add asset");

    let detail = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: added.id.clone(),
            viewer_subject: None,
        })
        .await
        .expect("an asset with no dimensions has to stay retrievable");
    assert_eq!(detail.asset.id, added.id);
    assert_eq!(
        (detail.asset.width_px, detail.asset.height_px),
        (None, None)
    );
}

/// **Half a resolution is refused, from either side.**
///
/// The wire fields both default, so a sender can omit one and
/// `{"width_px": 1920}` deserialises to `(Some, None)`. `add` is the one
/// funnel every `AddAssetCommand` passes through — importer, HTTP, MCP,
/// the desktop paste — so this is where the pair invariant the DB cannot
/// express is asserted, and the same polarity as `author_kind` /
/// `author_subject`.
///
/// Both cases also have to leave **no row behind**: a refusal that landed
/// the asset first would be a message rather than a gate.
#[tokio::test(flavor = "multi_thread")]
async fn a_half_written_pair_is_refused_and_lands_nothing() {
    let (_tmp, core, persona, corpus) = one_persona("dims-half").await;

    let mut width_only = add_command(&persona, &a_file(&corpus, "width.png"), 1_785_000_000_000);
    width_only.width_px = Some(1920);
    let mut height_only = add_command(&persona, &a_file(&corpus, "height.png"), 1_785_000_001_000);
    height_only.height_px = Some(1080);

    for (what, command, named) in [
        ("width without height", width_only, "height_px"),
        ("height without width", height_only, "width_px"),
    ] {
        let err = core
            .asset_service
            .add(command, &unattributed())
            .await
            .expect_err("half a resolution is not a smaller answer than none");
        match &err {
            DomainError::Validation(msg) => assert!(
                msg.contains(named),
                "{what}: the message names the missing half: {msg}"
            ),
            other => panic!("{what}: expected a Validation error, got {other:?}"),
        }
    }

    // Nothing was written on the way to either refusal.
    let page = core
        .asset_service
        .list(asterism_contract::query::ListAssetsQuery {
            persona_id: Some(persona.clone()),
            ..Default::default()
        })
        .await
        .expect("list");
    assert!(
        page.items.is_empty(),
        "a refused command left a row behind: {:?}",
        page.items.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
}

/// Four rows for the resolution band and the `Pixels` axis.
///
/// | name         | coded pair  | pixels |
/// |--------------|-------------|--------|
/// | `wide`       | 4000 × 1000 |  4.0 M |
/// | `tall`       | 1000 × 4000 |  4.0 M |
/// | `small`      |  800 ×  600 | 0.48 M |
/// | `unmeasured` | —           |    —   |
///
/// `wide` and `tall` are the **same pair transposed**, which is the whole
/// design in one fixture: the two are one rotation apart, the columns
/// hold them differently, and every question this feature answers has to
/// answer them the same. A band over either side alone cannot — `width
/// >= 2000` keeps `wide` and drops `tall`.
async fn a_library_of_four_resolutions(
    tag: &str,
) -> (
    tempfile::TempDir,
    CoreCtx,
    String,
    std::collections::HashMap<&'static str, String>,
) {
    let (tmp, core, persona, corpus) = one_persona(tag).await;
    let plan: [(&'static str, Option<u32>, Option<u32>); 4] = [
        ("wide", Some(4000), Some(1000)),
        ("tall", Some(1000), Some(4000)),
        ("small", Some(800), Some(600)),
        ("unmeasured", None, None),
    ];
    let mut ids = std::collections::HashMap::new();
    for (i, (name, width_px, height_px)) in plan.into_iter().enumerate() {
        let mut command = add_command(
            &persona,
            &a_file(&corpus, &format!("{name}.png")),
            1_785_000_000_000 + i as i64 * 1_000,
        );
        command.width_px = width_px;
        command.height_px = height_px;
        let dto = core
            .asset_service
            .add(command, &unattributed())
            .await
            .expect("add asset");
        ids.insert(name, dto.id);
    }
    (tmp, core, persona, ids)
}

/// Lists a persona under one band and returns the matched names, sorted.
async fn banded(
    core: &CoreCtx,
    persona: &str,
    ids: &std::collections::HashMap<&'static str, String>,
    pixels_min: Option<u64>,
    pixels_max: Option<u64>,
) -> Vec<&'static str> {
    let page = core
        .asset_service
        .list(asterism_contract::query::ListAssetsQuery {
            persona_id: Some(persona.to_string()),
            pixels_min,
            pixels_max,
            ..Default::default()
        })
        .await
        .expect("list");
    let mut names: Vec<&'static str> = page
        .items
        .iter()
        .map(|card| {
            *ids.iter()
                .find(|(_, id)| **id == card.id)
                .map(|(name, _)| name)
                .expect("every listed row is one of the fixtures")
        })
        .collect();
    names.sort_unstable();
    names
}

/// **The band reads the product, and a rotation does not move a row.**
///
/// `wide` and `tall` are the same pixels stored transposed. Every band
/// below holds them together — which is the claim that makes this axis
/// answerable at all, given that the columns are coded dimensions with no
/// orientation applied. A band implemented over either side alone splits
/// the pair on the first case and fails here rather than in a library.
#[tokio::test(flavor = "multi_thread")]
async fn the_resolution_band_reads_the_product_and_ignores_orientation() {
    let (_tmp, core, persona, ids) = a_library_of_four_resolutions("dims-band").await;

    // No band named: everything, including the row nobody measured.
    assert_eq!(
        banded(&core, &persona, &ids, None, None).await,
        vec!["small", "tall", "unmeasured", "wide"],
        "with no band named, the unmeasured row stays — the exclusion is a \
         consequence of asking, not a standing rule"
    );

    // A lower bound alone. Both 4 MP rows clear it despite being stored
    // one rotation apart; `small` and the unmeasured row are out.
    assert_eq!(
        banded(&core, &persona, &ids, Some(1_000_000), None).await,
        vec!["tall", "wide"],
        "a lower bound reads the product, so the transposed pair answers together"
    );

    // An upper bound alone — the direction where folding "unmeasured"
    // into zero would look most reasonable and would sweep the row in.
    assert_eq!(
        banded(&core, &persona, &ids, None, Some(1_000_000)).await,
        vec!["small"],
        "an upper bound excludes the unmeasured row rather than treating it as zero"
    );

    // Inclusive at both ends, on the exact product.
    assert_eq!(
        banded(&core, &persona, &ids, Some(4_000_000), Some(4_000_000)).await,
        vec!["tall", "wide"],
        "the bounds are inclusive and min == max is a one-value band"
    );

    // A closed band that admits nothing is an empty page, not an error:
    // the request is well formed and the library simply has no such row.
    assert!(
        banded(&core, &persona, &ids, Some(5_000_000), Some(9_000_000))
            .await
            .is_empty(),
        "a well-formed band matching nothing answers with an empty page"
    );
}

/// An inverted band is a `400`, not an empty page.
///
/// Same rule the length and size bands follow, and for the same reason:
/// an empty page here would read as "this library holds nothing that
/// large", which is a claim about the corpus rather than about the
/// request.
#[tokio::test(flavor = "multi_thread")]
async fn an_inverted_resolution_band_is_refused() {
    let (_tmp, core, persona, _ids) = a_library_of_four_resolutions("dims-inverted").await;

    let err = core
        .asset_service
        .list(asterism_contract::query::ListAssetsQuery {
            persona_id: Some(persona),
            pixels_min: Some(12_000_000),
            pixels_max: Some(2_000_000),
            ..Default::default()
        })
        .await
        .expect_err("an inverted band must be refused");
    match &err {
        DomainError::Validation(msg) => assert!(
            msg.contains("pixels_min") && msg.contains("pixels_max"),
            "the message names both ends of the band: {msg}"
        ),
        other => panic!("expected a Validation error, got {other:?}"),
    }
}

/// **The card projection carries the count, and carries it as a product.**
///
/// The grid sorts client-side over these rows, so a card that reported
/// `None` here would make the `Pixels` axis compare absent values on
/// every row and answer in the default order while claiming to sort — the
/// exact gap the length and size axes sat in for a wave.
///
/// `wide` and `tall` pin the shape: both must report 4 MP, so a
/// projection that shipped one side, or that swapped the two columns,
/// fails.
#[tokio::test(flavor = "multi_thread")]
async fn the_card_projection_carries_the_pixel_count() {
    let (_tmp, core, persona, ids) = a_library_of_four_resolutions("dims-card").await;

    let page = core
        .asset_service
        .list(asterism_contract::query::ListAssetsQuery {
            persona_id: Some(persona),
            ..Default::default()
        })
        .await
        .expect("list");

    let count_of = |name: &str| -> Option<u64> {
        page.items
            .iter()
            .find(|card| card.id == ids[name])
            .expect("the fixture is listed")
            .pixel_count
    };
    assert_eq!(count_of("wide"), Some(4_000_000));
    assert_eq!(
        count_of("tall"),
        Some(4_000_000),
        "a transposed pair is the same count"
    );
    assert_eq!(count_of("small"), Some(480_000));
    assert_eq!(
        count_of("unmeasured"),
        None,
        "an unmeasured row reports absence, not zero"
    );
}
