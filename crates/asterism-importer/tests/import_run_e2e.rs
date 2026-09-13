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
//! The cost is the dev-dependency below it: this crate is a thin CLI
//! that otherwise builds in seconds, and testing it now builds the
//! server. That is the price of a test that spans two processes, and
//! the alternative — pointing at a path under `target/` and hoping —
//! fails on exactly the machine where nobody has built the binary yet.
//! No cycle: the server does not depend on this crate.

use std::sync::Arc;

use asterism_contract::command::{
    DefineImportCommand, RegisterPersonaCommand, RunImportDefinitionCommand,
};
use asterism_core::domain::attribution::AttributionContext;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;

/// A caller that states nothing, which records nothing.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// Boots a core, serves it, and tells the launcher where that is.
///
/// The last step is the one worth naming: the address does not exist
/// until a listener is bound, so `core_init` hands out an empty cell and
/// whoever binds fills it. A test that forgot would get "this server has
/// not finished starting" rather than a mystery.
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
    core.import_api_base
        .set(format!("http://127.0.0.1:{port}"))
        .expect("nothing else has bound this core");
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
///
/// `CARGO_BIN_EXE_<name>` is cargo's own answer to "where did you put
/// the binary", and using it rather than guessing a target directory is
/// what keeps this working under a workspace, a custom `--target-dir`,
/// and whatever a CI runner does.
fn importer_binary() -> &'static str {
    env!("CARGO_BIN_EXE_asterism-import")
}

/// A directory of text files for the importer to read.
fn corpus(dir: &std::path::Path, names: &[&str]) {
    std::fs::create_dir_all(dir).expect("corpus dir");
    for name in names {
        std::fs::write(dir.join(name), name.as_bytes()).expect("write");
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
///
/// So the phases run in sequence, each named where it starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_stored_import_runs_recording_what_happened_each_way() {
    a_stored_import_runs_and_its_records_land().await;
    a_credential_that_is_not_set_is_said_plainly().await;
    // Last, because it empties `$PATH` and points `$ASTERISM_IMPORT` at
    // nothing — a state nothing after it could run in.
    a_missing_binary_is_recorded_as_the_machine_s_problem().await;
}

/// **A stored import runs, and its records land in the persona it
/// names.**
///
/// The criterion this slice exists for. Nobody types a command line:
/// the definition is stored once, and running it spawns the binary,
/// waits for it, and records what it did.
async fn a_stored_import_runs_and_its_records_land() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("notes");
    corpus(&dir, &["a.txt", "b.txt", "c.txt"]);

    // SAFETY: this test binary is the only thing reading it, and it is
    // set before the server that reads it is asked to launch anything.
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let (core, _port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run").await;

    let defined = core
        .import_run_service
        .define(
            DefineImportCommand {
                persona_id: persona.clone(),
                name: "notes".into(),
                subcommand: "text".into(),
                args: vec!["--dir".into(), dir.display().to_string()],
                secret_ref: None,
            },
            &unattributed(),
        )
        .await
        .expect("a definition");

    let run = core
        .import_run_service
        .run(
            RunImportDefinitionCommand {
                id: defined.id.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("the run");

    assert_eq!(
        (run.outcome.as_str(), run.imported, run.failed),
        ("ok", 3, 0),
        "three files, and the importer's own count survived into the record: {:?}",
        run.ended_by_message
    );
    assert!(run.ended_at.is_some(), "a finished run has an end");

    // And the second run imports nothing, because #295's position was
    // kept by the child and found by the next child. Two processes
    // agreeing through the server is the whole of transport (iii).
    let again = core
        .import_run_service
        .run(
            RunImportDefinitionCommand { id: defined.id },
            &unattributed(),
        )
        .await
        .expect("the second run");
    assert_eq!(
        (again.outcome.as_str(), again.imported),
        ("ok", 0),
        "the files were handled by the first run and are not read again"
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

    let (core, _port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-missing").await;

    let defined = core
        .import_run_service
        .define(
            DefineImportCommand {
                persona_id: persona,
                name: "nowhere".into(),
                subcommand: "text".into(),
                args: vec!["--dir".into(), tmp.path().display().to_string()],
                secret_ref: None,
            },
            &unattributed(),
        )
        .await
        .expect("a definition");

    let run = core
        .import_run_service
        .run(
            RunImportDefinitionCommand { id: defined.id },
            &unattributed(),
        )
        .await
        .expect("a run that could not start is still a run that is recorded");

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

/// A definition naming a credential that is not set is refused before
/// anything runs, and says which variable.
///
/// Running anyway would reach the source unauthenticated and record
/// whatever that looks like — a 401 read as a configuration problem,
/// which it is, but not the one that is actually wrong.
async fn a_credential_that_is_not_set_is_said_plainly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("ASTERISM_IMPORT", importer_binary()) };

    let (core, _port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-import-run-secret").await;

    let defined = core
        .import_run_service
        .define(
            DefineImportCommand {
                persona_id: persona,
                name: "needs-a-token".into(),
                subcommand: "text".into(),
                args: vec!["--dir".into(), tmp.path().display().to_string()],
                secret_ref: Some("A_VARIABLE_NOBODY_SET".into()),
            },
            &unattributed(),
        )
        .await
        .expect("a definition");

    // The stored definition holds the name and could not hold a value:
    // there is no field for one. Asserted because it is the constraint
    // this whole slice is built around.
    let stored = core
        .import_run_service
        .list()
        .await
        .expect("a read")
        .into_iter()
        .find(|d| d.id == defined.id)
        .expect("the definition just stored");
    assert_eq!(stored.secret_ref.as_deref(), Some("A_VARIABLE_NOBODY_SET"));

    let run = core
        .import_run_service
        .run(
            RunImportDefinitionCommand { id: defined.id },
            &unattributed(),
        )
        .await
        .expect("a run that could not start is still recorded");

    assert_eq!(run.outcome, "unstarted");
    assert!(
        run.ended_by_message
            .as_deref()
            .unwrap_or_default()
            .contains("A_VARIABLE_NOBODY_SET"),
        "it names the variable an operator has to set: {:?}",
        run.ended_by_message
    );
}
