//! An import somebody can run without typing it, and the record of
//! having run one.
//!
//! #295 gave an import a memory and #297 gave it a source that can
//! refuse; neither made anything *start* one. A definition is what a
//! person types once so that nothing has to type it again, and a run is
//! what happened the times it went.
//!
//! ## A definition is a stored command line
//!
//! Which is exactly where a credential wants to go, and the one thing
//! this must be built not to hold. [`ImportDefinition::secret_ref`]
//! carries the **name** of an environment variable and never its value,
//! following `auth.secret_ref` on the outbound side for the reason
//! stated there: a value resolved when the importer starts is in
//! neither this row nor anything written down afterwards.
//!
//! The arguments beside it are stored verbatim and are readable by
//! anything that can read the definition. That is a real trade and it
//! is the operator's to make knowingly, so it is said on the field
//! rather than left to be discovered.
//!
//! ## A run is written whatever happened
//!
//! Including for one whose importer never started. A definition that
//! looks as though it was never run, when somebody asked for it and
//! something went wrong, is the failure mode a record of runs exists to
//! prevent — and "the binary is not there" is the most likely first
//! thing to go wrong on a machine nobody has configured yet.

use chrono::{DateTime, Utc};

/// An import that can be run on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDefinition {
    /// Stable id.
    pub id: String,
    /// Persona the records land in.
    pub persona_id: String,
    /// What this import is called, unique within its persona.
    pub name: String,
    /// Importer subcommand.
    pub subcommand: String,
    /// Arguments, exactly as the importer receives them.
    ///
    /// Stored verbatim and readable by any caller that can read the
    /// definition — see the module doc.
    pub args: Vec<String>,
    /// Name of the environment variable holding this import's
    /// credential, never the credential.
    pub secret_ref: Option<String>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Started and has not been recorded as finished.
    ///
    /// The state a row is written in *before* the importer is
    /// launched, which is what makes the overlap check possible at all:
    /// a second run asks whether one is `Running` and finds the answer
    /// already on the table rather than in the memory of whichever
    /// process happens to be holding it.
    Running,
    /// The importer ran and reported no failure that cost the run.
    Ok,
    /// The importer ran and something cost it — the class is on the
    /// run, because that is what a caller acts on.
    Failed,
    /// The importer never ran: no binary, or it could not be spawned.
    ///
    /// Kept apart from [`Failed`](Self::Failed) because they ask
    /// different things of whoever reads the record. One means the
    /// source or the configuration; this one means the machine, and no
    /// amount of looking at the source will explain it.
    Unstarted,
}

impl RunOutcome {
    /// The slug stored and put on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Unstarted => "unstarted",
        }
    }

    /// Reads a stored slug. An unknown one is `None` rather than a
    /// guess: a row written by a newer build is a row this one cannot
    /// interpret, and reporting it as a success would be worse than
    /// declining to report it.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "running" => Some(Self::Running),
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            "unstarted" => Some(Self::Unstarted),
            _ => None,
        }
    }
}

/// What happened one time an import was run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRun {
    /// Stable id.
    pub id: String,
    /// Definition this was a run of.
    pub definition_id: String,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// When it ended; `None` while it is still going.
    pub ended_at: Option<DateTime<Utc>>,
    /// How it ended.
    pub outcome: RunOutcome,
    /// Records the importer landed.
    pub imported: u64,
    /// Records that did not land.
    pub failed: u64,
    /// The class of whatever ended the run early.
    pub ended_by_class: Option<String>,
    /// What that failure said.
    pub ended_by_message: Option<String>,
    /// How long the source asked us to wait, when it said.
    pub retry_after_secs: Option<u64>,
}
