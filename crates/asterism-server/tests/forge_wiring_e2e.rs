//! The forge through the app's own wiring.
//!
//! `forge_over_ports_e2e` proves the services against a store the test
//! builds. This proves the store the *application* builds: the same
//! verbs, reached from `CoreCtx`, over the database `init_core` opened
//! and migrated.
//!
//! # What that catches and nothing else does
//!
//! A service wired to the wrong thing. Every port here has two
//! implementations, and constructing `PursuitService` with the
//! in-memory store, or with a second `SqliteForge` over a different
//! connection, compiles and passes every test that builds its own
//! world. What it does not do is land work on a line the same process
//! can then read.
//!
//! It also answers the one thing a wiring can be wrong about without
//! anybody noticing: the forge's ports are three faces of one adapter,
//! and a close writes through two of them at once. If those were not
//! the same object over the same connection, the change point and the
//! ending would go to different places and the read below would come
//! back short.
//!
//! # Neither service has a transport
//!
//! So this is where they are reachable from, and the test is the only
//! caller they have. That is stated rather than worked around: the
//! surface they belong on is decided in another issue, and wiring them
//! to nothing until then would leave the adapter unexercised by the
//! app that owns it.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::op::Op;
use asterism_core::domain::forge::model::pursuit::{Intent, Outcome};
use asterism_core::domain::forge::model::strategy::Strategy;
use asterism_core::domain::forge::model::value::{Content, Name};
use asterism_core::domain::forge::strategies::MainlineFirst;
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

fn who(subject: &str) -> AttributionContext {
    AttributionContext::asserted(Some(Author::Subject(subject.into())), None).expect("a subject")
}

fn name(text: &str) -> Name {
    Name::new(text).expect("a name")
}

async fn core(tmp: &std::path::Path) -> CoreCtx {
    init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core")
}

/// A persona and one asset of theirs, through the raw layer's own
/// verbs — because the forge holds what it names, and what it names
/// has to be a row somebody really added.
async fn an_asset(core: &CoreCtx, tmp: &std::path::Path) -> (PersonaId, Content) {
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "ana".into(),
                pack_id: Some("e2e-forge-wiring".into()),
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("a persona");

    let file = tmp.join("one.md");
    std::fs::write(&file, b"one").expect("a file");
    let asset = core
        .asset_service
        .add(
            AddAssetCommand {
                persona_id: persona.id.clone(),
                source_kind: "fs".into(),
                locator: file.to_string_lossy().into_owned(),
                modality: Some("dialogue".into()),
                occurred_at_ms: 0,
                session_id: None,
                external_session_key: None,
                external_key: None,
                bundle_id: None,
                labels: Vec::new(),
                register_note: None,
                platform: None,
                file_size_bytes: None,
                duration_ms: None,
                width_px: None,
                height_px: None,
                extra_json: None,
                cover_hint: None,
                auto_organize_base_dir: None,
                derived_from: None,
                author_kind: None,
                author_subject: None,
                operator_ai: None,
                on_duplicate: None,
                declared_content_hash: None,
                album_meta: Default::default(),
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("an asset");

    (
        PersonaId::from_uuid(persona.id.parse().expect("a persona id")),
        Content::of(AssetId::from_uuid(asset.id.parse().expect("an asset id"))),
    )
}

/// Work opens, lands, and the line the application holds says so.
#[tokio::test(flavor = "multi_thread")]
async fn the_wired_forge_lands_work_and_reads_it_back() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let core = core(tmp.path()).await;
    let (persona, held) = an_asset(&core, tmp.path()).await;

    let line = core
        .line_service
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line opens over the real database");

    let work = core
        .pursuit_service
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .expect("work opens against it");
    core.pursuit_service
        .push(
            &work.id(),
            &persona,
            vec![Op::add(held, name("one"))],
            None,
            &who("ana"),
        )
        .await
        .expect("the boundary agrees this persona holds that asset");
    core.pursuit_service
        .close(&work.id(), Outcome::Satisfied, None, &who("ana"))
        .await
        .expect("nothing is in the way");

    // Both halves landed, and they landed in the same place: the line
    // read back through the service the application built carries what
    // the work asked for, and the change point names the work.
    let read_back = core
        .line_service
        .get(&line.id())
        .await
        .expect("read it back");
    let states = read_back.states();
    let alive: Vec<&str> = states
        .values()
        .filter(|state| state.alive)
        .filter_map(|state| state.name.as_ref().map(Name::as_str))
        .collect();
    assert_eq!(alive, vec!["one"]);

    let chain = read_back.history().changes();
    assert_eq!(chain.len(), 1, "one close, one change point");
    assert_eq!(chain[0].from(), work.id());

    let ended = core
        .pursuit_service
        .get(&work.id())
        .await
        .expect("the work");
    assert_eq!(ended.outcome(), Some(Outcome::Satisfied));
    assert_eq!(chain[0].by(), ended.head(), "one act, both logs");
}

/// The asset the line is holding cannot be purged out from under it,
/// and the refusal says what is holding it.
///
/// The whole guard, end to end: a foreign key in the schema, a check
/// in the persona repository, and a line that a service put there.
#[tokio::test(flavor = "multi_thread")]
async fn purging_the_persona_of_a_held_asset_is_refused_through_the_wiring() {
    use asterism_contract::command::{PurgePersonaCommand, TrashPersonaCommand};

    let tmp = tempfile::tempdir().expect("tempdir");
    let core = core(tmp.path()).await;
    let (persona, held) = an_asset(&core, tmp.path()).await;

    let line = core
        .line_service
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .unwrap();
    let work = core
        .pursuit_service
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .unwrap();
    core.pursuit_service
        .push(
            &work.id(),
            &persona,
            vec![Op::add(held, name("one"))],
            None,
            &who("ana"),
        )
        .await
        .unwrap();
    core.pursuit_service
        .close(&work.id(), Outcome::Satisfied, None, &who("ana"))
        .await
        .unwrap();

    let wire_id = persona.as_uuid().to_string();
    core.persona_service
        .trash(
            TrashPersonaCommand {
                persona_id: wire_id.clone(),
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("trash comes first, as everywhere else");

    let refused = core
        .persona_service
        .purge(
            PurgePersonaCommand {
                persona_id: wire_id,
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect_err("the line is holding one of its assets");
    let said = refused.to_string();
    assert!(
        said.contains("the forge is holding"),
        "the refusal says what is in the way: {said}"
    );
    assert!(said.contains("ROOT"), "and which line: {said}");
}
