//! Registering a series Strategy from outside the process, and what an
//! edit to one costs.
//!
//! The axis has derived keys since S3 and had no door: the rule seeded by
//! V73 was the only one any library could hold. This binary drives the
//! door — `POST` / `PATCH` / `DELETE` on `/asterism/series-strategies` —
//! through the real [`asterism_server::http::router`], and then reads
//! `material_series` to see whether the library actually moved.
//!
//! **Nothing writes a fingerprint or a derived row by hand.** The
//! fixtures are PNG files with `tEXt` chunks; `add` enqueues the hash
//! job, the worker walks the chunks into `meta_kv`, and the derivation
//! pass applies every registered rule. Writing rows directly would assert
//! the repository and nothing about whether registering a rule over HTTP
//! ever reaches a key — which is the whole claim of this slice.
//!
//! The read side goes over a second isle on the same database file:
//! `material_series` has no wire surface, deliberately (the grouping
//! query's shape follows from a reader that does not exist yet), and the
//! same arrangement `tag_admin_e2e` and `png_text_is_meta_e2e` document.
//!
//! `Full` mode is what makes any of it real — it takes the writer lock
//! and spawns the job worker, so an enqueued walk runs.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, RegisterPersonaCommand};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The rule V73 seeds, at the id the migration froze. It selects the
/// `vdsl` chunk's `script`, so every fixture below carries one — it is
/// the *other* rule every assertion about "this rule's rows" is measured
/// against.
const SEEDED_VDSL_RULE: &str = "019fe8f8-1400-7000-8000-000000000001";

/// One `material_series` row as this file reads it: which material, the
/// key (absent for the two answers that are not keys), and when it was
/// filed.
type DerivedRow = (uuid::Uuid, Option<String>, i64);

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about rules and keys, not
/// about who registered them.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

// ---- the wire ------------------------------------------------------

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
    // A rejection from the `Json` extractor answers in plain text, so a
    // body that will not parse is carried through as a string rather
    // than panicked on — one of the refusals below is exactly that
    // case, and the assertion it owes is about the status and the table,
    // not about the shape of the complaint.
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, json)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build GET")
}

fn with_body(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request")
}

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())
        .expect("build DELETE")
}

// ---- the fixtures --------------------------------------------------

/// A PNG the metadata walker accepts: signature, then
/// `length || type || payload || CRC` per chunk. CRCs are zero — the
/// walker reads past them, and its doc says why.
fn png(text: &[(&str, &str)]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&[0u8; 4]);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    chunk(
        &mut out,
        b"IDAT",
        b"a compressed stream, near enough for a walker",
    );
    for (keyword, value) in text {
        let mut payload = keyword.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(value.as_bytes());
        chunk(&mut out, b"tEXt", &payload);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// One export's metadata: the `vdsl` chunk the seeded rule reads, and a
/// `gen` chunk nothing registered reads yet.
///
/// Two chunks because every assertion below is about **one rule's** rows,
/// and with a single chunk "this rule's rows" and "the table" would be
/// the same set. The `gen` chunk is what the rule this file registers
/// selects from, so the seeded rule's rows are the control.
///
/// Built by the serialiser rather than typed out: a chunk one escape away
/// from being invalid JSON derives no key, which would read here as the
/// rule having failed rather than as the fixture being wrong.
fn export(recipe: &str, seed: u64) -> Vec<(String, String)> {
    let chunk = |value: serde_json::Value| {
        serde_json::to_string(&value).expect("the fixture is built by the serialiser")
    };
    vec![
        (
            "vdsl".to_string(),
            chunk(serde_json::json!({"script": recipe, "version": "0.4.0"})),
        ),
        (
            "gen".to_string(),
            chunk(serde_json::json!({"recipe": recipe, "seed": seed})),
        ),
    ]
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

/// Boots a core over a fresh temp profile, registers a persona, and
/// imports `exports.len()` PNGs carrying the given chunks.
///
/// Returns the core (which must outlive the router), the router, and the
/// database path the read side opens.
async fn library(
    tmp: &Path,
    tag: &str,
    exports: &[Vec<(String, String)>],
) -> (CoreCtx, Router, std::path::PathBuf) {
    let corpus = tmp.join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let db_path = tmp.join("asterism.db");

    let core = init_core_with(
        &db_path,
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(format!("e2e-{tag}")),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    for (index, chunks) in exports.iter().enumerate() {
        let path = corpus.join(format!("export-{index}.png"));
        let text: Vec<(&str, &str)> = chunks
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        std::fs::write(&path, png(&text)).expect("write export");
        core.asset_service
            .add(
                add_command(
                    &persona.id,
                    path.to_str().expect("utf-8 path"),
                    1_785_000_000_000 + index as i64 * 1_000,
                ),
                &unattributed(),
            )
            .await
            .expect("add asset");
    }

    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    (core, router, db_path)
}

// ---- the read side -------------------------------------------------

/// Waits until nothing is pending or running in the job queue.
///
/// Every write on this axis is answered by a background pass, so a read
/// taken when the request returned would be a reading of the scheduler.
/// A drain rather than a poll for the rows themselves, because two of
/// the assertions below are about rows that must **not** move: waiting
/// for a condition on the rows can only wait for something to happen,
/// and what has to be established here is that the pipeline is finished.
///
/// `pending + running == 0` is the drained test the queue's own doc
/// gives. Failed rows do not count towards either — which matters here,
/// since these fixtures are PNG headers with no decodable image data and
/// every `thumb_gen` over them fails.
async fn drain(router: &Router, what: &str) {
    for _ in 0..600 {
        let (status, depth) = call(router, get("/asterism/jobs/depth")).await;
        assert_eq!(status, StatusCode::OK, "{depth}");
        let owed = depth["pending"].as_u64().expect("pending")
            + depth["running"].as_u64().expect("running");
        if owed == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what}: the queue never drained");
}

/// The rows one rule has filed, oldest material first — read over a
/// second handle on the same database, because `material_series` has no
/// wire surface yet.
async fn rows_of(db: &Path, strategy: &str) -> Vec<DerivedRow> {
    let strategy = uuid::Uuid::parse_str(strategy).expect("strategy id is a uuid");
    let (isle, driver) = asterism_infra::sqlite::open_and_migrate(db)
        .await
        .expect("second isle");
    let rows: Vec<DerivedRow> = isle
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT asset_id, key, derived_at FROM material_series \
                  WHERE strategy_id = ?1 ORDER BY asset_id",
            )?;
            stmt.query_map(rusqlite::params![strategy], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()
        })
        .await
        .expect("read material_series");
    driver.shutdown().await.ok();
    rows
}

/// The distinct keys among a rule's rows — the grouping, which is the
/// only thing about a key that means anything.
fn distinct_keys(rows: &[DerivedRow]) -> std::collections::BTreeSet<String> {
    rows.iter().filter_map(|(_, key, _)| key.clone()).collect()
}

/// Registers a rule over `POST` and returns its id.
async fn register(router: &Router, body: serde_json::Value) -> String {
    let (status, dto) = call(
        router,
        with_body("POST", "/asterism/series-strategies", body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register: {dto}");
    dto["id"]
        .as_str()
        .expect("the response carries the id")
        .to_string()
}

// ---- the tests -----------------------------------------------------

/// **The whole road: a rule registered over HTTP reaches a key, an edit
/// throws those keys away and gets new ones, and a rename does neither.**
///
/// Three exports, two of them off one recipe. The rule selects
/// `["gen","recipe"]`, so it starts with **two** keys over three
/// materials — the split, not the count, is the claim: a derivation that
/// answered a constant would satisfy "three rows".
///
/// Then `include` moves to `["gen","seed"]`, which differs per export, so
/// the same three materials have to come back on **three** keys. That is
/// what makes this an invalidation test rather than a row count: the old
/// rows would satisfy every cardinality assertion here, and only the
/// grouping tells them apart.
///
/// Two properties are asserted around the edit that nothing else in this
/// slice checks:
///
/// - the **seeded** rule's rows are untouched, key for key and stamp for
///   stamp. A `clear_derived` that forgot its `WHERE` would re-derive the
///   whole library, which on a real one is the difference between a
///   keystroke and a sweep — and would be invisible to a key comparison,
///   since re-deriving an unchanged rule reproduces its keys exactly.
///   The stamp is the only witness, which is why every read here is taken
///   from a drained queue;
/// - the new keys arrive **without another request**. Deleting the rows
///   is only half an invalidation — the walk has to be asked to run, or
///   the library sits keyless until the next launch.
///
/// The rename is the counterweight, and it is asserted on the same rows:
/// a rename that cleared would show three new stamps once the queue
/// drained again.
///
/// Checked by mutation on 2026-08-11, twice:
///
/// - `clear_derived`'s `WHERE strategy_id = ?` dropped → the seeded
///   rule's stamps moved with the edit (*"the edit cleared another
///   rule's answers"*), while its keys stayed identical — which is the
///   whole reason the comparison is on rows and not on keys;
/// - the walk enqueue removed from `SeriesStrategyService::update` →
///   *"left `0`, right `3`"*: the keys were thrown away and never came
///   back, which is what a library would look like until its next
///   launch.
///
/// Restored, it passes.
#[tokio::test(flavor = "multi_thread")]
async fn a_registered_rule_derives_keys_and_editing_it_re_derives_only_its_own() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let exports = [
        export("phase8_hires.lua", 1_000),
        export("phase8_hires.lua", 1_001),
        export("phase9_portrait.lua", 2_000),
    ];
    let (_core, router, db) = library(tmp.path(), "series-edit", &exports).await;
    drain(&router, "the import").await;

    // The seeded rule answering all three is how this file knows the
    // chunks were walked into `meta_kv` at all — everything below reads
    // the same column.
    let seeded_before = rows_of(&db, SEEDED_VDSL_RULE).await;
    assert_eq!(seeded_before.len(), 3, "{seeded_before:#?}");
    assert_eq!(
        distinct_keys(&seeded_before).len(),
        2,
        "two recipes, two keys — the fixture says nothing unless the seeded \
         rule groups: {seeded_before:#?}"
    );

    // ---- register -------------------------------------------------
    let id = register(
        &router,
        serde_json::json!({
            "name": "generator recipe",
            "applies_to": "image/png",
            "decode": "raw_json",
            "include": [["gen", "recipe"]],
        }),
    )
    .await;
    drain(&router, "the walk a registration asks for").await;

    let first = rows_of(&db, &id).await;
    assert_eq!(
        first.len(),
        3,
        "registering a rule reached a key per material without a second \
         request: {first:#?}"
    );
    assert!(first.iter().all(|(_, key, _)| key.is_some()), "{first:#?}");
    let first_keys = distinct_keys(&first);
    assert_eq!(
        first_keys.len(),
        2,
        "three materials off two recipes are two groups: {first:#?}"
    );
    assert!(
        first_keys.iter().all(|key| key.starts_with("sk1-sha256:")),
        "a series key carries its own tag and not a duplicate axis's: {first_keys:?}"
    );

    // ---- an edit that changes what the rule selects ----------------
    let (status, dto) = call(
        &router,
        with_body(
            "PATCH",
            &format!("/asterism/series-strategies/{id}"),
            serde_json::json!({ "include": [["gen", "seed"]] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dto}");
    assert_eq!(dto["include"], serde_json::json!([["gen", "seed"]]));
    drain(&router, "the walk the edit asks for").await;

    let second = rows_of(&db, &id).await;
    assert_eq!(second.len(), 3, "{second:#?}");
    assert!(
        second.iter().all(|(_, key, _)| key.is_some()),
        "{second:#?}"
    );
    let second_keys = distinct_keys(&second);
    assert!(
        second_keys.is_disjoint(&first_keys),
        "every key was derived under the old rule and none was re-derived — \
         the invalidation did not happen, or nothing was asked to run: \
         {second:#?}"
    );
    assert_eq!(
        second_keys.len(),
        3,
        "the seed differs per export, so the new rule separates all three: {second:#?}"
    );

    // …and the rule the edit was not about did not move.
    assert_eq!(
        rows_of(&db, SEEDED_VDSL_RULE).await,
        seeded_before,
        "the edit cleared another rule's answers — key and stamp both have to \
         be identical, or the invalidation was not scoped to its own id"
    );

    // ---- a rename, which is not an edit the derivation reads -------
    // Far enough apart that a re-derivation could not land on the stamps
    // above, which is what carries the assertion.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (status, dto) = call(
        &router,
        with_body(
            "PATCH",
            &format!("/asterism/series-strategies/{id}"),
            serde_json::json!({ "name": "generator seed" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dto}");
    assert_eq!(dto["name"], "generator seed");
    assert_eq!(
        dto["include"],
        serde_json::json!([["gen", "seed"]]),
        "an omitted field is left alone"
    );
    drain(&router, "whatever the rename asked for").await;

    assert_eq!(
        rows_of(&db, &id).await,
        second,
        "renaming a rule threw its keys away — a name is a label the \
         derivation never reads, and re-deriving a library over one is the \
         cost this comparison exists to refuse"
    );
}

/// **Deleting a rule takes its keys and leaves the others', and a seeded
/// rule is editable — with its stamp moved.**
///
/// The two halves are in one fixture because they are the same question
/// from opposite sides: `system` records that a migration wrote the row
/// and grants it nothing, so the seed can be edited like any other rule —
/// and the thing that keeps that safe is the `updated_at` this edit
/// moves, which is how a later corrective migration tells a pristine seed
/// from one somebody took over (`system = 1 AND updated_at =
/// created_at`).
///
/// The delete is asserted through `material_series` rather than through
/// the listing: the rule row going is what `GET` can see, and the keys
/// going with it is the cascade, which no HTTP read reaches.
///
/// Checked by mutation on 2026-08-11, twice:
///
/// - `updated_at` dropped from the `UPDATE`'s `SET` list → *"an edited
///   seed must stop reading as pristine"*, with the response still
///   carrying `updated_at_ms == created_at_ms`;
/// - `ON DELETE CASCADE` removed from V73's `strategy_id` foreign key →
///   the delete answered *"500 … FOREIGN KEY constraint failed"*, which
///   is the schema refusing to let a rule go while anything derived
///   under it stands.
///
/// Restored, it passes.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_rule_cascades_and_a_seeded_rule_is_editable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let exports = [
        export("phase8_hires.lua", 1_000),
        export("phase9.lua", 2_000),
    ];
    let (_core, router, db) = library(tmp.path(), "series-delete", &exports).await;

    let id = register(
        &router,
        serde_json::json!({
            "name": "generator recipe",
            "applies_to": "image/png",
            "decode": "raw_json",
            "include": [["gen", "recipe"]],
        }),
    )
    .await;
    drain(&router, "the import and the walk it asked for").await;
    assert_eq!(rows_of(&db, &id).await.len(), 2, "the rule derived");
    let seeded = rows_of(&db, SEEDED_VDSL_RULE).await;
    assert_eq!(seeded.len(), 2, "and so did the seeded one");

    // ---- the seeded rule is editable, and the edit is dateable -----
    let (status, listed) = call(&router, get("/asterism/series-strategies")).await;
    assert_eq!(status, StatusCode::OK);
    let seed_dto = listed
        .as_array()
        .expect("a list")
        .iter()
        .find(|rule| rule["id"] == SEEDED_VDSL_RULE)
        .expect("the migration seeded a rule")
        .clone();
    assert_eq!(seed_dto["system"], true, "and it says a migration wrote it");
    assert_eq!(
        seed_dto["created_at_ms"], seed_dto["updated_at_ms"],
        "nothing has edited it yet — the pair a corrective migration reads"
    );

    let (status, edited) = call(
        &router,
        with_body(
            "PATCH",
            &format!("/asterism/series-strategies/{SEEDED_VDSL_RULE}"),
            serde_json::json!({ "name": "VDSL recipe (mine)" }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "`system` is provenance, not permission: {edited}"
    );
    assert_eq!(edited["name"], "VDSL recipe (mine)");
    assert_eq!(edited["system"], true, "editing it does not un-seed it");
    assert_eq!(
        edited["created_at_ms"], seed_dto["created_at_ms"],
        "when the migration wrote it is not something an edit changes"
    );
    assert!(
        edited["updated_at_ms"].as_i64().expect("a stamp")
            > edited["created_at_ms"].as_i64().expect("a stamp"),
        "an edited seed must stop reading as pristine, or the next corrective \
         migration overwrites somebody's rule: {edited}"
    );

    // ---- and the delete ------------------------------------------
    let (status, body) = call(
        &router,
        delete(&format!("/asterism/series-strategies/{id}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, listed) = call(&router, get("/asterism/series-strategies")).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = listed
        .as_array()
        .expect("a list")
        .iter()
        .map(|rule| rule["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(
        ids,
        vec![SEEDED_VDSL_RULE],
        "the deleted rule is gone and the seeded one is not: {ids:?}"
    );

    drain(&router, "anything the delete asked for").await;
    assert!(
        rows_of(&db, &id).await.is_empty(),
        "the deleted rule's keys did not go with it"
    );
    assert_eq!(
        rows_of(&db, SEEDED_VDSL_RULE).await,
        seeded,
        "the cascade took the whole table rather than one rule's rows"
    );

    // A second delete names nothing, and says so rather than reporting a
    // library change that did not happen.
    let (status, _) = call(
        &router,
        delete(&format!("/asterism/series-strategies/{id}")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// **A rule this build could not carry out is refused at the door and is
/// not in the table.**
///
/// This is the one test in the slice that is not optional, and the reason
/// is the blast radius rather than tidiness: the derivation walk promotes
/// every rule on its page, so **one** `series_strategy` row this build
/// cannot read makes every page fail and no material gets a key under
/// *any* rule. The column holds what a writer put there, and this route
/// is the writer.
///
/// Six bodies, and the last one is refused a floor lower: `include` as a
/// flat list is a shape `serde` rejects, so `Json` answers `422` before
/// the handler runs, while the five the type system cannot express reach
/// the service and answer `400`. The distinction is not the property
/// being asserted — **nothing was stored** is — so the status assertion
/// is client-error for that one and exact for the rest.
///
/// Three of the five are media types, because `MimeType::parse` answers
/// for every string: `"   "`, `"png"` and `"image/*"` all store without
/// complaint and then match nothing for the life of the rule, which from
/// outside is indistinguishable from a rule that is broken. The positive
/// at the end is what keeps that guard on shape rather than on
/// vocabulary — a subtype this build has never heard of is a media type
/// a material can carry, and a rule against it has to register.
///
/// Checked by mutation on 2026-08-11, twice:
///
/// - the empty-path refusal removed from `parse_paths` → *"a path with
///   no segments … left `200`, right `400`"*, with the response body
///   showing the rule that landed, `include` spelled
///   `[["gen","recipe"],[]]`;
/// - `parse_applies_to` reverted to its first form (refuse blank, accept
///   anything else) → *"a media type with no subtype … left `200`, right
///   `400`"*, and the rule landed carrying `"applies_to":"png"`.
///
/// Restored, it passes.
#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_rule_is_refused_and_lands_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router, _db) = library(tmp.path(), "series-refuse", &[]).await;

    let well_formed = serde_json::json!({
        "name": "generator recipe",
        "applies_to": "image/png",
        "decode": "raw_json",
        "include": [["gen", "recipe"]],
    });

    for (what, body, named) in [
        // `"exif"` stood here while it was the decoder that had not
        // shipped, which is the shape of the case: a rule registered
        // against a later build's decoder set. It shipped — with the
        // JPEG meta axis — so the case needs a token that has not, or it
        // stops being about a refusal at all.
        (
            "a decoder this build has not shipped",
            {
                let mut body = well_formed.clone();
                body["decode"] = serde_json::json!("prose_pairs");
                body
            },
            "prose_pairs",
        ),
        (
            "a rule that claims no media type",
            {
                let mut body = well_formed.clone();
                body["applies_to"] = serde_json::json!("   ");
                body
            },
            "applies_to",
        ),
        // The two `MimeType::parse` answers for happily and
        // `Strategy::claims` can never match. `"png"` is the one a person
        // types; `"image/*"` is the one the schema resource promises is
        // refused, which it has to be or the resource is describing a
        // guard that does not exist.
        (
            "a media type with no subtype",
            {
                let mut body = well_formed.clone();
                body["applies_to"] = serde_json::json!("png");
                body
            },
            "applies_to",
        ),
        (
            "a wildcard, which reads as widening and matches nothing",
            {
                let mut body = well_formed.clone();
                body["applies_to"] = serde_json::json!("image/*");
                body
            },
            "applies_to",
        ),
        (
            "a path with no segments",
            {
                let mut body = well_formed.clone();
                body["include"] = serde_json::json!([["gen", "recipe"], []]);
                body
            },
            "include[1]",
        ),
    ] {
        let (status, error) = call(
            &router,
            with_body("POST", "/asterism/series-strategies", body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {error}");
        assert_eq!(error["kind"], "Validation", "{what}: {error}");
        assert!(
            error["message"]
                .as_str()
                .expect("a message")
                .contains(named),
            "{what}: the message names what was wrong: {error}"
        );
    }

    // One path where a list of them belongs — the mistake that spells
    // most like the right thing, and the one `serde` catches.
    let mut flattened = well_formed.clone();
    flattened["include"] = serde_json::json!(["gen", "recipe"]);
    let (status, error) = call(
        &router,
        with_body("POST", "/asterism/series-strategies", flattened),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an include that is not a list of paths must be refused at the door, \
         got {status}: {error}"
    );

    // The point of all four: the table still holds exactly what the
    // migration put in it.
    let (status, listed) = call(&router, get("/asterism/series-strategies")).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = listed
        .as_array()
        .expect("a list")
        .iter()
        .map(|rule| rule["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(
        ids,
        vec![SEEDED_VDSL_RULE],
        "a refused rule reached the column — one row this build cannot read \
         stops the whole axis: {ids:?}"
    );

    // And a well-formed one still lands, so the assertions above are
    // about these bodies rather than about the route being broken.
    let id = register(&router, well_formed.clone()).await;
    let (status, listed) = call(&router, get("/asterism/series-strategies")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().expect("a list").len(), 2);
    assert!(
        listed
            .as_array()
            .expect("a list")
            .iter()
            .any(|rule| rule["id"] == id),
        "the registered rule is listed: {listed}"
    );

    // The media-type guard is on shape and not on vocabulary: a subtype
    // this build has never heard of is one an importer may legitimately
    // have declared, so a rule against it registers. Without this the
    // three refusals above would be satisfied by a guard that admitted
    // only the handful of types `MimeType` names, which would refuse
    // rules against formats a library already holds.
    let mut unfamiliar = well_formed;
    unfamiliar["applies_to"] = serde_json::json!("image/jxl");
    let (status, dto) = call(
        &router,
        with_body("POST", "/asterism/series-strategies", unfamiliar),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dto}");
    assert_eq!(dto["applies_to"], "image/jxl");
}
