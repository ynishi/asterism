//! The auth rate limiter — hand-rolled, per-key, sliding-window.
//!
//! ## Why hand-rolled
//!
//! #83 §5 requires one limiter over ALL auth endpoints from v0. The
//! workspace has no per-key rate-limiting dependency: `tower`'s own
//! limit layers are global concurrency/rate caps with no notion of a
//! client key, and the crates that do keyed limiting (`tower_governor`
//! and friends) would arrive for exactly one middleware.
//! A sliding-window log over a `Mutex<HashMap>` is ~40 lines, has no
//! background task, and its failure mode (a mutex) is simpler than a
//! dependency's upgrade treadmill — so the decision, recorded here, is
//! to hand-roll until a second consumer wants something richer.
//!
//! ## Key choice
//!
//! Per client IP when the connection carries one
//! (`into_make_service_with_connect_info` in `main.rs`); a fixed
//! `"local"` key otherwise (in-process tests drive the router without
//! a socket). Keying by IP rather than by login means a spray across
//! many logins from one address is still one bucket, and a distributed
//! attacker burns addresses, not accounts.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Keys above which a check opportunistically drops empty buckets, so
/// an address-rotating client cannot grow the map without bound.
const SWEEP_THRESHOLD: usize = 4096;

/// A per-key sliding-window counter: at most `max` accepted hits per
/// `window`, per key.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    /// Builds a limiter allowing `max` hits per `window` for each key.
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Records an attempt for `key` and answers whether it is allowed.
    /// A refused attempt is **not** recorded — the client recovers as
    /// soon as the window slides past its accepted hits, rather than
    /// pushing its own lockout forward by retrying.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().expect("rate limiter mutex poisoned");
        if hits.len() > SWEEP_THRESHOLD {
            let window = self.window;
            hits.retain(|_, bucket| {
                while bucket
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= window)
                {
                    bucket.pop_front();
                }
                !bucket.is_empty()
            });
        }
        let bucket = hits.entry(key.to_string()).or_default();
        while bucket
            .front()
            .is_some_and(|t| now.duration_since(*t) >= self.window)
        {
            bucket.pop_front();
        }
        if bucket.len() as u32 >= self.max {
            return false;
        }
        bucket.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_applies_per_key() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
        assert!(
            !limiter.check("a"),
            "the third hit within the window is refused"
        );
        // Another key has its own budget.
        assert!(limiter.check("b"));
    }

    #[test]
    fn the_window_slides_and_refusals_do_not_extend_it() {
        let limiter = RateLimiter::new(1, Duration::from_millis(40));
        assert!(limiter.check("a"));
        assert!(!limiter.check("a"));
        std::thread::sleep(Duration::from_millis(60));
        // The accepted hit has aged out; the refusals in between did
        // not push the lockout forward.
        assert!(limiter.check("a"));
    }
}
