//! End-to-end guard for the random draw (`AssetService::sample`) — the
//! read behind the sidebar's "🎲 Random".
//!
//! # What is asserted, and what deliberately is not
//!
//! The picks are **not reproducible**: the same call answers with
//! different ids in a different sequence every time (the
//! Retrieval side promises no determinism, and this is a
//! Retrieval-shaped read that happens to be written in SQL). So nothing
//! here asserts *which* ids come back or in what order. Pinning either
//! would be pinning the opposite of the contract, and would go red on a
//! shuffle that did its job.
//!
//! What is left is everything a caller may actually rely on:
//!
//!   1. the draw stops at `k`, while `set_total` counts the whole pool —
//!      the two are different numbers, and the fixture makes them
//!      differ so an implementation returning `items.len()` for both
//!      cannot pass;
//!   2. the filter narrows the pool, and an asset outside it never
//!      appears — checked over repeated draws, because a single draw
//!      could miss a leaking asset by luck;
//!   3. a `sort` axis is refused rather than dropped, for the reason
//!      the search path refuses it: the order *is* the shuffle.
//!
//! # Why an e2e and not a unit test
//!
//! The pool is built by the shared `QueryParts` predicate, hydrated
//! through `cards_by_ids`, and counted through the list path. A
//! repository test would see the SQL and not the service's assembly of
//! it; this drives the same graph the desktop grid does.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, AttachTagCommand, RegisterPersonaCommand};
use asterism_contract::query::{ListAssetsQuery, RandomAssetsQuery};
use asterism_contract::sort::{SortOrder, SortSpec, SortTarget};
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about the draw, not about
/// who ingested each row.
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
        // No band on any metric axis: naming one end would drop every row
        // carrying no length / no recorded size / no measured dimensions,
        // and the fixtures below are stills registered without any of
        // those columns — the pool these cases draw from would be empty
        // rather than narrowed.
        duration_min_ms: None,
        duration_max_ms: None,
        size_min_bytes: None,
        size_max_bytes: None,
        pixels_min: None,
        pixels_max: None,
        viewer_subject: None,
        trash: None,
        // The draw refuses an axis; the cases that expect an answer must
        // therefore leave this alone.
        sort: None,
        // Ignored by the random path — `k` is the only size knob. Left
        // at a value that disagrees with every `k` below so an
        // implementation reading `limit` instead would show.
        offset: 0,
        limit: 3,
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn random_draw_is_capped_filtered_and_refuses_an_axis() {
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
                pack_id: Some("e2e-random-sample".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Six assets, two of them tagged. The pool is deliberately larger
    // than every `k` used below, so "returned everything" and "returned
    // k" are distinguishable answers.
    let mut ids = Vec::new();
    for n in 0..6 {
        let file = corpus.join(format!("{n}.md"));
        std::fs::write(&file, format!("asset {n}\n")).expect("write asset");
        let dto = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    file.to_str().unwrap(),
                    1_785_000_000_000 + n as i64 * 1_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(dto.id);
    }
    let tagged: Vec<String> = ids[..2].to_vec();
    let mut tag_id = None;
    for id in &tagged {
        let tag = core
            .asset_service
            .attach_tag(
                AttachTagCommand {
                    asset_id: id.clone(),
                    name: "keepsake".into(),
                },
                &unattributed(),
            )
            .await
            .expect("attach tag");
        tag_id = Some(tag.id);
    }
    let tag_id = tag_id.expect("two attaches ran");

    // (a) The draw stops at `k` while the count describes the pool.
    // `k = 2` against six assets: an implementation that answered with
    // the whole set, or that reported `picked` as the set size, differs
    // from this in both numbers.
    let capped = core
        .asset_service
        .sample(RandomAssetsQuery {
            filter: blank_filter(),
            k: Some(2),
        })
        .await
        .expect("capped draw");
    assert_eq!(capped.picked, 2, "the draw stops at k");
    assert_eq!(
        capped.items.len(),
        2,
        "`picked` must describe the cards actually returned"
    );
    assert_eq!(
        capped.set_total, 6,
        "`set_total` counts the pool the picks were drawn from, not the \
         picks — the two disagree here on purpose"
    );
    let mut seen: Vec<&String> = capped.items.iter().map(|c| &c.id).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 2, "a draw must not repeat the same asset");
    for card in &capped.items {
        assert!(
            ids.contains(&card.id),
            "a pick that is not in the library came back: {}",
            card.id
        );
    }

    // A `k` wider than the pool takes the whole pool and says so, rather
    // than padding or failing.
    let wide = core
        .asset_service
        .sample(RandomAssetsQuery {
            filter: blank_filter(),
            k: Some(50),
        })
        .await
        .expect("wide draw");
    assert_eq!(wide.picked, 6, "fewer than k only when the set is smaller");
    assert_eq!(wide.set_total, 6);

    // (b) The filter narrows the pool, and nothing outside it leaks.
    // Repeated because a single draw could miss a leak by luck: with
    // `k` at the pool size, twenty clean draws over a six-asset library
    // is not something a broken predicate survives.
    let mut filter = blank_filter();
    filter.tag_ids = vec![tag_id.clone()];
    for round in 0..20 {
        let drawn = core
            .asset_service
            .sample(RandomAssetsQuery {
                filter: filter.clone(),
                k: Some(6),
            })
            .await
            .expect("filtered draw");
        assert_eq!(
            drawn.set_total, 2,
            "round {round}: the chip must narrow the count as well as the picks"
        );
        assert_eq!(
            drawn.picked, 2,
            "round {round}: both tagged assets fit in k"
        );
        for card in &drawn.items {
            assert!(
                tagged.contains(&card.id),
                "round {round}: an untagged asset came back under a tag chip: {}",
                card.id
            );
        }
    }

    // (c) An axis is refused, not dropped. The order is the shuffle, so
    // answering a sorted request with a shuffled list would let the
    // caller believe a sort happened.
    let mut filter = blank_filter();
    filter.sort = Some(SortSpec {
        target: SortTarget::OccurredAt,
        order: SortOrder::Updated,
        reverse: false,
        collation: None,
    });
    let refused = core
        .asset_service
        .sample(RandomAssetsQuery { filter, k: Some(2) })
        .await;
    match refused {
        Err(asterism_core::DomainError::Validation(message)) => {
            assert!(
                message.contains("sort is not supported"),
                "the refusal must say what was refused: {message}"
            );
        }
        other => panic!("a sort axis must be a validation error, got: {other:?}"),
    }
}
