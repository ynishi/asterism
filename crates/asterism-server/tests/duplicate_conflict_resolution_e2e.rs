//! End-to-end: a raised duplicate question is read, answered, and gone.
//!
//! `duplicate_detection_e2e` stops where this starts. It asserts that
//! registering a second copy leaves a row on the conflict queue — read
//! straight out of SQLite, because at the time nothing could read it.
//! This binary covers the surface that answers it: the listing that
//! hydrates both sides as cards, the confirm verb, and what each of them
//! refuses.
//!
//! `Full` mode throughout, for the reason the detection e2e gives: the
//! queue row only exists because the job worker read the file off disk
//! and the digest landed. A fixture that set the hash by hand would
//! assert the resolution surface against a conflict nothing detected.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{
    AddAssetCommand, ConflictResolution, RegisterPersonaCommand, ResolveDuplicateConflictCommand,
    RestoreAssetCommand, TrashAssetCommand,
};
use asterism_contract::dto::{DuplicateAxis, DuplicateConflictDto};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Same bytes for every copy — what makes them one question.
const BYTES: &[u8] = b"the same photograph, byte for byte\n";

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about which of two rows is
/// the duplicate, not about who registered either one.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str, occurred_at_ms: i64) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: None,
        occurred_at_ms,
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
    }
}

/// A library holding one raised question: two copies of one file, one
/// unrelated file, and the router over the same services.
struct Fixture {
    tmp: tempfile::TempDir,
    core: CoreCtx,
    router: Router,
    persona_id: String,
    /// The row that arrived first and was fingerprinted first — the
    /// incumbent of the pair.
    incumbent: String,
    /// The second copy, whose arrival raised the question.
    newcomer: String,
    corpus: std::path::PathBuf,
}

impl Fixture {
    /// Ingests the pair and waits until the question is on the queue.
    ///
    /// The incumbent is registered **and fingerprinted** before the copy
    /// is registered, and that is asserted rather than assumed: against
    /// a fixture where both land at once, "the newcomer is the second
    /// copy" could pass on a detection that fired from either end.
    async fn raised() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let corpus = tmp.path().join("corpus");
        std::fs::create_dir_all(&corpus).expect("corpus dir");
        std::fs::write(corpus.join("original.png"), BYTES).expect("write original");
        std::fs::write(corpus.join("copy.png"), BYTES).expect("write copy");
        std::fs::write(corpus.join("other.png"), b"a different photograph\n").expect("write other");

        let core = init_core_with(
            &tmp.path().join("asterism.db"),
            Arc::new(LogEmitter),
            CoreMode::Full,
            Some(&tmp.path().join("tantivy")),
        )
        .await
        .expect("init_core");
        let router = asterism_server::http::router(ServerCtx::from_core(&core));

        let persona = core
            .persona_service
            .register(
                RegisterPersonaCommand {
                    name: "E2E".into(),
                    pack_id: Some(format!("e2e-conflict-{}", uuid::Uuid::now_v7())),
                },
                &unattributed(),
            )
            .await
            .expect("register persona");

        let incumbent = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    corpus.join("original.png").to_str().unwrap(),
                    1_785_000_000_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add the original")
            .id;
        let mut hashed = false;
        for _ in 0..120 {
            let detail = core
                .asset_service
                .detail(asterism_contract::query::GetAssetDetailQuery {
                    asset_id: incumbent.clone(),
                    viewer_subject: None,
                })
                .await
                .expect("detail of the original");
            if detail.asset.content_hash.is_some() {
                hashed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(hashed, "the incumbent was fingerprinted within 30s");

        // An unrelated file in the same wave: whatever ends up on the
        // queue, it is not "everything that was imported".
        core.asset_service
            .add(
                add_command(
                    &persona.id,
                    corpus.join("other.png").to_str().unwrap(),
                    1_785_000_001_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add the unrelated file");

        let newcomer = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    corpus.join("copy.png").to_str().unwrap(),
                    1_785_000_002_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add the copy")
            .id;

        let fixture = Self {
            tmp,
            core,
            router,
            persona_id: persona.id,
            incumbent,
            newcomer,
            corpus,
        };
        fixture.await_one_conflict().await;
        fixture
    }

    async fn conflicts(&self) -> Vec<DuplicateConflictDto> {
        self.core
            .asset_service
            .list_duplicate_conflicts(None, None)
            .await
            .expect("conflict listing")
    }

    /// Polls until exactly one question is on the panel and returns it.
    async fn await_one_conflict(&self) -> DuplicateConflictDto {
        for _ in 0..120 {
            let open = self.conflicts().await;
            if open.len() == 1 {
                return open.into_iter().next().expect("one");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("no question reached the panel within 30s");
    }

    /// Registers another copy of the same bytes under a new path.
    async fn add_copy(&self, name: &str, occurred_at_ms: i64) -> String {
        let path = self.corpus.join(name);
        std::fs::write(&path, BYTES).expect("write copy");
        self.core
            .asset_service
            .add(
                add_command(&self.persona_id, path.to_str().unwrap(), occurred_at_ms),
                &unattributed(),
            )
            .await
            .expect("add another copy")
            .id
    }

    /// Reads `folded_into` off a row over a second connection — the
    /// fold's own effect, which no read DTO carries.
    async fn folded_into(&self, asset_id: &str) -> Option<String> {
        let (isle, driver) =
            asterism_infra::sqlite::open_and_migrate(&self.tmp.path().join("asterism.db"))
                .await
                .expect("second isle");
        let id: uuid::Uuid = asset_id.parse().expect("asset id");
        let keeper = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT folded_into FROM asset WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get::<_, Option<uuid::Uuid>>(0),
                )
            })
            .await
            .expect("read folded_into");
        driver.shutdown().await.ok();
        keeper.map(|k| k.to_string())
    }

    /// `(asset rows, duplicate_conflict rows)` — both counted over a
    /// second connection, and both counting *everything*: a closed
    /// question is still a row, and what these fixtures need to see is
    /// whether a second one was written.
    async fn row_counts(&self) -> (i64, i64) {
        let (isle, driver) =
            asterism_infra::sqlite::open_and_migrate(&self.tmp.path().join("asterism.db"))
                .await
                .expect("second isle");
        let counts = isle
            .call(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM duplicate_conflict", [], |r| r.get(0))?,
                ))
            })
            .await
            .expect("count the rows");
        driver.shutdown().await.ok();
        counts
    }

    /// Re-registers a path that is already in the library — one item of
    /// a second sweep over the same corpus.
    async fn rescan(&self, name: &str) -> String {
        let path = self.corpus.join(name);
        assert!(path.exists(), "a re-scan is a re-scan of something");
        self.core
            .asset_service
            .add(
                add_command(&self.persona_id, path.to_str().unwrap(), 1_785_000_009_000),
                &unattributed(),
            )
            .await
            .expect("a record arriving again is not a failure")
            .id
    }

    /// Polls until the queued fold has run.
    async fn await_fold(&self, headstone: &str) -> String {
        for _ in 0..120 {
            if let Some(keeper) = self.folded_into(headstone).await {
                return keeper;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("the queued fold did not run within 30s");
    }
}

fn fold_onto(conflict_id: &str, keeper_id: &str) -> ResolveDuplicateConflictCommand {
    ResolveDuplicateConflictCommand {
        conflict_id: conflict_id.to_string(),
        resolution: ConflictResolution::Folded,
        keeper_id: Some(keeper_id.to_string()),
    }
}

fn keep_apart(conflict_id: &str) -> ResolveDuplicateConflictCommand {
    ResolveDuplicateConflictCommand {
        conflict_id: conflict_id.to_string(),
        resolution: ConflictResolution::Kept,
        keeper_id: None,
    }
}

/// The whole loop: two copies land, the question appears with both
/// sides drawn, a person folds it, the fold runs, and the panel is
/// empty.
#[tokio::test(flavor = "multi_thread")]
async fn a_question_is_raised_answered_and_leaves_the_panel() {
    let fx = Fixture::raised().await;

    let conflict = fx.await_one_conflict().await;
    assert_eq!(
        (
            conflict.newcomer.id.as_str(),
            conflict.incumbent.id.as_str()
        ),
        (fx.newcomer.as_str(), fx.incumbent.as_str()),
        "the arrival is the newcomer, the row already there is the incumbent"
    );
    // These copies are named `.png` and are not PNGs, so the walker
    // refuses them and only the artefact axis has anything to say about
    // the pair — which is why detection raised the question on that axis.
    assert_eq!(conflict.axis, DuplicateAxis::Artefact);
    assert!(
        conflict.content_hash.starts_with("sha256:"),
        "the question carries the digest it is about: {}",
        conflict.content_hash
    );
    assert_eq!(
        conflict.fold_exclusion, None,
        "nobody asked for an automatic fold, so no rule declined one"
    );
    assert!(conflict.detected_at_ms > 0, "the question is timestamped");
    // Both sides arrive drawn, not as bare ids: a panel that had to
    // fetch them would be N+1 reads per page.
    assert!(
        conflict.newcomer.source_locator.ends_with("copy.png")
            && conflict.incumbent.source_locator.ends_with("original.png"),
        "each side carries its own card"
    );

    // Answered: keep the older row, fold the copy into it.
    let outcome = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(fold_onto(&conflict.id, &fx.incumbent), &unattributed())
        .await
        .expect("the fold is accepted");
    assert_eq!(outcome.resolution, "folded");
    assert_eq!(
        (
            outcome.keeper_id.as_deref(),
            outcome.headstone_id.as_deref()
        ),
        (Some(fx.incumbent.as_str()), Some(fx.newcomer.as_str())),
        "the answer says which row stayed and which one goes"
    );

    // The row is closed immediately; the fold itself is a queued job.
    assert!(
        fx.conflicts().await.is_empty(),
        "an answered question leaves the panel"
    );
    assert_eq!(
        fx.await_fold(&fx.newcomer).await,
        fx.incumbent,
        "the queued fold stood the newcomer up as a headstone pointing at the keeper"
    );
    assert_eq!(
        fx.folded_into(&fx.incumbent).await,
        None,
        "the keeper is still a live row"
    );

    // And the report agrees: a folded row is not a live duplicate.
    let report = fx
        .core
        .asset_service
        .list_duplicate_groups(None, None, None)
        .await
        .expect("duplicate report");
    assert!(
        report.groups.is_empty(),
        "the pair is no longer two live rows holding the same bytes"
    );
}

/// A `kept` ruling closes the question and writes nothing to either
/// row — which is visible from the outside, because a third copy of the
/// same bytes still raises a question.
///
/// That last assertion is the one carrying the decision. `fold_policy =
/// keep` on either row would suppress **every** pair that row takes
/// part in, so writing it here would swallow the third copy's question
/// silently. The pair-level answer is carried by the closed queue row
/// instead, which is why the ruled pair stays gone while the new pair
/// arrives.
#[tokio::test(flavor = "multi_thread")]
async fn a_kept_ruling_answers_one_pair_and_not_the_rows() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;

    let outcome = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(keep_apart(&conflict.id), &unattributed())
        .await
        .expect("the ruling is accepted");
    assert_eq!(outcome.resolution, "kept");
    assert_eq!(
        (outcome.keeper_id, outcome.headstone_id),
        (None, None),
        "nothing was folded, so nothing was kept instead of anything"
    );
    assert!(fx.conflicts().await.is_empty(), "the question is answered");

    // Both rows stand.
    assert_eq!(fx.folded_into(&fx.newcomer).await, None);
    assert_eq!(fx.folded_into(&fx.incumbent).await, None);
    // …and the *report* still finds them, which is the deliberate
    // difference between the two lists: they really do hold the same
    // bytes, and a person said that is fine.
    let report = fx
        .core
        .asset_service
        .list_duplicate_groups(None, None, None)
        .await
        .expect("duplicate report");
    assert_eq!(report.groups.len(), 1, "still two live rows, same bytes");

    // A third copy: the rows were not ruled, so this is a new question.
    let third = fx.add_copy("third.png", 1_785_000_003_000).await;
    let raised = fx.await_one_conflict().await;
    assert_eq!(
        (raised.newcomer.id.as_str(), raised.incumbent.id.as_str()),
        (third.as_str(), fx.incumbent.as_str()),
        "the newest copy is asked about against the oldest holder"
    );
    assert_ne!(
        raised.id, conflict.id,
        "and it is a new question, not the answered one coming back"
    );
}

/// **An answered question stays answered across a re-scan.**
///
/// This is what the pre-mint lookup is *for*: it has to come before the
/// mint. A closed conflict row is keyed on the
/// pair of `AssetId`s, so it only keeps a pair from being asked again
/// while the pair keeps its ids. An ingest that minted first and asked
/// afterwards would give every arrival a fresh id — every pair would be
/// new, the answer a person gave would never match again, and one sweep
/// of a corpus under `ScanMode::Enumerate` would re-raise every question
/// in the library.
///
/// The `kept` ruling is the one to test it on: both rows stand and both
/// keep their bytes, so nothing but the closed row is stopping the
/// question from coming back. The last third of the fixture is the
/// disagreement that keeps the assertion honest — the same bytes at a
/// path nobody has scanned before *do* raise a question, so "no new
/// conflict" is a fact about re-arrival and not about a detector that
/// stopped detecting.
#[tokio::test(flavor = "multi_thread")]
async fn a_re_scan_does_not_re_ask_an_answered_question() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;

    fx.core
        .asset_service
        .resolve_duplicate_conflict(keep_apart(&conflict.id), &unattributed())
        .await
        .expect("the ruling is accepted");
    assert!(fx.conflicts().await.is_empty(), "the question is answered");

    let (assets_before, conflicts_before) = fx.row_counts().await;
    assert_eq!(
        conflicts_before, 1,
        "one question was raised and answered — the row is the answer"
    );

    // The second sweep: both sides of the answered pair arrive again.
    assert_eq!(
        (fx.rescan("original.png").await, fx.rescan("copy.png").await),
        (fx.incumbent.clone(), fx.newcomer.clone()),
        "both are records arriving again, so both are answered with the ids they already had"
    );

    let (assets_after, conflicts_after) = fx.row_counts().await;
    assert_eq!(
        assets_after, assets_before,
        "a re-scan minted a row; from here every answer a person gave is about ids that no \
         longer arrive"
    );
    assert_eq!(
        conflicts_after, conflicts_before,
        "the answered pair was asked about again"
    );
    assert!(
        fx.conflicts().await.is_empty(),
        "and the panel is still empty"
    );

    // The disagreement: same bytes, a path this library has not seen.
    let third = fx.add_copy("third.png", 1_785_000_010_000).await;
    let raised = fx.await_one_conflict().await;
    assert_eq!(
        raised.newcomer.id, third,
        "detection is still running — what it stopped doing is re-asking about the same rows"
    );
    assert_ne!(raised.id, conflict.id);
}

/// The three ways a confirm can name the wrong rows, all refused before
/// anything is written.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirm_that_names_the_wrong_rows_is_refused() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;

    // Folding without saying which row stays. The keeper is not
    // defaulted: this row is on the queue precisely because the choice
    // was taken away from the machine.
    let err = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(
            ResolveDuplicateConflictCommand {
                conflict_id: conflict.id.clone(),
                resolution: ConflictResolution::Folded,
                keeper_id: None,
            },
            &unattributed(),
        )
        .await
        .expect_err("a fold with no keeper is refused");
    assert!(
        err.to_string().contains("keeper_id"),
        "the refusal names what is missing: {err}"
    );

    // A keeper from outside the pair — a well-formed id of a real
    // asset, so what is being refused is the pairing rather than the
    // parse.
    let outsider = fx.add_copy("outsider.png", 1_785_000_004_000).await;
    let err = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(fold_onto(&conflict.id, &outsider), &unattributed())
        .await
        .expect_err("a keeper from another pair is refused");
    assert!(
        err.to_string().contains("not part of this conflict"),
        "the refusal says why: {err}"
    );

    // A keeper on a "these are two things" ruling is a contradiction.
    // Ignoring it would let the caller believe it had folded something.
    let err = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(
            ResolveDuplicateConflictCommand {
                conflict_id: conflict.id.clone(),
                resolution: ConflictResolution::Kept,
                keeper_id: Some(fx.incumbent.clone()),
            },
            &unattributed(),
        )
        .await
        .expect_err("a keeper has no meaning on a kept ruling");
    assert!(
        err.to_string().contains("both rows stay"),
        "the refusal says what kept means: {err}"
    );

    // Nothing was written by any of the three.
    assert_eq!(
        fx.folded_into(&fx.newcomer).await,
        None,
        "no fold was queued behind a refused confirm"
    );
    assert!(
        fx.conflicts()
            .await
            .iter()
            .any(|open| open.id == conflict.id),
        "the question is still waiting to be answered"
    );
}

/// One question, one answer. The second confirm is refused whichever
/// answer it carries, and the first one stands.
#[tokio::test(flavor = "multi_thread")]
async fn a_question_can_only_be_answered_once() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;

    fx.core
        .asset_service
        .resolve_duplicate_conflict(keep_apart(&conflict.id), &unattributed())
        .await
        .expect("the first answer lands");

    let err = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(fold_onto(&conflict.id, &fx.incumbent), &unattributed())
        .await
        .expect_err("the second answer is refused");
    assert!(
        err.to_string().contains("already resolved as kept"),
        "the refusal names the answer on record: {err}"
    );

    // Repeating the *same* answer is refused too — there is no second
    // act of ruling, and a success would report that this call recorded
    // something.
    assert!(
        fx.core
            .asset_service
            .resolve_duplicate_conflict(keep_apart(&conflict.id), &unattributed())
            .await
            .is_err()
    );

    // The refused fold enqueued nothing: the pair a person ruled apart
    // is still two live rows.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fx.folded_into(&fx.newcomer).await, None);
    assert_eq!(fx.folded_into(&fx.incumbent).await, None);
}

/// A question whose side is in the trash drops off the panel, and is
/// refused when named by id — until the row comes back.
///
/// The reversibility is the point: nothing is stamped on the queue row
/// when a side goes away, because the verb that restores it knows
/// nothing about this queue.
#[tokio::test(flavor = "multi_thread")]
async fn a_question_whose_side_is_in_the_trash_waits_for_it() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;

    fx.core
        .asset_service
        .trash(
            TrashAssetCommand {
                asset_id: fx.newcomer.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("trash one side");

    assert!(
        fx.conflicts().await.is_empty(),
        "a pair with a row on its way out is not worth interrupting anyone over"
    );
    let err = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(fold_onto(&conflict.id, &fx.incumbent), &unattributed())
        .await
        .expect_err("named by id, it is refused rather than silently answered");
    assert!(
        err.to_string().contains("in the trash"),
        "the refusal says which way to make it answerable: {err}"
    );

    fx.core
        .asset_service
        .restore(
            RestoreAssetCommand {
                asset_id: fx.newcomer.clone(),
            },
            &unattributed(),
        )
        .await
        .expect("restore it");

    let back = fx.await_one_conflict().await;
    assert_eq!(
        back.id, conflict.id,
        "the same question is live again — nothing was written to close it"
    );
    fx.core
        .asset_service
        .resolve_duplicate_conflict(keep_apart(&conflict.id), &unattributed())
        .await
        .expect("and now it can be answered");
}

/// A question one of whose sides has become a headstone is refused: a
/// folded row is not a thing to compare.
///
/// Built from three copies so that answering one pair turns a side of
/// the *other* pair into a headstone while that other question is still
/// open — which is the only way to reach this branch, since a pair
/// answered directly is refused as already answered.
#[tokio::test(flavor = "multi_thread")]
async fn a_question_whose_side_was_folded_away_is_refused() {
    let fx = Fixture::raised().await;
    let first = fx.await_one_conflict().await;

    // A third copy raises a second question, also against the oldest
    // holder.
    let third = fx.add_copy("third.png", 1_785_000_003_000).await;
    let mut second = None;
    for _ in 0..120 {
        let open = fx.conflicts().await;
        if open.len() == 2 {
            second = open.into_iter().find(|c| c.newcomer.id == third);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let second = second.expect("the third copy raised its own question");

    // Answer the second one by keeping the *third* copy: that makes the
    // incumbent — which is also one side of the first question — a
    // headstone.
    fx.core
        .asset_service
        .resolve_duplicate_conflict(fold_onto(&second.id, &third), &unattributed())
        .await
        .expect("fold the incumbent into the third copy");
    assert_eq!(fx.await_fold(&fx.incumbent).await, third);

    let err = fx
        .core
        .asset_service
        .resolve_duplicate_conflict(fold_onto(&first.id, &fx.newcomer), &unattributed())
        .await
        .expect_err("the surviving question names a row that has gone");
    assert!(
        err.to_string().contains("folded away"),
        "the refusal says the side is gone rather than 'no such conflict': {err}"
    );
    assert!(
        fx.conflicts().await.is_empty(),
        "and the panel already stopped showing it"
    );
}

/// The listing is readable over HTTP, and the confirm answers there.
#[tokio::test(flavor = "multi_thread")]
async fn the_conflict_surface_is_reachable_over_http() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;

    let (status, body) = get(&fx.router, "/asterism/duplicates/conflicts").await;
    assert_eq!(status, StatusCode::OK);
    let listed = body.as_array().expect("an array of questions");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], conflict.id);
    assert_eq!(listed[0]["axis"], "artefact");
    assert_eq!(listed[0]["newcomer"]["id"], fx.newcomer);
    assert_eq!(listed[0]["incumbent"]["id"], fx.incumbent);

    // The persona filter is the same one the report takes.
    let (status, body) = get(
        &fx.router,
        &format!(
            "/asterism/duplicates/conflicts?persona_id={}",
            fx.persona_id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().map(Vec::len), Some(1));

    // And the report grew the axis field it was missing.
    let (status, body) = get(&fx.router, "/asterism/duplicates").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["groups"][0]["axis"], "artefact",
        "a group says which agreement it reports: {body}"
    );
    // Omitting the parameter is the artefact axis, and asking for it
    // says the same thing — so a caller that names no axis keeps getting
    // the report it was getting.
    let (status, named) = get(&fx.router, "/asterism/duplicates?axis=artefact").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        named, body,
        "?axis=artefact is what omitting it already meant"
    );
    // The slug this axis carried before V64 is not a second spelling of
    // it. Accepting it would answer, under a `200`, for a caller built
    // against a vocabulary this server does not have.
    let (status, old) = get(&fx.router, "/asterism/duplicates?axis=file").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "`file` was renamed, not aliased: {old}"
    );
    // The backlog the content axis owes rides on every answer, so a
    // caller can read it before switching axis.
    assert_eq!(
        body["unwalked_count"], 0,
        "this fixture was imported after the column existed: {body}"
    );

    // The other axis is reachable over the same route and answers about
    // the other column. These copies are named `.png` and are not PNGs,
    // so the walker refuses them by signature and the content column
    // holds a marker — the pair is a finding on the artefact axis and
    // nothing at all on this one. That asymmetry is the point: the two
    // axes are different questions, and the artefact axis is what still
    // catches a format nothing walks.
    let (status, content) = get(&fx.router, "/asterism/duplicates?axis=content").await;
    assert_eq!(status, StatusCode::OK, "{content}");
    assert_eq!(
        content["groups"].as_array().map(Vec::len),
        Some(0),
        "a marker shared by both rows is not an agreement: {content}"
    );

    // A misspelling is refused. Falling back to the default would
    // answer a question nobody asked, under a `200`.
    let (status, refused) = get(&fx.router, "/asterism/duplicates?axis=perceptual").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unknown axis is not a spelling of `artefact`: {refused}"
    );

    let (status, body) = post(
        &fx.router,
        "/asterism/duplicates/conflicts/resolve",
        serde_json::to_value(keep_apart(&conflict.id)).expect("serialise the confirm"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the confirm answered: {body}");
    assert_eq!(body["resolution"], "kept");
    assert!(
        body["keeper_id"].is_null(),
        "a kept ruling names no keeper: {body}"
    );

    // Answered twice is a conflict, not a 500 and not a silent success.
    let (status, _) = post(
        &fx.router,
        "/asterism/duplicates/conflicts/resolve",
        serde_json::to_value(keep_apart(&conflict.id)).expect("serialise the confirm"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, body) = get(&fx.router, "/asterism/duplicates/conflicts").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().map(Vec::len), Some(0), "the panel is empty");
}

/// The listing and the confirm are published as MCP tools, with input
/// schemas generated from the contract types, and they answer against
/// the same rows HTTP sees.
#[tokio::test(flavor = "multi_thread")]
async fn the_conflict_surface_is_reachable_over_mcp() {
    let fx = Fixture::raised().await;
    let conflict = fx.await_one_conflict().await;
    let session = handshake(&fx.router).await;

    let (status, reply) = mcp_call(
        &fx.router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tools = reply["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list has no tools array: {reply}"));
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert!(
        names.contains(&"duplicate_conflicts") && names.contains(&"duplicate_conflict_resolve"),
        "both halves of the surface are published: {names:?}"
    );

    // The confirm's schema comes from the command type, so the closed
    // answer set is discoverable rather than guessed at — the same
    // reason `asset_add` publishes `on_duplicate`'s three values.
    let resolve = tools
        .iter()
        .find(|tool| tool["name"] == "duplicate_conflict_resolve")
        .expect("the confirm is published");
    let schema = resolve["inputSchema"].to_string();
    for token in ["conflict_id", "resolution", "keeper_id", "folded", "kept"] {
        assert!(
            schema.contains(token),
            "the confirm's schema should follow the command, {token} is missing: {schema}"
        );
    }

    let listed = tool_json(
        &tool_call(
            &fx.router,
            &session,
            10,
            "duplicate_conflicts",
            serde_json::json!({}),
        )
        .await,
    );
    let rows = listed.as_array().expect("an array of questions");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], conflict.id);
    assert_eq!(rows[0]["incumbent"]["id"], fx.incumbent);

    let answered = tool_json(
        &tool_call(
            &fx.router,
            &session,
            11,
            "duplicate_conflict_resolve",
            serde_json::to_value(fold_onto(&conflict.id, &fx.incumbent)).expect("serialise"),
        )
        .await,
    );
    assert_eq!(answered["resolution"], "folded");
    assert_eq!(answered["keeper_id"], fx.incumbent);
    assert_eq!(answered["headstone_id"], fx.newcomer);

    // A domain refusal is a readable tool error, not a protocol one.
    let repeat = tool_call(
        &fx.router,
        &session,
        12,
        "duplicate_conflict_resolve",
        serde_json::to_value(fold_onto(&conflict.id, &fx.incumbent)).expect("serialise"),
    )
    .await;
    assert_eq!(repeat["isError"], true, "answered twice: {repeat}");
    assert_eq!(tool_json(&repeat)["kind"], "Conflict");

    assert!(fx.conflicts().await.is_empty());
}

// ---------------------------------------------------------------- HTTP

async fn get(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build GET");
    send(router, request).await
}

async fn post(
    router: &Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST");
    send(router, request).await
}

async fn send(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router.clone().oneshot(request).await.expect("router");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, body)
}

// ----------------------------------------------------------------- MCP
//
// The same client shape as `mcp_transport_e2e`: legacy session flow,
// answers arriving as SSE `data:` frames.

async fn mcp_call(
    router: &Router,
    session: Option<&str>,
    message: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1")
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json");
    if let Some(session) = session {
        builder = builder.header("mcp-session-id", session);
    }
    let request = builder
        .body(Body::from(message.to_string()))
        .expect("build MCP POST");
    let (status, session_id, bytes) = tokio::time::timeout(Duration::from_secs(20), async {
        let response = router.clone().oneshot(request).await.expect("router");
        let status = response.status();
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        (status, session, bytes)
    })
    .await
    .expect("MCP exchange timed out — the response stream never terminated");
    let text = String::from_utf8_lossy(&bytes);
    let mut last = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).expect("SSE data frame is JSON"))
        .or_else(|| {
            (!bytes.is_empty() && text.trim_start().starts_with('{'))
                .then(|| serde_json::from_slice(&bytes).expect("JSON body"))
        })
        .unwrap_or(serde_json::Value::Null);
    if let (Some(id), serde_json::Value::Object(map)) = (session_id, &mut last) {
        map.insert("__session".into(), serde_json::Value::String(id));
    }
    (status, last)
}

async fn handshake(router: &Router) -> String {
    let (status, reply) = mcp_call(
        router,
        None,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "e2e", "version": "0"},
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "initialize failed: {reply}");
    let session = reply["__session"]
        .as_str()
        .expect("initialize answers with a session id")
        .to_owned();
    let (status, _) = mcp_call(
        router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "initialized notification");
    session
}

async fn tool_call(
    router: &Router,
    session: &str,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let (status, reply) = mcp_call(
        router,
        Some(session),
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tools/call {name}: {reply}");
    assert!(
        reply["error"].is_null(),
        "tools/call {name} answered a protocol error: {reply}"
    );
    reply["result"].clone()
}

fn tool_json(result: &serde_json::Value) -> serde_json::Value {
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result has no text content: {result}"));
    serde_json::from_str(text).expect("tool content is JSON")
}
