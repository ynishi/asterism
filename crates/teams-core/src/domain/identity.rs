//! `identity` — who exists, who belongs to a team, and who may do what.
//!
//! Three separations carry this module, all from #83 §1:
//!
//! - **Membership is not identity.** A [`User`] exists independently of
//!   any team; a [`Membership`] is one `(user, team, role)` row and a
//!   team's set of them is what every invariant is evaluated over.
//! - **The operator is not a member.** [`InstanceOperator`] is the
//!   env/CLI bootstrap identity and lives *outside* the membership
//!   set — owning a team is an explicit membership row like anyone
//!   else's, and when the operator acts inside a team the ledger stamp
//!   says so ([`LedgerActor::Operator`]): an operator action is never
//!   disguised as a member's.
//! - **Authorization reads state, not history.** The decision functions
//!   here take the current membership set ([`TeamRoster`]); the ledger
//!   records what was decided and is never consulted to decide.
//!
//! The one absolute invariant is the last-owner rule: every team has
//! ≥1 owner at all times, so the last owner cannot leave, be removed,
//! or self-demote (the GitHub/GitLab/Gitea rule). It is enforced as
//! `check_*` methods on [`TeamRoster`] so every write path asks the
//! same three lines instead of re-deriving them.

use crate::error::DomainError;
use uuid::Uuid;

/// A person known to the instance — the identity teams are built from.
///
/// `user_id` is immutable (private field, read accessor only): it is
/// what memberships, locators and ledger stamps refer to, and an id
/// that could be reassigned would silently re-attribute all of them.
/// `display_name` is the mutable half — renaming is an ordinary edit
/// ([`User::rename`]) precisely because the ledger records the name
/// *at write time* ([`ActorStamp`]), so a rename never rewrites what
/// past entries say.
///
/// Credentials are deliberately absent: they live behind
/// [`port::auth`](crate::port::auth), so the auth provider can be
/// swapped without touching this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    user_id: Uuid,
    display_name: String,
}

impl User {
    /// Builds a user. The id is taken rather than minted — this crate
    /// does not decide id-generation policy, and rehydration from
    /// storage passes back the id a row already carries.
    ///
    /// A blank display name is refused: the stamp the ledger keeps
    /// would then say nothing about who acted.
    pub fn new(user_id: Uuid, display_name: impl Into<String>) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::Validation(
                "display_name is blank; the ledger stamps names at write time, \
                 so a nameless user would leave unreadable history"
                    .into(),
            ));
        }
        Ok(Self {
            user_id,
            display_name,
        })
    }

    /// The immutable id — what everything else refers to.
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// The current display name.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Changes the display name. Past ledger entries keep the name
    /// that was current when they were written — that is the
    /// [`ActorStamp`] contract, not this method's concern.
    pub fn rename(&mut self, display_name: impl Into<String>) -> Result<(), DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::Validation("display_name is blank".into()));
        }
        self.display_name = display_name;
        Ok(())
    }

    /// The stamp the ledger would record for this user acting *now* —
    /// id plus the name as it currently reads.
    pub fn stamp(&self) -> ActorStamp {
        ActorStamp {
            user_id: self.user_id,
            display_name: self.display_name.clone(),
        }
    }
}

/// A member's role within one team.
///
/// Stored as TEXT and validated here in app code rather than as a
/// database enum (#83 §1): SQLite-friendly, and a later tier is a new
/// word plus a new match arm, not a schema migration. Exactly two
/// values ship in v0, and [`Role::parse`] is the only way TEXT becomes
/// a `Role` — an unknown word is refused at the boundary instead of
/// defaulting to either side of the authority table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Role {
    /// May administer the team: delete, invite, remove, grant/revoke
    /// ownership, purge (#83 §1 authority table).
    Owner,
    /// May participate but not administer.
    Member,
}

impl Role {
    /// Parses the TEXT form. Only `"owner"` and `"member"` are valid;
    /// anything else — including case variants, because the storage
    /// form is exactly these two lowercase words — is a validation
    /// error rather than a guess.
    pub fn parse(text: &str) -> Result<Self, DomainError> {
        match text {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            other => Err(DomainError::Validation(format!(
                "role {other:?} is not one of \"owner\" / \"member\""
            ))),
        }
    }

    /// The TEXT form — what the membership table stores.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }
}

impl TryFrom<String> for Role {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Role> for String {
    fn from(value: Role) -> Self {
        value.as_str().to_string()
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One `(user, team, role)` row — a user's standing inside one team.
///
/// Plain data on purpose: the invariants are not properties of one row
/// but of a team's whole set of them, which is [`TeamRoster`]'s job.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Membership {
    /// The member.
    pub user_id: Uuid,
    /// The team the row belongs to.
    pub team_id: Uuid,
    /// The member's role, already validated ([`Role::parse`]).
    pub role: Role,
}

/// The env/CLI bootstrap identity — a distinct actor kind, **not** a
/// membership (#83 §1).
///
/// The operator lives outside the membership table and is never
/// implicitly inside a team: if the operator wants to *own* a team,
/// that is an explicit [`Membership`] row like anyone else's. What
/// this type exists for is the other direction — when the operator
/// acts inside a team without being a member (deleting it, say), the
/// ledger must record an operator stamp, distinguishable from a
/// member's ([`LedgerActor::Operator`]), so the action is never
/// disguised as a member's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceOperator {
    user_id: Uuid,
    display_name: String,
}

impl InstanceOperator {
    /// Builds the operator identity. Same blank-name refusal as
    /// [`User::new`], for the same reason: the stamp must say who.
    pub fn new(user_id: Uuid, display_name: impl Into<String>) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::Validation(
                "operator display_name is blank".into(),
            ));
        }
        Ok(Self {
            user_id,
            display_name,
        })
    }

    /// The operator's stable id.
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// The operator's display name.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The stamp the ledger records when the operator acts.
    pub fn stamp(&self) -> ActorStamp {
        ActorStamp {
            user_id: self.user_id,
            display_name: self.display_name.clone(),
        }
    }
}

/// What the ledger records about who acted: `{ user_id, display_name
/// at write time }` (#83 §1).
///
/// The name is captured, not referenced — a rename or account deletion
/// after the fact never rewrites past entries, so history keeps saying
/// what it said when it was written.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActorStamp {
    /// Who acted.
    pub user_id: Uuid,
    /// The actor's display name as it read at write time.
    pub display_name: String,
}

/// The actor of a ledger event — a member's stamp or the operator's,
/// and the two are never the same value.
///
/// An enum rather than a stamp plus a boolean, because the
/// distinguishability is the requirement (#83 §1: an operator action
/// inside a team is never disguised as a member's): a boolean field is
/// a value a writer can forget, a variant is a choice every
/// constructor makes. The serde tag (`actor_kind`) carries the same
/// distinction onto the wire and into storage.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "actor_kind", content = "stamp", rename_all = "snake_case")]
pub enum LedgerActor {
    /// A team member acted, in their member capacity.
    Member(ActorStamp),
    /// The instance operator acted — ledger-stamped as such even when
    /// the same person also holds a membership row.
    Operator(ActorStamp),
}

impl LedgerActor {
    /// A member acting as a member.
    pub fn member(stamp: ActorStamp) -> Self {
        Self::Member(stamp)
    }

    /// The operator acting as the operator. Takes the
    /// [`InstanceOperator`] identity rather than a bare stamp, so an
    /// operator-tagged entry can only be minted from the operator.
    pub fn operator(operator: &InstanceOperator) -> Self {
        Self::Operator(operator.stamp())
    }

    /// The stamp, whichever kind of actor it belongs to.
    pub fn stamp(&self) -> &ActorStamp {
        match self {
            Self::Member(stamp) | Self::Operator(stamp) => stamp,
        }
    }

    /// Whether this is the operator acting — the question the §1 rule
    /// exists to keep answerable.
    pub const fn is_operator(&self) -> bool {
        matches!(self, Self::Operator(_))
    }
}

/// The administrative verbs of the #83 §1 authority table — everything
/// but team creation, which needs the registration policy and no
/// roster ([`may_create_team`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamVerb {
    /// Delete the team.
    Delete,
    /// Invite a user into the team.
    Invite,
    /// Remove a member from the team.
    Remove,
    /// Grant the owner role.
    GrantOwner,
    /// Revoke the owner role.
    RevokeOwner,
    /// Mark the team's stored blobs for purge (grace window, then
    /// reclaim — the same trash→purge discipline as core).
    Purge,
}

/// Who is asking, reduced to what the authority table cares about:
/// the actor's current role in *this* team, or the fact that the
/// actor is the instance operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamAuthority {
    /// The instance operator, acting from outside the membership set.
    Operator,
    /// A member, with their current role.
    Member(Role),
}

/// The #83 §1 authority table for in-team verbs, as one decision
/// function.
///
/// Owners may do everything. The operator may **delete** a team
/// (ledger-stamped as the operator) and nothing else in this table:
/// §1 grants the operator no implicit invite / remove / role-grant /
/// purge inside a team it does not own — for those the operator joins
/// like anyone else and acts through a membership row.
///
/// This answers "does the role permit the verb" and nothing more; the
/// last-owner rule is a property of the target and the roster, checked
/// separately ([`TeamRoster::check_remove`] and friends), so a verb
/// can be permitted and still refused.
pub const fn verb_allowed(authority: TeamAuthority, verb: TeamVerb) -> bool {
    match verb {
        TeamVerb::Delete => matches!(
            authority,
            TeamAuthority::Operator | TeamAuthority::Member(Role::Owner)
        ),
        TeamVerb::Invite
        | TeamVerb::Remove
        | TeamVerb::GrantOwner
        | TeamVerb::RevokeOwner
        | TeamVerb::Purge => matches!(authority, TeamAuthority::Member(Role::Owner)),
    }
}

/// Whether the instance accepts team creation from ordinary users, or
/// only from the operator (#83 §1: "a closed-registration flag flips
/// this to operator").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationPolicy {
    /// Any authenticated user may create a team.
    Open,
    /// Only the instance operator may create a team.
    Closed,
}

/// Who is asking to create a team. Creation happens before any roster
/// exists, so the actor here is not a [`TeamAuthority`] — there is no
/// role to have yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationActor {
    /// Any authenticated user.
    AuthenticatedUser,
    /// The instance operator.
    Operator,
}

/// The team-create row of the #83 §1 authority table.
pub const fn may_create_team(actor: CreationActor, policy: RegistrationPolicy) -> bool {
    match policy {
        RegistrationPolicy::Open => true,
        RegistrationPolicy::Closed => matches!(actor, CreationActor::Operator),
    }
}

/// One team's membership set — the value every membership invariant is
/// evaluated over.
///
/// Built from the current state rows (never from the ledger — #83 §1:
/// authorization always reads current membership state) and consulted
/// before a write. The `check_*` methods answer for the write without
/// performing it: this crate has no storage to perform it on, and
/// keeping the decision pure is what makes the last-owner rule
/// testable here rather than in an integration suite.
#[derive(Debug, Clone)]
pub struct TeamRoster {
    team_id: Uuid,
    members: Vec<Membership>,
}

impl TeamRoster {
    /// Assembles the roster for `team_id` from its membership rows.
    ///
    /// Two shapes are refused because each would make every later
    /// answer a lie: a row belonging to another team (the owner count
    /// would count strangers), and two rows for one user (a role
    /// lookup would depend on iteration order).
    ///
    /// An **empty** roster is accepted: it is the state a team is in
    /// while being created, before its first owner row lands. What is
    /// enforced is not "a roster always has an owner" but "no
    /// operation may take an owned team to zero owners" — the check
    /// sits on the transitions, where the rule actually bites.
    pub fn new(team_id: Uuid, members: Vec<Membership>) -> Result<Self, DomainError> {
        for row in &members {
            if row.team_id != team_id {
                return Err(DomainError::Validation(format!(
                    "membership row for user {} belongs to team {}, not {team_id}",
                    row.user_id, row.team_id
                )));
            }
        }
        for (index, row) in members.iter().enumerate() {
            if members[..index]
                .iter()
                .any(|prior| prior.user_id == row.user_id)
            {
                return Err(DomainError::Validation(format!(
                    "user {} appears twice in team {team_id}'s roster",
                    row.user_id
                )));
            }
        }
        Ok(Self { team_id, members })
    }

    /// The team this roster describes.
    pub fn team_id(&self) -> Uuid {
        self.team_id
    }

    /// How many owners the team currently has.
    pub fn owner_count(&self) -> usize {
        self.members
            .iter()
            .filter(|row| row.role == Role::Owner)
            .count()
    }

    /// The current role of `user_id`, or `None` for a non-member.
    pub fn role_of(&self, user_id: Uuid) -> Option<Role> {
        self.members
            .iter()
            .find(|row| row.user_id == user_id)
            .map(|row| row.role)
    }

    /// Whether `user_id` may leave — the voluntary spelling of the
    /// departure rule. Same invariant as [`Self::check_remove`]: the
    /// last owner cannot go, however the going is phrased.
    pub fn check_leave(&self, user_id: Uuid) -> Result<(), DomainError> {
        self.check_departure(user_id)
    }

    /// Whether `target` may be removed by someone else. Authority (is
    /// the remover an owner?) is [`verb_allowed`]'s question; this one
    /// is about the team the removal would leave behind.
    pub fn check_remove(&self, target: Uuid) -> Result<(), DomainError> {
        self.check_departure(target)
    }

    /// Whether `target`'s role may become `new_role`. Covers
    /// self-demotion — an owner demoting themself is a role change
    /// like any other, and the last-owner rule does not care whose
    /// hand is on it.
    pub fn check_role_change(&self, target: Uuid, new_role: Role) -> Result<(), DomainError> {
        let current = self.member_role(target)?;
        if current == Role::Owner && new_role == Role::Member && self.owner_count() == 1 {
            return Err(DomainError::LastOwner {
                team_id: self.team_id,
            });
        }
        Ok(())
    }

    /// The one departure rule, spelled once: a member may go unless
    /// they are the last owner.
    fn check_departure(&self, user_id: Uuid) -> Result<(), DomainError> {
        let role = self.member_role(user_id)?;
        if role == Role::Owner && self.owner_count() == 1 {
            return Err(DomainError::LastOwner {
                team_id: self.team_id,
            });
        }
        Ok(())
    }

    /// The role `user_id` holds, or a validation error for a
    /// non-member — asking to remove or re-role someone who is not in
    /// the team is a malformed request, not a no-op.
    fn member_role(&self, user_id: Uuid) -> Result<Role, DomainError> {
        self.role_of(user_id).ok_or_else(|| {
            DomainError::Validation(format!(
                "user {user_id} is not a member of team {}",
                self.team_id
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster(rows: &[(Uuid, Role)]) -> (Uuid, TeamRoster) {
        let team_id = Uuid::now_v7();
        let members = rows
            .iter()
            .map(|(user_id, role)| Membership {
                user_id: *user_id,
                team_id,
                role: *role,
            })
            .collect();
        (team_id, TeamRoster::new(team_id, members).unwrap())
    }

    #[test]
    fn the_last_owner_cannot_leave_be_removed_or_self_demote() {
        let owner = Uuid::now_v7();
        let member = Uuid::now_v7();
        let (team_id, roster) = roster(&[(owner, Role::Owner), (member, Role::Member)]);

        // All three phrasings of the same departure are the same
        // refusal, and it names the team.
        for result in [
            roster.check_leave(owner),
            roster.check_remove(owner),
            roster.check_role_change(owner, Role::Member),
        ] {
            match result {
                Err(DomainError::LastOwner { team_id: refused }) => {
                    assert_eq!(refused, team_id)
                }
                other => panic!("expected LastOwner, got {other:?}"),
            }
        }

        // The plain member is not what the rule protects.
        roster.check_leave(member).unwrap();
        roster.check_remove(member).unwrap();
    }

    #[test]
    fn with_two_owners_the_departure_is_ordinary() {
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        let (_, roster) = roster(&[(first, Role::Owner), (second, Role::Owner)]);

        roster.check_leave(first).unwrap();
        roster.check_remove(first).unwrap();
        roster.check_role_change(first, Role::Member).unwrap();
    }

    #[test]
    fn a_non_member_is_a_malformed_target_not_a_no_op() {
        let owner = Uuid::now_v7();
        let (_, roster) = roster(&[(owner, Role::Owner)]);
        let stranger = Uuid::now_v7();

        for result in [
            roster.check_leave(stranger),
            roster.check_remove(stranger),
            roster.check_role_change(stranger, Role::Owner),
        ] {
            assert!(matches!(result, Err(DomainError::Validation(_))));
        }
    }

    #[test]
    fn a_roster_refuses_foreign_rows_and_duplicate_users() {
        let team_id = Uuid::now_v7();
        let user = Uuid::now_v7();

        let foreign = Membership {
            user_id: user,
            team_id: Uuid::now_v7(),
            role: Role::Member,
        };
        assert!(matches!(
            TeamRoster::new(team_id, vec![foreign]),
            Err(DomainError::Validation(_))
        ));

        let row = Membership {
            user_id: user,
            team_id,
            role: Role::Member,
        };
        assert!(matches!(
            TeamRoster::new(team_id, vec![row.clone(), row]),
            Err(DomainError::Validation(_))
        ));
    }

    #[test]
    fn role_text_admits_exactly_the_two_shipped_words() {
        assert_eq!(Role::parse("owner").unwrap(), Role::Owner);
        assert_eq!(Role::parse("member").unwrap(), Role::Member);

        // Anything else is refused rather than guessed at — including
        // case variants, because the storage form is exact, and blank,
        // because a defaulted role is a defaulted authority.
        for invalid in ["Owner", "OWNER", "admin", "guest", "", " member"] {
            assert!(
                matches!(Role::parse(invalid), Err(DomainError::Validation(_))),
                "{invalid:?} must not parse as a role"
            );
        }

        // The round trip is exact, so what the table stores is what
        // parse admits.
        for role in [Role::Owner, Role::Member] {
            assert_eq!(Role::parse(role.as_str()).unwrap(), role);
        }
    }

    #[test]
    fn an_operator_stamp_is_never_a_member_stamp() {
        let person = User::new(Uuid::now_v7(), "Hoshino").unwrap();
        let operator = InstanceOperator::new(person.user_id(), "Hoshino").unwrap();

        // Same human, same id, same name — the two actors still
        // compare unequal, because the capacity is part of the value.
        let as_member = LedgerActor::member(person.stamp());
        let as_operator = LedgerActor::operator(&operator);
        assert_ne!(as_member, as_operator);
        assert!(!as_member.is_operator());
        assert!(as_operator.is_operator());
        assert_eq!(as_member.stamp(), as_operator.stamp());

        // The distinction survives serialization — what storage and
        // the wire see is tagged, so no reader downstream can confuse
        // the two even with identical stamps.
        let member_json = serde_json::to_value(&as_member).unwrap();
        let operator_json = serde_json::to_value(&as_operator).unwrap();
        assert_eq!(member_json["actor_kind"], serde_json::json!("member"));
        assert_eq!(operator_json["actor_kind"], serde_json::json!("operator"));
    }

    #[test]
    fn the_authority_table_reads_as_specified() {
        use TeamAuthority::{Member, Operator};

        let owner = Member(Role::Owner);
        let plain = Member(Role::Member);

        for verb in [
            TeamVerb::Delete,
            TeamVerb::Invite,
            TeamVerb::Remove,
            TeamVerb::GrantOwner,
            TeamVerb::RevokeOwner,
            TeamVerb::Purge,
        ] {
            // Owners may do everything; plain members nothing in this
            // table.
            assert!(verb_allowed(owner, verb));
            assert!(!verb_allowed(plain, verb));
            // The operator's one grant is delete — ledger-stamped as
            // the operator, never disguised (§1).
            assert_eq!(verb_allowed(Operator, verb), verb == TeamVerb::Delete);
        }
    }

    #[test]
    fn team_creation_follows_the_registration_policy() {
        use CreationActor::{AuthenticatedUser, Operator};

        assert!(may_create_team(AuthenticatedUser, RegistrationPolicy::Open));
        assert!(may_create_team(Operator, RegistrationPolicy::Open));
        assert!(!may_create_team(
            AuthenticatedUser,
            RegistrationPolicy::Closed
        ));
        assert!(may_create_team(Operator, RegistrationPolicy::Closed));
    }

    #[test]
    fn a_rename_moves_the_next_stamp_not_the_past_one() {
        let mut user = User::new(Uuid::now_v7(), "before").unwrap();
        let stamped_then = user.stamp();
        user.rename("after").unwrap();

        // The old stamp is a captured value, untouched by the rename —
        // which is exactly what lets the ledger keep it forever.
        assert_eq!(stamped_then.display_name, "before");
        assert_eq!(user.stamp().display_name, "after");

        assert!(matches!(
            user.rename("   "),
            Err(DomainError::Validation(_))
        ));
        assert!(User::new(Uuid::now_v7(), "").is_err());
    }
}
