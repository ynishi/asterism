//! Line use cases — opening one, reading what is on it, and moving its
//! own description.
//!
//! ```text
//!   open / rename / set_strategy      writes the line's description
//!   get / states / strategies         reads
//! ```
//!
//! Nothing here writes to a line's history. A line moves when work
//! ends, and that is [`PursuitService::close`](super::PursuitService),
//! which is also the only place both logs are written at once.
//!
//! # What this service is allowed to decide
//!
//! Nothing. It loads, calls the model, and writes back what came out.
//! The two checks it does make are not judgements: that a line exists
//! before it is written to, and that the rule a caller names is one
//! this deployment carries. Both are lookups with one answer.
//!
//! # Choosing a rule is a real choice
//!
//! [`LineService::strategies`] exists so that it can be made. A line
//! settles collisions by a rule, and the rules differ in what happens
//! to somebody's work — so the list a person picks from is built from
//! the rules themselves, and every one of them says what it does.
//!
//! There is no fallback for a name nothing answers to. A line settled
//! by whatever rule happened to be available would be settled by a
//! rule nobody chose, and no record would say so.

use std::sync::Arc;

use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::Actors;
use crate::domain::forge::clock::Clock;
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::strategy::About;
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{LineId, Name, StrategyId};
use crate::domain::forge::strategies::Strategies;
use crate::error::DomainError;

/// Line use-case service.
pub struct LineService {
    lines: Arc<dyn Lines>,
    strategies: Arc<dyn Strategies>,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl LineService {
    /// Wires the service around its ports.
    pub fn new(
        lines: Arc<dyn Lines>,
        strategies: Arc<dyn Strategies>,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            lines,
            strategies,
            actors,
            clock,
        }
    }

    /// Opens a line, genesis and all.
    ///
    /// `strategy` is required rather than defaulted, because how a line
    /// settles collisions changes what happens to somebody's work, and
    /// a line that quietly got the first available answer would settle
    /// by a rule nobody chose. What to offer as the usual pick is a
    /// question for whoever assembles the list.
    pub async fn open(
        &self,
        name: Name,
        strategy: StrategyId,
        by: &AttributionContext,
    ) -> Result<Line, DomainError> {
        self.known(&strategy)?;
        let line = Line::open(name, strategy, self.act(by).await?);
        self.lines.open(&line).await?;
        Ok(line)
    }

    /// Reads a line back whole, history included.
    pub async fn get(&self, id: &LineId) -> Result<Line, DomainError> {
        self.lines
            .get(id)
            .await?
            .ok_or_else(|| DomainError::not_found("line", id))
    }

    /// Every line there is.
    ///
    /// Whole lines, which is affordable because a line is a
    /// repository: there are as many as somebody made on purpose.
    /// Which of them a person may see is not answered here — a line
    /// carries no owner, so scoping is for whoever knows what a person
    /// is.
    pub async fn list(&self) -> Result<Vec<Line>, DomainError> {
        self.lines.list().await
    }

    /// What is on the line: alive, under what name, at which content.
    ///
    /// Folded from the history on every call. There is no stored copy
    /// to be out of date with.
    pub async fn states(&self, id: &LineId) -> Result<EntryStates, DomainError> {
        Ok(self.get(id).await?.states())
    }

    /// Renames a line. Its history does not move.
    pub async fn rename(
        &self,
        id: &LineId,
        name: &Name,
        by: &AttributionContext,
    ) -> Result<(), DomainError> {
        self.get(id).await?;
        let act = self.act(by).await?;
        self.lines.rename(id, name, &act).await
    }

    /// Points a line at a different rule.
    ///
    /// Takes effect from here on. Divergences a previous rule already
    /// wrote stay exactly where they are — they are passes in a work
    /// log, and nothing rewrites those.
    pub async fn set_strategy(
        &self,
        id: &LineId,
        strategy: &StrategyId,
        by: &AttributionContext,
    ) -> Result<(), DomainError> {
        self.known(strategy)?;
        self.get(id).await?;
        let act = self.act(by).await?;
        self.lines.set_strategy(id, strategy, &act).await
    }

    /// Every rule a line can be pointed at, and what each one does.
    ///
    /// Built from the rules this deployment carries rather than from a
    /// list kept beside them, so it cannot describe a rule that is not
    /// there or miss one that is.
    ///
    /// Nothing here awaits anything — the rules are code, not rows.
    /// It is `async` all the same, because the guard that checks every
    /// verb of this layer for attribution reads only the asynchronous
    /// ones, and a verb outside that population is a verb nobody is
    /// checking.
    pub async fn strategies(&self) -> Vec<(StrategyId, About)> {
        self.strategies
            .all()
            .into_iter()
            .map(|rule| (rule.id(), rule.about()))
            .collect()
    }

    /// Refuses a rule this deployment does not carry.
    fn known(&self, strategy: &StrategyId) -> Result<(), DomainError> {
        if self.strategies.get(strategy).is_none() {
            return Err(DomainError::Validation(format!(
                "no rule named {strategy:?} — a line settles by one of the rules this instance \
                 carries"
            )));
        }
        Ok(())
    }

    /// Stamps an act: now, by whoever this write is from.
    async fn act(&self, by: &AttributionContext) -> Result<Act, DomainError> {
        Ok(Act::new(
            self.clock.now(),
            Actor::User(self.actors.resolve(by).await?),
        ))
    }
}
