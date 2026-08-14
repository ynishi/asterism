//! What applying a record to a file actually achieved.
//!
//! Here rather than beside the writer that produces it, because two
//! layers need to name it and neither may depend on the other: the port
//! is declared in `asterism-core` (ports live in the core; adapters
//! never define traits) and the implementation lives in
//! `asterism-infra`. This crate is the one both can already see.
//!
//! # Why a failure lives in the value and not only in the error
//!
//! Applying a disclosure is two operations against one file, and either
//! can fail without the other being affected. An error return can say
//! only "the whole thing failed", so a writer that reports a failed
//! manifest that way discards the packet it had already produced —
//! which is how an expired certificate came to withhold the half that
//! needs no certificate at all.
//!
//! So the two halves report their own outcome, including their own
//! failure, and the error channel is left for the case where nothing
//! could be attempted: the file could not be read, or its container is
//! one this build does not write into. What a caller does with a
//! half that failed is the caller's decision — a manifest that did not
//! land while the packet did is still a disclosed file, and calling
//! that an export failure is the judgement this type exists to let
//! somebody else make.

/// What became of one half of a disclosure.
///
/// Three states rather than a `bool`, because "not written" is at least
/// three different answers and they lead somewhere different. A video
/// cannot carry an XMP packet and never will; a build with no
/// certificate configured is doing exactly what it was configured to
/// do; a certificate that expired last night is a fault somebody has to
/// go and fix. A boolean reports all three as `false` and leaves the
/// caller to work out which — the shape the digest axes already refuse
/// (`MaterialMeta`, `MetaRaw`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Half {
    /// The mark was written into the file.
    Written,
    /// Nothing was attempted, and that is not a fault.
    Skipped(Skipped),
    /// It was attempted and it failed. The other half is unaffected.
    ///
    /// Carries words rather than a typed cause. The causes belong to
    /// the adapter — a JUMBF box, an expired certificate, a JPEG
    /// segment that will not hold a packet — and naming them here would
    /// put the container formats into the crate that deliberately does
    /// not know them. What a caller can act on is that this half did
    /// not land, and why, in a sentence it can log.
    Failed(String),
}

impl Half {
    /// Whether this half put a mark in the file.
    pub fn written(&self) -> bool {
        matches!(self, Self::Written)
    }

    /// This half, as the value a caller records beside the artefact.
    ///
    /// Here rather than at the call site because the shape and the
    /// meaning are the same question: a reader deciding whether to
    /// re-apply needs to tell "no certificate was configured" from "the
    /// certificate stopped working", and that distinction is this
    /// type's, not the recording caller's.
    ///
    /// An object rather than a string so a reason can be added without
    /// re-parsing what was written before — `{"state": "skipped",
    /// "reason": "no_signing_identity"}` reads the same to something
    /// that only knows about `state`.
    pub fn to_note(&self) -> serde_json::Value {
        match self {
            Self::Written => serde_json::json!({ "state": "written" }),
            Self::Skipped(why) => serde_json::json!({
                "state": "skipped",
                "reason": why.as_str(),
            }),
            Self::Failed(cause) => serde_json::json!({
                "state": "failed",
                "cause": cause,
            }),
        }
    }

    /// The failure, when this half was attempted and failed.
    pub fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed(cause) => Some(cause),
            _ => None,
        }
    }
}

/// Why a half was not attempted.
///
/// Every variant is a state the application is meant to be able to be
/// in. None of them is a fault, which is what separates this from
/// [`Half::Failed`] — the distinction a caller needs to decide whether
/// to tell anybody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// The record asserts nothing, so there is no mark to write.
    ///
    /// The honest answer for an asset whose container carried no
    /// generator or camera evidence: the mapping refuses to state a
    /// term it cannot establish, and a file that gets no mark because
    /// there was nothing to say is not a failed stamp.
    NothingToDisclose,
    /// This container cannot carry this half.
    ///
    /// MP4 and MOV take a manifest and no packet. The gap is real and
    /// is described in the adapter's module docs; saying so here is
    /// what stops it reading as a failure in a log.
    ContainerCannotCarryIt,
    /// No signing identity is configured.
    ///
    /// The state every install starts in, and a supported one. An
    /// untrusted manifest is worse than no manifest, so a build with no
    /// certificate writes the packet and stops — deliberately, not for
    /// want of trying.
    NoSigningIdentity,
}

impl Skipped {
    /// The reason, as it is recorded and read back.
    ///
    /// A stable string rather than a `Debug` rendering: it goes into a
    /// row somebody will query on, so renaming the variant must not
    /// silently rename the stored value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NothingToDisclose => "nothing_to_disclose",
            Self::ContainerCannotCarryIt => "container_cannot_carry_it",
            Self::NoSigningIdentity => "no_signing_identity",
        }
    }
}

/// The result of writing a [`DisclosureRecord`](crate::DisclosureRecord)
/// into a file.
///
/// Every field is an outcome rather than an intention. The two halves
/// fail independently — a container may take an XMP packet and no
/// manifest, or a manifest and no packet — so a caller that recorded
/// "provenance applied" would be recording something no particular file
/// necessarily has.
///
/// There is no `Default`. An outcome that says nothing happened, and
/// does not say why, is precisely the value this type was reshaped to
/// remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    /// What became of the IPTC/XMP packet.
    pub xmp: Half,
    /// What became of the signed C2PA manifest.
    pub manifest: Half,
    /// The prompt was dropped to fit the packet into a JPEG segment.
    ///
    /// Reported because it cannot be recovered afterwards: a file whose
    /// prompt did not fit and a file that never had one are
    /// indistinguishable once written, so the difference has to leave
    /// the call that made it. Only meaningful when [`Self::xmp`] is
    /// [`Half::Written`] — nothing else writes a packet to drop it
    /// from.
    pub prompt_dropped: bool,
}

impl Stamped {
    /// An outcome with both halves accounted for and no prompt dropped.
    pub fn new(xmp: Half, manifest: Half) -> Self {
        Self {
            xmp,
            manifest,
            prompt_dropped: false,
        }
    }

    /// Whether the file carries a machine-readable mark of any kind.
    ///
    /// The question the obligation asks, and deliberately not "did
    /// everything succeed". One mark is a disclosure; reporting a
    /// partial application as a failure would push a caller towards
    /// treating a marked file as unmarked.
    pub fn discloses(&self) -> bool {
        self.xmp.written() || self.manifest.written()
    }

    /// Every half that was attempted and failed.
    ///
    /// What the caller decides on. An empty list does not mean both
    /// halves landed — a half that was skipped is neither a failure nor
    /// a mark — which is why this is read beside
    /// [`discloses`](Self::discloses) rather than instead of it.
    pub fn failures(&self) -> Vec<&str> {
        [&self.xmp, &self.manifest]
            .into_iter()
            .filter_map(Half::failure)
            .collect()
    }

    /// This outcome, as the value a caller records beside the artefact.
    ///
    /// Carries no timestamp: when it happened is the recording caller's
    /// to add, because this type is produced in places that do not
    /// write anything down.
    ///
    /// [`discloses`](Self::discloses) is written out rather than left
    /// to be re-derived. It is the question the obligation asks, and a
    /// reader that computed it from the two halves would be a second
    /// place the rule lives.
    pub fn to_note(&self) -> serde_json::Value {
        serde_json::json!({
            "xmp": self.xmp.to_note(),
            "manifest": self.manifest.to_note(),
            "prompt_dropped": self.prompt_dropped,
            "discloses": self.discloses(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_mark_on_its_own_is_a_disclosure() {
        assert!(
            !Stamped::new(
                Half::Skipped(Skipped::NothingToDisclose),
                Half::Skipped(Skipped::NoSigningIdentity)
            )
            .discloses()
        );
        assert!(Stamped::new(Half::Written, Half::Skipped(Skipped::NoSigningIdentity)).discloses());
        assert!(
            Stamped::new(
                Half::Skipped(Skipped::ContainerCannotCarryIt),
                Half::Written
            )
            .discloses()
        );
    }

    #[test]
    fn a_failed_half_does_not_cancel_a_written_one() {
        let outcome = Stamped::new(
            Half::Written,
            Half::Failed("the certificate expired".into()),
        );

        // The file is disclosed. Whether that counts as a failed export
        // is the caller's call, and it has both facts to make it with.
        assert!(outcome.discloses());
        assert_eq!(outcome.failures(), vec!["the certificate expired"]);
    }

    #[test]
    fn the_note_keeps_apart_what_the_type_keeps_apart() {
        // The reason a reader re-applies or does not: "no certificate
        // was configured" and "the certificate stopped working" are
        // different answers, and a note that flattened both to `false`
        // would be the boolean this type replaced.
        let unconfigured = Stamped::new(Half::Written, Half::Skipped(Skipped::NoSigningIdentity));
        let broken = Stamped::new(
            Half::Written,
            Half::Failed("the certificate expired".into()),
        );

        assert_eq!(
            unconfigured.to_note()["manifest"],
            serde_json::json!({ "state": "skipped", "reason": "no_signing_identity" })
        );
        assert_eq!(
            broken.to_note()["manifest"],
            serde_json::json!({ "state": "failed", "cause": "the certificate expired" })
        );
        // Both disclosed the file: the XMP half landed either way.
        assert_eq!(unconfigured.to_note()["discloses"], serde_json::json!(true));
        assert_eq!(broken.to_note()["discloses"], serde_json::json!(true));
    }

    #[test]
    fn a_skipped_half_is_not_a_failure() {
        let outcome = Stamped::new(
            Half::Skipped(Skipped::ContainerCannotCarryIt),
            Half::Skipped(Skipped::NoSigningIdentity),
        );

        assert!(outcome.failures().is_empty());
        assert!(!outcome.discloses());
    }
}
