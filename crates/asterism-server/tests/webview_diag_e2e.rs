//! The webview diagnostics write path, end to end: a captured
//! console/error moment arrives as `POST /asterism/diag`, is re-emitted
//! through the process-global tracing subscriber, and comes back out of
//! `GET /asterism/diag?target=asterism_webview` — the same pipe every
//! native diagnostic rides, which is the point (the webview half of
//! the diagnostics story died with `tauri-plugin-log`
//! and this is its replacement).
//!
//! Everything lives in one test fn on purpose: the diag sink is a
//! process-global (`observe::install` + `attach`), so two cores in
//! parallel test fns would race for the single drain channel and the
//! records of one would land in the database of the other. Its own
//! test binary because `init_core` opens the profile-global Tantivy
//! index (one core per test binary, as with the sibling e2e files).

use std::sync::Arc;

use asterism_server::core_init::{CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn post_diag(
    router: &axum::Router,
    payload: serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/asterism/diag")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .expect("build request"),
        )
        .await
        .expect("route answers");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::json!(null));
    (status, json)
}

async fn list_webview_diag(router: &axum::Router) -> Vec<serde_json::Value> {
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/asterism/diag?target=asterism_webview&limit=20")
                .body(axum::body::Body::empty())
                .expect("build request"),
        )
        .await
        .expect("route answers");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice::<Vec<serde_json::Value>>(&body).expect("json array")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_webview_diagnostic_round_trips_into_diag_log() {
    // Order matters: the subscriber must exist before the core opens
    // the database (`init_core` calls `observe::attach`, which is a
    // no-op unless `install` ran) — the same order `run()` and the
    // headless `main` use.
    asterism_infra::observe::install();

    let tmp = tempfile::tempdir().expect("tempdir");
    let core = init_core_with(
        &tmp.path().join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::Full,
        Some(&tmp.path().join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));

    // A valid capture is accepted…
    let (status, body) = post_diag(
        &router,
        serde_json::json!({
            "level": "error",
            "event": "webview.console_error",
            "message": "TypeError: x is not a function",
            "attrs_json": "{\"stack\":\"at DetailPane.svelte:42\"}",
        }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(body.get("recorded"), Some(&serde_json::json!(true)));

    // …and lands in `diag_log` under the webview target. The sink
    // drains through a channel, so ride a short poll rather than
    // asserting on the very next read.
    let mut row = None;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if let Some(found) = list_webview_diag(&router).await.into_iter().next() {
            row = Some(found);
            break;
        }
    }
    let row = row.expect("the record reaches diag_log within the poll budget");
    assert_eq!(
        row.get("event").and_then(|v| v.as_str()),
        Some("webview.console_error")
    );
    assert_eq!(row.get("level").and_then(|v| v.as_str()), Some("ERROR"));
    assert_eq!(
        row.get("message").and_then(|v| v.as_str()),
        Some("TypeError: x is not a function")
    );
    let attrs = row
        .get("attrs_json")
        .and_then(|v| v.as_str())
        .expect("attrs blob");
    assert!(
        attrs.contains("DetailPane.svelte:42"),
        "the client context rides along: {attrs}"
    );

    // The strict edges: an unknown level, an event outside the
    // `webview.` namespace (which could otherwise steer the record
    // into the perf/job stream routing), and an empty message are all
    // rejected as validation errors, not guessed into shape.
    for bad in [
        serde_json::json!({ "level": "fatal", "event": "webview.x", "message": "m", "attrs_json": null }),
        serde_json::json!({ "level": "error", "event": "job.sneaky", "message": "m", "attrs_json": null }),
        serde_json::json!({ "level": "error", "event": "webview.x", "message": "  ", "attrs_json": null }),
    ] {
        let (status, body) = post_diag(&router, bad.clone()).await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "payload {bad} must be rejected, got {body}"
        );
    }
}
