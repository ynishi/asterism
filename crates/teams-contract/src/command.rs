//! Command DTOs — inputs of the state-changing `/teams/*` routes no
//! client sends.
//!
//! What a **member's client** sends is in `asterism-teams-wire`, which
//! the client may link and this crate it may not (#148 decision 15):
//! login, team creation, the three content verbs, and — since #210
//! gave an owner a screen to say them from — the roster writes.
//!
//! What stays is the substrate's own upload, and the line it is on is
//! who sends it rather than whose act it is: uploading into a team's
//! store is a member's act, and the route refuses an admin's implicit
//! one. No client sends it because content reaches a team through the
//! promotion path instead.
//!
//! The session token is **not** a field on any of these: it travels in
//! the `Authorization: Bearer` header, resolved by the server's gate
//! middleware before a handler sees the body (#83 §5 — every route:
//! session token → user_id → membership gate).

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

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
