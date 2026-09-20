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
use chrono::{DateTime, Utc};
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
    every_minutes: Option<i64>,
}

impl DefinitionRow {
    const COLUMNS: &'static str = "id, persona_id, name, subcommand, args_json, \
                                   secret_ref, secret_header, every_minutes";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            name: row.get(2)?,
            subcommand: row.get(3)?,
            args_json: row.get(4)?,
            secret_ref: row.get(5)?,
            secret_header: row.get(6)?,
            every_minutes: row.get(7)?,
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
        // An interval this build cannot read is an error and not "no
        // schedule", for the reason this method's doc gives about
        // `args_json`. A
        // definition silently demoted to manual is an import that
        // quietly stops filling, which is the failure the whole slice
        // exists to prevent, and it would look exactly like one nobody
        // had scheduled.
        let every_minutes = self
            .every_minutes
            .map(|minutes| {
                u32::try_from(minutes).map_err(|_| {
                    DomainError::Infra(anyhow::anyhow!(
                        "import definition {}: {minutes} is not an interval this \
                         build can read",
                        self.id
                    ))
                })
            })
            .transpose()?;
        Ok(ImportDefinition {
            id: self.id,
            persona_id: self.persona_id,
            name: self.name,
            subcommand: self.subcommand,
            args,
            secret_ref: self.secret_ref,
            secret_header: self.secret_header,
            every_minutes,
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
        let every_minutes = definition.every_minutes.map(i64::from);
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
                          secret_header, created_at, every_minutes)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(id) DO UPDATE SET
                         persona_id    = excluded.persona_id,
                         name          = excluded.name,
                         subcommand    = excluded.subcommand,
                         args_json     = excluded.args_json,
                         secret_ref    = excluded.secret_ref,
                         secret_header = excluded.secret_header,
                         every_minutes = excluded.every_minutes",
                    params![
                        id,
                        persona_id,
                        name,
                        subcommand,
                        args_json,
                        secret_ref,
                        secret_header,
                        now,
                        every_minutes
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

    /// One query, and the whole of "when is this due" is in it.
    ///
    /// The port's doc states the three rules; what is worth saying here
    /// is how they are spelled. `COALESCE(..., 0)` is the definition
    /// that has never run — an absent subquery result becomes an epoch
    /// nothing is earlier than, so such a definition is due. `max()` of
    /// the two candidate times is the interval against the wait the
    /// source stated, and the `CASE` floors the second at `0` rather
    /// than letting it be `NULL`, because SQLite's `max()` answers
    /// `NULL` if any argument is.
    ///
    /// A run still going has no `ended_at`, so it contributes only its
    /// interval — which is what should happen, for the reason the port
    /// gives about not asking here whether one is going.
    async fn due(&self, now: DateTime<Utc>) -> Result<Vec<ImportDefinition>, DomainError> {
        let now_ms = datetime_to_ms(&now);
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM import_definition
                      WHERE every_minutes IS NOT NULL
                        AND ?1 >= COALESCE((
                              SELECT max(
                                       r.started_at
                                         + import_definition.every_minutes * 60000,
                                       CASE WHEN r.retry_after_secs IS NOT NULL
                                                 AND r.ended_at IS NOT NULL
                                            THEN r.ended_at + r.retry_after_secs * 1000
                                            ELSE 0
                                       END)
                                FROM import_run r
                               WHERE r.definition_id = import_definition.id
                            ORDER BY r.started_at DESC
                               LIMIT 1), 0)
                   ORDER BY created_at",
                    DefinitionRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![now_ms], DefinitionRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(DefinitionRow::into_domain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use chrono::Duration;

    /// A repository over an empty in-memory database.
    async fn repo() -> (SqliteImportDefinitionRepository, impl Sized) {
        let (isle, driver) = open_and_migrate_in_memory().await.expect("open");
        (SqliteImportDefinitionRepository::new(isle), driver)
    }

    fn definition(name: &str, every_minutes: Option<u32>) -> ImportDefinition {
        ImportDefinition {
            id: uuid::Uuid::now_v7().to_string(),
            persona_id: "p1".into(),
            name: name.into(),
            subcommand: "text".into(),
            args: vec!["--dir".into(), "/corpus".into()],
            secret_ref: None,
            secret_header: None,
            every_minutes,
        }
    }

    /// A finished run of `definition`, started `ago` before `now`.
    fn run_started(
        definition: &ImportDefinition,
        now: DateTime<Utc>,
        started_ago: Duration,
        ended_ago: Option<Duration>,
        retry_after_secs: Option<u64>,
    ) -> ImportRun {
        ImportRun {
            id: uuid::Uuid::now_v7().to_string(),
            definition_id: definition.id.clone(),
            started_at: now - started_ago,
            ended_at: ended_ago.map(|ago| now - ago),
            outcome: RunOutcome::Ok,
            imported: 0,
            failed: 0,
            ended_by_class: None,
            ended_by_message: None,
            retry_after_secs,
        }
    }

    async fn due_names(repo: &SqliteImportDefinitionRepository, now: DateTime<Utc>) -> Vec<String> {
        repo.due(now)
            .await
            .expect("a read")
            .into_iter()
            .map(|d| d.name)
            .collect()
    }

    /// Nothing but a person starts a definition with no interval, and
    /// no amount of time changes that.
    #[tokio::test]
    async fn a_definition_with_no_interval_is_never_due() {
        let (repo, _driver) = repo().await;
        let manual = definition("manual", None);
        repo.upsert(&manual).await.expect("stored");

        let now = Utc::now();
        assert!(due_names(&repo, now).await.is_empty());
        assert!(
            due_names(&repo, now + Duration::days(365)).await.is_empty(),
            "a year later it is still nobody's to start but a person's"
        );
    }

    /// Setting an interval is asking for the archive to start filling,
    /// not to start filling an interval from now.
    #[tokio::test]
    async fn a_scheduled_definition_that_never_ran_is_due_at_once() {
        let (repo, _driver) = repo().await;
        repo.upsert(&definition("hourly", Some(60)))
            .await
            .expect("stored");

        assert_eq!(due_names(&repo, Utc::now()).await, vec!["hourly"]);
    }

    /// The interval is a cadence, not a rest.
    ///
    /// The case that tells the two apart: a run that started forty
    /// minutes ago and ended five minutes ago, on a thirty-minute
    /// interval. Measured from the start it is due; measured from the
    /// end it has twenty-five minutes to wait. An import that takes
    /// longer than its interval would otherwise drift further behind
    /// every time it ran.
    #[tokio::test]
    async fn due_is_measured_from_the_start_of_the_last_run() {
        let (repo, _driver) = repo().await;
        let now = Utc::now();

        let long = definition("long", Some(30));
        repo.upsert(&long).await.expect("stored");
        repo.record_run(&run_started(
            &long,
            now,
            Duration::minutes(40),
            Some(Duration::minutes(5)),
            None,
        ))
        .await
        .expect("recorded");

        assert_eq!(
            due_names(&repo, now).await,
            vec!["long"],
            "forty minutes since it started is past a thirty-minute interval"
        );
    }

    /// And the same definition is not due before the interval is up.
    #[tokio::test]
    async fn a_definition_inside_its_interval_is_not_due() {
        let (repo, _driver) = repo().await;
        let now = Utc::now();

        let recent = definition("recent", Some(30));
        repo.upsert(&recent).await.expect("stored");
        repo.record_run(&run_started(
            &recent,
            now,
            Duration::minutes(10),
            Some(Duration::minutes(9)),
            None,
        ))
        .await
        .expect("recorded");

        assert!(due_names(&repo, now).await.is_empty());
        assert_eq!(
            due_names(&repo, now + Duration::minutes(21)).await,
            vec!["recent"],
            "and it is due when the interval is up"
        );
    }

    /// A wait the source stated wins over the interval when it lands
    /// later.
    ///
    /// This is the consumer `retry_after_secs` has been recorded for
    /// since #297. A source that answered 429 and said "in an hour" is
    /// not something a thirty-minute interval overrides.
    #[tokio::test]
    async fn a_stated_wait_holds_the_next_start_back() {
        let (repo, _driver) = repo().await;
        let now = Utc::now();

        let limited = definition("limited", Some(30));
        repo.upsert(&limited).await.expect("stored");
        repo.record_run(&run_started(
            &limited,
            now,
            Duration::minutes(40),
            Some(Duration::minutes(39)),
            Some(3600),
        ))
        .await
        .expect("recorded");

        assert!(
            due_names(&repo, now).await.is_empty(),
            "the interval is up, and the source said not for an hour"
        );
        assert_eq!(
            due_names(&repo, now + Duration::minutes(22)).await,
            vec!["limited"],
            "and it is due once the hour the source asked for is over"
        );
    }

    /// A wait shorter than the interval does not pull a start in.
    ///
    /// The pair that makes "the later of the two" a rule rather than a
    /// description of one case: here the interval is the later, and it
    /// is what holds.
    #[tokio::test]
    async fn a_short_wait_does_not_shorten_the_interval() {
        let (repo, _driver) = repo().await;
        let now = Utc::now();

        let brief = definition("brief", Some(30));
        repo.upsert(&brief).await.expect("stored");
        repo.record_run(&run_started(
            &brief,
            now,
            Duration::minutes(10),
            Some(Duration::minutes(9)),
            Some(60),
        ))
        .await
        .expect("recorded");

        assert!(
            due_names(&repo, now).await.is_empty(),
            "the minute the source asked for is over and the interval is not"
        );
    }

    /// Only the most recent run is consulted.
    ///
    /// An old run that was rate limited must not hold a definition back
    /// after a later run went through — which is what reading anything
    /// but the newest row would do.
    #[tokio::test]
    async fn an_older_run_s_wait_is_not_still_in_force() {
        let (repo, _driver) = repo().await;
        let now = Utc::now();

        let recovered = definition("recovered", Some(30));
        repo.upsert(&recovered).await.expect("stored");
        repo.record_run(&run_started(
            &recovered,
            now,
            Duration::hours(5),
            Some(Duration::hours(5)),
            Some(86_400),
        ))
        .await
        .expect("the rate-limited run");
        repo.record_run(&run_started(
            &recovered,
            now,
            Duration::minutes(40),
            Some(Duration::minutes(39)),
            None,
        ))
        .await
        .expect("the run that went through");

        assert_eq!(
            due_names(&repo, now).await,
            vec!["recovered"],
            "yesterday's wait is not still in force"
        );
    }

    /// An interval survives the round trip, and so does its absence.
    #[tokio::test]
    async fn an_interval_is_read_back_as_it_was_written() {
        let (repo, _driver) = repo().await;
        let scheduled = definition("scheduled", Some(45));
        let manual = definition("manual", None);
        repo.upsert(&scheduled).await.expect("stored");
        repo.upsert(&manual).await.expect("stored");

        assert_eq!(
            repo.find(&scheduled.id)
                .await
                .expect("a read")
                .expect("stored")
                .every_minutes,
            Some(45)
        );
        assert_eq!(
            repo.find(&manual.id)
                .await
                .expect("a read")
                .expect("stored")
                .every_minutes,
            None
        );
    }

    /// An interval this build cannot read is an error, not a silent
    /// demotion to manual.
    ///
    /// Written straight into the column, because nothing above this
    /// layer can produce one — which is the point: the row outlives the
    /// build that wrote it, and a definition quietly demoted is an
    /// import that stops filling and looks like one nobody scheduled.
    ///
    /// The value is `2^32`, which is the band the `CHECK` does not
    /// cover: the constraint says "null or positive" and says nothing
    /// about how large, so the two guards divide the space between them
    /// rather than overlapping.
    #[tokio::test]
    async fn an_unreadable_interval_is_refused_rather_than_ignored() {
        let (repo, _driver) = repo().await;
        let odd = definition("odd", Some(30));
        repo.upsert(&odd).await.expect("stored");

        let id = odd.id.clone();
        repo.isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE import_definition SET every_minutes = 4294967296 \
                     WHERE id = ?1",
                    params![id],
                )?;
                Ok(())
            })
            .await
            .expect("a write the CHECK allows");

        let err = repo
            .find(&odd.id)
            .await
            .expect_err("an interval no u32 can hold");
        assert!(
            err.to_string().contains("4294967296"),
            "and it says what it found: {err}"
        );
    }

    /// Zero never reaches the column, whatever wrote it.
    ///
    /// `ImportRunService::define` refuses one first and is the only one
    /// that can say why in words; this pins the backstop behind it, and
    /// the reason the migration stopped leaving the column
    /// unconstrained.
    #[tokio::test]
    async fn the_column_refuses_an_interval_of_no_minutes() {
        let (repo, _driver) = repo().await;
        let fine = definition("fine", Some(30));
        repo.upsert(&fine).await.expect("stored");

        let id = fine.id.clone();
        let refused = repo
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE import_definition SET every_minutes = 0 WHERE id = ?1",
                    params![id],
                )?;
                Ok(())
            })
            .await;
        assert!(refused.is_err(), "zero is not a schedule at any layer");

        // And neither is a negative one, which is the other half of
        // what the constraint says.
        let id = fine.id.clone();
        let refused = repo
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE import_definition SET every_minutes = -1 WHERE id = ?1",
                    params![id],
                )?;
                Ok(())
            })
            .await;
        assert!(refused.is_err());

        assert_eq!(
            repo.find(&fine.id)
                .await
                .expect("a read")
                .expect("stored")
                .every_minutes,
            Some(30),
            "and the row is what it was"
        );
    }
}
