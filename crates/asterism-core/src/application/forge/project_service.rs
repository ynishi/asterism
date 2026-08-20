//! Project use cases — opening the context work files under, and
//! reading it back (#63 decisions 1–2).
//!
//! **Deprecated.** The model this serves is replaced by
//! [`model`](crate::domain::forge::model), where a line is the top and
//! nothing groups lines inside the forge.
//!
//! Thin next to [`pursuit_service`](super::pursuit_service), and it
//! stays that way while the merge is unwritten: a project has no
//! lifecycle of its own. It is opened, it is read, and everything that
//! happens *to* it happens through the pursuits filed under it.
//!
//! Two rules live here rather than in the schema, for opposite
//! reasons. **Name uniqueness among one persona's projects** is
//! application-side and read-checked, so two callers racing can both
//! find the name free and both write it — the rule is advisory under
//! concurrency, and closing that is a schema decision (a partial
//! UNIQUE) rather than a service one. **The line minted with the
//! project** is the other way round: the repository writes both in one
//! transaction, and this layer only decides that there is exactly one
//! and what it is called.

use std::sync::Arc;

use chrono::Utc;

use crate::application::forge::mapping::parse_project_id;
use crate::application::mapping::parse_persona_id;
use crate::domain::attribution::AttributionContext;
use crate::domain::forge::line::Line;
use crate::domain::forge::project::Project;
use crate::domain::forge::repository::ProjectRepository;
use crate::domain::repository::PersonaRepository;
use crate::error::DomainError;
use asterism_contract::command::OpenProjectCommand;
use asterism_contract::dto::{LineDto, ProjectDto};

/// Project use-case service.
pub struct ProjectService {
    projects: Arc<dyn ProjectRepository>,
    personas: Arc<dyn PersonaRepository>,
}

impl ProjectService {
    /// Wires the service around its ports.
    pub fn new(projects: Arc<dyn ProjectRepository>, personas: Arc<dyn PersonaRepository>) -> Self {
        Self { projects, personas }
    }

    /// Opens a project and the line it lands on.
    ///
    /// The line is not a parameter. v1 mints exactly one, named
    /// [`MAIN`](Line::MAIN), because a project with nothing to land on
    /// could not be merged into and there is no second thing to call
    /// it yet — the schema admits siblings so that stays a caller
    /// change later rather than a migration.
    pub async fn open(
        &self,
        command: OpenProjectCommand,
        attribution: &AttributionContext,
    ) -> Result<ProjectDto, DomainError> {
        let persona_id = parse_persona_id(&command.persona_id)?;
        self.personas
            .find(&persona_id)
            .await?
            .ok_or_else(|| DomainError::not_found("persona", &command.persona_id))?;
        let now = Utc::now();
        // Built before the name check so the trim the domain performs
        // is the string the lookup uses — one normalization, not two
        // that agree by coincidence.
        let project = Project::new(persona_id, command.name, command.note, now, attribution)?;
        if let Some(taken) = self.projects.find_named(&persona_id, &project.name).await? {
            return Err(DomainError::Conflict(format!(
                "persona already has a project named {:?} ({})",
                project.name, taken.id
            )));
        }
        let line = Line::main(project.id, now);
        self.projects
            .create(&project, std::slice::from_ref(&line))
            .await?;
        Ok(project_to_dto(&project, &[line]))
    }

    /// Fetches one project with its lines.
    ///
    /// Addressed by id and not scoped to a persona, which is the house
    /// treatment for id-addressed reads (`PursuitService::view` does
    /// the same). Worth stating because the write side one file over
    /// goes out of its way to refuse a cross-persona *reference* — the
    /// asymmetry is deliberate: naming another persona's project in
    /// your own row is a lasting relation, reading one by an id you
    /// already hold is not.
    pub async fn get(&self, project_id: &str) -> Result<ProjectDto, DomainError> {
        let id = parse_project_id(project_id)?;
        let project = self
            .projects
            .find(&id)
            .await?
            .ok_or_else(|| DomainError::not_found("project", project_id))?;
        let lines = self.projects.lines_of(&id).await?;
        Ok(project_to_dto(&project, &lines))
    }

    /// Lists a persona's projects, most-recent first, each with its
    /// lines.
    ///
    /// One `lines_of` per project rather than one query for the page —
    /// an N+1, named here rather than left to be found. v1 mints one
    /// line per project and the page is clamped below, so the cost is
    /// bounded and small; a page that needs more than that wants a
    /// batched read on the port instead, not a bigger loop.
    pub async fn list(&self, persona_id: &str, limit: u32) -> Result<Vec<ProjectDto>, DomainError> {
        let parsed = parse_persona_id(persona_id)?;
        // A caller-supplied `u32::MAX` would otherwise reach SQL and
        // then the loop below.
        let limit = limit.clamp(1, 500);
        let projects = self.projects.list(&parsed, limit).await?;
        let mut out = Vec::with_capacity(projects.len());
        for project in projects {
            let lines = self.projects.lines_of(&project.id).await?;
            out.push(project_to_dto(&project, &lines));
        }
        Ok(out)
    }
}

/// Projects a project and its lines into the wire shape.
fn project_to_dto(project: &Project, lines: &[Line]) -> ProjectDto {
    ProjectDto {
        id: project.id.to_string(),
        persona_id: project.persona_id.to_string(),
        name: project.name.clone(),
        note: project.note.clone(),
        lines: lines.iter().map(line_to_dto).collect(),
        created_at_ms: project.created_at.timestamp_millis(),
    }
}

/// Projects one line into the wire shape.
fn line_to_dto(line: &Line) -> LineDto {
    LineDto {
        id: line.id.to_string(),
        project_id: line.project_id.to_string(),
        name: line.name.clone(),
        created_at_ms: line.created_at.timestamp_millis(),
    }
}
