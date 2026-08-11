//! Re-export shim for the shared boundary type
//! [`asterism_contract::DerivedDto`], plus the character-count
//! constants exporter authors want to pre-enforce.
//!
//! Rationale: the wire shape lives in `asterism-contract` (the
//! leaf DTO crate) so both the SDK and the core can talk about
//! `Derived` without either depending on the other — the SDK
//! stays a pure adapter surface, the core stays free of any
//! adapter dep. Exporter authors keep writing `use
//! asterism_dispatch_sdk::Derived;` unchanged.

pub use asterism_contract::dto::{
    DERIVED_COVER_MAX_CHARS as COVER_MAX_CHARS, DERIVED_REGISTER_MAX_CHARS as REGISTER_MAX_CHARS,
    DerivedDto as Derived,
};
