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
//! The middle rung is the one worth naming, because it is the rung a
//! development build lands on: `cargo build` puts `asterism-ui` and
//! `asterism-import` in one target directory, and neither is on
//! anybody's `PATH`. Nothing bundles the importer today —
//! `tauri.bundle.conf.json`'s `externalBin` lists the ffmpeg sidecar
//! and nothing else — so a shipped app reaches an importer by the
//! first rung or the third. A failure names every rung it tried,
//! because "not found" without the list is a message an operator
//! cannot act on.
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
//! # Where the records go, and how the child is told
//!
//! `--server`, resolved here from the active profile. It is the same
//! question the serving process asks — `asterism-ui` binds
//! `active_profile().default_http_port()` — so two processes reading
//! one profile meet on one port without either being handed the
//! other's address.
//!
//! It is passed only when the definition did not say. A server
//! started on `--port` is the case a profile cannot answer, and a
//! definition's own `--server` is what answers it — the same argument
//! a person running the importer by hand types.
//!
//! Omitted rather than overridden, because the importer refuses a
//! repeated `--server` outright ("cannot be used multiple times")
//! rather than taking the last one. That is clap's answer and not a
//! choice made here, so passing both would turn every definition that
//! names its own server into a run that exits 2 before it starts. The
//! credential's flag goes the other way — appended after the
//! definition's arguments, where a second occurrence is *also* refused,
//! which is exactly the point: a definition cannot quietly substitute
//! its own.
//!
//! Two earlier shapes failed here, in opposite directions, and both are
//! worth keeping because either is easy to rebuild.
//!
//! The first had this file *given* an address, which meant something
//! had to hand it one after binding a listener — an
//! `Arc<OnceLock<String>>` filled by whoever served. Nothing filled it.
//! Every shipped binary called `axum::serve` directly, the cell stayed
//! empty, and every run in the product would have answered "this server
//! has not finished starting" forever; the end-to-end test passed
//! because the *test* filled it.
//!
//! The second passed nothing and claimed the child worked the address
//! out for itself from `$ASTERISM_PROFILE`. It does not.
//! `asterism-import` carries a literal `http://127.0.0.1:8989` default
//! and does not depend on this crate, so under any profile but dogfood
//! the child posted at a door with nothing behind it — silently, since
//! a refused push is a failed run and not a wrong one.
//!
//! Both are one mistake: an address known in one process and needed in
//! another, carried by a step somebody has to remember. Asking the
//! profile at both ends carries nothing.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use asterism_contract::import_report::ImportReport;
use asterism_core::application::{ImportLauncher, LaunchOutcome, LaunchSpec};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use tokio::io::AsyncReadExt;

use crate::paths::DataProfile;

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

/// Where a spawned importer pushes, unless its definition says
/// otherwise.
///
/// Split from [`default_api_base`] so the mapping can be checked
/// without an environment: the defect this is here for is a profile
/// resolving to a port nothing is listening on, and that is a property
/// of the mapping, not of the lookup.
///
/// An unreadable profile falls back to dogfood's port, which is what
/// `asterism-ui`'s own argument parse does. Matching it is the point:
/// the two ends have to fail the same way or they stop meeting.
fn api_base_for(profile: Option<DataProfile>) -> String {
    let port = profile.map_or(8989, DataProfile::default_http_port);
    format!("http://127.0.0.1:{port}")
}

/// [`api_base_for`] over the profile this process is running under.
fn default_api_base() -> String {
    api_base_for(crate::paths::active_profile().ok())
}

/// Whether the definition already says where to push.
///
/// Both spellings, because `--server=http://…` is one argument and
/// `--server http://…` is two, and a definition written either way has
/// said the same thing.
fn states_its_own_server(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "--server" || arg.starts_with("--server="))
}

/// The child's argument vector, without the credential's flag.
///
/// Built apart from the `Command` so what it contains can be asserted
/// without spawning anything — in particular that the resolved address
/// is absent whenever the definition carries one of its own, which the
/// importer would otherwise refuse.
fn child_args(spec: &LaunchSpec, report_path: &Path, api_base: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        spec.subcommand.clone().into(),
        "--persona-id".into(),
        spec.persona_id.clone().into(),
        "--report".into(),
        report_path.as_os_str().to_owned(),
    ];
    if !states_its_own_server(&spec.args) {
        args.push("--server".into());
        args.push(api_base.into());
    }
    args.extend(spec.args.iter().map(Into::into));
    args
}

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

    // Named rather than described. "beside this executable" is a
    // location an operator then has to work out, and the whole reason
    // the list exists is to be actionable.
    match std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(BINARY_NAME)))
    {
        Some(beside) if beside.is_file() => return Ok(beside),
        Some(beside) => tried.push(beside.display().to_string()),
        None => tried.push(format!(
            "beside this executable ({BINARY_NAME}), whose own path this process cannot read"
        )),
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
            .args(child_args(&spec, &report_path, &default_api_base()))
            // `--header-secret` is appended below, after the
            // definition's arguments, so that a definition supplying
            // one of its own makes a run that visibly refuses to start
            // rather than one that quietly authenticates differently.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(args: &[&str]) -> LaunchSpec {
        LaunchSpec {
            subcommand: "text".into(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            persona_id: "p1".into(),
            secret_ref: None,
            secret_header: None,
        }
    }

    /// The defect this file was written wrong for: a child sent to
    /// dogfood's port while the app serves dev's.
    ///
    /// The ports are written out rather than read back from
    /// `default_http_port`, because a test that asks the mapping what
    /// the mapping says would have passed against the literal
    /// `http://127.0.0.1:8989` this replaces.
    #[test]
    fn the_base_follows_the_profile() {
        assert_eq!(
            api_base_for(Some(DataProfile::Dev)),
            "http://127.0.0.1:18989"
        );
        assert_eq!(
            api_base_for(Some(DataProfile::Bench)),
            "http://127.0.0.1:28989"
        );
        assert_eq!(
            api_base_for(Some(DataProfile::Dogfood)),
            "http://127.0.0.1:8989"
        );
        // An unreadable profile lands where `asterism-ui`'s own parse
        // lands, so the two ends still meet.
        assert_eq!(api_base_for(None), "http://127.0.0.1:8989");
    }

    /// A definition that says where it pushes is the only one that
    /// says.
    ///
    /// Not "ours first and theirs last": the importer refuses a
    /// repeated `--server` and exits 2, so a second occurrence is not
    /// an override, it is a run that never starts. The end-to-end test
    /// found this the hard way and this is where it is pinned.
    #[test]
    fn a_definition_that_states_a_server_gets_no_second_one() {
        let report = PathBuf::from("/tmp/report.json");
        for stated in [
            vec!["--server", "http://127.0.0.1:41999", "--root", "/corpus"],
            vec!["--server=http://127.0.0.1:41999", "--root", "/corpus"],
        ] {
            let args = child_args(&spec(&stated), &report, "http://127.0.0.1:8989");
            let servers = args
                .iter()
                .filter(|a| *a == "--server" || a.to_string_lossy().starts_with("--server="))
                .count();
            assert_eq!(servers, 1, "exactly the definition's: {args:?}");
            assert!(
                !args.iter().any(|a| a == "http://127.0.0.1:8989"),
                "and the resolved one is not passed at all: {args:?}"
            );
        }
    }

    /// A definition that says nothing is sent the resolved address,
    /// and everything it did write is still passed.
    #[test]
    fn a_definition_that_states_none_is_sent_the_resolved_one() {
        let report = PathBuf::from("/tmp/report.json");
        let args = child_args(&spec(&["--root", "/corpus"]), &report, "http://base");
        let server = args.iter().position(|a| a == "--server").expect("--server");
        assert_eq!(args[server + 1], "http://base");
        let root = args.iter().position(|a| a == "--root").expect("--root");
        assert_eq!(args[root + 1], "/corpus");
        assert_eq!(args[0], "text", "the subcommand leads");
    }
}
