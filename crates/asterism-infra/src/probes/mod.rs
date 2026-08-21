//! The probes this build has, and the one question a caller asks of all
//! of them at once.
//!
//! Each probe is one format's reading of the two walking fingerprint
//! axes ([`ArtefactProbe`]); this module is the list of probes, and
//! which formats each one answers for is that probe's own statement
//! ([`ArtefactProbe::declares`]) rather than a second list kept here. So
//! a new format is one implementation and one line below, and nothing in
//! `asterism-core` is edited when it arrives.
//!
//! That is what the format costs for the next file read, not for the
//! library already on disk. Rows imported before the probe landed hold
//! the `unsupported` status with the format in the reason column, and
//! that is a final answer to "has anybody looked"
//! ([`is_axis_answer`](asterism_core::domain::content_hash::is_axis_answer)),
//! so the ordinary fingerprint pass never offers them again: new imports
//! would take digests while the rows that were there first keep the
//! status, and one axis would hold two meanings with nothing to tell
//! them apart. A format therefore also arrives owing a way back to the
//! rows it was refused on, **per axis and per column**, since a probe
//! may claim one of them a slice before the other — which is what JPEG
//! did.
//!
//! Two shapes have been used for that, and which one a case wants is
//! decided by how much reading it costs. A numbered `UPDATE` writing
//! NULL over the one stale marker is the cheap one, and it is what both
//! of JPEG's axes took (V72 for content, V76 for meta): the rows rejoin
//! the ordinary walk, which is already built for reads that must not
//! happen inside a transaction.
//! [`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk)
//! is the other — a second predicate over one column, driven by a
//! migration-chain read — and it stays reserved for the case it was
//! written for, a region definition that is versioned up. It still reads
//! the content column and only that; the meta axis has no equivalent
//! predicate and has not needed one.
//!
//! # Answering for a set of probes
//!
//! The gates are **OR** — a file is read if any probe claims it — and
//! the readings are **first claim wins**, in registration order. That
//! pairing is deliberate: the gate has to be optimistic because it runs
//! before the file is open and its only input is a guess from a
//! filename, while the reading happens with the bytes in hand and can
//! afford to be decided. Two probes claiming one mime **on one axis** is
//! a mistake at registration rather than a case to arbitrate at runtime,
//! and `tests::every_probe_here_is_reachable_through_the_registry` is
//! where it shows up.
//!
//! Per axis, because the axes are counted separately everywhere else
//! ([`ArtefactProbe`]): one probe reading a container's pixels and
//! another reading the metadata alongside it is a legitimate
//! arrangement, and a rule that counted claimants across both axes at
//! once would call it a collision.
//!
//! Nothing claiming the format is not a failure: the row falls back to
//! the file axis, which groups byte-identical copies perfectly well, and
//! the columns say which format was declined rather than pretending the
//! bytes were read.

use asterism_core::domain::content_hash::{CONTENT_DIGEST_PREFIX, ContentHasher, DIGEST_PREFIX};
use asterism_core::domain::content_region::{self, ContentRegion};
use asterism_core::domain::material_meta::{self, MaterialMeta};
use asterism_core::domain::material_meta_raw::MetaRaw;
use asterism_core::domain::probe::{ArtefactProbe, ProbeGates};
use asterism_core::domain::value::MimeType;

pub mod jpeg;
pub mod json;
pub mod png;

/// Every probe this build carries, in the order they are asked.
///
/// A slice of `&'static dyn` rather than a builder or a registry type:
/// the set is fixed at compile time, nothing configures it, and a
/// function returning it is the smallest thing that keeps the list in
/// one place. Neither probe has state, so the value is a constant.
fn probes() -> &'static [&'static dyn ArtefactProbe] {
    const ALL: &[&dyn ArtefactProbe] = &[&png::PngProbe, &jpeg::JpegProbe, &json::JsonProbe];
    ALL
}

/// The same SHA-256, under the content axis's own tag.
///
/// [`ContentHasher`] is the workspace's one hasher and it finishes in
/// the file axis's spelling, so the tag is swapped rather than a second
/// SHA-256 being reached for: the hex is the same hex, and there stays
/// one place in the tree that turns 32 bytes into 64 characters.
/// `content_hash::CONTENT_REGION_EMPTY` is defined by the same swap.
///
/// Here rather than in a probe because there are two probes now, and
/// what they have to agree about is not the swap itself but the value it
/// produces: two spellings of `cr1-` would be two column vocabularies
/// telling a reader nothing about which format wrote a row. A format's
/// judgement about its own container is the part that differs per
/// probe — this is the part that must not.
fn under_region_tag(hasher: ContentHasher) -> String {
    let file_form = hasher.finish();
    let hex = file_form.strip_prefix(DIGEST_PREFIX).unwrap_or(&file_form);
    format!("{CONTENT_DIGEST_PREFIX}{hex}")
}

/// Whether **any** probe reads the content axis for this declared
/// format — the question a caller asks before opening the file.
pub fn walks_content(declared_mime: Option<&MimeType>) -> bool {
    probes().iter().any(|p| p.walks_content(declared_mime))
}

/// Whether **any** probe reads the meta axis for this declared format.
///
/// Asked separately from [`walks_content`], on the terms
/// [`ArtefactProbe`] sets out: they are two definitions, and a single
/// gate would either skip a walk that would have worked or read a file
/// whole for a walk that answers nothing.
pub fn walks_meta(declared_mime: Option<&MimeType>) -> bool {
    probes().iter().any(|p| p.walks_meta(declared_mime))
}

/// The content-axis reading, from the first probe that claims the
/// format.
///
/// When none does, the answer is the same marker the caller would have
/// stored without opening the file at all
/// ([`content_region::unsupported_format`]) — one artefact must not
/// carry two different values depending on which side of a size gate it
/// fell.
pub fn content(bytes: &[u8], declared_mime: Option<&MimeType>) -> ContentRegion {
    match probes().iter().find(|p| p.walks_content(declared_mime)) {
        Some(probe) => probe.content(bytes, declared_mime),
        None => content_region::unsupported_format(declared_mime),
    }
}

/// The meta-axis reading, from the first probe that claims the format.
pub fn meta(bytes: &[u8], declared_mime: Option<&MimeType>) -> MaterialMeta {
    match probes().iter().find(|p| p.walks_meta(declared_mime)) {
        Some(probe) => probe.meta(bytes, declared_mime),
        None => material_meta::unsupported_format(declared_mime),
    }
}

/// The metadata bytes the same probe keeps — the meta axis's other
/// column ([`material_meta_raw`](asterism_core::domain::material_meta_raw)).
///
/// Selected by [`walks_meta`] rather than a gate of its own, so the
/// bytes in a row and the digest in the row beside it come from one
/// probe's reading of one container. The fall-through is
/// [`MetaRaw::Absent`] and not a marker: a format nothing claims has no
/// metadata region, and NULL is what that says.
pub fn meta_raw(bytes: &[u8], declared_mime: Option<&MimeType>) -> MetaRaw {
    match probes().iter().find(|p| p.walks_meta(declared_mime)) {
        Some(probe) => probe.meta_raw(bytes, declared_mime),
        None => MetaRaw::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::probe::{FormatClaim, GateOpen};
    use asterism_core::domain::value::{ImageFormat, MimeType};

    /// The character-card PNG this repo already ships — bytes that
    /// announce themselves as a PNG in their first eight, which is what
    /// the undeclared case below needs in order to prove anything.
    const CARD_PNG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../asterism-importer-sdk/tests/fixtures/character-card-lyra.png"
    ));

    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    /// The completeness rule, as a function of a probe list.
    ///
    /// One argument, not two. The formats a build covers are read off
    /// the probes themselves ([`ArtefactProbe::declares`]), which is the
    /// same place the gates are read from, so there is nothing here for
    /// a registry to disagree with. A second list — the formats this
    /// module *means* to cover — would be the same fact written twice,
    /// and the copy that drifts is the one nothing consults at runtime:
    /// a probe answering for a format the list never named opens the
    /// gate in `fingerprint::hash_artefact` all the same, and every
    /// artefact of that format is read whole, refused on its signature,
    /// and stored under a marker it did not carry yesterday.
    ///
    /// Written as a function returning the complaint rather than as a
    /// body of assertions, because the rule has to be run over
    /// registries this build does not carry. It is a claim about *any*
    /// registry, and the only way to show a claim like that has teeth is
    /// to hand it registries that break it — see
    /// [`the_rule_wants_one_axis_rather_than_both`].
    ///
    /// The three things it asks:
    ///
    /// - **Every registered probe declares a format.** A probe that
    ///   declares none is registered and never asked anything: the
    ///   parser exists, its own tests pass, and nothing routes to it.
    ///   That is invisible from the probe's side and from the column's
    ///   (a marker is a legitimate value), so it is refused here.
    /// - **Every claim has an axis behind it.** At least one, not both.
    ///   A probe that reads a container's metadata and cannot yet say
    ///   which of its bytes are the picture is answering the two
    ///   questions the way [`ArtefactProbe`] says they may be answered —
    ///   separately — and demanding both would leave one shortest way to
    ///   pass: claim the content axis anyway. That claim opens the gate
    ///   in `fingerprint::hash_artefact`, so every artefact of the format
    ///   would be read whole, up to 64 MiB each, on its way to a probe
    ///   with nothing to say about them. A completeness test is not
    ///   worth a corpus-wide `read_to_end`. Neither axis is the other
    ///   failure: a declaration that answers nothing.
    /// - **No format is claimed twice on one axis.** Per axis, since the
    ///   readings are first-claim-wins per axis: a second content
    ///   claimant is dead code that looks live, while a content probe and
    ///   a meta probe over one container are two answers to two
    ///   questions. Counted through the gates rather than the flags,
    ///   because the gates are what the caller will ask.
    fn completeness(registered: &[&dyn ArtefactProbe]) -> Result<(), String> {
        for (at, probe) in registered.iter().enumerate() {
            if probe.declares().is_empty() {
                return Err(format!(
                    "the probe registered at index {at} declares no format, so nothing reaches it"
                ));
            }
            for claim in probe.declares() {
                if !claim.content && !claim.meta {
                    return Err(format!(
                        "{:?} is declared by the probe at index {at} \
                         with neither axis behind it",
                        claim.mime
                    ));
                }
            }
        }

        for claim in registered.iter().flat_map(|probe| probe.declares()) {
            let declared = Some(&claim.mime);
            let content = registered
                .iter()
                .filter(|p| p.walks_content(declared))
                .count();
            let meta = registered.iter().filter(|p| p.walks_meta(declared)).count();
            if content > 1 || meta > 1 {
                return Err(format!(
                    "{:?} is claimed by more than one probe on one axis \
                     (content: {content}, meta: {meta})",
                    claim.mime
                ));
            }
        }

        Ok(())
    }

    /// **Every probe in this file is reachable through the registry, and
    /// on the axes it declared.**
    ///
    /// The failure this catches is writing a probe and not registering
    /// it — the parser exists, its tests pass, and every artefact of its
    /// format quietly stores the `unsupported` status because nothing
    /// ever asks it anything. That is invisible from the probe's own
    /// side and invisible from the row's (`unsupported` is a legitimate
    /// state), so it is checked here.
    ///
    /// Same shape as `repo::asset`'s
    /// `the_three_column_groups_cover_the_table`, which holds a schema
    /// and a set of column lists against each other. What this holds
    /// against each other is a probe's declaration and the registry's
    /// answers: every claim has to reach the free function a caller
    /// asks, on the axes the claim names, rather than falling through to
    /// the marker.
    ///
    /// The declarations themselves are checked by [`completeness`],
    /// which is where an unregistered probe and a probe that declares
    /// nothing both fail.
    #[test]
    fn every_probe_here_is_reachable_through_the_registry() {
        completeness(probes()).expect("this build's probes declare a coherent set of formats");

        // Asked per axis, and only of the axis the claim names: a
        // meta-only probe's format *should* reach the fall-through on
        // the content axis, and that is the answer, not a gap.
        for claim in probes().iter().flat_map(|probe| probe.declares()) {
            let declared = Some(&claim.mime);
            if claim.content {
                assert!(
                    walks_content(declared),
                    "{:?} is declared on the content axis but the registry's gate says no",
                    claim.mime
                );
                assert_ne!(
                    content(&[], declared),
                    content_region::unsupported_format(declared),
                    "{:?} reaches the content fall-through instead of its probe",
                    claim.mime
                );
            }
            if claim.meta {
                assert!(
                    walks_meta(declared),
                    "{:?} is declared on the meta axis but the registry's gate says no",
                    claim.mime
                );
                assert_ne!(
                    meta(&[], declared),
                    material_meta::unsupported_format(declared),
                    "{:?} reaches the meta fall-through instead of its probe",
                    claim.mime
                );
            }
        }
    }

    /// A probe that reads one axis and declines the other — the shape
    /// the next format arrives in, and the shape `jpeg::JpegProbe`
    /// arrived in.
    ///
    /// It is the mirror image of the real one: this reads JPEG's
    /// metadata and declines its picture, where the probe this build
    /// registers reads the picture and declines the metadata. A stub
    /// rather than a second real probe, because what is being measured
    /// is the registry rule — and beside `ContentOnlyJpeg` below it
    /// builds a registry with two claimants on one mime and opposite
    /// axes, which this build's own probes cannot make.
    #[derive(Debug)]
    struct MetaOnlyJpeg;

    /// The same container from the other side: a probe that says which
    /// of a JPEG's bytes are the picture and has no reading of its
    /// metadata.
    ///
    /// Beside [`MetaOnlyJpeg`] it is the registry the per-axis count
    /// exists to **permit** — one mime, two probes, opposite axes —
    /// which is the arrangement a count across both axes would refuse.
    /// Nothing else in this file is that registry: this build's own
    /// probes are one probe answering both axes, where the two rules
    /// agree.
    #[derive(Debug)]
    struct ContentOnlyJpeg;

    /// A probe declaring a format with neither axis behind it — a claim
    /// that answers nothing.
    #[derive(Debug)]
    struct DeafProbe;

    /// A probe that declares nothing at all: registered, routed to by no
    /// format, and asked nothing for as long as it stays that way.
    #[derive(Debug)]
    struct SilentProbe;

    /// JPEG's metadata, and no opinion about which of its bytes are the
    /// picture.
    const JPEG_META_ONLY: &[FormatClaim] = &[FormatClaim {
        mime: MimeType::Image(ImageFormat::Jpeg),
        content: false,
        meta: true,
    }];

    /// Which of a JPEG's bytes are the picture, and no reading of what
    /// is written about it.
    const JPEG_CONTENT_ONLY: &[FormatClaim] = &[FormatClaim {
        mime: MimeType::Image(ImageFormat::Jpeg),
        content: true,
        meta: false,
    }];

    /// JPEG claimed on neither axis. Representable so that the rule has
    /// something to refuse — the type could forbid it, and then the
    /// clause that catches it would be a comment.
    const JPEG_NEITHER_AXIS: &[FormatClaim] = &[FormatClaim {
        mime: MimeType::Image(ImageFormat::Jpeg),
        content: false,
        meta: false,
    }];

    // Every reading in the four impls below answers `EmptySpan`, on
    // declared axes and declined ones alike — deliberately *not* the
    // marker a refusal stores.
    //
    // The stubs exist for `completeness`, which reads declarations and
    // never calls a reading, so on a declared axis the value is
    // arbitrary. On a declined one it is the measurement: answering an
    // axis it did not claim is the drift the port closes, and a stub
    // whose declined answer was already the marker could not tell a port
    // that asks the gate from one that does not. See
    // `an_axis_a_probe_did_not_declare_is_refused_by_the_port`.
    impl ArtefactProbe for MetaOnlyJpeg {
        fn declares(&self) -> &'static [FormatClaim] {
            JPEG_META_ONLY
        }

        fn content_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> ContentRegion {
            ContentRegion::EmptySpan
        }

        fn meta_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> MaterialMeta {
            MaterialMeta::EmptySpan
        }
    }

    impl ArtefactProbe for ContentOnlyJpeg {
        fn declares(&self) -> &'static [FormatClaim] {
            JPEG_CONTENT_ONLY
        }

        fn content_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> ContentRegion {
            ContentRegion::EmptySpan
        }

        fn meta_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> MaterialMeta {
            MaterialMeta::EmptySpan
        }
    }

    impl ArtefactProbe for DeafProbe {
        fn declares(&self) -> &'static [FormatClaim] {
            JPEG_NEITHER_AXIS
        }

        fn content_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> ContentRegion {
            ContentRegion::EmptySpan
        }

        fn meta_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> MaterialMeta {
            MaterialMeta::EmptySpan
        }
    }

    impl ArtefactProbe for SilentProbe {
        fn declares(&self) -> &'static [FormatClaim] {
            &[]
        }

        fn content_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> ContentRegion {
            ContentRegion::EmptySpan
        }

        fn meta_of(
            &self,
            _bytes: &[u8],
            _declared_mime: Option<&MimeType>,
            _gate: GateOpen,
        ) -> MaterialMeta {
            MaterialMeta::EmptySpan
        }
    }

    /// The complaint, for a case that is supposed to produce one.
    ///
    /// The rejections below are asserted on their text rather than on
    /// `is_err`, because three of the rule's clauses can refuse the same
    /// registry and a bare `is_err` would be satisfied by whichever
    /// fired first. Removing the clause a case was written for would
    /// then leave the case passing — which is the failure this test
    /// exists to rule out, arriving in the test itself.
    fn refusal(checked: Result<(), String>) -> String {
        checked.expect_err("this registry is supposed to be refused")
    }

    /// **The rule accepts one axis, and rejects none, no reader, and two
    /// readers.**
    ///
    /// [`completeness`] is checked against this build's registry, which
    /// has one probe answering both axes — a set on which "at least one
    /// axis" and "both axes" are the same rule, so the version this
    /// replaces passed for a reason that had nothing to do with what it
    /// meant to say. The stubs are the disagreement, made into
    /// registries the rule can be handed.
    ///
    /// The version this replaces required both axes of every claimed
    /// format. It would fail the first case here, and the cheapest way
    /// to make it pass would be a claim on the content axis behind a
    /// `content` that returns a marker — which reads every artefact of
    /// the format off disk to hand it to a probe that has nothing to say
    /// about them.
    ///
    /// The stubs are also why the rule is not vacuous now that a probe
    /// states its own formats. Read off one honest probe, every clause
    /// holds by construction; the five registries below are the ones
    /// where the declaration and the answers can still part company.
    ///
    /// Two of the five are accepted, and the one at the end is what
    /// keeps the collision clause honest. The clause counts claimants
    /// *per axis*, and both this module's doc and [`completeness`]'s
    /// argue at length that counting across the two would refuse a
    /// legitimate registry — a claim nothing measured until the
    /// content-only stub arrived to build that registry. Degrade the
    /// clause to either-axis and it is the only case that moves
    /// [measured: `content: 2, meta: 2` on `Image(Jpeg)`, the other four
    /// unchanged].
    #[test]
    fn the_rule_wants_one_axis_rather_than_both() {
        // One axis is enough: JPEG is read on the meta axis only, and
        // that is a complete answer rather than half of one.
        completeness(&[&png::PngProbe, &MetaOnlyJpeg])
            .expect("a probe reading one axis covers the format it declares");

        // Neither axis is not. The declaration is the one accepted above
        // with the reading taken away, so what separates the two cases
        // is the probe's answer and nothing else.
        assert!(
            refusal(completeness(&[&png::PngProbe, &DeafProbe]))
                .contains("with neither axis behind it"),
            "a format declared with nothing reading it has to fail on that clause"
        );

        // A probe no format reaches: registered, asked nothing,
        // invisible from both its own side and the column's.
        assert!(
            refusal(completeness(&[&png::PngProbe, &SilentProbe])).contains("declares no format"),
            "a probe that declares nothing has to fail on that clause"
        );

        // Two claimants on one axis. The same probe twice is the
        // smallest way to say it, and it is exactly what a second PNG
        // adapter would look like to this rule.
        assert!(
            refusal(completeness(&[&png::PngProbe, &png::PngProbe]))
                .contains("more than one probe on one axis"),
            "one format claimed twice on one axis has to fail on that clause"
        );

        // One container, two probes, opposite axes — the pixels read by
        // one and the metadata by the other. This is the registry the
        // per-axis count exists to permit: count claimants across both
        // axes instead and JPEG has two, so a legitimate arrangement is
        // refused as a collision.
        //
        // Last rather than beside the other accepted registry, because
        // this is the assertion a change to that clause trips: from up
        // there it would panic before the three refusals below ever ran,
        // and a degraded clause would be reported as one failure instead
        // of shown against the cases it still handles.
        completeness(&[&png::PngProbe, &MetaOnlyJpeg, &ContentOnlyJpeg]).expect(
            "a content probe and a meta probe over one mime are two answers, not two claimants",
        );
    }

    /// **One artefact, one answer, whichever door it came through.**
    ///
    /// The registry picks a probe from the declared mime and nothing
    /// else, so an unnamed row is refused here before any probe is asked.
    /// A probe reached directly has to refuse it too. Both directions
    /// are asserted over the *same bytes* — a real PNG, signature and
    /// all — because the disagreement worth catching is exactly the one
    /// a signature check creates: sniff the bytes and the direct call
    /// says `cr1-sha256:…` where the registry says `unsupported:unknown`,
    /// and those are not two spellings of one answer but a digest and a
    /// statement that there is none.
    ///
    /// Not reachable from `fingerprint::hash_artefact` today — its gate
    /// short-circuits an unnamed row before the read — which is why it
    /// is pinned here rather than left to be discovered. An artefact
    /// whose bytes are held rather than named reaches
    /// [`content`]/[`meta`] without that gate, and would then be
    /// fingerprinted differently from the identical file on disk.
    ///
    /// The refusal half of that is now the port's rather than this
    /// probe's, so the first four assertions hold for any probe and not
    /// because `PngProbe` was written carefully — see
    /// [`an_axis_a_probe_did_not_declare_is_refused_by_the_port`], which
    /// says the same thing over a probe that was not. What stays worth
    /// asserting here is that the two labels are one *value*: the
    /// registry's fall-through and the port's refusal are computed in
    /// different places and a row must not be able to tell them apart.
    #[test]
    fn a_probe_reached_directly_answers_what_the_registry_would_have() {
        // No claim, real PNG bytes: a marker on both sides, and the same
        // marker the caller stores when it skips the read entirely.
        assert_eq!(
            content(CARD_PNG, None),
            png::PngProbe.content(CARD_PNG, None),
            "an unnamed row is refused the same way through either door"
        );
        assert_eq!(
            meta(CARD_PNG, None),
            png::PngProbe.meta(CARD_PNG, None),
            "and on the meta axis too"
        );
        assert_eq!(
            content(CARD_PNG, None),
            asterism_core::domain::content_region::ContentRegion::Unsupported("unknown".into())
        );
        assert_eq!(
            meta(CARD_PNG, None),
            asterism_core::domain::material_meta::MaterialMeta::Unsupported("unknown".into())
        );

        // The same bytes with the claim restored: a digest on both
        // sides, and the same digest. Without this half the assertions
        // above are satisfied by a probe that refuses everything.
        let declared = mime("image/png");
        assert_eq!(
            content(CARD_PNG, Some(&declared)),
            png::PngProbe.content(CARD_PNG, Some(&declared))
        );
        assert_eq!(
            meta(CARD_PNG, Some(&declared)),
            png::PngProbe.meta(CARD_PNG, Some(&declared))
        );
        assert!(matches!(
            content(CARD_PNG, Some(&declared)),
            ContentRegion::Digest(_)
        ));
        assert!(matches!(
            meta(CARD_PNG, Some(&declared)),
            MaterialMeta::Digest { .. }
        ));
    }

    /// **An axis a probe did not declare answers the marker, whatever
    /// the probe itself would have said.**
    ///
    /// The stubs answer `EmptySpan` everywhere, including the axes they
    /// declined, so each call below reaches a reading that would gladly
    /// answer — and the assertion is that it does not get to. What is
    /// measured is that [`ProbeGates::content`] asks the gate at all,
    /// rather than that any particular probe remembered to: written
    /// against `PngProbe`, which refuses by its own signature check, the
    /// same assertions would pass with the gate removed.
    ///
    /// The other half is not assertable and does not need to be.
    /// [`ArtefactProbe::content_of`] takes a
    /// [`GateOpen`](asterism_core::domain::probe::GateOpen), whose field
    /// is private to `asterism-core`, so no expression here constructs
    /// one — `MetaOnlyJpeg.content_of(bytes, None, GateOpen(()))` is
    /// `E0423: cannot initialize a tuple struct which contains private
    /// fields`. Reaching a reading past its gate is a compile error
    /// rather than a test somebody has to remember to write.
    #[test]
    fn an_axis_a_probe_did_not_declare_is_refused_by_the_port() {
        let jpeg = MimeType::Image(ImageFormat::Jpeg);
        let declared = Some(&jpeg);

        assert_eq!(
            MetaOnlyJpeg.content(CARD_PNG, declared),
            content_region::unsupported_format(declared),
            "the content axis is not this probe's to answer"
        );
        assert_eq!(
            ContentOnlyJpeg.meta(CARD_PNG, declared),
            material_meta::unsupported_format(declared),
            "nor the meta axis this one's"
        );

        // The control. Without it a port that refused everything would
        // satisfy the two above, and the probes would be unreachable
        // rather than gated.
        assert_eq!(
            MetaOnlyJpeg.meta(CARD_PNG, declared),
            MaterialMeta::EmptySpan,
            "the axis it did declare reaches it"
        );
        assert_eq!(
            ContentOnlyJpeg.content(CARD_PNG, declared),
            ContentRegion::EmptySpan,
            "and so does this one's"
        );
    }

    /// A format nothing claims gets the value it would have got without
    /// the file being opened.
    ///
    /// The two paths have to agree or one artefact carries two different
    /// markers depending on whether the caller reached the read.
    ///
    /// `image/jpeg` used to stand in the list below as the parameterised
    /// spelling. It moved out when a probe claimed it, and `image/gif`
    /// took its place, so the spelling-normalisation half of the loop is
    /// still measured on a format nothing here reads.
    ///
    /// It then spent a slice in a third position — claimed on the
    /// content axis and refused on the meta one — and that case is gone
    /// from this file with the claim: **no format this build registers
    /// is claimed on one axis only.** What used to be asserted here is
    /// asserted over the stubs instead
    /// ([`an_axis_a_probe_did_not_declare_is_refused_by_the_port`]),
    /// which is where it belongs — a registry that answered its gates
    /// per *probe* rather than per axis is a defect in this module, and
    /// a real probe that happens to claim both axes cannot show it
    /// either way.
    #[test]
    fn an_unclaimed_format_falls_through_to_the_same_marker_either_way() {
        for raw in [
            Some("video/mp4"),
            Some("IMAGE/GIF; charset=binary"),
            Some("text/plain"),
            Some("   "),
            None,
        ] {
            let parsed = raw.map(MimeType::parse);
            let declared = parsed.as_ref();
            assert!(!walks_content(declared), "{raw:?} has no probe here");
            assert!(!walks_meta(declared), "{raw:?} has no probe here");
            assert_eq!(
                content(b"whatever these bytes are", declared),
                content_region::unsupported_format(declared),
                "{raw:?}"
            );
            assert_eq!(
                meta(b"whatever these bytes are", declared),
                material_meta::unsupported_format(declared),
                "{raw:?}"
            );
        }

        // …and the formats that do route, however they are spelled. A
        // registry reading either as "no" would stop fingerprinting the
        // corpus these axes exist for.
        for raw in [
            "image/png",
            "IMAGE/PNG; charset=binary",
            " image/png ",
            "image/jpeg",
            "IMAGE/JPEG; charset=binary",
            " image/jpeg ",
        ] {
            assert!(walks_content(Some(&mime(raw))), "{raw:?} routes to a probe");
            assert!(walks_meta(Some(&mime(raw))), "{raw:?} routes to a probe");
        }
    }
}
