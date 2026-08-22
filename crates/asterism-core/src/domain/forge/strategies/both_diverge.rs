//! Put both versions on entries of their own, and take the old one
//! off.
//!
//! ```text
//!   before      line  E "cut-01" = theirs        work  E = mine
//!
//!   writes      add(E',  theirs, "cut-01")       the line's version, kept
//!               add(E'', mine,   "cut-01 (2)")   this work's, beside it
//!               remove(E)                        and the entry they argued over goes
//!
//!   after       line  E' "cut-01" = theirs, E'' "cut-01 (2)" = mine
//! ```
//!
//! Neither side inherits the original entry.
//! [`MainlineFirst`](super::MainlineFirst) leaves the line's version
//! standing on it; here it is taken off and both versions arrive on
//! entries of their own — the disagreement is the point, both are
//! candidates, and the entry they were arguing over is not one of
//! them.
//!
//! The removal is what makes that true. Leaving `E` on the line would
//! put a third thing there, holding whichever value won by accident.
//!
//! Nothing is lost by removing it — taking an entry off a line is a
//! change point that says so, and what it held stays readable.
//!
//! The name still has to go somewhere, and it goes to the line's
//! version: the line's is named first, so this work's is the one that
//! ends up numbered. That is the whole of the difference from
//! [`MineFirst`](super::MineFirst), which writes the same three
//! operations the other way round.

use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::strategy::{About, Divergence, Strategy, StrategyError};
use crate::domain::forge::model::value::{Name, StrategyId};
use crate::domain::forge::strategies::naming::{claimed_by, free};

/// Puts both versions on new entries and takes the old one off.
#[derive(Debug, Clone, Copy, Default)]
pub struct BothDiverge;

impl BothDiverge {
    /// What this rule is called where a line records it.
    pub const NAME: &'static str = "both-diverge";
}

impl Strategy for BothDiverge {
    fn id(&self) -> StrategyId {
        StrategyId::new(Self::NAME).expect("a literal name is not blank")
    }

    fn about(&self) -> About {
        About {
            name: "Keep both, side by side".into(),
            summary: "Both versions arrive as new entries, and the entry they disagreed about \
                      is taken off the line. The line's version keeps the name it answered to; \
                      this work's is kept beside it under a numbered one."
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

            // The entry is going off the line in the same breath, so
            // the name it answers to is free for one of the two.
            let for_theirs = free(&wanted, at.taken(), &[entry], &claimed)?;
            claimed.push(for_theirs.clone());
            let for_mine = free(&wanted, at.taken(), &[entry], &claimed)?;
            claimed.push(for_mine.clone());

            ops.push(Op::add(theirs, for_theirs));
            ops.push(Op::add(mine, for_mine));
            ops.push(Op::remove(entry));
        }

        Ok(ops)
    }
}
