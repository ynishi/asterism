//! Duplicate detection over real storage: what the second copy of a
//! file does, per declared strategy.
//!
//! The unit tests beside the function cover the two pure decisions (how
//! absence resolves, what the backfill refuses to do on its own). This
//! binary covers the part that needs a database: the lookup, the edge,
//! the queue row, and the fold enqueue — with the SQLite adapters the
//! job worker actually runs against and a job queue stub that records
//! what it was handed.
//!
//! Every fixture **seeds the incumbent first and asserts it is
//! findable** before the second copy is fingerprinted. A fixture with
//! one row would let every assertion below pass with detection deleted.

use std::sync::Mutex;

use asterism_core::application_support::duplicate_detection::{
    Detection, DetectionOrigin, DetectionPorts, LINEAGE_PROBE_BUDGET, detect_duplicate,
};
use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::axis_status::{AxisRecord, AxisStatus};
use asterism_core::domain::content_hash::{EMPTY, UNHASHABLE};
use asterism_core::domain::duplicate_conflict::{
    ConflictResolution, DuplicateAxis, DuplicateConflict, FoldExclusion,
};
use asterism_core::domain::edge::{ConstellationEdge, EdgeKind};
use asterism_core::domain::job::JobKind;
use asterism_core::domain::material::Material;
use asterism_core::domain::repository::{
    AssetRepository, EdgeRepository, JobQueue, MaterialFingerprint,
};
use asterism_core::domain::value::{
    AssetId, FoldPolicy, Modality, OnDuplicate, PersonaId, SourceKind, SourceRef,
};
use asterism_core::error::DomainError;
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::{SqliteAssetRepository, SqliteEdgeRepository};
use chrono::{DateTime, Utc};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about which row is the
/// duplicate of which, not about who registered either one.
fn unattributed() -> AttributionContext {
    AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// A job queue that records rather than runs — the fold branch's only
/// observable effect is what it enqueues.
#[derive(Default)]
struct RecordingQueue {
    pushed: Mutex<Vec<(JobKind, serde_json::Value)>>,
    /// When set, every push fails. Used to show that a detection which
    /// falls over leaves the fingerprint alone.
    fail: bool,
}

#[async_trait::async_trait]
impl JobQueue for RecordingQueue {
    async fn enqueue(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
    ) -> Result<String, DomainError> {
        if self.fail {
            return Err(DomainError::Infra(anyhow::anyhow!("queue is down")));
        }
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
    fn folds(&self) -> Vec<serde_json::Value> {
        self.pushed
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _)| *kind == JobKind::AssetFold)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The fingerprint the fixtures below detect on: [`DIGEST`] on the
/// artefact axis, an answered status on the two walking ones.
fn artefact_only(digest: &str) -> MaterialFingerprint {
    MaterialFingerprint {
        file: AxisRecord::computed(digest.to_string()),
        content: AxisRecord::bare(AxisStatus::EmptySpan),
        meta: AxisRecord::bare(AxisStatus::EmptySpan),
        meta_kv: None,
        meta_raw: None,
        meta_text: None,
    }
}

fn at(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap()
}

async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
    let pid = Uuid::now_v7();
    let pack = format!("pack-{pid}");
    isle.call(move |conn| {
        conn.execute(
            "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
             VALUES (?1, ?2, 'P', 0, 0)",
            rusqlite::params![pid, pack],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    PersonaId::from_uuid(pid)
}

/// One asset with its primary material already fingerprinted to
/// `digest`, registered with `declared`.
async fn copy_of(
    repo: &SqliteAssetRepository,
    persona: PersonaId,
    locator: &str,
    digest: &str,
    occurred_at: DateTime<Utc>,
    declared: Option<OnDuplicate>,
) -> Asset {
    hashed_row(
        repo,
        persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        locator,
        digest,
        occurred_at,
        declared,
    )
    .await
}

/// The same, minted the way a dispatch's output is: `source_kind` built
/// by the domain factory rather than a string spelled here, so the test
/// and the rule read the convention from the same place.
async fn export_of(
    repo: &SqliteAssetRepository,
    persona: PersonaId,
    exporter_slug: &str,
    locator: &str,
    digest: &str,
    occurred_at: DateTime<Utc>,
    declared: Option<OnDuplicate>,
) -> Asset {
    hashed_row(
        repo,
        persona,
        SourceKind::for_dispatch(exporter_slug).unwrap(),
        locator,
        digest,
        occurred_at,
        declared,
    )
    .await
}

async fn hashed_row(
    repo: &SqliteAssetRepository,
    persona: PersonaId,
    kind: SourceKind,
    locator: &str,
    digest: &str,
    occurred_at: DateTime<Utc>,
    declared: Option<OnDuplicate>,
) -> Asset {
    fingerprinted_row(
        repo,
        persona,
        kind,
        locator,
        &MaterialFingerprint {
            file: AxisRecord::computed(digest.to_string()),
            // These fixtures ask an artefact-axis question, so the two
            // walking axes are answered with a status: the row leaves
            // the fingerprint walk the way a real pass would leave it,
            // and a status carries no digest, so neither of those
            // axes finds anything and the assertions are about the axis
            // under test. `the_content_axis_fires_on_a_metadata_only_-
            // difference` and `the_meta_axis_fires_on_a_pixels_only_-
            // difference` are the fixtures that put real digests here.
            content: AxisRecord::bare(AxisStatus::EmptySpan),
            meta: AxisRecord::bare(AxisStatus::EmptySpan),
            meta_kv: None,
            meta_raw: None,
            meta_text: None,
        },
        occurred_at,
        declared,
    )
    .await
}

/// One asset whose primary material carries `fingerprint` on both axes.
async fn fingerprinted_row(
    repo: &SqliteAssetRepository,
    persona: PersonaId,
    kind: SourceKind,
    locator: &str,
    fingerprint: &MaterialFingerprint,
    occurred_at: DateTime<Utc>,
    declared: Option<OnDuplicate>,
) -> Asset {
    let mut asset = Asset::new(
        persona,
        SourceRef::new(kind, locator).unwrap(),
        Some(Modality::new("tape").unwrap()),
        occurred_at,
        &unattributed(),
    );
    asset.on_duplicate = declared;
    let mut material = Material::primary(asset.source.locator.clone(), Some(1), occurred_at);
    material.mime = Some(asterism_core::domain::value::MimeType::parse("image/png"));
    asset.materials = vec![material];
    repo.save(&asset).await.unwrap();
    repo.set_material_fingerprint(&asset.id, 0, fingerprint)
        .await
        .unwrap();
    asset
}

struct Fixture {
    _driver: rusqlite_isle::AsyncIsleDriver,
    isle: AsyncIsle,
    assets: SqliteAssetRepository,
    edges: SqliteEdgeRepository,
    persona: PersonaId,
}

impl Fixture {
    async fn new() -> Self {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let assets = SqliteAssetRepository::new(isle.clone());
        let edges = SqliteEdgeRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        Self {
            _driver: driver,
            isle,
            assets,
            edges,
            persona,
        }
    }

    /// The ports, wired to this fixture's storage.
    fn ports<'a>(&'a self, queue: &'a RecordingQueue) -> DetectionPorts<'a> {
        DetectionPorts {
            assets: &self.assets,
            edges: &self.edges,
            queue,
        }
    }

    /// Detects on `asset`'s primary material, as the given pass would.
    async fn detect(
        &self,
        queue: &RecordingQueue,
        asset: &Asset,
        origin: DetectionOrigin,
    ) -> Detection {
        self.detect_fingerprint(queue, asset, &artefact_only(DIGEST), origin)
            .await
    }

    /// The same, for a fixture that has something on the content axis.
    async fn detect_fingerprint(
        &self,
        queue: &RecordingQueue,
        asset: &Asset,
        fingerprint: &MaterialFingerprint,
        origin: DetectionOrigin,
    ) -> Detection {
        detect_duplicate(
            self.ports(queue),
            &asset.id,
            0,
            fingerprint,
            origin,
            at(9_000),
        )
        .await
        .expect("detection ran")
    }

    async fn open_questions(&self) -> Vec<(AssetId, AssetId)> {
        self.assets
            .list_open_duplicate_conflicts(Some(&self.persona), 50)
            .await
            .unwrap()
            .into_iter()
            .map(|c| (c.newcomer, c.incumbent))
            .collect()
    }

    /// The rule each open question records as having declined an
    /// automatic fold — what the panel needs in order to say more than
    /// "these two look the same".
    async fn declined_folds(&self) -> Vec<Option<FoldExclusion>> {
        self.assets
            .list_open_duplicate_conflicts(Some(&self.persona), 50)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.fold_exclusion)
            .collect()
    }

    /// Declares `child` as derived from `parent`, the direction reify
    /// and the correlation-ingest claim both write.
    async fn derive(&self, child: &Asset, parent: &Asset) {
        self.edges
            .add_edges(vec![
                ConstellationEdge::new(child.id, parent.id, EdgeKind::DerivedFrom).unwrap(),
            ])
            .await
            .unwrap();
    }

    /// A row with nothing on it but an id — something for an edge to
    /// point at.
    async fn bare_row(&self, locator: &str, occurred_at: DateTime<Utc>) -> Asset {
        let asset = Asset::new(
            self.persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            Some(Modality::new("tape").unwrap()),
            occurred_at,
            &unattributed(),
        );
        self.assets.save(&asset).await.unwrap();
        asset
    }

    /// `(from, to, label)` of every `identical_to` edge in the database.
    async fn identical_edges(&self) -> Vec<(Uuid, Uuid, Option<String>)> {
        self.isle
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT from_asset, to_asset, label FROM edge \
                      WHERE kind = 'identical_to' ORDER BY rowid",
                )?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<_, _>>()
            })
            .await
            .unwrap()
    }

    /// Rules one row a thing of its own.
    ///
    /// Written in SQL because `save` deliberately does not carry
    /// `fold_policy`: the column is a person's ruling, and a metadata
    /// round-trip must not be able to restate it. The verb that will
    /// write it belongs to the resolution surface, which is a later
    /// wave — so a fixture that needs the ruling sets the column the
    /// same way `a_folded_row_survives_a_whole_row_save` does.
    async fn rule_apart(&self, asset: &AssetId) {
        let id = *asset.as_uuid();
        self.isle
            .call(move |conn| {
                let touched = conn.execute(
                    "UPDATE asset SET fold_policy = 'keep' WHERE id = ?1",
                    rusqlite::params![id],
                )?;
                assert_eq!(touched, 1, "the ruling landed on a row that exists");
                Ok(())
            })
            .await
            .unwrap();
    }

    /// The one question on the panel, whole. Fails loudly on any other
    /// count — a resolution test that answered "some" question would
    /// assert nothing.
    async fn only_open_conflict(&self) -> DuplicateConflict {
        let mut open = self
            .assets
            .list_open_duplicate_conflicts(Some(&self.persona), 50)
            .await
            .unwrap();
        assert_eq!(open.len(), 1, "exactly one open question");
        open.remove(0)
    }

    async fn hash_of(&self, asset: &AssetId) -> Option<String> {
        self.assets
            .find(asset)
            .await
            .unwrap()
            .unwrap()
            .materials
            .first()
            .unwrap()
            .content_hash
            .clone()
    }
}

/// The first copy of anything raises nothing. The lookup returns the row
/// that was just fingerprinted, and mistaking that for a match is how a
/// library folds every asset into itself.
#[tokio::test]
async fn the_first_copy_finds_only_itself() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let only = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/a.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    assert_eq!(
        fx.assets
            .find_by_content_hash(&fx.persona, DuplicateAxis::Artefact, DIGEST)
            .await
            .unwrap()
            .len(),
        1,
        "the fixture holds exactly one holder of these bytes"
    );

    assert_eq!(
        fx.detect(&queue, &only, DetectionOrigin::Ingest).await,
        Detection::Unique
    );
    assert!(fx.identical_edges().await.is_empty());
    assert!(fx.open_questions().await.is_empty());
    assert!(queue.folds().is_empty());
}

/// The default path: nobody declared anything, so the question goes to a
/// person — and the column stays `NULL`, because resolving absence is
/// the detector's job and writing the answer in would forge a request
/// nobody made.
#[tokio::test]
async fn an_undeclared_second_copy_queues_the_question() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    assert_eq!(
        fx.assets
            .find_by_content_hash(&fx.persona, DuplicateAxis::Artefact, DIGEST)
            .await
            .unwrap()
            .len(),
        1,
        "the incumbent is there before the copy arrives"
    );
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        None,
    )
    .await;

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id)
    );
    assert_eq!(
        fx.open_questions().await,
        vec![(newcomer.id, incumbent.id)],
        "the question names the arrival and the row that was already there"
    );
    assert_eq!(
        fx.identical_edges().await,
        vec![(
            *newcomer.id.as_uuid(),
            *incumbent.id.as_uuid(),
            Some(DuplicateAxis::Artefact.as_str().to_string())
        )],
        "newcomer → incumbent, labelled with the axis that agreed"
    );
    assert!(queue.folds().is_empty(), "ask acts on nothing by itself");

    // The registration is still undeclared. This is the half that keeps
    // a future lane default meaningful.
    assert_eq!(
        fx.assets
            .find(&newcomer.id)
            .await
            .unwrap()
            .unwrap()
            .on_duplicate,
        None
    );
}

/// `ask` declared explicitly does exactly what absence resolves to —
/// same queue row, same edge, same lack of a fold. The two are
/// *stored* differently on purpose; they must not *behave* differently
/// while `ask` is the only default there is.
#[tokio::test]
async fn a_declared_ask_matches_the_undeclared_default() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Ask),
    )
    .await;

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id)
    );
    assert_eq!(fx.open_questions().await, vec![(newcomer.id, incumbent.id)]);
    assert_eq!(fx.identical_edges().await.len(), 1);
    assert!(queue.folds().is_empty());
    assert_eq!(
        fx.assets
            .find(&newcomer.id)
            .await
            .unwrap()
            .unwrap()
            .on_duplicate,
        Some(OnDuplicate::Ask),
        "a declared strategy is still readable as declared"
    );
}

/// `fold` acts without asking: the match is recorded and an
/// `asset_fold` job is enqueued to put the newcomer into the oldest
/// holder. Nothing goes on the queue — there is no question left.
#[tokio::test]
async fn a_declared_fold_enqueues_the_fold_and_asks_nothing() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    // A middle-aged holder, so "the oldest" is a choice the assertion
    // can see rather than the only row available.
    let middle = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/second.png",
        DIGEST,
        at(1_500),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    assert_eq!(
        fx.assets
            .find_by_content_hash(&fx.persona, DuplicateAxis::Artefact, DIGEST)
            .await
            .unwrap()
            .len(),
        3
    );

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Folding(incumbent.id)
    );
    assert_eq!(
        queue.folds(),
        vec![serde_json::json!({
            "asset_id": newcomer.id.to_string(),
            "keeper_id": incumbent.id.to_string(),
        })],
        "the keeper is the oldest holder, not the nearest one"
    );
    assert_ne!(incumbent.id, middle.id);
    assert!(
        fx.open_questions().await.is_empty(),
        "a lane that said fold is not asked to confirm"
    );
    assert_eq!(
        fx.identical_edges().await.len(),
        1,
        "the match is still recorded"
    );
}

/// `separate` records the coincidence and stops. The lane produces
/// identical material on purpose; queueing it would fill the panel with
/// questions whose answer is already on the row.
#[tokio::test]
async fn a_declared_separate_records_the_match_and_stops() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/variant.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Separate),
    )
    .await;

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Recorded(incumbent.id)
    );
    assert_eq!(
        fx.identical_edges().await.len(),
        1,
        "the bytes agreed, and that stays traceable"
    );
    assert!(fx.open_questions().await.is_empty());
    assert!(queue.folds().is_empty());
}

/// Values that sit in the hash column without standing for bytes never
/// raise anything. Every fragment of every conversation log shares the
/// one `unhashable:` marker, and every empty file shares the empty
/// digest — the known failure shape is a whole corpus reported as one
/// duplicate.
#[tokio::test]
async fn markers_never_conflict_however_many_rows_share_them() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    // Both axes' markers, and the crossed pair: whichever column a
    // value lands in, a marker never becomes a key.
    for marker in [
        UNHASHABLE,
        EMPTY,
        asterism_core::domain::content_region::EMPTY_SPAN,
    ] {
        let mut rows = Vec::new();
        for i in 0..3 {
            rows.push(
                copy_of(
                    &fx.assets,
                    fx.persona,
                    &format!("/logs/{marker}-{i}.jsonl"),
                    marker,
                    at(1_000 + i),
                    None,
                )
                .await,
            );
        }
        // The rows really do share the value — without this the
        // assertion below would pass over an empty table.
        let sharing: i64 = fx
            .isle
            .call({
                let marker = marker.to_string();
                move |conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM material WHERE content_hash = ?1",
                        rusqlite::params![marker],
                        |r| r.get(0),
                    )
                }
            })
            .await
            .unwrap();
        assert_eq!(sharing, 3, "three rows carry {marker}");

        for row in &rows {
            let outcome = detect_duplicate(
                fx.ports(&queue),
                &row.id,
                0,
                // `computed` around a value that is not this axis's
                // digest: the shape a hand-edited row or a careless
                // writer produces, which is exactly what the reserved
                // exclusion has to keep refusing.
                &MaterialFingerprint {
                    file: AxisRecord::computed(marker.to_string()),
                    content: AxisRecord::computed(marker.to_string()),
                    meta: AxisRecord::computed(marker.to_string()),
                    meta_kv: None,
                    meta_text: None,
                    meta_raw: None,
                },
                DetectionOrigin::Ingest,
                at(9_000),
            )
            .await
            .expect("a marker is not an error, it is simply not a key");
            assert_eq!(outcome, Detection::NotApplicable, "{marker}");
        }
    }

    assert!(fx.identical_edges().await.is_empty());
    assert!(fx.open_questions().await.is_empty());
    assert!(queue.folds().is_empty());
}

/// Detecting the same pair twice — a re-run of the job, or the other
/// end of the pair being fingerprinted — leaves one question and one
/// edge.
#[tokio::test]
async fn re_detection_does_not_multiply_the_queue() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        None,
    )
    .await;

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id)
    );
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::AlreadyQueued(incumbent.id),
        "the second run says so rather than queueing again"
    );
    // …and from the other end, which is what the backfill produces when
    // it reaches the older row second.
    assert_eq!(
        fx.detect(&queue, &incumbent, DetectionOrigin::Backfill)
            .await,
        Detection::AlreadyQueued(incumbent.id),
        "the oldest holder is the incumbent whichever row is being hashed"
    );

    assert_eq!(fx.open_questions().await.len(), 1);
    assert_eq!(fx.identical_edges().await.len(), 1);
}

/// **An answered question stays answered, and its pair is never asked
/// about again** — without anything being written to either asset.
///
/// This is the fact the `kept` ruling is built on. The queue's key is
/// `(pair_lo, pair_hi, axis)` with no `resolved_at` in it, so a
/// re-detection of a pair that already has a row — open or closed —
/// inserts nothing; and the listing skips answered rows. Together
/// those two mean the closed row alone keeps the pair off the panel.
///
/// So the resolution verb does **not** set `fold_policy = keep`, and
/// the last assertions here are what that decision rests on: the rows
/// come out of a `kept` ruling exactly as unruled as they went in. That
/// column suppresses every pair its row takes part in, which is a
/// wider claim than the one somebody made about these two.
///
/// Kill either half — make the closed row invisible to the insert, or
/// let the listing return answered rows — and this fails.
#[tokio::test]
async fn an_answered_pair_is_closed_and_never_asked_again() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        None,
    )
    .await;
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id)
    );
    let raised = fx.only_open_conflict().await;
    assert!(raised.is_open(), "freshly raised, unanswered");

    // Ruled two separate things.
    assert!(
        fx.assets
            .close_duplicate_conflict(&raised.id, ConflictResolution::Kept, at(3_000))
            .await
            .unwrap(),
        "the open row was closed by this call"
    );

    assert!(
        fx.open_questions().await.is_empty(),
        "an answered question is off the panel"
    );
    // Closed, not deleted: the record that it was raised and ruled on
    // is what a later reader traces the decision through.
    let closed = fx
        .assets
        .find_duplicate_conflict(&raised.id)
        .await
        .unwrap()
        .expect("the row is still there");
    assert_eq!(closed.resolution, Some(ConflictResolution::Kept));
    assert_eq!(closed.resolved_at, Some(at(3_000)));
    assert!(!closed.is_open());

    // The pair, re-detected from both ends — a re-run of the job, and
    // the backfill reaching the older row second. Neither raises it.
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::AlreadyQueued(incumbent.id)
    );
    assert_eq!(
        fx.detect(&queue, &incumbent, DetectionOrigin::Backfill)
            .await,
        Detection::AlreadyQueued(incumbent.id)
    );
    assert!(
        fx.open_questions().await.is_empty(),
        "the ruled pair does not come back"
    );
    assert!(queue.folds().is_empty(), "a kept ruling folds nothing");

    // And neither row acquired a claim about every *other* pair it
    // might turn out to be in.
    for side in [newcomer.id, incumbent.id] {
        assert_eq!(
            fx.assets.find(&side).await.unwrap().unwrap().fold_policy,
            FoldPolicy::Auto,
            "the ruling was about this pair, so nothing was written to the row"
        );
    }
}

/// Two panels answering at once: the first answer stands and the second
/// is told it wrote nothing.
///
/// The verb reads the row, decides what it is answering, then writes —
/// and the write is conditional on the row still being open, which is
/// what makes the gap between those two steps safe. Drop the
/// `resolved_at IS NULL` from the update and the second answer silently
/// replaces the first.
#[tokio::test]
async fn the_second_answer_to_one_question_writes_nothing() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        None,
    )
    .await;
    fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await;
    let raised = fx.only_open_conflict().await;

    assert!(
        fx.assets
            .close_duplicate_conflict(&raised.id, ConflictResolution::Kept, at(3_000))
            .await
            .unwrap()
    );
    assert!(
        !fx.assets
            .close_duplicate_conflict(&raised.id, ConflictResolution::Folded, at(4_000))
            .await
            .unwrap(),
        "the row was not open, so this call wrote nothing"
    );

    let closed = fx
        .assets
        .find_duplicate_conflict(&raised.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (closed.resolution, closed.resolved_at),
        (Some(ConflictResolution::Kept), Some(at(3_000))),
        "the first answer is the one on record"
    );
    // The incumbent is untouched by the refused second answer.
    assert!(
        fx.assets
            .find(&incumbent.id)
            .await
            .unwrap()
            .unwrap()
            .folded_into
            .is_none()
    );
}

/// A row somebody ruled a thing of its own suppresses the queue and the
/// fold **from either side** — and keeps the edge, which is what stops
/// the pair from being rediscovered as news.
#[tokio::test]
async fn a_keep_ruling_suppresses_the_question_from_either_side() {
    // The incumbent was ruled apart, and the newcomer's lane says fold.
    // Nothing folds: the ruling is about the row, in whichever
    // direction the pair is asked about.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    fx.rule_apart(&incumbent.id).await;
    assert_eq!(
        fx.assets
            .find(&incumbent.id)
            .await
            .unwrap()
            .unwrap()
            .fold_policy,
        FoldPolicy::Keep,
        "the ruling is on the row before anything is detected"
    );

    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Recorded(incumbent.id)
    );
    assert!(
        queue.folds().is_empty(),
        "a row ruled apart is not folded, whatever the lane declared"
    );
    assert!(fx.open_questions().await.is_empty());
    assert_eq!(
        fx.identical_edges().await.len(),
        1,
        "the byte-level fact is exactly what keeps the ruling from being re-asked"
    );

    // The other side: this time the arrival carries the ruling.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();
    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        None,
    )
    .await;
    fx.rule_apart(&newcomer.id).await;
    assert_eq!(
        fx.assets
            .find(&newcomer.id)
            .await
            .unwrap()
            .unwrap()
            .fold_policy,
        FoldPolicy::Keep,
        "this time the arrival is the row that was ruled apart"
    );

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Recorded(incumbent.id)
    );
    assert!(fx.open_questions().await.is_empty());
    assert_eq!(fx.identical_edges().await.len(), 1);
}

/// The backfill never folds on its own. Both rows have been in the
/// library long enough to be used, and folding two rows somebody has
/// been working with is always a confirmed act — so the same fixture
/// that folds on ingest queues on the backfill.
#[tokio::test]
async fn the_backfill_queues_what_ingest_would_fold() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Backfill)
            .await,
        Detection::Queued(incumbent.id),
        "found by the walk, so it is asked rather than acted on"
    );
    assert!(queue.folds().is_empty());
    assert_eq!(fx.open_questions().await.len(), 1);

    // The contrast that makes the rule deliberate rather than an
    // accident of this fixture: the identical pair, found on ingest,
    // folds.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();
    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Folding(incumbent.id)
    );
    assert!(fx.open_questions().await.is_empty());
}

/// A detection that falls over leaves the fingerprint where it is.
///
/// The digest is an observation about bytes and it is already written;
/// a conflict is a derivation from it. Undoing the observation because
/// the derivation failed would put the row back in front of every future
/// backfill pass — a walk that re-reads the whole file forever — while
/// the unraised conflict is raised again the next time either side is
/// fingerprinted.
#[tokio::test]
async fn a_failed_detection_leaves_the_hash_alone() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue {
        fail: true,
        ..Default::default()
    };

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;

    let err = detect_duplicate(
        fx.ports(&queue),
        &newcomer.id,
        0,
        &artefact_only(DIGEST),
        DetectionOrigin::Ingest,
        at(9_000),
    )
    .await
    .expect_err("the queue was down");
    assert!(matches!(err, DomainError::Infra(_)), "{err:?}");

    assert_eq!(fx.hash_of(&newcomer.id).await.as_deref(), Some(DIGEST));
    assert_eq!(fx.hash_of(&incumbent.id).await.as_deref(), Some(DIGEST));
    assert!(fx.open_questions().await.is_empty());
}

/// A material that is not the asset's primary one is out of the key.
/// The RAW beside a JPEG holding the same bytes as somebody's primary is
/// not two of the same asset, and folding on it would discard the row
/// that owns the other resource.
#[tokio::test]
async fn a_secondary_material_is_not_a_duplicate_key() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/copy.png",
        DIGEST,
        at(2_000),
        None,
    )
    .await;
    // The same call at ord 0 does raise the question — so what the
    // assertion below measures is the ord, not the fixture.
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id)
    );

    let secondary = detect_duplicate(
        fx.ports(&queue),
        &newcomer.id,
        2,
        &artefact_only(DIGEST),
        DetectionOrigin::Ingest,
        at(9_000),
    )
    .await
    .unwrap();
    assert_eq!(secondary, Detection::NotApplicable);
}

/// The edge is written on every branch, which is what makes it the
/// record of the fact rather than the record of a decision. Asserted
/// across all three strategies in one place so a branch that quietly
/// stopped writing it is visible as a difference between the arms.
#[tokio::test]
async fn every_strategy_records_the_match() {
    for declared in [
        None,
        Some(OnDuplicate::Ask),
        Some(OnDuplicate::Fold),
        Some(OnDuplicate::Separate),
    ] {
        let fx = Fixture::new().await;
        let queue = RecordingQueue::default();
        let incumbent = copy_of(
            &fx.assets,
            fx.persona,
            "/pics/original.png",
            DIGEST,
            at(1_000),
            None,
        )
        .await;
        let newcomer = copy_of(
            &fx.assets,
            fx.persona,
            "/pics/copy.png",
            DIGEST,
            at(2_000),
            declared,
        )
        .await;
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await;

        assert_eq!(
            fx.identical_edges().await,
            vec![(
                *newcomer.id.as_uuid(),
                *incumbent.id.as_uuid(),
                Some("artefact".to_string())
            )],
            "{declared:?} did not record the match"
        );
        // And it is the asserted kind, so a constellation rebuild cannot
        // take it as collateral.
        assert!(!EdgeKind::IdenticalTo.is_synth());
        assert_eq!(
            fx.edges
                .edges_incident(&incumbent.id, Some(EdgeKind::IdenticalTo), 10)
                .await
                .unwrap()
                .len(),
            1,
            "{declared:?}: the incumbent can see the link too"
        );
    }
}

// ---- the exclusions ------------------------------------------------
//
// Every fixture below declares `fold` on the newcomer, because that is
// the only strategy the exclusions bear on. A pair that reaches the
// queue here reached it *instead of* being folded, and the reason it
// carries is the difference between a panel that can warn and one that
// cannot.

/// A copy that came out of its own original is not folded back into it.
/// One `derived_from` hop is the whole shape of the accident: an
/// exporter in copy mode writes the input's bytes verbatim, and a fold
/// would put the output into the input and delete the record that the
/// export produced anything.
#[tokio::test]
async fn a_child_is_not_folded_into_its_parent() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let parent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/plate.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let child = copy_of(
        &fx.assets,
        fx.persona,
        "/outbox/plate.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    fx.derive(&child, &parent).await;

    assert_eq!(
        fx.detect(&queue, &child, DetectionOrigin::Ingest).await,
        Detection::Queued(parent.id),
        "the lane said fold; the lineage says ask a person"
    );
    assert!(queue.folds().is_empty());
    assert_eq!(fx.open_questions().await, vec![(child.id, parent.id)]);
    assert_eq!(
        fx.declined_folds().await,
        vec![Some(FoldExclusion::Lineage)],
        "the panel is told which rule took the decision away from the machine"
    );
    // The link the exclusion was protecting is still there — which is
    // what a fold would have destroyed.
    assert_eq!(
        fx.edges
            .edges_incident(&child.id, Some(EdgeKind::DerivedFrom), 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Not just the parent: an ancestor two hops up bars the fold as well.
/// The middle row holds different bytes, so the pair being asked about
/// really is the two ends of the chain and not two adjacent links.
#[tokio::test]
async fn an_ancestor_two_hops_up_bars_the_fold() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let root = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/plate.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    // A different digest, so `find_by_content_hash` returns two rows and
    // the middle of the chain is not itself one of the pair.
    let middle = copy_of(
        &fx.assets,
        fx.persona,
        "/work/plate-edit.png",
        "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        at(1_500),
        None,
    )
    .await;
    let leaf = copy_of(
        &fx.assets,
        fx.persona,
        "/outbox/plate-edit-export.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    fx.derive(&middle, &root).await;
    fx.derive(&leaf, &middle).await;
    assert_eq!(
        fx.assets
            .find_by_content_hash(&fx.persona, DuplicateAxis::Artefact, DIGEST)
            .await
            .unwrap()
            .len(),
        2,
        "the pair is the two ends of the chain"
    );

    assert_eq!(
        fx.detect(&queue, &leaf, DetectionOrigin::Ingest).await,
        Detection::Queued(root.id),
        "the walk keeps going past the first hop"
    );
    assert!(queue.folds().is_empty());
    assert_eq!(
        fx.declined_folds().await,
        vec![Some(FoldExclusion::Lineage)]
    );
    assert_ne!(middle.id, root.id);
}

/// Two exports of one input are siblings, and siblings are not folded
/// together either.
///
/// This is the case that makes the rule "share an ancestor" rather than
/// "one is an ancestor of the other". Neither row descends from the
/// other; they are two runs over the same material that happened to
/// come out byte-identical, and the duplicate design is explicit that
/// in this library a deliberate second variant is a second thing.
/// Folding one away deletes the record that the second run happened.
#[tokio::test]
async fn two_variants_of_one_input_are_not_folded_together() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let input = fx.bare_row("/pics/plate.png", at(500)).await;
    let first_run = copy_of(
        &fx.assets,
        fx.persona,
        "/outbox/run-1.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let second_run = copy_of(
        &fx.assets,
        fx.persona,
        "/outbox/run-2.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    fx.derive(&first_run, &input).await;
    fx.derive(&second_run, &input).await;

    assert_eq!(
        fx.detect(&queue, &second_run, DetectionOrigin::Ingest)
            .await,
        Detection::Queued(first_run.id),
        "neither descends from the other, and they are still one lineage"
    );
    assert!(queue.folds().is_empty());
    assert_eq!(
        fx.declined_folds().await,
        vec![Some(FoldExclusion::Lineage)]
    );
}

/// A pair with no lineage between them still folds.
///
/// The control that keeps every assertion above from passing on an
/// exclusion that excludes everything. Both rows have parents — so the
/// walk runs, reads edges, and comes back with an answer — and the
/// parents are different rows, so the answer is "not connected".
#[tokio::test]
async fn a_pair_from_two_unrelated_lineages_still_folds() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let one_source = fx.bare_row("/pics/left.png", at(100)).await;
    let other_source = fx.bare_row("/pics/right.png", at(200)).await;
    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/work/left-out.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/work/right-out.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    fx.derive(&incumbent, &one_source).await;
    fx.derive(&newcomer, &other_source).await;
    assert_ne!(one_source.id, other_source.id);

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Folding(incumbent.id),
        "a graph that says nothing connects them does not stop the fold"
    );
    assert_eq!(queue.folds().len(), 1);
    assert!(fx.open_questions().await.is_empty());
}

/// A graph too wide to finish walking falls to *not* folding.
///
/// The budget is a bound on work, not a verdict. The row here has more
/// `derived_from` edges than one probe will look at, so the walk stops
/// without having seen whatever might connect the pair — and the rule
/// exists to protect lineage, so an unfinished look is a reason to hand
/// the pair to a person rather than a licence to fold it.
#[tokio::test]
async fn a_lineage_too_wide_to_walk_is_not_folded() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/composite.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    // A composite assembled from more inputs than the probe will read.
    // None of them is the incumbent or reaches it, so a walk that ran
    // to completion would answer "unrelated" and fold.
    for index in 0..LINEAGE_PROBE_BUDGET {
        let source = fx
            .bare_row(&format!("/pics/frame-{index}.png"), at(100))
            .await;
        fx.derive(&newcomer, &source).await;
    }

    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id),
        "could not finish looking, so it did not fold"
    );
    assert!(queue.folds().is_empty());
    assert_eq!(
        fx.declined_folds().await,
        vec![Some(FoldExclusion::Lineage)],
        "the reason recorded is the relation that was left undetermined"
    );

    // The contrast that makes this the budget and not the fixture: a
    // narrower fan of the same shape folds. Two fewer rather than one,
    // because the row being walked from counts against the budget as
    // well as its parents.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();
    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/original.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let newcomer = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/composite.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    for index in 0..(LINEAGE_PROBE_BUDGET - 2) {
        let source = fx
            .bare_row(&format!("/pics/frame-{index}.png"), at(100))
            .await;
        fx.derive(&newcomer, &source).await;
    }
    assert_eq!(
        fx.detect(&queue, &newcomer, DetectionOrigin::Ingest).await,
        Detection::Folding(incumbent.id),
        "a walk that finished and found nothing is an answer, not a stop"
    );
}

/// An export run's output is not folded automatically — from either
/// side of the pair.
///
/// Either side, because the accident is one-sided by construction: the
/// copy-mode product sits beside an ordinary import, never beside
/// another product. Which of the two would become the headstone is
/// decided by age, so a one-sided rule would protect the run's record
/// or not depending on when the export happened to run.
#[tokio::test]
async fn an_export_product_is_not_folded_from_either_side() {
    // The arrival is the export.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();
    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/plate.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let export = export_of(
        &fx.assets,
        fx.persona,
        "file",
        "/outbox/plate.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    assert!(
        export
            .source
            .kind
            .as_str()
            .starts_with(SourceKind::DISPATCH_PREFIX),
        "the fixture really is a dispatch product: {}",
        export.source.kind.as_str()
    );
    // No `derived_from` edge anywhere in this fixture: what is being
    // measured is the column, not the graph.
    assert_eq!(
        fx.detect(&queue, &export, DetectionOrigin::Ingest).await,
        Detection::Queued(incumbent.id)
    );
    assert!(queue.folds().is_empty());
    assert_eq!(
        fx.declined_folds().await,
        vec![Some(FoldExclusion::Dispatch)]
    );

    // The other side: the export is the row already there, and an
    // ordinary import arrives holding its bytes.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();
    let export = export_of(
        &fx.assets,
        fx.persona,
        "file",
        "/outbox/plate.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let arrival = copy_of(
        &fx.assets,
        fx.persona,
        "/inbox/plate.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    assert_eq!(
        fx.detect(&queue, &arrival, DetectionOrigin::Ingest).await,
        Detection::Queued(export.id),
        "the rule reads the pair, not the row being hashed"
    );
    assert!(queue.folds().is_empty());
    assert_eq!(
        fx.declined_folds().await,
        vec![Some(FoldExclusion::Dispatch)]
    );

    // …and two ordinary imports, same bytes, no lineage: still folds.
    // Without this the two assertions above could be measuring a
    // detector that stopped folding altogether.
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();
    let incumbent = copy_of(
        &fx.assets,
        fx.persona,
        "/pics/plate.png",
        DIGEST,
        at(1_000),
        None,
    )
    .await;
    let plain = copy_of(
        &fx.assets,
        fx.persona,
        "/inbox/plate.png",
        DIGEST,
        at(2_000),
        Some(OnDuplicate::Fold),
    )
    .await;
    assert_eq!(
        fx.detect(&queue, &plain, DetectionOrigin::Ingest).await,
        Detection::Folding(incumbent.id)
    );
}

/// A question nobody asked to fold carries no reason.
///
/// `None` in the column is a statement — "no automatic fold was
/// declined" — and it has to stay distinguishable from the rows that
/// were. Both shapes that produce it are here: the default `ask`, and
/// the backfill's own downgrade, which is a fact about the pass rather
/// than about the pair and so is deliberately not one of the rules a
/// panel warns about.
#[tokio::test]
async fn a_question_that_was_never_a_fold_records_no_reason() {
    for (declared, origin) in [
        (None, DetectionOrigin::Ingest),
        (Some(OnDuplicate::Fold), DetectionOrigin::Backfill),
    ] {
        let fx = Fixture::new().await;
        let queue = RecordingQueue::default();
        let incumbent = copy_of(
            &fx.assets,
            fx.persona,
            "/pics/original.png",
            DIGEST,
            at(1_000),
            None,
        )
        .await;
        let newcomer = copy_of(
            &fx.assets,
            fx.persona,
            "/pics/copy.png",
            DIGEST,
            at(2_000),
            declared,
        )
        .await;
        // Connected, so a rule *would* have had something to say if it
        // had been consulted.
        fx.derive(&newcomer, &incumbent).await;

        assert_eq!(
            fx.detect(&queue, &newcomer, origin).await,
            Detection::Queued(incumbent.id)
        );
        assert_eq!(
            fx.declined_folds().await,
            vec![None],
            "{declared:?} / {origin:?}: no fold was declined, so no rule is named"
        );
    }
}

// ---- the content axis ----------------------------------------------
//
// The two fixtures below are a pair, and neither is
// meaningful without the other: one shows the content axis answering a
// question the artefact axis cannot, the other shows it staying quiet
// when the artefact axis has already answered.
//
// **What widening the detection path buys, exactly.** The content axis
// is format-gated: the probe registry (`asterism_infra::probes`)
// decides, anything no probe claims is stored as `unsupported:<format>`,
// and today the only probe registered is PNG's. So this is the PNG
// re-export case and only that — video and audio need their own probes
// before they get anything.

/// The PNG this repo already ships, borrowed from the importer SDK's
/// fixtures. A whole file with real chunk framing rather than a byte
/// string assembled in this test, so the digests below are computed
/// over the same shape of bytes the walker was measured on.
const CARD_PNG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../asterism-importer-sdk/tests/fixtures/character-card-lyra.png"
));

/// The same PNG with one more `tEXt` chunk in it — a caption written on
/// re-save, an exporter's timestamp, a workflow blob.
///
/// Spliced in ahead of `IEND` (the last twelve bytes of any PNG: a zero
/// length, the type, and its CRC) with a correctly computed CRC of its
/// own, so what the assertions run on is a valid PNG rather than a
/// shape only this walker would accept.
fn with_extra_text_chunk(png: &[u8], key: &str, value: &str) -> Vec<u8> {
    let mut payload = key.as_bytes().to_vec();
    payload.push(0);
    payload.extend_from_slice(value.as_bytes());
    with_extra_chunk(png, b"tEXt", &payload)
}

/// The same PNG with one more chunk of any type spliced in ahead of
/// `IEND`, with a correctly computed CRC of its own — so what the
/// assertions run on is a valid PNG rather than a shape only these
/// walkers would accept.
fn with_extra_chunk(png: &[u8], kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let split = png.len() - 12;
    assert_eq!(
        &png[split + 4..split + 8],
        b"IEND",
        "the fixture does not end with an IEND chunk"
    );

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(payload);
    chunk.extend_from_slice(&png_crc32(&crc_input).to_be_bytes());

    let mut out = png[..split].to_vec();
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[split..]);
    out
}

/// The same PNG with one bit of its pixel stream flipped — **a
/// different picture, made the same way.**
///
/// The inverse of [`with_extra_text_chunk`], and the two are a pair:
/// one holds the pixels still and moves the metadata, this one holds
/// the metadata still and moves the pixels. Either alone is consistent
/// with a single axis.
///
/// The chunk's CRC is recomputed, so the result is a file a decoder
/// would accept rather than one only a lenient walker would.
fn with_flipped_pixel(png: &[u8]) -> Vec<u8> {
    let mut out = png.to_vec();
    let mut at = 8; // past the signature
    while at + 8 <= out.len() {
        let declared =
            u32::from_be_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]) as usize;
        let kind: [u8; 4] = out[at + 4..at + 8].try_into().unwrap();
        let payload = at + 8;
        if &kind == b"IDAT" && declared > 0 {
            out[payload] ^= 0x01;
            let mut crc_input = kind.to_vec();
            crc_input.extend_from_slice(&out[payload..payload + declared]);
            out[payload + declared..payload + declared + 4]
                .copy_from_slice(&png_crc32(&crc_input).to_be_bytes());
            assert_eq!(out.len(), png.len(), "one bit, not one byte more");
            return out;
        }
        at = payload + declared + 4;
    }
    panic!("the fixture carries no pixel data to move");
}

/// PNG's CRC-32 (the standard reflected polynomial).
fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Every axis of one artefact, computed the way the hash job computes
/// them — the file digest over every byte, the region digest over what
/// survives the walk, and the meta digest over what the walk drops.
fn fingerprint_of(png: &[u8]) -> MaterialFingerprint {
    let mime = asterism_core::domain::value::MimeType::parse("image/png");
    let meta = asterism_infra::probes::meta(png, Some(&mime));
    MaterialFingerprint {
        file: AxisRecord::computed(asterism_core::domain::content_hash::of_bytes(png)),
        content: asterism_infra::probes::content(png, Some(&mime)).record(),
        meta_kv: meta.canonical().map(str::to_string),
        meta: meta.record(),
        // Not a digest and not what these fixtures ask about: the
        // recovered-text column plays no part in duplicate detection.
        meta_text: None,
        meta_raw: asterism_infra::probes::meta_raw(png, Some(&mime)).stored_value(),
    }
}

/// **The content axis fires**: two PNGs differing only by a `tEXt`
/// chunk are proposed as duplicates, on the axis that agreed.
///
/// A byte-identical pair would prove nothing here — `Artefact` answers
/// first and `Content` is never reached — so the fixture is a pair whose
/// file digests *disagree*, asserted before anything is detected. That
/// disagreement is the whole test: on the artefact axis these two rows
/// are unrelated, and before this change nothing would have been
/// proposed about them at all.
#[tokio::test]
async fn the_content_axis_fires_on_a_metadata_only_difference() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let original = fingerprint_of(CARD_PNG);
    let recaptioned = fingerprint_of(&with_extra_text_chunk(
        CARD_PNG,
        "parameters",
        r#"{"prompt":"re-saved with a caption"}"#,
    ));

    // The fixture disagrees on the axis that is *not* under test and
    // agrees on the one that is. Without both halves this test would
    // pass with the content axis switched off.
    assert_ne!(
        original.file, recaptioned.file,
        "the two files must not be byte-identical, or Artefact answers first"
    );
    assert_eq!(
        original.content, recaptioned.content,
        "the region digest is what has to see past the tEXt chunk"
    );
    assert!(
        original
            .content
            .digest()
            .is_some_and(|d| d.starts_with("cr1-sha256:")),
        "a status is not a digest and would group nothing: {:?}",
        original.content
    );

    let incumbent = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/pics/original.png",
        &original,
        at(1_000),
        None,
    )
    .await;
    let newcomer = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/pics/recaptioned.png",
        &recaptioned,
        at(2_000),
        None,
    )
    .await;

    // On the artefact axis the newcomer is alone — stated against the
    // repository, so "the content axis found it" cannot be confused
    // with "the artefact axis found it under another name".
    assert_eq!(
        fx.assets
            .find_by_content_hash(
                &fx.persona,
                DuplicateAxis::Artefact,
                recaptioned.file.digest().expect("the file axis computed"),
            )
            .await
            .unwrap()
            .len(),
        1,
        "nobody else holds these exact bytes"
    );

    assert_eq!(
        fx.detect_fingerprint(&queue, &newcomer, &recaptioned, DetectionOrigin::Ingest)
            .await,
        Detection::Queued(incumbent.id),
        "the same picture, differently captioned, reaches a person"
    );

    let conflict = fx.only_open_conflict().await;
    assert_eq!(
        conflict.axis,
        DuplicateAxis::Content,
        "the question names the axis that actually agreed"
    );
    assert_eq!(conflict.newcomer, newcomer.id);
    assert_eq!(conflict.incumbent, incumbent.id);
    assert_eq!(
        Some(conflict.content_hash.as_str()),
        recaptioned.content.digest(),
        "and carries the region digest, not the file one"
    );
    assert_eq!(
        fx.identical_edges().await,
        vec![(
            *newcomer.id.as_uuid(),
            *incumbent.id.as_uuid(),
            Some(DuplicateAxis::Content.as_str().to_string())
        )],
        "the edge's label and the queue row's column say the same word"
    );
}

/// **The meta axis fires**: two frames off one workflow, differing only
/// in their pixels, are proposed as duplicates on the axis that agreed.
///
/// The inverse of the fixture above, and the pair is the point.
/// `the_content_axis_fires_on_a_metadata_only_difference` holds the
/// pixels still and moves the metadata; this one holds the metadata
/// still and moves the pixels. **Either alone is consistent with a
/// single axis** — one column read twice under two names would pass
/// whichever of the two was written — so both are required for the
/// claim "`Meta` is not `Content` under another name" to mean anything.
///
/// The disagreements are asserted before anything is detected: on the
/// artefact axis and on the content axis these two rows are unrelated,
/// which is what makes the proposal below a reading of the third
/// column.
///
/// Bounded honestly: the walk is format-gated and PNG-only today, so
/// what this buys is the PNG generator case and only that. Video and
/// audio need their own walkers before they get anything.
#[tokio::test]
async fn the_meta_axis_fires_on_a_pixels_only_difference() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    // One workflow written into both files — the fixture's own text is
    // what the two share, so nothing here has to invent a corpus.
    let workflow = r#"{"nodes":[{"class":"KSampler","seed":7}]}"#;
    let seed_a = with_extra_text_chunk(CARD_PNG, "workflow", workflow);
    let seed_b = with_flipped_pixel(&seed_a);

    let first = fingerprint_of(&seed_a);
    let second = fingerprint_of(&seed_b);

    // The fixture disagrees on both axes that are *not* under test and
    // agrees on the one that is. Without all three halves this would
    // pass with the meta axis switched off, or with it wired to the
    // wrong column.
    assert_ne!(
        first.file, second.file,
        "the two files must not be byte-identical, or Artefact answers first"
    );
    assert_ne!(
        first.content, second.content,
        "the pixels have to differ, or Content answers first and this says nothing"
    );
    assert_eq!(
        first.meta, second.meta,
        "the metadata digest is what has to see past the pixels"
    );
    assert!(
        first
            .meta
            .digest()
            .is_some_and(|d| d.starts_with("m1-sha256:")),
        "a status is not a digest and would group nothing: {:?}",
        first.meta
    );
    assert_eq!(
        first.meta_kv, second.meta_kv,
        "and the object the digest was taken over travels with it"
    );
    // The object is the container's own text, kept unparsed: the
    // chunk's JSON survives as one *string* value rather than being
    // re-serialised into the form, which is the rule the digest's
    // meaning rests on.
    let fields: std::collections::BTreeMap<String, String> =
        serde_json::from_str(first.meta_kv.as_deref().expect("a digest carries its body"))
            .expect("the canonical form is a flat key → value object");
    assert_eq!(
        fields.get("workflow").map(String::as_str),
        Some(workflow),
        "the workflow is carried verbatim"
    );

    let incumbent = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/comfy/seed-a.png",
        &first,
        at(1_000),
        None,
    )
    .await;
    let newcomer = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/comfy/seed-b.png",
        &second,
        at(2_000),
        None,
    )
    .await;

    // On the two stronger axes the newcomer is alone — stated against
    // the repository, so "the meta axis found it" cannot be confused
    // with "one of the others found it under another name".
    for (axis, digest) in [
        (DuplicateAxis::Artefact, second.file.digest().unwrap()),
        (DuplicateAxis::Content, second.content.digest().unwrap()),
    ] {
        assert_eq!(
            fx.assets
                .find_by_content_hash(&fx.persona, axis, digest)
                .await
                .unwrap()
                .len(),
            1,
            "nobody else holds this row's {} digest",
            axis.as_str()
        );
    }

    assert_eq!(
        fx.detect_fingerprint(&queue, &newcomer, &second, DetectionOrigin::Ingest)
            .await,
        Detection::Queued(incumbent.id),
        "two frames off one workflow reach a person"
    );

    let conflict = fx.only_open_conflict().await;
    assert_eq!(
        conflict.axis,
        DuplicateAxis::Meta,
        "the question names the axis that actually agreed"
    );
    assert_eq!(conflict.newcomer, newcomer.id);
    assert_eq!(conflict.incumbent, incumbent.id);
    assert_eq!(
        Some(conflict.content_hash.as_str()),
        second.meta.digest(),
        "and carries the metadata digest, not one of the others"
    );
    assert_eq!(
        fx.identical_edges().await,
        vec![(
            *newcomer.id.as_uuid(),
            *incumbent.id.as_uuid(),
            Some(DuplicateAxis::Meta.as_str().to_string())
        )],
        "the edge's label and the queue row's column say the same word"
    );
}

/// **The strongest axis that agrees is the only one reported.**
///
/// A byte-identical pair agrees on both axes, and `duplicate_conflict`
/// is unique on `(pair_lo, pair_hi, axis)` — nothing in the schema
/// stops one pair being queued twice, once per axis, for a person to
/// answer twice. `Artefact` implies `Content`, so the second row would
/// be the same finding stated more weakly.
///
/// The fixture is byte-identical *on purpose*, which is the one case
/// where that is not vacuous: what is being measured is the axis the
/// single row carries, and the assertion that the weaker axis would
/// have matched is what stops this passing on a build where the content
/// axis simply never runs.
#[tokio::test]
async fn only_the_strongest_agreeing_axis_is_reported() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    let both = fingerprint_of(CARD_PNG);
    assert!(
        both.content
            .digest()
            .is_some_and(|d| d.starts_with("cr1-sha256:")),
        "the weaker axes have to be able to match, or 'and stop' is untested"
    );
    assert!(
        both.meta
            .digest()
            .is_some_and(|d| d.starts_with("m1-sha256:")),
        "{:?}",
        both.meta
    );

    let incumbent = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/pics/original.png",
        &both,
        at(1_000),
        None,
    )
    .await;
    let newcomer = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/pics/copy.png",
        &both,
        at(2_000),
        None,
    )
    .await;

    // Every axis really would answer: two holders on each.
    for (axis, digest) in [
        (DuplicateAxis::Artefact, both.file.digest().unwrap()),
        (DuplicateAxis::Content, both.content.digest().unwrap()),
        (DuplicateAxis::Meta, both.meta.digest().unwrap()),
    ] {
        assert_eq!(
            fx.assets
                .find_by_content_hash(&fx.persona, axis, digest)
                .await
                .unwrap()
                .len(),
            2,
            "{} would have matched on its own",
            axis.as_str()
        );
    }

    assert_eq!(
        fx.detect_fingerprint(&queue, &newcomer, &both, DetectionOrigin::Ingest)
            .await,
        Detection::Queued(incumbent.id)
    );

    // One question, on the strong axis — not two.
    let conflict = fx.only_open_conflict().await;
    assert_eq!(conflict.axis, DuplicateAxis::Artefact);
    assert_eq!(Some(conflict.content_hash.as_str()), both.file.digest());
    assert_eq!(
        fx.identical_edges().await,
        vec![(
            *newcomer.id.as_uuid(),
            *incumbent.id.as_uuid(),
            Some(DuplicateAxis::Artefact.as_str().to_string())
        )],
        "one edge as well: the weaker axes never ran"
    );

    // Re-detecting does not add the weaker ones either — the second pass
    // takes the same branch, which is where a "record whatever is
    // missing" repair would have crept in.
    assert_eq!(
        fx.detect_fingerprint(&queue, &newcomer, &both, DetectionOrigin::Ingest)
            .await,
        Detection::AlreadyQueued(incumbent.id)
    );
    assert_eq!(fx.open_questions().await.len(), 1);
}

/// **What stopping costs when neither axis implies the other.**
///
/// `Artefact` implies `Content` and `Meta`, so dropping those is the
/// same finding stated more weakly. `Content` and `Meta` imply nothing
/// about each other, and this is the pair where that bites: same
/// pixels, same embedded text, different file bytes — a re-encode that
/// preserved every payload the two walkers read. Both weaker axes
/// agree, and **one** question is raised, on `Content`.
///
/// This is the documented cost of the walk order rather than an
/// accident, so it is pinned here with the recovery beside it: the meta
/// digests are on both rows, so "they also share their metadata" is a
/// comparison anyone can make against the stored column. Nothing that
/// is discarded is unrecoverable, which is what makes stopping
/// defensible where implication does not hold.
#[tokio::test]
async fn a_pair_agreeing_on_both_weak_axes_is_asked_about_once() {
    let fx = Fixture::new().await;
    let queue = RecordingQueue::default();

    // A `tIME` chunk: excluded from the content region (it is metadata)
    // and not `tEXt` (so it is not in the meta set either), which makes
    // it exactly a file-byte difference and nothing else.
    let original = fingerprint_of(CARD_PNG);
    let restamped = fingerprint_of(&with_extra_chunk(
        CARD_PNG,
        b"tIME",
        &[0x07, 0xe9, 8, 6, 12, 0, 0],
    ));

    assert_ne!(original.file, restamped.file, "the files have to differ");
    assert_eq!(original.content, restamped.content, "same picture");
    assert_eq!(original.meta, restamped.meta, "and the same metadata");
    assert!(
        original
            .content
            .digest()
            .is_some_and(|d| d.starts_with("cr1-sha256:"))
    );
    assert!(
        original
            .meta
            .digest()
            .is_some_and(|d| d.starts_with("m1-sha256:"))
    );

    let incumbent = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/pics/original.png",
        &original,
        at(1_000),
        None,
    )
    .await;
    let newcomer = fingerprinted_row(
        &fx.assets,
        fx.persona,
        SourceKind::new(SourceKind::FS).unwrap(),
        "/pics/re-encoded.png",
        &restamped,
        at(2_000),
        None,
    )
    .await;

    // Both weak axes would answer on their own — otherwise "one
    // question" would be a fact about a pair only one axis matched.
    for (axis, digest) in [
        (DuplicateAxis::Content, restamped.content.digest().unwrap()),
        (DuplicateAxis::Meta, restamped.meta.digest().unwrap()),
    ] {
        assert_eq!(
            fx.assets
                .find_by_content_hash(&fx.persona, axis, digest)
                .await
                .unwrap()
                .len(),
            2,
            "{} would have matched on its own",
            axis.as_str()
        );
    }
    assert_eq!(
        fx.assets
            .find_by_content_hash(
                &fx.persona,
                DuplicateAxis::Artefact,
                restamped.file.digest().expect("the file axis computed"),
            )
            .await
            .unwrap()
            .len(),
        1,
        "and the axis that implies both does not, or the stop is the ordinary one"
    );

    assert_eq!(
        fx.detect_fingerprint(&queue, &newcomer, &restamped, DetectionOrigin::Ingest)
            .await,
        Detection::Queued(incumbent.id)
    );

    let questions = fx.open_questions().await;
    assert_eq!(questions.len(), 1, "one pair is one question");
    assert_eq!(
        fx.only_open_conflict().await.axis,
        DuplicateAxis::Content,
        "reported on the stronger claim about sameness of the two that agreed"
    );

    // The discarded finding is still recoverable from the rows, which
    // is the whole justification for discarding it.
    let held = fx
        .assets
        .find_by_content_hash(
            &fx.persona,
            DuplicateAxis::Meta,
            restamped.meta.digest().expect("the meta axis computed"),
        )
        .await
        .unwrap();
    assert_eq!(
        held.iter()
            .map(|asset| asset.id)
            .collect::<std::collections::HashSet<_>>(),
        std::collections::HashSet::from([incumbent.id, newcomer.id]),
        "the meta agreement is on the stored columns even though no row was queued for it"
    );
}

/// A digest the caller **stated** proposes and never folds, even when
/// the lane declared `fold`.
///
/// The claim is unverified until the hashing job re-reads the file, and
/// there is no unfold verb — so a fold driven by it would not be
/// reversible. The contrast is the whole assertion: the same fixture on
/// the pass that measured the bytes *does* fold.
#[tokio::test]
async fn a_declared_digest_proposes_where_a_measured_one_folds() {
    for (origin, expected_folds) in [
        (DetectionOrigin::Declared, 0usize),
        (DetectionOrigin::Ingest, 1usize),
    ] {
        let fx = Fixture::new().await;
        let queue = RecordingQueue::default();

        let incumbent = copy_of(
            &fx.assets,
            fx.persona,
            "/pics/original.png",
            DIGEST,
            at(1_000),
            None,
        )
        .await;
        let newcomer = copy_of(
            &fx.assets,
            fx.persona,
            "/pics/copy.png",
            DIGEST,
            at(2_000),
            Some(OnDuplicate::Fold),
        )
        .await;

        let outcome = fx.detect(&queue, &newcomer, origin).await;
        assert_eq!(queue.folds().len(), expected_folds, "{origin:?}");
        if expected_folds == 0 {
            assert_eq!(outcome, Detection::Queued(incumbent.id), "{origin:?}");
            assert_eq!(
                fx.open_questions().await,
                vec![(newcomer.id, incumbent.id)],
                "{origin:?}: the pair still reaches a person"
            );
            assert_eq!(
                fx.declined_folds().await,
                vec![None],
                "{origin:?}: the pass declined the fold, which is not a rule about the pair"
            );
        } else {
            assert_eq!(outcome, Detection::Folding(incumbent.id), "{origin:?}");
        }
    }
}
