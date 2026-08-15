//! The shape of an exported artefact's `<file>.meta.json` sidecar.
//!
//! Two crates that must not depend on each other need this vocabulary:
//! an exporter writes the sidecar (adapter side), and the ingest path
//! reads it back when a returning file declares `derived_from:
//! sidecar` (core side). Adapters do not depend on the core and the
//! core does not depend on adapters, so the words live here, in the
//! leaf DTO crate both already use — the same reasoning that puts
//! [`DerivedDto`](crate::dto::DerivedDto) here.
//!
//! Constants rather than a struct: the body is an
//! [`AssetCardDto`](crate::dto::AssetCardDto) projection (possibly
//! field-filtered by the caller) with one extra key, and the reader
//! walks it as JSON. A struct would claim a rigidity the file does not
//! have — it can be hand-written, truncated by an allowlist, or come
//! from a version that knew fewer fields.

/// Key under which a sidecar carries the export's own identity.
///
/// Underscore-prefixed so it cannot collide with an `AssetCardDto`
/// field name, present or future.
///
/// The value is an object:
///
/// ```json
/// {
///   "schema": "asterism.sidecar/1",
///   "dispatch_id": "<DispatchId>",
///   "pursuit_id": "<PursuitId>",
///   "exporter_slug": "file",
///   "source_asset_id": "<AssetId>"
/// }
/// ```
///
/// `pursuit_id` (#29) is present when the dispatch carried its stamp:
/// the dispatch names the hop the file travelled through, the pursuit
/// names the line of work it belongs to. On re-ingest both are
/// **claims** — recorded in `_trace` and resolved independently. The
/// rule the membership read follows (a later slice of #29): where the
/// dispatch join and a sidecar copy disagree, the dispatch row's own
/// stamp answers, because the copy can be stale after a restamp.
pub const SIDECAR_IDENTITY_KEY: &str = "_asterism";

/// Field inside the identity block naming the dispatch (the hop).
///
/// A const for the same reason the block key is one: the writer
/// (exporter crates) and the reader (the ingest path) cannot see each
/// other, and a divergence between their spellings fails silently as
/// "no identity in this sidecar".
pub const SIDECAR_DISPATCH_ID_FIELD: &str = "dispatch_id";

/// Field inside the identity block naming the pursuit (the line of
/// work, #29). Same silent-divergence reasoning as
/// [`SIDECAR_DISPATCH_ID_FIELD`].
pub const SIDECAR_PURSUIT_ID_FIELD: &str = "pursuit_id";

/// Version tag written into the identity block.
///
/// This JSON leaves the machine and can come back months later, after
/// a round trip through tools that know nothing about Asterism. A
/// reader that cannot tell "a shape I understand" from "a shape I do
/// not" will keep interpreting a stale file confidently, which is
/// worse than refusing it.
pub const SIDECAR_SCHEMA: &str = "asterism.sidecar/1";

/// Suffix appended to an artefact's locator to find its sidecar.
pub const SIDECAR_SUFFIX: &str = ".meta.json";
