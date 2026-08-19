//! `DomainError` — the innermost error type of the teams plane.
//!
//! Same convention as `asterism-core`'s enum of the same name: outer
//! layers (`teams-server`'s HTTP / MCP errors, once they exist) convert
//! from this, infrastructure failures are collapsed into `Infra`, and
//! the domain vocabulary stays clean. The two enums are deliberately
//! separate types — the teams plane and the local app do not share an
//! error surface, only `asterism-core`'s domain vocabulary (#83 §4).

use uuid::Uuid;

/// Errors raised by the teams domain layer.
///
/// Convention: `teams-infra` converts adapter failures into
/// `anyhow::Error` and passes them through [`Self::Infra`]; the domain
/// itself returns the specific variants directly.
#[derive(thiserror::Error, Debug)]
pub enum DomainError {
    /// Value-object / entity construction failed validation (bad role
    /// text, malformed event kind, digest with the wrong shape, and so
    /// on).
    #[error("validation error: {0}")]
    Validation(String),

    /// The operation would leave the team with zero owners — the one
    /// membership invariant #83 §1 states as absolute: the last owner
    /// cannot leave, be removed, or self-demote. A variant of its own
    /// rather than a `Validation` string, because the transport layer
    /// will want to tell "your request was malformed" apart from "the
    /// team's state refuses this", and callers may reasonably branch
    /// on it (offer "transfer ownership first" instead of a generic
    /// failure).
    #[error("team {team_id} would be left with no owner")]
    LastOwner {
        /// The team whose owner count the operation would drop to zero.
        team_id: Uuid,
    },

    /// A promotion's declared digest did not match what the bytes
    /// hashed to — promote-time TOCTOU (#83 §3). The whole operation
    /// is rejected: no copy, no ledger event. Both sides are carried
    /// because a reader who sees only one of them cannot tell which
    /// side to go and look at (the same reasoning as `asterism-core`'s
    /// `declaration_verdict`).
    #[error(
        "declared digest {declared} does not match computed {computed}; the promotion is rejected whole"
    )]
    DigestMismatch {
        /// What the client asserted the content is.
        declared: String,
        /// What the server hashed while writing.
        computed: String,
    },

    /// Failure originating from infrastructure (SQLite, filesystem,
    /// auth provider, etc.) — never constructed by this crate, which
    /// performs no IO; it exists so the port traits declared here can
    /// carry adapter failures without inventing a second error type.
    #[error("infrastructure error: {0}")]
    Infra(#[from] anyhow::Error),
}
