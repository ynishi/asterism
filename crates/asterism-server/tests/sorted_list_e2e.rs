//! End-to-end guard for the server-side sort axis on `list`.
//!
//! Wires the real service graph through
//! [`asterism_server::core_init::init_core_with`] and asserts that naming a
//! [`SortSpec`] on [`ListAssetsQuery`] reproduces the order the desktop grid
//! shows for the same `Sort` / `Order` pick.
//!
//! # Why an e2e and not a unit test
//!
//! The ordering the grid displays was assembled from two halves that no
//! single layer owned: SQL chose the arrival order and attached
//! `primary_group_position`, and the client comparator
//! (`lib/sort/card-cmp.ts`) re-sorted on it. Both halves passed their own
//! tests while disagreeing about *which* Group a card's `position` came
//! from, so a card filed in two Groups sorted by the wrong one — visible
//! only by driving the UI (fixed in 727c6a3). With the axis on the wire the
//! same question is answerable in-process, which is what this pins.
//!
//! # Two read paths, one answer
//!
//! `list` (cards) and `list_index` (light rows) are separate methods, and
//! the index one is what the grid reads for a non-search list. It
//! accepted `sort` and dropped it until 2026-07-30 — the desktop client
//! sends no axis, so the only symptom was an HTTP caller silently
//! getting arrival order. `index_and_list_agree_on_every_axis` is the
//! test that would have caught it: it asserts the two paths answer the
//! same sequence, so neither can quietly stop sorting.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, AddAssetToGroupCommand, CreateGroupCommand, RegisterPersonaCommand,
    UpdateAssetMetaCommand,
};
use asterism_contract::query::ListAssetsQuery;
use asterism_contract::sort::{SortOrder, SortSpec, SortTarget};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about ordering, not about
/// who ingested each row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

fn add_command(persona_id: &str, locator: &str, occurred_at_ms: i64) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some("image".into()),
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

/// [`add_command`] with the two metric columns filled in.
///
/// `None` is not a placeholder here but a state under test: a still
/// image has no playback length, and an original whose bytes were never
/// recorded has no size. The axes keep both outside the ordering, which
/// is what the tail assertions below pin.
fn measured_command(
    persona_id: &str,
    locator: &str,
    occurred_at_ms: i64,
    duration_ms: Option<u64>,
    file_size_bytes: Option<u64>,
) -> AddAssetCommand {
    AddAssetCommand {
        duration_ms,
        file_size_bytes,
        ..add_command(persona_id, locator, occurred_at_ms)
    }
}

fn spec(target: SortTarget, order: SortOrder) -> SortSpec {
    SortSpec {
        target,
        order,
        reverse: false,
        collation: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_orders_by_the_axis_the_caller_named() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let older = corpus.join("older.md");
    let newer = corpus.join("newer.md");
    std::fs::write(&older, "older\n").expect("write older");
    std::fs::write(&newer, "newer\n").expect("write newer");

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
                pack_id: Some("e2e-sorted-list".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let older_dto = core
        .asset_service
        .add(
            add_command(&persona.id, older.to_str().unwrap(), 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add older");
    let newer_dto = core
        .asset_service
        .add(
            add_command(&persona.id, newer.to_str().unwrap(), 1_785_000_001_000),
            &unattributed(),
        )
        .await
        .expect("add newer");

    // Two Groups holding the same pair in opposite order: each asset's slot
    // says which Group answered, which is the whole point of the second
    // assertion below. `A` is newest-first on purpose so that whichever
    // Group ends up filtered, its arrangement and the time axis disagree —
    // a fixture where they coincide would pass even with the sort skipped
    // entirely (asserted below).
    let mut groups = Vec::new();
    for (name, order) in [
        ("A", [&newer_dto, &older_dto]),
        ("B", [&older_dto, &newer_dto]),
    ] {
        let group = core
            .asset_service
            .create_group(
                CreateGroupCommand {
                    persona_id: persona.id.clone(),
                    name: name.into(),
                    description: None,
                },
                &unattributed(),
            )
            .await
            .expect("create group");
        for asset in order {
            core.asset_service
                .add_asset_to_group(
                    AddAssetToGroupCommand {
                        asset_id: asset.id.clone(),
                        group_id: group.id.clone(),
                    },
                    &unattributed(),
                )
                .await
                .expect("add to group");
        }
        groups.push(group);
    }
    // Filter on the Group whose id sorts *higher*: the primary-group rule
    // used to answer with the lower id regardless of the filter, so this is
    // the side that fails when that regresses.
    let filtered = if groups[0].id < groups[1].id {
        groups[1].clone()
    } else {
        groups[0].clone()
    };
    let expected_arrangement = if filtered.name == "A" {
        vec![newer_dto.id.clone(), older_dto.id.clone()]
    } else {
        vec![older_dto.id.clone(), newer_dto.id.clone()]
    };
    let newest_first = vec![newer_dto.id.clone(), older_dto.id.clone()];
    // The fixture only discriminates while these two disagree. Ids are
    // time-ordered (uuid v7), so `filtered` is the Group created second
    // today — if that ever stops holding, fail here instead of quietly
    // asserting the same order three times.
    assert_ne!(
        expected_arrangement, newest_first,
        "fixture must keep the arrangement and the time axis apart"
    );

    let query = |sort: Option<SortSpec>| ListAssetsQuery {
        persona_id: Some(persona.id.clone()),
        group_ids: vec![filtered.id.clone()],
        sort,
        ..Default::default()
    };

    // 1. Newest-first: the axis has to actually run, so the answer differs
    //    from the manual arrangement whenever that arrangement is not
    //    already newest-first — which is why the fixture crosses them.
    let by_time = core
        .asset_service
        .list(query(Some(spec(
            SortTarget::OccurredAt,
            SortOrder::Updated,
        ))))
        .await
        .expect("list by occurred_at");
    let ids: Vec<String> = by_time.items.iter().map(|c| c.id.clone()).collect();
    assert_eq!(
        ids, newest_first,
        "occurred_at + updated must answer newest first"
    );
    assert_eq!(by_time.total, Some(2), "total counts the filtered set");
    assert_eq!(by_time.offset, 0);

    // 2. The hand arrangement of the Group being browsed — not the other
    //    Group's, which is what the primary-group rule used to hand back.
    let by_arrangement = core
        .asset_service
        .list(query(Some(spec(SortTarget::Group, SortOrder::Ordered))))
        .await
        .expect("list by group arrangement");
    let ids: Vec<String> = by_arrangement.items.iter().map(|c| c.id.clone()).collect();
    assert_eq!(
        ids, expected_arrangement,
        "group + ordered must follow the filtered Group's own positions"
    );

    // 3. No axis named: the repository's arrival order still stands, which
    //    for a single-Group filter *is* that Group's arrangement. Asking for
    //    nothing and asking for an axis are different requests, and both
    //    have to remain answerable.
    let unsorted = core
        .asset_service
        .list(query(None))
        .await
        .expect("list unsorted");
    let ids: Vec<String> = unsorted.items.iter().map(|c| c.id.clone()).collect();
    assert_eq!(
        ids, expected_arrangement,
        "arrival order for one Group is its arrangement"
    );
}

/// The star rating is answerable as both an axis and a band, on both
/// read paths.
///
/// The two halves are separate wirings — the axis is a `sort_eval`
/// branch fed by `fetch_sortable_assets`, the band is a `QueryParts`
/// predicate — and each has its own way of passing while doing nothing:
/// an axis that never runs answers in arrival order, and a predicate
/// that never runs answers with the whole set. The fixture is built so
/// neither degradation can be mistaken for the right answer: occurrence
/// time runs *opposite* to the star order, so arrival order and every
/// rating order are different sequences.
#[tokio::test(flavor = "multi_thread")]
async fn rating_axis_and_band_are_answerable_over_the_wire() {
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

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-rating".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    //   name      rating   occurred
    //   five      5        oldest
    //   one       1        ↑
    //   three     3        ↑
    //   unrated   —        newest
    //
    // Arrival order is `occurred_at DESC` = [unrated, three, one, five],
    // which shares no position with best-first [five, three, one,
    // unrated] or worst-first [one, three, five, unrated].
    let plan = [
        ("five", Some(5u8), 1_785_000_000_000_i64),
        ("one", Some(1), 1_785_000_001_000),
        ("three", Some(3), 1_785_000_002_000),
        ("unrated", None, 1_785_000_003_000),
    ];
    let mut id = std::collections::HashMap::new();
    for (name, rating, occurred) in plan {
        let path = corpus.join(format!("{name}.md"));
        std::fs::write(&path, format!("{name}\n")).expect("write asset");
        let dto = core
            .asset_service
            .add(
                add_command(&persona.id, path.to_str().unwrap(), occurred),
                &unattributed(),
            )
            .await
            .expect("add asset");
        if let Some(stars) = rating {
            core.asset_service
                .update_meta(
                    UpdateAssetMetaCommand {
                        asset_id: dto.id.clone(),
                        labels: None,
                        register_note: None,
                        cover: None,
                        title: None,
                        rating: Some(stars),
                        modality: None,
                        bundle_id: None,
                    },
                    &unattributed(),
                )
                .await
                .expect("rate asset");
        }
        id.insert(name, dto.id);
    }
    let ordered = |names: &[&str]| -> Vec<String> { names.iter().map(|n| id[n].clone()).collect() };

    let query = |sort: Option<SortSpec>| ListAssetsQuery {
        persona_id: Some(persona.id.clone()),
        sort,
        ..Default::default()
    };
    let list_ids = |q: ListAssetsQuery| {
        let service = core.asset_service.clone();
        async move {
            service
                .list(q)
                .await
                .expect("list")
                .items
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        }
    };

    let arrival = list_ids(query(None)).await;
    assert_eq!(
        arrival,
        ordered(&["unrated", "three", "one", "five"]),
        "arrival order is occurred_at DESC"
    );

    // 1. The axis, both directions. Unrated stays at the tail across the
    //    flip — the property a "0 stars" stand-in would break.
    let best_first = list_ids(query(Some(spec(SortTarget::Rating, SortOrder::Updated)))).await;
    assert_eq!(
        best_first,
        ordered(&["five", "three", "one", "unrated"]),
        "rating names best-first"
    );
    let worst_first = list_ids(query(Some(SortSpec {
        reverse: true,
        ..spec(SortTarget::Rating, SortOrder::Updated)
    })))
    .await;
    assert_eq!(
        worst_first,
        ordered(&["one", "three", "five", "unrated"]),
        "reversed reads worst-first and still tails the unrated"
    );

    // 2. The index path has its own sort branch; it must agree.
    let index_best_first: Vec<String> = core
        .asset_service
        .list_index(query(Some(spec(SortTarget::Rating, SortOrder::Updated))))
        .await
        .expect("index by rating")
        .items
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(
        index_best_first, best_first,
        "index and list must answer the same rating order"
    );

    // 3. The band. Each case is a strict subset of the corpus, and the
    //    unrated asset is in none of them.
    let banded = |min: Option<u8>, max: Option<u8>| {
        let mut q = query(None);
        q.rating_min = min;
        q.rating_max = max;
        list_ids(q)
    };
    assert_eq!(
        banded(Some(3), None).await,
        ordered(&["three", "five"]),
        "rating_min=3 keeps 3 and 5, in arrival order"
    );
    assert_eq!(
        banded(None, Some(1)).await,
        ordered(&["one"]),
        "an upper bound alone excludes the unrated asset"
    );
    assert_eq!(
        banded(Some(2), Some(4)).await,
        ordered(&["three"]),
        "a closed band selects what sits inside it"
    );

    // 4. The band and the axis compose: filter first, then order the
    //    survivors.
    let mut q = query(Some(spec(SortTarget::Rating, SortOrder::Updated)));
    q.rating_min = Some(1);
    assert_eq!(
        list_ids(q).await,
        ordered(&["five", "three", "one"]),
        "the axis orders exactly the banded set"
    );

    // 5. A band outside the scale is refused rather than clamped, on
    //    both read paths — a clamp would answer with the five-star
    //    assets and look like it worked.
    let mut q = query(None);
    q.rating_min = Some(6);
    let err = core
        .asset_service
        .list(q.clone())
        .await
        .expect_err("out-of-range band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m) if m.contains("rating_min")),
        "expected a validation error naming the bound, got {err:?}"
    );
    assert!(
        core.asset_service.list_index(q).await.is_err(),
        "the index path must refuse the same band"
    );

    let mut q = query(None);
    q.rating_min = Some(4);
    q.rating_max = Some(2);
    let err = core
        .asset_service
        .list(q)
        .await
        .expect_err("inverted band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m) if m.contains("rating_max")),
        "expected a validation error naming the inverted band, got {err:?}"
    );
}

/// `list` and `list_index` must answer the same sequence for the same
/// axis.
///
/// The index path is what the grid reads, and it accepted `sort` while
/// ignoring it — the axis was dropped in `list_index`, which had no
/// branch on `query.sort` at all. A test asserting only "the order looks
/// right" would not have caught it either: for a single-Group filter the
/// arrival order *is* the arrangement, so `group` + `ordered` agreed by
/// accident. Hence a fixture where every axis disagrees with arrival
/// order, and a comparison against the path that was already correct.
#[tokio::test(flavor = "multi_thread")]
async fn index_and_list_agree_on_every_axis() {
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

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-index-parity".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Three assets whose label alphabet, cover alphabet and occurrence
    // time all disagree, filed into the Group in a fourth order — so
    // arrival order reproduces none of the axes under test.
    //
    //   filing   label      cover        occurred
    //   1st      c-third    m-middle     oldest
    //   2nd      a-first    z-last       newest
    //   3rd      b-second   a-earliest   middle
    //
    // `cover_hint` is set rather than left to the cover-generation job:
    // an asset with no cover sorts by the tie-break, which would make
    // the `Cover` row of the matrix below pass without the axis running.
    let plan = [
        ("c-third", "m-middle", 1_785_000_000_000_i64),
        ("a-first", "z-last", 1_785_000_002_000),
        ("b-second", "a-earliest", 1_785_000_001_000),
    ];
    let mut ids_in_filing_order = Vec::new();
    for (label, cover, occurred) in plan {
        let path = corpus.join(format!("{label}.md"));
        std::fs::write(&path, format!("{label}\n")).expect("write asset");
        let mut command = add_command(&persona.id, path.to_str().unwrap(), occurred);
        command.labels = vec![label.to_string()];
        command.cover_hint = Some(cover.to_string());
        let dto = core
            .asset_service
            .add(command, &unattributed())
            .await
            .expect("add asset");
        ids_in_filing_order.push(dto.id);
    }

    let group = core
        .asset_service
        .create_group(
            CreateGroupCommand {
                persona_id: persona.id.clone(),
                name: "Parity".into(),
                description: None,
            },
            &unattributed(),
        )
        .await
        .expect("create group");
    for id in &ids_in_filing_order {
        core.asset_service
            .add_asset_to_group(
                AddAssetToGroupCommand {
                    asset_id: id.clone(),
                    group_id: group.id.clone(),
                },
                &unattributed(),
            )
            .await
            .expect("add to group");
    }

    let query = |sort: Option<SortSpec>| ListAssetsQuery {
        persona_id: Some(persona.id.clone()),
        group_ids: vec![group.id.clone()],
        sort,
        ..Default::default()
    };

    // Every axis the wire offers for this fixture. `Cover` is included
    // on purpose: the index projection drops cover text, so an
    // implementation that sorted the light rows instead of the sortable
    // ones would degrade exactly here.
    let axes = [
        (SortTarget::OccurredAt, SortOrder::Updated),
        (SortTarget::CreatedAt, SortOrder::Updated),
        (SortTarget::Tag, SortOrder::Alpha),
        (SortTarget::Tag, SortOrder::Updated),
        (SortTarget::Group, SortOrder::Ordered),
        (SortTarget::Cover, SortOrder::Alpha),
        (SortTarget::Persona, SortOrder::Alpha),
        (SortTarget::Modality, SortOrder::Alpha),
    ];

    let arrival: Vec<String> = core
        .asset_service
        .list_index(query(None))
        .await
        .expect("index unsorted")
        .items
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(
        arrival, ids_in_filing_order,
        "arrival order for one Group is its filing order"
    );

    let mut axes_that_moved = 0;
    for (target, order) in axes {
        let want: Vec<String> = core
            .asset_service
            .list(query(Some(spec(target, order))))
            .await
            .unwrap_or_else(|e| panic!("list by {target:?}/{order:?}: {e}"))
            .items
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let got: Vec<String> = core
            .asset_service
            .list_index(query(Some(spec(target, order))))
            .await
            .unwrap_or_else(|e| panic!("index by {target:?}/{order:?}: {e}"))
            .items
            .iter()
            .map(|i| i.id.clone())
            .collect();
        assert_eq!(
            got, want,
            "index and list disagree on {target:?} / {order:?}"
        );
        if want != arrival {
            axes_that_moved += 1;
        }
    }

    // Without this the whole loop could pass on a fixture where every
    // axis happens to reproduce arrival order — which is how the bug
    // survived. Four axes (occurrence, ingest, label alphabet, cover
    // alphabet) are crossed against the filing order by construction;
    // `persona` / `modality` cannot move a single-persona, single-modality
    // set and are here to prove the two paths agree on that too.
    assert!(
        axes_that_moved >= 4,
        "fixture no longer discriminates: only {axes_that_moved} axes differ from arrival order"
    );
}

/// The differential-sync loop closes without leaving the list response:
/// every card and index row carries `updated_at_ms`, the `updated_at`
/// axis orders by it, and handing the value straight back as
/// `updated_from_ms` returns exactly the rows at or after that instant.
///
/// Each half fails silently on its own. A missing stamp is not a
/// compile error on the wire — a consumer just cannot find its cursor
/// and falls back to `asset_get` per row, which is the N+1 the light
/// path exists to avoid. An axis that never runs answers in arrival
/// order, and a window predicate that never runs answers with the whole
/// set; both look like a working sync until a row goes missing.
///
/// So the fixture crosses all three time axes. Occurrence descends
/// across the three assets, ingest ascends, and the edits run
/// `alpha` then `gamma` (leaving `beta` at its ingest stamp) — which
/// puts modification order in agreement with neither. The sleeps are
/// what make "in agreement with neither" a fact rather than a hope:
/// `update_meta` stamps `Utc::now()`, so without them two edits can land
/// in one millisecond and the ordering claim becomes a coin flip. The
/// preconditions below assert the separation actually happened.
#[tokio::test(flavor = "multi_thread")]
async fn modification_stamp_and_axis_close_the_sync_loop() {
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

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-updated-cursor".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Occurrence descends in creation order, so arrival (`occurred_at
    // DESC`) is [alpha, beta, gamma] while ingest order is the reverse.
    let mut added = std::collections::HashMap::new();
    for (name, occurred) in [
        ("alpha", 1_785_000_002_000_i64),
        ("beta", 1_785_000_001_000),
        ("gamma", 1_785_000_000_000),
    ] {
        let path = corpus.join(format!("{name}.md"));
        std::fs::write(&path, format!("{name}\n")).expect("write asset");
        let dto = core
            .asset_service
            .add(
                add_command(&persona.id, path.to_str().unwrap(), occurred),
                &unattributed(),
            )
            .await
            .expect("add asset");
        added.insert(name, dto);
    }

    // Edit two of the three, `alpha` first. Modification order therefore
    // reads [gamma, alpha, beta] — a sequence neither arrival order nor
    // ingest order produces.
    let edit = |name: &'static str| {
        let service = core.asset_service.clone();
        let asset_id = added[name].id.clone();
        async move {
            service
                .update_meta(
                    UpdateAssetMetaCommand {
                        asset_id,
                        labels: None,
                        register_note: None,
                        cover: None,
                        title: Some(format!("{name} retitled")),
                        rating: None,
                        modality: None,
                        bundle_id: None,
                    },
                    &unattributed(),
                )
                .await
                .expect("edit asset")
        }
    };
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let alpha_edited = edit("alpha").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let gamma_edited = edit("gamma").await;

    let beta_stamp = added["beta"].updated_at_ms;
    assert!(
        beta_stamp < alpha_edited.updated_at_ms
            && alpha_edited.updated_at_ms < gamma_edited.updated_at_ms,
        "fixture precondition: the three modification stamps must be distinct and ordered \
         beta < alpha < gamma (beta={beta_stamp}, alpha={}, gamma={})",
        alpha_edited.updated_at_ms,
        gamma_edited.updated_at_ms
    );

    let ids =
        |names: &[&str]| -> Vec<String> { names.iter().map(|n| added[*n].id.clone()).collect() };
    let query = |sort: Option<SortSpec>| ListAssetsQuery {
        persona_id: Some(persona.id.clone()),
        sort,
        ..Default::default()
    };

    // 1. The stamp reaches both projections, and it is the *current*
    //    value rather than the one the row was ingested with. `beta`
    //    pins the untouched case: its stamp still equals its ingest
    //    stamp, so a mapping that wired `updated_at_ms` to `created_at`
    //    passes on beta and fails on the two edited rows.
    let cards = core
        .asset_service
        .list(query(None))
        .await
        .expect("list")
        .items;
    let card_stamp: std::collections::HashMap<&str, i64> = ["alpha", "beta", "gamma"]
        .iter()
        .map(|name| {
            let id = &added[*name].id;
            let card = cards
                .iter()
                .find(|c| &c.id == id)
                .unwrap_or_else(|| panic!("{name} missing from the page"));
            (*name, card.updated_at_ms)
        })
        .collect();
    assert_eq!(card_stamp["alpha"], alpha_edited.updated_at_ms);
    assert_eq!(card_stamp["gamma"], gamma_edited.updated_at_ms);
    assert_eq!(
        card_stamp["beta"], beta_stamp,
        "an untouched row keeps the stamp it was ingested with"
    );
    let beta_card = cards
        .iter()
        .find(|c| c.id == added["beta"].id)
        .expect("beta card");
    assert_eq!(
        beta_card.updated_at_ms, beta_card.created_at_ms,
        "the two stamps coincide only until something edits the row"
    );
    let alpha_card = cards
        .iter()
        .find(|c| c.id == added["alpha"].id)
        .expect("alpha card");
    assert!(
        alpha_card.updated_at_ms > alpha_card.created_at_ms,
        "an edited row's stamps must have parted, or the field is reading created_at"
    );

    let index = core
        .asset_service
        .list_index(query(None))
        .await
        .expect("index")
        .items;
    for name in ["alpha", "beta", "gamma"] {
        let id = &added[name].id;
        let row = index
            .iter()
            .find(|i| &i.id == id)
            .unwrap_or_else(|| panic!("{name} missing from the index page"));
        assert_eq!(
            row.updated_at_ms, card_stamp[name],
            "index and card must report the same cursor for {name}"
        );
    }

    // 2. The axis, on both read paths. Arrival order is asserted first so
    //    "the sort ran" is distinguishable from "the sort was skipped".
    let list_ids = |q: ListAssetsQuery| {
        let service = core.asset_service.clone();
        async move {
            service
                .list(q)
                .await
                .expect("list")
                .items
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        list_ids(query(None)).await,
        ids(&["alpha", "beta", "gamma"]),
        "arrival order is occurred_at DESC"
    );
    let by_change = list_ids(query(Some(spec(SortTarget::UpdatedAt, SortOrder::Updated)))).await;
    assert_eq!(
        by_change,
        ids(&["gamma", "alpha", "beta"]),
        "updated_at names most-recently-changed first"
    );
    assert_eq!(
        list_ids(query(Some(SortSpec {
            reverse: true,
            ..spec(SortTarget::UpdatedAt, SortOrder::Updated)
        })))
        .await,
        ids(&["beta", "alpha", "gamma"]),
        "reversed reads oldest-change first"
    );
    // Ingest order is the reverse of arrival here, so a `created_at`
    // branch answering the `updated_at` question would produce
    // [gamma, beta, alpha] — close enough to the right answer to slip
    // past a fixture with fewer than three rows.
    assert_ne!(
        list_ids(query(Some(spec(SortTarget::CreatedAt, SortOrder::Updated)))).await,
        by_change,
        "fixture no longer discriminates: ingest order matches modification order"
    );
    let index_by_change: Vec<String> = core
        .asset_service
        .list_index(query(Some(spec(SortTarget::UpdatedAt, SortOrder::Updated))))
        .await
        .expect("index by updated_at")
        .items
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(
        index_by_change, by_change,
        "index and list must answer the same modification order"
    );

    // 3. The round trip. Replaying a card's own stamp as the lower bound
    //    returns that row and everything stamped after it — the
    //    inclusive boundary the doc promises, which is also why the row
    //    on the cursor comes back and a consumer has to be idempotent.
    let mut q = query(Some(spec(SortTarget::UpdatedAt, SortOrder::Updated)));
    q.updated_from_ms = Some(alpha_edited.updated_at_ms);
    assert_eq!(
        list_ids(q).await,
        ids(&["gamma", "alpha"]),
        "the cursor row is re-delivered along with everything newer"
    );

    // Past the newest stamp there is nothing left — the steady state of
    // a poll loop, and the case a predicate that never ran would answer
    // with the whole library.
    let mut q = query(None);
    q.updated_from_ms = Some(gamma_edited.updated_at_ms + 1);
    assert!(
        list_ids(q).await.is_empty(),
        "a cursor past the newest change has nothing to report"
    );
}

/// Playback length and stored size are answerable as axes, in both
/// directions, on both read paths — with the rows carrying no value at
/// the tail either way.
///
/// # The fixture is the claim
///
/// An ordering assertion is only worth its line if the axis under test
/// *disagrees* with the tie-break. This file has shipped two that did
/// not: the `Cover` row of `index_and_list_agree_on_every_axis` once ran
/// over assets that all had `None` for cover, and
/// `list_orders_by_the_axis_the_caller_named` once asserted an order
/// under a single-Group filter where arrival order already *was*
/// `asset_bucket.position`. Both passed with the axis doing nothing.
///
/// So these five rows put four sequences in play, and none of them —
/// nor arrival order, which is `occurred_at` DESC and also the
/// tie-break every axis falls through to — is any of the others:
///
/// | id        | length | size   | occurred |
/// |-----------|--------|--------|----------|
/// | `brief`   |    1 s | 2.0 MB | oldest   |
/// | `feature` |  120 s | 0.5 MB |     ↑    |
/// | `clip`    |   30 s | 9.0 MB |     ↑    |
/// | `still`   |      — | 7.0 MB |     ↑    |
/// | `unsized` |   60 s |      — | newest   |
///
/// The two metrics run *against* each other on purpose — the longest
/// clip is the smallest file — so an implementation reading one column
/// for both axes answers one of the two wrongly rather than looking
/// right on both. And the two absent states are held by different rows,
/// each carrying a mid-range value on the *other* axis: neither `still`
/// nor `unsized` can reach the tail by being extreme, only by being
/// absent. The pairwise check below fails loudly if any of that ever
/// stops holding.
#[tokio::test(flavor = "multi_thread")]
async fn duration_and_size_axes_are_answerable_over_the_wire() {
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

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-metric-axes".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let plan = [
        (
            "brief",
            Some(1_000u64),
            Some(2_000_000u64),
            1_785_000_000_000i64,
        ),
        ("feature", Some(120_000), Some(500_000), 1_785_000_001_000),
        ("clip", Some(30_000), Some(9_000_000), 1_785_000_002_000),
        ("still", None, Some(7_000_000), 1_785_000_003_000),
        ("unsized", Some(60_000), None, 1_785_000_004_000),
    ];
    let mut id = std::collections::HashMap::new();
    for (name, duration_ms, file_size_bytes, occurred) in plan {
        let path = corpus.join(format!("{name}.md"));
        std::fs::write(&path, format!("{name}\n")).expect("write asset");
        let dto = core
            .asset_service
            .add(
                measured_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    occurred,
                    duration_ms,
                    file_size_bytes,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        id.insert(name, dto.id);
    }
    let ordered = |names: &[&str]| -> Vec<String> { names.iter().map(|n| id[n].clone()).collect() };

    let query = |sort: Option<SortSpec>| ListAssetsQuery {
        persona_id: Some(persona.id.clone()),
        sort,
        ..Default::default()
    };
    let list_ids = |q: ListAssetsQuery| {
        let service = core.asset_service.clone();
        async move {
            service
                .list(q)
                .await
                .expect("list")
                .items
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        }
    };
    let index_ids = |q: ListAssetsQuery| {
        let service = core.asset_service.clone();
        async move {
            service
                .list_index(q)
                .await
                .expect("index")
                .items
                .iter()
                .map(|i| i.id.clone())
                .collect::<Vec<_>>()
        }
    };
    let metric = |target: SortTarget, reverse: bool| SortSpec {
        reverse,
        ..spec(target, SortOrder::Updated)
    };

    let arrival = list_ids(query(None)).await;
    assert_eq!(
        arrival,
        ordered(&["unsized", "still", "clip", "feature", "brief"]),
        "arrival order is occurred_at DESC"
    );

    let longest_first = ordered(&["feature", "unsized", "clip", "brief", "still"]);
    let shortest_first = ordered(&["brief", "clip", "unsized", "feature", "still"]);
    let largest_first = ordered(&["clip", "still", "brief", "feature", "unsized"]);
    let smallest_first = ordered(&["feature", "brief", "still", "clip", "unsized"]);

    // Precondition, not decoration: every assertion below is only
    // evidence while these five sequences stay pairwise distinct. If a
    // future edit to the table collapses two of them, this says so
    // instead of letting the collapsed pair pass with the axis skipped.
    let sequences = [
        ("arrival order", &arrival),
        ("longest first", &longest_first),
        ("shortest first", &shortest_first),
        ("largest first", &largest_first),
        ("smallest first", &smallest_first),
    ];
    for (i, (left_name, left)) in sequences.iter().enumerate() {
        for (right_name, right) in &sequences[i + 1..] {
            assert_ne!(
                left, right,
                "fixture no longer discriminates: {left_name} and {right_name} are the \
                 same sequence"
            );
        }
    }

    // 1. Length, both directions. Natural is longest-first, matching the
    //    other numeric axis (`Rating` reads best-first), and `still`
    //    holds the tail across the flip.
    assert_eq!(
        list_ids(query(Some(metric(SortTarget::Duration, false)))).await,
        longest_first,
        "duration names longest first"
    );
    assert_eq!(
        list_ids(query(Some(metric(SortTarget::Duration, true)))).await,
        shortest_first,
        "reversed reads shortest first and still tails the row with no length"
    );

    // 2. Size, both directions, over the same rows — and answering a
    //    different order from length in each direction, which is what
    //    separates the two branches from one branch used twice.
    assert_eq!(
        list_ids(query(Some(metric(SortTarget::FileSize, false)))).await,
        largest_first,
        "file size names largest first"
    );
    assert_eq!(
        list_ids(query(Some(metric(SortTarget::FileSize, true)))).await,
        smallest_first,
        "reversed reads smallest first and still tails the row with no recorded size"
    );

    // 3. The absent state stated on its own, because it is the property
    //    a stand-in number would break rather than fail: folding "no
    //    length" into `0` satisfies shortest-first and puts a still
    //    image at the *head* of longest-first. Each axis has exactly one
    //    absent row, so the tail is a single named id rather than a set.
    for (target, absent) in [
        (SortTarget::Duration, "still"),
        (SortTarget::FileSize, "unsized"),
    ] {
        for reverse in [false, true] {
            let out = list_ids(query(Some(metric(target, reverse)))).await;
            assert_eq!(
                out.last(),
                Some(&id[absent]),
                "{target:?} must keep {absent} at the tail with reverse={reverse}"
            );
        }
    }

    // 4. The index path carries its own sort branch and its own
    //    projection — it is also what the grid reads — so it has to
    //    answer the same sequence for the same axis.
    for target in [SortTarget::Duration, SortTarget::FileSize] {
        for reverse in [false, true] {
            let want = list_ids(query(Some(metric(target, reverse)))).await;
            let got = index_ids(query(Some(metric(target, reverse)))).await;
            assert_eq!(
                got, want,
                "index and list disagree on {target:?} with reverse={reverse}"
            );
        }
    }

    // 4b. The two sequences agreeing is necessary and not sufficient:
    //     they also agreed while the *projection* carried neither metric,
    //     because both sides were ordered server-side and the light rows
    //     only had to arrive in the order handed to them. What the grid
    //     does is different — it sorts these rows itself — so the values
    //     have to be on them, and that is a claim about the payload
    //     rather than about the sequence. Every row is named here, both
    //     absent states included, so a projection that dropped one column
    //     or defaulted an absent value to zero fails on the row that
    //     proves it rather than passing on the four that do not.
    let index_page = core
        .asset_service
        .list_index(query(None))
        .await
        .expect("index page");
    assert_eq!(index_page.items.len(), plan.len(), "the page lost rows");
    for (name, duration_ms, file_size_bytes, _occurred) in plan {
        let row = index_page
            .items
            .iter()
            .find(|i| i.id == id[name])
            .unwrap_or_else(|| panic!("{name} is missing from the index page"));
        assert_eq!(
            row.duration_ms, duration_ms,
            "index row for {name} lost its length"
        );
        assert_eq!(
            row.file_size_bytes, file_size_bytes,
            "index row for {name} lost its size"
        );
    }

    // 5. An inverted band is refused on both read paths rather than
    //    answered with an empty page: the empty page reads as a claim
    //    about the library ("nothing is that long") when the fault is in
    //    the request. Same rule as the rating band one axis over.
    let mut q = query(None);
    q.duration_min_ms = Some(2_000);
    q.duration_max_ms = Some(1_000);
    let err = core
        .asset_service
        .list(q.clone())
        .await
        .expect_err("an inverted length band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m)
            if m.contains("duration_min_ms") && m.contains("duration_max_ms")),
        "expected a validation error naming both ends of the length band, got {err:?}"
    );
    assert!(
        core.asset_service.list_index(q).await.is_err(),
        "the index path must refuse the same length band"
    );

    let mut q = query(None);
    q.size_min_bytes = Some(2_000);
    q.size_max_bytes = Some(1_000);
    let err = core
        .asset_service
        .list(q.clone())
        .await
        .expect_err("an inverted size band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m)
            if m.contains("size_min_bytes") && m.contains("size_max_bytes")),
        "expected a validation error naming both ends of the size band, got {err:?}"
    );
    assert!(
        core.asset_service.list_index(q).await.is_err(),
        "the index path must refuse the same size band"
    );
}
