//! Command DTOs — inputs of the state-changing `/teams/*` routes an
//! owner, an admin or an operator's tooling calls.
//!
//! What a **member's client** sends moved to `asterism-teams-wire` when the
//! leaf landed (#148 decision 15): login, team creation and the three
//! content verbs live there now, because the client may not link this
//! crate. What stayed is what is not a member's vocabulary — the
//! roster verbs, which are an owner's, and the substrate's own upload.
//!
//! The session token is **not** a field on any of these: it travels in
//! the `Authorization: Bearer` header, resolved by the server's gate
//! middleware before a handler sees the body (#83 §5 — every route:
//! session token → user_id → membership gate).

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

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

/// Uploads a blob into the team's store
/// (`PUT /teams/{team_id}/blobs?digest=sha256:<hex>`, members only).
///
/// Its fields travel in the **query string**: the request body is the
/// blob's raw bytes, streamed, so there is no JSON body for them to
/// ride in (the OCI registry `PUT ?digest=` shape, #83 §3).
/// `asterism_teams_wire::command::EnterContentCommand` is the same shape for
/// the same reason.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct UploadBlobCommand {
    /// The digest the client **declares** the bytes to have — computed
    /// by re-reading the file at promote time, in the shared
    /// `sha256:<64hex>` notation.
    ///
    /// Mandatory: omitting it is a `400`, it is typed `Option` only so
    /// the server can answer that omission in the house error shape
    /// instead of a framework rejection. A mismatch against what the
    /// server hashes while writing rejects the whole operation with a
    /// `409` carrying both sides — no blob, no link, no ledger event.
    pub digest: Option<String>,
}
