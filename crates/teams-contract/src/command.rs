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
/// authenticated user when registration is open, admins only when it
/// is closed.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateTeamCommand {
    /// The founding owner's user id.
    ///
    /// **Required when an admin creates the team**: an admin is never
    /// implicitly a member (#83 §1), so the owner row must name
    /// whichever user will own it — which may be the admin's own user
    /// id, making that ownership an explicit membership row like
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

/// Uploads a blob into the team's store
/// (`PUT /teams/{team_id}/blobs?digest=sha256:<hex>`, members only).
///
/// Its fields travel in the **query string**: the request body is the
/// blob's raw bytes, streamed, so there is no JSON body for them to
/// ride in (the OCI registry `PUT ?digest=` shape, #83 §3).
/// [`EnterContentCommand`] is the same shape for the same reason.
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

/// Brings content into a team against open work
/// (`PUT /teams/{team_id}/forge/pursuits/{id}/content?digest=sha256:<hex>`,
/// members only — #148 decision 5).
///
/// [`UploadBlobCommand`]'s shape for [`UploadBlobCommand`]'s reason.
/// What differs is what the write leaves behind: this one mints the
/// team asset a round can name, and the work it entered against is the
/// path's.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EnterContentCommand {
    /// The digest the client declares the bytes to have, in the shared
    /// `sha256:<64hex>` notation. Mandatory, and typed `Option` only so
    /// the omission answers in the house error shape.
    pub digest: Option<String>,
}

/// Asks what a team holds for a list of its own asset ids
/// (`POST /teams/{team_id}/forge/content/resolve`, members only).
///
/// A body rather than a query string because the list is the request:
/// a client reconciling what it promoted asks about as many ids as it
/// has, and a URL is not where that belongs.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ResolveContentCommand {
    /// Team asset ids, hyphenated UUIDs. An id this team did not mint
    /// comes back as unknown rather than as a refusal.
    pub asset_ids: Vec<String>,
}

/// Asks which digests a team already has
/// (`POST /teams/{team_id}/forge/content/have`, members only).
///
/// **This exists to avoid re-sending bytes and for nothing else.** It
/// answers inside one team, to that team's members, about digests the
/// caller is holding and could upload anyway — so what it reveals is
/// what the asker could learn by uploading, minus the upload. That
/// bound is the design rather than a caveat on it: the same question
/// asked across teams, or by anyone outside one, is the deduplication
/// side channel Harnik et al. (2010) describes, and #83 §3 closes it
/// by making the link row the visibility boundary. This route is
/// inside that boundary.
///
/// A POST for a read, for [`ResolveContentCommand`]'s reason.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct HaveContentCommand {
    /// Digests in the shared `sha256:<64hex>` notation. One that does
    /// not parse is a `400` about the request rather than a quiet
    /// "not held".
    pub digests: Vec<String>,
}
