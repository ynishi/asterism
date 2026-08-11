//! Tiny progress reporter — running success / failure counters, plus
//! an event stream to stderr.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cheaply cloneable progress handle. All counters live on the same
/// shared atomics so multiple pipeline stages can bump them without
/// coordination.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    ok: AtomicU64,
    err: AtomicU64,
}

impl Progress {
    /// Builds a fresh progress handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one successful item, logs a short line to stderr, and
    /// returns the new success count.
    pub fn record_ok(&self, locator: &str) -> u64 {
        let n = self.inner.ok.fetch_add(1, Ordering::Relaxed) + 1;
        eprintln!("ok  [{n}] {locator}");
        n
    }

    /// Records one failed item, logs the reason to stderr, and returns
    /// the new failure count.
    pub fn record_err(&self, locator: &str, reason: &str) -> u64 {
        let n = self.inner.err.fetch_add(1, Ordering::Relaxed) + 1;
        eprintln!("err [{n}] {locator}: {reason}");
        n
    }

    /// Current successful-item count.
    pub fn ok_count(&self) -> u64 {
        self.inner.ok.load(Ordering::Relaxed)
    }

    /// Current failed-item count.
    pub fn err_count(&self) -> u64 {
        self.inner.err.load(Ordering::Relaxed)
    }
}
