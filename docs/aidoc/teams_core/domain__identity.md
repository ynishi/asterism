# teams-core::domain::identity

`identity` — who exists, who belongs to a team, and who may do what.

Three separations carry this module, all from #83 §1:

- **Membership is not identity.** A [`User`] exists independently of
  any team; a [`Membership`] is one `(user, team, role)` row and a
  team's set of them is what every invariant is evaluated over.
- **The admin is not a member.** [`InstanceAdmin`] is the env/CLI
  bootstrap identity and lives *outside* the membership set —
  owning a team is an explicit membership row like anyone else's,
  and when an admin acts inside a team the ledger stamp says so
  ([`LedgerActor::Admin`]): an admin action is never disguised as
  a member's.
- **Authorization reads state, not history.** The decision functions
  here take the current membership set ([`TeamRoster`]); the ledger
  records what was decided and is never consulted to decide.

The one absolute invariant is the last-owner rule: every team has
≥1 owner at all times, so the last owner cannot leave, be removed,
or self-demote (the GitHub/GitLab/Gitea rule). It is enforced as
`check_*` methods on [`TeamRoster`] so every write path asks the
same three lines instead of re-deriving them.

## Functions

- `may_create_team` — The team-create row of the #83 §1 authority table.
- `verb_allowed` — The #83 §1 authority table for in-team verbs, as one decision

## Types

- `ActorStamp` — What the ledger records about who acted: `{ user_id, display_name
- `CreationActor` — Who is asking to create a team. Creation happens before any roster
- `InstanceAdmin` — The env/CLI bootstrap identity — a distinct actor kind, **not** a
- `LedgerActor` — The actor of a ledger event — a member's stamp or an admin's, and
- `Membership` — One `(user, team, role)` row — a user's standing inside one team.
- `RegistrationPolicy` — Whether the instance accepts team creation from ordinary users, or
- `Role` — A member's role within one team.
- `TeamAuthority` — Who is asking, reduced to what the authority table cares about:
- `TeamRoster` — One team's membership set — the value every membership invariant is
- `TeamVerb` — The verbs an authority table answers for — everything
- `User` — A person known to the instance — the identity teams are built from.

