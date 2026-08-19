//! # teams-server — the hosted Team plane's server library
//!
//! Third slice of #83 (#91): auth v0 and the team/membership HTTP API.
//! The binary (`main.rs`) owns the CLI; this library owns what the
//! route tests drive in-process:
//!
//! - [`http`] — the axum `/teams/*` router: the session → user →
//!   membership gate, the authority checks over `teams-core`'s
//!   decision functions, and the domain-refusal → 4xx mapping.
//! - [`rate_limit`] — the one limiter every auth endpoint sits behind
//!   (#83 §5: from v0, not retrofitted).
//! - [`state`] — the shared [`TeamsCtx`](state::TeamsCtx) the handlers
//!   read: repository, credential store, registration policy.
//!
//! The MCP surface, blob routes, purge and backup are later slices —
//! the module docs say which issue owns each.

#![warn(missing_docs)]

pub mod http;
pub mod rate_limit;
pub mod state;
