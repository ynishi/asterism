//! AlbumMeta end to end — what somebody says about an asset, and where
//! it has to survive.
//!
//! The interesting assertions here are all about *coexistence*. A
//! statement lands in `_trace`, which four other kinds of statement
//! already share, and the failure mode this design is exposed to is not
//! "the value is wrong" but "writing one thing quietly took another
//! with it". That already happened once on this bag: the provenance
//! writer replaced the whole object and carried off the declared hash
//! [measured 2026-08-06, fixed in the commit before this one]. So the
//! neighbours are asserted, not just the entry.
//!
//! The other half is what AlbumMeta deliberately does *not* do: no
//! edge, no resolution, no effect on identity. An external identifier
//! recorded here is a sentence, not a key — that is the whole reason it
//! is recorded here rather than becoming one.
//!
//! The last section is the filter, which is where that distinction gets
//! tested rather than asserted: the value *does* lead back to the row,
//! and it does so as a secondary index (`asset_album_meta`, kept level
//! with the bag by triggers) rather than as an identity. The cases pin
//! both halves — that the lookup works, and that nothing about it
//! promises the answer is one row.

use std::sync::Arc;

use asterism_contract::command::{
    AddAssetCommand, DeclareAssetMetaCommand, DeclareProvenanceCommand, RegisterPersonaCommand,
};
use asterism_contract::query::ListAssetsQuery;
use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};

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

fn meta(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

struct Fixture {
    core: asterism_server::core_init::CoreCtx,
    persona: String,
    asset: String,
    other: String,
    corpus: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

impl Fixture {
    /// Writes a fresh file under the corpus and returns an ingest
    /// command pointed at it.
    fn ingest(&self, name: &str) -> AddAssetCommand {
        let path = self.corpus.join(name);
        std::fs::write(&path, format!("# {name}\n")).expect("write file");
        add_command(&self.persona, path.to_str().unwrap(), 1_785_000_600_000)
    }
}

async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");

    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-album-meta".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    let mut ids = Vec::new();
    for (index, name) in ["subject", "other"].iter().enumerate() {
        let path = corpus.join(format!("{name}.md"));
        std::fs::write(&path, format!("# {name}\n")).expect("write file");
        let asset = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().unwrap(),
                    1_785_000_000_000 + index as i64 * 60_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(asset.id);
    }

    Fixture {
        core,
        persona: persona.id.clone(),
        asset: ids[0].clone(),
        other: ids[1].clone(),
        corpus,
        _tmp: tmp,
    }
}

fn declare(asset_id: &str, key: &str, value: Option<&str>) -> DeclareAssetMetaCommand {
    DeclareAssetMetaCommand {
        asset_id: asset_id.to_string(),
        key: key.to_string(),
        value: value.map(str::to_string),
        operator_ai: None,
    }
}

fn bag(asset: &asterism_contract::dto::AssetDto) -> serde_json::Value {
    serde_json::from_str(asset.extra_json.as_deref().unwrap_or("{}")).expect("extra is json")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_statement_is_filed_under_the_name_it_was_given() {
    let fx = fixture().await;
    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(
            DeclareAssetMetaCommand {
                asset_id: fx.asset.clone(),
                key: "workflow-id".into(),
                value: Some("wf-2026-08-06-a".into()),
                operator_ai: Some("claude-code".into()),
            },
            &unattributed(),
        )
        .await
        .expect("declare meta");

    let extra = bag(&asset);
    let entry = &extra["_trace"]["meta"]["workflow-id"];
    assert_eq!(entry["value"], serde_json::json!("wf-2026-08-06-a"));
    // The channel, from the provenance vocabulary: this verb is the
    // after-the-fact one, so every statement through it is `manual`.
    assert_eq!(entry["source"], serde_json::json!("manual"));
    assert_eq!(entry["operator"], serde_json::json!("claude-code"));
    assert!(entry["declared_at_ms"].is_i64());
    // And nothing was inferred from it. A recorded identifier is a
    // sentence; making it mean something is a different layer.
    assert!(entry.get("verified").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn statements_under_different_names_coexist() {
    let fx = fixture().await;
    for (key, value) in [("workflow-id", "wf-1"), ("plate", "offwhite")] {
        fx.core
            .asset_service
            .declare_asset_meta(declare(&fx.asset, key, Some(value)), &unattributed())
            .await
            .expect("declare meta");
    }
    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "note", Some("third")), &unattributed())
        .await
        .expect("declare meta");

    let extra = bag(&asset);
    let meta = extra["_trace"]["meta"].as_object().expect("meta object");
    // The whole set: a writer that replaced the object instead of
    // inserting into it would pass an assertion about the last key
    // alone. That is the exact shape of the defect this bag already had.
    let mut keys: Vec<&String> = meta.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["note", "plate", "workflow-id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_later_statement_under_one_name_wins() {
    let fx = fixture().await;
    fx.core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "plate", Some("first")), &unattributed())
        .await
        .expect("declare meta");
    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "plate", Some("second")), &unattributed())
        .await
        .expect("re-declare meta");

    // Single slot, unlike a provenance claim: two statements under one
    // name are a correction and its subject, and keeping both would
    // leave a reader guessing which is current.
    assert_eq!(
        bag(&asset)["_trace"]["meta"]["plate"]["value"],
        serde_json::json!("second")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_last_statement_takes_the_container_with_it() {
    let fx = fixture().await;
    fx.core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "plate", Some("first")), &unattributed())
        .await
        .expect("declare meta");
    fx.core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "other", Some("kept")), &unattributed())
        .await
        .expect("declare a second");

    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "plate", None), &unattributed())
        .await
        .expect("remove one");
    let extra = bag(&asset);
    assert!(extra["_trace"]["meta"].get("plate").is_none());
    // A removal takes one key, not the neighbour.
    assert_eq!(
        extra["_trace"]["meta"]["other"]["value"],
        serde_json::json!("kept")
    );

    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "other", None), &unattributed())
        .await
        .expect("remove the last");
    // Not `{}`: an empty container is a second thing to check for the
    // same "nobody has said anything" the absent key already says.
    assert!(bag(&asset)["_trace"].get("meta").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_statement_and_a_provenance_claim_do_not_evict_each_other() {
    let fx = fixture().await;
    fx.core
        .asset_service
        .declare_asset_meta(
            declare(&fx.asset, "plate", Some("offwhite")),
            &unattributed(),
        )
        .await
        .expect("declare meta");
    let asset = fx
        .core
        .asset_service
        .declare_provenance(
            DeclareProvenanceCommand {
                asset_id: fx.asset.clone(),
                derived_from: format!("asset:{}", fx.other),
                relation: Some("reference".into()),
                operator_ai: None,
            },
            &unattributed(),
        )
        .await
        .expect("declare provenance");

    let extra = bag(&asset);
    // Both directions of the shared bag, in one row.
    assert_eq!(
        extra["_trace"]["meta"]["plate"]["value"],
        serde_json::json!("offwhite")
    );
    assert_eq!(extra["_trace"]["relation"], serde_json::json!("reference"));

    // …and now the other order.
    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "plate2", Some("gray")), &unattributed())
        .await
        .expect("declare a second statement after the claim");
    let extra = bag(&asset);
    assert_eq!(extra["_trace"]["relation"], serde_json::json!("reference"));
    assert_eq!(
        extra["_trace"]["meta"]["plate2"]["value"],
        serde_json::json!("gray")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_statement_draws_no_edge_and_moves_no_identity() {
    let fx = fixture().await;
    // The case the design was argued against: something that looks like
    // an identifier arrives and is recorded. It must stay a sentence.
    let identifier = format!("xmp.did:{}", fx.other);
    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(
            declare(&fx.asset, "xmp-did", Some(&identifier)),
            &unattributed(),
        )
        .await
        .expect("declare an identifier-shaped statement");

    assert_eq!(asset.id, fx.asset, "the asset id is untouched");
    let edges = fx
        .core
        .asset_service
        .edges_of(&fx.asset, None, 20)
        .await
        .expect("edges");
    assert!(
        edges.is_empty(),
        "recording a statement is not declaring a relation: {edges:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_request_costs_no_write() {
    let fx = fixture().await;
    for (key, value) in [("a.b", Some("v")), ("plate", Some("  "))] {
        let err = fx
            .core
            .asset_service
            .declare_asset_meta(declare(&fx.asset, key, value), &unattributed())
            .await
            .expect_err("refused");
        assert!(!err.to_string().is_empty());
    }
    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(declare(&fx.asset, "ok", Some("v")), &unattributed())
        .await
        .expect("a well-formed one still lands");
    let extra = bag(&asset);
    let meta = extra["_trace"]["meta"].as_object().expect("meta");
    // Only the accepted key is there: a refusal that had written first
    // would leave `plate` behind with an empty value.
    assert_eq!(meta.keys().collect::<Vec<_>>(), vec!["ok"]);
}

// ---------------------------------------------------------------
// The registration path — the same statements, arriving with the
// payload instead of after the fact.
// ---------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_statement_made_at_registration_lands_with_the_row() {
    let fx = fixture().await;
    let mut command = fx.ingest("registered.md");
    command.album_meta = meta(&[("workflow-id", "wf-2026-08-06-b")]);
    let asset = fx
        .core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("add with a statement");

    let entry = &bag(&asset)["_trace"]["meta"]["workflow-id"];
    assert_eq!(entry["value"], serde_json::json!("wf-2026-08-06-b"));
    // `pushed`, not `manual`: this arrived with the payload. The
    // distinction is the whole reason the entry records a channel — a
    // value the caller handed over and a value somebody typed later are
    // different kinds of evidence about the same name.
    assert_eq!(entry["source"], serde_json::json!("pushed"));
    // No operator on the entry. The row has its own `operator_ai`
    // column and the ingest *is* the operation this describes; naming
    // it twice would let the two disagree.
    assert!(entry.get("operator").is_none(), "{entry}");
    assert!(entry["declared_at_ms"].is_i64());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_registration_statement_survives_everything_written_beside_it() {
    let fx = fixture().await;
    let mut command = fx.ingest("crowded.md");
    command.album_meta = meta(&[("plate", "offwhite"), ("catalogue", "c-12")]);
    command.derived_from = Some(format!("asset:{}", fx.other));
    command.declared_content_hash = Some(format!("sha256:{}", "a".repeat(64)));
    command.extra_json = Some(r#"{"camera":"X100"}"#.into());
    let asset = fx
        .core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("add with every writer at once");

    // Four writers touch this row on one call — `extra_json` replaces
    // the bag, then the claim, the declared hash and the statements go
    // in after it. Each one has taken a neighbour off `_trace` at some
    // point in this design's history, so all four are asserted.
    let extra = bag(&asset);
    assert_eq!(extra["camera"], serde_json::json!("X100"));
    assert_eq!(
        extra["_trace"]["claim"],
        serde_json::json!(format!("asset:{}", fx.other))
    );
    assert!(
        extra["_trace"]["declared_hash"]["value"].is_string(),
        "{extra}"
    );
    assert_eq!(
        extra["_trace"]["meta"]["plate"]["value"],
        serde_json::json!("offwhite")
    );
    assert_eq!(
        extra["_trace"]["meta"]["catalogue"]["value"],
        serde_json::json!("c-12")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_statement_takes_the_whole_ingest_with_it() {
    let fx = fixture().await;
    let mut command = fx.ingest("refused.md");
    let locator = command.locator.clone();
    command.album_meta = meta(&[("fine", "v"), ("a.b", "v")]);
    let err = fx
        .core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect_err("a malformed key is refused");
    assert!(!err.to_string().is_empty());

    // And nothing landed. Partial acceptance would leave a row that
    // looks imported and answers to none of the names it was registered
    // under — so the retry has to find the locator free.
    let mut retry = fx.ingest("refused.md");
    retry.locator.clone_from(&locator);
    retry.album_meta = meta(&[("fine", "v")]);
    let asset = fx
        .core
        .asset_service
        .add(retry, &unattributed())
        .await
        .expect("the corrected request lands on the same locator");
    assert_eq!(
        bag(&asset)["_trace"]["meta"]["fine"]["value"],
        serde_json::json!("v")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_mints_no_session_for_an_ingest_that_never_lands() {
    let fx = fixture().await;
    let mut command = fx.ingest("no-session.md");
    command.external_session_key = Some("e2e.album-meta.refused".into());
    command.album_meta = meta(&[("a.b", "v")]);
    fx.core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect_err("a malformed key is refused");

    // Resolving an external session key *writes* — it mints the
    // composite the member would have joined. So a command this service
    // cannot accept has to be refused above that call, or the container
    // outlives the ingest it was created for and sits in the sidebar
    // holding nothing.
    let sessions = fx
        .core
        .asset_service
        .list_sessions(ListAssetsQuery::default())
        .await
        .expect("list sessions");
    assert!(
        sessions.items.is_empty(),
        "a refused ingest left a Session behind: {:?}",
        sessions.items
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_declaration_verb_corrects_a_statement_made_at_registration() {
    let fx = fixture().await;
    let mut command = fx.ingest("corrected.md");
    command.album_meta = meta(&[("plate", "guessed")]);
    let asset = fx
        .core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("add with a statement");

    let asset = fx
        .core
        .asset_service
        .declare_asset_meta(
            declare(&asset.id, "plate", Some("measured")),
            &unattributed(),
        )
        .await
        .expect("correct it after the fact");

    // One slot, reached from both paths: the two write the same name
    // rather than each keeping its own copy for a reader to choose
    // between. The channel moves with the current statement.
    let entry = &bag(&asset)["_trace"]["meta"]["plate"];
    assert_eq!(entry["value"], serde_json::json!("measured"));
    assert_eq!(entry["source"], serde_json::json!("manual"));
}

// ---------------------------------------------------------------
// The filter — a recorded statement leads back to its row, as an index
// over identity rather than as one.
// ---------------------------------------------------------------

/// Ingests `name` carrying the given statements and returns its id.
async fn stated(fx: &Fixture, name: &str, pairs: &[(&str, &str)]) -> String {
    let mut command = fx.ingest(name);
    command.album_meta = meta(pairs);
    fx.core
        .asset_service
        .add(command, &unattributed())
        .await
        .expect("add with statements")
        .id
}

/// Runs a filter and returns the ids it matched, in page order.
async fn found(fx: &Fixture, key: Option<&str>, value: Option<&str>) -> Vec<String> {
    fx.core
        .asset_service
        .list(ListAssetsQuery {
            album_meta_key: key.map(str::to_string),
            album_meta_value: value.map(str::to_string),
            ..Default::default()
        })
        .await
        .expect("list")
        .items
        .into_iter()
        .map(|card| card.id)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recorded_statement_leads_back_to_its_row() {
    let fx = fixture().await;
    let target = stated(&fx, "found.md", &[("workflow-id", "wf-a")]).await;
    stated(&fx, "sibling.md", &[("workflow-id", "wf-b")]).await;
    stated(&fx, "unstated.md", &[]).await;

    assert_eq!(
        found(&fx, Some("workflow-id"), Some("wf-a")).await,
        vec![target.clone()]
    );
    // Naming the value alone finds it under whatever name it was filed
    // under — the case somebody pasting an identifier is actually in.
    assert_eq!(found(&fx, None, Some("wf-a")).await, vec![target.clone()]);
    // Naming the key alone asks which rows have anything to say under
    // that name, which is a different question and a wider answer.
    assert_eq!(found(&fx, Some("workflow-id"), None).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_and_a_value_have_to_meet_on_one_statement() {
    let fx = fixture().await;
    let real = stated(&fx, "real.md", &[("workflow-id", "wf-a")]).await;
    // The decoy carries both tokens and neither together: the name on
    // one statement, the value on another. Two independent clauses would
    // return it, which is the loosening this asserts against.
    let decoy = stated(
        &fx,
        "decoy.md",
        &[("workflow-id", "wf-z"), ("plate", "wf-a")],
    )
    .await;

    assert_eq!(
        found(&fx, Some("workflow-id"), Some("wf-a")).await,
        vec![real]
    );
    // …and the decoy really does carry both tokens, or the line above
    // is vacuous.
    assert!(found(&fx, Some("workflow-id"), None).await.contains(&decoy));
    assert!(found(&fx, None, Some("wf-a")).await.contains(&decoy));
}

#[tokio::test(flavor = "multi_thread")]
async fn one_value_can_lead_to_more_than_one_row() {
    let fx = fixture().await;
    // The property that separates a filter from an identity. Two rows
    // may carry the same recorded identifier — a re-export, a file
    // copied and re-registered — and the answer is both of them, not a
    // conflict and not a fold.
    let first = stated(&fx, "one.md", &[("workflow-id", "shared")]).await;
    let second = stated(&fx, "two.md", &[("workflow-id", "shared")]).await;

    let mut hits = found(&fx, Some("workflow-id"), Some("shared")).await;
    hits.sort();
    let mut both = vec![first, second];
    both.sort();
    assert_eq!(hits, both);
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_the_statement_back_takes_the_way_to_the_row_with_it() {
    let fx = fixture().await;
    let id = stated(&fx, "retracted.md", &[("workflow-id", "wf-a")]).await;
    assert_eq!(found(&fx, None, Some("wf-a")).await.len(), 1);

    fx.core
        .asset_service
        .declare_asset_meta(declare(&id, "workflow-id", None), &unattributed())
        .await
        .expect("remove the statement");

    // The index is a projection of the bag, so a retraction has to reach
    // it — a stale row here would answer with an asset that no longer
    // says anything of the kind.
    assert!(found(&fx, None, Some("wf-a")).await.is_empty());

    // A correction moves it rather than leaving both reachable.
    fx.core
        .asset_service
        .declare_asset_meta(
            declare(&id, "workflow-id", Some("wf-corrected")),
            &unattributed(),
        )
        .await
        .expect("state it again");
    assert!(found(&fx, None, Some("wf-a")).await.is_empty());
    assert_eq!(found(&fx, None, Some("wf-corrected")).await, vec![id]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_no_statement_could_answer_is_refused_rather_than_answered_empty() {
    let fx = fixture().await;
    stated(&fx, "any.md", &[("workflow-id", "wf-a")]).await;

    for (key, value) in [
        (Some("Workflow"), None),
        (Some("a.b"), None),
        (None, Some(" ")),
    ] {
        let err = fx
            .core
            .asset_service
            .list(ListAssetsQuery {
                album_meta_key: key.map(str::to_string),
                album_meta_value: value.map(str::to_string),
                ..Default::default()
            })
            .await
            .expect_err("a shape no statement could carry is a fact about the request");
        assert!(!err.to_string().is_empty());
    }
}
