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
//!
//! **And a run is only ever a record.** It is not also the lock that
//! stops a second run starting: that question is "is a child of mine
//! still going", which is about one process and dies with it, and
//! answering it from a table made a crash wedge a definition for good.
//! [`RunOutcome::Abandoned`] is what a row inherits when the process
//! that wrote it did not come back.

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
    ///
    /// Always accompanied by [`secret_header`](Self::secret_header):
    /// a credential named with nowhere to go is one that is resolved,
    /// handed over, and spent on a request that went out without it.
    pub secret_ref: Option<String>,
    /// Header the credential is sent as.
    ///
    /// The destination belongs here and not in [`args`](Self::args),
    /// where the only way to check it was present was to look for
    /// another binary's flag by name. A pair the type keeps together
    /// cannot be half-filled.
    pub secret_header: Option<String>,
    /// Minutes between starts, or `None` for an import nothing starts
    /// on its own.
    ///
    /// Measured from one run's **start**, so an import taking twenty
    /// minutes on a thirty-minute interval runs every thirty and not
    /// every fifty: this is a cadence, not a rest between runs.
    ///
    /// A definition that has never run and carries one is due at once.
    /// Somebody setting an interval is asking for the archive to start
    /// filling, not to start filling an interval from now.
    ///
    /// It is not the only thing that decides when the next run starts.
    /// A `retry_after_secs` on the last run is a wait the *source*
    /// stated, and it wins when it lands later — see
    /// [`ImportDefinitionRepository::due`]. Nothing else backs off: a
    /// source that is simply down is tried again on the interval,
    /// because an invented backoff is a second policy with its own
    /// failure modes and belongs to whoever asks for it.
    ///
    /// [`ImportDefinitionRepository::due`]: crate::domain::repository::ImportDefinitionRepository::due
    pub every_minutes: Option<u32>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Started and has not been recorded as finished.
    ///
    /// Written before the importer is launched so that a caller asking
    /// what is happening is told, and so that a run which never reaches
    /// an ending is still visible as having been asked for. It is a
    /// statement about the past, not a claim on the future — see
    /// [`Abandoned`](Self::Abandoned) for what happens to one whose
    /// process is gone.
    Running,
    /// The importer ran and reported no failure that cost the run.
    Ok,
    /// The importer ran and something cost it — the class is on the
    /// run, because that is what a caller acts on.
    Failed,
    /// Nothing the importer did can explain this run: no binary, it
    /// could not be spawned, the credential it named was not set, or
    /// the launcher itself failed.
    ///
    /// Kept apart from [`Failed`](Self::Failed) because they ask
    /// different things of whoever reads the record. `Failed` sends a
    /// reader to the source or to the definition; this one sends them
    /// to the machine this ran on. An unset credential is the case
    /// that sits between the two, and it is here because the fix is on
    /// the machine — the variable is not set in the process that
    /// spawns importers.
    Unstarted,
    /// The process that started it did not outlive it.
    ///
    /// Written at startup over every row still saying
    /// [`Running`](Self::Running), because a run is a child of the
    /// process that spawned it: if that process is gone, so is the
    /// child, and nothing is ever going to finish the row.
    ///
    /// This is the whole of the sweeping that the first shape of this
    /// needed a verb for. That shape read `running` rows to decide
    /// whether to start another — making the row a lock as well as a
    /// record — so a crash wedged a definition permanently and the only
    /// recovery was editing SQLite by hand. Whether a run is going is
    /// now the supervisor's own question about its own children, and
    /// the row went back to being history.
    Abandoned,
}

impl RunOutcome {
    /// The slug stored and put on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Unstarted => "unstarted",
            Self::Abandoned => "abandoned",
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
            "abandoned" => Some(Self::Abandoned),
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
