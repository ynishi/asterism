//! End-to-end: an import keeps its own position, and the next run over
//! an unchanged source imports nothing.
//!
//! The criterion #295 exists for, and the one no unit test can answer.
//! The store has unit tests over a fake repository, the runner has them
//! over an in-memory store, and both would keep passing if the two
//! halves never met: a partition spelled one way going out and another
//! coming back, a `null` the client read as an error, a route wired to
//! nothing. What this file drives is the whole chain — `FsScanner` over
//! real files, `run_import_with` over `HttpSyncStore`, the actual
//! router, the actual SQLite table — with nobody typing a resumption
//! point at any stage.
//!
//! `ReadOnly` throughout: this is about what the importer and the store
//! say to each other, and a job worker reading files would add a second
//! process to a question that does not involve one.
//!
//! Its own test binary because `init_core` opens a Tantivy index (one
//! core per test binary, as with the sibling e2e files).

use std::sync::Arc;

use asterism_contract::command::RegisterPersonaCommand;
use asterism_importer_sdk::{
    ApiClient, Footprint, FootprintSource, FsScanner, HttpSyncStore, ImportOptions, Note,
    ParseError, RawItem, Resume, ScanMode, SourceParser, SourceScanner, StateKey, SyncStore,
    run_import_with,
};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;

/// A caller that states nothing, which records nothing.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

async fn boot(tmp: &std::path::Path) -> (CoreCtx, u16) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
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

/// One note per text file — the smallest parser that makes a file into
/// something the server will store.
struct NoteParser;

impl SourceParser for NoteParser {
    fn parse(&self, raw: RawItem) -> Result<Vec<Footprint>, ParseError> {
        Ok(vec![Footprint::Note(Note {
            source: FootprintSource {
                kind: raw.source_kind,
                locator: raw.locator,
                platform: None,
                external_id: None,
            },
            occurred_at: raw.occurred_at.unwrap_or_else(chrono::Utc::now),
            occurred_source: Default::default(),
            body: String::from_utf8(raw.payload).expect("the fixture writes text"),
            source_app: None,
            labels: vec![],
            bundle_id: None,
            extra: raw.extra,
        })])
    }
}

fn corpus(dir: &std::path::Path, names: &[&str]) {
    std::fs::create_dir_all(dir).expect("corpus dir");
    for name in names {
        std::fs::write(dir.join(name), name.as_bytes()).expect("write");
    }
}

fn options(persona: &str, port: u16) -> ImportOptions {
    let mut options = ImportOptions::new(persona);
    options.server = format!("http://127.0.0.1:{port}");
    options
}

/// **A second run over an unchanged source imports nothing.**
///
/// Nobody types a resumption point. The first run earns one and puts it
/// where the server keeps it; the second finds it and takes up after
/// it, so the three files it has already handled never reach a parser,
/// a digest or a POST.
///
/// A third leg adds a file, because "imports nothing" on its own is
/// also what a broken import looks like: the run that follows has to
/// pick up the new file and only the new file, or resumption has simply
/// turned the importer off.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_run_over_an_unchanged_source_imports_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("notes");
    corpus(&dir, &["a.txt", "b.txt", "c.txt"]);

    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-state").await;
    let scanner = FsScanner::new(&dir).with_extensions(["txt"]);
    let store = HttpSyncStore::new(ApiClient::new(format!("http://127.0.0.1:{port}")));

    let first = run_import_with(
        &scanner,
        &NoteParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("the first run");
    assert_eq!((first.imported, first.failed), (3, 0));

    let second = run_import_with(
        &scanner,
        &NoteParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("the second run");
    assert_eq!(
        (second.imported, second.failed),
        (0, 0),
        "the three files were handled by the first run and are not read again"
    );

    // And the importer is still an importer.
    std::fs::write(dir.join("d.txt"), b"d.txt").expect("write");
    let third = run_import_with(
        &scanner,
        &NoteParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("the third run");
    assert_eq!(
        (third.imported, third.failed),
        (1, 0),
        "the file added since is imported, and it alone"
    );
}

/// The position is filed under the scanner's own partition, and reading
/// it back gives the same offset text that went out.
///
/// The round trip is the assertion. This workspace builds `serde_json`
/// with `preserve_order`, so an offset that came back with its keys
/// reordered would be the same value and different text — and the
/// adapter that wrote it is the only thing entitled to read it, byte
/// for byte.
#[tokio::test(flavor = "multi_thread")]
async fn the_stored_offset_survives_the_round_trip_unchanged() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("notes");
    corpus(&dir, &["a.txt"]);

    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-state-offset").await;
    let scanner = FsScanner::new(&dir).with_extensions(["txt"]);
    let store = HttpSyncStore::new(ApiClient::new(format!("http://127.0.0.1:{port}")));

    let summary = run_import_with(
        &scanner,
        &NoteParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("the run");
    let earned = summary.resume_from.expect("a clean run earns a point");

    let key = StateKey::new(
        &persona,
        SourceScanner::partition(&scanner).expect("a walk has a partition"),
    );
    let stored = store
        .read(&key)
        .await
        .expect("reading it back")
        .expect("the point the run just stored");
    assert_eq!(
        stored, earned,
        "the partition it was filed under is the scanner's own, and the \
         offset is the text that went out"
    );
}

/// A key nothing has written answers "nothing", over the wire.
///
/// The first-run case, and the one a status code would get wrong: a
/// client that read a `404` as a failure would refuse to start an
/// import that has simply never run before.
#[tokio::test(flavor = "multi_thread")]
async fn a_source_nobody_has_imported_answers_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-state-absent").await;
    let store = HttpSyncStore::new(ApiClient::new(format!("http://127.0.0.1:{port}")));

    let got = store
        .read(&StateKey::new(&persona, "kind=fs|root=/nowhere|ext="))
        .await
        .expect("absence is an answer, not a failure");
    assert!(got.is_none());
}

/// `--no-resume`'s rule, end to end: the whole source is read again and
/// the stored position is left where it was.
#[tokio::test(flavor = "multi_thread")]
async fn a_forced_full_read_leaves_the_stored_position_where_it_was() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("notes");
    corpus(&dir, &["a.txt", "b.txt"]);

    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-state-full-read").await;
    let scanner = FsScanner::new(&dir).with_extensions(["txt"]);
    let store = HttpSyncStore::new(ApiClient::new(format!("http://127.0.0.1:{port}")));
    let key = StateKey::new(
        &persona,
        SourceScanner::partition(&scanner).expect("a walk has a partition"),
    );

    run_import_with(
        &scanner,
        &NoteParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("the first run");
    let after_first = store.read(&key).await.expect("a read").expect("a point");

    // The same two files, read again on purpose. They land on the
    // locators they already have, so the server answers them as it
    // answers any re-import; what this asserts is that they were *read*
    // — the run saw two records rather than taking up past them.
    let mut forced = options(&persona, port);
    forced.resume = Resume::No;
    let again = run_import_with(
        &scanner,
        &NoteParser,
        ScanMode::Enumerate,
        forced,
        Some(&store),
    )
    .await
    .expect("the forced run");
    assert_eq!(
        again.imported + again.failed,
        2,
        "both files were read rather than skipped"
    );

    assert_eq!(
        store.read(&key).await.expect("a read"),
        Some(after_first),
        "and the position the next ordinary run will use is untouched"
    );
}
