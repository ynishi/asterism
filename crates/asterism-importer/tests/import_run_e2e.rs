//! End-to-end: a stored import runs without anybody typing it.
//!
//! The real `asterism-import` binary, spawned by the real service,
//! against a real server, over a source the test stands up. Nothing
//! here fakes the launcher — the whole point of this slice is the seam
//! between two processes, and a test that stubbed it would leave that
//! seam untested while looking thorough.
//!
//! ## Why it lives in the importer's crate and not the server's
//!
//! `CARGO_BIN_EXE_asterism-import` is defined only for tests of the
//! crate that declares that binary, and it is the only way to be told
//! where cargo put it rather than to guess from a layout cargo does not
//! promise. Having it also means cargo *builds* the binary for this
//! test, so the seam cannot be exercised against something stale or
//! absent.
//!
//! The cost is the dev-dependency: this crate is a thin CLI that
//! otherwise builds in seconds, and testing it now builds the server.
//! That is the price of a test that spans two processes. No cycle: the
//! server does not depend on this crate.
//!
//! ## What this test does not have to do any more
//!
//! An earlier shape bound a listener and then handed the server's
//! address to the launcher itself, so that a spawned child could be
//! told where to post. It passed, and every shipped binary left that
//! address unset — a run in the product could never have started. A
//! test performing wiring the product does not is a test that proves
//! the wrong thing.
//!
//! Nothing is wired now: the child resolves its own server the way an
//! operator's shell does, and a server on a port of its own is named in
//! the definition's arguments, which is what this test does below.

use std::sync::Arc;

use asterism_contract::command::{
    DefineImportCommand, RegisterPersonaCommand, RunImportDefinitionCommand,
};
use asterism_contract::dto::ImportRunDto;
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

/// Waits for a run to stop saying `running`, and answers what it became.
///
/// The shape the API now has: starting answers immediately with the run
/// it opened, and the outcome lands on that row. A caller that wanted
/// to block would do this, which is why the polling is here and not in
/// the service.
async fn settled(core: &CoreCtx, definition_id: &str, run_id: &str) -> ImportRunDto {
    for _ in 0..300 {
        let runs = core
            .import_run_service
            .runs(definition_id, 20)
            .await
            .expect("a read");
        if let Some(run) = runs.iter().find(|r| r.id == run_id)
            && run.outcome != "running"
        {
            return run.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("run {run_id} never finished");
}

/// A definition pointed at this test's own server.
///
/// `--server` in the arguments, because the child resolves the active
/// profile's port by default and this server is on whatever the OS
/// handed out. This is exactly what an operator with a server on a
/// non-default port writes, which is the point: there is no wiring step
/// for anybody to forget, only an argument that is either right or
/// visibly wrong.
fn define(persona: &str, name: &str, port: u16, mut args: Vec<String>) -> DefineImportCommand {
    args.extend(["--server".into(), format!("http://127.0.0.1:{port}")]);
    DefineImportCommand {
        persona_id: persona.into(),
        name: name.into(),
        subcommand: "text".into(),
        args,
        secret_ref: None,
        secret_header: None,
    }
}

/// A loopback source that answers however the test says, after however
/// long the test says, and remembers what it was sent.
async fn spawn_source(status: u16, delay: std::time::Duration) -> (u16, Heard) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let heard = Heard::default();
    let log = heard.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
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
                for line in request.lines() {
                    if let Some(value) = line
                        .strip_prefix("authorization: ")
                        .or_else(|| line.strip_prefix("Authorization: "))
                    {
                        log.0
                            .lock()
                            .expect("the source's log")
                            .push(value.trim().to_string());
                    }
                }
                tokio::time::sleep(delay).await;
                let body = r#"{"data":{"items":[]},"paging":{"next":null}}"#;
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (port, heard)
}

/// Every `Authorization` value the source was sent.
#[derive(Clone, Default)]
struct Heard(Arc<std::sync::Mutex<Vec<String>>>);

impl Heard {
    fn values(&self) -> Vec<String> {
        self.0.lock().expect("the source's log").clone()
    }
}

/// An `http` import pointed at the fake source and at this server.
fn http_definition(persona: &str, name: &str, source: u16, port: u16) -> DefineImportCommand {
    DefineImportCommand {
        persona_id: persona.into(),
        name: name.into(),
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
    }
}

/// Everything this slice promises, in one test and in order.
///
/// **One** test because the launcher reads the process's environment —
/// which binary to run, and the credential — and cargo runs tests
/// concurrently inside one process. Three separate tests each setting
/// `$ASTERISM_IMPORT` raced, and the one that wanted it *absent* found
/// the path another had just set. A lock would not help: the launcher
/// reads the variable without taking one, which is correct for the
/// launcher and fatal for the test suite.
#[tokio::test(flavor = "multi_thread")]
async fn a_stored_import_runs_recording_what_happened_each_way() {
    a_stored_import_runs_and_its_records_land().await;
    a_resolved_credential_reaches_the_child_and_nothing_else().await;
    a_second_run_of_one_import_is_refused_while_the_first_is_going().await;
    a_credential_with_nowhere_to_go_is_refused_when_it_is_defined().await;
    a_credential_that_is_not_set_is_said_plainly().await;
    // Last, because it empties `$PATH` and points `$ASTERISM_IMPORT` at
    // nothing — a state nothing after it could run in.
    a_missing_binary_is_recorded_as_the_machine_s_problem().await;
}

/// **A stored import runs, and its records land in the persona it
/// names.**
///
/// The criterion this slice exists for. Nobody types a command line:
/// the definition is stored once, and starting it spawns the binary and
/// records what it did on the run it opened.
async fn a_stored_import_runs_and_its_records_land() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("notes");
    corpus(&dir, &["a.txt", "b.txt", "c.txt"]);

    // SAFETY: the phases of this test run in sequence, and nothing else
    // in this process reads these.
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run").await;

    let defined = core
        .import_run_service
        .define(
            define(
                &persona,
                "notes",
                port,
                vec!["--dir".into(), dir.display().to_string()],
            ),
            &unattributed(),
        )
        .await
        .expect("a definition");

    let opened = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("the run starts");
    assert_eq!(
        opened.outcome, "running",
        "starting answers with the run it opened, not with what it will become"
    );

    let run = settled(&core, &defined.id, &opened.id).await;
    assert_eq!(
        (run.outcome.as_str(), run.imported, run.failed),
        ("ok", 3, 0),
        "three files, and the importer's own count survived into the record: {:?}",
        run.ended_by_message
    );
    assert!(run.ended_at.is_some(), "a finished run has an end");

    // The records are in the persona the definition named, read back
    // from the server rather than taken from the importer's word for
    // it.
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
        "the three files are assets of the persona the definition named"
    );

    // And a second run imports nothing, because #295's position was
    // kept by the child and found by the next child. Two processes
    // agreeing through the server is the whole of transport (iii).
    let again = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("the second run starts");
    let again = settled(&core, &defined.id, &again.id).await;
    assert_eq!(
        (again.outcome.as_str(), again.imported),
        ("ok", 0),
        "the files were handled by the first run and are not read again"
    );
}

/// **The credential reaches the child and appears nowhere else.**
///
/// The constraint the whole slice is built around, and the half a
/// definition holding only a name does not prove on its own: the
/// launcher records the child's stderr tail, that tail becomes
/// `ended_by_message`, and that is handed back by the route listing
/// runs. The importer writes whole URLs into its failure messages, so a
/// credential that had gone anywhere near an argument or a URL would
/// surface exactly there.
///
/// So this runs an import that really resolves a secret, against a
/// source that really refuses it, and then looks for the value in
/// everything the run left behind.
async fn a_resolved_credential_reaches_the_child_and_nothing_else() {
    const VALUE: &str = "s3cret-value-no-other-string-looks-like";

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };
    unsafe { std::env::set_var("A_TOKEN_FOR_THIS_TEST", VALUE) };

    let (source, heard) = spawn_source(401, std::time::Duration::ZERO).await;
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-secret-reaches").await;

    let mut command = http_definition(&persona, "guarded", source, port);
    command.secret_ref = Some("A_TOKEN_FOR_THIS_TEST".into());
    command.secret_header = Some("Authorization".into());
    let defined = core
        .import_run_service
        .define(command, &unattributed())
        .await
        .expect("a definition");

    let opened = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("the run starts");
    let run = settled(&core, &defined.id, &opened.id).await;

    assert_eq!(
        heard.values(),
        vec![VALUE.to_string()],
        "the child sent the resolved value, so this test is about a real \
         credential and not an absent one"
    );
    assert_eq!(
        run.outcome, "failed",
        "a 401 is the configuration's, and the run is recorded as having failed"
    );
    assert_eq!(run.ended_by_class.as_deref(), Some("config"));

    // Now the part that matters: it is in none of what was kept.
    let stored = core
        .import_run_service
        .list()
        .await
        .expect("a read")
        .into_iter()
        .find(|d| d.id == defined.id)
        .expect("the definition just stored");
    let definition_text = format!("{stored:?}");
    assert!(
        !definition_text.contains(VALUE),
        "the definition holds the variable's name and never its value: {definition_text}"
    );
    assert_eq!(stored.secret_ref.as_deref(), Some("A_TOKEN_FOR_THIS_TEST"));
    assert_eq!(stored.secret_header.as_deref(), Some("Authorization"));

    let runs = core
        .import_run_service
        .runs(&defined.id, 10)
        .await
        .expect("a read");
    let runs_text = format!("{runs:?}");
    assert!(
        !runs_text.contains(VALUE),
        "and nothing the run recorded carries it either — this is the assertion \
         the stderr tail makes necessary: {runs_text}"
    );
}

/// **A second run of one import is refused while the first is going.**
///
/// Two importers over one source would each take up from a position the
/// other is about to move. The first run is held open by a source that
/// answers slowly, which is what makes the window real rather than
/// theoretical.
async fn a_second_run_of_one_import_is_refused_while_the_first_is_going() {
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let (source, _) = spawn_source(200, std::time::Duration::from_secs(3)).await;
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-overlap").await;

    let defined = core
        .import_run_service
        .define(
            http_definition(&persona, "slow", source, port),
            &unattributed(),
        )
        .await
        .expect("a definition");

    let opened = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("the first run starts");

    let refused = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect_err("a second run of one import is refused while the first is going");
    let said = refused.to_string();
    assert!(
        said.contains("slow"),
        "and says which import it was: {said}"
    );

    settled(&core, &defined.id, &opened.id).await;

    // Once it is done, the definition can be run again — `Blocked`
    // means the same request works after something else changes, and
    // this is that something.
    core.import_run_service
        .run(
            RunImportDefinitionCommand { id: defined.id },
            &unattributed(),
        )
        .await
        .expect("the slot was released when the first run finished");
}

/// A credential named with nowhere to go is refused when the import is
/// defined, not discovered when it runs.
///
/// The pair exists so this can be a type-level question rather than a
/// search of somebody else's command line for a flag.
async fn a_credential_with_nowhere_to_go_is_refused_when_it_is_defined() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-half-pair").await;

    let mut command = define(&persona, "half", port, vec!["--dir".into(), "/tmp".into()]);
    command.secret_ref = Some("SOME_VARIABLE".into());
    let err = core
        .import_run_service
        .define(command, &unattributed())
        .await
        .expect_err("half a credential is not a credential");
    assert!(
        err.to_string().contains("secret_header"),
        "and it says which half is missing: {err}"
    );
}

/// A definition naming a credential that is not set is refused before
/// anything runs, and says which variable.
async fn a_credential_that_is_not_set_is_said_plainly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let (source, _) = spawn_source(200, std::time::Duration::ZERO).await;
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-secret").await;

    let mut command = http_definition(&persona, "needs-a-token", source, port);
    command.secret_ref = Some("A_VARIABLE_NOBODY_SET".into());
    command.secret_header = Some("Authorization".into());
    let defined = core
        .import_run_service
        .define(command, &unattributed())
        .await
        .expect("a definition");

    let opened = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("the run starts");
    let run = settled(&core, &defined.id, &opened.id).await;

    assert_eq!(run.outcome, "unstarted");
    assert_eq!(
        run.ended_by_class, None,
        "nothing ran, so nothing classified anything"
    );
    assert!(
        run.ended_by_message
            .as_deref()
            .unwrap_or_default()
            .contains("A_VARIABLE_NOBODY_SET"),
        "it names the variable an operator has to set: {:?}",
        run.ended_by_message
    );
}

/// A definition whose binary is not there records a run that says so,
/// and says where it looked.
///
/// `Unstarted` and not `Failed`: the two ask different things of
/// whoever reads the record. This one means the machine, and no amount
/// of looking at the source will explain it.
async fn a_missing_binary_is_recorded_as_the_machine_s_problem() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // SAFETY: as above. Pointed at a path that is not a file, with no
    // sibling and nothing of this name on PATH inside the test's
    // environment — so every rung is tried and every rung fails.
    unsafe { std::env::set_var("ASTERISM_IMPORT", tmp.path().join("not-here")) };
    unsafe { std::env::set_var("PATH", "") };

    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-missing").await;

    let defined = core
        .import_run_service
        .define(
            define(
                &persona,
                "nowhere",
                port,
                vec!["--dir".into(), tmp.path().display().to_string()],
            ),
            &unattributed(),
        )
        .await
        .expect("a definition");

    let opened = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("a run that cannot start is still a run that is opened");
    let run = settled(&core, &defined.id, &opened.id).await;

    assert_eq!(run.outcome, "unstarted");
    let said = run.ended_by_message.expect("it says what happened");
    for rung in ["ASTERISM_IMPORT", "beside this executable", "PATH"] {
        assert!(
            said.contains(rung),
            "the message names every rung it tried, so an operator knows where to \
             put the binary — {rung} is missing from: {said}"
        );
    }
}
