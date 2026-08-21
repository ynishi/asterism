//! Finding a name nothing alive answers to.
//!
//! Every rule that mints an entry needs one, and they all want the
//! same thing: the name somebody meant, or the nearest free form of
//! it. Numbering upward — `cut-01`, `cut-01 (2)`, `cut-01 (3)` — is
//! not clever, and that is most of the argument. Somebody reading the
//! line sees which entry the new one came out of, and somebody
//! renaming it types one thing.
//!
//! It is also why these rules practically never refuse: there is
//! always a higher number, and a name has no length to run into. The
//! refusal exists because the contract has it, and because a rule that
//! handed back a taken name would break the line's own uniqueness rule
//! at the far end of the work instead of here.
//!
//! # An entry on its way off is not holding its name
//!
//! A rule that takes an entry off the line in the same breath frees
//! the name it was answering to, and should be able to use it. The
//! line cannot say so yet — the removal has not landed — so the rule
//! says which entries it is taking off, and those stop counting.

use crate::domain::forge::model::strategy::StrategyError;
use crate::domain::forge::model::table::EntryStates;
use crate::domain::forge::model::value::{EntryId, Name};

/// The wanted name, or the first numbered form of it that nothing
/// alive on the line answers to.
///
/// `claimed` is what this same call has already handed out. Two
/// entries minted in one pass must not be given one name, and the line
/// cannot object yet because neither of them is on it.
pub(super) fn free(
    wanted: &Name,
    taken: &EntryStates,
    leaving: &[EntryId],
    claimed: &[Name],
) -> Result<Name, StrategyError> {
    let live = |candidate: &Name| {
        claimed.contains(candidate)
            || taken.iter().any(|(entry, state)| {
                state.alive && !leaving.contains(entry) && state.name.as_ref() == Some(candidate)
            })
    };

    if !live(wanted) {
        return Ok(wanted.clone());
    }
    for suffix in 2..=u16::MAX {
        let candidate = Name::new(format!("{wanted} ({suffix})"))
            .map_err(|_| StrategyError::NoNameLeft(wanted.clone()))?;
        if !live(&candidate) {
            return Ok(candidate);
        }
    }
    Err(StrategyError::NoNameLeft(wanted.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    use crate::domain::forge::model::table::EntryState;
    use crate::domain::forge::model::value::{Content, EntryId};

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn line(entries: &[(&str, bool)]) -> EntryStates {
        let mut states = EntryStates::new();
        for (called, alive) in entries {
            states.insert(
                EntryId::new(),
                EntryState {
                    alive: *alive,
                    content: Some(Content::from_uuid(Uuid::now_v7())),
                    name: Some(name(called)),
                },
            );
        }
        states
    }

    #[test]
    fn a_free_name_is_used_as_it_is() {
        let free = free(&name("cut-01"), &line(&[("other", true)]), &[], &[]).unwrap();

        assert_eq!(free.as_str(), "cut-01");
    }

    /// A rule taking an entry off the line in the same breath may use
    /// the name it was answering to.
    #[test]
    fn a_name_held_by_an_entry_on_its_way_off_is_free() {
        let states = line(&[("cut-01", true)]);
        let leaving: Vec<_> = states.keys().copied().collect();

        let free = free(&name("cut-01"), &states, &leaving, &[]).unwrap();

        assert_eq!(free.as_str(), "cut-01");
    }

    #[test]
    fn numbering_climbs_past_every_live_name() {
        let states = line(&[("cut-01", true), ("cut-01 (2)", true), ("cut-01 (3)", true)]);

        let free = free(&name("cut-01"), &states, &[], &[]).unwrap();

        assert_eq!(free.as_str(), "cut-01 (4)");
    }

    /// An entry that is off the line does not hold its name.
    #[test]
    fn a_name_only_a_dead_entry_answers_to_is_free() {
        let free = free(&name("cut-01"), &line(&[("cut-01", false)]), &[], &[]).unwrap();

        assert_eq!(free.as_str(), "cut-01");
    }

    #[test]
    fn a_name_this_call_already_handed_out_is_not_free_again() {
        let free = free(&name("cut-01"), &line(&[]), &[], &[name("cut-01")]).unwrap();

        assert_eq!(free.as_str(), "cut-01 (2)");
    }
}
