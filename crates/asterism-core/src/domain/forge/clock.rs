//! What time it is.
//!
//! A port for something every other service in this codebase reads off
//! the system clock directly, and the difference is what a timestamp
//! means once it is written.
//!
//! ```text
//!   elsewhere      a row's updated_at      convenient, overwritten by the next write
//!   here           an act's `at`           evidence, kept forever, ordered by nothing
//! ```
//!
//! A forge node does not move once recorded, so a wrong time is wrong
//! for good. And the chain — not the clock — is what orders a history,
//! so a wrong time breaks nothing: no fold reads it, no comparison
//! depends on it, nothing fails. What it does instead is answer a
//! question incorrectly, quietly, for as long as the record exists.
//!
//! That question is not incidental. A record of a selection has to say
//! what was chosen, out of what, by whom, **and when** — those are the
//! terms on which this layer is worth building at all. A value that
//! nothing verifies and nothing depends on is exactly the value that
//! has to be pinned by a test, and pinning it means the service asking
//! something rather than reading a global.
//!
//! # Not for the model
//!
//! Nothing under [`model`](crate::domain::forge::model) takes a clock.
//! A decision is handed an [`Act`](crate::domain::forge::model::act::Act)
//! that already carries its time, which is what keeps deciding
//! reproducible: the same line, the same work and the same act give the
//! same answer, today and in a test. This port belongs to the services
//! that assemble an act, and to nothing below them.
//!
//! # Not for the rest of the codebase
//!
//! The other services keep calling the system clock, and should. Their
//! timestamps say when a row was last touched, a row that will be
//! touched again — turning those into evidence would be paying this
//! cost everywhere for a claim only this layer makes.

use chrono::{DateTime, Utc};

/// Says what time it is.
///
/// Synchronous: reading a clock is not I/O, and making it `async`
/// would put an await in every act a service assembles for no
/// question anybody is waiting on an answer to.
pub trait Clock: Send + Sync {
    /// Now, in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// The clock a running system uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the port: a test can say what time it is, and then
    /// assert on what got recorded.
    #[test]
    fn a_fixed_clock_answers_the_same_thing_every_time() {
        use chrono::TimeZone;

        struct Fixed(DateTime<Utc>);

        impl Clock for Fixed {
            fn now(&self) -> DateTime<Utc> {
                self.0
            }
        }

        let at = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let clock = Fixed(at);

        assert_eq!(clock.now(), at);
        assert_eq!(clock.now(), clock.now());
    }
}
