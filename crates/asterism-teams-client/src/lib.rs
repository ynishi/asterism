//! # asterism-teams-client — the member's half of a shared line
//!
//! A team hosts a forge and holds what its members brought (#148
//! decisions 1 and 2). This is what a member's machine runs to talk to
//! one: a session, the reads over a shared line, the forge's verbs,
//! and the composite act that is the point of the whole issue —
//! [`promotion`], handing an Asset over so the team converts it into
//! something of its own.
//!
//! ## Where the boundary is
//!
//! `asterism-* -> teams-*` is forbidden in any form (#83 §4), so this
//! crate reaches the wire through two doors and no others:
//! `asterism-teams-wire`, the leaf both planes depend on and which depends on
//! neither, and `asterism-contract::forge`, whose DTOs were already
//! MIT/Apache and already read on both sides. What follows from that
//! is worth stating plainly: **this crate never links the team plane's
//! code**, only its vocabulary. Everything it knows about a team it
//! learned over HTTP.
//!
//! ## The three things that do not cross
//!
//! - **Ids.** Each plane mints its own surrogates (#148 decision 6). A
//!   team's id for anything arrives here as a
//!   [`TeamScopedId`](asterism_core::domain::team_link::TeamScopedId),
//!   which is a different type from `AssetId` with no conversion in
//!   either direction, so the forbidden read does not compile.
//!   Subjects and digests do cross, and they are the two the issue
//!   says may.
//! - **Anything re-derivable.** A promotion sends the material and the
//!   marks a person wrote, and nothing else (#148 decision 4).
//!   Thumbnails, indexed bodies and `Imported`/`Machine` marks stay
//!   home because the receiving side can make them again.
//! - **Anything undeclared.** What may travel in a projection is
//!   declared at [`mapper`], and a local field nobody has declared
//!   does not leave (#148 decision 13). A column added to the Asset
//!   next year starts out staying home.
//!
//! ## The clone
//!
//! [`clone`] takes one entry of a team's line onto this machine (#148
//! decision 10). It is an import rather than a forge concept — new
//! ids, no relation row, and where it came from spelled in
//! `source_kind` and `source_locator` the way every other import
//! spells it.
//!
//! It stops where the local plane's own vocabulary begins: recording
//! an arrival is [`clone::Imports`], a port a caller fills, for the
//! reason the relation's port is one — `asterism-contract`'s `command`
//! module is the local app's plumbing and stays off this crate's graph.
//!
//! ## What is not here
//!
//! **A local copy of a shared line.** Decision 16 serves shared lines
//! through rather than mirroring them, so every read here is a request
//! and there is no staleness to reason about. Wanting it locally is
//! what a clone is for.

#![warn(missing_docs)]

pub mod client;
pub mod clone;
pub mod link;
pub mod mapper;
pub mod promotion;

pub use client::{TeamsClient, TeamsClientError};
pub use clone::{Arrival, CloneRequest, Cloned, Imports, clone_entry};
pub use link::{DanglingLink, LinkVerification};
pub use mapper::{
    DeclaredInput, LocalSubject, ProjectionView, PromotedMark, ReadMark, projection_body,
    read_projection_body,
};
pub use promotion::{Promotion, PromotionOutcome};
