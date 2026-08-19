//! `store` — the team-side view of instance-owned blobs, and the
//! declared-digest contract (#83 §3).
//!
//! The store proper is a global CAS the instance owns; what this
//! module types is the layer everything consults instead of bytes:
//!
//! - [`TeamBlobLink`] — the visibility AND dedupe boundary. A digest
//!   "exists" for a caller iff a link row sits in a team they belong
//!   to; purge scope is a team's links, never the CAS directly.
//! - [`Locator`] — a *private* link to content the instance does not
//!   hold: a uri, and at most a digest **hint** from the last
//!   sighting. The instance guarantees nothing about it.
//! - The promotion verification rule — [`verify_declared_digest`],
//!   whose two outcomes are the whole point of this module.
//!
//! # Digest notation is `asterism-core`'s, reused as-is
//!
//! Every digest here is the `sha256:`-prefixed storage form defined by
//! `asterism_core::domain::content_hash` (and the contract crate under
//! it). [`parse_digest`] delegates to that parser rather than
//! re-spelling the grammar — one notation, one set of shape rules,
//! across the local app and the teams plane (#83 §3: "core digest
//! reused, `sha256:` prefix kept; strip only at the path-mapping
//! edge").

use asterism_core::domain::content_hash;
use asterism_core::domain::duplicate_conflict::DuplicateAxis;
use uuid::Uuid;

use crate::error::DomainError;

/// Validates a whole-file digest in the shared notation and hands back
/// its canonical string.
///
/// Delegates the shape rules (`sha256:` + 64 lowercase hex) to
/// `asterism-core`'s parser, then narrows to the artefact axis: the
/// teams store addresses whole-file bytes only, so a content-region
/// (`cr1-sha256:`) or metadata (`m1-sha256:`) digest — valid claims in
/// the local app — are refused here rather than silently admitted into
/// a CAS keyed by file digests.
pub fn parse_digest(raw: &str) -> Result<String, DomainError> {
    let value = content_hash::parse_declaration(raw).map_err(|e| {
        DomainError::Validation(format!("not a digest the teams store accepts: {e}"))
    })?;
    match content_hash::axis_of(&value) {
        Some(DuplicateAxis::Artefact) => Ok(value),
        _ => Err(DomainError::Validation(format!(
            "digest {value:?} is not a whole-file (\"{}\") digest; the teams \
             store addresses blobs by file bytes only",
            content_hash::DIGEST_PREFIX
        ))),
    }
}

/// One `(team, digest)` row — the boundary that makes a blob visible.
///
/// The CAS holds one physical copy per instance; this row is what
/// makes that copy *exist* for a team's members, and removing a team's
/// rows is what purge means (the bytes go when no team links them —
/// swept async, registry-GC shape). Kept as a validated pair rather
/// than two loose fields so a link row cannot be minted around the
/// digest grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamBlobLink {
    team_id: Uuid,
    digest: String,
}

impl TeamBlobLink {
    /// Builds a link row, validating the digest through
    /// [`parse_digest`].
    pub fn new(team_id: Uuid, digest: &str) -> Result<Self, DomainError> {
        Ok(Self {
            team_id,
            digest: parse_digest(digest)?,
        })
    }

    /// The team the blob is visible to.
    pub fn team_id(&self) -> Uuid {
        self.team_id
    }

    /// The digest the link addresses.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// A private-space link: where a user last saw some content, outside
/// the instance's custody (#83 §3).
///
/// The digest field is named `digest_hint` and typed `Option` because
/// both facts are the contract: there may not be one, and when there
/// is, it is an **index hint from the last sighting — never a
/// verification source**. Nothing may compare content against it and
/// call the result verified; verification is what promotion's declared
/// digest exists for ([`verify_declared_digest`]), computed by
/// re-reading the file at promote time. The instance does not
/// guarantee the uri resolves at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    /// Whose private space the link lives in.
    pub user_id: Uuid,
    /// Where the content was seen. Opaque to the domain beyond being
    /// non-empty — resolvability is explicitly not promised.
    pub uri: String,
    /// The digest observed at the last sighting, if any. A hint for
    /// index lookups only; see the type-level doc for why it can
    /// never verify anything.
    pub digest_hint: Option<String>,
    /// When the sighting happened, epoch milliseconds.
    pub seen_at_ms: i64,
}

impl Locator {
    /// Builds a locator. The hint, when present, must still be a
    /// well-formed digest — a hint in a broken notation would poison
    /// index lookups without ever being caught, since nothing later
    /// verifies it.
    pub fn new(
        user_id: Uuid,
        uri: impl Into<String>,
        digest_hint: Option<&str>,
        seen_at_ms: i64,
    ) -> Result<Self, DomainError> {
        let uri = uri.into();
        if uri.trim().is_empty() {
            return Err(DomainError::Validation(
                "locator uri is blank; a locator that points nowhere is not a sighting".into(),
            ));
        }
        if seen_at_ms < 0 {
            return Err(DomainError::Validation(format!(
                "seen_at_ms {seen_at_ms} predates the epoch"
            )));
        }
        let digest_hint = digest_hint.map(parse_digest).transpose()?;
        Ok(Self {
            user_id,
            uri,
            digest_hint,
            seen_at_ms,
        })
    }
}

/// The digest a promotion **declares** — mandatory, computed by the
/// client at promote time by re-reading the file (#83 §3).
///
/// A newtype rather than a `String` parameter so "the declaration was
/// parsed" is a fact the verification signature can demand: there is
/// no way to hand [`verify_declared_digest`] an unvalidated claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDigest(String);

impl DeclaredDigest {
    /// Parses a declaration in the shared notation.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        Ok(Self(parse_digest(raw)?))
    }

    /// The declared value, canonical form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Proof that a promotion's bytes hashed to what was declared — the
/// **only** way a digest enters the link layer from a promotion.
///
/// The field is private and the sole constructor is
/// [`verify_declared_digest`]'s accept arm. That is the "no
/// accept-new-digest path exists in the type" rule made structural: a
/// caller holding a mismatch cannot build one of these around the
/// computed value, because nothing public takes a computed value alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCopy {
    digest: String,
}

impl VerifiedCopy {
    /// The verified digest — declared and computed, which the
    /// existence of this value says are the same string.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// The declared-digest verification rule (#83 §3), with exactly two
/// outcomes: **accept** (declared equals computed) or **reject the
/// whole op** (they differ — no copy, no link row, no ledger event).
///
/// A mismatch is promote-time TOCTOU: the file changed between the
/// user choosing it and the bytes arriving. The right response is to
/// reject and let the user re-confirm, because a promotion asserts
/// "content X", not "whatever the path holds now" — accepting the new
/// digest would promote bytes nobody looked at under a claim nobody
/// made. That third outcome is not an error case this function
/// returns; it is a function this module refuses to contain, and
/// [`VerifiedCopy`]'s private constructor is what keeps a caller from
/// writing it outside.
///
/// `computed` goes through [`parse_digest`] too: the hasher side is
/// this workspace's own code, but a malformed computed value meeting a
/// well-formed declaration must read as "the comparison is broken",
/// never as an ordinary mismatch a user is told to re-confirm away.
pub fn verify_declared_digest(
    declared: &DeclaredDigest,
    computed: &str,
) -> Result<VerifiedCopy, DomainError> {
    let computed = parse_digest(computed)?;
    if declared.as_str() == computed {
        Ok(VerifiedCopy { digest: computed })
    } else {
        Err(DomainError::DigestMismatch {
            declared: declared.as_str().to_string(),
            computed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::content_hash::of_bytes;

    #[test]
    fn a_matching_declaration_is_accepted_and_carries_the_digest() {
        let digest = of_bytes(b"promoted bytes");
        let declared = DeclaredDigest::parse(&digest).unwrap();

        let verified = verify_declared_digest(&declared, &digest).unwrap();
        assert_eq!(verified.digest(), digest);
    }

    #[test]
    fn a_mismatch_rejects_the_whole_op_with_both_sides_named() {
        let declared = DeclaredDigest::parse(&of_bytes(b"what the user chose")).unwrap();
        let computed = of_bytes(b"what the path held at upload");

        match verify_declared_digest(&declared, &computed) {
            Err(DomainError::DigestMismatch {
                declared: d,
                computed: c,
            }) => {
                assert_eq!(d, declared.as_str());
                assert_eq!(c, computed);
            }
            other => panic!("expected DigestMismatch, got {other:?}"),
        }
        // There is no accept-new-digest to assert against: the type
        // has no path from `computed` to a `VerifiedCopy` except
        // through equality above, which is the design being pinned.
    }

    #[test]
    fn the_store_admits_only_whole_file_digests() {
        let digest = of_bytes(b"bytes");
        assert!(DeclaredDigest::parse(&digest).is_ok());
        assert!(TeamBlobLink::new(Uuid::now_v7(), &digest).is_ok());

        // Valid notations in the local app that are still not
        // whole-file digests, plus plain malformations.
        for wrong in [
            format!("cr1-sha256:{}", "a".repeat(64)),
            format!("m1-sha256:{}", "a".repeat(64)),
            "unhashable:no-bytes".to_string(),
            "a".repeat(64), // bare hex — no algorithm tag
            format!("sha256:{}", "A".repeat(64)),
            String::new(),
        ] {
            assert!(
                matches!(parse_digest(&wrong), Err(DomainError::Validation(_))),
                "{wrong:?} must not be admitted"
            );
        }
    }

    #[test]
    fn a_locator_hint_is_optional_and_validated_but_named_a_hint() {
        let user = Uuid::now_v7();
        let digest = of_bytes(b"seen once");

        let with_hint = Locator::new(user, "file:///tmp/thing.png", Some(&digest), 1).unwrap();
        assert_eq!(with_hint.digest_hint.as_deref(), Some(digest.as_str()));

        let without = Locator::new(user, "file:///tmp/thing.png", None, 1).unwrap();
        assert_eq!(without.digest_hint, None);

        assert!(Locator::new(user, "  ", None, 1).is_err());
        assert!(Locator::new(user, "file:///x", Some("bare-hex"), 1).is_err());
        assert!(Locator::new(user, "file:///x", None, -1).is_err());
    }
}
