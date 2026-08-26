//! What a clone is recorded as, over a real database (#148 decision
//! 10).
//!
//! A clone says where it came from through `source_kind` and
//! `source_locator` "the way every other import does — and the existing
//! duplicate machinery notices when you clone something you already
//! have". That second half is a claim about `AssetService::add`, not
//! about the clone: the clone's whole contribution is that the pair it
//! hands over is built from the four ids it copied from and carries
//! nothing the caller chose, so asking twice asks the same question
//! twice.
//!
//! The fixtures below spell the locator with a fixed `.png`, which is
//! deliberate and is also what this suite cannot see. The extension is
//! the one part of a clone's path that is not an id — it comes from
//! what the line calls the entry — so a rename that changes it changes
//! the pair. That is stated at `cloned_locator` and tested nowhere,
//! because reproducing it here would mean reproducing the path builder
//! this crate must not name.
//!
//! The clone's own ordering — ask, fetch, record — is tested against a
//! live server in `teams-server/tests/clone_publish_e2e.rs`, where the
//! library it records into is a fake. This is the other half: the real
//! service, the real table, and the real lookup, asked the question the
//! clone asks.
//!
//! Nothing here names a `teams-*` crate. It does not need to — by the
//! time an arrival reaches this door there is no team in it, which is
//! the point of the seam.

use std::sync::{Arc, Mutex};

use asterism_contract::command::AddAssetCommand;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::job::JobKind;
use asterism_core::domain::repository::{
    AssetIndexer, AssetRepository, IndexDoc, JobQueue, RetrievalQuery, Retrieved, SourceLookupScope,
};
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind};
use asterism_core::error::DomainError;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::SqliteAssetRepository;
use uuid::Uuid;

/// A caller that states nothing. Who cloned is the surface's answer and
/// not this suite's subject.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None).expect("stating nobody is always valid")
}

#[derive(Default)]
struct SilentQueue(Mutex<Vec<JobKind>>);

#[async_trait::async_trait]
impl JobQueue for SilentQueue {
    async fn enqueue(
        &self,
        kind: JobKind,
        _payload: serde_json::Value,
    ) -> Result<String, DomainError> {
        self.0.lock().unwrap().push(kind);
        Ok("task-1".into())
    }

    async fn enqueue_with_priority(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
        _priority: i32,
    ) -> Result<String, DomainError> {
        self.enqueue(kind, payload).await
    }

    async fn has_pending_batch(&self, _kind: JobKind) -> Result<bool, DomainError> {
        Ok(false)
    }
}

struct SilentIndexer;

#[async_trait::async_trait]
impl AssetIndexer for SilentIndexer {
    async fn upsert(&self, _doc: &IndexDoc) -> Result<(), DomainError> {
        Ok(())
    }

    async fn remove(&self, _asset_id: &AssetId) -> Result<(), DomainError> {
        Ok(())
    }

    async fn flush(&self) -> Result<(), DomainError> {
        Ok(())
    }
}

struct EmptyRetriever;

#[async_trait::async_trait]
impl asterism_core::domain::repository::AssetRetriever for EmptyRetriever {
    async fn retrieve(&self, _query: &RetrievalQuery) -> Result<Retrieved, DomainError> {
        Ok(Retrieved {
            candidates: Vec::new(),
            truncated: false,
        })
    }
}

struct Fixture {
    service: Arc<asterism_core::application::AssetService>,
    assets: Arc<SqliteAssetRepository>,
    queue: Arc<SilentQueue>,
    persona: PersonaId,
    _previews: tempfile::TempDir,
    _driver: rusqlite_isle::AsyncIsleDriver,
}

async fn fixture() -> Fixture {
    use asterism_infra::sqlite::repo;

    let (isle, driver) = open_and_migrate_in_memory().await.unwrap();

    let persona_uuid = Uuid::now_v7();
    isle.call(move |conn| {
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, 'p', 'P', 0, 0)",
            rusqlite::params![persona_uuid],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let queue = Arc::new(SilentQueue::default());
    let assets = Arc::new(SqliteAssetRepository::new(isle.clone()));
    let previews = tempfile::tempdir().unwrap();
    let service = Arc::new(asterism_core::application::AssetService::new(
        assets.clone(),
        Arc::new(repo::SqlitePersonaRepository::new(isle.clone())),
        Arc::new(repo::SqliteTagRepository::new(isle.clone())),
        Arc::new(repo::group::SqliteGroupRepository::new(isle.clone())),
        Arc::new(repo::SqliteAssetCommentRepository::new(isle.clone())),
        Arc::new(repo::SqliteDirRepository::new(isle.clone())),
        Arc::new(repo::SqliteEdgeRepository::new(isle.clone())),
        Arc::new(repo::SqliteSnapshotRepository::new(isle.clone())),
        Arc::new(repo::SqliteDispatchRepository::new(isle.clone())),
        Arc::new(asterism_infra::source_text::FsSourceTextReader::new()),
        queue.clone(),
        Arc::new(EmptyRetriever),
        Arc::new(SilentIndexer),
        Arc::new(repo::SqliteAssetBodyRepository::new(isle.clone())),
        Arc::new(repo::SqliteQueryGroupRepository::new(isle.clone())),
        asterism_core::application::query_group_invalidation::QueryGroupInvalidator::new(
            queue.clone(),
        ),
        Arc::new(asterism_core::application::SessionService::new(Arc::new(
            repo::SqliteSessionRepository::new(isle.clone()),
        ))),
        previews.path().to_path_buf(),
        Arc::new(repo::SqliteTagEvidenceRepository::new(isle.clone())),
        Arc::new(std::sync::OnceLock::new()),
        Arc::new(asterism_infra::heads::FsTagHeadStore::new(
            previews.path().join("heads"),
        )),
        Arc::new(std::sync::OnceLock::new()),
    ));

    Fixture {
        service,
        assets,
        queue,
        persona: PersonaId::from_uuid(persona_uuid),
        _previews: previews,
        _driver: driver,
    }
}

/// The pair a clone hands over, spelled the way the clone spells it.
///
/// Kept here as a literal rather than by calling
/// `asterism_teams_client::clone::cloned_locator`, because that crate
/// is not on this one's graph and must not be — #83 §4 puts
/// `asterism-infra` first on the never-list, and a test that reached
/// across for convenience would be the edge itself. What is being
/// checked is a property of the shape, and the shape is four ids under
/// a root.
fn cloned(root: &str, team: Uuid, line: Uuid, entry: Uuid, team_asset: Uuid) -> String {
    format!("{root}/{team}/{line}/{entry}/{team_asset}.png")
}

fn arrival(persona: &PersonaId, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona.to_string(),
        source_kind: SourceKind::TEAM_LINE.into(),
        locator: locator.to_string(),
        modality: None,
        occurred_at_ms: 1_700_000_000_000,
        session_id: None,
        external_session_key: None,
        external_key: None,
        bundle_id: None,
        labels: Vec::new(),
        register_note: None,
        platform: None,
        file_size_bytes: Some(12),
        duration_ms: None,
        width_px: None,
        height_px: None,
        extra_json: None,
        cover_hint: Some("what the promoter called it".into()),
        auto_organize_base_dir: None,
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

/// The claim decision 10 makes, against the real lookup.
#[tokio::test]
async fn cloning_the_same_entry_twice_lands_one_asset() {
    let f = fixture().await;
    let (team, line, entry, team_asset) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let locator = cloned("/clones", team, line, entry, team_asset);

    let first = f
        .service
        .add(arrival(&f.persona, &locator), &unattributed())
        .await
        .expect("the first clone lands");
    let again = f
        .service
        .add(arrival(&f.persona, &locator), &unattributed())
        .await
        .expect("the second is answered");

    assert_eq!(
        first.id, again.id,
        "cloning the same entry twice minted two assets; the pair is what the lookup \
         compares and it did not match"
    );

    let held = f
        .assets
        .find_by_source(
            &f.persona,
            &SourceKind::new(SourceKind::TEAM_LINE).unwrap(),
            &SourceLocator::from_wire(&locator).unwrap(),
            SourceLookupScope::Live,
        )
        .await
        .unwrap()
        .expect("the row is findable by what it says about where it came from");
    assert_eq!(held.id.to_string(), first.id);
}

/// The leaf is the team's asset, and that is what makes a replacement a
/// different copy rather than a stale hit on the old one.
#[tokio::test]
async fn a_replaced_entry_clones_as_its_own_asset() {
    let f = fixture().await;
    let (team, line, entry) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    let before = f
        .service
        .add(
            arrival(
                &f.persona,
                &cloned("/clones", team, line, entry, Uuid::now_v7()),
            ),
            &unattributed(),
        )
        .await
        .unwrap();
    let after = f
        .service
        .add(
            arrival(
                &f.persona,
                &cloned("/clones", team, line, entry, Uuid::now_v7()),
            ),
            &unattributed(),
        )
        .await
        .unwrap();

    assert_ne!(
        before.id, after.id,
        "the entry's content was replaced, and cloning it again handed back the copy of \
         what it used to hold"
    );
}

/// Two teams that happen to have named an entry the same thing are two
/// different things to copy.
#[tokio::test]
async fn the_same_entry_id_in_another_team_is_another_copy() {
    let f = fixture().await;
    let (line, entry, team_asset) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    let here = f
        .service
        .add(
            arrival(
                &f.persona,
                &cloned("/clones", Uuid::now_v7(), line, entry, team_asset),
            ),
            &unattributed(),
        )
        .await
        .unwrap();
    let there = f
        .service
        .add(
            arrival(
                &f.persona,
                &cloned("/clones", Uuid::now_v7(), line, entry, team_asset),
            ),
            &unattributed(),
        )
        .await
        .unwrap();

    assert_ne!(here.id, there.id);
}

/// A clone's locator is a path, and the reason is that everything
/// downstream that wants bytes asks this question.
///
/// The other locator kinds would say where the copy came from more
/// literally and answer `None` here, which costs the copy its hash, its
/// thumbnail, its cover text and its onward promotion — silently, one
/// reader at a time.
#[tokio::test]
async fn a_clone_has_bytes_a_reader_can_find() {
    let f = fixture().await;
    let locator = cloned(
        "/clones",
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );

    let landed = f
        .service
        .add(arrival(&f.persona, &locator), &unattributed())
        .await
        .unwrap();

    let value = SourceLocator::from_wire(&locator).unwrap();
    assert_eq!(
        value.local_path().map(|p| p.display().to_string()),
        Some(locator.clone()),
        "the clone's locator did not read back as a file"
    );

    let held = f
        .assets
        .find(&AssetId::from_uuid(Uuid::parse_str(&landed.id).unwrap()))
        .await
        .unwrap()
        .expect("the row");
    let material = held.materials.first().expect("a primary material attached");
    assert_eq!(
        material
            .locator
            .local_path()
            .map(|p| p.display().to_string()),
        Some(locator),
        "the material a reader opens is not the copy on disk"
    );

    assert!(
        f.queue.0.lock().unwrap().contains(&JobKind::MaterialHash),
        "nothing was queued to hash the copy, so its fingerprint never arrives"
    );
}
