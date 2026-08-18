//! SQLite adapter for the `ProjectRepository` port (#63 decisions 1–2).
//!
//! Two tables: `project` (thin, immutable, insert-only, like `pursuit`)
//! and `line`, which is the same but owned by a project rather than a
//! persona. The pair is written in one transaction — a project whose
//! line is missing has nothing a merge could land on, and no later
//! read would notice the difference between that and a project nobody
//! has merged into yet.

use asterism_core::domain::attribution::PersistedAttribution;
use asterism_core::domain::forge::line::Line;
use asterism_core::domain::forge::project::Project;
use asterism_core::domain::forge::repository::ProjectRepository;
use asterism_core::domain::forge::value::{LineId, ProjectId};
use asterism_core::domain::value::PersonaId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `ProjectRepository`.
#[derive(Clone)]
pub struct SqliteProjectRepository {
    isle: AsyncIsle,
}

impl SqliteProjectRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

struct ProjectRow {
    id: Uuid,
    persona_id: Uuid,
    name: String,
    note: Option<String>,
    author_kind: Option<String>,
    author_subject: Option<String>,
    operator_ai: Option<String>,
    attributed_via: Option<String>,
    created_at: i64,
}

impl ProjectRow {
    const COLUMNS: &'static str = "id, persona_id, name, note,
                                   author_kind, author_subject, operator_ai, attributed_via,
                                   created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            persona_id: row.get(1)?,
            name: row.get(2)?,
            note: row.get(3)?,
            author_kind: row.get(4)?,
            author_subject: row.get(5)?,
            operator_ai: row.get(6)?,
            attributed_via: row.get(7)?,
            created_at: row.get(8)?,
        })
    }

    fn into_domain(self) -> Result<Project, DomainError> {
        let attribution = PersistedAttribution::from_columns(
            self.author_kind.as_deref(),
            self.author_subject.as_deref(),
            self.operator_ai.as_deref(),
            self.attributed_via.as_deref(),
        )?;
        Ok(Project::from_persisted(
            ProjectId::from_uuid(self.id),
            PersonaId::from_uuid(self.persona_id),
            self.name,
            self.note,
            ms_to_datetime(self.created_at)?,
            attribution,
        ))
    }
}

struct LineRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    created_at: i64,
}

impl LineRow {
    const COLUMNS: &'static str = "id, project_id, name, created_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            project_id: row.get(1)?,
            name: row.get(2)?,
            created_at: row.get(3)?,
        })
    }

    fn into_domain(self) -> Result<Line, DomainError> {
        Ok(Line::from_persisted(
            LineId::from_uuid(self.id),
            ProjectId::from_uuid(self.project_id),
            self.name,
            ms_to_datetime(self.created_at)?,
        ))
    }
}

#[async_trait]
impl ProjectRepository for SqliteProjectRepository {
    async fn create(&self, project: &Project, lines: &[Line]) -> Result<(), DomainError> {
        let id = *project.id.as_uuid();
        let persona_id = *project.persona_id.as_uuid();
        let name = project.name.clone();
        let note = project.note.clone();
        let (author_kind, author_subject, operator_ai, attributed_via) =
            super::attribution_guard::attribution_columns(
                "project",
                &project.persisted_attribution(),
            )?;
        let created = datetime_to_ms(&project.created_at);
        // A project with no line is the state the transaction below
        // exists to prevent, and an empty slice would commit it
        // without the loop running once — the same row a merge could
        // never target, arrived at by a different route.
        if lines.is_empty() {
            return Err(DomainError::Validation(
                "a project opens with at least one line".into(),
            ));
        }
        // Checked here rather than trusted: a line belonging to another
        // project would be written under this project's id by the
        // INSERT below, and the row would look ordinary afterwards.
        let line_rows: Vec<(Uuid, Uuid, String, i64)> = lines
            .iter()
            .map(|line| {
                if line.project_id != project.id {
                    return Err(DomainError::Validation(
                        "a line opened with a project must belong to it".into(),
                    ));
                }
                Ok((
                    *line.id.as_uuid(),
                    // The line's own project, not the project's id.
                    // Identical while the check above stands, and the
                    // difference is the point: relaxing that check
                    // makes the write fail on the foreign key rather
                    // than silently re-parent.
                    *line.project_id.as_uuid(),
                    line.name.clone(),
                    datetime_to_ms(&line.created_at),
                ))
            })
            .collect::<Result<_, _>>()?;
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO project
                         (id, persona_id, name, note,
                          author_kind, author_subject, operator_ai, attributed_via,
                          created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        id,
                        persona_id,
                        name,
                        note,
                        author_kind,
                        author_subject,
                        operator_ai,
                        attributed_via,
                        created,
                    ],
                )?;
                for (line_id, line_project, line_name, line_created) in &line_rows {
                    tx.execute(
                        "INSERT INTO line (id, project_id, name, created_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![line_id, line_project, line_name, line_created],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(|err| {
                // Three UNIQUEs are reachable, and they mean different
                // things, so the message says which: both primary keys
                // (SQLite reports a non-INTEGER PK as a unique index),
                // and `(project_id, name)` on `line`. Project *name*
                // uniqueness is not among them — that rule is
                // application-side and never reaches the schema.
                // Matched on the message text, the `pursuit` adapter's
                // precedent.
                let msg = err.to_string();
                if msg.contains("project.id") {
                    DomainError::Conflict(format!("project {id} already exists"))
                } else if msg.contains("line.id") {
                    DomainError::Conflict("a line id is already in use".to_string())
                } else if msg.contains("UNIQUE") || msg.contains("unique") {
                    DomainError::Conflict("a project holds one line per name".to_string())
                } else {
                    infra_err(err)
                }
            })
    }

    async fn find(&self, id: &ProjectId) -> Result<Option<Project>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT {} FROM project WHERE id = ?1", ProjectRow::COLUMNS),
                    params![uuid],
                    ProjectRow::from_row,
                )
                .map(Some)
                .or_else(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(ProjectRow::into_domain).transpose()
    }

    async fn find_named(
        &self,
        persona_id: &PersonaId,
        name: &str,
    ) -> Result<Option<Project>, DomainError> {
        let uuid = *persona_id.as_uuid();
        // Bound as given. The domain trims on the way in, so trimming
        // again here would be a second normalization agreeing with the
        // first by luck — and it would quietly contradict the port,
        // which promises the column's own byte-exact comparison.
        let name = name.to_string();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM project WHERE persona_id = ?1 AND name = ?2",
                        ProjectRow::COLUMNS
                    ),
                    params![uuid, name],
                    ProjectRow::from_row,
                )
                .map(Some)
                .or_else(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(ProjectRow::into_domain).transpose()
    }

    async fn list(&self, persona_id: &PersonaId, limit: u32) -> Result<Vec<Project>, DomainError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let uuid = *persona_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM project WHERE persona_id = ?1
                     ORDER BY created_at DESC, id DESC LIMIT ?2",
                    ProjectRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![uuid, limit], ProjectRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(ProjectRow::into_domain).collect()
    }

    async fn lines_of(&self, project_id: &ProjectId) -> Result<Vec<Line>, DomainError> {
        let uuid = *project_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM line WHERE project_id = ?1 ORDER BY created_at, id",
                    LineRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map(params![uuid], LineRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(LineRow::into_domain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::attribution::AttributionContext;
    use chrono::{Duration, TimeZone, Utc};

    async fn seed_persona(isle: &AsyncIsle, name: &str, order: i64) -> PersonaId {
        let persona = Uuid::now_v7();
        let name = name.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 0, 0, 0)",
                params![persona, name, order],
            )
        })
        .await
        .unwrap();
        PersonaId::from_uuid(persona)
    }

    fn opened(persona: PersonaId, name: &str, at: chrono::DateTime<Utc>) -> (Project, Line) {
        let project = Project::new(
            persona,
            name.to_string(),
            None,
            at,
            &AttributionContext::owner_surface(),
        )
        .unwrap();
        let line = Line::main(project.id, at);
        (project, line)
    }

    #[tokio::test]
    async fn a_project_and_its_line_are_written_and_read_back_together() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle, "p", 0).await;
        let repo = SqliteProjectRepository::new(isle.clone());
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let (project, line) = opened(persona, "album", t0);
        repo.create(&project, std::slice::from_ref(&line))
            .await
            .unwrap();

        assert_eq!(repo.find(&project.id).await.unwrap(), Some(project.clone()));
        assert_eq!(repo.find(&ProjectId::new()).await.unwrap(), None);
        assert_eq!(repo.lines_of(&project.id).await.unwrap(), vec![line]);

        driver.shutdown().await.unwrap();
    }

    /// The pair is one fact. A project left standing without the line
    /// it was opened with would answer `lines_of` with nothing, and
    /// nothing downstream could tell that apart from a project whose
    /// line is simply empty — so the failing half has to take the
    /// other with it.
    ///
    /// Asserted against a project written just before, because "the
    /// row is absent" alone cannot tell a rollback from an insert that
    /// never happened. The survivor is what makes it a rollback.
    #[tokio::test]
    async fn a_project_whose_line_is_refused_is_not_left_behind() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle, "p", 0).await;
        let repo = SqliteProjectRepository::new(isle.clone());
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let (survivor, survivor_line) = opened(persona, "kept", t0);
        repo.create(&survivor, &[survivor_line]).await.unwrap();

        let (project, line) = opened(persona, "album", t0);
        let twin = Line::main(project.id, t0);
        let refused = repo.create(&project, &[line, twin]).await;
        assert!(
            matches!(refused, Err(DomainError::Conflict(_))),
            "two lines of one name collide: {refused:?}"
        );
        assert_eq!(
            repo.find(&project.id).await.unwrap(),
            None,
            "the project went back with the line that failed"
        );
        assert_eq!(
            repo.lines_of(&project.id).await.unwrap(),
            vec![],
            "and took the line that had already gone in with it"
        );
        assert_eq!(
            repo.find(&survivor.id).await.unwrap(),
            Some(survivor),
            "the rollback reached its own transaction and no further"
        );

        driver.shutdown().await.unwrap();
    }

    /// The arity the transaction cannot enforce. An empty slice would
    /// commit a project whose loop never ran — the same unusable row,
    /// reached without any failure to roll back.
    #[tokio::test]
    async fn a_project_opened_with_no_line_is_refused() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle, "p", 0).await;
        let repo = SqliteProjectRepository::new(isle.clone());
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let (project, _) = opened(persona, "album", t0);
        assert!(matches!(
            repo.create(&project, &[]).await,
            Err(DomainError::Validation(_))
        ));
        assert_eq!(repo.find(&project.id).await.unwrap(), None);

        driver.shutdown().await.unwrap();
    }

    /// A line naming a different project would be written under this
    /// one's id and read back looking ordinary, so the mismatch is
    /// refused before any row is written.
    #[tokio::test]
    async fn a_line_belonging_to_another_project_is_refused() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = seed_persona(&isle, "p", 0).await;
        let repo = SqliteProjectRepository::new(isle.clone());
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let (project, _) = opened(persona, "album", t0);
        let (elsewhere, stray) = opened(persona, "other", t0);
        assert!(matches!(
            repo.create(&project, &[stray]).await,
            Err(DomainError::Validation(_))
        ));
        assert_eq!(repo.find(&project.id).await.unwrap(), None);
        assert_eq!(repo.find(&elsewhere.id).await.unwrap(), None);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn names_are_looked_up_within_one_persona_and_listed_newest_first() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let mine = seed_persona(&isle, "p", 0).await;
        let theirs = seed_persona(&isle, "q", 1).await;
        let repo = SqliteProjectRepository::new(isle.clone());
        let t0 = Utc.timestamp_millis_opt(1_000).unwrap();

        let (first, first_line) = opened(mine, "album", t0);
        repo.create(&first, &[first_line]).await.unwrap();
        let (second, second_line) = opened(mine, "sketches", t0 + Duration::seconds(1));
        repo.create(&second, &[second_line]).await.unwrap();
        // The same name under another persona: uniqueness is per
        // persona, so this is a legal row and must not be found below.
        let (foreign, foreign_line) = opened(theirs, "album", t0);
        repo.create(&foreign, &[foreign_line]).await.unwrap();

        assert_eq!(
            repo.find_named(&mine, "album").await.unwrap(),
            Some(first.clone()),
            "the persona's own project, not the other's of the same name"
        );
        // Byte-exact, as the port promises: this adapter normalizes
        // nothing. Trimming is the domain's, done once on the way in,
        // and a caller that reaches the port with an untrimmed string
        // is asking for a name that was never stored. The service goes
        // through `Project::new` first, which is why opening `"  album
        // "` still collides there.
        assert_eq!(
            repo.find_named(&mine, "  album  ").await.unwrap(),
            None,
            "the adapter compares what it is given, padding included"
        );
        assert_eq!(
            repo.find_named(&mine, "Album").await.unwrap(),
            None,
            "and case, on the column's own BINARY collation"
        );
        assert_eq!(repo.find_named(&mine, "absent").await.unwrap(), None);
        assert_eq!(
            repo.list(&mine, 10).await.unwrap(),
            vec![second.clone(), first],
            "most-recent first, and only this persona's"
        );
        assert_eq!(
            repo.list(&mine, 1).await.unwrap(),
            vec![second],
            "the limit truncates from the newest end"
        );
        assert_eq!(
            repo.list(&mine, 0).await.unwrap(),
            vec![],
            "and asking for none returns none rather than everything"
        );

        driver.shutdown().await.unwrap();
    }
}
