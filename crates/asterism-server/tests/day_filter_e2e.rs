//! End-to-end guard for the calendar filter — `day_from` / `day_until`
//! / `day_of_year` with `time_zone` on [`ListAssetsQuery`] — over the
//! real service graph and both transports that carry the query: the
//! HTTP list route and the MCP `asset_search` tool.
//!
//! # What this is asked to prove
//!
//! The filter cuts an asset's **resolved** time, not its `occurred_at`
//! column: the stamp its source says it means (`created_at` for an
//! `import`-sourced row, `occurred_at` otherwise), read in the row's
//! own zone when it carries one and in the viewer's otherwise
//! (`asterism_core::domain::asset_zone`). Each rule is invisible to
//! every layer taken alone — the mapper resolves a `DayFilter`, the
//! repository spells the predicate, the service passes it on — and
//! what is worth pinning is that a row lands on exactly one side of
//! each cut once every layer is live. So every assertion here is an
//! equality over ids, not a "contains".
//!
//! # Why the fixtures are instants written in UTC
//!
//! Asia/Tokyo is UTC+9 with no transitions and America/Phoenix is
//! UTC-7 with none, so every window below is arithmetic a reader can
//! check by hand, and no date in this file depends on which year's
//! rule the tz database applied. What a zone that transitions does is
//! the unit tests' subject (`asset_zone`); the e2e is about which rows
//! a live query returns.
//!
//! The one instant that is not written down is the `import`-sourced
//! row's: its time is its arrival, which is the moment the test adds
//! it. That case reads the stamp back off the DTO and asks about the
//! day it fell on.

use std::sync::Arc;
use std::time::Duration;

use asterism_contract::command::{AddAssetCommand, OccurredSource, RegisterPersonaCommand};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The attribution these fixtures write with: a caller that states
/// nothing, which records nothing. They are about when things happened,
/// not about who ingested them.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

/// Same tempdir harness as `mcp_transport_e2e` — the Tantivy index
/// override keeps the test out of the developer's profile.
async fn harness(tmp: &std::path::Path) -> (CoreCtx, Router) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    (core, router)
}

/// Epoch ms at `hh:mm:ss` UTC on a date the calendar has.
fn utc_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("a UTC instant the calendar has")
        .timestamp_millis()
}

/// Tokyo's hours east of UTC. A fixed-offset zone (see the module
/// doc), which is what lets the expectations be written as arithmetic.
const TOKYO_HOURS: i64 = 9;

/// The calendar day an instant falls on in a fixed-offset zone.
fn local_date(ms: i64, offset_hours: i64) -> NaiveDate {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms + offset_hours * 3_600_000)
        .expect("an instant chrono can hold")
        .date_naive()
}

fn add_command(persona_id: &str, locator: &str, occurred_at_ms: i64) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".into(),
        locator: locator.to_string(),
        modality: Some("image".into()),
        occurred_at_ms,
        occurred_source: Default::default(),
        time_zone: None,
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

/// Writes one fixture file under `corpus` and adds it as `command`
/// with the locator filled in. Returns the DTO, which is where the
/// arrival stamp and the recorded zone are read back from.
async fn seed(
    core: &CoreCtx,
    corpus: &std::path::Path,
    name: &str,
    body: &str,
    command: AddAssetCommand,
) -> asterism_contract::dto::AssetDto {
    let path = corpus.join(format!("{name}.md"));
    std::fs::write(&path, format!("{body}\n")).expect("write fixture");
    core.asset_service
        .add(
            AddAssetCommand {
                locator: path.to_str().unwrap().to_owned(),
                ..command
            },
            &unattributed(),
        )
        .await
        .unwrap_or_else(|e| panic!("add {name}: {e}"))
}

/// One `GET` through the router, answered as status + parsed JSON.
async fn get(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build GET");
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
    let json = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body is not JSON ({e}): {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, json)
}

/// The ids on a page, sorted, so two answers compare as sets.
fn ids_of(page: &serde_json::Value) -> Vec<String> {
    let mut ids: Vec<String> = page["items"]
        .as_array()
        .unwrap_or_else(|| panic!("page carries no items array: {page}"))
        .iter()
        .map(|item| item["id"].as_str().expect("item id").to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(ids: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    ids.sort();
    ids
}

/// `GET /asterism/assets` under one calendar range in one zone, as a
/// sorted id set. `zone` is spliced in as given: the two names used
/// here carry a `/`, which a query string may hold raw.
async fn list_range(router: &Router, from: &str, until: &str, zone: &str) -> Vec<String> {
    let uri = format!("/asterism/assets?day_from={from}&day_until={until}&time_zone={zone}");
    let (status, page) = get(router, &uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {page}");
    ids_of(&page)
}

/// `GET /asterism/assets` under one day-of-year in one zone. The
/// object rides the query string JSON-encoded, the dual form `sort`
/// takes (`ListAssetsQuery::day_of_year`); the braces, quotes, colon
/// and comma are percent-encoded by hand because that is the whole of
/// what the value contains.
async fn list_day_of_year(router: &Router, month: u32, day: u32, zone: &str) -> Vec<String> {
    let encoded = format!("%7B%22month%22%3A{month}%2C%22day%22%3A{day}%7D");
    let uri = format!("/asterism/assets?day_of_year={encoded}&time_zone={zone}");
    let (status, page) = get(router, &uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {page}");
    ids_of(&page)
}

// ---------------------------------------------------------------------
// MCP client, the same legacy session flow `mcp_transport_e2e` speaks.
// ---------------------------------------------------------------------

/// One JSON-RPC message POSTed to `/mcp`. Returns the status, the
/// `Mcp-Session-Id` response header (present on `initialize`), and the
/// last `data:` SSE frame parsed as JSON.
async fn mcp_call(
    router: &Router,
    session: Option<&str>,
    message: serde_json::Value,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        // `oneshot` builds a raw request with no Host header, but the
        // service's DNS-rebinding guard requires one and allows only
        // loopback names. Real clients always send it.
        .header("host", "127.0.0.1")
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json");
    if let Some(session) = session {
        builder = builder.header("mcp-session-id", session);
    }
    let request = builder
        .body(Body::from(message.to_string()))
        .expect("build MCP POST");
    let (status, session, bytes) = tokio::time::timeout(Duration::from_secs(20), async {
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
    let last_data = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).expect("SSE data frame is JSON"))
        .or_else(|| {
            (!bytes.is_empty() && text.trim_start().starts_with('{'))
                .then(|| serde_json::from_slice(&bytes).expect("JSON body"))
        })
        .unwrap_or(serde_json::Value::Null);
    (status, session, last_data)
}

/// Runs the `initialize` → `notifications/initialized` handshake and
/// returns the session id.
async fn handshake(router: &Router) -> String {
    let (status, session, reply) = mcp_call(
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
    let session = session.expect("initialize answers with a session id");
    let (status, _, _) = mcp_call(
        router,
        Some(&session),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "initialized notification");
    session
}

/// Calls `asset_search` and returns the parsed `CallToolResult`.
async fn asset_search(
    router: &Router,
    session: &str,
    id: u64,
    text: &str,
    filter: serde_json::Value,
) -> serde_json::Value {
    let (status, _, reply) = mcp_call(
        router,
        Some(session),
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {
                "name": "asset_search",
                "arguments": {"text": text, "filter": filter},
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tools/call asset_search: {reply}");
    assert!(
        reply["error"].is_null(),
        "asset_search answered a protocol error: {reply}"
    );
    reply["result"].clone()
}

/// Parses the JSON payload a tool packed into its text content block.
fn tool_json(result: &serde_json::Value) -> serde_json::Value {
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result has no text content: {result}"));
    serde_json::from_str(text).expect("tool content is JSON")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_calendar_filter_cuts_the_resolved_time_on_both_transports() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("corpus dir");
    let (core, router) = harness(tmp.path()).await;

    let persona = core
        .persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some("e2e-day-filter".into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona");

    // Every body carries "lantern" so the text search reaches all of
    // them and the calendar filter is what narrows; one carries a
    // second word so the text and the day can be seen composing.
    let plain = |ms: i64| add_command(&persona.id, "", ms);

    // Tokyo's 14 March 2026 is [2026-03-13T15:00Z, 2026-03-14T15:00Z).
    // Two rows inside it at its two edges, two rows one second and one
    // instant outside it.
    let inside_early = seed(
        &core,
        &corpus,
        "inside-early",
        "lantern paper",
        plain(utc_ms(2026, 3, 13, 15, 0, 0)),
    )
    .await;
    let inside_late = seed(
        &core,
        &corpus,
        "inside-late",
        "lantern",
        plain(utc_ms(2026, 3, 14, 14, 30, 0)),
    )
    .await;
    let before = seed(
        &core,
        &corpus,
        "before",
        "lantern",
        plain(utc_ms(2026, 3, 13, 14, 59, 59)),
    )
    .await;
    let after = seed(
        &core,
        &corpus,
        "after",
        "lantern",
        plain(utc_ms(2026, 3, 14, 15, 0, 0)),
    )
    .await;
    // The same day seven years earlier, and the day before it, for the
    // day-of-year cut. Noon in Tokyo, so no boundary is near.
    let other_year = seed(
        &core,
        &corpus,
        "other-year",
        "lantern",
        plain(utc_ms(2019, 3, 14, 3, 0, 0)),
    )
    .await;
    let other_year_off = seed(
        &core,
        &corpus,
        "other-year-off",
        "lantern",
        plain(utc_ms(2019, 3, 13, 3, 0, 0)),
    )
    .await;
    // A row whose importer had no occurrence and wrote the import
    // moment: its `occurred_at` says 2019, its source says `import`,
    // and its time is its arrival — the moment this call runs.
    let imported = seed(
        &core,
        &corpus,
        "imported",
        "lantern",
        AddAssetCommand {
            occurred_source: OccurredSource::Import,
            ..plain(utc_ms(2019, 3, 14, 3, 0, 0))
        },
    )
    .await;
    // A row that knows where it happened: 20:00Z on the 14th is 05:00
    // on 15 March in Tokyo, and the row says Tokyo. Its unzoned twin
    // sits at the same instant and is read in whatever zone the viewer
    // names.
    let zoned = seed(
        &core,
        &corpus,
        "zoned",
        "lantern",
        AddAssetCommand {
            time_zone: Some("Asia/Tokyo".into()),
            ..plain(utc_ms(2026, 3, 14, 20, 0, 0))
        },
    )
    .await;
    let zoned_twin = seed(
        &core,
        &corpus,
        "zoned-twin",
        "lantern",
        plain(utc_ms(2026, 3, 14, 20, 0, 0)),
    )
    .await;
    assert_eq!(zoned.time_zone.as_deref(), Some("Asia/Tokyo"));
    assert_eq!(imported.occurred_source, "import");

    // A range in Tokyo returns exactly the rows whose resolved instant
    // falls in it: both edges in, the second before out, the instant
    // that opens the next day out, the zoned row out on its own local
    // day (15 March), the imported row out because its time is today.
    assert_eq!(
        list_range(&router, "2026-03-14", "2026-03-15", "Asia/Tokyo").await,
        sorted(&[&inside_early.id, &inside_late.id]),
        "Tokyo's 14 March 2026"
    );
    // The range is half-open at the end, so widening it by a day picks
    // up exactly the instant that was excluded above, and the twin
    // (05:00 on the 15th in Tokyo). The zoned row's local day is also
    // the 15th, and the string comparison admits it.
    assert_eq!(
        list_range(&router, "2026-03-14", "2026-03-16", "Asia/Tokyo").await,
        sorted(&[
            &inside_early.id,
            &inside_late.id,
            &after.id,
            &zoned_twin.id,
            &zoned.id
        ]),
        "Tokyo's 14 and 15 March 2026"
    );

    // The import-sourced row is found on the day it arrived and not on
    // the day its `occurred_at` column names. The arrival is read back
    // off the DTO rather than off the clock, so the expectation and the
    // row agree to the millisecond.
    let arrived = local_date(imported.created_at_ms, TOKYO_HOURS);
    assert!(
        arrived > NaiveDate::from_ymd_opt(2026, 3, 31).expect("a date"),
        "the fixtures' fixed dates must lie before any run: {arrived}"
    );
    let next = arrived.succ_opt().expect("a next day");
    assert_eq!(
        list_range(
            &router,
            &arrived.to_string(),
            &next.to_string(),
            "Asia/Tokyo"
        )
        .await,
        sorted(&[&imported.id]),
        "the day the import-sourced row arrived, in Tokyo"
    );
    assert_eq!(
        list_range(&router, "2019-03-14", "2019-03-15", "Asia/Tokyo").await,
        sorted(&[&other_year.id]),
        "the day the import-sourced row's occurred_at names holds only the row that means it"
    );

    // A zoned row is read on its own local day and the viewer's zone
    // never enters. Phoenix's 15 March 2026 is [2026-03-15T07:00Z,
    // 2026-03-16T07:00Z), which holds neither twin's instant; the
    // zoned row is found all the same, because its own day is the
    // 15th. Phoenix's 14 March holds the instant, and the zoned row is
    // not in it, because its own day is not the 14th — while its
    // unzoned twin is.
    assert_eq!(
        list_range(&router, "2026-03-15", "2026-03-16", "America/Phoenix").await,
        sorted(&[&zoned.id]),
        "Phoenix's 15 March 2026 reaches the zoned row by its local day"
    );
    assert_eq!(
        list_range(&router, "2026-03-14", "2026-03-15", "America/Phoenix").await,
        sorted(&[&inside_late.id, &after.id, &zoned_twin.id]),
        "Phoenix's 14 March 2026 reads the twin in Phoenix and leaves the zoned row on its own day"
    );

    // Day of year: 14 March across every year the corpus spans, in
    // Tokyo. Both edge rows and the 2019 row; the imported row only
    // when its arrival happens to be a 14 March, which the expectation
    // reads off the DTO rather than assuming away.
    let on_day = |month: u32, day: u32, ids: &[&str]| {
        let mut ids = sorted(ids);
        if (arrived.month(), arrived.day()) == (month, day) {
            ids.push(imported.id.clone());
            ids.sort();
        }
        ids
    };
    assert_eq!(
        list_day_of_year(&router, 3, 14, "Asia/Tokyo").await,
        on_day(3, 14, &[&inside_early.id, &inside_late.id, &other_year.id]),
        "14 March, every year, in Tokyo"
    );
    // 15 March: the instant that opens it, the twin (05:00 on the
    // 15th), and the zoned row by its own local day.
    assert_eq!(
        list_day_of_year(&router, 3, 15, "Asia/Tokyo").await,
        on_day(3, 15, &[&after.id, &zoned_twin.id, &zoned.id]),
        "15 March, every year, in Tokyo"
    );
    // 13 March holds the second-before row and the 2019 neighbour.
    assert_eq!(
        list_day_of_year(&router, 3, 13, "Asia/Tokyo").await,
        on_day(3, 13, &[&before.id, &other_year_off.id]),
        "13 March, every year, in Tokyo"
    );

    // A day without a zone is refused, on both transports, as the
    // request's fault and not the corpus's.
    let (status, body) = get(
        &router,
        "/asterism/assets?day_from=2026-03-14&day_until=2026-03-15",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["kind"], "Validation", "{body}");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("time_zone")),
        "the refusal names the missing field: {body}"
    );

    // MCP `asset_search`: the calendar filter narrows the text search
    // the way every other chip does, and composes with the text.
    // Indexing is asynchronous (enqueue on add, worker reads the file
    // and commits to Tantivy), so poll until the unfiltered search
    // reaches every row before asking anything narrower.
    let session = handshake(&router).await;
    let all = 9;
    let mut indexed = false;
    for _ in 0..120 {
        let page =
            tool_json(&asset_search(&router, &session, 2, "lantern", serde_json::json!({})).await);
        if page["items"].as_array().map(Vec::len) == Some(all) {
            indexed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(indexed, "all {all} lantern rows indexed within 30s");

    let narrowed = tool_json(
        &asset_search(
            &router,
            &session,
            3,
            "lantern",
            serde_json::json!({
                "day_from": "2026-03-14",
                "day_until": "2026-03-15",
                "time_zone": "Asia/Tokyo",
            }),
        )
        .await,
    );
    assert_eq!(
        ids_of(&narrowed),
        sorted(&[&inside_early.id, &inside_late.id]),
        "asset_search under Tokyo's 14 March 2026"
    );
    assert_eq!(narrowed["matched"], 2, "{narrowed}");

    let composed = tool_json(
        &asset_search(
            &router,
            &session,
            4,
            "paper",
            serde_json::json!({
                "day_of_year": {"month": 3, "day": 14},
                "time_zone": "Asia/Tokyo",
            }),
        )
        .await,
    );
    assert_eq!(
        ids_of(&composed),
        sorted(&[&inside_early.id]),
        "the text and the day-of-year compose"
    );
    let disjoint = tool_json(
        &asset_search(
            &router,
            &session,
            5,
            "paper",
            serde_json::json!({
                "day_from": "2019-03-14",
                "day_until": "2019-03-15",
                "time_zone": "Asia/Tokyo",
            }),
        )
        .await,
    );
    assert!(
        ids_of(&disjoint).is_empty(),
        "a day that excludes every text hit is an empty page, not the unfiltered one: {disjoint}"
    );

    let refused = asset_search(
        &router,
        &session,
        6,
        "lantern",
        serde_json::json!({"day_from": "2026-03-14", "day_until": "2026-03-15"}),
    )
    .await;
    assert_eq!(
        refused["isError"], true,
        "a day without a zone is a tool error: {refused}"
    );
    let error = tool_json(&refused);
    assert_eq!(error["kind"], "Validation", "{error}");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|m| m.contains("time_zone")),
        "the refusal names the missing field: {error}"
    );
}
