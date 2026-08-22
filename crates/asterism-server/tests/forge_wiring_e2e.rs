//! The forge through the app's own wiring.
//!
//! `forge_over_ports_e2e` proves the services against a store the test
//! builds. This proves the store the *application* builds: the same
//! verbs, reached from `CoreCtx`, over the database `init_core` opened
//! and migrated.
//!
//! # What that catches and nothing else does
//!
//! A service wired to the wrong thing. A port here has more than one
//! implementation, and constructing `PursuitService` with the
//! in-memory store, or with a second `SqliteForge` over a different
//! connection, compiles and passes every test that builds its own
//! world. What it does not do is land work on a line the same process
//! can then read.
//!
//! It also answers the one thing a wiring can be wrong about without
//! anybody noticing: the forge's ports are four faces of one adapter,
//! and a close writes through two of them at once. If those were not
//! the same object over the same connection, the change point and the
//! ending would go to different places and the read below would come
//! back short.
//!
//! # What this covers that the routes do not
//!
//! A route test drives the wiring the router reaches. What is asked
//! here is whether `CoreCtx` handed the forge's ports one adapter over
//! one connection, which stays worth asking however much of the
//! surface is routed. A close writes through two of them at once, and
//! no request can tell whether those were the same object — only the row
//! that comes back short can, which is what the reads below are for.

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
    an_asset_of(core, tmp, "ana", "e2e-forge-wiring", "one.md").await
}

/// The same, for a named person and a named file — two callers means
/// two personas, which is the whole subject of one test below.
async fn an_asset_of(
    core: &CoreCtx,
    tmp: &std::path::Path,
    who: &str,
    pack: &str,
    file_name: &str,
) -> (PersonaId, Content) {
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: who.into(),
                pack_id: Some(pack.into()),
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("a persona");

    let file = tmp.join(file_name);
    std::fs::write(&file, file_name.as_bytes()).expect("a file");
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
    let (_persona, held) = an_asset(&core, tmp.path()).await;

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
            vec![Op::add(held, name("one"))],
            None,
            &who("ana"),
        )
        .await
        .expect("the boundary agrees that asset is real");
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

/// And dropping the line is the way out: what it releases is what the
/// purge was refused over, and the same purge then goes through.
///
/// The refusal above is only half a rule. A guard with no way past it
/// is a guard that turns one held asset into a persona nobody can ever
/// delete — so the half worth pinning is that the way past exists, is
/// reachable through services, and frees exactly what it said it
/// would.
///
/// Three services and the schema, in the order a person would meet
/// them: archive the line, drop it, purge the persona.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_line_releases_the_asset_and_the_purge_then_goes_through() {
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

    // An open line is not droppable, and the refusal comes from the
    // model rather than from anything the store noticed.
    let too_soon = core.line_service.discard(&line.id(), &who("ana")).await;
    assert!(
        too_soon.is_err(),
        "dropping is reachable only through the archive: {too_soon:?}"
    );

    core.line_service
        .archive(&line.id(), &who("ana"))
        .await
        .unwrap();
    let released = core
        .line_service
        .discard(&line.id(), &who("ana"))
        .await
        .expect("archived, and the work against it has ended");
    assert!(
        released.contains(&held),
        "what the purge was refused over is what the drop released"
    );

    // The line and its work are gone rather than emptied.
    assert!(core.line_service.get(&line.id()).await.is_err());
    assert!(core.pursuit_service.get(&work.id()).await.is_err());

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
    core.persona_service
        .purge(
            PurgePersonaCommand {
                persona_id: wire_id,
            },
            &AttributionContext::owner_surface(),
        )
        .await
        .expect("nothing in the forge is holding it any more");
}

/// One person's work can name another person's asset, and the line
/// takes it.
///
/// This is what the forge asking `exists` rather than `owns` means,
/// and it is the behaviour a shared line is for: private work rises
/// into something shared, so the content on a line comes from whoever
/// had it. A line carries no owner for the alternative to be measured
/// against.
///
/// It used to be refused — `push` took a `PersonaId` and the boundary
/// asked whether that persona held the content. That could not refuse
/// a caller who wanted to pass: it chose both halves of the pair, and
/// naming the asset's own persona always succeeded. What it did
/// instead was make this — the case the design is for — impossible to
/// express.
#[tokio::test(flavor = "multi_thread")]
async fn work_can_name_an_asset_that_belongs_to_somebody_else() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let core = core(tmp.path()).await;
    let (_mine, _my_asset) =
        an_asset_of(&core, tmp.path(), "ana", "e2e-forge-mine", "mine.md").await;
    let (_theirs, their_asset) =
        an_asset_of(&core, tmp.path(), "boro", "e2e-forge-theirs", "theirs.md").await;

    let line = core
        .line_service
        .open(name(Line::ROOT), MainlineFirst.id(), &who("ana"))
        .await
        .expect("a line");
    let work = core
        .pursuit_service
        .open(&line.id(), None, Intent::default(), &who("ana"))
        .await
        .expect("work against it");

    core.pursuit_service
        .push(
            &work.id(),
            vec![Op::add(their_asset, name("what boro made"))],
            None,
            &who("ana"),
        )
        .await
        .expect("whose the asset is is not this layer's question");
    core.pursuit_service
        .close(&work.id(), Outcome::Satisfied, None, &who("ana"))
        .await
        .expect("and it lands");

    let states = core
        .line_service
        .states(&line.id())
        .await
        .expect("what is on the line");
    let (_, state) = states.iter().next().expect("one entry landed");
    assert_eq!(
        state.content,
        Some(their_asset),
        "the line carries the other person's asset"
    );
}
