//! Keep what the line says; carry this work's version onto an entry of
//! its own.
//!
//! ```text
//!   before      line  E = theirs          work  E = mine
//!
//!   writes      add(E', was[E])           fork the entry as it was
//!               replace(E', mine)         apply this work's change there
//!               replace(E, theirs)        and take the line's answer for E
//!
//!   after       line  E = theirs, E' = mine
//! ```
//!
//! Three ordinary operations — the ones somebody resolving by hand
//! would write. Nothing about them is special to resolving: an entry
//! is forked, a change is applied to it, and a value is adopted for the
//! original.
//!
//! # Why the third operation is there
//!
//! Without it the work still says `E = mine`, and closing would put
//! that on the line — the opposite of what this rule is called. Taking
//! the line's value for `E` is what "keep the line's version" actually
//! is, said in the only way work can say anything.
//!
//! It is a claim like any other, and it stays true only as long as it
//! is true. If the line moves `E` again, this work says the older value
//! and collides again, and is resolved again. That is what resolving
//! against a line that keeps moving means, and it is why the third
//! operation cannot be left out and cannot be silent.
//!
//! # Why the fork starts at the old value
//!
//! `add(E', was[E])` then `replace(E', mine)` reaches the same place as
//! `add(E', mine)` in one step. What it also does is record where the
//! new entry came from: it was this entry, before this work changed it.
//! The value is nowhere else by the time anybody reads the log.

use crate::domain::forge::model::op::Op;
use crate::domain::forge::model::strategy::{About, Divergence, Strategy, StrategyError};
use crate::domain::forge::model::value::{Name, StrategyId};
use crate::domain::forge::strategies::naming::{claimed_by, free};

/// Keeps the line's version and carries this work's onto a new entry.
#[derive(Debug, Clone, Copy, Default)]
pub struct MainlineFirst;

impl MainlineFirst {
    /// What this rule is called where a line records it.
    pub const NAME: &'static str = "mainline-first";
}

impl Strategy for MainlineFirst {
    fn id(&self) -> StrategyId {
        StrategyId::new(Self::NAME).expect("a literal name is not blank")
    }

    fn about(&self) -> About {
        About {
            name: "Keep the line's version".into(),
            summary: "What is on the line stays. This work's version is carried onto a new entry \
                      beside it, under a numbered name, and both are kept."
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
            ops.push(Op::replace(entry, theirs));
        }

        Ok(ops)
    }
}
