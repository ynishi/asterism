//! Record what this work tried, then drop it and take the line's
//! answer.
//!
//! ```text
//!   before      line  E = theirs          work  E = mine
//!
//!   writes      add(E', was[E])           fork the entry as it was
//!               replace(E', mine)         apply this work's change there
//!               remove(E')                and drop it
//!               replace(E, theirs)        taking the line's answer for E
//!
//!   after       line  E = theirs          E' is off the line, and readable
//! ```
//!
//! [`MainlineFirst`](super::MainlineFirst) with one more operation, and
//! the operation is the whole difference: the forked entry is taken
//! off the line rather than left on it.
//!
//! # Why fork at all, if it is only going to be removed
//!
//! Because otherwise nothing says what was tried. Dropping a change by
//! never writing it leaves a line that looks as though nobody ever
//! wanted anything else — and this layer exists to be able to answer
//! what was considered and what became of it. The fork, the change and
//! the removal are that answer, and they cost three operations in a
//! log that keeps everything anyway.
//!
//! What was dropped stays reachable. Wanting it back later is an
//! ordinary add of the entry that was removed.

use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::strategy::{About, Divergence, Strategy, StrategyError};
use crate::domain::forge::model::value::{Name, StrategyId};
use crate::domain::forge::strategies::naming::free;

/// Records this work's version as a removed entry and takes the
/// line's.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardMine;

impl DiscardMine {
    /// What this rule is called where a line records it.
    pub const NAME: &'static str = "discard-mine";
}

impl Strategy for DiscardMine {
    fn id(&self) -> StrategyId {
        StrategyId::new(Self::NAME).expect("a literal name is not blank")
    }

    fn about(&self) -> About {
        About {
            name: "Keep the line's version, and drop this one".into(),
            summary: "What is on the line stays. This work's version is written down and then \
                      taken off, so what was tried stays readable without staying on the line."
                .into(),
        }
    }

    fn resolve(&self, at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError> {
        let mut ops = Vec::new();
        let mut claimed: Vec<Name> = Vec::new();

        for entry in at.entries() {
            let Some(row) = at.request().get(&entry) else {
                continue;
            };
            let Some(mine) = row.content() else {
                continue;
            };
            let Some(theirs) = at.taken().get(&entry).and_then(|state| state.content) else {
                continue;
            };

            let wanted = match row.name() {
                Some(name) => name.clone(),
                None => match at.taken().get(&entry).and_then(|state| state.name.clone()) {
                    Some(name) => name,
                    None => continue,
                },
            };
            let called = free(&wanted, at.taken(), &[], &claimed)?;
            claimed.push(called.clone());

            let started = at
                .was()
                .get(&entry)
                .and_then(|state| state.content)
                .unwrap_or(mine);

            let fork = Op::add(started, called);
            let forked = fork.entry();
            ops.push(fork);
            ops.push(Op::replace(forked, mine));
            ops.push(Op::remove(forked));
            ops.push(Op::replace(entry, theirs));
        }

        Ok(ops)
    }
}
