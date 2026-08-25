//! End-to-end guard for `GET /asterism/heads/status` (#130).
//!
//! The route is thin, so what can be wrong is the wiring: the method,
//! the path, and the shape of what comes back. A screen renders a
//! different state for each field of this body, so a renamed or
//! dropped one is a blank panel rather than an error.
//!
//! What it asserts is the answer for a library that has never trained,
//! which is every library on its first launch: no pointer, nothing
//! bound, nothing a relaunch would change, and a readiness line whose
//! counts are zero rather than absent. The store is read from beside
//! the database, so a tempdir database means a tempdir heads store and
//! this answer does not depend on what the machine running the test
//! has trained.

use std::sync::Arc;

use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

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
    let json = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body is not JSON ({e}): {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, json)
}

#[tokio::test]
async fn head_status_answers_a_library_that_has_never_trained() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_core, router) = harness(tmp.path()).await;

    let (status, body) = call(
        &router,
        Request::builder()
            .uri("/asterism/heads/status")
            .body(Body::empty())
            .expect("build GET"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["promoted"], serde_json::Value::Null);
    assert_eq!(body["bound"], serde_json::Value::Null);
    assert_eq!(body["restart_required"], serde_json::json!(false));
    assert_eq!(body["run"], serde_json::Value::Null);
    // The floor travels with the counts so the panel does not carry a
    // copy of a constant the trainer owns.
    assert_eq!(body["readiness"]["rulings"], serde_json::json!(0));
    assert_eq!(body["readiness"]["tags_with_rulings"], serde_json::json!(0));
    assert_eq!(body["readiness"]["tags_ready"], serde_json::json!(0));
    assert_eq!(
        body["readiness"]["min_rulings_per_class"],
        serde_json::json!(asterism_core::domain::tag_head::MIN_RULINGS_PER_CLASS)
    );
}
