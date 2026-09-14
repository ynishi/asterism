//! SQLite adapter for the `ImportDefinitionRepository` port.
//!
//! Two tables behind one port, for the reason the port states: a
//! definition and its runs are read and written together, so splitting
//! them would make one act reach through two ports.
//!
//! It does not answer whether a run is going. That question belongs to
//! the process holding the children — see the service — and the only
//! thing left of it here is [`abandon_running`], a sweep over rows a
//! previous process left open.
//!
//! [`abandon_running`]: asterism_core::domain::repository::ImportDefinitionRepository::abandon_running
//!
//! `args_json` holds the argument vector as a JSON array. A column per
//! argument is not a shape a command line has, and a single
//! space-joined string would have to be re-split by something that
//! guessed at quoting — the one part of a command line it is genuinely
//! easy to get wrong, and getting it wrong here means an importer run
//! with arguments nobody wrote.

use asterism_core::domain::import_definition::{ImportDefinition, ImportRun, RunOutcome};
use asterism_core::domain::repository::ImportDefinitionRepository;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::{OptionalExtension, params};
use rusqlite_isle::AsyncIsle;

use crate::fault::StoreFault;
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `ImportDefinitionRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteImportDefinitionRepository {
    isle: AsyncIsle,
}

impl SqliteImportDefinitionRepository {
    /// Wraps a writer `AsyncIsle` handle (same discipline as the sibling
    /// adapters).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Definition row as scanned inside the isle closure.
struct DefinitionRow {
    id: String,
    persona_id: String,
    name: String,
    subcommand: String,
    args_json: String,
    secret_ref: Option<String>,
    secret_header: Option<String>,
}

impl DefinitionRow {
    const COLUMNS: &'static str =
        "id, persona_id, name, subcommand, args_json, secret_ref, secret_header";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            name: row.get(2)?,
            subcommand: row.get(3)?,
            args_json: row.get(4)?,
            secret_ref: row.get(5)?,
            secret_header: row.get(6)?,
        })
    }

    /// Promotes the row.
    ///
    /// An `args_json` this build cannot read is an error and not an
    /// empty argument list: running an importer with no arguments
    /// because its arguments would not parse is how a definition
    /// pointed at one source silently becomes a run against another.
    fn into_domain(self) -> Result<ImportDefinition, DomainError> {
        let args: Vec<String> = serde_json::from_str(&self.args_json).map_err(|err| {
            DomainError::Infra(anyhow::anyhow!(
                "import definition {}: arguments did not parse: {err}",
                self.id
            ))
        })?;
        Ok(ImportDefinition {
            id: self.id,
            persona_id: self.persona_id,
            name: self.name,
            subcommand: self.subcommand,
            args,
            secret_ref: self.secret_ref,
            secret_header: self.secret_header,
        })
    }
}

/// Run row as scanned inside the isle closure.
struct RunRow {
    id: String,
    definition_id: String,
    started_at: i64,
    ended_at: Option<i64>,
    outcome: String,
    imported: i64,
    failed: i64,
    ended_by_class: Option<String>,
    ended_by_message: Option<String>,
    retry_after_secs: Option<i64>,
}

impl RunRow {
    const COLUMNS: &'static str = "id, definition_id, started_at, ended_at, outcome, \
                                   imported, failed, ended_by_class, ended_by_message, \
                                   retry_after_secs";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            definition_id: row.get(1)?,
            started_at: row.get(2)?,
            ended_at: row.get(3)?,
            outcome: row.get(4)?,
            imported: row.get(5)?,
            failed: row.get(6)?,
            ended_by_class: row.get(7)?,
            ended_by_message: row.get(8)?,
            retry_after_secs: row.get(9)?,
        })
    }

    fn into_domain(self) -> Result<ImportRun, DomainError> {
        let outcome = RunOutcome::parse(&self.outcome).ok_or_else(|| {
            DomainError::Infra(anyhow::anyhow!(
                "import run {}: this build does not know the outcome {:?}",
                self.id,
                self.outcome
            ))
        })?;
        Ok(ImportRun {
            id: self.id,
            definition_id: self.definition_id,
            started_at: ms_to_datetime(self.started_at)?,
            ended_at: self.ended_at.map(ms_to_datetime).transpose()?,
            outcome,
            imported: self.imported.max(0) as u64,
            failed: self.failed.max(0) as u64,
            ended_by_class: self.ended_by_class,
            ended_by_message: self.ended_by_message,
            retry_after_secs: self.retry_after_secs.map(|secs| secs.max(0) as u64),
        })
    }
}

#[async_trait]
impl ImportDefinitionRepository for SqliteImportDefinitionRepository {
    /// Writes a definition, refusing a second one by a name the persona
    /// already uses.
    ///
    /// The `(persona_id, name)` index is a rule an ordinary caller
    /// breaks by ordinary means — two people naming an import
    /// "Bluesky" — so it is asked rather than tripped. Tripped, it
    /// arrives as `Infra` and answers 500, which tells the person
    /// nothing they can act on and reads like a fault in the server.
    ///
    /// The read and the write are one `call`, so they are one turn on
    /// one connection and nothing writes between them. A check made out
    /// here would be a race the index still catches — as a 500.
    async fn upsert(&self, definition: &ImportDefinition) -> Result<(), DomainError> {
        let id = definition.id.clone();
        let persona_id = definition.persona_id.clone();
        let name = definition.name.clone();
        let subcommand = definition.subcommand.clone();
        let args_json = serde_json::to_string(&definition.args).map_err(|err| {
            DomainError::Infra(anyhow::anyhow!("arguments did not serialise: {err}"))
        })?;
        let secret_ref = definition.secret_ref.clone();
        let secret_header = definition.secret_header.clone();
        let now = datetime_to_ms(&chrono::Utc::now());
        let clash_name = name.clone();
        let clashed = self
            .isle
            .call(move |conn| {
                let held_by: Option<String> = conn
                    .query_row(
                        "SELECT id FROM import_definition
                          WHERE persona_id = ?1 AND name = ?2",
                        params![&persona_id, &name],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(held_by) = held_by
                    && held_by != id
                {
                    return Ok(Some(held_by));
                }
                conn.execute(
                    "INSERT INTO import_definition
                         (id, persona_id, name, subcommand, args_json, secret_ref,
                          secret_header, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                         persona_id    = excluded.persona_id,
                         name          = excluded.name,
                         subcommand    = excluded.subcommand,
                         args_json     = excluded.args_json,
                         secret_ref    = excluded.secret_ref,
                         secret_header = excluded.secret_header",
                    params![
                        id,
                        persona_id,
                        name,
                        subcommand,
                        args_json,
                        secret_ref,
                        secret_header,
                        now
                    ],
                )?;
                Ok(None)
            })
            .await
            .map_err(infra_err)?;
        match clashed {
            None => Ok(()),
            // Through `StoreFault`, because what a refusal means to a
            // caller is not this crate's to say — see
            // `tests/store_fault_is_the_only_door.rs`. The id of the
            // definition holding the name is left out: it is the name
            // the caller chose and can change.
            Some(_) => Err(StoreFault::taken(
                "an import's name inside its persona",
                format!("{clash_name:?}"),
            )
            .into()),
        }
    }

    async fn find(&self, id: &str) -> Result<Option<ImportDefinition>, DomainError> {
        let id = id.to_string();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM import_definition WHERE id = ?1",
                        DefinitionRow::COLUMNS
                    ),
                    params![id],
                    DefinitionRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(DefinitionRow::into_domain).transpose()
    }

    async fn list(&self) -> Result<Vec<ImportDefinition>, DomainError> {
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM import_definition ORDER BY created_at, id",
                    DefinitionRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map([], DefinitionRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(DefinitionRow::into_domain).collect()
    }

    async fn abandon_running(&self) -> Result<u64, DomainError> {
        let now = datetime_to_ms(&chrono::Utc::now());
        let swept = self
            .isle
            .call(move |conn| {
                // `ended_at` is set to now rather than left null: the
                // run did end, at some unknown moment before this
                // process started, and a row with an outcome and no end
                // reads as still going to anything sorting by it.
                let swept = conn.execute(
                    "UPDATE import_run \
                        SET outcome = 'abandoned', ended_at = COALESCE(ended_at, ?1) \
                      WHERE outcome = 'running'",
                    params![now],
                )?;
                Ok(swept)
            })
            .await
            .map_err(infra_err)?;
        Ok(swept as u64)
    }

    async fn record_run(&self, run: &ImportRun) -> Result<(), DomainError> {
        let id = run.id.clone();
        let definition_id = run.definition_id.clone();
        let started_at = datetime_to_ms(&run.started_at);
        let ended_at = run.ended_at.as_ref().map(datetime_to_ms);
        let outcome = run.outcome.as_str();
        let imported = run.imported as i64;
        let failed = run.failed as i64;
        let class = run.ended_by_class.clone();
        let message = run.ended_by_message.clone();
        let retry = run.retry_after_secs.map(|secs| secs as i64);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO import_run
                         (id, definition_id, started_at, ended_at, outcome, imported,
                          failed, ended_by_class, ended_by_message, retry_after_secs)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(id) DO UPDATE SET
                         ended_at         = excluded.ended_at,
                         outcome          = excluded.outcome,
                         imported         = excluded.imported,
                         failed           = excluded.failed,
                         ended_by_class   = excluded.ended_by_class,
                         ended_by_message = excluded.ended_by_message,
                         retry_after_secs = excluded.retry_after_secs",
                    params![
                        id,
                        definition_id,
                        started_at,
                        ended_at,
                        outcome,
                        imported,
                        failed,
                        class,
                        message,
                        retry
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn runs_for(
        &self,
        definition_id: &str,
        limit: u32,
    ) -> Result<Vec<ImportRun>, DomainError> {
        let definition_id = definition_id.to_string();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM import_run WHERE definition_id = ?1 \
                     ORDER BY started_at DESC LIMIT ?2",
                    RunRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![definition_id, limit], RunRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(RunRow::into_domain).collect()
    }
}
