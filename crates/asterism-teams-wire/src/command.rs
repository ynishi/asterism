//! Command shapes — inputs of the `/teams/*` routes a member's client
//! calls.
//!
//! An owner's roster writes are here too, and that is not an exception
//! to the line above. They moved from `teams-contract` when an owner
//! gained a screen to say them from (#210), and an owner saying them
//! is a member's client saying them. What stayed behind stayed for the
//! reason the crate doc gives — no client sends it — and not for
//! anything about whose act it is: the substrate's own upload is a
//! member's act that no client happens to send.
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

/// Presents a device token to `POST /teams/auth/device/login` (#204).
///
/// Answers with the same session the password arm answers with —
/// device tokens sit in front of sessions rather than replacing them,
/// so a client that has logged in this way is holding an ordinary
/// session and everything downstream of the gate is unchanged.
///
/// **No derived `Debug`**, for the reason
/// [`SessionDto`](crate::dto::SessionDto)'s hand-written one gives:
/// this body carries the one credential a client's disk is allowed to
/// hold.
#[derive(Clone, Serialize, Deserialize, SchemaBridge)]
pub struct DeviceLoginCommand {
    /// The device token, as the mint answered with it. A token that
    /// does not resolve is a `401` whose body carries a `reason` —
    /// `expired`, `idle` or `revoked` (#163), `revoked_by_instance` or
    /// `locked` (#213) — and a message worded for it. Which end each
    /// names, and why they are told apart, is
    /// `teams-infra`'s `DeviceTokenResolution`.
    pub token: String,
}

impl std::fmt::Debug for DeviceLoginCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceLoginCommand")
            .field("token", &"<not shown>")
            .finish()
    }
}

/// Starts a sign-in through the provider
/// (`POST /teams/auth/oidc/attempts`, #163).
///
/// The app makes a secret it keeps, and sends its SHA-256 here. The
/// instance stores the hash beside the attempt and will hand the
/// session only to a collect that presents the secret — so the attempt
/// id, which travels through a browser and a provider's logs, is not by
/// itself a way to collect what the person signed in for.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OidcAttemptCommand {
    /// SHA-256 of the app's secret, hex. Not the secret.
    pub collector: String,
    /// What the start page shows the person before they go on to the
    /// provider — "Yutaka's MacBook". Blank is refused.
    pub label: String,
    /// The port the app is listening on at `127.0.0.1`. The provider's
    /// answer reaches the app through the browser, as a redirect to
    /// this port carrying a one-time grant — which is what ties the
    /// sign-in to the machine the app runs on, and why there is no
    /// poll to fall back to. `0` is refused.
    pub loopback_port: u16,
}

/// Collects a sign-in attempt
/// (`POST /teams/auth/oidc/attempts/{id}/collect`, #163).
///
/// Two things, for two questions: the secret says this caller is the
/// app that started the attempt, the grant says the browser that
/// finished it was sent to that app's machine. A resolved attempt is
/// collected with both, and a collect missing either is answered as
/// though the attempt did not exist. A refused attempt is answered to
/// the secret alone — there was no grant to deliver, and the app that
/// started it is owed the refusal.
///
/// **No derived `Debug`**: until the collect lands, these two are what
/// stand between the attempt id and a session.
#[derive(Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CollectOidcAttemptCommand {
    /// The secret whose hash started the attempt.
    pub secret: String,
    /// The grant the browser delivered to the app's loopback listener.
    pub grant: String,
}

impl std::fmt::Debug for CollectOidcAttemptCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectOidcAttemptCommand")
            .field("secret", &"<not shown>")
            .field("grant", &"<not shown>")
            .finish()
    }
}

/// Asks for a device token (`POST /teams/auth/device`, #204).
///
/// Carries no account: whose token it is comes from the session the
/// gate resolved, which is what keeps the mint from being a way to
/// issue a credential for somebody else.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MintDeviceTokenCommand {
    /// What to call this device in the owner's listing — "Yutaka's
    /// MacBook". Blank is refused; the label is how a person tells one
    /// row from another when deciding what to revoke.
    pub label: String,
}

/// Creates a team (`POST /teams/create`).
///
/// Who may call this follows the registration policy (#83 §1): any
/// authenticated user when registration is open, admins only when it
/// is closed.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CreateTeamCommand {
    /// The team's name (#218) — asked for at founding, refused blank
    /// the way [`teams_core::domain::identity::Team::new`] refuses
    /// it. `#[serde(default)]` so a request from before this field
    /// existed decodes rather than 422s on the wire — the domain's
    /// blank-name refusal is what turns a missing name into a `400`,
    /// with a message that says why, instead of a bare deserialize
    /// failure.
    #[serde(default)]
    pub name: String,
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

/// Renames a team (`POST /teams/{team_id}/rename`, #218) — an
/// owner-only verb, per the authority table.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameTeamCommand {
    /// The new name. Refused blank, the same rule founding applies.
    pub name: String,
}

/// Invites a user into the team (`POST /teams/{team_id}/members/invite`,
/// owner only).
///
/// **Exactly one of `user_id` / `login` (#218).** A login is resolved
/// to an account on the server, the same way it always was implicit
/// in `user_id` being an account's id; the id form stays reachable for
/// when the login is not known, or is ambiguous to type by hand. Both
/// set, or neither, is a `400` — the command names one invitee, not a
/// choice between two.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct InviteMemberCommand {
    /// The invitee's account id — must hold an account on this
    /// instance. Mutually exclusive with `login`.
    #[serde(default)]
    pub user_id: Option<String>,
    /// The invitee's login, resolved to an account on the server.
    /// Mutually exclusive with `user_id`.
    #[serde(default)]
    pub login: Option<String>,
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

/// Brings content into a team against open work
/// (`PUT /teams/{team_id}/forge/pursuits/{id}/content?digest=sha256:<hex>`,
/// members only — #148 decision 5).
///
/// Its one field travels in the **query string**: the request body is
/// the content's raw bytes, streamed, so there is no JSON body for it
/// to ride in (the OCI registry `PUT ?digest=` shape, #83 §3). What
/// the write leaves behind is what separates this from the substrate's
/// own upload: it mints the team asset a round can name, and the work
/// it entered against is the path's.
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
