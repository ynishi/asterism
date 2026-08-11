//! Deriving the grouping key that ties the footprints of one container
//! together.
//!
//! A parser handed one `RawItem` that really is many records — a
//! character card's slots, a harvest envelope's conversations — needs a
//! key every one of them can carry so `edge_rebuild` draws a
//! `same-bundle` edge across the set. The key has to be **derived from
//! the container's locator**: a random one would move on every
//! re-import and split a set that arrived twice into two.
//!
//! # Why this is not in `png_text` any more
//!
//! It used to be. That module read a PNG's `tEXt` chunks and emitted one
//! `Footprint::Note` per chunk, and this function existed to bundle
//! those notes back to the image they came out of. The notes are gone —
//! the text inside an image is that image's metadata rather than a
//! record of its own, and it now travels on the image's row as the
//! `Meta` axis. What survived
//! is this derivation, and it never had anything to do with PNG text:
//! its callers are the card parser and the harvest parser, both of them
//! over real containers.
//!
//! **The namespace bytes are unchanged**, so every id this has ever
//! produced still comes back the same — a card imported before the move
//! keeps its bundle.

/// UUID v5 namespace used to derive stable, deterministic bundle keys
/// from a container locator. Random bytes fixed once — DO NOT change,
/// or every previously imported container stops grouping with itself on
/// re-import.
const BUNDLE_NS: uuid::Uuid = uuid::Uuid::from_bytes([
    0xa5, 0xf1, 0x3d, 0x82, 0x9c, 0x6a, 0x4e, 0x18, 0x8b, 0x71, 0x5d, 0x2c, 0x0b, 0x4a, 0x9e, 0x33,
]);

/// Derives the shared grouping key for every footprint decomposed from
/// one container. Stable across re-imports of the same locator, so
/// `edge_rebuild` keeps clustering them.
///
/// The name says `session` because the card parser routes the value
/// into `external_session_key` for dialogue greetings and into
/// `bundle_id` for everything else; treat it as an opaque key rather
/// than a Session id.
pub fn session_id_for(locator: &str) -> String {
    uuid::Uuid::new_v5(&BUNDLE_NS, locator.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_deterministic() {
        let a = session_id_for("/tmp/img.png");
        let b = session_id_for("/tmp/img.png");
        assert_eq!(a, b);
        assert_ne!(a, session_id_for("/tmp/other.png"));
    }

    /// The derivation, frozen as a literal.
    ///
    /// The 16 namespace bytes moved here verbatim from `png_text`, so
    /// the move itself changed nothing; what this pins is the future. A
    /// literal rather than a re-derivation from [`BUNDLE_NS`], because
    /// re-deriving would agree with itself whatever the bytes became —
    /// and the failure that matters is silent: every container imported
    /// before such an edit would land in a second bundle of one, with
    /// nothing raising.
    #[test]
    fn the_namespace_is_frozen() {
        assert_eq!(
            session_id_for("/tmp/img.png"),
            "c369cfd9-49d6-52b6-b734-4430d57a35b2"
        );
    }
}
