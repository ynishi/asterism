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
//! without a clock. The timer is also where
//! [`Disposition::FailedRetryable`](crate::domain::import_definition)'s
//! stated wait finally gets a consumer; this slice only records it.
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

use std::sync::Arc;

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
}

/// What starting one produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOutcome {
    /// The report the importer wrote, when it got far enough to write
    /// one. `None` means it did not — it failed to start, or it died
    /// before finishing, and either way there is nothing to count.
    pub report: Option<ImportReport>,
    /// What the child's exit said, for a person reading a run that has
    /// no report. Empty when the child never started.
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
}

impl ImportRunService {
    /// Wraps the persistence port and the launcher.
    pub fn new(
        repo: Arc<dyn ImportDefinitionRepository>,
        launcher: Arc<dyn ImportLauncher>,
    ) -> Self {
        Self { repo, launcher }
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
        let definition = ImportDefinition {
            id: Uuid::now_v7().to_string(),
            persona_id: command.persona_id,
            name: command.name,
            subcommand: command.subcommand,
            args: command.args,
            secret_ref: command.secret_ref,
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

    /// Runs one now, and records what happened.
    ///
    /// # The row is written before the importer starts
    ///
    /// A run is recorded as `running` first, then launched, then
    /// recorded again with what it did. Written in that order because
    /// the record is what the overlap check reads: a run that only
    /// appeared on the table once it had finished would be invisible
    /// for exactly the window the check exists to cover.
    ///
    /// The cost is a crashed process leaving a row that says `running`
    /// forever, and a definition that can then never be started again.
    /// Nothing sweeps those yet. That is worse than the alternative in
    /// one direction and better in the other, and the direction it is
    /// better in is the one that loses data: two importers over one
    /// source each take up from a position the other is about to move,
    /// and neither notices.
    pub async fn run(
        &self,
        command: RunImportDefinitionCommand,
        _by: &AttributionContext,
    ) -> Result<ImportRunDto, DomainError> {
        let definition = self
            .repo
            .find(&command.id)
            .await?
            .ok_or_else(|| DomainError::not_found("import definition", &command.id))?;

        if let Some(running) = self.repo.running_for(&definition.id).await? {
            // `Blocked`: the same request works once the running one
            // finishes, which is what that kind is for and what the
            // message is required to say.
            return Err(DomainError::conflict(
                ConflictKind::Blocked,
                format!(
                    "{} is already running (started {}), and two importers over one \
                     source would each take up from a position the other is about to \
                     move",
                    definition.name, running.started_at
                ),
            ));
        }

        let mut run = ImportRun {
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

        let outcome = self
            .launcher
            .launch(LaunchSpec {
                subcommand: definition.subcommand.clone(),
                args: definition.args.clone(),
                persona_id: definition.persona_id.clone(),
                secret_ref: definition.secret_ref.clone(),
            })
            .await;

        run.ended_at = Some(Utc::now());
        match outcome {
            Ok(outcome) => apply(&mut run, outcome),
            // The port itself failed, which is still a run that was
            // asked for and did not happen. Recorded as such rather
            // than thrown, for the reason `launch` states.
            Err(err) => {
                run.outcome = RunOutcome::Unstarted;
                run.ended_by_class = Some("source".into());
                run.ended_by_message = Some(err.to_string());
            }
        }
        self.repo.record_run(&run).await?;
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
