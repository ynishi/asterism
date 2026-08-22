//! How a line settles a collision — the contract, and not the rule.
//!
//! ```text
//!   collision ──► Strategy::resolve ──► Ok(complete ops)  ──► a round carries them
//!                       │                                     and the collision is gone
//!                       └──────────────► Err              ──► no op is written at all
//! ```
//!
//! When work moves an axis the line moved first, the answer is not to
//! pick a winner. It is to put a second entry on the line beside the
//! first, so both candidates survive and the history stays one chain —
//! and then which one lives is an ordinary later choice rather than a
//! decision somebody is cornered into now.
//!
//! # Nothing here says how
//!
//! There is more than one sensible way to do that: keep the line's
//! side and mint a new entry for the work's, keep the work's side and
//! move the line's, mint new entries for both. Which one a line uses
//! is a setting, and what the divergent entry ends up being called
//! follows from it. **None of that is the model's to decide**, so this
//! module holds the contract and no rule that satisfies it.
//!
//! What the model does require is one sentence: a strategy decides an
//! id and a name, and returns them as operations complete enough to go
//! straight into a round. There is no half-written entry, no entry that
//! gets a name later, and no path that writes some operations and then
//! fails — a refusal writes nothing, and the work stays open carrying
//! its collision.
//!
//! # Naming belongs to the rule
//!
//! A divergent entry is born named, because an entry with no name is a
//! shape only this one path would have, and every rule about names
//! would need an exception for it. What it is called depends on how
//! the split was made, so the rule that split it is what names it.
//!
//! A rule that cannot decide a name on its own — because the answer
//! depends on something outside the forge — holds whatever it needs to
//! ask, and whoever assembles it supplies that. The model does not
//! see it either way.
//!
//! # Refusing is a real outcome
//!
//! [`StrategyError`] has two cases: no name is available, and the rule
//! cannot decide. Both leave the work exactly
//! as it was, open and colliding, which is a state somebody can act on
//! by hand. What they must not leave is half a divergence.
//!
//! # Doing nothing is a rule, not a missing one
//!
//! A line where nobody wants entries minted automatically points at a
//! rule that returns no operations. That is a rule like any other, and
//! it keeps "this line does not settle collisions by itself" from
//! being a flag every caller has to branch on.
//!
//! # A rule is picked, so it says what it is
//!
//! Choosing how a line settles is somebody's decision, and a list of
//! slugs is not something anybody can choose from. Every rule carries
//! an [`About`] — what it is called, and what it does to which side —
//! and carries it itself, so the list a person reads is built out of
//! the rules that exist rather than out of a table of labels that has
//! to be kept in step with them.
//!
//! What a line records is the [`id`](Strategy::id), never the label.
//! A rule can be renamed, translated or reworded without moving a
//! single line off it.

use crate::domain::forge::model::change::Collision;
use crate::domain::forge::model::op::{Op, Rows};
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{Name, StrategyId};

/// What a rule is given to work from.
///
/// Four things, because writing the operations a person would have
/// written takes all four: what collided, what this work is saying,
/// what the line says now, and what the line said when the work began.
/// The last is the one that is easy to leave out and impossible to
/// recover later — an entry forked out of a collision starts as the
/// entry was before anybody touched it, and by resolution time that
/// value is nowhere else.
pub struct Divergence<'a> {
    collisions: &'a [Collision],
    request: &'a Rows,
    taken: &'a EntryStates,
    was: &'a EntryStates,
}

impl<'a> Divergence<'a> {
    /// Gathers what a rule needs.
    pub fn new(
        collisions: &'a [Collision],
        request: &'a Rows,
        taken: &'a EntryStates,
        was: &'a EntryStates,
    ) -> Self {
        Self {
            collisions,
            request,
            taken,
            was,
        }
    }

    /// The collisions this rule is being asked about.
    pub fn collisions(&self) -> &[Collision] {
        self.collisions
    }

    /// Every entry the collisions are about, each once, however many
    /// of its axes collided.
    pub fn entries(&self) -> Vec<crate::domain::forge::model::value::EntryId> {
        let mut entries: Vec<_> = self.collisions.iter().map(|found| found.entry).collect();
        entries.sort();
        entries.dedup();
        entries
    }

    /// What this work is saying, folded.
    pub fn request(&self) -> &Rows {
        self.request
    }

    /// What the line carries now.
    pub fn taken(&self) -> &EntryStates {
        self.taken
    }

    /// What the line carried when this work was cut from it.
    pub fn was(&self) -> &EntryStates {
        self.was
    }
}

/// Why a rule wrote nothing.
///
/// Both cases are the rule saying it will not guess. Neither is the
/// line refusing anything — it has not been asked yet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StrategyError {
    /// Every name the rule would have used is taken by something alive
    /// on the line, and it has no other to offer.
    #[error("no name is left for a diverging entry (wanted {0:?})")]
    NoNameLeft(Name),
    /// The rule could not decide, for a reason of its own.
    ///
    /// Carries what it said. A rule is written outside the model, so
    /// the model cannot enumerate its reasons — what it can do is
    /// refuse to let one fail silently.
    #[error("the strategy could not decide: {0}")]
    Undecidable(String),
}

/// What a rule says about itself, for whoever is choosing one.
///
/// Somebody picks the rule a line settles by, and picking from a list
/// of slugs is picking blind. A rule therefore carries what a person
/// needs to read to choose it, and carries it itself — a table of
/// labels kept somewhere else would be a second list to keep in step
/// with the rules that actually exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    /// What it is called where somebody chooses it.
    ///
    /// Display only. What a line stores is the
    /// [`id`](Strategy::id), so renaming a rule moves no line off it
    /// — the two are separate for exactly that reason.
    pub name: String,
    /// What it does, in the words of whoever has to live with the
    /// result: which side stays put, what the other side becomes.
    pub summary: String,
}

/// One rule for turning a collision into a divergence.
///
/// Implemented outside the model. The model states what a rule owes
/// and never picks one: a line points at a rule by [`StrategyId`], and
/// whoever runs the forge decides what that name resolves to.
pub trait Strategy: Send + Sync {
    /// Which rule this is. The value a line stores to point here.
    ///
    /// Stable, and not a label: it is what a line has already recorded,
    /// so it survives the rule being renamed and outlives any list of
    /// rules a given deployment happens to offer.
    fn id(&self) -> StrategyId;

    /// What to show somebody choosing between rules.
    fn about(&self) -> About;

    /// Turns the collisions into the operations somebody would have
    /// written by hand to resolve them.
    ///
    /// Ordinary operations, in the four verbs everybody else uses.
    /// There is no special vocabulary for resolving, because resolving
    /// is not a special act — it is work deciding something, and the
    /// record of what it decided is the operations it wrote.
    ///
    /// Returning an empty list is allowed and means "nothing to
    /// write": the rule that leaves collisions standing for a person
    /// to deal with is spelled that way rather than as a missing rule.
    ///
    /// What comes back is checked — the caller folds it into what the
    /// work says and refuses the rule if the collisions it was asked
    /// about are still there.
    fn resolve(&self, at: &Divergence<'_>) -> Result<Vec<Op>, StrategyError>;
}
