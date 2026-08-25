//! # asterism-teams-wire — what a member's client and a team server
//! both say
//!
//! The leaf #148 decision 15 asks for: MIT/Apache, depended on by both
//! planes, depending on neither. #83 §4 forbids `asterism-* ->
//! teams-*` in any form, so the member's client cannot reach the
//! vocabulary it needs where that vocabulary used to live; §4's own
//! second choice for exactly this case is a leaf, and this is it.
//!
//! ## What lives here, and what does not
//!
//! Here: the shapes that are **teams-shaped** — a session, a team and
//! its roster, a page of the ledger, the three content verbs hosting
//! adds, and the projection envelope (#148 decisions 12 to 14).
//!
//! Not here, and each for a stated reason:
//!
//! - **The forge's verbs.** Their DTOs are already MIT/Apache in
//!   `asterism-contract::forge`, and #148 revision 10 says both planes
//!   may name them there. A copy here would be a second home for one
//!   type, which is the failure this crate is meant to prevent rather
//!   than a shape it should take.
//! - **The substrate's own surfaces.** The blob upload, the purge
//!   two-step and the head registry stay in `teams-contract`: they are
//!   an operator's and an admin's vocabulary, and a member's client
//!   does not speak them. The dividing line is *who says it*, not what
//!   it is about.
//! - **Validation.** Same as `teams-contract`: role words, ids and
//!   digests are parsed by whichever plane receives them. This crate
//!   defines shapes.
//!
//! ## Wire representation
//!
//! Unchanged from the surfaces these shapes came off, because a moved
//! type that re-spelled its fields would not be a move:
//!
//! - Ids: UUID hyphenated `String`.
//! - Timestamps: unix epoch milliseconds as `i64`.
//! - Opaque JSON — a ledger event's payload, a projection's body —
//!   serialised into a `String`. Schema-bridge does not render
//!   `serde_json::Value`, and in the projection's case the opacity is
//!   the design rather than a limitation of the renderer (#148
//!   decision 14).

#![warn(missing_docs)]

pub mod command;
pub mod dto;
pub mod projection;
