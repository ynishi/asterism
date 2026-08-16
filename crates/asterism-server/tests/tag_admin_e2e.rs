//! End-to-end guard for the tag *administration* routes — rename,
//! delete, merge.
//!
//! The per-asset verbs (`attach` / `detach`) could only ever grow the
//! channel vocabulary: an automatic tagger running over a large library
//! mints synonyms and spelling variants, and until these three routes
//! existed there was no way back. What each one has to get right is not
//! the SQL alone but the boundary decisions around it, which is why the
//! assertions here drive the real router:
//!
//! 1. **rename does not merge.** A name already held by another tag is
//!    a `409`, not a silent fold — otherwise fixing a typo would delete
//!    a channel without saying so.
//! 2. **merge de-duplicates.** An asset carrying both ends must come out
//!    with one link, not a constraint violation and not two.
//! 3. **`dry_run` writes nothing.** Merge has no undo, so the preview
//!    has to be a genuine rollback rather than a differently-shaped
//!    write; the numbers it reports must match what the real call then
//!    does.
//!
//! Every count assertion uses a fixture where the axis under test
//! disagrees with the default: the merge fixture has both overlapping
//! and non-overlapping assets, so `affected_assets` and `already_tagged`
//! are distinct non-zero numbers that a stubbed implementation cannot
//! hit by accident.
//!
//! **Why `CoreMode::ReadOnly`.** Every assertion here reads a tag list
//! or a tag count exactly, and `auto_tag` mines the asset's file stem
//! for keywords and links a tag per token — so under a live worker the
//! fixture's own name (`tag-delete-0` → `tag`, `delete`) lands on the
//! asset it seeded and joins the list being compared. Whether that
//! commits before or after the read is a race, and renaming the packs
//! is not the way out of it: the miner keeps every token of two
//! characters or more that holds a letter, so a name that mines
//! nothing today sits one heuristic change away from mining something,
//! and the exactness of these assertions is what would break. Nothing
//! under test needs the worker — these routes fold inside their own
//! transactions and no assertion reads a cover, an index or a keyword
//! — so the queue is opened and left undrained. The two other ways out
//! were a quiescence wait in the harness, a facility this suite has no
//! use for, and weakening the assertions to "the deleted tag is
//! absent", which would stop them catching what they exist for.

use std::sync::Arc;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about tag administration,
/// not about who ingested the row.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Spins up a core over a tempdir and returns it with the router built
/// on top. `init_core_with` keeps the Tantivy index inside the tempdir
/// rather than the developer's active profile, and `ReadOnly` opens the
/// job queue without a worker — see the module note for what a live one
/// does to these fixtures.
async fn harness(tmp: &std::path::Path) -> (CoreCtx, Router) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    (core, router)
}

async fn call(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "body is not JSON ({e}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, json)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build GET")
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build POST")
}

fn add_command(persona_id: &str, locator: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some("image".into()),
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

/// Registers a persona and `count` assets, returning `(persona_id, asset_ids)`.
async fn seed(
    core: &CoreCtx,
    tmp: &std::path::Path,
    pack: &str,
    count: usize,
) -> (String, Vec<String>) {
    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "Tags".into(),
                pack_id: Some(pack.to_string()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");
    let mut ids = Vec::with_capacity(count);
    for n in 0..count {
        let asset = core
            .asset_service
            .add(
                add_command(
                    &persona.id,
                    tmp.join(format!("{pack}-{n}.png")).to_str().unwrap(),
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
        ids.push(asset.id);
    }
    (persona.id, ids)
}

/// Attaches `name` to `asset` over HTTP and returns the tag id.
async fn attach(router: &Router, asset: &str, name: &str) -> String {
    let (status, body) = call(
        router,
        post(
            "/asterism/tags/attach",
            serde_json::json!({ "asset_id": asset, "name": name }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "attach {name}: {body}");
    body["id"].as_str().expect("tag id").to_string()
}

/// `name -> asset_count` from the sidebar aggregate.
async fn counts(router: &Router) -> std::collections::BTreeMap<String, u64> {
    let (status, body) = call(router, get("/asterism/tags/counts")).await;
    assert_eq!(status, StatusCode::OK, "counts: {body}");
    body.as_array()
        .expect("array")
        .iter()
        .map(|row| {
            (
                row["tag"]["name"].as_str().expect("name").to_string(),
                row["asset_count"].as_u64().expect("count"),
            )
        })
        .collect()
}

/// Rename in place: the id survives, the name changes, and the value
/// written is the *normalised* one.
///
/// The normalisation assertion is the point of the padded input: attach
/// and rename have to agree about what a name is, or `" final"` and
/// `"final"` become two channels that look identical in the sidebar.
/// The asset is attached through a padded name too, so both directions
/// of the shared rule are pinned by one fixture.
#[tokio::test(flavor = "multi_thread")]
async fn a_tag_renames_in_place_under_the_attach_path_normalisation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (_persona, assets) = seed(&core, tmp.path(), "tag-rename", 1).await;

    // Padded on attach: the stored name is already trimmed.
    let tag_id = attach(&router, &assets[0], "  draft  ").await;
    assert_eq!(counts(&router).await.get("draft"), Some(&1), "attach trims");

    let (status, renamed) = call(
        &router,
        post(
            "/asterism/tags/rename",
            serde_json::json!({ "tag_id": tag_id, "name": "  final  " }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rename: {renamed}");
    assert_eq!(
        renamed["name"], "final",
        "rename applies the same trim the attach path does"
    );
    assert_eq!(
        renamed["id"], tag_id,
        "rename is in place — a new row would orphan every existing link"
    );

    let after = counts(&router).await;
    assert_eq!(after.get("final"), Some(&1), "the link survived the rename");
    assert!(
        !after.contains_key("draft"),
        "the old name is gone: {after:?}"
    );

    // The asset still carries exactly one tag, now under the new name.
    let (_, detail) = call(&router, get(&format!("/asterism/assets/{}", assets[0]))).await;
    let names: Vec<&str> = detail["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["final"]);

    // Renaming to the name it already carries is a no-op success, not a
    // self-inflicted conflict.
    let (status, same) = call(
        &router,
        post(
            "/asterism/tags/rename",
            serde_json::json!({ "tag_id": tag_id, "name": "final" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "no-op rename: {same}");
    assert_eq!(same["name"], "final");

    // Whitespace-only is rejected before it can materialise a nameless
    // channel — same rule as attach.
    let (status, _) = call(
        &router,
        post(
            "/asterism/tags/rename",
            serde_json::json!({ "tag_id": tag_id, "name": "   " }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "an empty name is refused");

    // An unknown tag is a 404, not a silently created row.
    let (status, _) = call(
        &router,
        post(
            "/asterism/tags/rename",
            serde_json::json!({
                "tag_id": "019f0000-0000-7000-8000-000000000000",
                "name": "ghost",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A rename onto an occupied name is refused, and refused *without*
/// touching either tag.
///
/// The alternative — folding the two together — is what `merge` is for.
/// A rename that silently merged would delete a channel as a side
/// effect of fixing a typo, so the response also has to point at the
/// route that does it on purpose.
#[tokio::test(flavor = "multi_thread")]
async fn renaming_onto_an_occupied_name_is_a_conflict_that_names_merge() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (_persona, assets) = seed(&core, tmp.path(), "tag-conflict", 2).await;

    let sketch = attach(&router, &assets[0], "sketch").await;
    attach(&router, &assets[1], "sketches").await;

    let (status, body) = call(
        &router,
        post(
            "/asterism/tags/rename",
            serde_json::json!({ "tag_id": sketch, "name": "sketches" }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "rename must not merge: {body}"
    );
    let message = body["message"].as_str().expect("message");
    assert!(
        message.contains("merge"),
        "the refusal has to name the way through: {message}"
    );

    // Both channels stand, both keep their asset.
    let after = counts(&router).await;
    assert_eq!(after.get("sketch"), Some(&1));
    assert_eq!(after.get("sketches"), Some(&1));
}

/// Delete drops the channel and every link to it in one call.
///
/// The links are what makes this different from `detach`: an asset that
/// carried the tag must come back with it gone, and the tag must leave
/// the sidebar aggregate entirely rather than linger at zero.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_tag_removes_the_channel_and_every_link() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (_persona, assets) = seed(&core, tmp.path(), "tag-delete", 2).await;

    let doomed = attach(&router, &assets[0], "doomed").await;
    attach(&router, &assets[1], "doomed").await;
    // A bystander channel on the same asset: delete must be surgical.
    attach(&router, &assets[0], "kept").await;

    let (status, body) = call(
        &router,
        post(
            "/asterism/tags/delete",
            serde_json::json!({ "tag_id": doomed }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete: {body}");
    assert_eq!(body["deleted"], true);
    assert_eq!(
        body["detached_assets"], 2,
        "both links are reported, not just the tag row"
    );

    let after = counts(&router).await;
    assert!(
        !after.contains_key("doomed"),
        "the channel left the sidebar: {after:?}"
    );
    assert_eq!(after.get("kept"), Some(&1), "the bystander survived");

    let (_, detail) = call(&router, get(&format!("/asterism/assets/{}", assets[0]))).await;
    let names: Vec<&str> = detail["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["kept"], "the deleted tag left the asset");

    // Gone means gone: the second delete is a 404, not a silent success.
    // A caller retrying a timed-out delete needs to be able to tell the
    // two apart, because this verb is not idempotent by construction —
    // there is no trash for tags.
    let (status, _) = call(
        &router,
        post(
            "/asterism/tags/delete",
            serde_json::json!({ "tag_id": doomed }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The two reads above both join through `tag`, so they cannot tell
    // "the links went in the same transaction" from "the tag row went
    // and the links are orphaned" (they pass either way once the tag
    // row is gone). Ask `asset_tag` directly, over a second handle on
    // the same database file.
    let doomed_uuid = uuid::Uuid::parse_str(&doomed).expect("tag id is a uuid");
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(&tmp.path().join("asterism.db"))
        .await
        .expect("second isle");
    let orphans: i64 = isle
        .call(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM asset_tag WHERE tag_id = ?1",
                rusqlite::params![doomed_uuid],
                |row| row.get(0),
            )
        })
        .await
        .expect("count orphan links");
    assert_eq!(orphans, 0, "no orphan asset_tag row survives the delete");
    driver.shutdown().await.ok();
}

/// Merge: the counts, the de-duplication, the dissolved source — and
/// the dry run that predicts all three without writing.
///
/// The fixture deliberately mixes the two cases. `alpha` sits on three
/// assets, one of which already carries `beta`, so a correct merge
/// reports `affected_assets = 2` and `already_tagged = 1` — two
/// different non-zero numbers. An implementation that moved every link
/// blindly would report 3/0 and would also hit the
/// `PRIMARY KEY (asset_id, tag_id)` constraint on the overlapping row.
#[tokio::test(flavor = "multi_thread")]
async fn merging_moves_links_deduplicates_the_overlap_and_dissolves_the_source() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (_persona, assets) = seed(&core, tmp.path(), "tag-merge", 4).await;

    let alpha = attach(&router, &assets[0], "alpha").await;
    attach(&router, &assets[1], "alpha").await;
    attach(&router, &assets[2], "alpha").await;
    let beta = attach(&router, &assets[2], "beta").await; // the overlap
    attach(&router, &assets[3], "beta").await;

    let before = counts(&router).await;
    assert_eq!(before.get("alpha"), Some(&3));
    assert_eq!(before.get("beta"), Some(&2));

    // --- dry run: same numbers, nothing written ---------------------
    let (status, preview) = call(
        &router,
        post(
            "/asterism/tags/merge",
            serde_json::json!({
                "source_tag_id": alpha,
                "target_tag_id": beta,
                "dry_run": true,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "dry run: {preview}");
    assert_eq!(preview["affected_assets"], 2);
    assert_eq!(preview["already_tagged"], 1);
    assert_eq!(
        preview["source_removed"], false,
        "a dry run must not claim the source is gone"
    );
    assert_eq!(
        counts(&router).await,
        before,
        "a dry run that changed the counts is a merge with extra steps"
    );

    // --- the real merge ---------------------------------------------
    let (status, merged) = call(
        &router,
        post(
            "/asterism/tags/merge",
            serde_json::json!({ "source_tag_id": alpha, "target_tag_id": beta }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "merge: {merged}");
    assert_eq!(
        (
            merged["affected_assets"].clone(),
            merged["already_tagged"].clone()
        ),
        (
            preview["affected_assets"].clone(),
            preview["already_tagged"].clone()
        ),
        "the preview is only worth reading if it matches what happens"
    );
    assert_eq!(merged["source_removed"], true);

    let after = counts(&router).await;
    assert!(
        !after.contains_key("alpha"),
        "the source dissolved: {after:?}"
    );
    assert_eq!(
        after.get("beta"),
        Some(&4),
        "every distinct asset ends up on the target exactly once"
    );

    // The overlapping asset carries one `beta`, not two — the sidebar
    // count above aggregates DISTINCT assets and would hide a doubled
    // link, so the per-asset read is the one that can see it.
    let (_, detail) = call(&router, get(&format!("/asterism/assets/{}", assets[2]))).await;
    let names: Vec<&str> = detail["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["beta"], "the overlap kept a single link");

    // A moved asset now reads as the target.
    let (_, moved) = call(&router, get(&format!("/asterism/assets/{}", assets[0]))).await;
    let names: Vec<&str> = moved["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["beta"]);
}

/// Merge's two refusals: a tag into itself, and an unknown end.
///
/// Self-merge is a `400` rather than a no-op success because the only
/// way to ask for it is a caller bug — answering "0 assets affected,
/// source removed" would be a lie in the second field.
#[tokio::test(flavor = "multi_thread")]
async fn merge_refuses_a_self_merge_and_an_unknown_end() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, router) = harness(tmp.path()).await;
    let (_persona, assets) = seed(&core, tmp.path(), "tag-merge-refuse", 1).await;
    let solo = attach(&router, &assets[0], "solo").await;
    const ABSENT: &str = "019f0000-0000-7000-8000-000000000000";

    let (status, body) = call(
        &router,
        post(
            "/asterism/tags/merge",
            serde_json::json!({ "source_tag_id": solo, "target_tag_id": solo }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "self-merge: {body}");

    for (source, target) in [(solo.as_str(), ABSENT), (ABSENT, solo.as_str())] {
        let (status, body) = call(
            &router,
            post(
                "/asterism/tags/merge",
                serde_json::json!({ "source_tag_id": source, "target_tag_id": target }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "missing end ({source} -> {target}): {body}"
        );
    }

    // Nothing moved, nothing dissolved.
    assert_eq!(counts(&router).await.get("solo"), Some(&1));
}
