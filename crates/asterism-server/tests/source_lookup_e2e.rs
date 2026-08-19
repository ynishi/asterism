//! End-to-end: what a Source value arriving twice does.
//!
//! Until V61 the answer came from the database — `(source_kind,
//! source_locator)` was UNIQUE, the second `INSERT` was refused, and
//! `add` read the refusal back out of the driver's error text to decide
//! whether to report "already imported" or "in the trash". The Source
//! value is now looked up *before* an `AssetId` is minted, so a record
//! arriving again is handed the row that was already there and nothing
//! fails.
//!
//! These drive the real `AssetService::add`, because the change is the
//! order of two steps inside it. A repository-level fixture can show
//! that two rows may hold one value; only this level can show that the
//! second arrival did not mint one.
//!
//! Each fixture puts the axis under test in **disagreement** with the
//! rest of it: a control that does mint beside the arrival that does
//! not, and the undeclared registration beside the `Separate` one.
//! Without the control half, "one row" would also be what a broken
//! ingest that refused everything produced.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, MergeAssetsCommand, OnDuplicate, RegisterPersonaCommand,
};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about which arrivals mint a
/// row, not about who registered any of them.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// What `asset.source_locator` holds for a locator spelled the way the
/// caller spells it — the caller's spelling through the wire reader, the
/// value through the storage rendering, which is what `save` does.
///
/// The column carries the tagged form, so a fixture reading it raw has
/// to say so; comparing against the path string would be comparing
/// against the spelling rather than the value.
fn stored(locator: &str) -> String {
    asterism_core::domain::source_locator::SourceLocator::from_wire(locator)
        .expect("a fixture locator")
        .to_storage()
}

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: None,
        occurred_at_ms: 1_785_000_000_000,
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

struct Fixture {
    tmp: tempfile::TempDir,
    core: CoreCtx,
    corpus: std::path::PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let corpus = tmp.path().join("corpus");
        std::fs::create_dir_all(&corpus).expect("corpus dir");
        let core = init_core_with(
            &tmp.path().join("asterism.db"),
            Arc::new(LogEmitter),
            CoreMode::Full,
            Some(&tmp.path().join("tantivy")),
        )
        .await
        .expect("init_core");
        Self { tmp, core, corpus }
    }

    async fn persona(&self, pack: &str) -> String {
        self.core
            .persona_service
            .register(
                RegisterPersonaCommand {
                    name: pack.into(),
                    pack_id: Some(format!("e2e-{pack}-{}", uuid::Uuid::now_v7())),
                },
                &unattributed(),
            )
            .await
            .expect("register persona")
            .id
    }

    /// Writes a file into the corpus and returns its path as a locator.
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.corpus.join(name);
        std::fs::write(&path, bytes).expect("write file");
        path.to_str().expect("utf-8 path").to_string()
    }

    async fn add(&self, persona: &str, locator: &str) -> String {
        self.core
            .asset_service
            .add(add_command(persona, locator), &unattributed())
            .await
            .expect("add asset")
            .id
    }

    async fn add_declaring(&self, persona: &str, locator: &str, strategy: OnDuplicate) -> String {
        let mut command = add_command(persona, locator);
        command.on_duplicate = Some(strategy);
        self.core
            .asset_service
            .add(command, &unattributed())
            .await
            .expect("add asset")
            .id
    }

    /// Rules that `discard` and `keeper` are one thing, and carries it
    /// out — the real verb, committed, with the outcome asserted here
    /// rather than at the call sites. A merge that refused would
    /// otherwise leave every assertion below testing an unfolded pair.
    async fn fold(&self, discard: &str, into_keeper: &str) {
        let outcome = self
            .core
            .asset_service
            .merge_assets(
                MergeAssetsCommand {
                    keeper_id: into_keeper.to_string(),
                    discard_ids: vec![discard.to_string()],
                    member_ids: vec![into_keeper.to_string(), discard.to_string()],
                    dry_run: false,
                },
                &unattributed(),
            )
            .await
            .expect("merge runs");
        assert!(outcome.committed, "the fixture must actually fold");
        assert_eq!(
            outcome.folded_ids,
            vec![discard.to_string()],
            "…and fold the row it was told to: {:?}",
            outcome.refusals
        );
    }

    /// Throws a row away through the real verb. Its whole point here is
    /// what it does *not* do: `trashed_at` lands on the keeper and
    /// nothing touches the headstone pointing at it, which is the state
    /// the dead-end assertions are about.
    async fn trash(&self, asset: &str) {
        self.core
            .asset_service
            .trash(
                asterism_contract::command::TrashAssetCommand {
                    asset_id: asset.to_string(),
                    comment: None,
                },
                &unattributed(),
            )
            .await
            .expect("trash the keeper");
    }

    /// Every asset row in the database, read over a second connection:
    /// what was *minted* is not visible through any read DTO, and the
    /// count is the whole question here.
    async fn asset_rows(&self) -> Vec<(uuid::Uuid, uuid::Uuid, String)> {
        let (isle, driver) =
            asterism_infra::sqlite::open_and_migrate(&self.tmp.path().join("asterism.db"))
                .await
                .expect("second isle");
        let rows = isle
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, persona_id, source_locator FROM asset ORDER BY created_at, id",
                )?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<_, _>>()
            })
            .await
            .expect("read the asset table");
        driver.shutdown().await.ok();
        rows
    }
}

/// **A re-scan mints nothing.** One scan run twice: the second run
/// returns the first run's id and the library is the same size.
///
/// This is the assertion the whole ordering exists for. `ScanMode::
/// Enumerate` emits every current item on every sweep, so an ingest that
/// minted first and asked afterwards would produce one copy of the
/// library per sweep.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_scan_of_one_corpus_mints_nothing_and_answers_with_the_first_ids() {
    let fx = Fixture::new().await;
    let persona = fx.persona("rescan").await;

    let one = fx.file("one.png", b"the first picture\n");
    let two = fx.file("two.png", b"the second picture\n");

    let first: Vec<String> = vec![fx.add(&persona, &one).await, fx.add(&persona, &two).await];
    assert_eq!(
        fx.asset_rows().await.len(),
        2,
        "the first sweep mints, or the second sweep proves nothing"
    );

    let second: Vec<String> = vec![fx.add(&persona, &one).await, fx.add(&persona, &two).await];

    assert_eq!(
        second, first,
        "the second sweep is the same records arriving again, so it is answered with their ids"
    );
    let rows = fx.asset_rows().await;
    assert_eq!(
        rows.len(),
        2,
        "…and nothing was minted for them: {} rows after two sweeps",
        rows.len()
    );

    // The control: a file the sweep has not seen before still mints, so
    // the two assertions above are about re-arrival rather than about an
    // ingest that stopped registering anything.
    let three = fx.file("three.png", b"a new picture\n");
    let minted = fx.add(&persona, &three).await;
    assert!(!first.contains(&minted));
    assert_eq!(fx.asset_rows().await.len(), 3);
}

/// **Two rows may hold one Source value — when the caller says so.**
///
/// Both halves are in one fixture on purpose: the `Separate` half alone
/// would pass against an ingest that had never stopped minting, and the
/// undeclared half alone would pass against one that could no longer
/// mint at all. The declaration is what the two halves differ by.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_registration_mints_only_where_it_declares_separate() {
    let fx = Fixture::new().await;
    let persona = fx.persona("separate").await;

    // The lane that re-scans: one path, registered twice, no
    // declaration. One row.
    let rescanned = fx.file("rescanned.png", b"a picture that is scanned twice\n");
    let first = fx.add(&persona, &rescanned).await;
    let again = fx.add(&persona, &rescanned).await;
    assert_eq!(
        first, again,
        "an undeclared re-arrival is that record again"
    );

    // The lane that writes every run's output to one path, and says so.
    // Same shape of arrival, opposite answer — which is the only thing
    // that makes the declaration observable.
    let overwritten = fx.file("run-latest.png", b"whatever the last run produced\n");
    let run_one = fx.add(&persona, &overwritten).await;
    let run_two = fx
        .add_declaring(&persona, &overwritten, OnDuplicate::Separate)
        .await;
    assert_ne!(
        run_one, run_two,
        "`Separate` says this lane produces identical material deliberately: keep both rows"
    );

    let locators: Vec<String> = fx
        .asset_rows()
        .await
        .into_iter()
        .map(|(_, _, locator)| locator)
        .collect();
    let (rescanned, overwritten) = (stored(&rescanned), stored(&overwritten));
    assert_eq!(
        locators.iter().filter(|l| *l == &rescanned).count(),
        1,
        "the undeclared path holds one row: {locators:?}"
    );
    assert_eq!(
        locators.iter().filter(|l| *l == &overwritten).count(),
        2,
        "the declared one holds two: {locators:?}"
    );
}

/// **A path that was folded away answers with the keeper.**
///
/// The fold leaves the locator on the headstone — the keeper carries the
/// path *it* was imported from, not this one — so the ingest lookup finds
/// a row that no listing and no duplicate group will ever show. Handing
/// that id back is handing the importer a receipt for something it cannot
/// open. The ruling said these two rows are one thing; after it, this
/// path names the row that survived.
///
/// Three fixtures in disagreement, because "the keeper's id" is also
/// what several broken implementations return:
///
/// * the same path is asserted to answer with **its own** id *before*
///   the fold, so the change afterwards is the fold's
/// * a third path nobody ruled on still answers with its own id, so an
///   implementation that resolved everything to one row fails
/// * the row count never moves, so an implementation that answered by
///   minting a fresh row fails
#[tokio::test(flavor = "multi_thread")]
async fn a_folded_locator_resolves_to_its_keeper() {
    let fx = Fixture::new().await;
    let persona = fx.persona("folded").await;

    // Distinct bytes on purpose: this is a person ruling that two rows
    // are one thing, which needs no fingerprint agreement, and identical
    // files would let the asynchronous hash job have an opinion about
    // the pair while the assertions run.
    let filed_twice = fx.file("filed-twice.png", b"the copy that was filed again\n");
    let keeps = fx.file("keeps.png", b"the row the person kept\n");
    let unruled = fx.file("unruled.png", b"a picture nobody ruled on\n");

    let headstone = fx.add(&persona, &filed_twice).await;
    let keeper = fx.add(&persona, &keeps).await;
    let untouched = fx.add(&persona, &unruled).await;
    assert_eq!(fx.asset_rows().await.len(), 3);

    assert_eq!(
        fx.add(&persona, &filed_twice).await,
        headstone,
        "before the ruling this path is its own row, which is what the ruling changes"
    );

    fx.fold(&headstone, &keeper).await;

    assert_eq!(
        fx.add(&persona, &filed_twice).await,
        keeper,
        "after the ruling the path names the row that survived it, not the headstone"
    );
    assert_eq!(
        fx.asset_rows().await.len(),
        3,
        "…and answering did not mint anything"
    );

    assert_eq!(
        fx.add(&persona, &unruled).await,
        untouched,
        "a path no ruling touched still answers with its own row"
    );
    assert_eq!(fx.asset_rows().await.len(), 3);
}

/// **A chain is followed to the end.** Two rulings made one after the
/// other leave `A → B → C`: the second one folds `B`, and nothing
/// rewrites the headstone already pointing at it.
///
/// `B` is asserted to be `A`'s answer *between* the two rulings, so a
/// walk that stops after one hop cannot pass here — it would answer with
/// `B`, which is by then a headstone in no listing.
#[tokio::test(flavor = "multi_thread")]
async fn a_fold_chain_resolves_to_the_end() {
    let fx = Fixture::new().await;
    let persona = fx.persona("chain").await;

    let first = fx.file("first.png", b"the row folded first\n");
    let middle = fx.file(
        "middle.png",
        b"the row that kept it, then was folded itself\n",
    );
    let last = fx.file("last.png", b"the row that survived both rulings\n");

    let a = fx.add(&persona, &first).await;
    let b = fx.add(&persona, &middle).await;
    let c = fx.add(&persona, &last).await;

    fx.fold(&a, &b).await;
    assert_eq!(
        fx.add(&persona, &first).await,
        b,
        "one ruling in, the first path names B"
    );

    fx.fold(&b, &c).await;
    assert_eq!(
        fx.add(&persona, &first).await,
        c,
        "two rulings in, it names C — B is a headstone now and would be an id nothing can show"
    );
    assert_eq!(
        fx.add(&persona, &middle).await,
        c,
        "and so does B's own path, one hop from the end"
    );
    assert_eq!(
        fx.asset_rows().await.len(),
        3,
        "three rows throughout: nothing was minted for any of those arrivals"
    );
}

/// **A chain that leads nowhere mints once, not once per sweep.**
///
/// The shape: a path is filed twice, the duplicate is ruled on, and the
/// keeper is later thrown away. The headstone is now live (a fold writes
/// no `trashed_at`), still the oldest row carrying that path, and points
/// at a row the ingest scope cannot answer with.
///
/// So it is the row the lookup reaches first on every single sweep. If a
/// dead end ends the lookup, the answer is "unregistered" every time and
/// the freshly minted row is never once seen — the folder grows a copy
/// of the library per sweep, which is the failure this whole file is
/// named after. The dead end has to send the question to the next row
/// holding the path instead.
///
/// The first mint is asserted too, and it is the half that keeps this
/// from passing on an implementation that answers with the headstone:
/// nothing live held the path at that moment, and minting was right.
#[tokio::test(flavor = "multi_thread")]
async fn a_dead_end_chain_does_not_mint_a_second_time() {
    let fx = Fixture::new().await;
    let persona = fx.persona("dead-end").await;

    let filed_twice = fx.file("filed-twice.png", b"the copy that was filed again\n");
    let keeps = fx.file("keeps.png", b"the row the person kept, then threw away\n");

    let headstone = fx.add(&persona, &filed_twice).await;
    let keeper = fx.add(&persona, &keeps).await;
    fx.fold(&headstone, &keeper).await;
    fx.trash(&keeper).await;
    assert_eq!(
        fx.asset_rows().await.len(),
        2,
        "the fold and the trash moved no rows: the headstone is still standing"
    );

    // Sweep one. Nothing a person can open holds this path, so a new row
    // is right — and this is the only arrival in the test that mints.
    let minted = fx.add(&persona, &filed_twice).await;
    assert_ne!(
        minted, headstone,
        "a headstone is in no listing; handing it back would be handing over an invisible id"
    );
    assert_ne!(minted, keeper, "and the keeper is in the trash");
    assert_eq!(fx.asset_rows().await.len(), 3);

    // Sweep two, and three. `ScanMode::Enumerate` re-emits the same path
    // every time, and every time the lookup starts at the same headstone.
    for sweep in 2..=3 {
        assert_eq!(
            fx.add(&persona, &filed_twice).await,
            minted,
            "sweep {sweep} must find the row sweep one minted, past the dead end"
        );
        assert_eq!(
            fx.asset_rows().await.len(),
            3,
            "sweep {sweep} minted a row: this is the copy-per-sweep failure"
        );
    }
}

/// **The lookup is persona-scoped.** Persona B importing a path persona
/// A already holds is B's first import, not a duplicate — being handed
/// A's row would hand B a row B cannot see.
///
/// Asserted against A's own re-import in the same fixture, so "B minted"
/// cannot be read as "everything mints".
#[tokio::test(flavor = "multi_thread")]
async fn two_personas_holding_one_path_are_two_first_imports() {
    let fx = Fixture::new().await;
    let a = fx.persona("persona-a").await;
    let b = fx.persona("persona-b").await;

    let shared = fx.file("shared.png", b"a file on a disk two people read\n");

    let a_row = fx.add(&a, &shared).await;
    let b_row = fx.add(&b, &shared).await;
    assert_ne!(
        a_row, b_row,
        "B is saying this file is B's for the first time, which is not a duplicate resolution"
    );

    // A re-importing is still a re-arrival: the scope is the persona,
    // not "always mint".
    assert_eq!(fx.add(&a, &shared).await, a_row);

    let rows = fx.asset_rows().await;
    assert_eq!(rows.len(), 2, "one row each, and no third: {rows:?}");
    let owner_of = |id: &str| -> uuid::Uuid {
        rows.iter()
            .find(|(row_id, _, _)| row_id.to_string() == id)
            .map(|(_, persona, _)| *persona)
            .unwrap_or_else(|| panic!("no row {id}"))
    };
    assert_eq!(owner_of(&a_row).to_string(), a, "A's row is A's");
    assert_eq!(owner_of(&b_row).to_string(), b, "and B's is B's");
}
