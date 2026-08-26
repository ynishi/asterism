//! What a merge owes the search index.
//!
//! A fold moves text between two rows inside one transaction, and both
//! sides of it are stale the moment that transaction commits. The
//! discards become headstones — rows the grid does not show, so a hit
//! resolving to one hands the user a card they cannot open — and the
//! keeper absorbs their columns, so its own document is now missing
//! words that belong to it.
//!
//! Neither effect is visible from inside `merge_into`: the transaction
//! returns ids, and what happens to the index afterwards is the
//! application verb's to arrange. So these tests build the real service
//! over real SQLite repositories and record only the two ports the
//! claim is about — the indexer and the queue.

use std::sync::{Arc, Mutex};

use asterism_contract::command::MergeAssetsCommand;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::job::JobKind;
use asterism_core::domain::repository::{
    AssetIndexer, AssetRepository, IndexDoc, JobQueue, RetrievalQuery, Retrieved,
};
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind, SourceRef};
use asterism_core::error::DomainError;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::SqliteAssetRepository;
use uuid::Uuid;

/// A caller that states nothing, which records nothing. These fixtures
/// are about which row is folded into which, not about who ruled it.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None).expect("stating nobody is always valid")
}

/// Records enqueues instead of running them.
#[derive(Default)]
struct RecordingQueue {
    pushed: Mutex<Vec<(JobKind, serde_json::Value)>>,
}

#[async_trait::async_trait]
impl JobQueue for RecordingQueue {
    async fn enqueue(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
    ) -> Result<String, DomainError> {
        self.pushed.lock().unwrap().push((kind, payload));
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

impl RecordingQueue {
    /// The asset ids this queue was asked to re-index, in order.
    fn reindexed(&self) -> Vec<String> {
        self.pushed
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _)| *kind == JobKind::IndexRebuild)
            .filter_map(|(_, payload)| {
                payload
                    .get("asset_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect()
    }
}

/// Records removals instead of writing an index.
#[derive(Default)]
struct RecordingIndexer {
    removed: Mutex<Vec<AssetId>>,
}

#[async_trait::async_trait]
impl AssetIndexer for RecordingIndexer {
    async fn upsert(&self, _doc: &IndexDoc) -> Result<(), DomainError> {
        Ok(())
    }

    async fn remove(&self, asset_id: &AssetId) -> Result<(), DomainError> {
        self.removed.lock().unwrap().push(*asset_id);
        Ok(())
    }

    async fn flush(&self) -> Result<(), DomainError> {
        Ok(())
    }
}

/// The read side is not what these tests are about; it exists because
/// the service holds one.
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

/// Everything the verb touches, held together so a test can read the
/// two recorders afterwards.
struct Fixture {
    service: Arc<asterism_core::application::AssetService>,
    queue: Arc<RecordingQueue>,
    indexer: Arc<RecordingIndexer>,
    assets: Arc<SqliteAssetRepository>,
    persona: PersonaId,
    _previews: tempfile::TempDir,
    /// Holds the in-memory database open for the length of the test —
    /// the same shape the sibling suite's fixture uses.
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

    let queue = Arc::new(RecordingQueue::default());
    let indexer = Arc::new(RecordingIndexer::default());
    let assets = Arc::new(SqliteAssetRepository::new(isle.clone()));
    let query_groups = Arc::new(repo::SqliteQueryGroupRepository::new(isle.clone()));
    let sessions = Arc::new(asterism_core::application::SessionService::new(Arc::new(
        repo::SqliteSessionRepository::new(isle.clone()),
    )));
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
        indexer.clone(),
        Arc::new(repo::SqliteAssetBodyRepository::new(isle.clone())),
        query_groups,
        asterism_core::application::query_group_invalidation::QueryGroupInvalidator::new(
            queue.clone(),
        ),
        sessions,
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
        queue,
        indexer,
        assets,
        persona: PersonaId::from_uuid(persona_uuid),
        _previews: previews,
        _driver: driver,
    }
}

impl Fixture {
    /// Saves one live asset carrying a title, so the row has something
    /// a document could be composed from.
    async fn asset(&self, locator: &str, title: &str) -> AssetId {
        let mut asset = asterism_core::domain::asset::Asset::new(
            self.persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            None,
            chrono::Utc::now(),
            &unattributed(),
        );
        asset.title = Some(title.to_string());
        self.assets.save(&asset).await.unwrap();
        asset.id
    }
}

/// Teeth: a merge takes the folded rows out of search and puts the
/// keeper back in front of the composer.
///
/// Both halves matter and they fail for different reasons. A headstone
/// left in the index is a hit the grid cannot open. A keeper left
/// un-recomposed keeps a document that predates the words it just
/// absorbed — the discards' keywords, labels and comment threads follow
/// their rows into it, and none of them reach search until something
/// re-derives the text.
#[tokio::test]
async fn a_merge_unindexes_the_folded_rows_and_recomposes_the_keeper() {
    let fx = fixture().await;
    let keeper = fx.asset("/notes/keeper.md", "the one we keep").await;
    let discard = fx.asset("/notes/copy.md", "the one we fold").await;

    fx.service
        .merge_assets(
            MergeAssetsCommand {
                keeper_id: keeper.to_string(),
                discard_ids: vec![discard.to_string()],
                member_ids: vec![keeper.to_string(), discard.to_string()],
                dry_run: false,
            },
            &unattributed(),
        )
        .await
        .expect("the merge goes through");

    assert_eq!(
        fx.indexer.removed.lock().unwrap().clone(),
        vec![discard],
        "the folded row leaves search, and the keeper does not"
    );
    assert!(
        fx.queue.reindexed().contains(&keeper.to_string()),
        "the keeper is re-composed: {:?}",
        fx.queue.reindexed()
    );
    assert!(
        !fx.queue.reindexed().contains(&discard.to_string()),
        "and the row that no longer exists is not queued for a document"
    );

    // The fold really happened — otherwise the two assertions above
    // would hold for a verb that did nothing at all.
    let folded = fx.assets.find(&discard).await.unwrap();
    assert!(
        folded.is_none_or(|row| row.folded_into.is_some()),
        "the discard is a headstone afterwards"
    );
}

/// A preview moves nothing, so it owes the index nothing.
///
/// The same command with `dry_run` set answers the caller's "what would
/// this do" and leaves both rows live. Removing the keeper's document
/// (or the discard's) there would take rows out of search that the
/// person then decided not to merge — a preview with a side effect on
/// the thing being previewed.
#[tokio::test]
async fn a_dry_run_leaves_the_index_alone() {
    let fx = fixture().await;
    let keeper = fx.asset("/notes/keeper.md", "the one we keep").await;
    let discard = fx.asset("/notes/copy.md", "the one we fold").await;

    fx.service
        .merge_assets(
            MergeAssetsCommand {
                keeper_id: keeper.to_string(),
                discard_ids: vec![discard.to_string()],
                member_ids: vec![keeper.to_string(), discard.to_string()],
                dry_run: true,
            },
            &unattributed(),
        )
        .await
        .expect("a preview is not a failure");

    assert!(
        fx.indexer.removed.lock().unwrap().is_empty(),
        "a preview removes nothing from search"
    );
    assert!(
        fx.queue.reindexed().is_empty(),
        "and asks for no document to be recomposed"
    );
}
