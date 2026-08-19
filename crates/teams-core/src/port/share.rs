//! `port::share` — reserved for the share domain.
//!
//! #63 owns the share vocabulary: pursuit/cull/merge semantics,
//! promotion copy *timing* (at IN vs at merge), and the minimum
//! provenance payload at promotion. This port is filled after #63
//! fixes those; it exists now so the crate layering already has the
//! seam in the place the design says it goes (#83 §0), and so nothing
//! grows share-shaped methods on the other ports in the meantime.

/// Reserved — filled after #63. Deliberately empty: an invented verb
/// here would be this crate deciding what #63 exists to decide.
pub trait SharePort {}
