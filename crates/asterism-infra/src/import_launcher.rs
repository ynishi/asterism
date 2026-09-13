//! Starting `asterism-import`, and the two things that is really about.
//!
//! The [`ImportLauncher`] port's implementation, and the only place an
//! importer is spawned. Everything above it is arranged so that this
//! file is the only one on the import path that holds a credential or
//! knows a process exists — the job handlers spawn `ffmpeg` on their
//! own account, which is a different path with the same rule.
//!
//! # Where the binary is
//!
//! Three rungs: an explicit `$ASTERISM_IMPORT`, then a sibling of the
//! running executable, then `PATH`. The first three of the ffmpeg
//! sidecar's four — it also probes fixed install prefixes, which is
//! right for a thing people install with a package manager and wrong
//! for one shipped beside the server. They differ at the first rung
//! too: ffmpeg's override is terminal, and this one falls through, so
//! an override pointing at nothing is still a run that finds the
//! binary.
//!
//! The middle rung is the one worth naming — the comment beside
//! `thumb_ffmpeg`'s ladder records what skipping it cost there, "it
//! reported `ffmpeg is required` on exactly the machines the sidecar
//! exists for", and a bundled importer beside a bundled server is the
//! identical shape. A failure names every rung it tried, because "not
//! found" without the list is a message an operator cannot act on.
//!
//! # Where the credential is, and where it is not
//!
//! The definition names an environment variable; this reads it and puts
//! the value into the **child's environment**, under a fixed name the
//! importer knows. It is therefore in: this process's environment, the
//! child's environment. It is not in: the definition, the run record,
//! the child's argument vector, or any message this file produces.
//!
//! That last exclusion is why this diverges from the outbound side's
//! `{{secret}}` template, which renders a credential into the thing it
//! is building. There, the thing is an HTTP header inside one process.
//! Here it would be an argument vector, and an argument vector is
//! readable by every other process on the machine — `ps` is not a
//! privilege. So the value goes through the environment instead, and
//! what `ps` shows is the *name* of a header.
//!
//! Loading a `.env` is the binary's job, done once at startup, for the
//! reason the outbound side gives: an adapter that went looking for
//! dotenv files itself would make "which file did this credential come
//! from" invisible to the definition that named it.
//!
//! # Where the records go, and why nothing here says
//!
//! Nothing passes `--server`. The child resolves it the way the
//! operator's shell does — the active profile's port — and it inherits
//! `$ASTERISM_PROFILE` from this process, so it resolves the same
//! profile this one is serving. A server on a port of its own puts
//! `--server` in the definition's arguments, which is what a person
//! running the importer by hand does already.
//!
//! The first shape told the child instead, which meant this file had to
//! be given an address, which meant something had to hand it one after
//! binding a listener — an `Arc<OnceLock<String>>` filled by whoever
//! served. Nothing filled it. Every shipped binary called `axum::serve`
//! directly, the cell stayed empty, and every run in the product would
//! have answered "this server has not finished starting" forever. The
//! end-to-end test passed because the *test* filled it. A wiring step
//! that can be forgotten is one that will be, and the way to not forget
//! it turned out to be not having one.

use std::path::PathBuf;
use std::process::Stdio;

use asterism_contract::import_report::ImportReport;
use asterism_core::application::{ImportLauncher, LaunchOutcome, LaunchSpec};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use tokio::io::AsyncReadExt;

/// Environment variable naming an explicit importer binary.
const BINARY_OVERRIDE: &str = "ASTERISM_IMPORT";

/// The importer's file name, looked for beside this executable and on
/// `PATH`.
const BINARY_NAME: &str = "asterism-import";

/// The variable the child reads its credential out of.
///
/// A fixed name rather than the definition's own, so that the importer
/// does not have to be told which variable to look in and the value
/// cannot be selected by anything the caller writes into an argument.
pub const CHILD_SECRET_VAR: &str = "ASTERISM_IMPORT_SECRET";

/// How much of the child's stderr is kept for a run that left no report.
///
/// A tail rather than the whole stream: a run that failed on its
/// thousandth page has a thousand progress lines in front of the thing
/// that went wrong, and the record is read by a person.
const STDERR_TAIL: usize = 4 * 1024;

/// Spawns `asterism-import` and waits for it.
#[derive(Default)]
pub struct SubprocessImportLauncher;

impl SubprocessImportLauncher {
    /// A launcher. There is nothing to configure.
    pub fn new() -> Self {
        Self
    }
}

/// The importer binary, or the rungs that were tried.
///
/// Returns the list rather than logging it, so the failure a run records
/// says where to put the binary instead of only that there wasn't one.
fn find_binary() -> Result<PathBuf, Vec<String>> {
    let mut tried = Vec::new();

    if let Some(explicit) = std::env::var_os(BINARY_OVERRIDE) {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(path);
        }
        tried.push(format!("${BINARY_OVERRIDE} = {}", path.display()));
    } else {
        tried.push(format!("${BINARY_OVERRIDE} (unset)"));
    }

    match std::env::current_exe().ok().and_then(|exe| {
        let beside = exe.parent()?.join(BINARY_NAME);
        beside.is_file().then_some(beside)
    }) {
        Some(beside) => return Ok(beside),
        None => tried.push(format!("beside this executable ({BINARY_NAME})")),
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(BINARY_NAME);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    tried.push(format!("$PATH ({BINARY_NAME})"));

    Err(tried)
}

#[async_trait]
impl ImportLauncher for SubprocessImportLauncher {
    async fn launch(&self, spec: LaunchSpec) -> Result<LaunchOutcome, DomainError> {
        let binary = match find_binary() {
            Ok(path) => path,
            Err(tried) => {
                return Ok(LaunchOutcome {
                    report: None,
                    detail: format!("no importer binary; tried {}", tried.join(", ")),
                    started: false,
                });
            }
        };

        // A file rather than a stream, because it has to survive the
        // child: a process killed after it wrote the report still has
        // one to read, where anything buffered on a pipe is gone with
        // it. It also keeps the two audiences apart — the importer
        // prints for a person on stderr, and writes this for whatever
        // started it.
        let report_dir = tempfile::tempdir().map_err(|err| {
            DomainError::Infra(anyhow::anyhow!(
                "no directory to receive the run's report: {err}"
            ))
        })?;
        let report_path = report_dir.path().join("report.json");

        let mut command = tokio::process::Command::new(&binary);
        command
            .arg(&spec.subcommand)
            .arg("--persona-id")
            .arg(&spec.persona_id)
            .arg("--report")
            .arg(&report_path)
            .args(&spec.args)
            // `--header-secret` is appended below, after the
            // definition's arguments, so that a definition cannot
            // supply one of its own and have it win.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        // Resolved here and nowhere else. A named variable that is not
        // set is the operator's to fix and is said so plainly — running
        // anyway would reach the source unauthenticated and report
        // whatever that looks like, which is never the true reason.
        if let Some(name) = &spec.secret_ref {
            let Some(value) = std::env::var_os(name) else {
                return Ok(LaunchOutcome {
                    report: None,
                    detail: format!(
                        "the credential this import names lives in ${name}, and it is \
                         not set in this process's environment"
                    ),
                    started: false,
                });
            };
            command.env(CHILD_SECRET_VAR, value);
            // The definition says *where* the credential goes; which
            // argument carries that to this particular binary is this
            // file's business, which is why the flag is written here
            // and not stored.
            if let Some(header) = &spec.secret_header {
                command.arg("--header-secret").arg(header);
            }
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                return Ok(LaunchOutcome {
                    report: None,
                    detail: format!("{} would not start: {err}", binary.display()),
                    started: false,
                });
            }
        };

        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr).await;
        }
        let status = child.wait().await.map_err(|err| {
            DomainError::Infra(anyhow::anyhow!("waiting for the importer: {err}"))
        })?;

        let report = tokio::fs::read(&report_path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ImportReport>(&bytes).ok());

        Ok(LaunchOutcome {
            report,
            detail: format!("exit {status}\n{}", tail(&stderr)),
            started: true,
        })
    }
}

/// The last `STDERR_TAIL` bytes, cut at a character boundary.
fn tail(text: &str) -> &str {
    if text.len() <= STDERR_TAIL {
        return text;
    }
    let mut start = text.len() - STDERR_TAIL;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
