//! End-to-end guard for the search read path's filter surface.
//!
//! Wires the real service graph through
//! [`asterism_server::core_init::init_core`] in [`CoreMode::Full`] — real
//! SQLite, real Tantivy, real job worker — and asserts that a text search
//! honours the grid's filter chips.
//!
//! The filter is [`ListAssetsQuery`] verbatim, so it also carries one
//! field this path *cannot* honour: `sort`. The last two cases pin that
//! it is refused rather than dropped, which is the same class of defect
//! as the one below (a filter field the search path accepted and ignored)
//! seen from the other side.
//!
//! # Why an e2e and not a unit test
//!
//! The bug this pins was invisible at every layer taken alone: the SQL
//! builder applied every filter, the Tantivy index ranked correctly, and
//! the Query Group evaluator intersected the two properly. Only the read
//! path skipped the intersection, dropping everything but `persona_id`
//! (`AssetService::search`). Catching that needs both halves live in one
//! process, which is exactly what `init_core` assembles.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand, UpdateAssetMetaCommand};
use asterism_contract::query::{ListAssetsQuery, SearchAssetsQuery};
use asterism_contract::sort::{SortOrder, SortSpec, SortTarget};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the search filter, not
/// about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// Filter with every axis disabled — the "no chip lit" baseline each
/// case narrows from.
fn blank_filter() -> ListAssetsQuery {
    ListAssetsQuery {
        persona_id: None,
        modality: None,
        occurred_from_ms: None,
        occurred_until_ms: None,
        created_from_ms: None,
        created_until_ms: None,
        updated_from_ms: None,
        updated_until_ms: None,
        tag_ids: Vec::new(),
        // Inert with no tag named; `Any` is what an omitting caller gets.
        tag_match: asterism_contract::query::TagMatch::Any,
        group_ids: Vec::new(),
        session_id: None,
        label: None,
        text_match: None,
        format: None,
        color: None,
        rating_min: None,
        rating_max: None,
        album_meta_key: None,
        album_meta_value: None,
        duration_min_ms: None,
        duration_max_ms: None,
        size_min_bytes: None,
        size_max_bytes: None,
        pixels_min: None,
        pixels_max: None,
        viewer_subject: None,
        trash: None,
        // No axis named: these cases assert on the filter, so they read
        // the repository's arrival order (see `sort` on the query).
        sort: None,
        offset: 0,
        limit: 50,
    }
}

fn add_command(
    persona_id: &str,
    locator: &str,
    modality: &str,
    occurred_at_ms: i64,
) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some(modality.to_string()),
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

/// [`add_command`] with the two metric columns filled in. `None` on
/// either is a state under test, not a placeholder: a still image has no
/// playback length, and an original whose bytes were never recorded has
/// no size.
fn measured_command(
    persona_id: &str,
    locator: &str,
    modality: &str,
    occurred_at_ms: i64,
    duration_ms: Option<u64>,
    file_size_bytes: Option<u64>,
) -> AddAssetCommand {
    AddAssetCommand {
        duration_ms,
        file_size_bytes,
        ..add_command(persona_id, locator, modality, occurred_at_ms)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn search_honours_the_active_filter_chips() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    // Two documents share the query term across different modalities; the
    // third is the negative control that must never match the text.
    let a = corpus.join("a.md");
    let b = corpus.join("b.md");
    let c = corpus.join("c.md");
    std::fs::write(
        &a,
        "Stargazing on the rooftop, three constellations before the cold.\n",
    )
    .expect("write a");
    std::fs::write(
        &b,
        "We went stargazing again and talked about shared archives.\n",
    )
    .expect("write b");
    std::fs::write(&c, "Morning coffee, overcast all day, nothing recorded.\n").expect("write c");

    // `Full` mode takes the writer lock and spawns the job worker, so the
    // `index_rebuild` job that feeds Tantivy actually runs.
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
                pack_id: Some("e2e-search-filter".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let a_dto = core
        .asset_service
        .add(
            add_command(&persona.id, a.to_str().unwrap(), "image", 1_785_000_000_000),
            &unattributed(),
        )
        .await
        .expect("add a");
    let b_dto = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                b.to_str().unwrap(),
                "dialogue",
                1_785_000_001_000,
            ),
            &unattributed(),
        )
        .await
        .expect("add b");
    core.asset_service
        .add(
            add_command(&persona.id, c.to_str().unwrap(), "image", 1_785_000_002_000),
            &unattributed(),
        )
        .await
        .expect("add c");

    // Indexing is asynchronous (enqueue on add, worker reads the file and
    // commits to Tantivy). Poll rather than sleep a fixed amount so a slow
    // machine does not turn into a flake.
    let mut unfiltered = None;
    for _ in 0..120 {
        let page = core
            .asset_service
            .search(SearchAssetsQuery {
                text: "stargazing".into(),
                filter: blank_filter(),
            })
            .await
            .expect("search");
        if page.items.len() == 2 {
            unfiltered = Some(page);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let unfiltered = unfiltered.expect("both stargazing docs indexed within 30s");
    assert_eq!(unfiltered.matched, 2, "text alone matches a and b, not c");
    assert!(
        !unfiltered.truncated,
        "three documents is nowhere near the candidate ceiling, so the \
         shortlist must not claim there is more behind it"
    );

    // The regression: with `modality=image` lit, only `a` may come back.
    // Before the fix every axis but `persona_id` was dropped here, so this
    // returned both documents.
    let mut filter = blank_filter();
    filter.modality = Some("image".into());
    let filtered = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("filtered search");
    assert_eq!(
        filtered.items.len(),
        1,
        "modality chip must narrow the text search"
    );
    assert_eq!(
        filtered.items[0].id, a_dto.id,
        "the image, not the dialogue"
    );
    assert_eq!(
        filtered.matched, 1,
        "`matched` counts the candidates that survived the chip"
    );

    // A chip that excludes every hit yields an empty page rather than
    // silently falling back to the unfiltered set.
    let mut filter = blank_filter();
    filter.modality = Some("tape".into());
    let empty = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("excluding search");
    assert!(empty.items.is_empty(), "no hit survives the chip");
    assert_eq!(empty.matched, 0);

    // The star band is the newest chip and travels the same road: it is
    // a `QueryParts` predicate, and search reaches `QueryParts` through
    // `filter_ids` rather than through `list`. Sharing the builder is
    // what should make this work, and this is what proves it does — the
    // defect this file exists for was exactly a path that had the
    // builder available and did not use it.
    //
    // `a` is rated, `b` is not; both match the text, so the band has to
    // do the narrowing.
    let rated_dto = core
        .asset_service
        .update_meta(
            UpdateAssetMetaCommand {
                asset_id: a_dto.id.clone(),
                labels: None,
                register_note: None,
                cover: None,
                title: None,
                rating: Some(4),
                modality: None,
                bundle_id: None,
            },
            &unattributed(),
        )
        .await
        .expect("rate a");

    let mut filter = blank_filter();
    filter.rating_min = Some(4);
    let rated = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("rating-filtered search");
    assert_eq!(
        rated.items.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        vec![a_dto.id.clone()],
        "the rating band must narrow the text search"
    );
    assert_eq!(rated.matched, 1);

    // And an upper bound alone drops the unrated hit rather than
    // sweeping it in: `b` has no rating, so it belongs to no band.
    let mut filter = blank_filter();
    filter.rating_max = Some(5);
    let banded = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("upper-bound search");
    assert_eq!(
        banded
            .items
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>(),
        vec![a_dto.id.clone()],
        "an unrated hit is outside every band, including a wide one"
    );

    // A band that excludes the only rated hit answers empty rather than
    // falling back to the unfiltered set.
    let mut filter = blank_filter();
    filter.rating_min = Some(5);
    let none = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("narrow band search");
    assert!(none.items.is_empty(), "no hit sits in the 5-star band");

    // The modification window rides the same `filter_ids` road, and it is
    // the chip an external agent syncs on ("what changed since I last
    // looked"), so the search path has to honour it too.
    //
    // The window is a single instant — `[stamp, stamp]` — which is only a
    // non-empty answer because **both** ends are inclusive. A half-open
    // upper bound, the shape `occurred_until_ms` uses, returns nothing
    // here.
    //
    // Both hits match the text, so the window does the narrowing: `a` was
    // re-stamped by the rating edit above, `b` still carries its ingest
    // stamp. That precondition is asserted rather than assumed — if the
    // two landed in the same millisecond the case would be testing
    // nothing, and this says so instead of passing quietly.
    assert!(
        b_dto.updated_at_ms < rated_dto.updated_at_ms,
        "fixture precondition: the rating edit must land after b's ingest \
         stamp (b={}, a={})",
        b_dto.updated_at_ms,
        rated_dto.updated_at_ms
    );
    let mut filter = blank_filter();
    filter.updated_from_ms = Some(rated_dto.updated_at_ms);
    filter.updated_until_ms = Some(rated_dto.updated_at_ms);
    let changed = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("modification-window search");
    assert_eq!(
        changed
            .items
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>(),
        vec![a_dto.id.clone()],
        "the modification window must narrow the text search, and both of \
         its ends are inclusive"
    );

    // The same window read as an ingest window answers differently: `a`
    // was ingested long before it was edited, so nothing sits there. This
    // is what fails if the two axes are cross-wired — a mistake that a
    // single-axis fixture cannot see, because every row's stamps move
    // together until something edits one of them.
    let mut filter = blank_filter();
    filter.created_from_ms = Some(rated_dto.updated_at_ms);
    let by_ingest = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect("ingest-window search");
    assert!(
        by_ingest.items.is_empty(),
        "no document was *ingested* at the moment `a` was edited; got {:?}",
        by_ingest
            .items
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    );

    // An impossible band is a `400`, not an empty page.
    let mut filter = blank_filter();
    filter.rating_min = Some(4);
    filter.rating_max = Some(2);
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect_err("an inverted band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m) if m.contains("rating_min")),
        "expected a validation error naming the band, got {err:?}"
    );

    // `filter` is the list query verbatim, so it carries a `sort` axis
    // this path cannot honour: the answer order is the BM25 ranking. A
    // named axis is refused rather than accepted and dropped — the
    // asymmetry that used to sit here (a misspelled axis was a parse
    // error, a well-spelled one was silently discarded) meant the
    // spelling decided whether the caller heard anything back.
    //
    // The unfiltered case at the top of this test is the other half: it
    // passes `sort: None` and gets its two hits, so the guard rejects the
    // axis rather than the search.
    let mut filter = blank_filter();
    filter.sort = Some(SortSpec {
        target: SortTarget::OccurredAt,
        order: SortOrder::Updated,
        reverse: false,
        collation: None,
    });
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "stargazing".into(),
            filter,
        })
        .await
        .expect_err("a sort axis must be refused on the search path");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m) if m.contains("relevance")),
        "expected a validation error explaining the relevance order, got {err:?}"
    );

    // And the refusal does not depend on the request happening to match
    // something: empty text short-circuits to an empty page, so a guard
    // placed after that early return would answer `200` here and `400`
    // for the same unsupported request one character later.
    let mut filter = blank_filter();
    filter.sort = Some(SortSpec {
        target: SortTarget::Cover,
        order: SortOrder::Alpha,
        reverse: true,
        collation: None,
    });
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "   ".into(),
            filter,
        })
        .await
        .expect_err("whether a request is answerable cannot depend on its text");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m) if m.contains("relevance")),
        "expected the same refusal for empty text, got {err:?}"
    );
}

/// `search_ids` — the retrieval reduced to its rank order (the second
/// composition form; the `✦ Relevance` grid axis).
///
/// Three things are asserted, and the fixture is built so none of them
/// can pass by accident:
///
///   1. **Rank order survives.** The better match is the *older* asset,
///      so the answer disagrees with the grid's default order
///      (`occurred_at` DESC) and with arrival order. An implementation
///      that returned `filter_ids`' own order, or skipped the rank
///      restore, would come back the other way round.
///   2. **The filter still prunes**, and does so *after* ranking: the
///      chip drops the top-ranked candidate and keeps the second, which
///      an implementation that filtered before ranking (or not at all)
///      could not produce.
///   3. **`candidates_considered` describes the net, not the answer.**
///      With the chip lit it stays 2 while one id comes back, so it
///      cannot be `ids.len()` under another name.
#[tokio::test(flavor = "multi_thread")]
async fn search_ids_returns_rank_order_and_reports_the_net() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    // `close` repeats the term in a short body; `distant` mentions it
    // once inside a long one. BM25 (term frequency, length
    // normalisation) puts `close` first — and `close` is the *older*
    // asset, so rank order and the grid's default `occurred_at` DESC
    // order point opposite ways. Without that disagreement this test
    // would pass with the rank restore deleted.
    let close = corpus.join("close.md");
    let distant = corpus.join("distant.md");
    let control = corpus.join("control.md");
    std::fs::write(&close, "Kestrel. Kestrel again, kestrel still.\n").expect("write close");
    std::fs::write(
        &distant,
        "A long entry about the harbour, the ferry timetable, the wind \
         off the water and the way the evening light falls across the \
         quay, in which a kestrel is mentioned exactly once before the \
         conversation moves on to dinner and the walk home.\n",
    )
    .expect("write distant");
    std::fs::write(&control, "Nothing here matches the query term.\n").expect("write control");

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
                pack_id: Some("e2e-search-ids".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let close_dto = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                close.to_str().unwrap(),
                "image",
                1_785_000_000_000,
            ),
            &unattributed(),
        )
        .await
        .expect("add close");
    let distant_dto = core
        .asset_service
        .add(
            add_command(
                &persona.id,
                distant.to_str().unwrap(),
                "dialogue",
                1_785_000_002_000,
            ),
            &unattributed(),
        )
        .await
        .expect("add distant");
    core.asset_service
        .add(
            add_command(
                &persona.id,
                control.to_str().unwrap(),
                "image",
                1_785_000_003_000,
            ),
            &unattributed(),
        )
        .await
        .expect("add control");

    // Indexing is asynchronous; poll like the case above rather than
    // sleeping a fixed amount.
    let mut ranked = None;
    for _ in 0..120 {
        let found = core
            .asset_service
            .search_ids(SearchAssetsQuery {
                text: "kestrel".into(),
                filter: blank_filter(),
            })
            .await
            .expect("search_ids");
        if found.ids.len() == 2 {
            ranked = Some(found);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let ranked = ranked.expect("both kestrel docs indexed within 30s");

    // (1) Rank order, best first — the reverse of both arrival order and
    // the grid's default `occurred_at` DESC.
    assert_eq!(
        ranked.ids,
        vec![close_dto.id.clone(), distant_dto.id.clone()],
        "ids must come back in rank order (the dense mention first), not \
         in arrival or occurred order"
    );
    assert_eq!(
        ranked.candidates_considered, 2,
        "two documents carry the term; the control must not be counted"
    );
    assert!(
        !ranked.truncated,
        "three documents is nowhere near the candidate ceiling"
    );

    // (2) + (3) The chip prunes the *top-ranked* candidate, and the net
    // stays reported at its own width.
    let mut filter = blank_filter();
    filter.modality = Some("dialogue".into());
    let narrowed = core
        .asset_service
        .search_ids(SearchAssetsQuery {
            text: "kestrel".into(),
            filter,
        })
        .await
        .expect("narrowed search_ids");
    assert_eq!(
        narrowed.ids,
        vec![distant_dto.id.clone()],
        "the modality chip must drop the better match and keep the one it \
         selects — filtering happens after ranking, not instead of it"
    );
    assert_eq!(
        narrowed.candidates_considered, 2,
        "`candidates_considered` counts what the retriever looked at (2), \
         not what survived the filter (1)"
    );
    assert!(!narrowed.truncated);

    // Empty text is answered, not refused: no candidates were looked at,
    // so the honest report is an empty order over a zero-wide net.
    let blank = core
        .asset_service
        .search_ids(SearchAssetsQuery {
            text: "   ".into(),
            filter: blank_filter(),
        })
        .await
        .expect("blank search_ids");
    assert!(blank.ids.is_empty());
    assert_eq!(blank.candidates_considered, 0);

    // An impossible band, however, is refused whatever the query — the
    // same terms as `search` (`metric_bands_narrow_the_text_search`,
    // case 8). The two bodies say they validate alike, and a caller
    // cannot be asked to remember which of two sibling read paths checks
    // a band and which quietly answers nothing.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(2_000);
    filter.duration_max_ms = Some(1_000);
    let err = core
        .asset_service
        .search_ids(SearchAssetsQuery {
            text: "   ".into(),
            filter,
        })
        .await
        .expect_err("an inverted band is refused on the rank-order path too");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m)
            if m.contains("duration_min_ms") && m.contains("duration_max_ms")),
        "expected a validation error naming both ends of the length band, got {err:?}"
    );

    // And only that pairing changed: a band that is merely narrow still
    // answers the empty order above rather than an error, so the parse
    // running earlier has not turned the short-circuit into a refusal.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(1_000);
    filter.duration_max_ms = Some(2_000);
    let banded_blank = core
        .asset_service
        .search_ids(SearchAssetsQuery {
            text: "   ".into(),
            filter,
        })
        .await
        .expect("a well-formed band with no query is answered, not refused");
    assert!(banded_blank.ids.is_empty());
    assert_eq!(banded_blank.candidates_considered, 0);
    assert!(!banded_blank.truncated);
}

/// The playback-length and stored-size bands narrow a text search, and
/// the rows carrying neither column drop out exactly when a band names
/// them — not before, and not on the wrong axis.
///
/// Four hits, all matching the term, so every case below is the band
/// doing the narrowing rather than the retrieval:
///
/// | id             | length | size   |
/// |----------------|--------|--------|
/// | `long_take`    |  600 s | 8.0 MB |
/// | `short_take`   |    5 s | 0.5 MB |
/// | `still`        |      — | 3.0 MB |
/// | `unsized_take` |   60 s |      — |
///
/// The two absent states sit on **different rows and different
/// columns**, which is what makes a crossed pair of bands visible: a
/// size band keeps `still` (which has no length) and drops
/// `unsized_take` (which has one), so an implementation aiming the size
/// pair at `duration_ms` answers with the other two rows rather than
/// with something that merely looks short.
///
/// # What the NULL cases prove, and what they do not
///
/// They prove the observable contract: name either end of a band and
/// rows with no value in that column are absent from the answer.
/// They do **not** prove the shape of the SQL. `duration_ms >= ?` already
/// drops NULL rows under three-valued logic, so deleting the explicit
/// `IS NOT NULL` conjunct changes no answer here — that conjunct is
/// carried for what it states to a reader and for index reachability,
/// and its presence is pinned by a unit test over `QueryParts`
/// (`metric_band_states_its_exclusion_once_and_only_when_asked`), not
/// from out here.
#[tokio::test(flavor = "multi_thread")]
async fn metric_bands_narrow_the_text_search() {
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
                pack_id: Some("e2e-metric-bands".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Every body carries the query term; the trailing sentence differs
    // only so the four documents are not byte-identical.
    let plan = [
        (
            "long_take",
            "auroras over the ridge, filmed until the battery gave out\n",
            "video",
            Some(600_000u64),
            Some(8_000_000u64),
            1_785_000_000_000i64,
        ),
        (
            "short_take",
            "auroras again, a few seconds before the cloud closed in\n",
            "video",
            Some(5_000),
            Some(500_000),
            1_785_000_001_000,
        ),
        (
            "still",
            "auroras, a single frame from the tripod that night\n",
            "image",
            None,
            Some(3_000_000),
            1_785_000_002_000,
        ),
        (
            "unsized_take",
            "auroras on the drive home, transferred from a card long gone\n",
            "video",
            Some(60_000),
            None,
            1_785_000_003_000,
        ),
    ];
    let mut id = std::collections::HashMap::new();
    for (name, body, modality, duration_ms, file_size_bytes, occurred) in plan {
        let path = corpus.join(format!("{name}.md"));
        std::fs::write(&path, body).expect("write asset");
        let dto = core
            .asset_service
            .add(
                measured_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    modality,
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
    // Answers come back in BM25 rank order, which is not what these
    // cases are about — compare them as sets.
    let expected = |names: &[&str]| -> Vec<String> {
        let mut ids: Vec<String> = names.iter().map(|n| id[n].clone()).collect();
        ids.sort();
        ids
    };
    let hits = |filter: ListAssetsQuery| {
        let service = core.asset_service.clone();
        async move {
            let page = service
                .search(SearchAssetsQuery {
                    text: "auroras".into(),
                    filter,
                })
                .await
                .expect("search");
            let mut ids: Vec<String> = page.items.iter().map(|c| c.id.clone()).collect();
            ids.sort();
            ids
        }
    };

    // Indexing is asynchronous; poll like the cases above rather than
    // sleeping a fixed amount.
    let everything = expected(&["long_take", "short_take", "still", "unsized_take"]);
    let mut indexed = false;
    for _ in 0..120 {
        if hits(blank_filter()).await == everything {
            indexed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(indexed, "all four documents must index within 30s");

    // 1. A lower bound alone. Inclusive at the boundary (`unsized_take`
    //    is exactly 60 s), and the row with no length is gone even
    //    though nothing about it was named.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(60_000);
    assert_eq!(
        hits(filter).await,
        expected(&["long_take", "unsized_take"]),
        "a lower length bound is inclusive and excludes the row with no length"
    );

    // 2. An upper bound alone — the case worth stating separately,
    //    because "under a minute" is where folding absent into `0` would
    //    look most reasonable and would sweep every still image in.
    let mut filter = blank_filter();
    filter.duration_max_ms = Some(60_000);
    assert_eq!(
        hits(filter).await,
        expected(&["short_take", "unsized_take"]),
        "an upper length bound excludes the row with no length rather than \
         treating it as zero"
    );

    // 3. A closed band selects what sits inside it.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(10_000);
    filter.duration_max_ms = Some(100_000);
    assert_eq!(
        hits(filter).await,
        expected(&["unsized_take"]),
        "a closed length band selects what sits inside it"
    );

    // 4. The size band reads its own column: it keeps `still` — which
    //    has no length at all — and drops `unsized_take`, which does.
    //    A size pair aimed at `duration_ms` cannot produce this pair.
    let mut filter = blank_filter();
    filter.size_min_bytes = Some(3_000_000);
    assert_eq!(
        hits(filter).await,
        expected(&["long_take", "still"]),
        "a lower size bound is inclusive, reads the size column, and excludes \
         the row with no recorded size"
    );

    let mut filter = blank_filter();
    filter.size_max_bytes = Some(1_000_000);
    assert_eq!(
        hits(filter).await,
        expected(&["short_take"]),
        "an upper size bound excludes the row with no recorded size"
    );

    // 5. The two bands compose, and each alone answers a different pair
    //    from their intersection — so this cannot pass with one of them
    //    dropped.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(10_000);
    filter.size_min_bytes = Some(1_000_000);
    assert_eq!(
        hits(filter).await,
        expected(&["long_take"]),
        "the two bands intersect: length alone keeps unsized_take, size alone \
         keeps still, and only long_take satisfies both"
    );

    // 6. Naming no band is the only state in which the rows carrying
    //    neither column stay. The exclusion is a consequence of asking,
    //    not a standing rule — a still image is still an asset.
    assert_eq!(
        hits(blank_filter()).await,
        everything,
        "with no length or size band named, the rows carrying neither column stay"
    );
    // The same reading one axis over, as the contrast: none of these
    // four is rated, so a rating band — however wide — empties the page.
    // "Absent is outside every band" is the shared rule; the metric
    // bands are not doing something special.
    let mut filter = blank_filter();
    filter.rating_max = Some(5);
    assert!(
        hits(filter).await.is_empty(),
        "an unrated hit is outside every rating band, including a wide one"
    );

    // 7. An inverted band is a `400`, not an empty page — the empty page
    //    would read as "nothing in the library is that long".
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(2_000);
    filter.duration_max_ms = Some(1_000);
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "auroras".into(),
            filter,
        })
        .await
        .expect_err("an inverted length band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m)
            if m.contains("duration_min_ms") && m.contains("duration_max_ms")),
        "expected a validation error naming both ends of the length band, got {err:?}"
    );

    let mut filter = blank_filter();
    filter.size_min_bytes = Some(2_000);
    filter.size_max_bytes = Some(1_000);
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "auroras".into(),
            filter,
        })
        .await
        .expect_err("an inverted size band must be refused");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m)
            if m.contains("size_min_bytes") && m.contains("size_max_bytes")),
        "expected a validation error naming both ends of the size band, got {err:?}"
    );

    // 8. And the refusal does not depend on the request happening to
    //    match something. The `sort` and `trash` refusals are checked
    //    before `search`'s empty-text early return for that reason; the
    //    bands went through `to_asset_query`, which sat below it, so they
    //    were the case that had been missed. The parse now runs with the
    //    other two, and the same impossible band is a `400` with a blank
    //    query — one character of text is not what decides whether the
    //    caller is told the band cannot be satisfied.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(2_000);
    filter.duration_max_ms = Some(1_000);
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "   ".into(),
            filter,
        })
        .await
        .expect_err("whether a band is answerable cannot depend on the text beside it");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m)
            if m.contains("duration_min_ms") && m.contains("duration_max_ms")),
        "expected a validation error naming both ends of the length band, got {err:?}"
    );

    // The rating band was in the same position and got there first — it
    // predates the length and size pair entirely. The one move that
    // lifted the parse closes it there too, and this case says so rather
    // than leaving the older hole to be rediscovered on its own.
    let mut filter = blank_filter();
    filter.rating_min = Some(4);
    filter.rating_max = Some(2);
    let err = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "   ".into(),
            filter,
        })
        .await
        .expect_err("the rating band is refused on the same terms as the metric ones");
    assert!(
        matches!(err, asterism_core::DomainError::Validation(ref m) if m.contains("rating_min")),
        "expected a validation error naming the band, got {err:?}"
    );

    // 9. The other side of the move: only the *unanswerable* pairing
    //    changed. A blank query carrying a band that is merely narrow is
    //    still an empty page, because having nothing to search for is not
    //    a fault in the request. Running the parse earlier must not turn
    //    the short-circuit into a refusal.
    let mut filter = blank_filter();
    filter.duration_min_ms = Some(1_000);
    filter.duration_max_ms = Some(2_000);
    let blank = core
        .asset_service
        .search(SearchAssetsQuery {
            text: "   ".into(),
            filter,
        })
        .await
        .expect("a well-formed band with no text is answered, not refused");
    assert!(blank.items.is_empty(), "nothing was searched for");
    assert_eq!(blank.matched, 0);
    assert_eq!(
        blank.candidates_considered, 0,
        "no net was cast, so none may be reported"
    );
}
