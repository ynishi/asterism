//! Snapshot `content_hash` — the canonical member-set fingerprint.
//!
//! A Snapshot is a pure content object (a git-tree
//! analogue): `UNIQUE(persona_id, content_hash)` is the
//! whole dedupe story, so every producer (the V19 migration backfill,
//! the dispatch freeze, the promote path, the re-dispatch reuse lookup)
//! must derive the hash from the member list the same way. This module
//! is that single definition.
//!
//! # Canonical form
//!
//! SHA-256 over the **ordered** member asset ids in their wire string
//! form (lowercase hyphenated UUID), each followed by a `\n` separator,
//! hex-encoded lowercase. Order is part of the identity — the same set
//! in a different order is a different snapshot, because `position` is
//! frozen content.

use sha2::{Digest, Sha256};

/// Hashes the ordered member ids into the snapshot `content_hash`.
///
/// `ordered_ids` must already be in frozen `position` order and in the
/// id wire form (`AssetId::to_string()`); the `\n` terminator after each
/// id makes the encoding prefix-free so `["ab", "c"]` and `["a", "bc"]`
/// can never collide.
pub fn content_hash<'a, I>(ordered_ids: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut hasher = Sha256::new();
    for id in ordered_ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        // Infallible for String.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_order_sensitive() {
        let a = content_hash(["one", "two"]);
        let b = content_hash(["one", "two"]);
        let c = content_hash(["two", "one"]);
        assert_eq!(a, b, "same ordered list → same hash");
        assert_ne!(a, c, "order is part of the identity");
    }

    #[test]
    fn empty_list_hashes() {
        // An empty snapshot is legal (a dispatch of zero members is not,
        // but the hash fn does not police that) and must be stable.
        assert_eq!(content_hash([]), content_hash([]));
        assert_ne!(content_hash([]), content_hash(["x"]));
    }

    #[test]
    fn separator_is_prefix_free() {
        assert_ne!(content_hash(["ab", "c"]), content_hash(["a", "bc"]));
    }

    #[test]
    fn hex_shape() {
        let h = content_hash(["x"]);
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
