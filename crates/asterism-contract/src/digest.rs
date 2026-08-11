//! The notation a digest is written in — the `sha256:` tag, and the
//! hasher that produces a value carrying it.
//!
//! # Why the grammar sits in the contract crate
//!
//! [`AddAssetCommand::declared_content_hash`](crate::command::AddAssetCommand::declared_content_hash)
//! is a field of this crate, and the grammar of a field's value belongs
//! with the field: a caller that may state a digest has to be able to
//! spell one without reaching for anything else.
//!
//! The caller that needs it most is an importer.
//! `asterism-importer-sdk` depends on exactly one Asterism crate — this
//! one — and that is the whole point of it: an importer states where
//! bytes are and what it found in them, and pointing it at
//! `asterism-core` would put the entire domain (repositories, services,
//! duplicate axes) behind a plugin whose job is to read files. The two
//! alternatives were both worse. A second `sha256:` and a second
//! `of_bytes` in the SDK is the two-crates-one-predicate shape the
//! `is_duplicate_error` family had before it was deleted — one rule,
//! two spellings, kept in step by whoever remembers. An SDK → core
//! dependency is a layering inversion that no later edit undoes.
//!
//! There is a verbatim precedent one file away: `chrono` was moved into
//! this crate's dependencies "so the SDK can drop its self-defined
//! Derived and the core can consume the shared shape without pulling
//! the SDK" (`Cargo.toml`). Same shape, same reason, same direction of
//! travel.
//!
//! # What deliberately did *not* come with it
//!
//! Only the notation moved. What a stored value **means** is domain and
//! stayed in `asterism_core::domain::content_hash`: the markers
//! (`unhashable:no-bytes`, the `unsupported:` family), the reserved
//! values, `is_duplicate_key`, which axis a value belongs to, and the
//! two versioned tags (`cr1-sha256:`, `m1-sha256:`) that the container
//! walkers produce. Core re-exports the three names below, so nothing
//! on its side spells a different import than it did before.
//!
//! An importer can therefore say "these bytes hash to this" and cannot
//! say "and that makes them a duplicate" — which is the correct
//! division: a declaration is an unverified assertion, and the rules
//! that read digests as sameness run where the bytes were actually
//! measured.

use sha2::{Digest, Sha256};

/// Algorithm tag and separator on every digest this module produces —
/// the storage form's prefix, written in one place.
///
/// A constant rather than a literal at each site because the prefix is
/// read in several languages: the domain's duplicate-key rule, the
/// `WHERE` clause the duplicate report hands to SQLite, and the
/// declaration parser that decides which axis a caller named. A literal
/// spelled out on each side is a rule that can be half-changed.
pub const DIGEST_PREFIX: &str = "sha256:";

/// Incremental hasher over an artefact's bytes.
///
/// Incremental because the caller streams: an original can be a 4 GB
/// video, and reading it into memory to hash it would be the largest
/// allocation in the process by two orders of magnitude.
#[derive(Debug, Default)]
pub struct ContentHasher {
    inner: Sha256,
}

impl ContentHasher {
    /// Starts a fresh hash.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the next chunk of the artefact.
    pub fn update(&mut self, chunk: &[u8]) {
        self.inner.update(chunk);
    }

    /// Finishes, returning the storage form (`sha256:<lowercase hex>`).
    pub fn finish(self) -> String {
        let digest = self.inner.finalize();
        let mut out = String::with_capacity(DIGEST_PREFIX.len() + digest.len() * 2);
        out.push_str(DIGEST_PREFIX);
        for byte in digest {
            use std::fmt::Write;
            // Infallible for String.
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

/// Hashes a whole slice — the convenience form for callers that already
/// hold the bytes (an importer that just read a file, tests, small
/// in-memory artefacts).
pub fn of_bytes(bytes: &[u8]) -> String {
    let mut hasher = ContentHasher::new();
    hasher.update(bytes);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_same_value_different_bytes_different_value() {
        assert_eq!(of_bytes(b"star"), of_bytes(b"star"));
        assert_ne!(of_bytes(b"star"), of_bytes(b"stars"));
    }

    #[test]
    fn value_carries_its_algorithm_and_a_full_digest() {
        let value = of_bytes(b"star");
        let (algorithm, hex) = value.split_once(':').expect("prefixed");
        assert_eq!(algorithm, "sha256");
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    #[test]
    fn streaming_in_chunks_matches_hashing_it_whole() {
        let mut streamed = ContentHasher::new();
        streamed.update(b"a starfield ");
        streamed.update(b"in three parts");
        assert_eq!(streamed.finish(), of_bytes(b"a starfield in three parts"));
    }

    #[test]
    fn empty_input_still_hashes() {
        // An empty file is a legitimate original, and two of them are
        // legitimately duplicates of each other — whether that counts
        // as sameness is a domain question decided elsewhere.
        assert_eq!(of_bytes(b""), of_bytes(b""));
        assert_ne!(of_bytes(b""), of_bytes(b"\0"));
    }
}
