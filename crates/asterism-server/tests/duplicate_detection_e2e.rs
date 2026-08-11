//! End-to-end: two copies of the same file are reported as duplicates.
//!
//! The unit tests around this feature each hold one piece — the hash
//! function, the SQL that groups on it, the scan that finds unhashed
//! rows — and every one of them sets the hash by hand. What none of
//! them exercises is the path that actually has to work: ingest
//! enqueues `material_hash`, the worker reads the file off disk, the
//! digest lands on the material, and the report finds the pair.
//!
//! `Full` mode is what makes this real: it takes the writer lock and
//! spawns the job worker, so the enqueued jobs run instead of sitting
//! in the queue (the same reason `search_filter_e2e` uses it).

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_contract::dto::DuplicateAxis;
use asterism_contract::query::GetAssetDetailQuery;
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about content hashing, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// A PNG the content walker accepts: signature, then
/// `length || type || payload || CRC` per chunk.
///
/// The CRCs are zero — the walker reads past them without checking, and
/// its own doc says why (a wrong CRC is a fact the file axis already
/// distinguishes) — so the fixture stays a few lines. `text` is the
/// metadata chunk that makes two files of one picture.
fn png(pixels: &[u8], text: Option<&[u8]>) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&[0u8; 4]);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    chunk(&mut out, b"IDAT", pixels);
    if let Some(text) = text {
        chunk(&mut out, b"tEXt", text);
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
async fn identical_originals_surface_as_one_duplicate_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(corpus.join("inbox")).expect("inbox dir");
    std::fs::create_dir_all(corpus.join("archive")).expect("archive dir");

    // The realistic shape of the problem: the same bytes filed twice
    // under different paths, plus an unrelated file as the control.
    let bytes = b"the same photograph, byte for byte\n";
    let original = corpus.join("inbox/a.png");
    let copy = corpus.join("archive/a.png");
    let other = corpus.join("inbox/b.png");
    std::fs::write(&original, bytes).expect("write original");
    std::fs::write(&copy, bytes).expect("write copy");
    std::fs::write(&other, b"a different photograph\n").expect("write other");

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
                pack_id: Some("e2e-duplicates".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for (index, path) in [&original, &copy, &other].into_iter().enumerate() {
        let dto = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    1_785_000_000_000 + index as i64 * 1_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(dto.id);
    }

    // Hashing is asynchronous (enqueued on add, the worker reads the
    // file). Poll rather than sleep so a slow machine does not flake.
    let mut report = None;
    for _ in 0..120 {
        let candidate = core
            .asset_service
            .list_duplicate_groups(None, None, None)
            .await
            .expect("duplicate report");
        if !candidate.groups.is_empty() {
            report = Some(candidate);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let report = report.expect("the duplicate pair is reported within 30s");

    assert_eq!(report.groups.len(), 1, "one pair, one group");
    let group = &report.groups[0];
    assert!(
        group.content_hash.starts_with("sha256:"),
        "the group is keyed by a real digest, not a marker: {}",
        group.content_hash
    );
    let members: std::collections::HashSet<&str> =
        group.members.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        members,
        std::collections::HashSet::from([ids[0].as_str(), ids[1].as_str()]),
        "the two copies, and not the unrelated file"
    );

    // The same digest has to be answerable one asset at a time, not
    // only as a group key: an agent deciding what to do about a
    // duplicate asks about a single asset. The mapping is unit-tested,
    // but what it reads is `asset.materials`, and whether the detail
    // read hydrates those is a property of the repository — so this
    // asserts the whole path (hash job → material row → hydration →
    // wire) rather than the last step of it.
    let detail = core
        .asset_service
        .detail(GetAssetDetailQuery {
            asset_id: ids[0].clone(),
            viewer_subject: None,
        })
        .await
        .expect("detail of the first copy");
    assert_eq!(
        detail.asset.content_hash.as_deref(),
        Some(group.content_hash.as_str()),
        "the detail payload reports the digest the report grouped on"
    );

    // Everything on this disk is readable, so the walk finishes: an
    // empty report here would mean "no duplicates", not "still
    // looking". That distinction is the whole point of the field.
    for _ in 0..120 {
        let candidate = core
            .asset_service
            .list_duplicate_groups(None, None, None)
            .await
            .expect("duplicate report");
        if candidate.unhashed_count == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("unhashed_count never reached zero on a fully readable corpus");
}

/// The finding this whole wave exists for, end to end: two exports of
/// one picture that differ only in a metadata chunk are **one** group on
/// the content axis and **two** rows on the file axis.
///
/// That shape is measured, not imagined — a 4,601-image ComfyUI corpus
/// holds 9 such groups, whose pixel bytes are byte-identical and whose
/// files differ because the workflow blob records where the canvas
/// happened to be sitting. Every one of them is invisible to the report
/// as it stood before this subtask.
///
/// It runs through the real chain rather than by writing digests in:
/// ingest enqueues `material_hash`, the worker reads the file, the
/// walker decides which bytes are the picture, both columns are written
/// in one statement, and the report groups on the column the caller
/// named. A fixture that set the fingerprints by hand would assert the
/// SQL and nothing else — and the SQL is the half that already had a
/// test.
#[tokio::test(flavor = "multi_thread")]
async fn one_picture_in_two_files_is_a_group_on_the_content_axis_only() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    // The measured shape: identical pixels, one file carrying a
    // workflow blob the other does not.
    let pixels = b"a compressed stream, near enough for a walker";
    let bare = corpus.join("run-1.png");
    let noted = corpus.join("run-1-again.png");
    std::fs::write(&bare, png(pixels, None)).expect("write the bare export");
    std::fs::write(
        &noted,
        png(
            pixels,
            Some(b"workflow\0{\"extra\":{\"ds\":{\"scale\":0.87}}}"),
        ),
    )
    .expect("write the annotated export");

    // …and a pair that is byte-identical, which both axes have to
    // agree about. Without it, "the content axis found more" could be
    // read as "the content axis found something else".
    let twin_pixels = b"a different picture, also compressed";
    let twin_a = corpus.join("run-2.png");
    let twin_b = corpus.join("archive/run-2.png");
    std::fs::create_dir_all(corpus.join("archive")).expect("archive dir");
    std::fs::write(&twin_a, png(twin_pixels, None)).expect("write the original");
    std::fs::write(&twin_b, png(twin_pixels, None)).expect("write the copy");

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
                pack_id: Some("e2e-content-axis".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for (index, path) in [&bare, &noted, &twin_a, &twin_b].into_iter().enumerate() {
        let dto = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    1_785_000_000_000 + index as i64 * 1_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(dto.id);
    }

    // Both axes are polled together: the interesting state is the one
    // where the walk has finished, and either axis alone can be
    // momentarily right for the wrong reason while the worker is still
    // going.
    let mut settled = None;
    for _ in 0..120 {
        let file = core
            .asset_service
            .list_duplicate_groups(None, Some("artefact"), None)
            .await
            .expect("artefact-axis report");
        let content = core
            .asset_service
            .list_duplicate_groups(None, Some("content"), None)
            .await
            .expect("content-axis report");
        if file.unhashed_count == 0 && content.groups.len() == 2 {
            settled = Some((file, content));
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let (file, content) = settled.expect("both axes answer within 30s");

    // The artefact axis sees the copy and not the pair of exports —
    // which is the behaviour this build already had, and has to keep.
    assert_eq!(file.groups.len(), 1, "one byte-identical pair");
    assert_eq!(file.groups[0].axis, DuplicateAxis::Artefact);
    assert!(
        file.groups[0].content_hash.starts_with("sha256:"),
        "keyed by a whole-file digest: {}",
        file.groups[0].content_hash
    );
    assert_eq!(
        file.groups[0]
            .members
            .iter()
            .map(|m| m.id.as_str())
            .collect::<std::collections::HashSet<_>>(),
        std::collections::HashSet::from([ids[2].as_str(), ids[3].as_str()]),
        "the two exports of one picture are two files, and stay two"
    );

    // The content axis sees both — the copy, and the pair the file axis
    // cannot reach.
    for group in &content.groups {
        assert_eq!(group.axis, DuplicateAxis::Content);
        assert!(
            group.content_hash.starts_with("cr1-sha256:"),
            "keyed by a region digest: {}",
            group.content_hash
        );
    }
    let members: Vec<std::collections::HashSet<&str>> = content
        .groups
        .iter()
        .map(|g| g.members.iter().map(|m| m.id.as_str()).collect())
        .collect();
    let exports = std::collections::HashSet::from([ids[0].as_str(), ids[1].as_str()]);
    let copies = std::collections::HashSet::from([ids[2].as_str(), ids[3].as_str()]);
    assert!(
        members.contains(&exports),
        "the metadata-only difference is one picture: {members:?}"
    );
    assert!(
        members.contains(&copies),
        "and byte-identical files agree on both axes: {members:?}"
    );

    // The count that keeps an empty content report honest. Everything
    // here was imported after the column existed, so nothing carries the
    // migration's marker — which is what makes a non-zero reading
    // elsewhere mean what it says.
    assert_eq!(
        content.unwalked_count, 0,
        "a library built after this wave owes the content axis nothing"
    );
    assert_eq!(
        file.unwalked_count, content.unwalked_count,
        "the backlog is a fact about the disk, so it reads the same from either axis"
    );

    // An axis nobody computes is refused rather than answered as the
    // default. Reading a typo as `file` would hand back a report about
    // a question that was not asked.
    let err = core
        .asset_service
        .list_duplicate_groups(None, Some("perceptual"), None)
        .await
        .expect_err("an unknown axis is not a spelling of the default");
    assert!(
        err.to_string().contains("perceptual"),
        "the refusal names what was asked for: {err}"
    );
}

/// The same wave, one question further on: registering the second copy
/// has to leave something a person can answer.
///
/// The report above is a query — it would keep finding the pair with
/// detection deleted, because it groups on the hash rather than on
/// anything detection wrote. This asserts what the fingerprint *caused*:
/// an `identical_to` edge oriented newcomer → incumbent, and one row on
/// the conflict queue. Neither exists unless the hash job asked the
/// lookup what its digest meant.
///
/// The queue is read straight out of SQLite over a second isle: the
/// panel's read verb belongs to the resolution surface, which is a
/// later wave, and asserting through a surface that does not exist yet
/// is how a test ends up measuring nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_second_copy_leaves_a_question_and_a_recorded_match() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let bytes = b"the same photograph, byte for byte\n";
    let original = corpus.join("original.png");
    let copy = corpus.join("copy.png");
    let unrelated = corpus.join("other.png");
    std::fs::write(&original, bytes).expect("write original");
    std::fs::write(&copy, bytes).expect("write copy");
    std::fs::write(&unrelated, b"a different photograph\n").expect("write other");

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
                pack_id: Some("e2e-conflicts".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // The incumbent is registered and fingerprinted *first*, and that
    // is asserted before the copy arrives: against a fixture where both
    // land at once, an assertion about the queue could pass on a
    // detection that fired from either side.
    let incumbent = core
        .asset_service
        .add(
            add_command(&persona.id, original.to_str().unwrap(), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add the original");
    let mut hashed = false;
    for _ in 0..120 {
        let detail = core
            .asset_service
            .detail(GetAssetDetailQuery {
                asset_id: incumbent.id.clone(),
                viewer_subject: None,
            })
            .await
            .expect("detail of the original");
        if detail.asset.content_hash.is_some() {
            hashed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(hashed, "the incumbent was fingerprinted within 30s");

    // An unrelated file in the same wave: whatever the queue ends up
    // holding, it is not "everything that was imported".
    core.asset_service
        .add(
            add_command(&persona.id, unrelated.to_str().unwrap(), 1_785_000_001_000),
            &unattributed(),
        )
        .await
        .expect("add the unrelated file");

    let newcomer = core
        .asset_service
        .add(
            add_command(&persona.id, copy.to_str().unwrap(), 1_785_000_002_000),
            &unattributed(),
        )
        .await
        .expect("add the copy");

    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(&db_path)
        .await
        .expect("second isle");

    let mut queued: Vec<(String, String, String, String)> = Vec::new();
    for _ in 0..120 {
        queued = isle
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT newcomer_id, incumbent_id, axis, content_hash \
                       FROM duplicate_conflict WHERE resolved_at IS NULL",
                )?;
                stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, uuid::Uuid>(0)?.to_string(),
                        r.get::<_, uuid::Uuid>(1)?.to_string(),
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .expect("read the conflict queue");
        if !queued.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    assert_eq!(queued.len(), 1, "one pair, one open question");
    let (newcomer_id, incumbent_id, axis, hash) = &queued[0];
    assert_eq!(newcomer_id, &newcomer.id, "the arrival raised it");
    assert_eq!(incumbent_id, &incumbent.id, "against the row already there");
    assert_eq!(axis, "artefact", "these two agree on every byte");
    assert!(
        hash.starts_with("sha256:"),
        "the question carries the digest it is about: {hash}"
    );

    // …and the fact itself, oriented the way the edge kind fixes.
    let edges: Vec<(String, String, Option<String>)> = isle
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT from_asset, to_asset, label FROM edge WHERE kind = 'identical_to'",
            )?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, uuid::Uuid>(0)?.to_string(),
                    r.get::<_, uuid::Uuid>(1)?.to_string(),
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<_, _>>()
        })
        .await
        .expect("read the edges");
    assert_eq!(
        edges,
        vec![(
            newcomer.id.clone(),
            incumbent.id.clone(),
            Some("artefact".to_string())
        )],
        "one edge, newcomer → incumbent, labelled with the axis"
    );

    driver.shutdown().await.ok();
}
