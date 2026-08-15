//! End-to-end: the importer states what the bytes hash to, and the two
//! producers of that value have to agree.
//!
//! Its sibling `declared_hash_integrity_e2e` drives the *server* half —
//! it types a declaration into `AddAssetCommand` and checks what the
//! hash job makes of it. That leaves the half this file is about
//! untested: nothing typed the claim in production, an importer computed
//! it, and "the importer's digest" and "the server's digest" are two
//! pieces of code that could drift apart without either side noticing.
//! A fixture that computed the expected value with the same call on both
//! sides would prove nothing at all — so every assertion here compares
//! **two produced values**, and never a produced value against one the
//! test worked out for itself.
//!
//! Both legs are real: the corpus is scanned by `FsScanner`, digested by
//! `run_import`, parsed by `ImageParser`, and POSTed to the actual
//! router over loopback.
//!
//! # The two modes, and why each fixture uses the one it does
//!
//! - `Full` spawns the job worker, so the file really is read and the
//!   claim really is checked. That is what the agreement fixture needs.
//! - `ReadOnly` opens the queue and spawns **nothing**, so no file is
//!   ever read. That is what turns "the server proposed a duplicate
//!   without opening the file" from a race into a fact: in that process
//!   there is no code path that could have opened it.
//!
//! Its own test binary because `init_core` opens a Tantivy index (one
//! core per test binary, as with the sibling e2e files).

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_contract::dto::AssetDetailDto;
use asterism_contract::query::{GetAssetDetailQuery, ListAssetsQuery};
use asterism_core::domain::content_hash;
use asterism_core::domain::content_region;
use asterism_core::domain::duplicate_conflict::DuplicateAxis;
use asterism_core::domain::repository::{AssetRepository, MaterialFingerprint};
use asterism_core::domain::value::{AssetId, OnDuplicate};
use asterism_importer_image::ImageParser;
use asterism_importer_sdk::scanner::sqlite::ColumnMap;
use asterism_importer_sdk::{
    Footprint, FootprintSource, FsScanner, ImportOptions, ImportSummary, Note, ParseError, RawItem,
    ScanMode, SourceParser, SqliteScanner, run_import,
};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// A 1×1 RGBA PNG, 67 bytes — the same fixture shape
/// `dispatch_rein_roundtrip_e2e` uses. Nothing here decodes pixels; the
/// bytes only have to be a file the image parser accepts.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR len + type
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, // bitdepth/colour + CRC
    0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, // IDAT len + type
    0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, //
    0x0D, 0x0A, 0x2D, 0xB4, // IDAT CRC
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND
    0xAE, 0x42, 0x60, 0x82,
];

/// Boots a core plus the real router on an ephemeral loopback port.
/// Bind happens before the spawn so the importer's first request cannot
/// arrive at a closed port.
async fn boot(tmp: &std::path::Path, mode: CoreMode) -> (CoreCtx, u16) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        mode,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (core, port)
}

async fn register(core: &CoreCtx, pack_id: &str) -> String {
    core.persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(pack_id.into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona")
        .id
}

/// Runs the real importer over one directory of PNGs: scan → digest →
/// parse → POST to the router on `port`.
async fn import_png_dir(root: &std::path::Path, persona_id: &str, port: u16) -> ImportSummary {
    let mut options = ImportOptions::new(persona_id);
    options.server = format!("http://127.0.0.1:{port}");
    run_import(
        &FsScanner::new(root).with_extensions(["png"]),
        &ImageParser::new(Some("E2E".into())),
        ScanMode::Enumerate,
        options,
    )
    .await
    .expect("import run")
}

async fn detail_of(core: &CoreCtx, asset_id: &str) -> AssetDetailDto {
    core.asset_service
        .detail(GetAssetDetailQuery {
            asset_id: asset_id.to_string(),
            viewer_subject: None,
        })
        .await
        .expect("read the asset back")
}

/// The asset holding `locator`, by the path the scanner recorded.
async fn asset_id_by_locator(
    core: &CoreCtx,
    persona_id: &str,
    locator: &std::path::Path,
) -> String {
    let wanted = locator.display().to_string();
    let page = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona_id.to_string()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list assets");
    page.items
        .iter()
        .find(|card| card.source_locator == wanted)
        .unwrap_or_else(|| panic!("no asset holds locator {wanted}"))
        .id
        .clone()
}

/// The `_trace.declared_hash` note as the wire carries it, or `None`
/// when the row has no such note.
fn declared_hash_note(detail: &AssetDetailDto) -> Option<serde_json::Value> {
    let extra: serde_json::Value =
        serde_json::from_str(detail.asset.extra_json.as_deref()?).expect("extra is valid JSON");
    extra.get("_trace")?.get("declared_hash").cloned()
}

/// Polls `detail` until `ready` holds.
async fn wait_for(
    core: &CoreCtx,
    asset_id: &str,
    what: &str,
    ready: impl Fn(&AssetDetailDto) -> bool,
) -> AssetDetailDto {
    for _ in 0..120 {
        let detail = detail_of(core, asset_id).await;
        if ready(&detail) {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{what} did not happen within 30s");
}

/// Every open conflict row, as `(newcomer, incumbent, axis, digest)`.
///
/// Read straight out of SQLite over a second isle, the way
/// `duplicate_detection_e2e` reads it: the panel's read verb belongs to
/// the resolution surface, and asserting through a surface that does not
/// exist yet is how a test ends up measuring nothing.
async fn open_conflicts(db_path: &std::path::Path) -> Vec<(String, String, String, String)> {
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(db_path)
        .await
        .expect("second isle");
    let rows = isle
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
    drop(driver);
    rows
}

/// Writes one material's artefact-axis digest by hand, over a second
/// isle.
///
/// The incumbent of a declared-digest proposal has to be holding a
/// digest in its column, and in production the hash job puts it there —
/// which is exactly the pass these fixtures need *not* to run. Seeding
/// it is the standard move in this repository's duplicate tests; what
/// matters is that the **newcomer's** side is untouched, and it is.
///
/// The two walker axes are stamped `not-walked` rather than given
/// invented digests: no container walk happened here, and that marker is
/// precisely "the bytes were never spent".
async fn seed_artefact_digest(db_path: &std::path::Path, asset_id: &str, digest: &str) {
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(db_path)
        .await
        .expect("second isle");
    let assets = asterism_infra::sqlite::repo::SqliteAssetRepository::new(isle);
    let id = AssetId::from_uuid(uuid::Uuid::parse_str(asset_id).expect("asset id is a uuid"));
    assets
        .set_material_fingerprint(
            &id,
            0,
            &MaterialFingerprint {
                file: digest.to_string(),
                content: content_region::NOT_WALKED.to_string(),
                meta: content_region::NOT_WALKED.to_string(),
                meta_kv: None,
                meta_text: None,
                meta_raw: None,
            },
        )
        .await
        .expect("seed the incumbent's fingerprint");
    drop(driver);
}

/// **The two producers agree.**
///
/// One digest is computed by `run_import` over the payload `FsScanner`
/// read into memory; the other by the `material_hash` job, streaming the
/// same file off disk in chunks. Nothing in this fixture computes a
/// third one to compare them against — `verified: true` is the server's
/// own comparison of the two, and the note's `value` is asserted equal
/// to the digest that ended up on the material.
///
/// What makes it non-vacuous is that both sides must exist: if the
/// pipeline stopped declaring, there would be no note and the wait would
/// time out; if the job stopped recomputing, there would be no verdict
/// to wait for.
#[tokio::test(flavor = "multi_thread")]
async fn the_digest_the_importer_declares_is_the_one_the_hash_job_computes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let (core, port) = boot(tmp.path(), CoreMode::Full).await;
    let persona = register(&core, "e2e-declared-importer-agreement").await;

    let summary = import_png_dir(&corpus, &persona, port).await;
    assert_eq!(
        summary,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let asset_id = asset_id_by_locator(&core, &persona, &plate).await;

    // Before the worker gets there the claim is on the row with no
    // verdict. Asserted where it is observable rather than required:
    // under `Full` the job may already have answered.
    if let Some(claim) = declared_hash_note(&detail_of(&core, &asset_id).await) {
        assert_eq!(
            claim["axis"],
            serde_json::json!("artefact"),
            "an importer can only declare the axis it can compute from bytes alone"
        );
    }

    let settled = wait_for(&core, &asset_id, "the declared digest was checked", |d| {
        declared_hash_note(d)
            .and_then(|note| note.get("verified").cloned())
            .is_some()
    })
    .await;

    let note = declared_hash_note(&settled).expect("the verdict is on the row");
    let computed = settled
        .asset
        .content_hash
        .as_deref()
        .expect("the job wrote what it read off the disk");
    assert_eq!(
        note["verified"],
        serde_json::json!(true),
        "the importer's digest and the job's disagreed: {note}"
    );
    assert_eq!(
        note["value"],
        serde_json::json!(computed),
        "the two producers must land on the same characters"
    );
    assert!(
        note.get("got").is_none(),
        "`got` appears only on a mismatch: {note}"
    );

    // A guard against both sides agreeing on nothing: neither producer
    // may have handed over a blank, a marker, or an untagged string.
    let hex = computed
        .strip_prefix(content_hash::DIGEST_PREFIX)
        .expect("the value carries its algorithm");
    assert_eq!(hex.len(), 64);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
}

/// **An exact copy is proposed at ingest, and the server never opened
/// it.**
///
/// `ReadOnly` is what makes the second half a fact rather than a race:
/// the process spawns no job worker, so no code path in it reads a file.
/// The newcomer's own fingerprint columns are still empty when the
/// conflict row exists — the only digest that could have produced that
/// row is the one the importer stated.
///
/// The incumbent's digest is seeded by hand for the same reason: giving
/// it to a worker would put a worker in the process, which is the thing
/// being ruled out.
#[tokio::test(flavor = "multi_thread")]
async fn an_exact_copy_is_proposed_at_ingest_without_the_server_reading_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let first_dir = tmp.path().join("first");
    let second_dir = tmp.path().join("second");
    for dir in [&first_dir, &second_dir] {
        std::fs::create_dir_all(dir).expect("scan dir");
    }
    // Two directories, one set of bytes: the copy has to be a different
    // *address* or the server answers it from the source lookup and
    // never reaches a digest at all.
    let original = first_dir.join("plate.png");
    let copy = second_dir.join("plate-copy.png");
    std::fs::write(&original, PNG_1X1).expect("write original");
    std::fs::write(&copy, PNG_1X1).expect("write copy");

    let db_path = tmp.path().join("asterism.db");
    let (core, port) = boot(tmp.path(), CoreMode::ReadOnly).await;
    let persona = register(&core, "e2e-declared-importer-proposal").await;

    // Leg 1: the incumbent arrives through the real importer.
    assert_eq!(
        import_png_dir(&first_dir, &persona, port).await,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let incumbent_id = asset_id_by_locator(&core, &persona, &original).await;
    let incumbent = detail_of(&core, &incumbent_id).await;
    assert!(
        incumbent.asset.content_hash.is_none(),
        "no worker runs in this process, so nothing has read a file"
    );
    let declared = declared_hash_note(&incumbent)
        .expect("the importer declared a digest for a whole file it read")["value"]
        .as_str()
        .expect("the claim is a string")
        .to_string();

    // What the hash job would have written, written by the test. The
    // newcomer's side is left exactly as the ingest left it.
    seed_artefact_digest(&db_path, &incumbent_id, &declared).await;
    assert!(
        open_conflicts(&db_path).await.is_empty(),
        "one holder is not a duplicate — the seeding itself must not raise anything"
    );

    // Leg 2: the copy, same importer, different directory.
    assert_eq!(
        import_png_dir(&second_dir, &persona, port).await,
        ImportSummary {
            imported: 1,
            failed: 0
        },
        "a duplicate is a finding, not a failed import"
    );
    let newcomer_id = asset_id_by_locator(&core, &persona, &copy).await;
    assert_ne!(newcomer_id, incumbent_id, "two addresses, two rows");

    // `add` proposes inline, so the row is there the moment the import
    // returned — no polling, which is itself part of the claim.
    let conflicts = open_conflicts(&db_path).await;
    assert_eq!(conflicts.len(), 1, "one pair, one question: {conflicts:?}");
    let (newcomer, incumbent_of, axis, digest) = &conflicts[0];
    assert_eq!(newcomer, &newcomer_id, "the copy is the newcomer");
    assert_eq!(
        incumbent_of, &incumbent_id,
        "the older row is the incumbent"
    );
    assert_eq!(
        axis,
        DuplicateAxis::Artefact.as_str(),
        "an importer can state every byte of a file and nothing else"
    );
    assert_eq!(digest, &declared);

    // The load-bearing half. Nothing hashed the newcomer — there is no
    // worker in this process to have done so — so the proposal above
    // cannot have come from a measurement.
    let newcomer_detail = detail_of(&core, &newcomer_id).await;
    assert!(
        newcomer_detail.asset.content_hash.is_none(),
        "the server proposed without opening the file, or this fixture is \
         measuring the ordinary ingest path instead"
    );

    // Proposed, not folded: both rows are still in the grid.
    let listed = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona.clone()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list");
    for id in [&incumbent_id, &newcomer_id] {
        assert!(
            listed.items.iter().any(|card| &card.id == id),
            "{id} left the grid; a claim nobody checked folded a row"
        );
    }
}

/// **A lane that asked to fold gets a question anyway.**
///
/// The fixture disagrees with the default on the axis under test:
/// `on_duplicate = fold` is declared, so under
/// [`DetectionOrigin::Ingest`] there would be **no conflict row at all**
/// — the pair would go straight to an `AssetFold`. The row's existence
/// is therefore the assertion, not decoration on one.
///
/// It goes through `asset_service.add` rather than the importer because
/// `spec_to_command` pins `on_duplicate` to `None` on purpose: what to
/// do about a duplicate is a policy about a run, and the lane layer that
/// would carry it does not exist yet. Driving the importer here would
/// assert `ask` against `ask` and measure nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_declared_digest_never_folds_even_when_the_lane_asked_for_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let original = corpus.join("plate.png");
    let copy = corpus.join("plate-copy.png");
    std::fs::write(&original, PNG_1X1).expect("write original");
    std::fs::write(&copy, PNG_1X1).expect("write copy");

    let db_path = tmp.path().join("asterism.db");
    let (core, _port) = boot(tmp.path(), CoreMode::ReadOnly).await;
    let persona = register(&core, "e2e-declared-importer-fold-guard").await;

    let digest = asterism_contract::digest::of_bytes(PNG_1X1);
    let incumbent = core
        .asset_service
        .add(
            add_command(&persona, original.to_str().unwrap(), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add the original");
    seed_artefact_digest(&db_path, &incumbent.id, &digest).await;

    let mut command = add_command(&persona, copy.to_str().unwrap(), 1_785_000_001_000);
    command.declared_content_hash = Some(digest.clone());
    command.on_duplicate = Some(asterism_contract::command::OnDuplicate::Fold);
    let newcomer = core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("a declaration is never a reason to refuse the file");

    let conflicts = open_conflicts(&db_path).await;
    assert_eq!(
        conflicts.len(),
        1,
        "a lane asking to fold on an unverified claim is answered with a \
         question; an empty queue here means it was folded instead: {conflicts:?}"
    );
    assert_eq!(conflicts[0].0, newcomer.id);
    assert_eq!(conflicts[0].1, incumbent.id);
    assert_eq!(conflicts[0].2, DuplicateAxis::Artefact.as_str());

    // And the declaration really was `fold` — otherwise the queue row
    // above is the ordinary `ask` and this fixture asserts nothing.
    let stored = detail_of(&core, &newcomer.id).await;
    assert_eq!(
        stored.asset.on_duplicate.as_deref(),
        Some(OnDuplicate::Fold.as_str()),
        "the row must hold the strategy that was supposed to be overruled"
    );
    // Both rows are still live: a fold would have made one a headstone.
    let listed = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona.clone()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list");
    for id in [&incumbent.id, &newcomer.id] {
        assert!(
            listed.items.iter().any(|card| &card.id == id),
            "{id} folded"
        );
    }
}

/// One row of a SQLite source, mapped the way the persona-journal
/// importer maps its own: the payload is a column, and the locator is
/// `<db>#<id>` — an address with no bytes behind it.
struct RowNoteParser;

impl SourceParser for RowNoteParser {
    fn parse(&self, raw: RawItem) -> Result<Vec<Footprint>, ParseError> {
        Ok(vec![Footprint::Note(Note {
            source: FootprintSource {
                kind: raw.source_kind,
                locator: raw.locator,
                platform: None,
                external_id: None,
            },
            occurred_at: raw.occurred_at.unwrap_or_else(chrono::Utc::now),
            body: String::from_utf8_lossy(&raw.payload).into_owned(),
            source_app: None,
            labels: Vec::new(),
            bundle_id: None,
            extra: serde_json::json!({}),
        })])
    }
}

/// **A source with no payload of its own declares nothing — and that is
/// what lets it ingest at all.**
///
/// The assertion is the `ImportSummary`. The server refuses a
/// declaration on a locator it can never read back ("nothing would ever
/// check it"), so a pipeline that digested a database column and
/// attached it to `<db>#<id>` would not merely record something wrong —
/// every row would come back `failed`. A clean summary is the negative
/// stated positively.
///
/// The filesystem import in the same test is the positive control. Same
/// runner, same server, same call: one declares because its scanner
/// handed over a whole file, the other does not because its scanner
/// handed over a value out of a row.
#[tokio::test(flavor = "multi_thread")]
async fn a_source_with_no_payload_declares_nothing_and_ingests_anyway() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let plate = corpus.join("plate.png");
    std::fs::write(&plate, PNG_1X1).expect("write plate");

    let source_db = tmp.path().join("source.sqlite");
    {
        let conn = rusqlite::Connection::open(&source_db).expect("open source db");
        conn.execute("CREATE TABLE entries (id TEXT PRIMARY KEY, body TEXT)", [])
            .expect("create entries");
        conn.execute(
            "INSERT INTO entries (id, body) VALUES ('e-1', 'a note kept in a database')",
            [],
        )
        .expect("insert entry");
    }

    let (core, port) = boot(tmp.path(), CoreMode::ReadOnly).await;
    let persona = register(&core, "e2e-declared-importer-no-payload").await;

    // Positive control first: the pipeline demonstrably can declare.
    assert_eq!(
        import_png_dir(&corpus, &persona, port).await,
        ImportSummary {
            imported: 1,
            failed: 0
        }
    );
    let file_id = asset_id_by_locator(&core, &persona, &plate).await;
    assert!(
        declared_hash_note(&detail_of(&core, &file_id).await).is_some(),
        "a whole file read off disk is exactly the case a digest is stated for"
    );

    let mut options = ImportOptions::new(&persona);
    options.server = format!("http://127.0.0.1:{port}");
    let summary = run_import(
        &SqliteScanner::new(
            &source_db,
            "SELECT id, body FROM entries",
            ColumnMap::new("id", "body"),
        )
        .with_source_kind("e2e-rows"),
        &RowNoteParser,
        ScanMode::Enumerate,
        options,
    )
    .await
    .expect("import run");
    assert_eq!(
        summary,
        ImportSummary {
            imported: 1,
            failed: 0
        },
        "a declaration on `<db>#<id>` would have been refused, and the row \
         would be counted here as a failure"
    );

    // The card renders a record's locator as its **container**
    // (`SourceLocator::to_display`), so the join key is the database
    // path rather than the `<db>#e-1` spelling the importer sent. One
    // row was imported, so it is unambiguous.
    let row_locator = source_db.display().to_string();
    let page = core
        .asset_service
        .list(ListAssetsQuery {
            persona_id: Some(persona.clone()),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list");
    let row_id = page
        .items
        .iter()
        .find(|card| card.source_locator == row_locator)
        .unwrap_or_else(|| panic!("no asset holds locator {row_locator}"))
        .id
        .clone();
    assert!(
        declared_hash_note(&detail_of(&core, &row_id).await).is_none(),
        "a value lifted out of a container is not the bytes at an address, \
         and stating a digest for it would be a claim nothing can check"
    );
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
        album_meta: Default::default(),
        declared_content_hash: None,
    }
}
