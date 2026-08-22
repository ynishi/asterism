//! Mint nothing; leave it to whoever is working.
//!
//! A line set to this one never has entries created on its behalf. A
//! collision is reported, the work stays open carrying it, and
//! somebody writes the operations that answer it — keep the line's
//! version, take theirs, put both on under names they chose.
//!
//! # Why this is a rule and not the absence of one
//!
//! "This line settles collisions by hand" is a decision somebody made,
//! and it reads as one here: a line points at it the way it points at
//! any other rule, and the code that runs rules has no case for a line
//! that has none. Spelled as an empty setting instead, it would be
//! indistinguishable from a line nobody has configured, and every
//! caller would carry a branch for it.
//!
//! # Returning nothing is not failing
//!
//! A refusal is a rule saying it could not decide, and this one has
//! decided: nothing is to be written. So it answers with an empty
//! list, no round is written, and the collision stays exactly where a
//! person can see it — which is the state they then act on.

use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::strategy::{About, Divergence, Strategy, StrategyError};
use crate::domain::forge::model::value::StrategyId;

/// Writes nothing, whatever collides.
#[derive(Debug, Clone, Copy, Default)]
pub struct ByHand;

impl ByHand {
    /// What this rule is called where a line records it.
    pub const NAME: &'static str = "by-hand";
}

impl Strategy for ByHand {
    fn id(&self) -> StrategyId {
        StrategyId::new(Self::NAME).expect("a literal name is not blank")
    }

    fn about(&self) -> About {
        About {
            name: "Leave it to me".into(),
            summary: "Nothing is created automatically. A collision is reported and waits for \
                      somebody to decide what happens to it."
                .into(),
        }
    }

    fn resolve(&self, _at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError> {
        Ok(Vec::new())
    }
}
