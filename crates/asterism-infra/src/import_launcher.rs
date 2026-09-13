//! Starting `asterism-import`, and the two things that is really about.
//!
//! The [`ImportLauncher`] port's implementation: the one place in this
//! workspace that spawns an importer. Everything above it is arranged so
//! that this file is the only one holding a credential and the only one
//! that knows a process exists.
//!
//! # Where the binary is
//!
//! Three rungs, and the same three the ffmpeg sidecar uses, in the same
//! order: an explicit `$ASTERISM_IMPORT`, then a sibling of the running
//! executable, then `PATH`. The middle rung is the one worth naming —
//! the comment beside `thumb_ffmpeg`'s ladder records what skipping it
//! cost there, "it reported `ffmpeg is required` on exactly the machines
//! the sidecar exists for", and a bundled importer beside a bundled
//! server is the identical shape. A failure names every rung it tried,
//! because "not found" without the list is a message an operator cannot
//! act on.
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

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};

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
pub struct SubprocessImportLauncher {
    /// Where the importer should post its records, filled in once the
    /// server knows.
    server: Arc<OnceLock<String>>,
}

impl SubprocessImportLauncher {
    /// Binds a launcher to the cell the serving address will appear in.
    ///
    /// A cell rather than a string because the address is not known
    /// when the core is assembled: the port is chosen when something
    /// binds a listener, which happens afterwards and, for a test,
    /// is whatever the OS handed out. The same shape `core_init` uses
    /// for the bound tag head, and for the same reason — a value one
    /// part of the startup learns and another needs.
    ///
    /// A launch before it is set fails as the machine's rather than the
    /// source's, and says which, because an importer pointed at a
    /// server that does not exist would otherwise report a connection
    /// refused and send somebody looking at the wrong thing.
    pub fn new(server: Arc<OnceLock<String>>) -> Self {
        Self { server }
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
        let Some(server) = self.server.get() else {
            return Ok(LaunchOutcome {
                report: None,
                detail: "this server has not finished starting: nothing has told the \
                         launcher which address to point an importer at"
                    .into(),
                started: false,
            });
        };

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

        // The report is a file rather than the child's stdout so that
        // the two are not competing for one stream: an importer is free
        // to print whatever it likes for a person, and what this reads
        // is a contract between two binaries.
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
            .arg("--server")
            .arg(server)
            .arg("--report")
            .arg(&report_path)
            .args(&spec.args)
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
