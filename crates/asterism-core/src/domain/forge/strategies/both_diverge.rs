//! Put both versions on entries of their own, and take the old one
//! off.
//!
//! ```text
//!   before      line  E = theirs          work  E = mine
//!
//!   writes      add(E', theirs)           the line's version, on its own entry
//!               add(E'', mine)            this work's, on its own
//!               remove(E)                 and the entry they disagreed about goes
//!
//!   after       line  E' = theirs, E'' = mine        E is off the line
//! ```
//!
//! Neither side inherits the original. Where the other two rules make
//! one version the continuation of what was there, this one says the
//! disagreement is the point: both are candidates, and the entry they
//! were arguing over is not one of them.
//!
//! The removal is what makes that true. Leaving `E` on the line would
//! put a third thing there, holding whichever value won by accident.
//!
//! Nothing is lost by removing it — taking an entry off a line is a
//! change point that says so, and what it held stays readable.

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
            summary: "Both versions arrive as new entries under numbered names, and the entry \
                      they disagreed about is taken off the line."
                .into(),
        }
    }

    fn resolve(&self, at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError> {
        let mut ops = Vec::new();
        // Seeded with what this work is already asking for: a
        // previous resolution's entries are in the request and not yet
        // on the line, so the line cannot say their names are taken.
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
