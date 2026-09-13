//! Defining an import, and running one without anybody typing it.
//!
//! The half of the transport decision #295 left open. Adapters run
//! themselves and push over HTTP, which settles how an import *talks*
//! to Asterism and says nothing about what starts it. This starts it.
//!
//! ## What is here and what is deliberately not
//!
//! Running one on demand. **No schedule**: a timer belongs on top of
//! this and calls it, and every hard question — where the binary is,
//! how a credential reaches it, what happens when a run is already
//! going, what is left behind when nothing worked — is answerable
//! without a clock. The timer is also where the wait a rate limit
//! states — carried here as `ImportRun::retry_after_secs` — finally
//! gets a consumer; this slice records it and reads it nowhere.
//!
//! ## Starting is not waiting
//!
//! [`ImportRunService::run`] answers as soon as the importer is on its
//! way, with the run it just opened, and the outcome arrives on that
//! row. The first shape held the answer until the child exited, which
//! read well — the answer *is* the outcome — and was wrong twice over:
//! an import of any size outlives an HTTP client's patience, and the
//! caller this exists for is a scheduler, which wants to start things
//! and not to sit with them.
//!
//! ## The lock is this process's, not the table's
//!
//! Two runs of one definition would each take up from a position the
//! other is about to move, so one has to be refused. The question is
//! "is a child of **mine** still going", and that is about one process:
//! it is true only while this process lives, and a process that died
//! took its children with it.
//!
//! The first shape asked the table instead — a run row said `running`
//! and a second run read it — which made the row a lock as well as a
//! record. A crash then left a row nothing would ever close and a
//! definition nothing could ever start, recoverable only by editing
//! SQLite. It also raced: two callers could both read "nothing running"
//! before either wrote.
//!
//! So the answer lives in a set held here, and
//! [`ImportDefinitionRepository::abandon_running`] closes at startup
//! whatever a previous process left open.
//!
//! **A process-local answer is only sound while one process opens a
//! core, and that is now enforced rather than assumed.** The Tantivy
//! index is opened for writing unconditionally (#300), which takes an
//! exclusive writer lock, so a second core over the same index does not
//! start. A review round was spent asking which process should host
//! this supervisor; the answer is that there is one, and nothing here
//! is conditioned on a mode. That question existed because `CoreMode`
//! did.
//!
//! ## Spawning is not this layer's
//!
//! [`ImportLauncher`] is the port, and the reason it exists rather than
//! a `Command` a few lines down is the reason every port here exists:
//! this crate does not touch the machine. It also keeps the credential
//! out of this layer entirely — the launcher is handed a variable
//! *name* and resolves it while building the child's environment, so
//! the value never enters a type anything here holds, logs, or could
//! accidentally persist.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use asterism_contract::command::{DefineImportCommand, RunImportDefinitionCommand};
use asterism_contract::dto::{ImportDefinitionDto, ImportRunDto};
use asterism_contract::import_report::ImportReport;
use chrono::Utc;
use uuid::Uuid;

use crate::domain::attribution::AttributionContext;
use crate::domain::import_definition::{ImportDefinition, ImportRun, RunOutcome};
use crate::domain::repository::ImportDefinitionRepository;
use crate::error::{ConflictKind, DomainError};

/// What the launcher is asked to start.
///
/// Carries `secret_ref` rather than a secret: see the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// Importer subcommand.
    pub subcommand: String,
    /// Arguments as the importer receives them.
    pub args: Vec<String>,
    /// Persona the records land in.
    pub persona_id: String,
    /// Name of the environment variable holding the credential, if the
    /// definition named one.
    pub secret_ref: Option<String>,
    /// Header the credential is sent as, paired with `secret_ref`.
    ///
    /// The launcher turns the pair into whatever argument the importer
    /// reads it by. Which argument that is belongs to the launcher: a
    /// definition states where a credential goes, not how a particular
    /// binary is told.
    pub secret_header: Option<String>,
}

/// What starting one produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOutcome {
    /// The report the importer wrote, when it got far enough to write
    /// one. `None` means it did not — it failed to start, or it died
    /// before finishing, and either way there is nothing to count.
    pub report: Option<ImportReport>,
    /// What happened, for a person reading a run that has no report.
    ///
    /// The child's exit and the tail of what it said when one ran; why
    /// none could be started when one did not. Always set on the
    /// second, because a run recorded as `unstarted` with nothing said
    /// is a run nobody can act on.
    pub detail: String,
    /// Whether the importer ran at all.
    ///
    /// The cut between "the source or the settings" and "the machine",
    /// which is [`RunOutcome::Unstarted`]'s whole reason for being.
    pub started: bool,
}

/// Starts an importer and waits for it.
///
/// Implemented in the infrastructure layer, which is where a process
/// may be spawned and where an environment may be read.
#[async_trait::async_trait]
pub trait ImportLauncher: Send + Sync {
    /// Runs `spec` to completion.
    ///
    /// Returns `Err` only for a failure of this port itself. An
    /// importer that ran and failed, or a binary that is not there, is
    /// an `Ok` carrying a [`LaunchOutcome`] that says so — because both
    /// are things a run should be *recorded* as, and an error that
    /// unwound past the caller would leave no record at all.
    async fn launch(&self, spec: LaunchSpec) -> Result<LaunchOutcome, DomainError>;
}

/// Stored imports, and running them.
pub struct ImportRunService {
    repo: Arc<dyn ImportDefinitionRepository>,
    launcher: Arc<dyn ImportLauncher>,
    /// Definitions this process currently has a child running for.
    ///
    /// The lock, and the reason it is here rather than in the table is
    /// the module doc's. A `std::sync::Mutex` and not a `tokio` one on
    /// purpose: nothing is awaited while it is held, so a guard cannot
    /// cross a suspension point and the async flavour would only buy a
    /// way to hold it across one.
    running: Mutex<HashSet<String>>,
}

/// Releases a definition's slot however the run leaves it.
///
/// A guard rather than a call at the end, because the end is reached
/// four ways — the launcher answered, the launcher failed, the
/// recording failed, the task was dropped — and a slot left held is a
/// definition nothing can start again until the process restarts.
struct RunSlot {
    service: Arc<ImportRunService>,
    definition_id: String,
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        self.service
            .running
            .lock()
            .expect("the running set")
            .remove(&self.definition_id);
    }
}

impl ImportRunService {
    /// Wraps the persistence port and the launcher.
    pub fn new(
        repo: Arc<dyn ImportDefinitionRepository>,
        launcher: Arc<dyn ImportLauncher>,
    ) -> Self {
        Self {
            repo,
            launcher,
            running: Mutex::new(HashSet::new()),
        }
    }

    /// Closes every run a previous process left open, and answers how
    /// many there were.
    ///
    /// Called once at startup. A run is a child of the process that
    /// spawned it, so a row still saying `running` when a process
    /// starts belongs to one that is gone: nothing will finish it, and
    /// leaving it says a run is in progress that is not.
    pub async fn abandon_orphans(&self) -> Result<u64, DomainError> {
        self.repo.abandon_running().await
    }

    /// Stores an import.
    pub async fn define(
        &self,
        command: DefineImportCommand,
        _by: &AttributionContext,
    ) -> Result<ImportDefinitionDto, DomainError> {
        if command.name.trim().is_empty() {
            return Err(DomainError::Validation(
                "an import needs a name: it is how a run is asked for and how one is \
                 reported on"
                    .into(),
            ));
        }
        if command.subcommand.trim().is_empty() {
            return Err(DomainError::Validation(
                "an import needs a subcommand to run".into(),
            ));
        }
        // A named credential with nowhere to go is the quiet failure
        // this check exists for: the launcher would resolve the
        // variable, hand the value to a child that sends it nowhere,
        // and the run would reach the source unauthenticated and report
        // whatever that looks like. Which is a true report of the wrong
        // thing.
        //
        // Asked of the pair rather than of the arguments. The first
        // shape looked for `--header-secret` in `args` — guessing at
        // another binary's command line, and quiet the day that flag
        // was renamed.
        if command.secret_ref.is_some() != command.secret_header.is_some() {
            return Err(DomainError::Validation(
                "a credential needs both halves: `secret_ref` names the variable \
                 holding it and `secret_header` says which header it is sent as, \
                 and one without the other is a credential that goes nowhere"
                    .into(),
            ));
        }
        // A watch never ends, so it can never be a run that finishes —
        // it would hold the row that stops a second run open for as
        // long as the process lived, and the request that started it
        // with it.
        if command.args.iter().any(|arg| arg == "--watch") {
            return Err(DomainError::Validation(
                "a stored import cannot watch: a watch never ends, and a run that \
                 never ends holds the position of the one that would follow it"
                    .into(),
            ));
        }
        let definition = ImportDefinition {
            id: Uuid::now_v7().to_string(),
            persona_id: command.persona_id,
            name: command.name,
            subcommand: command.subcommand,
            args: command.args,
            secret_ref: command.secret_ref,
            secret_header: command.secret_header,
        };
        self.repo.upsert(&definition).await?;
        Ok(to_definition_dto(definition))
    }

    /// Every stored import.
    pub async fn list(&self) -> Result<Vec<ImportDefinitionDto>, DomainError> {
        Ok(self
            .repo
            .list()
            .await?
            .into_iter()
            .map(to_definition_dto)
            .collect())
    }

    /// Starts one now, and answers with the run it opened.
    ///
    /// Does **not** wait for the importer. The outcome lands on the row
    /// this returns, which [`runs`](Self::runs) reads — see the module
    /// doc for why holding the answer until the child exited was the
    /// wrong shape twice over.
    ///
    /// Refused while a run of the same definition is going, because two
    /// importers over one source would each take up from a position the
    /// other is about to move. The refusal is decided against this
    /// process's own set, under one lock, so two callers arriving
    /// together cannot both win.
    pub async fn run(
        self: &Arc<Self>,
        command: RunImportDefinitionCommand,
        _by: &AttributionContext,
    ) -> Result<ImportRunDto, DomainError> {
        let definition = self
            .repo
            .find(&command.id)
            .await?
            .ok_or_else(|| DomainError::not_found("import definition", &command.id))?;

        // Taken before anything else can fail, and released by the
        // guard however this run ends. `insert` answering `false` is
        // the refusal: the set already held it, under this same lock,
        // so there is no window between asking and taking.
        let slot = {
            let mut running = self.running.lock().expect("the running set");
            if !running.insert(definition.id.clone()) {
                // `Blocked`: the same request works once the running
                // one finishes, which is what that kind is for and what
                // the message is required to say.
                return Err(DomainError::conflict(
                    ConflictKind::Blocked,
                    format!(
                        "{} is already running, and two importers over one source \
                         would each take up from a position the other is about to \
                         move",
                        definition.name
                    ),
                ));
            }
            RunSlot {
                service: Arc::clone(self),
                definition_id: definition.id.clone(),
            }
        };

        let run = ImportRun {
            id: Uuid::now_v7().to_string(),
            definition_id: definition.id.clone(),
            started_at: Utc::now(),
            ended_at: None,
            outcome: RunOutcome::Running,
            imported: 0,
            failed: 0,
            ended_by_class: None,
            ended_by_message: None,
            retry_after_secs: None,
        };
        self.repo.record_run(&run).await?;

        let service = Arc::clone(self);
        let mut finishing = run.clone();
        tokio::spawn(async move {
            // The guard moves in here, so the slot is held for exactly
            // as long as the child is and is released even if this task
            // is dropped.
            let _slot = slot;
            let outcome = service
                .launcher
                .launch(LaunchSpec {
                    subcommand: definition.subcommand,
                    args: definition.args,
                    persona_id: definition.persona_id,
                    secret_ref: definition.secret_ref,
                    secret_header: definition.secret_header,
                })
                .await;

            finishing.ended_at = Some(Utc::now());
            match outcome {
                Ok(outcome) => apply(&mut finishing, outcome),
                // The port itself failed, which is still a run that was
                // asked for and did not happen. No class: nothing ran,
                // so nothing classified anything.
                Err(err) => {
                    finishing.outcome = RunOutcome::Unstarted;
                    finishing.ended_by_message = Some(err.to_string());
                }
            }
            if let Err(err) = service.repo.record_run(&finishing).await {
                // Nowhere left to report it to: the caller was answered
                // when the run opened. The row keeps saying `running`
                // until a restart sweeps it, which is the same state a
                // killed process leaves and is read the same way.
                tracing::error!(
                    event = "diag.import_run.record_failed",
                    run = %finishing.id,
                    error = %err,
                    "could not record how an import run ended"
                );
            }
        });

        Ok(to_run_dto(run))
    }

    /// Recent runs of one import, newest first.
    pub async fn runs(
        &self,
        definition_id: &str,
        limit: u32,
    ) -> Result<Vec<ImportRunDto>, DomainError> {
        Ok(self
            .repo
            .runs_for(definition_id, limit.clamp(1, 200))
            .await?
            .into_iter()
            .map(to_run_dto)
            .collect())
    }
}

/// Folds what the launcher saw onto the run being recorded.
fn apply(run: &mut ImportRun, outcome: LaunchOutcome) {
    if !outcome.started {
        // As above: no class. Nothing classified anything, because
        // nothing ran.
        run.outcome = RunOutcome::Unstarted;
        run.ended_by_message = Some(outcome.detail);
        return;
    }
    let Some(report) = outcome.report else {
        // It started and left no report: it died, or was killed. Not
        // `Unstarted` — something did run, and a reader looking for why
        // should be sent to the source rather than to the machine.
        run.outcome = RunOutcome::Failed;
        run.ended_by_class = Some("source".into());
        run.ended_by_message = Some(outcome.detail);
        return;
    };
    run.imported = report.imported;
    run.failed = report.failed;
    // A lost record is not a failed run — the port's own rule, and the
    // reason `failed` sits beside the outcome rather than deciding it.
    run.outcome = if report.ended_by.is_some() {
        RunOutcome::Failed
    } else {
        RunOutcome::Ok
    };
    if let Some(failure) = report.ended_by {
        run.ended_by_class = Some(failure.class);
        run.ended_by_message = Some(failure.message);
        run.retry_after_secs = failure.retry_after_secs;
    }
}

fn to_definition_dto(definition: ImportDefinition) -> ImportDefinitionDto {
    ImportDefinitionDto {
        id: definition.id,
        persona_id: definition.persona_id,
        name: definition.name,
        subcommand: definition.subcommand,
        args: definition.args,
        secret_ref: definition.secret_ref,
        secret_header: definition.secret_header,
    }
}

fn to_run_dto(run: ImportRun) -> ImportRunDto {
    ImportRunDto {
        id: run.id,
        definition_id: run.definition_id,
        started_at: run.started_at.to_rfc3339(),
        ended_at: run.ended_at.map(|at| at.to_rfc3339()),
        outcome: run.outcome.as_str().to_string(),
        imported: run.imported,
        failed: run.failed,
        ended_by_class: run.ended_by_class,
        ended_by_message: run.ended_by_message,
        retry_after_secs: run.retry_after_secs,
    }
}
