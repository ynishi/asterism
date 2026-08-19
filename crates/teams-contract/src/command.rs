//! Command DTOs — inputs of the state-changing `/teams/*` routes.
//!
//! The session token is **not** a field on any of these: it travels in
//! the `Authorization: Bearer` header, resolved by the server's gate
//! middleware before a handler sees the body (#83 §5 — every route:
//! session token → user_id → membership gate).

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// Presents a credential to `POST /teams/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct LoginCommand {
    /// The login name the account was created under.
    pub login: String,
    /// The password. A wrong password and an unknown login produce the
    /// same `401` — the API does not say which half failed.
    pub password: String,
}

/// Creates a team (`POST /teams/create`).
///
/// Who may call this follows the registration policy (#83 §1): any
/// authenticated user when registration is open, the operator only
/// when it is closed.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateTeamCommand {
    /// The founding owner's user id.
    ///
    /// **Required when the operator creates the team**: the operator is
    /// never implicitly a member (#83 §1), so the owner row must name
    /// whichever user will own it — which may be the operator's own
    /// user id, making that ownership an explicit membership row like
    /// anyone else's. A regular user founds their own team: they omit
    /// this (or name themselves; naming anyone else is refused).
    pub owner_user_id: Option<String>,
}

/// Invites a user into the team (`POST /teams/{team_id}/members/invite`,
/// owner only).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct InviteMemberCommand {
    /// The invitee — must hold an account on this instance.
    pub user_id: String,
    /// The role the invitee joins with: `"owner"` or `"member"`
    /// (validated by the domain's parser; anything else is a `400`).
    pub role: String,
}

/// Removes a member (`POST /teams/{team_id}/members/remove`, owner
/// only). Removing the last owner is refused with a `409` and changes
/// nothing (#83 §1).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RemoveMemberCommand {
    /// The member to remove.
    pub user_id: String,
}

/// Grants the owner role (`POST /teams/{team_id}/owners/grant`, owner
/// only). The resulting ledger event carries both the old and the new
/// role in its payload.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct GrantOwnerCommand {
    /// The member whose role becomes `owner`.
    pub user_id: String,
}

/// Revokes the owner role (`POST /teams/{team_id}/owners/revoke`,
/// owner only). Revoking the last owner — including yourself — is a
/// `409` (#83 §1: the last owner cannot self-demote).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RevokeOwnerCommand {
    /// The owner whose role becomes `member`.
    pub user_id: String,
}
