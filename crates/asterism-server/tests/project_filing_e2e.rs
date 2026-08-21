//! Opening a project and filing a pursuit under it (#63 decisions
//! 1–2), end to end through `CoreCtx`: the line that comes with the
//! project, the name rule among one persona's projects, and the
//! cross-persona refusal no foreign key can express.
//!
//! One test per scenario over its own core, as the sibling e2e files
//! do.

use std::sync::Arc;

use asterism_contract::command::{OpenProjectCommand, OpenPursuitCommand, RegisterPersonaCommand};
use asterism_contract::dto::PersonaDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

async fn boot(tag: &str) -> (tempfile::TempDir, CoreCtx, PersonaDto) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let persona = register(&core, tag).await;
    (tmp, core, persona)
}

async fn register(core: &CoreCtx, tag: &str) -> PersonaDto {
    core.persona_service
        .register(
            RegisterPersonaCommand {
                name: tag.into(),
                pack_id: Some(format!("e2e-project-{tag}")),
            },
            &unattributed(),
        )
        .await
        .expect("register persona")
}

async fn open_project(
    core: &CoreCtx,
    persona: &str,
    name: &str,
) -> asterism_contract::dto::ProjectDto {
    core.project_service
        .open(
            OpenProjectCommand {
                persona_id: persona.to_string(),
                name: name.into(),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("open project")
}

/// The line is not something the caller asks for — it arrives with the
/// project, named `main`, and reads back through both the open and the
/// later get.
#[tokio::test]
async fn a_project_opens_with_the_line_its_work_will_land_on() {
    let (_tmp, core, persona) = boot("opens").await;

    let opened = open_project(&core, &persona.id, "  album  ").await;
    assert_eq!(opened.name, "album", "the name is stored trimmed");
    assert_eq!(opened.lines.len(), 1);
    assert_eq!(opened.lines[0].name, "main");
    assert_eq!(opened.lines[0].project_id, opened.id);

    let read = core.project_service.get(&opened.id).await.expect("get");
    assert_eq!(read.id, opened.id);
    assert_eq!(read.lines.len(), 1, "the line survives the round trip");
    assert_eq!(read.lines[0].id, opened.lines[0].id);
    assert_eq!(read.lines[0].name, "main");

    let listed = core
        .project_service
        .list(&persona.id, 10)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, opened.id);
}

/// The name rule is per persona and read-checked, so it refuses a
/// second project of the same name — and does not refuse the same name
/// under a different persona, which is a legal row.
#[tokio::test]
async fn a_name_is_taken_within_one_persona_and_free_in_another() {
    let (_tmp, core, mine) = boot("mine").await;
    let theirs = register(&core, "theirs").await;

    open_project(&core, &mine.id, "album").await;

    // Padded, because the contract says the name is compared as given
    // *once trimmed* — so this has to collide with the stored `album`
    // rather than open a second project beside it.
    let again = core
        .project_service
        .open(
            OpenProjectCommand {
                persona_id: mine.id.clone(),
                name: "  album  ".into(),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        matches!(again, Err(asterism_core::DomainError::Conflict(_))),
        "a second project of the same name: {again:?}"
    );
    assert_eq!(
        core.project_service
            .list(&mine.id, 10)
            .await
            .expect("list")
            .len(),
        1,
        "and the refusal left nothing behind"
    );

    // Same name, other persona — uniqueness is per persona.
    let elsewhere = open_project(&core, &theirs.id, "album").await;
    assert_eq!(elsewhere.name, "album");
    assert_eq!(
        core.project_service
            .list(&theirs.id, 10)
            .await
            .expect("list")
            .len(),
        1
    );

    // A blank name is not a name.
    let blank = core
        .project_service
        .open(
            OpenProjectCommand {
                persona_id: mine.id.clone(),
                name: "   ".into(),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await;
    assert!(matches!(
        blank,
        Err(asterism_core::DomainError::Validation(_))
    ));
}

/// Filing is what puts a pursuit on a line. The foreign key cannot say
/// that the project belongs to the same persona — `project` carries its
/// own `persona_id` and the column references only `project(id)` — so
/// the refusal has to come from the layer that sees both rows.
#[tokio::test]
async fn a_pursuit_files_under_its_own_personas_project_and_no_others() {
    let (_tmp, core, mine) = boot("filer").await;
    let theirs = register(&core, "stranger").await;

    let project = open_project(&core, &mine.id, "album").await;
    let foreign = open_project(&core, &theirs.id, "theirs").await;

    let filed = core
        .legacy_pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: mine.id.clone(),
                pursuit_id: None,
                project_id: Some(project.id.clone()),
                parent_pursuit_id: None,
                title: Some("key visual".into()),
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("file under my own project");
    assert_eq!(filed.project_id.as_deref(), Some(project.id.as_str()));
    // Read back rather than trusting the DTO `open` built from the
    // in-memory row: if the INSERT dropped the column, the assertion
    // above would still pass.
    let seen = core
        .legacy_pursuit_service
        .view(&filed.id)
        .await
        .expect("view the filed pursuit");
    assert_eq!(
        seen.pursuit.project_id.as_deref(),
        Some(project.id.as_str()),
        "the filing reached the row, not just the answer"
    );

    let across = core
        .legacy_pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: mine.id.clone(),
                pursuit_id: None,
                project_id: Some(foreign.id.clone()),
                parent_pursuit_id: None,
                title: None,
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await;
    // Named rather than matched on the variant alone: `Validation` is
    // what several other refusals on this path return, and the point of
    // the test is which one fired.
    match across {
        Err(asterism_core::DomainError::Validation(message)) => assert!(
            message.contains("different persona"),
            "refused for the wrong reason: {message}"
        ),
        other => panic!("filing under another persona's project: {other:?}"),
    }

    let absent = core
        .legacy_pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: mine.id.clone(),
                pursuit_id: None,
                project_id: Some(uuid::Uuid::now_v7().to_string()),
                parent_pursuit_id: None,
                title: None,
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await;
    assert!(
        matches!(absent, Err(asterism_core::DomainError::NotFound { .. })),
        "filing under a project that is not there: {absent:?}"
    );

    // Unfiled stays legal, and says so rather than guessing a project.
    let unfiled = core
        .legacy_pursuit_service
        .open(
            OpenPursuitCommand {
                persona_id: mine.id.clone(),
                pursuit_id: None,
                project_id: None,
                parent_pursuit_id: Some(filed.id.clone()),
                title: None,
                note: None,
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("an unfiled child is legal");
    assert_eq!(
        unfiled.project_id, None,
        "a child does not inherit its parent's filing"
    );
}
