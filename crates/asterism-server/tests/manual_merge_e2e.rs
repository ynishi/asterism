//! End-to-end: a person rules that a set of rows is one thing, and the
//! manual merge verb carries it out.
//!
//! The queued half of the fold surface is covered by
//! `duplicate_conflict_resolution_e2e`: that binary drives detection,
//! the queue and the confirm handler. This one covers the other entry
//! point — the verb that reaches the fold without a fingerprint match
//! ever having raised a question — through the service directly. The
//! rules that stop an *automatic* fold (lineage, dispatch output) are
//! deliberately not binding here: they surface as warnings so the panel
//! can say what was overridden, and the run still goes through.
//!
//! # Fixture that carries the weight
//!
//! `three_assets` seeds three rows under one persona and returns their
//! ids in the order the plan will discard them in. `discard_source_kind`
//! and `discard_a_derived_from` are the two axes the warning teeth
//! disagree over — a plain `fs` run and a run where the first discard
//! shares a lineage with the keeper (or was born of a dispatch) share
//! this fixture and vary only the input the axis under test names.
//!
//! `Full` mode throughout, for the reason `duplicate_conflict_resolution_e2e`
//! gives: the pipeline the caller is exercising is the one the worker
//! actually runs, and a `ReadOnly` init would test the merge against a
//! partial version of it.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, MergeAssetsCommand, RegisterPersonaCommand, TrashAssetCommand,
};
use asterism_contract::dto::MergeAssetsDto;
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the merge, not about
/// who ran it.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// One `AddAssetCommand` with the two axes the teeth disagree over
/// (`source_kind` and `derived_from`) exposed and everything else at
/// the shape ordinary callers pass.
fn add_command(
    persona_id: &str,
    source_kind: &str,
    locator: &str,
    occurred_at_ms: i64,
    derived_from: Option<String>,
) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: source_kind.into(),
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
        derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

/// A CoreCtx over a fresh SQLite + Tantivy home, with one persona and
/// three assets under it.
struct Fixture {
    _tmp: tempfile::TempDir,
    core: CoreCtx,
    keeper: String,
    /// The first row folded — the one the warning teeth vary.
    discard_a: String,
    /// The second row folded — a plain `fs` row for every test.
    discard_b: String,
}

impl Fixture {
    /// Ingests three rows: the keeper, then `discard_a` under
    /// `discard_source_kind` and optionally under a `derived_from`
    /// claim, then a plain `discard_b`. The keeper is added first so a
    /// `derived_from: asset:<keeper>` can be resolved on `discard_a`'s
    /// ingest.
    async fn three_assets(discard_source_kind: &str, discard_a_derived_from_keeper: bool) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let corpus = tmp.path().join("corpus");
        std::fs::create_dir_all(&corpus).expect("corpus dir");
        // Write real files so an ingest doesn't 404 in the source_text
        // reader; the merge never reads bytes and the fingerprint jobs
        // that do are asynchronous, so the teeth do not wait for them.
        for name in ["keeper.png", "a.png", "b.png"] {
            std::fs::write(corpus.join(name), b"placeholder\n").expect("write");
        }

        let core = init_core_with(
            &tmp.path().join("asterism.db"),
            Arc::new(LogEmitter),
            CoreMode::Full,
            Some(&tmp.path().join("tantivy")),
        )
        .await
        .expect("init_core");

        let persona = core
            .persona_service
            .register(
                RegisterPersonaCommand {
                    name: "E2E".into(),
                    pack_id: Some(format!("e2e-manual-merge-{}", uuid::Uuid::now_v7())),
                },
                &unattributed(),
            )
            .await
            .expect("register persona");

        let keeper = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    "fs",
                    corpus.join("keeper.png").to_str().unwrap(),
                    1_786_000_000_000,
                    None,
                ),
                &unattributed(),
            )
            .await
            .expect("add keeper")
            .id;

        let derived_from = if discard_a_derived_from_keeper {
            Some(format!("asset:{keeper}"))
        } else {
            None
        };
        let discard_a = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    discard_source_kind,
                    corpus.join("a.png").to_str().unwrap(),
                    1_786_000_001_000,
                    derived_from,
                ),
                &unattributed(),
            )
            .await
            .expect("add discard_a")
            .id;

        let discard_b = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    "fs",
                    corpus.join("b.png").to_str().unwrap(),
                    1_786_000_002_000,
                    None,
                ),
                &unattributed(),
            )
            .await
            .expect("add discard_b")
            .id;

        Self {
            _tmp: tmp,
            core,
            keeper,
            discard_a,
            discard_b,
        }
    }

    /// The command that folds `discard_a` and `discard_b` into
    /// `keeper` — the ruling every teeth here is a variant of.
    fn merge(&self, dry_run: bool) -> MergeAssetsCommand {
        MergeAssetsCommand {
            keeper_id: self.keeper.clone(),
            discard_ids: vec![self.discard_a.clone(), self.discard_b.clone()],
            member_ids: vec![
                self.keeper.clone(),
                self.discard_a.clone(),
                self.discard_b.clone(),
            ],
            dry_run,
        }
    }

    /// Reads `folded_into` off a row over a second connection — the
    /// fold's own effect, which no read DTO carries.
    async fn folded_into(&self, asset_id: &str) -> Option<String> {
        let (isle, driver) =
            asterism_infra::sqlite::open_and_migrate(&self._tmp.path().join("asterism.db"))
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
}

/// A run following a preview reads the answer back on the **same
/// shape** — the port doc contract the DTO is here to carry. The only
/// field the two disagree on is `committed`, and that is by design.
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_predicts_the_run_and_the_run_agrees() {
    let fx = Fixture::three_assets("fs", false).await;

    let preview: MergeAssetsDto = fx
        .core
        .asset_service
        .merge_assets(fx.merge(true), &unattributed())
        .await
        .expect("dry_run runs");

    assert!(
        !preview.committed,
        "a preview reports a prediction, not a run"
    );
    assert!(
        preview.warnings.is_empty(),
        "no rule declined this pair — nothing to warn about"
    );
    assert!(
        preview.refusals.is_empty(),
        "every row was foldable when the plan was checked"
    );
    assert_eq!(
        preview.folded_ids,
        vec![fx.discard_a.clone(), fx.discard_b.clone()],
        "the preview names the rows it would fold, in the plan's order"
    );
    assert_eq!(preview.keeper_id, fx.keeper);

    // Nothing was written by the preview — read `folded_into` off both
    // discards to be sure of it, since the DTO's `committed` flag is
    // the very thing we would be second-guessing.
    assert_eq!(fx.folded_into(&fx.discard_a).await, None);
    assert_eq!(fx.folded_into(&fx.discard_b).await, None);

    let run: MergeAssetsDto = fx
        .core
        .asset_service
        .merge_assets(fx.merge(false), &unattributed())
        .await
        .expect("commit runs");

    assert!(run.committed, "the commit was kept");
    assert!(
        run.warnings.is_empty(),
        "the commit branch never returns warnings — the caller has \
         already seen them on the preview"
    );
    assert_eq!(
        run.folded_ids, preview.folded_ids,
        "the run folded the rows the preview predicted"
    );
    assert_eq!(run.refusals.len(), preview.refusals.len());
    assert_eq!(
        run.keeper_id, preview.keeper_id,
        "one keeper across the two calls"
    );

    // …and now the fold is visible on the rows themselves.
    assert_eq!(
        fx.folded_into(&fx.discard_a).await.as_deref(),
        Some(fx.keeper.as_str())
    );
    assert_eq!(
        fx.folded_into(&fx.discard_b).await.as_deref(),
        Some(fx.keeper.as_str())
    );
    assert_eq!(
        fx.folded_into(&fx.keeper).await,
        None,
        "the keeper stayed live"
    );
}

/// A declaration whose `member_ids` do not match the keeper plus the
/// discards is refused **before** `merge_into` is called — the whole
/// check is `MergePlan::declare`'s, and this teeth asserts the verb
/// does not restate any of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_ruling_without_all_the_rows_is_refused_before_any_write() {
    let fx = Fixture::three_assets("fs", false).await;

    // Discard both rows, but declare only two members — the person
    // ticked two rows on a screen that had three. `MergePlan::declare`
    // is the layer that catches this; the verb refuses on it directly.
    let bad = MergeAssetsCommand {
        keeper_id: fx.keeper.clone(),
        discard_ids: vec![fx.discard_a.clone(), fx.discard_b.clone()],
        member_ids: vec![fx.keeper.clone(), fx.discard_a.clone()],
        dry_run: true,
    };

    let err = fx
        .core
        .asset_service
        .merge_assets(bad, &unattributed())
        .await
        .expect_err("a mismatched declaration is refused");

    assert!(
        matches!(
            err,
            asterism_core::error::DomainError::Validation(ref msg)
                if msg.contains("does not account for every declared member")
        ),
        "the refusal names what MergePlan::declare found: {err}"
    );

    // Nothing moved on either row: the check runs above the transaction.
    assert_eq!(fx.folded_into(&fx.discard_a).await, None);
    assert_eq!(fx.folded_into(&fx.discard_b).await, None);
}

/// A merge over a pair the lineage rule would have declined **warns on
/// the preview** and **still runs on the commit**. The rule stops the
/// automatic fold, not the person's ruling.
#[tokio::test(flavor = "multi_thread")]
async fn a_lineage_pair_warns_on_preview_and_the_run_still_folds() {
    // `discard_a` is derived from `keeper`: ancestor(discard_a) ⊇
    // {keeper}, ancestor(keeper) = {keeper}, so the two share `keeper`
    // as an ancestor and `lineage_connects` says yes.
    let fx = Fixture::three_assets("fs", true).await;

    let preview = fx
        .core
        .asset_service
        .merge_assets(fx.merge(true), &unattributed())
        .await
        .expect("dry_run runs");

    // The **exact** warning list, both ends of the pair and the slug
    // — a subset assertion would let a warning about the wrong pair
    // pass this teeth.
    assert_eq!(preview.warnings.len(), 1, "one warning, no more no less");
    let warning = &preview.warnings[0];
    assert_eq!(
        (
            warning.keeper_id.as_str(),
            warning.headstone_id.as_str(),
            warning.kind.as_str(),
        ),
        (fx.keeper.as_str(), fx.discard_a.as_str(), "lineage"),
        "one warning, on the pair the rule caught"
    );
    // The preview folds anyway — the rule is not binding on a person's
    // ruling.
    assert_eq!(
        preview.folded_ids,
        vec![fx.discard_a.clone(), fx.discard_b.clone()]
    );
    assert!(!preview.committed);

    let run = fx
        .core
        .asset_service
        .merge_assets(fx.merge(false), &unattributed())
        .await
        .expect("commit runs");

    assert!(run.committed, "the person's ruling went through the fold");
    assert!(
        run.warnings.is_empty(),
        "the commit branch never returns warnings"
    );
    assert_eq!(
        fx.folded_into(&fx.discard_a).await.as_deref(),
        Some(fx.keeper.as_str()),
        "the lineage pair was folded — the rule warned, it did not refuse"
    );
}

/// A merge over a pair the dispatch rule would have declined behaves
/// the same way — same warning shape, same "still folds" outcome. The
/// two rules read out of one function, so the teeth's job here is to
/// show that "dispatch" arrives on the wire as its own slug.
///
/// Same fixture builder as the lineage teeth, but the first discard's
/// `source_kind` is `dispatch-file` instead of `fs` and there is no
/// `derived_from` claim. Reusing the fixture keeps the teeth to the
/// axis under test — everything else about the merge is identical.
#[tokio::test(flavor = "multi_thread")]
async fn a_dispatch_pair_warns_the_same_way_and_the_run_still_folds() {
    let fx = Fixture::three_assets("dispatch-file", false).await;

    let preview = fx
        .core
        .asset_service
        .merge_assets(fx.merge(true), &unattributed())
        .await
        .expect("dry_run runs");

    assert_eq!(preview.warnings.len(), 1, "one warning, no more no less");
    let warning = &preview.warnings[0];
    assert_eq!(
        (
            warning.keeper_id.as_str(),
            warning.headstone_id.as_str(),
            warning.kind.as_str(),
        ),
        (fx.keeper.as_str(), fx.discard_a.as_str(), "dispatch"),
        "one warning, and the slug is `dispatch` — not `dispatch-file` \
         or a rendered sentence"
    );
    assert!(!preview.committed);

    let run = fx
        .core
        .asset_service
        .merge_assets(fx.merge(false), &unattributed())
        .await
        .expect("commit runs");

    assert!(run.committed);
    assert_eq!(
        fx.folded_into(&fx.discard_a).await.as_deref(),
        Some(fx.keeper.as_str()),
        "the dispatch product was folded — the rule warned, it did not refuse"
    );
}

/// Refusals from `merge_into` come back on the preview **and** on a
/// commit, and both leave `committed = false`. One refusal abandons
/// the whole merge (all-or-nothing).
#[tokio::test(flavor = "multi_thread")]
async fn refusals_come_back_on_the_preview_and_stop_the_run() {
    let fx = Fixture::three_assets("fs", false).await;

    // Trash the keeper: every discard now refuses with `KeeperTrashed`.
    // Read out of `merge_into` before it writes anything, so the teeth
    // covers both the "no fold happened" and "the transaction was not
    // kept" halves of a refused merge.
    fx.core
        .asset_service
        .trash(
            TrashAssetCommand {
                asset_id: fx.keeper.clone(),
                comment: None,
            },
            &unattributed(),
        )
        .await
        .expect("trash keeper");

    let preview = fx
        .core
        .asset_service
        .merge_assets(fx.merge(true), &unattributed())
        .await
        .expect("dry_run runs — a refusal is not an error");

    assert!(
        !preview.committed,
        "a refused merge is never committed, preview or otherwise"
    );
    assert!(
        preview.folded_ids.is_empty(),
        "the all-or-nothing rule leaves nothing on the folded list when a \
         refusal is reached"
    );
    assert_eq!(
        preview.refusals.len(),
        2,
        "both discards are refused with the same keeper"
    );
    for refusal in &preview.refusals {
        assert_eq!(
            refusal.reason, "the keeper is in the trash",
            "the slug names why: {}",
            refusal.reason
        );
    }
    let refused_ids: Vec<_> = preview
        .refusals
        .iter()
        .map(|r| r.asset_id.clone())
        .collect();
    assert_eq!(
        refused_ids,
        vec![fx.discard_a.clone(), fx.discard_b.clone()]
    );

    // The same command on the commit branch produces the same shape —
    // the two answers agree on refusals, and neither writes anything.
    let run = fx
        .core
        .asset_service
        .merge_assets(fx.merge(false), &unattributed())
        .await
        .expect("commit runs");

    assert!(!run.committed, "a commit that refused is still `false`");
    assert_eq!(run.refusals.len(), preview.refusals.len());
    assert!(run.folded_ids.is_empty());

    assert_eq!(
        fx.folded_into(&fx.discard_a).await,
        None,
        "no discard was moved on the refused run"
    );
    assert_eq!(fx.folded_into(&fx.discard_b).await, None);
}
