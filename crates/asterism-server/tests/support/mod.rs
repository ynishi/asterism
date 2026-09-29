//! What an e2e that waits on the job worker says when the wait runs out.
//!
//! Included with `mod support;` by the files whose assertions wait on a
//! background job. A wait that times out says only that nothing
//! arrived; these two say what the queue and the worker were doing
//! instead — jobs left pending, a job still running, or a worker that
//! stopped claiming. Nothing else records that in a test process: the
//! per-run records and the worker's own errors go out through
//! `tracing`, and a test has no subscriber unless it installs one.

use asterism_server::core_init::CoreCtx;

/// Routes `tracing` into the test's captured output: warnings from
/// anywhere, and every job run's record (kind, outcome, duration) from
/// the job module. Captured, so a passing test prints none of it.
///
/// The first call in a test binary installs the subscriber and every
/// later one is a no-op, which is why each test may call it.
pub fn trace_jobs() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "warn,asterism_infra::jobs=info",
        ))
        .try_init();
}

/// The job queue by status and by kind, for a timeout message.
pub async fn queue_state(core: &CoreCtx) -> String {
    match asterism_infra::jobs::jobs_snapshot(&core.jobs_pool).await {
        Ok(snapshot) => format!("{snapshot:?}"),
        Err(err) => format!("unreadable: {err}"),
    }
}
