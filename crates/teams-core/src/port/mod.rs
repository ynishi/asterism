//! Ports — the traits `teams-infra` implements (dependency inversion,
//! same discipline as `asterism-core`'s repository traits). Declared
//! here so the domain can be exercised against fakes with no adapter
//! compiled.

pub mod auth;
pub mod blob;
