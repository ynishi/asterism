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
//! `asterism_core::application::DEFAULT_TICK`; the interval is a minute
//! because the first phase's closing assertion is that a definition
//! already run is *not* started again by the forty ticks that follow.

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

/// Both halves of what a schedule is, in order.
///
/// One test because the launcher reads this process's environment for
/// which binary to run, and cargo runs the tests in a binary
/// concurrently — the sibling file learned that the hard way and says
/// so at length.
#[tokio::test(flavor = "multi_thread")]
async fn what_an_interval_starts_and_what_a_source_can_say_about_it() {
    an_interval_starts_an_import_and_an_absent_one_does_not().await;
    a_wait_a_source_stated_reaches_the_run_record().await;
}

/// **An import runs because its interval said so, and one that has no
/// interval does not.**
///
/// The criterion #302 exists for, and the one thing a person cannot
/// observe by reading code: nobody in this test asks for a run.
async fn an_interval_starts_an_import_and_an_absent_one_does_not() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scheduled_dir = tmp.path().join("scheduled");
    let manual_dir = tmp.path().join("manual");
    corpus(&scheduled_dir, &["a.txt", "b.txt", "c.txt"]);
    corpus(&manual_dir, &["d.txt"]);

    // SAFETY: the phases of this test run in sequence, and nothing else
    // in this process reads or writes this.
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

    // Refused at the door as well as by the column, and this is the
    // one that can say it in words.
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

/// A loopback source that refuses with 429 and says how long to wait.
///
/// A raw socket rather than a second axum router beside `boot`'s,
/// because this end never reads the request: whatever arrives, the
/// answer is 429 with a `Retry-After`, which is the one thing a real
/// source will not say on demand, and one function is the whole of it.
async fn spawn_rate_limited_source(retry_after_secs: u64) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = String::new();
                let mut buf = vec![0u8; 8 * 1024];
                loop {
                    let Ok(n) = socket.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if request.contains("\r\n\r\n") {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 429 Too Many Requests\r\n\
                     Retry-After: {retry_after_secs}\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    port
}

/// **A wait the source stated survives two processes and lands on the
/// run.**
///
/// The seam this slice claims to close, and the one no test crossed:
/// #297 made `RateLimited` reachable and #299 recorded
/// `retry_after_secs`, and until now every assertion about that column
/// was made against a row built by hand. Here a real source answers a
/// real 429 with a real `Retry-After`, a real child reads it, writes a
/// report, and this end puts it on the record.
///
/// What `due` then does with the number is pinned next to `due` —
/// `a_stated_wait_holds_the_next_start_back` and
/// `a_short_wait_does_not_shorten_the_interval` in
/// `asterism-infra`'s `import_definition` tests. Splitting it there is
/// deliberate: an hour's wait cannot be waited out here, and due-ness is
/// a pure function of the row this phase proves gets written.
async fn a_wait_a_source_stated_reaches_the_run_record() {
    const STATED: u64 = 3600;

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let source = spawn_rate_limited_source(STATED).await;
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-schedule-429").await;

    let limited = core
        .import_run_service
        .define(
            DefineImportCommand {
                persona_id: persona.clone(),
                name: "rate-limited".into(),
                subcommand: "http".into(),
                args: vec![
                    "--url".into(),
                    format!("http://127.0.0.1:{source}/records"),
                    "--items-path".into(),
                    "data.items".into(),
                    "--cursor-path".into(),
                    "paging.next".into(),
                    "--server".into(),
                    format!("http://127.0.0.1:{port}"),
                ],
                secret_ref: None,
                secret_header: None,
                every_minutes: Some(1),
            },
            &unattributed(),
        )
        .await
        .expect("a definition");

    let _timer = ImportSchedule::spawn(core.import_run_service.clone(), Duration::from_millis(50));

    let run = settled_run(&core, &limited.id).await;
    assert_eq!(
        run.outcome, "failed",
        "a refusal is a failed run: {:?}",
        run.ended_by_message
    );
    assert_eq!(
        run.ended_by_class.as_deref(),
        Some("rate_limited"),
        "and the class the source's 429 earns is its own, not `source` or \
         `transient`: {:?}",
        run.ended_by_message
    );
    // `due` reads the pair — a wait is measured from the end of the run
    // that earned it — so a row carrying the seconds and no end would be
    // a row `due` ignores. It holds by construction (`run` sets both in
    // one write) and is asserted anyway, because "by construction" is
    // what this phase exists to stop taking on trust.
    assert!(
        run.ended_at.is_some(),
        "and the run has an end for the wait to be measured from"
    );
    assert_eq!(
        run.retry_after_secs,
        Some(STATED),
        "and the seconds the source stated are on the row, which is the whole \
         of what #297 recorded and nothing read"
    );
}
