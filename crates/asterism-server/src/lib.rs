//! # asterism-server — library surface.
//!
//! Exposes the shared backend initialisation ([`core_init`]) and the HTTP
//! transport ([`http::router`] over [`state::ServerCtx`]) so both the
//! standalone `asterism-server` binary and the Tauri UI can build the same
//! service graph. The binary (`main.rs`) is a thin CLI shell over this
//! library.

#![warn(missing_docs)]

pub mod attribution;
pub mod core_init;
pub mod http;
pub mod mcp;
pub mod mcp_proxy;
pub mod state;
