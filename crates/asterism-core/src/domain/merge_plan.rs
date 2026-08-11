//! `MergePlan` — a person's ruling that a set of rows is one thing, and
//! which of them survives it.
//!
//! The automatic half of duplicate resolution folds a **detected pair**:
//! two rows hold the same bytes, and one of them is folded into the
//! other. This is the other entry point. Somebody looked at several rows
//! and decided they are one thing — which needs no fingerprint to agree,
//! no queue row to have been raised, and is not bound by the exclusions
//! that stop an *automatic* fold (a person looking at two rows can
//! see what the rule was protecting).
//!
//! # Why not part of `duplicate_conflict`
//!
//! That module is the **question** a fingerprint match raises and the
//! answer somebody gives it — every value in it is keyed to a detected
//! pair. A plan reaches the merge verb without any of that: it may name
//! five rows, it may name rows no fingerprint ever compared, and there
//! may be no queue row anywhere. Putting it there would make the
//! module's own doc false, and would suggest to the next reader that a
//! merge is a conflict resolution with more members — which is exactly
//! the thing that is not true about it.
//!
//! # What this type does *not* do
//!
//! It does not look at the database. Whether a row exists, is already a
//! headstone, or has been thrown out is state, and state can change
//! between the moment a person clicks and the moment the transaction
//! runs — so it is re-read inside that transaction
//! ([`AssetRepository::merge_into`](crate::domain::repository::AssetRepository::merge_into)),
//! never here. What is checkable without a database is whether the
//! *declaration* is a declaration at all, and that is the whole job of
//! this type.

use std::collections::BTreeSet;

use crate::domain::value::AssetId;
use crate::error::DomainError;

/// A checked declaration: these rows are one thing, and this is the one
/// that stays.
///
/// Construction is the check ([`declare`](Self::declare)); there is no
/// way to build one that has not been through it, and no setter that
/// could take it back out of the checked state afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    keeper: AssetId,
    discard: Vec<AssetId>,
}

impl MergePlan {
    /// Checks a ruling and returns it, or says which part of it does not
    /// add up.
    ///
    /// # Why `members` is a parameter
    ///
    /// `keeper` and `discard` alone are a complete instruction, and that
    /// is the problem. A person looking at five rows who ticks three of
    /// them has said one of two things — "the other two are separate
    /// things" or "I did not notice the other two" — and the two produce
    /// the *same* call. Naming the whole set the ruling was made over
    /// splits them: a plan whose members are exactly the keeper plus the
    /// discarded rows is a ruling about all of them, and anything else
    /// is a caller who lost track of a row between the screen and the
    /// verb.
    ///
    /// This is also the check that cannot be added later. The rows a
    /// person saw stop being knowable the moment the call is made, and
    /// no amount of care inside the transaction can reconstruct them.
    ///
    /// # What is refused
    ///
    /// - an empty `discard` — there is no second row, so there is
    ///   nothing to fold and the call means nothing;
    /// - the keeper listed among the discarded — a row cannot be folded
    ///   into itself, and a ruling that says so is not a ruling that
    ///   went wrong halfway, it is one that was never coherent;
    /// - a repeated id in `discard` — the same row folded twice; the
    ///   counts a caller gets back would describe a set that does not
    ///   exist;
    /// - `members` that is not exactly the keeper plus the discarded
    ///   rows, each appearing once — including a `members` that repeats
    ///   an id, since "how many rows were ruled over" is the number the
    ///   declaration exists to state.
    ///
    /// Each refusal names the ids involved. A caller that is handed
    /// "invalid merge plan" has to go and diff two lists by hand to find
    /// out which of five rows went missing.
    ///
    /// # Order
    ///
    /// `discard` keeps the caller's order, and the merge folds in it —
    /// see
    /// [`merge_into`](crate::domain::repository::AssetRepository::merge_into)
    /// for why that order is a decision and whose it is.
    pub fn declare(
        keeper: AssetId,
        discard: Vec<AssetId>,
        members: &[AssetId],
    ) -> Result<Self, DomainError> {
        if discard.is_empty() {
            return Err(DomainError::Validation(format!(
                "a merge into {keeper} names no row to fold into it: \
                 a merge of one row is not a merge"
            )));
        }
        let mut planned = BTreeSet::from([keeper]);
        for id in &discard {
            if *id == keeper {
                return Err(DomainError::Validation(format!(
                    "the keeper {keeper} is also listed among the rows to fold into it"
                )));
            }
            if !planned.insert(*id) {
                return Err(DomainError::Validation(format!(
                    "the row {id} is listed twice among the rows to fold into {keeper}"
                )));
            }
        }

        let declared: BTreeSet<AssetId> = members.iter().copied().collect();
        if declared.len() != members.len() {
            return Err(DomainError::Validation(format!(
                "the declared members of the merge into {keeper} list the same row \
                 more than once, so they do not say how many rows were ruled over"
            )));
        }
        if declared != planned {
            let unaccounted = id_list(declared.difference(&planned));
            let undeclared = id_list(planned.difference(&declared));
            return Err(DomainError::Validation(format!(
                "the merge into {keeper} does not account for every declared member: \
                 declared but neither kept nor folded: [{unaccounted}]; \
                 kept or folded but not declared as a member: [{undeclared}]"
            )));
        }

        Ok(Self { keeper, discard })
    }

    /// The row that survives the merge.
    pub fn keeper(&self) -> AssetId {
        self.keeper
    }

    /// The rows folded into the keeper, in the order they are folded.
    pub fn discard(&self) -> &[AssetId] {
        &self.discard
    }
}

/// Ids as a comma-separated list, for a refusal that has to name them.
fn id_list<'a>(ids: impl Iterator<Item = &'a AssetId>) -> String {
    ids.map(AssetId::to_string).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ruling_over_the_whole_set_is_accepted_and_keeps_its_order() {
        let keeper = AssetId::new();
        let (first, second) = (AssetId::new(), AssetId::new());

        let plan = MergePlan::declare(keeper, vec![second, first], &[first, keeper, second])
            .expect("the members are exactly the keeper and the two folded rows");

        assert_eq!(plan.keeper(), keeper);
        // Not sorted, not re-ordered to match `members`: the sequence
        // the caller gave is the sequence the merge folds in, and
        // `register_note` paragraphs come out in it.
        assert_eq!(plan.discard(), [second, first]);
    }

    #[test]
    fn a_merge_needs_something_to_fold() {
        let keeper = AssetId::new();
        let refused = MergePlan::declare(keeper, vec![], &[keeper]).unwrap_err();
        assert!(
            refused.to_string().contains("no row to fold"),
            "the refusal should say what is missing: {refused}"
        );
    }

    #[test]
    fn a_row_cannot_be_folded_into_itself_or_folded_twice() {
        let keeper = AssetId::new();
        let other = AssetId::new();

        let itself = MergePlan::declare(keeper, vec![keeper], &[keeper]).unwrap_err();
        assert!(
            itself.to_string().contains(&keeper.to_string()),
            "the refusal should name the row: {itself}"
        );

        let twice = MergePlan::declare(keeper, vec![other, other], &[keeper, other]).unwrap_err();
        assert!(
            twice.to_string().contains("listed twice"),
            "the refusal should say the row appears twice: {twice}"
        );
    }

    /// The check the whole parameter exists for, from both sides: a row
    /// the person saw and ruled on nothing about, and a row the ruling
    /// names that the person never declared having looked at.
    #[test]
    fn the_members_have_to_be_exactly_the_keeper_and_the_folded_rows() {
        let keeper = AssetId::new();
        let folded = AssetId::new();
        let bystander = AssetId::new();

        // Five on screen, three ticked: the two left over are the case
        // this refuses, because "leave them alone" and "I missed them"
        // are the same call without it.
        let leftover =
            MergePlan::declare(keeper, vec![folded], &[keeper, folded, bystander]).unwrap_err();
        let leftover = leftover.to_string();
        assert!(
            leftover.contains(&bystander.to_string()),
            "the refusal should name the row nobody ruled on: {leftover}"
        );

        // And the other way: a row folded without having been declared
        // a member is a row the person is not on record as having seen.
        let undeclared =
            MergePlan::declare(keeper, vec![folded, bystander], &[keeper, folded]).unwrap_err();
        let undeclared = undeclared.to_string();
        assert!(
            undeclared.contains(&bystander.to_string()),
            "the refusal should name the undeclared row: {undeclared}"
        );

        // A doubled member is not a count of anything.
        let doubled =
            MergePlan::declare(keeper, vec![folded], &[keeper, folded, folded]).unwrap_err();
        assert!(
            doubled.to_string().contains("more than once"),
            "the refusal should say the members repeat: {doubled}"
        );
    }
}
