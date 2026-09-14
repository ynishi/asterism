//! End-to-end: an import that runs because its interval said so.
//!
//! The real `asterism-import` binary, spawned by the real service,
//! started by the real timer, against a real server. Nothing here fakes
//! the clock or the launcher — what #302 claims is that an archive
//! keeps filling with nobody watching, and a test that drove
//! `start_due` by hand would leave the part that does the watching
//! untested while looking thorough.
//!
//! ## Its own test binary
//!
//! `import_run_e2e.rs` explains why its phases are one test: the
//! launcher reads this process's environment, and cargo runs tests
//! inside one binary concurrently. A second *binary* is a second
//! process, so this file gets an environment of its own — which is why
//! the schedule lives here rather than as another phase over there.
//!
//! ## The two numbers
//!
//! The timer's tick and the definition's interval are separate, and the
//! test needs them far apart to say anything. The tick here is
//! milliseconds because a test cannot wait out
//! `ImportSchedule::DEFAULT_TICK`; the interval is a minute because the
//! point of the second half is that a definition already run is *not*
//! started again by the forty ticks that follow.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{DefineImportCommand, RegisterPersonaCommand};
use asterism_contract::dto::ImportRunDto;
use asterism_core::application::ImportSchedule;
use asterism_core::domain::attribution::AttributionContext;
use asterism_server::core_init::{CoreCtx, JobWorker, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;

/// A caller that states nothing, which records nothing.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

async fn boot(tmp: &std::path::Path) -> (CoreCtx, u16) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        // No worker: nothing here enqueues anything it then waits on,
        // and a worker reading files would put a second process's worth
        // of activity behind assertions about one importer.
        JobWorker::None,
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

/// The importer built alongside this test.
fn importer_binary() -> &'static str {
    env!("CARGO_BIN_EXE_asterism-import")
}

fn corpus(dir: &std::path::Path, names: &[&str]) {
    std::fs::create_dir_all(dir).expect("corpus dir");
    for name in names {
        std::fs::write(dir.join(name), name.as_bytes()).expect("write");
    }
}

/// A definition over `dir`, pointed at this test's own server.
///
/// `--server` in the arguments because this server is on whatever port
/// the OS handed out, which is the case the launcher's profile lookup
/// cannot answer and a definition's own argument is for.
fn define(
    persona: &str,
    name: &str,
    dir: &std::path::Path,
    port: u16,
    every_minutes: Option<u32>,
) -> DefineImportCommand {
    DefineImportCommand {
        persona_id: persona.into(),
        name: name.into(),
        subcommand: "text".into(),
        args: vec![
            "--dir".into(),
            dir.display().to_string(),
            "--server".into(),
            format!("http://127.0.0.1:{port}"),
        ],
        secret_ref: None,
        secret_header: None,
        every_minutes,
    }
}

/// Waits for one import to have a run that is over, and answers it.
async fn settled_run(core: &CoreCtx, definition_id: &str) -> ImportRunDto {
    for _ in 0..300 {
        let runs = core
            .import_run_service
            .runs(definition_id, 20)
            .await
            .expect("a read");
        if let Some(run) = runs.first()
            && run.outcome != "running"
        {
            return run.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("nothing ever ran {definition_id}");
}

async fn run_count(core: &CoreCtx, definition_id: &str) -> usize {
    core.import_run_service
        .runs(definition_id, 50)
        .await
        .expect("a read")
        .len()
}

/// **An import runs because its interval said so, and one that has no
/// interval does not.**
///
/// The criterion #302 exists for, and the one thing a person cannot
/// observe by reading code: nobody in this test asks for a run.
#[tokio::test(flavor = "multi_thread")]
async fn an_interval_starts_an_import_and_an_absent_one_does_not() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scheduled_dir = tmp.path().join("scheduled");
    let manual_dir = tmp.path().join("manual");
    corpus(&scheduled_dir, &["a.txt", "b.txt", "c.txt"]);
    corpus(&manual_dir, &["d.txt"]);

    // SAFETY: this test binary runs one test, so nothing else in this
    // process reads or writes this.
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-schedule").await;

    let scheduled = core
        .import_run_service
        .define(
            define(&persona, "every-minute", &scheduled_dir, port, Some(1)),
            &unattributed(),
        )
        .await
        .expect("a scheduled definition");
    let manual = core
        .import_run_service
        .define(
            define(&persona, "by-hand", &manual_dir, port, None),
            &unattributed(),
        )
        .await
        .expect("a manual definition");

    // An interval of zero is a typo, and reading it as "manual" would
    // answer one by doing nothing for as long as nobody looked.
    let refused = core
        .import_run_service
        .define(
            define(&persona, "never", &manual_dir, port, Some(0)),
            &unattributed(),
        )
        .await
        .expect_err("zero minutes is not a schedule");
    assert!(
        refused.to_string().contains("zero minutes"),
        "and it says so in the words the person typed: {refused}"
    );

    // Nothing below asks for a run. This is the only line that could
    // cause one, and it starts the same function the desktop starts.
    let _timer = ImportSchedule::spawn(core.import_run_service.clone(), Duration::from_millis(50));

    let run = settled_run(&core, &scheduled.id).await;
    assert_eq!(
        (run.outcome.as_str(), run.imported, run.failed),
        ("ok", 3, 0),
        "the timer started it and the importer's own count survived: {:?}",
        run.ended_by_message
    );

    // The records are in the persona the definition named — read back
    // from the server rather than taken from the importer's word.
    let listed = core
        .asset_service
        .list(asterism_contract::query::ListAssetsQuery {
            persona_id: Some(persona.clone()),
            limit: 50,
            ..Default::default()
        })
        .await
        .expect("a listing");
    assert_eq!(
        listed.items.len(),
        3,
        "three files, and not the fourth: the manual definition's corpus is \
         one file and nothing started it"
    );

    // Forty more ticks. The scheduled import is not due again for a
    // minute, so a timer that treated every tick as an occasion — or
    // that owed a run per window it had ever missed — would show here.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        run_count(&core, &scheduled.id).await,
        1,
        "one run, not one per tick"
    );
    assert_eq!(
        run_count(&core, &manual.id).await,
        0,
        "and a definition with no interval is nobody's to start but a person's"
    );
}
