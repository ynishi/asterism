//! The captured projection — descriptive metadata a promoter said at
//! the time (#148 decisions 12, 13 and 14).
//!
//! ## What this crate's part in it is
//!
//! Decision 14 keeps the body opaque, and the transport's share of
//! that is small and absolute: **the body is a `String` here and no
//! shape in this crate has a field that came from inside it.** Why the
//! rule exists, and why
//! [`EntryProjectionEnvelope::version`] is a fact about the envelope
//! rather than a breach of it, are argued once in
//! `teams_core::domain::projection`.
//!
//! Decision 13's declaration is likewise settled before a body reaches
//! here — it lives at the member's mapper, the only place that knows
//! both the local model and the body. By the time this crate sees one,
//! the answer is a string. A filter expressed on the wire would be a
//! second place to forget something, and forgetting there fails in the
//! unsafe direction.

use schema_bridge::SchemaBridge;
use serde::{Deserialize, Serialize};

/// The current envelope version — what a client writing today stamps,
/// and what a reader may assume when it recognises nothing else.
pub const PROJECTION_VERSION: u32 = 1;

/// One entry's projection, as it rides onto a round push.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EntryProjectionEnvelope {
    /// The entry this describes, hyphenated UUID.
    ///
    /// It must be an entry the round it rides on operates on; the
    /// server checks that rather than trusting it, and says why where
    /// it does.
    pub entry_id: String,
    /// Which mapper wrote [`Self::body`] — a fact about this envelope
    /// rather than a field of the body.
    pub version: u32,
    /// The description itself, serialised JSON, **opaque to everything
    /// between the two mappers** (#148 decision 14).
    pub body: String,
}

/// A round push as a team takes it: whatever the forge's push command
/// is, plus the projections riding with it.
///
/// **Generic over the push command on purpose.** Decision 19 rides the
/// projection on the push rather than giving it a verb, so the wire
/// shape is "a push, and also these" — and this crate may not name the
/// push, which lives in `asterism-contract::forge` where both planes
/// already read it (#148 revision 10). Taking it as a parameter is how
/// the composition gets written once instead of once per side: the
/// server deserialises `WithProjections<PushForgeRoundCommand>` and
/// the client serialises the same type, over the same flattened body.
///
/// The flatten is what keeps the mirror a mirror. A body with no
/// `projections` key is exactly the push command it always was, so a
/// client that knows nothing about projections talks to this route
/// unchanged, and the route's shape below the prefix still matches the
/// local surface's (#148 decision 19).
///
/// No `SchemaBridge` here, unlike everything else in this crate: a
/// generic has no one rendered schema, and the two shapes it composes
/// each have theirs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithProjections<P> {
    /// The push itself.
    #[serde(flatten)]
    pub push: P,
    /// What rides with it, possibly nothing.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub projections: Vec<EntryProjectionEnvelope>,
}

/// One captured projection, read back
/// (`GET /teams/{team_id}/forge/lines/{line_id}/entries/{entry_id}/projection`).
///
/// The team does not edit it and neither does this shape: what comes
/// back is what the promoter said at the time, on the same discipline
/// as an `ActorStamp` capturing a display name at write time (#148
/// decision 12).
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct EntryProjectionDto {
    /// The line the entry is on, hyphenated UUID.
    pub line_id: String,
    /// The entry, hyphenated UUID.
    pub entry_id: String,
    /// Which mapper wrote [`Self::body`].
    pub version: u32,
    /// The description, verbatim and still opaque.
    pub body: String,
    /// The member whose push captured it.
    pub promoted_by: String,
    /// When it was captured, epoch ms.
    pub pushed_at_ms: i64,
}
