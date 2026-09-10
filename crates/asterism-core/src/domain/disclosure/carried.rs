//! What a file currently carries, and whether it still stands up.
//!
//! The read side of what [`outcome`](super::outcome) says for the write
//! side. `Stamped` says what an application put into a file; this says
//! what is in one now, which is a different question with a different
//! set of answers — a file nobody here ever stamped has an answer, and
//! a file stamped last week may have stopped matching its own bytes
//! since.
//!
//! # Two axes, and folding them is the trap
//!
//! *Does the mark still describe these bytes* and *is the signer
//! somebody we trust* are separate questions, and the second one is not
//! about the file at all. C2PA says so itself: a manifest whose only
//! failure is `signingCredential.untrusted` stays in the `Valid`
//! validation state, and the reference implementation filters that
//! status out of the summary a person reads.
//!
//! The reason to keep them apart here is sharper than tidiness. In the
//! reference implementation trust checking is on by default while the
//! anchor set is empty outside its own test configuration, so a release
//! build reports **every** signer as untrusted — a genuine
//! conformance-program certificate included — until anchors are
//! configured. A verdict that folded trust into integrity would show a
//! correctly signed file as broken in production and fine under `cargo
//! test`, which is the shape of bug that survives a test suite.
//!
//! So [`Mark`] answers only for the bytes, [`Signer`] answers only for
//! the certificate, and [`Carried`] holds both without mixing them.
//!
//! # Why an absent mark is a value and not a `None`
//!
//! "This file carries nothing" is an answer somebody acts on — it is
//! the row that says *re-apply from the database* — and it is not the
//! same as "nobody looked". The series key already records that
//! distinction for a different question, and the reasoning carries: an
//! `Option` would spend the one shape that means "unasked" on a state
//! the reader deliberately reached.

/// What one kind of mark says about the bytes it sits in.
///
/// The integrity axis, and only that. Whether the signature came from
/// somebody the deployment trusts is [`Signer`], and no variant here
/// moves when that answer changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// The file carries no mark of this kind.
    ///
    /// Never stamped, or stamped and then stripped — a re-encode that
    /// does not preserve metadata leaves exactly this. Not an error and
    /// not a failure: it is the state that says the database is the
    /// only copy of the statement, which is the premise the write side
    /// is built on.
    Absent,
    /// A mark is present and the bindings it carries match the bytes.
    Intact,
    /// A mark is present and its bindings no longer match the bytes.
    ///
    /// The bytes changed after the mark was made. What the mark says
    /// may still be true, but nothing here can say so — the statement
    /// and the artefact have come apart, and the answer is to re-derive
    /// rather than to read it.
    ///
    /// Carries words rather than a typed cause, on the same terms
    /// [`Half::Failed`](super::outcome::Half::Failed) does: a caller
    /// can log a sentence, and the causes need no enum here to be
    /// acted on.
    Broken(String),
    /// A mark is present, carrying a failure this build does not know.
    ///
    /// The variant that keeps the mapping honest against an open set. A
    /// validator reports failures by code, new codes arrive with new
    /// versions, and a build that treated an unrecognised one as
    /// success would answer `Intact` about a file it did not understand
    /// — a fabricated fact rather than a missing one.
    Unrecognised(String),
    /// A mark is present, the question was asked, and it cannot be
    /// answered.
    ///
    /// Not the same as [`Unrecognised`](Self::Unrecognised): the
    /// failure is one this build knows, and knowing it is what
    /// establishes that it settles nothing. The case that produces it
    /// is on [`SIGNATURE_MISMATCH`] — one code covering both a forgery
    /// and an upstream defect, with nothing in the report to separate
    /// them when the signer has no name.
    ///
    /// "The bytes changed" and "I cannot tell whether the bytes
    /// changed" lead somewhere different, which is this module's own
    /// argument about `Option` applied one step further in. Carries the
    /// reason as words, for the same reason [`Broken`](Self::Broken)
    /// does.
    Undetermined(String),
}

impl Mark {
    /// Whether a mark of this kind is in the file at all.
    ///
    /// True for every state but [`Absent`](Self::Absent), including the
    /// ones that say the mark does not stand: a broken mark is still a
    /// mark, and a caller deciding whether to write one needs that
    /// distinction rather than "is it fine".
    pub fn present(&self) -> bool {
        !matches!(self, Self::Absent)
    }

    /// Whether this mark describes the bytes it sits in.
    ///
    /// [`Intact`](Self::Intact) alone. Neither
    /// [`Unrecognised`](Self::Unrecognised) nor
    /// [`Undetermined`](Self::Undetermined) is folded in — an
    /// unanswered question is not an answer, which is the whole reason
    /// both exist.
    pub fn stands(&self) -> bool {
        matches!(self, Self::Intact)
    }
}

/// What the deployment can say about who signed.
///
/// The trust axis. It says nothing about the bytes, and a value here
/// never justifies a [`Mark`] — the two are read together by a person
/// and never collapsed into one verdict by this code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    /// There is no signature to have an opinion about.
    ///
    /// A file carrying no manifest, or carrying only an XMP packet,
    /// which is unsigned by construction.
    None,
    /// Trust was not evaluated.
    ///
    /// The honest answer when the deployment has configured no anchors:
    /// with an empty anchor set every certificate fails a trust check,
    /// so running one and reporting the result would answer "untrusted"
    /// about every file in the library, including the correctly signed
    /// ones. Not evaluating and saying so is the difference between a
    /// missing fact and a wrong one.
    NotEvaluated,
    /// A trust list was consulted and the signer is not on it.
    ///
    /// Says nothing about the bytes, and it is the answer every file
    /// gets from a build with no anchors configured — measured, not
    /// assumed: both certificates in
    /// `what_the_sdk_reports_for_this_builds_own_signatures` come back
    /// on this axis. The mark beside it is [`Mark::Intact`] when the
    /// certificate carries an organisation and
    /// [`Mark::Undetermined`] when it does not, which is the upstream
    /// defect rather than a fact about either file.
    Untrusted,
    /// A trust list was consulted and the signer is on it.
    Trusted,
}

/// What one file carries, on both axes, for both kinds of mark.
///
/// The XMP packet and the C2PA manifest are reported separately, the
/// way [`Stamped`](super::outcome::Stamped) reports the two halves it
/// wrote: they fail independently, they are written by different code
/// against different container rules, and a caller that could only see
/// "the file is marked" could not tell a stripped manifest from a
/// stripped packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    /// Whether the file carries an XMP packet, and whether it could be
    /// read.
    ///
    /// A packet carries no binding of its own — nothing in it commits
    /// to the bytes around it — so [`Mark::Intact`] here says the
    /// packet is present and parsed, and no more than that. What
    /// establishes that a packet still describes its file is the
    /// manifest signed over both, which is why the write side writes
    /// the packet first and why that verdict is reported on
    /// [`manifest`](Self::manifest) rather than duplicated here. A
    /// packet that is present and will not parse is
    /// [`Mark::Unrecognised`].
    pub xmp: Mark,
    /// What the C2PA manifest says about the bytes.
    pub manifest: Mark,
    /// What can be said about the manifest's certificate.
    ///
    /// [`Signer::None`] whenever [`manifest`](Self::manifest) is
    /// [`Mark::Absent`] — there is no certificate to ask about — and
    /// the type does not enforce that, because a reader that found a
    /// manifest it could not parse has a real answer for one field and
    /// not the other.
    pub signer: Signer,
}

impl Carried {
    /// A file that carries nothing.
    ///
    /// The common answer, and a legitimate one: most files in a library
    /// were never stamped. Written as a constructor rather than left to
    /// callers so that "nothing here" is one value rather than three
    /// fields somebody assembles the same way each time.
    pub fn nothing() -> Self {
        Self {
            xmp: Mark::Absent,
            manifest: Mark::Absent,
            signer: Signer::None,
        }
    }

    /// Whether the file carries any mark at all.
    pub fn discloses(&self) -> bool {
        self.xmp.present() || self.manifest.present()
    }

    /// Whether every mark the file carries describes its bytes.
    ///
    /// A file carrying nothing answers `true`, and the pairing with
    /// [`discloses`](Self::discloses) is deliberate: "nothing here is
    /// wrong" and "there is something here" are the two questions, and
    /// a single verdict that tried to answer both would have to decide
    /// which of them an unmarked file fails.
    pub fn stands(&self) -> bool {
        !matches!(
            self.xmp,
            Mark::Broken(_) | Mark::Unrecognised(_) | Mark::Undetermined(_)
        ) && !matches!(
            self.manifest,
            Mark::Broken(_) | Mark::Unrecognised(_) | Mark::Undetermined(_)
        )
    }
}

/// Validation status codes that say the bytes changed after signing.
///
/// The hard binding: a hash taken over the asset's own bytes, which is
/// the only thing in a validation report that answers the integrity
/// question. `assertion.dataHash.mismatch` is the general case and
/// `assertion.bmffHash.mismatch` its BMFF (MP4, MOV) counterpart.
pub const BINDING_FAILURES: &[&str] =
    &["assertion.dataHash.mismatch", "assertion.bmffHash.mismatch"];

/// The validation status code that belongs to the trust axis rather
/// than to the bytes.
///
/// `signingCredential.untrusted` is the expected verdict on a
/// certificate with no anchor behind it, and one C2PA's own validation
/// state does not treat as invalidating. It is evidence about a trust
/// list, not about a file.
///
/// One code rather than a list, and the list it replaced is the finding
/// worth keeping: `claimSignature.mismatch` sat here too, on the
/// reasoning that `c2pa` sets it from the trust check rather than from
/// verifying the signature bytes. That is measurably not what it means.
/// A file this repository signs under a certificate carrying an
/// `organizationName` comes back with this code and no other, while the
/// same file under a certificate without one carries both — so the
/// mismatch tracks the missing attribute, not the trust verdict, and
/// reading it as a trust verdict would let a forged claim signature
/// over untouched asset bytes come back intact. The measurement is
/// `what_the_sdk_reports_for_this_builds_own_signatures` in
/// `asterism-infra`'s disclosure tests; what the code actually is, and
/// why it cannot be decided on its own, is on [`SIGNATURE_MISMATCH`].
pub const TRUST_FAILURES: &[&str] = &["signingCredential.untrusted"];

/// The code two different things produce, and the reason this mapping
/// needs more than the codes.
///
/// A genuine forgery emits `claimSignature.mismatch`. So does a
/// perfectly good signature under a certificate whose subject carries
/// no `organizationName`: `c2pa` 0.90.12 fetches the organisation for
/// display *after* the signature has already verified and turns its
/// absence into an error, which is then folded into this one code
/// (contentauth/c2pa-rs#2262, accepted upstream, fix unmerged; the C2PA
/// specification requires no such attribute).
///
/// Nothing in a validation report separates the two. One field outside
/// it does: the manifest's signature issuer, which is absent exactly
/// when the attribute is, and which `c2pa` reads out of the certificate
/// rather than out of the verification, so it answers even for a
/// signature that did not verify. Measured on this repository's own
/// signed files by
/// `what_the_sdk_reports_for_this_builds_own_signatures` in
/// `asterism-infra`, which pins both certificates' failure lists so
/// that this reasoning fails loudly when the upstream fix lands.
///
/// A named signer therefore rules the upstream defect out and the code
/// means what it says. An unnamed one rules nothing in, and the honest
/// answer is that the question cannot be settled — see
/// [`Mark::Undetermined`].
pub const SIGNATURE_MISMATCH: &str = "claimSignature.mismatch";

/// The integrity verdict for a manifest, from the failure codes a
/// validator reported and whether the signer has a name.
///
/// Which failures are about the bytes and which are about the
/// certificate is domain knowledge, and it was written out in a
/// `#[cfg(test)]` helper in the infrastructure crate, where no
/// production path could reach it. That helper now calls this.
///
/// `signer_named` is the second input the codes cannot supply, and
/// [`SIGNATURE_MISMATCH`] is why it exists: one code covers a forgery
/// and an upstream defect, and only the issuer field tells them apart.
/// Pass whether the manifest's signature carries an issuer name.
///
/// Order matters. A binding failure outranks everything beside it,
/// because a hash taken over the asset's own bytes is the strongest
/// statement available about them; after that an unsettled signature
/// outranks an unrecognised code, because it is the more specific
/// finding.
pub fn integrity_of<'a>(failures: impl IntoIterator<Item = &'a str>, signer_named: bool) -> Mark {
    let mut mismatch = false;
    let mut unrecognised: Option<&str> = None;
    for code in failures {
        if BINDING_FAILURES.contains(&code) {
            return Mark::Broken(code.to_string());
        }
        if code == SIGNATURE_MISMATCH {
            mismatch = true;
            continue;
        }
        if TRUST_FAILURES.contains(&code) {
            continue;
        }
        unrecognised.get_or_insert(code);
    }
    if mismatch {
        return if signer_named {
            Mark::Broken(SIGNATURE_MISMATCH.to_string())
        } else {
            Mark::Undetermined(
                "the claim signature is reported mismatched and the signer has no name, \
                 which this SDK version also does to a valid signature"
                    .to_string(),
            )
        };
    }
    match unrecognised {
        Some(code) => Mark::Unrecognised(code.to_string()),
        None => Mark::Intact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signer with a name, which is the ordinary case and the one
    /// that leaves `claimSignature.mismatch` meaning what it says.
    const NAMED: bool = true;
    /// A signer with none, where that code settles nothing.
    const UNNAMED: bool = false;

    #[test]
    fn no_failures_is_an_intact_mark() {
        assert_eq!(integrity_of([], NAMED), Mark::Intact);
        assert_eq!(integrity_of([], UNNAMED), Mark::Intact);
    }

    /// The code a self-signed certificate produces, and the one this
    /// mapping exists to get right: it is about a trust list rather
    /// than about the bytes, so the mark stands and the verdict belongs
    /// to `Signer`.
    #[test]
    fn an_untrusted_signer_leaves_the_mark_standing() {
        assert_eq!(
            integrity_of(["signingCredential.untrusted"], NAMED),
            Mark::Intact
        );
    }

    #[test]
    fn a_binding_failure_breaks_the_mark() {
        assert_eq!(
            integrity_of(["assertion.dataHash.mismatch"], NAMED),
            Mark::Broken("assertion.dataHash.mismatch".into())
        );
        assert_eq!(
            integrity_of(["assertion.bmffHash.mismatch"], NAMED),
            Mark::Broken("assertion.bmffHash.mismatch".into())
        );
    }

    /// A self-signed file that was also edited reports both kinds of
    /// failure, and the answer about the bytes is the one to give.
    #[test]
    fn a_binding_failure_outranks_the_codes_beside_it() {
        assert_eq!(
            integrity_of(
                [
                    "signingCredential.untrusted",
                    "assertion.dataHash.mismatch",
                    "general.error",
                ],
                UNNAMED
            ),
            Mark::Broken("assertion.dataHash.mismatch".into())
        );
    }

    /// A named signer rules the upstream defect out, so the code means
    /// what it says: this manifest's claim signature did not verify.
    #[test]
    fn a_named_signer_makes_a_signature_mismatch_a_broken_mark() {
        assert_eq!(
            integrity_of([SIGNATURE_MISMATCH], NAMED),
            Mark::Broken(SIGNATURE_MISMATCH.to_string())
        );
    }

    /// The case with no honest answer, and the one a forged claim
    /// signature used to slip through: unnamed signer, mismatched
    /// signature, and nothing in the report to say whether the file was
    /// forged or the SDK is doing this to a valid signature.
    #[test]
    fn an_unnamed_signers_mismatch_is_settled_neither_way() {
        let mark = integrity_of([SIGNATURE_MISMATCH], UNNAMED);
        assert!(
            matches!(mark, Mark::Undetermined(_)),
            "a question that cannot be answered is not an answer: {mark:?}"
        );
        assert!(!mark.stands());
        assert!(mark.present());
        assert_ne!(mark, Mark::Intact, "and never reads as a standing file");
    }

    /// Trust is still its own axis beside an unsettled signature: the
    /// untrusted code does not decide the integrity question either
    /// way.
    #[test]
    fn an_untrusted_unnamed_signer_still_leaves_the_signature_unsettled() {
        assert!(matches!(
            integrity_of(["signingCredential.untrusted", SIGNATURE_MISMATCH], UNNAMED),
            Mark::Undetermined(_)
        ));
    }

    /// The variant that keeps this honest. The codes are an open set,
    /// and a build that read an unknown failure as success would state
    /// that a file it did not understand is fine.
    #[test]
    fn an_unrecognised_failure_is_not_success() {
        assert_eq!(
            integrity_of(["assertion.notAFailureThisBuildKnows"], NAMED),
            Mark::Unrecognised("assertion.notAFailureThisBuildKnows".into())
        );
        assert_ne!(integrity_of(["something.new"], NAMED), Mark::Intact);
        assert!(!integrity_of(["something.new"], NAMED).stands());
    }

    /// An unrecognised code beside a trust code still surfaces: the
    /// trust codes are skipped rather than treated as an answer.
    #[test]
    fn a_trust_code_does_not_hide_an_unrecognised_one() {
        assert_eq!(
            integrity_of(["signingCredential.untrusted", "something.new"], NAMED),
            Mark::Unrecognised("something.new".into())
        );
    }

    #[test]
    fn a_file_carrying_nothing_is_a_verdict_rather_than_an_absence() {
        let carried = Carried::nothing();
        assert!(!carried.discloses());
        assert!(
            carried.stands(),
            "nothing here is wrong with an unmarked file"
        );
        assert!(!carried.xmp.present());
        assert_eq!(carried.signer, Signer::None);
    }

    /// The pair a correctly signed file under an unanchored
    /// certificate produces: intact and untrusted, which is not
    /// "invalid".
    #[test]
    fn intact_and_untrusted_is_a_standing_file() {
        let carried = Carried {
            xmp: Mark::Intact,
            manifest: Mark::Intact,
            signer: Signer::Untrusted,
        };
        assert!(carried.discloses());
        assert!(carried.stands());
    }

    #[test]
    fn a_broken_half_does_not_stand_and_the_other_half_is_unaffected() {
        let carried = Carried {
            xmp: Mark::Intact,
            manifest: Mark::Broken("assertion.dataHash.mismatch".into()),
            signer: Signer::Untrusted,
        };
        assert!(carried.discloses());
        assert!(!carried.stands());
        assert!(carried.xmp.stands(), "the packet is still what it was");
    }
}
