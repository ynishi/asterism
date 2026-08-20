//! Where a PNG's `tEXt` chunks end up now that they are not rows.
//!
//! The importer used to read them and emit one `Footprint::Note` per
//! chunk, addressed `<image>.png#<keyword>`. It does not any more, and
//! nothing
//! replaced it on that side — no field on the wire, no declaration in
//! the command. The claim this binary tests is that the chunks reach
//! the image's own row anyway, because the server reads them off the
//! artefact's bytes in the `material_hash` job.
//!
//! It runs through the real chain rather than writing fingerprints in:
//! `add` enqueues the job, the worker opens the file, `material_meta`
//! walks it, and both meta columns land in the same statement. Writing
//! them by hand would assert the repository and nothing about whether
//! an ordinary import ever gets there.
//!
//! `Full` mode is what makes that real — it takes the writer lock and
//! spawns the job worker, so the enqueued job runs. The read side goes
//! through a second isle over the same file, because `meta_kv` is a
//! material column with no wire surface of its own yet; the same
//! arrangement `dispatch_copy_fold_e2e` documents.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_core::domain::axis_status::AxisStatus;
use asterism_core::domain::content_hash::META_DIGEST_PREFIX;
use asterism_core::domain::material::Material;
use asterism_core::domain::repository::AssetRepository;
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::domain::value::AssetId;
use asterism_infra::sqlite;
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. This is about what the bytes carry,
/// not about who ingested them.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// A PNG the walkers accept: signature, then
/// `length || type || payload || CRC` per chunk.
///
/// CRCs are zero — both walkers read past them without checking, and
/// their docs say why (a wrong CRC is a fact the artefact axis already
/// distinguishes). `text` is a list so a fixture can carry the several
/// chunks a real export does.
fn png(pixels: &[u8], text: &[(&str, &str)]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&[0u8; 4]);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    chunk(&mut out, b"IDAT", pixels);
    for (keyword, value) in text {
        let mut payload = keyword.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(value.as_bytes());
        chunk(&mut out, b"tEXt", &payload);
    }
    chunk(&mut out, b"IEND", &[]);
    out
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

#[tokio::test(flavor = "multi_thread")]
async fn a_pngs_text_chunks_land_on_the_image_row_as_its_meta_axis() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    // The shape a ComfyUI export has: two chunks, one of them a JSON
    // document. Both keywords were ones the deleted importer path
    // turned into their own asset rows.
    let workflow = r#"{"nodes":[{"class":"KSampler"}]}"#;
    let annotated = corpus.join("run-1.png");
    std::fs::write(
        &annotated,
        png(
            b"a compressed stream, near enough for a walker",
            &[("prompt", "1girl, purple eyes"), ("workflow", workflow)],
        ),
    )
    .expect("write the annotated export");

    // The control, and the reason this fixture measures anything: the
    // same pixels with no chunks at all. If the meta column were
    // filled by something other than a reading of the chunks — a
    // constant, the artefact digest, `{}` — both rows would carry the
    // same value and the assertions below would pass on a walker that
    // never looked.
    let bare = corpus.join("run-2.png");
    std::fs::write(
        &bare,
        png(b"a compressed stream, near enough for a walker", &[]),
    )
    .expect("write the bare export");

    let db_path = tmp.path().join("asterism.db");
    let core = init_core_with(
        &db_path,
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
                pack_id: Some("e2e-png-text-meta".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for (index, path) in [&annotated, &bare].into_iter().enumerate() {
        let dto = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().expect("utf-8 path"),
                    1_785_000_000_000 + index as i64 * 1_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(AssetId::from_uuid(dto.id.parse().expect("asset uuid")));
    }

    // The read side. A second isle over the same file: `meta_kv` is a
    // material column and no query exposes it yet.
    let (isle, driver) = sqlite::open_and_migrate(&db_path)
        .await
        .expect("second isle");
    // Dropping the driver joins the SQLite thread and takes the isle
    // with it, so it has to outlive every call below.
    std::mem::forget(driver);
    let assets = sqlite::repo::SqliteAssetRepository::new(isle.clone());

    // Hashing is asynchronous. Poll for the state where the worker has
    // answered both rows, so neither assertion reads a half-finished
    // walk as a verdict.
    let mut settled = None;
    for _ in 0..120 {
        let annotated_row = assets.find(&ids[0]).await.expect("find").expect("row");
        let bare_row = assets.find(&ids[1]).await.expect("find").expect("row");
        if primary(&annotated_row.materials).meta_hash_status != AxisStatus::Pending
            && primary(&bare_row.materials).meta_hash_status != AxisStatus::Pending
        {
            settled = Some((annotated_row, bare_row));
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let (annotated_row, bare_row) = settled.expect("the hash job answers both rows within 30s");

    // ---- the metadata is on the image's own row ----------------------

    let material = primary(&annotated_row.materials);
    let digest = material.meta_hash.as_deref().expect("polled for Some");
    assert!(
        digest.starts_with(META_DIGEST_PREFIX),
        "a real digest, not a marker: {digest}"
    );

    let fields = material
        .meta_fields()
        .expect("a digest travels with the object it was taken over");
    assert_eq!(
        fields.get("prompt").map(String::as_str),
        Some("1girl, purple eyes"),
        "the chunk keywords are the image row's metadata: {fields:?}"
    );
    assert_eq!(
        fields.get("workflow").map(String::as_str),
        Some(workflow),
        "and the value is the container's text, unparsed"
    );

    // The same fact through the accessor a consumer holding an `Asset`
    // reads, so this is not only true of the column.
    assert_eq!(
        annotated_row
            .material_meta()
            .and_then(|fields| fields.get("prompt").cloned()),
        Some("1girl, purple eyes".to_string())
    );

    // ---- the control disagrees ---------------------------------------

    let bare_material = primary(&bare_row.materials);
    assert_eq!(
        (
            bare_material.meta_hash_status,
            bare_material.meta_hash.as_deref()
        ),
        (AxisStatus::EmptySpan, None),
        "a PNG with no chunks is walked and found to carry nothing — \
         so the digest above is a reading of the chunks, not of the file"
    );
    assert_eq!(
        bare_material.meta_kv, None,
        "no object, because the container carried none"
    );

    // ---- and the row is addressed as a whole file --------------------

    for row in [&annotated_row, &bare_row] {
        assert!(
            matches!(row.source.locator, SourceLocator::File(_)),
            "an imported PNG addresses a file, not a record inside one: {:?}",
            row.source.locator
        );
        assert!(
            !row.source.locator.to_display().contains('#'),
            "and nothing spelled a fragment into it: {}",
            row.source.locator.to_display()
        );
    }
}

/// The primary original. `ord == 0` for the reason the card projection
/// reads its `mime` there: a secondary original is a different artefact
/// carrying its own metadata.
fn primary(materials: &[Material]) -> &Material {
    materials
        .iter()
        .find(|m| m.ord == 0)
        .expect("an imported file has a primary material")
}
