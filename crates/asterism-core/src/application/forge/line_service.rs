//! Line use cases — opening one, reading what is on it, and moving its
//! own description.
//!
//! ```text
//!   open                              writes the whole line, genesis
//!                                       and history included
//!   rename / set_strategy             writes the line's description
//!   archive / reopen                  writes the line's standing
//!   get / states / strategies         reads
//!   discard                           reads both logs, writes neither
//!                                       — it takes them away
//! ```
//!
//! Nothing here writes to a line's history. A line moves when work
//! ends, and that is
//! [`PursuitService::close`](super::PursuitService).
//!
//! # The one verb that reads the other log
//!
//! [`LineService::discard`]. What a drop releases is the union of what
//! the line holds and what the work against it holds, so the call that
//! drops has to ask the pursuits — and it is the only one here that
//! does. Every other verb on this service can answer from a line
//! alone, which is why the dependency is worth naming rather than
//! assuming.
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

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::Actors;
use crate::domain::forge::clock::Clock;
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::discard::releases;
use crate::domain::forge::model::line::{Line, Standing};
use crate::domain::forge::model::strategy::About;
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{Content, LineId, Name, PursuitId, StrategyId};
use crate::domain::forge::pursuits::Pursuits;
use crate::domain::forge::strategies::Strategies;
use crate::error::DomainError;

/// Line use-case service.
pub struct LineService {
    lines: Arc<dyn Lines>,
    /// Read for one verb only — see [`LineService::discard`]. What a
    /// drop releases is the union of both logs, so the one call that
    /// drops has to have the other log in hand.
    pursuits: Arc<dyn Pursuits>,
    strategies: Arc<dyn Strategies>,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl LineService {
    /// Wires the service around its ports.
    pub fn new(
        lines: Arc<dyn Lines>,
        pursuits: Arc<dyn Pursuits>,
        strategies: Arc<dyn Strategies>,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            lines,
            pursuits,
            strategies,
            actors,
            clock,
        }
    }

    /// Opens a line, genesis and all — made here, then recorded.
    ///
    /// `strategy` is required rather than defaulted. What to offer as
    /// the usual pick is a question for whoever assembles the list.
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

    /// Reads a line back, refusing when there is no such line.
    pub async fn get(&self, id: &LineId) -> Result<Line, DomainError> {
        self.lines
            .get(id)
            .await?
            .ok_or_else(|| DomainError::not_found("line", id))
    }

    /// Every line there is, unscoped — see [`Lines::list`].
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
    /// wrote stay exactly where they are — they are rounds in a work
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

    /// Finishes with a line. Idempotent.
    ///
    /// Work against an archived line can still be abandoned, and
    /// nothing lands on it.
    ///
    /// This is also the only way to reach [`discard`](Self::discard),
    /// which is the model's rule rather than this service's: a line
    /// somebody is still working on is not a line anybody meant to
    /// delete, and the archive is where saying so happens.
    pub async fn archive(&self, id: &LineId, by: &AttributionContext) -> Result<(), DomainError> {
        self.set_standing(id, Standing::Archived, by).await
    }

    /// Takes a line back out of the archive. Idempotent.
    pub async fn reopen(&self, id: &LineId, by: &AttributionContext) -> Result<(), DomainError> {
        self.set_standing(id, Standing::Open, by).await
    }

    /// Drops the line, and answers with what that let go of.
    ///
    /// The line, its history and every pursuit against it, in one
    /// write. What comes back is every content the two logs were
    /// holding, which is what the layer that holds the bytes may now
    /// let go of — and answering with it is the point of the call: a
    /// caller that dropped a line and then had to work out what had
    /// been freed would be deriving it from a record that no longer
    /// exists.
    ///
    /// # What this does not say
    ///
    /// That any of it is now deletable. The layer below is where that
    /// is answered — which is why the set is handed over rather than
    /// acted on here.
    ///
    /// # Refusals
    ///
    /// The model's, unchanged: a line still open, or work against it
    /// that has not ended. Both come from
    /// [`releases`](crate::domain::forge::model::discard::releases), which
    /// is asked before anything is written.
    ///
    /// # Whose drop it was outlives nothing here
    ///
    /// This verb takes an attribution like every other write on this
    /// layer, and it is the one whose record does not survive to carry
    /// it: there is no row left to stamp. What the context still buys
    /// is the order — the caller is resolved to a handle, and the line
    /// is read, before anything is destroyed, so a caller nobody can
    /// attribute is refused while there is still something to refuse.
    pub async fn discard(
        &self,
        id: &LineId,
        by: &AttributionContext,
    ) -> Result<BTreeSet<Content>, DomainError> {
        self.act(by).await?;
        let line = self.get(id).await?;
        let work = self.pursuits.of_line(id).await?;

        // Decided here and written there, against the same list. The
        // port takes the ids so that work opened in between is a
        // refusal rather than an answer that quietly left something
        // out.
        let released = releases(&line, &work)?;
        let covering: Vec<PursuitId> = work.iter().map(|one| one.id()).collect();

        self.lines.discard(id, &covering).await?;
        Ok(released)
    }

    /// Every rule a line can be pointed at, and what each one does.
    ///
    /// Built from the rules this deployment carries rather than from a
    /// list kept beside them, so it cannot describe a rule that is not
    /// there or miss one that is.
    ///
    /// Nothing here awaits anything — the rules are code, not rows. It
    /// is `async` all the same, under the rule
    /// `tests/attribution_guards.rs` holds for every verb of this
    /// layer.
    pub async fn strategies(&self) -> Vec<(StrategyId, About)> {
        self.strategies
            .all()
            .into_iter()
            .map(|rule| (rule.id(), rule.about()))
            .collect()
    }

    /// The one write behind [`archive`](Self::archive) and
    /// [`reopen`](Self::reopen).
    ///
    /// The model moves the standing on a value; this moves it on a
    /// row. Nothing is decided in between — the two verbs differ by
    /// which standing they name, and the line is read first for the
    /// same reason every other write here reads it: so that a write to
    /// a line that is not there says so.
    async fn set_standing(
        &self,
        id: &LineId,
        standing: Standing,
        by: &AttributionContext,
    ) -> Result<(), DomainError> {
        self.get(id).await?;
        let act = self.act(by).await?;
        self.lines.set_standing(id, standing, &act).await
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
