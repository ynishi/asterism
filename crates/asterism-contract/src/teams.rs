//! The team plane's shapes, as they cross this app's own boundary.
//!
//! A topic module for the same reason [`forge`](crate::forge) is one:
//! the team plane answers to a model of its own, and a reader of
//! [`TeamLedgerPageDto`] needs the types beside it rather than the
//! asset DTOs two hundred lines up.
//!
//! # Why these exist here as well as on the wire
//!
//! Two boundaries, not one. `asterism-teams-wire` carries what a
//! member's client and a team server say to each other over HTTP; this
//! carries what the desktop app says to its own frontend. They meet at
//! one place — the Tauri command — and the shapes look alike there
//! because the same facts cross both.
//!
//! Alike is not the same as shared. This crate imports no other
//! Asterism crate, which is what keeps it a leaf and what stops a
//! dependency cycle; and the frontend has one vocabulary rather than
//! two, which is what `bindings.ts` being a projection of *this* crate
//! means. A command that returned a wire type would hand the second
//! vocabulary to every screen that called it.
//!
//! So the duplication is the boundary, and the mapping lives in the
//! command that crosses it.
//!
//! # What the ledger's shape carries
//!
//! An append-only record of what a team did, in what capacity. The
//! read is paged over `seq` rather than whole, because the forge
//! writes a row per push and a table that grew by the occasional
//! membership gesture does not any more.
//!
//! Two properties of it decide how a screen may read it, and both are
//! stated on the fields they belong to: a null cursor is not an end,
//! and an actor's display name is a snapshot rather than a lookup.

use serde::{Deserialize, Serialize};

use schema_bridge::SchemaBridge;

/// One page of a team's ledger, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamLedgerPageDto {
    /// The events on this page, `seq` ascending.
    pub events: Vec<TeamLedgerEventDto>,
    /// Where the next page resumes, or `null`.
    ///
    /// **A null cursor is not the end of the ledger.** A page that
    /// filled the limit it asked for always carries one — even when it
    /// happened to end on the last event there is, because whether
    /// anything follows is only answerable by asking. So null means a
    /// short page, and a short page says nothing lay past here *when it
    /// was taken*. A ledger has no final page; a caller following one
    /// keeps the last `seq` it saw and asks again.
    pub next_after: Option<i64>,
}

/// One act, as the ledger recorded it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamLedgerEventDto {
    /// Position within the team's stream, from 1.
    pub seq: i64,
    /// Globally unique id of the event.
    pub event_id: String,
    /// The stream it belongs to.
    pub team_id: String,
    /// `"member"` or `"admin"`.
    ///
    /// The capacity an act was performed in, kept apart from who
    /// performed it: an instance admin acting inside a team without a
    /// membership row is recorded as an admin and never as a member.
    /// A surface showing the actor without this is answering half the
    /// question the ledger exists for.
    pub actor_kind: String,
    /// Who acted.
    pub actor_user_id: String,
    /// The actor's display name **as it read when the act happened**.
    ///
    /// A snapshot rather than a reference. Resolving it against
    /// whatever the name is now would let a rename change a record of
    /// something that already happened.
    pub actor_display_name: String,
    /// When, epoch ms.
    pub occurred_at_ms: i64,
    /// The namespaced and versioned kind, e.g.
    /// `"teams.membership.role_changed/1"`.
    ///
    /// The version is part of the identity rather than decoration, and
    /// the set grows — a caller meeting a kind it has never seen is an
    /// ordinary event and not an error.
    pub kind: String,
    /// The typed references this act makes.
    pub subjects: Vec<TeamSubjectRefDto>,
    /// The kind-versioned body, serialised.
    ///
    /// Opaque here on purpose: what a body carries is fixed by its
    /// kind, and a type that named the fields of one kind would have to
    /// name them for every kind the server ever adds.
    pub payload_json: String,
}

/// One typed reference an act makes.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamSubjectRefDto {
    /// The reference's kind: `"digest"`, `"user"`, `"blob"` or
    /// `"forge_identity"`.
    pub ref_type: String,
    /// The reference's value in its storage spelling.
    pub value: String,
}
