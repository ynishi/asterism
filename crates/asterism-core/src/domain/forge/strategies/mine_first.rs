//! Keep this work's version under the name it had; move the line's
//! aside.
//!
//! ```text
//!   before      line  E "cut-01" = theirs        work  E = mine
//!
//!   writes      add(E',  theirs, "cut-01 (2)")   the line's version, kept
//!               add(E'', mine,   "cut-01")       this work's, under the name
//!               remove(E)                        and the entry they argued over goes
//!
//!   after       line  E'' "cut-01" = mine, E' "cut-01 (2)" = theirs
//! ```
//!
//! # Why the original entry cannot simply be kept
//!
//! The obvious shape — leave `E` saying `mine`, put `theirs` on a new
//! entry — does not work, and finding out why is worth more than the
//! rule is.
//!
//! A collision is an axis this work writes that the line moved since
//! the work was cut. Nothing about that mentions intent: as long as
//! the work is still writing `E`'s content, the disagreement is live,
//! and writing `mine` a second time changes nothing because a fold
//! keeps the last value and not the arguments for it. **The only way
//! to stop colliding over an axis is to stop writing it.**
//!
//! So a rule that wants this work's version to win cannot win *in
//! place*. It wins by taking the contested entry off the line and
//! putting the value on an entry nobody is arguing about — which is
//! what a person would have to do too, for exactly the same reason.
//!
//! What carries across is the name: `E''` answers to what `E`
//! answered to, so everything that looked the entry up by name still
//! finds this work's version. That is the whole of what "mine first"
//! can mean here, and it is worth being honest that it is a rename
//! rather than an override.
//!
//! # Compared to letting both start over
//!
//! [`BothDiverge`](super::BothDiverge) writes the same three
//! operations and hands the original name the other way. The
//! difference is the answer to "what does `cut-01` mean now" — here it
//! means this work's version, there it means the line's, and this
//! work's is the one that ends up numbered.

use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::strategy::{About, Divergence, Strategy, StrategyError};
use crate::domain::forge::model::value::{Name, StrategyId};
use crate::domain::forge::strategies::naming::{claimed_by, free};

/// Keeps this work's version under the contested name and moves the
/// line's aside.
#[derive(Debug, Clone, Copy, Default)]
pub struct MineFirst;

impl MineFirst {
    /// What this rule is called where a line records it.
    pub const NAME: &'static str = "mine-first";
}

impl Strategy for MineFirst {
    fn id(&self) -> StrategyId {
        StrategyId::new(Self::NAME).expect("a literal name is not blank")
    }

    fn about(&self) -> About {
        About {
            name: "Keep this work's version".into(),
            summary: "This work's version keeps the name. What was on the line is kept beside it \
                      under a numbered one, and the entry they disagreed about is taken off."
                .into(),
        }
    }

    fn resolve(&self, at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError> {
        let mut ops = Vec::new();
        let mut claimed: Vec<Name> = claimed_by(at.request());

        for entry in at.entries() {
            let Some(row) = at.request().get(&entry) else {
                continue;
            };
            let Some(mine) = row.content() else {
                continue;
            };
            let Some(state) = at.taken().get(&entry) else {
                continue;
            };
            let Some(theirs) = state.content else {
                continue;
            };
            let Some(wanted) = state.name.clone().or_else(|| row.name().cloned()) else {
                continue;
            };

            // This work's version is named first, so it gets the plain
            // name and the line's takes the numbered one. `entry` is
            // going off the line in the same breath, so the name it
            // answers to is available to be handed on.
            let for_mine = free(&wanted, at.taken(), &[entry], &claimed)?;
            claimed.push(for_mine.clone());
            let for_theirs = free(&wanted, at.taken(), &[entry], &claimed)?;
            claimed.push(for_theirs.clone());

            ops.push(Op::add(theirs, for_theirs));
            ops.push(Op::add(mine, for_mine));
            ops.push(Op::remove(entry));
        }

        Ok(ops)
    }
}
